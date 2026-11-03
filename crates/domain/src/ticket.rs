//! 工单（Ticket / Work Order）领域模型：枚举、状态机、优先级 SLA 与查询/建单输入。
//!
//! 契约见 `docs/design/ticket-implementation-contract.md` §1/§3。数据库行模型在
//! `qe-db`，本模块只保留纯业务规则（不依赖 SQLx/HTTP/Redis）：
//!
//! - 枚举与字符串互转（非法值 → `DomainError::BadRequest`，API 映射 400）；
//! - 状态机纯函数 `can_transition` / `validate_transition`（非法流转 → 409）；
//! - 优先级默认时效 `default_due_at`：紧急 1h / 高 4h / 普通 24h / 低 72h。

use std::{collections::BTreeMap, fmt, str::FromStr};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{DomainError, DomainResult};

/// 工单状态。`open` 待处理 / `in_progress` 处理中 / `resolved` 已解决 /
/// `closed` 已关闭 / `cancelled` 已取消。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    Open,
    InProgress,
    Resolved,
    Closed,
    Cancelled,
}

impl TicketStatus {
    pub const ALL: [Self; 5] = [
        Self::Open,
        Self::InProgress,
        Self::Resolved,
        Self::Closed,
        Self::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Resolved => "resolved",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 未结束状态：只有这些状态参与 `dedupe_key` 部分唯一索引与“活跃工单”统计。
    pub fn is_active(self) -> bool {
        matches!(self, Self::Open | Self::InProgress)
    }

    /// 已结束状态：不再允许修改内容/转派。
    pub fn is_finished(self) -> bool {
        !self.is_active()
    }

    /// 中文文案（事件/审计 detail 里用，避免前端重复维护）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "待处理",
            Self::InProgress => "处理中",
            Self::Resolved => "已解决",
            Self::Closed => "已关闭",
            Self::Cancelled => "已取消",
        }
    }
}

impl fmt::Display for TicketStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TicketStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "open" => Ok(Self::Open),
            "in_progress" => Ok(Self::InProgress),
            "resolved" => Ok(Self::Resolved),
            "closed" => Ok(Self::Closed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(DomainError::bad_request(format!("非法的工单状态：{other}"))),
        }
    }
}

/// 工单优先级。`urgent` 紧急 / `high` 高 / `normal` 普通 / `low` 低。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketPriority {
    Urgent,
    High,
    Normal,
    Low,
}

impl TicketPriority {
    pub const ALL: [Self; 4] = [Self::Urgent, Self::High, Self::Normal, Self::Low];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Urgent => "urgent",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Urgent => "紧急",
            Self::High => "高",
            Self::Normal => "普通",
            Self::Low => "低",
        }
    }

    /// 默认时效（SLA）：紧急 1h / 高 4h / 普通 24h / 低 72h。
    pub fn sla(self) -> Duration {
        match self {
            Self::Urgent => Duration::hours(1),
            Self::High => Duration::hours(4),
            Self::Normal => Duration::hours(24),
            Self::Low => Duration::hours(72),
        }
    }
}

impl fmt::Display for TicketPriority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TicketPriority {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "urgent" => Ok(Self::Urgent),
            "high" => Ok(Self::High),
            "normal" => Ok(Self::Normal),
            "low" => Ok(Self::Low),
            other => Err(DomainError::bad_request(format!(
                "非法的工单优先级：{other}"
            ))),
        }
    }
}

/// 工单分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketCategory {
    MachineHealth,
    DeviceOffline,
    DeliveryFailed,
    DistributionFailed,
    Security,
    Other,
}

impl TicketCategory {
    pub const ALL: [Self; 6] = [
        Self::MachineHealth,
        Self::DeviceOffline,
        Self::DeliveryFailed,
        Self::DistributionFailed,
        Self::Security,
        Self::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MachineHealth => "machine_health",
            Self::DeviceOffline => "device_offline",
            Self::DeliveryFailed => "delivery_failed",
            Self::DistributionFailed => "distribution_failed",
            Self::Security => "security",
            Self::Other => "other",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MachineHealth => "机器健康",
            Self::DeviceOffline => "设备离线",
            Self::DeliveryFailed => "推送失败",
            Self::DistributionFailed => "分发失败",
            Self::Security => "安全",
            Self::Other => "其他",
        }
    }
}

impl fmt::Display for TicketCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TicketCategory {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "machine_health" => Ok(Self::MachineHealth),
            "device_offline" => Ok(Self::DeviceOffline),
            "delivery_failed" => Ok(Self::DeliveryFailed),
            "distribution_failed" => Ok(Self::DistributionFailed),
            "security" => Ok(Self::Security),
            "other" => Ok(Self::Other),
            other => Err(DomainError::bad_request(format!("非法的工单分类：{other}"))),
        }
    }
}

/// 工单来源：自动规则 / 人工上报。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketSource {
    Auto,
    Manual,
}

impl TicketSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动",
            Self::Manual => "人工",
        }
    }
}

impl fmt::Display for TicketSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TicketSource {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "auto" => Ok(Self::Auto),
            "manual" => Ok(Self::Manual),
            other => Err(DomainError::bad_request(format!("非法的工单来源：{other}"))),
        }
    }
}

