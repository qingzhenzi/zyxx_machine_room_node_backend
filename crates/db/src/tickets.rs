//! 工单仓储：`tickets` + `ticket_events` 的读写与历史分析聚合。
//!
//! 契约见 `docs/design/ticket-implementation-contract.md` §5，表结构见
//! `migrations/0018_tickets.sql`。
//!
//! 设计要点：
//! - **去重**：自动单用部分唯一索引 `tickets_dedupe_active` +
//!   `INSERT ... ON CONFLICT DO NOTHING`；冲突时改为「累加重复发生」，
//!   多副本并发只会有一行插入成功，不依赖 Redis 锁；
//! - **occurred 幂等**：`ticket_events_occurred_key` 部分唯一索引保证同一
//!   `(ticket_id, source_type, source_id)` 只追加一次，因此 `occurrence_count`
//!   不会每个 tick 都膨胀；
//! - **状态机**：所有流转在事务内 `SELECT ... FOR UPDATE` 锁行后校验，
//!   非法流转返回 `DbError::Conflict`（API 映射 409）；
//! - **时间线**：所有写操作在同一事务内追加 `ticket_events`；
//! - 全部使用运行时 `sqlx` 查询（不依赖编译期 DATABASE_URL 宏）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{FromRow, PgExecutor, PgPool};
use uuid::Uuid;

use qe_domain::models::Page;
use qe_domain::ticket::{
    NewTicket, Ticket, TicketAssignee, TicketAssigneeStat, TicketCategory, TicketCategoryStat,
    TicketContext, TicketDayStat, TicketEvent, TicketEventKind, TicketExportRow, TicketFilter,
    TicketPriority, TicketPriorityStat, TicketSource, TicketStats, TicketStatsTotals, TicketStatus,
    TicketStreamEvent, TicketSummary, TicketTopDevice, TicketTopRoom, TransitionInput,
    default_due_at,
};

use crate::error::{DbError, DbResult};

/// 工单查询列（含上报人/处理人显示名）。
const TICKET_COLUMNS: &str = "t.id, t.code, t.title, t.description, t.status, t.priority, \
     t.category, t.source, t.source_type, t.source_id, t.dedupe_key, t.area_id, t.room_id, \
     t.serial, t.device_slot, t.reporter_id, reporter.display_name AS reporter_name, \
     t.assignee_id, assignee.display_name AS assignee_name, t.due_at, t.acknowledged_at, \
     t.resolved_at, t.closed_at, t.resolution, t.root_cause, \
     t.occurrence_count::bigint AS occurrence_count, \
     t.last_seen_at, t.metadata, \
     t.created_at, t.updated_at";

const TICKET_FROM: &str = "FROM tickets t \
     LEFT JOIN users reporter ON reporter.id = t.reporter_id \
     LEFT JOIN users assignee ON assignee.id = t.assignee_id";

const EVENT_COLUMNS: &str = "e.id, e.ticket_id, e.kind, e.actor_id, \
     COALESCE(actor.display_name, '系统') AS actor_name, e.from_status, e.to_status, e.note, \
     e.payload, e.source_type, e.source_id, e.created_at";

const EVENT_FROM: &str = "FROM ticket_events e \
     LEFT JOIN users actor ON actor.id = e.actor_id";

/// 列表/计数共用的过滤条件（$1..$14）。
const LIST_WHERE: &str = "
 WHERE ($1::text IS NULL OR t.status = $1)
   AND ($2::text IS NULL OR t.category = $2)
   AND ($3::text IS NULL OR t.priority = $3)
   AND ($4::text IS NULL OR t.source = $4)
   AND ($5::uuid IS NULL OR t.assignee_id = $5)
   AND (NOT $6::bool OR (t.assignee_id IS NULL AND $5::uuid IS NULL))
   AND ($7::uuid IS NULL OR t.assignee_id = $7)
   AND (NOT $8::bool OR t.status IN ('open','in_progress'))
   AND (NOT $9::bool OR (t.due_at IS NOT NULL AND t.due_at < now()
                         AND t.status IN ('open','in_progress')))
   AND ($10::uuid IS NULL OR COALESCE(t.area_id, room.area_id) = $10)
   AND ($11::uuid IS NULL OR t.room_id = $11)
   AND ($12::text IS NULL OR t.serial = $12)
   AND ($13::text IS NULL OR t.title ILIKE ('%' || $13 || '%') ESCAPE '\\'
                        OR t.description ILIKE ('%' || $13 || '%') ESCAPE '\\'
                        OR t.code ILIKE ('%' || $13 || '%') ESCAPE '\\')
   AND ($14::timestamptz IS NULL OR t.created_at >= $14)
   AND ($15::timestamptz IS NULL OR t.created_at < $15)
   AND ($16::uuid[] IS NULL OR COALESCE(t.area_id, room.area_id) = ANY($16))";

