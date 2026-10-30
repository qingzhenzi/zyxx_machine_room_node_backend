//! 防抖会话层（契约第 6 节）。
//!
//! 解决的问题：网络抖动会造成设备反复上线/下线，如果每次都广播状态变更，
//! 上游会被事件风暴打爆。这里的策略是：
//!
//! 1. `seq` 去重 —— 重连后重放的消息按 `(serial, seq)` 幂等
//! 2. 防抖窗口 —— 状态翻转先进待定槽，窗口（默认 250ms）结束后只发一次最终状态
//! 3. 心跳判定 —— 连续 `MISS_THRESHOLD` 次（默认 3）未达才判 offline，恢复同理
//!
//! 内存态用 `DashMap`，关键状态落 `device_session` 表，进程重启可恢复。

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;

/// 会话的对外可见状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Online,
    Offline,
}

impl SessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionState::Online => "online",
            SessionState::Offline => "offline",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "offline" => SessionState::Offline,
            _ => SessionState::Online,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub serial: String,
    pub device_slot: String,
    pub state: SessionState,
    pub last_seen_seq: i64,
}

/// 单会话的内存状态。
#[derive(Debug)]
struct SessionEntry {
    device_slot: String,
    /// 已确认的最新状态
    confirmed: SessionState,
    /// 待定状态（防抖窗口内可能被覆盖）
    pending: Option<SessionState>,
    last_seen_seq: i64,
    last_seen_at: Instant,
    consecutive_misses: u32,
}

#[derive(Debug, Clone)]
pub struct DebounceConfig {
    /// 防抖窗口
    pub window: Duration,
    /// 心跳超时（超过这个时间未收到消息算 miss）
    pub heartbeat_timeout: Duration,
    /// 连续多少次 miss 判 offline
    pub miss_threshold: u32,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_millis(250),
            heartbeat_timeout: Duration::from_secs(5),
            miss_threshold: 3,
        }
    }
}

/// 防抖会话管理器。
pub struct SessionManager {
    sessions: Arc<DashMap<String, SessionEntry>>,
    config: DebounceConfig,
    tx: mpsc::Sender<SessionEvent>,
}

/// 消息去重结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Dedup {
    /// 新消息，应被处理
    Fresh,
    /// 重复消息（seq 不大于已见值），应丢弃
    Duplicate,
}

impl SessionManager {
    /// 创建管理器，返回 `(manager, event_receiver)`。
    pub fn new(config: DebounceConfig) -> (Self, mpsc::Receiver<SessionEvent>) {
        let (tx, rx) = mpsc::channel(256);
        (
            Self {
                sessions: Arc::new(DashMap::new()),
                config,
                tx,
            },
            rx,
        )
    }

    /// 恢复/注册会话的内存态。
    pub fn upsert(&self, serial: &str, device_slot: &str, state: SessionState, seq: i64) {
        self.sessions.insert(
            serial.to_owned(),
            SessionEntry {
                device_slot: device_slot.to_owned(),
                confirmed: state,
                pending: None,
                last_seen_seq: seq,
                last_seen_at: Instant::now(),
                consecutive_misses: 0,
            },
        );
    }

    /// 记录一次活动消息并做 seq 去重。
    ///
    /// 返回 `Duplicate` 时调用方应丢弃该消息，不重复处理副作用。
    pub fn observe(&self, serial: &str, seq: i64) -> Dedup {
        let mut entry = match self.sessions.get_mut(serial) {
            Some(e) => e,
            None => return Dedup::Fresh, // 未知会话不在这里拦截
        };
        if seq <= entry.last_seen_seq {
            return Dedup::Duplicate;
        }
        entry.last_seen_seq = seq;
        entry.last_seen_at = Instant::now();
        entry.consecutive_misses = 0;
        Dedup::Fresh
    }

    pub fn last_seen_seq(&self, serial: &str) -> Option<i64> {
        self.sessions.get(serial).map(|e| e.last_seen_seq)
    }

    /// 标记一次心跳 miss。达到阈值时把状态翻转排进防抖窗口。
    pub async fn heartbeat_miss(&self, serial: &str) {
        let (slot, should_offline) = {
            let mut entry = match self.sessions.get_mut(serial) {
                Some(e) => e,
                None => return,
            };
            if entry.last_seen_at.elapsed() < self.config.heartbeat_timeout {
                return;
            }
            entry.consecutive_misses += 1;
            let hit = entry.consecutive_misses >= self.config.miss_threshold
                && entry.confirmed == SessionState::Online;
            if hit {
                entry.consecutive_misses = 0;
            }
            (entry.device_slot.clone(), hit)
        };
        if should_offline {
            self.schedule_flip(serial, &slot, SessionState::Offline).await;
        }
    }

    /// 收到活动消息时，若当前是 offline 则排一次 online 翻转。
    pub async fn mark_seen(&self, serial: &str) {
        let (slot, should_online) = {
            let mut entry = match self.sessions.get_mut(serial) {
                Some(e) => e,
                None => return,
            };
            entry.last_seen_at = Instant::now();
            entry.consecutive_misses = 0;
            let hit = entry.confirmed == SessionState::Offline;
            (entry.device_slot.clone(), hit)
        };
        if should_online {
            self.schedule_flip(serial, &slot, SessionState::Online).await;
        }
    }

