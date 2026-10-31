use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::Page,
        rbac::permissions,
        schedule::{Area, DEFAULT_TIMEZONE, NewArea, UpdateArea},
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct AreaQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub include_inactive: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAreaRequest {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
    /// IANA 时区名；缺省 `Asia/Shanghai`。
    #[serde(default)]
    pub timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAreaRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// 缺失 = 不修改；`null` = 清空上级区域。
    #[serde(default)]
    pub parent_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub is_active: Option<bool>,
    /// 缺失 = 不修改；`null` = 重置为 `{}`。
    #[serde(default)]
    pub metadata: Option<Option<Value>>,
    /// 缺失 = 不修改；变更成功后自动重建该区域未来的排课实例。
    #[serde(default)]
    pub timezone: Option<String>,
}

/// 区域/房间名称没有独立的领域校验函数，这里做与其它名称字段一致的长度约束。
pub(super) fn normalize_name(raw: &str, label: &str) -> AppResult<String> {
    let name = raw.trim().to_owned();
    let length = name.chars().count();
    if !(1..=200).contains(&length) {
        return Err(AppError::bad_request(format!(
            "{label}长度需为 1-200 个字符"
        )));
    }
    Ok(name)
}

fn normalize_metadata(metadata: Option<Value>) -> AppResult<Value> {
    match metadata {
        Some(value) => Ok(validation::validate_metadata(value)?),
        None => Ok(serde_json::json!({})),
    }
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<AreaQuery>,
) -> AppResult<Json<Page<Area>>> {
    authorization::require_permission(&state, id, permissions::AREA_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    let include_inactive = query.include_inactive.unwrap_or(false);

    Ok(Json(
        state
            .areas
            .list(&keyword, include_inactive, page, page_size)
            .await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(area_id): Path<Uuid>,
) -> AppResult<Json<Area>> {
    authorization::require_permission(&state, id, permissions::AREA_READ).await?;

    let area = state
        .areas
        .get(area_id)
        .await?
        .ok_or_else(|| AppError::not_found("区域不存在"))?;
    Ok(Json(area))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateAreaRequest>,
) -> AppResult<(StatusCode, Json<Area>)> {
    authorization::require_permission(&state, id, permissions::AREA_MANAGE).await?;

    let timezone = match request.timezone.as_deref() {
        Some(raw) => validation::validate_timezone(raw)?,
        None => DEFAULT_TIMEZONE.to_owned(),
    };
    let input = NewArea {
        code: validation::normalize_code(&request.code, "区域编码")?,
        name: normalize_name(&request.name, "区域名称")?,
        description: validation::normalize_description(&request.description)?,
        parent_id: request.parent_id,
        is_active: request.is_active.unwrap_or(true),
        metadata: normalize_metadata(request.metadata)?,
        timezone,
    };
    let area = state.areas.create(&input).await?;

    super::audit::record_or_log(
        &state,
        id,
        "area.create",
        "area",
        Some(area.id.to_string()),
        serde_json::json!({
            "code": area.code,
            "name": area.name,
            "parent_id": area.parent_id,
            "timezone": area.timezone,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(area)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(area_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateAreaRequest>,
) -> AppResult<Json<Area>> {
    authorization::require_permission(&state, id, permissions::AREA_MANAGE).await?;

    if request.code.is_none()
        && request.name.is_none()
        && request.description.is_none()
        && request.parent_id.is_none()
        && request.is_active.is_none()
        && request.metadata.is_none()
        && request.timezone.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }

    // 时区变更后需要按新时区重算该区域下所有课表的未来实例。
    let timezone_changed = request.timezone.is_some();

    let input = UpdateArea {
        code: request
            .code
            .as_deref()
            .map(|code| validation::normalize_code(code, "区域编码"))
            .transpose()?,
        name: request
            .name
            .as_deref()
            .map(|name| normalize_name(name, "区域名称"))
            .transpose()?,
        description: request
            .description
            .as_deref()
            .map(validation::normalize_description)
            .transpose()?,
        parent_id: request.parent_id,
        is_active: request.is_active,
        metadata: match request.metadata {
            None => None,
            Some(None) => Some(serde_json::json!({})),
            Some(Some(value)) => Some(validation::validate_metadata(value)?),
        },
        timezone: request
            .timezone
            .as_deref()
            .map(validation::validate_timezone)
            .transpose()?,
    };
    let area = state
        .areas
        .update(area_id, &input)
        .await?
        .ok_or_else(|| AppError::not_found("区域不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "area.update",
        "area",
        Some(area.id.to_string()),
        serde_json::json!({
            "code": area.code,
            "name": area.name,
            "is_active": area.is_active,
            "parent_id": area.parent_id,
            "timezone": area.timezone,
        }),
        ip.as_deref(),
    )
    .await;

    if timezone_changed {
        rebuild_area_occurrences(&state, area.id).await?;
    }

    Ok(Json(area))
}

/// 区域时区变更后按新时区重建该区域下所有课表的未来排课实例。
///
/// 重建在写事务之外执行：写操作已提交，重建失败返回 500 并记录日志，可稍后重试。
async fn rebuild_area_occurrences(state: &AppState, area_id: Uuid) -> AppResult<usize> {
    match state
        .schedules
        .rebuild_area_occurrences(area_id, state.config.schedule_horizon_days)
        .await
    {
        Ok(count) => Ok(count),
        Err(error) => {
            tracing::error!(%error, %area_id, "区域时区变更后重建排课实例失败");
            Err(AppError::internal("重建排课实例失败，请稍后重试"))
        }
    }
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(area_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::AREA_MANAGE).await?;

    // 区域下仍有房间时仓储层返回 Conflict，统一映射为 409。
    if !state.areas.delete(area_id).await? {
        return Err(AppError::not_found("区域不存在"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "area.delete",
        "area",
        Some(area_id.to_string()),
        serde_json::json!({}),
        ip.as_deref(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
