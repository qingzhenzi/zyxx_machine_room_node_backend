//! 毫秒级实时指标 SSE：浏览器 ⇄ qe-api ⇄ device-pki 内部 WS（内存广播）。
//!
//! 不写数据库；历史快照仍由 `/machine/report` 负责。浏览器用 fetch 流式读取
//! （EventSource 不能带 Authorization，和既有机型快照流一致）。
//!
//! SSE 事件：`metric`（实时样本）、`op`（长操作进度）、`command`（命令结果）、`heartbeat`。
//!
//! **按需推流（2026-11-14）**：agent 默认不在后台持续 POST 毫秒指标。
//! 浏览器订阅本 SSE 时，qe-api 通过 `device_downlink` 下发 `telemetry.live`
//! `{enabled:true, interval_ms, expires_at_ms}`，agent 在 TTL 内按请求频率推送；
//! SSE 存活期间每 20s 续期一次，浏览器断开后 agent 最多 30s 自动停止。
//! 老 agent 不认这个下行 kind（会 ack 跳过），行为与之前一致（只在显式开启
//! `ROOM_AGENT_LIVE_PUSH=1` 时持续推流）。

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{SinkExt, StreamExt, stream};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::Message;

use crate::api::extractors::AuthUser;
use crate::auth::authorization;
use crate::domain::rbac::permissions;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// 实时样本默认间隔（毫秒）。
const LIVE_DEFAULT_INTERVAL_MS: u64 = 1000;
/// 允许的最小间隔（与 agent 采样下限一致，避免压垮设备与中转）。
const LIVE_MIN_INTERVAL_MS: u64 = 200;
/// 允许的最大间隔。
const LIVE_MAX_INTERVAL_MS: u64 = 5000;
/// agent 侧需求 TTL：浏览器断开后最多这么久停止推流。
const LIVE_TTL_MS: i64 = 30_000;
/// SSE 存活期间续期间隔（小于 TTL，留出一次重试余量）。
const LIVE_RENEW_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug, Deserialize)]
pub struct LiveMetricsQuery {
    /// 期望采样/推送间隔（毫秒）；缺省 1000。
    #[serde(default)]
    pub interval_ms: Option<u64>,
}

fn parse_interval_ms(raw: Option<u64>) -> AppResult<u64> {
    let interval = raw.unwrap_or(LIVE_DEFAULT_INTERVAL_MS);
    if !(LIVE_MIN_INTERVAL_MS..=LIVE_MAX_INTERVAL_MS).contains(&interval) {
        return Err(AppError::bad_request(format!(
            "interval_ms 需在 {LIVE_MIN_INTERVAL_MS}-{LIVE_MAX_INTERVAL_MS} 之间"
        )));
    }
    Ok(interval)
}

fn metrics_ws_url(base: &str, serial: &str, token: &str, interval_ms: u64) -> AppResult<String> {
    if token.is_empty() {
        return Err(AppError::service_unavailable(
            "未配置 DEVICE_PKI_INTERNAL_TOKEN，实时指标中转不可用",
        ));
    }
    if serial.is_empty()
        || serial.len() > 128
        || !serial.chars().all(|ch| ch.is_ascii_alphanumeric())
    {
        return Err(AppError::bad_request("serial 不合法"));
    }
    let base = base.trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_owned()
    } else {
        return Err(AppError::internal(
            "DEVICE_PKI_INTERNAL_URL 必须是 http(s):// 或 ws(s)://",
        ));
    };
    Ok(format!(
        "{ws_base}/internal/metrics/{serial}?token={token}&interval_ms={interval_ms}"
    ))
}

/// 下发一次 `telemetry.live` 需求（含 TTL，agent 按 `expires_at_ms` 自动停止）。
///
/// 失败只告警：实时中转不可用时不应让整个 SSE 报错（前端会显示等待数据）。
async fn request_live_demand(state: &AppState, slot: &str, interval_ms: u64) {
    let payload = serde_json::json!({
        "enabled": true,
        "interval_ms": interval_ms,
        "expires_at_ms": chrono::Utc::now().timestamp_millis() + LIVE_TTL_MS,
    });
    if let Err(error) =
        super::machines::enqueue_device_downlink(state, slot, "telemetry.live", &payload).await
    {
        tracing::warn!(%slot, %error, "下发实时指标需求失败（agent 可能不支持按需推流）");
    }
}

