use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::Page,
        rbac::permissions,
        schedule::{
            EventFilter, EventRule, NewEventRule, ScheduleEvent, TriggerKind, local_rfc3339,
        },
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ReplaceEventRulesRequest {
    #[serde(default)]
    pub rules: Vec<NewEventRule>,
}

#[derive(Debug, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub to: Option<DateTime<Utc>>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

/// 事件对外视图：在事件字段之上补充按事件时区快照格式化的本地触发时间。
#[derive(Debug, Serialize)]
pub struct ScheduleEventView {
    #[serde(flatten)]
    event: ScheduleEvent,
    triggered_at_local: String,
}

impl From<ScheduleEvent> for ScheduleEventView {
    fn from(event: ScheduleEvent) -> Self {
        let triggered_at_local = local_rfc3339(event.triggered_at, &event.timezone);
        Self {
            event,
            triggered_at_local,
        }
    }
}

async fn ensure_room(state: &AppState, room_id: Uuid) -> AppResult<()> {
    if state.rooms.get(room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }
    Ok(())
}

/// `GET /rooms/{id}/event-rules`：触发规则属于房间。
pub async fn list_event_rules(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(room_id): Path<Uuid>,
) -> AppResult<Json<Vec<EventRule>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;

    ensure_room(&state, room_id).await?;
    Ok(Json(state.event_rules.list_by_room(room_id).await?))
}

/// `PUT /rooms/{id}/event-rules`：全量替换该房间的触发规则。
pub async fn replace_event_rules(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<Uuid>,
    ApiJson(request): ApiJson<ReplaceEventRulesRequest>,
) -> AppResult<Json<Vec<EventRule>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    ensure_room(&state, room_id).await?;
    let rules = validation::validate_event_rules(&request.rules)?;
    let stored = state
        .event_rules
        .replace_for_room(room_id, &rules)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "room.set_event_rules",
        "room",
        Some(room_id.to_string()),
        json!({ "count": stored.len() }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(stored))
}

pub async fn list_events(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<EventQuery>,
) -> AppResult<Json<Page<ScheduleEventView>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_EVENT_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let kind = query
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|raw| {
            raw.parse::<TriggerKind>()
                .map(|kind| kind.as_str().to_owned())
                .map_err(|_| {
                    AppError::bad_request(
                        "无效的事件类型，可选值为 occurrence_start / occurrence_end / \
                         occurrence_start_before / daily_time / room_occupied / room_idle",
                    )
                })
        })
        .transpose()?;
    let filter = EventFilter {
        room_id: query.room_id,
        kind,
        from: query.from,
        to: query.to,
    };

    let page = state.schedule_events.list(&filter, page, page_size).await?;
    let items = page
        .items
        .into_iter()
        .map(ScheduleEventView::from)
        .collect();
    Ok(Json(Page::new(
        items,
        page.total,
        page.page,
        page.page_size,
    )))
}

/// `GET /schedule-events/{id}`：事件 + 投递明细。
pub async fn get_event(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(event_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_EVENT_READ).await?;

    let event = state
        .schedule_events
        .get(event_id)
        .await?
        .ok_or_else(|| AppError::not_found("事件不存在"))?;
    let deliveries = state.schedule_events.deliveries_for(event_id).await?;

    Ok(Json(json!({
        "event": ScheduleEventView::from(event),
        "deliveries": deliveries,
    })))
}

/// `POST /schedule-events/{id}/replay`：把该事件全部投递重置为 pending。
pub async fn replay_event(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(event_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_EVENT_MANAGE).await?;

    if state.schedule_events.get(event_id).await?.is_none() {
        return Err(AppError::not_found("事件不存在"));
    }
    let reset = state.schedule_events.replay_event(event_id).await?;

    super::audit::record_or_log(
        &state,
        id,
        "schedule_event.replay",
        "schedule_event",
        Some(event_id.to_string()),
        json!({ "reset": reset }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(json!({ "reset": reset })))
}

/// `POST /schedule-events/{id}/deliveries/{delivery_id}/retry`：单条投递立即重试。
pub async fn retry_delivery(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path((event_id, delivery_id)): Path<(Uuid, i64)>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_EVENT_MANAGE).await?;

    if !state
        .schedule_events
        .retry_delivery(event_id, delivery_id)
        .await?
    {
        return Err(AppError::not_found("投递记录不存在"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "schedule_event.retry",
        "schedule_event",
        Some(event_id.to_string()),
        json!({ "delivery_id": delivery_id }),
        ip.as_deref(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