#[derive(Debug, FromRow)]
struct TicketRow {
    id: Uuid,
    code: String,
    title: String,
    description: String,
    status: String,
    priority: String,
    category: String,
    source: String,
    source_type: Option<String>,
    source_id: Option<String>,
    dedupe_key: Option<String>,
    area_id: Option<Uuid>,
    room_id: Option<Uuid>,
    serial: Option<String>,
    device_slot: Option<String>,
    reporter_id: Option<Uuid>,
    reporter_name: Option<String>,
    assignee_id: Option<Uuid>,
    assignee_name: Option<String>,
    due_at: Option<DateTime<Utc>>,
    acknowledged_at: Option<DateTime<Utc>>,
    resolved_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    resolution: Option<String>,
    root_cause: Option<String>,
    occurrence_count: i64,
    last_seen_at: DateTime<Utc>,
    metadata: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TicketRow> for Ticket {
    fn from(row: TicketRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            title: row.title,
            description: row.description,
            // 数据库 CHECK 约束保证枚举合法；解析失败时退化为默认值而不是 panic。
            status: row.status.parse().unwrap_or(TicketStatus::Open),
            priority: row.priority.parse().unwrap_or(TicketPriority::Normal),
            category: row.category.parse().unwrap_or(TicketCategory::Other),
            source: row.source.parse().unwrap_or(TicketSource::Manual),
            source_type: row.source_type,
            source_id: row.source_id,
            dedupe_key: row.dedupe_key,
            area_id: row.area_id,
            room_id: row.room_id,
            serial: row.serial,
            device_slot: row.device_slot,
            reporter_id: row.reporter_id,
            reporter_name: row.reporter_name,
            assignee_id: row.assignee_id,
            assignee_name: row.assignee_name,
            due_at: row.due_at,
            acknowledged_at: row.acknowledged_at,
            resolved_at: row.resolved_at,
            closed_at: row.closed_at,
            resolution: row.resolution,
            root_cause: row.root_cause,
            occurrence_count: row.occurrence_count,
            last_seen_at: row.last_seen_at,
            metadata: row.metadata,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct TicketEventRow {
    id: i64,
    ticket_id: Uuid,
    kind: String,
    actor_id: Option<Uuid>,
    actor_name: String,
    from_status: Option<String>,
    to_status: Option<String>,
    note: String,
    payload: Value,
    source_type: Option<String>,
    source_id: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<TicketEventRow> for TicketEvent {
    fn from(row: TicketEventRow) -> Self {
        Self {
            id: row.id,
            ticket_id: row.ticket_id,
            kind: row.kind.parse().unwrap_or(TicketEventKind::StatusChanged),
            actor_id: row.actor_id,
            actor_name: row.actor_name,
            from_status: row.from_status.and_then(|value| value.parse().ok()),
            to_status: row.to_status.and_then(|value| value.parse().ok()),
            note: row.note,
            payload: row.payload,
            source_type: row.source_type,
            source_id: row.source_id,
            created_at: row.created_at,
        }
    }
}

/// 自动规则信号：机器健康异常（某 serial 最新连续 N 条非 ok）。
#[derive(Debug, Clone, FromRow)]
pub struct MachineHealthSignal {
    pub serial: String,
    pub device_slot: String,
    pub snapshot_id: i64,
    pub health_status: String,
    pub health_score: Option<i32>,
    pub collected_at: DateTime<Utc>,
    pub payload: Value,
    pub room_id: Option<Uuid>,
    pub area_id: Option<Uuid>,
}

/// 自动规则信号：设备离线超过阈值。
#[derive(Debug, Clone, FromRow)]
pub struct DeviceOfflineSignal {
    pub serial: String,
    pub device_slot: String,
    pub session_updated_at: DateTime<Utc>,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    pub area_id: Uuid,
    pub area_name: String,
}

/// 自动规则信号：课表事件投递失败（按房间 + 目标 + 本地日期聚合的原始行）。
#[derive(Debug, Clone, FromRow)]
pub struct FailedDeliverySignal {
    pub delivery_id: i64,
    pub event_id: Uuid,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    pub area_id: Uuid,
    pub target: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub triggered_at: DateTime<Utc>,
    pub payload: Value,
    /// `areas.timezone` 下的本地日期（`YYYY-MM-DD`），用于去重键。
    pub local_date: String,
}

/// 自动规则信号：在线事件 webhook 投递失败（P1）。
#[derive(Debug, Clone, FromRow)]
pub struct PresenceDeliverySignal {
    pub delivery_id: i64,
    pub event_id: i64,
    pub serial: String,
    pub target: String,
    pub last_error: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub room_id: Option<Uuid>,
    pub area_id: Option<Uuid>,
    pub local_date: String,
}

/// 自动规则信号：存储分发失败/过期（P1）。
#[derive(Debug, Clone, FromRow)]
pub struct TransferSignal {
    pub transfer_id: Uuid,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    pub area_id: Uuid,
    pub area_name: String,
    pub status: String,
    pub error: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub local_date: String,
}

/// 自动规则信号：镜像验证失败（P1）。
#[derive(Debug, Clone, FromRow)]
pub struct ImageVerifySignal {
    pub verification_id: Uuid,
    pub artifact_id: Uuid,
    pub image_name: String,
    pub image_version: String,
    pub report: Value,
    pub verified_at: DateTime<Utc>,
}

/// 超时提醒候选（P1）。
#[derive(Debug, Clone, FromRow)]
pub struct OverdueTicket {
    pub id: Uuid,
    pub code: String,
    pub title: String,
    pub due_at: DateTime<Utc>,
    pub notify_user_id: Option<Uuid>,
    pub notify_email: Option<String>,
    pub notify_name: Option<String>,
}

#[derive(Debug, FromRow)]
struct ExportRow {
    code: String,
    title: String,
    status: String,
    priority: String,
    category: String,
    source: String,
    area_name: Option<String>,
    room_name: Option<String>,
    device_slot: Option<String>,
    serial: Option<String>,
    reporter_name: Option<String>,
    assignee_name: Option<String>,
    created_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    resolution: Option<String>,
}

impl From<ExportRow> for TicketExportRow {
    fn from(row: ExportRow) -> Self {
        Self {
            code: row.code,
            title: row.title,
            status: row.status.parse().unwrap_or(TicketStatus::Open),
            priority: row.priority.parse().unwrap_or(TicketPriority::Normal),
            category: row.category.parse().unwrap_or(TicketCategory::Other),
            source: row.source.parse().unwrap_or(TicketSource::Manual),
            area_name: row.area_name,
            room_name: row.room_name,
            device_slot: row.device_slot,
            serial: row.serial,
            reporter_name: row.reporter_name,
            assignee_name: row.assignee_name,
            created_at: row.created_at,
            resolved_at: row.resolved_at,
            closed_at: row.closed_at,
            resolution: row.resolution,
        }
    }
}

#[derive(Debug, FromRow)]
struct StreamEventRow {
    id: i64,
    ticket_id: Uuid,
    ticket_code: String,
    kind: String,
    status: String,
    actor_id: Option<Uuid>,
    actor_name: String,
    note: String,
    payload: Value,
    created_at: DateTime<Utc>,
}

impl From<StreamEventRow> for TicketStreamEvent {
    fn from(row: StreamEventRow) -> Self {
        Self {
            id: row.id,
            ticket_id: row.ticket_id,
            ticket_code: row.ticket_code,
            kind: row.kind.parse().unwrap_or(TicketEventKind::Updated),
            status: row.status.parse().unwrap_or(TicketStatus::Open),
            actor_id: row.actor_id,
            actor_name: row.actor_name,
            note: row.note,
            payload: row.payload,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct ResolveDurationRow {
    avg_secs: Option<i64>,
    median_secs: Option<i64>,
}

#[derive(Debug, FromRow)]
struct DayCountRow {
    date: String,
    count: i64,
}

#[derive(Debug, FromRow)]
struct CountRow {
    count: i64,
}

#[derive(Debug, FromRow)]
struct TextCountRow {
    key: String,
    count: i64,
}

#[derive(Debug, FromRow)]
struct TopRoomRow {
    room_id: Uuid,
    room_name: String,
    count: i64,
}

#[derive(Debug, FromRow)]
struct TopDeviceRow {
    serial: String,
    device_slot: Option<String>,
    count: i64,
}

#[derive(Debug, FromRow)]
struct AssigneeStatRow {
    assignee_id: Uuid,
    display_name: String,
    active: i64,
    resolved: i64,
    avg_resolve_secs: Option<i64>,
}

#[derive(Debug, FromRow)]
struct AssigneeRow {
    id: Uuid,
    display_name: String,
    roles: Vec<String>,
}

#[derive(Debug, FromRow)]
struct SummaryRow {
    open: i64,
    in_progress: i64,
    resolved: i64,
    closed: i64,
    cancelled: i64,
    unassigned: i64,
    overdue: i64,
    mine_active: i64,
}

#[derive(Debug, FromRow)]
struct RoomContextRow {
    id: Uuid,
    code: String,
    name: String,
    area_id: Uuid,
    area_name: String,
}

#[derive(Debug, FromRow)]
struct MachineHealthContextRow {
    health_status: String,
    health_score: Option<i32>,
    collected_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct SessionContextRow {
    state: String,
}

/// 工单仓储。持有连接池克隆（成本很低），供 `AppState` 与后台 worker 共享。
#[derive(Clone)]
pub struct TicketRepo {
    db: PgPool,
}

impl TicketRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    // ------------------------------------------------------------------
    // 写入
    // ------------------------------------------------------------------

    /// 建单。自动单在 `dedupe_key` 冲突时累加重复发生（`occurrence_count + 1`、
    /// 更新 `last_seen_at`、追加 `occurred` 事件），返回当前活跃工单。
    pub async fn insert_ticket(&self, input: &NewTicket) -> DbResult<Ticket> {
        let mut tx = self.db.begin().await?;
        let due_secs = input.priority.sla().num_seconds();
        let inserted_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO tickets (
                 code, title, description, priority, category, source, source_type, source_id,
                 dedupe_key, area_id, room_id, serial, device_slot, reporter_id, due_at, metadata
             )
             VALUES (
                 'T-' || to_char(
                     now() AT TIME ZONE COALESCE(
                         (SELECT timezone FROM areas WHERE id = $9),
                         (SELECT a.timezone FROM rooms r JOIN areas a ON a.id = r.area_id
                           WHERE r.id = $10),
                         'Asia/Shanghai'
                     ),
                     'YYYYMMDD'
                 ) || '-' || lpad(nextval('tickets_code_seq')::text, 4, '0'),
                 $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                 now() + make_interval(secs => $14::bigint), $15
             )
             ON CONFLICT DO NOTHING
             RETURNING id",
        )
        .bind(&input.title)
        .bind(&input.description)
        .bind(input.priority.as_str())
        .bind(input.category.as_str())
        .bind(input.source.as_str())
        .bind(input.source_type.as_deref())
        .bind(input.source_id.as_deref())
        .bind(input.dedupe_key.as_deref())
        .bind(input.area_id)
        .bind(input.room_id)
        .bind(input.serial.as_deref())
        .bind(input.device_slot.as_deref())
        .bind(input.reporter_id)
        .bind(due_secs)
        .bind(&input.metadata)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(id) = inserted_id {
            insert_event(
                &mut *tx,
                id,
                TicketEventKind::Created,
                input.reporter_id,
                None,
                Some(TicketStatus::Open),
                "",
                json!({
                    "source": input.source.as_str(),
                    "source_type": input.source_type,
                    "source_id": input.source_id,
                    "priority": input.priority.as_str(),
                    "category": input.category.as_str(),
                }),
                input.source_type.as_deref(),
                input.source_id.as_deref(),
            )
            .await?;
            let ticket = fetch_ticket(&mut *tx, id)
                .await?
                .ok_or_else(|| DbError::internal("建单后无法读取工单"))?;
            tx.commit().await?;
            return Ok(ticket);
        }

        // 去重冲突：已有未结束的同 dedupe_key 工单 → 累加重复发生。
        let bumped = match (&input.dedupe_key, &input.source_type, &input.source_id) {
            (Some(key), Some(source_type), Some(source_id)) => {
                bump_occurrence_tx(&mut *tx, key, source_type, source_id, &input.metadata).await?
            }
            _ => None,
        };
        if let Some(ticket) = bumped {
            tx.commit().await?;
            return Ok(ticket);
        }

        // 同一源行已记录过（幂等）：返回当前活跃工单；若已结束则让调用方下轮重试。
        let existing = match input.dedupe_key.as_deref() {
            Some(key) => fetch_ticket_by_dedupe(&mut *tx, key).await?,
            None => None,
        };
        match existing {
            Some(ticket) => {
                tx.commit().await?;
                Ok(ticket)
            }
            None => {
                tx.rollback().await?;
                Err(DbError::conflict("工单并发创建冲突，请重试"))
            }
        }
    }

    /// 累加一次重复发生：`occurrence_count + 1`、刷新 `last_seen_at`/`metadata`，
    /// 并追加 `occurred` 事件（同一源行只追加一次）。
    ///
    /// 返回 `None` 表示该源行已记录过（或已无未结束工单），本次不重复计数。
    pub async fn bump_occurrence(
        &self,
        dedupe_key: &str,
        source_type: &str,
        source_id: &str,
        payload: &Value,
    ) -> DbResult<Option<Ticket>> {
        let mut tx = self.db.begin().await?;
        let bumped =
            bump_occurrence_tx(&mut tx, dedupe_key, source_type, source_id, payload).await?;
        tx.commit().await?;
        Ok(bumped)
    }

    /// 更新内容（标题/描述/优先级/分类）。仅未结束工单可改。
    pub async fn update_content(
        &self,
        id: Uuid,
        actor_id: Uuid,
        title: Option<&str>,
        description: Option<&str>,
        priority: Option<TicketPriority>,
        category: Option<TicketCategory>,
    ) -> DbResult<Ticket> {
        let mut tx = self.db.begin().await?;
        let (status, _) = lock_ticket(&mut *tx, id).await?;
        let current = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT title, description, priority, category FROM tickets WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        let old_title = current.0;
        let old_description = current.1;
        let old_priority = current.2.parse().unwrap_or(TicketPriority::Normal);
        let old_category = current.3.parse().unwrap_or(TicketCategory::Other);
        if status.is_finished() {
            tx.rollback().await?;
            return Err(DbError::conflict("工单已结束，不能修改内容"));
        }

        sqlx::query(
            "UPDATE tickets
                SET title = COALESCE($2, title),
                    description = COALESCE($3, description),
                    priority = COALESCE($4, priority),
                    category = COALESCE($5, category)
              WHERE id = $1",
        )
        .bind(id)
        .bind(title)
        .bind(description)
        .bind(priority.map(TicketPriority::as_str))
        .bind(category.map(TicketCategory::as_str))
        .execute(&mut *tx)
        .await?;

        let new_title = title.unwrap_or(&old_title);
        let new_description = description.unwrap_or(&old_description);
        let new_priority = priority.unwrap_or(old_priority);
        let new_category = category.unwrap_or(old_category);

        // 标题/描述/分类变化 → `updated` 事件（契约 §5：所有写操作都要写时间线）。
        let mut changed_fields: Vec<&str> = Vec::new();
        if new_title != old_title {
            changed_fields.push("title");
        }
        if new_description != old_description {
            changed_fields.push("description");
        }
        if new_category != old_category {
            changed_fields.push("category");
        }
        if !changed_fields.is_empty() {
            insert_event(
                &mut *tx,
                id,
                TicketEventKind::Updated,
                Some(actor_id),
                None,
                None,
                "",
                json!({
                    "changed_fields": changed_fields,
                    "title_from": old_title,
                    "title_to": new_title,
                    "description_from": old_description,
                    "description_to": new_description,
                    "category_from": old_category.as_str(),
                    "category_to": new_category.as_str(),
                }),
                None,
                None,
            )
            .await?;
        }
        if new_priority != old_priority {
            insert_event(
                &mut *tx,
                id,
                TicketEventKind::PriorityChanged,
                Some(actor_id),
                None,
                None,
                "",
                json!({
                    "priority_from": old_priority.as_str(),
                    "priority_to": new_priority.as_str(),
                }),
                None,
                None,
            )
            .await?;
        }

        let ticket = fetch_ticket(&mut *tx, id)
            .await?
            .ok_or_else(|| DbError::internal("更新工单后无法读取"))?;
        tx.commit().await?;
        Ok(ticket)
    }

    /// 认领：只允许 `open -> in_progress`（已被他人认领 → 409）。
    pub async fn claim(&self, id: Uuid, user_id: Uuid, now: DateTime<Utc>) -> DbResult<Ticket> {
        let mut tx = self.db.begin().await?;
        let (status, assignee_id) = lock_ticket(&mut *tx, id).await?;
        match status {
            TicketStatus::Open => {
                sqlx::query(
                    "UPDATE tickets
                        SET status = 'in_progress',
                            assignee_id = $2,
                            acknowledged_at = COALESCE(acknowledged_at, $3)
                      WHERE id = $1",
                )
                .bind(id)
                .bind(user_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_event(
                    &mut *tx,
                    id,
                    TicketEventKind::Claimed,
                    Some(user_id),
                    Some(TicketStatus::Open),
                    Some(TicketStatus::InProgress),
                    "",
                    json!({ "assignee_id": user_id }),
                    None,
                    None,
                )
                .await?;
            }
            TicketStatus::InProgress => {
                if assignee_id != Some(user_id) {
                    tx.rollback().await?;
                    return Err(DbError::conflict(match assignee_id {
                        Some(_) => "工单已被他人认领",
                        None => "工单已在处理中且未分配给当前用户",
                    }));
                }
                // 幂等：已经是自己的处理中工单，直接返回。
            }
            other => {
                tx.rollback().await?;
                return Err(DbError::conflict(format!(
                    "工单状态为「{}」，不能认领",
                    other.label()
                )));
            }
        }
        let ticket = fetch_ticket(&mut *tx, id)
            .await?
            .ok_or_else(|| DbError::internal("认领后无法读取工单"))?;
        tx.commit().await?;
        Ok(ticket)
    }

    /// 转派（`assignee_id = None` 表示释放认领）。目标用户必须具备工单处理权限
    /// （`ticket:handle` 或细粒度 `ticket:claim/comment/transition` 任一）。
    pub async fn assign(
        &self,
        id: Uuid,
        assignee_id: Option<Uuid>,
        actor_id: Uuid,
        now: DateTime<Utc>,
    ) -> DbResult<Ticket> {
        let mut tx = self.db.begin().await?;
        let (status, previous_assignee) = lock_ticket(&mut *tx, id).await?;
        if status.is_finished() {
            tx.rollback().await?;
            return Err(DbError::conflict("工单已结束，不能转派"));
        }
        if let Some(target) = assignee_id {
            let allowed = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                     SELECT 1
                       FROM users u
                       JOIN user_roles ur ON ur.user_id = u.id
                       JOIN role_permissions rp ON rp.role_id = ur.role_id
                       JOIN permissions p ON p.id = rp.permission_id
                      WHERE u.id = $1 AND u.is_active
                        AND p.code IN ('ticket:handle', 'ticket:claim', 'ticket:comment', 'ticket:transition')
                 )",
            )
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
            if !allowed {
                tx.rollback().await?;
                return Err(DbError::bad_request("目标处理人没有工单处理权限"));
            }
        }

        sqlx::query("UPDATE tickets SET assignee_id = $2 WHERE id = $1")
            .bind(id)
            .bind(assignee_id)
            .execute(&mut *tx)
            .await?;
        insert_event(
            &mut *tx,
            id,
            TicketEventKind::Assigned,
            Some(actor_id),
            None,
            None,
            "",
            json!({
                "assignee_id": assignee_id,
                "previous_assignee_id": previous_assignee,
                "assigned_at": now,
            }),
            None,
            None,
        )
        .await?;

        let ticket = fetch_ticket(&mut *tx, id)
            .await?
            .ok_or_else(|| DbError::internal("转派后无法读取工单"))?;
        tx.commit().await?;
        Ok(ticket)
    }

    /// 状态流转（服务端状态机校验；非法 → `Conflict` → 409）。
    pub async fn transition(
        &self,
        id: Uuid,
        input: &TransitionInput,
        actor_id: Uuid,
        now: DateTime<Utc>,
    ) -> DbResult<Ticket> {
        let mut tx = self.db.begin().await?;
        let (from, assignee_id) = lock_ticket(&mut *tx, id).await?;
        qe_domain::ticket::validate_transition(from, input.to_status, input.resolution.as_deref())
            .map_err(map_domain_error)?;

        let resolution = input
            .resolution
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match input.to_status {
            TicketStatus::InProgress => {
                // 与 claim 一致：未分配 → 归 actor；已是 actor → 保持；是他人 → 409。
                match assignee_id {
                    None => {}
                    Some(current) if current == actor_id => {}
                    Some(_) => {
                        tx.rollback().await?;
                        return Err(DbError::conflict("工单已被他人认领"));
                    }
                }
                sqlx::query(
                    "UPDATE tickets
                        SET status = 'in_progress',
                            assignee_id = $2,
                            acknowledged_at = COALESCE(acknowledged_at, $3)
                      WHERE id = $1",
                )
                .bind(id)
                .bind(actor_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
            TicketStatus::Resolved => {
                sqlx::query(
                    "UPDATE tickets
                        SET status = 'resolved', resolved_at = $2, resolution = $3,
                            root_cause = COALESCE($4, root_cause)
                      WHERE id = $1",
                )
                .bind(id)
                .bind(now)
                .bind(resolution)
                .bind(input.root_cause.as_deref())
                .execute(&mut *tx)
                .await?;
            }
            TicketStatus::Closed => {
                // 允许 resolved→closed 时补充/修正根因。
                sqlx::query(
                    "UPDATE tickets
                        SET status = 'closed', closed_at = $2,
                            root_cause = COALESCE($3, root_cause)
                      WHERE id = $1",
                )
                .bind(id)
                .bind(now)
                .bind(input.root_cause.as_deref())
                .execute(&mut *tx)
                .await?;
            }
            TicketStatus::Open => {
                // 重开：清空 resolved_at / closed_at，保留旧时间线。
                sqlx::query(
                    "UPDATE tickets
                        SET status = 'open', resolved_at = NULL, closed_at = NULL
                      WHERE id = $1",
                )
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            TicketStatus::Cancelled => {
                sqlx::query("UPDATE tickets SET status = 'cancelled' WHERE id = $1")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        insert_event(
            &mut *tx,
            id,
            TicketEventKind::StatusChanged,
            Some(actor_id),
            Some(from),
            Some(input.to_status),
            input.note.as_deref().unwrap_or(""),
            json!({ "resolution": resolution, "root_cause": input.root_cause }),
            None,
            None,
        )
        .await?;

        let ticket = fetch_ticket(&mut *tx, id)
            .await?
            .ok_or_else(|| DbError::internal("状态流转后无法读取工单"))?;
        tx.commit().await?;
        Ok(ticket)
    }

    /// 追加评论事件（同时刷新工单 `updated_at`，让列表按最近活动排序）。
    pub async fn add_comment(&self, id: Uuid, actor_id: Uuid, body: &str) -> DbResult<TicketEvent> {
        let mut tx = self.db.begin().await?;
        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM tickets WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            tx.rollback().await?;
            return Err(DbError::not_found("工单不存在"));
        }
        sqlx::query("UPDATE tickets SET updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let event_id = insert_event(
            &mut *tx,
            id,
            TicketEventKind::Commented,
            Some(actor_id),
            None,
            None,
            body,
            json!({}),
            None,
            None,
        )
        .await?;
        let event = fetch_event(&mut *tx, event_id)
            .await?
            .ok_or_else(|| DbError::internal("写评论后无法读取事件"))?;
        tx.commit().await?;
        Ok(event)
    }

    // ------------------------------------------------------------------
    // 读取
    // ------------------------------------------------------------------

    /// 列表分页（`updated_at DESC`），含上报人/处理人显示名。
    pub async fn list(&self, filter: &TicketFilter) -> DbResult<Page<Ticket>> {
        let status = filter.status.map(|value| value.as_str().to_owned());
        let category = filter.category.map(|value| value.as_str().to_owned());
        let priority = filter.priority.map(|value| value.as_str().to_owned());
        let source = filter.source.map(|value| value.as_str().to_owned());
        // 转义 LIKE 通配符（`%` / `_` / `\`），避免 `keyword=%` 匹配全部工单。
        let keyword = filter.keyword.as_deref().map(escape_like);
        let offset = (filter.page - 1) * filter.page_size;

        let sql = format!(
            "SELECT {TICKET_COLUMNS} {TICKET_FROM}
             LEFT JOIN rooms room ON room.id = t.room_id
             {LIST_WHERE}
             ORDER BY t.updated_at DESC, t.id
             LIMIT $17 OFFSET $18"
        );
        let rows = sqlx::query_as::<_, TicketRow>(&sql)
            .bind(status.as_deref())
            .bind(category.as_deref())
            .bind(priority.as_deref())
            .bind(source.as_deref())
            .bind(filter.assignee_id)
            .bind(filter.unassigned)
            .bind(filter.mine)
            .bind(filter.active)
            .bind(filter.overdue)
            .bind(filter.area_id)
            .bind(filter.room_id)
            .bind(filter.serial.as_deref())
            .bind(keyword.as_deref())
            .bind(filter.from)
            .bind(filter.to)
            .bind(filter.scoped_area_ids.as_deref())
            .bind(filter.page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        let count_sql = format!(
            "SELECT count(*) FROM tickets t
             LEFT JOIN rooms room ON room.id = t.room_id
             {LIST_WHERE}"
        );
        let total = sqlx::query_scalar::<_, i64>(&count_sql)
            .bind(status.as_deref())
            .bind(category.as_deref())
            .bind(priority.as_deref())
            .bind(source.as_deref())
            .bind(filter.assignee_id)
            .bind(filter.unassigned)
            .bind(filter.mine)
            .bind(filter.active)
            .bind(filter.overdue)
            .bind(filter.area_id)
            .bind(filter.room_id)
            .bind(filter.serial.as_deref())
            .bind(keyword.as_deref())
            .bind(filter.from)
            .bind(filter.to)
            .bind(filter.scoped_area_ids.as_deref())
            .fetch_one(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(Ticket::from).collect(),
            total,
            filter.page,
            filter.page_size,
        ))
    }

    /// 工单 + 时间线（事件按 id 升序）。
    pub async fn get(&self, id: Uuid) -> DbResult<Option<(Ticket, Vec<TicketEvent>)>> {
        let Some(ticket) = fetch_ticket(&self.db, id).await? else {
            return Ok(None);
        };
        let rows = sqlx::query_as::<_, TicketEventRow>(&format!(
            "SELECT {EVENT_COLUMNS} {EVENT_FROM} WHERE e.ticket_id = $1 ORDER BY e.id ASC"
        ))
        .bind(id)
        .fetch_all(&self.db)
        .await?;
        Ok(Some((
            ticket,
            rows.into_iter().map(TicketEvent::from).collect(),
        )))
    }

    /// 单张工单（不含时间线）。
    pub async fn get_ticket(&self, id: Uuid) -> DbResult<Option<Ticket>> {
        fetch_ticket(&self.db, id).await
    }

    /// 工单的有效区域（`tickets.area_id` 优先，否则取房间所属区域）。
    pub async fn ticket_area(&self, id: Uuid) -> DbResult<Option<Uuid>> {
        let area = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT COALESCE(t.area_id, room.area_id)
               FROM tickets t
               LEFT JOIN rooms room ON room.id = t.room_id
              WHERE t.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?
        .flatten();
        Ok(area)
    }

    /// 轻量计数（按数据范围过滤）。
    pub async fn summary(
        &self,
        user_id: Uuid,
        scoped_area_ids: Option<&[Uuid]>,
    ) -> DbResult<TicketSummary> {
        let row = sqlx::query_as::<_, SummaryRow>(
            "SELECT
                 count(*) FILTER (WHERE t.status = 'open')::bigint AS open,
                 count(*) FILTER (WHERE t.status = 'in_progress')::bigint AS in_progress,
                 count(*) FILTER (WHERE t.status = 'resolved')::bigint AS resolved,
                 count(*) FILTER (WHERE t.status = 'closed')::bigint AS closed,
                 count(*) FILTER (WHERE t.status = 'cancelled')::bigint AS cancelled,
                 count(*) FILTER (WHERE t.assignee_id IS NULL
                                     AND t.status IN ('open','in_progress'))::bigint AS unassigned,
                 count(*) FILTER (WHERE t.due_at IS NOT NULL AND t.due_at < now()
                                     AND t.status IN ('open','in_progress'))::bigint AS overdue,
                 count(*) FILTER (WHERE t.assignee_id = $1
                                     AND t.status IN ('open','in_progress'))::bigint AS mine_active
             FROM tickets t
             LEFT JOIN rooms room ON room.id = t.room_id
             WHERE ($2::uuid[] IS NULL OR COALESCE(t.area_id, room.area_id) = ANY($2))",
        )
        .bind(user_id)
        .bind(scoped_area_ids)
        .fetch_one(&self.db)
        .await?;

        let mut by_status = std::collections::BTreeMap::new();
        for (status, count) in [
            (TicketStatus::Open, row.open),
            (TicketStatus::InProgress, row.in_progress),
            (TicketStatus::Resolved, row.resolved),
            (TicketStatus::Closed, row.closed),
            (TicketStatus::Cancelled, row.cancelled),
        ] {
            by_status.insert(status.as_str().to_owned(), count);
        }
        Ok(TicketSummary {
            by_status,
            unassigned: row.unassigned,
            overdue: row.overdue,
            mine_active: row.mine_active,
        })
    }

    /// 具备工单处理能力的启用用户（`ticket:handle` 或 claim/comment/transition 任一）。
    pub async fn assignees(&self) -> DbResult<Vec<TicketAssignee>> {
        let rows = sqlx::query_as::<_, AssigneeRow>(
            "SELECT u.id,
                    u.display_name,
                    COALESCE(
                        array_agg(DISTINCT r.code) FILTER (WHERE r.code IS NOT NULL),
                        '{}'
                    ) AS roles
               FROM users u
               JOIN user_roles ur ON ur.user_id = u.id
               JOIN roles r ON r.id = ur.role_id
               JOIN role_permissions rp ON rp.role_id = r.id
               JOIN permissions p ON p.id = rp.permission_id
              WHERE u.is_active
                AND p.code IN ('ticket:handle', 'ticket:claim', 'ticket:comment', 'ticket:transition')
              GROUP BY u.id, u.display_name
              ORDER BY u.display_name",
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| TicketAssignee {
                id: row.id,
                display_name: row.display_name,
                roles: row.roles,
            })
            .collect())
    }

    /// 关联上下文：房间/设备/机器最新健康与在线摘要。
    pub async fn context(&self, ticket: &Ticket) -> DbResult<TicketContext> {
        let mut context = TicketContext {
            room_id: ticket.room_id,
            serial: ticket.serial.clone(),
            device_slot: ticket.device_slot.clone(),
            ..TicketContext::default()
        };

        if let Some(room_id) = ticket.room_id {
            let room = sqlx::query_as::<_, RoomContextRow>(
                "SELECT r.id, r.code, r.name, a.id AS area_id, a.name AS area_name
                   FROM rooms r JOIN areas a ON a.id = r.area_id
                  WHERE r.id = $1",
            )
            .bind(room_id)
            .fetch_optional(&self.db)
            .await?;
            if let Some(room) = room {
                context.room_id = Some(room.id);
                context.room_code = Some(room.code);
                context.room_name = Some(room.name);
                context.area_id = Some(room.area_id);
                context.area_name = Some(room.area_name);
            }
        }
        if context.area_id.is_none() {
            context.area_id = ticket.area_id;
        }

        let Some(serial) = ticket.serial.as_deref() else {
            return Ok(context);
        };
        if context.device_slot.is_none() {
            context.device_slot = sqlx::query_scalar::<_, Option<String>>(
                "SELECT device_slot FROM device_enrollment WHERE serial = $1",
            )
            .bind(serial)
            .fetch_optional(&self.db)
            .await?
            .flatten();
        }

        let snapshot = sqlx::query_as::<_, MachineHealthContextRow>(
            "SELECT health_status, health_score, collected_at
               FROM machine_snapshots WHERE serial = $1
              ORDER BY collected_at DESC LIMIT 1",
        )
        .bind(serial)
        .fetch_optional(&self.db)
        .await?;
        let session = sqlx::query_as::<_, SessionContextRow>(
            "SELECT state FROM device_session WHERE serial = $1",
        )
        .bind(serial)
        .fetch_optional(&self.db)
        .await?;

        let now = Utc::now();
        let snapshot_fresh = snapshot
            .as_ref()
            .is_some_and(|row| row.collected_at >= now - chrono::Duration::minutes(5));
        let has_device = snapshot.is_some() || session.is_some();
        context.online = has_device
            .then(|| session.as_ref().is_some_and(|row| row.state == "online") || snapshot_fresh);
        if let Some(snapshot) = snapshot {
            context.health_status = Some(snapshot.health_status);
            context.health_score = snapshot.health_score;
            context.collected_at = Some(snapshot.collected_at);
        }
        Ok(context)
    }

    /// 历史分析聚合（`GET /admin/tickets/stats`，按数据范围过滤）。
    pub async fn stats(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        scoped_area_ids: Option<&[Uuid]>,
    ) -> DbResult<TicketStats> {
        // 统一别名：t = tickets，r = rooms；$3 = 数据范围区域数组。
        const STATS_FROM: &str = "FROM tickets t LEFT JOIN rooms r ON r.id = t.room_id";
        const SCOPE: &str = " AND ($3::uuid[] IS NULL OR COALESCE(t.area_id, r.area_id) = ANY($3))";
        // 无时间参数的查询把范围参数放在 $1。
        const SCOPE1: &str =
            " AND ($1::uuid[] IS NULL OR COALESCE(t.area_id, r.area_id) = ANY($1))";

        let created = sqlx::query_as::<_, CountRow>(&format!(
            "SELECT count(*) AS count {STATS_FROM}
              WHERE t.created_at >= $1 AND t.created_at < $2 {SCOPE}"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_one(&self.db)
        .await?
        .count;
        let resolved = sqlx::query_as::<_, CountRow>(&format!(
            "SELECT count(*) AS count {STATS_FROM}
              WHERE t.resolved_at IS NOT NULL
                AND t.resolved_at >= $1 AND t.resolved_at < $2 {SCOPE}"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_one(&self.db)
        .await?
        .count;
        let closed = sqlx::query_as::<_, CountRow>(&format!(
            "SELECT count(*) AS count {STATS_FROM}
              WHERE t.closed_at IS NOT NULL
                AND t.closed_at >= $1 AND t.closed_at < $2 {SCOPE}"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_one(&self.db)
        .await?
        .count;
        let open_now = sqlx::query_as::<_, CountRow>(&format!(
            "SELECT count(*) AS count {STATS_FROM}
              WHERE t.status IN ('open','in_progress') {SCOPE1}"
        ))
        .bind(scoped_area_ids)
        .fetch_one(&self.db)
        .await?
        .count;
        let overdue = sqlx::query_as::<_, CountRow>(&format!(
            "SELECT count(*) AS count {STATS_FROM}
              WHERE t.due_at IS NOT NULL AND t.due_at < now()
                AND t.status IN ('open','in_progress') {SCOPE1}"
        ))
        .bind(scoped_area_ids)
        .fetch_one(&self.db)
        .await?
        .count;
        let resolve = sqlx::query_as::<_, ResolveDurationRow>(&format!(
            "SELECT avg(extract(epoch FROM (t.resolved_at - t.created_at)))::bigint AS avg_secs,
                    (percentile_cont(0.5) WITHIN GROUP (
                        ORDER BY extract(epoch FROM (t.resolved_at - t.created_at))
                    ))::bigint AS median_secs
               {STATS_FROM}
              WHERE t.resolved_at IS NOT NULL AND t.resolved_at >= $1 AND t.resolved_at < $2
                AND t.status <> 'cancelled' {SCOPE}"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_one(&self.db)
        .await?;

        let by_day = self.daily_counts(from, to, scoped_area_ids).await?;
        let by_category = sqlx::query_as::<_, TextCountRow>(&format!(
            "SELECT t.category AS key, count(*) AS count {STATS_FROM}
              WHERE t.created_at >= $1 AND t.created_at < $2 {SCOPE}
              GROUP BY t.category ORDER BY count(*) DESC, t.category"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(|row| TicketCategoryStat {
            category: row.key.parse().unwrap_or(TicketCategory::Other),
            count: row.count,
        })
        .collect();
        let by_priority = sqlx::query_as::<_, TextCountRow>(&format!(
            "SELECT t.priority AS key, count(*) AS count {STATS_FROM}
              WHERE t.created_at >= $1 AND t.created_at < $2 {SCOPE}
              GROUP BY t.priority ORDER BY count(*) DESC, t.priority"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(|row| TicketPriorityStat {
            priority: row.key.parse().unwrap_or(TicketPriority::Normal),
            count: row.count,
        })
        .collect();
        let top_rooms = sqlx::query_as::<_, TopRoomRow>(&format!(
            "SELECT r.id AS room_id, r.name AS room_name, count(*) AS count
               FROM tickets t JOIN rooms r ON r.id = t.room_id
              WHERE t.created_at >= $1 AND t.created_at < $2 {SCOPE}
              GROUP BY r.id, r.name
              ORDER BY count(*) DESC, r.name
              LIMIT 5"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(|row| TicketTopRoom {
            room_id: row.room_id,
            room_name: row.room_name,
            count: row.count,
        })
        .collect();
        let top_devices = sqlx::query_as::<_, TopDeviceRow>(&format!(
            "SELECT t.serial, max(t.device_slot) AS device_slot, count(*) AS count
               {STATS_FROM}
              WHERE t.serial IS NOT NULL AND t.created_at >= $1 AND t.created_at < $2 {SCOPE}
              GROUP BY t.serial
              ORDER BY count(*) DESC, t.serial
              LIMIT 5"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(|row| TicketTopDevice {
            serial: row.serial,
            device_slot: row.device_slot,
            count: row.count,
        })
        .collect();
        let by_assignee = sqlx::query_as::<_, AssigneeStatRow>(&format!(
            "SELECT t.assignee_id,
                    u.display_name,
                    count(*) FILTER (WHERE t.status IN ('open','in_progress'))::bigint AS active,
                    count(*) FILTER (WHERE t.status IN ('resolved','closed'))::bigint AS resolved,
                    (avg(extract(epoch FROM (t.resolved_at - t.created_at)))
                        FILTER (WHERE t.resolved_at IS NOT NULL))::bigint AS avg_resolve_secs
               FROM tickets t
               LEFT JOIN rooms r ON r.id = t.room_id
               JOIN users u ON u.id = t.assignee_id
              WHERE t.assignee_id IS NOT NULL
                AND t.created_at >= $1 AND t.created_at < $2 {SCOPE}
              GROUP BY t.assignee_id, u.display_name
              ORDER BY resolved DESC, active DESC, u.display_name
              LIMIT 20"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .map(|row| TicketAssigneeStat {
            assignee_id: row.assignee_id,
            display_name: row.display_name,
            active: row.active,
            resolved: row.resolved,
            avg_resolve_secs: row.avg_resolve_secs,
        })
        .collect();

        Ok(TicketStats {
            from,
            to,
            totals: TicketStatsTotals {
                created,
                resolved,
                closed,
                open_now,
                overdue,
                avg_resolve_secs: resolve.avg_secs,
                median_resolve_secs: resolve.median_secs,
            },
            by_day,
            by_category,
            by_priority,
            top_rooms,
            top_devices,
            by_assignee,
        })
    }

    async fn daily_counts(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        scoped_area_ids: Option<&[Uuid]>,
    ) -> DbResult<Vec<TicketDayStat>> {
        // 按关联区域时区聚合本地日期，避免跨 0 点差一天。
        const DAILY_FROM: &str = "FROM tickets t
               LEFT JOIN rooms r ON r.id = t.room_id
               LEFT JOIN areas a ON a.id = COALESCE(t.area_id, r.area_id)";
        const SCOPE: &str = " AND ($3::uuid[] IS NULL OR COALESCE(t.area_id, r.area_id) = ANY($3))";

        let created = sqlx::query_as::<_, DayCountRow>(&format!(
            "SELECT to_char(t.created_at AT TIME ZONE
                        COALESCE(a.timezone, 'Asia/Shanghai'), 'YYYY-MM-DD') AS date,
                    count(*) AS count
               {DAILY_FROM}
              WHERE t.created_at >= $1 AND t.created_at < $2 {SCOPE}
              GROUP BY 1"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?;
        let resolved = sqlx::query_as::<_, DayCountRow>(&format!(
            "SELECT to_char(t.resolved_at AT TIME ZONE
                        COALESCE(a.timezone, 'Asia/Shanghai'), 'YYYY-MM-DD') AS date,
                    count(*) AS count
               {DAILY_FROM}
              WHERE t.resolved_at IS NOT NULL
                AND t.resolved_at >= $1 AND t.resolved_at < $2 {SCOPE}
              GROUP BY 1"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?;
        let closed = sqlx::query_as::<_, DayCountRow>(&format!(
            "SELECT to_char(t.closed_at AT TIME ZONE
                        COALESCE(a.timezone, 'Asia/Shanghai'), 'YYYY-MM-DD') AS date,
                    count(*) AS count
               {DAILY_FROM}
              WHERE t.closed_at IS NOT NULL
                AND t.closed_at >= $1 AND t.closed_at < $2 {SCOPE}
              GROUP BY 1"
        ))
        .bind(from)
        .bind(to)
        .bind(scoped_area_ids)
        .fetch_all(&self.db)
        .await?;

        let mut days: std::collections::BTreeMap<String, TicketDayStat> =
            std::collections::BTreeMap::new();
        for row in created {
            days.entry(row.date.clone())
                .or_insert(TicketDayStat {
                    date: row.date,
                    created: 0,
                    resolved: 0,
                    closed: 0,
                })
                .created = row.count;
        }
        for row in resolved {
            days.entry(row.date.clone())
                .or_insert(TicketDayStat {
                    date: row.date,
                    created: 0,
                    resolved: 0,
                    closed: 0,
                })
                .resolved = row.count;
        }
        for row in closed {
            days.entry(row.date.clone())
                .or_insert(TicketDayStat {
                    date: row.date,
                    created: 0,
                    resolved: 0,
                    closed: 0,
                })
                .closed = row.count;
        }
        Ok(days.into_values().collect())
    }

    // ------------------------------------------------------------------
    // 自动建单 worker 的信号查询
    // ------------------------------------------------------------------

    /// 机器健康规则：每 serial 最新连续 `consecutive` 条快照均为非 ok。
    pub async fn machine_health_candidates(
        &self,
        consecutive: i64,
    ) -> DbResult<Vec<MachineHealthSignal>> {
        let rows = sqlx::query_as::<_, MachineHealthSignal>(
            "WITH recent AS (
                 SELECT id, serial, collected_at, health_status,
                        row_number() OVER (
                            PARTITION BY serial ORDER BY collected_at DESC
                        ) AS rn
                   FROM machine_snapshots
             ),
             unhealthy AS (
                 SELECT serial FROM recent WHERE rn <= $1
                  GROUP BY serial
                 HAVING count(*) >= $1 AND bool_and(health_status <> 'ok')
             ),
             latest AS (
                 SELECT DISTINCT ON (ms.serial)
                        ms.id, ms.serial, ms.device_slot, ms.health_status,
                        ms.health_score, ms.collected_at, ms.payload
                   FROM machine_snapshots ms
                  WHERE ms.serial IN (SELECT serial FROM unhealthy)
                  ORDER BY ms.serial, ms.collected_at DESC
             )
             SELECT l.serial,
                    l.device_slot,
                    l.id AS snapshot_id,
                    l.health_status,
                    l.health_score,
                    l.collected_at,
                    l.payload,
                    mapping.room_id,
                    mapping.area_id
               FROM latest l
               LEFT JOIN (
                   SELECT de.serial,
                          COALESCE(rd.room_id, r1.id) AS room_id,
                          a.id AS area_id
                     FROM device_enrollment de
                     LEFT JOIN room_devices rd ON rd.device_slot = de.device_slot
                     LEFT JOIN rooms r1 ON r1.device_slot = de.device_slot
                     LEFT JOIN rooms rr ON rr.id = COALESCE(rd.room_id, r1.id)
                     LEFT JOIN areas a ON a.id = rr.area_id
               ) mapping ON mapping.serial = l.serial
              ORDER BY l.serial",
        )
        .bind(consecutive)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 设备离线规则：`device_session` 离线且超过 `offline_minutes`，房间/区域启用中。
    pub async fn device_offline_candidates(
        &self,
        offline_minutes: i64,
    ) -> DbResult<Vec<DeviceOfflineSignal>> {
        let rows = sqlx::query_as::<_, DeviceOfflineSignal>(
            "WITH mapping AS (
                 SELECT de.serial, COALESCE(rd.room_id, r1.id) AS room_id
                   FROM device_enrollment de
                   LEFT JOIN room_devices rd ON rd.device_slot = de.device_slot
                   LEFT JOIN rooms r1 ON r1.device_slot = de.device_slot
             )
             SELECT ds.serial,
                    ds.device_slot,
                    ds.updated_at AS session_updated_at,
                    r.id AS room_id,
                    r.code AS room_code,
                    r.name AS room_name,
                    a.id AS area_id,
                    a.name AS area_name
               FROM device_session ds
               JOIN mapping m ON m.serial = ds.serial
               JOIN rooms r ON r.id = m.room_id AND r.is_active
               JOIN areas a ON a.id = r.area_id AND a.is_active
              WHERE ds.state = 'offline'
                AND ds.updated_at <= now() - make_interval(mins => $1::int)
              ORDER BY ds.serial",
        )
        .bind(offline_minutes)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 课表投递失败规则：lookback 内进入 failed 终态的投递（逐行返回，由 worker 聚合）。
    pub async fn failed_delivery_candidates(
        &self,
        lookback_hours: i64,
    ) -> DbResult<Vec<FailedDeliverySignal>> {
        let rows = sqlx::query_as::<_, FailedDeliverySignal>(
            "SELECT d.id AS delivery_id,
                    e.id AS event_id,
                    e.room_id,
                    r.code AS room_code,
                    r.name AS room_name,
                    a.id AS area_id,
                    d.target,
                    d.attempts,
                    d.last_error,
                    e.triggered_at,
                    e.payload,
                    to_char(e.triggered_at AT TIME ZONE a.timezone, 'YYYY-MM-DD') AS local_date
               FROM schedule_event_deliveries d
               JOIN schedule_events e ON e.id = d.event_id
               JOIN rooms r ON r.id = e.room_id
               JOIN areas a ON a.id = r.area_id
              WHERE d.status = 'failed'
                AND e.triggered_at >= now() - make_interval(hours => $1::int)
              ORDER BY d.id",
        )
        .bind(lookback_hours)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // P1：新信号源 / 超时提醒 / 导出 / SSE
    // ------------------------------------------------------------------

    /// 在线事件投递失败规则：lookback 内进入 failed 的 webhook 投递。
    pub async fn presence_delivery_candidates(
        &self,
        lookback_hours: i64,
    ) -> DbResult<Vec<PresenceDeliverySignal>> {
        let rows = sqlx::query_as::<_, PresenceDeliverySignal>(
            "SELECT d.id AS delivery_id,
                    e.id AS event_id,
                    e.serial,
                    d.target_url AS target,
                    d.last_error,
                    e.occurred_at,
                    e.room_id,
                    e.area_id,
                    to_char(e.occurred_at AT TIME ZONE COALESCE(a.timezone, 'Asia/Shanghai'),
                            'YYYY-MM-DD') AS local_date
               FROM device_presence_deliveries d
               JOIN device_presence_events e ON e.id = d.event_id
               LEFT JOIN areas a ON a.id = e.area_id
              WHERE d.status = 'failed'
                AND d.updated_at >= now() - make_interval(hours => $1::int)
              ORDER BY d.id",
        )
        .bind(lookback_hours)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 存储分发失败/过期规则：lookback 内进入 failed/expired 的分发任务。
    pub async fn transfer_candidates(&self, lookback_hours: i64) -> DbResult<Vec<TransferSignal>> {
        let rows = sqlx::query_as::<_, TransferSignal>(
            "SELECT t.id AS transfer_id,
                    t.room_id,
                    r.code AS room_code,
                    r.name AS room_name,
                    r.area_id,
                    a.name AS area_name,
                    t.status,
                    t.error,
                    t.updated_at,
                    to_char(t.updated_at AT TIME ZONE COALESCE(a.timezone, 'Asia/Shanghai'),
                            'YYYY-MM-DD') AS local_date
               FROM transfers t
               JOIN rooms r ON r.id = t.room_id
               LEFT JOIN areas a ON a.id = r.area_id
              WHERE t.status IN ('failed', 'expired')
                AND t.updated_at >= now() - make_interval(hours => $1::int)
              ORDER BY t.id",
        )
        .bind(lookback_hours)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 镜像验证失败规则：每个 artifact 最新一次验证为失败（且之后没有成功的验证）。
    pub async fn image_verify_candidates(
        &self,
        lookback_hours: i64,
    ) -> DbResult<Vec<ImageVerifySignal>> {
        let rows = sqlx::query_as::<_, ImageVerifySignal>(
            "SELECT v.id AS verification_id,
                    v.artifact_id,
                    art.name AS image_name,
                    art.version AS image_version,
                    v.report,
                    v.verified_at
               FROM image_verifications v
               JOIN image_artifacts art ON art.id = v.artifact_id
              WHERE v.ok = FALSE
                AND v.verified_at >= now() - make_interval(hours => $1::int)
                AND NOT EXISTS (
                    SELECT 1 FROM image_verifications newer
                     WHERE newer.artifact_id = v.artifact_id
                       AND newer.ok = TRUE
                       AND newer.verified_at > v.verified_at
                )
              ORDER BY v.verified_at DESC",
        )
        .bind(lookback_hours)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 超时提醒候选：已过 SLA、未结束、且超过提醒间隔（或从未提醒）。
    pub async fn overdue_tickets(&self, remind_hours: i64) -> DbResult<Vec<OverdueTicket>> {
        let rows = sqlx::query_as::<_, OverdueTicket>(
            "SELECT t.id,
                    t.code,
                    t.title,
                    t.due_at,
                    COALESCE(t.assignee_id, t.reporter_id) AS notify_user_id,
                    u.email AS notify_email,
                    u.display_name AS notify_name
               FROM tickets t
               LEFT JOIN users u ON u.id = COALESCE(t.assignee_id, t.reporter_id)
              WHERE t.due_at IS NOT NULL
                AND t.due_at < now()
                AND t.status IN ('open', 'in_progress')
                AND (t.last_reminded_at IS NULL
                     OR t.last_reminded_at < now() - make_interval(hours => $1::int))
              ORDER BY t.due_at ASC
              LIMIT 100",
        )
        .bind(remind_hours)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 原子抢占一次超时提醒（多副本并发安全）：只有把 `last_reminded_at`
    /// 从“空/超过窗口”更新成功的副本才返回 `true`，由它发送邮件并写 `reminded` 事件。
    pub async fn claim_reminder(&self, id: Uuid, remind_hours: i64) -> DbResult<bool> {
        let mut tx = self.db.begin().await?;
        let row = sqlx::query_as::<_, (Option<Uuid>, Option<DateTime<Utc>>)>(
            "UPDATE tickets SET last_reminded_at = now()
              WHERE id = $1
                AND status IN ('open', 'in_progress')
                AND (last_reminded_at IS NULL
                     OR last_reminded_at < now() - make_interval(hours => $2::int))
              RETURNING assignee_id, due_at",
        )
        .bind(id)
        .bind(remind_hours)
        .fetch_optional(&mut *tx)
        .await?;
        let claimed = row.is_some();
        if let Some((assignee_id, due_at)) = row {
            insert_event(
                &mut *tx,
                id,
                TicketEventKind::Reminded,
                None,
                None,
                None,
                "",
                json!({ "due_at": due_at, "assignee_id": assignee_id }),
                None,
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(claimed)
    }

    /// CSV 导出：复用列表筛选，最多 `limit` 行。
    pub async fn export_rows(
        &self,
        filter: &TicketFilter,
        limit: i64,
    ) -> DbResult<Vec<TicketExportRow>> {
        let status = filter.status.map(|value| value.as_str().to_owned());
        let category = filter.category.map(|value| value.as_str().to_owned());
        let priority = filter.priority.map(|value| value.as_str().to_owned());
        let source = filter.source.map(|value| value.as_str().to_owned());
        let keyword = filter.keyword.as_deref().map(escape_like);

        let sql = format!(
            "SELECT t.code, t.title, t.status, t.priority, t.category, t.source,
                    area.name AS area_name,
                    room.name AS room_name,
                    t.device_slot, t.serial,
                    reporter.display_name AS reporter_name,
                    assignee.display_name AS assignee_name,
                    t.created_at, t.resolved_at, t.closed_at, t.resolution
             {TICKET_FROM}
             LEFT JOIN rooms room ON room.id = t.room_id
             LEFT JOIN areas area ON area.id = COALESCE(t.area_id, room.area_id)
             {LIST_WHERE}
             ORDER BY t.created_at DESC
             LIMIT $17"
        );
        let rows = sqlx::query_as::<_, ExportRow>(&sql)
            .bind(status.as_deref())
            .bind(category.as_deref())
            .bind(priority.as_deref())
            .bind(source.as_deref())
            .bind(filter.assignee_id)
            .bind(filter.unassigned)
            .bind(filter.mine)
            .bind(filter.active)
            .bind(filter.overdue)
            .bind(filter.area_id)
            .bind(filter.room_id)
            .bind(filter.serial.as_deref())
            .bind(keyword.as_deref())
            .bind(filter.from)
            .bind(filter.to)
            .bind(filter.scoped_area_ids.as_deref())
            .bind(limit)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(TicketExportRow::from).collect())
    }

    /// SSE 增量：`ticket_events.id > after_id`，带工单编号与当前状态。
    pub async fn stream_events_after(
        &self,
        after_id: i64,
        limit: i64,
    ) -> DbResult<Vec<TicketStreamEvent>> {
        let rows = sqlx::query_as::<_, StreamEventRow>(
            "SELECT e.id, e.ticket_id, t.code AS ticket_code, e.kind, t.status,
                    e.actor_id, COALESCE(u.display_name, '系统') AS actor_name,
                    e.note, e.payload, e.created_at
               FROM ticket_events e
               JOIN tickets t ON t.id = e.ticket_id
               LEFT JOIN users u ON u.id = e.actor_id
              WHERE e.id > $1
              ORDER BY e.id ASC
              LIMIT $2",
        )
        .bind(after_id)
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(TicketStreamEvent::from).collect())
    }
}

// ----------------------------------------------------------------------
// 内部辅助
// ----------------------------------------------------------------------

fn map_domain_error(error: qe_domain::error::DomainError) -> DbError {
    use qe_domain::error::DomainError;
    match error {
        DomainError::BadRequest(message) => DbError::bad_request(message),
        DomainError::NotFound(message) => DbError::not_found(message),
        DomainError::Conflict(message) => DbError::conflict(message),
        DomainError::Internal(message) => DbError::internal(message),
    }
}

/// 锁行并返回 `(status, assignee_id)`；不存在 → `NotFound`。
async fn lock_ticket(
    executor: &mut sqlx::PgConnection,
    id: Uuid,
) -> DbResult<(TicketStatus, Option<Uuid>)> {
    let row = sqlx::query_as::<_, (String, Option<Uuid>)>(
        "SELECT status, assignee_id FROM tickets WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(executor)
    .await?;
    let (status, assignee_id) = row.ok_or_else(|| DbError::not_found("工单不存在"))?;
    Ok((status.parse().unwrap_or(TicketStatus::Open), assignee_id))
}

#[allow(clippy::too_many_arguments)]
async fn insert_event<'e, E>(
    executor: E,
    ticket_id: Uuid,
    kind: TicketEventKind,
    actor_id: Option<Uuid>,
    from_status: Option<TicketStatus>,
    to_status: Option<TicketStatus>,
    note: &str,
    payload: Value,
    source_type: Option<&str>,
    source_id: Option<&str>,
) -> DbResult<i64>
where
    E: PgExecutor<'e>,
{
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO ticket_events
             (ticket_id, kind, actor_id, from_status, to_status, note, payload,
              source_type, source_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id",
    )
    .bind(ticket_id)
    .bind(kind.as_str())
    .bind(actor_id)
    .bind(from_status.map(TicketStatus::as_str))
    .bind(to_status.map(TicketStatus::as_str))
    .bind(note)
    .bind(payload)
    .bind(source_type)
    .bind(source_id)
    .fetch_one(executor)
    .await?;
    Ok(id)
}

/// 事务内累加重复发生；`occurred` 事件唯一索引冲突时视为已记录过，返回 `None`。
async fn bump_occurrence_tx(
    tx: &mut sqlx::PgConnection,
    dedupe_key: &str,
    source_type: &str,
    source_id: &str,
    payload: &Value,
) -> DbResult<Option<Ticket>> {
    let ticket_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO ticket_events (ticket_id, kind, note, payload, source_type, source_id)
         SELECT t.id, 'occurred', '', $4, $2, $3
           FROM tickets t
          WHERE t.dedupe_key = $1 AND t.status IN ('open','in_progress')
            -- created 事件已经记录了首条源行，occurred 部分唯一索引不覆盖它，
            -- 这里显式排除，避免同一 source_id 被 created + occurred 双计；
            -- 多副本并发时仍由 ON CONFLICT DO NOTHING 兜底。
            AND NOT EXISTS (
                SELECT 1 FROM ticket_events e
                 WHERE e.ticket_id = t.id
                   AND e.source_type = $2
                   AND e.source_id = $3
            )
          ORDER BY t.created_at DESC
          LIMIT 1
         ON CONFLICT DO NOTHING
         RETURNING ticket_id",
    )
    .bind(dedupe_key)
    .bind(source_type)
    .bind(source_id)
    .bind(payload)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(ticket_id) = ticket_id else {
        return Ok(None);
    };

    sqlx::query(
        "UPDATE tickets
            SET occurrence_count = occurrence_count + 1,
                last_seen_at = now(),
                metadata = CASE WHEN $2::jsonb IS NULL THEN metadata ELSE $2::jsonb END
          WHERE id = $1",
    )
    .bind(ticket_id)
    .bind(payload)
    .execute(&mut *tx)
    .await?;

    fetch_ticket(&mut *tx, ticket_id).await
}

async fn fetch_ticket<'e, E>(executor: E, id: Uuid) -> DbResult<Option<Ticket>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as::<_, TicketRow>(&format!(
        "SELECT {TICKET_COLUMNS} {TICKET_FROM} WHERE t.id = $1"
    ))
    .bind(id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(Ticket::from))
}

async fn fetch_ticket_by_dedupe<'e, E>(executor: E, dedupe_key: &str) -> DbResult<Option<Ticket>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as::<_, TicketRow>(&format!(
        "SELECT {TICKET_COLUMNS} {TICKET_FROM}
          WHERE t.dedupe_key = $1 AND t.status IN ('open','in_progress')
          ORDER BY t.created_at DESC LIMIT 1"
    ))
    .bind(dedupe_key)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(Ticket::from))
}

async fn fetch_event<'e, E>(executor: E, id: i64) -> DbResult<Option<TicketEvent>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as::<_, TicketEventRow>(&format!(
        "SELECT {EVENT_COLUMNS} {EVENT_FROM} WHERE e.id = $1"
    ))
    .bind(id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(TicketEvent::from))
}

/// 转义 LIKE/ILIKE 通配符，配合 SQL 里的 `ESCAPE '\'` 使用。
fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '%' => escaped.push_str("\\%"),
            '_' => escaped.push_str("\\_"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// 便于 worker 单元测试：按优先级计算默认 due_at（与建单 SQL 一致）。
pub fn due_at_for(priority: TicketPriority, now: DateTime<Utc>) -> DateTime<Utc> {
    default_due_at(priority, now)
}

/// 自动单 metadata 的序列化辅助（worker / 测试共用）。
pub fn signal_metadata(value: &impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or_else(|_| json!({}))
}

#[cfg(test)]
mod tests {
    use super::escape_like;

    #[test]
    fn like_wildcards_are_escaped() {
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("room_101"), "room\\_101");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
        assert_eq!(escape_like("普通关键词"), "普通关键词");
    }
}
