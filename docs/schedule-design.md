# 课表（Schedule）设计与实现契约 v1

> 本文是实现契约：数据库 schema、领域类型、仓储签名、HTTP API、事件引擎语义都以此为准。
> 新增代码必须与本文一致；如需改动先改本文并在 PR 中说明。

## 1. 目标与范围

为「房间节点」提供课表能力：

- **区域（area）→ 房间（room）** 两级组织；房间是调度最小单位，可绑定设备槽位（`device_slot`）。
- **课表（schedule）挂在房间**：一份课表 = 生效日期范围 + 默认周循环（每周固定时段）+ 单日覆盖（停课 / 当天改时段、改课程、改房间）。
- 后端提供区域 / 房间 / 课表 / 周模板 / 单日覆盖 / 排课实例 / 触发规则 / 事件的全部管理 API。
- **到达条件产生事件**：按房间配置的触发规则，在到点时生成事件，并以四种方式投递：
  1. 落库（可查询、可重放）；
  2. Redis Pub/Sub（实时通知其他服务）；
  3. 房间设备下行（复用 `device_downlink` 表 + `qe-device-pki:downlink` 通知频道）；
  4. Webhook 回调（可重试）。
- 当前只实现「房间节点」；`areas.parent_id`、`rooms.metadata` 为将来的**总管理节点 / 子管理节点**预留，本版本不实现节点间同步。

不在本版本范围：学期管理、教师/课程独立实体、调课审批流、事件归档、跨节点同步。

## 2. 数据库迁移 `migrations/0009_schedule.sql`

使用 `0001_init.sql` 已定义的 `set_updated_at()` 触发器函数。全部 DDL：

```sql
-- 区域：楼栋 / 楼层 / 片区。parent_id 与 metadata 为总管理节点 / 子管理节点预留。
CREATE TABLE areas (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    timezone TEXT NOT NULL DEFAULT 'Asia/Shanghai', -- IANA 时区（地区）
    parent_id UUID REFERENCES areas(id) ON DELETE SET NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX areas_parent_idx ON areas (parent_id);

-- 房间：调度最小单位，可绑定一个设备槽位（device_enrollment.device_slot）。
CREATE TABLE rooms (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    area_id UUID NOT NULL REFERENCES areas(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    capacity INT NOT NULL DEFAULT 0 CHECK (capacity >= 0),
    device_slot TEXT,
    webhook_url TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX rooms_area_idx ON rooms (area_id);
CREATE UNIQUE INDEX rooms_device_slot_key ON rooms (device_slot) WHERE device_slot IS NOT NULL;

-- 课表：一个房间的排课计划（可存在多份，通常按学期/周期区分）。
CREATE TABLE schedules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_date <= end_date)
);
CREATE INDEX schedules_room_idx ON schedules (room_id, start_date, end_date);

-- 默认周循环模板：ISO 星期（周一=1..周日=7）+ 本地时段。
CREATE TABLE schedule_weekly_slots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    day_of_week SMALLINT NOT NULL CHECK (day_of_week BETWEEN 1 AND 7),
    start_time TIME NOT NULL,
    end_time TIME NOT NULL,
    course_name TEXT NOT NULL,
    teacher TEXT NOT NULL DEFAULT '',
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL, -- 调课到其他房间；空 = 课表所属房间
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_time < end_time)
);
CREATE INDEX schedule_weekly_slots_schedule_idx
    ON schedule_weekly_slots (schedule_id, day_of_week, start_time);

-- 单日覆盖：某天停课（cancel）或用自定义时段替换当天周模板（replace）。
CREATE TABLE schedule_exceptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    exception_date DATE NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('cancel', 'replace')),
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (schedule_id, exception_date)
);

CREATE TABLE schedule_exception_slots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    exception_id UUID NOT NULL REFERENCES schedule_exceptions(id) ON DELETE CASCADE,
    start_time TIME NOT NULL,
    end_time TIME NOT NULL,
    course_name TEXT NOT NULL,
    teacher TEXT NOT NULL DEFAULT '',
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
    note TEXT NOT NULL DEFAULT '',
    CHECK (start_time < end_time)
);
CREATE INDEX schedule_exception_slots_exception_idx
    ON schedule_exception_slots (exception_id, start_time);

-- 物化的排课实例：周模板 × 日期（或单日覆盖）展开后的具体一节课。
CREATE TABLE schedule_occurrences (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, -- 实际上课房间（可能调课）
    occurrence_date DATE NOT NULL,
    start_at TIMESTAMPTZ NOT NULL,
    end_at TIMESTAMPTZ NOT NULL,
    course_name TEXT NOT NULL,
    teacher TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL CHECK (source IN ('weekly', 'custom')),
    source_slot_id UUID NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_at < end_at),
    UNIQUE (schedule_id, source, source_slot_id, occurrence_date)
);
CREATE INDEX schedule_occurrences_room_time_idx ON schedule_occurrences (room_id, start_at);
CREATE INDEX schedule_occurrences_schedule_time_idx ON schedule_occurrences (schedule_id, start_at);
CREATE INDEX schedule_occurrences_time_idx ON schedule_occurrences (start_at, end_at);

-- 触发规则：按房间配置。config 为 JSONB（targets / webhook_url / downlink_kind / payload）。
CREATE TABLE schedule_event_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    trigger_kind TEXT NOT NULL CHECK (trigger_kind IN (
        'occurrence_start', 'occurrence_end', 'occurrence_start_before',
        'daily_time', 'room_occupied', 'room_idle'
    )),
    offset_minutes INT NOT NULL DEFAULT 0 CHECK (offset_minutes BETWEEN 0 AND 1440),
    at_time TIME,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (trigger_kind <> 'daily_time' OR at_time IS NOT NULL)
);
CREATE INDEX schedule_event_rules_room_idx
    ON schedule_event_rules (room_id) WHERE enabled;

-- 事件：event_key 唯一，保证多副本 / 重算时幂等。
CREATE TABLE schedule_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_key TEXT NOT NULL UNIQUE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    schedule_id UUID REFERENCES schedules(id) ON DELETE SET NULL,
    occurrence_id UUID REFERENCES schedule_occurrences(id) ON DELETE SET NULL,
    rule_id UUID REFERENCES schedule_event_rules(id) ON DELETE SET NULL,
    event_kind TEXT NOT NULL,
    timezone TEXT NOT NULL, -- 生成时的区域时区快照
    title TEXT NOT NULL DEFAULT '',
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    triggered_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX schedule_events_room_time_idx ON schedule_events (room_id, triggered_at DESC);
CREATE INDEX schedule_events_created_idx ON schedule_events (created_at DESC);

-- 投递：每个目标一行；重试由 next_attempt_at 驱动。
CREATE TABLE schedule_event_deliveries (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES schedule_events(id) ON DELETE CASCADE,
    target TEXT NOT NULL CHECK (target IN ('pubsub', 'downlink', 'webhook')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'delivered', 'failed', 'skipped')),
    attempts INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    delivered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (event_id, target)
);
CREATE INDEX schedule_event_deliveries_pending_idx
    ON schedule_event_deliveries (next_attempt_at) WHERE status = 'pending';

-- 新权限
INSERT INTO permissions (code, name, description) VALUES
    ('area:read',           '查看区域',     '查看区域列表与详情'),
    ('area:manage',         '管理区域',     '创建、修改、删除区域'),
    ('room:read',           '查看房间',     '查看房间列表与详情'),
    ('room:manage',         '管理房间',     '创建、修改、删除房间与触发规则'),
    ('schedule:read',       '查看课表',     '查看课表、周模板、单日覆盖与排课实例'),
    ('schedule:manage',     '管理课表',     '创建、修改、删除课表与触发规则'),
    ('schedule:event:read', '查看课表事件', '查看触发事件与投递状态'),
    ('schedule:event:manage','管理课表事件','重试或重放事件投递')
ON CONFLICT (code) DO NOTHING;

-- 系统管理员自动补齐（bootstrap 启动时也会补，这里保证未跑 bootstrap 的库也可用）
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

-- viewer / editor 获得只读权限
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r
JOIN permissions p ON p.code IN ('area:read', 'room:read', 'schedule:read', 'schedule:event:read')
WHERE r.code IN ('viewer', 'editor')
ON CONFLICT DO NOTHING;
```