/// 处理轨迹事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketEventKind {
    Created,
    Claimed,
    Assigned,
    StatusChanged,
    PriorityChanged,
    Commented,
    Occurred,
    /// 标题/描述/分类等内容修改（PATCH，0019 新增）。
    Updated,
    /// SLA 超时提醒已发送（0022 新增）。
    Reminded,
}

impl TicketEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Claimed => "claimed",
            Self::Assigned => "assigned",
            Self::StatusChanged => "status_changed",
            Self::PriorityChanged => "priority_changed",
            Self::Commented => "commented",
            Self::Occurred => "occurred",
            Self::Updated => "updated",
            Self::Reminded => "reminded",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Created => "创建",
            Self::Claimed => "认领",
            Self::Assigned => "转派",
            Self::StatusChanged => "状态变更",
            Self::PriorityChanged => "优先级变更",
            Self::Commented => "评论",
            Self::Occurred => "重复发生",
            Self::Updated => "内容更新",
            Self::Reminded => "超时提醒",
        }
    }
}

impl fmt::Display for TicketEventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TicketEventKind {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "created" => Ok(Self::Created),
            "claimed" => Ok(Self::Claimed),
            "assigned" => Ok(Self::Assigned),
            "status_changed" => Ok(Self::StatusChanged),
            "priority_changed" => Ok(Self::PriorityChanged),
            "commented" => Ok(Self::Commented),
            "occurred" => Ok(Self::Occurred),
            "updated" => Ok(Self::Updated),
            "reminded" => Ok(Self::Reminded),
            other => Err(DomainError::bad_request(format!(
                "非法的工单事件类型：{other}"
            ))),
        }
    }
}

/// 状态机：从 `from` 是否允许流转到 `to`。
///
/// | 动作 | 前置 | 后置 |
/// | --- | --- | --- |
/// | 认领/开始处理 | open | in_progress |
/// | 标记解决 | in_progress | resolved |
/// | 关闭 | resolved | closed |
/// | 重开 | resolved / closed | open |
/// | 取消 | open / in_progress | cancelled |
pub fn can_transition(from: TicketStatus, to: TicketStatus) -> bool {
    matches!(
        (from, to),
        (TicketStatus::Open, TicketStatus::InProgress)
            | (TicketStatus::Open, TicketStatus::Cancelled)
            | (TicketStatus::InProgress, TicketStatus::Resolved)
            | (TicketStatus::InProgress, TicketStatus::Cancelled)
            | (TicketStatus::Resolved, TicketStatus::Closed)
            | (TicketStatus::Resolved, TicketStatus::Open)
            | (TicketStatus::Closed, TicketStatus::Open)
    )
}

/// 校验一次状态流转，含“解决必须带 resolution”的附加规则。
///
/// 非法流转返回 `DomainError::Conflict`（API 映射 409）；缺少 resolution 返回
/// `DomainError::BadRequest`（API 映射 400）。
pub fn validate_transition(
    from: TicketStatus,
    to: TicketStatus,
    resolution: Option<&str>,
) -> DomainResult<()> {
    if !can_transition(from, to) {
        return Err(DomainError::conflict(format!(
            "工单状态不允许从「{}」变更为「{}」",
            from.label(),
            to.label()
        )));
    }
    if to == TicketStatus::Resolved
        && resolution
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Err(DomainError::bad_request("标记解决时必须填写解决说明"));
    }
    Ok(())
}

/// 按优先级计算默认时效截止时间（SLA）。
pub fn default_due_at(priority: TicketPriority, now: DateTime<Utc>) -> DateTime<Utc> {
    now + priority.sla()
}

/// 工单实体（API 响应字段与数据库列一一对应，另带处理人/上报人显示名）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: Uuid,
    pub code: String,
    pub title: String,
    pub description: String,
    pub status: TicketStatus,
    pub priority: TicketPriority,
    pub category: TicketCategory,
    pub source: TicketSource,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub serial: Option<String>,
    pub device_slot: Option<String>,
    pub reporter_id: Option<Uuid>,
    /// 上报人显示名（`users.display_name`；自动单为 `None`）。
    pub reporter_name: Option<String>,
    pub assignee_id: Option<Uuid>,
    /// 处理人显示名（`users.display_name`；未认领为 `None`）。
    pub assignee_name: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
    /// 根因分类（resolve/close 时写入；NULL = 未填写）。
    pub root_cause: Option<String>,
    pub occurrence_count: i64,
    pub last_seen_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 处理轨迹事件。系统事件（`actor_id = NULL`）的 `actor_name` 为 `"系统"`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketEvent {
    pub id: i64,
    pub ticket_id: Uuid,
    pub kind: TicketEventKind,
    pub actor_id: Option<Uuid>,
    /// 操作人显示名；系统事件固定为 `"系统"`。
    pub actor_name: String,
    pub from_status: Option<TicketStatus>,
    pub to_status: Option<TicketStatus>,
    pub note: String,
    pub payload: serde_json::Value,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 关联上下文：房间/设备/机器最新健康与在线摘要（全部可空）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TicketContext {
    pub room_id: Option<Uuid>,
    pub room_name: Option<String>,
    pub room_code: Option<String>,
    pub area_id: Option<Uuid>,
    pub area_name: Option<String>,
    pub serial: Option<String>,
    pub device_slot: Option<String>,
    /// 该 serial 最新 `machine_snapshots.health_status`。
    pub health_status: Option<String>,
    pub health_score: Option<i32>,
    /// 该 serial 最新快照的采集时间。
    pub collected_at: Option<DateTime<Utc>>,
    /// `true` = device-pki 会话在线或最近 5 分钟内有快照；无法判断时 `null`。
    pub online: Option<bool>,
}

/// 工单详情：工单 + 时间线 + 关联上下文。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketDetail {
    pub ticket: Ticket,
    pub events: Vec<TicketEvent>,
    pub context: TicketContext,
}

