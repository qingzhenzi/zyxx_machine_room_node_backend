//! 教师端镜像制作工作台（Image Studio v2）的领域模型与状态机。
//!
//! 契约见 `docs/image-studio.md` §2 / §3.2.1 以及附录 A（ISO 安装 / Windows）：
//! - [`BuildState`]：会话状态机（base：`preparing → creating → ready → finishing → making → done`；
//!   iso：`preparing → creating → installing → ejecting → ready → …`；任意步骤失败 `failed`，
//!   取消 `cancelled`，超时回收 `expired`）；
//! - [`BuildSession`]：`image_build_sessions` 一行（构建 VM 由服务端会话统一创建/回收）；
//! - [`RoomImageDefault`]：`room_image_defaults` 一行（房间默认底包 + 可选增量包）；
//! - [`session_steps`]：GET 会话响应里的 `steps[]`（契约 §3.2.1，教师端直接渲染）。
//!
//! 这里只放纯模型与纯函数，不依赖 SQLx/Axum：状态推进中的实际下发/查库由
//! `qe-db`（仓储）与 `qe-api`（handler）负责，本文件负责可单测的规则。

use std::collections::HashSet;
use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::Transfer;

/// 构建 VM 名前缀（契约 §1 / §2：`qe-build-<8hex>`）。
pub const BUILD_VM_PREFIX: &str = "qe-build-";

/// 非终态会话超过该小时数由后台 worker 回收（契约 §3.2：>2h）。
pub const SESSION_STALE_HOURS: i64 = 2;

/// `finishing` 阶段等待优雅关机的宽限期（秒）；超时自动 destroy（契约 §3.2）。
pub const SHUTDOWN_GRACE_SECS: i64 = 60;

/// 会话状态（`image_build_sessions.state`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildState {
    Preparing,
    Creating,
    /// ISO 模式：安装 VM 已创建，教师正在通过 VNC 安装系统（长驻交互态）。
    Installing,
    /// ISO 模式：已请求弹出 ISO（`vm.command eject`），等待结果。
    Ejecting,
    Ready,
    Finishing,
    Making,
    Done,
    Failed,
    Cancelled,
    Expired,
}

impl BuildState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Creating => "creating",
            Self::Installing => "installing",
            Self::Ejecting => "ejecting",
            Self::Ready => "ready",
            Self::Finishing => "finishing",
            Self::Making => "making",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    /// 终态：`done / failed / cancelled / expired`（不再被状态机推进）。
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Cancelled | Self::Expired
        )
    }

    /// 是否允许 `POST /finish`（契约 §3.2：仅 ready）。
    pub const fn can_finish(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// 是否允许 `POST /cancel`（任意非终态）。
    pub const fn can_cancel(self) -> bool {
        !self.is_terminal()
    }

    /// 是否仍占用「进行中的制作」（教师端恢复/active 列表口径）。
    pub const fn is_active(self) -> bool {
        !self.is_terminal()
    }
}

impl fmt::Display for BuildState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BuildState {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "preparing" => Ok(Self::Preparing),
            "creating" => Ok(Self::Creating),
            "installing" => Ok(Self::Installing),
            "ejecting" => Ok(Self::Ejecting),
            "ready" => Ok(Self::Ready),
            "finishing" => Ok(Self::Finishing),
            "making" => Ok(Self::Making),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "expired" => Ok(Self::Expired),
            _ => Err(()),
        }
    }
}

/// 会话来源（`image_build_sessions.source_mode`，迁移 0040）：
/// `base` = 已发布底包差分盘；`iso` = 已上传 ISO 安装（Windows/Linux）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    #[default]
    Base,
    Iso,
}

impl SourceMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Iso => "iso",
        }
    }

    /// ISO 模式默认磁盘总线：SATA（Windows 安装盘自带 AHCI 驱动，契约 A.4）。
    pub const fn default_disk_bus(self) -> DiskBus {
        match self {
            Self::Base => DiskBus::Virtio,
            Self::Iso => DiskBus::Sata,
        }
    }
}

impl fmt::Display for SourceMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SourceMode {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "base" => Ok(Self::Base),
            "iso" => Ok(Self::Iso),
            _ => Err(()),
        }
    }
}

/// 构建 VM 磁盘总线（`vm.command create` 的 `disk_bus`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiskBus {
    #[default]
    Virtio,
    Sata,
}

impl DiskBus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Virtio => "virtio",
            Self::Sata => "sata",
        }
    }
}

impl fmt::Display for DiskBus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DiskBus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "virtio" => Ok(Self::Virtio),
            "sata" => Ok(Self::Sata),
            _ => Err(()),
        }
    }
}

/// 构建 VM 网卡模式（`vm.command create` 的 `nic`）：`none` 离线安装（默认），
/// `user` / `default` 需要网络时使用。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NicMode {
    #[default]
    None,
    User,
    Default,
}

impl NicMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::User => "user",
            Self::Default => "default",
        }
    }
}

impl fmt::Display for NicMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for NicMode {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "user" => Ok(Self::User),
            "default" => Ok(Self::Default),
            _ => Err(()),
        }
    }
}

/// 校验 ISO 文件名（契约 A.3.1）：只允许文件名，拒绝路径分隔符与 `..`。
///
/// 返回 trim 后的文件名；长度 1..=128，不含控制字符。
pub fn validate_iso_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("iso_name 不能为空".to_owned());
    }
    if name.chars().count() > 128 {
        return Err("iso_name 不能超过 128 个字符".to_owned());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("iso_name 只能是 ISO 文件名（不能包含 / \\ 或 ..）".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err("iso_name 不能包含控制字符".to_owned());
    }
    Ok(name.to_owned())
}

