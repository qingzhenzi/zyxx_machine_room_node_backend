//! 机器总览：设备注册信息 + 房间/区域 + room-agent 最新快照与历史。
//!
//! 数据来源：`device_enrollment` / `device_session` / `rooms` / `areas` /
//! `machine_snapshots`（见 `migrations/0014_machine_telemetry.sql`）。
//! 读权限复用 `device:read`；机器数据属于设备观测面，不引入新的 RBAC 权限点。

use std::{collections::VecDeque, convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, Sse},
};
use chrono::{DateTime, Utc};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::time::Instant;
use uuid::Uuid;

use qe_db::machine::{MachineSnapshotMeta, MachineSnapshotRecord, MachineSummary};

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::Page,
        rbac::permissions,
        storage::{validate_image_name, validate_image_version, validate_platform},
        validation, wol,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct MachineQuery {
    #[serde(default)]
    pub area_id: Option<Uuid>,
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub keyword: Option<String>,
    /// `ok` / `warning` / `critical` / `missing`（从未上报）。
    #[serde(default)]
    pub health: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MachineDetailQuery {
    /// 历史快照条数，1-50，默认 20。
    #[serde(default)]
    pub history_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SnapshotQuery {
    /// 历史快照条数，1-200，默认 50。
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MachineStreamQuery {
    /// 从该快照 id 之后开始补发；缺省表示只推送连接之后产生的新快照。
    #[serde(default)]
    pub after_id: Option<i64>,
    /// 只订阅某台机器；缺省为全部机器。
    #[serde(default)]
    pub serial: Option<String>,
}

/// 机器详情：展开设备摘要 + 最新完整快照 + 历史元数据。
#[derive(Debug, Serialize)]
pub struct MachineDetail {
    #[serde(flatten)]
    pub device: MachineSummary,
    pub latest: Option<MachineSnapshotRecord>,
    pub history: Vec<MachineSnapshotMeta>,
}

fn normalize_health_filter(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let normalized = value.to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "ok" | "warning" | "critical" | "missing" | "unknown"
    ) {
        Ok(Some(normalized))
    } else {
        Err(AppError::bad_request(
            "health 只能是 ok / warning / critical / missing / unknown",
        ))
    }
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<MachineQuery>,
) -> AppResult<Json<Page<MachineSummary>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    let health = normalize_health_filter(query.health.as_deref())?;

    let filter = qe_db::machine::MachineFilter {
        area_id: query.area_id,
        room_id: query.room_id,
        keyword: (!keyword.is_empty()).then_some(keyword),
        health,
    };

    Ok(Json(state.machines.list(&filter, page, page_size).await?))
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
    Query(query): Query<MachineDetailQuery>,
) -> AppResult<Json<MachineDetail>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }

    let history_limit = query.history_limit.unwrap_or(20);
    if !(1..=50).contains(&history_limit) {
        return Err(AppError::bad_request("history_limit 需在 1-50 之间"));
    }

    let device = state
        .machines
        .summary(serial)
        .await?
        .ok_or_else(|| AppError::not_found("机器不存在"))?;
    let latest = state.machines.latest(serial).await?;
    let history = state.machines.history(serial, history_limit).await?;

    Ok(Json(MachineDetail {
        device,
        latest,
        history,
    }))
}

pub async fn snapshots(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
    Query(query): Query<SnapshotQuery>,
) -> AppResult<Json<Vec<MachineSnapshotMeta>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(AppError::bad_request("limit 需在 1-200 之间"));
    }

    if state.machines.summary(serial).await?.is_none() {
        return Err(AppError::not_found("机器不存在"));
    }
    Ok(Json(state.machines.history(serial, limit).await?))
}

/// WOL：每个目标 MAC 发送的 magic packet 次数（UDP 无连接，重复提高到达率）。
const WOL_SEND_COUNT: usize = 3;
/// Wake-on-LAN 标准端口 9 的 IPv4 广播地址（需与目标机同一二层网络）。
const WOL_BROADCAST_ADDR: &str = "255.255.255.255:9";

/// 网络唤醒响应：`ok` 表示至少有一个 magic packet 成功发出；广播失败只进 `warnings`，
/// HTTP 仍是 200（契约见 `docs/wol.md`）。
#[derive(Debug, Serialize)]
pub struct WakeReply {
    pub ok: bool,
    pub serial: String,
    /// 本次尝试的目标 MAC（来自最新快照 `sections.agent.host.macs`）。
    pub macs: Vec<String>,
    /// 至少成功发出一次 magic packet 的 MAC。
    pub sent_to: Vec<String>,
    /// 发送失败的 MAC/原因（不阻塞 HTTP 语义）。
    pub warnings: Vec<String>,
}

