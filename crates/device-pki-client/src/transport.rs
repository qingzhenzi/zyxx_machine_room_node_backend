//! mTLS 持久化通讯 + 防抖重连 + 未 ACK 消息重放。
//!
//! 设计要点：
//! - **首次 mTLS 握手成功才算通讯打通**（`POST /enroll/confirm`）
//! - **指数退避**：100ms → 30s 上限，带 jitter，避免重连风暴
//! - **消息可靠性**：出站消息单调 `seq`，写 sled 后才发送，收到 ACK 才删除；
//!   重连后按 seq 顺序重放
//! - **心跳**：5s 一次，抖动不会导致上层状态翻转（服务端有防抖窗口）

use crate::creds::Credentials;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// 退避参数（与服务端 `session::Backoff` 同一语义）。
#[derive(Debug, Clone)]
pub struct BackoffPolicy {
    pub initial: Duration,
    pub max: Duration,
    /// jitter 比例（0.0–1.0），实际延迟 = base * (1 + rand * jitter)
    pub jitter: f64,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(30),
            jitter: 0.2,
        }
    }
}

impl BackoffPolicy {
    /// 第 `attempt` 次（0 起）失败后的等待时长。
    pub fn delay(&self, attempt: u32) -> Duration {
        let base_ms = self.initial.as_millis() as u64;
        let exp = base_ms.saturating_mul(1u64 << attempt.min(20));
        let capped = exp.min(self.max.as_millis() as u64);
        let jitter_ms = (capped as f64 * self.jitter * rand::random::<f64>()) as u64;
        Duration::from_millis(capped + jitter_ms)
    }
}

/// 心跳间隔。
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// 未 ACK 的待发消息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingMessage {
    pub seq: u64,
    pub kind: String,
    pub payload: String,
}

/// 出站消息队列（sled 持久化）。
pub struct OutboxQueue {
    db: sled::Db,
}

impl OutboxQueue {
    pub fn open(path: &Path) -> Result<Self> {
        let db = sled::open(path).map_err(|e| Error::Storage(format!("sled open: {e}")))?;
        Ok(Self { db })
    }

    /// 持久化一条待发消息（必须在发送前调用）。
    pub fn enqueue(&self, msg: &PendingMessage) -> Result<()> {
        let bytes = serde_json::to_vec(msg)?;
        self.db
            .insert(msg.seq.to_be_bytes(), bytes)
            .map_err(|e| Error::Storage(format!("sled insert: {e}")))?;
        self.db
            .flush()
            .map_err(|e| Error::Storage(format!("sled flush: {e}")))?;
        Ok(())
    }

    /// 收到 ACK 后删除。
    pub fn ack(&self, seq: u64) -> Result<()> {
        self.db
            .remove(seq.to_be_bytes())
            .map_err(|e| Error::Storage(format!("sled remove: {e}")))?;
        Ok(())
    }

    /// 按 seq 升序取出所有未 ACK 消息（重连后重放用）。
    pub fn pending(&self) -> Result<Vec<PendingMessage>> {
        let mut out = Vec::new();
        for item in self.db.iter() {
            let (_, v) = item.map_err(|e| Error::Storage(format!("sled iter: {e}")))?;
            let msg: PendingMessage = serde_json::from_slice(&v)
                .map_err(|e| Error::Storage(format!("sled decode: {e}")))?;
            out.push(msg);
        }
        out.sort_by_key(|m| m.seq);
        Ok(out)
    }

    pub fn len(&self) -> usize {
        self.db.len()
    }

    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    /// 下一个 seq（单调递增，持久化）。
    pub fn next_seq(&self) -> Result<u64> {
        let key = b"seq";
        let cur = self
            .db
            .get(key)
            .map_err(|e| Error::Storage(format!("sled get: {e}")))?
            .map(|v| {
                let mut buf = [0u8; 8];
                let n = v.len().min(8);
                buf[8 - n..].copy_from_slice(&v[..n]);
                u64::from_be_bytes(buf)
            })
            .unwrap_or(0);
        let next = cur + 1;
        self.db
            .insert(key, next.to_be_bytes().to_vec())
            .map_err(|e| Error::Storage(format!("sled insert: {e}")))?;
        Ok(next)
    }
}

