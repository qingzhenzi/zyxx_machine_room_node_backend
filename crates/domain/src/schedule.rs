//! 课表领域模型与纯展开算法。
//!
//! 契约见 `docs/schedule-design.md` 第 3 节。对外 DTO 的时间字段约定：
//! `TIME` 用字符串 `"HH:MM"`（也接受 `"HH:MM:SS"`）、`DATE` 用 [`NaiveDate`]、
//! 时间点用 [`DateTime<Utc>`]。时区统一用「UTC 偏移分钟」，不引入 `chrono-tz`。

use std::{fmt, str::FromStr};

use chrono::{DateTime, Datelike, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    device::RoomDevice,
    error::{DomainError, DomainResult},
    validation::{parse_time, parse_timezone},
};

/// 默认时区（IANA）：Asia/Shanghai。
pub const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";
/// 排课实例物化的默认天数。
pub const DEFAULT_HORIZON_DAYS: i64 = 30;

pub use crate::validation::validate_timezone;

/// 区域：楼栋 / 楼层 / 片区。`parent_id` 与 `metadata` 为总管理节点 / 子管理节点预留。
#[derive(Debug, Clone, Serialize)]
pub struct Area {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: String,
    pub timezone: String,
    /// 区域级设备在线事件 webhook；房间未配置时使用（再回退到全局配置）。
    pub webhook_url: Option<String>,
    pub parent_id: Option<Uuid>,
    pub is_active: bool,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewArea {
    pub code: String,
    pub name: String,
    pub description: String,
    pub timezone: String,
    pub webhook_url: Option<String>,
    pub parent_id: Option<Uuid>,
    pub is_active: bool,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateArea {
    pub code: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub timezone: Option<String>,
    pub webhook_url: Option<Option<String>>, // Some(None) = 清空
    pub parent_id: Option<Option<Uuid>>,     // Some(None) = 清空
    pub is_active: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

/// 房间：调度最小单位，可绑定一个设备槽位（`device_enrollment.device_slot`）。
#[derive(Debug, Clone, Serialize)]
pub struct Room {
    pub id: Uuid,
    pub area_id: Uuid,
    pub area_name: String,
    pub timezone: String,
    pub code: String,
    pub name: String,
    pub capacity: i32,
    pub device_slot: Option<String>,
    /// 只读摘要：房间绑定槽位后的设备注册/在线状态。
    pub device: Option<RoomDevice>,
    pub webhook_url: Option<String>,
    pub is_active: bool,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewRoom {
    pub area_id: Uuid,
    pub code: String,
    pub name: String,
    pub capacity: i32,
    pub device_slot: Option<String>,
    pub webhook_url: Option<String>,
    pub is_active: bool,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateRoom {
    pub area_id: Option<Uuid>,
    pub code: Option<String>,
    pub name: Option<String>,
    pub capacity: Option<i32>,
    pub device_slot: Option<Option<String>>,
    pub webhook_url: Option<Option<String>>,
    pub is_active: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

/// 课表：一个房间的排课计划（可存在多份，通常按学期 / 周期区分）。
#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub id: Uuid,
    pub room_id: Uuid,
    pub room_name: String,
    pub timezone: String,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_active: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewSchedule {
    pub room_id: Uuid,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_active: bool,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSchedule {
    pub name: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub is_active: Option<bool>,
}

/// 默认周循环模板：ISO 星期（周一 = 1 .. 周日 = 7）+ 本地时段。
#[derive(Debug, Clone, Serialize)]
pub struct WeeklySlot {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub day_of_week: i16,
    pub start_time: String,
    pub end_time: String,
    pub course_name: String,
    pub teacher: String,
    pub room_id: Option<Uuid>,
    pub room_name: Option<String>,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewWeeklySlot {
    pub day_of_week: i16,
    pub start_time: String,
    pub end_time: String,
    pub course_name: String,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct ValidatedWeeklySlot {
    pub day_of_week: i16,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub course_name: String,
    pub teacher: String,
    pub room_id: Option<Uuid>,
    pub note: String,
}

/// 单日覆盖类型：`cancel` 停课 / `replace` 用自定义时段替换当天周模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleExceptionKind {
    Cancel,
    Replace,
}

impl ScheduleExceptionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Replace => "replace",
        }
    }
}

impl fmt::Display for ScheduleExceptionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ScheduleExceptionKind {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cancel" => Ok(Self::Cancel),
            "replace" => Ok(Self::Replace),
            _ => Err(()),
        }
    }
}

/// 单日覆盖：某天停课（`cancel`）或用自定义时段替换当天周模板（`replace`）。
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleException {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub exception_date: NaiveDate,
    pub kind: ScheduleExceptionKind,
    pub note: String,
    pub slots: Vec<ScheduleExceptionSlot>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleExceptionSlot {
    pub id: Uuid,
    pub exception_id: Uuid,
    pub start_time: String,
    pub end_time: String,
    pub course_name: String,
    pub teacher: String,
    pub room_id: Option<Uuid>,
    pub room_name: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewExceptionSlot {
    pub start_time: String,
    pub end_time: String,
    pub course_name: String,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct ValidatedExceptionSlot {
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub course_name: String,
    pub teacher: String,
    pub room_id: Option<Uuid>,
    pub note: String,
}

/// 排课实例来源：`weekly` 周模板 / `custom` 单日覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OccurrenceSource {
    Weekly,
    Custom,
}

impl OccurrenceSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Weekly => "weekly",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for OccurrenceSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 物化的排课实例：周模板 × 日期（或单日覆盖）展开后的具体一节课。
#[derive(Debug, Clone, Serialize)]
pub struct Occurrence {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub room_id: Uuid,
    pub timezone: String,
    pub occurrence_date: NaiveDate,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub course_name: String,
    pub teacher: String,
    pub source: OccurrenceSource,
    pub source_slot_id: Uuid,
    pub note: String,
}

/// 触发规则类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    OccurrenceStart,
    OccurrenceEnd,
    OccurrenceStartBefore,
    DailyTime,
    RoomOccupied,
    RoomIdle,
}

impl TriggerKind {
    pub const ALL: &'static [TriggerKind] = &[
        Self::OccurrenceStart,
        Self::OccurrenceEnd,
        Self::OccurrenceStartBefore,
        Self::DailyTime,
        Self::RoomOccupied,
        Self::RoomIdle,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OccurrenceStart => "occurrence_start",
            Self::OccurrenceEnd => "occurrence_end",
            Self::OccurrenceStartBefore => "occurrence_start_before",
            Self::DailyTime => "daily_time",
            Self::RoomOccupied => "room_occupied",
            Self::RoomIdle => "room_idle",
        }
    }
}

impl fmt::Display for TriggerKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TriggerKind {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "occurrence_start" => Ok(Self::OccurrenceStart),
            "occurrence_end" => Ok(Self::OccurrenceEnd),
            "occurrence_start_before" => Ok(Self::OccurrenceStartBefore),
            "daily_time" => Ok(Self::DailyTime),
            "room_occupied" => Ok(Self::RoomOccupied),
            "room_idle" => Ok(Self::RoomIdle),
            _ => Err(()),
        }
    }
}