/// 轻量计数（列表页 KPI / 状态 tabs / 顶栏「我的待办」角标）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TicketSummary {
    /// 五种状态的全量计数（缺失状态补 0）。
    pub by_status: BTreeMap<String, i64>,
    pub unassigned: i64,
    pub overdue: i64,
    pub mine_active: i64,
}

/// 可转派人员（拥有 `ticket:handle` 的启用用户）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketAssignee {
    pub id: Uuid,
    pub display_name: String,
    pub roles: Vec<String>,
}

/// 历史分析聚合（`GET /admin/tickets/stats`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketStats {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub totals: TicketStatsTotals,
    pub by_day: Vec<TicketDayStat>,
    pub by_category: Vec<TicketCategoryStat>,
    pub by_priority: Vec<TicketPriorityStat>,
    pub top_rooms: Vec<TicketTopRoom>,
    pub top_devices: Vec<TicketTopDevice>,
    pub by_assignee: Vec<TicketAssigneeStat>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TicketStatsTotals {
    pub created: i64,
    pub resolved: i64,
    pub closed: i64,
    pub open_now: i64,
    pub overdue: i64,
    /// 区间内解决工单的平均解决时长（秒）；无数据为 `null`。
    pub avg_resolve_secs: Option<i64>,
    /// 中位数解决时长（秒），避免个别长单拉偏平均；无数据为 `null`。
    pub median_resolve_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketDayStat {
    pub date: String,
    pub created: i64,
    pub resolved: i64,
    pub closed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketCategoryStat {
    pub category: TicketCategory,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketPriorityStat {
    pub priority: TicketPriority,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketTopRoom {
    pub room_id: Uuid,
    pub room_name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketTopDevice {
    pub serial: String,
    pub device_slot: Option<String>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketAssigneeStat {
    pub assignee_id: Uuid,
    pub display_name: String,
    pub active: i64,
    pub resolved: i64,
    pub avg_resolve_secs: Option<i64>,
}

/// 列表过滤条件（由 API query 解析后传入仓储）。
#[derive(Debug, Clone, Default)]
pub struct TicketFilter {
    pub status: Option<TicketStatus>,
    pub category: Option<TicketCategory>,
    pub priority: Option<TicketPriority>,
    pub source: Option<TicketSource>,
    pub assignee_id: Option<Uuid>,
    /// 只看未分配工单（`assignee_id IS NULL`）；与 `assignee_id` 同时出现时忽略。
    pub unassigned: bool,
    /// `Some(user_id)` = 只看分配给该用户的工单（`mine=true`）。
    pub mine: Option<Uuid>,
    /// 只看未结束状态（`status IN ('open','in_progress')`）。
    pub active: bool,
    /// 只看超时未结束工单（`due_at < now()`）。
    pub overdue: bool,
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub serial: Option<String>,
    pub keyword: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// 数据范围：`Some(areas)` 时只匹配这些区域；`None` = 不限制（scope:all 或无授权行）。
    pub scoped_area_ids: Option<Vec<Uuid>>,
    pub page: i64,
    pub page_size: i64,
}

/// 建单输入（人工上报与自动规则共用）。
#[derive(Debug, Clone)]
pub struct NewTicket {
    pub title: String,
    pub description: String,
    pub priority: TicketPriority,
    pub category: TicketCategory,
    pub source: TicketSource,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    /// 自动单幂等键；人工单为 `None`。
    pub dedupe_key: Option<String>,
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub serial: Option<String>,
    pub device_slot: Option<String>,
    pub reporter_id: Option<Uuid>,
    pub metadata: serde_json::Value,
}

/// `POST /tickets/{id}/transition` 的输入。
#[derive(Debug, Clone)]
pub struct TransitionInput {
    pub to_status: TicketStatus,
    pub resolution: Option<String>,
    pub note: Option<String>,
    /// 根因分类（resolve/close 时写入；允许在 resolved→closed 时补充）。
    pub root_cause: Option<String>,
}

/// 单号生成：`T-YYYYMMDD-####`（与 `migrations/0018_tickets.sql` 的序列配合）。
///
/// 实际落库由 SQL 生成（保证同一事务内日期稳定），这里提供纯函数便于测试。
pub fn ticket_code(date: chrono::NaiveDate, sequence: i64) -> String {
    format!("T-{}-{:04}", date.format("%Y%m%d"), sequence)
}

// ----------------------------------------------------------------------
// P1 Wave 1：自动建单规则 / SLA 提醒 / SSE / CSV 导出
// ----------------------------------------------------------------------

/// 自动建单信号源（`ticket_rules.signal_type`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketSignalType {
    MachineHealth,
    DeviceOffline,
    ScheduleDelivery,
    PresenceDelivery,
    Transfer,
    ImageVerify,
}

impl TicketSignalType {
    pub const ALL: [Self; 6] = [
        Self::MachineHealth,
        Self::DeviceOffline,
        Self::ScheduleDelivery,
        Self::PresenceDelivery,
        Self::Transfer,
        Self::ImageVerify,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MachineHealth => "machine_health",
            Self::DeviceOffline => "device_offline",
            Self::ScheduleDelivery => "schedule_delivery",
            Self::PresenceDelivery => "presence_delivery",
            Self::Transfer => "transfer",
            Self::ImageVerify => "image_verify",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MachineHealth => "机器健康",
            Self::DeviceOffline => "设备离线",
            Self::ScheduleDelivery => "课表投递",
            Self::PresenceDelivery => "在线事件投递",
            Self::Transfer => "存储分发",
            Self::ImageVerify => "镜像验证",
        }
    }

    /// 该信号源支持的阈值键与范围（契约 §2）。
    pub fn threshold_ranges(self) -> &'static [(&'static str, i64, i64)] {
        match self {
            Self::MachineHealth => &[("consecutive", 1, 10)],
            Self::DeviceOffline => &[("offline_minutes", 1, 1440)],
            Self::ScheduleDelivery
            | Self::PresenceDelivery
            | Self::Transfer
            | Self::ImageVerify => &[("lookback_hours", 1, 168)],
        }
    }
}

impl fmt::Display for TicketSignalType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TicketSignalType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "machine_health" => Ok(Self::MachineHealth),
            "device_offline" => Ok(Self::DeviceOffline),
            "schedule_delivery" => Ok(Self::ScheduleDelivery),
            "presence_delivery" => Ok(Self::PresenceDelivery),
            "transfer" => Ok(Self::Transfer),
            "image_verify" => Ok(Self::ImageVerify),
            other => Err(DomainError::bad_request(format!(
                "非法的规则信号类型：{other}"
            ))),
        }
    }
}