/// 构建会话（`image_build_sessions` 一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub room_id: Uuid,
    pub machine_serial: String,
    pub build_vm_name: String,
    /// `base` / `iso`（迁移 0040）；iso 模式没有底包，`base_image_id=None`。
    pub source_mode: SourceMode,
    /// iso 模式：目标机器 ISO 目录下的文件名（服务端只存文件名，不存宿主路径）。
    pub iso_name: Option<String>,
    pub disk_bus: DiskBus,
    pub nic: NicMode,
    /// iso 模式：安装完成后是否已弹出 ISO（eject ok 后置 true，finish 前必须为 true）。
    pub media_ejected: bool,
    /// iso 模式 preparing 阶段 `vm.command iso.list` 的 request_id。
    pub iso_check_request_id: Option<Uuid>,
    /// iso 模式 ejecting 阶段 `vm.command eject` 的 request_id。
    pub eject_request_id: Option<Uuid>,
    /// base 模式必填；iso 模式为 `None`（迁移 0040 已 DROP NOT NULL）。
    pub base_image_id: Option<Uuid>,
    pub delta_image_id: Option<Uuid>,
    pub base_transfer_id: Option<Uuid>,
    pub name: String,
    pub version: String,
    pub platform: String,
    pub vcpus: i32,
    pub memory_mib: i32,
    pub disk_gb: i32,
    pub description: Option<String>,
    pub state: BuildState,
    pub error: Option<String>,
    pub create_request_id: Option<Uuid>,
    pub shutdown_request_id: Option<Uuid>,
    pub destroy_request_id: Option<Uuid>,
    pub make_request_id: Option<Uuid>,
    pub image_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl BuildSession {
    /// 会话是否属于该用户（或 admin 可见）。
    pub fn visible_to(&self, user_id: Uuid, is_admin: bool) -> bool {
        is_admin || self.user_id == user_id
    }
}

/// 新建会话的输入（id 由 API 层预生成：`build_vm_name` 由它派生）。
#[derive(Debug, Clone)]
pub struct NewBuildSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub room_id: Uuid,
    pub machine_serial: String,
    pub build_vm_name: String,
    /// `base` / `iso`（迁移 0040）；iso 模式没有底包，`base_image_id=None`。
    pub source_mode: SourceMode,
    /// iso 模式：目标机器 ISO 目录下的文件名（服务端只存文件名，不存宿主路径）。
    pub iso_name: Option<String>,
    pub disk_bus: DiskBus,
    pub nic: NicMode,
    /// iso 模式 preparing 阶段 `vm.command iso.list` 的 request_id。
    pub iso_check_request_id: Option<Uuid>,
    /// base 模式必填；iso 模式为 `None`（迁移 0040 已 DROP NOT NULL）。
    pub base_image_id: Option<Uuid>,
    pub delta_image_id: Option<Uuid>,
    pub base_transfer_id: Option<Uuid>,
    pub name: String,
    pub version: String,
    pub platform: String,
    pub vcpus: i32,
    pub memory_mib: i32,
    pub disk_gb: i32,
    pub description: Option<String>,
}

/// 房间默认底包（`room_image_defaults` 一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomImageDefault {
    pub room_id: Uuid,
    pub base_image_id: Uuid,
    pub delta_image_id: Option<Uuid>,
    pub updated_by: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

/// 由会话 id 派生构建 VM 名：`qe-build-<id 前 8 位十六进制>`（契约 §1 示例）。
pub fn build_vm_name(id: Uuid) -> String {
    let simple = id.simple().to_string();
    format!("{BUILD_VM_PREFIX}{}", &simple[..8])
}

/// 底包在 agent 存储池中的卷名：`<image name>-<version>`（契约 §3.2）。
pub fn base_volume_name(name: &str, version: &str) -> String {
    format!("{name}-{version}")
}

/// `preparing` 阶段根据 base transfer 状态得到的下一步（纯函数，便于单测）。
///
/// - `completed` → 该创建 VM 了（返回 `Ok(())`，调用方发 `vm.command create`）；
/// - `failed` / `expired` → 会话失败（带 error 文案）；
/// - `pending` / `running` → 继续等待（`None`）。
pub fn preparing_outcome(
    status: crate::storage::TransferStatus,
    error: Option<&str>,
) -> Option<Result<(), String>> {
    use crate::storage::TransferStatus;
    match status {
        TransferStatus::Completed => Some(Ok(())),
        TransferStatus::Failed => Some(Err(error.unwrap_or("基础镜像拉取失败").to_owned())),
        TransferStatus::Expired => Some(Err("基础镜像拉取已过期".to_owned())),
        TransferStatus::Pending | TransferStatus::Running => None,
    }
}

