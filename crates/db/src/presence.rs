//! 设备在线事件与可靠投递仓储（P3）。
//!
//! 契约见 `docs/storage-design.md` §18.1–§18.3，表结构见
//! `migrations/0013_device_presence.sql`。
//!
//! 设计要点：
//! - **事件幂等**：唯一约束 `(serial, current_state, source_updated_at)`，
//!   多副本同时扫描到同一翻转时只有一行能插入成功；
//! - **投递幂等**：唯一约束 `(event_id, target_url)`，重复 `add_delivery` 不会建第二行；
//! - **多副本领取**：`claim_due` 用 `FOR UPDATE SKIP LOCKED` 原子领取到期投递，
//!   并把 `next_attempt_at` 推迟 60 秒作为租约，避免同一行被两个副本同时处理；
//! - 全部使用运行时 `sqlx` 查询（不依赖编译期 DATABASE_URL 宏）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::error::DbResult;

/// 待插入的设备在线事件。`payload` 由 worker 组装（对外 webhook / SSE 的 JSON）。
#[derive(Debug, Clone)]
pub struct NewPresenceEvent {
    pub serial: String,
    pub device_slot: String,
    pub room_id: Option<Uuid>,
    pub area_id: Option<Uuid>,
    pub previous_state: String,
    pub current_state: String,
    /// 触发翻转的 `device_session.updated_at`；与 `(serial, current_state)` 一起做幂等键。
    pub source_updated_at: DateTime<Utc>,
    pub payload: Value,
}

/// 已落库的设备在线事件（SSE / 管理端查询返回）。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PresenceEventRecord {
    pub id: i64,
    pub serial: String,
    pub device_slot: String,
    pub room_id: Option<Uuid>,
    pub area_id: Option<Uuid>,
    pub previous_state: String,
    pub current_state: String,
    pub occurred_at: DateTime<Utc>,
    pub payload: Value,
}

/// 投递记录（排障查询返回）。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PresenceDeliveryRecord {
    pub id: i64,
    pub event_id: i64,
    pub target_url: String,
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// `claim_due` 领取到的一条到期投递。
#[derive(Debug, Clone, FromRow)]
pub struct DueDelivery {
    pub id: i64,
    pub event_id: i64,
    pub target_url: String,
    pub attempts: i32,
}

/// PostgreSQL 适配器：只负责 SQL，不含 HTTP / 重试策略（策略在 qe-api worker）。
#[derive(Clone)]
pub struct PresenceRepo {
    db: PgPool,
}

