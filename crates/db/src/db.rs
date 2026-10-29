use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

const MIN_CONNECTIONS: u32 = 1;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// PostgreSQL 连接池的封装。API 层只持有 `Db`，不需要依赖 SQLx。
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    /// 建立连接池，并为每条新连接设置语句/锁/空闲事务超时。
    ///
    /// 这些上限让数据库抖动时请求能在有限时间内失败，而不是把连接和
    /// 请求一起拖住；上层再把连接类错误映射成 503。
    pub async fn connect(database_url: &str, max_connections: u32) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections.max(MIN_CONNECTIONS))
            .min_connections(MIN_CONNECTIONS)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .idle_timeout(Some(IDLE_TIMEOUT))
            .max_lifetime(Some(MAX_LIFETIME))
            // 从池里取连接前做一次轻量探活，避免拿到已经断开的连接。
            .test_before_acquire(true)
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    for statement in [
                        "SET statement_timeout = '10s'",
                        "SET lock_timeout = '3s'",
                        "SET idle_in_transaction_session_timeout = '15s'",
                    ] {
                        sqlx::query(statement).execute(&mut *connection).await?;
                    }
                    Ok(())
                })
            })
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// 执行内嵌迁移；sqlx 使用 advisory lock，多副本同时启动也安全。
    ///
    /// 大表 DDL 可能超过连接的 `statement_timeout`，生产迁移建议用独立 job
    /// 并关闭应用内迁移（`RUN_MIGRATIONS=false`）。
    pub async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::migrate!("../../migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }

    /// 仓储构造需要连接池；只在 `qe-api` 装配 `AppState` 时使用。
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
