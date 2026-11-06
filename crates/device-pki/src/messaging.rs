//! 设备上行消息：收件箱（去重 + 审计 + 自愈）与业务分发。
//!
//! ## 为什么单独抽出来
//!
//! `qe-device-pki` 是 **lib crate**：这一层不依赖任何 HTTP 细节，只要依赖
//! 数据库和会话层，别的服务（比如将来的 `crates/api`）可以直接依赖本 crate，
//! 在自己的进程里订阅消息类型：
//!
//! ```ignore
//! let dispatcher = Arc::new(messaging::Dispatcher::new(Duration::from_secs(10)));
//! dispatcher.subscribe("telemetry", |m: messaging::Message| async move {
//!     // 落库 / 转发 / 告警……返回 Err 则客户端会重发
//!     Ok(())
//! });
//! ```
//!
//! ## 语义：at-least-once + 幂等 ACK
//!
//! - 客户端没收到 2xx 就会重发（同 seq），所以**重复到达是常态**
//! - 收件箱按 `(serial, seq)` 唯一：重复插入失败即视为已收到
//! - **处理成功才推水位**：handler 失败 → 不推水位、返回 5xx → 客户端重发
//! - 收件箱记 `attempts`/`last_error`/`processed_at`：失败的重发会自愈，
//!   重启后也还能从库里看出哪条没处理完
//!
//! 为什么不用"先回 2xx 再慢慢处理"：那样 handler 失败时客户端已经放手，
//! 消息就永久丢了。要异步处理，得再加一条服务端内部重试队列，得不偿失。

use crate::error::{Error, Result};
use crate::session::SessionManager;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::task::JoinHandle;

/// 离线判定的轮询间隔。
///
/// 取 5s：与契约 §6 的心跳间隔一致 —— 「连续 3 次未达」的判定本身在
/// `SessionManager::heartbeat_miss` 里做，这里只负责按时推动它。
pub const OFFLINE_TICK_INTERVAL: Duration = Duration::from_secs(5);

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 单条消息最大载荷（上行与下行共用）。
///
/// 消息通道不是消息队列：超大载荷应走对象存储，这里给个硬上限防止
/// 一条消息打爆连接与内存。
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// 一条已通过身份与格式校验的上行消息。
#[derive(Debug, Clone)]
pub struct Message {
    /// 设备证书 serial（规范化 hex），服务端身份来源，不可伪造
    pub serial: String,
    /// 设备展示名（房间内绑定名，来自 DB；不是证书 CN）
    pub slot: String,
    pub seq: i64,
    pub kind: String,
    pub payload: String,
    pub received_at: chrono::DateTime<chrono::Utc>,
}

impl Message {
    /// 同一条消息在进程内的唯一键（in-flight 保护用）。
    pub fn key(&self) -> String {
        format!("{}:{}", self.serial, self.seq)
    }
}

/// 业务处理器。任何 `Fn(Message) -> impl Future<Output = Result<()>>` 都满足。
///
/// 约定：**快速返回**。慢活（大计算、外部 IO）请自己 spawn 到后台任务里，
/// 否则会占住这次 HTTP 请求直到超时。
pub trait MessageHandler: Send + Sync + 'static {
    fn handle(&self, msg: Message) -> BoxFuture<'static, Result<()>>;
}

impl<F, Fut> MessageHandler for F
where
    F: Fn(Message) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    fn handle(&self, msg: Message) -> BoxFuture<'static, Result<()>> {
        Box::pin((self)(msg))
    }
}

/// 一次分发的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// 至少一个 handler 成功处理完
    Processed,
    /// 没有订阅这个 kind（也没有通配订阅）：按"协议不匹配"记档，不再重试
    NoHandler,
}

/// 按 `kind` 分发的处理器注册表。
///
/// 同一 `kind` 可以有多个 handler（全部都要成功）；`"*"` 是通配订阅。
pub struct Dispatcher {
    handlers: RwLock<HashMap<String, Vec<Arc<dyn MessageHandler>>>>,
    /// 正在处理中的 `(serial, seq)`：防止同一进程内并发重复处理
    in_flight: Mutex<HashSet<String>>,
    handler_timeout: Duration,
}

