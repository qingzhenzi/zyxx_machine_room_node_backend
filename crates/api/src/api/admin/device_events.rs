//! 设备在线事件查询与 SSE 实时流（P3）。
//!
//! 契约见 `docs/storage-design.md` §18.5：
//! - `GET /device-events`：最近事件（id 倒序，`limit` 1–200，默认 50）；
//! - `GET /device-events/stream`：SSE，先补发 `after_id` 之后的历史事件，再每 2 秒查库增量；
//! - `GET /device-presence-deliveries`：投递记录（按状态过滤，排障用）。
//!
//! SSE 采用**查库轮询**而不是依赖本地广播：多副本部署时每个副本都能看到其它副本写入
//! 的事件。`AppState::presence_events` 的本地广播可供其它进程内订阅者做低延迟优化，
//! 但不作为 SSE 的唯一数据源。

use std::{collections::VecDeque, convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use qe_db::presence::{PresenceDeliveryRecord, PresenceEventRecord};

use crate::{
    api::extractors::AuthUser,
    auth::authorization,
    domain::rbac::permissions,
    error::{AppError, AppResult},
    state::AppState,
};

/// 事件列表默认条数。
const EVENTS_DEFAULT_LIMIT: i64 = 50;
/// 投递列表默认条数。
const DELIVERIES_DEFAULT_LIMIT: i64 = 100;
/// 列表接口上限（契约 §18.5：1–200）。
const LIST_MAX_LIMIT: i64 = 200;
/// SSE 首次补发条数。
const STREAM_BACKLOG_LIMIT: i64 = 200;
/// SSE 每轮增量查询条数。
const STREAM_POLL_LIMIT: i64 = 100;
/// SSE 增量查询间隔。
const STREAM_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// SSE 心跳间隔。
const STREAM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// 投递状态过滤值（与 `device_presence_deliveries.status` 的 CHECK 一致）。
const DELIVERY_STATUSES: [&str; 3] = ["pending", "delivered", "failed"];

#[derive(Debug, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    /// 只返回 `id > after_id` 的事件（SSE 断线续传 / 增量拉取）。
    #[serde(default)]
    pub after_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub after_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct DeliveryQuery {
    /// `pending` / `delivered` / `failed`；缺省不过滤。
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct EventListResponse {
    pub items: Vec<PresenceEventRecord>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct DeliveryListResponse {
    pub items: Vec<PresenceDeliveryRecord>,
    pub total: usize,
}

fn parse_limit(raw: Option<i64>, default: i64) -> AppResult<i64> {
    let limit = raw.unwrap_or(default);
    if !(1..=LIST_MAX_LIMIT).contains(&limit) {
        return Err(AppError::bad_request(format!(
            "limit 需在 1-{LIST_MAX_LIMIT} 之间"
        )));
    }
    Ok(limit)
}

fn parse_after_id(raw: Option<i64>) -> AppResult<Option<i64>> {
    if let Some(after_id) = raw
        && after_id < 0
    {
        return Err(AppError::bad_request("after_id 不能为负数"));
    }
    Ok(raw)
}

fn parse_delivery_status(raw: Option<&str>) -> AppResult<Option<&str>> {
    match raw {
        None => Ok(None),
        Some(status) if DELIVERY_STATUSES.contains(&status) => Ok(Some(status)),
        Some(status) => Err(AppError::bad_request(format!(
            "status 只能是 pending / delivered / failed，收到 {status}"
        ))),
    }
}

/// 最近事件（id 倒序）。`total` 为本次返回条数，便于前端判断是否还有更早的事件。
pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<EventQuery>,
) -> AppResult<Json<EventListResponse>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let limit = parse_limit(query.limit, EVENTS_DEFAULT_LIMIT)?;
    let after_id = parse_after_id(query.after_id)?;
    let items = state.presence.list_events(limit, after_id).await?;
    let total = items.len();
    Ok(Json(EventListResponse { items, total }))
}

/// 投递记录（id 倒序），支持按 `status` 过滤。
pub async fn list_deliveries(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<DeliveryQuery>,
) -> AppResult<Json<DeliveryListResponse>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let limit = parse_limit(query.limit, DELIVERIES_DEFAULT_LIMIT)?;
    let status = parse_delivery_status(query.status.as_deref())?;
    let items = state.presence.list_deliveries(limit, status).await?;
    let total = items.len();
    Ok(Json(DeliveryListResponse { items, total }))
}

