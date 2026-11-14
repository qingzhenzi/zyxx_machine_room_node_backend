//! 平台 ISO 资产库仓储（迁移 0041 / 0042，契约 `docs/image-studio.md` 附录 B / C）。
//!
//! 只负责 `install_isos` 的 SQL 与行映射；上传状态机/权限/seed 扫描编排在
//! `qe-api`（`api/admin/isos.rs` + `iso_seed.rs`）。
//!
//! 可见性过滤在 SQL 里做兜底（preset OR uploader OR admin），避免调用方忘了解析权限
//! 时泄露他人的私有 ISO。

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::install_iso::{
    InstallIso, IsoChunk, IsoStatus, IsoVisibility, NewInstallIso, ShareStatus,
};

use crate::error::DbResult;

const ISO_COLUMNS: &str = "i.id, i.filename, i.display_name, i.description, i.size_bytes,
     i.sha256, i.status, i.storage_key, i.is_preset,
     i.visibility, i.share_status, i.share_requested_by, i.share_reviewed_by,
     i.share_reviewed_at, i.share_note,
     i.upload_chunk_size, i.upload_total_chunks, i.upload_updated_at,
     i.uploader_user_id, u.display_name AS uploader_name,
     i.created_at, i.updated_at, i.uploaded_at";

/// 非终态会话状态（删除保护用，与 image_studio 的 TERMINAL_STATES 保持一致）。
const ACTIVE_SESSION_STATES: &str = "'done', 'failed', 'cancelled', 'expired'";

fn status_from_str(raw: &str) -> IsoStatus {
    raw.parse().unwrap_or(IsoStatus::Failed)
}

fn visibility_from_str(raw: &str) -> IsoVisibility {
    raw.parse().unwrap_or(IsoVisibility::Private)
}

fn share_status_from_str(raw: &str) -> ShareStatus {
    raw.parse().unwrap_or(ShareStatus::None)
}

#[derive(Debug, Clone, FromRow)]
struct InstallIsoRow {
    id: Uuid,
    filename: String,
    display_name: String,
    description: Option<String>,
    size_bytes: i64,
    sha256: Option<String>,
    status: String,
    storage_key: Option<String>,
    is_preset: bool,
    visibility: String,
    share_status: String,
    share_requested_by: Option<Uuid>,
    share_reviewed_by: Option<Uuid>,
    share_reviewed_at: Option<DateTime<Utc>>,
    share_note: Option<String>,
    upload_chunk_size: Option<i64>,
    upload_total_chunks: Option<i32>,
    upload_updated_at: Option<DateTime<Utc>>,
    uploader_user_id: Option<Uuid>,
    uploader_name: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    uploaded_at: Option<DateTime<Utc>>,
}

impl From<InstallIsoRow> for InstallIso {
    fn from(row: InstallIsoRow) -> Self {
        Self {
            id: row.id,
            filename: row.filename,
            display_name: row.display_name,
            description: row.description,
            size_bytes: row.size_bytes,
            sha256: row.sha256,
            status: status_from_str(&row.status),
            storage_key: row.storage_key,
            is_preset: row.is_preset,
            visibility: visibility_from_str(&row.visibility),
            share_status: share_status_from_str(&row.share_status),
            share_requested_by: row.share_requested_by,
            share_reviewed_by: row.share_reviewed_by,
            share_reviewed_at: row.share_reviewed_at,
            share_note: row.share_note,
            upload_chunk_size: row.upload_chunk_size,
            upload_total_chunks: row.upload_total_chunks,
            upload_updated_at: row.upload_updated_at,
            uploader_user_id: row.uploader_user_id,
            uploader_name: row.uploader_name,
            created_at: row.created_at,
            updated_at: row.updated_at,
            uploaded_at: row.uploaded_at,
        }
    }
}

/// ISO 资产仓储。
#[derive(Clone)]
pub struct InstallIsoRepo {
    db: PgPool,
}