/// iso 模式 `preparing` 阶段根据 `vm.command iso.list` 结果判断 ISO 是否存在。
///
/// - `pending` → 继续等待（`None`）；
/// - `ok` 且 `data.isos[]` 里存在同名项 → `Some(Ok(()))`（可以创建安装 VM）；
/// - `ok` 但列表里没有该 ISO / `error` / `timeout` → `Some(Err(文案))`。
///
/// `data` 形状按契约 A.3.1：`{"isos":[{"name","size_bytes"}]}`。
pub fn iso_check_outcome(
    command_status: &str,
    data: Option<&serde_json::Value>,
    error: Option<&str>,
    iso_name: &str,
) -> Option<Result<(), String>> {
    match command_status.trim().to_ascii_lowercase().as_str() {
        "pending" => None,
        "ok" => {
            let found = data
                .and_then(|data| data.get("isos"))
                .and_then(serde_json::Value::as_array)
                .map(|isos| {
                    isos.iter()
                        .filter_map(|iso| iso.get("name").and_then(serde_json::Value::as_str))
                        .any(|name| name == iso_name)
                })
                .unwrap_or(false);
            Some(if found {
                Ok(())
            } else {
                Err(format!(
                    "目标机器上没有找到安装 ISO：{iso_name}（请确认已上传到 ISO 目录）"
                ))
            })
        }
        other => Some(Err(error
            .map(str::to_owned)
            .unwrap_or_else(|| format!("校验安装 ISO 失败（{other}）")))),
    }
}

/// `finishing` 阶段现在是否可以发 `image.make`（VM 已关机）。
///
/// `force_destroy_finished = true` 表示自动 destroy 已经回 ok（VM 也被强制关掉）。
pub fn can_make_image(shutdown_ok: bool, destroy_ok: bool) -> bool {
    shutdown_ok || destroy_ok
}

/// `finishing` 阶段是否已超过优雅关机宽限期（需要自动 destroy）。
pub fn shutdown_grace_expired(entered_finishing_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - entered_finishing_at >= chrono::Duration::seconds(SHUTDOWN_GRACE_SECS)
}

/// 非终态会话是否已超时（>2h），由后台 worker 回收。
pub fn is_session_stale(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - updated_at >= chrono::Duration::hours(SESSION_STALE_HOURS)
}

/// 会话响应 `steps[]` 里单步的状态（契约 §3.2.1：`ok|running|pending|error`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Ok,
    Running,
    Pending,
    Error,
}

impl StepStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Running => "running",
            Self::Pending => "pending",
            Self::Error => "error",
        }
    }
}

/// 契约 §3.2.1 的 `steps[]` 元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildStep {
    pub key: String,
    pub label: String,
    pub status: StepStatus,
    pub detail: String,
}

impl BuildStep {
    fn new(key: &str, label: &str, status: StepStatus, detail: impl Into<String>) -> Self {
        Self {
            key: key.to_owned(),
            label: label.to_owned(),
            status,
            detail: detail.into(),
        }
    }
}

/// base 模式 `steps[]` 的 key 顺序（契约 §3.2.1）。
pub const STEP_KEYS: [&str; 5] = ["base_pull", "create_vm", "console", "shutdown", "upload"];

/// iso 模式 `steps[]` 的 key 顺序（契约 A.3.2）。
pub const ISO_STEP_KEYS: [&str; 6] = [
    "iso_check",
    "create_vm",
    "install",
    "eject_iso",
    "shutdown",
    "upload",
];

/// 会话 `steps[]` 的 key 顺序（按来源模式）。
pub fn step_keys(source_mode: SourceMode) -> &'static [&'static str] {
    match source_mode {
        SourceMode::Base => &STEP_KEYS,
        SourceMode::Iso => &ISO_STEP_KEYS,
    }
}

/// 计算会话响应里的 `steps[]`（纯函数，教师端直接渲染）。
///
/// 规则：已完成的前置步骤恒为 `ok`；当前步骤按状态标 `running`；`failed` 时按最近
/// 一个已下发的命令定位失败步骤并标 `error`；`cancelled` / `expired` 时未完成步骤
/// 标 `pending` 且 detail 说明已取消/超时。
pub fn session_steps(session: &BuildSession, base_transfer: Option<&Transfer>) -> Vec<BuildStep> {
    match session.source_mode {
        SourceMode::Base => base_session_steps(session, base_transfer),
        SourceMode::Iso => iso_session_steps(session),
    }
}

