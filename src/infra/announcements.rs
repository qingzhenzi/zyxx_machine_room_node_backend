use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::models::{Announcement, AnnouncementStatus, Page},
    error::AppResult,
};

#[derive(Clone)]
pub struct AnnouncementRepo {
    db: PgPool,
}

impl AnnouncementRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        status: Option<AnnouncementStatus>,
        keyword: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<Page<Announcement>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM announcements a
             WHERE ($1::text IS NULL OR a.status = $1)
               AND ($2 = ''
                    OR a.title ILIKE '%' || $2 || '%'
                    OR a.content ILIKE '%' || $2 || '%')",
        )
        .bind(status)
        .bind(keyword)
        .fetch_one(&self.db)
        .await?;

        let items = sqlx::query_as::<_, Announcement>(
            "SELECT a.id,
                    a.title,
                    a.content,
                    a.status,
                    a.is_pinned,
                    a.author_id,
                    u.display_name AS author_name,
                    a.published_at,
                    a.created_at,
                    a.updated_at
             FROM announcements a
             LEFT JOIN users u ON u.id = a.author_id
             WHERE ($1::text IS NULL OR a.status = $1)
               AND ($2 = ''
                    OR a.title ILIKE '%' || $2 || '%'
                    OR a.content ILIKE '%' || $2 || '%')
             ORDER BY a.is_pinned DESC, a.published_at DESC NULLS LAST, a.created_at DESC
             LIMIT $3 OFFSET $4",
        )
        .bind(status)
        .bind(keyword)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.db)
        .await?;

        Ok(Page::new(items, total, page, page_size))
    }

    pub async fn get(&self, id: Uuid) -> AppResult<Option<Announcement>> {
        Ok(sqlx::query_as::<_, Announcement>(
            "SELECT a.id,
                    a.title,
                    a.content,
                    a.status,
                    a.is_pinned,
                    a.author_id,
                    u.display_name AS author_name,
                    a.published_at,
                    a.created_at,
                    a.updated_at
             FROM announcements a
             LEFT JOIN users u ON u.id = a.author_id
             WHERE a.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?)
    }

    pub async fn create(
        &self,
        author_id: Uuid,
        title: &str,
        content: &str,
        status: AnnouncementStatus,
        is_pinned: bool,
    ) -> AppResult<Announcement> {
        Ok(sqlx::query_as::<_, Announcement>(
            "WITH inserted AS (
                INSERT INTO announcements (title, content, status, is_pinned, author_id, published_at)
                VALUES (
                    $1,
                    $2,
                    $3,
                    $4,
                    $5,
                    CASE WHEN $3::text = 'published' THEN now() ELSE NULL END
                )
                RETURNING *
             )
             SELECT i.id,
                    i.title,
                    i.content,
                    i.status,
                    i.is_pinned,
                    i.author_id,
                    u.display_name AS author_name,
                    i.published_at,
                    i.created_at,
                    i.updated_at
             FROM inserted i
             LEFT JOIN users u ON u.id = i.author_id",
        )
        .bind(title)
        .bind(content)
        .bind(status)
        .bind(is_pinned)
        .bind(author_id)
        .fetch_one(&self.db)
        .await?)
    }

    pub async fn update(
        &self,
        id: Uuid,
        title: Option<&str>,
        content: Option<&str>,
        is_pinned: Option<bool>,
        status: Option<AnnouncementStatus>,
    ) -> AppResult<Option<Announcement>> {
        Ok(sqlx::query_as::<_, Announcement>(
            "WITH updated AS (
                UPDATE announcements SET
                    title = COALESCE($2, title),
                    content = COALESCE($3, content),
                    is_pinned = COALESCE($4, is_pinned),
                    status = COALESCE($5, status),
                    published_at = CASE
                        WHEN $5::text = 'published' AND published_at IS NULL THEN now()
                        ELSE published_at
                    END
                WHERE id = $1
                RETURNING *
             )
             SELECT a.id,
                    a.title,
                    a.content,
                    a.status,
                    a.is_pinned,
                    a.author_id,
                    u.display_name AS author_name,
                    a.published_at,
                    a.created_at,
                    a.updated_at
             FROM updated a
             LEFT JOIN users u ON u.id = a.author_id",
        )
        .bind(id)
        .bind(title)
        .bind(content)
        .bind(is_pinned)
        .bind(status)
        .fetch_optional(&self.db)
        .await?)
    }

    pub async fn delete(&self, id: Uuid) -> AppResult<bool> {
        let deleted = sqlx::query("DELETE FROM announcements WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}
