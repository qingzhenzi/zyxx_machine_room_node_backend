//! 巡检计划仓储（Wave 3）：计划 CRUD、原子领取到期计划、执行记录与 scope 内机器摘要。

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::ticket::{
    InspectionLastRun, InspectionMachineSummary, InspectionPlan, InspectionPlanInput,
    InspectionPlanPatch, InspectionPlanView, InspectionRun, InspectionRunStatus, TicketScopeType,
};

use crate::error::{DbError, DbResult};

const PLAN_COLUMNS: &str = "p.id, p.name, p.scope_type, p.scope_id, p.interval_hours, p.enabled, \
     p.created_by, u.display_name AS created_by_name, p.last_run_at, p.next_run_at, \
     p.created_at, p.updated_at, \
     lr.status AS last_status, lr.started_at AS last_started_at, \
     lr.issue_machines AS last_issue_machines, lr.ticket_id AS last_ticket_id";

const PLAN_FROM: &str = "FROM inspection_plans p \
     LEFT JOIN users u ON u.id = p.created_by \
     LEFT JOIN LATERAL ( \
         SELECT r.status, r.started_at, r.issue_machines, r.ticket_id \
           FROM inspection_runs r \
          WHERE r.plan_id = p.id \
          ORDER BY r.started_at DESC, r.id DESC \
          LIMIT 1 \
     ) lr ON TRUE";

#[derive(Debug, FromRow)]
struct PlanRow {
    id: Uuid,
    name: String,
    scope_type: String,
    scope_id: Option<Uuid>,
    interval_hours: i32,
    enabled: bool,
    created_by: Option<Uuid>,
    created_by_name: Option<String>,
    last_run_at: Option<DateTime<Utc>>,
    next_run_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_status: Option<String>,
    last_started_at: Option<DateTime<Utc>>,
    last_issue_machines: Option<i32>,
    last_ticket_id: Option<Uuid>,
}