/// `POST /api/v1/admin/machines/{serial}/wake` —— 局域网唤醒（Wake-on-LAN）。
///
/// - 权限：`device:wake`（管理端默认只有 `admin` 角色，迁移 0030 播种）；
/// - 目标 MAC 取该 serial **最新一条** `machine_snapshots.payload` 的
///   `sections.agent.host.macs`；设备不存在 404、无快照/无 MAC 422；
/// - 每个 MAC 发 3 次 102 字节 magic packet（6×0xFF + MAC×16）到 UDP 255.255.255.255:9；
/// - 发送结果写审计 `device.wake`（resource_type=`machine`）。
pub async fn wake(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(serial): Path<String>,
) -> AppResult<Json<WakeReply>> {
    authorization::require_permission(&state, id, permissions::DEVICE_WAKE).await?;

    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }

    let device = state
        .machines
        .summary(serial)
        .await?
        .ok_or_else(|| AppError::not_found("机器不存在"))?;
    let latest = state
        .machines
        .latest(&device.serial)
        .await?
        .ok_or_else(|| {
            AppError::unprocessable_entity(
                "该机器尚未上报快照，无法获取网卡 MAC（sections.agent.host.macs）",
            )
        })?;
    let macs = wol::macs_from_payload(&latest.payload);
    if macs.is_empty() {
        return Err(AppError::unprocessable_entity(
            "最新快照未上报可用网卡 MAC（sections.agent.host.macs），无法发送唤醒包",
        ));
    }

    let mac_labels: Vec<String> = macs.iter().map(ToString::to_string).collect();
    let (sent_to, warnings) = send_wake_packets(&macs).await;
    let ok = !sent_to.is_empty();
    if !warnings.is_empty() {
        tracing::warn!(
            serial = %device.serial,
            ?warnings,
            "Wake-on-LAN 部分或全部发送失败"
        );
    }

    super::audit::record_or_log(
        &state,
        id,
        "device.wake",
        "machine",
        Some(device.serial.clone()),
        serde_json::json!({
            "serial": device.serial,
            "macs": mac_labels,
            "sent_to": sent_to,
            "warnings": warnings,
            "snapshot_id": latest.id,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(WakeReply {
        ok,
        serial: device.serial,
        macs: mac_labels,
        sent_to,
        warnings,
    }))
}

/// 发送 magic packet：每个 MAC 发 [`WOL_SEND_COUNT`] 次；返回 `(成功送达的 MAC, 警告)`。
///
/// 广播发送是「尽力而为」：UDP 无连接、无 ACK，只要 socket 写入成功就算发出，
/// 目标机是否真的醒来由目标机 BIOS/网卡决定（见 `docs/wol.md` 排障）。
async fn send_wake_packets(macs: &[wol::MacAddress]) -> (Vec<String>, Vec<String>) {
    let mut sent_to = Vec::new();
    let mut warnings = Vec::new();

    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(socket) => socket,
        Err(error) => {
            warnings.push(format!(
                "创建 UDP socket 失败（本机可能无网络权限）: {error}"
            ));
            return (sent_to, warnings);
        }
    };
    if let Err(error) = socket.set_broadcast(true) {
        warnings.push(format!("开启 UDP 广播失败: {error}"));
    }

    for mac in macs {
        let packet = wol::magic_packet(*mac);
        let mut succeeded = 0usize;
        let mut last_error: Option<String> = None;
        for _ in 0..WOL_SEND_COUNT {
            match socket.send_to(&packet, WOL_BROADCAST_ADDR).await {
                Ok(sent) if sent == packet.len() => succeeded += 1,
                Ok(sent) => {
                    last_error = Some(format!("仅发送 {sent}/{} 字节", packet.len()));
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if succeeded > 0 {
            sent_to.push(mac.to_string());
        } else {
            warnings.push(format!(
                "{mac} 发送失败: {}",
                last_error.unwrap_or_else(|| "未知错误".to_owned())
            ));
        }
    }

    (sent_to, warnings)
}

// ---------------------------------------------------------------------------
// VM 远程控制（kind = vm.command，经 device_downlink 下发；结果回写 0031 表）
//
// 阶段 1.5：命令参数从 {op,name?} 扩展为 {op,args?}（保留旧 name 兼容），
// args 按各 op 校验后原样透传给 room-agent；另提供教师「批量下发环境」API。
// 契约见 docs/remote-vm-control.md 与 docs/vm-base-image-and-diff-disk.md §3/§4。
// ---------------------------------------------------------------------------

/// 下发 VM 命令的 device_downlink kind（与 room-agent 侧契约一致）。
const VM_COMMAND_DOWNLINK_KIND: &str = "vm.command";
/// pending 超过该秒数视为超时（GET 时顺手把表更新为 timeout）。
const VM_COMMAND_PENDING_TIMEOUT_SECS: i64 = 120;
/// `image.make`（导出→分片上传→登记）的 pending 超时阈值（设计 §8.5：600s）。
const VM_COMMAND_IMAGE_MAKE_TIMEOUT_SECS: i64 = 600;

/// 允许的 VM 操作白名单（阶段 1 + 阶段 1.5 底包/差分盘/快照）。
const VM_COMMAND_OPS: &[&str] = &[
    "list",
    "start",
    "shutdown",
    "reboot",
    "destroy",
    "delete",
    "screenshot",
    "create",
    "snapshot.create",
    "snapshot.list",
    "snapshot.restore",
    "snapshot.delete",
    "disk.info",
    "export",
    "image.make",
    "vnc.info",
];

/// 容器命令下行 kind（与 agent `container_control` 对齐）。
const CONTAINER_COMMAND_DOWNLINK_KIND: &str = "container.command";
/// 允许的容器操作白名单（Incus 后端）。
const CONTAINER_COMMAND_OPS: &[&str] = &[
    "list",
    "start",
    "stop",
    "restart",
    "delete",
    "snapshot.create",
    "snapshot.list",
    "snapshot.restore",
    "snapshot.delete",
];

/// VM 名/snapshot 名的通用约束（防路径注入）：`[A-Za-z0-9._-]{1,64}`。
const VM_NAME_MAX_CHARS: usize = 64;
/// 批量下发时 name_prefix 上限（给 `<prefix>-<slot尾号>` 留空间）。
const VM_PREFIX_MAX_CHARS: usize = 32;
/// GPU 直通地址上限。
const VM_GPU_MAX: usize = 2;
/// create 默认/取值范围。
const VM_DEFAULT_VCPUS: u64 = 2;
const VM_DEFAULT_MEMORY_MIB: u64 = 2_048;
const VM_DEFAULT_DISK_GB: u64 = 20;
const VM_VCPUS_RANGE: (u64, u64) = (1, 64);
const VM_MEMORY_MIB_RANGE: (u64, u64) = (256, 262_144);
const VM_DISK_GB_RANGE: (u64, u64) = (1, 2_048);
/// `create` 允许的 args 字段（未知字段直接 400，避免拼写错误静默透传）。
const VM_CREATE_ARGS: &[&str] = &[
    "name",
    "vcpus",
    "memory_mib",
    "disk_gb",
    "base_volume",
    "base_path",
    "gpu",
    "start",
];

/// `POST /admin/machines/{serial}/vm/commands` 请求体。
#[derive(Debug, Deserialize)]
pub struct VmCommandRequest {
    /// 操作：list / start / ... / export（见 [`VM_COMMAND_OPS`]）。
    pub op: String,
    /// 阶段 1 兼容字段：等价于 `args.name`。
    #[serde(default)]
    pub name: Option<String>,
    /// 阶段 1.5：各 op 的参数对象（原样透传给 agent）。
    #[serde(default)]
    pub args: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 校验后的 VM 命令（args 已规范化、未知字段已拒绝）。
#[derive(Debug, Clone, PartialEq)]
struct ValidatedVmCommand {
    op: String,
    args: serde_json::Map<String, serde_json::Value>,
}

/// 202 响应：只回 request_id，前端据此轮询结果。
#[derive(Debug, Serialize)]
pub struct VmCommandAccepted {
    pub request_id: Uuid,
}

/// GET 结果响应（`data` 为 agent 回传的 result；截图/快照/磁盘链/导出都在 data 里）。
#[derive(Debug, Serialize)]
pub struct VmCommandReply {
    pub request_id: Uuid,
    pub status: String,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `machine_command_results` 行（表由迁移 0031 创建）。
#[derive(Debug, sqlx::FromRow)]
struct MachineCommandRow {
    request_id: Uuid,
    status: String,
    result: Option<serde_json::Value>,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// 下发 payload（`{request_id,op,args}`），用于按 op 决定 pending 超时阈值。
    payload: Option<serde_json::Value>,
}

// ---- 参数校验（纯函数，便于单测） ----

fn bad_arg(message: impl Into<String>) -> AppError {
    AppError::bad_request(message.into())
}

fn reject_unknown_args(
    args: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
) -> AppResult<()> {
    for key in args.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(bad_arg(format!(
                "不支持的参数 `{key}`（允许：{}）",
                allowed.join(" / ")
            )));
        }
    }
    Ok(())
}

fn expect_string(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> AppResult<Option<String>> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(raw)) => Ok(Some(raw.trim().to_owned())),
        Some(_) => Err(bad_arg(format!("{key} 必须是字符串"))),
    }
}

fn expect_u64(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> AppResult<Option<u64>> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| bad_arg(format!("{key} 必须是非负整数"))),
        Some(serde_json::Value::String(raw)) => raw
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| bad_arg(format!("{key} 必须是非负整数"))),
        Some(_) => Err(bad_arg(format!("{key} 必须是非负整数"))),
    }
}

fn expect_bool(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> AppResult<Option<bool>> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(bad_arg(format!("{key} 必须是布尔值"))),
    }
}

fn expect_string_array(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> AppResult<Option<Vec<String>>> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Array(items)) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let Some(value) = item.as_str() else {
                    return Err(bad_arg(format!("{key} 必须是字符串数组")));
                };
                values.push(value.trim().to_owned());
            }
            Ok(Some(values))
        }
        Some(_) => Err(bad_arg(format!("{key} 必须是字符串数组"))),
    }
}

/// 校验 `[A-Za-z0-9._-]{1,max}`，返回 trim 后的规范值。
pub(crate) fn validate_name_token(raw: &str, field: &str, max_chars: usize) -> AppResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(bad_arg(format!("{field} 不能为空")));
    }
    if value.chars().count() > max_chars {
        return Err(bad_arg(format!("{field} 不能超过 {max_chars} 个字符")));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(bad_arg(format!(
            "{field} 只允许字母、数字、点、下划线、短横线（{value}）"
        )));
    }
    Ok(value.to_owned())
}

/// libvirt 存储池卷名：不能含 `/` 或 NUL（卷名即文件路径的一段）。
fn validate_volume_name(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(bad_arg("base_volume 不能为空"));
    }
    if value.chars().count() > 128 {
        return Err(bad_arg("base_volume 不能超过 128 个字符"));
    }
    if value.contains('/') || value.contains('\0') || value.contains("..") {
        return Err(bad_arg("base_volume 含有非法字符"));
    }
    Ok(value.to_owned())
}

fn validate_base_path(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    if !value.starts_with('/') {
        return Err(bad_arg("base_path 必须是绝对路径"));
    }
    if value.chars().count() > 512 {
        return Err(bad_arg("base_path 不能超过 512 个字符"));
    }
    if value.contains('\0') {
        return Err(bad_arg("base_path 含有非法字符"));
    }
    Ok(value.to_owned())
}

/// GPU PCI 直通地址：`dddd:bb:dd.f`（十六进制，function 0-7）。
fn validate_gpu_address(raw: &str) -> AppResult<String> {
    let value = raw.trim();
    let invalid = || bad_arg(format!("gpu 地址非法（{value}），应为 dddd:bb:dd.f"));
    let mut parts = value.split(':');
    let (Some(domain), Some(bus), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid());
    };
    if parts.next().is_some() {
        return Err(invalid());
    }
    let (device, function) = rest.split_once('.').ok_or_else(invalid)?;
    let hex_ok =
        |part: &str, len: usize| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit());
    if !hex_ok(domain, 4) || !hex_ok(bus, 2) || !hex_ok(device, 2) {
        return Err(invalid());
    }
    if function.len() != 1 || !matches!(function, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7") {
        return Err(invalid());
    }
    Ok(value.to_owned())
}

