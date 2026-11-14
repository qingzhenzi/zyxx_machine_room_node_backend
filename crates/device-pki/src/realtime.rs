//! 机器实时 WS 通道（mTLS）：一条长连接承载实时指标样本 + 服务端按需/限速。
//!
//! 目标（类似 Pterodactyl/Wings 的实时流）：把「每个样本一次 HTTP POST」换成
//! 一条长连接，并让**服务端**在有人看的时候才要求 agent 按指定频率推送：
//!
//! ```text
//! qe-api 订阅 /internal/metrics/{serial}?interval_ms=1000
//!    └─ device-pki：viewer_opened → 向该 serial 的实时通道发 {"type":"want","interval_ms":1000}
//! agent 收到 want → 每 interval_ms 发一帧 {"type":"sample","payload":{...}}
//!    └─ device-pki MetricsHub.publish → qe-api 内部 WS → 浏览器 SSE
//! qe-api 断开 → viewer_closed → {"type":"idle"} → agent 停止推送
//! ```
//!
//! 兼容性：`POST /metrics` 与 `telemetry.live` 下行保留；agent 连不上本通道时
//! 会退回旧的 HTTP 路径（见 room_agent `crates/qe-agent-telemetry/src/realtime.rs`）。
//!
//! ## 帧协议（JSON 文本帧）
//!
//! ```text
//! 设备 → 服务端
//!   {"type":"hello","agent_version":"0.1.0","capabilities":["metrics"]}
//!   {"type":"sample","payload":{...}}          // 一条实时样本（≤16KiB）
//!   {"type":"op_progress","request_id":"...","kind":"poweroff","stage":"cleanup","percent":50,"message":"..."}
//!   {"type":"ping"} / {"type":"pong"}
//!
//! 服务端 → 设备
//!   {"type":"ready","serial":"..."}
//!   {"type":"want","interval_ms":1000}         // 有人订阅，按该频率推流
//!   {"type":"idle"}                            // 没有订阅者，停止推流
//!   {"type":"ping"} / {"type":"pong"}
//!   {"type":"err","code":"...","message":"..."}
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::{Error, Result};
use crate::routes::AppState;
use crate::tls::PeerCert;

/// 单帧上限（实时样本很小；op_progress 文案也不会超过这个量级）。
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// 单样本上限（与 `metrics::MAX_SAMPLE_BYTES` 对齐）。
const MAX_SAMPLE_BYTES: usize = crate::metrics::MAX_SAMPLE_BYTES;
/// 每台机器待发送帧的队列上限：满了就丢弃（实时流可以丢，不能阻塞会话）。
const PEER_QUEUE: usize = 16;
/// 服务端主动 ping 间隔；同时也是「连接仍活着」的下限判定。
const PING_INTERVAL: Duration = Duration::from_secs(20);
/// 允许的实时推送间隔（与 qe-api `interval_ms` 约束一致）。
pub const MIN_INTERVAL_MS: u64 = 200;
pub const MAX_INTERVAL_MS: u64 = 5000;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Hello {
        #[serde(default)]
        agent_version: String,
        /// agent 声明的能力（`metrics` / `op_progress` 等，服务端可据此降级）。
        #[serde(default)]
        capabilities: Vec<String>,
    },
    Sample {
        payload: serde_json::Value,
    },
    /// 长操作进度（关机清理 / 镜像制作等），给控制台做「执行中」提示。
    OpProgress {
        request_id: String,
        kind: String,
        stage: String,
        #[serde(default)]
        percent: Option<f64>,
        #[serde(default)]
        message: String,
    },
    Ping,
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Ready { serial: String },
    Want { interval_ms: u64 },
    Idle,
    Ping,
    Pong,
    Err { code: String, message: String },
}

/// 一条实时通道的发送端。
struct Peer {
    tx: mpsc::Sender<ServerFrame>,
}

/// 某台机器当前的观测需求。
#[derive(Debug, Clone, Copy)]
struct Demand {
    interval_ms: u64,
    viewers: usize,
}

/// 机器实时通道注册表：谁连着、有没有人看、要推多快。
#[derive(Default)]
pub struct RealtimeHub {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    peers: HashMap<String, Peer>,
    demand: HashMap<String, Demand>,
}

/// 注册一台 agent 的发送端；返回接收端供会话循环消费。
///
/// 如果此刻已经有人在看，会立即补发一帧 `Want`（agent 连上就开始推流）。
pub fn register(hub: &RealtimeHub, serial: &str) -> mpsc::Receiver<ServerFrame> {
    let (tx, rx) = mpsc::channel(PEER_QUEUE);
    let mut inner = hub.inner.lock().unwrap_or_else(|e| e.into_inner());
    let want = inner.demand.get(serial).map(|demand| ServerFrame::Want {
        interval_ms: demand.interval_ms,
    });
    inner.peers.insert(serial.to_owned(), Peer { tx });
    drop(inner);
    if let Some(want) = want {
        let _ = hub.try_send(serial, want);
    }
    rx
}