/// 触发规则：按房间配置。`config` 为 JSONB（targets / webhook_url / downlink_kind / payload）。
#[derive(Debug, Clone, Serialize)]
pub struct EventRule {
    pub id: Uuid,
    pub room_id: Uuid,
    pub trigger_kind: TriggerKind,
    pub offset_minutes: i32,
    pub at_time: Option<String>, // "HH:MM"
    pub enabled: bool,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewEventRule {
    pub trigger_kind: TriggerKind,
    #[serde(default)]
    pub offset_minutes: i32,
    #[serde(default)]
    pub at_time: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_config")]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ValidatedEventRule {
    pub trigger_kind: TriggerKind,
    pub offset_minutes: i32,
    pub at_time: Option<NaiveTime>,
    pub enabled: bool,
    pub config: serde_json::Value,
}

fn default_true() -> bool {
    true
}

fn default_config() -> serde_json::Value {
    serde_json::json!({})
}

/// 事件投递目标：`pubsub` / `downlink` / `webhook`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryTarget {
    Pubsub,
    Downlink,
    Webhook,
}

impl DeliveryTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pubsub => "pubsub",
            Self::Downlink => "downlink",
            Self::Webhook => "webhook",
        }
    }
}

impl fmt::Display for DeliveryTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DeliveryTarget {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pubsub" => Ok(Self::Pubsub),
            "downlink" => Ok(Self::Downlink),
            "webhook" => Ok(Self::Webhook),
            _ => Err(()),
        }
    }
}

