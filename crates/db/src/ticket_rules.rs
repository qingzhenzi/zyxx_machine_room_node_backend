//! 工单自动建单规则仓储（P1 Wave 1，契约 §2）。
//!
//! 规则由 worker 每 tick 读取：禁用/删除后对应信号不再自动建单（最迟一个 tick 生效）。
//! 阈值/范围/存在性校验在 API 层完成（`qe_domain::ticket` 的纯函数）。

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::ticket::{
    TicketPriority, TicketRule, TicketRuleInput, TicketRulePatch, TicketScopeType, TicketSignalType,
};

use crate::error::{DbError, DbResult};

const RULE_COLUMNS: &str = "id, name, signal_type, enabled, priority, scope_type, scope_id, \
                            thresholds, created_at, updated_at";

#[derive(Debug, FromRow)]
struct TicketRuleRow {
    id: Uuid,
    name: String,
    signal_type: String,
    enabled: bool,
    priority: String,
    scope_type: String,
    scope_id: Option<Uuid>,
    thresholds: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TicketRuleRow> for TicketRule {
    fn from(row: TicketRuleRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            signal_type: row
                .signal_type
                .parse()
                .unwrap_or(TicketSignalType::MachineHealth),
            enabled: row.enabled,
            priority: row.priority.parse().unwrap_or(TicketPriority::Normal),
            scope_type: row.scope_type.parse().unwrap_or(TicketScopeType::Global),
            scope_id: row.scope_id,
            thresholds: row.thresholds,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Clone)]
pub struct TicketRuleRepo {
    db: PgPool,
}

impl TicketRuleRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 全量规则（按 signal_type, name 排序）。
    pub async fn list(&self) -> DbResult<Vec<TicketRule>> {
        let rows = sqlx::query_as::<_, TicketRuleRow>(&format!(
            "SELECT {RULE_COLUMNS} FROM ticket_rules ORDER BY signal_type, name"
        ))
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(TicketRule::from).collect())
    }

    /// 仅启用规则（worker 每 tick 读取）。
    pub async fn list_enabled(&self) -> DbResult<Vec<TicketRule>> {
        let rows = sqlx::query_as::<_, TicketRuleRow>(&format!(
            "SELECT {RULE_COLUMNS} FROM ticket_rules WHERE enabled ORDER BY signal_type, name"
        ))
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(TicketRule::from).collect())
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<TicketRule>> {
        let row = sqlx::query_as::<_, TicketRuleRow>(&format!(
            "SELECT {RULE_COLUMNS} FROM ticket_rules WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(TicketRule::from))
    }

    pub async fn create(&self, input: &TicketRuleInput) -> DbResult<TicketRule> {
        let row = sqlx::query_as::<_, TicketRuleRow>(&format!(
            "INSERT INTO ticket_rules (name, signal_type, enabled, priority, scope_type,
                                       scope_id, thresholds)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING {RULE_COLUMNS}"
        ))
        .bind(&input.name)
        .bind(input.signal_type.as_str())
        .bind(input.enabled)
        .bind(input.priority.as_str())
        .bind(input.scope_type.as_str())
        .bind(input.scope_id)
        .bind(&input.thresholds)
        .fetch_one(&self.db)
        .await?;
        Ok(TicketRule::from(row))
    }

    /// 部分更新：`None` 字段保持原值；`scope_id = Some(None)` 清空。
    pub async fn update(&self, id: Uuid, patch: &TicketRulePatch) -> DbResult<TicketRule> {
        let mut tx = self.db.begin().await?;
        let current = sqlx::query_as::<_, TicketRuleRow>(&format!(
            "SELECT {RULE_COLUMNS} FROM ticket_rules WHERE id = $1 FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::not_found("规则不存在"))?;
        let current = TicketRule::from(current);

        let scope_type = patch.scope_type.unwrap_or(current.scope_type);
        let scope_id = match patch.scope_id {
            Some(value) => value,
            None => current.scope_id,
        };
        // global 范围不允许残留 scope_id。
        let scope_id = if scope_type == TicketScopeType::Global {
            None
        } else {
            scope_id
        };

        let row = sqlx::query_as::<_, TicketRuleRow>(&format!(
            "UPDATE ticket_rules
                SET name = COALESCE($2, name),
                    enabled = COALESCE($3, enabled),
                    priority = COALESCE($4, priority),
                    scope_type = $5,
                    scope_id = $6,
                    thresholds = COALESCE($7, thresholds)
              WHERE id = $1
            RETURNING {RULE_COLUMNS}"
        ))
        .bind(id)
        .bind(patch.name.as_deref())
        .bind(patch.enabled)
        .bind(patch.priority.map(TicketPriority::as_str))
        .bind(scope_type.as_str())
        .bind(scope_id)
        .bind(patch.thresholds.as_ref())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(TicketRule::from(row))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let deleted = sqlx::query("DELETE FROM ticket_rules WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}
