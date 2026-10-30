use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// 契约第 8 节错误码。序列化为 {"code","message"} JSON。
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("token 不存在/过期/已用")]
    TokenInvalid,
    #[error("slot 已有存活证书")]
    SlotConflict,
    #[error("challenge 已过期")]
    ChallengeExpired,
    #[error("challenge 已被消费")]
    ChallengeConsumed,
    #[error("challenge 与请求不匹配")]
    ChallengeMismatch,
    #[error("持有证明验签失败")]
    PopInvalid,
    #[error("TOFU 指纹与服务端实际不符（疑似 MITM）")]
    FpMismatch,
    #[error("注册单待审批")]
    PendingApproval,
    #[error("注册单已被拒绝")]
    ApprovalRejected,
    #[error("注册单不存在")]
    EnrollmentNotFound,
    #[error("证书已被下载，不可再取")]
    AlreadyDownloaded,
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("内部错误")]
    Internal(#[source] anyhow::Error),
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::TokenInvalid => "token_invalid",
            Error::SlotConflict => "slot_conflict",
            Error::ChallengeExpired => "challenge_expired",
            Error::ChallengeConsumed => "challenge_consumed",
            Error::ChallengeMismatch => "challenge_mismatch",
            Error::PopInvalid => "pop_invalid",
            Error::FpMismatch => "fp_mismatch",
            Error::PendingApproval => "pending_approval",
            Error::ApprovalRejected => "approval_rejected",
            Error::EnrollmentNotFound => "enrollment_not_found",
            Error::AlreadyDownloaded => "already_downloaded",
            Error::Unauthorized => "unauthorized",
            Error::BadRequest(_) => "bad_request",
            Error::Internal(_) => "internal",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Error::TokenInvalid => StatusCode::UNAUTHORIZED,
            Error::SlotConflict => StatusCode::CONFLICT,
            Error::ChallengeExpired
            | Error::ChallengeConsumed
            | Error::ChallengeMismatch
            | Error::PopInvalid
            | Error::FpMismatch
            | Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::PendingApproval => StatusCode::ACCEPTED,
            Error::ApprovalRejected => StatusCode::FORBIDDEN,
            Error::EnrollmentNotFound => StatusCode::NOT_FOUND,
            Error::AlreadyDownloaded => StatusCode::GONE,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: self.code(),
            message: self.to_string(),
        };
        (self.status(), Json(body)).into_response()
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Internal(e)
    }
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Error::Internal(e.into())
    }
}

impl From<rcgen::Error> for Error {
    fn from(e: rcgen::Error) -> Self {
        Error::Internal(e.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
