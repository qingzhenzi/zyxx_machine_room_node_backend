use uuid::Uuid;

use crate::error::{AuthError, AuthResult};

/// 权限缓存有效期。业务改了角色/权限会通过代际（`rbac:version`）立即失效，
/// 这个 TTL 只是 Redis 故障或代际异常时的兜底。
pub const PERMISSION_CACHE_TTL_SECS: u64 = 300;

/// 权限判定的存储端口：按 RBAC 代际分 key 的读缓存与回写。
pub trait PermissionCache {
    async fn rbac_version(&self) -> AuthResult<i64>;
    async fn cached_permissions(&self, key: &str) -> AuthResult<Option<Vec<String>>>;
    async fn store_permissions(
        &self,
        key: &str,
        permissions: &[String],
        ttl_secs: u64,
    ) -> AuthResult<()>;
    async fn bump_rbac_version(&self) -> AuthResult<i64>;
}

/// 权限的事实来源（数据库）。
pub trait PermissionSource {
    async fn permissions_of(&self, user_id: Uuid) -> AuthResult<Vec<String>>;
}

/// 判断用户是否拥有某个权限（精确匹配，不做前缀/通配）。
pub fn has_permission(granted: &[String], required: &str) -> bool {
    granted
        .iter()
        .any(|permission| permission.as_str() == required)
}

/// 判断用户是否拥有任意一个所需权限。
pub fn has_any_permission(granted: &[String], required: &[&str]) -> bool {
    required
        .iter()
        .any(|permission| has_permission(granted, permission))
}

/// 读取用户权限：先看缓存，Redis 故障时静默回退数据库。
pub async fn permissions_of(
    cache: &impl PermissionCache,
    source: &impl PermissionSource,
    user_id: Uuid,
) -> AuthResult<Vec<String>> {
    let version = match cache.rbac_version().await {
        Ok(version) => version,
        Err(error) => {
            tracing::warn!(%error, "读取 RBAC 版本失败，回退到数据库");
            0
        }
    };
    let cache_key = format!("rbac:v{version}:user:{user_id}:perms");

    match cache.cached_permissions(&cache_key).await {
        Ok(Some(permissions)) => return Ok(permissions),
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "读取权限缓存失败，回退到数据库"),
    }

    let permissions = source.permissions_of(user_id).await?;
    if let Err(error) = cache
        .store_permissions(&cache_key, &permissions, PERMISSION_CACHE_TTL_SECS)
        .await
    {
        tracing::warn!(%error, "写入权限缓存失败");
    }
    Ok(permissions)
}

pub async fn require_permission(
    cache: &impl PermissionCache,
    source: &impl PermissionSource,
    user_id: Uuid,
    permission: &str,
) -> AuthResult<()> {
    let granted = permissions_of(cache, source, user_id).await?;
    if has_permission(&granted, permission) {
        Ok(())
    } else {
        Err(AuthError::forbidden(format!("缺少权限：{permission}")))
    }
}

pub async fn require_any_permission(
    cache: &impl PermissionCache,
    source: &impl PermissionSource,
    user_id: Uuid,
    permissions: &[&str],
) -> AuthResult<()> {
    let granted = permissions_of(cache, source, user_id).await?;
    if has_any_permission(&granted, permissions) {
        Ok(())
    } else {
        Err(AuthError::forbidden("缺少访问该资源所需的权限"))
    }
}

/// 角色/权限/用户角色发生变化后调用，使所有权限缓存失效。
pub async fn bump_permission_version(cache: &impl PermissionCache) -> AuthResult<i64> {
    cache.bump_rbac_version().await
}

#[cfg(test)]
mod tests {
    use super::{has_any_permission, has_permission};

    fn codes(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn permission_matching_is_exact() {
        let granted = codes(&["announcement:read"]);
        assert!(has_permission(&granted, "announcement:read"));
        assert!(!has_permission(&granted, "announcement:write"));
        assert!(!has_permission(
            &granted,
            "announcement:read:any-extra-suffix"
        ));
    }

    #[test]
    fn any_permission_matches_one() {
        let granted = codes(&["user:manage"]);
        assert!(has_any_permission(
            &granted,
            &["role:manage", "user:manage"]
        ));
        assert!(!has_any_permission(
            &granted,
            &["role:manage", "announcement:delete"]
        ));
    }
}