/// SSE 实时流：先补发历史，再每 2 秒查库增量，每 15 秒心跳。
///
/// 事件格式：
/// ```text
/// id: 42
/// event: device.presence
/// data: {"id":42,"serial":"…","current_state":"offline",…}
///
/// event: heartbeat
/// data: {}
/// ```
///
/// `Sse` 自身不缓冲；外层 compression 默认排除 `text/event-stream`，timeout 只作用于
/// 响应头阶段，因此事件会按写入顺序立即下发。客户端断开后 stream 被 drop，循环自然结束。
pub async fn stream(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<StreamQuery>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let after_id = parse_after_id(query.after_id)?;
    let initial = StreamState {
        state,
        after_id,
        last_id: after_id,
        pending: VecDeque::new(),
        backlog_loaded: false,
        last_heartbeat: Instant::now(),
    };

    // `unfold` 的状态机每轮只产出一个 SSE 帧：
    // 1. 有待发事件时逐条 yield（不 sleep，保证历史补发不额外延迟）；
    // 2. 首次进入时拉取 `after_id` 之后最近 200 条历史，按 id 正序补发；
    // 3. 之后每 2 秒拉取一次增量，攒够心跳间隔时先发心跳。
    let stream = stream::unfold(initial, |mut state| async move {
        loop {
            if let Some(record) = state.pending.pop_front() {
                state.last_id = Some(record.id);
                let event = presence_frame(record);
                return Some((Ok(event), state));
            }

            if !state.backlog_loaded {
                state.backlog_loaded = true;
                match state
                    .state
                    .presence
                    .list_events(STREAM_BACKLOG_LIMIT, state.after_id)
                    .await
                {
                    Ok(events) => {
                        state.enqueue(events);
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "SSE 补发设备在线事件失败，稍后重试");
                    }
                }
            }

            tokio::time::sleep(STREAM_POLL_INTERVAL).await;

            if state.last_heartbeat.elapsed() >= STREAM_HEARTBEAT_INTERVAL {
                state.last_heartbeat = Instant::now();
                return Some((Ok(Event::default().event("heartbeat").data("{}")), state));
            }

            match state
                .state
                .presence
                .list_events(STREAM_POLL_LIMIT, state.last_id)
                .await
            {
                Ok(events) => state.enqueue(events),
                Err(error) => {
                    tracing::warn!(%error, "SSE 增量查询设备在线事件失败");
                }
            }
        }
    });

    Ok(Sse::new(stream))
}

/// SSE 流状态机：`pending` 始终按 id 正序排队。
struct StreamState {
    state: AppState,
    /// 客户端指定的起点；首次补发后不再变化。
    after_id: Option<i64>,
    /// 已发送（或已入队）的最大事件 id，用于下一次增量查询。
    last_id: Option<i64>,
    pending: VecDeque<PresenceEventRecord>,
    backlog_loaded: bool,
    last_heartbeat: Instant,
}

impl StreamState {
    /// `list_events` 按 id 倒序返回；SSE 要求按 id 正序补发，这里统一反转。
    fn enqueue(&mut self, mut events: Vec<PresenceEventRecord>) {
        events.sort_by_key(|event| event.id);
        self.pending.extend(events);
    }
}

fn presence_frame(record: PresenceEventRecord) -> Event {
    let id = record.id.to_string();
    let data = serde_json::to_string(&record).unwrap_or_else(|error| {
        tracing::error!(%error, event_id = record.id, "序列化设备在线事件失败");
        "{}".to_owned()
    });
    Event::default().id(id).event("device.presence").data(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_status_accepts_contract_values_only() {
        assert_eq!(parse_delivery_status(None).expect("none"), None);
        assert_eq!(
            parse_delivery_status(Some("pending")).expect("pending"),
            Some("pending")
        );
        assert_eq!(
            parse_delivery_status(Some("delivered")).expect("delivered"),
            Some("delivered")
        );
        assert_eq!(
            parse_delivery_status(Some("failed")).expect("failed"),
            Some("failed")
        );
        assert!(parse_delivery_status(Some("skipped")).is_err());
        assert!(parse_delivery_status(Some("")).is_err());
    }

    #[test]
    fn limit_and_after_id_are_bounded() {
        assert_eq!(parse_limit(None, 50).expect("default"), 50);
        assert_eq!(parse_limit(Some(1), 50).expect("min"), 1);
        assert_eq!(parse_limit(Some(200), 50).expect("max"), 200);
        assert!(parse_limit(Some(0), 50).is_err());
        assert!(parse_limit(Some(201), 50).is_err());
        assert_eq!(parse_after_id(None).expect("none"), None);
        assert_eq!(parse_after_id(Some(0)).expect("zero"), Some(0));
        assert!(parse_after_id(Some(-1)).is_err());
    }
}