/// base 模式 steps（契约 §3.2.1，保持 v1 行为）。
fn base_session_steps(session: &BuildSession, base_transfer: Option<&Transfer>) -> Vec<BuildStep> {
    let progress = base_transfer.map(|transfer| transfer.progress).unwrap_or(0);
    let pull_status = match base_transfer.map(|transfer| transfer.status) {
        Some(crate::storage::TransferStatus::Completed) => StepStatus::Ok,
        Some(crate::storage::TransferStatus::Failed)
        | Some(crate::storage::TransferStatus::Expired) => StepStatus::Error,
        Some(crate::storage::TransferStatus::Running) => StepStatus::Running,
        Some(crate::storage::TransferStatus::Pending) => StepStatus::Pending,
        None => StepStatus::Pending,
    };
    let pull_detail = match pull_status {
        StepStatus::Ok => "100%".to_owned(),
        StepStatus::Error => base_transfer
            .and_then(|transfer| transfer.error.clone())
            .unwrap_or_else(|| "拉取失败".to_owned()),
        _ => format!("{progress}%"),
    };

    // 各步骤的状态按 5 步顺序推进：一旦某步未完成，后面的步骤都是 pending。
    let (mut steps, mut active) = (
        [
            BuildStep::new("base_pull", "拉取系统底包", pull_status, pull_detail),
            BuildStep::new("create_vm", "创建构建虚拟机", StepStatus::Pending, ""),
            BuildStep::new("console", "远程操作", StepStatus::Pending, ""),
            BuildStep::new("shutdown", "关机", StepStatus::Pending, ""),
            BuildStep::new("upload", "导出并分片上传", StepStatus::Pending, ""),
        ],
        true,
    );
    let mut finish = |index: usize, status: StepStatus, detail: &str| {
        if let Some(step) = steps.get_mut(index) {
            step.status = status;
            step.detail = detail.to_owned();
        }
    };

    // 基础镜像拉取失败时，后续步骤不再标记（仍为 pending）。
    if pull_status == StepStatus::Error {
        active = false;
    }

    if active {
        match session.state {
            BuildState::Preparing => {}
            BuildState::Creating => finish(1, StepStatus::Running, "创建中…"),
            BuildState::Ready | BuildState::Finishing | BuildState::Making | BuildState::Done => {
                finish(1, StepStatus::Ok, session.build_vm_name.as_str());
                finish(2, StepStatus::Ok, "已就绪");
            }
            _ => {}
        }
    }
    if active {
        match session.state {
            BuildState::Finishing => finish(3, StepStatus::Running, "关机中…"),
            BuildState::Making | BuildState::Done => finish(3, StepStatus::Ok, "已关机"),
            _ => {}
        }
    }
    if active {
        match session.state {
            BuildState::Making => finish(4, StepStatus::Running, "导出并上传中…"),
            BuildState::Done => {
                let detail = session
                    .image_id
                    .map(|id| format!("已生成镜像 {id}"))
                    .unwrap_or_else(|| "已完成".to_owned());
                finish(4, StepStatus::Ok, &detail);
            }
            _ => {}
        }
    }

    match session.state {
        BuildState::Failed => {
            // 按最近一个已下发的命令定位失败步骤：make → shutdown → create → base_pull。
            let failed_index = if session.make_request_id.is_some()
                || session.image_id.is_some()
                || session.shutdown_request_id.is_some() && session.destroy_request_id.is_some()
            {
                4
            } else if session.shutdown_request_id.is_some() {
                3
            } else if session.create_request_id.is_some() {
                1
            } else {
                0
            };
            let error = session
                .error
                .clone()
                .unwrap_or_else(|| "制作失败".to_owned());
            if let Some(step) = steps.get_mut(failed_index)
                && step.status != StepStatus::Ok
            {
                step.status = StepStatus::Error;
                step.detail = error;
            }
        }
        BuildState::Cancelled => {
            for step in steps.iter_mut() {
                if step.status != StepStatus::Ok {
                    step.status = StepStatus::Pending;
                    step.detail = "已取消".to_owned();
                }
            }
        }
        BuildState::Expired => {
            for step in steps.iter_mut() {
                if step.status != StepStatus::Ok {
                    step.status = StepStatus::Pending;
                    step.detail = "会话超时已回收".to_owned();
                }
            }
        }
        _ => {}
    }

    steps.into_iter().collect()
}

/// 设置某一步的状态与 detail（越界忽略，避免状态机脏数据 panic）。
fn set_step(steps: &mut [BuildStep], index: usize, status: StepStatus, detail: &str) {
    if let Some(step) = steps.get_mut(index) {
        step.status = status;
        step.detail = detail.to_owned();
    }
}