impl std::fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dispatcher")
            .field("kinds", &self.handlers.read().map(|h| h.len()).unwrap_or(0))
            .field("handler_timeout", &self.handler_timeout)
            .finish_non_exhaustive()
    }
}

impl Dispatcher {
    pub fn new(handler_timeout: Duration) -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
            handler_timeout,
        }
    }

    /// 订阅某个 `kind`（多个 handler 都会被调用，任一失败即整体失败）。
    pub fn subscribe<H: MessageHandler>(&self, kind: &str, handler: H) {
        self.subscribe_arc(kind, Arc::new(handler));
    }

    /// 通配订阅：所有 kind 都会收到（用于审计、指标、通用落库）。
    pub fn subscribe_all<H: MessageHandler>(&self, handler: H) {
        self.subscribe_arc("*", Arc::new(handler));
    }

    fn subscribe_arc(&self, kind: &str, handler: Arc<dyn MessageHandler>) {
        self.handlers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(kind.to_owned())
            .or_default()
            .push(handler);
    }

    /// 是否有 handler 会处理这个 kind。
    pub fn has_handler(&self, kind: &str) -> bool {
        let map = self.handlers.read().unwrap_or_else(|e| e.into_inner());
        map.contains_key(kind) || map.contains_key("*")
    }

    /// 当前是否正在处理这条消息（进程内）。
    pub fn is_in_flight(&self, key: &str) -> bool {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(key)
    }

    fn claim(&self, key: &str) -> bool {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_owned())
    }

    fn release(&self, key: &str) {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    /// 分发一条消息。`Err(原因)` 表示处理失败（调用方应视为 5xx 并保留重试）。
    ///
    /// 这里负责 in-flight 保护与超时；收件箱与水位由调用方负责。
    pub async fn dispatch(&self, msg: Message) -> Result<Dispatch> {
        let key = msg.key();
        let targets: Vec<Arc<dyn MessageHandler>> = {
            let map = self.handlers.read().unwrap_or_else(|e| e.into_inner());
            let mut v = Vec::new();
            if let Some(list) = map.get(&msg.kind) {
                v.extend(list.iter().cloned());
            }
            if let Some(list) = map.get("*") {
                v.extend(list.iter().cloned());
            }
            v
        };

        if targets.is_empty() {
            tracing::warn!(
                serial = %msg.serial,
                kind = %msg.kind,
                seq = msg.seq,
                "没有订阅这个消息类型，按协议不匹配记档（不会重试）"
            );
            return Ok(Dispatch::NoHandler);
        }

        // 同一条消息并发到达时只处理一次
        if !self.claim(&key) {
            return Err(Error::BadRequest(format!("消息 {key} 正在处理中")));
        }

        let result = tokio::time::timeout(self.handler_timeout, async {
            for handler in targets {
                handler.handle(msg.clone()).await?;
            }
            Ok::<(), Error>(())
        })
        .await;

        self.release(&key);

        match result {
            Ok(inner) => inner.map(|()| Dispatch::Processed),
            Err(_elapsed) => Err(Error::Internal(anyhow::anyhow!(
                "handler 超时：超过 {:?} 未返回",
                self.handler_timeout
            ))),
        }
    }
}

// ---------------- 收件箱（去重 + 审计 + 自愈） ----------------

/// 收件箱里的一行。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InboxRow {
    pub attempts: i32,
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
}

