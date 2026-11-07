//! VNC 中继（noVNC 阶段 2）：agent mTLS 隧道 ↔ qe-api 内部明文隧道配对。
//!
//! 链路（见 `docs/novnc-design.md` §5）：
//!
//! ```text
//! 浏览器 ⇄ qe-api WS 中继 ⇄ device-pki 内部明文 WS(/internal/vnc/{session})
//!                              ⇄ 配对 ⇄ agent mTLS WS(/vnc/{serial}?session=) ⇄ VM VNC TCP
//! ```
//!
//! - **agent 端**：`GET /vnc/{serial}?session=<id>`（mTLS listener）；证书 serial 必须等于
//!   路径 serial；session id 只允许 `[A-Za-z0-9_-]{16,64}`。
//! - **内部端**：`GET /internal/vnc/{session}?token=...`（内部明文 listener，默认只绑
//!   `127.0.0.1:4442`）；token 与 `INTERNAL_TOKEN` 常量时间比较。
//! - **配对**：同一 session 只允许一对；两端 30s 内完成配对，超时关闭；配成后二进制帧
//!   双向透传（≤2 MiB/帧，文本帧丢弃），空闲 30min 关闭。
//!
//! 配对状态机对 WebSocket 负载做了泛型（生产用 [`VncHub<WebSocket>`]，单测用字符串），
//! 这样无需真实 socket 即可覆盖「先到等待 / 后到交接 / 同端重复 / 过期清理」。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::SinkExt;
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::error::{Error, Result};
use crate::routes::{AppState, resolve_identity};
use crate::tls::PeerCert;

/// 两端配对超时：agent 与 qe-api 内部端必须在 30s 内都连上。
pub const VNC_PAIR_TIMEOUT: Duration = Duration::from_secs(30);
/// 配对成功后的空闲超时（30 分钟）。
pub const VNC_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 单帧上限 2 MiB（noVNC RFB 帧通常远小于此，防止大帧占满内存）。
pub const VNC_MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// 校验 session id：`[A-Za-z0-9_-]{16,64}`。
pub fn validate_session_id(raw: &str) -> Result<String> {
    let session = raw.trim();
    let length = session.chars().count();
    if !(16..=64).contains(&length) {
        return Err(Error::BadRequest("session 长度必须在 16..=64".into()));
    }
    if !session
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err(Error::BadRequest(
            "session 只允许字母、数字、下划线与短横线".into(),
        ));
    }
    Ok(session.to_owned())
}

/// 常量时间比较（内部 token 不应因比较耗时泄漏前缀）。
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// 隧道端点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VncSide {
    /// agent 端（mTLS listener）。
    Agent,
    /// qe-api 内部端（明文 listener）。
    Internal,
}

/// 配对失败原因（同端重复连接）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VncReserveError {
    /// 同一 session 已存在同端连接（一个 session 只允许一对）。
    Duplicate,
}

struct Waiting<T> {
    side: VncSide,
    created_at: Instant,
    tx: oneshot::Sender<T>,
}

/// 配对结果：先到的一端 `Wait` 等对端；后到的一端 `Handoff` 把自己交给等待方。
pub enum VncReservation<T> {
    Wait(oneshot::Receiver<T>),
    Handoff(oneshot::Sender<T>),
}

/// session 配对表（进程内，单实例）。
pub struct VncHub<T> {
    inner: Mutex<HashMap<String, Waiting<T>>>,
    pair_timeout: Duration,
}

