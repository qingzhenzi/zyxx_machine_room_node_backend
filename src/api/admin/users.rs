use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::{
        extractors::{ApiJson, AuthUser},
        validation::PageQuery,
    },
    auth::{authorization, password},
    domain::{
        models::{Page, UserSummary},
        rbac::{permissions, roles},
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    /// 传空字符串表示清空邮箱；不传表示不修改。
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetUserRolesRequest {
    #[serde(default)]
    pub role_codes: Vec<String>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<Page<UserSummary>>> {
    authorization::require_permission(&state, id, permissions::USER_MANAGE).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;

    Ok(Json(state.users.list(page, page_size, &keyword).await?))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ApiJson(request): ApiJson<CreateUserRequest>,
) -> AppResult<(StatusCode, Json<UserSummary>)> {
    authorization::require_permission(&state, id, permissions::USER_MANAGE).await?;

    let username = validation::normalize_username(&request.username)?;
    validation::validate_password(&request.password)?;
    let display_name = validation::normalize_display_name(&request.display_name)?;
    let email = match request.email.as_deref() {
        Some(raw) => validation::normalize_optional_email(raw)?,
        None => None,
    };
    let mut role_codes = validation::normalize_role_codes(&request.role_codes)?;
    if role_codes.is_empty() {
        role_codes.push(roles::VIEWER.to_owned());
    }

    let password_hash = password::hash_password_async(request.password).await?;
    let user = state
        .users
        .create_with_roles(
            &username,
            email.as_deref(),
            &display_name,
            &password_hash,
            &role_codes,
        )
        .await?;

    authorization::bump_permission_version(&state).await;
    let summary = state
        .users
        .find_summary(user.id)
        .await?
        .ok_or_else(|| AppError::internal("创建用户后无法读取用户信息"))?;

    Ok((StatusCode::CREATED, Json(summary)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id: actor_id }: AuthUser,
    Path(user_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateUserRequest>,
) -> AppResult<Json<UserSummary>> {
    authorization::require_permission(&state, actor_id, permissions::USER_MANAGE).await?;

    if request.display_name.is_none()
        && request.email.is_none()
        && request.is_active.is_none()
        && request.password.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }

    let Some(target) = state.users.find_by_id(user_id).await? else {
        return Err(AppError::not_found("用户不存在"));
    };

    if request.is_active == Some(false) {
        if user_id == actor_id {
            return Err(AppError::bad_request("不能停用当前登录账号"));
        }
        let target_roles = state.users.roles_of(user_id).await?;
        if target.is_active
            && target_roles
                .iter()
                .any(|role| role.as_str() == roles::ADMIN)
        {
            let active_admins = state.users.count_active_with_role(roles::ADMIN).await?;
            if active_admins <= 1 {
                return Err(AppError::conflict("不能停用最后一个管理员"));
            }
        }
    }

    let display_name = request
        .display_name
        .as_deref()
        .map(validation::normalize_display_name)
        .transpose()?;
    let email_change = match request.email.as_deref() {
        Some(raw) => Some(validation::normalize_optional_email(raw)?),
        None => None,
    };
    let password_hash = match request.password {
        Some(password) => {
            validation::validate_password(&password)?;
            Some(password::hash_password_async(password).await?)
        }
        None => None,
    };

    state
        .users
        .update(
            user_id,
            email_change,
            display_name,
            password_hash,
            request.is_active,
        )
        .await?
        .ok_or_else(|| AppError::not_found("用户不存在"))?;

    if request.is_active.is_some() {
        authorization::bump_permission_version(&state).await;
    }

    let summary = state
        .users
        .find_summary(user_id)
        .await?
        .ok_or_else(|| AppError::internal("更新用户后无法读取用户信息"))?;
    Ok(Json(summary))
}

pub async fn set_roles(
    State(state): State<AppState>,
    AuthUser { id: actor_id }: AuthUser,
    Path(user_id): Path<Uuid>,
    ApiJson(request): ApiJson<SetUserRolesRequest>,
) -> AppResult<Json<UserSummary>> {
    authorization::require_permission(&state, actor_id, permissions::USER_MANAGE).await?;

    let Some(target) = state.users.find_by_id(user_id).await? else {
        return Err(AppError::not_found("用户不存在"));
    };
    let role_codes = validation::normalize_role_codes(&request.role_codes)?;

    let target_roles = state.users.roles_of(user_id).await?;
    let target_is_admin = target_roles
        .iter()
        .any(|role| role.as_str() == roles::ADMIN);
    let next_is_admin = role_codes.iter().any(|role| role.as_str() == roles::ADMIN);
    if target.is_active && target_is_admin && !next_is_admin {
        let active_admins = state.users.count_active_with_role(roles::ADMIN).await?;
        if active_admins <= 1 {
            return Err(AppError::conflict("不能移除最后一个管理员"));
        }
    }

    state.users.set_roles(user_id, &role_codes).await?;
    authorization::bump_permission_version(&state).await;

    let summary = state
        .users
        .find_summary(user_id)
        .await?
        .ok_or_else(|| AppError::internal("更新角色后无法读取用户信息"))?;
    Ok(Json(summary))
}
