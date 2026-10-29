use redis::{AsyncCommands, aio::ConnectionManager};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::error::CacheResult;

const RBAC_VERSION_KEY: &str = "rbac:version";
const ANNOUNCEMENT_VERSION_KEY: &str = "announcements:version";

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

    /// 公告缓存代际。公告变更时递增，旧键自然失效。
    pub async fn announcement_version(&self) -> CacheResult<i64> {
        let mut connection = self.connection.clone();
        let version: Option<i64> = connection.get(ANNOUNCEMENT_VERSION_KEY).await?;
        Ok(version.unwrap_or(0))
    }

    pub async fn bump_announcement_version(&self) -> CacheResult<i64> {
        let mut connection = self.connection.clone();
        let version: i64 = redis::cmd("INCR")
            .arg(ANNOUNCEMENT_VERSION_KEY)
            .query_async(&mut connection)
            .await?;
        Ok(version)
    }

    /// 返回登录锁定剩余秒数；未锁定时为 None。
    pub async fn login_retry_after(&self, username: &str) -> CacheResult<Option<u64>> {
        let mut connection = self.connection.clone();
        let ttl: i64 = redis::cmd("TTL")
            .arg(login_lock_key(username))
            .query_async(&mut connection)
            .await?;
        Ok((ttl > 0).then_some(ttl as u64))
    }

    /// 原子记录一次失败，达到阈值后写入锁定键。返回当前窗口内失败次数。
    pub async fn record_login_failure(
        &self,
        username: &str,
        window_secs: u64,
        max_attempts: u32,
        lock_secs: u64,
    ) -> CacheResult<u64> {
        let mut connection = self.connection.clone();
        let script = redis::Script::new(
            r#"
            local count = redis.call('INCR', KEYS[1])
            if count == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end
            if count >= tonumber(ARGV[2]) then
                redis.call('SET', KEYS[2], '1', 'EX', ARGV[3])
            end
            return count
            "#,
        );
        let count: u64 = script
            .key(login_fail_key(username))
            .key(login_lock_key(username))
            .arg(window_secs.max(1))
            .arg(max_attempts.max(1))
            .arg(lock_secs.max(1))
            .invoke_async(&mut connection)
            .await?;
        Ok(count)
    }

    /// 登录成功后清理失败窗口和锁定。
    pub async fn clear_login_failures(&self, username: &str) -> CacheResult<()> {
        let mut connection = self.connection.clone();
        let _: i64 = connection
            .del(&[login_fail_key(username), login_lock_key(username)])
            .await?;
        Ok(())
    }

    /// 热路径会话缓存：存在表示会话仍有效。撤销时必须删除。
    pub async fn mark_session_active(&self, session_id: Uuid, ttl_secs: u64) -> CacheResult<()> {
        let mut connection = self.connection.clone();
        let _: () = connection
            .set_ex(session_key(session_id), "1", ttl_secs.max(1))
            .await?;
        Ok(())
    }

    pub async fn cached_session_active(&self, session_id: Uuid) -> CacheResult<Option<bool>> {
        let mut connection = self.connection.clone();
        let value: Option<String> = connection.get(session_key(session_id)).await?;
        Ok(value.map(|_| true))
    }

    pub async fn revoke_session_cache(&self, session_id: Uuid) -> CacheResult<()> {
        let mut connection = self.connection.clone();
        let _: i64 = connection.del(session_key(session_id)).await?;
        Ok(())
    }
}

fn login_fail_key(username: &str) -> String {
    format!("auth:login:fail:{username}")
}

fn login_lock_key(username: &str) -> String {
    format!("auth:login:lock:{username}")
}

fn session_key(session_id: Uuid) -> String {
    format!("auth:session:{session_id}")
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Cache").finish_non_exhaustive()
    }
}
