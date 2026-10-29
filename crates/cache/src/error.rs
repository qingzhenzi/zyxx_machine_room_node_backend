use thiserror::Error;

pub type CacheResult<T> = Result<T, CacheError>;

/// 缓存层错误。Redis/序列化失败都属于内部错误，由 API 层统一记录。
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("{0}")]
    Internal(String),
}

impl CacheError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl From<redis::RedisError> for CacheError {
    fn from(error: redis::RedisError) -> Self {
        Self::internal(format!("redis error: {error}"))
    }
}

impl From<serde_json::Error> for CacheError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(format!("json error: {error}"))
    }
}
