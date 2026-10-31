//! v3 设备接入区域/房间：设备列表/详情、房间设备摘要、绑定/迁移/解绑、签发注册 token。
//!
//! 契约见 `docs/schedule-design.md` 第 11.4 节。设备注册记录本身由 `device_enrollment`
//! 管理，本模块只读设备状态，并在 `rooms.device_slot` 上完成绑定关系；设备在线状态来自
//! `device_session`。`Room` 响应里的 `device` 摘要由 `qe-db` 的 Room 查询自动带出。

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        device::{BootstrapTokenKind, DeviceFilter, DeviceInfo, DeviceStatus, RoomDevice},
        models::Page,
        rbac::permissions,
        schedule::Room,
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

/// 设备注册 token 的默认有效期（秒）。与设备侧一次性 token 的默认值保持一致。
const DEFAULT_DEVICE_TOKEN_TTL_SECS: i64 = 900;

#[derive(Debug, Deserialize)]
pub struct DeviceQuery {
    #[serde(default)]
    pub area_id: Option<Uuid>,
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub status: Option<String>,
    /// `true` = 只看已绑定房间的设备；`false` = 只看未绑定的设备。
    #[serde(default)]
    pub assigned: Option<bool>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

/// `PUT /rooms/{id}/device` 请求体。
///
/// `device_slot` 必须显式出现：字符串表示绑定/迁移，`null` 表示解绑；
/// 缺失该字段视为非法请求（避免“误传空 body 导致意外解绑”）。
#[derive(Debug, Deserialize)]
pub struct BindDeviceRequest {
    pub device_slot: Option<String>,
}

/// `POST /rooms/{id}/device-token` 请求体：`kind` 缺省 `auto`，`ttl_secs` 缺省 900。
#[derive(Debug, Deserialize)]
pub struct DeviceTokenRequest {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub ttl_secs: Option<i64>,
}

/// 签发结果：明文 token 只在这一次响应里出现，库中只存 SHA-256。
#[derive(Debug, Serialize)]
pub struct DeviceTokenReply {
    pub token: String,
    pub kind: BootstrapTokenKind,
    pub device_slot: String,
    pub expires_at: DateTime<Utc>,
    /// 设备注册链接：`{PUBLIC_ENROLL_ENDPOINT}/enroll?token=…&slot=…`。
    pub link: String,
}

fn parse_device_status(raw: Option<&str>) -> AppResult<Option<DeviceStatus>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => value.parse::<DeviceStatus>().map(Some).map_err(|_| {
            AppError::bad_request(
                "无效的设备状态，可选值为 pending / issued / active / rejected / revoked",
            )
        }),
    }
}

fn parse_token_kind(raw: Option<&str>) -> AppResult<BootstrapTokenKind> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(BootstrapTokenKind::Auto),
        Some(value) => value
            .parse::<BootstrapTokenKind>()
            .map_err(|_| AppError::bad_request("无效的注册 token 类型，可选值为 auto / approval")),
    }
}

/// `GET /devices`：设备分页（含房间/区域与在线状态）。
pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<DeviceQuery>,
) -> AppResult<Json<Page<DeviceInfo>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    let filter = DeviceFilter {
        area_id: query.area_id,
        room_id: query.room_id,
        status: parse_device_status(query.status.as_deref())?,
        assigned: query.assigned,
        keyword: (!keyword.is_empty()).then_some(keyword),
    };

    Ok(Json(state.devices.list(&filter, page, page_size).await?))
}

/// `GET /devices/{serial}`：设备详情；不存在 404。
pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(serial): Path<String>,
) -> AppResult<Json<DeviceInfo>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let device = state
        .devices
        .get(&serial)
        .await?
        .ok_or_else(|| AppError::not_found("设备不存在"))?;
    Ok(Json(device))
}

/// `GET /rooms/{id}/device`：房间设备摘要；未绑定槽位时返回 `null`，房间不存在 404。
pub async fn get_room_device(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(room_id): Path<Uuid>,
) -> AppResult<Json<Option<RoomDevice>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    // Room 查询已带出 device 摘要，这里只需保证房间存在。
    let room = state
        .rooms
        .get(room_id)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;
    Ok(Json(room.device))
}

/// `PUT /rooms/{id}/device`：绑定/迁移/解绑。
///
/// 同一槽位已绑定到其它房间时由仓储层在事务内自动迁移；并发冲突（槽位唯一索引）
/// 会以 `DbError::Conflict` 冒泡，统一映射为 409。
pub async fn bind_room_device(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<Uuid>,
    ApiJson(request): ApiJson<BindDeviceRequest>,
) -> AppResult<Json<Room>> {
    authorization::require_permission(&state, id, permissions::DEVICE_MANAGE).await?;

    let device_slot = request
        .device_slot
        .as_deref()
        .map(validation::normalize_device_slot)
        .transpose()?;

    let room = state
        .devices
        .bind_to_room(room_id, device_slot.as_deref())
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;

    let (action, detail) = match device_slot.as_deref() {
        Some(slot) => (
            "room.bind_device",
            json!({ "device_slot": slot, "room_code": room.code }),
        ),
        None => ("room.unbind_device", json!({ "room_code": room.code })),
    };
    super::audit::record_or_log(
        &state,
        id,
        action,
        "room",
        Some(room_id.to_string()),
        detail,
        ip.as_deref(),
    )
    .await;

    Ok(Json(room))
}

/// `POST /rooms/{id}/device-token`：确保房间有设备槽位后签发一次性注册 token。
///
/// 返回的 `link` 直接交给设备端使用；token 明文只出现一次。
pub async fn issue_device_token(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<Uuid>,
    ApiJson(request): ApiJson<DeviceTokenRequest>,
) -> AppResult<Json<DeviceTokenReply>> {
    authorization::require_permission(&state, id, permissions::DEVICE_MANAGE).await?;

    let kind = parse_token_kind(request.kind.as_deref())?;
    let ttl_secs = validation::validate_token_ttl_secs(
        request.ttl_secs.unwrap_or(DEFAULT_DEVICE_TOKEN_TTL_SECS),
    )?;

    // 先确保房间有槽位：已有则复用，否则用房间 code 自动生成。
    let device_slot = state
        .devices
        .ensure_room_slot(room_id)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;

    let issued = state
        .devices
        .issue_bootstrap_token(&device_slot, kind, &id.to_string(), ttl_secs)
        .await?;

    // token 为 64 位 hex、槽位只含 ascii 字母数字 - _ .，无需再做 URL 编码。
    let link = format!(
        "{}/enroll?token={}&slot={}",
        state.config.public_enroll_endpoint, issued.token, device_slot
    );

    super::audit::record_or_log(
        &state,
        id,
        "room.issue_device_token",
        "room",
        Some(room_id.to_string()),
        json!({
            "device_slot": device_slot,
            "kind": kind.as_str(),
            "expires_at": issued.expires_at,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(DeviceTokenReply {
        token: issued.token,
        kind,
        device_slot,
        expires_at: issued.expires_at,
        link,
    }))
}
