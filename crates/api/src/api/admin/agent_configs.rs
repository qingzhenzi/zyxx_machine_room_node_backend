//! 节点配置管理 P2：作用域策略 CRUD 与二次下发。
//!
//! 契约见 `docs/storage-design.md` §17.4 与 `docs/machine-telemetry.md` v1.1。策略按
//! **global → area → room → device** 合并：
//! - `area` / `room` 用 `scope_id`（UUID）定位；
//! - `device` 用 `device_slot`（TEXT，`device_enrollment` / `rooms.device_slot` /
//!   `room_devices` 任一存在即可）定位，一个槽位一行；
//! - 管理端写入后对作用域内**每台设备槽位**（房间主设备 + `room_devices` 子机）
//!   各做一次 `config.update` 二次下发，payload 用 `effective_for_slot` 合并，
//!   保证 device 覆盖不会被 room 级下发冲掉；
//! - 下发是尽力而为：无设备 / 入队失败 / Redis 通知失败都只记 `warn!`，不影响写入结果。

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
    domain::agent_config::{
        AgentConfig, AgentPolicy, AgentPolicyPatch, ConfigScopeType, validate_policy_patch,
    },
    domain::rbac::permissions,
    error::{AppError, AppResult},
    state::AppState,
};

/// `PUT /admin/agent-configs` 请求体。
#[derive(Debug, Deserialize)]
pub struct UpdateAgentConfigRequest {
    pub scope_type: ConfigScopeType,
    /// `area` / `room` 作用域的目标 UUID。
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    /// `device` 作用域的目标设备槽位（其余作用域必须省略）。
    #[serde(default)]
    pub device_slot: Option<String>,
    #[serde(default)]
    pub policy: AgentPolicyPatch,
}

/// `GET /admin/agent-configs` 响应。
#[derive(Debug, Serialize)]
pub struct AgentConfigListReply {
    pub items: Vec<AgentConfig>,
    pub total: i64,
}

/// `GET /admin/agent-configs/effective` 查询参数：`room_id` 与 `device_slot` 二选一。
#[derive(Debug, Deserialize)]
pub struct EffectiveAgentConfigQuery {
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub device_slot: Option<String>,
}

/// `GET /admin/agent-configs/effective` 响应：合并后的完整策略。
#[derive(Debug, Serialize)]
pub struct EffectiveAgentPolicyReply {
    /// 按房间查询时返回房间 id。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_id: Option<Uuid>,
    /// 按设备查询时返回设备槽位。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_slot: Option<String>,
    pub policy: AgentPolicy,
    pub revision: i64,
}

/// `PUT /admin/agent-configs` 响应：写入后的配置 + 成功下发到设备槽位的数量。
#[derive(Debug, Serialize)]
pub struct UpdateAgentConfigReply {
    pub config: AgentConfig,
    /// 成功入队的设备槽位数。字段名为兼容旧契约保留（早期只有房间主设备）。
    pub pushed_rooms: i64,
    /// 成功入队的设备槽位数（= `pushed_rooms`，语义更准确的新字段）。
    pub pushed_slots: i64,
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

/// `GET /admin/agent-configs/effective?room_id=` / `?device_slot=`：合并后的生效策略。
pub async fn effective(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<EffectiveAgentConfigQuery>,
) -> AppResult<Json<EffectiveAgentPolicyReply>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    match (query.room_id, query.device_slot) {
        (Some(room_id), None) => {
            if state.rooms.get(room_id).await?.is_none() {
                return Err(AppError::not_found("房间不存在"));
            }
            let effective = state.agent_configs.effective_for_room(room_id).await?;
            Ok(Json(EffectiveAgentPolicyReply {
                room_id: Some(room_id),
                device_slot: None,
                policy: effective.policy,
                revision: effective.revision,
            }))
        }
        (None, Some(raw_slot)) => {
            let slot = raw_slot.trim();
            if slot.is_empty() {
                return Err(AppError::bad_request("device_slot 不能为空"));
            }
            if !state.agent_configs.device_slot_exists(slot).await? {
                return Err(AppError::not_found("设备不存在"));
            }
            let effective = state.agent_configs.effective_for_slot(slot).await?;
            Ok(Json(EffectiveAgentPolicyReply {
                room_id: None,
                device_slot: Some(slot.to_owned()),
                policy: effective.policy,
                revision: effective.revision,
            }))
        }
        (Some(_), Some(_)) => Err(AppError::bad_request("room_id 与 device_slot 只能二选一")),
        (None, None) => Err(AppError::bad_request("必须提供 room_id 或 device_slot")),
    }
}

