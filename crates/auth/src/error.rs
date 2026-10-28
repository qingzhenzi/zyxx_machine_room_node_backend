use thiserror::Error;

pub type AuthResult<T> = Result<T, AuthError>;

/// 认证层的错误。
///
/// 刻意不实现 `IntoResponse`：HTTP 状态码和错误码格式是应用层的约定，
/// 由 `backend` 把它映射成自己的 `AppError` 再输出。
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Internal(String),
}

impl AuthError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn is_internal(&self) -> bool {
        matches!(self, Self::Internal(_))
    }
}

impl From<argon2::password_hash::Error> for AuthError {
    fn from(error: argon2::password_hash::Error) -> Self {
        Self::internal(format!("password hash error: {error}"))
    }
}
