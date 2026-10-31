//! 节点配置管理 P2：作用域策略 CRUD 与二次下发。
//!
//! 契约见 `docs/storage-design.md` §17.4。策略按 **global → area → room** 合并；
//! 管理端写入后对该作用域下所有已绑定设备槽位的房间做 `config.update` 二次下发。
//! 下发是尽力而为：无设备 / 入队失败 / Redis 通知失败都只记 `warn!`，不影响写入结果。

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        agent_config::{
            AgentConfig, AgentPolicy, AgentPolicyPatch, ConfigScopeType, validate_policy_patch,
        },
        rbac::permissions,
    },
    error::{AppError, AppResult},
    state::AppState,
};

/// `PUT /admin/agent-configs` 请求体。
#[derive(Debug, Deserialize)]
pub struct UpdateAgentConfigRequest {
    pub scope_type: ConfigScopeType,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    #[serde(default)]
    pub policy: AgentPolicyPatch,
}

/// `GET /admin/agent-configs` 响应。
#[derive(Debug, Serialize)]
pub struct AgentConfigListReply {
    pub items: Vec<AgentConfig>,
    pub total: i64,
}

/// `GET /admin/agent-configs/effective` 查询参数。
#[derive(Debug, Deserialize)]
pub struct EffectiveAgentConfigQuery {
    pub room_id: Uuid,
}

/// `GET /admin/agent-configs/effective` 响应：合并后的完整策略。
#[derive(Debug, Serialize)]
pub struct EffectiveAgentPolicyReply {
    pub room_id: Uuid,
    pub policy: AgentPolicy,
    pub revision: i64,
}

/// `PUT /admin/agent-configs` 响应：写入后的配置 + 成功下发到设备槽位的房间数。
#[derive(Debug, Serialize)]
pub struct UpdateAgentConfigReply {
    pub config: AgentConfig,
    pub pushed_rooms: i64,
}

/// `GET /admin/agent-configs`：所有作用域的配置（含 policy patch）。
pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<AgentConfigListReply>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let items = state.agent_configs.list().await?;
    let total = items.len() as i64;
    Ok(Json(AgentConfigListReply { items, total }))
}

/// `GET /admin/agent-configs/effective?room_id=`：某房间合并后的生效策略。
pub async fn effective(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<EffectiveAgentConfigQuery>,
) -> AppResult<Json<EffectiveAgentPolicyReply>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    if state.rooms.get(query.room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }
    let effective = state
        .agent_configs
        .effective_for_room(query.room_id)
        .await?;
    Ok(Json(EffectiveAgentPolicyReply {
        room_id: query.room_id,
        policy: effective.policy,
        revision: effective.revision,
    }))
}

/// `PUT /admin/agent-configs`：校验作用域与策略 → upsert → 审计 → 二次下发。
pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<UpdateAgentConfigRequest>,
) -> AppResult<Json<UpdateAgentConfigReply>> {
    authorization::require_permission(&state, id, permissions::DEVICE_MANAGE).await?;

    ensure_scope(&state, request.scope_type, request.scope_id).await?;
    validate_policy_patch(&request.policy)?;

    let config = state
        .agent_configs
        .upsert(request.scope_type, request.scope_id, &request.policy, id)
        .await?;

    super::audit::record_or_log(
        &state,
        id,
        "agent_config.update",
        "agent_config",
        Some(config.id.to_string()),
        json!({
            "scope_type": config.scope_type.as_str(),
            "scope_id": config.scope_id,
            "policy": config.policy,
            "revision": config.revision,
        }),
        ip.as_deref(),
    )
    .await;

    let room_ids = state
        .agent_configs
        .rooms_for_scope(request.scope_type, request.scope_id)
        .await?;
    let mut pushed_rooms = 0_i64;
    for room_id in room_ids {
        if dispatch_config_update(&state, room_id).await {
            pushed_rooms += 1;
        }
    }

    Ok(Json(UpdateAgentConfigReply {
        config,
        pushed_rooms,
    }))
}

/// 作用域与 `scope_id` 的组合约束，并校验 area/room 目标存在（404）。
async fn ensure_scope(
    state: &AppState,
    scope_type: ConfigScopeType,
    scope_id: Option<Uuid>,
) -> AppResult<()> {
    match scope_type {
        ConfigScopeType::Global => {
            if scope_id.is_some() {
                return Err(AppError::bad_request(
                    "scope_type=global 时不能提供 scope_id",
                ));
            }
        }
        ConfigScopeType::Area => {
            let area_id = scope_id
                .ok_or_else(|| AppError::bad_request("scope_type=area 时必须提供 scope_id"))?;
            if state.areas.get(area_id).await?.is_none() {
                return Err(AppError::not_found("区域不存在"));
            }
        }
        ConfigScopeType::Room => {
            let room_id = scope_id
                .ok_or_else(|| AppError::bad_request("scope_type=room 时必须提供 scope_id"))?;
            if state.rooms.get(room_id).await?.is_none() {
                return Err(AppError::not_found("房间不存在"));
            }
        }
    }
    Ok(())
}

/// 向单个房间做 `config.update` 二次下发；返回是否成功入队。
///
/// 尽力而为：房间不存在 / 未绑定槽位 / 没有存活设备 / 入队失败都只记 `warn!`。
async fn dispatch_config_update(state: &AppState, room_id: Uuid) -> bool {
    let slot = match state.rooms.get(room_id).await {
        Ok(Some(room)) => room.device_slot,
        Ok(None) => {
            tracing::warn!(%room_id, "房间不存在，跳过 config.update 下发");
            return false;
        }
        Err(error) => {
            tracing::warn!(%error, %room_id, "读取房间失败，跳过 config.update 下发");
            return false;
        }
    };
    let Some(slot) = slot
        .map(|slot| slot.trim().to_owned())
        .filter(|slot| !slot.is_empty())
    else {
        tracing::warn!(%room_id, "房间未绑定设备槽位，config.update 未下发");
        return false;
    };

    let effective = match state.agent_configs.effective_for_room(room_id).await {
        Ok(effective) => effective,
        Err(error) => {
            tracing::warn!(%error, %room_id, "读取生效配置失败，跳过 config.update 下发");
            return false;
        }
    };
    let payload = json!({
        "policy": effective.policy,
        "revision": effective.revision,
    })
    .to_string();

    match state
        .storage_repo
        .enqueue_downlink_for_slot(&slot, "config.update", &payload)
        .await
    {
        Ok(Some(serial)) => {
            // 唤醒长连接（跨进程尽力而为；失败还有兜底轮询）。
            if let Err(error) = state
                .cache
                .publish(&state.config.schedule_downlink_notify_channel, &serial)
                .await
            {
                tracing::warn!(%error, %serial, "发布设备下行通知失败，config.update 已入队");
            }
            true
        }
        Ok(None) => {
            tracing::warn!(%room_id, %slot, "设备槽位当前没有存活设备，config.update 未下发");
            false
        }
        Err(error) => {
            tracing::warn!(%error, %room_id, %slot, "config.update 入队失败");
            false
        }
    }
}