/// `PUT /admin/agent-configs`：校验作用域与策略 → upsert → 审计 → 二次下发。
pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<UpdateAgentConfigRequest>,
) -> AppResult<Json<UpdateAgentConfigReply>> {
    authorization::require_permission(&state, id, permissions::DEVICE_MANAGE).await?;

    let scope_key = ensure_scope(&state, &request).await?;
    validate_policy_patch(&request.policy)?;
    // 写入前预演合并结果，拦掉「单字段合法但合并后非法」的策略（如 full < fast）。
    state
        .agent_configs
        .preview_effective(
            request.scope_type,
            request.scope_id,
            scope_key.as_deref(),
            &request.policy,
        )
        .await?;

    let config = state
        .agent_configs
        .upsert(
            request.scope_type,
            request.scope_id,
            scope_key.as_deref(),
            &request.policy,
            id,
        )
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
            "device_slot": config.scope_key,
            "policy": config.policy,
            "revision": config.revision,
        }),
        ip.as_deref(),
    )
    .await;

    // 受影响设备槽位：主设备 + room_devices 子机，各自合并后下发。
    let slots = state
        .agent_configs
        .device_slots_for_scope(request.scope_type, request.scope_id, scope_key.as_deref())
        .await?;
    let mut pushed_slots = 0_i64;
    for slot in slots {
        if dispatch_config_update(&state, &slot).await {
            pushed_slots += 1;
        }
    }

    Ok(Json(UpdateAgentConfigReply {
        config,
        pushed_rooms: pushed_slots,
        pushed_slots,
    }))
}

/// 校验作用域与 `scope_id` / `device_slot` 的组合，并检查目标存在（404）。
///
/// 返回 device 作用域的槽位 key（其余作用域为 `None`）。
async fn ensure_scope(
    state: &AppState,
    request: &UpdateAgentConfigRequest,
) -> AppResult<Option<String>> {
    let device_slot = request
        .device_slot
        .as_deref()
        .map(str::trim)
        .filter(|slot| !slot.is_empty())
        .map(str::to_owned);

    match request.scope_type {
        ConfigScopeType::Global => {
            if request.scope_id.is_some() {
                return Err(AppError::bad_request(
                    "scope_type=global 时不能提供 scope_id",
                ));
            }
            if device_slot.is_some() {
                return Err(AppError::bad_request(
                    "scope_type=global 时不能提供 device_slot",
                ));
            }
            Ok(None)
        }
        ConfigScopeType::Area => {
            if device_slot.is_some() {
                return Err(AppError::bad_request(
                    "scope_type=area 时不能提供 device_slot",
                ));
            }
            let area_id = request
                .scope_id
                .ok_or_else(|| AppError::bad_request("scope_type=area 时必须提供 scope_id"))?;
            if state.areas.get(area_id).await?.is_none() {
                return Err(AppError::not_found("区域不存在"));
            }
            Ok(None)
        }
        ConfigScopeType::Room => {
            if device_slot.is_some() {
                return Err(AppError::bad_request(
                    "scope_type=room 时不能提供 device_slot",
                ));
            }
            let room_id = request
                .scope_id
                .ok_or_else(|| AppError::bad_request("scope_type=room 时必须提供 scope_id"))?;
            if state.rooms.get(room_id).await?.is_none() {
                return Err(AppError::not_found("房间不存在"));
            }
            Ok(None)
        }
        ConfigScopeType::Device => {
            if request.scope_id.is_some() {
                return Err(AppError::bad_request(
                    "scope_type=device 时不能提供 scope_id（用 device_slot）",
                ));
            }
            let slot = device_slot
                .ok_or_else(|| AppError::bad_request("scope_type=device 时必须提供 device_slot"))?;
            if !state.agent_configs.device_slot_exists(&slot).await? {
                return Err(AppError::not_found("设备不存在"));
            }
            Ok(Some(slot))
        }
    }
}

/// 向单个设备槽位做 `config.update` 二次下发；返回是否成功入队。
///
/// 使用 `effective_for_slot` 合并（global → area → room → device），保证 device 级
/// 覆盖随下发一起生效。尽力而为：槽位无存活设备 / 入队失败都只记 `warn!`。
async fn dispatch_config_update(state: &AppState, slot: &str) -> bool {
    let effective = match state.agent_configs.effective_for_slot(slot).await {
        Ok(effective) => effective,
        Err(error) => {
            tracing::warn!(%error, %slot, "读取生效配置失败，跳过 config.update 下发");
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
        .enqueue_downlink_for_slot(slot, "config.update", &payload)
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
            tracing::warn!(%slot, "设备槽位当前没有存活设备，config.update 未下发");
            false
        }
        Err(error) => {
            tracing::warn!(%error, %slot, "config.update 入队失败");
            false
        }
    }
}
