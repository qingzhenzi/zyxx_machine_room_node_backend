//! P2 节点配置仓储：作用域配置的读写与 global → area → room 合并。
//!
//! 契约见 `docs/storage-design.md` §17.2；表结构见 `migrations/0012_agent_config.sql`。
//!
//! 约定：
//! - `agent_configs.policy` 保存的是 [`AgentPolicyPatch`] 的 JSONB 序列化，不是完整策略；
//! - `global` 行的 `scope_id` 为 `NULL`，`area` / `room` 行必须非空；
//! - upsert 依赖 0012 的两个部分唯一索引，存在时 `revision + 1`；
//! - 合并顺序固定为 global → area → room，缺失的作用域行直接跳过。

use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::agent_config::{
    AgentConfig, AgentPolicy, AgentPolicyPatch, ConfigScopeType, EffectiveAgentPolicy,
    validate_policy_patch,
};

use crate::error::{DbError, DbResult};

/// 配置行的通用列（`list` / `upsert` 共用）。
const CONFIG_COLUMNS: &str = "id, scope_type, scope_id, policy, revision,
     updated_by, created_at, updated_at";

#[derive(Debug, Clone, FromRow)]
struct AgentConfigRow {
    id: Uuid,
    scope_type: String,
    scope_id: Option<Uuid>,
    policy: Value,
    revision: i64,
    updated_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl AgentConfigRow {
    fn into_domain(self) -> DbResult<AgentConfig> {
        let scope_type = ConfigScopeType::from_str(&self.scope_type).map_err(|_| {
            DbError::internal(format!(
                "agent_configs.scope_type 非法: {}",
                self.scope_type
            ))
        })?;
        let policy = serde_json::from_value::<AgentPolicyPatch>(self.policy)
            .map_err(|error| DbError::internal(format!("agent_configs.policy 非法: {error}")))?;
        Ok(AgentConfig {
            id: self.id,
            scope_type,
            scope_id: self.scope_id,
            policy,
            revision: self.revision,
            updated_by: self.updated_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// `effective_for_room` 只需要的列。
#[derive(Debug, Clone, FromRow)]
struct EffectiveRow {
    scope_type: String,
    policy: Value,
    revision: i64,
}

/// 校验作用域与 `scope_id` 的搭配；同时兜住数据库 CHECK 约束（避免只报 23514）。
fn ensure_scope(scope_type: ConfigScopeType, scope_id: Option<Uuid>) -> DbResult<()> {
    match (scope_type, scope_id) {
        (ConfigScopeType::Global, None) => Ok(()),
        (ConfigScopeType::Global, Some(_)) => {
            Err(DbError::bad_request("global 作用域不能指定 scope_id"))
        }
        (ConfigScopeType::Area | ConfigScopeType::Room, None) => {
            Err(DbError::bad_request("area / room 作用域必须指定 scope_id"))
        }
        (ConfigScopeType::Area | ConfigScopeType::Room, Some(_)) => Ok(()),
    }
}

#[derive(Clone)]
pub struct AgentConfigRepo {
    db: PgPool,
}

impl AgentConfigRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 列出所有作用域配置：global → area → room，同作用域按 `scope_id` 排序。
    pub async fn list(&self) -> DbResult<Vec<AgentConfig>> {
        let sql = format!(
            "SELECT {CONFIG_COLUMNS}
             FROM agent_configs
             ORDER BY CASE scope_type
                          WHEN 'global' THEN 0
                          WHEN 'area' THEN 1
                          ELSE 2
                      END,
                      scope_id NULLS FIRST,
                      created_at"
        );
        let rows = sqlx::query_as::<_, AgentConfigRow>(&sql)
            .fetch_all(&self.db)
            .await?;
        rows.into_iter().map(AgentConfigRow::into_domain).collect()
    }

    /// upsert 一个作用域的 patch：不存在则 `revision = 1`，存在则 `revision + 1`。
    ///
    /// 两个分支分别匹配 0012 的部分唯一索引：
    /// - `scope_id IS NULL` → `agent_configs_global_uniq`；
    /// - `scope_id IS NOT NULL` → `agent_configs_scoped_uniq`。
    pub async fn upsert(
        &self,
        scope_type: ConfigScopeType,
        scope_id: Option<Uuid>,
        patch: &AgentPolicyPatch,
        actor: Uuid,
    ) -> DbResult<AgentConfig> {
        ensure_scope(scope_type, scope_id)?;
        validate_policy_patch(patch).map_err(|error| DbError::bad_request(error.to_string()))?;
        let policy = serde_json::to_value(patch)
            .map_err(|error| DbError::internal(format!("序列化配置 patch 失败: {error}")))?;

        let sql = if scope_id.is_some() {
            format!(
                "INSERT INTO agent_configs (scope_type, scope_id, policy, revision, updated_by)
                 VALUES ($1, $2, $3, 1, $4)
                 ON CONFLICT (scope_type, scope_id) WHERE scope_id IS NOT NULL DO UPDATE
                 SET policy = EXCLUDED.policy,
                     revision = agent_configs.revision + 1,
                     updated_by = EXCLUDED.updated_by,
                     updated_at = now()
                 RETURNING {CONFIG_COLUMNS}"
            )
        } else {
            format!(
                "INSERT INTO agent_configs (scope_type, scope_id, policy, revision, updated_by)
                 VALUES ($1, $2, $3, 1, $4)
                 ON CONFLICT (scope_type) WHERE scope_id IS NULL DO UPDATE
                 SET policy = EXCLUDED.policy,
                     revision = agent_configs.revision + 1,
                     updated_by = EXCLUDED.updated_by,
                     updated_at = now()
                 RETURNING {CONFIG_COLUMNS}"
            )
        };

        let row = sqlx::query_as::<_, AgentConfigRow>(&sql)
            .bind(scope_type.as_str())
            .bind(scope_id)
            .bind(policy)
            .bind(actor)
            .fetch_one(&self.db)
            .await?;
        row.into_domain()
    }

    /// 合并某房间的生效策略：global → area(room.area_id) → room。
    ///
    /// 房间不存在时（或作用域没有配置行）跳过对应层，只应用 global；
    /// `revision` 取实际应用到的行的最大值，没有任何行时为 0。
    pub async fn effective_for_room(&self, room_id: Uuid) -> DbResult<EffectiveAgentPolicy> {
        let area_id: Option<Uuid> = sqlx::query_scalar("SELECT area_id FROM rooms WHERE id = $1")
            .bind(room_id)
            .fetch_optional(&self.db)
            .await?;
        // 房间不存在时，即便有同 id 的孤儿配置行也不应用。
        let room_scope = area_id.map(|_| room_id);

        let rows = sqlx::query_as::<_, EffectiveRow>(
            "SELECT scope_type, policy, revision
             FROM agent_configs
             WHERE (scope_type = 'global' AND scope_id IS NULL)
                OR ($1::uuid IS NOT NULL AND scope_type = 'area' AND scope_id = $1)
                OR ($2::uuid IS NOT NULL AND scope_type = 'room' AND scope_id = $2)
             ORDER BY CASE scope_type
                          WHEN 'global' THEN 0
                          WHEN 'area' THEN 1
                          ELSE 2
                      END",
        )
        .bind(area_id)
        .bind(room_scope)
        .fetch_all(&self.db)
        .await?;

        let mut policy = AgentPolicy::default();
        let mut revision = 0i64;
        for row in rows {
            // 约束保证只有三个合法值；异常数据按内部错误处理，而不是静默忽略。
            ConfigScopeType::from_str(&row.scope_type).map_err(|_| {
                DbError::internal(format!("agent_configs.scope_type 非法: {}", row.scope_type))
            })?;
            let patch =
                serde_json::from_value::<AgentPolicyPatch>(row.policy).map_err(|error| {
                    DbError::internal(format!("agent_configs.policy 非法: {error}"))
                })?;
            patch.apply_to(&mut policy);
            revision = revision.max(row.revision);
        }

        Ok(EffectiveAgentPolicy { policy, revision })
    }

    /// 某作用域受影响的房间 id。
    ///
    /// - global：所有绑定了 `device_slot` 的房间；
    /// - area：该区域所有房间（含未绑定设备的房间，调用方自行判断）；
    /// - room：该房间本身。`area` / `room` 不存在时返回 [`DbError::NotFound`]。
    pub async fn rooms_for_scope(
        &self,
        scope_type: ConfigScopeType,
        scope_id: Option<Uuid>,
    ) -> DbResult<Vec<Uuid>> {
        ensure_scope(scope_type, scope_id)?;
        match scope_type {
            ConfigScopeType::Global => sqlx::query_scalar(
                "SELECT id FROM rooms
                     WHERE device_slot IS NOT NULL
                     ORDER BY code, id",
            )
            .fetch_all(&self.db)
            .await
            .map_err(DbError::from),
            ConfigScopeType::Area => {
                let area_id =
                    scope_id.ok_or_else(|| DbError::bad_request("area 作用域必须指定 scope_id"))?;
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM areas WHERE id = $1)")
                        .bind(area_id)
                        .fetch_one(&self.db)
                        .await?;
                if !exists {
                    return Err(DbError::not_found("区域不存在"));
                }
                sqlx::query_scalar("SELECT id FROM rooms WHERE area_id = $1 ORDER BY code, id")
                    .bind(area_id)
                    .fetch_all(&self.db)
                    .await
                    .map_err(DbError::from)
            }
            ConfigScopeType::Room => {
                let room_id =
                    scope_id.ok_or_else(|| DbError::bad_request("room 作用域必须指定 scope_id"))?;
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM rooms WHERE id = $1)")
                        .bind(room_id)
                        .fetch_one(&self.db)
                        .await?;
                if !exists {
                    return Err(DbError::not_found("房间不存在"));
                }
                Ok(vec![room_id])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_scope_id_must_match() {
        assert!(ensure_scope(ConfigScopeType::Global, None).is_ok());
        assert!(ensure_scope(ConfigScopeType::Global, Some(Uuid::nil())).is_err());
        assert!(ensure_scope(ConfigScopeType::Area, None).is_err());
        assert!(ensure_scope(ConfigScopeType::Room, None).is_err());
        assert!(ensure_scope(ConfigScopeType::Area, Some(Uuid::nil())).is_ok());
        assert!(ensure_scope(ConfigScopeType::Room, Some(Uuid::nil())).is_ok());
    }

    #[test]
    fn config_row_maps_patch() {
        let id = Uuid::new_v4();
        let row = AgentConfigRow {
            id,
            scope_type: "room".to_owned(),
            scope_id: Some(Uuid::nil()),
            policy: serde_json::json!({ "lxc_auto_start": true }),
            revision: 3,
            updated_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let config = row.into_domain().expect("config");
        assert_eq!(config.id, id);
        assert_eq!(config.scope_type, ConfigScopeType::Room);
        assert_eq!(config.policy.lxc_auto_start, Some(true));
        assert_eq!(config.revision, 3);
    }
}
