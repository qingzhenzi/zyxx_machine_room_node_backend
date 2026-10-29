use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{AuthError, AuthResult};

const REFRESH_TOKEN_PREFIX: &str = "auth:refresh:";

/// 刷新令牌的存储端口。
///
/// 生产实现是 Redis；测试可以换成内存实现，不需要起 Redis。
pub trait TokenStore {
    async fn set_ex(&self, key: &str, value: &str, ttl_secs: u64) -> AuthResult<()>;
    async fn get_del(&self, key: &str) -> AuthResult<Option<String>>;
    async fn del(&self, key: &str) -> AuthResult<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct RefreshSession {
    pub user_id: Uuid,
    pub session_id: Uuid,
}

/// 生成不透明刷新令牌并写入存储。键是令牌的 SHA-256 摘要，
/// 存储泄露也无法直接拿它登录。值绑定 user_id 与设备会话 ID。
pub async fn create_refresh_token(
    store: &impl TokenStore,
    user_id: Uuid,
    session_id: Uuid,
    ttl_secs: i64,
) -> AuthResult<String> {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    store
        .set_ex(
            &refresh_key(&token),
            &format!("{user_id}:{session_id}"),
            ttl_secs.max(1) as u64,
        )
        .await?;
    Ok(token)
}

/// 一次性消费刷新令牌（GETDEL），实现轮换并降低重放风险。
pub async fn consume_refresh_token(
    store: &impl TokenStore,
    token: &str,
) -> AuthResult<RefreshSession> {
    let value = store
        .get_del(&refresh_key(token))
        .await?
        .ok_or_else(|| AuthError::unauthorized("刷新令牌无效或已过期"))?;
    parse_refresh_value(&value)
}

pub async fn revoke_refresh_token(
    store: &impl TokenStore,
    token: &str,
) -> AuthResult<Option<RefreshSession>> {
    let value = store.get_del(&refresh_key(token)).await?;
    value.map(|value| parse_refresh_value(&value)).transpose()
}

pub fn refresh_token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

fn parse_refresh_value(value: &str) -> AuthResult<RefreshSession> {
    let (user_id, session_id) = value
        .split_once(':')
        .ok_or_else(|| AuthError::unauthorized("刷新令牌无效"))?;
    Ok(RefreshSession {
        user_id: Uuid::parse_str(user_id).map_err(|_| AuthError::unauthorized("刷新令牌无效"))?,
        session_id: Uuid::parse_str(session_id)
            .map_err(|_| AuthError::unauthorized("刷新令牌无效"))?,
    })
}

fn refresh_key(token: &str) -> String {
    format!("{REFRESH_TOKEN_PREFIX}{}", refresh_token_hash(token))
}

#[cfg(test)]
mod tests {
    use super::refresh_key;

    #[test]
    fn refresh_key_is_hashed_and_stable() {
        let first = refresh_key("token-value");
        let second = refresh_key("token-value");
        assert_eq!(first, second);
        assert!(first.starts_with("auth:refresh:"));
        assert!(!first.contains("token-value"));
        assert_ne!(first, refresh_key("another-token"));
    }
}