impl<T> VncHub<T> {
    pub fn new(pair_timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            pair_timeout,
        }
    }

    pub fn pair_timeout(&self) -> Duration {
        self.pair_timeout
    }

    /// 登记一侧连接：表为空 → `Wait`；已有对端 → 取出并 `Handoff`；同端重复 → `Duplicate`。
    ///
    /// 过期的等待项（超过 pair_timeout，对端始终没来）会在每次登记时顺手清理。
    pub fn reserve(
        &self,
        session: &str,
        side: VncSide,
    ) -> std::result::Result<VncReservation<T>, VncReserveError> {
        let now = Instant::now();
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.retain(|_, waiting| now.duration_since(waiting.created_at) < self.pair_timeout);

        if let Some(waiting) = map.remove(session) {
            if waiting.side == side {
                // 同端重复：把原等待项放回去，新连接拒绝。
                map.insert(session.to_owned(), waiting);
                return Err(VncReserveError::Duplicate);
            }
            return Ok(VncReservation::Handoff(waiting.tx));
        }

        let (tx, rx) = oneshot::channel();
        map.insert(
            session.to_owned(),
            Waiting {
                side,
                created_at: now,
                tx,
            },
        );
        Ok(VncReservation::Wait(rx))
    }

    /// 当前等待配对的 session 数（测试/排障用）。
    pub fn pending(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// 关闭 socket（忽略错误）。
async fn close_socket(mut socket: WebSocket) {
    let _ = socket.close().await;
}

// ---------------------------------------------------------------------------
// 内部 listener 鉴权中间件
// ---------------------------------------------------------------------------

/// 内部明文 listener 的 token 鉴权（在 WebSocketUpgrade extractor 之前执行，
/// 保证 token 错误稳定返回 403，而不是先被 WS extractor 的 400 覆盖）。
pub async fn require_internal_token(
    State(st): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response> {
    let expected = st
        .vnc_internal_token
        .as_deref()
        .ok_or_else(|| Error::Forbidden("内部 VNC 监听未启用".into()))?;
    let query = request.uri().query().unwrap_or_default();
    let provided = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .unwrap_or_default();
    if provided.is_empty() || !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(Error::Forbidden("内部 token 无效".into()));
    }
    Ok(next.run(request).await)
}

// ---------------------------------------------------------------------------
// 两端 handler
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AgentTunnelQuery {
    pub session: String,
}

/// `GET /vnc/{serial}?session=`（mTLS listener）—— agent 发起 VNC 隧道。
pub async fn agent_tunnel(
    State(st): State<AppState>,
    peer: PeerCert,
    Path(serial): Path<String>,
    Query(query): Query<AgentTunnelQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    // 存活注册校验（吊销/未激活设备在 resolve_identity 里 fail-closed）。
    resolve_identity(&st, &peer).await?;

    let serial = serial.trim();
    if serial.is_empty() {
        return Err(Error::BadRequest("serial 不能为空".into()));
    }
    let peer_serial = peer.serial_hex.as_deref().ok_or(Error::Unauthorized)?;
    if !peer_serial.eq_ignore_ascii_case(serial) {
        return Err(Error::Forbidden("证书 serial 与路径 serial 不一致".into()));
    }
    let session = validate_session_id(&query.session)?;

    match st.vnc.reserve(&session, VncSide::Agent) {
        Ok(VncReservation::Wait(rx)) => {
            let timeout = st.vnc.pair_timeout();
            Ok(ws
                .on_upgrade(move |socket| async move {
                    match tokio::time::timeout(timeout, rx).await {
                        Ok(Ok(internal)) => relay(socket, internal, &session).await,
                        _ => {
                            tracing::warn!(%session, "VNC agent 端等待内部端配对超时");
                            close_socket(socket).await;
                        }
                    }
                })
                .into_response())
        }
        Ok(VncReservation::Handoff(tx)) => Ok(ws
            .on_upgrade(move |socket| async move {
                if let Err(socket) = tx.send(socket) {
                    close_socket(socket).await;
                }
            })
            .into_response()),
        Err(VncReserveError::Duplicate) => {
            Err(Error::Conflict("同一 session 已存在 agent 端连接".into()))
        }
    }
}

/// `GET /internal/vnc/{session}?token=`（内部明文 listener）—— qe-api 发起中继。
pub async fn internal_tunnel(
    State(st): State<AppState>,
    Path(session): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    let session = validate_session_id(&session)?;

    match st.vnc.reserve(&session, VncSide::Internal) {
        Ok(VncReservation::Wait(rx)) => {
            let timeout = st.vnc.pair_timeout();
            Ok(ws
                .on_upgrade(move |socket| async move {
                    match tokio::time::timeout(timeout, rx).await {
                        Ok(Ok(agent)) => relay(agent, socket, &session).await,
                        _ => {
                            tracing::warn!(%session, "VNC 内部端等待 agent 端配对超时");
                            close_socket(socket).await;
                        }
                    }
                })
                .into_response())
        }
        Ok(VncReservation::Handoff(tx)) => Ok(ws
            .on_upgrade(move |socket| async move {
                if let Err(socket) = tx.send(socket) {
                    close_socket(socket).await;
                }
            })
            .into_response()),
        Err(VncReserveError::Duplicate) => {
            Err(Error::Conflict("同一 session 已存在内部端连接".into()))
        }
    }
}

/// 内部明文监听路由（main 单独起一个 listener；默认只绑 127.0.0.1）。
pub fn internal_router(st: AppState) -> Router {
    let token_state = st.clone();
    Router::new()
        .route("/internal/vnc/{session}", get(internal_tunnel))
        .layer(axum::middleware::from_fn_with_state(
            token_state,
            require_internal_token,
        ))
        .with_state(st)
}

// ---------------------------------------------------------------------------
// 配对后的二进制帧中继
// ---------------------------------------------------------------------------

/// 一帧的转发结果。
enum ForwardOutcome {
    Sent,
    Skip,
    Close,
}

/// 双向中继：只透传二进制帧（RFB 走二进制）；文本/控制帧丢弃；
/// 帧 > [`VNC_MAX_FRAME_BYTES`] 或任一端关闭即结束并关闭另一端。
async fn relay(mut agent: WebSocket, mut internal: WebSocket, session: &str) {
    let mut agent_bytes: u64 = 0;
    let mut internal_bytes: u64 = 0;
    loop {
        let idle = tokio::time::sleep(VNC_IDLE_TIMEOUT);
        tokio::pin!(idle);
        tokio::select! {
            message = agent.recv() => {
                match message {
                    Some(Ok(message)) => match forward(message, &mut internal, &mut agent_bytes).await {
                        ForwardOutcome::Sent | ForwardOutcome::Skip => {}
                        ForwardOutcome::Close => break,
                    },
                    _ => break,
                }
            }
            message = internal.recv() => {
                match message {
                    Some(Ok(message)) => match forward(message, &mut agent, &mut internal_bytes).await {
                        ForwardOutcome::Sent | ForwardOutcome::Skip => {}
                        ForwardOutcome::Close => break,
                    },
                    _ => break,
                }
            }
            _ = &mut idle => {
                tracing::info!(%session, "VNC 中继空闲超时（30min），关闭");
                break;
            }
        }
    }
    close_socket(agent).await;
    close_socket(internal).await;
    tracing::info!(%session, agent_bytes, internal_bytes, "VNC 中继结束");
}

async fn forward(message: Message, target: &mut WebSocket, bytes: &mut u64) -> ForwardOutcome {
    match message {
        Message::Binary(data) => {
            if data.len() > VNC_MAX_FRAME_BYTES {
                tracing::warn!(
                    frame_bytes = data.len(),
                    limit = VNC_MAX_FRAME_BYTES,
                    "VNC 帧超过上限，关闭隧道"
                );
                return ForwardOutcome::Close;
            }
            *bytes += data.len() as u64;
            if target.send(Message::Binary(data)).await.is_err() {
                return ForwardOutcome::Close;
            }
            ForwardOutcome::Sent
        }
        // noVNC 的 RFB 数据只走二进制帧；文本帧没有意义，直接丢弃。
        Message::Text(_) => ForwardOutcome::Skip,
        // Ping/Pong 由 axum/tungstenite 自动处理，不需要手动转发。
        Message::Ping(_) | Message::Pong(_) => ForwardOutcome::Skip,
        Message::Close(_) => ForwardOutcome::Close,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> VncHub<String> {
        VncHub::new(Duration::from_secs(30))
    }

    #[test]
    fn session_id_validation() {
        assert!(validate_session_id("abcdef0123456789").is_ok());
        assert!(validate_session_id("A-Z_09-abcdefgh1").is_ok());
        assert!(validate_session_id("short").is_err());
        assert!(validate_session_id(&"a".repeat(65)).is_err());
        assert!(validate_session_id("bad session id 1234").is_err());
        assert!(validate_session_id("../etc/passwd-1234").is_err());
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq(b"token-123", b"token-123"));
        assert!(!constant_time_eq(b"token-123", b"token-124"));
        assert!(!constant_time_eq(b"token", b"token-123"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[tokio::test]
    async fn first_side_waits_and_second_side_hands_off() {
        let hub = hub();
        let VncReservation::Wait(rx) = hub.reserve("session-abcdefgh", VncSide::Agent).unwrap()
        else {
            panic!("agent 端应先等待");
        };
        assert_eq!(hub.pending(), 1);
        let VncReservation::Handoff(tx) =
            hub.reserve("session-abcdefgh", VncSide::Internal).unwrap()
        else {
            panic!("内部端应交接给等待方");
        };
        assert_eq!(hub.pending(), 0, "配对成功后表项应移除");
        tx.send("internal-socket".to_owned()).unwrap();
        assert_eq!(rx.await.unwrap(), "internal-socket");
    }

    #[tokio::test]
    async fn duplicate_same_side_is_rejected() {
        let hub = hub();
        assert!(matches!(
            hub.reserve("session-abcdefgh", VncSide::Agent).unwrap(),
            VncReservation::Wait(_)
        ));
        assert!(matches!(
            hub.reserve("session-abcdefgh", VncSide::Agent),
            Err(VncReserveError::Duplicate)
        ));
        // 对端仍然可以正常配对。
        assert!(matches!(
            hub.reserve("session-abcdefgh", VncSide::Internal).unwrap(),
            VncReservation::Handoff(_)
        ));
    }

    #[tokio::test]
    async fn independent_sessions_do_not_cross_pair() {
        let hub = hub();
        let VncReservation::Wait(_a) = hub.reserve("session-aaaaaaaa", VncSide::Agent).unwrap()
        else {
            panic!()
        };
        let VncReservation::Wait(_b) = hub.reserve("session-bbbbbbbb", VncSide::Internal).unwrap()
        else {
            panic!()
        };
        assert_eq!(hub.pending(), 2);
        assert!(matches!(
            hub.reserve("session-aaaaaaaa", VncSide::Internal).unwrap(),
            VncReservation::Handoff(_)
        ));
        assert!(matches!(
            hub.reserve("session-bbbbbbbb", VncSide::Agent).unwrap(),
            VncReservation::Handoff(_)
        ));
        assert_eq!(hub.pending(), 0);
    }

    #[tokio::test]
    async fn stale_waiting_side_is_cleaned_up() {
        let hub = VncHub::<String>::new(Duration::from_millis(5));
        let VncReservation::Wait(rx) = hub.reserve("session-cccccccc", VncSide::Agent).unwrap()
        else {
            panic!()
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        // 第二次登记会清掉过期表项，重新等待（不会错误配对）。
        let VncReservation::Wait(_rx2) =
            hub.reserve("session-cccccccc", VncSide::Internal).unwrap()
        else {
            panic!("过期表项应被清理后重新等待");
        };
        assert!(rx.await.is_err(), "旧的等待方应收到取消");
    }
}