impl From<PlanRow> for InspectionPlanView {
    fn from(row: PlanRow) -> Self {
        let last_run = match (row.last_status, row.last_started_at) {
            (Some(status), Some(started_at)) => Some(InspectionLastRun {
                status: status.parse().unwrap_or(InspectionRunStatus::Failed),
                started_at,
                issue_machines: i64::from(row.last_issue_machines.unwrap_or(0)),
                ticket_id: row.last_ticket_id,
            }),
            _ => None,
        };
        Self {
            plan: InspectionPlan {
                id: row.id,
                name: row.name,
                scope_type: row.scope_type.parse().unwrap_or(TicketScopeType::Global),
                scope_id: row.scope_id,
                interval_hours: i64::from(row.interval_hours),
                enabled: row.enabled,
                created_by: row.created_by,
                created_by_name: row.created_by_name,
                last_run_at: row.last_run_at,
                next_run_at: row.next_run_at,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
            last_run,
        }
    }
}

#[derive(Debug, FromRow)]
struct PlanCoreRow {
    id: Uuid,
    name: String,
    scope_type: String,
    scope_id: Option<Uuid>,
    interval_hours: i32,
    enabled: bool,
    created_by: Option<Uuid>,
    last_run_at: Option<DateTime<Utc>>,
    next_run_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<PlanCoreRow> for InspectionPlan {
    fn from(row: PlanCoreRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            scope_type: row.scope_type.parse().unwrap_or(TicketScopeType::Global),
            scope_id: row.scope_id,
            interval_hours: i64::from(row.interval_hours),
            enabled: row.enabled,
            created_by: row.created_by,
            created_by_name: None,
            last_run_at: row.last_run_at,
            next_run_at: row.next_run_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct RunRow {
    id: Uuid,
    plan_id: Uuid,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    status: String,
    checked_machines: i32,
    issue_machines: i32,
    ticket_id: Option<Uuid>,
    summary: Value,
    error: Option<String>,
}

impl From<RunRow> for InspectionRun {
    fn from(row: RunRow) -> Self {
        Self {
            id: row.id,
            plan_id: row.plan_id,
            started_at: row.started_at,
            finished_at: row.finished_at,
            status: row.status.parse().unwrap_or(InspectionRunStatus::Failed),
            checked_machines: i64::from(row.checked_machines),
            issue_machines: i64::from(row.issue_machines),
            ticket_id: row.ticket_id,
            summary: row.summary,
            error: row.error,
        }
    }
}

#[derive(Debug, FromRow)]
struct MachineSummaryRow {
    serial: String,
    device_slot: String,
    room_id: Option<Uuid>,
    room_name: Option<String>,
    area_id: Option<Uuid>,
    health_status: Option<String>,
    health_score: Option<i32>,
    issues: Value,
    collected_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct TicketInspectionRepo {
    db: PgPool,
}

impl TicketInspectionRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 全部计划（含最近一次执行摘要），按创建时间倒序。
    pub async fn list(&self) -> DbResult<Vec<InspectionPlanView>> {
        let rows = sqlx::query_as::<_, PlanRow>(&format!(
            "SELECT {PLAN_COLUMNS} {PLAN_FROM} ORDER BY p.created_at DESC, p.id"
        ))
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(InspectionPlanView::from).collect())
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<InspectionPlanView>> {
        let row = sqlx::query_as::<_, PlanRow>(&format!(
            "SELECT {PLAN_COLUMNS} {PLAN_FROM} WHERE p.id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(InspectionPlanView::from))
    }

    pub async fn create(
        &self,
        input: &InspectionPlanInput,
        created_by: Uuid,
    ) -> DbResult<InspectionPlan> {
        let row = sqlx::query_as::<_, PlanCoreRow>(
            "INSERT INTO inspection_plans
                 (name, scope_type, scope_id, interval_hours, enabled, created_by)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, name, scope_type, scope_id, interval_hours, enabled, created_by,
                       last_run_at, next_run_at, created_at, updated_at",
        )
        .bind(&input.name)
        .bind(input.scope_type.as_str())
        .bind(input.scope_id)
        .bind(input.interval_hours as i32)
        .bind(input.enabled)
        .bind(created_by)
        .fetch_one(&self.db)
        .await?;
        Ok(InspectionPlan::from(row))
    }

    /// 部分更新；`scope_type=global` 时强制清空 scope_id。
    pub async fn update(&self, id: Uuid, patch: &InspectionPlanPatch) -> DbResult<InspectionPlan> {
        let mut tx = self.db.begin().await?;
        let current = sqlx::query_as::<_, PlanCoreRow>(
            "SELECT id, name, scope_type, scope_id, interval_hours, enabled, created_by,
                    last_run_at, next_run_at, created_at, updated_at
               FROM inspection_plans WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::not_found("巡检计划不存在"))?;
        let current = InspectionPlan::from(current);

        let scope_type = patch.scope_type.unwrap_or(current.scope_type);
        let scope_id = if scope_type == TicketScopeType::Global {
            None
        } else {
            match patch.scope_id {
                Some(value) => value,
                None => current.scope_id,
            }
        };
        let row = sqlx::query_as::<_, PlanCoreRow>(
            "UPDATE inspection_plans
                SET name = COALESCE($2, name),
                    scope_type = $3,
                    scope_id = $4,
                    interval_hours = COALESCE($5, interval_hours),
                    enabled = COALESCE($6, enabled)
              WHERE id = $1
            RETURNING id, name, scope_type, scope_id, interval_hours, enabled, created_by,
                      last_run_at, next_run_at, created_at, updated_at",
        )
        .bind(id)
        .bind(patch.name.as_deref())
        .bind(scope_type.as_str())
        .bind(scope_id)
        .bind(patch.interval_hours.map(|value| value as i32))
        .bind(patch.enabled)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(InspectionPlan::from(row))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let deleted = sqlx::query("DELETE FROM inspection_plans WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    /// 原子领取到期计划：`FOR UPDATE SKIP LOCKED` + 前移 next_run_at，多副本安全。
    pub async fn claim_due_plans(&self, limit: i64) -> DbResult<Vec<InspectionPlan>> {
        let rows = sqlx::query_as::<_, PlanCoreRow>(
            "UPDATE inspection_plans p
                SET next_run_at = now() + make_interval(hours => p.interval_hours)
              WHERE p.id IN (
                    SELECT id
                      FROM inspection_plans
                     WHERE enabled
                       AND (next_run_at IS NULL OR next_run_at <= now())
                     ORDER BY next_run_at NULLS FIRST, created_at
                     LIMIT $1
                     FOR UPDATE SKIP LOCKED
              )
            RETURNING p.id, p.name, p.scope_type, p.scope_id, p.interval_hours, p.enabled,
                      p.created_by, p.last_run_at, p.next_run_at, p.created_at, p.updated_at",
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(InspectionPlan::from).collect())
    }

    /// 开启一次执行（status=running）。
    pub async fn insert_run(&self, plan_id: Uuid) -> DbResult<InspectionRun> {
        let row = sqlx::query_as::<_, RunRow>(
            "INSERT INTO inspection_runs (plan_id, status)
             VALUES ($1, 'running')
             RETURNING id, plan_id, started_at, finished_at, status, checked_machines,
                       issue_machines, ticket_id, summary, error",
        )
        .bind(plan_id)
        .fetch_one(&self.db)
        .await?;
        Ok(InspectionRun::from(row))
    }

    /// 结束执行：写 run 结果 + 更新计划 last_run_at。
    #[allow(clippy::too_many_arguments)]
    pub async fn finish_run(
        &self,
        run_id: Uuid,
        plan_id: Uuid,
        status: InspectionRunStatus,
        checked_machines: i64,
        issue_machines: i64,
        ticket_id: Option<Uuid>,
        summary: &Value,
        error: Option<&str>,
    ) -> DbResult<InspectionRun> {
        let mut tx = self.db.begin().await?;
        let row = sqlx::query_as::<_, RunRow>(
            "UPDATE inspection_runs
                SET finished_at = now(),
                    status = $2,
                    checked_machines = $3,
                    issue_machines = $4,
                    ticket_id = $5,
                    summary = $6,
                    error = $7
              WHERE id = $1
            RETURNING id, plan_id, started_at, finished_at, status, checked_machines,
                      issue_machines, ticket_id, summary, error",
        )
        .bind(run_id)
        .bind(status.as_str())
        .bind(checked_machines as i32)
        .bind(issue_machines as i32)
        .bind(ticket_id)
        .bind(summary)
        .bind(error)
        .fetch_one(&mut *tx)
        .await?;
        // 手动执行也要把计划推到下一个周期，避免 next_run_at 为 NULL 时 worker 立刻重复执行。
        sqlx::query(
            "UPDATE inspection_plans
                SET last_run_at = now(),
                    next_run_at = now() + make_interval(hours => interval_hours)
              WHERE id = $1",
        )
        .bind(plan_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(InspectionRun::from(row))
    }

    /// 执行历史（plan_id 为空 = 全部计划），按 started_at 倒序。
    pub async fn list_runs(
        &self,
        plan_id: Option<Uuid>,
        limit: i64,
    ) -> DbResult<Vec<InspectionRun>> {
        let rows = sqlx::query_as::<_, RunRow>(
            "SELECT id, plan_id, started_at, finished_at, status, checked_machines,
                    issue_machines, ticket_id, summary, error
               FROM inspection_runs
              WHERE ($2::uuid IS NULL OR plan_id = $2)
              ORDER BY started_at DESC, id DESC
              LIMIT $1",
        )
        .bind(limit)
        .bind(plan_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(InspectionRun::from).collect())
    }

    /// scope 内每台机器的最新快照摘要。
    pub async fn machine_summary_in_scope(
        &self,
        scope_type: TicketScopeType,
        scope_id: Option<Uuid>,
    ) -> DbResult<Vec<InspectionMachineSummary>> {
        let rows = sqlx::query_as::<_, MachineSummaryRow>(
            "WITH mapping AS (
                 SELECT de.serial,
                        de.device_slot,
                        COALESCE(rd.room_id, r1.id) AS room_id
                   FROM device_enrollment de
                   LEFT JOIN room_devices rd ON rd.device_slot = de.device_slot
                   LEFT JOIN rooms r1 ON r1.device_slot = de.device_slot
             ),
             scoped AS (
                 SELECT m.serial, m.device_slot, m.room_id
                   FROM mapping m
                   LEFT JOIN rooms r ON r.id = m.room_id
                  WHERE $1::text = 'global'
                     OR ($1::text = 'area' AND r.area_id = $2)
                     OR ($1::text = 'room' AND m.room_id = $2)
             )
             SELECT s.serial,
                    s.device_slot,
                    s.room_id,
                    r.name AS room_name,
                    r.area_id,
                    latest.health_status,
                    latest.health_score,
                    COALESCE(latest.payload->'health'->'issues', '[]'::jsonb) AS issues,
                    latest.collected_at
               FROM scoped s
               LEFT JOIN rooms r ON r.id = s.room_id
               LEFT JOIN LATERAL (
                   SELECT ms.health_status, ms.health_score, ms.collected_at, ms.payload
                     FROM machine_snapshots ms
                    WHERE ms.serial = s.serial
                    ORDER BY ms.collected_at DESC
                    LIMIT 1
               ) latest ON TRUE
              ORDER BY s.serial",
        )
        .bind(scope_type.as_str())
        .bind(scope_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| InspectionMachineSummary {
                serial: row.serial,
                device_slot: row.device_slot,
                room_id: row.room_id,
                room_name: row.room_name,
                area_id: row.area_id,
                health_status: row.health_status,
                health_score: row.health_score,
                issues: row.issues.as_array().cloned().unwrap_or_default(),
                collected_at: row.collected_at,
            })
            .collect())
    }
}
