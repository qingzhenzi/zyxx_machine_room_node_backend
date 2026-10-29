use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::{Permission, RoleSummary},
        rbac::permissions,
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub permission_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetRolePermissionsRequest {
    #[serde(default)]
    pub permission_codes: Vec<String>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<RoleSummary>>> {
    authorization::require_any_permission(
        &state,
        id,
        &[permissions::ROLE_MANAGE, permissions::USER_MANAGE],
    )
    .await?;
    Ok(Json(state.roles.list().await?))
}

pub async fn list_permissions(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<Permission>>> {
    authorization::require_any_permission(
        &state,
        id,
        &[permissions::ROLE_MANAGE, permissions::USER_MANAGE],
    )
    .await?;
    Ok(Json(state.roles.list_permissions().await?))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateRoleRequest>,
) -> AppResult<(StatusCode, Json<RoleSummary>)> {
    authorization::require_permission(&state, id, permissions::ROLE_MANAGE).await?;

    let code = validation::normalize_role_code(&request.code)?;
    let name = validation::normalize_role_name(&request.name)?;
    let description = validation::normalize_description(&request.description)?;
    let permission_codes = validation::normalize_permission_codes(&request.permission_codes)?;

    let role = state
        .roles
        .create(&code, &name, &description, &permission_codes)
        .await?;
    authorization::bump_permission_version(&state).await;

    super::audit::record_or_log(
        &state,
        id,
        "role.create",
        "role",
        Some(role.id.to_string()),
        serde_json::json!({ "code": role.code, "permissions": role.permissions }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(role)))
}

pub async fn set_permissions(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(role_id): Path<Uuid>,
    ApiJson(request): ApiJson<SetRolePermissionsRequest>,
) -> AppResult<Json<RoleSummary>> {
    authorization::require_permission(&state, id, permissions::ROLE_MANAGE).await?;

    let Some(role) = state.roles.find_by_id(role_id).await? else {
        return Err(AppError::not_found("角色不存在"));
    };
    if role.is_system {
        return Err(AppError::conflict("系统内置角色的权限不可修改"));
    }

    let permission_codes = validation::normalize_permission_codes(&request.permission_codes)?;
    state
        .roles
        .set_permissions(role_id, &permission_codes)
        .await?;
    authorization::bump_permission_version(&state).await;

    let summary = state
        .roles
        .find_summary(role_id)
        .await?
        .ok_or_else(|| AppError::internal("更新权限后无法读取角色信息"))?;

    super::audit::record_or_log(
        &state,
        id,
        "role.set_permissions",
        "role",
        Some(role_id.to_string()),
        serde_json::json!({ "permissions": summary.permissions }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(summary))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(role_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::ROLE_MANAGE).await?;

    let Some(role) = state.roles.find_by_id(role_id).await? else {
        return Err(AppError::not_found("角色不存在"));
    };
    if role.is_system {
        return Err(AppError::conflict("系统内置角色不可删除"));
    }

    if !state.roles.delete(role_id).await? {
        return Err(AppError::not_found("角色不存在"));
    }
    authorization::bump_permission_version(&state).await;

    super::audit::record_or_log(
        &state,
        id,
        "role.delete",
        "role",
        Some(role_id.to_string()),
        serde_json::json!({ "code": role.code }),
        ip.as_deref(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