/// agent 断开：移除发送端；观测需求保留（qe-api 可能仍在订阅，重连后补发）。
pub fn unregister(hub: &RealtimeHub, serial: &str) {
    let mut inner = hub.inner.lock().unwrap_or_else(|e| e.into_inner());
    inner.peers.remove(serial);
}

impl RealtimeHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否有 agent 的实时通道在线。
    pub fn is_connected(&self, serial: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .peers
            .contains_key(serial)
    }

    /// 一条下行帧：队列满就丢弃（实时帧可丢，不阻塞）。
    fn try_send(&self, serial: &str, frame: ServerFrame) {
        let sender = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.peers.get(serial).map(|peer| peer.tx.clone())
        };
        if let Some(sender) = sender {
            let _ = sender.try_send(frame);
        }
    }

    /// 浏览器/管理端开始观测某台机器。
    pub fn viewer_opened(&self, serial: &str, interval_ms: u64) {
        let interval_ms = interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let demand = inner.demand.entry(serial.to_owned()).or_insert(Demand {
            interval_ms,
            viewers: 0,
        });
        demand.interval_ms = interval_ms;
        demand.viewers += 1;
        drop(inner);
        self.try_send(serial, ServerFrame::Want { interval_ms });
    }

    /// 观测结束；最后一个观察者离开时通知 agent 停止推流。
    pub fn viewer_closed(&self, serial: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(demand) = inner.demand.get_mut(serial) else {
            return;
        };
        demand.viewers = demand.viewers.saturating_sub(1);
        let idle = demand.viewers == 0;
        if idle {
            inner.demand.remove(serial);
        }
        drop(inner);
        if idle {
            self.try_send(serial, ServerFrame::Idle);
        }
    }

    /// 观察者数量（诊断/测试用）。
    pub fn viewers(&self, serial: &str) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .demand
            .get(serial)
            .map(|demand| demand.viewers)
            .unwrap_or(0)
    }
}

/// `GET /machine/realtime` —— 设备实时通道（mTLS，客户端证书必需）。
pub async fn upgrade(
    State(st): State<AppState>,
    peer: PeerCert,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;
    let (_device_id, slot) = crate::routes::resolve_identity(&st, &peer).await?;
    // 建连本身就是一次活动：刷新会话并落库，让「在线」不依赖 2s 的 HTTP 轮询。
    crate::routes::mark_device_activity(&st, &serial).await;
    tracing::info!(serial = %serial, slot = %slot, "机器实时通道建立");
    Ok(ws
        .on_upgrade(move |socket| session(st, socket, serial))
        .into_response())
}

async fn session(st: AppState, socket: WebSocket, serial: String) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = register(&st.realtime, &serial);

    if send(
        &mut sink,
        &ServerFrame::Ready {
            serial: serial.clone(),
        },
    )
    .await
    .is_err()
    {
        unregister(&st.realtime, &serial);
        return;
    }

    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await;

    loop {
        tokio::select! {
            outgoing = rx.recv() => {
                match outgoing {
                    Some(frame) => {
                        if send(&mut sink, &frame).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_FRAME_BYTES {
                            let _ = send(&mut sink, &err_frame("frame_too_large", "帧过大")).await;
                            continue;
                        }
                        match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(frame) => handle_client_frame(&st, &mut sink, &serial, frame).await,
                            Err(error) => {
                                let _ = send(
                                    &mut sink,
                                    &err_frame("bad_frame", &format!("无法解析帧: {error}")),
                                )
                                .await;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(Message::Binary(_))) => {
                        let _ = send(&mut sink, &err_frame("bad_frame", "只接受文本帧")).await;
                    }
                }
            }
            _ = ping.tick() => {
                if send(&mut sink, &ServerFrame::Ping).await.is_err() {
                    break;
                }
            }
        }
    }

    unregister(&st.realtime, &serial);
    tracing::info!(serial = %serial, "机器实时通道关闭");
}

