use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use qe_domain::models::UserSession;

use crate::{error::DbResult, rows::UserSessionRow};

#[derive(Clone)]
pub struct SecurityRepo {
    db: PgPool,
}

impl SecurityRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn create_password_reset(
        &self,
        user_id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
             VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn consume_password_reset(&self, token_hash: &str) -> DbResult<Option<Uuid>> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE password_reset_tokens
             SET used_at = now()
             WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
             RETURNING user_id",
        )
        .bind(token_hash)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(|value| value.0))
    }

    pub async fn create_email_verification(
        &self,
        user_id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO email_verification_tokens (user_id, token_hash, expires_at)
             VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn consume_email_verification(&self, token_hash: &str) -> DbResult<Option<Uuid>> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE email_verification_tokens
             SET used_at = now()
             WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
             RETURNING user_id",
        )
        .bind(token_hash)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(|value| value.0))
    }

    pub async fn create_session(
        &self,
        user_id: Uuid,
        token_hash: &str,
        user_agent: &str,
        ip: Option<&str>,
    ) -> DbResult<Uuid> {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO user_sessions (user_id, token_hash, user_agent, ip)
             VALUES ($1, $2, $3, $4)
             RETURNING id",
        )
        .bind(user_id)
        .bind(token_hash)
        .bind(user_agent)
        .bind(ip)
        .fetch_one(&self.db)
        .await?;
        Ok(id)
    }

    pub async fn create_session_with_id(
        &self,
        id: Uuid,
        user_id: Uuid,
        token_hash: &str,
        user_agent: &str,
        ip: Option<&str>,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO user_sessions (id, user_id, token_hash, user_agent, ip)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(user_id)
        .bind(token_hash)
        .bind(user_agent)
        .bind(ip)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn rotate_session_token(&self, id: Uuid, token_hash: &str) -> DbResult<bool> {
        let updated = sqlx::query(
            "UPDATE user_sessions
             SET token_hash = $2, last_seen_at = now()
             WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(token_hash)
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    pub async fn touch_session(&self, id: Uuid) -> DbResult<bool> {
        let updated = sqlx::query(
            "UPDATE user_sessions SET last_seen_at = now() WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    pub async fn session_active(&self, id: Uuid) -> DbResult<bool> {
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM user_sessions WHERE id = $1 AND revoked_at IS NULL)",
        )
        .bind(id)
        .fetch_one(&self.db)
        .await?)
    }

    pub async fn list_sessions(&self, user_id: Uuid) -> DbResult<Vec<UserSession>> {
        let rows = sqlx::query_as::<_, UserSessionRow>(
            "SELECT id, user_id, user_agent, ip, created_at, last_seen_at, revoked_at
             FROM user_sessions
             WHERE user_id = $1 AND revoked_at IS NULL
             ORDER BY last_seen_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(UserSession::from).collect())
    }

    pub async fn revoke_session(&self, user_id: Uuid, id: Uuid) -> DbResult<bool> {
        let updated = sqlx::query(
            "UPDATE user_sessions SET revoked_at = now()
             WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    pub async fn revoke_all_sessions(&self, user_id: Uuid) -> DbResult<u64> {
        Ok(sqlx::query(
            "UPDATE user_sessions SET revoked_at = now()
             WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .execute(&self.db)
        .await?
        .rows_affected())
    }
}
