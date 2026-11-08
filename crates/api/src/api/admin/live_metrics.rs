//! 毫秒级实时指标 SSE：浏览器 ⇄ qe-api ⇄ device-pki 内部 WS（内存广播）。
//!
//! 不写数据库；历史快照仍由 `/machine/report` 负责。浏览器用 fetch 流式读取
//! （EventSource 不能带 Authorization，和既有机型快照流一致）。

use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{SinkExt, StreamExt, stream};
use tokio_tungstenite::tungstenite::Message;

use crate::api::extractors::AuthUser;
use crate::auth::authorization;
use crate::domain::rbac::permissions;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

fn metrics_ws_url(base: &str, serial: &str, token: &str) -> AppResult<String> {
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
    Ok(format!("{ws_base}/internal/metrics/{serial}?token={token}"))
}

/// `GET /api/v1/admin/machines/{serial}/metrics/stream` —— SSE 实时指标。
pub async fn stream(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
) -> AppResult<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;
    let serial = serial.trim().to_owned();
    let token = state
        .config
        .device_pki_internal_token
        .as_deref()
        .unwrap_or_default();
    let url = metrics_ws_url(&state.config.device_pki_internal_url, &serial, token)?;
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|error| AppError::service_unavailable(format!("连接实时指标中转失败: {error}")))?;

    let stream = stream::unfold((socket, false), |(mut socket, done)| async move {
        if done {
            return None;
        }
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let text = text.to_string();
                    let event = if text.contains("\"type\":\"heartbeat\"") {
                        Event::default().event("heartbeat").data("{}")
                    } else {
                        Event::default().event("metric").data(text)
                    };
                    return Some((Ok(event), (socket, false)));
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = socket.send(Message::Pong(payload)).await;
                }
                Some(Ok(Message::Close(_))) | None => return None,
                Some(Err(_)) => return None,
                _ => continue,
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_internal_metrics_url() {
        assert_eq!(
            metrics_ws_url("http://127.0.0.1:4442", "abc123", "tok").unwrap(),
            "ws://127.0.0.1:4442/internal/metrics/abc123?token=tok"
        );
        assert_eq!(
            metrics_ws_url("https://pki:4442/", "abc123", "tok").unwrap(),
            "wss://pki:4442/internal/metrics/abc123?token=tok"
        );
    }

    #[test]
    fn rejects_bad_serial_or_missing_token() {
        assert!(metrics_ws_url("http://127.0.0.1:4442", "../x", "tok").is_err());
        assert!(metrics_ws_url("http://127.0.0.1:4442", "abc", "").is_err());
    }
}