注意：本迁移必须幂等友好（`ON CONFLICT DO NOTHING`），但 `CREATE TABLE` 不需要重复执行保护（sqlx 迁移只跑一次）。

除上表外，迁移还会为 5 张带 `updated_at` 的表挂上 `set_updated_at()` 触发器
（`areas`、`rooms`、`schedules`、`schedule_exceptions`、`schedule_event_rules`）：

```sql
DROP TRIGGER IF EXISTS areas_set_updated_at ON areas;
CREATE TRIGGER areas_set_updated_at
    BEFORE UPDATE ON areas
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
-- rooms / schedules / schedule_exceptions / schedule_event_rules 同理
```

## 3. 领域层 `qe-domain`：新增模块 `schedule`

文件：`crates/domain/src/schedule.rs`（新）、`crates/domain/src/lib.rs`（加 `pub mod schedule;`）、
`crates/domain/src/rbac.rs`（权限常量）、`crates/domain/src/validation.rs`（校验函数）。

时间字段约定：对外 DTO 中 `TIME` 一律用字符串 `"HH:MM"`（也接受 `"HH:MM:SS"`），
`DATE` 用 `chrono::NaiveDate`（serde `"YYYY-MM-DD"`），时间点用 `chrono::DateTime<Utc>`（RFC3339）。
内部计算用 `NaiveTime`；时区统一用「UTC 偏移分钟」，默认 480（Asia/Shanghai），不引入 chrono-tz。

```rust
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";
pub const DEFAULT_HORIZON_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize)]
pub struct Area {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: String,
    pub timezone: String, // IANA
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
    pub timezone: String, // IANA，API 缺省 Asia/Shanghai
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
    pub parent_id: Option<Option<Uuid>>, // Some(None) = 清空
    pub is_active: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Room {
    pub id: Uuid,
    pub area_id: Uuid,
    pub area_name: String,
    pub code: String,
    pub name: String,
    pub capacity: i32,
    pub device_slot: Option<String>,
    pub webhook_url: Option<String>,
    pub timezone: String, // 只读：继承区域
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

#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub id: Uuid,
    pub room_id: Uuid,
    pub room_name: String,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub timezone: String, // 只读：继承区域
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

#[derive(Debug, Clone, Serialize)]
pub struct WeeklySlot {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub day_of_week: i16,          // 1..=7 (ISO)
    pub start_time: String,        // "HH:MM"
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleExceptionKind { Cancel, Replace }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OccurrenceSource { Weekly, Custom }

#[derive(Debug, Clone, Serialize)]
pub struct Occurrence {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub room_id: Uuid,
    pub occurrence_date: NaiveDate,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub course_name: String,
    pub teacher: String,
    pub source: OccurrenceSource,
    pub source_slot_id: Uuid,
    pub note: String,
    pub timezone: String, // 只读：课表→房间→区域
}

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
    pub const ALL: &'static [TriggerKind];
    pub const fn as_str(self) -> &'static str;
}
impl std::fmt::Display for TriggerKind;      // as_str
impl std::str::FromStr for TriggerKind;      // 反向解析，Err = ()

#[derive(Debug, Clone, Serialize)]
pub struct EventRule {
    pub id: Uuid,
    pub room_id: Uuid,
    pub trigger_kind: TriggerKind,
    pub offset_minutes: i32,
    pub at_time: Option<String>,   // "HH:MM"
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryTarget { Pubsub, Downlink, Webhook }
impl DeliveryTarget { pub const fn as_str(self) -> &'static str; }
impl std::str::FromStr for DeliveryTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryStatus { Pending, Delivered, Failed, Skipped }

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleEvent {
    pub id: Uuid,
    pub event_key: String,
    pub room_id: Uuid,
    pub schedule_id: Option<Uuid>,
    pub occurrence_id: Option<Uuid>,
    pub rule_id: Option<Uuid>,
    pub event_kind: String,
    pub timezone: String,
    pub title: String,
    pub payload: serde_json::Value,
    pub triggered_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewScheduleEvent {
    pub event_key: String,
    pub room_id: Uuid,
    pub schedule_id: Option<Uuid>,
    pub occurrence_id: Option<Uuid>,
    pub rule_id: Option<Uuid>,
    pub event_kind: String,
    pub timezone: String,
    pub title: String,
    pub payload: serde_json::Value,
    pub triggered_at: DateTime<Utc>,
}

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

#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub room_id: Option<Uuid>,
    pub kind: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}
```

### 3.1 纯展开算法

```rust
#[derive(Debug, Clone, Copy)]
pub struct ExpansionSchedule {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub timezone: String, // IANA
}

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

/// 把「周模板 + 单日覆盖」展开成 `[from, to]` 内的具体排课实例（按 start_at 升序）。
/// 规则：
/// - 日期先 clamp 到 `[schedule.start_date, schedule.end_date]`；
/// - 该日有 `cancel` 覆盖：当天不排；
/// - 该日有 `replace` 覆盖：只用覆盖时段，忽略周模板；
/// - 否则用 `day_of_week == date.weekday().number_from_monday()` 的周模板；
/// - 本地时间（`start_time`）按 `timezone`（IANA）转 UTC；DST 重复时段取较早、跳变不存在时段返回 400。
pub fn expand_occurrences(
    schedule: &ExpansionSchedule,
    weekly_slots: &[WeeklySlot],
    exceptions: &[ScheduleException],
    from: NaiveDate,
    to: NaiveDate,
) -> DomainResult<Vec<PlannedOccurrence>>;
```

### 3.2 校验函数（放在 `validation.rs`）