async fn handle_client_frame(
    st: &AppState,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    serial: &str,
    frame: ClientFrame,
) {
    match frame {
        ClientFrame::Hello {
            agent_version,
            capabilities,
        } => {
            tracing::info!(
                serial = %serial,
                agent_version = %agent_version,
                capabilities = ?capabilities,
                "机器实时通道握手"
            );
        }
        ClientFrame::Sample { payload } => {
            // 与 HTTP `POST /metrics` 完全同一条落点：只广播、不写 DB。
            let body = payload.to_string();
            if body.len() > MAX_SAMPLE_BYTES {
                let _ = send(sink, &err_frame("sample_too_large", "实时样本过大")).await;
                return;
            }
            if !payload.is_object() {
                let _ = send(sink, &err_frame("bad_sample", "实时样本必须是 JSON 对象")).await;
                return;
            }
            st.metrics.publish(serial, Arc::from(body));
        }
        ClientFrame::OpProgress {
            request_id,
            kind,
            stage,
            percent,
            message,
        } => {
            tracing::info!(
                serial = %serial,
                request_id = %request_id,
                kind = %kind,
                stage = %stage,
                percent = ?percent,
                message = %message,
                "机器长操作进度"
            );
            // 与实时样本共用同一条广播：qe-api 识别 `type=op_progress` 后以
            // SSE `op` 事件转发，控制台不必再 1s 轮询命令结果。
            let payload = serde_json::json!({
                "type": "op_progress",
                "request_id": request_id,
                "kind": kind,
                "stage": stage,
                "percent": percent,
                "message": message,
                "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            });
            st.metrics.publish(serial, Arc::from(payload.to_string()));
        }
        ClientFrame::Ping => {
            crate::routes::mark_device_activity(st, serial).await;
            let _ = send(sink, &ServerFrame::Pong).await;
        }
        ClientFrame::Pong => {}
    }
}

fn err_frame(code: &str, message: &str) -> ServerFrame {
    ServerFrame::Err {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

async fn send(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: &ServerFrame,
) -> std::result::Result<(), ()> {
    let text = match serde_json::to_string(frame) {
        Ok(text) => text,
        Err(_) => return Err(()),
    };
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_frames_parse() {
        let sample: ClientFrame =
            serde_json::from_str(r#"{"type":"sample","payload":{"cpu_usage_percent":12.5}}"#)
                .unwrap();
        assert!(matches!(sample, ClientFrame::Sample { .. }));

        let want: ServerFrame =
            serde_json::from_str(r#"{"type":"want","interval_ms":500}"#).unwrap();
        assert!(matches!(want, ServerFrame::Want { interval_ms: 500 }));

        let progress: ClientFrame = serde_json::from_str(
            r#"{"type":"op_progress","request_id":"r1","kind":"poweroff","stage":"cleanup","percent":50.0,"message":"关闭 VM"}"#,
        )
        .unwrap();
        assert!(matches!(progress, ClientFrame::OpProgress { .. }));
    }

    #[test]
    fn server_frames_serialize_with_type_tag() {
        let value = serde_json::to_value(ServerFrame::Want { interval_ms: 1000 }).unwrap();
        assert_eq!(value["type"], "want");
        assert_eq!(value["interval_ms"], 1000);
        let value = serde_json::to_value(ServerFrame::Idle).unwrap();
        assert_eq!(value["type"], "idle");
    }

    #[test]
    fn viewer_demand_is_refcounted_and_survives_reconnect() {
        let hub = RealtimeHub::new();
        let mut rx = register(&hub, "s1");
        // 没有观察者时不应有 Want。
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        hub.viewer_opened("s1", 1000);
        assert_eq!(hub.viewers("s1"), 1);
        assert!(matches!(
            rx.try_recv().unwrap(),
            ServerFrame::Want { interval_ms: 1000 }
        ));

        // 第二个观察者 + 修改频率：以最后一次为准。
        hub.viewer_opened("s1", 500);
        assert_eq!(hub.viewers("s1"), 2);
        assert!(matches!(
            rx.try_recv().unwrap(),
            ServerFrame::Want { interval_ms: 500 }
        ));

        // 关掉一个观察者仍保留需求；全部关闭才 Idle。
        hub.viewer_closed("s1");
        assert_eq!(hub.viewers("s1"), 1);
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        hub.viewer_closed("s1");
        assert_eq!(hub.viewers("s1"), 0);
        assert!(matches!(rx.try_recv().unwrap(), ServerFrame::Idle));

        // 重连（先断开再注册）时若已有观察者，应立刻补发 Want。
        hub.viewer_opened("s1", 200);
        unregister(&hub, "s1");
        let mut rx = register(&hub, "s1");
        assert!(matches!(
            rx.try_recv().unwrap(),
            ServerFrame::Want { interval_ms: 200 }
        ));
    }

    #[test]
    fn interval_is_clamped() {
        let hub = RealtimeHub::new();
        let mut rx = register(&hub, "s2");
        hub.viewer_opened("s2", 10);
        assert!(matches!(
            rx.try_recv().unwrap(),
            ServerFrame::Want {
                interval_ms: MIN_INTERVAL_MS
            }
        ));
        hub.viewer_opened("s2", 60_000);
        let mut saw_max = false;
        while let Ok(frame) = rx.try_recv() {
            if matches!(
                frame,
                ServerFrame::Want {
                    interval_ms: MAX_INTERVAL_MS
                }
            ) {
                saw_max = true;
            }
        }
        assert!(saw_max);
    }
}
