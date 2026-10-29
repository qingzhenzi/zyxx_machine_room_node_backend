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
}

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
    use super::DbError;

    #[test]
    fn pool_timeout_is_classified_as_unavailable() {
        assert!(matches!(
            DbError::from(sqlx::Error::PoolTimedOut),
            DbError::Unavailable(_)
        ));
    }
}