```rust
pub fn normalize_code(raw: &str, label: &str) -> DomainResult<String>;
    // 去空白、小写；长度 2..=64；首字符字母；允许 ascii 字母数字 - _ .
pub fn normalize_schedule_name(raw: &str) -> DomainResult<String>;   // 1..=200
pub fn normalize_course_name(raw: &str) -> DomainResult<String>;     // 1..=200
pub fn normalize_teacher(raw: &str) -> DomainResult<String>;         // 允许空，<=100
pub fn normalize_note(raw: &str) -> DomainResult<String>;            // <=1000
pub fn normalize_webhook_url(raw: &str) -> DomainResult<Option<String>>;
    // 空 => None；只允许 http/https；<=2000；去空白
pub fn validate_capacity(capacity: i32) -> DomainResult<i32>;        // 0..=100000
pub fn validate_timezone(raw: &str) -> DomainResult<String>;        // IANA 时区名
pub fn validate_metadata(metadata: serde_json::Value) -> DomainResult<serde_json::Value>;
    // 必须是 object；序列化后 <= 16KB
pub fn validate_date_range(start: NaiveDate, end: NaiveDate) -> DomainResult<()>;
    // start <= end
pub fn parse_time(raw: &str) -> DomainResult<NaiveTime>;             // "HH:MM" 或 "HH:MM:SS"
pub fn parse_optional_time(raw: Option<&str>) -> DomainResult<Option<NaiveTime>>;
pub fn validate_weekly_slots(slots: &[NewWeeklySlot]) -> DomainResult<Vec<ValidatedWeeklySlot>>;
    // 每个 slot：day_of_week 1..=7、start<end；同一星期几内时段不得重叠（排序后检查）；
    // 最多 200 条
pub fn validate_exception_slots(slots: &[NewExceptionSlot]) -> DomainResult<Vec<ValidatedExceptionSlot>>;
    // start<end、同日时段不重叠、最多 100 条
pub fn validate_event_rules(rules: &[NewEventRule]) -> DomainResult<Vec<ValidatedEventRule>>;
    // at_time 仅 daily_time 使用且必填；offset_minutes 0..=1440；
    // occurrence_start_before 的 offset 必须 > 0；config 必须是 object、<=8KB；
    // 同一房间同一 (trigger_kind, offset_minutes, at_time) 去重；最多 50 条
pub fn validate_occurrence_range(from: Option<NaiveDate>, to: Option<NaiveDate>, today: NaiveDate)
    -> DomainResult<(NaiveDate, NaiveDate)>;
    // 默认 [today, today+7]；to >= from；跨度 <= 366 天
```

### 3.3 权限常量（`rbac.rs`）

在 `permissions` 模块新增并加入 `ALL`：

```
AREA_READ = "area:read"; AREA_MANAGE = "area:manage";
ROOM_READ = "room:read"; ROOM_MANAGE = "room:manage";
SCHEDULE_READ = "schedule:read"; SCHEDULE_MANAGE = "schedule:manage";
SCHEDULE_EVENT_READ = "schedule:event:read"; SCHEDULE_EVENT_MANAGE = "schedule:event:manage";
```

## 4. 仓储层 `qe-db`

新文件 `crates/db/src/schedule.rs`；`crates/db/src/lib.rs` 加 `pub mod schedule;`。
行模型（`FromRow`）直接定义在 `schedule.rs` 内，不改 `rows.rs`，避免冲突。
时间输出统一 `format("%H:%M")`。

```rust
pub struct AreaRepo { /* PgPool */ }
impl AreaRepo {
    pub fn new(db: PgPool) -> Self;
    pub async fn list(&self, keyword: &str, include_inactive: bool, page: i64, page_size: i64) -> DbResult<Page<Area>>;
    pub async fn get(&self, id: Uuid) -> DbResult<Option<Area>>;
    pub async fn create(&self, input: &NewArea) -> DbResult<Area>;
    pub async fn update(&self, id: Uuid, input: &UpdateArea) -> DbResult<Option<Area>>;
    pub async fn delete(&self, id: Uuid) -> DbResult<bool>; // 仍有房间时返回 Conflict
}

pub struct RoomRepo { /* PgPool */ }
impl RoomRepo {
    pub fn new(db: PgPool) -> Self;
    pub async fn list(&self, area_id: Option<Uuid>, keyword: &str, include_inactive: bool, page: i64, page_size: i64) -> DbResult<Page<Room>>;
    pub async fn get(&self, id: Uuid) -> DbResult<Option<Room>>;
    pub async fn create(&self, input: &NewRoom) -> DbResult<Room>;
    pub async fn update(&self, id: Uuid, input: &UpdateRoom) -> DbResult<Option<Room>>;
    pub async fn delete(&self, id: Uuid) -> DbResult<bool>;
    pub async fn find_by_device_slot(&self, slot: &str) -> DbResult<Option<Room>>;
}

pub struct ScheduleRepo { /* PgPool */ }
impl ScheduleRepo {
    pub fn new(db: PgPool) -> Self;
    pub async fn list(&self, room_id: Option<Uuid>, page: i64, page_size: i64) -> DbResult<Page<Schedule>>;
    pub async fn get(&self, id: Uuid) -> DbResult<Option<Schedule>>;
    pub async fn create(&self, input: &NewSchedule) -> DbResult<Schedule>;
    pub async fn update(&self, id: Uuid, input: &UpdateSchedule) -> DbResult<Option<Schedule>>;
    pub async fn delete(&self, id: Uuid) -> DbResult<bool>;
    pub async fn weekly_slots(&self, schedule_id: Uuid) -> DbResult<Vec<WeeklySlot>>;
    /// 全量替换（事务内 DELETE + INSERT）；schedule 不存在返回 None。
    pub async fn replace_weekly_slots(&self, schedule_id: Uuid, slots: &[ValidatedWeeklySlot]) -> DbResult<Option<Vec<WeeklySlot>>>;
    pub async fn exceptions(&self, schedule_id: Uuid, from: Option<NaiveDate>, to: Option<NaiveDate>) -> DbResult<Vec<ScheduleException>>;
    /// upsert 单日覆盖，并全量替换该覆盖的 slots。
    pub async fn upsert_exception(&self, schedule_id: Uuid, exception_date: NaiveDate, kind: ScheduleExceptionKind, note: &str, slots: &[ValidatedExceptionSlot]) -> DbResult<Option<ScheduleException>>;
    pub async fn delete_exception(&self, schedule_id: Uuid, exception_date: NaiveDate) -> DbResult<bool>;
    /// 展开并重写该课表「未来」的排课实例（保留已过去的实例与事件引用），返回写入条数。
    pub async fn rebuild_occurrences(&self, schedule_id: Uuid, horizon_days: i64) -> DbResult<usize>;
    pub async fn rebuild_all_occurrences(&self, horizon_days: i64) -> DbResult<usize>;
    pub async fn occurrences(&self, schedule_id: Uuid, from: DateTime<Utc>, to: DateTime<Utc>) -> DbResult<Vec<Occurrence>>;
    pub async fn room_occurrences(&self, room_id: Uuid, from: DateTime<Utc>, to: DateTime<Utc>) -> DbResult<Vec<Occurrence>>;
    /// 事件引擎：start_at ∈ [from, to) 的全部实例
    pub async fn occurrences_starting(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> DbResult<Vec<Occurrence>>;
    /// 事件引擎：end_at ∈ [from, to) 的全部实例
    pub async fn occurrences_ending(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> DbResult<Vec<Occurrence>>;
    /// 事件引擎：与 [from, to) 有交集的实例（start_at < to AND end_at > from）
    pub async fn occurrences_overlapping(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> DbResult<Vec<Occurrence>>;
}

pub struct EventRuleRepo { /* PgPool */ }
impl EventRuleRepo {
    pub fn new(db: PgPool) -> Self;
    pub async fn list_by_room(&self, room_id: Uuid) -> DbResult<Vec<EventRule>>;
    pub async fn replace_for_room(&self, room_id: Uuid, rules: &[ValidatedEventRule]) -> DbResult<Option<Vec<EventRule>>>;
    /// 引擎用：所有启用规则 + 房间快照。
    pub async fn enabled_rules(&self) -> DbResult<Vec<EnabledRule>>;
}
pub struct EnabledRule {
    pub rule: EventRule,
    pub room_code: String,
    pub room_name: String,
    pub device_slot: Option<String>,
    pub webhook_url: Option<String>,
    pub timezone: String,
}

pub struct ScheduleEventRepo { /* PgPool */ }
impl ScheduleEventRepo {
    pub fn new(db: PgPool) -> Self;
    /// 幂等插入事件；已存在返回 None。插入成功时同事务写入目标投递行。
    pub async fn insert_if_absent(&self, event: &NewScheduleEvent, targets: &[DeliveryTarget]) -> DbResult<Option<ScheduleEvent>>;
    pub async fn list(&self, filter: &EventFilter, page: i64, page_size: i64) -> DbResult<Page<ScheduleEvent>>;
    pub async fn get(&self, id: Uuid) -> DbResult<Option<ScheduleEvent>>;
    pub async fn deliveries_for(&self, event_id: Uuid) -> DbResult<Vec<ScheduleEventDelivery>>;
    pub async fn due_deliveries(&self, limit: i64) -> DbResult<Vec<DeliveryJob>>;
    pub async fn mark_delivered(&self, delivery_id: i64) -> DbResult<()>;
    pub async fn mark_skipped(&self, delivery_id: i64, reason: &str) -> DbResult<()>;
    pub async fn mark_retry(&self, delivery_id: i64, attempts: i32, next_attempt_at: DateTime<Utc>, error: &str) -> DbResult<()>;
    pub async fn mark_failed(&self, delivery_id: i64, attempts: i32, error: &str) -> DbResult<()>;
    /// 重置某事件的全部投递为 pending（重放）。
    pub async fn replay_event(&self, event_id: Uuid) -> DbResult<u64>;
    pub async fn retry_delivery(&self, event_id: Uuid, delivery_id: i64) -> DbResult<bool>;
    /// 向房间设备槽位投递下行消息：返回 Some(serial) / None（无存活设备）。
    /// 逻辑与 qe-device-pki::messaging::downlink_publish_to_slot 保持一致：
    /// 选 status ∈ {issued, active} 的最新证书；未 ACK 队列 > 1000 时返回 Conflict。
    pub async fn enqueue_downlink_for_slot(&self, slot: &str, kind: &str, payload: &str) -> DbResult<Option<String>>;
}
```

