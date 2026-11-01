//! P2 节点配置仓储：作用域配置的读写与 global → area → room → device 合并。
//!
//! 契约见 `docs/storage-design.md` §17.2 与 `docs/machine-telemetry.md` v1.1；
//! 表结构见 `migrations/0012_agent_config.sql` + `migrations/0017_agent_config_device.sql`。
//!
//! 约定：
//! - `agent_configs.policy` 保存的是 [`AgentPolicyPatch`] 的 JSONB 序列化，不是完整策略；
//! - `global` 行的 `scope_id` / `scope_key` 均为 `NULL`；`area` / `room` 行必须有
//!   `scope_id` 且 `scope_key` 为 `NULL`；`device` 行 `scope_id` 为 `NULL`、
//!   `scope_key` 必须是非空 `device_slot`；
//! - upsert 依赖 0012 / 0017 的部分唯一索引，存在时 `revision + 1`；
//! - 合并顺序固定为 global → area → room → device，缺失的作用域行直接跳过；
//! - device 层随机器不同而不同：`effective_for_room` 保持「只到 room」语义，
//!   需要设备级覆盖时用 [`AgentConfigRepo::effective_for_slot`]。

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
const CONFIG_COLUMNS: &str = "id, scope_type, scope_id, scope_key, policy, revision,
     updated_by, created_at, updated_at";

#[derive(Debug, Clone, FromRow)]
struct AgentConfigRow {
    id: Uuid,
    scope_type: String,
    scope_id: Option<Uuid>,
    scope_key: Option<String>,
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
            scope_key: self.scope_key,
            policy,
            revision: self.revision,
            updated_by: self.updated_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// 合并查询只需要的作用域 + patch 列。
#[derive(Debug, Clone, FromRow)]
struct EffectiveRow {
    scope_type: String,
    scope_id: Option<Uuid>,
    scope_key: Option<String>,
    policy: Value,
    revision: i64,
}

/// 校验作用域与 `scope_id` / `scope_key` 的搭配；同时兜住数据库 CHECK 约束
/// （避免只报 23514）。
fn ensure_scope(
    scope_type: ConfigScopeType,
    scope_id: Option<Uuid>,
    scope_key: Option<&str>,
) -> DbResult<()> {
    match (scope_type, scope_id, scope_key) {
        (ConfigScopeType::Global, None, None) => Ok(()),
        (ConfigScopeType::Global, _, _) => Err(DbError::bad_request(
            "global 作用域不能指定 scope_id / scope_key",
        )),
        (ConfigScopeType::Area | ConfigScopeType::Room, Some(_), None) => Ok(()),
        (ConfigScopeType::Area | ConfigScopeType::Room, None, _) => {
            Err(DbError::bad_request("area / room 作用域必须指定 scope_id"))
        }
        (ConfigScopeType::Area | ConfigScopeType::Room, Some(_), Some(_)) => {
            Err(DbError::bad_request("area / room 作用域不能指定 scope_key"))
        }
        (ConfigScopeType::Device, None, Some(key)) if !key.trim().is_empty() => Ok(()),
        (ConfigScopeType::Device, None, Some(_)) => {
            Err(DbError::bad_request("device 作用域 scope_key 不能为空"))
        }
        (ConfigScopeType::Device, Some(_), _) => {
            Err(DbError::bad_request("device 作用域不能指定 scope_id"))
        }
        (ConfigScopeType::Device, None, None) => {
            Err(DbError::bad_request("device 作用域必须指定 device_slot"))
        }
    }
}

/// 解析合并上下文：某个设备槽位所在房间（主设备或子机）。
#[derive(Debug, Clone, Copy)]
struct SlotContext {
    room_id: Uuid,
    area_id: Uuid,
}

#[derive(Clone)]
pub struct AgentConfigRepo {
    db: PgPool,
}

impl AgentConfigRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 列出所有作用域配置：global → area → room → device，同作用域按目标排序。
    pub async fn list(&self) -> DbResult<Vec<AgentConfig>> {
        let sql = format!(
            "SELECT {CONFIG_COLUMNS}
             FROM agent_configs
             ORDER BY CASE scope_type
                          WHEN 'global' THEN 0
                          WHEN 'area' THEN 1
                          WHEN 'room' THEN 2
                          ELSE 3
                      END,
                      scope_id NULLS FIRST,
                      scope_key NULLS FIRST,
                      created_at"
        );
        let rows = sqlx::query_as::<_, AgentConfigRow>(&sql)
            .fetch_all(&self.db)
            .await?;
        rows.into_iter().map(AgentConfigRow::into_domain).collect()
    }

