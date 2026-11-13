//! 镜像制作工单后台维护（契约 `docs/image-build-lifecycle.md` §4.2）。
//!
//! 制作工单默认 30 分钟有效：控制面下发 `image.make` 时落 `pending`，
//! 设备在有效期内 register 才会消费。设备掉线/制作失败/用户关页面后，
//! 工单会一直停在 `pending`——本任务周期把 `expires_at <= now()` 的工单
//! 置为 `expired`，避免同名唯一索引被永久占用（同一设备同名同版本无法再次下发）。
//!
//! 只做状态收敛，不删除 draft artifact：草稿保留供人工排查/后续清理策略，
//! 也符合契约「失败保留 draft、不产生同名孤儿」的取舍。
//!
//! 多副本下 `UPDATE ... WHERE state='pending'` 天然幂等，不需要分布式锁。

use std::time::Duration;

use tokio::time::{MissedTickBehavior, interval};

use crate::state::AppState;

/// 扫描周期：60s。工单有效期是分钟级，没必要更频繁。
const EXPIRY_TICK: Duration = Duration::from_secs(60);

/// 启动后台工单过期扫描（生命周期跟随进程）。
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(EXPIRY_TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match state.storage_repo.expire_build_requests().await {
                Ok(0) => {}
                Ok(expired) => {
                    tracing::info!(expired, "镜像制作工单过期清理完成");
                }
                Err(error) => {
                    // 数据库瞬时不可用：只告警，下一 tick 继续。
                    tracing::warn!(%error, "镜像制作工单过期清理失败，下轮重试");
                }
            }
        }
    })
}