实现要点：

- `rebuild_occurrences`：在一个事务里 `SELECT ... FOR UPDATE` 课表 → 读周模板与单日覆盖 →
  `expand_occurrences`（from = max(today_local, start_date)，to = min(end_date, today_local + horizon)）→
  `DELETE FROM schedule_occurrences WHERE schedule_id=$1 AND start_at > now()` → 批量 INSERT。
- `replace_weekly_slots` 之后调用方（API）应再调用 `rebuild_occurrences`；repo 不隐式做，方便组合。
- `insert_if_absent`：`INSERT ... ON CONFLICT (event_key) DO NOTHING RETURNING *`，命中则同一事务
  `INSERT INTO schedule_event_deliveries (event_id, target) SELECT ... ON CONFLICT DO NOTHING`。
- `due_deliveries`：`status='pending' AND next_attempt_at <= now()` JOIN 事件、房间，`ORDER BY next_attempt_at LIMIT`。

## 5. HTTP API（`qe-api`）

所有接口挂在 `/api/v1/admin` 下，需要 Bearer 认证 + 对应权限。响应沿用统一错误格式。
新增文件：`crates/api/src/api/admin/{areas,rooms,schedules,schedule_events}.rs`，
`admin/mod.rs` 注册路由，`state.rs` 增加 repo。

分页参数 `page` / `page_size` 复用 `validation::page_params`。

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| GET | `/areas?page=&page_size=&keyword=&include_inactive=` | area:read | 区域分页 |
| POST | `/areas` | area:manage | 创建区域；body `{code,name,timezone?,description?,parent_id?,is_active?,metadata?}` |
| GET | `/areas/{id}` | area:read | 区域详情 |
| PATCH | `/areas/{id}` | area:manage | 局部更新；`parent_id` / `metadata` 可传 null |
| DELETE | `/areas/{id}` | area:manage | 有房间时 409 |
| GET | `/rooms?area_id=&page=&page_size=&keyword=&include_inactive=` | room:read | 房间分页 |
| POST | `/rooms` | room:manage | body `{area_id,code,name,capacity?,device_slot?,webhook_url?,is_active?,metadata?}`；`timezone` 只读继承 |
| GET | `/rooms/{id}` | room:read | 房间详情 |
| PATCH | `/rooms/{id}` | room:manage | 局部更新；`device_slot` / `webhook_url` 可传 null 清空 |
| DELETE | `/rooms/{id}` | room:manage | 级联删除课表与规则 |
| GET | `/schedules?room_id=&page=&page_size=` | schedule:read | 课表分页 |
| POST | `/schedules` | schedule:manage | body `{room_id,name,start_date,end_date,is_active?}`；`timezone` 只读继承 |
| GET | `/schedules/{id}` | schedule:read | 课表详情 |
| PATCH | `/schedules/{id}` | schedule:manage | 局部更新；改日期后自动重建未来实例 |
| DELETE | `/schedules/{id}` | schedule:manage | 删除并级联实例 |
| GET | `/schedules/{id}/weekly-slots` | schedule:read | 周模板 |
| PUT | `/schedules/{id}/weekly-slots` | schedule:manage | 全量替换；body `{slots:[NewWeeklySlot...]}`；成功后重建实例 |
| GET | `/schedules/{id}/exceptions?from=&to=` | schedule:read | 单日覆盖列表 |
| PUT | `/schedules/{id}/exceptions/{date}` | schedule:manage | upsert；body `{kind:"cancel"\|"replace",note?,slots?:[...]}`；成功后重建 |
| DELETE | `/schedules/{id}/exceptions/{date}` | schedule:manage | 删除覆盖并重建 |
| GET | `/schedules/{id}/occurrences?from=&to=` | schedule:read | 排课实例（默认今天起 7 天，跨度 ≤366 天） |
| GET | `/rooms/{id}/occurrences?from=&to=` | schedule:read | 房间维度排课实例 |
| POST | `/schedules/{id}/rebuild` | schedule:manage | 手动重建未来实例，返回 `{occurrences: n}` |
| GET | `/rooms/{id}/event-rules` | schedule:read | 触发规则 |
| PUT | `/rooms/{id}/event-rules` | schedule:manage | 全量替换；body `{rules:[NewEventRule...]}` |
| GET | `/schedule-events?room_id=&kind=&from=&to=&page=&page_size=` | schedule:event:read | 事件分页（`triggered_at` 倒序） |
| GET | `/schedule-events/{id}` | schedule:event:read | 事件 + 投递明细 `{event, deliveries}` |
| POST | `/schedule-events/{id}/replay` | schedule:event:manage | 全部投递重置为 pending，返回 `{reset: n}` |
| POST | `/schedule-events/{id}/deliveries/{delivery_id}/retry` | schedule:event:manage | 单条重试，204 |

