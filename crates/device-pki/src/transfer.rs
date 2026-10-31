//! 上行传输进度 / 完成（契约 §16.1）：把 agent 的拉取结果落到 `transfers`
//! 与 `image_assignments.rollout_state`。
//!
//! 两条消息都走现有 `/messages`（或 WebSocket 隧道）通道，`payload` 是 JSON
//! **字符串**；身份仍由 mTLS 证书决定，`transfer_id` 则是排给该设备的下行
//! `image.pull` 里带来的能力（UUID 不可猜）。
//!
//! 语义与上行其它消息一致：handler 返回 `Err` → 不推水位、设备重发；
//! 因此这里的状态映射必须**幂等**（同一 payload 重放结果相同）。

use crate::error::{Error, Result};
use crate::messaging::{Dispatcher, Message};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

/// `transfer.progress` 的 kind。
pub const KIND_PROGRESS: &str = "transfer.progress";
/// `transfer.completed` 的 kind。
pub const KIND_COMPLETED: &str = "transfer.completed";

/// 契约允许的阶段；未知阶段按 400 拒绝（客户端 bug 应尽早暴露）。
pub const PHASES: [&str; 3] = ["download", "verify", "import"];

/// `transfer.progress` 的 payload（契约 §16.1）。
#[derive(Debug, Clone, Deserialize)]
pub struct TransferProgress {
    pub transfer_id: Uuid,
    /// download | verify | import
    #[serde(default)]
    pub phase: Option<String>,
    /// 0..100（越界会被 clamp，避免一条脏数据把进度条推到 100 之外）
    #[serde(default)]
    pub progress: Option<i32>,
    #[serde(default)]
    pub bytes_done: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `transfer.completed` 的 payload（契约 §16.1）。
#[derive(Debug, Clone, Deserialize)]
pub struct TransferCompleted {
    pub transfer_id: Uuid,
    pub ok: bool,
    /// 设备实际校验通过的 manifest digest（用于审计/对账，目前不覆盖 transfer 的列）
    #[serde(default)]
    pub manifest_digest: Option<String>,
    /// 设备是否通过 manifest 验签 / 分片校验
    #[serde(default)]
    pub verified: Option<bool>,
    /// 导入后的本地引用（docker:name:version 等）
    #[serde(default)]
    pub imported_ref: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// 解析 `transfer.progress` payload：坏 JSON / 未知阶段 → 400。
pub fn parse_progress(payload: &str) -> Result<TransferProgress> {
    let update: TransferProgress = serde_json::from_str(payload)
        .map_err(|e| Error::BadRequest(format!("transfer.progress payload 不是合法 JSON: {e}")))?;
    validate_phase(update.phase.as_deref())?;
    Ok(update)
}

/// 解析 `transfer.completed` payload。
pub fn parse_completed(payload: &str) -> Result<TransferCompleted> {
    serde_json::from_str(payload)
        .map_err(|e| Error::BadRequest(format!("transfer.completed payload 不是合法 JSON: {e}")))
}

/// 阶段白名单校验（`None` = 客户端没带，允许）。
pub fn validate_phase(phase: Option<&str>) -> Result<()> {
    match phase {
        None => Ok(()),
        Some(phase) if PHASES.contains(&phase) => Ok(()),
        Some(phase) => Err(Error::BadRequest(format!(
            "phase 必须是 download|verify|import，收到 {phase}"
        ))),
    }
}

/// 进度 clamp 到 0..=100。
pub fn clamp_progress(progress: Option<i32>) -> Option<i32> {
    progress.map(|value| value.clamp(0, 100))
}

/// `completed.ok` → `transfers.status`。
pub fn completed_status(ok: bool) -> &'static str {
    if ok { "completed" } else { "failed" }
}

/// `completed.ok` → 是否强制把进度推到 100。
///
/// 失败时保留最后一次已知进度：agent 已经拉到 60% 才导入失败，
/// 把进度清零会掩盖"下载本身是好的"。
pub fn completed_progress(ok: bool) -> Option<i32> {
    if ok { Some(100) } else { None }
}

/// `completed.ok` → `image_assignments.rollout_state`。
pub fn rollout_state(ok: bool) -> &'static str {
    if ok { "deployed" } else { "failed" }
}

/// 更新 `transfers`：`running` + 进度，并清掉上一次的错误。
pub async fn apply_progress(pool: &PgPool, update: &TransferProgress) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE transfers
         SET status = 'running', progress = COALESCE($2, progress),
             error = NULL, updated_at = now()
         WHERE id = $1",
    )
    .bind(update.transfer_id)
    .bind(clamp_progress(update.progress))
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 更新 `transfers` 终态 + 对应 `image_assignments.rollout_state`（同一事务）。
pub async fn apply_completed(pool: &PgPool, update: &TransferCompleted) -> Result<u64> {
    let mut tx = pool.begin().await?;

    let error = if update.ok {
        None
    } else {
        Some(update.error.as_deref().unwrap_or("transfer failed"))
    };
    let res = sqlx::query(
        "UPDATE transfers
         SET status = $2, progress = COALESCE($3, progress),
             error = $4, updated_at = now()
         WHERE id = $1",
    )
    .bind(update.transfer_id)
    .bind(completed_status(update.ok))
    .bind(completed_progress(update.ok))
    .bind(error)
    .execute(&mut *tx)
    .await?;

    // 定位 assignment 靠 transfer 自己的 (room_id, image_id)，而不是请求体，
    // 避免设备伪造一个别的房间的分配状态。
    sqlx::query(
        "UPDATE image_assignments ia
         SET rollout_state = $2, updated_at = now()
         FROM transfers t
         WHERE t.id = $1 AND ia.room_id = t.room_id AND ia.image_id = t.image_id",
    )
    .bind(update.transfer_id)
    .bind(rollout_state(update.ok))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(res.rows_affected())
}

