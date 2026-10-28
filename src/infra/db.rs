use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

/// 建立 PostgreSQL 连接池。连接超时故意设得较短，便于容器编排快速失败/重试。
pub async fn connect(database_url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// 执行 `migrations/` 下内嵌到二进制中的迁移。
/// sqlx 使用 PostgreSQL advisory lock，多副本同时启动也是安全的。
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await?;
    Ok(())
}