请求 DTO（`Update*` 中可选字段语义：缺失=不改；对可空字段传 `null`=清空）：

```rust
CreateAreaRequest { code: String, name: String, #[serde(default)] timezone: Option<String>,
                    #[serde(default)] description: String, #[serde(default)] parent_id: Option<Uuid>,
                    #[serde(default)] is_active: Option<bool>, #[serde(default)] metadata: Option<Value> }
UpdateAreaRequest { #[serde(default)] code: Option<String>, ... parent_id: Option<Option<Uuid>>, ... }
CreateRoomRequest { area_id: Uuid, code: String, name: String, #[serde(default)] capacity: Option<i32>,
                    #[serde(default)] device_slot: Option<String>, #[serde(default)] webhook_url: Option<String>,
                    #[serde(default)] is_active: Option<bool>, #[serde(default)] metadata: Option<Value> }
UpdateRoomRequest { ..., device_slot: Option<Option<String>>, webhook_url: Option<Option<String>>, ... }
CreateScheduleRequest { room_id, name, start_date: NaiveDate, end_date: NaiveDate,
                        #[serde(default)] is_active: Option<bool> }
UpdateScheduleRequest { #[serde(default)] name/start_date/end_date/is_active }
ReplaceWeeklySlotsRequest { slots: Vec<NewWeeklySlot> }
UpsertExceptionRequest { kind: ScheduleExceptionKind, #[serde(default)] note: String,
                         #[serde(default)] slots: Vec<NewExceptionSlot> }
ReplaceEventRulesRequest { rules: Vec<NewEventRule> }
EventQuery { room_id: Option<Uuid>, kind: Option<String>, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>, page, page_size }
```

handler 行为：

- 写操作成功后 `audit::record_or_log`：`area.create/update/delete`、`room.create/update/delete`、
  `schedule.create/update/delete`、`schedule.set_weekly_slots`、`schedule.set_exception`、
  `schedule.delete_exception`、`room.set_event_rules`、`schedule_event.replay`、`schedule_event.retry`。
- `GET /schedule-events` 的 `kind` 对应 `event_kind`（即 trigger_kind 字符串）。
- 重建失败不应吞掉写操作结果：写操作在事务外重建，重建失败返回 500 并记日志（数据可手动 rebuild）。
  为减少问题，`PUT weekly-slots`/`PUT exception`/`PATCH schedule` 在写成功后调用 `rebuild_occurrences`，
  失败返回 `AppError::internal`。
- `DELETE area/room` 的冲突语义由 `DbError::Conflict -> 409` 自动映射。

## 6. 事件引擎（`crates/api/src/scheduler.rs`，`pub mod scheduler;` 于 `lib.rs`）

启动：`run()` 装配 `AppState` 后调用 `scheduler::spawn(state.clone())`；
`SCHEDULE_ENGINE_ENABLED=false` 时不启动（默认 true）。

```rust
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()>;
/// 可测试：以 now 为基准跑一次触发扫描，返回创建的事件数与投递数。
pub async fn tick_once(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<TickSummary>;
/// 可测试：处理一批到期的投递。
pub async fn deliver_once(state: &AppState) -> anyhow::Result<DeliverySummary>;
pub async fn rebuild_all(state: &AppState) -> anyhow::Result<usize>;
pub struct TickSummary { pub events_created: usize, pub deliveries_created: usize }
pub struct DeliverySummary { pub delivered: usize, pub retried: usize, pub failed: usize, pub skipped: usize }
```

### 6.1 触发语义

- 扫描窗口：`window_start = now - lookback`，`window_end = now`，
  `lookback = max(60s, tick_secs * 4)`（配置见 §7）。事件 `event_key` 唯一，重叠窗口天然幂等。
- 载入数据：
  - `EventRuleRepo::enabled_rules()`
  - `ScheduleRepo::occurrences_overlapping(window_start - 12h, window_end + max_offset(≤1440min))`
  - 在内存里按 room 分组，判断触发。
- 触发条件（`trigger_kind`）：
  - `occurrence_start`：`occurrence.start_at ∈ (window_start, window_end]`
  - `occurrence_end`：`occurrence.end_at ∈ (window_start, window_end]`
  - `occurrence_start_before`：`(occurrence.start_at - offset_minutes) ∈ (window_start, window_end]`
  - `room_occupied`：`occurrence.start_at` 命中窗口，且该房间在此刻**没有更早开始、仍未结束**的
    其他实例（双排课去重）
  - `room_idle`：`occurrence.end_at` 命中窗口，且该房间在此刻**没有其他仍在进行**的实例
  - `daily_time`：按房间所属区域的 IANA 时区计算当前本地日期/时间；本地 `at_time`
    落在窗口内时触发（窗口跨午夜时两天都判断）
- 事件键：
  - 实例类：`occ:{rule_id}:{occurrence_id}:{trigger_kind}`
  - 每日定时：`daily:{rule_id}:{local_date}`
- `triggered_at`：实际触发时刻（实例开始/结束/开始-offset；每日定时为本地 at_time 转 UTC）。
- 事件 `title`（中文）：`课程开始：{course}` / `课程结束：{course}` /
  `上课提醒（{offset} 分钟）：{course}` / `每日事件` / `房间占用：{course}` / `房间空闲`。
- `payload` 基础对象（在此之上合并 rule.config.payload）：
  ```json
  {
    "event_kind": "...", "room_id": "...", "room_code": "...", "room_name": "...",
    "schedule_id": "...", "occurrence_id": "...", "course_name": "...", "teacher": "...",
    "start_at": "RFC3339", "end_at": "RFC3339", "triggered_at": "RFC3339",
    "downlink_kind": "schedule.event"
  }
  ```
- 目标解析（`config`）：
  ```json
  {
    "targets": ["pubsub", "downlink", "webhook"],   // 默认全部三种
    "webhook_url": "https://...",                   // 默认 room.webhook_url
    "downlink_kind": "schedule.event",              // 默认 schedule.event
    "payload": { "任意": "附加字段" }                // 可选，浅合并进 payload
  }
  ```
  - `pubsub`：总是可投递。
  - `downlink`：房间 `device_slot` 为空时投递行直接 `skipped`（reason 记录）。
  - `webhook`：`config.webhook_url` 优先，其次 `room.webhook_url`；都没有则 `skipped`。
- 事件插入幂等：`insert_if_absent` 返回 None 时不创建投递。

### 6.2 投递语义

- 每个 tick（同一循环）调用 `deliver_once`；用 Redis 锁 `schedule:engine:deliver`（TTL 60s）避免多副本重复。
- 每次最多处理 50 条 `due_deliveries`。
- `pubsub`：`cache.publish(channel, payload_json)`，成功 → delivered。
- `downlink`：`payload.downlink_kind` + payload JSON 字符串调用
  `ScheduleEventRepo::enqueue_downlink_for_slot(room.device_slot, kind, payload)`；
  成功有设备 → delivered，并通过 `cache.publish(downlink_notify_channel, serial)` 唤醒长连接；
  无设备 → retry（设备可能稍后上线），超过最大次数 → failed。