/// 把两条 handler 注册到分发器（main 启动时调用）。
///
/// 找不到 transfer 时**不报错**：消息是 at-least-once 的，让设备为一条
/// 已被清理/删除的 transfer 无限重发只会卡死它自己的 outbox。
pub fn install(dispatcher: &Dispatcher, db: PgPool) {
    let db_progress = db.clone();
    dispatcher.subscribe(KIND_PROGRESS, move |msg: Message| {
        let db = db_progress.clone();
        async move {
            let update = parse_progress(&msg.payload)?;
            let rows = apply_progress(&db, &update).await?;
            if rows == 0 {
                tracing::warn!(
                    transfer_id = %update.transfer_id,
                    serial = %msg.serial,
                    "transfer.progress 对应的 transfer 不存在，按幂等 no-op 处理"
                );
            } else {
                tracing::debug!(
                    transfer_id = %update.transfer_id,
                    phase = update.phase.as_deref().unwrap_or("-"),
                    progress = update.progress.unwrap_or(-1),
                    "transfer.progress"
                );
            }
            Ok(())
        }
    });

    dispatcher.subscribe(KIND_COMPLETED, move |msg: Message| {
        let db = db.clone();
        async move {
            let update = parse_completed(&msg.payload)?;
            let rows = apply_completed(&db, &update).await?;
            if rows == 0 {
                tracing::warn!(
                    transfer_id = %update.transfer_id,
                    serial = %msg.serial,
                    "transfer.completed 对应的 transfer 不存在，按幂等 no-op 处理"
                );
            } else {
                tracing::info!(
                    transfer_id = %update.transfer_id,
                    ok = update.ok,
                    verified = update.verified.unwrap_or(false),
                    imported_ref = update.imported_ref.as_deref().unwrap_or("-"),
                    "transfer.completed"
                );
            }
            Ok(())
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSFER_ID: &str = "0b6f6f8e-6f3c-4b0e-9a3e-5c2f4c6d8e10";

    fn progress_payload() -> String {
        format!(
            r#"{{"transfer_id":"{TRANSFER_ID}","phase":"download",
                "progress":42,"bytes_done":1024,"error":null}}"#
        )
    }

    #[test]
    fn parses_progress_payload() {
        let update = parse_progress(&progress_payload()).unwrap();
        assert_eq!(update.transfer_id.to_string(), TRANSFER_ID);
        assert_eq!(update.phase.as_deref(), Some("download"));
        assert_eq!(update.progress, Some(42));
        assert_eq!(update.bytes_done, Some(1024));
        assert!(update.error.is_none());
    }

    #[test]
    fn parses_completed_payload() {
        let payload = format!(
            r#"{{"transfer_id":"{TRANSFER_ID}","ok":true,
                "manifest_digest":"{}","verified":true,"imported_ref":"docker:img:1","error":null}}"#,
            "ab".repeat(32)
        );
        let update = parse_completed(&payload).unwrap();
        assert!(update.ok);
        assert_eq!(update.verified, Some(true));
        assert_eq!(update.imported_ref.as_deref(), Some("docker:img:1"));
    }

    #[test]
    fn rejects_bad_json_and_bad_transfer_id() {
        assert_eq!(
            parse_progress("not json").unwrap_err().code(),
            "bad_request"
        );
        let bad_id = r#"{"transfer_id":"not-a-uuid","ok":true}"#.to_owned();
        assert_eq!(parse_completed(&bad_id).unwrap_err().code(), "bad_request");
    }

    #[test]
    fn rejects_unknown_phase() {
        let payload = format!(r#"{{"transfer_id":"{TRANSFER_ID}","phase":"install"}}"#);
        let err = parse_progress(&payload).unwrap_err();
        assert_eq!(err.code(), "bad_request");
        assert!(err.to_string().contains("phase"));
        for phase in PHASES {
            let payload = format!(r#"{{"transfer_id":"{TRANSFER_ID}","phase":"{phase}"}}"#);
            assert!(parse_progress(&payload).is_ok(), "{phase}");
        }
        // 不带 phase 也合法
        let payload = format!(r#"{{"transfer_id":"{TRANSFER_ID}","progress":1}}"#);
        assert!(parse_progress(&payload).is_ok());
    }

    #[test]
    fn progress_is_clamped_to_percent_range() {
        assert_eq!(clamp_progress(Some(-1)), Some(0));
        assert_eq!(clamp_progress(Some(0)), Some(0));
        assert_eq!(clamp_progress(Some(99)), Some(99));
        assert_eq!(clamp_progress(Some(100)), Some(100));
        assert_eq!(clamp_progress(Some(101)), Some(100));
        assert_eq!(clamp_progress(None), None);
    }

    #[test]
    fn completion_maps_to_contract_states() {
        assert_eq!(completed_status(true), "completed");
        assert_eq!(completed_progress(true), Some(100));
        assert_eq!(rollout_state(true), "deployed");

        assert_eq!(completed_status(false), "failed");
        // 失败保留原进度
        assert_eq!(completed_progress(false), None);
        assert_eq!(rollout_state(false), "failed");
    }
}
