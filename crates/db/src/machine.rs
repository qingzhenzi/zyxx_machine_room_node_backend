//! 机器遥测仓储：`machine_snapshots` 的写入/裁剪与机器总览查询。
//!
//! 契约见 `docs/machine-telemetry.md`。机器人（room-agent）按 `device_slot` 上报，
//! 服务端解析到 `device_enrollment.serial` 后落库；管理端按设备维度分页查询最新快照。
//!
//! 设计要点：
//! - `(serial, collected_at)` 唯一，重试/重复上报天然幂等；
//! - 完整上报 JSON 存 `payload`，列表只读列级冗余指标（CPU/内存/磁盘/健康分）；
//! - 每个 serial 只保留最近 `keep` 条，插入后在同一仓储内裁剪，避免后台任务。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::models::Page;

use crate::error::DbResult;

/// 机器人上报解析后、待落库的快照。
#[derive(Debug, Clone)]
pub struct NewMachineSnapshot {
    pub serial: String,
    pub device_slot: String,
    /// 采集档位：`fast`（指标/健康）或 `full`（含外部工具分段）。
    pub tier: qe_domain::machine::MachineReportTier,
    pub collected_at: DateTime<Utc>,
    pub agent_version: String,
    pub hostname: String,
    pub os_name: Option<String>,
    pub kernel_version: Option<String>,
    pub health_status: String,
    pub health_score: Option<i32>,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub disk_usage_percent: Option<f64>,
    pub uptime_secs: Option<i64>,
    /// 完整上报 JSON（原样存档）。
    pub payload: Value,
}

/// 快照元数据（历史列表 / 上报响应使用，不含 payload）。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MachineSnapshotMeta {
    pub id: i64,
    pub serial: String,
    pub device_slot: String,
    pub collected_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub agent_version: String,
    pub hostname: String,
    pub os_name: Option<String>,
    pub kernel_version: Option<String>,
    pub health_status: String,
    pub health_score: Option<i32>,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub disk_usage_percent: Option<f64>,
    pub uptime_secs: Option<i64>,
}

/// 快照详情（含完整 payload）。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MachineSnapshotRecord {
    pub id: i64,
    pub serial: String,
    pub device_slot: String,
    pub collected_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub agent_version: String,
    pub hostname: String,
    pub os_name: Option<String>,
    pub kernel_version: Option<String>,
    pub health_status: String,
    pub health_score: Option<i32>,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub disk_usage_percent: Option<f64>,
    pub uptime_secs: Option<i64>,
    pub payload: Value,
}

/// 最新一次上报的摘要（`MachineSummary.report`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineReportSummary {
    pub collected_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub agent_version: String,
    pub hostname: String,
    pub os_name: Option<String>,
    pub kernel_version: Option<String>,
    pub health_status: String,
    pub health_score: Option<i32>,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub disk_usage_percent: Option<f64>,
    pub uptime_secs: Option<i64>,
}

/// 机器总览的一行：设备注册信息 + 房间/区域 + 最新快照摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineSummary {
    pub serial: String,
    pub device_slot: String,
    pub status: String,
    pub online: Option<String>,
    /// `device_session.state` 原始值（不含 agent 快照并集），用于排障与页面区分展示。
    pub session_online: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub room_id: Option<Uuid>,
    pub room_code: Option<String>,
    pub room_name: Option<String>,
    pub area_id: Option<Uuid>,
    pub area_code: Option<String>,
    pub area_name: Option<String>,
    pub report: Option<MachineReportSummary>,
    /// 最新快照超过 5 分钟未更新（agent 可能离线）。
    pub report_stale: bool,
}