- `webhook`：`reqwest` POST JSON，头 `X-QE-Event: {event_kind}`、`X-QE-Event-Id: {event_id}`、
  `X-QE-Delivery-Id: {delivery_id}`；2xx → delivered；非 2xx / 超时 / 网络错误 → retry。
- 重试退避：`next_attempt_at = now + min(5 * 2^attempts, 300) 秒`；
  `attempts >= SCHEDULE_WEBHOOK_MAX_ATTEMPTS`（默认 5）→ `failed`。
- 事件引擎自身错误：只记 `tracing::error!`，下一 tick 继续；启动时的重建失败不影响进程启动。

### 6.3 多副本与锁

- tick：Redis `SET schedule:engine:tick NX EX {max(60, tick*4)}`，拿不到锁直接跳过本轮。
- deliver：`schedule:engine:deliver`，TTL 60s。
- 重建：`schedule:engine:rebuild`，TTL 600s；启动时 + 每 `SCHEDULE_REBUILD_INTERVAL_SECONDS`（默认 3600）执行。
- 缓存新增方法（`qe-cache`）：
  ```rust
  pub async fn try_lock(&self, key: &str, ttl_secs: u64) -> CacheResult<bool>; // SET NX EX
  pub async fn unlock(&self, key: &str) -> CacheResult<()>;
  pub async fn publish(&self, channel: &str, payload: &str) -> CacheResult<()>;
  ```

## 7. 配置（`config.rs` + `.env.example`）

| 环境变量 | 默认 | 说明 |
| --- | --- | --- |
| `SCHEDULE_ENGINE_ENABLED` | `true` | 是否启动课表事件引擎 |
| `SCHEDULE_TICK_SECONDS` | `15` | 触发扫描/投递周期，5..=300 |
| `SCHEDULE_HORIZON_DAYS` | `30` | 排课实例物化天数，1..=366 |
| `SCHEDULE_REBUILD_INTERVAL_SECONDS` | `3600` | 全量重建周期，60..=86400 |
| `SCHEDULE_PUBSUB_CHANNEL` | `qe-schedule:events` | 事件 Redis 频道 |
| `SCHEDULE_DOWNLINK_NOTIFY_CHANNEL` | `qe-device-pki:downlink` | 设备下行通知频道（与 device-pki 一致） |
| `SCHEDULE_WEBHOOK_TIMEOUT_SECONDS` | `5` | 单次 webhook 超时，1..=60 |
| `SCHEDULE_WEBHOOK_MAX_ATTEMPTS` | `5` | 最大投递尝试次数，1..=20 |
| `SCHEDULE_DELIVERY_BATCH` | `50` | 每轮最多处理投递数，1..=500 |

字段加入 `Config` 结构体，命名：`schedule_engine_enabled`、`schedule_tick_secs`、
`schedule_horizon_days`、`schedule_rebuild_interval_secs`、`schedule_pubsub_channel`、
`schedule_downlink_notify_channel`、`schedule_webhook_timeout_secs`、
`schedule_webhook_max_attempts`、`schedule_delivery_batch`。

## 8. 文档

- `README.md` 增加「## 课表与事件」章节：数据模型、API 表、权限、触发语义、投递语义、配置。
- `openapi.rs` 增加 tags `Areas/Rooms/Schedules/ScheduleEvents` 与上述路径、schema。
- 本文档保持更新。

## 9. 验收标准

1. `cargo check --workspace --all-targets` 通过。
2. `cargo test --workspace` 通过（含领域层展开/校验单测、缓存与 db 单测）。
3. `make check`（fmt + clippy -D warnings + test）通过。
4. `scripts/test-integration.sh` 通过，且新增 `crates/api/tests/schedule.rs`：
   - 区域→房间→课表→周模板→单日覆盖全链路；
   - 展开结果正确（含 cancel / replace / 跨天边界 / 周几映射）；
   - 事件 tick 产生事件与投递行（start/end/before/daily/occupied/idle 至少覆盖 start、before、daily）；
   - webhook 投递到本地测试 HTTP 服务成功；downlink 在无设备时 skipped/retry；
   - 事件 replay/retry 生效。
5. 所有写接口有审计记录；错误格式统一；并发/多副本安全（Redis 锁）。

---

## 10. v2 调整：IANA 时区（本节覆盖前文冲突处）

> 结论：**具体时间一律存 UTC**（PostgreSQL `TIMESTAMPTZ`，API 返回 RFC3339 `Z`）；
> **按地区（区域）的 IANA 时区展示本地时间**；周模板的 `TIME` 是本地钟面时间，由所属区域时区解释。
> 本节是 v2 变更记录，相关字段/语义已同步到前文；若仍有遗漏以本节为准。

### 10.1 数据模型变化

- `areas.timezone TEXT NOT NULL DEFAULT 'Asia/Shanghai'`：IANA 时区名（如 `Asia/Shanghai`、`Europe/Berlin`）。
- 删除 `rooms.utc_offset_minutes`、`schedules.utc_offset_minutes`：房间/课表**继承所在区域时区**。
- `schedule_events.timezone TEXT NOT NULL`：事件生成时所属区域时区快照（用于展示与审计）。
- 其余时间列不变：`TIMESTAMPTZ` 存 UTC；`schedule_weekly_slots.start_time/end_time`、
  `schedule_exception_slots.start_time/end_time`、`schedule_event_rules.at_time` 仍是本地 `TIME`。
- 迁移 `0009_schedule.sql` 仍可直接修改（尚未提交，仅本地 `qestudio_test` 应用过；会重置测试库）。

### 10.2 领域类型变化

```rust
pub const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";

pub struct Area { /* 其余不变 */ pub timezone: String }
pub struct NewArea { /* 其余不变 */ pub timezone: String }
pub struct UpdateArea { /* 其余不变 */ pub timezone: Option<String> }

pub struct Room { /* 其余不变；删除 utc_offset_minutes */ pub timezone: String } // 只读：继承自区域
pub struct NewRoom { /* 其余不变；删除 utc_offset_minutes */ }
pub struct UpdateRoom { /* 其余不变；删除 utc_offset_minutes */ }

pub struct Schedule { /* 其余不变；删除 utc_offset_minutes */ pub timezone: String } // 只读：继承自区域
pub struct NewSchedule { /* 其余不变；删除 utc_offset_minutes */ }
pub struct UpdateSchedule { /* 其余不变；删除 utc_offset_minutes */ }

pub struct Occurrence { /* 其余不变 */ pub timezone: String } // 只读：来自课表→房间→区域

pub struct ScheduleEvent { /* 其余不变 */ pub timezone: String }
pub struct NewScheduleEvent { /* 其余不变 */ pub timezone: String }

pub struct ExpansionSchedule {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub timezone: String, // IANA
}
```

新增领域辅助函数（`qe_domain::schedule`）：

```rust
/// 解析并规范化 IANA 时区名；返回 `Tz::name()`。
pub fn validate_timezone(raw: &str) -> DomainResult<String>;
/// 本地日期 + 本地时刻 → UTC；夏令时重复时段取较早的一个，跳变不存在时段返回 400。
pub fn local_to_utc(date: NaiveDate, time: NaiveTime, timezone: &str) -> DomainResult<DateTime<Utc>>;
pub fn local_date_of(instant: DateTime<Utc>, timezone: &str) -> DomainResult<NaiveDate>;
pub fn local_clock_of(instant: DateTime<Utc>, timezone: &str) -> DomainResult<NaiveTime>;
/// 格式化带偏移的本地时间，如 `2026-11-02T08:00:00+08:00`；时区非法时退化为 UTC。
pub fn local_rfc3339(instant: DateTime<Utc>, timezone: &str) -> String;
```

