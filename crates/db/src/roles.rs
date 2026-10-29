use sqlx::PgPool;
use uuid::Uuid;

use qe_domain::models::{Permission, Role, RoleSummary};

use crate::{
    error::{DbError, DbResult},
    rows::{PermissionRow, RoleRow, RoleSummaryRow},
};

#[derive(Clone)]
pub struct RoleRepo {
    db: PgPool,
}

impl RoleRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<RoleSummary>> {
        let rows = sqlx::query_as::<_, RoleSummaryRow>(
            "SELECT r.id,
                    r.code,
                    r.name,
                    r.description,
                    r.is_system,
                    r.created_at,
                    r.updated_at,
                    COALESCE(
                        array_agg(p.code ORDER BY p.code)
                            FILTER (WHERE p.code IS NOT NULL),
                        '{}'
                    ) AS permissions
             FROM roles r
             LEFT JOIN role_permissions rp ON rp.role_id = r.id
             LEFT JOIN permissions p ON p.id = rp.permission_id
             GROUP BY r.id
             ORDER BY r.is_system DESC, r.code",
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(RoleSummary::from).collect())
    }

    pub async fn find_summary(&self, id: Uuid) -> DbResult<Option<RoleSummary>> {
        let row = sqlx::query_as::<_, RoleSummaryRow>(
            "SELECT r.id,
                    r.code,
                    r.name,
                    r.description,
                    r.is_system,
                    r.created_at,
                    r.updated_at,
                    COALESCE(
                        array_agg(p.code ORDER BY p.code)
                            FILTER (WHERE p.code IS NOT NULL),
                        '{}'
                    ) AS permissions
             FROM roles r
             LEFT JOIN role_permissions rp ON rp.role_id = r.id
             LEFT JOIN permissions p ON p.id = rp.permission_id
             WHERE r.id = $1
             GROUP BY r.id",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(RoleSummary::from))
    }

    pub async fn find_by_code(&self, code: &str) -> DbResult<Option<Role>> {
        let row = sqlx::query_as::<_, RoleRow>("SELECT * FROM roles WHERE code = $1")
            .bind(code)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Role::from))
    }

    pub async fn find_by_id(&self, id: Uuid) -> DbResult<Option<Role>> {
        let row = sqlx::query_as::<_, RoleRow>("SELECT * FROM roles WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Role::from))
    }

    pub async fn list_permissions(&self) -> DbResult<Vec<Permission>> {
        let rows = sqlx::query_as::<_, PermissionRow>("SELECT * FROM permissions ORDER BY code")
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(Permission::from).collect())
    }

    pub async fn create(
        &self,
        code: &str,
        name: &str,
        description: &str,
        permission_codes: &[String],
    ) -> DbResult<RoleSummary> {
        let mut transaction = self.db.begin().await?;

        let row = sqlx::query_as::<_, RoleRow>(
            "INSERT INTO roles (code, name, description, is_system)
             VALUES ($1, $2, $3, FALSE)
             RETURNING *",
        )
        .bind(code)
        .bind(name)
        .bind(description)
        .fetch_one(&mut *transaction)
        .await?;
        let role = Role::from(row);

        if !permission_codes.is_empty() {
            let inserted = sqlx::query(
                "INSERT INTO role_permissions (role_id, permission_id)
                 SELECT $1, id FROM permissions WHERE code = ANY($2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(role.id)
            .bind(permission_codes.to_vec())
            .execute(&mut *transaction)
            .await?
            .rows_affected();

            if inserted != permission_codes.len() as u64 {
                transaction.rollback().await?;
                return Err(DbError::bad_request("包含不存在的权限"));
            }
        }

        transaction.commit().await?;
        self.find_summary(role.id)
            .await?
            .ok_or_else(|| DbError::internal("创建角色后无法读取角色信息"))
    }

    pub async fn set_permissions(
        &self,
        role_id: Uuid,
        permission_codes: &[String],
    ) -> DbResult<()> {
        let mut transaction = self.db.begin().await?;

        sqlx::query("DELETE FROM role_permissions WHERE role_id = $1")
            .bind(role_id)
            .execute(&mut *transaction)
            .await?;

        if !permission_codes.is_empty() {
            let inserted = sqlx::query(
                "INSERT INTO role_permissions (role_id, permission_id)
                 SELECT $1, id FROM permissions WHERE code = ANY($2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(role_id)
            .bind(permission_codes.to_vec())
            .execute(&mut *transaction)
            .await?
            .rows_affected();

            if inserted != permission_codes.len() as u64 {
                transaction.rollback().await?;
                return Err(DbError::bad_request("包含不存在的权限"));
            }
        }

        transaction.commit().await?;
        Ok(())
    }

    /// 返回是否真的删除了记录。系统角色由调用方拦截。
    pub async fn delete(&self, role_id: Uuid) -> DbResult<bool> {
        let deleted = sqlx::query("DELETE FROM roles WHERE id = $1 AND is_system = FALSE")
            .bind(role_id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    /// 保证系统管理员角色始终拥有当前全部权限（包括后续迁移新增的权限）。
    pub async fn grant_all_permissions_to_admin(&self) -> DbResult<u64> {
        let inserted = sqlx::query(
            "INSERT INTO role_permissions (role_id, permission_id)
             SELECT r.id, p.id
             FROM roles r
             CROSS JOIN permissions p
             WHERE r.code = 'admin'
             ON CONFLICT DO NOTHING",
        )
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(inserted)
    }
}