/// iso 模式 steps（契约 A.3.2）：
/// `iso_check → create_vm → install → eject_iso → shutdown → upload`。
fn iso_session_steps(session: &BuildSession) -> Vec<BuildStep> {
    let iso_name = session.iso_name.as_deref().unwrap_or("");
    let iso_ok_detail = format!("ISO 已找到：{iso_name}");
    let mut steps = vec![
        BuildStep::new("iso_check", "校验安装 ISO", StepStatus::Pending, ""),
        BuildStep::new("create_vm", "创建安装虚拟机", StepStatus::Pending, ""),
        BuildStep::new("install", "远程安装/配置", StepStatus::Pending, ""),
        BuildStep::new("eject_iso", "弹出 ISO", StepStatus::Pending, ""),
        BuildStep::new("shutdown", "关机", StepStatus::Pending, ""),
        BuildStep::new("upload", "导出并分片上传", StepStatus::Pending, ""),
    ];

    match session.state {
        BuildState::Preparing => set_step(&mut steps, 0, StepStatus::Running, "校验安装 ISO…"),
        BuildState::Creating => {
            set_step(&mut steps, 0, StepStatus::Ok, &iso_ok_detail);
            set_step(&mut steps, 1, StepStatus::Running, "创建中…");
        }
        BuildState::Installing => {
            set_step(&mut steps, 0, StepStatus::Ok, &iso_ok_detail);
            set_step(
                &mut steps,
                1,
                StepStatus::Ok,
                session.build_vm_name.as_str(),
            );
            set_step(&mut steps, 2, StepStatus::Running, "等待在 VM 内完成安装…");
            set_step(
                &mut steps,
                3,
                StepStatus::Pending,
                "安装完成后点『安装完成，弹出 ISO』",
            );
        }
        BuildState::Ejecting => {
            set_step(&mut steps, 0, StepStatus::Ok, &iso_ok_detail);
            set_step(
                &mut steps,
                1,
                StepStatus::Ok,
                session.build_vm_name.as_str(),
            );
            set_step(&mut steps, 2, StepStatus::Ok, "安装完成");
            set_step(&mut steps, 3, StepStatus::Running, "正在弹出 ISO…");
        }
        BuildState::Ready | BuildState::Finishing | BuildState::Making | BuildState::Done => {
            set_step(&mut steps, 0, StepStatus::Ok, &iso_ok_detail);
            set_step(
                &mut steps,
                1,
                StepStatus::Ok,
                session.build_vm_name.as_str(),
            );
            set_step(&mut steps, 2, StepStatus::Ok, "安装完成");
            set_step(&mut steps, 3, StepStatus::Ok, "ISO 已弹出");
        }
        // 取消/超时：已完成的阶段（由 command id 推断）保留 ok，其余在下面标 pending。
        BuildState::Cancelled | BuildState::Expired => {
            if session.create_request_id.is_some() {
                set_step(&mut steps, 0, StepStatus::Ok, &iso_ok_detail);
            }
            if session.eject_request_id.is_some() {
                set_step(
                    &mut steps,
                    1,
                    StepStatus::Ok,
                    session.build_vm_name.as_str(),
                );
                set_step(&mut steps, 2, StepStatus::Ok, "安装完成");
            }
        }
        _ => {}
    }

    match session.state {
        BuildState::Finishing => set_step(&mut steps, 4, StepStatus::Running, "关机中…"),
        BuildState::Making | BuildState::Done => set_step(&mut steps, 4, StepStatus::Ok, "已关机"),
        _ => {}
    }
    match session.state {
        BuildState::Making => set_step(&mut steps, 5, StepStatus::Running, "导出并上传中…"),
        BuildState::Done => {
            let detail = session
                .image_id
                .map(|id| format!("已生成镜像 {id}"))
                .unwrap_or_else(|| "已完成".to_owned());
            set_step(&mut steps, 5, StepStatus::Ok, &detail);
        }
        _ => {}
    }

    match session.state {
        BuildState::Failed => {
            // 定位失败步骤：make → shutdown → eject → create_vm → iso_check。
            let failed_index = if session.make_request_id.is_some()
                || session.image_id.is_some()
                || session.shutdown_request_id.is_some() && session.destroy_request_id.is_some()
            {
                5
            } else if session.shutdown_request_id.is_some() {
                4
            } else if session.eject_request_id.is_some() {
                3
            } else if session.create_request_id.is_some() {
                1
            } else {
                0
            };
            let error = session
                .error
                .clone()
                .unwrap_or_else(|| "制作失败".to_owned());
            if let Some(step) = steps.get_mut(failed_index)
                && step.status != StepStatus::Ok
            {
                step.status = StepStatus::Error;
                step.detail = error;
            }
        }
        BuildState::Cancelled => {
            for step in steps.iter_mut() {
                if step.status != StepStatus::Ok {
                    step.status = StepStatus::Pending;
                    step.detail = "已取消".to_owned();
                }
            }
        }
        BuildState::Expired => {
            for step in steps.iter_mut() {
                if step.status != StepStatus::Ok {
                    step.status = StepStatus::Pending;
                    step.detail = "会话超时已回收".to_owned();
                }
            }
        }
        _ => {}
    }

    steps
}

/// 合并「配置镜像 + 房间默认底包/增量包」并按 image_id 去重、保持首次出现顺序。
///
/// 用于 `/agent/startup`：无论是否配置 startprofile，房间默认底包都要下发
/// （契约 `docs/image-studio.md` §3.1）。
pub fn merge_default_image_ids(
    configured: &[Uuid],
    defaults: Option<&RoomImageDefault>,
) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    let mut merged = Vec::with_capacity(configured.len() + 2);
    for image_id in configured {
        if seen.insert(*image_id) {
            merged.push(*image_id);
        }
    }
    if let Some(defaults) = defaults {
        for image_id in [Some(defaults.base_image_id), defaults.delta_image_id]
            .into_iter()
            .flatten()
        {
            if seen.insert(image_id) {
                merged.push(image_id);
            }
        }
    }
    merged
}

