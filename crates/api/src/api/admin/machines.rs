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
    domain::{models::Page, rbac::permissions, validation, wol},
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
// ---------------------------------------------------------------------------

/// 下发 VM 命令的 device_downlink kind（与 room-agent 侧契约一致）。
const VM_COMMAND_DOWNLINK_KIND: &str = "vm.command";
/// pending 超过该秒数视为超时（GET 时顺手把表更新为 timeout）。
const VM_COMMAND_PENDING_TIMEOUT_SECS: i64 = 120;
/// 允许的 VM 操作白名单。`list` 不需要 name，其它命令必须带非空 name。
const VM_COMMAND_OPS: &[&str] = &[
    "list",
    "start",
    "shutdown",
    "reboot",
    "destroy",
    "screenshot",
];

/// `POST /admin/machines/{serial}/vm/commands` 请求体。
#[derive(Debug, Deserialize)]
pub struct VmCommandRequest {
    /// 操作：list / start / shutdown / reboot / destroy / screenshot。
    pub op: String,
    /// 虚拟机名；list 不传，其它命令必填。
    #[serde(default)]
    pub name: Option<String>,
}

/// 校验后的 VM 命令。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedVmCommand {
    op: String,
    name: Option<String>,
}

/// 202 响应：只回 request_id，前端据此轮询结果。
#[derive(Debug, Serialize)]
pub struct VmCommandAccepted {
    pub request_id: Uuid,
}

/// GET 结果响应（`data` 为 agent 回传的 result；截图时含 image_base64）。
#[derive(Debug, Serialize)]
pub struct VmCommandReply {
    pub request_id: Uuid,
    pub status: String,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `machine_command_results` 行（表由迁移 0031 创建，T1 负责）。
#[derive(Debug, sqlx::FromRow)]
struct MachineCommandRow {
    request_id: Uuid,
    status: String,
    result: Option<serde_json::Value>,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// 校验 op 白名单与 name 约束（纯函数，便于单测）。
///
/// - `list`：不接受 name（传了非空 name 视为 400）；
/// - `start/shutdown/reboot/destroy/screenshot`：必须带非空 name（≤128 字符）。
fn validate_vm_command(request: VmCommandRequest) -> AppResult<ValidatedVmCommand> {
    let op = request.op.trim().to_ascii_lowercase();
    if !VM_COMMAND_OPS.contains(&op.as_str()) {
        return Err(AppError::bad_request(format!(
            "op 只能是 {}",
            VM_COMMAND_OPS.join(" / ")
        )));
    }
    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if op == "list" {
        if name.is_some() {
            return Err(AppError::bad_request("list 命令不接受 name"));
        }
        return Ok(ValidatedVmCommand { op, name: None });
    }
    let Some(name) = name else {
        return Err(AppError::bad_request(format!("{op} 命令必须提供非空 name")));
    };
    if name.chars().count() > 128 {
        return Err(AppError::bad_request("name 长度不能超过 128 字符"));
    }
    Ok(ValidatedVmCommand {
        op,
        name: Some(name.to_owned()),
    })
}

/// 构造 `vm.command` 下行 payload：`{"request_id","op","args":{"name"}}`。
///
/// `list` 的 args 为空对象（不带 name）。
fn vm_command_downlink_payload(
    request_id: Uuid,
    command: &ValidatedVmCommand,
) -> serde_json::Value {
    let args = match command.name.as_deref() {
        Some(name) => serde_json::json!({ "name": name }),
        None => serde_json::json!({}),
    };
    serde_json::json!({
        "request_id": request_id,
        "op": command.op,
        "args": args,
    })
}

/// `POST /api/v1/admin/machines/{serial}/vm/commands` —— 下发 VM 远程控制命令。
///
/// 权限 `vm:control`；设备不存在 404；op/name 非法 400。
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
    sqlx::query(
        "INSERT INTO machine_command_results (request_id, serial, kind, payload, status)
         VALUES ($1, $2, $3, $4, 'pending')",
    )
    .bind(request_id)
    .bind(&device.serial)
    .bind(VM_COMMAND_DOWNLINK_KIND)
    .bind(&downlink)
    .execute(state.db.pool())
    .await
    .map_err(|error| AppError::internal(format!("写入 VM 命令失败: {error}")))?;

    let downlink_text = downlink.to_string();
    match state
        .storage_repo
        .enqueue_downlink_for_slot(
            &device.device_slot,
            VM_COMMAND_DOWNLINK_KIND,
            &downlink_text,
        )
        .await
    {
        Ok(Some(device_serial)) => {
            // 唤醒长连接（跨进程尽力而为；失败还有 2s 兜底轮询）。
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
            Ok((StatusCode::ACCEPTED, Json(VmCommandAccepted { request_id })))
        }
        Ok(None) => {
            cleanup_pending_vm_command(&state, request_id).await;
            Err(AppError::service_unavailable(
                "设备当前没有存活证书，VM 命令未下发",
            ))
        }
        Err(error) => {
            cleanup_pending_vm_command(&state, request_id).await;
            Err(error.into())
        }
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
        "SELECT request_id, status, result, error, created_at, updated_at
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
    if status == "pending"
        && row.created_at + chrono::Duration::seconds(VM_COMMAND_PENDING_TIMEOUT_SECS) < Utc::now()
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
        }
    }

    #[test]
    fn vm_ops_whitelist_is_enforced() {
        for op in [
            "list",
            "start",
            "shutdown",
            "reboot",
            "destroy",
            "screenshot",
        ] {
            let name = if op == "list" { None } else { Some("debian12") };
            assert!(validate_vm_command(request(op, name)).is_ok(), "op={op}");
        }
        for op in ["exec", "rm-rf", ""] {
            assert!(
                validate_vm_command(request(op, Some("debian12"))).is_err(),
                "应拒绝 op={op}"
            );
        }
    }

    #[test]
    fn vm_command_name_rules_are_enforced() {
        // list 不接受 name
        assert!(validate_vm_command(request("list", Some("debian12"))).is_err());
        assert!(validate_vm_command(request("list", None)).is_ok());
        // 其它命令必须有非空 name
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
        // 超长 name
        assert!(validate_vm_command(request("start", Some(&"x".repeat(129)))).is_err());
    }

    #[test]
    fn vm_command_is_normalized_to_lowercase() {
        let command = validate_vm_command(request(" START ", Some(" debian12 "))).unwrap();
        assert_eq!(command.op, "start");
        assert_eq!(command.name.as_deref(), Some("debian12"));
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
    }
}