- 删除 `validate_utc_offset`、`DEFAULT_UTC_OFFSET_MINUTES`、`MIN/MAX_UTC_OFFSET_MINUTES`。
- `expand_occurrences` 内部改用 `local_to_utc`（DST 正确）；其余展开规则不变。

### 10.3 仓储变化

- `AreaRepo::create/update` 处理 `timezone`（创建缺省 `DEFAULT_TIMEZONE`，已在 API 层校验）。
- `RoomRow`、`ScheduleRow`、`OccurrenceRow` 通过 `JOIN rooms→areas` 带出**生效时区** `timezone`。
- `EventRuleRepo::enabled_rules()` 的 `EnabledRule.utc_offset_minutes` 改为 `timezone: String`（房间所属区域）。
- `ScheduleRepo::rebuild_occurrences`：读取课表的生效时区，`ExpansionSchedule { timezone }`。
- 新增 `ScheduleRepo::rebuild_area_occurrences(area_id: Uuid, horizon_days: i64) -> DbResult<usize>`：
  重建某区域下所有课表的未来实例（区域时区变更后调用）。
- `ScheduleEventRepo::insert_if_absent` 写入 `timezone`；查询返回 `ScheduleEvent.timezone`。

### 10.4 API 变化（`/api/v1/admin`）

- `Area` 响应包含 `timezone`；`POST/PATCH /areas` 接受可选 `timezone`（创建缺省 `Asia/Shanghai`，
  修改时区成功后调用 `rebuild_area_occurrences(area.id, schedule_horizon_days)`）。
- `Room`/`Schedule` 响应包含只读 `timezone`（继承区域）；请求体**删除** `utc_offset_minutes`。
- `Occurrence` 响应在原有字段外增加：
  ```json
  { "timezone": "Asia/Shanghai",
    "start_local": "2026-11-02T08:00:00+08:00",
    "end_local": "2026-11-02T09:40:00+08:00" }
  ```
  `start_at`/`end_at` 仍为 UTC。查询默认日期区间按课表所属区域时区计算“今天”。
- `ScheduleEvent` 响应增加只读 `timezone` 与 `triggered_at_local`（值同 payload 中字段）。
- 事件 payload 额外字段：`timezone`、`start_local`、`end_local`、`triggered_at_local`；
  `start_at`/`end_at`/`triggered_at` 保持 UTC。
- 周模板/单日覆盖的 `start_time`/`end_time` 仍返回本地 `"HH:MM"`；所属时区见 `Schedule.timezone`。

### 10.5 引擎变化

- `daily_time` 按 `EnabledRule.timezone` 解释 `at_time`；跨午夜仍两天都判断。
- 事件 payload 用 `local_rfc3339` 生成 `start_local`/`end_local`/`triggered_at_local`；
  `NewScheduleEvent.timezone` 取房间所属区域时区。
- 其余触发/投递/锁/重试语义不变。

### 10.6 依赖与测试

- `qe-domain/Cargo.toml` 新增 `chrono-tz = "0.10"`（仅领域层使用；db/api/scheduler 通过领域辅助函数）。
- 领域单测新增：`Asia/Shanghai` 转换、`America/New_York` 夏令时跳变（不存在 → 400；重复 → 取较早）、
  时区名校验、`local_rfc3339` 输出偏移。
- 集成测试更新：`NewArea` 带 `timezone`、`NewRoom/NewSchedule` 去掉偏移字段；断言
  `occurrence.timezone` 与 API 的 `start_local`/`end_local`；区域时区变更后实例重建。
- 重置本地 `qestudio_test` schema 后重新迁移验证（迁移文件已改动）。

---

## 11. v3：设备接入区域/房间（设备 PKI ↔ 房间列表）

> 目标：把 `device-pki` 管理的设备纳入区域/房间管理，使负责人可以在**房间列表里直接看到、添加、迁移设备**，
> 并直接从房间生成设备注册 token/链接。设备本身仍由 `device_enrollment` 管理；**绑定唯一事实来源是
> `rooms.device_slot` ↔ `device_enrollment.device_slot`**（同 slot 只允许一个非终态证书），
> 设备状态来自 `device_enrollment.status` 与 `device_session.state/updated_at`。

### 11.1 迁移 `migrations/0010_device_room.sql`

无表结构变更（`rooms.device_slot` 与部分唯一索引 `rooms_device_slot_key` 已存在）。只新增权限：

```sql
INSERT INTO permissions (code, name, description) VALUES
    ('device:read',   '查看设备', '查看设备列表、注册状态与房间绑定'),
    ('device:manage', '管理设备', '绑定/解绑设备到房间、生成设备注册 token')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r
JOIN permissions p ON p.code = 'device:read'
WHERE r.code IN ('viewer', 'editor')
ON CONFLICT DO NOTHING;
```

