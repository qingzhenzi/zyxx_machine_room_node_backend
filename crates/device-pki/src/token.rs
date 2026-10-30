//! 一次性 bootstrap token（契约第 3 节）。
//!
//! 明文只在生成时返回一次，库中仅存 SHA-256 hash。

use crate::error::{Error, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    /// 即时批准：服务端已备好节点信息，注册后直接签发
    Auto,
    /// 人工批准：客户端上报注册信息，管理员审批后签发
    Approval,
}

impl TokenKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenKind::Auto => "auto",
            TokenKind::Approval => "approval",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(TokenKind::Auto),
            "approval" => Ok(TokenKind::Approval),
            other => Err(Error::BadRequest(format!("unknown token kind: {other}"))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IssuedToken {
    /// 明文 token，只在这一次返回
    pub token: String,
    pub token_hash: [u8; 32],
    pub kind: TokenKind,
    pub device_slot: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenRow {
    pub token_hash: Vec<u8>,
    pub kind: String,
    pub device_slot: String,
    pub used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub fn hash_token(token: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().into()
}

/// 生成一次性 token（默认 TTL 15 分钟）。
pub async fn issue(
    db: &PgPool,
    kind: TokenKind,
    device_slot: &str,
    created_by: &str,
    ttl: chrono::Duration,
) -> Result<IssuedToken> {
    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    let token = hex::encode(raw);
    let token_hash = hash_token(&token);
    let expires_at = chrono::Utc::now() + ttl;

    sqlx::query(
        "INSERT INTO device_bootstrap_token (token_hash, kind, device_slot, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(token_hash.as_slice())
    .bind(kind.as_str())
    .bind(device_slot)
    .bind(created_by)
    .bind(expires_at)
    .execute(db)
    .await?;

    Ok(IssuedToken {
        token,
        token_hash,
        kind,
        device_slot: device_slot.to_owned(),
        expires_at,
    })
}

/// 只读校验：token 是否存在、未用过、未过期、slot 是否匹配。
/// 真正的"一次性"由 `consume` 在签发事务里用条件 UPDATE 仲裁。
pub async fn verify(
    db: &PgPool,
    token: &str,
    device_slot: &str,
) -> Result<(TokenRow, TokenKind)> {
    let token_hash = hash_token(token);
    let row: Option<TokenRow> = sqlx::query_as(
        "SELECT token_hash, kind, device_slot, used_at, expires_at
         FROM device_bootstrap_token WHERE token_hash = $1",
    )
    .bind(token_hash.as_slice())
    .fetch_optional(db)
    .await?;

    let row = row.ok_or(Error::TokenInvalid)?;
    if row.used_at.is_some() {
        return Err(Error::TokenInvalid);
    }
    if row.expires_at < chrono::Utc::now() {
        return Err(Error::TokenInvalid);
    }
    if row.device_slot != device_slot {
        return Err(Error::TokenInvalid);
    }
    let kind = TokenKind::parse(&row.kind)?;
    Ok((row, kind))
}

/// 消费 token。返回 true 表示本次调用赢得了仲裁（此前未使用）。
pub async fn consume<'e, E>(executor: E, token_hash: &[u8]) -> Result<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query(
        "UPDATE device_bootstrap_token SET used_at = now()
         WHERE token_hash = $1 AND used_at IS NULL",
    )
    .bind(token_hash)
    .execute(executor)
    .await?;
    Ok(res.rows_affected() == 1)
}

/// 生成注册链接
pub fn enroll_link(endpoint: &str, token: &str, slot: &str) -> String {
    format!("{endpoint}/enroll?token={token}&slot={slot}")
}

/// 解析注册链接中的 token 与 slot
pub fn parse_enroll_link(link: &str) -> Result<(String, String)> {
    let (_scheme, rest) = link
        .split_once("://")
        .ok_or_else(|| Error::BadRequest("link missing scheme".into()))?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => return Err(Error::BadRequest("link missing path".into())),
    };
    let query = path
        .split_once('?')
        .map(|(_, q)| q)
        .ok_or_else(|| Error::BadRequest("link missing query".into()))?;
    let mut token = None;
    let mut slot = None;
    for pair in query.split('&') {
        let (k, v) = match pair.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        match k {
            "token" => token = Some(v.to_owned()),
            "slot" => slot = Some(v.to_owned()),
            _ => {}
        }
    }
    let _ = authority;
    match (token, slot) {
        (Some(t), Some(s)) => Ok((t, s)),
        _ => Err(Error::BadRequest("link missing token or slot".into())),
    }
}

pub fn new_challenge_id() -> Uuid {
    Uuid::new_v4()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_sized() {
        let a = hash_token("abc");
        let b = hash_token("abc");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert_ne!(a, hash_token("abd"));
    }

    #[test]
    fn link_roundtrip() {
        let link = enroll_link("https://node.example:4440", "deadbeef", "slot-1");
        let (t, s) = parse_enroll_link(&link).unwrap();
        assert_eq!(t, "deadbeef");
        assert_eq!(s, "slot-1");
    }

    #[test]
    fn link_missing_parts_rejected() {
        assert!(parse_enroll_link("https://node/enroll?token=x").is_err());
        assert!(parse_enroll_link("no-scheme").is_err());
    }

    #[test]
    fn token_kind_parse() {
        assert_eq!(TokenKind::parse("auto").unwrap(), TokenKind::Auto);
        assert_eq!(TokenKind::parse("approval").unwrap(), TokenKind::Approval);
        assert!(TokenKind::parse("other").is_err());
    }
}