/// 规则生效范围（global / area / room）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketScopeType {
    Global,
    Area,
    Room,
}

impl TicketScopeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Area => "area",
            Self::Room => "room",
        }
    }

    /// 范围越具体优先级越高（room > area > global）。
    pub fn rank(self) -> u8 {
        match self {
            Self::Global => 0,
            Self::Area => 1,
            Self::Room => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "全局",
            Self::Area => "区域",
            Self::Room => "房间",
        }
    }
}

impl fmt::Display for TicketScopeType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TicketScopeType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "global" => Ok(Self::Global),
            "area" => Ok(Self::Area),
            "room" => Ok(Self::Room),
            other => Err(DomainError::bad_request(format!(
                "非法的规则范围类型：{other}"
            ))),
        }
    }
}

/// 自动建单规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketRule {
    pub id: Uuid,
    pub name: String,
    pub signal_type: TicketSignalType,
    pub enabled: bool,
    pub priority: TicketPriority,
    pub scope_type: TicketScopeType,
    pub scope_id: Option<Uuid>,
    pub thresholds: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TicketRule {
    pub fn threshold_i64(&self, key: &str) -> Option<i64> {
        self.thresholds.get(key).and_then(Value::as_i64)
    }

    /// 信号是否落在本规则范围内。
    pub fn matches_scope(&self, area_id: Option<Uuid>, room_id: Option<Uuid>) -> bool {
        match self.scope_type {
            TicketScopeType::Global => true,
            TicketScopeType::Area => self.scope_id.is_some() && self.scope_id == area_id,
            TicketScopeType::Room => self.scope_id.is_some() && self.scope_id == room_id,
        }
    }

    /// 生成工单时使用的优先级（machine_health critical 强制 high，契约 §2）。
    pub fn effective_priority(&self, health_status: Option<&str>) -> TicketPriority {
        if self.signal_type == TicketSignalType::MachineHealth && health_status == Some("critical")
        {
            TicketPriority::High
        } else {
            self.priority
        }
    }
}

/// 建规则输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketRuleInput {
    pub name: String,
    pub signal_type: TicketSignalType,
    #[serde(default = "default_rule_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: TicketPriority,
    #[serde(default)]
    pub scope_type: TicketScopeType,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    #[serde(default)]
    pub thresholds: Value,
}

fn default_rule_enabled() -> bool {
    true
}

impl Default for TicketPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl Default for TicketScopeType {
    fn default() -> Self {
        Self::Global
    }
}

/// PATCH 规则的部分更新输入。
#[derive(Debug, Clone)]
pub struct TicketRulePatch {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<TicketPriority>,
    pub scope_type: Option<TicketScopeType>,
    /// 缺失 = 不修改；显式 `null` = 清空（scope_type=global 时使用）。
    pub scope_id: Option<Option<Uuid>>,
    pub thresholds: Option<Value>,
}

