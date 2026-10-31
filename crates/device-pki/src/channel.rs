//! WebSocket 隧道（mTLS listener）：一条连接同时承担「注册完成」与「双向消息」。
//!
//! ## 为什么把 confirm 折叠进建连
//!
//! 契约原本是两步：`POST /enroll/confirm` 把 enrollment 推到 `active`，
//! 然后才是业务通讯。对设备来说这两步没有区分意义 —— **能建立一条经过
//! 证书认证的业务连接，本身就证明证书可用**。所以这里在 WS 升级前调用
//! `activate_enrollment`：连上即注册完成（幂等，重复建连也安全）。
//!
//! ## 帧协议（JSON 文本帧）
//!
//! ```text
//! 设备 → 服务端
//!   {"type":"msg","seq":1,"kind":"heartbeat","payload":"{}"}
//!   {"type":"ack","up_to":7}          // 下行消息处理成功
//!   {"type":"ping"}
//!
//! 服务端 → 设备
//!   {"type":"ready","status":"active","serial":"..."}   // 注册已激活
//!   {"type":"ack_msg","seq":1,"status":"ok","duplicate":false}
//!   {"type":"down","id":7,"kind":"config","payload":"{}"}
//!   {"type":"pong"}
//!   {"type":"err","code":"...","message":"..."}
//! ```
//!
//! ## 与 HTTP 端点的关系
//!
//! 隧道**不是**另一套语义：上行走同一个 `process_incoming`（收件箱去重 →
//! 业务分发 → 成功才推水位），下行走同一个 `downlink_fetch`/`downlink_ack`。
//! HTTP 端点保留作为兜底（设备无法维持长连接时仍能收发）。

use crate::error::{Error, Result};
use crate::routes::{self, AppState};
use crate::tls::PeerCert;
use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 心跳间隔：服务端主动发 `ping` 帧。
pub const CHANNEL_HEARTBEAT: Duration = Duration::from_secs(20);
/// 连续几个心跳周期没收到任何帧就认为连接已死。
pub const HEARTBEAT_MISS_LIMIT: u32 = 2;
/// 兜底轮询间隔。进程内通知器覆盖不到跨进程投递，所以保留低频轮询。
pub const FALLBACK_POLL: Duration = Duration::from_secs(2);
/// 单次下行推送的批大小。
pub const DOWNLINK_BATCH: i64 = 50;

// ---------------- 帧 ----------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// 设备 → 服务端：一条上行消息（语义同 `POST /messages`）
    Msg {
        seq: i64,
        kind: String,
        payload: String,
    },
    /// 设备 → 服务端：确认已处理的下行游标（含）
    Ack {
        up_to: i64,
    },
    Ping,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    /// 建连成功，注册已激活
    Ready {
        status: String,
        serial: String,
    },
    /// 上行消息处理结果
    AckMsg {
        seq: i64,
        status: String,
        duplicate: bool,
    },
    /// 服务端 → 设备：一条下行消息
    Down {
        id: i64,
        kind: String,
        payload: String,
    },
    Pong,
    Err {
        code: String,
        message: String,
    },
}

/// `GET /channel` —— 建立 WebSocket 隧道（mTLS，客户端证书必需）。
///
/// 建连即激活注册单（等价于 `POST /enroll/confirm`，且幂等）。
pub async fn channel_upgrade(
    State(st): State<AppState>,
    peer: PeerCert,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    let slot = peer.common_name().ok_or(Error::Unauthorized)?.to_owned();
    // 连上即注册完成：能建立这条证书认证的连接，本身就证明证书可用
    let activated = routes::activate_enrollment(&st, &slot).await?;
    tracing::info!(slot = %slot, serial = %activated.serial, "设备通道建立，注册已激活");

    Ok(ws
        .on_upgrade(move |socket| session(st, socket, peer, activated.serial))
        .into_response())
}