### 11.2 领域层：新增 `crates/domain/src/device.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus { Pending, Issued, Active, Rejected, Revoked }
impl DeviceStatus { pub const fn as_str(self) -> &'static str; }
impl std::str::FromStr for DeviceStatus { type Err = (); }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceOnlineState { Online, Offline }
impl DeviceOnlineState { pub const fn as_str(self) -> &'static str; }
impl std::str::FromStr for DeviceOnlineState { type Err = (); }

/// 房间上展示的设备摘要（`Room.device`）。None 表示房间未绑定设备槽位。
#[derive(Debug, Clone, Serialize)]
pub struct RoomDevice {
    pub device_slot: String,
    pub serial: Option<String>,                    // None = 已分配槽位但尚未注册
    pub status: Option<DeviceStatus>,
    pub online: Option<DeviceOnlineState>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub pending_token: bool,                       // 存在未使用且未过期的注册 token
}

/// 设备列表条目（`GET /admin/devices`）。
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub serial: String,
    pub device_slot: String,
    pub status: DeviceStatus,
    pub online: Option<DeviceOnlineState>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub downloaded: bool,
    pub client_info: Option<serde_json::Value>,
    pub issued_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub room_id: Option<Uuid>,
    pub room_code: Option<String>,
    pub room_name: Option<String>,
    pub area_id: Option<Uuid>,
    pub area_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceFilter {
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub status: Option<DeviceStatus>,
    pub assigned: Option<bool>, // true=已绑定房间，false=未绑定
    pub keyword: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapTokenKind { Auto, Approval }
impl BootstrapTokenKind { pub const fn as_str(self) -> &'static str; }
impl std::str::FromStr for BootstrapTokenKind { type Err = (); }

#[derive(Debug, Clone)]
pub struct IssuedBootstrapToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}
```

- `Room` 增加只读字段 `pub device: Option<RoomDevice>`（其余不变；`device_slot` 保留）。
- `crates/domain/src/lib.rs` 增加 `pub mod device;`。
- `crates/domain/src/rbac.rs` 增加 `DEVICE_READ = "device:read"`、`DEVICE_MANAGE = "device:manage"` 并加入 `ALL`。
- 校验（放 `validation.rs`）：
  ```rust
  pub fn normalize_device_slot(raw: &str) -> DomainResult<String>;
  // 去空白；1..=128；只允许 ascii 字母数字 - _ .
  pub fn validate_token_ttl_secs(seconds: i64) -> DomainResult<i64>; // 60..=86400
  ```

### 11.3 仓储：新增 `crates/db/src/device.rs`（`pub mod device;`）

```rust
pub struct DeviceRepo { /* PgPool */ }
impl DeviceRepo {
    pub fn new(db: PgPool) -> Self;
    pub async fn list(&self, filter: &DeviceFilter, page: i64, page_size: i64) -> DbResult<Page<DeviceInfo>>;
    pub async fn get(&self, serial: &str) -> DbResult<Option<DeviceInfo>>;
    /// 绑定/迁移/解绑。Some(slot)：若该 slot 已绑定到其它房间则自动迁移（事务内先清空旧房间）；
    /// None：清空本房间绑定。返回更新后的房间；房间不存在返回 None。
    pub async fn bind_to_room(&self, room_id: Uuid, device_slot: Option<&str>) -> DbResult<Option<Room>>;
    /// 确保房间有 device_slot：已有则返回；否则用房间 code 生成（冲突时追加 8 位 id 片段）。
    pub async fn ensure_room_slot(&self, room_id: Uuid) -> DbResult<Option<String>>;
    /// 生成一次性 bootstrap token（库中只存 SHA-256）。TTL 由调用方校验。
    pub async fn issue_bootstrap_token(&self, slot: &str, kind: BootstrapTokenKind, created_by: &str, ttl_secs: i64) -> DbResult<IssuedBootstrapToken>;
}
```

- 依赖：`qe-db` 增加 `rand = "0.8"`、`sha2 = "0.10"`、`hex = "0.4"`（生成 32 字节随机 token，`hex` 明文，`sha2` 摘要存 `device_bootstrap_token.token_hash`）。
- `Room` 查询（list/get/create/update）在现有 `ROOM_COLUMNS` 上补充：
  ```sql
  -- 取该 slot 最近的注册记录（live 状态优先）
  LEFT JOIN LATERAL (
      SELECT de.serial, de.status, de.client_info, de.issued_at
      FROM device_enrollment de
      WHERE de.device_slot = r.device_slot
      ORDER BY (de.status IN ('pending','issued','active')) DESC,
               de.issued_at DESC NULLS LAST, de.serial
      LIMIT 1
  ) de ON TRUE
  LEFT JOIN device_session ds ON ds.serial = de.serial
  ```
  选择列追加：`de.serial AS device_serial, de.status AS device_status, ds.state AS device_online,
  ds.updated_at AS device_last_seen_at,
  EXISTS (SELECT 1 FROM device_bootstrap_token t
          WHERE t.device_slot = r.device_slot AND t.used_at IS NULL AND t.expires_at > now())
      AS device_pending_token`
  映射：`room.device_slot.is_some()` 时构造 `Some(RoomDevice { ... })`。
- `DeviceRepo::list` 查询（分页 + count）：`device_enrollment de LEFT JOIN rooms r ON r.device_slot = de.device_slot
  LEFT JOIN areas a ON a.id = r.area_id LEFT JOIN device_session ds ON ds.serial = de.serial`，
  过滤 `area_id/room_id/status/assigned/keyword(serial|device_slot|room.code|room.name ILIKE)`，按 `issued_at DESC NULLS LAST` 排序。
- `issue_bootstrap_token` 插入 `device_bootstrap_token(token_hash, kind, device_slot, created_by, expires_at)`，
  明文 token 只返回一次；不写审计（审计在 API 层）。
- `ensure_room_slot`：读房间 code + device_slot；生成候选 `room.code`，若已被其它房间占用则
  `format!("{code}-{short_id}")`（截断到 128）；`UPDATE rooms SET device_slot = ...`。

### 11.4 API：新增 `crates/api/src/api/admin/devices.rs`

所有接口需 Bearer + 权限；复用统一错误格式与审计。`AppState` 增加 `pub devices: DeviceRepo`。

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| GET | `/devices?area_id=&room_id=&status=&assigned=&keyword=&page=&page_size=` | `device:read` | 设备分页（含房间/区域、在线状态） |
| GET | `/devices/{serial}` | `device:read` | 设备详情；不存在 404 |
| GET | `/rooms/{id}/device` | `device:read` | 房间设备摘要；未绑定返回 `null` |
| PUT | `/rooms/{id}/device` | `device:manage` | body `{"device_slot": "node-101"}` 绑定/迁移；`{"device_slot": null}` 解绑；房间不存在 404 |
| POST | `/rooms/{id}/device-token` | `device:manage` | body `{"kind":"auto"\|"approval","ttl_secs":900}`；确保房间有槽位后签发 token |

- `Room` 响应自动带 `device: RoomDevice | null`（列表/详情/创建/更新都生效）。
- `POST /rooms/{id}/device-token` 响应：
  ```json
  { "token": "…64 hex…", "kind": "auto", "device_slot": "…", "expires_at": "…", "link": "https://<enroll>/enroll?token=…&slot=…" }
  ```
  链接使用新增配置 `PUBLIC_ENROLL_ENDPOINT`（默认 `https://localhost:4440`）。
- 审计动作：`room.bind_device`、`room.unbind_device`、`room.issue_device_token`。
- `robots` 无；`openapi.rs` 增加 `Devices` tag 与上述路径/schema；`README` 增加「设备与区域/房间」小节。
- 新增配置（`config.rs`）：`public_enroll_endpoint: String`（`PUBLIC_ENROLL_ENDPOINT`，默认 `https://localhost:4440`，
  去掉末尾 `/`）；`.env.example` 同步。
- 权限说明：`device:read` / `device:manage`；`admin` 自动全量，`viewer`/`editor` 只读。

### 11.5 行为与边界

- **添加设备流程**：房间 → `POST /rooms/{id}/device-token`（自动分配槽位并签发 token）→ 设备用链接注册 →
  房间 `device` 从 `pending_token=true, serial=null` 变为对应注册状态；无需额外操作。
- **迁移设备**：`PUT /rooms/{new}/device` 绑定同一槽位时，会自动从旧房间解绑（事务内），审计记录新房间。
- **解绑**：`PUT /rooms/{id}/device` 传 `null`；设备注册记录不删除，仍可在 `/devices` 看到但 `room_id=null`（未绑定）。
- **槽位唯一**：沿用 `rooms_device_slot_key` 部分唯一索引；并发绑定冲突返回 409。
- 设备状态/在线状态是设备侧事实，主 API 只读；`device-pki` 的 `/admin/*`（ADMIN_TOKEN）保持不变。

### 11.6 验收

- `cargo test -p qe-domain` / `qe-db` / `qe-api --lib`、`make check` 通过。
- 集成测试（`crates/api/tests/schedule.rs` 或新增 `device_room.rs`）覆盖：
  - 房间签发 token（`device_bootstrap_token` 落库，slot 自动分配）；
  - 直接插入 `device_enrollment` + `device_session` 后房间列表/详情返回设备摘要，`/devices` 可按区域/房间过滤；
  - 设备迁移/解绑、viewer 403、级联清理。