/// `GET /api/v1/admin/machines/{serial}/metrics/stream` —— SSE 实时指标。
pub async fn stream(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
    Query(query): Query<LiveMetricsQuery>,
) -> AppResult<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;
    let serial = serial.trim().to_owned();
    let interval_ms = parse_interval_ms(query.interval_ms)?;
    let token = state
        .config
        .device_pki_internal_token
        .as_deref()
        .unwrap_or_default();
    let url = metrics_ws_url(
        &state.config.device_pki_internal_url,
        &serial,
        token,
        interval_ms,
    )?;

    // 解析 device_slot：按需推流需要知道把 `telemetry.live` 发给哪个槽位。
    // 机器不存在直接 404，避免为一个不存在的 serial 建立 WS 后空等。
    let device = state
        .machines
        .summary(&serial)
        .await?
        .ok_or_else(|| AppError::not_found("机器不存在"))?;
    request_live_demand(&state, &device.device_slot, interval_ms).await;

    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|error| AppError::service_unavailable(format!("连接实时指标中转失败: {error}")))?;

    let demand = LiveDemand {
        state: state.clone(),
        slot: device.device_slot,
        interval_ms,
    };
    let stream = stream::unfold(
        (socket, false, demand, tokio::time::Instant::now()),
        |(mut socket, done, demand, last_renew)| async move {
            if done {
                return None;
            }
            loop {
                // 续期需求：SSE 仍存活就继续让 agent 推流（TTL 内自动停止）。
                if last_renew.elapsed() >= LIVE_RENEW_INTERVAL {
                    demand.renew().await;
                    return Some((
                        Ok(Event::default().event("heartbeat").data("{}")),
                        (socket, false, demand, tokio::time::Instant::now()),
                    ));
                }
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let text = text.to_string();
                        // 同一条中转既承载实时样本，也承载长操作进度（关机/镜像等）。
                        let event = if text.contains("\"type\":\"heartbeat\"") {
                            Event::default().event("heartbeat").data("{}")
                        } else if text.contains("\"type\":\"op_progress\"") {
                            Event::default().event("op").data(text)
                        } else if text.contains("\"type\":\"command_result\"") {
                            Event::default().event("command").data(text)
                        } else {
                            Event::default().event("metric").data(text)
                        };
                        return Some((Ok(event), (socket, false, demand, last_renew)));
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = socket.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => return None,
                    Some(Err(_)) => return None,
                    _ => continue,
                }
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// SSE 存续期间的需求续期句柄。
struct LiveDemand {
    state: AppState,
    slot: String,
    interval_ms: u64,
}

impl LiveDemand {
    async fn renew(&self) {
        request_live_demand(&self.state, &self.slot, self.interval_ms).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_internal_metrics_url() {
        assert_eq!(
            metrics_ws_url("http://127.0.0.1:4442", "abc123", "tok", 1000).unwrap(),
            "ws://127.0.0.1:4442/internal/metrics/abc123?token=tok&interval_ms=1000"
        );
        assert_eq!(
            metrics_ws_url("https://pki:4442/", "abc123", "tok", 200).unwrap(),
            "wss://pki:4442/internal/metrics/abc123?token=tok&interval_ms=200"
        );
    }

    #[test]
    fn rejects_bad_serial_or_missing_token() {
        assert!(metrics_ws_url("http://127.0.0.1:4442", "../x", "tok", 1000).is_err());
        assert!(metrics_ws_url("http://127.0.0.1:4442", "abc", "", 1000).is_err());
    }

    #[test]
    fn parses_interval_range() {
        assert_eq!(parse_interval_ms(None).unwrap(), LIVE_DEFAULT_INTERVAL_MS);
        assert_eq!(parse_interval_ms(Some(200)).unwrap(), 200);
        assert_eq!(parse_interval_ms(Some(5000)).unwrap(), 5000);
        assert!(parse_interval_ms(Some(199)).is_err());
        assert!(parse_interval_ms(Some(5001)).is_err());
    }
}
