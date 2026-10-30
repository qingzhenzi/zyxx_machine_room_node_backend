//! `AppState` 级别的认证/授权装配。
//!
//! `qe-auth` 只包含算法与端口定义；Redis/PostgreSQL 端口实现在
//! `qe-cache::auth_ports` 与 `qe-db::auth_ports`。这里只保留需要 `AppState` 的
//! `permissions_of` / `profile_of` / `require_permission` 等胶水函数。

pub use qe_auth::{jwt, password, session};

/// 会话活跃缓存 TTL。撤销会话时删除缓存；短 TTL 同时限制
/// “DB 读到 active 后与撤销并发”可能造成的过期窗口。
pub const SESSION_ACTIVE_CACHE_TTL_SECS: u64 = 60;

use uuid::Uuid;

use crate::{
    domain::models::{User, UserProfile},
    error::{AppError, AppResult},
    state::AppState,
};

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
