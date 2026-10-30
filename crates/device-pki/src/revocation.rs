//! 吊销状态内存缓存。
//!
//! ## 为什么需要它
//!
//! rustls 的 `verify_client_cert` 回调是**同步**的，不能 await。之前用
//! `block_in_place` + `block_on` 查库，但单线程 runtime 下 `block_in_place` 会 panic，
//! 于是降级为「放行」——这意味着**单线程部署下吊销根本不生效**。
//!
//! 现在的做法：后台任务定期把已吊销 serial 拉进 `DashSet`，verifier 只查内存。
//! - 没有任何 async 跨越同步边界
//! - 每次握手零 DB 延迟
//! - 刷新失败时**保留上一份快照**，不会因为 DB 抖动而误放行

use dashmap::DashSet;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

/// 默认刷新间隔。吊销生效延迟 = 这个值 + 一次握手时间。
pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// 已吊销 serial 的内存快照。
#[derive(Debug, Default)]
pub struct RevocationCache {
    revoked: DashSet<String>,
}

impl RevocationCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 一次性载入全量吊销集合。
    pub async fn reload(&self, db: &PgPool) -> Result<usize, sqlx::Error> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT serial FROM device_enrollment WHERE revoked_at IS NOT NULL",
        )
        .fetch_all(db)
        .await?;

        let fresh: Vec<String> = rows.into_iter().map(|(s,)| s).collect();
        let count = fresh.len();

        // 用新快照替换旧内容：先清空再加入，避免已删除记录残留
        self.revoked.clear();
        for s in fresh {
            self.revoked.insert(s);
        }
        Ok(count)
    }

    /// 本地标记吊销（签发新证书顶掉旧证书时立即生效，不必等下一轮刷新）。
    pub fn mark_revoked(&self, serial: &str) {
        self.revoked.insert(serial.to_owned());
    }

    pub fn is_revoked(&self, serial_hex: &str) -> bool {
        self.revoked.contains(serial_hex)
    }

    pub fn len(&self) -> usize {
        self.revoked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.revoked.is_empty()
    }
}

/// 启动后台刷新循环。返回句柄供测试/关闭使用。
pub fn spawn_refresher(
    cache: Arc<RevocationCache>,
    db: PgPool,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match cache.reload(&db).await {
                Ok(n) => tracing::debug!(revoked = n, "revocation cache refreshed"),
                Err(e) => {
                    // 关键：刷新失败保留旧快照，宁可少放行也不误放行
                    tracing::warn!(
                        error = %e,
                        kept = cache.len(),
                        "revocation cache refresh failed, keeping previous snapshot"
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_reports_not_revoked() {
        let c = RevocationCache::new();
        assert!(!c.is_revoked("abc"));
        assert!(c.is_empty());
    }

    #[test]
    fn mark_revoked_is_immediate() {
        let c = RevocationCache::new();
        c.mark_revoked("deadbeef");
        assert!(c.is_revoked("deadbeef"));
        assert_eq!(c.len(), 1);
        assert!(!c.is_revoked("other"));
    }

    #[test]
    fn mark_revoked_is_idempotent() {
        let c = RevocationCache::new();
        c.mark_revoked("x");
        c.mark_revoked("x");
        assert_eq!(c.len(), 1);
    }

    #[tokio::test]
    async fn refresher_survives_db_errors() {
        // 指向不存在的库，refresh 会持续失败，但任务不应退出
        let cache = Arc::new(RevocationCache::new());
        cache.mark_revoked("keepme");
        let db = match PgPool::connect_lazy("postgres://127.0.0.1:1/none") {
            Ok(p) => p,
            Err(_) => return, // 连接串无效时跳过
        };
        let handle = spawn_refresher(
            Arc::clone(&cache),
            db,
            Duration::from_millis(20),
        );
        tokio::time::sleep(Duration::from_millis(80)).await;
        handle.abort();
        // 刷新失败必须保留旧快照
        assert!(
            cache.is_revoked("keepme"),
            "刷新失败不应清空已有吊销集合"
        );
    }
}
