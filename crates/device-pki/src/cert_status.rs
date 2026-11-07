//! 证书可用性内存缓存（签发状态 + 吊销状态）。
//!
//! ## 为什么需要它
//!
//! rustls 的 `verify_client_cert` 回调是**同步**的，不能 await。之前用
//! `block_in_place` + `block_on` 查库，但单线程 runtime 下 `block_in_place` 会 panic，
//! 于是降级为「放行」——这意味着**单线程部署下吊销根本不生效**。
//!
//! 现在的做法：后台任务定期把 DB 里的状态拉成新快照并**整体原子替换**，verifier 只查内存。
//! - 没有任何 async 跨越同步边界
//! - 每次握手零 DB 延迟
//! - 刷新失败时**保留上一份快照**，不会因为 DB 抖动而误放行
//! - 整体替换而不是"先清空再插入"：后者存在几微秒的空窗，空窗里并发握手
//!   看不到任何吊销项 —— 恰好是 fail-open
//!
//! ## 不只是吊销
//!
//! verifier 还要求证书**在 `device_enrollment` 里且处于 `issued`/`active`**。
//! 只查吊销的话，`pending`（还没批）/`rejected`（被拒）阶段就已签好的证书
//! 只要 PEM 一旦泄露就永远可用——而这些证书的有效期是按年算的。
//!
//! **fail-closed**：一次都没成功刷新过时，`is_live()` 恒为 false，
//! 也就是拒绝所有握手，直到后台刷新成功。PKI 服务宁可短暂拒绝也不该放行。

use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// 默认刷新间隔。状态变化的可见延迟 = 这个值 + 一次握手时间。
pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// serial → 可用性快照。
#[derive(Debug, Default)]
pub struct CertStatusCache {
    /// 已吊销（`revoked_at IS NOT NULL`）或被本地立即标记的 serial
    revoked: RwLock<HashSet<String>>,
    /// `status IN ('issued','active')` 的 serial
    live: RwLock<HashSet<String>>,
    /// 至少成功刷新过一次
    loaded: AtomicBool,
}

impl CertStatusCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn read(set: &RwLock<HashSet<String>>) -> std::sync::RwLockReadGuard<'_, HashSet<String>> {
        set.read().unwrap_or_else(|e| e.into_inner())
    }

    /// 一次性载入全量状态（锁外构建，再一次原子替换）。
    pub async fn reload(&self, db: &PgPool) -> Result<usize, sqlx::Error> {
        let revoked_rows: Vec<(String,)> =
            sqlx::query_as("SELECT serial FROM device_enrollment WHERE revoked_at IS NOT NULL")
                .fetch_all(db)
                .await?;
        let live_rows: Vec<(String,)> = sqlx::query_as(
            "SELECT serial FROM device_enrollment WHERE status IN ('issued','active')",
        )
        .fetch_all(db)
        .await?;

        let revoked: HashSet<String> = revoked_rows.into_iter().map(|(s,)| s).collect();
        let live: HashSet<String> = live_rows.into_iter().map(|(s,)| s).collect();
        let count = revoked.len();

        *self.revoked.write().unwrap_or_else(|e| e.into_inner()) = revoked;
        *self.live.write().unwrap_or_else(|e| e.into_inner()) = live;
        self.loaded.store(true, Ordering::Release);
        Ok(count)
    }

    /// 本地立即标记可用（新签证书/审批通过时用，不等 5s 刷新）。
    pub fn mark_live(&self, serial: &str) {
        self.revoked
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(serial);
        self.live
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(serial.to_owned());
    }

    /// 本地立即标记吊销（顶掉旧证书 / 审批拒绝时用，不等下一轮刷新）。
    pub fn mark_revoked(&self, serial: &str) {
        self.revoked
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(serial.to_owned());
        self.live
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(serial);
    }

    pub fn is_revoked(&self, serial_hex: &str) -> bool {
        Self::read(&self.revoked).contains(serial_hex)
    }

    /// 证书是否可用于握手：处于 `issued`/`active` 且未被吊销。
    /// 从未成功刷新过 → false（fail-closed）。
    pub fn is_live(&self, serial_hex: &str) -> bool {
        if !self.loaded.load(Ordering::Acquire) {
            return false;
        }
        !self.is_revoked(serial_hex) && Self::read(&self.live).contains(serial_hex)
    }

    pub fn len(&self) -> usize {
        Self::read(&self.revoked).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 启动后台刷新循环。返回句柄供测试/关闭使用。
pub fn spawn_refresher(
    cache: Arc<CertStatusCache>,
    db: PgPool,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match cache.reload(&db).await {
                Ok(n) => tracing::debug!(revoked = n, "证书状态缓存已刷新"),
                Err(e) => {
                    // 关键：刷新失败保留旧快照，宁可少放行也不误放行
                    tracing::warn!(
                        error = %e,
                        kept = cache.len(),
                        "证书状态缓存刷新失败，保留上一份快照"
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
    fn empty_cache_is_fail_closed() {
        let c = CertStatusCache::new();
        assert!(!c.is_live("abc"), "从未刷新过时必须拒绝一切握手");
        assert!(!c.is_revoked("abc"));
        assert!(c.is_empty());
    }

    #[test]
    fn mark_revoked_is_immediate_and_drops_liveness() {
        let c = CertStatusCache::new();
        // 先模拟一次成功刷新（loaded=true）
        c.loaded.store(true, Ordering::Release);
        c.live.write().unwrap().insert("deadbeef".to_owned());
        assert!(c.is_live("deadbeef"));

        c.mark_revoked("deadbeef");
        assert!(c.is_revoked("deadbeef"));
        assert!(!c.is_live("deadbeef"), "本地标记后必须立刻不可用");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn mark_revoked_is_idempotent() {
        let c = CertStatusCache::new();
        c.mark_revoked("x");
        c.mark_revoked("x");
        assert_eq!(c.len(), 1);
        assert!(!c.is_revoked("y"));
    }

    #[tokio::test]
    async fn refresher_survives_db_errors() {
        // 指向不存在的库，refresh 会持续失败，但任务不应退出
        let cache = Arc::new(CertStatusCache::new());
        cache.mark_revoked("keepme");
        let db = match PgPool::connect_lazy("postgres://127.0.0.1:1/none") {
            Ok(p) => p,
            Err(_) => return, // 连接串无效时跳过
        };
        let handle = spawn_refresher(Arc::clone(&cache), db, Duration::from_millis(20));
        tokio::time::sleep(Duration::from_millis(80)).await;
        handle.abort();
        // 刷新失败必须保留旧快照
        assert!(cache.is_revoked("keepme"), "刷新失败不应清空已有吊销集合");
    }
}
