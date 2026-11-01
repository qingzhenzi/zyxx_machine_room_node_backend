//! 设备在线事件：持久化、可靠投递与本地广播（P3）。
//!
//! device-pki 负责心跳判定并把状态写进 `device_session`；本模块在 qe-api 侧周期
//! 扫描 `device_session ⋈ rooms ⋈ areas`，检测 `online -> offline`（可选
//! `offline -> online`）翻转：
//!
//! 1. 翻转事件先落 `device_presence_events`（唯一约束去重，多副本只插一行）；
//! 2. 目标 URL 按 **房间 → 区域 → 全局** 回退，有目标才建投递行
//!    （`device_presence_deliveries`，`(event_id, target_url)` 唯一）；
//! 3. 每 tick 用 `FOR UPDATE SKIP LOCKED` 领取到期投递（租约 60s），失败按
//!    `min(300s, base * 2^(attempts-1))` 指数退避，达到上限置 `failed`；
//! 4. 事件落库后通过 `tokio::sync::broadcast` 做本地推送（SSE 用），跨副本 SSE
//!    由查库兜底。
//!
//! 状态仍以 `device_session` 为准；事件/投递表只记录「发生过什么」，可安全重放。

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::{
    sync::broadcast,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use uuid::Uuid;

use crate::{
    db::presence::{NewPresenceEvent, PresenceEventRecord},
    state::AppState,
};

const EVENT_OFFLINE: &str = "device.offline";
const EVENT_ONLINE: &str = "device.online";
/// 指数退避上限：无论尝试多少次，下次重试最多等 5 分钟。
const MAX_RETRY_DELAY_SECS: u64 = 300;

/// 一行设备在线状态（含房间/区域与 webhook 配置）。
#[derive(Debug, Clone)]
pub struct PresenceRow {
    pub serial: String,
    pub device_slot: String,
    pub online: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    /// 触发本次扫描结果的 `device_session.updated_at`，作为事件幂等键的一部分。
    pub source_updated_at: DateTime<Utc>,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    /// 房间级 webhook（`rooms.webhook_url`）。
    pub webhook_url: Option<String>,
    pub area_id: Uuid,
    pub area_code: String,
    pub area_name: String,
    /// 区域级 webhook（`areas.webhook_url`）。
    pub area_webhook_url: Option<String>,
}

/// 对外投递的状态变化事件。`webhook_url` / `area_webhook_url` /
/// `source_updated_at` 只用于内部路由，不进入 webhook / SSE 的 JSON。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PresenceEvent {
    pub event: String,
    pub occurred_at: DateTime<Utc>,
    pub serial: String,
    pub device_slot: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub previous_state: String,
    pub current_state: String,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    pub area_id: Uuid,
    pub area_code: String,
    pub area_name: String,
    #[serde(skip)]
    pub webhook_url: Option<String>,
    #[serde(skip)]
    pub area_webhook_url: Option<String>,
    #[serde(skip)]
    pub source_updated_at: DateTime<Utc>,
}

/// 纯状态机：第一轮只建立基线，之后检测翻转。
#[derive(Debug, Default)]
pub struct PresenceTracker {
    last_online: HashMap<String, bool>,
}

impl PresenceTracker {
    /// 第一轮返回空（避免 qe-api 启动时把所有离线设备重放一遍）。
    pub fn observe(&mut self, rows: &[PresenceRow], notify_online: bool) -> Vec<PresenceEvent> {
        let mut events = Vec::new();
        let mut seen = std::collections::HashSet::with_capacity(rows.len());

        for row in rows {
            seen.insert(row.serial.clone());
            let previous = self.last_online.insert(row.serial.clone(), row.online);
            let Some(previous_online) = previous else {
                continue;
            };
            if previous_online == row.online {
                continue;
            }
            if !row.online || notify_online {
                events.push(row.to_event(previous_online));
            }
        }

        // 不再出现在扫描结果里的设备（解绑/解注册）从基线移除，避免残留状态。
        self.last_online.retain(|serial, _| seen.contains(serial));
        events
    }
}

