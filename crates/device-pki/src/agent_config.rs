//! mTLS `GET /agent/config`：按证书身份返回合并后的节点生效策略。
//!
//! 契约见 `docs/storage-design.md` §17.3 与 `docs/machine-telemetry.md` v1.1：
//! - `PeerCert` serial → `device_enrollment`（`issued` / `active`）→ `device_slot`；
//! - `device_slot` → `rooms.device_slot`（房间主设备）或 `room_devices`（子机）→ room + area；
//! - 合并顺序 **global → area → room → device**，device 行用证书 CN（= `device_slot`）
//!   匹配 `agent_configs.scope_key`；
//! - 未绑定房间时只应用 global（+device）；`revision` 取实际应用行的最大值（无行 0）；
//! - 鉴权失败 401，数据库错误 500。
//!
//! 本 crate 不依赖 `qe-db`（避免把 SQL 仓储层耦合进 PKI 服务），因此在这里用
//! `sqlx::query_as` 直接读取 `agent_configs` / `rooms` / `room_devices`；合并语义与
//! `qe_db::agent_config::AgentConfigRepo::effective_for_slot` 保持一致，共用
//! `qe-domain` 的 patch / apply 逻辑。

use std::str::FromStr;

use axum::Json;
use axum::extract::State;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::agent_config::{
    AgentPolicy, AgentPolicyPatch, ConfigScopeType, EffectiveAgentPolicy,
};

use crate::error::{Error, Result};
use crate::routes::AppState;

#[derive(Debug, Clone, FromRow)]
struct PolicyRow {
    scope_type: String,
    policy: Value,
    revision: i64,
}

/// `GET /agent/config` —— 设备拉取生效策略（mTLS listener）。
///
/// 身份只来自客户端证书，不接受请求体/查询参数覆盖。
pub async fn get_config(
    State(st): State<AppState>,
    peer: crate::tls::PeerCert,
) -> Result<Json<EffectiveAgentPolicy>> {
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;
    let effective = effective_for_serial(&st.db, &serial)
        .await?
        .ok_or(Error::Unauthorized)?;
    Ok(Json(effective))
}

/// serial → 存活注册记录 → 设备槽位；没有存活注册记录时返回 `None`（401）。
async fn effective_for_serial(db: &PgPool, serial: &str) -> Result<Option<EffectiveAgentPolicy>> {
    let slot: Option<String> = sqlx::query_scalar(
        "SELECT device_slot FROM device_enrollment
         WHERE serial = $1 AND status IN ('issued','active')",
    )
    .bind(serial)
    .fetch_optional(db)
    .await?;
    let Some(slot) = slot else {
        return Ok(None);
    };
    Ok(Some(load_effective_policy(db, &slot).await?))
}

/// 读取并合并 global → area → room → device 的 patch。
async fn load_effective_policy(db: &PgPool, slot: &str) -> Result<EffectiveAgentPolicy> {
    // 槽位 → 房间：主设备看 rooms.device_slot，子机看 room_devices（0015）。
    let context: Option<(Uuid, Uuid)> = sqlx::query_as(
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
    .fetch_optional(db)
    .await?;
    let (room_id, area_id) = match context {
        Some((room_id, area_id)) => (Some(room_id), Some(area_id)),
        None => (None, None),
    };

    let rows = sqlx::query_as::<_, PolicyRow>(
        "SELECT scope_type, policy, revision
         FROM agent_configs
         WHERE (scope_type = 'global' AND scope_id IS NULL AND scope_key IS NULL)
            OR ($1::uuid IS NOT NULL AND scope_type = 'area' AND scope_id = $1)
            OR ($2::uuid IS NOT NULL AND scope_type = 'room' AND scope_id = $2)
            OR (scope_type = 'device' AND scope_key = $3)
         ORDER BY CASE scope_type
                       WHEN 'global' THEN 0
                       WHEN 'area' THEN 1
                       WHEN 'room' THEN 2
                       ELSE 3
                   END",
    )
    .bind(area_id)
    .bind(room_id)
    .bind(slot)
    .fetch_all(db)
    .await?;

    let mut policy = AgentPolicy::default();
    let mut revision = 0i64;
    for row in rows {
        // 约束保证只有四个合法值；异常数据按 500 处理，而不是静默忽略。
        ConfigScopeType::from_str(&row.scope_type).map_err(|_| {
            Error::Internal(anyhow::anyhow!(
                "agent_configs.scope_type 非法: {}",
                row.scope_type
            ))
        })?;
        let patch = serde_json::from_value::<AgentPolicyPatch>(row.policy).map_err(|error| {
            Error::Internal(anyhow::anyhow!(
                "agent_configs.policy 不是合法 patch: {error}"
            ))
        })?;
        patch.apply_to(&mut policy);
        revision = revision.max(row.revision);
    }

    Ok(EffectiveAgentPolicy { policy, revision })
}