/// 投递状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryStatus {
    Pending,
    Delivered,
    Failed,
    Skipped,
}

impl DeliveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

impl fmt::Display for DeliveryStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 事件：`event_key` 唯一，保证多副本 / 重算时幂等。
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleEvent {
    pub id: Uuid,
    pub event_key: String,
    pub room_id: Uuid,
    pub timezone: String,
    pub schedule_id: Option<Uuid>,
    pub occurrence_id: Option<Uuid>,
    pub rule_id: Option<Uuid>,
    pub event_kind: String,
    pub title: String,
    pub payload: serde_json::Value,
    pub triggered_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewScheduleEvent {
    pub event_key: String,
    pub room_id: Uuid,
    pub timezone: String,
    pub schedule_id: Option<Uuid>,
    pub occurrence_id: Option<Uuid>,
    pub rule_id: Option<Uuid>,
    pub event_kind: String,
    pub title: String,
    pub payload: serde_json::Value,
    pub triggered_at: DateTime<Utc>,
}

/// 投递：每个目标一行；重试由 `next_attempt_at` 驱动。
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleEventDelivery {
    pub id: i64,
    pub event_id: Uuid,
    pub target: DeliveryTarget,
    pub status: DeliveryStatus,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// 投递 worker 的任务视图（事件 + 房间 + 投递）。
#[derive(Debug, Clone)]
pub struct DeliveryJob {
    pub delivery_id: i64,
    pub event_id: Uuid,
    pub target: DeliveryTarget,
    pub attempts: i32,
    pub event_kind: String,
    pub payload: serde_json::Value,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    pub device_slot: Option<String>,
    pub webhook_url: Option<String>,
}

/// 事件查询过滤条件。
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub room_id: Option<Uuid>,
    pub kind: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// 纯展开算法的课表视图（只依赖领域类型，便于单测）。
#[derive(Debug, Clone)]
pub struct ExpansionSchedule {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    /// IANA 时区名（继承自房间 → 区域）。
    pub timezone: String,
}

/// 展开后的一节课（尚未落库，`source_slot_id` 指向周模板或覆盖时段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedOccurrence {
    pub occurrence_date: NaiveDate,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub course_name: String,
    pub teacher: String,
    pub room_override: Option<Uuid>,
    pub source: OccurrenceSource,
    pub source_slot_id: Uuid,
    pub note: String,
}

/// 把「周模板 + 单日覆盖」展开成 `[from, to]` 内的具体排课实例（按 `start_at` 升序）。
///
/// 规则：
/// - 日期先 clamp 到 `[schedule.start_date, schedule.end_date]`；
/// - 该日有 `cancel` 覆盖：当天不排；
/// - 该日有 `replace` 覆盖：只用覆盖时段，忽略周模板；
/// - 否则用 `day_of_week == date.weekday().number_from_monday()` 的周模板；
/// - 本地时间（`start_time`）按 `schedule.timezone`（IANA 时区）经 [`local_to_utc`] 转 UTC，
///   夏令时跳变中不存在的时段返回 400，重复时段取较早的一个。
pub fn expand_occurrences(
    schedule: &ExpansionSchedule,
    weekly_slots: &[WeeklySlot],
    exceptions: &[ScheduleException],
    from: NaiveDate,
    to: NaiveDate,
) -> DomainResult<Vec<PlannedOccurrence>> {
    let first_date = from.max(schedule.start_date);
    let last_date = to.min(schedule.end_date);
    if first_date > last_date {
        return Ok(Vec::new());
    }

    let mut exceptions_by_date = std::collections::HashMap::new();
    for exception in exceptions {
        exceptions_by_date.insert(exception.exception_date, exception);
    }

    let mut planned = Vec::new();
    let mut date = first_date;
    while date <= last_date {
        if let Some(exception) = exceptions_by_date.get(&date) {
            match exception.kind {
                ScheduleExceptionKind::Cancel => {}
                ScheduleExceptionKind::Replace => {
                    for slot in &exception.slots {
                        planned.push(PlannedOccurrence {
                            occurrence_date: date,
                            start_at: local_to_utc(
                                date,
                                parse_slot_time(&slot.start_time)?,
                                &schedule.timezone,
                            )?,
                            end_at: local_to_utc(
                                date,
                                parse_slot_time(&slot.end_time)?,
                                &schedule.timezone,
                            )?,
                            course_name: slot.course_name.clone(),
                            teacher: slot.teacher.clone(),
                            room_override: slot.room_id,
                            source: OccurrenceSource::Custom,
                            source_slot_id: slot.id,
                            note: slot.note.clone(),
                        });
                    }
                }
            }
        } else {
            let day_of_week = date.weekday().number_from_monday() as i16;
            for slot in weekly_slots
                .iter()
                .filter(|slot| slot.day_of_week == day_of_week)
            {
                planned.push(PlannedOccurrence {
                    occurrence_date: date,
                    start_at: local_to_utc(
                        date,
                        parse_slot_time(&slot.start_time)?,
                        &schedule.timezone,
                    )?,
                    end_at: local_to_utc(
                        date,
                        parse_slot_time(&slot.end_time)?,
                        &schedule.timezone,
                    )?,
                    course_name: slot.course_name.clone(),
                    teacher: slot.teacher.clone(),
                    room_override: slot.room_id,
                    source: OccurrenceSource::Weekly,
                    source_slot_id: slot.id,
                    note: slot.note.clone(),
                });
            }
        }
        let Some(next_date) = date.succ_opt() else {
            break;
        };
        date = next_date;
    }

    planned.sort_by_key(|occurrence| occurrence.start_at);
    Ok(planned)
}

/// 解析库里存的 `"HH:MM"` 时段；库数据异常属于内部错误而非请求错误。
fn parse_slot_time(raw: &str) -> DomainResult<NaiveTime> {
    parse_time(raw).map_err(|_| DomainError::internal(format!("课表时段格式不正确：{raw}")))
}

/// 本地日期 + 本地时刻 → UTC（按 IANA 时区解析，自动处理夏令时）。
///
/// - 正常的本地时刻：[`LocalResult::Single`]；
/// - 夏令时重复时刻（秋季回拨）：取较早的一个（[`LocalResult::Ambiguous`]）；
/// - 夏令时跳变中不存在的本地时刻（春季拨快）：返回 400。
pub fn local_to_utc(
    date: NaiveDate,
    time: NaiveTime,
    timezone: &str,
) -> DomainResult<DateTime<Utc>> {
    let timezone = parse_timezone(timezone)?;
    let local = date.and_time(time);
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(instant) => Ok(instant.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, _later) => Ok(earlier.with_timezone(&Utc)),
        LocalResult::None => Err(DomainError::bad_request(format!(
            "本地时间 {local} 在时区 {} 不存在（夏令时跳变），请调整上课时间",
            timezone.name()
        ))),
    }
}

