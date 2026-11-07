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
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("资源不存在")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("对象存储未配置或不可用")]
    StorageUnavailable,
    #[error("请求过于频繁，请 {0}s 后再试")]
    RateLimited(u64),
    #[error("该设备未 ACK 的下行消息已达 {0} 条，暂停投递（背压）")]
    DownlinkFull(usize),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("内部错误: {0}")]
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
            Error::Forbidden(_) => "forbidden",
            Error::NotFound => "not_found",
            Error::Conflict(_) => "conflict",
            Error::StorageUnavailable => "storage_unavailable",
            Error::RateLimited(_) => "rate_limited",
            Error::DownlinkFull(_) => "downlink_full",
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
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::NotFound => StatusCode::NOT_FOUND,
            Error::Conflict(_) => StatusCode::CONFLICT,
            // 预签名依赖外部对象存储：没配置时不掩盖，让客户端按可重试处理
            Error::StorageUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Error::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            // 背压：让客户端稍后重试（拉取侧收到它就知道该等了）
            Error::DownlinkFull(_) => StatusCode::SERVICE_UNAVAILABLE,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    /// 限速错误的建议等待秒数；其它错误为 None。
    fn retry_after(&self) -> Option<u64> {
        match self {
            Error::RateLimited(secs) => Some(*secs),
            _ => None,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: self.code(),
            message: self.to_string(),
        };
        let mut resp = (self.status(), Json(body)).into_response();
        // 限速时告诉客户端什么时候再来，省得盲试
        if let Some(retry_after) = self.retry_after()
            && let Ok(v) = axum::http::HeaderValue::from_str(&retry_after.to_string())
        {
            resp.headers_mut()
                .insert(axum::http::header::RETRY_AFTER, v);
        }
        resp
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