fn check_range(value: u64, field: &str, range: (u64, u64)) -> AppResult<u64> {
    if !(range.0..=range.1).contains(&value) {
        return Err(bad_arg(format!(
            "{field} 必须在 {}..={} 之间（当前 {value}）",
            range.0, range.1
        )));
    }
    Ok(value)
}

fn required_name(args: &serde_json::Map<String, serde_json::Value>) -> AppResult<String> {
    let raw = expect_string(args, "name")?.ok_or_else(|| bad_arg("缺少 name"))?;
    validate_name_token(&raw, "name", VM_NAME_MAX_CHARS)
}

fn required_snapshot(args: &serde_json::Map<String, serde_json::Value>) -> AppResult<String> {
    let raw = expect_string(args, "snapshot")?.ok_or_else(|| bad_arg("缺少 snapshot"))?;
    validate_name_token(&raw, "snapshot", VM_NAME_MAX_CHARS)
}

/// 校验并规范化 `create` 的 args（补默认值；`base_volume` 与 `base_path` 二选一）。
fn build_create_args(
    args: &serde_json::Map<String, serde_json::Value>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    reject_unknown_args(args, VM_CREATE_ARGS)?;
    let mut out = serde_json::Map::new();
    out.insert("name".to_owned(), serde_json::json!(required_name(args)?));
    let vcpus = check_range(
        expect_u64(args, "vcpus")?.unwrap_or(VM_DEFAULT_VCPUS),
        "vcpus",
        VM_VCPUS_RANGE,
    )?;
    let memory_mib = check_range(
        expect_u64(args, "memory_mib")?.unwrap_or(VM_DEFAULT_MEMORY_MIB),
        "memory_mib",
        VM_MEMORY_MIB_RANGE,
    )?;
    let disk_gb = check_range(
        expect_u64(args, "disk_gb")?.unwrap_or(VM_DEFAULT_DISK_GB),
        "disk_gb",
        VM_DISK_GB_RANGE,
    )?;
    out.insert("vcpus".to_owned(), serde_json::json!(vcpus));
    out.insert("memory_mib".to_owned(), serde_json::json!(memory_mib));
    out.insert("disk_gb".to_owned(), serde_json::json!(disk_gb));

    let base_volume = expect_string(args, "base_volume")?
        .filter(|value| !value.is_empty())
        .map(|value| validate_volume_name(&value))
        .transpose()?;
    let base_path = expect_string(args, "base_path")?
        .filter(|value| !value.is_empty())
        .map(|value| validate_base_path(&value))
        .transpose()?;
    match (base_volume, base_path) {
        (Some(_), Some(_)) => {
            return Err(bad_arg("base_volume 与 base_path 只能二选一"));
        }
        (Some(volume), None) => {
            out.insert("base_volume".to_owned(), serde_json::json!(volume));
        }
        (None, Some(path)) => {
            out.insert("base_path".to_owned(), serde_json::json!(path));
        }
        (None, None) => {}
    }

    if let Some(gpu) = expect_string_array(args, "gpu")? {
        if gpu.len() > VM_GPU_MAX {
            return Err(bad_arg(format!("gpu 最多 {VM_GPU_MAX} 个")));
        }
        let gpu: Vec<String> = gpu
            .iter()
            .map(|value| validate_gpu_address(value))
            .collect::<AppResult<_>>()?;
        out.insert("gpu".to_owned(), serde_json::json!(gpu));
    }

    if let Some(start) = expect_bool(args, "start")? {
        out.insert("start".to_owned(), serde_json::json!(start));
    }
    Ok(out)
}

/// 校验 `image.make` 的 args（从 VM 导出 → 分片上传 → 登记镜像，agent 侧执行）。
///
/// args：`{vm_name, name, version, platform?, base_volume?, description?}`；
/// 镜像名/版本/平台复用 `qe_domain::storage` 的校验规则（服务端不信任 agent）。
fn build_image_make_args(
    args: &serde_json::Map<String, serde_json::Value>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    reject_unknown_args(
        args,
        &[
            "vm_name",
            "name",
            "version",
            "platform",
            "base_volume",
            "description",
        ],
    )?;
    let mut out = serde_json::Map::new();
    let vm_name = expect_string(args, "vm_name")?.ok_or_else(|| bad_arg("缺少 vm_name"))?;
    let vm_name = validate_name_token(&vm_name, "vm_name", VM_NAME_MAX_CHARS)?;
    out.insert("vm_name".to_owned(), serde_json::json!(vm_name));

    let name = expect_string(args, "name")?.ok_or_else(|| bad_arg("缺少镜像名 name"))?;
    let name = validate_image_name(&name).map_err(|error| bad_arg(error.to_string()))?;
    out.insert("name".to_owned(), serde_json::json!(name));

    let version = expect_string(args, "version")?.ok_or_else(|| bad_arg("缺少镜像版本 version"))?;
    let version = validate_image_version(&version).map_err(|error| bad_arg(error.to_string()))?;
    out.insert("version".to_owned(), serde_json::json!(version));

    if let Some(platform) = expect_string(args, "platform")?.filter(|value| !value.is_empty()) {
        let platform = validate_platform(&platform).map_err(|error| bad_arg(error.to_string()))?;
        out.insert("platform".to_owned(), serde_json::json!(platform));
    }
    if let Some(base_volume) = expect_string(args, "base_volume")?.filter(|value| !value.is_empty())
    {
        let base_volume = validate_volume_name(&base_volume)?;
        out.insert("base_volume".to_owned(), serde_json::json!(base_volume));
    }
    if let Some(description) = expect_string(args, "description")?.filter(|value| !value.is_empty())
    {
        if description.chars().count() > 255 {
            return Err(bad_arg("description 不能超过 255 个字符"));
        }
        out.insert("description".to_owned(), serde_json::json!(description));
    }
    Ok(out)
}

/// 校验 op 白名单与各 op 的 args（纯函数，便于单测）。
fn validate_vm_command(request: VmCommandRequest) -> AppResult<ValidatedVmCommand> {
    let op = request.op.trim().to_ascii_lowercase();
    if !VM_COMMAND_OPS.contains(&op.as_str()) {
        return Err(bad_arg(format!("op 只能是 {}", VM_COMMAND_OPS.join(" / "))));
    }

    let mut args = request.args.unwrap_or_default();
    // 阶段 1 兼容：`{op,name}` ≡ `{op,args:{name}}`；两者同时给出时必须一致。
    if let Some(legacy_name) = request.name {
        let legacy_name = legacy_name.trim().to_owned();
        if !legacy_name.is_empty() {
            match args.get("name") {
                Some(serde_json::Value::String(existing)) if existing.trim() == legacy_name => {}
                Some(_) => {
                    return Err(bad_arg("name 与 args.name 不一致，请只使用 args"));
                }
                None => {
                    args.insert("name".to_owned(), serde_json::json!(legacy_name));
                }
            }
        }
    }

    if op == "list" {
        if !args.is_empty() {
            return Err(bad_arg("list 命令不接受参数"));
        }
        return Ok(ValidatedVmCommand {
            op,
            args: serde_json::Map::new(),
        });
    }

    let normalized = match op.as_str() {
        "create" => build_create_args(&args)?,
        "snapshot.create" => {
            reject_unknown_args(&args, &["name", "snapshot", "description"])?;
            let mut out = serde_json::Map::new();
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            out.insert(
                "snapshot".to_owned(),
                serde_json::json!(required_snapshot(&args)?),
            );
            if let Some(description) = expect_string(&args, "description")? {
                if description.chars().count() > 255 {
                    return Err(bad_arg("description 不能超过 255 个字符"));
                }
                out.insert("description".to_owned(), serde_json::json!(description));
            }
            out
        }
        "snapshot.restore" | "snapshot.delete" => {
            reject_unknown_args(&args, &["name", "snapshot"])?;
            let mut out = serde_json::Map::new();
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            out.insert(
                "snapshot".to_owned(),
                serde_json::json!(required_snapshot(&args)?),
            );
            out
        }
        "snapshot.list" | "disk.info" => {
            reject_unknown_args(&args, &["name"])?;
            let mut out = serde_json::Map::new();
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            out
        }
        "export" => {
            reject_unknown_args(&args, &["name", "format"])?;
            let mut out = serde_json::Map::new();
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            let format = expect_string(&args, "format")?
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "qcow2".to_owned())
                .to_ascii_lowercase();
            if format != "qcow2" {
                return Err(bad_arg("export 目前只支持 format=qcow2"));
            }
            out.insert("format".to_owned(), serde_json::json!("qcow2"));
            out
        }
        "image.make" => build_image_make_args(&args)?,
        "delete" => {
            reject_unknown_args(&args, &["name", "remove_disks"])?;
            let mut out = serde_json::Map::new();
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            if let Some(remove_disks) = expect_bool(&args, "remove_disks")? {
                if remove_disks {
                    out.insert("remove_disks".to_owned(), serde_json::json!(true));
                }
            }
            out
        }
        // 单机生命周期/截图：只接受 name。
        _ => {
            reject_unknown_args(&args, &["name"])?;
            let mut out = serde_json::Map::new();
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            out
        }
    };

    Ok(ValidatedVmCommand {
        op,
        args: normalized,
    })
}

