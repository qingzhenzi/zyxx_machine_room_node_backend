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

    let version = state.cache.announcement_version().await.unwrap_or(0);
    let cache_key = format!("announcements:public:v{version}:p{page}:s{page_size}:k{keyword}");
    if let Ok(Some(cached)) = state.cache.get_json::<Page<Announcement>>(&cache_key).await {
        return Ok(Json(cached));
    }

    let mut result = state
        .announcements
        .list(
            Some(AnnouncementStatus::Published),
            &keyword,
            page,
            page_size,
        )
        .await?;
    for announcement in &mut result.items {
        announcement.content = validation::sanitize_announcement_content(&announcement.content);
    }

    if let Err(error) = state
        .cache
        .set_json(&cache_key, &result, state.config.announcement_cache_secs)
        .await
    {
        tracing::warn!(%error, "写入公告列表缓存失败");
    }

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
    let mut announcement = announcement;
    announcement.content = validation::sanitize_announcement_content(&announcement.content);
    Ok(Json(announcement))
}
