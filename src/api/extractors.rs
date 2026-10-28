use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Request, rejection::JsonRejection},
    http::{header::AUTHORIZATION, request::Parts},
};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

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

        let id = state.jwt.verify_access_token(token)?;
        Ok(Self { id })
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
