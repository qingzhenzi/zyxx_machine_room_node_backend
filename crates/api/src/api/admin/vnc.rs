//! noVNC 阶段 2：VNC 控制台会话与浏览器 WebSocket 代理（设计见 `docs/novnc-design.md` §5）。
//!
//! ```text
//! 浏览器 ——WS(/vnc/ws?session&ticket)——> qe-api 本模块 ——WS(内部, token)——> device-pki
//!   ——配对——> agent mTLS WS ——> VM 的 libvirt VNC TCP
//! ```
//!
//! - `POST /admin/machines/{serial}/vnc/session`（`vm:control`）：下发 `vm.command vnc.info`，
//!   最多等 5s 拿 port/password，生成一次性 session/ticket（TTL 60s）。
//! - `GET /admin/machines/{serial}/vnc/ws?session=&ticket=`：浏览器没有 JWT header，
//!   用一次性 ticket 鉴权；先通知 agent 建立隧道（downlink `vnc.open`），再连 device-pki
//!   内部中继，随后双向转发二进制帧（RFB）。

use std::time::Duration;

use axum::Json;
use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::machines;
use crate::{
    api::extractors::{ApiJson, AuthUser},
    auth::authorization,
    domain::rbac::permissions,
    error::{AppError, AppResult},
    state::AppState,
    vnc::{self, VncSession},
};

/// 单帧上限（与 device-pki 中继一致）。
const VNC_MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
/// 等 agent 返回 `vnc.info` 的上限。
const VNC_INFO_WAIT: Duration = Duration::from_secs(5);
/// 连接 device-pki 内部中继的超时。
const INTERNAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 下发给 agent 的 VNC 隧道打开命令 kind。
const VNC_OPEN_DOWNLINK_KIND: &str = "vnc.open";

#[derive(Debug, Deserialize)]
pub struct VncSessionRequest {
    /// VM 名（libvirt domain name）。
    pub vm: String,
}

#[derive(Debug, Serialize)]
pub struct VncSessionReply {
    pub session_id: String,
    pub ticket: String,
    pub vm: String,
    pub password: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct VncWsQuery {
    pub session: String,
    pub ticket: String,
}

/// 从 `vm.command vnc.info` 的 result 中解析 host/port/password。
///
/// agent 回传契约（设计 §5）：`{host?, port, password}`；兼容 `vnc_port`/`vnc_password` 别名。
fn parse_vnc_info(data: Option<&serde_json::Value>) -> AppResult<(String, u16, String)> {
    let root = data
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::bad_gateway("机器没有返回 vnc.info 结果"))?;
    let port = root
        .get("port")
        .or_else(|| root.get("vnc_port"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|port| *port > 0)
        .ok_or_else(|| AppError::bad_gateway("机器未返回有效的 VNC 端口"))?;
    // VNC 密码可以为空（libvirt 默认只监听回环且常常不设密码）：noVNC 用空字符串走
    // None 认证即可，不能把“没有密码”当成机器错误。
    let password = root
        .get("password")
        .or_else(|| root.get("vnc_password"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let host = root
        .get("host")
        .or_else(|| root.get("vnc_host"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("127.0.0.1");
    Ok((host.to_owned(), port, password.to_owned()))
}

/// `POST /api/v1/admin/machines/{serial}/vnc/session` —— 创建 VNC 控制台会话。
///
/// 权限 `vm:control`；下发 `vnc.info` 拿 port/password（≤5s），返回一次性 session/ticket。
pub async fn create_session(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
    ApiJson(request): ApiJson<VncSessionRequest>,
) -> AppResult<Json<VncSessionReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;

    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }
    let vm = machines::validate_name_token(request.vm.trim(), "vm", 64)?;
    let device = state
        .machines
        .summary(serial)
        .await?
        .ok_or_else(|| AppError::not_found("机器不存在"))?;

    let reply = machines::execute_vm_command(
        &state,
        &device.serial,
        &device.device_slot,
        "vnc.info",
        serde_json::json!({ "name": vm }),
        VNC_INFO_WAIT,
    )
    .await?;
    if reply.status != "ok" {
        let detail = reply.error.unwrap_or_else(|| "机器返回未知错误".to_owned());
        return Err(AppError::bad_gateway(format!("vnc.info 失败：{detail}")));
    }
    let (host, port, password) = parse_vnc_info(reply.data.as_ref())?;

    let now = Utc::now();
    let session_id = vnc::new_session_id();
    let ticket = vnc::new_ticket();
    let expires_at = vnc::session_expiry(now);
    state.vnc_sessions.insert(
        &session_id,
        &ticket,
        VncSession {
            serial: device.serial.clone(),
            vm: vm.clone(),
            password: password.clone(),
            created_at: now,
            expires_at,
        },
    );

    tracing::info!(
        %serial,
        %session_id,
        vm = %vm,
        host = %host,
        port,
        "已创建 VNC 控制台会话（ticket TTL 60s）"
    );

    Ok(Json(VncSessionReply {
        session_id,
        ticket,
        vm,
        password,
        expires_at,
    }))
}

/// 连接 device-pki 内部中继；失败映射为 502（浏览器会收到 WS 升级失败）。
async fn connect_internal(url: &str) -> AppResult<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    match tokio::time::timeout(INTERNAL_CONNECT_TIMEOUT, connect_async(url)).await {
        Err(_) => Err(AppError::bad_gateway(
            "连接 device-pki VNC 中继超时（agent 可能未连上）",
        )),
        Ok(Err(error)) => Err(AppError::bad_gateway(format!(
            "连接 device-pki VNC 中继失败: {error}"
        ))),
        Ok(Ok((stream, response))) => {
            tracing::debug!(status = %response.status(), "device-pki VNC 中继已连接");
            Ok(stream)
        }
    }
}

/// `GET /api/v1/admin/machines/{serial}/vnc/ws?session=&ticket=` —— 浏览器 VNC WS 代理。
///
/// ticket 一次性（TTL 60s）；成功后先下发 `vnc.open` 让 agent 连 device-pki，
/// 再由 qe-api 连内部中继，随后双向转发二进制帧。
pub async fn ws_proxy(
    State(state): State<AppState>,
    Path(serial): Path<String>,
    Query(query): Query<VncWsQuery>,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    let serial = serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("serial 不能为空"));
    }
    let session = state
        .vnc_sessions
        .consume(serial, &query.session, &query.ticket)
        .ok_or_else(|| AppError::unauthorized("ticket 无效、已过期或已使用，请重新创建会话"))?;
    let device = state
        .machines
        .summary(serial)
        .await?
        .ok_or_else(|| AppError::not_found("机器不存在"))?;

