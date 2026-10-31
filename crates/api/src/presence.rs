//! 设备在线状态变化通知（`device.offline` / `device.online` webhook）。
//!
//! device-pki 负责心跳判定并把状态写进 `device_session`；本模块在 qe-api 侧周期
//! 扫描该表，检测 `online -> offline`（可选 `offline -> online`）翻转，POST 到房间
//! 配置的 `rooms.webhook_url`。这样管理端不必一直轮询，也能收到节点下线通知。
//!
//! 限制：当前是**单实例**实现（多副本部署会重复投递）；没有持久化重试队列，
//! 投递失败只记录日志并在下一轮不再重试（事件不是状态，状态仍以 `device_session` 为准）。

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

use crate::state::AppState;

const EVENT_OFFLINE: &str = "device.offline";
const EVENT_ONLINE: &str = "device.online";

/// 一行设备在线状态（含房间/区域与 webhook 配置）。
#[derive(Debug, Clone)]
pub struct PresenceRow {
    pub serial: String,
    pub device_slot: String,
    pub online: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    pub webhook_url: Option<String>,
    pub area_id: Uuid,
    pub area_code: String,
    pub area_name: String,
}

/// 对外投递的状态变化事件。
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
        }
    }
}

fn state_name(online: bool) -> String {
    if online { "online" } else { "offline" }.to_owned()
}

/// 启动后台在线状态通知循环。
pub fn spawn(state: AppState) -> JoinHandle<()> {
    let tick_secs = state.config.device_presence_tick_secs.max(1);
    let timeout_secs = state.config.device_presence_webhook_timeout_secs.max(1);
    let notify_online = state.config.device_presence_notify_online;

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
            let rows = match load_rows(state.db.pool()).await {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%error, "扫描设备在线状态失败");
                    continue;
                }
            };
            for event in tracker.observe(&rows, notify_online) {
                tracing::info!(
                    event = %event.event,
                    serial = %event.serial,
                    device_slot = %event.device_slot,
                    room_id = %event.room_id,
                    "设备在线状态变化"
                );
                if let Some(url) = event.webhook_url.as_deref() {
                    deliver(&client, url, &event).await;
                }
            }
        }
    })
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
);

async fn load_rows(pool: &PgPool) -> Result<Vec<PresenceRow>, sqlx::Error> {
    let rows: Vec<RawPresenceRow> = sqlx::query_as(
        "SELECT ds.serial, ds.device_slot, ds.state, ds.updated_at,
                r.id, r.code, r.name, r.webhook_url,
                a.id, a.code, a.name
           FROM device_session ds
           JOIN rooms r ON r.device_slot = ds.device_slot
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
            room_id: row.4,
            room_code: row.5,
            room_name: row.6,
            webhook_url: row.7,
            area_id: row.8,
            area_code: row.9,
            area_name: row.10,
        })
        .collect())
}

async fn deliver(client: &reqwest::Client, url: &str, event: &PresenceEvent) {
    let event_id = Uuid::new_v4();
    let response = client
        .post(url)
        .header("X-QE-Event", &event.event)
        .header("X-QE-Event-Id", event_id.to_string())
        .header("X-QE-Delivery-Id", event_id.to_string())
        .json(event)
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => {
            tracing::info!(
                event = %event.event,
                serial = %event.serial,
                room_id = %event.room_id,
                status = %response.status(),
                "设备在线状态 webhook 投递成功"
            );
        }
        Ok(response) => {
            tracing::warn!(
                event = %event.event,
                serial = %event.serial,
                status = %response.status(),
                "设备在线状态 webhook 返回非 2xx"
            );
        }
        Err(error) => {
            tracing::warn!(
                event = %event.event,
                serial = %event.serial,
                error = %error,
                "设备在线状态 webhook 投递失败"
            );
        }
    }
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
            room_id: Uuid::nil(),
            room_code: "r1".into(),
            room_name: "房间 1".into(),
            webhook_url: Some("http://127.0.0.1:1/hook".into()),
            area_id: Uuid::nil(),
            area_code: "a1".into(),
            area_name: "区域 1".into(),
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
}