/// 校验容器命令（op 白名单 + args），复用 [`ValidatedVmCommand`] 形状。
fn validate_container_command(request: VmCommandRequest) -> AppResult<ValidatedVmCommand> {
    let op = request.op.trim().to_ascii_lowercase();
    if !CONTAINER_COMMAND_OPS.contains(&op.as_str()) {
        return Err(bad_arg(format!(
            "op 只能是 {}",
            CONTAINER_COMMAND_OPS.join(" / ")
        )));
    }
    let mut args = request.args.unwrap_or_default();
    if let Some(legacy_name) = request.name {
        let legacy_name = legacy_name.trim().to_owned();
        if !legacy_name.is_empty() {
            match args.get("name") {
                Some(serde_json::Value::String(existing)) if existing.trim() == legacy_name => {}
                Some(_) => return Err(bad_arg("name 与 args.name 不一致，请只使用 args")),
                None => {
                    args.insert("name".to_owned(), serde_json::json!(legacy_name));
                }
            }
        }
    }
    if op == "list" {
        if !args.is_empty() {
            return Err(bad_arg("list 命令不接受参数"));
        }
        return Ok(ValidatedVmCommand {
            op,
            args: serde_json::Map::new(),
        });
    }
    let mut out = serde_json::Map::new();
    match op.as_str() {
        "delete" | "stop" => {
            reject_unknown_args(&args, &["name", "force"])?;
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            if let Some(force) = expect_bool(&args, "force")? {
                if force {
                    out.insert("force".to_owned(), serde_json::json!(true));
                }
            }
        }
        "snapshot.create" | "snapshot.restore" | "snapshot.delete" => {
            reject_unknown_args(&args, &["name", "snapshot"])?;
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
            out.insert(
                "snapshot".to_owned(),
                serde_json::json!(required_snapshot(&args)?),
            );
        }
        _ => {
            // start/stop/restart/snapshot.list：只接受 name。
            reject_unknown_args(&args, &["name"])?;
            out.insert("name".to_owned(), serde_json::json!(required_name(&args)?));
        }
    }
    Ok(ValidatedVmCommand { op, args: out })
}

/// 构造 `vm.command` 下行 payload：`{"request_id","op","args":{...}}`。
fn vm_command_downlink_payload(
    request_id: Uuid,
    command: &ValidatedVmCommand,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "op": command.op,
        "args": command.args,
    })
}

// ---- 教师批量下发环境（room 级） ----

/// `POST /admin/rooms/{room_id}/vm/environments` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmEnvironmentRequest {
    /// `create` 的名称前缀：最终名为 `<name_prefix>-<槽位尾号>`。
    pub name_prefix: String,
    /// 底包 libvirt 卷名（`<image name>-<version>`）。
    pub base_volume: String,
    #[serde(default)]
    pub vcpus: Option<u64>,
    #[serde(default)]
    pub memory_mib: Option<u64>,
    #[serde(default)]
    pub disk_gb: Option<u64>,
    #[serde(default)]
    pub auto_start: Option<bool>,
}

/// 校验后的批量下发参数。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedVmEnvironment {
    name_prefix: String,
    base_volume: String,
    vcpus: u64,
    memory_mib: u64,
    disk_gb: u64,
    auto_start: bool,
}

/// 批量下发中的单个槽位结果。`request_id/serial` 为 `null` 表示该槽位没有存活设备，
/// 命令未入队（前端展示为「未下发」）。
#[derive(Debug, Serialize)]
pub struct VmEnvironmentTarget {
    pub device_slot: String,
    pub serial: Option<String>,
    pub request_id: Option<Uuid>,
}

/// 批量下发响应。
#[derive(Debug, Serialize)]
pub struct VmEnvironmentReply {
    pub room_id: Uuid,
    pub targets: Vec<VmEnvironmentTarget>,
}

fn validate_vm_environment(request: &VmEnvironmentRequest) -> AppResult<ValidatedVmEnvironment> {
    let name_prefix =
        validate_name_token(&request.name_prefix, "name_prefix", VM_PREFIX_MAX_CHARS)?;
    let base_volume = validate_volume_name(&request.base_volume)?;
    let vcpus = check_range(
        request.vcpus.unwrap_or(VM_DEFAULT_VCPUS),
        "vcpus",
        VM_VCPUS_RANGE,
    )?;
    let memory_mib = check_range(
        request.memory_mib.unwrap_or(VM_DEFAULT_MEMORY_MIB),
        "memory_mib",
        VM_MEMORY_MIB_RANGE,
    )?;
    let disk_gb = check_range(
        request.disk_gb.unwrap_or(VM_DEFAULT_DISK_GB),
        "disk_gb",
        VM_DISK_GB_RANGE,
    )?;
    Ok(ValidatedVmEnvironment {
        name_prefix,
        base_volume,
        vcpus,
        memory_mib,
        disk_gb,
        auto_start: request.auto_start.unwrap_or(false),
    })
}