    // 1) 通知 agent 建立隧道（downlink kind=vnc.open，payload {session_id, vm}）。
    machines::enqueue_device_downlink(
        &state,
        &device.device_slot,
        VNC_OPEN_DOWNLINK_KIND,
        &serde_json::json!({ "session_id": query.session, "vm": session.vm }),
    )
    .await?;

    // 2) 连 device-pki 内部中继（token 来自 env）。
    let token = state
        .config
        .device_pki_internal_token
        .as_deref()
        .ok_or_else(|| {
            AppError::service_unavailable("未配置 DEVICE_PKI_INTERNAL_TOKEN，noVNC 代理不可用")
        })?;
    let url = vnc::internal_ws_url(&state.config.device_pki_internal_url, &query.session, token)?;
    let internal = connect_internal(&url).await?;

    let serial_log = device.serial.clone();
    let session_log = query.session.clone();
    Ok(ws
        .on_upgrade(move |browser| async move {
            relay(browser, internal, serial_log, session_log).await;
        })
        .into_response())
}

/// 浏览器 WS ↔ device-pki 内部 WS 的二进制帧双向中继。
async fn relay(
    mut browser: WebSocket,
    mut internal: WebSocketStream<MaybeTlsStream<TcpStream>>,
    serial: String,
    session: String,
) {
    let mut browser_bytes: u64 = 0;
    let mut internal_bytes: u64 = 0;

    loop {
        tokio::select! {
            message = browser.recv() => {
                match message {
                    Some(Ok(AxumMessage::Binary(data))) => {
                        if data.len() > VNC_MAX_FRAME_BYTES {
                            tracing::warn!(%serial, %session, "浏览器 VNC 帧超过 2MiB，关闭");
                            break;
                        }
                        browser_bytes += data.len() as u64;
                        if internal.send(WsMessage::Binary(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(AxumMessage::Close(_))) | None => break,
                    // RFB 只走二进制；文本/Ping/Pong 丢弃（协议层自动回 pong）。
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::debug!(%serial, %session, %error, "浏览器 VNC WS 读取失败");
                        break;
                    }
                }
            }
            message = internal.next() => {
                match message {
                    Some(Ok(WsMessage::Binary(data))) => {
                        if data.len() > VNC_MAX_FRAME_BYTES {
                            tracing::warn!(%serial, %session, "上游 VNC 帧超过 2MiB，关闭");
                            break;
                        }
                        internal_bytes += data.len() as u64;
                        if browser.send(AxumMessage::Binary(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::debug!(%serial, %session, %error, "device-pki VNC 中继读取失败");
                        break;
                    }
                }
            }
        }
    }

    let _ = browser.send(AxumMessage::Close(None)).await;
    let _ = internal.close(None).await;
    tracing::info!(
        %serial,
        %session,
        browser_bytes,
        internal_bytes,
        "VNC 控制台 WS 代理结束"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vnc_info_variants() {
        let (host, port, password) = parse_vnc_info(Some(&serde_json::json!({
            "host": "127.0.0.1",
            "port": 5901,
            "password": "abc"
        })))
        .unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 5901);
        assert_eq!(password, "abc");

        // 别名 + 缺省 host
        let (host, port, password) = parse_vnc_info(Some(&serde_json::json!({
            "vnc_port": 5902,
            "vnc_password": "def"
        })))
        .unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 5902);
        assert_eq!(password, "def");
    }

    #[test]
    fn rejects_missing_vnc_info_fields() {
        assert!(parse_vnc_info(None).is_err());
        assert!(parse_vnc_info(Some(&serde_json::json!({}))).is_err());
        assert!(parse_vnc_info(Some(&serde_json::json!({"port": 0, "password": "x"}))).is_err());
        // 空/空白密码是合法的“无密码 VNC”。
        assert_eq!(
            parse_vnc_info(Some(&serde_json::json!({"port": 5901, "password": "  "})))
                .unwrap()
                .2,
            ""
        );
        assert_eq!(
            parse_vnc_info(Some(&serde_json::json!({"port": 5901}))).unwrap().2,
            ""
        );
    }
}
