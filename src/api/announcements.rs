use axum::{
    Json,
    extract::{Path, Query, State},
};
use uuid::Uuid;

use crate::{
    api::validation::PageQuery,
    domain::{
        models::{Announcement, AnnouncementStatus, Page},
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

/// 公开公告列表，只返回已发布内容，无需登录。
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<Page<Announcement>>> {
    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;

    let result = state
        .announcements
        .list(
            Some(AnnouncementStatus::Published),
            &keyword,
            page,
            page_size,
        )
        .await?;
    Ok(Json(result))
}

/// 公开公告详情，未发布的公告按 404 处理，避免泄露草稿。
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Announcement>> {
    let announcement = state
        .announcements
        .get(id)
        .await?
        .filter(|announcement| announcement.status == AnnouncementStatus::Published)
        .ok_or_else(|| AppError::not_found("公告不存在"))?;
    Ok(Json(announcement))
}
