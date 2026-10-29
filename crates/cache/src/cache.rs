use redis::{AsyncCommands, aio::ConnectionManager};
use serde::{Serialize, de::DeserializeOwned};

use crate::error::CacheResult;

const RBAC_VERSION_KEY: &str = "rbac:version";

/// Redis 缓存的薄封装。
///
/// `ConnectionManager` 内部复用连接并自动重连，克隆成本很低。
#[derive(Clone)]
pub struct Cache {
    connection: ConnectionManager,
}

impl Cache {
    pub async fn connect(redis_url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let connection = client.get_connection_manager().await?;
        Ok(Self { connection })
    }

    pub async fn ping(&self) -> CacheResult<()> {
        let mut connection = self.connection.clone();
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await?;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> CacheResult<Option<String>> {
        let mut connection = self.connection.clone();
        let value: Option<String> = connection.get(key).await?;
        Ok(value)
    }

    pub async fn set_ex(&self, key: &str, value: &str, ttl_secs: u64) -> CacheResult<()> {
        let mut connection = self.connection.clone();
        let _: () = connection.set_ex(key, value, ttl_secs.max(1)).await?;
        Ok(())
    }

    pub async fn del(&self, key: &str) -> CacheResult<bool> {
        let mut connection = self.connection.clone();
        let removed: i64 = connection.del(key).await?;
        Ok(removed > 0)
    }

    /// 原子地读取并删除，用于 refresh token 的一次性消费。
    pub async fn get_del(&self, key: &str) -> CacheResult<Option<String>> {
        let mut connection = self.connection.clone();
        let value: Option<String> = redis::cmd("GETDEL")
            .arg(key)
            .query_async(&mut connection)
            .await?;
        Ok(value)
    }

    pub async fn incr(&self, key: &str) -> CacheResult<i64> {
        let mut connection = self.connection.clone();
        let value: i64 = connection.incr(key, 1_i64).await?;
        Ok(value)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> CacheResult<Option<T>> {
        let Some(raw) = self.get(key).await? else {
            return Ok(None);
        };
        let value = serde_json::from_str(&raw)?;
        Ok(Some(value))
    }

    pub async fn set_json<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> CacheResult<()> {
        let raw = serde_json::to_string(value)?;
        self.set_ex(key, &raw, ttl_secs).await
    }

    /// 当前 RBAC 缓存代际。权限变更时递增，旧缓存键自然失效。
    pub async fn rbac_version(&self) -> CacheResult<i64> {
        let mut connection = self.connection.clone();
        let version: Option<i64> = connection.get(RBAC_VERSION_KEY).await?;
        Ok(version.unwrap_or(0))
    }

    pub async fn bump_rbac_version(&self) -> CacheResult<i64> {
        let mut connection = self.connection.clone();
        let version: i64 = redis::cmd("INCR")
            .arg(RBAC_VERSION_KEY)
            .query_async(&mut connection)
            .await?;
        Ok(version)
    }
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Cache").finish_non_exhaustive()
    }
}
