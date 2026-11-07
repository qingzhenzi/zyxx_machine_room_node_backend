use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

/// 面向 HTTP 的错误类型。
///
/// 5xx 只向客户端返回通用文案，详细原因写入服务端日志。
#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    MethodNotAllowed(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    UnprocessableEntity(String),
    #[error("{0}")]
    TooManyRequests(String),
    #[error("{0}")]
    ServiceUnavailable(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        Self::MethodNotAllowed(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    /// 422：请求语义合法，但当前资源状态无法完成操作（如机器没有可用 MAC）。
    pub fn unprocessable_entity(message: impl Into<String>) -> Self {
        Self::UnprocessableEntity(message.into())
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::TooManyRequests(message.into())
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::ServiceUnavailable(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed(_) => StatusCode::METHOD_NOT_ALLOWED,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::UnprocessableEntity(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::MethodNotAllowed(_) => "method_not_allowed",
            Self::Conflict(_) => "conflict",
            Self::UnprocessableEntity(_) => "unprocessable_entity",
            Self::TooManyRequests(_) => "too_many_requests",
            Self::ServiceUnavailable(_) => "service_unavailable",
            Self::Internal(_) => "internal_error",
        }
    }
}

impl From<qe_domain::error::DomainError> for AppError {
    fn from(error: qe_domain::error::DomainError) -> Self {
        match error {
            qe_domain::error::DomainError::BadRequest(message) => Self::BadRequest(message),
            qe_domain::error::DomainError::NotFound(message) => Self::NotFound(message),
            qe_domain::error::DomainError::Conflict(message) => Self::Conflict(message),
            qe_domain::error::DomainError::Internal(message) => Self::Internal(message),
        }
    }
}

impl From<qe_db::error::DbError> for AppError {
    fn from(error: qe_db::error::DbError) -> Self {
        match error {
            qe_db::error::DbError::BadRequest(message) => Self::BadRequest(message),
            qe_db::error::DbError::NotFound(message) => Self::NotFound(message),
            qe_db::error::DbError::Conflict(message) => Self::Conflict(message),
            qe_db::error::DbError::Unavailable(message) => Self::ServiceUnavailable(message),
            qe_db::error::DbError::Internal(message) => Self::Internal(message),
        }
    }
}

impl From<qe_cache::CacheError> for AppError {
    fn from(error: qe_cache::CacheError) -> Self {
        Self::Internal(error.to_string())
    }
}

/// 认证层错误统一落到 HTTP 错误上。
///
/// `AuthError::Internal` 的原始信息只在服务端日志里出现，
/// `IntoResponse` 会把 5xx 的响应体替换成通用文案。
impl From<qe_auth::AuthError> for AppError {
    fn from(error: qe_auth::AuthError) -> Self {
        match error {
            qe_auth::AuthError::BadRequest(message) => Self::BadRequest(message),
            qe_auth::AuthError::Unauthorized(message) => Self::Unauthorized(message),
            qe_auth::AuthError::Forbidden(message) => Self::Forbidden(message),
            qe_auth::AuthError::Internal(message) => Self::Internal(message),
        }
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status == StatusCode::SERVICE_UNAVAILABLE {
            tracing::warn!(error = %self, "request failed with service unavailable");
        } else if status.is_server_error() {
            tracing::error!(error = %self, "request failed with server error");
        }
        let message = if status == StatusCode::SERVICE_UNAVAILABLE {
            "服务暂时不可用，请稍后重试".to_owned()
        } else if status.is_server_error() {
            "服务器内部错误".to_owned()
        } else {
            self.to_string()
        };

        let body = ErrorResponse {
            error: ErrorBody {
                code: self.code(),
                message,
            },
        };
        (status, Json(body)).into_response()
    }
}