/// 记一条消息。返回 `false` 表示 `(serial, seq)` 已存在（重复到达）。
pub async fn inbox_store(pool: &PgPool, msg: &Message) -> Result<bool> {
    let res = sqlx::query(
        "INSERT INTO device_message_inbox
           (serial, seq, device_slot, kind, payload)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (serial, seq) DO NOTHING",
    )
    .bind(&msg.serial)
    .bind(msg.seq)
    .bind(&msg.slot)
    .bind(&msg.kind)
    .bind(&msg.payload)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// 读一行的处理状态。
pub async fn inbox_state(pool: &PgPool, serial: &str, seq: i64) -> Result<Option<InboxRow>> {
    sqlx::query_as(
        "SELECT attempts, processed_at, last_error
         FROM device_message_inbox WHERE serial = $1 AND seq = $2",
    )
    .bind(serial)
    .bind(seq)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// 记一次**成功**的处理结论（终态）。`note` 用于记"确定的非业务结论"，
/// 例如"没有订阅方"——它同样是终态，否则客户端会为一个重试也没用的
/// 协议不匹配反复重发。
pub async fn inbox_mark_processed(
    pool: &PgPool,
    serial: &str,
    seq: i64,
    note: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE device_message_inbox
         SET attempts = attempts + 1,
             processed_at = now(),
             last_error = $3
         WHERE serial = $1 AND seq = $2",
    )
    .bind(serial)
    .bind(seq)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(())
}

/// 记一次**失败**：不推 processed_at，这样客户端的重发会走自愈重投。
pub async fn inbox_mark_failed(pool: &PgPool, serial: &str, seq: i64, reason: &str) -> Result<()> {
    sqlx::query(
        "UPDATE device_message_inbox
         SET attempts = attempts + 1,
             processed_at = NULL,
             last_error = $3
         WHERE serial = $1 AND seq = $2",
    )
    .bind(serial)
    .bind(seq)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// 推进去重水位（只在**处理成功**后调用）。
pub async fn bump_watermark(pool: &PgPool, serial: &str, slot: &str, seq: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO device_session (serial, device_slot, last_seen_seq, state, updated_at)
         VALUES ($1, $2, $3, 'online', now())
         ON CONFLICT (serial) DO UPDATE SET
           last_seen_seq = GREATEST(device_session.last_seen_seq, EXCLUDED.last_seen_seq),
           state = 'online',
           updated_at = now()",
    )
    .bind(serial)
    .bind(slot)
    .bind(seq)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------- 下行队列（服务端 → 设备） ----------------

/// 单台设备**未 ACK** 的下行消息上限：达到后拒绝继续投递（背压）。
pub const MAX_UNACKED_DOWNLINK: i64 = 1000;

/// 进程内「该设备有新下行消息」的通知器。
///
/// 长连接（WebSocket 隧道）用它做到达即推，而不是每秒轮询数据库。
///
/// 两点说明：
/// - 这是**进程内**的；多进程部署时另一进程的投递不会唤醒本进程，
///   所以长连接侧仍保留一个低频兜底轮询
/// - `notify_waiters` 只唤醒"此刻正在等"的连接；正在处理消息时的投递会漏掉，
///   因此长连接在每处理完一条后都会再拉一次队列
#[derive(Debug, Default)]
pub struct Notifier {
    waiters: dashmap::DashMap<String, std::sync::Arc<tokio::sync::Notify>>,
}

impl Notifier {
    /// 取（或创建）某台设备的通知句柄。
    pub fn waiter(&self, serial: &str) -> std::sync::Arc<tokio::sync::Notify> {
        self.waiters.entry(serial.to_owned()).or_default().clone()
    }

    /// 唤醒该设备当前正在等待的长连接。
    pub fn notify(&self, serial: &str) {
        if let Some(w) = self.waiters.get(serial) {
            w.notify_waiters();
        }
    }
}

static GLOBAL_NOTIFIER: std::sync::LazyLock<Notifier> = std::sync::LazyLock::new(Notifier::default);

/// 进程级通知器（`downlink_publish*` 投递成功后会自动用它唤醒长连接）。
pub fn notifier() -> &'static Notifier {
    &GLOBAL_NOTIFIER
}

/// 跨进程的下行通知信号。
///
/// 进程内的唤醒由 [`Notifier`] 负责（零成本）；这个信号负责**告诉其它实例**
/// "该设备有新消息了"。不装它时多实例部署会退化为 2s 兜底轮询，
/// 装了之后延迟与单实例一致。
///
/// 实现约定：`publish` 应当**尽快返回**，失败不应影响主流程
/// （兜底轮询会把漏掉的消息捞回来）。
pub trait CrossProcessSignal: Send + Sync + 'static {
    fn publish(&self, serial: String) -> BoxFuture<'static, ()>;
}