    /// upsert 一个作用域的 patch：不存在则 `revision = 1`，存在则 `revision + 1`。
    ///
    /// 三个分支分别匹配对应的部分唯一索引：
    /// - `scope_id IS NOT NULL` → `agent_configs_scoped_uniq`（area / room）；
    /// - `scope_type = 'device'` → `agent_configs_device_uniq`（scope_key = device_slot）；
    /// - 其余（global）→ `agent_configs_global_uniq`。
    pub async fn upsert(
        &self,
        scope_type: ConfigScopeType,
        scope_id: Option<Uuid>,
        scope_key: Option<&str>,
        patch: &AgentPolicyPatch,
        actor: Uuid,
    ) -> DbResult<AgentConfig> {
        ensure_scope(scope_type, scope_id, scope_key)?;
        validate_policy_patch(patch).map_err(|error| DbError::bad_request(error.to_string()))?;
        let policy = serde_json::to_value(patch)
            .map_err(|error| DbError::internal(format!("序列化配置 patch 失败: {error}")))?;
        let scope_key = scope_key.map(str::trim).filter(|key| !key.is_empty());

        // 三个分支共用同一 VALUES 形状（$1 scope_type, $2 scope_id, $3 scope_key,
        // $4 policy, $5 actor），只有冲突目标不同。
        let sql = if scope_id.is_some() {
            format!(
                "INSERT INTO agent_configs (scope_type, scope_id, scope_key, policy, revision, updated_by)
                 VALUES ($1, $2, $3, $4, 1, $5)
                 ON CONFLICT (scope_type, scope_id) WHERE scope_id IS NOT NULL DO UPDATE
                 SET policy = EXCLUDED.policy,
                     revision = agent_configs.revision + 1,
                     updated_by = EXCLUDED.updated_by,
                     updated_at = now()
                 RETURNING {CONFIG_COLUMNS}"
            )
        } else if scope_key.is_some() {
            format!(
                "INSERT INTO agent_configs (scope_type, scope_id, scope_key, policy, revision, updated_by)
                 VALUES ($1, $2, $3, $4, 1, $5)
                 ON CONFLICT (scope_key) WHERE scope_type = 'device' DO UPDATE
                 SET policy = EXCLUDED.policy,
                     revision = agent_configs.revision + 1,
                     updated_by = EXCLUDED.updated_by,
                     updated_at = now()
                 RETURNING {CONFIG_COLUMNS}"
            )
        } else {
            format!(
                "INSERT INTO agent_configs (scope_type, scope_id, scope_key, policy, revision, updated_by)
                 VALUES ($1, $2, $3, $4, 1, $5)
                 ON CONFLICT (scope_type) WHERE scope_id IS NULL AND scope_key IS NULL DO UPDATE
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
            .bind(scope_key)
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
    ///
    /// 注意：本方法**不应用 device 层**（契约 `GET /admin/agent-configs/effective?room_id=`
    /// 保持原语义）；需要设备级覆盖用 [`Self::effective_for_slot`]。
    pub async fn effective_for_room(&self, room_id: Uuid) -> DbResult<EffectiveAgentPolicy> {
        let area_id: Option<Uuid> = sqlx::query_scalar("SELECT area_id FROM rooms WHERE id = $1")
            .bind(room_id)
            .fetch_optional(&self.db)
            .await?;
        // 房间不存在时，即便有同 id 的孤儿配置行也不应用。
        let (area_id, room_scope) = match area_id {
            Some(area_id) => (Some(area_id), Some(room_id)),
            None => (None, None),
        };
        let rows = self.load_rows(area_id, room_scope, None).await?;
        merge_rows(rows)
    }

    /// 合并某设备槽位的生效策略：global → area → room → device。
    ///
    /// 槽位未绑定房间时只应用 global（+设备行）；槽位可能来自 `rooms.device_slot`
    /// （主设备）或 `room_devices`（子机）。
    pub async fn effective_for_slot(&self, slot: &str) -> DbResult<EffectiveAgentPolicy> {
        let context = self.slot_context(slot).await?;
        let (area_id, room_id) = match context {
            Some(context) => (Some(context.area_id), Some(context.room_id)),
            None => (None, None),
        };
        let rows = self.load_rows(area_id, room_id, Some(slot)).await?;
        merge_rows(rows)
    }

    /// 预演「写入本作用域 patch 后的完整策略」，用于管理端写入前校验
    /// （范围 + `full >= fast` 等跨字段约束），不落库。
    ///
    /// 只合并**下层**作用域（global < area < room < device），本作用域旧行被忽略，
    /// 因为 upsert 会整体替换该行的 patch。
    pub async fn preview_effective(
        &self,
        scope_type: ConfigScopeType,
        scope_id: Option<Uuid>,
        scope_key: Option<&str>,
        patch: &AgentPolicyPatch,
    ) -> DbResult<AgentPolicy> {
        ensure_scope(scope_type, scope_id, scope_key)?;
        let (area_id, room_id, device_slot) = match scope_type {
            ConfigScopeType::Global => (None, None, None),
            ConfigScopeType::Area => (scope_id, None, None),
            ConfigScopeType::Room => {
                let area_id: Option<Uuid> = match scope_id {
                    Some(room_id) => {
                        sqlx::query_scalar("SELECT area_id FROM rooms WHERE id = $1")
                            .bind(room_id)
                            .fetch_optional(&self.db)
                            .await?
                    }
                    None => None,
                };
                (area_id, scope_id, None)
            }
            ConfigScopeType::Device => {
                let slot = scope_key
                    .map(str::trim)
                    .filter(|slot| !slot.is_empty())
                    .ok_or_else(|| DbError::bad_request("device 作用域必须指定 device_slot"))?;
                let context = self.slot_context(slot).await?;
                match context {
                    Some(context) => (
                        Some(context.area_id),
                        Some(context.room_id),
                        Some(slot.to_owned()),
                    ),
                    None => (None, None, Some(slot.to_owned())),
                }
            }
        };

        let rows = self
            .load_rows(area_id, room_id, device_slot.as_deref())
            .await?;
        let mut merged = merge_rows(rows.into_iter().filter(|row| {
            // 忽略本作用域的旧行：upsert 会整体替换它。
            if row_scope_matches(row, scope_type, scope_id, scope_key) {
                return false;
            }
            true
        }))?;
        patch.apply_to(&mut merged.policy);
        merged
            .policy
            .validate()
            .map_err(|error| DbError::bad_request(format!("合并后的策略不合法: {error}")))?;
        Ok(merged.policy)
    }

    /// 某作用域受影响的房间 id。
    ///
    /// - global：所有绑定了 `device_slot` 的房间；
    /// - area：该区域所有房间（含未绑定设备的房间，调用方自行判断）；
    /// - room：该房间本身；
    /// - device：包含该槽位的房间（主设备或子机，可能为空）。
    ///
    /// `area` / `room` 不存在时返回 [`DbError::NotFound`]。
    pub async fn rooms_for_scope(
        &self,
        scope_type: ConfigScopeType,
        scope_id: Option<Uuid>,
        scope_key: Option<&str>,
    ) -> DbResult<Vec<Uuid>> {
        ensure_scope(scope_type, scope_id, scope_key)?;
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
            ConfigScopeType::Device => {
                let slot = scope_key
                    .map(str::trim)
                    .filter(|slot| !slot.is_empty())
                    .ok_or_else(|| DbError::bad_request("device 作用域必须指定 device_slot"))?;
                sqlx::query_scalar(
                    "SELECT id FROM rooms WHERE device_slot = $1
                     UNION
                     SELECT room_id FROM room_devices WHERE device_slot = $1",
                )
                .bind(slot)
                .fetch_all(&self.db)
                .await
                .map_err(DbError::from)
            }
        }
    }

    /// 一次配置变更需要下发的**全部设备槽位**（主设备 + `room_devices` 子机）。
    ///
    /// push `config.update` 时必须按槽位各自合并（含 device 层），否则 device 覆盖
    /// 会在下发时被丢掉。
    pub async fn device_slots_for_scope(
        &self,
        scope_type: ConfigScopeType,
        scope_id: Option<Uuid>,
        scope_key: Option<&str>,
    ) -> DbResult<Vec<String>> {
        ensure_scope(scope_type, scope_id, scope_key)?;
        let slots: Vec<String> = match scope_type {
            ConfigScopeType::Global => sqlx::query_scalar(
                "SELECT device_slot FROM rooms WHERE device_slot IS NOT NULL
                     UNION
                     SELECT device_slot FROM room_devices
                     ORDER BY 1",
            )
            .fetch_all(&self.db)
            .await
            .map_err(DbError::from)?,
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
                sqlx::query_scalar(
                    "SELECT r.device_slot FROM rooms r
                      WHERE r.area_id = $1 AND r.device_slot IS NOT NULL
                     UNION
                     SELECT rd.device_slot FROM room_devices rd
                       JOIN rooms r ON r.id = rd.room_id
                      WHERE r.area_id = $1
                     ORDER BY 1",
                )
                .bind(area_id)
                .fetch_all(&self.db)
                .await
                .map_err(DbError::from)?
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
                sqlx::query_scalar(
                    "SELECT device_slot FROM rooms WHERE id = $1 AND device_slot IS NOT NULL
                     UNION
                     SELECT device_slot FROM room_devices WHERE room_id = $1
                     ORDER BY 1",
                )
                .bind(room_id)
                .fetch_all(&self.db)
                .await
                .map_err(DbError::from)?
            }
            ConfigScopeType::Device => {
                let slot = scope_key
                    .map(str::trim)
                    .filter(|slot| !slot.is_empty())
                    .ok_or_else(|| DbError::bad_request("device 作用域必须指定 device_slot"))?;
                vec![slot.to_owned()]
            }
        };
        Ok(slots)
    }

    /// 设备槽位是否存在（设备注册 / 房间主设备 / `room_devices` 子机任一命中）。
    pub async fn device_slot_exists(&self, slot: &str) -> DbResult<bool> {
        let slot = slot.trim();
        if slot.is_empty() {
            return Ok(false);
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM device_enrollment WHERE device_slot = $1)
                 OR EXISTS (SELECT 1 FROM rooms WHERE device_slot = $1)
                 OR EXISTS (SELECT 1 FROM room_devices WHERE device_slot = $1)",
        )
        .bind(slot)
        .fetch_one(&self.db)
        .await?;
        Ok(exists)
    }