/// 机器列表过滤条件。
#[derive(Debug, Clone, Default)]
pub struct MachineFilter {
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub keyword: Option<String>,
    /// `ok` / `warning` / `critical` / `missing`（从未上报）。
    pub health: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct MachineSummaryRow {
    serial: String,
    device_slot: String,
    status: String,
    online: Option<String>,
    session_online: Option<String>,
    last_seen_at: Option<DateTime<Utc>>,
    room_id: Option<Uuid>,
    room_code: Option<String>,
    room_name: Option<String>,
    area_id: Option<Uuid>,
    area_code: Option<String>,
    area_name: Option<String>,
    report_id: Option<i64>,
    collected_at: Option<DateTime<Utc>>,
    received_at: Option<DateTime<Utc>>,
    agent_version: Option<String>,
    hostname: Option<String>,
    os_name: Option<String>,
    kernel_version: Option<String>,
    health_status: Option<String>,
    health_score: Option<i32>,
    cpu_usage_percent: Option<f64>,
    memory_usage_percent: Option<f64>,
    disk_usage_percent: Option<f64>,
    uptime_secs: Option<i64>,
    report_stale: Option<bool>,
}

impl From<MachineSummaryRow> for MachineSummary {
    fn from(row: MachineSummaryRow) -> Self {
        let report = match (row.report_id, row.collected_at) {
            (Some(_), Some(collected_at)) => Some(MachineReportSummary {
                collected_at,
                received_at: row.received_at.unwrap_or(collected_at),
                agent_version: row.agent_version.unwrap_or_default(),
                hostname: row.hostname.unwrap_or_default(),
                os_name: row.os_name,
                kernel_version: row.kernel_version,
                health_status: row.health_status.unwrap_or_else(|| "unknown".to_owned()),
                health_score: row.health_score,
                cpu_usage_percent: row.cpu_usage_percent,
                memory_usage_percent: row.memory_usage_percent,
                disk_usage_percent: row.disk_usage_percent,
                uptime_secs: row.uptime_secs,
            }),
            _ => None,
        };

        Self {
            serial: row.serial,
            device_slot: row.device_slot,
            status: row.status,
            online: row.online,
            session_online: row.session_online,
            last_seen_at: row.last_seen_at,
            room_id: row.room_id,
            room_code: row.room_code,
            room_name: row.room_name,
            area_id: row.area_id,
            area_code: row.area_code,
            area_name: row.area_name,
            report,
            report_stale: row.report_stale.unwrap_or(false),
        }
    }
}

/// 设备注册记录的最小投影（上报时校验槽位）。
#[derive(Debug, Clone, FromRow)]
pub struct EnrollmentRef {
    pub serial: String,
    pub device_slot: String,
    pub status: String,
}

const SNAPSHOT_COLUMNS: &str = "id, serial, device_slot, collected_at, received_at, agent_version,
     hostname, os_name, kernel_version, health_status, health_score,
     cpu_usage_percent, memory_usage_percent, disk_usage_percent, uptime_secs";

/// 机器列表的 latest 快照 CTE + 设备/房间/区域 join。
/// 房间关联：主设备看 `rooms.device_slot`，子机看 `room_devices`（迁移 0015）。
const MACHINE_FROM: &str = "
    FROM device_enrollment de
    LEFT JOIN device_session ds ON ds.serial = de.serial
    LEFT JOIN rooms rp ON rp.device_slot = de.device_slot
    LEFT JOIN room_devices rd ON rd.device_slot = de.device_slot
    LEFT JOIN rooms r ON r.id = COALESCE(rp.id, rd.room_id)
    LEFT JOIN areas a ON a.id = r.area_id
    LEFT JOIN latest ms ON ms.serial = de.serial";

const MACHINE_FILTER: &str = "
    WHERE ($1::uuid IS NULL OR a.id = $1)
      AND ($2::uuid IS NULL OR r.id = $2)
      AND ($3::text IS NULL OR $3::text = ''
           OR de.serial ILIKE '%' || $3::text || '%'
           OR de.device_slot ILIKE '%' || $3::text || '%'
           OR r.code ILIKE '%' || $3::text || '%'
           OR r.name ILIKE '%' || $3::text || '%'
           OR ms.hostname ILIKE '%' || $3::text || '%')
      AND ($4::text IS NULL
           OR ($4::text = 'missing' AND ms.id IS NULL)
           OR ms.health_status = $4::text)";

const MACHINE_SELECT: &str = "
    SELECT de.serial, de.device_slot, de.status,
           CASE WHEN ds.state = 'online' THEN 'online'
                WHEN ms.collected_at IS NOT NULL
                 AND ms.collected_at >= now() - interval '5 minutes' THEN 'online'
                ELSE ds.state END AS online,
           ds.state AS session_online,
           GREATEST(ds.updated_at, ms.received_at) AS last_seen_at,
           r.id AS room_id, r.code AS room_code, r.name AS room_name,
           a.id AS area_id, a.code AS area_code, a.name AS area_name,
           ms.id AS report_id, ms.collected_at, ms.received_at, ms.agent_version,
           ms.hostname, ms.os_name, ms.kernel_version, ms.health_status, ms.health_score,
           ms.cpu_usage_percent, ms.memory_usage_percent, ms.disk_usage_percent, ms.uptime_secs,
           (ms.collected_at IS NOT NULL
            AND ms.collected_at < now() - interval '5 minutes') AS report_stale";

const LATEST_CTE: &str = "WITH latest AS (
    SELECT DISTINCT ON (serial) *
      FROM machine_snapshots
     ORDER BY serial, collected_at DESC
  )";

