use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Request, rejection::JsonRejection},
    http::{header::AUTHORIZATION, request::Parts},
};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::{auth::SESSION_ACTIVE_CACHE_TTL_SECS, error::AppError, state::AppState};

/// 已通过访问令牌认证的用户。
#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header_value = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| AppError::unauthorized("缺少 Authorization 请求头"))?;

        let token = header_value
            .split_once(' ')
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
            .map(|(_, token)| token.trim())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| AppError::unauthorized("Authorization 需为 `Bearer <token>` 格式"))?;

        let verified = state.jwt.verify_access_token(token)?;
        if let Some(session_id) = verified.session_id {
            let cached = state
                .cache
                .cached_session_active(session_id)
                .await
                .ok()
                .flatten();
            let active = match cached {
                Some(value) => value,
                None => {
                    let active = state.security.session_active(session_id).await?;
                    if active {
                        let _ = state
                            .cache
                            .mark_session_active(session_id, SESSION_ACTIVE_CACHE_TTL_SECS)
                            .await;
                    }
                    active
                }
            };
            if !active {
                return Err(AppError::unauthorized("会话已撤销，请重新登录"));
            }
        }
        Ok(Self {
            id: verified.user_id,
        })
    }
}

/// 反向代理后的客户端 IP。优先使用 nginx 设置的 `X-Real-IP` /
/// `X-Forwarded-For`；仅用于审计，不参与授权。
#[derive(Debug, Clone)]
pub struct ClientIp(pub Option<String>);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let forwarded = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        let real_ip = parts
            .headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        Ok(Self(real_ip.or(forwarded)))
    }
}

/// 与项目统一错误格式对齐的 JSON body extractor。
///
/// 直接使用 `axum::Json` 时，反序列化失败会返回纯文本 422；
/// 这里统一转换成 `{ "error": { ... } }`。
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|rejection: JsonRejection| AppError::bad_request(rejection.body_text()))?;
        Ok(Self(value))
    }
}