impl PresenceRow {
    fn to_event(&self, previous_online: bool) -> PresenceEvent {
        PresenceEvent {
            event: if self.online {
                EVENT_ONLINE.to_owned()
            } else {
                EVENT_OFFLINE.to_owned()
            },
            occurred_at: Utc::now(),
            serial: self.serial.clone(),
            device_slot: self.device_slot.clone(),
            last_seen_at: self.last_seen_at,
            previous_state: state_name(previous_online),
            current_state: state_name(self.online),
            room_id: self.room_id,
            room_code: self.room_code.clone(),
            room_name: self.room_name.clone(),
            area_id: self.area_id,
            area_code: self.area_code.clone(),
            area_name: self.area_name.clone(),
            webhook_url: self.webhook_url.clone(),
            area_webhook_url: self.area_webhook_url.clone(),
            source_updated_at: self.source_updated_at,
        }
    }
}

fn state_name(online: bool) -> String {
    if online { "online" } else { "offline" }.to_owned()
}

/// webhook 目标解析：房间 → 区域 → 全局；空串 / 纯空白视为未配置。
pub fn resolve_webhook_url<'a>(
    room_url: Option<&'a str>,
    area_url: Option<&'a str>,
    global_url: Option<&'a str>,
) -> Option<&'a str> {
    room_url
        .into_iter()
        .chain(area_url)
        .chain(global_url)
        .find(|url| !url.trim().is_empty())
}

/// 第 `attempts` 次失败后的重试间隔：`min(300s, base * 2^(attempts-1))`。
///
/// `attempts` 从 1 开始（首次失败后的下一次重试）。
fn retry_delay(base_secs: u64, attempts: i32) -> ChronoDuration {
    let exponent = attempts.saturating_sub(1).clamp(0, 16) as u32;
    let secs = base_secs
        .saturating_mul(1_u64 << exponent)
        .min(MAX_RETRY_DELAY_SECS);
    ChronoDuration::seconds(secs as i64)
}

/// 订阅本地事件广播。SSE handler 应先查库补发历史事件，再消费此 receiver。
pub fn subscribe(state: &AppState) -> broadcast::Receiver<PresenceEventRecord> {
    state.presence_events.subscribe()
}

/// 启动后台在线状态通知循环。
pub fn spawn(state: AppState) -> JoinHandle<()> {
    let tick_secs = state.config.device_presence_tick_secs.max(1);
    let timeout_secs = state.config.device_presence_webhook_timeout_secs.max(1);
    let notify_online = state.config.device_presence_notify_online;
    let global_webhook_url = state.config.device_presence_global_webhook_url.clone();
    let max_attempts = state.config.device_presence_webhook_max_attempts.max(1);
    let batch = state.config.device_presence_webhook_batch.max(1);
    let retry_base_secs = state.config.device_presence_retry_base_seconds.max(1);

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(%error, "构建设备在线状态 webhook 客户端失败，通知任务退出");
                return;
            }
        };
        let mut tracker = PresenceTracker::default();
        let mut ticker = interval(Duration::from_secs(tick_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            match load_rows(state.db.pool()).await {
                Ok(rows) => {
                    for event in tracker.observe(&rows, notify_online) {
                        persist_event(&state, event, global_webhook_url.as_deref()).await;
                    }
                }
                Err(error) => tracing::warn!(%error, "扫描设备在线状态失败"),
            }

            deliver_due(&state, &client, batch, max_attempts, retry_base_secs).await;
        }
    })
}