/// 规范化规则名称：去空白，1..=100 字符。
pub fn normalize_rule_name(raw: &str) -> DomainResult<String> {
    let name = raw.trim();
    let length = name.chars().count();
    if !(1..=100).contains(&length) {
        return Err(DomainError::bad_request("规则名称长度需为 1-100 个字符"));
    }
    Ok(name.to_owned())
}

/// 校验并规范化规则阈值：必须是 JSON 对象，键由 signal_type 决定且值在范围内。
pub fn validate_rule_thresholds(
    signal_type: TicketSignalType,
    thresholds: &Value,
) -> DomainResult<Value> {
    let object = thresholds
        .as_object()
        .ok_or_else(|| DomainError::bad_request("thresholds 必须是 JSON 对象"))?;
    let ranges = signal_type.threshold_ranges();
    let mut normalized = serde_json::Map::new();
    for (key, value) in object {
        let Some((_, minimum, maximum)) = ranges.iter().find(|(name, _, _)| *name == key.as_str())
        else {
            return Err(DomainError::bad_request(format!(
                "{} 规则不支持阈值 {}",
                signal_type.label(),
                key
            )));
        };
        let number = value
            .as_i64()
            .ok_or_else(|| DomainError::bad_request(format!("阈值 {key} 必须为整数")))?;
        if !(*minimum..=*maximum).contains(&number) {
            return Err(DomainError::bad_request(format!(
                "阈值 {key} 需在 {minimum}-{maximum} 之间"
            )));
        }
        normalized.insert(key.clone(), Value::from(number));
    }
    Ok(Value::Object(normalized))
}

/// 工单 SSE 帧数据：事件 + JOIN 出来的工单编号/当前状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketStreamEvent {
    pub id: i64,
    pub ticket_id: Uuid,
    pub ticket_code: String,
    pub kind: TicketEventKind,
    /// 工单当前状态。
    pub status: TicketStatus,
    pub actor_id: Option<Uuid>,
    pub actor_name: String,
    pub note: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

/// CSV 导出行（含区域/房间名称）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketExportRow {
    pub code: String,
    pub title: String,
    pub status: TicketStatus,
    pub priority: TicketPriority,
    pub category: TicketCategory,
    pub source: TicketSource,
    pub area_name: Option<String>,
    pub room_name: Option<String>,
    pub device_slot: Option<String>,
    pub serial: Option<String>,
    pub reporter_name: Option<String>,
    pub assignee_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
}

// ----------------------------------------------------------------------
// Wave 2：根因分类 / 区域数据范围 / 附件
// ----------------------------------------------------------------------

/// 允许的根因分类（契约 6 值）。
pub const ROOT_CAUSE_VALUES: [&str; 6] = [
    "hardware", "network", "software", "config", "usage", "other",
];

/// 根因中文文案。
pub fn root_cause_label(value: &str) -> &'static str {
    match value {
        "hardware" => "硬件",
        "network" => "网络",
        "software" => "软件",
        "config" => "配置",
        "usage" => "使用",
        _ => "其他",
    }
}

/// 校验/规范化根因：允许为空（`None`）；非法值 → 400。
pub fn validate_root_cause(raw: Option<&str>) -> DomainResult<Option<String>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) if ROOT_CAUSE_VALUES.contains(&value) => Ok(Some(value.to_owned())),
        Some(value) => Err(DomainError::bad_request(format!(
            "非法的根因分类：{value}（可选值：{}）",
            ROOT_CAUSE_VALUES.join(" / ")
        ))),
    }
}

/// 数据范围授权行（`GET /admin/ticket-scopes`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketScopeUser {
    pub user_id: Uuid,
    pub display_name: String,
    pub roles: Vec<String>,
    pub area_ids: Vec<Uuid>,
}

/// 工单附件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketAttachment {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub object_key: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
    pub uploaded_by: Option<Uuid>,
    pub uploaded_by_name: Option<String>,
    pub created_at: DateTime<Utc>,
    /// 下载预签名地址；未配置对象存储时为 `null`。
    pub download_url: Option<String>,
}

/// 附件大小上限：20 MiB。
pub const MAX_ATTACHMENT_BYTES: i64 = 20 * 1024 * 1024;
/// 附件文件名长度上限。
pub const MAX_ATTACHMENT_FILENAME_CHARS: usize = 255;

/// 允许的附件 content-type（契约）：image/*、pdf、text/plain、zip、gzip、octet-stream。
pub fn is_allowed_attachment_content_type(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    if value.starts_with("image/") {
        return true;
    }
    matches!(
        value.as_str(),
        "application/pdf"
            | "text/plain"
            | "application/zip"
            | "application/gzip"
            | "application/octet-stream"
    )
}

/// 校验附件元数据（上传前）。
pub fn validate_attachment_meta(
    filename: &str,
    content_type: &str,
    size: i64,
    sha256: &str,
) -> DomainResult<()> {
    if !(1..=MAX_ATTACHMENT_BYTES).contains(&size) {
        return Err(DomainError::bad_request(format!(
            "附件大小需在 1-{MAX_ATTACHMENT_BYTES} 字节之间"
        )));
    }
    if !is_allowed_attachment_content_type(content_type) {
        return Err(DomainError::bad_request(
            "不支持的附件类型（允许 image/*, application/pdf, text/plain, application/zip, application/gzip, application/octet-stream）",
        ));
    }
    let _ = sanitize_attachment_filename(filename)?;
    let digest = sha256.trim();
    if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(DomainError::bad_request(
            "sha256 必须为 64 位十六进制字符串",
        ));
    }
    Ok(())
}

