//! 工单自动建单规则管理接口（P1 Wave 1，契约 `docs/design/ticket-p1-contract.md` §2）。
//!
//! | 方法 | 路径 | 权限 |
//! | --- | --- | --- |
//! | GET | `/ticket-rules` | `ticket:rule:manage` 或 `ticket:manage` |
//! | POST | `/ticket-rules` | 同上 |
//! | PATCH | `/ticket-rules/{id}` | 同上 |
//! | DELETE | `/ticket-rules/{id}` | 同上 |
//!
//! 阈值/范围校验在 domain 纯函数完成；写操作统一审计 `ticket_rule.*`。

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        rbac::permissions,
        ticket::{
            TicketPriority, TicketRule, TicketRuleInput, TicketRulePatch, TicketScopeType,
            TicketSignalType, normalize_rule_name, validate_rule_thresholds,
        },
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateTicketRuleRequest {
    pub name: String,
    pub signal_type: TicketSignalType,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: TicketPriority,
    #[serde(default)]
    pub scope_type: TicketScopeType,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    #[serde(default)]
    pub thresholds: Value,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct UpdateTicketRuleRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// 仅用于显式拒绝修改 signal_type（契约：不可改）。
    #[serde(default)]
    pub signal_type: Option<TicketSignalType>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub priority: Option<TicketPriority>,
    #[serde(default)]
    pub scope_type: Option<TicketScopeType>,
    /// 缺失 = 不修改；显式 `null` = 清空（仅 scope_type=global 合法）。
    #[serde(default, deserialize_with = "super::deserialize_nullable")]
    pub scope_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub thresholds: Option<Value>,
}

async fn require_rule_manage(state: &AppState, user_id: Uuid) -> AppResult<()> {
    authorization::require_any_permission(
        state,
        user_id,
        &[permissions::TICKET_RULE_MANAGE, permissions::TICKET_MANAGE],
    )
    .await
}

/// 校验 scope 组合：area/room 时 scope_id 必填且存在；global 时忽略/清空。
async fn normalize_scope(
    state: &AppState,
    scope_type: TicketScopeType,
    scope_id: Option<Uuid>,
) -> AppResult<Option<Uuid>> {
    match scope_type {
        TicketScopeType::Global => Ok(None),
        TicketScopeType::Area => {
            let id =
                scope_id.ok_or_else(|| AppError::bad_request("区域级规则必须提供 scope_id"))?;
            if state.areas.get(id).await?.is_none() {
                return Err(AppError::bad_request("规则范围区域不存在"));
            }
            Ok(Some(id))
        }
        TicketScopeType::Room => {
            let id =
                scope_id.ok_or_else(|| AppError::bad_request("房间级规则必须提供 scope_id"))?;
            if state.rooms.get(id).await?.is_none() {
                return Err(AppError::bad_request("规则范围房间不存在"));
            }
            Ok(Some(id))
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    require_rule_manage(&state, id).await?;
    let items = state.ticket_rules.list().await?;
    Ok(Json(json!({ "items": items, "total": items.len() })))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateTicketRuleRequest>,
) -> AppResult<(StatusCode, Json<TicketRule>)> {
    require_rule_manage(&state, id).await?;

    let name = normalize_rule_name(&request.name)?;
    let thresholds = validate_rule_thresholds(request.signal_type, &request.thresholds)?;
    if request.signal_type == TicketSignalType::ImageVerify
        && request.scope_type != TicketScopeType::Global
    {
        return Err(AppError::bad_request(
            "镜像验证是全局信号，规则范围只能是 global",
        ));
    }
    let scope_id = normalize_scope(&state, request.scope_type, request.scope_id).await?;

    let rule = state
        .ticket_rules
        .create(&TicketRuleInput {
            name: name.clone(),
            signal_type: request.signal_type,
            enabled: request.enabled,
            priority: request.priority,
            scope_type: request.scope_type,
            scope_id,
            thresholds: thresholds.clone(),
        })
        .await?;

    super::audit::record_or_log(
        &state,
        id,
        "ticket_rule.create",
        "ticket_rule",
        Some(rule.id.to_string()),
        json!({
            "name": name,
            "signal_type": request.signal_type.as_str(),
            "enabled": request.enabled,
            "priority": request.priority.as_str(),
            "scope_type": request.scope_type.as_str(),
            "scope_id": scope_id,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(rule)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(rule_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateTicketRuleRequest>,
) -> AppResult<Json<TicketRule>> {
    require_rule_manage(&state, id).await?;

    let existing = state
        .ticket_rules
        .get(rule_id)
        .await?
        .ok_or_else(|| AppError::not_found("规则不存在"))?;
    if let Some(signal_type) = request.signal_type
        && signal_type != existing.signal_type
    {
        return Err(AppError::bad_request("规则信号类型不可修改"));
    }

    let name = match request.name.as_deref() {
        Some(raw) => Some(normalize_rule_name(raw)?),
        None => None,
    };
    let thresholds = match request.thresholds.as_ref() {
        Some(value) => Some(validate_rule_thresholds(existing.signal_type, value)?),
        None => None,
    };

    // scope_type 与 scope_id 合并后校验最终组合。
    let final_scope_type = request.scope_type.unwrap_or(existing.scope_type);
    if existing.signal_type == TicketSignalType::ImageVerify
        && final_scope_type != TicketScopeType::Global
    {
        return Err(AppError::bad_request(
            "镜像验证是全局信号，规则范围只能是 global",
        ));
    }
    let final_scope_id = match request.scope_id {
        Some(value) => value,
        None => existing.scope_id,
    };
    let scope_id = normalize_scope(&state, final_scope_type, final_scope_id).await?;
    let scope_id_patch = if request.scope_type.is_some() || request.scope_id.is_some() {
        Some(scope_id)
    } else {
        None
    };

    let rule = state
        .ticket_rules
        .update(
            rule_id,
            &TicketRulePatch {
                name: name.clone(),
                enabled: request.enabled,
                priority: request.priority,
                scope_type: request.scope_type,
                scope_id: scope_id_patch,
                thresholds: thresholds.clone(),
            },
        )
        .await?;

    super::audit::record_or_log(
        &state,
        id,
        "ticket_rule.update",
        "ticket_rule",
        Some(rule.id.to_string()),
        json!({
            "name": name,
            "enabled": request.enabled,
            "priority": request.priority.map(|value| value.as_str()),
            "scope_type": request.scope_type.map(|value| value.as_str()),
            "scope_id": scope_id_patch,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(rule))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(rule_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    require_rule_manage(&state, id).await?;

    let deleted = state.ticket_rules.delete(rule_id).await?;
    if !deleted {
        return Err(AppError::not_found("规则不存在"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "ticket_rule.delete",
        "ticket_rule",
        Some(rule_id.to_string()),
        json!({}),
        ip.as_deref(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