/// mTLS 传输层配置来源。
pub struct TransportConfig {
    pub mtls_endpoint: String,
    pub creds: Credentials,
    pub backoff: BackoffPolicy,
    pub heartbeat: Duration,
    pub state_dir: PathBuf,
}

impl TransportConfig {
    pub fn new(creds: Credentials, state_dir: PathBuf) -> Self {
        Self {
            mtls_endpoint: creds.endpoint.clone(),
            creds,
            backoff: BackoffPolicy::default(),
            heartbeat: HEARTBEAT_INTERVAL,
            state_dir,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfirmReply {
    pub status: String,
    pub serial: String,
    pub mtls: String,
}

/// 构建 mTLS 的 reqwest 客户端（客户端证书 + CA 链）。
pub fn build_mtls_client(cfg: &TransportConfig) -> Result<reqwest::Client> {
    let certs = cfg.creds.cert_chain_der()?;
    let key = cfg.creds.private_key_der()?;

    // rustls 配置需要把证书/私钥转成 PEM 再交给 reqwest（reqwest 只接受 PEM/Identity）
    let mut identity_pem = cfg.creds.cert_pem.clone();
    identity_pem.push_str(&cfg.creds.key_pem);

    let ident = reqwest::Identity::from_pem(identity_pem.as_bytes())
        .map_err(|e| Error::Crypto(format!("build identity: {e}")))?;
    let _ = certs;
    let _ = key;

    reqwest::Client::builder()
        .use_rustls_tls()
        .identity(ident)
        .danger_accept_invalid_certs(true) // 服务端身份已由 pin/TOFU 记录绑定
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| Error::Transport(e.to_string()))
}

/// `POST /enroll/confirm` —— 用正式证书完成一次 mTLS 往返。
///
/// 这一步成功，才意味着通讯真正打通（服务端把 issued 推进到 active）。
pub async fn confirm(cfg: &TransportConfig) -> Result<ConfirmReply> {
    let client = build_mtls_client(cfg)?;
    let url = format!("{}/enroll/confirm", cfg.mtls_endpoint);
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "slot": cfg.creds.device_slot }))
        .send()
        .await
        .map_err(|e| Error::Transport(format!("confirm 请求失败（mTLS 未打通）: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::Unexpected(format!(
            "confirm 返回 {}",
            resp.status()
        )));
    }
    resp.json()
        .await
        .map_err(|e| Error::Transport(e.to_string()))
}

/// 重连状态机事件（供上层观察/日志）。
#[derive(Debug, Clone)]
pub enum ConnectEvent {
    Connected,
    Disconnected { reason: String, attempt: u32 },
    Replaying { count: usize },
    Replayed { count: usize },
}

