use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::models::{Page, User, UserSummary},
    error::{AppError, AppResult},
};

#[derive(Clone)]
pub struct UserRepo {
    db: PgPool,
}

impl UserRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn find_by_id(&self, id: Uuid) -> AppResult<Option<User>> {
        Ok(
            sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.db)
                .await?,
        )
    }

    pub async fn find_by_username(&self, username: &str) -> AppResult<Option<User>> {
        Ok(
            sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = $1")
                .bind(username)
                .fetch_optional(&self.db)
                .await?,
        )
    }

    pub async fn list(
        &self,
        page: i64,
        page_size: i64,
        keyword: &str,
    ) -> AppResult<Page<UserSummary>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users u
             WHERE ($1 = ''
                    OR u.username ILIKE '%' || $1 || '%'
                    OR u.display_name ILIKE '%' || $1 || '%')",
        )
        .bind(keyword)
        .fetch_one(&self.db)
        .await?;

        let items = sqlx::query_as::<_, UserSummary>(
            "SELECT u.id,
                    u.username,
                    u.email,
                    u.display_name,
                    u.is_active,
                    u.created_at,
                    u.updated_at,
                    COALESCE(
                        array_agg(r.code ORDER BY r.code)
                            FILTER (WHERE r.code IS NOT NULL),
                        '{}'
                    ) AS roles
             FROM users u
             LEFT JOIN user_roles ur ON ur.user_id = u.id
             LEFT JOIN roles r ON r.id = ur.role_id
             WHERE ($3 = ''
                    OR u.username ILIKE '%' || $3 || '%'
                    OR u.display_name ILIKE '%' || $3 || '%')
             GROUP BY u.id
             ORDER BY u.created_at DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(page_size)
        .bind(offset)
        .bind(keyword)
        .fetch_all(&self.db)
        .await?;

        Ok(Page::new(items, total, page, page_size))
    }

    pub async fn find_summary(&self, id: Uuid) -> AppResult<Option<UserSummary>> {
        Ok(sqlx::query_as::<_, UserSummary>(
            "SELECT u.id,
                    u.username,
                    u.email,
                    u.display_name,
                    u.is_active,
                    u.created_at,
                    u.updated_at,
                    COALESCE(
                        array_agg(r.code ORDER BY r.code)
                            FILTER (WHERE r.code IS NOT NULL),
                        '{}'
                    ) AS roles
             FROM users u
             LEFT JOIN user_roles ur ON ur.user_id = u.id
             LEFT JOIN roles r ON r.id = ur.role_id
             WHERE u.id = $1
             GROUP BY u.id",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?)
    }

    pub async fn create_with_roles(
        &self,
        username: &str,
        email: Option<&str>,
        display_name: &str,
        password_hash: &str,
        role_codes: &[String],
    ) -> AppResult<User> {
        let mut transaction = self.db.begin().await?;

        let user = sqlx::query_as::<_, User>(
            "INSERT INTO users (username, email, display_name, password_hash)
             VALUES ($1, $2, $3, $4)
             RETURNING *",
        )
        .bind(username)
        .bind(email)
        .bind(display_name)
        .bind(password_hash)
        .fetch_one(&mut *transaction)
        .await?;

        if !role_codes.is_empty() {
            let inserted = sqlx::query(
                "INSERT INTO user_roles (user_id, role_id)
                 SELECT $1, id FROM roles WHERE code = ANY($2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(user.id)
            .bind(role_codes.to_vec())
            .execute(&mut *transaction)
            .await?
            .rows_affected();

            if inserted != role_codes.len() as u64 {
                transaction.rollback().await?;
                return Err(AppError::bad_request("包含不存在的角色"));
            }
        }

        transaction.commit().await?;
        Ok(user)
    }

    pub async fn update(
        &self,
        id: Uuid,
        email_change: Option<Option<String>>,
        display_name: Option<String>,
        password_hash: Option<String>,
        is_active: Option<bool>,
    ) -> AppResult<Option<User>> {
        let (email_changed, email_value) = match email_change {
            Some(value) => (true, value),
            None => (false, None),
        };

        Ok(sqlx::query_as::<_, User>(
            "UPDATE users SET
                email = CASE WHEN $2 THEN $3 ELSE email END,
                display_name = COALESCE($4, display_name),
                password_hash = COALESCE($5, password_hash),
                is_active = COALESCE($6, is_active)
             WHERE id = $1
             RETURNING *",
        )
        .bind(id)
        .bind(email_changed)
        .bind(email_value)
        .bind(display_name)
        .bind(password_hash)
        .bind(is_active)
        .fetch_optional(&self.db)
        .await?)
    }

    pub async fn roles_of(&self, user_id: Uuid) -> AppResult<Vec<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT r.code
             FROM user_roles ur
             JOIN roles r ON r.id = ur.role_id
             WHERE ur.user_id = $1
             ORDER BY r.code",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?)
    }

    pub async fn permissions_of(&self, user_id: Uuid) -> AppResult<Vec<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT p.code
             FROM user_roles ur
             JOIN role_permissions rp ON rp.role_id = ur.role_id
             JOIN permissions p ON p.id = rp.permission_id
             JOIN users u ON u.id = ur.user_id
             WHERE ur.user_id = $1 AND u.is_active
             ORDER BY p.code",
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?)
    }

    /// 全量替换用户角色。调用方负责校验“最后一个管理员”等业务规则。
    pub async fn set_roles(&self, user_id: Uuid, role_codes: &[String]) -> AppResult<()> {
        let mut transaction = self.db.begin().await?;

        sqlx::query("DELETE FROM user_roles WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;

        if !role_codes.is_empty() {
            let inserted = sqlx::query(
                "INSERT INTO user_roles (user_id, role_id)
                 SELECT $1, id FROM roles WHERE code = ANY($2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(user_id)
            .bind(role_codes.to_vec())
            .execute(&mut *transaction)
            .await?
            .rows_affected();

            if inserted != role_codes.len() as u64 {
                transaction.rollback().await?;
                return Err(AppError::bad_request("包含不存在的角色"));
            }
        }

        transaction.commit().await?;
        Ok(())
    }

    /// 统计仍处于启用状态、且拥有指定角色的用户数。
    pub async fn count_active_with_role(&self, role_code: &str) -> AppResult<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT count(*)
             FROM user_roles ur
             JOIN roles r ON r.id = ur.role_id
             JOIN users u ON u.id = ur.user_id
             WHERE r.code = $1 AND u.is_active",
        )
        .bind(role_code)
        .fetch_one(&self.db)
        .await?)
    }
}
