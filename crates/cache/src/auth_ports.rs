//! `qe-auth` 的 Redis 端口实现。
//!
//! trait 在 `qe-auth`、`Cache` 在本 crate，只有本 crate 能写这些 impl。

use qe_auth::{AuthError, AuthResult};

use crate::{cache::Cache, error::CacheError};

fn as_auth_error(error: CacheError) -> AuthError {
    AuthError::internal(error.to_string())
}

impl qe_auth::session::TokenStore for Cache {
    async fn set_ex(&self, key: &str, value: &str, ttl_secs: u64) -> AuthResult<()> {
        Cache::set_ex(self, key, value, ttl_secs)
            .await
            .map_err(as_auth_error)
    }

    async fn get_del(&self, key: &str) -> AuthResult<Option<String>> {
        Cache::get_del(self, key).await.map_err(as_auth_error)
    }

    async fn del(&self, key: &str) -> AuthResult<()> {
        Cache::del(self, key).await.map_err(as_auth_error)?;
        Ok(())
    }
}

impl qe_auth::permission::PermissionCache for Cache {
    async fn rbac_version(&self) -> AuthResult<i64> {
        Cache::rbac_version(self).await.map_err(as_auth_error)
    }

    async fn cached_permissions(&self, key: &str) -> AuthResult<Option<Vec<String>>> {
        Cache::get_json(self, key).await.map_err(as_auth_error)
    }

    async fn store_permissions(
        &self,
        key: &str,
        permissions: &[String],
        ttl_secs: u64,
    ) -> AuthResult<()> {
        Cache::set_json(self, key, permissions, ttl_secs)
            .await
            .map_err(as_auth_error)
    }

    async fn bump_rbac_version(&self) -> AuthResult<i64> {
        Cache::bump_rbac_version(self).await.map_err(as_auth_error)
    }
}
