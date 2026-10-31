//! Challenge 生命周期（契约第 4 节）。
//!
//! 挑战值绑定 `(token_hash, device_slot)`，短 TTL（默认 60s），一次性消费。
//! 目的是让每次注册的 PoP transcript 都不同，从而抵抗重放与预计算签名。

use crate::error::{Error, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

pub const CHALLENGE_TTL_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct IssuedChallenge {
    pub challenge_id: Uuid,
    pub challenge: [u8; 32],
    pub ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub challenge_id: Uuid,
    /// hex 编码
    pub challenge: String,
    pub ttl_secs: u64,
}

impl From<IssuedChallenge> for ChallengeResponse {
    fn from(c: IssuedChallenge) -> Self {
        Self {
            challenge_id: c.challenge_id,
            challenge: hex::encode(c.challenge),
            ttl_secs: c.ttl_secs,
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChallengeRow {
    pub challenge_id: Uuid,
    pub challenge: Vec<u8>,
    pub token_hash: Vec<u8>,
    pub device_slot: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub consumed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 签发挑战值，绑定 token 与 slot。
pub async fn issue(db: &PgPool, token_hash: &[u8], device_slot: &str) -> Result<IssuedChallenge> {
    let mut challenge = [0u8; 32];
    rand::rng().fill_bytes(&mut challenge);
    let challenge_id = Uuid::new_v4();
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(CHALLENGE_TTL_SECS as i64);

    sqlx::query(
        "INSERT INTO device_enroll_challenge
           (challenge_id, challenge, token_hash, device_slot, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(challenge_id)
    .bind(challenge.as_slice())
    .bind(token_hash)
    .bind(device_slot)
    .bind(expires_at)
    .execute(db)
    .await?;

    Ok(IssuedChallenge {
        challenge_id,
        challenge,
        ttl_secs: CHALLENGE_TTL_SECS,
    })
}

/// 校验挑战三元组绑定关系（不消费）。
pub async fn verify(
    db: &PgPool,
    challenge_id: Uuid,
    token_hash: &[u8],
    device_slot: &str,
) -> Result<ChallengeRow> {
    let row: Option<ChallengeRow> = sqlx::query_as(
        "SELECT challenge_id, challenge, token_hash, device_slot, expires_at, consumed_at
         FROM device_enroll_challenge WHERE challenge_id = $1",
    )
    .bind(challenge_id)
    .fetch_optional(db)
    .await?;

    let row = row.ok_or(Error::ChallengeMismatch)?;
    if row.consumed_at.is_some() {
        return Err(Error::ChallengeConsumed);
    }
    if row.expires_at < chrono::Utc::now() {
        return Err(Error::ChallengeExpired);
    }
    // 三元组必须完全匹配，否则同 slot 的另一台设备可以盗用挑战值
    if row.token_hash != token_hash || row.device_slot != device_slot {
        return Err(Error::ChallengeMismatch);
    }
    Ok(row)
}

/// 消费挑战。返回 true 表示本次调用赢得仲裁。
pub async fn consume<'e, E>(executor: E, challenge_id: Uuid) -> Result<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query(
        "UPDATE device_enroll_challenge SET consumed_at = now()
         WHERE challenge_id = $1 AND consumed_at IS NULL",
    )
    .bind(challenge_id)
    .execute(executor)
    .await?;
    Ok(res.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_hex_roundtrip() {
        let c = IssuedChallenge {
            challenge_id: Uuid::new_v4(),
            challenge: [7u8; 32],
            ttl_secs: CHALLENGE_TTL_SECS,
        };
        let resp: ChallengeResponse = c.into();
        assert_eq!(resp.challenge.len(), 64);
        let back = hex::decode(&resp.challenge).unwrap();
        assert_eq!(back, vec![7u8; 32]);
    }

    #[test]
    fn ttl_is_short() {
        // 短 TTL 是安全属性的一部分，防止挑战值长时间可被利用
        const {
            assert!(CHALLENGE_TTL_SECS <= 120);
        }
    }
}
