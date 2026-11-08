//! 毫秒级实时指标中转：agent mTLS HTTP POST → device-pki 内存广播 → qe-api 内部 WS → 浏览器。
//!
//! 不逐样本写 DB：历史仍由 `/machine/report` 快照负责；这里只做低延迟转发。
//! - `POST /metrics`（mTLS）：agent 每 200–500ms 推一个小 JSON，证书身份即 serial。
//! - `GET /internal/metrics/{serial}`（内部明文 WS，token 校验）：qe-api 订阅串流。
//!
//! 广播按 serial 分频道；没有订阅者时只保留 latest，供新订阅者立即拿到一帧。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::sync::broadcast;
use tokio::time::{Duration, interval};

use crate::error::{Error, Result};
use crate::routes::AppState;
use crate::tls::PeerCert;

/// 单条实时指标 JSON 上限，防止异常客户端占满内存。
pub const MAX_SAMPLE_BYTES: usize = 16 * 1024;
const CHANNEL_CAPACITY: usize = 64;
const HEARTBEAT: Duration = Duration::from_secs(15);

/// 每个 serial 的最新样本 + 广播频道。
#[derive(Default)]
pub struct MetricsHub {
    latest: RwLock<HashMap<String, Arc<str>>>,
    channels: RwLock<HashMap<String, broadcast::Sender<Arc<str>>>>,
}

impl MetricsHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// 发布一条样本（调用方已校验 JSON 大小）。
    pub fn publish(&self, serial: &str, payload: Arc<str>) {
        self.latest
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(serial.to_owned(), payload.clone());
        let sender = self
            .channels
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(serial)
            .cloned();
        if let Some(sender) = sender {
            let _ = sender.send(payload);
        }
    }

    /// 订阅 serial；返回最新样本 + 后续广播。
    pub fn subscribe(&self, serial: &str) -> (Option<Arc<str>>, broadcast::Receiver<Arc<str>>) {
        let mut channels = self.channels.write().unwrap_or_else(|e| e.into_inner());
        let sender = channels
            .entry(serial.to_owned())
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .clone();
        let latest = self
            .latest
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(serial)
            .cloned();
        (latest, sender.subscribe())
    }
}

/// `POST /metrics`（mTLS）：接收 agent 实时样本并广播。
pub async fn ingest(
    State(st): State<AppState>,
    peer: PeerCert,
    body: String,
) -> Result<StatusCode> {
    if body.len() > MAX_SAMPLE_BYTES {
        return Err(Error::BadRequest("实时指标样本过大".into()));
    }
    let serial = peer
        .serial_hex
        .as_deref()
        .ok_or(Error::Unauthorized)?;
    // 只接受合法 JSON，避免浏览器收到坏帧。
    serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|_| Error::BadRequest("实时指标必须是 JSON".into()))?;
    st.metrics.publish(serial, Arc::from(body));
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, serde::Deserialize)]
pub struct InternalQuery {
    #[serde(default)]
    pub token: String,
}

/// 内部明文 WS：`GET /internal/metrics/{serial}?token=...`，qe-api 订阅。
pub async fn internal_stream(
    State(st): State<AppState>,
    Path(serial): Path<String>,
    Query(_query): Query<InternalQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    let serial = serial.trim();
    if serial.is_empty() || serial.len() > 128 || !serial.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(Error::BadRequest("serial 非法".into()));
    }
    let hub = Arc::clone(&st.metrics);
    let serial = serial.to_owned();
    Ok(ws
        .on_upgrade(move |socket| stream_socket(socket, hub, serial))
        .into_response())
}

async fn stream_socket(mut socket: WebSocket, hub: Arc<MetricsHub>, serial: String) {
    let (latest, mut receiver) = hub.subscribe(&serial);
    if let Some(latest) = latest {
        if socket.send(Message::Text(latest.to_string().into())).await.is_err() {
            return;
        }
    }
    let mut heartbeat = interval(HEARTBEAT);
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if socket.send(Message::Text("{\"type\":\"heartbeat\"}".into())).await.is_err() {
                    break;
                }
            }
            message = receiver.recv() => {
                match message {
                    Ok(sample) => {
                        if socket.send(Message::Text(sample.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

/// 内部 listener 增加实时指标 WS 路由（外层已有 token middleware）。
pub fn internal_router(st: AppState) -> Router {
    let token_state = st.clone();
    Router::new()
        .route("/internal/metrics/{serial}", get(internal_stream))
        .layer(axum::middleware::from_fn_with_state(
            token_state,
            crate::vnc::require_internal_token,
        ))
        .with_state(st)
}
