use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::Page,
        rbac::permissions,
        schedule::{
            NewExceptionSlot, NewSchedule, NewWeeklySlot, Occurrence, OccurrenceSource, Schedule,
            ScheduleException, ScheduleExceptionKind, UpdateSchedule, WeeklySlot, local_date_of,
            local_rfc3339, local_to_utc,
        },
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ScheduleQuery {
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateScheduleRequest {
    pub room_id: Uuid,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateScheduleRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ReplaceWeeklySlotsRequest {
    #[serde(default)]
    pub slots: Vec<NewWeeklySlot>,
}

#[derive(Debug, Deserialize)]
pub struct ExceptionQuery {
    #[serde(default)]
    pub from: Option<NaiveDate>,
    #[serde(default)]
    pub to: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertExceptionRequest {
    pub kind: ScheduleExceptionKind,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub slots: Vec<NewExceptionSlot>,
}

#[derive(Debug, Deserialize)]
pub struct OccurrenceQuery {
    #[serde(default)]
    pub from: Option<NaiveDate>,
    #[serde(default)]
    pub to: Option<NaiveDate>,
}

/// 排课实例的对外视图：UTC 时间点 + 按课表所属区域时区格式化的本地时间。
#[derive(Debug, Serialize)]
pub struct OccurrenceView {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub room_id: Uuid,
    pub occurrence_date: NaiveDate,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub timezone: String,
    pub start_local: String,
    pub end_local: String,
    pub course_name: String,
    pub teacher: String,
    pub source: OccurrenceSource,
    pub source_slot_id: Uuid,
    pub note: String,
}

impl From<Occurrence> for OccurrenceView {
    fn from(occurrence: Occurrence) -> Self {
        Self {
            start_local: local_rfc3339(occurrence.start_at, &occurrence.timezone),
            end_local: local_rfc3339(occurrence.end_at, &occurrence.timezone),
            timezone: occurrence.timezone.clone(),
            id: occurrence.id,
            schedule_id: occurrence.schedule_id,
            room_id: occurrence.room_id,
            occurrence_date: occurrence.occurrence_date,
            start_at: occurrence.start_at,
            end_at: occurrence.end_at,
            course_name: occurrence.course_name,
            teacher: occurrence.teacher,
            source: occurrence.source,
            source_slot_id: occurrence.source_slot_id,
            note: occurrence.note,
        }
    }
}

async fn ensure_schedule(state: &AppState, schedule_id: Uuid) -> AppResult<Schedule> {
    state
        .schedules
        .get(schedule_id)
        .await?
        .ok_or_else(|| AppError::not_found("课表不存在"))
}

/// 写操作成功后重建未来排课实例。
///
/// 重建在事务外进行：即使重建失败，写操作本身已经提交，返回 500 并记日志，
/// 之后可由 `POST /schedules/{id}/rebuild` 或后台任务补齐。
async fn rebuild_future_occurrences(state: &AppState, schedule_id: Uuid) -> AppResult<usize> {
    match state
        .schedules
        .rebuild_occurrences(schedule_id, state.config.schedule_horizon_days)
        .await
    {
        Ok(count) => Ok(count),
        Err(error) => {
            tracing::error!(%error, %schedule_id, "重建排课实例失败，数据可稍后手动重建");
            Err(AppError::internal("重建排课实例失败，请稍后重试或手动重建"))
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<ScheduleQuery>,
) -> AppResult<Json<Page<Schedule>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    Ok(Json(
        state.schedules.list(query.room_id, page, page_size).await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(schedule_id): Path<Uuid>,
) -> AppResult<Json<Schedule>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    Ok(Json(ensure_schedule(&state, schedule_id).await?))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateScheduleRequest>,
) -> AppResult<(StatusCode, Json<Schedule>)> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    if state.rooms.get(request.room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }

    validation::validate_date_range(request.start_date, request.end_date)?;
    let input = NewSchedule {
        room_id: request.room_id,
        name: validation::normalize_schedule_name(&request.name)?,
        start_date: request.start_date,
        end_date: request.end_date,
        is_active: request.is_active.unwrap_or(true),
        created_by: Some(id),
    };
    let schedule = state.schedules.create(&input).await?;

    super::audit::record_or_log(
        &state,
        id,
        "schedule.create",
        "schedule",
        Some(schedule.id.to_string()),
        serde_json::json!({
            "room_id": schedule.room_id,
            "name": schedule.name,
            "start_date": schedule.start_date,
            "end_date": schedule.end_date,
            "timezone": schedule.timezone,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(schedule)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(schedule_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateScheduleRequest>,
) -> AppResult<Json<Schedule>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    if request.name.is_none()
        && request.start_date.is_none()
        && request.end_date.is_none()
        && request.is_active.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }

    // 只改单个日期时，需结合库里的另一端日期校验区间。
    let existing = ensure_schedule(&state, schedule_id).await?;
    validation::validate_date_range(
        request.start_date.unwrap_or(existing.start_date),
        request.end_date.unwrap_or(existing.end_date),
    )?;

    let input = UpdateSchedule {
        name: request
            .name
            .as_deref()
            .map(validation::normalize_schedule_name)
            .transpose()?,
        start_date: request.start_date,
        end_date: request.end_date,
        is_active: request.is_active,
    };
    let schedule = state
        .schedules
        .update(schedule_id, &input)
        .await?
        .ok_or_else(|| AppError::not_found("课表不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "schedule.update",
        "schedule",
        Some(schedule.id.to_string()),
        serde_json::json!({
            "name": schedule.name,
            "start_date": schedule.start_date,
            "end_date": schedule.end_date,
            "is_active": schedule.is_active,
            "timezone": schedule.timezone,
        }),
        ip.as_deref(),
    )
    .await;

    rebuild_future_occurrences(&state, schedule_id).await?;

    Ok(Json(schedule))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(schedule_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    if !state.schedules.delete(schedule_id).await? {
        return Err(AppError::not_found("课表不存在"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "schedule.delete",
        "schedule",
        Some(schedule_id.to_string()),
        serde_json::json!({}),
        ip.as_deref(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn weekly_slots(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(schedule_id): Path<Uuid>,
) -> AppResult<Json<Vec<WeeklySlot>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;

    ensure_schedule(&state, schedule_id).await?;
    Ok(Json(state.schedules.weekly_slots(schedule_id).await?))
}

pub async fn replace_weekly_slots(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(schedule_id): Path<Uuid>,
    ApiJson(request): ApiJson<ReplaceWeeklySlotsRequest>,
) -> AppResult<Json<Vec<WeeklySlot>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    ensure_schedule(&state, schedule_id).await?;
    let validated = validation::validate_weekly_slots(&request.slots)?;
    let slots = state
        .schedules
        .replace_weekly_slots(schedule_id, &validated)
        .await?
        .ok_or_else(|| AppError::not_found("课表不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "schedule.set_weekly_slots",
        "schedule",
        Some(schedule_id.to_string()),
        serde_json::json!({ "count": slots.len() }),
        ip.as_deref(),
    )
    .await;

    rebuild_future_occurrences(&state, schedule_id).await?;

    Ok(Json(slots))
}

pub async fn exceptions(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(schedule_id): Path<Uuid>,
    Query(query): Query<ExceptionQuery>,
) -> AppResult<Json<Vec<ScheduleException>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;

    ensure_schedule(&state, schedule_id).await?;
    Ok(Json(
        state
            .schedules
            .exceptions(schedule_id, query.from, query.to)
            .await?,
    ))
}

pub async fn upsert_exception(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path((schedule_id, exception_date)): Path<(Uuid, NaiveDate)>,
    ApiJson(request): ApiJson<UpsertExceptionRequest>,
) -> AppResult<Json<ScheduleException>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    ensure_schedule(&state, schedule_id).await?;
    let note = validation::normalize_note(&request.note)?;
    let slots = validation::validate_exception_slots(&request.slots)?;
    let exception = state
        .schedules
        .upsert_exception(schedule_id, exception_date, request.kind, &note, &slots)
        .await?
        .ok_or_else(|| AppError::not_found("课表不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "schedule.set_exception",
        "schedule",
        Some(schedule_id.to_string()),
        serde_json::json!({
            "date": exception_date,
            "kind": request.kind,
            "slots": exception.slots.len(),
        }),
        ip.as_deref(),
    )
    .await;

    rebuild_future_occurrences(&state, schedule_id).await?;

    Ok(Json(exception))
}

pub async fn delete_exception(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path((schedule_id, exception_date)): Path<(Uuid, NaiveDate)>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    if !state
        .schedules
        .delete_exception(schedule_id, exception_date)
        .await?
    {
        return Err(AppError::not_found("单日覆盖不存在"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "schedule.delete_exception",
        "schedule",
        Some(schedule_id.to_string()),
        serde_json::json!({ "date": exception_date }),
        ip.as_deref(),
    )
    .await;

    rebuild_future_occurrences(&state, schedule_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn occurrences(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(schedule_id): Path<Uuid>,
    Query(query): Query<OccurrenceQuery>,
) -> AppResult<Json<Vec<OccurrenceView>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;

    let schedule = ensure_schedule(&state, schedule_id).await?;
    let (from, to) = validation::validate_occurrence_range(
        query.from,
        query.to,
        local_date_of(Utc::now(), &schedule.timezone)?,
    )?;
    let (from_at, to_at) = occurrence_bounds(from, to, &schedule.timezone)?;

    Ok(Json(
        state
            .schedules
            .occurrences(schedule_id, from_at, to_at)
            .await?
            .into_iter()
            .map(OccurrenceView::from)
            .collect(),
    ))
}

/// 房间维度的排课实例（`GET /rooms/{id}/occurrences`，注册在 `rooms` 路径下）。
pub async fn room_occurrences(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(room_id): Path<Uuid>,
    Query(query): Query<OccurrenceQuery>,
) -> AppResult<Json<Vec<OccurrenceView>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;

    let room = state
        .rooms
        .get(room_id)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;
    let (from, to) = validation::validate_occurrence_range(
        query.from,
        query.to,
        local_date_of(Utc::now(), &room.timezone)?,
    )?;
    let (from_at, to_at) = occurrence_bounds(from, to, &room.timezone)?;

    Ok(Json(
        state
            .schedules
            .room_occurrences(room_id, from_at, to_at)
            .await?
            .into_iter()
            .map(OccurrenceView::from)
            .collect(),
    ))
}

pub async fn rebuild(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(schedule_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    ensure_schedule(&state, schedule_id).await?;
    let occurrences = rebuild_future_occurrences(&state, schedule_id).await?;
    Ok(Json(serde_json::json!({ "occurrences": occurrences })))
}

fn midnight() -> NaiveTime {
    NaiveTime::from_hms_opt(0, 0, 0).expect("00:00:00 是合法时间")
}

/// 本地日期的 00:00 转 UTC；`to` 取次日 00:00 作为开区间上界。
fn occurrence_bounds(
    from: NaiveDate,
    to: NaiveDate,
    timezone: &str,
) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    let end_date = to.succ_opt().unwrap_or(to);
    Ok((
        local_to_utc(from, midnight(), timezone)?,
        local_to_utc(end_date, midnight(), timezone)?,
    ))
}