/// 规范化附件文件名：仅保留安全字符（字母数字、`.`、`-`、`_`），其余替换为 `_`。
pub fn sanitize_attachment_filename(raw: &str) -> DomainResult<String> {
    let basename = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let length = basename.chars().count();
    if !(1..=MAX_ATTACHMENT_FILENAME_CHARS).contains(&length) {
        return Err(DomainError::bad_request(format!(
            "附件文件名长度需为 1-{MAX_ATTACHMENT_FILENAME_CHARS} 个字符"
        )));
    }
    let mut sanitized = String::with_capacity(basename.len());
    for character in basename.chars() {
        if character.is_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
            sanitized.push(character);
        } else if !character.is_control() {
            sanitized.push('_');
        }
    }
    let sanitized = sanitized.trim().to_owned();
    if sanitized.is_empty() {
        return Err(DomainError::bad_request("附件文件名不合法"));
    }
    Ok(sanitized)
}

// ----------------------------------------------------------------------
// Wave 3：巡检计划（inspection plans / runs）
// ----------------------------------------------------------------------

/// 巡检执行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionRunStatus {
    Running,
    Ok,
    Issues,
    Failed,
}

impl InspectionRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Issues => "issues",
            Self::Failed => "failed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "执行中",
            Self::Ok => "正常",
            Self::Issues => "有异常",
            Self::Failed => "失败",
        }
    }
}

impl fmt::Display for InspectionRunStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for InspectionRunStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "running" => Ok(Self::Running),
            "ok" => Ok(Self::Ok),
            "issues" => Ok(Self::Issues),
            "failed" => Ok(Self::Failed),
            other => Err(DomainError::bad_request(format!(
                "非法的巡检执行状态：{other}"
            ))),
        }
    }
}

/// 巡检计划。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionPlan {
    pub id: Uuid,
    pub name: String,
    pub scope_type: TicketScopeType,
    pub scope_id: Option<Uuid>,
    pub interval_hours: i64,
    pub enabled: bool,
    pub created_by: Option<Uuid>,
    pub created_by_name: Option<String>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 最近一次执行摘要（计划列表用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionLastRun {
    pub status: InspectionRunStatus,
    pub started_at: DateTime<Utc>,
    pub issue_machines: i64,
    pub ticket_id: Option<Uuid>,
}

/// 计划 + 最近一次执行（`GET /admin/inspection-plans`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionPlanView {
    #[serde(flatten)]
    pub plan: InspectionPlan,
    pub last_run: Option<InspectionLastRun>,
}

/// 建计划输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionPlanInput {
    pub name: String,
    #[serde(default)]
    pub scope_type: TicketScopeType,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    #[serde(default = "default_inspection_interval")]
    pub interval_hours: i64,
    #[serde(default = "default_rule_enabled")]
    pub enabled: bool,
}

fn default_inspection_interval() -> i64 {
    24
}

/// PATCH 计划输入。
#[derive(Debug, Clone)]
pub struct InspectionPlanPatch {
    pub name: Option<String>,
    pub scope_type: Option<TicketScopeType>,
    pub scope_id: Option<Option<Uuid>>,
    pub interval_hours: Option<i64>,
    pub enabled: Option<bool>,
}

/// 一次巡检执行记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionRun {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: InspectionRunStatus,
    pub checked_machines: i64,
    pub issue_machines: i64,
    pub ticket_id: Option<Uuid>,
    pub summary: Value,
    pub error: Option<String>,
}

/// scope 内一台机器的最新快照摘要（巡检 worker 使用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectionMachineSummary {
    pub serial: String,
    pub device_slot: String,
    pub room_id: Option<Uuid>,
    pub room_name: Option<String>,
    pub area_id: Option<Uuid>,
    pub health_status: Option<String>,
    pub health_score: Option<i32>,
    pub issues: Vec<Value>,
    pub collected_at: Option<DateTime<Utc>>,
}

/// 校验巡检计划名称：1..=100 字符。
pub fn normalize_inspection_name(raw: &str) -> DomainResult<String> {
    let name = raw.trim();
    let length = name.chars().count();
    if !(1..=100).contains(&length) {
        return Err(DomainError::bad_request(
            "巡检计划名称长度需为 1-100 个字符",
        ));
    }
    Ok(name.to_owned())
}

/// 校验巡检周期：1..=168 小时。
pub fn validate_inspection_interval(hours: i64) -> DomainResult<i64> {
    if !(1..=168).contains(&hours) {
        return Err(DomainError::bad_request("巡检周期需在 1-168 小时之间"));
    }
    Ok(hours)
}