    /// 把状态翻转排进防抖窗口：窗口内多次翻转只发一次最终状态。
    ///
    /// 核心防抖语义：同一会话已有 pending 时不新起计时器，
    /// pending 被最新状态覆盖，窗口到期一次提交。
    async fn schedule_flip(&self, serial: &str, device_slot: &str, state: SessionState) {
        let arm_timer = {
            let mut entry = match self.sessions.get_mut(serial) {
                Some(e) => e,
                None => return,
            };
            if entry.pending.is_none() {
                entry.pending = Some(state);
                true
            } else {
                entry.pending = Some(state);
                false
            }
        };
        if !arm_timer {
            return;
        }

        let sessions = Arc::clone(&self.sessions);
        let tx = self.tx.clone();
        let window = self.config.window;
        let serial_owned = serial.to_owned();
        let slot_owned = device_slot.to_owned();

        tokio::spawn(async move {
            sleep(window).await;
            let committed = {
                let mut entry = match sessions.get_mut(&serial_owned) {
                    Some(e) => e,
                    None => return,
                };
                match entry.pending.take() {
                    Some(state) if state != entry.confirmed => {
                        entry.confirmed = state;
                        Some((state, entry.last_seen_seq))
                    }
                    _ => None,
                }
            };
            if let Some((state, seq)) = committed {
                let _ = tx
                    .send(SessionEvent {
                        serial: serial_owned,
                        device_slot: slot_owned,
                        state,
                        last_seen_seq: seq,
                    })
                    .await;
            }
        });
    }

    pub fn confirmed_state(&self, serial: &str) -> Option<SessionState> {
        self.sessions.get(serial).map(|e| e.confirmed)
    }
}

/// 指数退避参数（客户端复用同一份语义）。
#[derive(Debug, Clone)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    attempt: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(30),
            attempt: 0,
        }
    }
}

impl Backoff {
    /// 下一次退避时长（含 jitter）。
    pub fn next_delay(&mut self) -> Duration {
        let exp = self.initial.saturating_mul(1u32 << self.attempt.min(9));
        let capped = exp.min(self.max);
        self.attempt = self.attempt.saturating_add(1);
        // jitter: ±20%
        let jitter_range = capped.as_millis() as u64 / 5;
        let jitter = if jitter_range == 0 {
            0
        } else {
            (fastrand_u64() % jitter_range) as u64
        };
        capped + Duration::from_millis(jitter)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// 轻量随机数（避免为 jitter 引入额外 RNG 依赖）。
fn fastrand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(Instant::now().elapsed().as_nanos() as u64);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_seq_is_rejected() {
        let (mgr, _rx) = SessionManager::new(DebounceConfig::default());
        mgr.upsert("s1", "slot-1", SessionState::Online, 0);
        assert_eq!(mgr.observe("s1", 5), Dedup::Fresh);
        assert_eq!(mgr.observe("s1", 5), Dedup::Duplicate);
        assert_eq!(mgr.observe("s1", 4), Dedup::Duplicate);
        assert_eq!(mgr.observe("s1", 6), Dedup::Fresh);
        assert_eq!(mgr.last_seen_seq("s1"), Some(6));
    }

    #[tokio::test]
    async fn flip_is_debounced_into_single_event() {
        let cfg = DebounceConfig {
            window: Duration::from_millis(50),
            ..Default::default()
        };
        let (mgr, mut rx) = SessionManager::new(cfg);
        mgr.upsert("s1", "slot-1", SessionState::Online, 0);

        // 连续排入 offline → online，窗口到期只应提交最后一个
        mgr.schedule_flip("s1", "slot-1", SessionState::Offline).await;
        mgr.schedule_flip("s1", "slot-1", SessionState::Online).await;

        // online == confirmed，所以不应产生事件
        let got = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(got.is_err() || got.unwrap().is_none());
        assert_eq!(mgr.confirmed_state("s1"), Some(SessionState::Online));
    }

    #[tokio::test]
    async fn single_flip_emits_one_event() {
        let cfg = DebounceConfig {
            window: Duration::from_millis(30),
            ..Default::default()
        };
        let (mgr, mut rx) = SessionManager::new(cfg);
        mgr.upsert("s1", "slot-1", SessionState::Online, 7);
        mgr.schedule_flip("s1", "slot-1", SessionState::Offline).await;
        let ev = tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .expect("event within window")
            .expect("event");
        assert_eq!(ev.state, SessionState::Offline);
        assert_eq!(ev.device_slot, "slot-1");
        assert_eq!(ev.last_seen_seq, 7);
    }

    #[tokio::test]
    async fn repeated_same_flip_is_idempotent() {
        let cfg = DebounceConfig {
            window: Duration::from_millis(30),
            ..Default::default()
        };
        let (mgr, mut rx) = SessionManager::new(cfg);
        mgr.upsert("s1", "slot-1", SessionState::Offline, 1);
        for _ in 0..5 {
            mgr.schedule_flip("s1", "slot-1", SessionState::Online).await;
        }
        let first = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
        assert!(first.is_ok(), "should emit one flip");
        // 不应有第二个事件
        let second = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(second.is_err(), "no duplicate event");
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mut b = Backoff {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(1),
            attempt: 0,
        };
        let d0 = b.next_delay();
        let d1 = b.next_delay();
        let d2 = b.next_delay();
        assert!(d0 >= Duration::from_millis(100));
        assert!(d1 >= d0);
        assert!(d2 >= d1);
        for _ in 0..20 {
            let d = b.next_delay();
            assert!(d <= Duration::from_millis(1200), "capped with jitter: {d:?}");
        }
        b.reset();
        assert!(b.next_delay() >= Duration::from_millis(100));
    }

    #[test]
    fn state_string_roundtrip() {
        assert_eq!(SessionState::parse(SessionState::Online.as_str()), SessionState::Online);
        assert_eq!(SessionState::parse(SessionState::Offline.as_str()), SessionState::Offline);
    }
}
