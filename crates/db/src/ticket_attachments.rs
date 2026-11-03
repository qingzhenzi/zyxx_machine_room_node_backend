//! 工单附件仓储（Wave 2 C）：附件登记、列表、删除。
//!
//! 对象本体存 S3 兼容存储（预签名上传/下载），本表只保存元数据；下载地址由 API 层
//! 按需 `presign_get` 生成（未配置存储时为 `null`）。

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::ticket::TicketAttachment;

use crate::error::{DbError, DbResult};

#[derive(Debug, FromRow)]
struct AttachmentRow {
    id: Uuid,
    ticket_id: Uuid,
    object_key: String,
    filename: String,
    content_type: String,
    size: i64,
    sha256: String,
    uploaded_by: Option<Uuid>,
    uploaded_by_name: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<AttachmentRow> for TicketAttachment {
    fn from(row: AttachmentRow) -> Self {
        Self {
            id: row.id,
            ticket_id: row.ticket_id,
            object_key: row.object_key,
            filename: row.filename,
            content_type: row.content_type,
            size: row.size,
            sha256: row.sha256,
            uploaded_by: row.uploaded_by,
            uploaded_by_name: row.uploaded_by_name,
            created_at: row.created_at,
            download_url: None,
        }
    }
}

#[derive(Clone)]
pub struct TicketAttachmentRepo {
    db: PgPool,
}

impl TicketAttachmentRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn insert(
        &self,
        ticket_id: Uuid,
        object_key: &str,
        filename: &str,
        content_type: &str,
        size: i64,
        sha256: &str,
        uploaded_by: Uuid,
    ) -> DbResult<TicketAttachment> {
        let row = sqlx::query_as::<_, AttachmentRow>(
            "INSERT INTO ticket_attachments
                 (ticket_id, object_key, filename, content_type, size, sha256, uploaded_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, ticket_id, object_key, filename, content_type, size, sha256,
                       uploaded_by, NULL::text AS uploaded_by_name, created_at",
        )
        .bind(ticket_id)
        .bind(object_key)
        .bind(filename)
        .bind(content_type)
        .bind(size)
        .bind(sha256)
        .bind(uploaded_by)
        .fetch_one(&self.db)
        .await?;
        Ok(TicketAttachment::from(row))
    }

    pub async fn list(&self, ticket_id: Uuid) -> DbResult<Vec<TicketAttachment>> {
        let rows = sqlx::query_as::<_, AttachmentRow>(
            "SELECT a.id, a.ticket_id, a.object_key, a.filename, a.content_type, a.size,
                    a.sha256, a.uploaded_by, u.display_name AS uploaded_by_name, a.created_at
               FROM ticket_attachments a
               LEFT JOIN users u ON u.id = a.uploaded_by
              WHERE a.ticket_id = $1
              ORDER BY a.created_at ASC, a.id",
        )
        .bind(ticket_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(TicketAttachment::from).collect())
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<TicketAttachment>> {
        let row = sqlx::query_as::<_, AttachmentRow>(
            "SELECT a.id, a.ticket_id, a.object_key, a.filename, a.content_type, a.size,
                    a.sha256, a.uploaded_by, u.display_name AS uploaded_by_name, a.created_at
               FROM ticket_attachments a
               LEFT JOIN users u ON u.id = a.uploaded_by
              WHERE a.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.map(TicketAttachment::from))
    }

    /// 删除附件元数据；返回 `true` 表示确实删除了一行。
    pub async fn delete(&self, ticket_id: Uuid, id: Uuid) -> DbResult<bool> {
        let deleted =
            sqlx::query("DELETE FROM ticket_attachments WHERE id = $1 AND ticket_id = $2")
                .bind(id)
                .bind(ticket_id)
                .execute(&self.db)
                .await?
                .rows_affected();
        if deleted == 0 {
            return Err(DbError::not_found("附件不存在"));
        }
        Ok(true)
    }
}
