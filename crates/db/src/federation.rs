//! 总控阶段 2：多控制节点联邦（`federation_nodes`）。
//!
//! MVP 只做「注册 + 刷新 + 只读聚合」：
//! - 本地表保存其他控制节点的地址/token/最近摘要缓存；
//! - 刷新时由 qe-api 主动拉取远端 `GET /api/v1/admin/federation/summary`；
//! - 总控页把本节点 + 各 enabled 节点的计数聚合成跨机房 KPI。
//!
//! 设计见 `docs/federation.md` 与 `docs/total-control.md` §3。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, DbResult},
    rows::FederationNodeRow,
};

/// 一个控制节点的只读摘要计数（本节点 `/federation/summary` 的 `counts` 字段）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct LocalSummary {
    /// 启用中的房间数。
    pub rooms: i64,
    /// 启用中的区域数。
    pub areas: i64,
    /// 已注册（issued/active）的设备/机器数。
    pub machines: i64,
    /// 在线机器数（device-pki 会话 online 或 5 分钟内上报过快照）。
    pub online_machines: i64,
    /// 未结束工单数（open/in_progress/reopened 等，排除 resolved/closed/cancelled）。
    pub open_tickets: i64,
    /// 镜像制品总数（image_artifacts）。
    pub images: i64,
}

/// 联邦节点（含明文 token，**仅内部使用**，API 返回前必须脱敏）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationNode {
    pub id: Uuid,
    pub name: String,
    pub base_url: String,
    pub token: String,
    pub enabled: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub cached_summary: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FederationNode {
    /// 转成行模型（`rows::FederationNodeRow` 的字段顺序/类型）。
    pub fn from_row(row: FederationNodeRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            base_url: row.base_url,
            token: row.token,
            enabled: row.enabled,
            last_seen_at: row.last_seen_at,
            last_status: row.last_status,
            cached_summary: row.cached_summary,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// 新建联邦节点的输入。
#[derive(Debug, Clone)]
pub struct NewFederationNode {
    pub name: String,
    pub base_url: String,
    pub token: String,
}

/// 联邦节点仓储。
#[derive(Clone)]
pub struct FederationRepo {
    db: PgPool,
}

impl FederationRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 全部节点（含 disabled），按名称排序。
    pub async fn list(&self) -> DbResult<Vec<FederationNode>> {
        let rows = sqlx::query_as::<_, FederationNodeRow>(
            "SELECT id, name, base_url, token, enabled, last_seen_at, last_status,
                    cached_summary, created_at, updated_at
               FROM federation_nodes
              ORDER BY name",
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(FederationNode::from_row).collect())
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<FederationNode>> {
        let row = sqlx::query_as::<_, FederationNodeRow>(
            "SELECT id, name, base_url, token, enabled, last_seen_at, last_status,
                    cached_summary, created_at, updated_at
               FROM federation_nodes
              WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(FederationNode::from_row))
    }

    pub async fn create(&self, input: &NewFederationNode) -> DbResult<FederationNode> {
        let row = sqlx::query_as::<_, FederationNodeRow>(
            "INSERT INTO federation_nodes (name, base_url, token)
             VALUES ($1, $2, $3)
             RETURNING id, name, base_url, token, enabled, last_seen_at, last_status,
                       cached_summary, created_at, updated_at",
        )
        .bind(&input.name)
        .bind(&input.base_url)
        .bind(&input.token)
        .fetch_one(&self.db)
        .await?;
        Ok(FederationNode::from_row(row))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let affected = sqlx::query("DELETE FROM federation_nodes WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(affected > 0)
    }

    /// 刷新成功：记录 online、最近成功拉取时间与远端摘要缓存。
    pub async fn mark_online(&self, id: Uuid, summary: &Value) -> DbResult<Option<FederationNode>> {
        let row = sqlx::query_as::<_, FederationNodeRow>(
            "UPDATE federation_nodes
                SET last_status = 'online', last_seen_at = now(), cached_summary = $2,
                    updated_at = now()
              WHERE id = $1
              RETURNING id, name, base_url, token, enabled, last_seen_at, last_status,
                        cached_summary, created_at, updated_at",
        )
        .bind(id)
        .bind(summary)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(FederationNode::from_row))
    }

    /// 刷新失败：标记 offline（保留上次成功的 cached_summary 供页面展示旧数据）。
    pub async fn mark_offline(&self, id: Uuid) -> DbResult<Option<FederationNode>> {
        let row = sqlx::query_as::<_, FederationNodeRow>(
            "UPDATE federation_nodes
                SET last_status = 'offline', updated_at = now()
              WHERE id = $1
              RETURNING id, name, base_url, token, enabled, last_seen_at, last_status,
                        cached_summary, created_at, updated_at",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(FederationNode::from_row))
    }

    /// 本节点摘要计数（一次查询聚合；供 `/federation/summary` 与总控页使用）。
    pub async fn local_summary(&self) -> DbResult<LocalSummary> {
        let summary = sqlx::query_as::<_, LocalSummary>(
            "SELECT
                (SELECT count(*) FROM rooms WHERE is_active) AS rooms,
                (SELECT count(*) FROM areas WHERE is_active) AS areas,
                (SELECT count(*) FROM device_enrollment
                  WHERE status IN ('issued', 'active')) AS machines,
                (SELECT count(*) FROM device_enrollment e
                  WHERE e.status IN ('issued', 'active')
                    AND (EXISTS (SELECT 1 FROM device_session ds
                                  WHERE ds.serial = e.serial AND ds.state = 'online')
                      OR EXISTS (SELECT 1 FROM machine_snapshots ms
                                  WHERE ms.serial = e.serial
                                    AND ms.collected_at >= now() - interval '5 minutes')))
                  AS online_machines,
                (SELECT count(*) FROM tickets
                  WHERE status NOT IN ('resolved', 'closed', 'cancelled')) AS open_tickets,
                (SELECT count(*) FROM image_artifacts) AS images",
        )
        .fetch_one(&self.db)
        .await?;
        Ok(summary)
    }
}

/// `(name, version, platform)` 唯一冲突等映射沿用 [DbError]；这里仅做类型别名导出。
pub type FederationResult<T> = std::result::Result<T, DbError>;