impl InstallIsoRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 列表：`preset OR uploader=viewer OR admin` 可见；可选 `status` 过滤。
    pub async fn list(
        &self,
        viewer_user_id: Uuid,
        is_admin: bool,
        status: Option<IsoStatus>,
    ) -> DbResult<Vec<InstallIso>> {
        let status_text = status.map(|status| status.as_str().to_owned());
        let sql = format!(
            "SELECT {ISO_COLUMNS}
               FROM install_isos i
               LEFT JOIN users u ON u.id = i.uploader_user_id
              WHERE ($1::bool
                     OR i.is_preset
                     OR i.uploader_user_id = $2
                     OR (i.visibility = 'shared' AND i.share_status = 'approved'))
                AND ($3::text IS NULL OR i.status = $3)
              ORDER BY i.is_preset DESC, i.created_at DESC, i.id
              LIMIT 500"
        );
        let rows = sqlx::query_as::<_, InstallIsoRow>(&sql)
            .bind(is_admin)
            .bind(viewer_user_id)
            .bind(status_text.as_deref())
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(InstallIso::from).collect())
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<InstallIso>> {
        let sql = format!(
            "SELECT {ISO_COLUMNS}
               FROM install_isos i
               LEFT JOIN users u ON u.id = i.uploader_user_id
              WHERE i.id = $1"
        );
        let row = sqlx::query_as::<_, InstallIsoRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(InstallIso::from))
    }

    /// 按 filename（大小写不敏感）查重。
    pub async fn find_by_filename(&self, filename: &str) -> DbResult<Option<InstallIso>> {
        let sql = format!(
            "SELECT {ISO_COLUMNS}
               FROM install_isos i
               LEFT JOIN users u ON u.id = i.uploader_user_id
              WHERE lower(i.filename) = lower($1)
              LIMIT 1"
        );
        let row = sqlx::query_as::<_, InstallIsoRow>(&sql)
            .bind(filename)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(InstallIso::from))
    }

    /// 新建 `uploading` 记录（size=0、storage_key=NULL）。
    pub async fn create(&self, input: &NewInstallIso) -> DbResult<InstallIso> {
        let sql = format!(
            "INSERT INTO install_isos
                 (id, filename, display_name, description, size_bytes, sha256, status,
                  storage_key, is_preset, uploader_user_id,
                  upload_chunk_size, upload_total_chunks, upload_updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'uploading', NULL, $7, $8, $9, $10,
                     CASE WHEN $9::bigint IS NULL THEN NULL ELSE now() END)
             RETURNING id"
        );
        let id: Uuid = sqlx::query_scalar(&sql)
            .bind(input.id)
            .bind(&input.filename)
            .bind(&input.display_name)
            .bind(input.description.as_deref())
            .bind(input.size_bytes)
            .bind(input.sha256.as_deref())
            .bind(input.is_preset)
            .bind(input.uploader_user_id)
            .bind(input.upload_chunk_size)
            .bind(input.upload_total_chunks)
            .fetch_one(&self.db)
            .await?;
        // RETURNING 只取 id：再查一次拿 join 出的 uploader_name，保证响应形状统一。
        self.get(id)
            .await?
            .ok_or_else(|| crate::error::DbError::internal("刚创建的 ISO 记录不可见"))
    }

    /// 上传完成：置 `ready` 并回填大小/sha/storage_key/uploaded_at。
    pub async fn update_ready(
        &self,
        id: Uuid,
        size_bytes: i64,
        sha256: &str,
        storage_key: &str,
    ) -> DbResult<Option<InstallIso>> {
        let updated = sqlx::query(
            "UPDATE install_isos
                SET status = 'ready',
                    size_bytes = $2,
                    sha256 = $3,
                    storage_key = $4,
                    uploaded_at = now(),
                    updated_at = now()
              WHERE id = $1 AND status IN ('uploading', 'failed')",
        )
        .bind(id)
        .bind(size_bytes)
        .bind(sha256)
        .bind(storage_key)
        .execute(&self.db)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        self.get(id).await
    }

    /// 上传失败：置 `failed`（保留记录，便于前端展示/重试；重新上传需先删除再新建）。
    pub async fn update_failed(&self, id: Uuid) -> DbResult<Option<InstallIso>> {
        let updated = sqlx::query(
            "UPDATE install_isos
                SET status = 'failed', updated_at = now()
              WHERE id = $1 AND status = 'uploading'",
        )
        .bind(id)
        .execute(&self.db)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        self.get(id).await
    }

    /// 删除记录（调用方需先做删除保护/权限校验）。
    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let deleted = sqlx::query("DELETE FROM install_isos WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    /// 删除保护：是否有非终态构建会话引用该 ISO。
    pub async fn count_active_sessions(&self, iso_id: Uuid) -> DbResult<i64> {
        let sql = format!(
            "SELECT count(*)
               FROM image_build_sessions
              WHERE iso_id = $1
                AND state NOT IN ({ACTIVE_SESSION_STATES})"
        );
        sqlx::query_scalar(&sql)
            .bind(iso_id)
            .fetch_one(&self.db)
            .await
            .map_err(Into::into)
    }

    // ---- 分片上传（迁移 0042，附录 C.1） ------------------------------------

    /// 已接收分片（按 index 升序）。
    pub async fn list_chunks(&self, iso_id: Uuid) -> DbResult<Vec<IsoChunk>> {
        let rows = sqlx::query_as::<_, (i32, i64, String, DateTime<Utc>)>(
            "SELECT chunk_index, size_bytes, sha256, received_at
               FROM install_iso_chunks
              WHERE iso_id = $1
              ORDER BY chunk_index",
        )
        .bind(iso_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(chunk_index, size_bytes, sha256, received_at)| IsoChunk {
                iso_id,
                chunk_index,
                size_bytes,
                sha256,
                received_at,
            })
            .collect())
    }

    /// 记录分片（重复上传同 index 幂等覆盖），并刷新 `upload_updated_at`。
    pub async fn upsert_chunk(
        &self,
        iso_id: Uuid,
        chunk_index: i32,
        size_bytes: i64,
        sha256: &str,
    ) -> DbResult<()> {
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO install_iso_chunks (iso_id, chunk_index, size_bytes, sha256, received_at)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (iso_id, chunk_index) DO UPDATE
                SET size_bytes = EXCLUDED.size_bytes,
                    sha256 = EXCLUDED.sha256,
                    received_at = now()",
        )
        .bind(iso_id)
        .bind(chunk_index)
        .bind(size_bytes)
        .bind(sha256)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE install_isos SET upload_updated_at = now() WHERE id = $1")
            .bind(iso_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// 已收分片的 `(片数, 字节和)`（complete 前校验齐全用）。
    pub async fn received_summary(&self, iso_id: Uuid) -> DbResult<(i64, i64)> {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT count(*), COALESCE(sum(size_bytes), 0)::bigint
               FROM install_iso_chunks
              WHERE iso_id = $1",
        )
        .bind(iso_id)
        .fetch_one(&self.db)
        .await
        .map_err(Into::into)
    }

    /// 清理分片行（complete 成功 / 中止上传 / 单次 PUT 完成后调用）。
    pub async fn delete_chunks(&self, iso_id: Uuid) -> DbResult<u64> {
        let deleted = sqlx::query("DELETE FROM install_iso_chunks WHERE iso_id = $1")
            .bind(iso_id)
            .execute(&self.db)
            .await?
            .rows_affected();
        Ok(deleted)
    }

    // ---- 共享审批（迁移 0042，附录 C.2） ------------------------------------

    /// 申请共享：`none/rejected → pending`（preset 不能申请）。返回最新记录。
    pub async fn request_share(&self, iso_id: Uuid, actor: Uuid) -> DbResult<Option<InstallIso>> {
        sqlx::query(
            "UPDATE install_isos
                SET share_status = 'pending',
                    visibility = 'private',
                    share_requested_by = $2,
                    share_reviewed_by = NULL,
                    share_reviewed_at = NULL,
                    share_note = NULL,
                    updated_at = now()
              WHERE id = $1 AND is_preset = false
                AND share_status IN ('none', 'rejected')",
        )
        .bind(iso_id)
        .bind(actor)
        .execute(&self.db)
        .await?;
        self.get(iso_id).await
    }

    /// 取消 pending 申请 → `none`。
    pub async fn cancel_share(&self, iso_id: Uuid) -> DbResult<Option<InstallIso>> {
        sqlx::query(
            "UPDATE install_isos
                SET share_status = 'none',
                    share_requested_by = NULL,
                    share_reviewed_by = NULL,
                    share_reviewed_at = NULL,
                    share_note = NULL,
                    updated_at = now()
              WHERE id = $1 AND share_status = 'pending'",
        )
        .bind(iso_id)
        .execute(&self.db)
        .await?;
        self.get(iso_id).await
    }

    /// 管理员审批：`pending → approved(shared) / rejected(private)`。
    pub async fn review_share(
        &self,
        iso_id: Uuid,
        approve: bool,
        reviewer: Uuid,
        note: Option<&str>,
    ) -> DbResult<Option<InstallIso>> {
        let updated = sqlx::query(
            "UPDATE install_isos
                SET share_status = CASE WHEN $2 THEN 'approved' ELSE 'rejected' END,
                    visibility = CASE WHEN $2 THEN 'shared' ELSE 'private' END,
                    share_reviewed_by = $3,
                    share_reviewed_at = now(),
                    share_note = $4,
                    updated_at = now()
              WHERE id = $1 AND share_status = 'pending'",
        )
        .bind(iso_id)
        .bind(approve)
        .bind(reviewer)
        .bind(note)
        .execute(&self.db)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        self.get(iso_id).await
    }

    /// 撤销共享：`approved/shared → private/none`。
    pub async fn unshare(&self, iso_id: Uuid) -> DbResult<Option<InstallIso>> {
        sqlx::query(
            "UPDATE install_isos
                SET share_status = 'none',
                    visibility = 'private',
                    share_requested_by = NULL,
                    updated_at = now()
              WHERE id = $1 AND (visibility = 'shared' OR share_status IN ('approved', 'pending'))",
        )
        .bind(iso_id)
        .execute(&self.db)
        .await?;
        self.get(iso_id).await
    }

    /// 审批列表：`is_preset=false` 且共享状态非 none；可选按状态过滤。
    pub async fn list_share_requests(
        &self,
        status: Option<ShareStatus>,
    ) -> DbResult<Vec<InstallIso>> {
        let status_text = status.map(|status| status.as_str().to_owned());
        let sql = format!(
            "SELECT {ISO_COLUMNS}
               FROM install_isos i
               LEFT JOIN users u ON u.id = i.uploader_user_id
              WHERE i.is_preset = false
                AND i.share_status <> 'none'
                AND ($1::text IS NULL OR i.share_status = $1)
              ORDER BY i.updated_at DESC, i.id
              LIMIT 500"
        );
        let rows = sqlx::query_as::<_, InstallIsoRow>(&sql)
            .bind(status_text.as_deref())
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(InstallIso::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_session_state_list_matches_contract() {
        // 删除保护只拦非终态；与 image_studio 的终态集合互补。
        for state in ["done", "failed", "cancelled", "expired"] {
            assert!(ACTIVE_SESSION_STATES.contains(state));
        }
    }

    #[test]
    fn unknown_status_falls_back_to_failed() {
        assert_eq!(status_from_str("ready"), IsoStatus::Ready);
        assert_eq!(status_from_str("uploading"), IsoStatus::Uploading);
        assert_eq!(status_from_str("bogus"), IsoStatus::Failed);
    }

    #[test]
    fn sharing_text_fallbacks_are_safe() {
        assert_eq!(visibility_from_str("shared"), IsoVisibility::Shared);
        assert_eq!(visibility_from_str("bogus"), IsoVisibility::Private);
        assert_eq!(share_status_from_str("pending"), ShareStatus::Pending);
        assert_eq!(share_status_from_str("approved"), ShareStatus::Approved);
        assert_eq!(share_status_from_str("rejected"), ShareStatus::Rejected);
        assert_eq!(share_status_from_str("bogus"), ShareStatus::None);
    }
}
