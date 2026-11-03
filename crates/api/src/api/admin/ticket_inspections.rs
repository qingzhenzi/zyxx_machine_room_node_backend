//! 巡检计划接口（Wave 3）：计划 CRUD、手动执行、执行历史。
//!
//! 权限：`ticket:inspection:manage` 或 `ticket:manage`（`ticket:manage` 为兼容旧管理员）。
//! 审计：`inspection_plan.create / update / delete / run`，`resource_type = "inspection_plan"`。

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use qe_domain::ticket::{
    InspectionPlanInput, InspectionPlanPatch, InspectionPlanView, InspectionRun, TicketScopeType,
    normalize_inspection_name, validate_inspection_interval, validate_inspection_scope,
};

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::rbac::permissions,
    error::{AppError, AppResult},
    state::AppState,
};

const RUNS_DEFAULT_LIMIT: i64 = 20;
const RUNS_MAX_LIMIT: i64 = 100;

async fn require_manage(state: &AppState, user_id: Uuid) -> AppResult<()> {
    authorization::require_any_permission(
        state,
        user_id,
        &[
            permissions::TICKET_INSPECTION_MANAGE,
            permissions::TICKET_MANAGE,
        ],
    )
    .await
}

/// 校验 scope 指向的区域/房间存在。
async fn ensure_scope_exists(
    state: &AppState,
    scope_type: TicketScopeType,
    scope_id: Option<Uuid>,
) -> AppResult<()> {
    match scope_type {
        TicketScopeType::Global => Ok(()),
        TicketScopeType::Area => {
            let area_id = scope_id.ok_or_else(|| AppError::bad_request("缺少 scope_id"))?;
            if state.areas.get(area_id).await?.is_none() {
                return Err(AppError::not_found("区域不存在"));
            }
            Ok(())
        }
        TicketScopeType::Room => {
            let room_id = scope_id.ok_or_else(|| AppError::bad_request("缺少 scope_id"))?;
            if state.rooms.get(room_id).await?.is_none() {
                return Err(AppError::not_found("房间不存在"));
            }
            Ok(())
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateInspectionPlanRequest {
    pub name: String,
    #[serde(default)]
    pub scope_type: Option<TicketScopeType>,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    #[serde(default)]
    pub interval_hours: Option<i64>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateInspectionPlanRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub scope_type: Option<TicketScopeType>,
    /// 缺失 = 不修改；显式 `null` = 清空（scope_type=global 时使用）。
    #[serde(default, deserialize_with = "super::deserialize_nullable")]
    pub scope_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub interval_hours: Option<i64>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct InspectionRunsQuery {
    #[serde(default)]
    pub plan_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<InspectionPlanView>>> {
    require_manage(&state, id).await?;
    Ok(Json(state.ticket_inspections.list().await?))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateInspectionPlanRequest>,
) -> AppResult<(StatusCode, Json<InspectionPlanView>)> {
    require_manage(&state, id).await?;

    let name = normalize_inspection_name(&request.name)?;
    let interval_hours = validate_inspection_interval(request.interval_hours.unwrap_or(24))?;
    let scope_type = request.scope_type.unwrap_or(TicketScopeType::Global);
    let scope_id = validate_inspection_scope(scope_type, request.scope_id)?;
    ensure_scope_exists(&state, scope_type, scope_id).await?;

    let plan = state
        .ticket_inspections
        .create(
            &InspectionPlanInput {
                name,
                scope_type,
                scope_id,
                interval_hours,
                enabled: request.enabled.unwrap_or(true),
            },
            id,
        )
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "inspection_plan.create",
        "inspection_plan",
        Some(plan.id.to_string()),
        json!({
            "name": plan.name,
            "scope_type": plan.scope_type.as_str(),
            "scope_id": plan.scope_id,
            "interval_hours": plan.interval_hours,
            "enabled": plan.enabled,
        }),
        ip.as_deref(),
    )
    .await;

    let view = state
        .ticket_inspections
        .get(plan.id)
        .await?
        .ok_or_else(|| AppError::internal("创建巡检计划后无法读取"))?;
    Ok((StatusCode::CREATED, Json(view)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateInspectionPlanRequest>,
) -> AppResult<Json<InspectionPlanView>> {
    require_manage(&state, id).await?;

    let current = state
        .ticket_inspections
        .get(plan_id)
        .await?
        .ok_or_else(|| AppError::not_found("巡检计划不存在"))?
        .plan;
    let name = match request.name.as_deref() {
        Some(raw) => Some(normalize_inspection_name(raw)?),
        None => None,
    };
    let interval_hours = match request.interval_hours {
        Some(value) => Some(validate_inspection_interval(value)?),
        None => None,
    };
    let scope_type = request.scope_type.unwrap_or(current.scope_type);
    let scope_id_input = match request.scope_id {
        Some(value) => value,
        None => current.scope_id,
    };
    let scope_id = validate_inspection_scope(scope_type, scope_id_input)?;
    ensure_scope_exists(&state, scope_type, scope_id).await?;

    let plan = state
        .ticket_inspections
        .update(
            plan_id,
            &InspectionPlanPatch {
                name: name.clone(),
                scope_type: Some(scope_type),
                scope_id: Some(scope_id),
                interval_hours,
                enabled: request.enabled,
            },
        )
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "inspection_plan.update",
        "inspection_plan",
        Some(plan.id.to_string()),
        json!({
            "name": name,
            "scope_type": plan.scope_type.as_str(),
            "scope_id": plan.scope_id,
            "interval_hours": interval_hours,
            "enabled": request.enabled,
        }),
        ip.as_deref(),
    )
    .await;
    let view = state
        .ticket_inspections
        .get(plan.id)
        .await?
        .ok_or_else(|| AppError::internal("更新巡检计划后无法读取"))?;
    Ok(Json(view))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    require_manage(&state, id).await?;

    let plan = state
        .ticket_inspections
        .get(plan_id)
        .await?
        .ok_or_else(|| AppError::not_found("巡检计划不存在"))?
        .plan;
    if !state.ticket_inspections.delete(plan_id).await? {
        return Err(AppError::not_found("巡检计划不存在"));
    }
    super::audit::record_or_log(
        &state,
        id,
        "inspection_plan.delete",
        "inspection_plan",
        Some(plan_id.to_string()),
        json!({ "name": plan.name }),
        ip.as_deref(),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// 手动立即执行一次（同步返回 run；不影响计划的 next_run_at 周期）。
pub async fn run(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
) -> AppResult<Json<InspectionRun>> {
    require_manage(&state, id).await?;

    let plan = state
        .ticket_inspections
        .get(plan_id)
        .await?
        .ok_or_else(|| AppError::not_found("巡检计划不存在"))?
        .plan;
    let run = crate::ticket_worker::run_plan(&state, &plan)
        .await
        .map_err(|error| {
            tracing::error!(%error, plan_id = %plan_id, "手动执行巡检计划失败");
            AppError::internal("巡检执行失败")
        })?;
    super::audit::record_or_log(
        &state,
        id,
        "inspection_plan.run",
        "inspection_plan",
        Some(plan_id.to_string()),
        json!({
            "run_id": run.id,
            "status": run.status.as_str(),
            "checked_machines": run.checked_machines,
            "issue_machines": run.issue_machines,
            "ticket_id": run.ticket_id,
        }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(run))
}

pub async fn runs(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<InspectionRunsQuery>,
) -> AppResult<Json<Vec<InspectionRun>>> {
    require_manage(&state, id).await?;

    let limit = query.limit.unwrap_or(RUNS_DEFAULT_LIMIT);
    if !(1..=RUNS_MAX_LIMIT).contains(&limit) {
        return Err(AppError::bad_request(format!(
            "limit 需在 1-{RUNS_MAX_LIMIT} 之间"
        )));
    }
    Ok(Json(
        state
            .ticket_inspections
            .list_runs(query.plan_id, limit)
            .await?,
    ))
}