/// 取槽位尾号作为 VM 名后缀：`room-101` → `101`，`w3f-crit-slot-1793663233` → `1793663233`；
/// 只保留 `[A-Za-z0-9._-]`，空则退回 `1`。
fn sanitize_slot_suffix(slot: &str) -> String {
    let trimmed = slot.trim();
    let candidate = trimmed
        .rsplit(['-', '_'])
        .find(|part| !part.is_empty())
        .unwrap_or(trimmed);
    let cleaned: String = candidate
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        .collect();
    let cleaned = cleaned.trim_matches(|ch| matches!(ch, '.' | '_' | '-'));
    if cleaned.is_empty() {
        "1".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// 生成 `<prefix>-<槽位尾号>`，总长不超过 [`VM_NAME_MAX_CHARS`]。
fn batch_vm_name(prefix: &str, slot: &str) -> String {
    let mut suffix = sanitize_slot_suffix(slot);
    let max_suffix = VM_NAME_MAX_CHARS.saturating_sub(2);
    if suffix.len() > max_suffix {
        suffix.truncate(max_suffix);
    }
    let max_prefix = VM_NAME_MAX_CHARS.saturating_sub(suffix.len() + 1).max(1);
    let prefix: String = prefix.chars().take(max_prefix).collect();
    format!("{prefix}-{suffix}")
}

/// 构造批量 `create` 的 args（与单机 create 同构）。
fn environment_create_args(
    env: &ValidatedVmEnvironment,
    slot: &str,
) -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "name": batch_vm_name(&env.name_prefix, slot),
        "vcpus": env.vcpus,
        "memory_mib": env.memory_mib,
        "disk_gb": env.disk_gb,
        "base_volume": env.base_volume,
        "start": env.auto_start,
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

// ---- 数据库写入 / 下行 ----

async fn insert_pending_vm_command(
    state: &AppState,
    request_id: Uuid,
    serial: &str,
    downlink: &serde_json::Value,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO machine_command_results (request_id, serial, kind, payload, status)
         VALUES ($1, $2, $3, $4, 'pending')",
    )
    .bind(request_id)
    .bind(serial)
    .bind(VM_COMMAND_DOWNLINK_KIND)
    .bind(downlink)
    .execute(state.db.pool())
    .await
    .map_err(|error| AppError::internal(format!("写入 VM 命令失败: {error}")))?;
    Ok(())
}

/// 入队 `vm.command`；成功时顺手唤醒 device-pki 长连接（失败还有 2s 兜底轮询）。
async fn enqueue_vm_downlink(
    state: &AppState,
    slot: &str,
    downlink_text: &str,
) -> AppResult<Option<String>> {
    match state
        .storage_repo
        .enqueue_downlink_for_slot(slot, VM_COMMAND_DOWNLINK_KIND, downlink_text)
        .await
    {
        Ok(Some(device_serial)) => {
            if let Err(error) = state
                .cache
                .publish(
                    &state.config.schedule_downlink_notify_channel,
                    &device_serial,
                )
                .await
            {
                tracing::warn!(%error, %device_serial, "发布设备下行通知失败，vm.command 已入队");
            }
            Ok(Some(device_serial))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// 入队失败时清掉刚插入的 pending 行，避免留下永远不会被处理的命令。
async fn cleanup_pending_vm_command(state: &AppState, request_id: Uuid) {
    if let Err(error) = sqlx::query(
        "DELETE FROM machine_command_results WHERE request_id = $1 AND status = 'pending'",
    )
    .bind(request_id)
    .execute(state.db.pool())
    .await
    {
        tracing::warn!(%error, %request_id, "清理未下发的 vm.command pending 行失败");
    }
}

/// 插入一条 pending 命令（kind 通用：vm.command / container.command）。
async fn insert_pending_command(
    state: &AppState,
    request_id: Uuid,
    serial: &str,
    kind: &str,
    downlink: &serde_json::Value,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO machine_command_results (request_id, serial, kind, payload, status)
         VALUES ($1, $2, $3, $4, 'pending')",
    )
    .bind(request_id)
    .bind(serial)
    .bind(kind)
    .bind(downlink)
    .execute(state.db.pool())
    .await
    .map_err(|error| AppError::internal(format!("写入命令失败: {error}")))?;
    Ok(())
}

/// 清理未下发的 pending 行（kind 通用）。
async fn cleanup_pending_command(state: &AppState, request_id: Uuid) {
    if let Err(error) = sqlx::query(
        "DELETE FROM machine_command_results WHERE request_id = $1 AND status = 'pending'",
    )
    .bind(request_id)
    .execute(state.db.pool())
    .await
    {
        tracing::warn!(%error, %request_id, "清理未下发的 pending 命令失败");
    }
}

/// 解析槽位当前存活的设备 serial（与 `enqueue_downlink_for_slot` 的选取口径一致）。
async fn resolve_live_serial(state: &AppState, slot: &str) -> AppResult<Option<String>> {
    sqlx::query_scalar(
        "SELECT serial
           FROM device_enrollment
          WHERE device_slot = $1 AND status IN ('issued', 'active')
          ORDER BY (status = 'active') DESC, activated_at DESC NULLS LAST
          LIMIT 1",
    )
    .bind(slot)
    .fetch_optional(state.db.pool())
    .await
    .map_err(|error| AppError::internal(format!("查询设备槽位失败: {error}")))
}

/// 向设备槽位入队一条普通下行（非命令结果类，如 `vnc.open`），并尽力唤醒长连接。
pub(crate) async fn enqueue_device_downlink(
    state: &AppState,
    slot: &str,
    kind: &str,
    payload: &serde_json::Value,
) -> AppResult<()> {
    let text = payload.to_string();
    match state
        .storage_repo
        .enqueue_downlink_for_slot(slot, kind, &text)
        .await
    {
        Ok(Some(serial)) => {
            if let Err(error) = state
                .cache
                .publish(&state.config.schedule_downlink_notify_channel, &serial)
                .await
            {
                tracing::warn!(%error, %serial, kind, "发布设备下行通知失败，下行已入队");
            }
            Ok(())
        }
        Ok(None) => Err(AppError::service_unavailable(
            "设备当前没有存活证书，无法建立 VNC 隧道",
        )),
        Err(error) => Err(error.into()),
    }
}

/// 下发一条 `vm.command` 并同步等待终态（VNC 会话等需要立即拿结果的场景）。
///
/// `wait` 内未收到终态返回 502；pending 行保留（agent 之后仍可回写结果）。
pub(crate) async fn execute_vm_command(
    state: &AppState,
    serial: &str,
    device_slot: &str,
    op: &str,
    args: serde_json::Value,
    wait: Duration,
) -> AppResult<VmCommandReply> {
    let request_id = Uuid::new_v4();
    let command = ValidatedVmCommand {
        op: op.to_owned(),
        args: args.as_object().cloned().unwrap_or_default(),
    };
    let downlink = vm_command_downlink_payload(request_id, &command);
    insert_pending_vm_command(state, request_id, serial, &downlink).await?;
    match enqueue_vm_downlink(state, device_slot, &downlink.to_string()).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            cleanup_pending_vm_command(state, request_id).await;
            return Err(AppError::service_unavailable(
                "设备当前没有存活证书，命令未下发",
            ));
        }
        Err(error) => {
            cleanup_pending_vm_command(state, request_id).await;
            return Err(error);
        }
    }

    let deadline = Instant::now() + wait;
    loop {
        let row = sqlx::query_as::<_, MachineCommandRow>(
            "SELECT request_id, status, result, error, created_at, updated_at, payload
               FROM machine_command_results
              WHERE request_id = $1 AND serial = $2",
        )
        .bind(request_id)
        .bind(serial)
        .fetch_optional(state.db.pool())
        .await
        .map_err(|error| AppError::internal(format!("查询 VM 命令失败: {error}")))?;
        if let Some(row) = row {
            if row.status != "pending" {
                return Ok(VmCommandReply {
                    request_id: row.request_id,
                    status: row.status,
                    data: row.result,
                    error: row.error,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(AppError::bad_gateway(format!(
                "机器在 {}s 内未返回 {op} 结果",
                wait.as_secs()
            )));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ---- HTTP handlers ----

/// `POST /api/v1/admin/machines/{serial}/vm/commands` —— 下发 VM 远程控制命令。
///
/// 权限 `vm:control`；设备不存在 404；op/args 非法 400。
/// 成功：插入 `machine_command_results(pending)` + enqueue downlink `vm.command`，
/// 返回 **202** `{request_id}`；设备无存活证书/入队失败时清理 pending 行并返回 4xx/503。
pub async fn create_vm_command(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
    ApiJson(request): ApiJson<VmCommandRequest>,
) -> AppResult<(StatusCode, Json<VmCommandAccepted>)> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;

    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }
    let command = validate_vm_command(request)?;
    let device = state
        .machines
        .summary(serial)
        .await?
        .ok_or_else(|| AppError::not_found("机器不存在"))?;

    let request_id = Uuid::new_v4();
    let downlink = vm_command_downlink_payload(request_id, &command);
    insert_pending_vm_command(&state, request_id, &device.serial, &downlink).await?;

    let downlink_text = downlink.to_string();
    match enqueue_vm_downlink(&state, &device.device_slot, &downlink_text).await {
        Ok(Some(_)) => Ok((StatusCode::ACCEPTED, Json(VmCommandAccepted { request_id }))),
        Ok(None) => {
            cleanup_pending_vm_command(&state, request_id).await;
            Err(AppError::service_unavailable(
                "设备当前没有存活证书，VM 命令未下发",
            ))
        }
        Err(error) => {
            cleanup_pending_vm_command(&state, request_id).await;
            Err(error)
        }
    }
}

/// `POST /api/v1/admin/machines/{serial}/container/commands` —— 下发容器生命周期命令。
///
/// 权限 `vm:control`（实验环境控制）；op/args 非法 400；无存活证书 503。
pub async fn create_container_command(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
    ApiJson(request): ApiJson<VmCommandRequest>,
) -> AppResult<(StatusCode, Json<VmCommandAccepted>)> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }
    let command = validate_container_command(request)?;
    let device = state
        .machines
        .summary(serial)
        .await?
        .ok_or_else(|| AppError::not_found("机器不存在"))?;

    let request_id = Uuid::new_v4();
    let downlink = vm_command_downlink_payload(request_id, &command);
    insert_pending_command(&state, request_id, &device.serial, CONTAINER_COMMAND_DOWNLINK_KIND, &downlink).await?;
    match enqueue_device_downlink(
        &state,
        &device.device_slot,
        CONTAINER_COMMAND_DOWNLINK_KIND,
        &downlink,
    )
    .await
    {
        Ok(()) => Ok((StatusCode::ACCEPTED, Json(VmCommandAccepted { request_id }))),
        Err(error) => {
            cleanup_pending_command(&state, request_id).await;
            Err(error)
        }
    }
}

/// `POST /api/v1/admin/rooms/{room_id}/vm/environments` —— 教师批量下发实验环境。
///
/// 对房间主设备槽位 + `room_devices` 子机逐槽位下发 `create`（VM 名 `<prefix>-<槽位尾号>`），
/// 每个槽位返回自己的 `request_id`（前端逐个轮询）。没有绑定任何槽位 → 409。
pub async fn create_room_vm_environment(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(room_id): Path<Uuid>,
    ApiJson(request): ApiJson<VmEnvironmentRequest>,
) -> AppResult<(StatusCode, Json<VmEnvironmentReply>)> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    let env = validate_vm_environment(&request)?;
    let room = state
        .rooms
        .get(room_id)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;

    let mut slots: Vec<String> = Vec::new();
    if let Some(slot) = room.device_slot.as_deref() {
        let slot = slot.trim();
        if !slot.is_empty() && !slots.iter().any(|existing| existing == slot) {
            slots.push(slot.to_owned());
        }
    }
    let machines = state
        .devices
        .list_room_machines(room_id)
        .await?
        .unwrap_or_default();
    for machine in machines {
        let slot = machine.device_slot.trim();
        if !slot.is_empty() && !slots.iter().any(|existing| existing == slot) {
            slots.push(slot.to_owned());
        }
    }
    if slots.is_empty() {
        return Err(AppError::conflict(
            "房间没有绑定设备槽位，无法批量下发环境配置",
        ));
    }

    let mut targets = Vec::with_capacity(slots.len());
    for slot in slots {
        let request_id = Uuid::new_v4();
        let Some(serial) = resolve_live_serial(&state, &slot).await? else {
            // 槽位已分配但设备未注册/离线：没有 serial 无法落 pending，标记为未下发。
            targets.push(VmEnvironmentTarget {
                device_slot: slot,
                serial: None,
                request_id: None,
            });
            continue;
        };

        let command = ValidatedVmCommand {
            op: "create".to_owned(),
            args: environment_create_args(&env, &slot),
        };
        let downlink = vm_command_downlink_payload(request_id, &command);
        insert_pending_vm_command(&state, request_id, &serial, &downlink).await?;
        match enqueue_vm_downlink(&state, &slot, &downlink.to_string()).await {
            Ok(Some(_)) => targets.push(VmEnvironmentTarget {
                device_slot: slot,
                serial: Some(serial),
                request_id: Some(request_id),
            }),
            Ok(None) => {
                cleanup_pending_vm_command(&state, request_id).await;
                targets.push(VmEnvironmentTarget {
                    device_slot: slot,
                    serial: Some(serial),
                    request_id: None,
                });
            }
            Err(error) => {
                cleanup_pending_vm_command(&state, request_id).await;
                tracing::warn!(%error, %slot, "批量下发环境：vm.command 入队失败");
                targets.push(VmEnvironmentTarget {
                    device_slot: slot,
                    serial: Some(serial),
                    request_id: None,
                });
            }
        }
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(VmEnvironmentReply { room_id, targets }),
    ))
}

/// `GET /api/v1/admin/machines/{serial}/vm/commands/{request_id}` —— 查询命令结果。
///
/// 权限 `vm:control`；`(serial, request_id)` 不存在 404。pending 超过 120s 时
/// 顺手把表更新为 `timeout`（多副本并发更新是幂等的）。
pub async fn get_vm_command(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path((serial, request_id)): Path<(String, String)>,
) -> AppResult<Json<VmCommandReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;

    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }
    let request_id = request_id
        .trim()
        .parse::<Uuid>()
        .map_err(|_| AppError::bad_request("request_id 不是合法 UUID"))?;

    let row = sqlx::query_as::<_, MachineCommandRow>(
        "SELECT request_id, status, result, error, created_at, updated_at, payload
           FROM machine_command_results
          WHERE request_id = $1 AND serial = $2",
    )
    .bind(request_id)
    .bind(serial)
    .fetch_optional(state.db.pool())
    .await
    .map_err(|error| AppError::internal(format!("查询 VM 命令失败: {error}")))?
    .ok_or_else(|| AppError::not_found("命令不存在"))?;

    let mut status = row.status;
    let mut updated_at = row.updated_at;
    // image.make 要做「导出 qcow2 + 分片上传」，耗时远高于普通命令（设计 §8.5 超时 600s）。
    let pending_timeout_secs = match row
        .payload
        .as_ref()
        .and_then(|payload| payload.get("op"))
        .and_then(|op| op.as_str())
    {
        Some("image.make") => VM_COMMAND_IMAGE_MAKE_TIMEOUT_SECS,
        _ => VM_COMMAND_PENDING_TIMEOUT_SECS,
    };
    if status == "pending"
        && row.created_at + chrono::Duration::seconds(pending_timeout_secs) < Utc::now()
    {
        sqlx::query(
            "UPDATE machine_command_results
                SET status = 'timeout', updated_at = now()
              WHERE request_id = $1 AND status = 'pending'",
        )
        .bind(request_id)
        .execute(state.db.pool())
        .await
        .map_err(|error| AppError::internal(format!("更新 VM 命令超时失败: {error}")))?;
        status = "timeout".to_owned();
        updated_at = Utc::now();
    }

    Ok(Json(VmCommandReply {
        request_id: row.request_id,
        status,
        data: row.result,
        error: row.error,
        created_at: row.created_at,
        updated_at,
    }))
}