static CROSS_PROCESS: std::sync::OnceLock<Arc<dyn CrossProcessSignal>> = std::sync::OnceLock::new();

/// 安装跨进程通知信号。**只能装一次**（进程级单例），装过返回 `false`。
pub fn install_cross_process(signal: Arc<dyn CrossProcessSignal>) -> bool {
    CROSS_PROCESS.set(signal).is_ok()
}

fn cross_process() -> Option<&'static Arc<dyn CrossProcessSignal>> {
    CROSS_PROCESS.get()
}

/// 唤醒**所有进程**里等待该设备的长连接。
///
/// 顺序有讲究：先唤醒本进程（同步、零延迟），再通知其它进程（异步、尽力而为）。
pub async fn signal_downlink(serial: &str) {
    notifier().notify(serial);
    if let Some(signal) = cross_process() {
        signal.publish(serial.to_owned()).await;
    }
}

/// 一次下行投递。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DownlinkMessage {
    /// 全局自增游标：设备用它做"从哪继续拉"
    pub id: i64,
    pub kind: String,
    pub payload: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 校验 `kind`（与上行同一套规则，两端口径一致）。
pub fn validate_kind(kind: &str) -> Result<()> {
    if kind.is_empty() || kind.len() > 64 {
        return Err(Error::BadRequest("kind 长度必须在 1..=64".into()));
    }
    Ok(())
}

/// 校验载荷大小。
pub fn validate_payload(payload: &str, max: usize) -> Result<()> {
    if payload.len() > max {
        return Err(Error::BadRequest(format!("payload 超过 {max} 字节")));
    }
    Ok(())
}

/// 给指定设备排一条下行消息，返回游标 `id`。
///
/// 背压：单台设备未 ACK 的消息超过 [`MAX_UNACKED_DOWNLINK`] 时报错，
/// 而不是无限堆积 —— 设备失联时业务方必须感知，而不是把库写爆。
///
/// 投递成功会顺手唤醒该设备的长连接（见 [`notifier`]），实现到达即推。
pub async fn downlink_publish(
    pool: &PgPool,
    serial: &str,
    slot: &str,
    kind: &str,
    payload: &str,
) -> Result<i64> {
    validate_kind(kind)?;
    validate_payload(payload, MAX_MESSAGE_BYTES)?;

    let unacked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM device_downlink
         WHERE serial = $1 AND delivered_at IS NULL",
    )
    .bind(serial)
    .fetch_one(pool)
    .await?;
    if unacked >= MAX_UNACKED_DOWNLINK {
        return Err(Error::DownlinkFull(unacked as usize));
    }

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO device_downlink (serial, device_slot, kind, payload)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(serial)
    .bind(slot)
    .bind(kind)
    .bind(payload)
    .fetch_one(pool)
    .await?;
    // 唤醒长连接：本进程立即、其它进程尽力而为（跨进程信号未安装时退化为兜底轮询）
    signal_downlink(serial).await;
    Ok(id)
}

/// 按 slot 给**当前存活证书**投递；没有存活证书时返回 `Ok(None)`。
///
/// 解析范围是 `issued`/`active`（与 mTLS 握手允许的范围一致）：设备拿到证书
/// 但还没跑 `confirm` 时也可以先把消息排上，等它回连自然会拉到。
pub async fn downlink_publish_to_slot(
    pool: &PgPool,
    slot: &str,
    kind: &str,
    payload: &str,
) -> Result<Option<i64>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT serial, device_slot FROM device_enrollment
         WHERE device_slot = $1 AND status IN ('issued','active')
         ORDER BY (status = 'active') DESC, activated_at DESC NULLS LAST
         LIMIT 1",
    )
    .bind(slot)
    .fetch_optional(pool)
    .await?;
    match row {
        Some((serial, device_slot)) => downlink_publish(pool, &serial, &device_slot, kind, payload)
            .await
            .map(Some),
        None => Ok(None),
    }
}

