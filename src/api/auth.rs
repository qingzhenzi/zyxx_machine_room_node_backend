use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    api::extractors::{ApiJson, AuthUser},
    auth::{authorization, password, session},
    domain::{
        models::{User, UserProfile},
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub token_type: &'static str,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub user: UserProfile,
}

pub async fn login(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<LoginRequest>,
) -> AppResult<Json<TokenResponse>> {
    let username = validation::normalize_username(&request.username)
        .map_err(|_| AppError::unauthorized("用户名或密码错误"))?;

    let Some(user) = state.users.find_by_username(&username).await? else {
        return Err(AppError::unauthorized("用户名或密码错误"));
    };

    let password_valid =
        password::verify_password_async(request.password, user.password_hash.clone()).await?;
    if !password_valid {
        return Err(AppError::unauthorized("用户名或密码错误"));
    }
    if !user.is_active {
        return Err(AppError::forbidden("账号已被停用"));
    }

    Ok(Json(issue_tokens(&state, &user).await?))
}

pub async fn refresh(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<RefreshRequest>,
) -> AppResult<Json<TokenResponse>> {
    let user_id =
        session::consume_refresh_token(&state.cache, request.refresh_token.trim()).await?;
    let Some(user) = state.users.find_by_id(user_id).await? else {
        return Err(AppError::unauthorized("账号不存在"));
    };
    if !user.is_active {
        return Err(AppError::forbidden("账号已被停用"));
    }

    Ok(Json(issue_tokens(&state, &user).await?))
}

pub async fn logout(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<RefreshRequest>,
) -> AppResult<StatusCode> {
    if !request.refresh_token.trim().is_empty() {
        session::revoke_refresh_token(&state.cache, request.refresh_token.trim()).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<UserProfile>> {
    let Some(user) = state.users.find_by_id(id).await? else {
        return Err(AppError::unauthorized("账号不存在"));
    };
    if !user.is_active {
        return Err(AppError::forbidden("账号已被停用"));
    }
    Ok(Json(authorization::profile_of(&state, &user).await?))
}

async fn issue_tokens(state: &AppState, user: &User) -> AppResult<TokenResponse> {
    let access_token = state
        .jwt
        .issue_access_token(user.id, state.config.access_token_ttl_secs)?;
    let refresh_token =
        session::create_refresh_token(&state.cache, user.id, state.config.refresh_token_ttl_secs)
            .await?;
    let user_profile = authorization::profile_of(state, user).await?;

    Ok(TokenResponse {
        token_type: "Bearer",
        access_token,
        refresh_token,
        expires_in: state.config.access_token_ttl_secs,
        user: user_profile,
    })
}