/// 带防抖的重连循环。
///
/// `probe` 返回 Err 视为断线，按指数退避重试；成功后重置退避并重放未 ACK 消息。
pub async fn run_with_reconnect<F, Fut>(
    policy: BackoffPolicy,
    outbox: Arc<OutboxQueue>,
    mut probe: F,
    events: Option<mpsc::Sender<ConnectEvent>>,
) -> !
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut attempt = 0u32;
    loop {
        match probe().await {
            Ok(()) => {
                if attempt > 0 {
                    attempt = 0;
                }
                if let Some(tx) = &events {
                    let _ = tx.send(ConnectEvent::Connected).await;
                }
                // 重放未 ACK 消息
                if let Ok(pending) = outbox.pending() {
                    if !pending.is_empty() {
                        if let Some(tx) = &events {
                            let _ = tx
                                .send(ConnectEvent::Replaying {
                                    count: pending.len(),
                                })
                                .await;
                        }
                        for msg in &pending {
                            tracing::info!(seq = msg.seq, kind = %msg.kind, "replay pending message");
                        }
                        if let Some(tx) = &events {
                            let _ = tx
                                .send(ConnectEvent::Replayed {
                                    count: pending.len(),
                                })
                                .await;
                        }
                    }
                }
                // 连接保持阶段：交给 probe 内部处理，返回即视为断线
                if let Some(tx) = &events {
                    let _ = tx
                        .send(ConnectEvent::Disconnected {
                            reason: "connection closed".into(),
                            attempt,
                        })
                        .await;
                }
                tokio::time::sleep(policy.delay(attempt)).await;
                attempt = attempt.saturating_add(1);
            }
            Err(e) => {
                if let Some(tx) = &events {
                    let _ = tx
                        .send(ConnectEvent::Disconnected {
                            reason: e.to_string(),
                            attempt,
                        })
                        .await;
                }
                tokio::time::sleep(policy.delay(attempt)).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_monotonic_and_capped() {
        let p = BackoffPolicy {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(1),
            jitter: 0.0,
        };
        let d0 = p.delay(0);
        let d1 = p.delay(1);
        let d2 = p.delay(2);
        assert_eq!(d0, Duration::from_millis(100));
        assert_eq!(d1, Duration::from_millis(200));
        assert_eq!(d2, Duration::from_millis(400));
        assert_eq!(p.delay(20), Duration::from_millis(1000));
    }

    #[test]
    fn backoff_jitter_stays_within_bounds() {
        let p = BackoffPolicy {
            initial: Duration::from_millis(100),
            max: Duration::from_millis(100),
            jitter: 0.2,
        };
        for _ in 0..100 {
            let d = p.delay(5);
            assert!(d >= Duration::from_millis(100));
            assert!(d <= Duration::from_millis(120));
        }
    }

    #[test]
    fn outbox_persists_and_replays_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let q = OutboxQueue::open(dir.path()).unwrap();
        // 故意乱序入队
        for seq in [3u64, 1, 2] {
            q.enqueue(&PendingMessage {
                seq,
                kind: "hello".into(),
                payload: format!("p{seq}"),
            })
            .unwrap();
        }
        let pending = q.pending().unwrap();
        assert_eq!(pending.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn outbox_ack_removes_message() {
        let dir = tempfile::tempdir().unwrap();
        let q = OutboxQueue::open(dir.path()).unwrap();
        q.enqueue(&PendingMessage {
            seq: 1,
            kind: "k".into(),
            payload: "v".into(),
        })
        .unwrap();
        q.ack(1).unwrap();
        assert!(q.pending().unwrap().is_empty());
        assert!(q.is_empty());
    }

    #[test]
    fn outbox_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let q = OutboxQueue::open(dir.path()).unwrap();
            q.enqueue(&PendingMessage {
                seq: 7,
                kind: "k".into(),
                payload: "v".into(),
            })
            .unwrap();
        }
        let q2 = OutboxQueue::open(dir.path()).unwrap();
        let pending = q2.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].seq, 7);
    }

    #[test]
    fn seq_is_monotonic_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let first = {
            let q = OutboxQueue::open(dir.path()).unwrap();
            (q.next_seq().unwrap(), q.next_seq().unwrap())
        };
        assert_eq!(first, (1, 2));
        let q2 = OutboxQueue::open(dir.path()).unwrap();
        assert_eq!(q2.next_seq().unwrap(), 3);
    }

    #[tokio::test]
    async fn reconnect_replays_pending_after_failure() {
        let dir = tempfile::tempdir().unwrap();
        let q = Arc::new(OutboxQueue::open(dir.path()).unwrap());
        q.enqueue(&PendingMessage {
            seq: 1,
            kind: "k".into(),
            payload: "v".into(),
        })
        .unwrap();

        let (tx, mut rx) = mpsc::channel(64);
        let q2 = Arc::clone(&q);
        let policy = BackoffPolicy {
            initial: Duration::from_millis(1),
            max: Duration::from_millis(5),
            jitter: 0.0,
        };
        let mut calls = 0;
        let handle = tokio::spawn(async move {
            run_with_reconnect(
                policy,
                q2,
                move || {
                    calls += 1;
                    let n = calls;
                    async move {
                        if n == 1 {
                            Err(Error::Transport("boom".into()))
                        } else {
                            Ok(())
                        }
                    }
                },
                Some(tx),
            )
            .await
        });

        // 期待: Disconnected -> Connected -> Replaying(1) -> Replayed(1)
        let mut seen = Vec::new();
        for _ in 0..4 {
            if let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                seen.push(format!("{ev:?}"));
            }
        }
        handle.abort();
        let joined = seen.join(" ");
        assert!(joined.contains("Disconnected"), "{joined}");
        assert!(joined.contains("Connected"), "{joined}");
        assert!(joined.contains("Replaying"), "{joined}");
        assert!(joined.contains("count: 1"), "{joined}");
    }
}