/// 落库一个翻转事件：插入成功（非重复）后广播，并按回退链创建投递。
async fn persist_event(state: &AppState, event: PresenceEvent, global_webhook_url: Option<&str>) {
    tracing::info!(
        event = %event.event,
        serial = %event.serial,
        device_slot = %event.device_slot,
        room_id = %event.room_id,
        "设备在线状态变化"
    );

    let payload = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
    let new_event = NewPresenceEvent {
        serial: event.serial.clone(),
        device_slot: event.device_slot.clone(),
        room_id: Some(event.room_id),
        area_id: Some(event.area_id),
        previous_state: event.previous_state.clone(),
        current_state: event.current_state.clone(),
        source_updated_at: event.source_updated_at,
        payload: payload.clone(),
    };

    let event_id = match state.presence.insert_event(&new_event).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            // 多副本同时扫描到同一翻转，另一个副本已经落库。
            tracing::debug!(serial = %event.serial, "重复的设备在线事件已忽略");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, serial = %event.serial, "写入设备在线事件失败");
            return;
        }
    };

    let record = PresenceEventRecord {
        id: event_id,
        serial: event.serial.clone(),
        device_slot: event.device_slot.clone(),
        room_id: Some(event.room_id),
        area_id: Some(event.area_id),
        previous_state: event.previous_state.clone(),
        current_state: event.current_state.clone(),
        occurred_at: event.occurred_at,
        payload,
    };
    // 没有订阅者时返回 Err，属于正常情况，忽略即可。
    let _ = state.presence_events.send(record);

    let target = resolve_webhook_url(
        event.webhook_url.as_deref(),
        event.area_webhook_url.as_deref(),
        global_webhook_url,
    );
    let Some(target) = target else {
        tracing::debug!(event_id, serial = %event.serial, "无 webhook 目标，事件仅落库");
        return;
    };

    match state.presence.add_delivery(event_id, target).await {
        Ok(true) => {
            tracing::debug!(event_id, target_url = %target, "已创建设备在线事件投递");
        }
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, event_id, target_url = %target, "创建设备在线事件投递失败");
        }
    }
}

/// 领取并投递一批到期事件；失败按配置退避重试。
async fn deliver_due(
    state: &AppState,
    client: &reqwest::Client,
    batch: i64,
    max_attempts: u32,
    retry_base_secs: u64,
) {
    let deliveries = match state.presence.claim_due(batch).await {
        Ok(deliveries) => deliveries,
        Err(error) => {
            tracing::warn!(%error, "领取设备在线事件投递失败");
            return;
        }
    };

    for delivery in deliveries {
        let event = match state.presence.get_event(delivery.event_id).await {
            Ok(Some(event)) => event,
            Ok(None) => {
                // 事件被删除（投递表本应级联删除，这里只做兜底）。
                let attempts = delivery.attempts.saturating_add(1);
                if let Err(error) = state
                    .presence
                    .mark_failed(delivery.id, attempts, "关联事件不存在")
                    .await
                {
                    tracing::warn!(%error, delivery_id = delivery.id, "标记投递失败状态出错");
                }
                continue;
            }
            Err(error) => {
                // 保留 60s 租约，下一轮再试，避免一次查询失败就把投递判死。
                tracing::warn!(%error, delivery_id = delivery.id, "读取设备在线事件失败");
                continue;
            }
        };

        let event_name = event
            .payload
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("device.presence");
        let response = client
            .post(&delivery.target_url)
            .header("X-QE-Event", event_name)
            .header("X-QE-Event-Id", event.id.to_string())
            .header("X-QE-Delivery-Id", delivery.id.to_string())
            .json(&event.payload)
            .send()
            .await;

        match response {
            Ok(response) if response.status().is_success() => {
                if let Err(error) = state.presence.mark_delivered(delivery.id).await {
                    tracing::warn!(%error, delivery_id = delivery.id, "标记投递成功状态出错");
                } else {
                    tracing::info!(
                        %event_name,
                        serial = %event.serial,
                        delivery_id = delivery.id,
                        attempts = delivery.attempts + 1,
                        status = %response.status(),
                        "设备在线状态 webhook 投递成功"
                    );
                }
            }
            Ok(response) => {
                let reason = format!("HTTP {}", response.status());
                handle_delivery_failure(
                    state,
                    delivery.id,
                    delivery.attempts,
                    max_attempts,
                    retry_base_secs,
                    &reason,
                )
                .await;
            }
            Err(error) => {
                handle_delivery_failure(
                    state,
                    delivery.id,
                    delivery.attempts,
                    max_attempts,
                    retry_base_secs,
                    &error.to_string(),
                )
                .await;
            }
        }
    }
}