    /// 读取合并上下文内的配置行（global → area → room → device 顺序）。
    async fn load_rows(
        &self,
        area_id: Option<Uuid>,
        room_id: Option<Uuid>,
        device_slot: Option<&str>,
    ) -> DbResult<Vec<EffectiveRow>> {
        sqlx::query_as::<_, EffectiveRow>(
            "SELECT scope_type, scope_id, scope_key, policy, revision
             FROM agent_configs
             WHERE (scope_type = 'global' AND scope_id IS NULL AND scope_key IS NULL)
                OR ($1::uuid IS NOT NULL AND scope_type = 'area' AND scope_id = $1)
                OR ($2::uuid IS NOT NULL AND scope_type = 'room' AND scope_id = $2)
                OR ($3::text IS NOT NULL AND scope_type = 'device' AND scope_key = $3)
             ORDER BY CASE scope_type
                          WHEN 'global' THEN 0
                          WHEN 'area' THEN 1
                          WHEN 'room' THEN 2
                          ELSE 3
                      END",
        )
        .bind(area_id)
        .bind(room_id)
        .bind(device_slot)
        .fetch_all(&self.db)
        .await
        .map_err(DbError::from)
    }

    /// 槽位 → 所在房间（主设备 `rooms.device_slot` 或子机 `room_devices`）。
    async fn slot_context(&self, slot: &str) -> DbResult<Option<SlotContext>> {
        let row: Option<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT r.id, r.area_id
               FROM rooms r
              WHERE r.device_slot = $1
             UNION ALL
             SELECT r.id, r.area_id
               FROM room_devices rd
               JOIN rooms r ON r.id = rd.room_id
              WHERE rd.device_slot = $1
              LIMIT 1",
        )
        .bind(slot)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(|(room_id, area_id)| SlotContext { room_id, area_id }))
    }
}