/// 一条设备连接的会话循环。
async fn session(st: AppState, mut socket: WebSocket, peer: PeerCert, serial: String) {
    let waiter = crate::messaging::notifier().waiter(&serial);
    let mut fallback = tokio::time::interval(FALLBACK_POLL);
    fallback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // 跳过第一个立即触发的 tick：刚建连就发 ping 既没意义，还会在 ready 之后
    // 立刻混进一帧 pong，让"首帧语义"变模糊
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + CHANNEL_HEARTBEAT,
        CHANNEL_HEARTBEAT,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut missed: u32 = 0;
    // 本会话已经推出去的最大下行游标（重连后从 0 重投未 ACK 的消息）
    let mut pushed: i64 = 0;

    if !send(
        &mut socket,
        &ServerFrame::Ready {
            status: "active".into(),
            serial: serial.clone(),
        },
        &serial,
    )
    .await
    {
        return;
    }

    loop {
        enum Tick {
            /// 已把传输层错误折叠成 `None`（= 断开）
            Frame(Option<WsMessage>),
            Notify,
            Fallback,
            Heartbeat,
        }

        let tick = tokio::select! {
            f = socket.next() => Tick::Frame(match f {
                Some(Ok(msg)) => Some(msg),
                Some(Err(e)) => {
                    tracing::debug!(serial = %serial, error = %e, "读取失败");
                    None
                }
                None => None,
            }),
            _ = waiter.notified() => Tick::Notify,
            _ = fallback.tick() => Tick::Fallback,
            _ = heartbeat.tick() => Tick::Heartbeat,
        };

        match tick {
            Tick::Frame(Some(WsMessage::Text(text))) => {
                missed = 0;
                let frame: ClientFrame = match serde_json::from_str(&text) {
                    Ok(f) => f,
                    Err(e) => {
                        send_err(
                            &mut socket,
                            &serial,
                            "bad_frame",
                            &format!("无法解析帧: {e}"),
                        )
                        .await;
                        continue;
                    }
                };
                match frame {
                    ClientFrame::Msg { seq, kind, payload } => {
                        // 与 HTTP POST /messages 完全同一条处理路径
                        match routes::process_incoming(&st, &peer, seq, &kind, &payload).await {
                            Ok(r) => {
                                send(
                                    &mut socket,
                                    &ServerFrame::AckMsg {
                                        seq,
                                        status: r.status,
                                        duplicate: r.duplicate,
                                    },
                                    &serial,
                                )
                                .await;
                            }
                            Err(e) => {
                                // 失败不推水位：设备收到 err 后应重发同一条
                                send_err(&mut socket, &serial, e.code(), &e.to_string()).await;
                            }
                        }
                        // 处理完上行通常会有回复，立刻再推一轮下行
                        if !push_downlink(&st, &mut socket, &serial, &mut pushed).await {
                            break;
                        }
                    }
                    ClientFrame::Ack { up_to } => {
                        missed = 0;
                        if up_to <= 0 {
                            send_err(&mut socket, &serial, "bad_request", "up_to 必须为正整数")
                                .await;
                        } else if let Err(e) =
                            crate::messaging::downlink_ack(&st.db, &serial, up_to).await
                        {
                            tracing::warn!(error = %e, "下行 ACK 落库失败");
                        }
                    }
                    ClientFrame::Ping => {
                        missed = 0;
                        if !send(&mut socket, &ServerFrame::Pong, &serial).await {
                            break;
                        }
                    }
                }
            }
            // 协议层 ping/pong 与文本帧同等对待（都算"活着"）
            Tick::Frame(Some(WsMessage::Ping(_))) | Tick::Frame(Some(WsMessage::Pong(_))) => {
                missed = 0;
            }
            Tick::Frame(Some(WsMessage::Close(_))) | Tick::Frame(None) => {
                tracing::info!(serial = %serial, "设备通道关闭");
                break;
            }
            // 协议约定只有文本帧：二进制帧一律拒绝
            Tick::Frame(Some(WsMessage::Binary(_))) => {
                send_err(&mut socket, &serial, "bad_frame", "只接受文本帧").await;
                missed = 0;
            }
            Tick::Notify | Tick::Fallback => {
                if !push_downlink(&st, &mut socket, &serial, &mut pushed).await {
                    break;
                }
            }
            Tick::Heartbeat => {
                missed += 1;
                if missed > HEARTBEAT_MISS_LIMIT {
                    tracing::info!(serial = %serial, "设备连续无响应，关闭通道");
                    let _ = socket.close().await;
                    break;
                }
                if !send(&mut socket, &ServerFrame::Pong, &serial).await {
                    break;
                }
            }
        }
    }

    // 连接断开：把会话标成待判定（真正判 offline 由心跳定时器决定）
    let _ = st.sessions.mark_seen(&serial).await;
}

/// 推一批未 ACK 的下行消息。返回 `false` 表示连接已不可用。
async fn push_downlink(
    st: &AppState,
    socket: &mut WebSocket,
    serial: &str,
    pushed: &mut i64,
) -> bool {
    let rows = match crate::messaging::downlink_fetch(&st.db, serial, *pushed, DOWNLINK_BATCH).await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "拉取下行消息失败");
            return true;
        }
    };
    for m in rows {
        *pushed = (*pushed).max(m.id);
        if !send(
            socket,
            &ServerFrame::Down {
                id: m.id,
                kind: m.kind,
                payload: m.payload,
            },
            serial,
        )
        .await
        {
            return false;
        }
    }
    true
}

async fn send(socket: &mut WebSocket, frame: &ServerFrame, serial: &str) -> bool {
    let text = match serde_json::to_string(frame) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "下行帧序列化失败");
            return false;
        }
    };
    match socket.send(WsMessage::Text(text.into())).await {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(serial = %serial, error = %e, "发送失败，关闭通道");
            false
        }
    }
}

async fn send_err(socket: &mut WebSocket, serial: &str, code: &str, message: &str) {
    send(
        socket,
        &ServerFrame::Err {
            code: code.to_owned(),
            message: message.to_owned(),
        },
        serial,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 帧是契约的一部分：两端序列化格式必须稳定。
    #[test]
    fn frame_wire_format_is_stable() {
        let down = ServerFrame::Down {
            id: 7,
            kind: "config".into(),
            payload: "{}".into(),
        };
        let json = serde_json::to_string(&down).unwrap();
        assert_eq!(
            json,
            r#"{"type":"down","id":7,"kind":"config","payload":"{}"}"#
        );

        let ack: ClientFrame = serde_json::from_str(r#"{"type":"ack","up_to":7}"#).unwrap();
        assert!(matches!(ack, ClientFrame::Ack { up_to: 7 }));

        let msg: ClientFrame =
            serde_json::from_str(r#"{"type":"msg","seq":1,"kind":"k","payload":"p"}"#).unwrap();
        assert!(matches!(msg, ClientFrame::Msg { seq: 1, .. }));

        let err = ServerFrame::Err {
            code: "token_invalid".into(),
            message: "x".into(),
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"type":"err","code":"token_invalid","message":"x"}"#
        );

        // 未知 type 必须被拒（不能静默当成心跳之类的默认值）
        assert!(serde_json::from_str::<ClientFrame>(r#"{"type":"nope"}"#).is_err());
    }
}