/// 时刻在给定时区下的本地日期。
pub fn local_date_of(instant: DateTime<Utc>, timezone: &str) -> DomainResult<NaiveDate> {
    let timezone = parse_timezone(timezone)?;
    Ok(instant.with_timezone(&timezone).date_naive())
}

/// 时刻在给定时区下的本地钟面时间。
pub fn local_clock_of(instant: DateTime<Utc>, timezone: &str) -> DomainResult<NaiveTime> {
    let timezone = parse_timezone(timezone)?;
    Ok(instant.with_timezone(&timezone).time())
}

/// 格式化带偏移的本地时间（RFC3339），如 `2026-11-02T08:00:00+08:00`；时区非法时退化为 UTC。
pub fn local_rfc3339(instant: DateTime<Utc>, timezone: &str) -> String {
    match timezone.trim().parse::<Tz>() {
        Ok(timezone) => instant.with_timezone(&timezone).to_rfc3339(),
        Err(_) => instant.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;

    use super::*;

    fn date(raw: &str) -> NaiveDate {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").expect("日期")
    }

    fn utc(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .map(|value| value.with_timezone(&Utc))
            .expect("时间")
    }

    fn time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).expect("时间")
    }

    fn schedule(start: &str, end: &str, timezone: &str) -> ExpansionSchedule {
        ExpansionSchedule {
            start_date: date(start),
            end_date: date(end),
            timezone: timezone.to_owned(),
        }
    }

    fn weekly_slot(
        day_of_week: i16,
        start: &str,
        end: &str,
        course: &str,
        room_id: Option<Uuid>,
    ) -> WeeklySlot {
        let slot_id = Uuid::new_v4();
        WeeklySlot {
            id: slot_id,
            schedule_id: Uuid::new_v4(),
            day_of_week,
            start_time: start.to_owned(),
            end_time: end.to_owned(),
            course_name: course.to_owned(),
            teacher: "张老师".to_owned(),
            room_id,
            room_name: None,
            note: String::new(),
            created_at: Utc::now(),
        }
    }

    fn exception(
        day: &str,
        kind: ScheduleExceptionKind,
        slots: Vec<ScheduleExceptionSlot>,
    ) -> ScheduleException {
        ScheduleException {
            id: Uuid::new_v4(),
            schedule_id: Uuid::new_v4(),
            exception_date: date(day),
            kind,
            note: String::new(),
            slots,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn exception_slot(start: &str, end: &str, course: &str) -> ScheduleExceptionSlot {
        ScheduleExceptionSlot {
            id: Uuid::new_v4(),
            exception_id: Uuid::new_v4(),
            start_time: start.to_owned(),
            end_time: end.to_owned(),
            course_name: course.to_owned(),
            teacher: String::new(),
            room_id: None,
            room_name: None,
            note: String::new(),
        }
    }

    #[test]
    fn weekly_slots_expand_by_iso_weekday() {
        // 2026-10-26 是周一，2026-10-27 是周二，2026-11-01 是周日。
        let schedule = schedule("2026-10-26", "2026-11-01", DEFAULT_TIMEZONE);
        let slots = vec![
            weekly_slot(1, "08:00", "09:30", "数学", None),
            weekly_slot(3, "14:00", "15:00", "英语", None),
        ];

        let planned = expand_occurrences(
            &schedule,
            &slots,
            &[],
            date("2026-10-26"),
            date("2026-11-01"),
        )
        .expect("展开成功");

        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].occurrence_date, date("2026-10-26"));
        assert_eq!(planned[0].course_name, "数学");
        assert_eq!(planned[0].source, OccurrenceSource::Weekly);
        assert_eq!(planned[0].source_slot_id, slots[0].id);
        assert_eq!(planned[0].start_at, utc("2026-10-26T00:00:00Z"));
        assert_eq!(planned[0].end_at, utc("2026-10-26T01:30:00Z"));
        assert_eq!(planned[1].occurrence_date, date("2026-10-28"));
        assert_eq!(planned[1].course_name, "英语");
        assert_eq!(planned[1].start_at, utc("2026-10-28T06:00:00Z"));
    }

    #[test]
    fn expansion_range_is_clamped_to_schedule_dates() {
        let schedule = schedule("2026-10-26", "2026-11-01", DEFAULT_TIMEZONE);
        let slots = vec![weekly_slot(1, "08:00", "09:00", "数学", None)];

        // 查询范围远大于课表范围，但只应展开课表内的周一（10-26、11-02 不在范围内）。
        let planned = expand_occurrences(
            &schedule,
            &slots,
            &[],
            date("2026-01-01"),
            date("2027-12-31"),
        )
        .expect("展开成功");
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].occurrence_date, date("2026-10-26"));

        // 查询范围完全落在课表外 → 空。
        let planned = expand_occurrences(
            &schedule,
            &slots,
            &[],
            date("2027-01-01"),
            date("2027-02-01"),
        )
        .expect("展开成功");
        assert!(planned.is_empty());
    }

    #[test]
    fn cancel_exception_skips_the_day() {
        // 2026-10-26、2026-11-02 都是周一。
        let schedule = schedule("2026-10-26", "2026-11-02", DEFAULT_TIMEZONE);
        let slots = vec![weekly_slot(1, "08:00", "09:00", "数学", None)];
        let exceptions = vec![exception(
            "2026-10-26",
            ScheduleExceptionKind::Cancel,
            Vec::new(),
        )];

        let planned = expand_occurrences(
            &schedule,
            &slots,
            &exceptions,
            date("2026-10-26"),
            date("2026-11-02"),
        )
        .expect("展开成功");
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].occurrence_date, date("2026-11-02"));
    }

    #[test]
    fn replace_exception_ignores_weekly_template() {
        let schedule = schedule("2026-10-26", "2026-10-26", DEFAULT_TIMEZONE);
        let slots = vec![weekly_slot(1, "08:00", "09:00", "数学", None)];
        let override_room = Uuid::new_v4();
        let mut slot = exception_slot("10:00", "11:00", "自习");
        slot.room_id = Some(override_room);
        let exception_id = slot.id;
        let exceptions = vec![exception(
            "2026-10-26",
            ScheduleExceptionKind::Replace,
            vec![slot],
        )];

        let planned = expand_occurrences(
            &schedule,
            &slots,
            &exceptions,
            date("2026-10-26"),
            date("2026-10-26"),
        )
        .expect("展开成功");
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].source, OccurrenceSource::Custom);
        assert_eq!(planned[0].source_slot_id, exception_id);
        assert_eq!(planned[0].room_override, Some(override_room));
        assert_eq!(planned[0].course_name, "自习");
        assert_eq!(planned[0].start_at, utc("2026-10-26T02:00:00Z"));
        assert_eq!(planned[0].end_at, utc("2026-10-26T03:00:00Z"));
    }

    #[test]
    fn expansion_crosses_month_and_year_boundaries() {
        // 2026-12-28 与 2027-01-04 都是周一，跨年。
        let schedule = schedule("2026-12-28", "2027-01-10", DEFAULT_TIMEZONE);
        let slots = vec![weekly_slot(1, "08:00", "09:00", "数学", None)];

        let planned = expand_occurrences(
            &schedule,
            &slots,
            &[],
            date("2026-12-01"),
            date("2027-02-01"),
        )
        .expect("展开成功");
        let dates: Vec<NaiveDate> = planned.iter().map(|item| item.occurrence_date).collect();
        assert_eq!(dates, vec![date("2026-12-28"), date("2027-01-04")]);
        assert!(
            planned
                .iter()
                .all(|item| item.occurrence_date.weekday() == chrono::Weekday::Mon)
        );
        assert_eq!(planned[1].start_at, utc("2027-01-04T00:00:00Z"));
    }

    #[test]
    fn local_times_are_converted_to_utc_with_timezone() {
        let day = date("2026-10-26");
        let slots = vec![weekly_slot(1, "08:00", "09:00", "数学", None)];

        // Asia/Shanghai（UTC+8）：08:00 => 00:00Z。
        let shanghai = schedule("2026-10-26", "2026-10-26", DEFAULT_TIMEZONE);
        let planned = expand_occurrences(&shanghai, &slots, &[], day, day).expect("展开成功");
        assert_eq!(planned[0].start_at, utc("2026-10-26T00:00:00Z"));

        // UTC：08:00 => 08:00Z。
        let utc_zone = schedule("2026-10-26", "2026-10-26", "UTC");
        let planned = expand_occurrences(&utc_zone, &slots, &[], day, day).expect("展开成功");
        assert_eq!(planned[0].start_at, utc("2026-10-26T08:00:00Z"));

        // America/New_York：10-26 仍在夏令时（EDT, UTC-4）=> 12:00Z。
        let new_york = schedule("2026-10-26", "2026-10-26", "America/New_York");
        let planned = expand_occurrences(&new_york, &slots, &[], day, day).expect("展开成功");
        assert_eq!(planned[0].start_at, utc("2026-10-26T12:00:00Z"));

        // 冬季（EST, UTC-5）：2026-01-05 周一 08:00 => 13:00Z。
        let winter = schedule("2026-01-05", "2026-01-05", "America/New_York");
        let winter_day = date("2026-01-05");
        let planned =
            expand_occurrences(&winter, &slots, &[], winter_day, winter_day).expect("展开成功");
        assert_eq!(planned[0].start_at, utc("2026-01-05T13:00:00Z"));
    }

    #[test]
    fn local_helpers_convert_with_timezone() {
        let instant = utc("2026-11-02T00:00:00Z");
        assert_eq!(
            local_to_utc(date("2026-11-02"), time(8, 0), DEFAULT_TIMEZONE).unwrap(),
            instant
        );
        assert_eq!(
            local_date_of(instant, DEFAULT_TIMEZONE).unwrap(),
            date("2026-11-02")
        );
        assert_eq!(
            local_clock_of(instant, DEFAULT_TIMEZONE).unwrap(),
            time(8, 0)
        );
        // 跨日：Shanghai 的 2026-11-03 04:00 = 2026-11-02T20:00Z。
        let evening = utc("2026-11-02T20:00:00Z");
        assert_eq!(
            local_date_of(evening, DEFAULT_TIMEZONE).unwrap(),
            date("2026-11-03")
        );
        assert_eq!(
            local_clock_of(evening, DEFAULT_TIMEZONE).unwrap(),
            time(4, 0)
        );
        // 非法时区：400。
        assert!(matches!(
            local_to_utc(date("2026-11-02"), time(8, 0), "Mars/Phobos"),
            Err(DomainError::BadRequest(_))
        ));
        assert!(matches!(
            local_date_of(instant, "Mars/Phobos"),
            Err(DomainError::BadRequest(_))
        ));
        assert!(matches!(
            local_clock_of(instant, "Mars/Phobos"),
            Err(DomainError::BadRequest(_))
        ));
    }

    #[test]
    fn new_york_spring_forward_gap_is_rejected() {
        // 2026-03-08 02:00 EST → 03:00 EDT，本地 02:30 不存在。
        let error =
            local_to_utc(date("2026-03-08"), time(2, 30), "America/New_York").expect_err("不存在");
        assert!(matches!(error, DomainError::BadRequest(_)));

        // 展开该天同一时段同样返回 400。
        let gap_day = date("2026-03-08");
        let gap_schedule = schedule("2026-03-08", "2026-03-08", "America/New_York");
        let slots = vec![weekly_slot(7, "02:30", "03:30", "数学", None)]; // 2026-03-08 是周日
        let error =
            expand_occurrences(&gap_schedule, &slots, &[], gap_day, gap_day).expect_err("不存在");
        assert!(matches!(error, DomainError::BadRequest(_)));

        // 跳变之后的 03:30 正常（EDT, UTC-4）= 07:30Z。
        assert_eq!(
            local_to_utc(date("2026-03-08"), time(3, 30), "America/New_York").unwrap(),
            utc("2026-03-08T07:30:00Z")
        );
    }

    #[test]
    fn new_york_fall_back_overlap_takes_earlier() {
        // 2026-11-01 02:00 EDT → 01:00 EST，本地 01:30 出现两次；取较早（EDT, UTC-4）= 05:30Z。
        assert_eq!(
            local_to_utc(date("2026-11-01"), time(1, 30), "America/New_York").unwrap(),
            utc("2026-11-01T05:30:00Z")
        );

        let overlap_day = date("2026-11-01");
        let fall_back = schedule("2026-11-01", "2026-11-01", "America/New_York");
        let slots = vec![weekly_slot(7, "01:30", "02:30", "数学", None)]; // 2026-11-01 是周日
        let planned = expand_occurrences(&fall_back, &slots, &[], overlap_day, overlap_day)
            .expect("展开成功");
        assert_eq!(planned[0].start_at, utc("2026-11-01T05:30:00Z"));
        // 02:30 已回拨为 EST（UTC-5）=> 07:30Z。
        assert_eq!(planned[0].end_at, utc("2026-11-01T07:30:00Z"));
    }

    #[test]
    fn local_rfc3339_formats_offsets() {
        let instant = utc("2026-11-02T00:00:00Z");
        assert_eq!(
            local_rfc3339(instant, DEFAULT_TIMEZONE),
            "2026-11-02T08:00:00+08:00"
        );
        assert_eq!(
            local_rfc3339(instant, "Asia/Tokyo"),
            "2026-11-02T09:00:00+09:00"
        );
        // 夏令时：Berlin 夏季 +02:00，冬季 +01:00。
        assert_eq!(
            local_rfc3339(utc("2026-07-01T12:00:00Z"), "Europe/Berlin"),
            "2026-07-01T14:00:00+02:00"
        );
        assert_eq!(
            local_rfc3339(utc("2026-01-15T12:00:00Z"), "Europe/Berlin"),
            "2026-01-15T13:00:00+01:00"
        );
        // 时区非法时退化为 UTC。
        let fallback = local_rfc3339(instant, "Mars/Phobos");
        assert!(fallback.starts_with("2026-11-02T00:00:00"), "{fallback}");
        assert!(fallback.ends_with("+00:00"), "{fallback}");
    }

    #[test]
    fn occurrences_are_sorted_by_start_at() {
        let schedule = schedule("2026-10-26", "2026-10-28", DEFAULT_TIMEZONE);
        let slots = vec![
            weekly_slot(1, "15:00", "16:00", "下午课", None),
            weekly_slot(1, "08:00", "09:00", "上午课", None),
            weekly_slot(3, "08:00", "09:00", "周三课", None),
        ];

        let planned = expand_occurrences(
            &schedule,
            &slots,
            &[],
            date("2026-10-26"),
            date("2026-10-28"),
        )
        .expect("展开成功");
        let courses: Vec<&str> = planned
            .iter()
            .map(|item| item.course_name.as_str())
            .collect();
        assert_eq!(courses, vec!["上午课", "下午课", "周三课"]);
        assert!(
            planned
                .windows(2)
                .all(|pair| pair[0].start_at <= pair[1].start_at)
        );
    }

    #[test]
    fn invalid_stored_time_is_an_internal_error() {
        let schedule = schedule("2026-10-26", "2026-10-26", DEFAULT_TIMEZONE);
        let slots = vec![weekly_slot(1, "8:00", "09:00", "数学", None)];
        let error = expand_occurrences(
            &schedule,
            &slots,
            &[],
            date("2026-10-26"),
            date("2026-10-26"),
        )
        .expect_err("非法时段应报错");
        assert!(matches!(error, DomainError::Internal(_)));
    }

    #[test]
    fn trigger_kind_round_trips_and_serializes_snake_case() {
        assert_eq!(TriggerKind::ALL.len(), 6);
        for kind in TriggerKind::ALL {
            assert_eq!(kind.to_string(), kind.as_str());
            assert_eq!(kind.as_str().parse::<TriggerKind>(), Ok(*kind));
            let json = serde_json::to_value(kind).expect("序列化");
            assert_eq!(json.as_str(), Some(kind.as_str()));
        }
        assert_eq!(
            serde_json::from_value::<TriggerKind>(serde_json::json!("room_idle"))
                .expect("反序列化"),
            TriggerKind::RoomIdle
        );
        assert!("unknown".parse::<TriggerKind>().is_err());
    }

    #[test]
    fn enum_serde_shapes_match_database_values() {
        assert_eq!(
            serde_json::to_value(ScheduleExceptionKind::Cancel).expect("序列化"),
            serde_json::json!("cancel")
        );
        assert_eq!(
            serde_json::to_value(ScheduleExceptionKind::Replace).expect("序列化"),
            serde_json::json!("replace")
        );
        assert_eq!(
            serde_json::to_value(OccurrenceSource::Weekly).expect("序列化"),
            serde_json::json!("weekly")
        );
        assert_eq!(
            serde_json::to_value(OccurrenceSource::Custom).expect("序列化"),
            serde_json::json!("custom")
        );
        assert_eq!(
            serde_json::to_value(DeliveryTarget::Webhook).expect("序列化"),
            serde_json::json!("webhook")
        );
        assert_eq!(
            serde_json::to_value(DeliveryStatus::Pending).expect("序列化"),
            serde_json::json!("pending")
        );
        assert_eq!(
            serde_json::from_value::<DeliveryStatus>(serde_json::json!("skipped"))
                .expect("反序列化"),
            DeliveryStatus::Skipped
        );
    }

    #[test]
    fn delivery_target_parses_from_database_text() {
        for target in [
            DeliveryTarget::Pubsub,
            DeliveryTarget::Downlink,
            DeliveryTarget::Webhook,
        ] {
            assert_eq!(target.as_str().parse::<DeliveryTarget>(), Ok(target));
            assert_eq!(target.to_string(), target.as_str());
        }
        assert!("smtp".parse::<DeliveryTarget>().is_err());
    }

    #[test]
    fn new_event_rule_applies_contract_defaults() {
        let rule: NewEventRule =
            serde_json::from_value(serde_json::json!({"trigger_kind": "daily_time"}))
                .expect("反序列化");
        assert_eq!(rule.trigger_kind, TriggerKind::DailyTime);
        assert_eq!(rule.offset_minutes, 0);
        assert!(rule.enabled);
        assert_eq!(rule.config, serde_json::json!({}));
        assert_eq!(rule.at_time, None);

        let rule: NewEventRule = serde_json::from_value(serde_json::json!({
            "trigger_kind": "occurrence_start_before",
            "offset_minutes": 15,
            "at_time": "07:30",
            "enabled": false,
            "config": {"targets": ["pubsub"]}
        }))
        .expect("反序列化");
        assert_eq!(rule.trigger_kind, TriggerKind::OccurrenceStartBefore);
        assert_eq!(rule.offset_minutes, 15);
        assert_eq!(rule.at_time.as_deref(), Some("07:30"));
        assert!(!rule.enabled);
        assert_eq!(rule.config, serde_json::json!({"targets": ["pubsub"]}));
    }
}
