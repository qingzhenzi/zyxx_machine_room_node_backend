//! 跨进程下行通知：Redis pub/sub 扇出（特性 `redis-notify`）。
//!
//! ## 为什么只要**一条**订阅连接
//!
//! 所有设备的"有新消息"通知都发到同一个频道，载荷里带 `serial`；进程收到后
//! 用进程内的 [`Notifier`](crate::messaging::Notifier) 唤醒对应的长连接。
//! 这样连接数是 **O(1)**，而不是 O(设备数) —— 后者在几千台设备时会把
//! Redis 的订阅连接数与内存直接打爆。
//!
//! ## 失败语义
//!
//! - 发送失败：只打日志。本进程已经唤醒过自己的连接；其它进程漏掉的消息会被
//!   WebSocket 会话里的 2s 兜底轮询捞回来，不会丢
//! - 订阅断开：指数退避重连。重连期间漏掉的通知同样由兜底轮询兜住

use crate::messaging::{self, BoxFuture, CrossProcessSignal};
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use std::sync::Arc;
use std::time::Duration;

/// 默认频道名（可用 `NOTIFY_REDIS_CHANNEL` 覆盖）。
pub const DEFAULT_CHANNEL: &str = "qe-device-pki:downlink";

/// 重连退避。
const RETRY_INITIAL: Duration = Duration::from_millis(200);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// 发布端：`downlink_publish` 成功后向频道里发一条 `serial`。
pub struct RedisSignal {
    conn: MultiplexedConnection,
    channel: String,
}

impl RedisSignal {
    pub async fn connect(url: &str, channel: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(url.to_owned())?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self {
            conn,
            channel: channel.to_owned(),
        })
    }
}

impl CrossProcessSignal for RedisSignal {
    fn publish(&self, serial: String) -> BoxFuture<'static, ()> {
        let mut conn = self.conn.clone();
        let channel = self.channel.clone();
        Box::pin(async move {
            if let Err(e) = conn.publish::<_, _, ()>(&channel, &serial).await {
                tracing::warn!(error = %e, "跨进程下行通知发送失败（退化为兜底轮询）");
            }
        })
    }
}

/// 安装跨进程信号 + 启动扇出任务。连接失败返回 `Err`（调用方降级为兜底轮询）。
pub async fn install(url: &str, channel: &str) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let signal = RedisSignal::connect(url, channel).await?;
    let installed = messaging::install_cross_process(Arc::new(signal));
    if !installed {
        tracing::warn!("跨进程通知信号已被安装过，忽略本次安装");
    }
    Ok(spawn_fanout(url.to_owned(), channel.to_owned()))
}

/// 启动扇出任务：订阅频道，收到 `serial` 就唤醒本进程内对应的长连接。
///
/// 断线会指数退避重连；重连期间漏掉的通知由会话侧的兜底轮询兜住。
pub fn spawn_fanout(url: String, channel: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = RETRY_INITIAL;
        loop {
            match run_once(&url, &channel).await {
                Ok(()) => {
                    tracing::warn!("下行通知订阅流结束，稍后重连");
                    backoff = RETRY_INITIAL;
                }
                Err(e) => {
                    tracing::warn!(error = %e, ?backoff, "下行通知订阅失败，重试");
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(RETRY_MAX);
        }
    })
}

async fn run_once(url: &str, channel: &str) -> anyhow::Result<()> {
    let client = redis::Client::open(url.to_owned())?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(channel).await?;
    tracing::info!(channel, "已订阅跨进程下行通知频道");

    let mut stream = pubsub.on_message();
    while let Some(msg) = futures_util::StreamExt::next(&mut stream).await {
        let serial: String = match msg.get_payload() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "通知载荷解析失败，忽略");
                continue;
            }
        };
        // 只做进程内唤醒：真正的消息体仍在 device_downlink 队列里
        messaging::notifier().notify(&serial);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 扇出的核心语义：收到 serial → 唤醒对应设备的等待者。
    /// 用真实 Redis 验证；没有 Redis 时跳过（集成脚本会启动）。
    #[tokio::test]
    async fn fanout_wakes_the_right_device() {
        let url = match std::env::var("TEST_REDIS_URL") {
            Ok(v) => v,
            Err(_) => {
                eprintln!("skip: 未设置 TEST_REDIS_URL");
                return;
            }
        };
        let channel = format!("test-fanout-{}", uuid::Uuid::new_v4().simple());

        // 订阅侧：等目标设备的通知
        let waiter = messaging::notifier().waiter("serial-under-test");
        let url2 = url.clone();
        let channel2 = channel.clone();
        let fanout = spawn_fanout(url2, channel2);

        let signal = RedisSignal::connect(&url, &channel).await.expect("connect");
        // 等订阅就绪
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 发一条别的设备的通知，不应唤醒目标设备
        signal.publish("other-serial".into()).await;
        let early = tokio::time::timeout(Duration::from_millis(200), waiter.notified()).await;
        assert!(early.is_err(), "不该被别的设备的通知唤醒");

        // 发目标设备的，应当被唤醒
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        tokio::spawn(async move {
            waiter.notified().await;
            h.fetch_add(1, Ordering::SeqCst);
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        signal.publish("serial-under-test".into()).await;

        let got = tokio::time::timeout(Duration::from_secs(2), async {
            while hits.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(got.is_ok(), "目标设备应被跨进程通知唤醒");
        fanout.abort();
    }
}
