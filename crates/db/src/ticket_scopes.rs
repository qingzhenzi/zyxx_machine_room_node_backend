//! 工单数据范围仓储（Wave 2 B）：`ticket_scopes` 的读写与当前用户区域集合。
//!
//! 语义：拥有 `ticket:scope:all` 的用户不受限制（在 API 层判定）；有 scope 行的用户
//! 只能访问这些区域的工单；没有 scope 行的用户默认不受限制（向后兼容）。

use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::ticket::TicketScopeUser;

use crate::error::{DbError, DbResult};

#[derive(Debug, FromRow)]
struct ScopeUserRow {
    user_id: Uuid,
    display_name: String,
    roles: Vec<String>,
    area_ids: Vec<Uuid>,
}

#[derive(Clone)]
pub struct TicketScopeRepo {
    db: PgPool,
}

impl TicketScopeRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 所有启用且拥有 `ticket:read` 的用户及其实时区域范围（含空行用户）。
    pub async fn list(&self) -> DbResult<Vec<TicketScopeUser>> {
        let rows = sqlx::query_as::<_, ScopeUserRow>(
            "SELECT u.id AS user_id,
                    u.display_name,
                    COALESCE(array_agg(DISTINCT r.code) FILTER (WHERE r.code IS NOT NULL), '{}')
                        AS roles,
                    COALESCE(array_agg(DISTINCT ts.area_id) FILTER (WHERE ts.area_id IS NOT NULL), '{}')
                        AS area_ids
               FROM users u
               JOIN user_roles ur ON ur.user_id = u.id
               JOIN roles r ON r.id = ur.role_id
               JOIN role_permissions rp ON rp.role_id = r.id
               JOIN permissions p ON p.id = rp.permission_id AND p.code = 'ticket:read'
               LEFT JOIN ticket_scopes ts ON ts.user_id = u.id
              WHERE u.is_active
              GROUP BY u.id, u.display_name
              ORDER BY u.display_name",
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| TicketScopeUser {
                user_id: row.user_id,
                display_name: row.display_name,
                roles: row.roles,
                area_ids: row.area_ids,
            })
            .collect())
    }

    /// 用户被授权的区域 id（空 = 未配置，调用方按“不限制”处理）。
    pub async fn area_ids_for_user(&self, user_id: Uuid) -> DbResult<Vec<Uuid>> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT area_id FROM ticket_scopes WHERE user_id = $1 ORDER BY area_id",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        Ok(ids)
    }

    /// 整体替换某用户的区域范围；区域必须存在。
    pub async fn replace(&self, user_id: Uuid, area_ids: &[Uuid]) -> DbResult<Vec<Uuid>> {
        let mut area_ids = area_ids.to_vec();
        area_ids.sort_unstable();
        area_ids.dedup();

        let mut tx = self.db.begin().await?;
        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            tx.rollback().await?;
            return Err(DbError::not_found("用户不存在"));
        }
        if !area_ids.is_empty() {
            let found = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM areas WHERE id = ANY($1::uuid[])",
            )
            .bind(&area_ids)
            .fetch_one(&mut *tx)
            .await?;
            if found != area_ids.len() as i64 {
                tx.rollback().await?;
                return Err(DbError::not_found("包含不存在的区域"));
            }
        }

        sqlx::query("DELETE FROM ticket_scopes WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        if !area_ids.is_empty() {
            sqlx::query(
                "INSERT INTO ticket_scopes (user_id, area_id)
                 SELECT $1, unnest($2::uuid[])",
            )
            .bind(user_id)
            .bind(&area_ids)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(area_ids)
    }
}
