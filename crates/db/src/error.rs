use thiserror::Error;

pub type DbResult<T> = Result<T, DbError>;

/// 数据库层错误。
///
/// 刻意不依赖 `AppError`/Axum；HTTP 状态码映射留在 `qe-api`。
#[derive(Debug, Error)]
pub enum DbError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    /// 连接池耗尽、连接中断、数据库重启等可重试状态。
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Internal(String),
}

impl DbError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// 是否属于「数据库繁忙 / 连接暂不可用」这类**可短退避重试**的错误。
    ///
    /// 只有 [`DbError::Unavailable`] 命中（连接池等待超时、连接中断、数据库重启、
    /// 锁超时/死锁/连接数打满等，见 `From<sqlx::Error>` 的分类）；参数/约束/冲突
    /// 等业务错误重试没有意义，调用方应立即失败。
    pub const fn is_busy(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// 分片上传等高频短事务的 DB 繁忙重试退避（毫秒）：最多重试 3 次。
///
/// 与 [`DbError::is_busy`] 配合：仅在 `Unavailable` 时退避重试，其余错误立即返回。
/// 用途：并发分片上传会瞬间占满连接池，偶发 `PoolTimedOut -> 503`；短退避通常
/// 50-150ms 就能拿到连接，避免把瞬时抖动暴露给教师端。
pub const DB_BUSY_RETRY_DELAYS_MS: [u64; 3] = [50, 150, 400];

impl From<sqlx::Error> for DbError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(database_error) = &error {
            if database_error.is_unique_violation() {
                return Self::conflict("资源已存在");
            }
            if database_error.is_foreign_key_violation() {
                return Self::bad_request("引用的资源不存在");
            }
            if database_error.is_check_violation() {
                return Self::bad_request("数据不满足约束条件");
            }
            // SQLSTATE class 08 = connection exception；57P0x = shutdown/crash。
            // 这两类通常是数据库重启/主备切换，属于可重试的不可用状态。
            if let Some(code) = database_error.code() {
                // 连接中断/数据库重启/主备切换
                if code.starts_with("08") || code.starts_with("57P") {
                    return Self::unavailable("数据库连接暂时不可用");
                }
                // 锁超时、死锁、连接数打满：通常是瞬时的，可让客户端重试
                if matches!(
                    code.as_ref(),
                    "55P03" | "40001" | "40P01" | "53300" | "57014" | "25P03"
                ) {
                    return Self::unavailable("数据库繁忙，请稍后重试");
                }
            }
        }

        match &error {
            sqlx::Error::PoolTimedOut => Self::unavailable("数据库连接池等待超时"),
            sqlx::Error::PoolClosed => Self::unavailable("数据库连接池已关闭"),
            sqlx::Error::Io(_) => Self::unavailable("数据库连接中断"),
            sqlx::Error::RowNotFound => Self::not_found("记录不存在"),
            _ => Self::internal(format!("database error: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DB_BUSY_RETRY_DELAYS_MS, DbError};

    #[test]
    fn pool_timeout_is_classified_as_unavailable() {
        assert!(matches!(
            DbError::from(sqlx::Error::PoolTimedOut),
            DbError::Unavailable(_)
        ));
    }

    #[test]
    fn busy_classification_only_covers_unavailable() {
        assert!(DbError::unavailable("数据库繁忙").is_busy());
        assert!(DbError::from(sqlx::Error::PoolTimedOut).is_busy());
        assert!(!DbError::bad_request("bad arg").is_busy());
        assert!(!DbError::not_found("missing").is_busy());
        assert!(!DbError::conflict("dup").is_busy());
        assert!(!DbError::internal("boom").is_busy());
    }

    #[test]
    fn busy_retry_delays_are_short_and_increasing() {
        assert_eq!(DB_BUSY_RETRY_DELAYS_MS, [50, 150, 400]);
        assert!(
            DB_BUSY_RETRY_DELAYS_MS
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(DB_BUSY_RETRY_DELAYS_MS.iter().all(|ms| *ms <= 500));
    }
}