/// 设备拉取：返回 `id > after` 的未投递消息（按 id 升序）。
///
/// **拉取不等于已投递**：设备必须处理成功后再 `downlink_ack`，
/// 否则下次还会拿到同一批（至少一次，与上行对称）。
pub async fn downlink_fetch(
    pool: &PgPool,
    serial: &str,
    after: i64,
    limit: i64,
) -> Result<Vec<DownlinkMessage>> {
    sqlx::query_as(
        "SELECT id, kind, payload, created_at FROM device_downlink
         WHERE serial = $1 AND id > $2 AND delivered_at IS NULL
         ORDER BY id
         LIMIT $3",
    )
    .bind(serial)
    .bind(after)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 设备 ACK：把 `id <= up_to` 标记为已投递（保留一段时间供审计，之后由
/// [`downlink_cleanup`] 回收）。重复 ACK 是幂等的。
pub async fn downlink_ack(pool: &PgPool, serial: &str, up_to: i64) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE device_downlink SET delivered_at = now()
         WHERE serial = $1 AND id <= $2 AND delivered_at IS NULL",
    )
    .bind(serial)
    .bind(up_to)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 回收已投递且超过保留期的下行消息（运维定期调用）。
pub async fn downlink_cleanup(pool: &PgPool, keep: chrono::Duration) -> Result<u64> {
    let cutoff = chrono::Utc::now() - keep;
    let res = sqlx::query(
        "DELETE FROM device_downlink
         WHERE delivered_at IS NOT NULL AND delivered_at < $1",
    )
    .bind(cutoff)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

// ---------------- 在线 / 离线判定 ----------------

/// 跑一轮离线判定：把每个「超时未活动」的在线会话推进防抖窗口。
///
/// 单独抽出来是为了可测：真实的循环只是 `interval` + 这个函数。
pub async fn offline_tick(sessions: &SessionManager) {
    for serial in sessions.online_serials() {
        sessions.heartbeat_miss(&serial).await;
    }
}

/// 启动离线判定定时器。
///
/// 没有它，`SessionManager::heartbeat_miss` 永远没人调用 —— 设备只会被
/// "消息到达时翻回 online"，永远不会被判 offline（契约 §6 的心跳判定落空）。
pub fn spawn_offline_detector(sessions: Arc<SessionManager>, interval: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            offline_tick(&sessions).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn dispatcher() -> Dispatcher {
        Dispatcher::new(Duration::from_secs(1))
    }

    fn msg(kind: &str) -> Message {
        Message {
            serial: "s1".into(),
            slot: "slot-1".into(),
            seq: 1,
            kind: kind.into(),
            payload: "{}".into(),
            received_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn dispatch_calls_subscribed_handler() {
        let d = dispatcher();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        d.subscribe("telemetry", move |_m: Message| {
            let h = Arc::clone(&h);
            async move {
                h.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        assert!(d.has_handler("telemetry"));
        assert_eq!(
            d.dispatch(msg("telemetry")).await.unwrap(),
            Dispatch::Processed
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            d.dispatch(msg("telemetry")).await.unwrap(),
            Dispatch::Processed
        );
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unknown_kind_is_nohandler_not_error() {
        let d = dispatcher();
        assert!(!d.has_handler("whatever"));
        assert_eq!(
            d.dispatch(msg("whatever")).await.unwrap(),
            Dispatch::NoHandler
        );
    }

    #[tokio::test]
    async fn wildcard_catches_everything() {
        let d = dispatcher();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        d.subscribe_all(move |_m: Message| {
            let h = Arc::clone(&h);
            async move {
                h.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        assert_eq!(d.dispatch(msg("a")).await.unwrap(), Dispatch::Processed);
        assert_eq!(d.dispatch(msg("b")).await.unwrap(), Dispatch::Processed);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn handler_failure_bubbles_up() {
        let d = dispatcher();
        d.subscribe("boom", |_m: Message| async {
            Err(Error::Internal(anyhow::anyhow!("boom")))
        });
        assert!(d.dispatch(msg("boom")).await.is_err());
        // 失败后 in-flight 必须释放，否则这条消息永远卡住
        assert!(!d.is_in_flight("s1:1"));
    }

    #[tokio::test]
    async fn slow_handler_is_timed_out() {
        let d = Dispatcher::new(Duration::from_millis(30));
        d.subscribe("slow", |_m: Message| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        });
        let err = d.dispatch(msg("slow")).await.expect_err("必须超时");
        assert!(err.to_string().contains("超时"), "{err}");
        assert!(!d.is_in_flight("s1:1"), "超时也要释放 in-flight");
    }

    /// 离线判定必须真的把「超时未活动」的会话推成 offline。
    ///
    /// 这条路径此前是死代码：`heartbeat_miss` 写好了但没有任何定时器调用，
    /// 设备永远不会被判 offline。
    #[tokio::test]
    async fn offline_tick_marks_idle_session_offline() {
        let cfg = crate::session::DebounceConfig {
            window: Duration::from_millis(10),
            heartbeat_timeout: Duration::from_millis(1),
            miss_threshold: 1,
        };
        let (sessions, mut rx) = crate::session::SessionManager::new(cfg);
        sessions.upsert("s1", "slot-1", crate::session::SessionState::Online, 0);
        assert_eq!(sessions.online_serials(), vec!["s1".to_owned()]);

        // 让它安静到超过 heartbeat_timeout
        tokio::time::sleep(Duration::from_millis(30)).await;
        offline_tick(&sessions).await;

        let ev = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("防抖窗口内应产出事件")
            .expect("事件通道不应关闭");
        assert_eq!(ev.serial, "s1");
        assert_eq!(ev.state, crate::session::SessionState::Offline);
        assert!(
            sessions.online_serials().is_empty(),
            "翻转后不应再被当成在线"
        );
    }

    /// 有活动的会话不能被判 offline。
    #[tokio::test]
    async fn offline_tick_keeps_active_session_online() {
        let cfg = crate::session::DebounceConfig {
            window: Duration::from_millis(10),
            heartbeat_timeout: Duration::from_secs(5),
            miss_threshold: 1,
        };
        let (sessions, mut rx) = crate::session::SessionManager::new(cfg);
        sessions.upsert("s1", "slot-1", crate::session::SessionState::Online, 0);
        offline_tick(&sessions).await;
        let ev = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(ev.is_err(), "刚活动过的会话不应被判 offline");
    }

    /// 同一条消息并发到达只能处理一次（at-least-once 的重复是常态，
    /// 但同一进程内不应并行跑两遍 handler）。
    #[tokio::test]
    async fn concurrent_same_message_is_not_double_processed() {
        let d = Arc::new(Dispatcher::new(Duration::from_secs(2)));
        let hits = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());

        let h = Arc::clone(&hits);
        let g = Arc::clone(&gate);
        d.subscribe("slow", move |_m: Message| {
            let h = Arc::clone(&h);
            let g = Arc::clone(&g);
            async move {
                h.fetch_add(1, Ordering::SeqCst);
                g.notified().await;
                Ok(())
            }
        });

        let d2 = Arc::clone(&d);
        let task = tokio::spawn(async move { d2.dispatch(msg("slow")).await });

        // 等第一条进入处理（claim 成功、handler 尚未结束）
        for _ in 0..200 {
            if d.is_in_flight("s1:1") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(d.is_in_flight("s1:1"), "第一条应处于处理中");

        let second = d.dispatch(msg("slow")).await;
        assert!(second.is_err(), "并发重复应被挡下，实际 {second:?}");

        gate.notify_one();
        let first = task.await.unwrap().expect("第一条应成功");
        assert_eq!(first, Dispatch::Processed);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "handler 只应被执行一次");
        assert!(!d.is_in_flight("s1:1"), "处理完必须释放 in-flight");
    }
}
