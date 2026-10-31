//! 课表（Schedule）仓储：区域 / 房间 / 课表 / 触发规则 / 事件与投递。
//!
//! 契约见 `docs/schedule-design.md` 第 4 节；v2 时区调整见第 10 节；表结构见
//! `migrations/0009_schedule.sql`。
//!
//! 约定：
//! - 领域模型来自 `qe_domain::schedule`，不认识 SQL；`FromRow` 行模型只出现在本文件；
//! - 对外时间（`TIME`）统一输出 `"HH:MM"`；日期/时间点用 `chrono` 类型；
//! - v2：具体时间一律存 UTC；本地钟面时间由「区域 IANA 时区」解释，房间 / 课表 / 实例
//!   通过 JOIN 继承生效时区。课表实例（occurrence）按课表生效时区计算「本地今天」，
//!   只重写未来实例（`start_at > now()`），保留已过去的实例与事件引用。

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

use qe_domain::{
    device::{DeviceOnlineState, DeviceStatus, RoomDevice},
    models::Page,
    schedule::{
        Area, DeliveryJob, DeliveryStatus, DeliveryTarget, EventFilter, EventRule,
        ExpansionSchedule, NewArea, NewRoom, NewSchedule, NewScheduleEvent, Occurrence,
        OccurrenceSource, PlannedOccurrence, Room, Schedule, ScheduleEvent, ScheduleEventDelivery,
        ScheduleException, ScheduleExceptionKind, ScheduleExceptionSlot, TriggerKind, UpdateArea,
        UpdateRoom, UpdateSchedule, ValidatedEventRule, ValidatedExceptionSlot,
        ValidatedWeeklySlot, WeeklySlot, expand_occurrences, local_date_of,
    },
};

use crate::error::{DbError, DbResult};

/// 与 `qe-device-pki::messaging::MAX_UNACKED_DOWNLINK` 保持一致：
/// 单台设备未 ACK 的下行消息达到该值即拒绝入队（背压，避免设备失联时写爆库）。
const MAX_UNACKED_DOWNLINK: i64 = 1000;
/// 与 `qe-device-pki::messaging::MAX_MESSAGE_BYTES` 保持一致（64 KiB）。
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_DOWNLINK_KIND_LEN: usize = 64;

/// `TIME` 列对外统一输出 `"HH:MM"`。
fn format_time(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

fn exception_kind_str(kind: ScheduleExceptionKind) -> &'static str {
    match kind {
        ScheduleExceptionKind::Cancel => "cancel",
        ScheduleExceptionKind::Replace => "replace",
    }
}

fn exception_kind_from_str(raw: &str) -> ScheduleExceptionKind {
    match raw {
        "replace" => ScheduleExceptionKind::Replace,
        _ => ScheduleExceptionKind::Cancel,
    }
}

fn occurrence_source_str(source: OccurrenceSource) -> &'static str {
    match source {
        OccurrenceSource::Weekly => "weekly",
        OccurrenceSource::Custom => "custom",
    }
}

fn occurrence_source_from_str(raw: &str) -> OccurrenceSource {
    match raw {
        "custom" => OccurrenceSource::Custom,
        _ => OccurrenceSource::Weekly,
    }
}

fn delivery_target_str(target: DeliveryTarget) -> &'static str {
    match target {
        DeliveryTarget::Pubsub => "pubsub",
        DeliveryTarget::Downlink => "downlink",
        DeliveryTarget::Webhook => "webhook",
    }
}

fn delivery_target_from_str(raw: &str) -> DeliveryTarget {
    match raw {
        "downlink" => DeliveryTarget::Downlink,
        "webhook" => DeliveryTarget::Webhook,
        _ => DeliveryTarget::Pubsub,
    }
}

fn delivery_status_from_str(raw: &str) -> DeliveryStatus {
    match raw {
        "delivered" => DeliveryStatus::Delivered,
        "failed" => DeliveryStatus::Failed,
        "skipped" => DeliveryStatus::Skipped,
        _ => DeliveryStatus::Pending,
    }
}

