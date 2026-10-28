use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::{
        extractors::{ApiJson, AuthUser},
        validation::PageQuery,
    },
    auth::authorization,
    domain::{
        models::{Announcement, AnnouncementStatus, Page},
        rbac::permissions,
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateAnnouncementRequest {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub status: Option<AnnouncementStatus>,
    #[serde(default)]
    pub is_pinned: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAnnouncementRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub is_pinned: Option<bool>,
    #[serde(default)]
    pub status: Option<AnnouncementStatus>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<Page<Announcement>>> {
    authorization::require_permission(&state, id, permissions::ANNOUNCEMENT_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    let status = validation::parse_announcement_status(query.status.as_deref())?;

    Ok(Json(
        state
            .announcements
            .list(status, &keyword, page, page_size)
            .await?,
    ))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ApiJson(request): ApiJson<CreateAnnouncementRequest>,
) -> AppResult<(StatusCode, Json<Announcement>)> {
    authorization::require_permission(&state, id, permissions::ANNOUNCEMENT_WRITE).await?;

    let status = request.status.unwrap_or(AnnouncementStatus::Draft);
    if status != AnnouncementStatus::Draft {
        authorization::require_permission(&state, id, permissions::ANNOUNCEMENT_PUBLISH).await?;
    }

    let title = validation::normalize_announcement_title(&request.title)?;
    let content = validation::normalize_announcement_content(&request.content)?;
    let announcement = state
        .announcements
        .create(
            id,
            &title,
            &content,
            status,
            request.is_pinned.unwrap_or(false),
        )
        .await?;

    Ok((StatusCode::CREATED, Json(announcement)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(announcement_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateAnnouncementRequest>,
) -> AppResult<Json<Announcement>> {
    authorization::require_permission(&state, id, permissions::ANNOUNCEMENT_WRITE).await?;

    if request.title.is_none()
        && request.content.is_none()
        && request.is_pinned.is_none()
        && request.status.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }

    // 任何状态变更都视为发布操作（包括把已发布内容改回草稿）。
    if request.status.is_some() {
        authorization::require_permission(&state, id, permissions::ANNOUNCEMENT_PUBLISH).await?;
    }

    let title = request
        .title
        .as_deref()
        .map(validation::normalize_announcement_title)
        .transpose()?;
    let content = request
        .content
        .as_deref()
        .map(validation::normalize_announcement_content)
        .transpose()?;

    let announcement = state
        .announcements
        .update(
            announcement_id,
            title.as_deref(),
            content.as_deref(),
            request.is_pinned,
            request.status,
        )
        .await?
        .ok_or_else(|| AppError::not_found("公告不存在"))?;

    Ok(Json(announcement))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(announcement_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::ANNOUNCEMENT_DELETE).await?;

    if !state.announcements.delete(announcement_id).await? {
        return Err(AppError::not_found("公告不存在"));
    }
    Ok(StatusCode::NO_CONTENT)
}
