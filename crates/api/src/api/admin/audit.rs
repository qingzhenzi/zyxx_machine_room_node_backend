use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;

use crate::{
    api::extractors::AuthUser,
    domain::{models::AuditLog, rbac::permissions, validation},
    error::AppResult,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<AuditQuery>,
) -> AppResult<Json<crate::domain::models::Page<AuditLog>>> {
    crate::auth::authorization::require_permission(&state, id, permissions::AUDIT_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let action = query
        .action
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let resource_type = query
        .resource_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    Ok(Json(
        state
            .audit
            .list(action, resource_type, page, page_size)
            .await?,
    ))
}

pub async fn record_or_log(
    state: &AppState,
    actor_id: uuid::Uuid,
    action: &str,
    resource_type: &str,
    resource_id: Option<String>,
    detail: serde_json::Value,
    ip: Option<&str>,
) {
    if let Err(error) = state
        .audit
        .record(
            actor_id,
            action,
            resource_type,
            resource_id.as_deref(),
            detail,
            ip,
        )
        .await
    {
        tracing::error!(%error, action, resource_type, "写入审计日志失败");
    }
}