impl PresenceRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 幂等插入事件；重复（多副本扫描到同一翻转）返回 `None`。
    pub async fn insert_event(&self, event: &NewPresenceEvent) -> DbResult<Option<i64>> {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO device_presence_events
                 (serial, device_slot, room_id, area_id, previous_state, current_state,
                  source_updated_at, payload)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (serial, current_state, source_updated_at) DO NOTHING
             RETURNING id",
        )
        .bind(&event.serial)
        .bind(&event.device_slot)
        .bind(event.room_id)
        .bind(event.area_id)
        .bind(&event.previous_state)
        .bind(&event.current_state)
        .bind(event.source_updated_at)
        .bind(&event.payload)
        .fetch_optional(&self.db)
        .await?;
        Ok(id)
    }

    /// 幂等创建投递；返回 `true` 表示本次新插入。
    pub async fn add_delivery(&self, event_id: i64, url: &str) -> DbResult<bool> {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO device_presence_deliveries (event_id, target_url)
             VALUES ($1, $2)
             ON CONFLICT (event_id, target_url) DO NOTHING
             RETURNING id",
        )
        .bind(event_id)
        .bind(url)
        .fetch_optional(&self.db)
        .await?;
        Ok(id.is_some())
    }

    /// 领取最多 `limit` 条到期投递。
    ///
    /// 单条 SQL 完成「选行 + 加锁 + 续租」：`FOR UPDATE SKIP LOCKED` 让并发副本
    /// 跳过彼此已锁定的行；`next_attempt_at = now() + 60s` 是租约，领取者崩溃后
    /// 60 秒自动重新可领取。
    pub async fn claim_due(&self, limit: i64) -> DbResult<Vec<DueDelivery>> {
        let rows = sqlx::query_as::<_, DueDelivery>(
            "UPDATE device_presence_deliveries d
                SET next_attempt_at = now() + interval '60 seconds',
                    updated_at = now()
              WHERE d.id IN (
                    SELECT id
                      FROM device_presence_deliveries
                     WHERE status = 'pending'
                       AND next_attempt_at <= now()
                     ORDER BY id
                     LIMIT $1
                     FOR UPDATE SKIP LOCKED
              )
          RETURNING d.id, d.event_id, d.target_url, d.attempts",
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 投递成功：置 `delivered` 并把实际尝试次数 +1。
    pub async fn mark_delivered(&self, id: i64) -> DbResult<()> {
        sqlx::query(
            "UPDATE device_presence_deliveries
                SET status = 'delivered',
                    attempts = attempts + 1,
                    last_attempt_at = now(),
                    last_error = NULL,
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// 投递失败但未达上限：记录本次尝试并安排下次重试。
    pub async fn mark_retry(
        &self,
        id: i64,
        attempts: i32,
        next_at: DateTime<Utc>,
        err: &str,
    ) -> DbResult<()> {
        sqlx::query(
            "UPDATE device_presence_deliveries
                SET status = 'pending',
                    attempts = $2,
                    next_attempt_at = $3,
                    last_attempt_at = now(),
                    last_error = $4,
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .bind(attempts)
        .bind(next_at)
        .bind(err)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// 投递失败且达到最大尝试次数：置 `failed`，不再重试。
    pub async fn mark_failed(&self, id: i64, attempts: i32, err: &str) -> DbResult<()> {
        sqlx::query(
            "UPDATE device_presence_deliveries
                SET status = 'failed',
                    attempts = $2,
                    last_attempt_at = now(),
                    last_error = $3,
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .bind(attempts)
        .bind(err)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// 按 id 倒序查询事件；`after_id` 表示只取 `id > after_id`（SSE 增量补发）。
    pub async fn list_events(
        &self,
        limit: i64,
        after_id: Option<i64>,
    ) -> DbResult<Vec<PresenceEventRecord>> {
        let rows = sqlx::query_as::<_, PresenceEventRecord>(
            "SELECT id, serial, device_slot, room_id, area_id, previous_state, current_state,
                    occurred_at, payload
               FROM device_presence_events
              WHERE ($2::bigint IS NULL OR id > $2)
              ORDER BY id DESC
              LIMIT $1",
        )
        .bind(limit)
        .bind(after_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 按 id 倒序查询投递记录；`status` 为 `None` 时不过滤。
    pub async fn list_deliveries(
        &self,
        limit: i64,
        status: Option<&str>,
    ) -> DbResult<Vec<PresenceDeliveryRecord>> {
        let rows = sqlx::query_as::<_, PresenceDeliveryRecord>(
            "SELECT id, event_id, target_url, status, attempts, next_attempt_at,
                    last_attempt_at, last_error
               FROM device_presence_deliveries
              WHERE ($2::text IS NULL OR status = $2)
              ORDER BY id DESC
              LIMIT $1",
        )
        .bind(limit)
        .bind(status)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 读取单条事件（worker 投递前取 payload）。
    pub async fn get_event(&self, id: i64) -> DbResult<Option<PresenceEventRecord>> {
        let row = sqlx::query_as::<_, PresenceEventRecord>(
            "SELECT id, serial, device_slot, room_id, area_id, previous_state, current_state,
                    occurred_at, payload
               FROM device_presence_events
              WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row)
    }
}
