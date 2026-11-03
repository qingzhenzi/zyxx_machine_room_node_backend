//! 工单数据范围管理接口（Wave 2 B）。
//!
//! - `GET /admin/ticket-scopes`：启用且拥有 `ticket:read` 的用户 + 其区域范围；
//! - `PUT /admin/ticket-scopes/{user_id}`：整体替换某用户的区域范围（校验区域存在）。
//!
//! 权限：`ticket:scope:manage` 或 `ticket:manage`。审计：`ticket_scope.update`。

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{rbac::permissions, ticket::TicketScopeUser},
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ReplaceTicketScopesRequest {
    #[serde(default)]
    pub area_ids: Vec<Uuid>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<TicketScopeUser>>> {
    require_scope_manage(&state, id).await?;
    Ok(Json(state.ticket_scopes.list().await?))
}

pub async fn replace(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(user_id): Path<Uuid>,
    ApiJson(request): ApiJson<ReplaceTicketScopesRequest>,
) -> AppResult<Json<TicketScopeUser>> {
    require_scope_manage(&state, id).await?;

    let area_ids = state
        .ticket_scopes
        .replace(user_id, &request.area_ids)
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "ticket_scope.update",
        "ticket_scope",
        Some(user_id.to_string()),
        json!({ "area_ids": area_ids }),
        ip.as_deref(),
    )
    .await;

    let entry = state
        .ticket_scopes
        .list()
        .await?
        .into_iter()
        .find(|row| row.user_id == user_id)
        .ok_or_else(|| AppError::not_found("用户不存在或没有 ticket:read 权限"))?;
    Ok(Json(entry))
}

async fn require_scope_manage(state: &AppState, user_id: Uuid) -> AppResult<()> {
    authorization::require_any_permission(
        state,
        user_id,
        &[permissions::TICKET_SCOPE_MANAGE, permissions::TICKET_MANAGE],
    )
    .await
}

#[derive(Debug, Serialize)]
pub struct ScopeAreaOption {
    pub id: Uuid,
    pub name: String,
    pub code: String,
}

/// 工单范围配置用区域选项（不要求 `area:read`，只要求 scope 管理权限）。
pub async fn areas(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<ScopeAreaOption>>> {
    require_scope_manage(&state, id).await?;
    let page = state.areas.list("", true, 1, 500).await?;
    Ok(Json(
        page.items
            .into_iter()
            .map(|area| ScopeAreaOption {
                id: area.id,
                name: area.name,
                code: area.code,
            })
            .collect(),
    ))
}
