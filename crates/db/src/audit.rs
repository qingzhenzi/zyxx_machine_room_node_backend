use sqlx::PgPool;
use uuid::Uuid;

use qe_domain::models::{AuditLog, Page};

use crate::{error::DbResult, rows::AuditLogRow};

#[derive(Clone)]
pub struct AuditLogRepo {
    db: PgPool,
}

impl AuditLogRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn record(
        &self,
        actor_id: Uuid,
        action: &str,
        resource_type: &str,
        resource_id: Option<&str>,
        detail: serde_json::Value,
        ip: Option<&str>,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO audit_logs
                (actor_id, actor_username, actor_display_name, action, resource_type, resource_id, detail, ip)
             SELECT u.id, u.username, u.display_name, $2, $3, $4, $5, $6
             FROM users u
             WHERE u.id = $1",
        )
        .bind(actor_id)
        .bind(action)
        .bind(resource_type)
        .bind(resource_id)
        .bind(detail)
        .bind(ip)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn list(
        &self,
        action: Option<&str>,
        resource_type: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<AuditLog>> {
        let offset = (page - 1) * page_size;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM audit_logs
             WHERE ($1::text IS NULL OR action = $1)
               AND ($2::text IS NULL OR resource_type = $2)",
        )
        .bind(action)
        .bind(resource_type)
        .fetch_one(&self.db)
        .await?;

        let rows = sqlx::query_as::<_, AuditLogRow>(
            "SELECT id, actor_id, actor_username, actor_display_name,
                    action, resource_type, resource_id, detail, ip, created_at
             FROM audit_logs
             WHERE ($1::text IS NULL OR action = $1)
               AND ($2::text IS NULL OR resource_type = $2)
             ORDER BY created_at DESC
             LIMIT $3 OFFSET $4",
        )
        .bind(action)
        .bind(resource_type)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.db)
        .await?;

        Ok(Page::new(
            rows.into_iter().map(AuditLog::from).collect(),
            total,
            page,
            page_size,
        ))
    }
}
