//! 认证 crate 的接入层。
//!
//! `qe-auth` 只包含算法与端口定义；这里负责把 Redis / PostgreSQL 实现接上去，
//! 并保留原来的 `authorization::*` 函数签名，让 handler 不必关心依赖从哪来。
//! 业务侧要看的是「谁在什么条件下能做什么」，所以 `profile_of`、
//! `bump_permission_version` 这类和 `AppState` 相关的胶水留在这里。

pub use qe_auth::{jwt, password, session};

use uuid::Uuid;

use crate::{
    domain::models::{User, UserProfile},
    error::{AppError, AppResult},
    infra::{cache::Cache, users::UserRepo},
    state::AppState,
};

fn as_auth_error(error: AppError) -> qe_auth::AuthError {
    qe_auth::AuthError::internal(error.to_string())
}

// 端口实现。用 `Cache::method(self, ..)` 的完全限定调用，避免与同名 trait 方法递归。

impl qe_auth::session::TokenStore for Cache {
    async fn set_ex(&self, key: &str, value: &str, ttl_secs: u64) -> qe_auth::AuthResult<()> {
        Cache::set_ex(self, key, value, ttl_secs)
            .await
            .map_err(as_auth_error)
    }

    async fn get_del(&self, key: &str) -> qe_auth::AuthResult<Option<String>> {
        Cache::get_del(self, key).await.map_err(as_auth_error)
    }

    async fn del(&self, key: &str) -> qe_auth::AuthResult<()> {
        Cache::del(self, key).await.map_err(as_auth_error)?;
        Ok(())
    }
}

impl qe_auth::permission::PermissionCache for Cache {
    async fn rbac_version(&self) -> qe_auth::AuthResult<i64> {
        Cache::rbac_version(self).await.map_err(as_auth_error)
    }

    async fn cached_permissions(&self, key: &str) -> qe_auth::AuthResult<Option<Vec<String>>> {
        Cache::get_json(self, key).await.map_err(as_auth_error)
    }

    async fn store_permissions(
        &self,
        key: &str,
        permissions: &[String],
        ttl_secs: u64,
    ) -> qe_auth::AuthResult<()> {
        Cache::set_json(self, key, permissions, ttl_secs)
            .await
            .map_err(as_auth_error)
    }

    async fn bump_rbac_version(&self) -> qe_auth::AuthResult<i64> {
        Cache::bump_rbac_version(self).await.map_err(as_auth_error)
    }
}

impl qe_auth::permission::PermissionSource for UserRepo {
    async fn permissions_of(&self, user_id: Uuid) -> qe_auth::AuthResult<Vec<String>> {
        UserRepo::permissions_of(self, user_id)
            .await
            .map_err(as_auth_error)
    }
}

pub mod authorization {
    use super::*;

    pub async fn permissions_of(state: &AppState, user_id: Uuid) -> AppResult<Vec<String>> {
        Ok(qe_auth::permission::permissions_of(&state.cache, &state.users, user_id).await?)
    }

    pub async fn require_permission(
        state: &AppState,
        user_id: Uuid,
        permission: &str,
    ) -> AppResult<()> {
        qe_auth::permission::require_permission(&state.cache, &state.users, user_id, permission)
            .await
            .map_err(AppError::from)
    }

    pub async fn require_any_permission(
        state: &AppState,
        user_id: Uuid,
        permissions: &[&str],
    ) -> AppResult<()> {
        qe_auth::permission::require_any_permission(
            &state.cache,
            &state.users,
            user_id,
            permissions,
        )
        .await
        .map_err(AppError::from)
    }

    /// 组装 API 返回的用户档案。`User` 是业务实体，所以这一步留在后端。
    pub async fn profile_of(state: &AppState, user: &User) -> AppResult<UserProfile> {
        let roles = state.users.roles_of(user.id).await?;
        let permissions = permissions_of(state, user.id).await?;
        Ok(UserProfile {
            id: user.id,
            username: user.username.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            is_active: user.is_active,
            roles,
            permissions,
            created_at: user.created_at,
        })
    }

    /// 角色/权限/用户角色发生变化后调用，使所有权限缓存失效。
    pub async fn bump_permission_version(state: &AppState) {
        if let Err(error) = qe_auth::permission::bump_permission_version(&state.cache).await {
            tracing::warn!(%error, "递增 RBAC 缓存版本失败，权限缓存可能短暂过期");
        }
    }
}
