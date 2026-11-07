//! 机器总览：设备注册信息 + 房间/区域 + room-agent 最新快照与历史。
//!
//! 数据来源：`device_enrollment` / `device_session` / `rooms` / `areas` /
//! `machine_snapshots`（见 `migrations/0014_machine_telemetry.sql`）。
//! 读权限复用 `device:read`；机器数据属于设备观测面，不引入新的 RBAC 权限点。

use std::{collections::VecDeque, convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    response::sse::{Event, Sse},
};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::time::Instant;
use uuid::Uuid;

use qe_db::machine::{MachineSnapshotMeta, MachineSnapshotRecord, MachineSummary};

use crate::{
    api::extractors::{AuthUser, ClientIp},
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