/// SSE 实时流：新快照落库后 2 秒内推送；首次连接从当前最新 id 开始（不补历史），
/// 断线重连带上最后收到的 `id` 作为 `after_id` 可补发最多 200 条。
///
/// 采用查库轮询而不是进程内广播：多副本部署时每个副本都能看到别的实例写入的快照。
pub async fn stream(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<MachineStreamQuery>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let serial = query
        .serial
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    if let Some(after_id) = query.after_id
        && after_id < 0
    {
        return Err(AppError::bad_request("after_id 不能为负数"));
    }

    let cursor = match query.after_id {
        Some(after_id) => after_id,
        // 不补历史：从当前最新 id 开始，只推送连接之后的新快照。
        None => state.machines.max_snapshot_id().await?,
    };

    let initial = MachineStreamState {
        state,
        serial,
        last_id: cursor,
        replay: query.after_id.is_some(),
        pending: VecDeque::new(),
        backlog_loaded: false,
        sent_ready: false,
        last_heartbeat: Instant::now(),
    };

    let stream = stream::unfold(initial, |mut state| async move {
        loop {
            // 建连后立刻发一帧 ready，浏览器 fetch 才能尽快拿到响应头并显示「已连接」。
            if !state.sent_ready {
                state.sent_ready = true;
                return Some((Ok(Event::default().event("ready").data("{}")), state));
            }

            if let Some(record) = state.pending.pop_front() {
                state.last_id = record.id;
                return Some((Ok(machine_snapshot_frame(record)), state));
            }

            if state.replay && !state.backlog_loaded {
                state.backlog_loaded = true;
                match state
                    .state
                    .machines
                    .list_after_id(state.last_id, state.serial.as_deref(), STREAM_BACKLOG_LIMIT)
                    .await
                {
                    Ok(records) => {
                        state.enqueue(records);
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "SSE 补发机器快照失败，稍后重试");
                    }
                }
            }

            tokio::time::sleep(STREAM_POLL_INTERVAL).await;

            if state.last_heartbeat.elapsed() >= STREAM_HEARTBEAT_INTERVAL {
                state.last_heartbeat = Instant::now();
                return Some((Ok(Event::default().event("heartbeat").data("{}")), state));
            }

            match state
                .state
                .machines
                .list_after_id(state.last_id, state.serial.as_deref(), STREAM_POLL_LIMIT)
                .await
            {
                Ok(records) => state.enqueue(records),
                Err(error) => tracing::warn!(%error, "SSE 增量查询机器快照失败"),
            }
        }
    });

    Ok(Sse::new(stream))
}

const STREAM_BACKLOG_LIMIT: i64 = 200;
const STREAM_POLL_LIMIT: i64 = 100;
const STREAM_POLL_INTERVAL: Duration = Duration::from_secs(2);
const STREAM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