/// PostgreSQL 适配器：只负责 SQL。
#[derive(Clone)]
pub struct MachineRepo {
    db: PgPool,
}

impl MachineRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 按槽位解析已注册设备（只接受 `issued` / `active`）。
    pub async fn resolve_enrollment(&self, device_slot: &str) -> DbResult<Option<EnrollmentRef>> {
        let row = sqlx::query_as::<_, EnrollmentRef>(
            "SELECT serial, device_slot, status
               FROM device_enrollment
              WHERE device_slot = $1
                AND status IN ('issued', 'active')
              ORDER BY issued_at DESC NULLS LAST
              LIMIT 1",
        )
        .bind(device_slot)
        .fetch_optional(&self.db)
        .await?;
        Ok(row)
    }

    /// 插入快照（幂等）并裁剪该 serial 的历史，返回落库后的元数据。
    pub async fn record(
        &self,
        snapshot: &NewMachineSnapshot,
        keep: i64,
    ) -> DbResult<MachineSnapshotMeta> {
        let inserted = sqlx::query_as::<_, MachineSnapshotMeta>(&format!(
            "INSERT INTO machine_snapshots
                 (serial, device_slot, collected_at, agent_version, hostname, os_name,
                  kernel_version, health_status, health_score, cpu_usage_percent,
                  memory_usage_percent, disk_usage_percent, uptime_secs, payload, tier)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (serial, collected_at) DO NOTHING
             RETURNING {SNAPSHOT_COLUMNS}"
        ))
        .bind(&snapshot.serial)
        .bind(&snapshot.device_slot)
        .bind(snapshot.collected_at)
        .bind(&snapshot.agent_version)
        .bind(&snapshot.hostname)
        .bind(&snapshot.os_name)
        .bind(&snapshot.kernel_version)
        .bind(&snapshot.health_status)
        .bind(snapshot.health_score)
        .bind(snapshot.cpu_usage_percent)
        .bind(snapshot.memory_usage_percent)
        .bind(snapshot.disk_usage_percent)
        .bind(snapshot.uptime_secs)
        .bind(&snapshot.payload)
        .bind(snapshot.tier.as_str())
        .fetch_optional(&self.db)
        .await?;

        let meta = match inserted {
            Some(meta) => meta,
            None => {
                sqlx::query_as::<_, MachineSnapshotMeta>(&format!(
                    "SELECT {SNAPSHOT_COLUMNS}
                       FROM machine_snapshots
                      WHERE serial = $1 AND collected_at = $2"
                ))
                .bind(&snapshot.serial)
                .bind(snapshot.collected_at)
                .fetch_one(&self.db)
                .await?
            }
        };

        // 裁剪：每个 serial 只保留最近 keep 条。keep 在服务端已做范围校验。
        sqlx::query(
            "DELETE FROM machine_snapshots
              WHERE serial = $1
                AND id NOT IN (
                    SELECT id FROM machine_snapshots
                     WHERE serial = $1
                     ORDER BY collected_at DESC
                     LIMIT $2
                )",
        )
        .bind(&snapshot.serial)
        .bind(keep)
        .execute(&self.db)
        .await?;

        Ok(meta)
    }

    /// 机器分页：设备 + 房间/区域 + 最新快照摘要。
    pub async fn list(
        &self,
        filter: &MachineFilter,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<MachineSummary>> {
        let offset = (page - 1) * page_size;
        let rows = sqlx::query_as::<_, MachineSummaryRow>(&format!(
            "{LATEST_CTE} {MACHINE_SELECT} {MACHINE_FROM} {MACHINE_FILTER}
             ORDER BY de.device_slot, de.serial
             LIMIT $5 OFFSET $6"
        ))
        .bind(filter.area_id)
        .bind(filter.room_id)
        .bind(&filter.keyword)
        .bind(&filter.health)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.db)
        .await?;

        let total = sqlx::query_scalar::<_, i64>(&format!(
            "{LATEST_CTE}
             SELECT count(*) {MACHINE_FROM} {MACHINE_FILTER}"
        ))
        .bind(filter.area_id)
        .bind(filter.room_id)
        .bind(&filter.keyword)
        .bind(&filter.health)
        .fetch_one(&self.db)
        .await?;

        Ok(Page::new(
            rows.into_iter().map(MachineSummary::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    /// 单台机器的摘要；不存在返回 `None`。
    pub async fn summary(&self, serial: &str) -> DbResult<Option<MachineSummary>> {
        let row = sqlx::query_as::<_, MachineSummaryRow>(&format!(
            "{LATEST_CTE} {MACHINE_SELECT} {MACHINE_FROM}
             WHERE de.serial = $1"
        ))
        .bind(serial)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(MachineSummary::from))
    }

    /// 最新一次完整快照（含 payload）。
    pub async fn latest(&self, serial: &str) -> DbResult<Option<MachineSnapshotRecord>> {
        let row = sqlx::query_as::<_, MachineSnapshotRecord>(&format!(
            "SELECT {SNAPSHOT_COLUMNS}, payload
               FROM machine_snapshots
              WHERE serial = $1
              ORDER BY collected_at DESC
              LIMIT 1"
        ))
        .bind(serial)
        .fetch_optional(&self.db)
        .await?;
        Ok(row)
    }

    /// 当前最大快照 id（SSE 首次连接时作为「从现在开始」的游标）。
    pub async fn max_snapshot_id(&self) -> DbResult<i64> {
        let id = sqlx::query_scalar::<_, Option<i64>>("SELECT max(id) FROM machine_snapshots")
            .fetch_one(&self.db)
            .await?;
        Ok(id.unwrap_or(0))
    }

    /// 按 id 增量查询快照元数据（SSE 实时流使用；`serial` 为空表示所有机器）。
    /// 返回 id 倒序，调用方需要按 id 正序发送。
    pub async fn list_after_id(
        &self,
        after_id: i64,
        serial: Option<&str>,
        limit: i64,
    ) -> DbResult<Vec<MachineSnapshotMeta>> {
        let rows = sqlx::query_as::<_, MachineSnapshotMeta>(&format!(
            "SELECT {SNAPSHOT_COLUMNS}
               FROM machine_snapshots
              WHERE id > $1
                AND ($3::text IS NULL OR serial = $3)
              ORDER BY id DESC
              LIMIT $2"
        ))
        .bind(after_id)
        .bind(limit)
        .bind(serial)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// 快照历史元数据（collected_at 倒序）。
    pub async fn history(&self, serial: &str, limit: i64) -> DbResult<Vec<MachineSnapshotMeta>> {
        let rows = sqlx::query_as::<_, MachineSnapshotMeta>(&format!(
            "SELECT {SNAPSHOT_COLUMNS}
               FROM machine_snapshots
              WHERE serial = $1
              ORDER BY collected_at DESC
              LIMIT $2"
        ))
        .bind(serial)
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }
}
