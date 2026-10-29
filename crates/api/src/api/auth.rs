use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::{authorization, password, session},
    domain::{
        models::{User, UserProfile, UserSession},
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

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyEmailRequest {
    pub token: String,
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
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ApiJson(request): ApiJson<LoginRequest>,
) -> AppResult<Json<TokenResponse>> {
    let username = validation::normalize_username(&request.username)
        .map_err(|_| AppError::unauthorized("用户名或密码错误"))?;

    if let Some(retry_after) = state.cache.login_retry_after(&username).await? {
        return Err(AppError::too_many_requests(format!(
            "登录失败次数过多，请 {retry_after} 秒后重试"
        )));
    }

    let Some(user) = state.users.find_by_username(&username).await? else {
        record_login_failure(&state, &username).await?;
        return Err(AppError::unauthorized("用户名或密码错误"));
    };

    let password_valid =
        password::verify_password_async(request.password, user.password_hash.clone()).await?;
    if !password_valid {
        record_login_failure(&state, &username).await?;
        return Err(AppError::unauthorized("用户名或密码错误"));
    }
    if !user.is_active {
        return Err(AppError::forbidden("账号已被停用"));
    }

    state.cache.clear_login_failures(&username).await?;
    let session_id = Uuid::new_v4();
    let user_agent = request_header(&headers, "user-agent");
    Ok(Json(
        issue_tokens(&state, &user, session_id, &user_agent, ip.as_deref(), true).await?,
    ))
}

async fn record_login_failure(state: &AppState, username: &str) -> AppResult<()> {
    let attempts = state
        .cache
        .record_login_failure(
            username,
            state.config.login_window_secs,
            state.config.login_max_attempts,
            state.config.login_lock_secs,
        )
        .await?;
    if attempts >= u64::from(state.config.login_max_attempts) {
        tracing::warn!(%username, attempts, "登录失败次数达到阈值，账号已被临时锁定");
    }
    Ok(())
}

pub async fn refresh(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<RefreshRequest>,
) -> AppResult<Json<TokenResponse>> {
    let consumed =
        session::consume_refresh_token(&state.cache, request.refresh_token.trim()).await?;
    if !state.security.session_active(consumed.session_id).await? {
        return Err(AppError::unauthorized("会话已撤销"));
    }

    let Some(user) = state.users.find_by_id(consumed.user_id).await? else {
        return Err(AppError::unauthorized("账号不存在"));
    };
    if !user.is_active {
        return Err(AppError::forbidden("账号已被停用"));
    }

    Ok(Json(
        issue_tokens(&state, &user, consumed.session_id, "", None, false).await?,
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<RefreshRequest>,
) -> AppResult<StatusCode> {
    if !request.refresh_token.trim().is_empty()
        && let Some(revoked) =
            session::revoke_refresh_token(&state.cache, request.refresh_token.trim()).await?
    {
        state
            .security
            .revoke_session(revoked.user_id, revoked.session_id)
            .await?;
        let _ = state.cache.revoke_session_cache(revoked.session_id).await;
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

pub async fn forgot_password(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ForgotPasswordRequest>,
) -> AppResult<StatusCode> {
    // 无论邮箱是否存在都返回 204，避免账号枚举。
    let email = request.email.trim();
    if let Some(user) = state.users.find_by_email(email).await?
        && user.is_active
    {
        let token = new_token();
        let expires_at =
            Utc::now() + Duration::seconds(state.config.password_reset_ttl_secs as i64);
        state
            .security
            .create_password_reset(user.id, &session::refresh_token_hash(&token), expires_at)
            .await?;
        state.email.send_password_reset(email, &token).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset_password(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ResetPasswordRequest>,
) -> AppResult<StatusCode> {
    validation::validate_password(&request.new_password)?;
    let token_hash = session::refresh_token_hash(request.token.trim());
    let Some(user_id) = state.security.consume_password_reset(&token_hash).await? else {
        return Err(AppError::bad_request("重置令牌无效或已过期"));
    };
    let password_hash = password::hash_password_async(request.new_password).await?;
    if !state.users.set_password(user_id, &password_hash).await? {
        return Err(AppError::not_found("账号不存在"));
    }
    let sessions = state.security.list_sessions(user_id).await?;
    state.security.revoke_all_sessions(user_id).await?;
    for session in sessions {
        let _ = state.cache.revoke_session_cache(session.id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn request_email_verification(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<StatusCode> {
    let Some(user) = state.users.find_by_id(id).await? else {
        return Err(AppError::not_found("账号不存在"));
    };
    let Some(email) = user.email.as_deref().filter(|value| !value.is_empty()) else {
        return Err(AppError::bad_request("账号未设置邮箱"));
    };

    let token = new_token();
    let expires_at =
        Utc::now() + Duration::seconds(state.config.email_verification_ttl_secs as i64);
    state
        .security
        .create_email_verification(id, &session::refresh_token_hash(&token), expires_at)
        .await?;
    state.email.send_email_verification(email, &token).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn verify_email(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<VerifyEmailRequest>,
) -> AppResult<StatusCode> {
    let token_hash = session::refresh_token_hash(request.token.trim());
    let Some(user_id) = state
        .security
        .consume_email_verification(&token_hash)
        .await?
    else {
        return Err(AppError::bad_request("验证令牌无效或已过期"));
    };
    if !state.users.mark_email_verified(user_id).await? {
        return Err(AppError::not_found("账号不存在"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<UserSession>>> {
    Ok(Json(state.security.list_sessions(id).await?))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(session_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    if !state.security.revoke_session(id, session_id).await? {
        return Err(AppError::not_found("会话不存在"));
    }
    let _ = state.cache.revoke_session_cache(session_id).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_all_sessions(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<StatusCode> {
    let sessions = state.security.list_sessions(id).await?;
    state.security.revoke_all_sessions(id).await?;
    for session in sessions {
        let _ = state.cache.revoke_session_cache(session.id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn issue_tokens(
    state: &AppState,
    user: &User,
    session_id: Uuid,
    user_agent: &str,
    ip: Option<&str>,
    new_session: bool,
) -> AppResult<TokenResponse> {
    let access_token =
        state
            .jwt
            .issue_access_token(user.id, session_id, state.config.access_token_ttl_secs)?;
    let refresh_token = session::create_refresh_token(
        &state.cache,
        user.id,
        session_id,
        state.config.refresh_token_ttl_secs,
    )
    .await?;
    let token_hash = session::refresh_token_hash(&refresh_token);

    if new_session {
        state
            .security
            .create_session_with_id(
                session_id,
                user.id,
                &token_hash,
                &user_agent.chars().take(512).collect::<String>(),
                ip,
            )
            .await?;
    } else if !state
        .security
        .rotate_session_token(session_id, &token_hash)
        .await?
    {
        return Err(AppError::unauthorized("会话已撤销"));
    }

    let _ = state
        .cache
        .mark_session_active(session_id, state.config.refresh_token_ttl_secs as u64)
        .await;

    let user_profile = authorization::profile_of(state, user).await?;
    Ok(TokenResponse {
        token_type: "Bearer",
        access_token,
        refresh_token,
        expires_in: state.config.access_token_ttl_secs,
        user: user_profile,
    })
}

fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn request_header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_owned()
}