/// 校验巡检范围：global 必须无 scope_id；area/room 必须提供。
pub fn validate_inspection_scope(
    scope_type: TicketScopeType,
    scope_id: Option<Uuid>,
) -> DomainResult<Option<Uuid>> {
    match scope_type {
        TicketScopeType::Global => Ok(None),
        TicketScopeType::Area | TicketScopeType::Room => {
            if scope_id.is_none() {
                return Err(DomainError::bad_request(
                    "区域/房间范围的巡检计划必须提供 scope_id",
                ));
            }
            Ok(scope_id)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_roundtrip_through_strings() {
        for status in TicketStatus::ALL {
            assert_eq!(status.as_str().parse::<TicketStatus>().unwrap(), status);
        }
        for priority in TicketPriority::ALL {
            assert_eq!(
                priority.as_str().parse::<TicketPriority>().unwrap(),
                priority
            );
        }
        for category in TicketCategory::ALL {
            assert_eq!(
                category.as_str().parse::<TicketCategory>().unwrap(),
                category
            );
        }
        for source in [TicketSource::Auto, TicketSource::Manual] {
            assert_eq!(source.as_str().parse::<TicketSource>().unwrap(), source);
        }
        for kind in [
            TicketEventKind::Created,
            TicketEventKind::Claimed,
            TicketEventKind::Assigned,
            TicketEventKind::StatusChanged,
            TicketEventKind::PriorityChanged,
            TicketEventKind::Commented,
            TicketEventKind::Occurred,
            TicketEventKind::Updated,
            TicketEventKind::Reminded,
        ] {
            assert_eq!(kind.as_str().parse::<TicketEventKind>().unwrap(), kind);
        }
    }

    #[test]
    fn invalid_enum_values_are_bad_requests() {
        assert!(matches!(
            "done".parse::<TicketStatus>(),
            Err(DomainError::BadRequest(_))
        ));
        assert!(matches!(
            "P0".parse::<TicketPriority>(),
            Err(DomainError::BadRequest(_))
        ));
        assert!(matches!(
            "unknown".parse::<TicketCategory>(),
            Err(DomainError::BadRequest(_))
        ));
    }

    #[test]
    fn state_machine_allows_expected_transitions() {
        use TicketStatus::*;
        for (from, to) in [
            (Open, InProgress),
            (Open, Cancelled),
            (InProgress, Resolved),
            (InProgress, Cancelled),
            (Resolved, Closed),
            (Resolved, Open),
            (Closed, Open),
        ] {
            assert!(can_transition(from, to), "{from} -> {to} 应允许");
        }
    }

    #[test]
    fn state_machine_rejects_illegal_transitions() {
        use TicketStatus::*;
        for (from, to) in [
            (Open, Resolved),
            (Open, Closed),
            (InProgress, Closed),
            (InProgress, Open),
            (Resolved, InProgress),
            (Closed, InProgress),
            (Cancelled, Open),
            (Cancelled, Resolved),
            (Open, Open),
        ] {
            assert!(!can_transition(from, to), "{from} -> {to} 不应允许");
        }
        assert!(matches!(
            validate_transition(Open, Closed, Some("done")),
            Err(DomainError::Conflict(_))
        ));
    }

    #[test]
    fn resolve_requires_resolution_text() {
        use TicketStatus::*;
        assert!(validate_transition(InProgress, Resolved, None).is_err());
        assert!(validate_transition(InProgress, Resolved, Some("   ")).is_err());
        assert!(validate_transition(InProgress, Resolved, Some("已更换网线")).is_ok());
        // 其它流转不要求 resolution。
        assert!(validate_transition(Resolved, Closed, None).is_ok());
        assert!(validate_transition(Resolved, Open, None).is_ok());
    }

    #[test]
    fn priority_sla_mapping() {
        use chrono::TimeZone;
        let now = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        assert_eq!(
            default_due_at(TicketPriority::Urgent, now),
            now + Duration::hours(1)
        );
        assert_eq!(
            default_due_at(TicketPriority::High, now),
            now + Duration::hours(4)
        );
        assert_eq!(
            default_due_at(TicketPriority::Normal, now),
            now + Duration::hours(24)
        );
        assert_eq!(
            default_due_at(TicketPriority::Low, now),
            now + Duration::hours(72)
        );
    }

    #[test]
    fn status_helpers_classify_active_and_finished() {
        assert!(TicketStatus::Open.is_active());
        assert!(TicketStatus::InProgress.is_active());
        assert!(TicketStatus::Resolved.is_finished());
        assert!(TicketStatus::Closed.is_finished());
        assert!(TicketStatus::Cancelled.is_finished());
    }

    #[test]
    fn ticket_code_is_human_readable() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert_eq!(ticket_code(date, 42), "T-20261101-0042");
    }

    #[test]
    fn p1_signal_and_scope_enums_roundtrip() {
        for signal in TicketSignalType::ALL {
            assert_eq!(signal.as_str().parse::<TicketSignalType>().unwrap(), signal);
        }
        for scope in [
            TicketScopeType::Global,
            TicketScopeType::Area,
            TicketScopeType::Room,
        ] {
            assert_eq!(scope.as_str().parse::<TicketScopeType>().unwrap(), scope);
        }
        assert!(TicketScopeType::Room.rank() > TicketScopeType::Area.rank());
        assert!(TicketScopeType::Area.rank() > TicketScopeType::Global.rank());
    }

    #[test]
    fn rule_thresholds_are_validated_by_signal_type() {
        let ok = validate_rule_thresholds(
            TicketSignalType::MachineHealth,
            &serde_json::json!({ "consecutive": 3 }),
        )
        .unwrap();
        assert_eq!(ok["consecutive"], 3);

        assert!(
            validate_rule_thresholds(
                TicketSignalType::MachineHealth,
                &serde_json::json!({ "consecutive": 0 })
            )
            .is_err()
        );
        assert!(
            validate_rule_thresholds(
                TicketSignalType::MachineHealth,
                &serde_json::json!({ "lookback_hours": 24 })
            )
            .is_err()
        );
        assert!(
            validate_rule_thresholds(
                TicketSignalType::Transfer,
                &serde_json::json!({ "lookback_hours": 200 })
            )
            .is_err()
        );
        assert!(
            validate_rule_thresholds(
                TicketSignalType::DeviceOffline,
                &serde_json::json!({ "offline_minutes": 10 })
            )
            .is_ok()
        );
        assert!(
            validate_rule_thresholds(TicketSignalType::ImageVerify, &serde_json::json!({})).is_ok()
        );
    }

    #[test]
    fn rule_scope_and_priority_resolution() {
        use chrono::TimeZone;
        let now = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        let base = TicketRule {
            id: Uuid::new_v4(),
            name: "规则".to_owned(),
            signal_type: TicketSignalType::MachineHealth,
            enabled: true,
            priority: TicketPriority::Normal,
            scope_type: TicketScopeType::Global,
            scope_id: None,
            thresholds: serde_json::json!({ "consecutive": 2 }),
            created_at: now,
            updated_at: now,
        };
        assert!(base.matches_scope(Some(Uuid::new_v4()), Some(Uuid::new_v4())));
        assert_eq!(base.threshold_i64("consecutive"), Some(2));
        assert_eq!(
            base.effective_priority(Some("critical")),
            TicketPriority::High
        );
        assert_eq!(
            base.effective_priority(Some("warning")),
            TicketPriority::Normal
        );

        let room_id = Uuid::new_v4();
        let room_rule = TicketRule {
            scope_type: TicketScopeType::Room,
            scope_id: Some(room_id),
            ..base.clone()
        };
        assert!(room_rule.matches_scope(None, Some(room_id)));
        assert!(!room_rule.matches_scope(None, Some(Uuid::new_v4())));
    }

    #[test]
    fn root_cause_is_validated() {
        assert_eq!(validate_root_cause(None).unwrap(), None);
        assert_eq!(validate_root_cause(Some("  ")).unwrap(), None);
        assert_eq!(
            validate_root_cause(Some("hardware")).unwrap().as_deref(),
            Some("hardware")
        );
        assert!(validate_root_cause(Some("bogus")).is_err());
        assert_eq!(root_cause_label("network"), "网络");
    }

    #[test]
    fn attachment_meta_is_validated() {
        let sha = "a".repeat(64);
        assert!(validate_attachment_meta("photo.png", "image/png", 1024, &sha).is_ok());
        assert!(validate_attachment_meta("a.pdf", "application/pdf", 1024, &sha).is_ok());
        assert!(validate_attachment_meta("a.exe", "application/x-msdownload", 1024, &sha).is_err());
        assert!(validate_attachment_meta("a.pdf", "application/pdf", 0, &sha).is_err());
        assert!(
            validate_attachment_meta("a.pdf", "application/pdf", MAX_ATTACHMENT_BYTES + 1, &sha)
                .is_err()
        );
        assert!(validate_attachment_meta("a.pdf", "application/pdf", 10, "abc").is_err());
    }

    #[test]
    fn attachment_filename_is_sanitized() {
        assert_eq!(
            sanitize_attachment_filename("../../etc/passwd").unwrap(),
            "passwd"
        );
        assert_eq!(
            sanitize_attachment_filename("报告 v1.pdf").unwrap(),
            "报告 v1.pdf"
        );
        assert!(sanitize_attachment_filename("   ").is_err());
        assert!(sanitize_attachment_filename(&"a".repeat(256)).is_err());
    }

    #[test]
    fn inspection_status_and_validation() {
        for status in [
            InspectionRunStatus::Running,
            InspectionRunStatus::Ok,
            InspectionRunStatus::Issues,
            InspectionRunStatus::Failed,
        ] {
            assert_eq!(
                status.as_str().parse::<InspectionRunStatus>().unwrap(),
                status
            );
        }
        assert_eq!(normalize_inspection_name(" 夜间巡检 ").unwrap(), "夜间巡检");
        assert!(normalize_inspection_name("").is_err());
        assert!(validate_inspection_interval(0).is_err());
        assert_eq!(validate_inspection_interval(24).unwrap(), 24);
        assert!(validate_inspection_scope(TicketScopeType::Area, None).is_err());
        let area = Uuid::new_v4();
        assert_eq!(
            validate_inspection_scope(TicketScopeType::Area, Some(area)).unwrap(),
            Some(area)
        );
        assert_eq!(
            validate_inspection_scope(TicketScopeType::Global, Some(area)).unwrap(),
            None
        );
    }

    #[test]
    fn status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&TicketStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(
            serde_json::from_str::<TicketPriority>("\"urgent\"").unwrap(),
            TicketPriority::Urgent
        );
    }
}