/// 失败处理：达到上限置 `failed`，否则写入下次重试时间。
async fn handle_delivery_failure(
    state: &AppState,
    delivery_id: i64,
    previous_attempts: i32,
    max_attempts: u32,
    retry_base_secs: u64,
    error: &str,
) {
    let attempts = previous_attempts.saturating_add(1);
    if attempts as u32 >= max_attempts {
        tracing::warn!(
            delivery_id,
            attempts,
            error,
            "设备在线状态 webhook 投递失败，已达最大尝试次数"
        );
        if let Err(error) = state
            .presence
            .mark_failed(delivery_id, attempts, error)
            .await
        {
            tracing::warn!(%error, delivery_id, "标记投递最终失败状态出错");
        }
        return;
    }

    let next_at = Utc::now() + retry_delay(retry_base_secs, attempts);
    tracing::warn!(
        delivery_id,
        attempts,
        next_attempt_at = %next_at,
        error,
        "设备在线状态 webhook 投递失败，稍后重试"
    );
    if let Err(error) = state
        .presence
        .mark_retry(delivery_id, attempts, next_at, error)
        .await
    {
        tracing::warn!(%error, delivery_id, "标记投递重试状态出错");
    }
}

type RawPresenceRow = (
    String,
    String,
    String,
    DateTime<Utc>,
    Uuid,
    String,
    String,
    Option<String>,
    Uuid,
    String,
    String,
    Option<String>,
);