struct MachineStreamState {
    state: AppState,
    serial: Option<String>,
    last_id: i64,
    replay: bool,
    pending: VecDeque<MachineSnapshotMeta>,
    backlog_loaded: bool,
    sent_ready: bool,
    last_heartbeat: Instant,
}

impl MachineStreamState {
    fn enqueue(&mut self, mut records: Vec<MachineSnapshotMeta>) {
        records.sort_by_key(|record| record.id);
        self.pending.extend(records);
    }
}

fn machine_snapshot_frame(record: MachineSnapshotMeta) -> Event {
    let id = record.id.to_string();
    let data = serde_json::to_string(&record).unwrap_or_else(|error| {
        tracing::error!(%error, snapshot_id = record.id, "序列化机器快照失败");
        "{}".to_owned()
    });
    Event::default().id(id).event("machine.snapshot").data(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(op: &str, name: Option<&str>) -> VmCommandRequest {
        VmCommandRequest {
            op: op.to_owned(),
            name: name.map(ToOwned::to_owned),
            args: None,
        }
    }

    fn request_args(op: &str, args: serde_json::Value) -> VmCommandRequest {
        VmCommandRequest {
            op: op.to_owned(),
            name: None,
            args: args.as_object().cloned(),
        }
    }

    #[test]
    fn vm_ops_whitelist_includes_phase15_ops() {
        for op in [
            "list",
            "start",
            "shutdown",
            "reboot",
            "destroy",
            "screenshot",
            "create",
            "snapshot.create",
            "snapshot.list",
            "snapshot.restore",
            "snapshot.delete",
            "disk.info",
            "export",
            "image.make",
            "vnc.info",
        ] {
            assert!(VM_COMMAND_OPS.contains(&op), "缺少 op：{op}");
        }
        for op in ["exec", "rm-rf", ""] {
            assert!(
                validate_vm_command(request(op, Some("debian12"))).is_err(),
                "应拒绝 op={op}"
            );
        }
    }

    #[test]
    fn legacy_name_is_mapped_to_args_name() {
        let command = validate_vm_command(request("START", Some(" debian12 "))).unwrap();
        assert_eq!(command.op, "start");
        assert_eq!(command.args["name"], serde_json::json!("debian12"));

        // name 与 args.name 冲突 → 400
        assert!(
            validate_vm_command(VmCommandRequest {
                op: "start".to_owned(),
                name: Some("a".to_owned()),
                args: serde_json::json!({"name":"b"}).as_object().cloned(),
            })
            .is_err()
        );
    }

    #[test]
    fn list_rejects_arguments() {
        assert!(validate_vm_command(request("list", None)).is_ok());
        assert!(validate_vm_command(request("list", Some("debian12"))).is_err());
        assert!(
            validate_vm_command(request_args("list", serde_json::json!({"name":"x"}))).is_err()
        );
    }

    #[test]
    fn lifecycle_ops_require_valid_name() {
        for op in ["start", "shutdown", "reboot", "destroy", "screenshot"] {
            assert!(validate_vm_command(request(op, None)).is_err(), "op={op}");
            assert!(
                validate_vm_command(request(op, Some("  "))).is_err(),
                "op={op} 空白 name 应拒绝"
            );
            assert!(
                validate_vm_command(request(op, Some("debian12"))).is_ok(),
                "op={op}"
            );
        }
        // 名字白名单（防路径注入）
        for bad in ["../evil", "a/b", "a b", "域名", &"x".repeat(65)] {
            assert!(
                validate_vm_command(request("start", Some(bad))).is_err(),
                "应拒绝 name={bad}"
            );
        }
        for good in ["debian12", "lab.1", "a_b-c"] {
            assert!(validate_vm_command(request("start", Some(good))).is_ok());
        }
    }

    #[test]
    fn create_applies_defaults_and_ranges() {
        let command = validate_vm_command(request_args(
            "create",
            serde_json::json!({"name":"lab-1","base_volume":"ubuntu-22.04"}),
        ))
        .unwrap();
        assert_eq!(command.args["name"], serde_json::json!("lab-1"));
        assert_eq!(command.args["vcpus"], serde_json::json!(2));
        assert_eq!(command.args["memory_mib"], serde_json::json!(2048));
        assert_eq!(command.args["disk_gb"], serde_json::json!(20));
        assert_eq!(
            command.args["base_volume"],
            serde_json::json!("ubuntu-22.04")
        );
        assert!(command.args.get("start").is_none());

        for (args, label) in [
            (serde_json::json!({"name":"a","vcpus":0}), "vcpus 越界"),
            (serde_json::json!({"name":"a","vcpus":65}), "vcpus 越界"),
            (
                serde_json::json!({"name":"a","memory_mib":255}),
                "memory 越界",
            ),
            (
                serde_json::json!({"name":"a","memory_mib":262145}),
                "memory 越界",
            ),
            (serde_json::json!({"name":"a","disk_gb":0}), "disk 越界"),
            (serde_json::json!({"name":"a","disk_gb":2049}), "disk 越界"),
            (
                serde_json::json!({"name":"a","base_volume":"x","base_path":"/tmp/x"}),
                "底包二选一",
            ),
            (
                serde_json::json!({"name":"a","base_path":"relative.qcow2"}),
                "base_path 非绝对",
            ),
            (
                serde_json::json!({"name":"a","gpu":["0000:01:00.0","0000:02:00.0","0000:03:00.0"]}),
                "gpu 超限",
            ),
            (
                serde_json::json!({"name":"a","gpu":["not-a-pci"]}),
                "gpu 格式",
            ),
            (serde_json::json!({"name":"a","unknown":1}), "未知字段"),
        ] {
            assert!(
                validate_vm_command(request_args("create", args)).is_err(),
                "应拒绝：{label}"
            );
        }

        let command = validate_vm_command(request_args(
            "create",
            serde_json::json!({
                "name":"lab-1","vcpus":4,"memory_mib":8192,"disk_gb":100,
                "base_volume":"ubuntu-22.04","gpu":["0000:01:00.0","0000:02:00.0"],
                "start":true
            }),
        ))
        .unwrap();
        assert_eq!(command.args["vcpus"], serde_json::json!(4));
        assert_eq!(command.args["memory_mib"], serde_json::json!(8192));
        assert_eq!(command.args["disk_gb"], serde_json::json!(100));
        assert_eq!(
            command.args["gpu"],
            serde_json::json!(["0000:01:00.0", "0000:02:00.0"])
        );
        assert_eq!(command.args["start"], serde_json::json!(true));
    }

    #[test]
    fn snapshot_and_disk_ops_validate_args() {
        // snapshot.create：name + snapshot 必填，description 可选≤255
        let command = validate_vm_command(request_args(
            "snapshot.create",
            serde_json::json!({"name":"lab-1","snapshot":"snap-1","description":"after install"}),
        ))
        .unwrap();
        assert_eq!(command.args["snapshot"], serde_json::json!("snap-1"));
        assert_eq!(
            command.args["description"],
            serde_json::json!("after install")
        );
        for args in [
            serde_json::json!({"name":"lab-1"}),
            serde_json::json!({"snapshot":"snap-1"}),
            serde_json::json!({"name":"lab-1","snapshot":"snap-1","description":"x".repeat(256)}),
        ] {
            assert!(validate_vm_command(request_args("snapshot.create", args)).is_err());
        }
        // restore/delete 必须 name+snapshot
        for op in ["snapshot.restore", "snapshot.delete"] {
            assert!(
                validate_vm_command(request_args(op, serde_json::json!({"name":"lab-1"}))).is_err()
            );
            assert!(
                validate_vm_command(request_args(
                    op,
                    serde_json::json!({"name":"lab-1","snapshot":"snap-1"})
                ))
                .is_ok()
            );
        }
        // list/disk.info 只接受 name
        for op in ["snapshot.list", "disk.info"] {
            assert!(validate_vm_command(request_args(op, serde_json::json!({}))).is_err());
            assert!(
                validate_vm_command(request_args(op, serde_json::json!({"name":"lab-1"}))).is_ok()
            );
        }
        // export 只支持 qcow2（缺省即 qcow2）
        let command =
            validate_vm_command(request_args("export", serde_json::json!({"name":"lab-1"})))
                .unwrap();
        assert_eq!(command.args["format"], serde_json::json!("qcow2"));
        assert!(
            validate_vm_command(request_args(
                "export",
                serde_json::json!({"name":"lab-1","format":"raw"})
            ))
            .is_err()
        );
        assert!(
            validate_vm_command(request_args(
                "export",
                serde_json::json!({"name":"lab-1","format":"qcow2"})
            ))
            .is_ok()
        );
    }

    #[test]
    fn image_make_args_are_validated() {
        let ok = validate_vm_command(request_args(
            "image.make",
            serde_json::json!({
                "vm_name":"lab-1","name":"lab-base","version":"1.0.0",
                "platform":"linux/amd64","base_volume":"ubuntu-22.04","description":"after install"
            }),
        ))
        .unwrap();
        assert_eq!(ok.args["vm_name"], serde_json::json!("lab-1"));
        assert_eq!(ok.args["name"], serde_json::json!("lab-base"));
        assert_eq!(ok.args["platform"], serde_json::json!("linux/amd64"));

        for (args, label) in [
            (
                serde_json::json!({"name":"lab-base","version":"1.0.0"}),
                "缺 vm_name",
            ),
            (
                serde_json::json!({"vm_name":"lab-1","version":"1.0.0"}),
                "缺镜像名",
            ),
            (
                serde_json::json!({"vm_name":"lab-1","name":"lab-base"}),
                "缺版本",
            ),
            (
                serde_json::json!({"vm_name":"lab-1","name":"Lab Base","version":"1.0.0"}),
                "镜像名非法",
            ),
            (
                serde_json::json!({"vm_name":"lab-1","name":"lab-base","version":"1 0"}),
                "版本非法",
            ),
            (
                serde_json::json!({"vm_name":"lab-1","name":"lab-base","version":"1.0.0","platform":"linux"}),
                "平台非法",
            ),
            (
                serde_json::json!({"vm_name":"lab-1","name":"lab-base","version":"1.0.0","unknown":1}),
                "未知字段",
            ),
        ] {
            assert!(
                validate_vm_command(request_args("image.make", args)).is_err(),
                "应拒绝：{label}"
            );
        }
    }

    #[test]
    fn downlink_payload_matches_contract() {
        let request_id = Uuid::nil();
        let list = validate_vm_command(request("list", None)).unwrap();
        let list_payload = vm_command_downlink_payload(request_id, &list);
        assert_eq!(list_payload["request_id"], serde_json::json!(request_id));
        assert_eq!(list_payload["op"], serde_json::json!("list"));
        assert_eq!(list_payload["args"], serde_json::json!({}));

        let start = validate_vm_command(request("start", Some("debian12"))).unwrap();
        let start_payload = vm_command_downlink_payload(request_id, &start);
        assert_eq!(start_payload["op"], serde_json::json!("start"));
        assert_eq!(start_payload["args"]["name"], serde_json::json!("debian12"));

        let create = validate_vm_command(request_args(
            "create",
            serde_json::json!({"name":"lab-1","base_volume":"ubuntu-22.04"}),
        ))
        .unwrap();
        let create_payload = vm_command_downlink_payload(request_id, &create);
        assert_eq!(create_payload["op"], serde_json::json!("create"));
        assert_eq!(
            create_payload["args"]["base_volume"],
            serde_json::json!("ubuntu-22.04")
        );
        assert_eq!(create_payload["args"]["vcpus"], serde_json::json!(2));
    }

    #[test]
    fn batch_slot_suffix_is_sanitized() {
        assert_eq!(sanitize_slot_suffix("room-101"), "101");
        assert_eq!(
            sanitize_slot_suffix("w3f-crit-slot-1793663233"),
            "1793663233"
        );
        assert_eq!(sanitize_slot_suffix("room_102"), "102");
        assert_eq!(sanitize_slot_suffix("slot42"), "slot42");
        assert_eq!(sanitize_slot_suffix("    "), "1");
        assert_eq!(sanitize_slot_suffix("a b/c"), "abc");
    }

    #[test]
    fn batch_vm_name_composes_prefix_and_suffix() {
        assert_eq!(batch_vm_name("lab", "room-101"), "lab-101");
        assert_eq!(batch_vm_name("lab", "room-102"), "lab-102");
        assert_eq!(
            batch_vm_name("w3f", "w3f-crit-slot-1793663233"),
            "w3f-1793663233"
        );
        // 总长不超过 64
        let long_prefix = "p".repeat(32);
        let long_slot = format!("room-{}", "9".repeat(60));
        let name = batch_vm_name(&long_prefix, &long_slot);
        assert!(name.chars().count() <= 64, "name={name}");
        assert!(name.starts_with("p"));
        assert!(name.contains('-'));
    }

    fn base_environment_request() -> VmEnvironmentRequest {
        VmEnvironmentRequest {
            name_prefix: "lab".to_owned(),
            base_volume: "ubuntu-22.04".to_owned(),
            vcpus: None,
            memory_mib: None,
            disk_gb: None,
            auto_start: None,
        }
    }

    #[test]
    fn batch_environment_validation_and_args() {
        let request = base_environment_request();
        let env = validate_vm_environment(&request).unwrap();
        assert_eq!(env.vcpus, 2);
        assert_eq!(env.memory_mib, 2048);
        assert_eq!(env.disk_gb, 20);
        assert!(!env.auto_start);

        let args = environment_create_args(&env, "room-101");
        assert_eq!(args["name"], serde_json::json!("lab-101"));
        assert_eq!(args["vcpus"], serde_json::json!(2));
        assert_eq!(args["base_volume"], serde_json::json!("ubuntu-22.04"));
        assert_eq!(args["start"], serde_json::json!(false));

        for (field, request) in [
            (
                "非法 prefix",
                VmEnvironmentRequest {
                    name_prefix: "lab/1".to_owned(),
                    ..base_environment_request()
                },
            ),
            (
                "空 base_volume",
                VmEnvironmentRequest {
                    base_volume: "  ".to_owned(),
                    ..base_environment_request()
                },
            ),
            (
                "vcpus 越界",
                VmEnvironmentRequest {
                    vcpus: Some(65),
                    ..base_environment_request()
                },
            ),
        ] {
            assert!(
                validate_vm_environment(&request).is_err(),
                "应拒绝：{field}"
            );
        }
    }

    #[test]
    fn delete_args_are_validated() {
        let ok = validate_vm_command(request_args(
            "delete",
            serde_json::json!({"name":"lab-1","remove_disks":true}),
        ))
        .unwrap();
        assert_eq!(ok.args["name"], serde_json::json!("lab-1"));
        assert_eq!(ok.args["remove_disks"], serde_json::json!(true));
        // 缺省不删盘
        let ok = validate_vm_command(request_args(
            "delete",
            serde_json::json!({"name":"lab-1"}),
        ))
        .unwrap();
        assert!(ok.args.get("remove_disks").is_none());
        // 未知字段 / 缺 name 拒绝
        assert!(validate_vm_command(request_args(
            "delete",
            serde_json::json!({"name":"lab-1","unknown":1})
        ))
        .is_err());
        assert!(validate_vm_command(request_args("delete", serde_json::json!({}))).is_err());
    }

    #[test]
    fn container_command_args_are_validated() {
        let ok = validate_container_command(request_args(
            "start",
            serde_json::json!({"name":"debian12-cfg"}),
        ))
        .unwrap();
        assert_eq!(ok.op, "start");
        assert_eq!(ok.args["name"], serde_json::json!("debian12-cfg"));
        let ok = validate_container_command(request_args(
            "delete",
            serde_json::json!({"name":"c1","force":true}),
        ))
        .unwrap();
        assert_eq!(ok.args["force"], serde_json::json!(true));
        let ok = validate_container_command(request_args(
            "snapshot.create",
            serde_json::json!({"name":"c1","snapshot":"s1"}),
        ))
        .unwrap();
        assert_eq!(ok.args["snapshot"], serde_json::json!("s1"));
        // list 不接受参数；snapshot 缺 snapshot；未知字段/非法 name 拒绝
        assert!(validate_container_command(request_args("list", serde_json::json!({"x":1}))).is_err());
        assert!(validate_container_command(request_args(
            "snapshot.create",
            serde_json::json!({"name":"c1"})
        ))
        .is_err());
        assert!(validate_container_command(request_args(
            "start",
            serde_json::json!({"name":"c1","unknown":1})
        ))
        .is_err());
        assert!(validate_container_command(request_args(
            "start",
            serde_json::json!({"name":"../x"})
        ))
        .is_err());
    }
}
