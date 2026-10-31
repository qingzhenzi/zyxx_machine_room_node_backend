//! mTLS `GET /agent/config`：按证书身份返回合并后的节点生效策略。
//!
//! 契约见 `docs/storage-design.md` §17.3：
//! - `PeerCert` serial → `device_enrollment`（`issued` / `active`）→ `device_slot`
//!   → `rooms.device_slot` → room + area；
//! - 未绑定房间时只应用 global；
//! - 合并顺序 global → area → room，`revision` 取实际应用行的最大值（无行 0）；
//! - 鉴权失败 401，数据库错误 500。
//!
//! 本 crate 不依赖 `qe-db`（避免把 SQL 仓储层耦合进 PKI 服务），因此在这里用
//! `sqlx::query_as` 直接读取 `agent_configs` / `rooms`；合并语义与
//! `qe_db::agent_config::AgentConfigRepo::effective_for_room` 保持一致，共用
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

/// serial → 存活注册记录 → 绑定房间；没有存活注册记录时返回 `None`（401）。
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

    // 已分配槽位但尚未绑定房间：只应用 global。
    let room: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id, area_id FROM rooms WHERE device_slot = $1")
            .bind(&slot)
            .fetch_optional(db)
            .await?;

    let (room_id, area_id) = match room {
        Some((room_id, area_id)) => (Some(room_id), Some(area_id)),
        None => (None, None),
    };

    Ok(Some(load_effective_policy(db, area_id, room_id).await?))
}

/// 读取并合并 global → area → room 的 patch。
async fn load_effective_policy(
    db: &PgPool,
    area_id: Option<Uuid>,
    room_id: Option<Uuid>,
) -> Result<EffectiveAgentPolicy> {
    let rows = sqlx::query_as::<_, PolicyRow>(
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
    .bind(room_id)
    .fetch_all(db)
    .await?;

    let mut policy = AgentPolicy::default();
    let mut revision = 0i64;
    for row in rows {
        // 约束保证只有三个合法值；异常数据按 500 处理，而不是静默忽略。
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