async fn load_rows(pool: &PgPool) -> Result<Vec<PresenceRow>, sqlx::Error> {
    let rows: Vec<RawPresenceRow> = sqlx::query_as(
        "SELECT ds.serial, ds.device_slot, ds.state, ds.updated_at,
                r.id, r.code, r.name, r.webhook_url,
                a.id, a.code, a.name, a.webhook_url
           FROM device_session ds
           LEFT JOIN rooms rp ON rp.device_slot = ds.device_slot
           LEFT JOIN room_devices rd ON rd.device_slot = ds.device_slot
           JOIN rooms r ON r.id = COALESCE(rp.id, rd.room_id)
           JOIN areas a ON a.id = r.area_id
          WHERE r.is_active = true",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| PresenceRow {
            serial: row.0,
            device_slot: row.1,
            online: row.2 == "online",
            last_seen_at: Some(row.3),
            source_updated_at: row.3,
            room_id: row.4,
            room_code: row.5,
            room_name: row.6,
            webhook_url: row.7,
            area_id: row.8,
            area_code: row.9,
            area_name: row.10,
            area_webhook_url: row.11,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(serial: &str, online: bool) -> PresenceRow {
        PresenceRow {
            serial: serial.to_owned(),
            device_slot: "slot-1".into(),
            online,
            last_seen_at: None,
            source_updated_at: Utc::now(),
            room_id: Uuid::nil(),
            room_code: "r1".into(),
            room_name: "房间 1".into(),
            webhook_url: Some("http://127.0.0.1:1/hook".into()),
            area_id: Uuid::nil(),
            area_code: "a1".into(),
            area_name: "区域 1".into(),
            area_webhook_url: None,
        }
    }

    #[test]
    fn first_round_only_seeds_baseline() {
        let mut tracker = PresenceTracker::default();
        assert!(tracker.observe(&[row("s1", false)], false).is_empty());
    }

    #[test]
    fn online_to_offline_emits_offline_event() {
        let mut tracker = PresenceTracker::default();
        assert!(tracker.observe(&[row("s1", true)], false).is_empty());
        let events = tracker.observe(&[row("s1", false)], false);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, EVENT_OFFLINE);
        assert_eq!(events[0].previous_state, "online");
        assert_eq!(events[0].current_state, "offline");
        assert_eq!(events[0].serial, "s1");
        assert_eq!(events[0].room_code, "r1");
        assert_eq!(events[0].area_code, "a1");
    }

    #[test]
    fn repeated_offline_does_not_repeat_event() {
        let mut tracker = PresenceTracker::default();
        tracker.observe(&[row("s1", true)], false);
        assert_eq!(tracker.observe(&[row("s1", false)], false).len(), 1);
        assert!(tracker.observe(&[row("s1", false)], false).is_empty());
    }

    #[test]
    fn online_events_are_opt_in() {
        let mut tracker = PresenceTracker::default();
        tracker.observe(&[row("s1", false)], false);
        assert!(tracker.observe(&[row("s1", true)], false).is_empty());

        let mut tracker = PresenceTracker::default();
        tracker.observe(&[row("s1", false)], true);
        let events = tracker.observe(&[row("s1", true)], true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, EVENT_ONLINE);
    }

    #[test]
    fn disappeared_device_is_dropped_from_baseline() {
        let mut tracker = PresenceTracker::default();
        tracker.observe(&[row("s1", true)], false);
        assert!(tracker.observe(&[], false).is_empty());
        // 重新出现时重新建立基线，不产生事件
        assert!(tracker.observe(&[row("s1", false)], false).is_empty());
    }

    fn ts(raw: &str) -> DateTime<Utc> {
        raw.parse::<DateTime<Utc>>()
            .expect("valid RFC 3339 timestamp")
    }

    #[test]
    fn event_keeps_last_seen_and_source_timestamp() {
        let mut tracker = PresenceTracker::default();
        let mut online = row("s1", true);
        online.last_seen_at = Some(ts("2026-10-31T00:00:00Z"));
        online.source_updated_at = ts("2026-10-31T00:00:01Z");
        tracker.observe(&[online], false);

        let mut offline = row("s1", false);
        offline.last_seen_at = Some(ts("2026-10-31T00:00:02Z"));
        offline.source_updated_at = ts("2026-10-31T00:00:03Z");
        let events = tracker.observe(&[offline], false);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].last_seen_at, Some(ts("2026-10-31T00:00:02Z")));
        assert_eq!(events[0].source_updated_at, ts("2026-10-31T00:00:03Z"));
    }

    #[test]
    fn webhook_target_falls_back_room_then_area_then_global() {
        assert_eq!(
            resolve_webhook_url(
                Some("http://room"),
                Some("http://area"),
                Some("http://global")
            ),
            Some("http://room")
        );
        assert_eq!(
            resolve_webhook_url(None, Some("http://area"), Some("http://global")),
            Some("http://area")
        );
        assert_eq!(
            resolve_webhook_url(None, None, Some("http://global")),
            Some("http://global")
        );
        assert_eq!(resolve_webhook_url(None, None, None), None);
    }

    #[test]
    fn webhook_target_ignores_blank_urls() {
        assert_eq!(
            resolve_webhook_url(Some("   "), Some(""), None),
            None,
            "空白地址不应被当成投递目标"
        );
        assert_eq!(
            resolve_webhook_url(Some(""), None, Some("http://global")),
            Some("http://global")
        );
    }

    #[test]
    fn retry_backoff_is_exponential_and_capped() {
        assert_eq!(retry_delay(5, 1), ChronoDuration::seconds(5));
        assert_eq!(retry_delay(5, 2), ChronoDuration::seconds(10));
        assert_eq!(retry_delay(5, 3), ChronoDuration::seconds(20));
        assert_eq!(retry_delay(5, 4), ChronoDuration::seconds(40));
        // 5 * 2^5 = 160，7 次时为 320，超过 300 秒上限。
        assert_eq!(retry_delay(5, 7), ChronoDuration::seconds(300));
        assert_eq!(retry_delay(60, 4), ChronoDuration::seconds(300));
    }

    #[test]
    fn webhook_payload_omits_internal_routing_fields() {
        let mut tracker = PresenceTracker::default();
        tracker.observe(&[row("s1", true)], false);
        let events = tracker.observe(&[row("s1", false)], false);
        let value = serde_json::to_value(&events[0]).expect("serialize event");

        assert_eq!(value["event"], EVENT_OFFLINE);
        assert!(value.get("webhook_url").is_none());
        assert!(value.get("area_webhook_url").is_none());
        assert!(value.get("source_updated_at").is_none());
    }
}