/// 合并多行 patch：global → area → room → device 顺序由 SQL 保证。
fn merge_rows(rows: impl IntoIterator<Item = EffectiveRow>) -> DbResult<EffectiveAgentPolicy> {
    let mut policy = AgentPolicy::default();
    let mut revision = 0i64;
    for row in rows {
        // 约束保证只有四个合法值；异常数据按内部错误处理，而不是静默忽略。
        ConfigScopeType::from_str(&row.scope_type).map_err(|_| {
            DbError::internal(format!("agent_configs.scope_type 非法: {}", row.scope_type))
        })?;
        let patch = serde_json::from_value::<AgentPolicyPatch>(row.policy)
            .map_err(|error| DbError::internal(format!("agent_configs.policy 非法: {error}")))?;
        patch.apply_to(&mut policy);
        revision = revision.max(row.revision);
    }
    Ok(EffectiveAgentPolicy { policy, revision })
}

/// 判断一行是否属于指定作用域（preview 时排除本作用域旧行）。
fn row_scope_matches(
    row: &EffectiveRow,
    scope_type: ConfigScopeType,
    scope_id: Option<Uuid>,
    scope_key: Option<&str>,
) -> bool {
    let Ok(row_scope) = ConfigScopeType::from_str(&row.scope_type) else {
        return false;
    };
    if row_scope != scope_type {
        return false;
    }
    match scope_type {
        ConfigScopeType::Global => true,
        ConfigScopeType::Area | ConfigScopeType::Room => row.scope_id == scope_id,
        ConfigScopeType::Device => {
            let key = scope_key.map(str::trim);
            row.scope_key.as_deref() == key
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_key_must_match() {
        let id = Uuid::nil();
        assert!(ensure_scope(ConfigScopeType::Global, None, None).is_ok());
        assert!(ensure_scope(ConfigScopeType::Global, Some(id), None).is_err());
        assert!(ensure_scope(ConfigScopeType::Global, None, Some("slot")).is_err());

        assert!(ensure_scope(ConfigScopeType::Area, None, None).is_err());
        assert!(ensure_scope(ConfigScopeType::Room, None, None).is_err());
        assert!(ensure_scope(ConfigScopeType::Area, Some(id), None).is_ok());
        assert!(ensure_scope(ConfigScopeType::Room, Some(id), None).is_ok());
        assert!(ensure_scope(ConfigScopeType::Area, Some(id), Some("slot")).is_err());

        assert!(ensure_scope(ConfigScopeType::Device, None, Some("slot-1")).is_ok());
        assert!(ensure_scope(ConfigScopeType::Device, None, Some("  ")).is_err());
        assert!(ensure_scope(ConfigScopeType::Device, None, None).is_err());
        assert!(ensure_scope(ConfigScopeType::Device, Some(id), Some("slot-1")).is_err());
    }

    #[test]
    fn config_row_maps_patch_and_scope_key() {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let row = AgentConfigRow {
            id,
            scope_type: "device".to_owned(),
            scope_id: None,
            scope_key: Some("room-101".to_owned()),
            policy: serde_json::json!({ "lxc_auto_start": true }),
            revision: 3,
            updated_by: None,
            created_at: now,
            updated_at: now,
        };
        let config = row.into_domain().expect("config");
        assert_eq!(config.id, id);
        assert_eq!(config.scope_type, ConfigScopeType::Device);
        assert_eq!(config.scope_id, None);
        assert_eq!(config.scope_key.as_deref(), Some("room-101"));
        assert_eq!(config.policy.lxc_auto_start, Some(true));
        assert_eq!(config.revision, 3);
    }

    #[test]
    fn merge_rows_applies_in_order_and_takes_max_revision() {
        let make = |scope_type: &str,
                    scope_id: Option<Uuid>,
                    scope_key: Option<&str>,
                    policy: Value,
                    revision: i64| EffectiveRow {
            scope_type: scope_type.to_owned(),
            scope_id,
            scope_key: scope_key.map(str::to_owned),
            policy,
            revision,
        };
        let area = Uuid::new_v4();
        let room = Uuid::new_v4();
        let rows = vec![
            make(
                "global",
                None,
                None,
                serde_json::json!({ "poll_interval_secs": 5 }),
                1,
            ),
            make(
                "area",
                Some(area),
                None,
                serde_json::json!({ "download_concurrency": 8 }),
                4,
            ),
            make(
                "room",
                Some(room),
                None,
                serde_json::json!({ "lxc_auto_start": true }),
                2,
            ),
            make(
                "device",
                None,
                Some("slot-1"),
                serde_json::json!({ "telemetry_fast_interval_secs": 20 }),
                7,
            ),
        ];
        let merged = merge_rows(rows).expect("merge");
        assert_eq!(merged.policy.poll_interval_secs, 5);
        assert_eq!(merged.policy.download_concurrency, 8);
        assert!(merged.policy.lxc_auto_start);
        assert_eq!(merged.policy.telemetry_fast_interval_secs, 20);
        assert_eq!(merged.revision, 7);
    }

    #[test]
    fn row_scope_matching_excludes_own_scope() {
        let area = Uuid::new_v4();
        let row = EffectiveRow {
            scope_type: "device".to_owned(),
            scope_id: None,
            scope_key: Some("slot-1".to_owned()),
            policy: serde_json::json!({}),
            revision: 1,
        };
        assert!(row_scope_matches(
            &row,
            ConfigScopeType::Device,
            None,
            Some("slot-1")
        ));
        assert!(!row_scope_matches(
            &row,
            ConfigScopeType::Device,
            None,
            Some("slot-2")
        ));
        let area_row = EffectiveRow {
            scope_type: "area".to_owned(),
            scope_id: Some(area),
            scope_key: None,
            policy: serde_json::json!({}),
            revision: 1,
        };
        assert!(row_scope_matches(
            &area_row,
            ConfigScopeType::Area,
            Some(area),
            None
        ));
        assert!(!row_scope_matches(
            &area_row,
            ConfigScopeType::Area,
            Some(Uuid::new_v4()),
            None
        ));
    }
}