/// 默认底包没有 startup profile 时的合成 revision：用 `updated_at` 的 Unix 秒，
/// 保证 `>0`（agent 只处理 `enabled && revision>0`），默认底包变更时 revision 变化。
pub fn synthesized_defaults_revision(updated_at: DateTime<Utc>) -> i64 {
    updated_at.timestamp().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{TransferDirection, TransferStatus};

    fn sample_session(state: BuildState) -> BuildSession {
        let now = Utc::now();
        BuildSession {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            machine_serial: "0638".to_owned(),
            build_vm_name: build_vm_name(Uuid::nil()),
            source_mode: SourceMode::Base,
            iso_name: None,
            disk_bus: DiskBus::Virtio,
            nic: NicMode::None,
            media_ejected: false,
            iso_check_request_id: None,
            eject_request_id: None,
            base_image_id: Some(Uuid::new_v4()),
            delta_image_id: None,
            base_transfer_id: None,
            name: "lab-base".to_owned(),
            version: "1.0.0".to_owned(),
            platform: "linux/amd64".to_owned(),
            vcpus: 2,
            memory_mib: 2_048,
            disk_gb: 40,
            description: None,
            state,
            error: None,
            create_request_id: None,
            shutdown_request_id: None,
            destroy_request_id: None,
            make_request_id: None,
            image_id: None,
            created_at: now,
            updated_at: now,
            finished_at: None,
        }
    }

    fn transfer(status: TransferStatus, progress: i32) -> Transfer {
        let now = Utc::now();
        Transfer {
            id: Uuid::new_v4(),
            direction: TransferDirection::Download,
            room_id: Uuid::new_v4(),
            image_id: Some(Uuid::new_v4()),
            manifest_digest: Some("sha256:abc".to_owned()),
            status,
            progress,
            error: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn state_text_round_trips_and_terminal_flags() {
        for state in [
            BuildState::Preparing,
            BuildState::Creating,
            BuildState::Installing,
            BuildState::Ejecting,
            BuildState::Ready,
            BuildState::Finishing,
            BuildState::Making,
            BuildState::Done,
            BuildState::Failed,
            BuildState::Cancelled,
            BuildState::Expired,
        ] {
            assert_eq!(state.as_str().parse::<BuildState>(), Ok(state));
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
        assert!("bogus".parse::<BuildState>().is_err());
        for state in [
            BuildState::Done,
            BuildState::Failed,
            BuildState::Cancelled,
            BuildState::Expired,
        ] {
            assert!(state.is_terminal());
            assert!(!state.is_active());
        }
        for state in [
            BuildState::Preparing,
            BuildState::Creating,
            BuildState::Installing,
            BuildState::Ejecting,
            BuildState::Ready,
            BuildState::Finishing,
            BuildState::Making,
        ] {
            assert!(!state.is_terminal());
            assert!(state.is_active());
        }
    }

    #[test]
    fn only_ready_can_finish_and_non_terminal_can_cancel() {
        assert!(BuildState::Ready.can_finish());
        for state in [
            BuildState::Preparing,
            BuildState::Creating,
            BuildState::Installing,
            BuildState::Ejecting,
            BuildState::Finishing,
            BuildState::Making,
            BuildState::Done,
            BuildState::Failed,
            BuildState::Cancelled,
            BuildState::Expired,
        ] {
            assert!(!state.can_finish(), "{state}");
        }
        assert!(!BuildState::Done.can_cancel());
        assert!(!BuildState::Failed.can_cancel());
        for state in [
            BuildState::Preparing,
            BuildState::Creating,
            BuildState::Installing,
            BuildState::Ejecting,
            BuildState::Ready,
            BuildState::Finishing,
            BuildState::Making,
        ] {
            assert!(state.can_cancel(), "{state}");
        }
    }

    #[test]
    fn build_vm_name_and_base_volume_follow_contract_examples() {
        let id = Uuid::parse_str("a1b2c3d4-0000-4000-8000-000000000000").unwrap();
        assert_eq!(build_vm_name(id), "qe-build-a1b2c3d4");
        assert_eq!(
            base_volume_name("real-debian-13", "0.1.1"),
            "real-debian-13-0.1.1"
        );
    }

    #[test]
    fn preparing_outcome_maps_transfer_status() {
        assert!(matches!(
            preparing_outcome(TransferStatus::Completed, None),
            Some(Ok(()))
        ));
        assert!(matches!(
            preparing_outcome(TransferStatus::Pending, None),
            None
        ));
        assert!(matches!(
            preparing_outcome(TransferStatus::Running, None),
            None
        ));
        let failed = preparing_outcome(TransferStatus::Failed, Some("磁盘满了")).unwrap();
        assert!(failed.unwrap_err().contains("磁盘满了"));
        assert!(
            preparing_outcome(TransferStatus::Expired, None)
                .unwrap()
                .unwrap_err()
                .contains("过期")
        );
    }

    #[test]
    fn session_steps_advance_with_state() {
        let completed = transfer(TransferStatus::Completed, 100);
        let ready = sample_session(BuildState::Ready);
        let steps = session_steps(&ready, Some(&completed));
        assert_eq!(
            steps
                .iter()
                .map(|step| step.key.as_str())
                .collect::<Vec<_>>(),
            STEP_KEYS
        );
        assert_eq!(steps[0].status, StepStatus::Ok);
        assert_eq!(steps[1].status, StepStatus::Ok);
        assert_eq!(steps[2].status, StepStatus::Ok);
        assert_eq!(steps[3].status, StepStatus::Pending);
        assert_eq!(steps[4].status, StepStatus::Pending);

        let making = sample_session(BuildState::Making);
        let steps = session_steps(&making, Some(&completed));
        assert_eq!(steps[3].status, StepStatus::Ok);
        assert_eq!(steps[4].status, StepStatus::Running);

        let mut done = sample_session(BuildState::Done);
        done.image_id = Some(Uuid::new_v4());
        let steps = session_steps(&done, Some(&completed));
        assert_eq!(steps[4].status, StepStatus::Ok);
        assert!(steps[4].detail.contains("已生成镜像"));
    }

    #[test]
    fn session_steps_show_transfer_progress_and_errors() {
        let running = transfer(TransferStatus::Running, 42);
        let session = sample_session(BuildState::Preparing);
        let steps = session_steps(&session, Some(&running));
        assert_eq!(steps[0].status, StepStatus::Running);
        assert_eq!(steps[0].detail, "42%");
        assert_eq!(steps[1].status, StepStatus::Pending);

        let mut failed_transfer = transfer(TransferStatus::Failed, 0);
        failed_transfer.error = Some("镜像不存在".to_owned());
        let steps = session_steps(&session, Some(&failed_transfer));
        assert_eq!(steps[0].status, StepStatus::Error);
        assert_eq!(steps[0].detail, "镜像不存在");
    }

    #[test]
    fn session_steps_mark_failed_step_from_last_command() {
        let completed = transfer(TransferStatus::Completed, 100);
        let mut session = sample_session(BuildState::Failed);
        session.error = Some("upload failed".to_owned());
        session.create_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, Some(&completed));
        assert_eq!(steps[1].status, StepStatus::Error);
        assert_eq!(steps[1].detail, "upload failed");

        // 已经发过 make 命令 → upload 步骤失败。
        session.make_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, Some(&completed));
        assert_eq!(steps[4].status, StepStatus::Error);

        // 取消/超时：未完成步骤 detail 提示。
        let mut cancelled = sample_session(BuildState::Cancelled);
        cancelled.create_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&cancelled, Some(&completed));
        assert_eq!(steps[0].status, StepStatus::Ok);
        assert_eq!(steps[1].status, StepStatus::Pending);
        assert_eq!(steps[1].detail, "已取消");

        let expired = sample_session(BuildState::Expired);
        let steps = session_steps(&expired, None);
        assert_eq!(steps[0].detail, "会话超时已回收");
    }

    #[test]
    fn defaults_merge_dedupes_and_keeps_order() {
        let configured = [Uuid::new_v4(), Uuid::new_v4()];
        let base = configured[1];
        let delta = Uuid::new_v4();
        let defaults = RoomImageDefault {
            room_id: Uuid::new_v4(),
            base_image_id: base,
            delta_image_id: Some(delta),
            updated_by: None,
            updated_at: Utc::now(),
        };
        let merged = merge_default_image_ids(&configured, Some(&defaults));
        assert_eq!(merged, vec![configured[0], base, delta]);
        assert_eq!(
            merge_default_image_ids(&configured, None),
            configured.to_vec()
        );
        assert!(merge_default_image_ids(&[], None).is_empty());
    }

    #[test]
    fn stale_and_grace_checks_use_contract_windows() {
        let now = Utc::now();
        assert!(!is_session_stale(now, now));
        assert!(!is_session_stale(now - chrono::Duration::hours(1), now));
        assert!(is_session_stale(now - chrono::Duration::hours(3), now));
        assert!(!shutdown_grace_expired(
            now - chrono::Duration::seconds(59),
            now
        ));
        assert!(shutdown_grace_expired(
            now - chrono::Duration::seconds(61),
            now
        ));
        assert!(can_make_image(true, false));
        assert!(can_make_image(false, true));
        assert!(!can_make_image(false, false));
    }

    #[test]
    fn synthesized_revision_is_always_positive() {
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        assert_eq!(synthesized_defaults_revision(epoch), 1);
        let later = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        assert_eq!(synthesized_defaults_revision(later), 1_700_000_000);
    }

    #[test]
    fn visibility_helper_admits_owner_or_admin_only() {
        let owner = Uuid::new_v4();
        let mut session = sample_session(BuildState::Ready);
        session.user_id = owner;
        assert!(session.visible_to(owner, false));
        assert!(!session.visible_to(Uuid::new_v4(), false));
        assert!(session.visible_to(Uuid::new_v4(), true));
    }

    // ---- v2：ISO 安装 / Windows（契约附录 A） ----

    fn iso_session(state: BuildState) -> BuildSession {
        let mut session = sample_session(state);
        session.source_mode = SourceMode::Iso;
        session.iso_name = Some("Win10_LTSC_2021.iso".to_owned());
        session.disk_bus = DiskBus::Sata;
        session.nic = NicMode::None;
        session.base_image_id = None;
        session.base_transfer_id = None;
        session
    }

    fn iso_list_data(names: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "isos": names
                .iter()
                .map(|name| serde_json::json!({ "name": name, "size_bytes": 5_368_709_120_i64 }))
                .collect::<Vec<_>>()
        })
    }

    #[test]
    fn iso_name_validation_rejects_paths_and_accepts_filename() {
        assert_eq!(
            validate_iso_name(" Win10_LTSC_2021.iso ").unwrap(),
            "Win10_LTSC_2021.iso"
        );
        assert_eq!(validate_iso_name("debian-13.iso").unwrap(), "debian-13.iso");
        for bad in [
            "",
            "   ",
            "../etc/passwd",
            "iso/win.iso",
            "iso\\win.iso",
            "a..b.iso",
            &"x".repeat(129),
        ] {
            assert!(validate_iso_name(bad).is_err(), "should reject {bad:?}");
        }
        assert!(validate_iso_name("weird\u{0007}.iso").is_err());
    }

    #[test]
    fn source_mode_disk_bus_nic_round_trip() {
        for mode in [SourceMode::Base, SourceMode::Iso] {
            assert_eq!(mode.as_str().parse::<SourceMode>(), Ok(mode));
        }
        assert_eq!(SourceMode::Base.default_disk_bus(), DiskBus::Virtio);
        assert_eq!(SourceMode::Iso.default_disk_bus(), DiskBus::Sata);
        for bus in [DiskBus::Virtio, DiskBus::Sata] {
            assert_eq!(bus.as_str().parse::<DiskBus>(), Ok(bus));
        }
        for nic in [NicMode::None, NicMode::User, NicMode::Default] {
            assert_eq!(nic.as_str().parse::<NicMode>(), Ok(nic));
        }
        assert!("sas".parse::<DiskBus>().is_err());
        assert!("bridge".parse::<NicMode>().is_err());
        assert_eq!(
            serde_json::to_value(SourceMode::Iso).unwrap(),
            serde_json::json!("iso")
        );
        assert_eq!(
            serde_json::to_value(DiskBus::Sata).unwrap(),
            serde_json::json!("sata")
        );
        assert_eq!(
            serde_json::to_value(NicMode::None).unwrap(),
            serde_json::json!("none")
        );
    }

    #[test]
    fn iso_check_outcome_requires_iso_in_list() {
        let data = iso_list_data(&["other.iso", "Win10_LTSC_2021.iso"]);
        assert!(matches!(
            iso_check_outcome("ok", Some(&data), None, "Win10_LTSC_2021.iso"),
            Some(Ok(()))
        ));
        let err = iso_check_outcome("ok", Some(&data), None, "missing.iso")
            .unwrap()
            .unwrap_err();
        assert!(err.contains("没有找到"), "{err}");
        assert!(matches!(
            iso_check_outcome("pending", Some(&data), None, "x.iso"),
            None
        ));
        let failed = iso_check_outcome("error", None, Some("agent 无响应"), "x.iso")
            .unwrap()
            .unwrap_err();
        assert!(failed.contains("agent 无响应"));
        // data 缺失 isos 数组 → 视为找不到。
        let empty = serde_json::json!({});
        assert!(
            iso_check_outcome("ok", Some(&empty), None, "x.iso")
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn iso_steps_full_state_machine() {
        assert_eq!(step_keys(SourceMode::Iso), &ISO_STEP_KEYS);
        assert_eq!(step_keys(SourceMode::Base), &STEP_KEYS);

        // preparing：校验 ISO 中。
        let steps = session_steps(&iso_session(BuildState::Preparing), None);
        let keys: Vec<&str> = steps.iter().map(|step| step.key.as_str()).collect();
        assert_eq!(keys, ISO_STEP_KEYS);
        assert_eq!(steps[0].status, StepStatus::Running);
        assert_eq!(steps[1].status, StepStatus::Pending);

        // creating：ISO 已找到，创建 VM。
        let mut session = iso_session(BuildState::Creating);
        session.iso_check_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, None);
        assert_eq!(steps[0].status, StepStatus::Ok);
        assert!(steps[0].detail.contains("Win10_LTSC_2021.iso"));
        assert_eq!(steps[1].status, StepStatus::Running);

        // installing：安装中，等待 eject。
        let mut session = iso_session(BuildState::Installing);
        session.create_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, None);
        assert_eq!(steps[1].status, StepStatus::Ok);
        assert_eq!(steps[2].status, StepStatus::Running);
        assert_eq!(steps[3].status, StepStatus::Pending);
        assert!(steps[3].detail.contains("弹出 ISO"));

        // ejecting。
        let mut session = iso_session(BuildState::Ejecting);
        session.create_request_id = Some(Uuid::new_v4());
        session.eject_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, None);
        assert_eq!(steps[2].status, StepStatus::Ok);
        assert_eq!(steps[3].status, StepStatus::Running);

        // ready：ISO 已弹出，等待 finish。
        let mut session = iso_session(BuildState::Ready);
        session.create_request_id = Some(Uuid::new_v4());
        session.eject_request_id = Some(Uuid::new_v4());
        session.media_ejected = true;
        let steps = session_steps(&session, None);
        assert_eq!(steps[3].status, StepStatus::Ok);
        assert_eq!(steps[4].status, StepStatus::Pending);

        // finishing → making → done。
        let mut session = iso_session(BuildState::Finishing);
        session.create_request_id = Some(Uuid::new_v4());
        session.eject_request_id = Some(Uuid::new_v4());
        session.shutdown_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, None);
        assert_eq!(steps[3].status, StepStatus::Ok);
        assert_eq!(steps[4].status, StepStatus::Running);
        assert_eq!(steps[5].status, StepStatus::Pending);

        let mut session = iso_session(BuildState::Making);
        session.create_request_id = Some(Uuid::new_v4());
        session.eject_request_id = Some(Uuid::new_v4());
        session.shutdown_request_id = Some(Uuid::new_v4());
        session.make_request_id = Some(Uuid::new_v4());
        session.image_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, None);
        assert_eq!(steps[4].status, StepStatus::Ok);
        assert_eq!(steps[5].status, StepStatus::Running);

        let mut session = iso_session(BuildState::Done);
        session.create_request_id = Some(Uuid::new_v4());
        session.eject_request_id = Some(Uuid::new_v4());
        session.shutdown_request_id = Some(Uuid::new_v4());
        session.make_request_id = Some(Uuid::new_v4());
        session.image_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, None);
        for step in &steps {
            assert_eq!(step.status, StepStatus::Ok, "step {} not ok", step.key);
        }
        assert!(steps[5].detail.contains("已生成镜像"));
    }

    #[test]
    fn iso_steps_mark_failed_step() {
        // ISO 不存在（preparing 失败）。
        let mut session = iso_session(BuildState::Failed);
        session.iso_check_request_id = Some(Uuid::new_v4());
        session.error = Some("目标机器上没有找到安装 ISO：x.iso".to_owned());
        let steps = session_steps(&session, None);
        assert_eq!(steps[0].status, StepStatus::Error);
        assert!(steps[0].detail.contains("没有找到"));

        // create 失败。
        let mut session = iso_session(BuildState::Failed);
        session.create_request_id = Some(Uuid::new_v4());
        session.error = Some("create failed".to_owned());
        let steps = session_steps(&session, None);
        assert_eq!(steps[1].status, StepStatus::Error);

        // eject 失败。
        let mut session = iso_session(BuildState::Failed);
        session.create_request_id = Some(Uuid::new_v4());
        session.eject_request_id = Some(Uuid::new_v4());
        session.error = Some("eject failed".to_owned());
        let steps = session_steps(&session, None);
        assert_eq!(steps[3].status, StepStatus::Error);

        // 取消：未完成步骤提示已取消。
        let mut session = iso_session(BuildState::Cancelled);
        session.create_request_id = Some(Uuid::new_v4());
        let steps = session_steps(&session, None);
        assert_eq!(steps[0].status, StepStatus::Ok);
        assert_eq!(steps[2].status, StepStatus::Pending);
        assert_eq!(steps[2].detail, "已取消");
    }
}
