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
        schedule::{NewRoom, Room, UpdateRoom},
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

use super::areas::normalize_name;

#[derive(Debug, Deserialize)]
pub struct RoomQuery {
    #[serde(default)]
    pub area_id: Option<Uuid>,
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
pub struct CreateRoomRequest {
    pub area_id: Uuid,
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub capacity: Option<i32>,
    #[serde(default)]
    pub device_slot: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoomRequest {
    #[serde(default)]
    pub area_id: Option<Uuid>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub capacity: Option<i32>,
    /// 缺失 = 不修改；`null` = 清空（解除设备槽位绑定）。
    #[serde(default)]
    pub device_slot: Option<Option<String>>,
    /// 缺失 = 不修改；`null` = 清空。
    #[serde(default)]
    pub webhook_url: Option<Option<String>>,
    #[serde(default)]
    pub is_active: Option<bool>,
    /// 缺失 = 不修改；`null` = 重置为 `{}`。
    #[serde(default)]
    pub metadata: Option<Option<Value>>,
}

/// `device_slot` 对应 `device_enrollment.device_slot`，格式由设备侧决定，
/// 这里只去空白并把空字符串归一为 `None`。
fn normalize_device_slot(raw: &str) -> AppResult<Option<String>> {
    let slot = raw.trim();
    if slot.is_empty() {
        return Ok(None);
    }
    if slot.chars().count() > 128 {
        return Err(AppError::bad_request("设备槽位标识不能超过 128 个字符"));
    }
    Ok(Some(slot.to_owned()))
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<RoomQuery>,
) -> AppResult<Json<Page<Room>>> {
    authorization::require_permission(&state, id, permissions::ROOM_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    let include_inactive = query.include_inactive.unwrap_or(false);

    Ok(Json(
        state
            .rooms
            .list(query.area_id, &keyword, include_inactive, page, page_size)
            .await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(room_id): Path<Uuid>,
) -> AppResult<Json<Room>> {
    authorization::require_permission(&state, id, permissions::ROOM_READ).await?;

    let room = state
        .rooms
        .get(room_id)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;
    Ok(Json(room))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateRoomRequest>,
) -> AppResult<(StatusCode, Json<Room>)> {
    authorization::require_permission(&state, id, permissions::ROOM_MANAGE).await?;

    if state.areas.get(request.area_id).await?.is_none() {
        return Err(AppError::not_found("区域不存在"));
    }

    let input = NewRoom {
        area_id: request.area_id,
        code: validation::normalize_code(&request.code, "房间编码")?,
        name: normalize_name(&request.name, "房间名称")?,
        capacity: validation::validate_capacity(request.capacity.unwrap_or(0))?,
        device_slot: match request.device_slot.as_deref() {
            Some(raw) => normalize_device_slot(raw)?,
            None => None,
        },
        webhook_url: request
            .webhook_url
            .as_deref()
            .map(validation::normalize_webhook_url)
            .transpose()?
            .flatten(),
        is_active: request.is_active.unwrap_or(true),
        metadata: match request.metadata {
            Some(value) => validation::validate_metadata(value)?,
            None => serde_json::json!({}),
        },
    };
    let room = state.rooms.create(&input).await?;

    super::audit::record_or_log(
        &state,
        id,
        "room.create",
        "room",
        Some(room.id.to_string()),
        serde_json::json!({
            "code": room.code,
            "name": room.name,
            "area_id": room.area_id,
            "device_slot": room.device_slot,
            "timezone": room.timezone,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(room)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateRoomRequest>,
) -> AppResult<Json<Room>> {
    authorization::require_permission(&state, id, permissions::ROOM_MANAGE).await?;

    if request.area_id.is_none()
        && request.code.is_none()
        && request.name.is_none()
        && request.capacity.is_none()
        && request.device_slot.is_none()
        && request.webhook_url.is_none()
        && request.is_active.is_none()
        && request.metadata.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }

    if let Some(area_id) = request.area_id
        && state.areas.get(area_id).await?.is_none()
    {
        return Err(AppError::not_found("区域不存在"));
    }

    let device_slot = match request.device_slot {
        Some(Some(raw)) => Some(normalize_device_slot(&raw)?),
        Some(None) => Some(None),
        None => None,
    };
    let webhook_url = match request.webhook_url {
        Some(Some(raw)) => Some(validation::normalize_webhook_url(&raw)?),
        Some(None) => Some(None),
        None => None,
    };

    let input = UpdateRoom {
        area_id: request.area_id,
        code: request
            .code
            .as_deref()
            .map(|code| validation::normalize_code(code, "房间编码"))
            .transpose()?,
        name: request
            .name
            .as_deref()
            .map(|name| normalize_name(name, "房间名称"))
            .transpose()?,
        capacity: request
            .capacity
            .map(validation::validate_capacity)
            .transpose()?,
        device_slot,
        webhook_url,
        is_active: request.is_active,
        metadata: match request.metadata {
            None => None,
            Some(None) => Some(serde_json::json!({})),
            Some(Some(value)) => Some(validation::validate_metadata(value)?),
        },
    };
    let room = state
        .rooms
        .update(room_id, &input)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "room.update",
        "room",
        Some(room.id.to_string()),
        serde_json::json!({
            "code": room.code,
            "name": room.name,
            "area_id": room.area_id,
            "device_slot": room.device_slot,
            "is_active": room.is_active,
            "timezone": room.timezone,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(room))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::ROOM_MANAGE).await?;

    // 课表、周模板、单日覆盖、实例与触发规则由外键级联删除。
    if !state.rooms.delete(room_id).await? {
        return Err(AppError::not_found("房间不存在"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "room.delete",
        "room",
        Some(room_id.to_string()),
        serde_json::json!({}),
        ip.as_deref(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