// ---------------------------------------------------------------------------
// 行模型与映射
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
struct AreaRow {
    id: Uuid,
    code: String,
    name: String,
    description: String,
    timezone: String,
    parent_id: Option<Uuid>,
    is_active: bool,
    metadata: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<AreaRow> for Area {
    fn from(row: AreaRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            description: row.description,
            timezone: row.timezone,
            parent_id: row.parent_id,
            is_active: row.is_active,
            metadata: row.metadata,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct RoomRow {
    id: Uuid,
    area_id: Uuid,
    area_name: String,
    code: String,
    name: String,
    capacity: i32,
    device_slot: Option<String>,
    webhook_url: Option<String>,
    /// 继承自区域的 IANA 时区（只读）。
    timezone: String,
    is_active: bool,
    metadata: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// 该槽位最近的注册记录（live 状态优先，见 `ROOM_DEVICE_JOINS`）。
    device_serial: Option<String>,
    device_status: Option<String>,
    /// `device_session.state`。
    device_online: Option<String>,
    device_last_seen_at: Option<DateTime<Utc>>,
    /// 该槽位存在未使用且未过期的 bootstrap token。
    device_pending_token: bool,
}

impl From<RoomRow> for Room {
    fn from(row: RoomRow) -> Self {
        // v3（契约 §11.3）：`device_slot` 非空时构造房间设备摘要；
        // 已分配槽位但尚未注册的设备表现为 `serial = None`。
        let device = row.device_slot.as_ref().map(|slot| RoomDevice {
            device_slot: slot.clone(),
            serial: row.device_serial.clone(),
            status: row
                .device_status
                .as_deref()
                .and_then(|raw| raw.parse::<DeviceStatus>().ok()),
            online: row
                .device_online
                .as_deref()
                .and_then(|raw| raw.parse::<DeviceOnlineState>().ok()),
            last_seen_at: row.device_last_seen_at,
            pending_token: row.device_pending_token,
        });

        Self {
            id: row.id,
            area_id: row.area_id,
            area_name: row.area_name,
            code: row.code,
            name: row.name,
            capacity: row.capacity,
            device_slot: row.device_slot,
            webhook_url: row.webhook_url,
            timezone: row.timezone,
            is_active: row.is_active,
            metadata: row.metadata,
            created_at: row.created_at,
            updated_at: row.updated_at,
            device,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ScheduleRow {
    id: Uuid,
    room_id: Uuid,
    room_name: String,
    name: String,
    start_date: NaiveDate,
    end_date: NaiveDate,
    /// 继承自房间所属区域的 IANA 时区（只读）。
    timezone: String,
    is_active: bool,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ScheduleRow> for Schedule {
    fn from(row: ScheduleRow) -> Self {
        Self {
            id: row.id,
            room_id: row.room_id,
            room_name: row.room_name,
            name: row.name,
            start_date: row.start_date,
            end_date: row.end_date,
            timezone: row.timezone,
            is_active: row.is_active,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// `rebuild_occurrences` 内部使用的课表核心列（不需要房间名）。
#[derive(Debug, Clone, FromRow)]
struct ScheduleCoreRow {
    room_id: Uuid,
    start_date: NaiveDate,
    end_date: NaiveDate,
    timezone: String,
}

#[derive(Debug, Clone, FromRow)]
struct WeeklySlotRow {
    id: Uuid,
    schedule_id: Uuid,
    day_of_week: i16,
    start_time: NaiveTime,
    end_time: NaiveTime,
    course_name: String,
    teacher: String,
    room_id: Option<Uuid>,
    room_name: Option<String>,
    note: String,
    created_at: DateTime<Utc>,
}

impl From<WeeklySlotRow> for WeeklySlot {
    fn from(row: WeeklySlotRow) -> Self {
        Self {
            id: row.id,
            schedule_id: row.schedule_id,
            day_of_week: row.day_of_week,
            start_time: format_time(row.start_time),
            end_time: format_time(row.end_time),
            course_name: row.course_name,
            teacher: row.teacher,
            room_id: row.room_id,
            room_name: row.room_name,
            note: row.note,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ExceptionRow {
    id: Uuid,
    schedule_id: Uuid,
    exception_date: NaiveDate,
    kind: String,
    note: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct ExceptionSlotRow {
    id: Uuid,
    exception_id: Uuid,
    start_time: NaiveTime,
    end_time: NaiveTime,
    course_name: String,
    teacher: String,
    room_id: Option<Uuid>,
    room_name: Option<String>,
    note: String,
}

impl From<ExceptionSlotRow> for ScheduleExceptionSlot {
    fn from(row: ExceptionSlotRow) -> Self {
        Self {
            id: row.id,
            exception_id: row.exception_id,
            start_time: format_time(row.start_time),
            end_time: format_time(row.end_time),
            course_name: row.course_name,
            teacher: row.teacher,
            room_id: row.room_id,
            room_name: row.room_name,
            note: row.note,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct OccurrenceRow {
    id: Uuid,
    schedule_id: Uuid,
    room_id: Uuid,
    occurrence_date: NaiveDate,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    course_name: String,
    teacher: String,
    source: String,
    source_slot_id: Uuid,
    note: String,
    /// 生效时区：课表 → 房间 → 区域（只读）。
    timezone: String,
}

impl From<OccurrenceRow> for Occurrence {
    fn from(row: OccurrenceRow) -> Self {
        Self {
            id: row.id,
            schedule_id: row.schedule_id,
            room_id: row.room_id,
            occurrence_date: row.occurrence_date,
            start_at: row.start_at,
            end_at: row.end_at,
            course_name: row.course_name,
            teacher: row.teacher,
            source: occurrence_source_from_str(&row.source),
            source_slot_id: row.source_slot_id,
            note: row.note,
            timezone: row.timezone,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct EventRuleRow {
    id: Uuid,
    room_id: Uuid,
    trigger_kind: String,
    offset_minutes: i32,
    at_time: Option<NaiveTime>,
    enabled: bool,
    config: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<EventRuleRow> for EventRule {
    fn from(row: EventRuleRow) -> Self {
        Self {
            id: row.id,
            room_id: row.room_id,
            trigger_kind: row
                .trigger_kind
                .parse::<TriggerKind>()
                .unwrap_or(TriggerKind::OccurrenceStart),
            offset_minutes: row.offset_minutes,
            at_time: row.at_time.map(format_time),
            enabled: row.enabled,
            config: row.config,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct EnabledRuleRow {
    id: Uuid,
    room_id: Uuid,
    trigger_kind: String,
    offset_minutes: i32,
    at_time: Option<NaiveTime>,
    enabled: bool,
    config: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    room_code: String,
    room_name: String,
    device_slot: Option<String>,
    webhook_url: Option<String>,
    /// 房间所属区域的 IANA 时区。
    timezone: String,
}

#[derive(Debug, Clone, FromRow)]
struct ScheduleEventRow {
    id: Uuid,
    event_key: String,
    room_id: Uuid,
    /// 事件生成时所属区域的 IANA 时区快照。
    timezone: String,
    schedule_id: Option<Uuid>,
    occurrence_id: Option<Uuid>,
    rule_id: Option<Uuid>,
    event_kind: String,
    title: String,
    payload: Value,
    triggered_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl From<ScheduleEventRow> for ScheduleEvent {
    fn from(row: ScheduleEventRow) -> Self {
        Self {
            id: row.id,
            event_key: row.event_key,
            room_id: row.room_id,
            timezone: row.timezone,
            schedule_id: row.schedule_id,
            occurrence_id: row.occurrence_id,
            rule_id: row.rule_id,
            event_kind: row.event_kind,
            title: row.title,
            payload: row.payload,
            triggered_at: row.triggered_at,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct EventDeliveryRow {
    id: i64,
    event_id: Uuid,
    target: String,
    status: String,
    attempts: i32,
    next_attempt_at: DateTime<Utc>,
    last_error: Option<String>,
    delivered_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<EventDeliveryRow> for ScheduleEventDelivery {
    fn from(row: EventDeliveryRow) -> Self {
        Self {
            id: row.id,
            event_id: row.event_id,
            target: delivery_target_from_str(&row.target),
            status: delivery_status_from_str(&row.status),
            attempts: row.attempts,
            next_attempt_at: row.next_attempt_at,
            last_error: row.last_error,
            delivered_at: row.delivered_at,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct DeliveryJobRow {
    delivery_id: i64,
    event_id: Uuid,
    target: String,
    attempts: i32,
    event_kind: String,
    payload: Value,
    room_id: Uuid,
    room_code: String,
    room_name: String,
    device_slot: Option<String>,
    webhook_url: Option<String>,
}

impl From<DeliveryJobRow> for DeliveryJob {
    fn from(row: DeliveryJobRow) -> Self {
        Self {
            delivery_id: row.delivery_id,
            event_id: row.event_id,
            target: delivery_target_from_str(&row.target),
            attempts: row.attempts,
            event_kind: row.event_kind,
            payload: row.payload,
            room_id: row.room_id,
            room_code: row.room_code,
            room_name: row.room_name,
            device_slot: row.device_slot,
            webhook_url: row.webhook_url,
        }
    }
}

// ---------------------------------------------------------------------------
// 通用 SQL 片段
// ---------------------------------------------------------------------------

const AREA_COLUMNS: &str = "a.id, a.code, a.name, a.description, a.timezone, a.parent_id,
     a.is_active, a.metadata, a.created_at, a.updated_at";

const ROOM_COLUMNS: &str = "r.id, r.area_id, a.name AS area_name, r.code, r.name, r.capacity,
     r.device_slot, r.webhook_url, a.timezone AS timezone, r.is_active, r.metadata,
     r.created_at, r.updated_at,
     de.serial AS device_serial, de.status AS device_status,
     ds.state AS device_online, ds.updated_at AS device_last_seen_at,
     EXISTS (SELECT 1 FROM device_bootstrap_token t
             WHERE t.device_slot = r.device_slot AND t.used_at IS NULL AND t.expires_at > now())
         AS device_pending_token";

/// v3（契约 §11.3）：取房间槽位最近的注册记录（live 状态优先）+ 持久化会话。
/// 房间未绑定槽位时 `de` / `ds` 全为 NULL，`Room.device` 由 `device_slot` 决定为 `None`。
const ROOM_DEVICE_JOINS: &str = "LEFT JOIN LATERAL (
         SELECT de.serial, de.status, de.client_info, de.issued_at
         FROM device_enrollment de
         WHERE de.device_slot = r.device_slot
         ORDER BY (de.status IN ('pending', 'issued', 'active')) DESC,
                  de.issued_at DESC NULLS LAST, de.serial
         LIMIT 1
     ) de ON TRUE
     LEFT JOIN device_session ds ON ds.serial = de.serial";

const SCHEDULE_COLUMNS: &str = "s.id, s.room_id, r.name AS room_name, s.name, s.start_date,
     s.end_date, a.timezone AS timezone, s.is_active, s.created_by, s.created_at, s.updated_at";

const OCCURRENCE_COLUMNS: &str = "o.id, o.schedule_id, o.room_id, o.occurrence_date,
     o.start_at, o.end_at, o.course_name, o.teacher, o.source, o.source_slot_id, o.note,
     a.timezone AS timezone";

/// 实例的生效时区：课表 → 所属房间 → 区域（与展开时使用的时区一致）。
const OCCURRENCE_JOINS: &str = "JOIN schedules s ON s.id = o.schedule_id
     JOIN rooms r ON r.id = s.room_id
     JOIN areas a ON a.id = r.area_id";

const EVENT_RULE_COLUMNS: &str =
    "er.id, er.room_id, er.trigger_kind, er.offset_minutes, er.at_time,
     er.enabled, er.config, er.created_at, er.updated_at";

const EVENT_COLUMNS: &str = "id, event_key, room_id, timezone, schedule_id, occurrence_id, rule_id,
     event_kind, title, payload, triggered_at, created_at";

const DELIVERY_COLUMNS: &str = "id, event_id, target, status, attempts, next_attempt_at,
     last_error, delivered_at, created_at";

const EXCEPTION_COLUMNS: &str =
    "id, schedule_id, exception_date, kind, note, created_at, updated_at";

const WEEKLY_SLOT_COLUMNS: &str = "w.id, w.schedule_id, w.day_of_week, w.start_time, w.end_time,
     w.course_name, w.teacher, w.room_id, rm.name AS room_name, w.note, w.created_at";

const EXCEPTION_SLOT_COLUMNS: &str = "es.id, es.exception_id, es.start_time, es.end_time,
     es.course_name, es.teacher, es.room_id, rm.name AS room_name, es.note";

/// 读取某课表的周模板（含调课房间名）。
async fn weekly_slots_on(
    connection: &mut PgConnection,
    schedule_id: Uuid,
) -> DbResult<Vec<WeeklySlot>> {
    let sql = format!(
        "SELECT {WEEKLY_SLOT_COLUMNS}
         FROM schedule_weekly_slots w
         LEFT JOIN rooms rm ON rm.id = w.room_id
         WHERE w.schedule_id = $1
         ORDER BY w.day_of_week, w.start_time, w.id"
    );
    let rows = sqlx::query_as::<_, WeeklySlotRow>(&sql)
        .bind(schedule_id)
        .fetch_all(&mut *connection)
        .await?;
    Ok(rows.into_iter().map(WeeklySlot::from).collect())
}

/// 读取课表的单日覆盖（可限定日期范围），slots 按开始时间排序。
async fn exceptions_on(
    connection: &mut PgConnection,
    schedule_id: Uuid,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> DbResult<Vec<ScheduleException>> {
    let sql = format!(
        "SELECT {EXCEPTION_COLUMNS}
         FROM schedule_exceptions
         WHERE schedule_id = $1
           AND ($2::date IS NULL OR exception_date >= $2)
           AND ($3::date IS NULL OR exception_date <= $3)
         ORDER BY exception_date"
    );
    let rows = sqlx::query_as::<_, ExceptionRow>(&sql)
        .bind(schedule_id)
        .bind(from)
        .bind(to)
        .fetch_all(&mut *connection)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let slot_sql = format!(
        "SELECT {EXCEPTION_SLOT_COLUMNS}
         FROM schedule_exception_slots es
         LEFT JOIN rooms rm ON rm.id = es.room_id
         WHERE es.exception_id = ANY($1)
         ORDER BY es.start_time, es.id"
    );
    let slot_rows = sqlx::query_as::<_, ExceptionSlotRow>(&slot_sql)
        .bind(&ids)
        .fetch_all(&mut *connection)
        .await?;

    let mut slots_by_exception: std::collections::HashMap<Uuid, Vec<ScheduleExceptionSlot>> =
        std::collections::HashMap::new();
    for row in slot_rows {
        slots_by_exception
            .entry(row.exception_id)
            .or_default()
            .push(ScheduleExceptionSlot::from(row));
    }

    Ok(rows
        .into_iter()
        .map(|row| ScheduleException {
            id: row.id,
            schedule_id: row.schedule_id,
            exception_date: row.exception_date,
            kind: exception_kind_from_str(&row.kind),
            note: row.note,
            slots: slots_by_exception.remove(&row.id).unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

/// 读取某房间的触发规则。
async fn event_rules_on(connection: &mut PgConnection, room_id: Uuid) -> DbResult<Vec<EventRule>> {
    let sql = format!(
        "SELECT {EVENT_RULE_COLUMNS}
         FROM schedule_event_rules er
         WHERE er.room_id = $1
         ORDER BY er.created_at, er.id"
    );
    let rows = sqlx::query_as::<_, EventRuleRow>(&sql)
        .bind(room_id)
        .fetch_all(&mut *connection)
        .await?;
    Ok(rows.into_iter().map(EventRule::from).collect())
}

// ---------------------------------------------------------------------------
// 区域
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AreaRepo {
    db: PgPool,
}

impl AreaRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        keyword: &str,
        include_inactive: bool,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Area>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM areas a
             WHERE ($1 = ''
                    OR a.code ILIKE '%' || $1 || '%'
                    OR a.name ILIKE '%' || $1 || '%')
               AND ($2 OR a.is_active)",
        )
        .bind(keyword)
        .bind(include_inactive)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {AREA_COLUMNS}
             FROM areas a
             WHERE ($1 = ''
                    OR a.code ILIKE '%' || $1 || '%'
                    OR a.name ILIKE '%' || $1 || '%')
               AND ($2 OR a.is_active)
             ORDER BY a.created_at DESC, a.id
             LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query_as::<_, AreaRow>(&sql)
            .bind(keyword)
            .bind(include_inactive)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(Area::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<Area>> {
        let sql = format!("SELECT {AREA_COLUMNS} FROM areas a WHERE a.id = $1");
        let row = sqlx::query_as::<_, AreaRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Area::from))
    }

    pub async fn create(&self, input: &NewArea) -> DbResult<Area> {
        let sql = format!(
            "WITH inserted AS (
                INSERT INTO areas (code, name, description, timezone, parent_id, is_active, metadata)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING *
             )
             SELECT {AREA_COLUMNS}
             FROM inserted a"
        );
        let row = sqlx::query_as::<_, AreaRow>(&sql)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.description)
            .bind(&input.timezone)
            .bind(input.parent_id)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_one(&self.db)
            .await?;
        Ok(Area::from(row))
    }

    pub async fn update(&self, id: Uuid, input: &UpdateArea) -> DbResult<Option<Area>> {
        let (parent_changed, parent_id) = match input.parent_id {
            Some(value) => (true, value),
            None => (false, None),
        };

        let sql = format!(
            "WITH updated AS (
                UPDATE areas SET
                    code = COALESCE($2, code),
                    name = COALESCE($3, name),
                    description = COALESCE($4, description),
                    timezone = COALESCE($5, timezone),
                    parent_id = CASE WHEN $6 THEN $7 ELSE parent_id END,
                    is_active = COALESCE($8, is_active),
                    metadata = COALESCE($9, metadata)
                WHERE id = $1
                RETURNING *
             )
             SELECT {AREA_COLUMNS}
             FROM updated a"
        );
        let row = sqlx::query_as::<_, AreaRow>(&sql)
            .bind(id)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.description)
            .bind(&input.timezone)
            .bind(parent_changed)
            .bind(parent_id)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Area::from))
    }

    /// 删除区域。仍有房间时返回 `Conflict`（rooms.area_id 是 `ON DELETE RESTRICT`）；
    /// 这里先做业务检查，让 API 得到明确的 409 而不是外键错误的 400。
    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let has_rooms: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM rooms WHERE area_id = $1)")
                .bind(id)
                .fetch_one(&self.db)
                .await?;
        if has_rooms {
            return Err(DbError::conflict("区域下仍有房间，无法删除"));
        }

        let deleted = sqlx::query("DELETE FROM areas WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}

// ---------------------------------------------------------------------------
// 房间
// ---------------------------------------------------------------------------

/// 按 id 读取房间（含 v3 设备摘要）。`RoomRepo` 与 `DeviceRepo::bind_to_room`
/// 共用，保证「创建 / 更新 / 绑定」返回的房间结构一致。
pub(crate) async fn room_on(connection: &mut PgConnection, id: Uuid) -> DbResult<Option<Room>> {
    let sql = format!(
        "SELECT {ROOM_COLUMNS}
         FROM rooms r
         JOIN areas a ON a.id = r.area_id
         {ROOM_DEVICE_JOINS}
         WHERE r.id = $1"
    );
    let row = sqlx::query_as::<_, RoomRow>(&sql)
        .bind(id)
        .fetch_optional(&mut *connection)
        .await?;
    Ok(row.map(Room::from))
}

#[derive(Clone)]
pub struct RoomRepo {
    db: PgPool,
}

impl RoomRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        area_id: Option<Uuid>,
        keyword: &str,
        include_inactive: bool,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Room>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM rooms r
             WHERE ($1::uuid IS NULL OR r.area_id = $1)
               AND ($2 = ''
                    OR r.code ILIKE '%' || $2 || '%'
                    OR r.name ILIKE '%' || $2 || '%')
               AND ($3 OR r.is_active)",
        )
        .bind(area_id)
        .bind(keyword)
        .bind(include_inactive)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {ROOM_COLUMNS}
             FROM rooms r
             JOIN areas a ON a.id = r.area_id
             {ROOM_DEVICE_JOINS}
             WHERE ($1::uuid IS NULL OR r.area_id = $1)
               AND ($2 = ''
                    OR r.code ILIKE '%' || $2 || '%'
                    OR r.name ILIKE '%' || $2 || '%')
               AND ($3 OR r.is_active)
             ORDER BY r.created_at DESC, r.id
             LIMIT $4 OFFSET $5"
        );
        let rows = sqlx::query_as::<_, RoomRow>(&sql)
            .bind(area_id)
            .bind(keyword)
            .bind(include_inactive)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(Room::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<Room>> {
        let mut connection = self.db.acquire().await?;
        room_on(&mut connection, id).await
    }

    pub async fn create(&self, input: &NewRoom) -> DbResult<Room> {
        let sql = format!(
            "WITH inserted AS (
                INSERT INTO rooms
                    (area_id, code, name, capacity, device_slot, webhook_url,
                     is_active, metadata)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                RETURNING *
             )
             SELECT {ROOM_COLUMNS}
             FROM inserted r
             JOIN areas a ON a.id = r.area_id
             {ROOM_DEVICE_JOINS}"
        );
        let row = sqlx::query_as::<_, RoomRow>(&sql)
            .bind(input.area_id)
            .bind(&input.code)
            .bind(&input.name)
            .bind(input.capacity)
            .bind(&input.device_slot)
            .bind(&input.webhook_url)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_one(&self.db)
            .await?;
        Ok(Room::from(row))
    }

    pub async fn update(&self, id: Uuid, input: &UpdateRoom) -> DbResult<Option<Room>> {
        let (device_slot_changed, device_slot) = match &input.device_slot {
            Some(value) => (true, value.clone()),
            None => (false, None),
        };
        let (webhook_changed, webhook_url) = match &input.webhook_url {
            Some(value) => (true, value.clone()),
            None => (false, None),
        };

        let sql = format!(
            "WITH updated AS (
                UPDATE rooms SET
                    area_id = COALESCE($2, area_id),
                    code = COALESCE($3, code),
                    name = COALESCE($4, name),
                    capacity = COALESCE($5, capacity),
                    device_slot = CASE WHEN $6 THEN $7 ELSE device_slot END,
                    webhook_url = CASE WHEN $8 THEN $9 ELSE webhook_url END,
                    is_active = COALESCE($10, is_active),
                    metadata = COALESCE($11, metadata)
                WHERE id = $1
                RETURNING *
             )
             SELECT {ROOM_COLUMNS}
             FROM updated r
             JOIN areas a ON a.id = r.area_id
             {ROOM_DEVICE_JOINS}"
        );
        let row = sqlx::query_as::<_, RoomRow>(&sql)
            .bind(id)
            .bind(input.area_id)
            .bind(&input.code)
            .bind(&input.name)
            .bind(input.capacity)
            .bind(device_slot_changed)
            .bind(device_slot)
            .bind(webhook_changed)
            .bind(webhook_url)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Room::from))
    }

    /// 删除房间。课表、触发规则与事件由外键级联清理。
    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let deleted = sqlx::query("DELETE FROM rooms WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    pub async fn find_by_device_slot(&self, slot: &str) -> DbResult<Option<Room>> {
        let sql = format!(
            "SELECT {ROOM_COLUMNS}
             FROM rooms r
             JOIN areas a ON a.id = r.area_id
             {ROOM_DEVICE_JOINS}
             WHERE r.device_slot = $1"
        );
        let row = sqlx::query_as::<_, RoomRow>(&sql)
            .bind(slot)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Room::from))
    }
}

// ---------------------------------------------------------------------------
// 课表
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ScheduleRepo {
    db: PgPool,
}

impl ScheduleRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        room_id: Option<Uuid>,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Schedule>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM schedules s
             WHERE ($1::uuid IS NULL OR s.room_id = $1)",
        )
        .bind(room_id)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {SCHEDULE_COLUMNS}
             FROM schedules s
             JOIN rooms r ON r.id = s.room_id
             JOIN areas a ON a.id = r.area_id
             WHERE ($1::uuid IS NULL OR s.room_id = $1)
             ORDER BY s.start_date DESC, s.created_at DESC, s.id
             LIMIT $2 OFFSET $3"
        );
        let rows = sqlx::query_as::<_, ScheduleRow>(&sql)
            .bind(room_id)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(Schedule::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<Schedule>> {
        let sql = format!(
            "SELECT {SCHEDULE_COLUMNS}
             FROM schedules s
             JOIN rooms r ON r.id = s.room_id
             JOIN areas a ON a.id = r.area_id
             WHERE s.id = $1"
        );
        let row = sqlx::query_as::<_, ScheduleRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Schedule::from))
    }

    pub async fn create(&self, input: &NewSchedule) -> DbResult<Schedule> {
        let sql = format!(
            "WITH inserted AS (
                INSERT INTO schedules
                    (room_id, name, start_date, end_date, is_active, created_by)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING *
             )
             SELECT {SCHEDULE_COLUMNS}
             FROM inserted s
             JOIN rooms r ON r.id = s.room_id
             JOIN areas a ON a.id = r.area_id"
        );
        let row = sqlx::query_as::<_, ScheduleRow>(&sql)
            .bind(input.room_id)
            .bind(&input.name)
            .bind(input.start_date)
            .bind(input.end_date)
            .bind(input.is_active)
            .bind(input.created_by)
            .fetch_one(&self.db)
            .await?;
        Ok(Schedule::from(row))
    }

    pub async fn update(&self, id: Uuid, input: &UpdateSchedule) -> DbResult<Option<Schedule>> {
        let sql = format!(
            "WITH updated AS (
                UPDATE schedules SET
                    name = COALESCE($2, name),
                    start_date = COALESCE($3, start_date),
                    end_date = COALESCE($4, end_date),
                    is_active = COALESCE($5, is_active)
                WHERE id = $1
                RETURNING *
             )
             SELECT {SCHEDULE_COLUMNS}
             FROM updated s
             JOIN rooms r ON r.id = s.room_id
             JOIN areas a ON a.id = r.area_id"
        );
        let row = sqlx::query_as::<_, ScheduleRow>(&sql)
            .bind(id)
            .bind(&input.name)
            .bind(input.start_date)
            .bind(input.end_date)
            .bind(input.is_active)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Schedule::from))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let deleted = sqlx::query("DELETE FROM schedules WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    pub async fn weekly_slots(&self, schedule_id: Uuid) -> DbResult<Vec<WeeklySlot>> {
        let mut connection = self.db.acquire().await?;
        weekly_slots_on(&mut connection, schedule_id).await
    }

    /// 全量替换周模板（事务内 DELETE + INSERT）；课表不存在返回 `None`。
    pub async fn replace_weekly_slots(
        &self,
        schedule_id: Uuid,
        slots: &[ValidatedWeeklySlot],
    ) -> DbResult<Option<Vec<WeeklySlot>>> {
        let mut tx = self.db.begin().await?;

        let exists = sqlx::query_scalar::<_, Uuid>("SELECT id FROM schedules WHERE id = $1")
            .bind(schedule_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        sqlx::query("DELETE FROM schedule_weekly_slots WHERE schedule_id = $1")
            .bind(schedule_id)
            .execute(&mut *tx)
            .await?;

        for slot in slots {
            sqlx::query(
                "INSERT INTO schedule_weekly_slots
                    (schedule_id, day_of_week, start_time, end_time, course_name, teacher, room_id, note)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(schedule_id)
            .bind(slot.day_of_week)
            .bind(slot.start_time)
            .bind(slot.end_time)
            .bind(&slot.course_name)
            .bind(&slot.teacher)
            .bind(slot.room_id)
            .bind(&slot.note)
            .execute(&mut *tx)
            .await?;
        }

        let result = weekly_slots_on(&mut tx, schedule_id).await?;
        tx.commit().await?;
        Ok(Some(result))
    }

    pub async fn exceptions(
        &self,
        schedule_id: Uuid,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    ) -> DbResult<Vec<ScheduleException>> {
        let mut connection = self.db.acquire().await?;
        exceptions_on(&mut connection, schedule_id, from, to).await
    }

    /// upsert 单日覆盖，并全量替换该覆盖的 slots。
    pub async fn upsert_exception(
        &self,
        schedule_id: Uuid,
        exception_date: NaiveDate,
        kind: ScheduleExceptionKind,
        note: &str,
        slots: &[ValidatedExceptionSlot],
    ) -> DbResult<Option<ScheduleException>> {
        let mut tx = self.db.begin().await?;

        let exists = sqlx::query_scalar::<_, Uuid>("SELECT id FROM schedules WHERE id = $1")
            .bind(schedule_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        let exception = sqlx::query_as::<_, ExceptionRow>(
            "INSERT INTO schedule_exceptions (schedule_id, exception_date, kind, note)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (schedule_id, exception_date) DO UPDATE
                SET kind = EXCLUDED.kind, note = EXCLUDED.note
             RETURNING *",
        )
        .bind(schedule_id)
        .bind(exception_date)
        .bind(exception_kind_str(kind))
        .bind(note)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM schedule_exception_slots WHERE exception_id = $1")
            .bind(exception.id)
            .execute(&mut *tx)
            .await?;

        for slot in slots {
            sqlx::query(
                "INSERT INTO schedule_exception_slots
                    (exception_id, start_time, end_time, course_name, teacher, room_id, note)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(exception.id)
            .bind(slot.start_time)
            .bind(slot.end_time)
            .bind(&slot.course_name)
            .bind(&slot.teacher)
            .bind(slot.room_id)
            .bind(&slot.note)
            .execute(&mut *tx)
            .await?;
        }

        let result = exceptions_on(
            &mut tx,
            schedule_id,
            Some(exception_date),
            Some(exception_date),
        )
        .await?;
        tx.commit().await?;
        Ok(result.into_iter().next())
    }

    pub async fn delete_exception(
        &self,
        schedule_id: Uuid,
        exception_date: NaiveDate,
    ) -> DbResult<bool> {
        let deleted = sqlx::query(
            "DELETE FROM schedule_exceptions WHERE schedule_id = $1 AND exception_date = $2",
        )
        .bind(schedule_id)
        .bind(exception_date)
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(deleted > 0)
    }

    /// 展开并重写该课表「未来」的排课实例（保留已过去的实例与事件引用），返回写入条数。
    ///
    /// 事务内：`SELECT ... FOR UPDATE` 锁课表 → 读周模板 / 单日覆盖 →
    /// `expand_occurrences` → 删除 `start_at > now()` 的实例 → 批量插入。
    /// 插入同样只写 `start_at > now()` 的实例，避免与「今天已过去」的旧实例唯一键冲突。
    pub async fn rebuild_occurrences(
        &self,
        schedule_id: Uuid,
        horizon_days: i64,
    ) -> DbResult<usize> {
        let mut tx = self.db.begin().await?;

        let schedule = sqlx::query_as::<_, ScheduleCoreRow>(
            "SELECT s.room_id, s.start_date, s.end_date, a.timezone
             FROM schedules s
             JOIN rooms r ON r.id = s.room_id
             JOIN areas a ON a.id = r.area_id
             WHERE s.id = $1
             FOR UPDATE OF s",
        )
        .bind(schedule_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(schedule) = schedule else {
            tx.rollback().await?;
            return Ok(0);
        };

        let weekly_slots = weekly_slots_on(&mut tx, schedule_id).await?;

        // 本地今天：按课表生效的 IANA 时区取「今天」，决定重写窗口。
        let local_today = local_date_of(Utc::now(), &schedule.timezone)
            .map_err(|error| DbError::internal(error.to_string()))?;
        let from = local_today.max(schedule.start_date);
        let to = schedule
            .end_date
            .min(local_today + Duration::days(horizon_days.max(1)));

        let exceptions = if from <= to {
            exceptions_on(&mut tx, schedule_id, Some(from), Some(to)).await?
        } else {
            Vec::new()
        };

        let plan: Vec<PlannedOccurrence> = if from <= to {
            let expansion = ExpansionSchedule {
                start_date: schedule.start_date,
                end_date: schedule.end_date,
                timezone: schedule.timezone.clone(),
            };
            expand_occurrences(&expansion, &weekly_slots, &exceptions, from, to)
                .map_err(|error| DbError::internal(error.to_string()))?
        } else {
            Vec::new()
        };

        sqlx::query("DELETE FROM schedule_occurrences WHERE schedule_id = $1 AND start_at > now()")
            .bind(schedule_id)
            .execute(&mut *tx)
            .await?;

        let inserted = if plan.is_empty() {
            0
        } else {
            let mut room_ids: Vec<Uuid> = Vec::with_capacity(plan.len());
            let mut dates: Vec<NaiveDate> = Vec::with_capacity(plan.len());
            let mut starts: Vec<DateTime<Utc>> = Vec::with_capacity(plan.len());
            let mut ends: Vec<DateTime<Utc>> = Vec::with_capacity(plan.len());
            let mut course_names: Vec<String> = Vec::with_capacity(plan.len());
            let mut teachers: Vec<String> = Vec::with_capacity(plan.len());
            let mut sources: Vec<String> = Vec::with_capacity(plan.len());
            let mut source_slot_ids: Vec<Uuid> = Vec::with_capacity(plan.len());
            let mut notes: Vec<String> = Vec::with_capacity(plan.len());

            for occurrence in &plan {
                room_ids.push(occurrence.room_override.unwrap_or(schedule.room_id));
                dates.push(occurrence.occurrence_date);
                starts.push(occurrence.start_at);
                ends.push(occurrence.end_at);
                course_names.push(occurrence.course_name.clone());
                teachers.push(occurrence.teacher.clone());
                sources.push(occurrence_source_str(occurrence.source).to_owned());
                source_slot_ids.push(occurrence.source_slot_id);
                notes.push(occurrence.note.clone());
            }

            sqlx::query(
                "INSERT INTO schedule_occurrences
                    (schedule_id, room_id, occurrence_date, start_at, end_at,
                     course_name, teacher, source, source_slot_id, note)
                 SELECT $1, u.room_id, u.occurrence_date, u.start_at, u.end_at,
                        u.course_name, u.teacher, u.source, u.source_slot_id, u.note
                 FROM unnest(
                     $2::uuid[], $3::date[], $4::timestamptz[], $5::timestamptz[],
                     $6::text[], $7::text[], $8::text[], $9::uuid[], $10::text[]
                 ) AS u(room_id, occurrence_date, start_at, end_at,
                        course_name, teacher, source, source_slot_id, note)
                 WHERE u.start_at > now()
                 ON CONFLICT (schedule_id, source, source_slot_id, occurrence_date) DO NOTHING",
            )
            .bind(schedule_id)
            .bind(&room_ids)
            .bind(&dates)
            .bind(&starts)
            .bind(&ends)
            .bind(&course_names)
            .bind(&teachers)
            .bind(&sources)
            .bind(&source_slot_ids)
            .bind(&notes)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        };

        tx.commit().await?;
        Ok(inserted as usize)
    }

    /// 遍历全部启用课表逐个重建未来实例。
    pub async fn rebuild_all_occurrences(&self, horizon_days: i64) -> DbResult<usize> {
        let schedule_ids =
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM schedules WHERE is_active ORDER BY id")
                .fetch_all(&self.db)
                .await?;

        let mut total = 0;
        for schedule_id in schedule_ids {
            total += self.rebuild_occurrences(schedule_id, horizon_days).await?;
        }
        Ok(total)
    }

    /// 重建某区域下所有课表的未来实例（区域时区变更后调用）。
    pub async fn rebuild_area_occurrences(
        &self,
        area_id: Uuid,
        horizon_days: i64,
    ) -> DbResult<usize> {
        let schedule_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT s.id
             FROM schedules s
             JOIN rooms r ON r.id = s.room_id
             WHERE r.area_id = $1
             ORDER BY s.id",
        )
        .bind(area_id)
        .fetch_all(&self.db)
        .await?;

        let mut total = 0;
        for schedule_id in schedule_ids {
            total += self.rebuild_occurrences(schedule_id, horizon_days).await?;
        }
        Ok(total)
    }

    pub async fn occurrences(
        &self,
        schedule_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> DbResult<Vec<Occurrence>> {
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM schedule_occurrences o
             {OCCURRENCE_JOINS}
             WHERE o.schedule_id = $1 AND o.start_at >= $2 AND o.start_at < $3
             ORDER BY o.start_at, o.id"
        );
        self.query_occurrences(&sql, schedule_id, from, to).await
    }

    pub async fn room_occurrences(
        &self,
        room_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> DbResult<Vec<Occurrence>> {
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM schedule_occurrences o
             {OCCURRENCE_JOINS}
             WHERE o.room_id = $1 AND o.start_at >= $2 AND o.start_at < $3
             ORDER BY o.start_at, o.id"
        );
        self.query_occurrences(&sql, room_id, from, to).await
    }

    /// 事件引擎：`start_at ∈ [from, to)` 的全部实例。
    pub async fn occurrences_starting(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> DbResult<Vec<Occurrence>> {
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM schedule_occurrences o
             {OCCURRENCE_JOINS}
             WHERE o.start_at >= $1 AND o.start_at < $2
             ORDER BY o.start_at, o.id"
        );
        self.query_occurrences_window(&sql, from, to).await
    }

    /// 事件引擎：`end_at ∈ [from, to)` 的全部实例。
    pub async fn occurrences_ending(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> DbResult<Vec<Occurrence>> {
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM schedule_occurrences o
             {OCCURRENCE_JOINS}
             WHERE o.end_at >= $1 AND o.end_at < $2
             ORDER BY o.end_at, o.id"
        );
        self.query_occurrences_window(&sql, from, to).await
    }

    /// 事件引擎：与 `[from, to)` 有交集的实例（`start_at < to AND end_at > from`）。
    pub async fn occurrences_overlapping(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> DbResult<Vec<Occurrence>> {
        let sql = format!(
            "SELECT {OCCURRENCE_COLUMNS}
             FROM schedule_occurrences o
             {OCCURRENCE_JOINS}
             WHERE o.start_at < $2 AND o.end_at > $1
             ORDER BY o.start_at, o.id"
        );
        self.query_occurrences_window(&sql, from, to).await
    }

    async fn query_occurrences(
        &self,
        sql: &str,
        id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> DbResult<Vec<Occurrence>> {
        let rows = sqlx::query_as::<_, OccurrenceRow>(sql)
            .bind(id)
            .bind(from)
            .bind(to)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(Occurrence::from).collect())
    }

    async fn query_occurrences_window(
        &self,
        sql: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> DbResult<Vec<Occurrence>> {
        let rows = sqlx::query_as::<_, OccurrenceRow>(sql)
            .bind(from)
            .bind(to)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(Occurrence::from).collect())
    }
}

// ---------------------------------------------------------------------------
// 触发规则
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct EventRuleRepo {
    db: PgPool,
}

impl EventRuleRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list_by_room(&self, room_id: Uuid) -> DbResult<Vec<EventRule>> {
        let mut connection = self.db.acquire().await?;
        event_rules_on(&mut connection, room_id).await
    }

    /// 全量替换房间的触发规则；房间不存在返回 `None`。
    pub async fn replace_for_room(
        &self,
        room_id: Uuid,
        rules: &[ValidatedEventRule],
    ) -> DbResult<Option<Vec<EventRule>>> {
        let mut tx = self.db.begin().await?;

        let exists = sqlx::query_scalar::<_, Uuid>("SELECT id FROM rooms WHERE id = $1")
            .bind(room_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        sqlx::query("DELETE FROM schedule_event_rules WHERE room_id = $1")
            .bind(room_id)
            .execute(&mut *tx)
            .await?;

        for rule in rules {
            sqlx::query(
                "INSERT INTO schedule_event_rules
                    (room_id, trigger_kind, offset_minutes, at_time, enabled, config)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(room_id)
            .bind(rule.trigger_kind.as_str())
            .bind(rule.offset_minutes)
            .bind(rule.at_time)
            .bind(rule.enabled)
            .bind(&rule.config)
            .execute(&mut *tx)
            .await?;
        }

        let result = event_rules_on(&mut tx, room_id).await?;
        tx.commit().await?;
        Ok(Some(result))
    }

    /// 引擎用：所有启用规则 + 房间快照。
    pub async fn enabled_rules(&self) -> DbResult<Vec<EnabledRule>> {
        let sql = format!(
            "SELECT {EVENT_RULE_COLUMNS},
                    r.code AS room_code, r.name AS room_name, r.device_slot, r.webhook_url,
                    a.timezone AS timezone
             FROM schedule_event_rules er
             JOIN rooms r ON r.id = er.room_id
             JOIN areas a ON a.id = r.area_id
             WHERE er.enabled
             ORDER BY er.room_id, er.created_at"
        );
        let rows = sqlx::query_as::<_, EnabledRuleRow>(&sql)
            .fetch_all(&self.db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| EnabledRule {
                rule: EventRule {
                    id: row.id,
                    room_id: row.room_id,
                    trigger_kind: row
                        .trigger_kind
                        .parse::<TriggerKind>()
                        .unwrap_or(TriggerKind::OccurrenceStart),
                    offset_minutes: row.offset_minutes,
                    at_time: row.at_time.map(format_time),
                    enabled: row.enabled,
                    config: row.config,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                },
                room_code: row.room_code,
                room_name: row.room_name,
                device_slot: row.device_slot,
                webhook_url: row.webhook_url,
                timezone: row.timezone,
            })
            .collect())
    }
}

/// 引擎用的「规则 + 房间快照」。
#[derive(Debug, Clone)]
pub struct EnabledRule {
    pub rule: EventRule,
    pub room_code: String,
    pub room_name: String,
    pub device_slot: Option<String>,
    pub webhook_url: Option<String>,
    /// 房间所属区域的 IANA 时区（`daily_time` 按它解释 `at_time`）。
    pub timezone: String,
}

// ---------------------------------------------------------------------------
// 事件与投递
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ScheduleEventRepo {
    db: PgPool,
}

impl ScheduleEventRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 幂等插入事件；已存在返回 `None`。插入成功时同事务写入目标投递行。
    pub async fn insert_if_absent(
        &self,
        event: &NewScheduleEvent,
        targets: &[DeliveryTarget],
    ) -> DbResult<Option<ScheduleEvent>> {
        let mut tx = self.db.begin().await?;

        let row = sqlx::query_as::<_, ScheduleEventRow>(
            "INSERT INTO schedule_events
                (event_key, room_id, timezone, schedule_id, occurrence_id, rule_id,
                 event_kind, title, payload, triggered_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (event_key) DO NOTHING
             RETURNING *",
        )
        .bind(&event.event_key)
        .bind(event.room_id)
        .bind(&event.timezone)
        .bind(event.schedule_id)
        .bind(event.occurrence_id)
        .bind(event.rule_id)
        .bind(&event.event_kind)
        .bind(&event.title)
        .bind(&event.payload)
        .bind(event.triggered_at)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(None);
        };

        if !targets.is_empty() {
            let target_names: Vec<String> = targets
                .iter()
                .map(|target| delivery_target_str(*target).to_owned())
                .collect();
            sqlx::query(
                "INSERT INTO schedule_event_deliveries (event_id, target)
                 SELECT $1, t FROM unnest($2::text[]) AS t
                 ON CONFLICT DO NOTHING",
            )
            .bind(row.id)
            .bind(&target_names)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(Some(ScheduleEvent::from(row)))
    }

    pub async fn list(
        &self,
        filter: &EventFilter,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<ScheduleEvent>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM schedule_events e
             WHERE ($1::uuid IS NULL OR e.room_id = $1)
               AND ($2::text IS NULL OR e.event_kind = $2)
               AND ($3::timestamptz IS NULL OR e.triggered_at >= $3)
               AND ($4::timestamptz IS NULL OR e.triggered_at < $4)",
        )
        .bind(filter.room_id)
        .bind(&filter.kind)
        .bind(filter.from)
        .bind(filter.to)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {EVENT_COLUMNS}
             FROM schedule_events e
             WHERE ($1::uuid IS NULL OR e.room_id = $1)
               AND ($2::text IS NULL OR e.event_kind = $2)
               AND ($3::timestamptz IS NULL OR e.triggered_at >= $3)
               AND ($4::timestamptz IS NULL OR e.triggered_at < $4)
             ORDER BY e.triggered_at DESC, e.id DESC
             LIMIT $5 OFFSET $6"
        );
        let rows = sqlx::query_as::<_, ScheduleEventRow>(&sql)
            .bind(filter.room_id)
            .bind(&filter.kind)
            .bind(filter.from)
            .bind(filter.to)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(ScheduleEvent::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<ScheduleEvent>> {
        let sql = format!("SELECT {EVENT_COLUMNS} FROM schedule_events e WHERE e.id = $1");
        let row = sqlx::query_as::<_, ScheduleEventRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(ScheduleEvent::from))
    }

    pub async fn deliveries_for(&self, event_id: Uuid) -> DbResult<Vec<ScheduleEventDelivery>> {
        let sql = format!(
            "SELECT {DELIVERY_COLUMNS}
             FROM schedule_event_deliveries
             WHERE event_id = $1
             ORDER BY id"
        );
        let rows = sqlx::query_as::<_, EventDeliveryRow>(&sql)
            .bind(event_id)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(ScheduleEventDelivery::from).collect())
    }

    /// 到期待投递：`status='pending' AND next_attempt_at <= now()`，JOIN 事件、房间与规则，
    /// 按 `next_attempt_at, id` 升序取最多 `limit` 条。
    ///
    /// `webhook_url` 在这里就完成解析（契约 §6.1）：规则 `config.webhook_url` 优先，
    /// 其次房间 `webhook_url`；这样引擎拿到 `DeliveryJob` 即可直接投递，无需回查规则。
    pub async fn due_deliveries(&self, limit: i64) -> DbResult<Vec<DeliveryJob>> {
        let rows = sqlx::query_as::<_, DeliveryJobRow>(
            "SELECT d.id AS delivery_id,
                    d.event_id,
                    d.target,
                    d.attempts,
                    e.event_kind,
                    e.payload,
                    e.room_id,
                    r.code AS room_code,
                    r.name AS room_name,
                    r.device_slot,
                    COALESCE(NULLIF(er.config->>'webhook_url', ''), r.webhook_url) AS webhook_url
             FROM schedule_event_deliveries d
             JOIN schedule_events e ON e.id = d.event_id
             JOIN rooms r ON r.id = e.room_id
             LEFT JOIN schedule_event_rules er ON er.id = e.rule_id
             WHERE d.status = 'pending' AND d.next_attempt_at <= now()
             ORDER BY d.next_attempt_at, d.id
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(DeliveryJob::from).collect())
    }

    pub async fn mark_delivered(&self, delivery_id: i64) -> DbResult<()> {
        sqlx::query(
            "UPDATE schedule_event_deliveries
             SET status = 'delivered', attempts = attempts + 1,
                 last_error = NULL, delivered_at = now()
             WHERE id = $1",
        )
        .bind(delivery_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn mark_skipped(&self, delivery_id: i64, reason: &str) -> DbResult<()> {
        sqlx::query(
            "UPDATE schedule_event_deliveries
             SET status = 'skipped', last_error = $2
             WHERE id = $1",
        )
        .bind(delivery_id)
        .bind(reason)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn mark_retry(
        &self,
        delivery_id: i64,
        attempts: i32,
        next_attempt_at: DateTime<Utc>,
        error: &str,
    ) -> DbResult<()> {
        sqlx::query(
            "UPDATE schedule_event_deliveries
             SET status = 'pending', attempts = $2, next_attempt_at = $3, last_error = $4
             WHERE id = $1",
        )
        .bind(delivery_id)
        .bind(attempts)
        .bind(next_attempt_at)
        .bind(error)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn mark_failed(&self, delivery_id: i64, attempts: i32, error: &str) -> DbResult<()> {
        sqlx::query(
            "UPDATE schedule_event_deliveries
             SET status = 'failed', attempts = $2, last_error = $3
             WHERE id = $1",
        )
        .bind(delivery_id)
        .bind(attempts)
        .bind(error)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// 重置某事件的全部投递为 `pending`（重放），返回受影响行数。
    pub async fn replay_event(&self, event_id: Uuid) -> DbResult<u64> {
        let affected = sqlx::query(
            "UPDATE schedule_event_deliveries
             SET status = 'pending', attempts = 0, next_attempt_at = now(),
                 last_error = NULL, delivered_at = NULL
             WHERE event_id = $1",
        )
        .bind(event_id)
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(affected)
    }

    /// 单条投递重试：仅当投递属于该事件时生效，返回是否命中。
    pub async fn retry_delivery(&self, event_id: Uuid, delivery_id: i64) -> DbResult<bool> {
        let affected = sqlx::query(
            "UPDATE schedule_event_deliveries
             SET status = 'pending', attempts = 0, next_attempt_at = now(),
                 last_error = NULL, delivered_at = NULL
             WHERE id = $2 AND event_id = $1",
        )
        .bind(event_id)
        .bind(delivery_id)
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(affected > 0)
    }

    /// 向房间设备槽位投递下行消息：返回 `Some(serial)` / `None`（无存活设备）。
    ///
    /// 语义与 `qe-device-pki::messaging::downlink_publish_to_slot` 一致：
    /// - 选 `status ∈ {issued, active}` 的证书，`active` 优先、`activated_at` 最新；
    /// - 未 ACK 的下行消息达到 1000 条时返回 `Conflict`（背压）；
    /// - 成功插入 `device_downlink` 后返回设备 `serial`（调用方可据此唤醒长连接）。
    pub async fn enqueue_downlink_for_slot(
        &self,
        slot: &str,
        kind: &str,
        payload: &str,
    ) -> DbResult<Option<String>> {
        if kind.is_empty() || kind.len() > MAX_DOWNLINK_KIND_LEN {
            return Err(DbError::bad_request("kind 长度必须在 1..=64"));
        }
        if payload.len() > MAX_MESSAGE_BYTES {
            return Err(DbError::bad_request("payload 超过 65536 字节"));
        }

        let mut tx = self.db.begin().await?;

        let device = sqlx::query_as::<_, (String, String)>(
            "SELECT serial, device_slot
             FROM device_enrollment
             WHERE device_slot = $1 AND status IN ('issued', 'active')
             ORDER BY (status = 'active') DESC, activated_at DESC NULLS LAST
             LIMIT 1",
        )
        .bind(slot)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((serial, device_slot)) = device else {
            tx.rollback().await?;
            return Ok(None);
        };

        let unacked: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM device_downlink
             WHERE serial = $1 AND delivered_at IS NULL",
        )
        .bind(&serial)
        .fetch_one(&mut *tx)
        .await?;
        if unacked >= MAX_UNACKED_DOWNLINK {
            return Err(DbError::conflict("设备未 ACK 的下行消息已达上限"));
        }

        sqlx::query(
            "INSERT INTO device_downlink (serial, device_slot, kind, payload)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&serial)
        .bind(&device_slot)
        .bind(kind)
        .bind(payload)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(serial))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_is_formatted_as_hh_mm() {
        assert_eq!(
            format_time(NaiveTime::from_hms_opt(8, 5, 0).expect("时间")),
            "08:05"
        );
        assert_eq!(
            format_time(NaiveTime::from_hms_opt(23, 59, 30).expect("时间")),
            "23:59"
        );
    }

    #[test]
    fn exception_kind_round_trip() {
        for kind in [
            ScheduleExceptionKind::Cancel,
            ScheduleExceptionKind::Replace,
        ] {
            assert_eq!(exception_kind_from_str(exception_kind_str(kind)), kind);
        }
    }

    #[test]
    fn occurrence_source_round_trip() {
        for source in [OccurrenceSource::Weekly, OccurrenceSource::Custom] {
            assert_eq!(
                occurrence_source_from_str(occurrence_source_str(source)),
                source
            );
        }
    }

    #[test]
    fn delivery_target_and_status_parsing() {
        for target in [
            DeliveryTarget::Pubsub,
            DeliveryTarget::Downlink,
            DeliveryTarget::Webhook,
        ] {
            assert_eq!(
                delivery_target_from_str(delivery_target_str(target)),
                target
            );
        }
        assert_eq!(
            delivery_status_from_str("delivered"),
            DeliveryStatus::Delivered
        );
        assert_eq!(delivery_status_from_str("failed"), DeliveryStatus::Failed);
        assert_eq!(delivery_status_from_str("skipped"), DeliveryStatus::Skipped);
        // 库数据异常时退化为 pending，而不是 panic。
        assert_eq!(delivery_status_from_str("unknown"), DeliveryStatus::Pending);
    }
}
