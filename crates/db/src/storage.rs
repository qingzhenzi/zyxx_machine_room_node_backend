//! 对象存储与镜像管理（P0）仓储。
//!
//! 契约见 `docs/storage-design.md` §14.4；表结构见 `migrations/0011_storage.sql`。
//!
//! 约定：
//! - 领域模型来自 `qe_domain::storage`，`FromRow` 行模型只出现在本文件；
//! - 枚举在库中以 snake_case / lowercase 文本存储（与 §14.2 的 serde 命名一致），
//!   本文件用显式 match 双向映射，未知值退化为默认值而不是 panic（与 `schedule.rs`
//!   处理脏数据的风格一致）；
//! - `image_chunks."offset"` 是 SQL 保留字，所有 SQL 中都要加双引号；
//! - 多表写入（`replace_chunks` / `publish` / `assign_room`）在事务内完成。

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::{
    models::Page,
    storage::{
        ArtifactStatus, Collection, CollectionStatus, Compression, ImageArtifact, ImageAssignment,
        ImageChannel, ImageChannelRow, ImageChunk, ImageFilter, ImageFormat, ImageKind,
        ImageVerification, NewImage, StorageObject, Transfer, TransferDirection, TransferStatus,
    },
};

use crate::error::{DbError, DbResult};

// ---------------------------------------------------------------------------
// 枚举 ↔ 库内文本
// ---------------------------------------------------------------------------

fn kind_str(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::Docker => "docker",
        ImageKind::Oci => "oci",
        ImageKind::LxcRootfs => "lxc_rootfs",
        ImageKind::VmDisk => "vm_disk",
    }
}

fn kind_from_str(raw: &str) -> ImageKind {
    match raw {
        "oci" => ImageKind::Oci,
        "lxc_rootfs" => ImageKind::LxcRootfs,
        "vm_disk" => ImageKind::VmDisk,
        _ => ImageKind::Docker,
    }
}

fn format_str(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::DockerArchive => "docker_archive",
        ImageFormat::OciLayout => "oci_layout",
        ImageFormat::Tar => "tar",
        ImageFormat::Qcow2 => "qcow2",
        ImageFormat::Raw => "raw",
    }
}

fn format_from_str(raw: &str) -> ImageFormat {
    match raw {
        "oci_layout" => ImageFormat::OciLayout,
        "tar" => ImageFormat::Tar,
        "qcow2" => ImageFormat::Qcow2,
        "raw" => ImageFormat::Raw,
        _ => ImageFormat::DockerArchive,
    }
}

fn compression_str(compression: Compression) -> &'static str {
    match compression {
        Compression::None => "none",
        Compression::Zstd => "zstd",
        Compression::Gzip => "gzip",
    }
}

fn compression_from_str(raw: &str) -> Compression {
    match raw {
        "zstd" => Compression::Zstd,
        "gzip" => Compression::Gzip,
        _ => Compression::None,
    }
}

fn artifact_status_str(status: ArtifactStatus) -> &'static str {
    match status {
        ArtifactStatus::Uploaded => "uploaded",
        ArtifactStatus::ManifestReady => "manifest_ready",
        ArtifactStatus::Verifying => "verifying",
        ArtifactStatus::Verified => "verified",
        ArtifactStatus::Published => "published",
        ArtifactStatus::Deprecated => "deprecated",
        ArtifactStatus::Revoked => "revoked",
    }
}

fn artifact_status_from_str(raw: &str) -> ArtifactStatus {
    match raw {
        "manifest_ready" => ArtifactStatus::ManifestReady,
        "verifying" => ArtifactStatus::Verifying,
        "verified" => ArtifactStatus::Verified,
        "published" => ArtifactStatus::Published,
        "deprecated" => ArtifactStatus::Deprecated,
        "revoked" => ArtifactStatus::Revoked,
        _ => ArtifactStatus::Uploaded,
    }
}

#[cfg(test)]
fn transfer_direction_str(direction: TransferDirection) -> &'static str {
    match direction {
        TransferDirection::Download => "download",
        TransferDirection::Upload => "upload",
    }
}

fn transfer_direction_from_str(raw: &str) -> TransferDirection {
    match raw {
        "upload" => TransferDirection::Upload,
        _ => TransferDirection::Download,
    }
}

#[cfg(test)]
fn transfer_status_str(status: TransferStatus) -> &'static str {
    match status {
        TransferStatus::Pending => "pending",
        TransferStatus::Running => "running",
        TransferStatus::Completed => "completed",
        TransferStatus::Failed => "failed",
        TransferStatus::Expired => "expired",
    }
}

fn transfer_status_from_str(raw: &str) -> TransferStatus {
    match raw {
        "running" => TransferStatus::Running,
        "completed" => TransferStatus::Completed,
        "failed" => TransferStatus::Failed,
        "expired" => TransferStatus::Expired,
        _ => TransferStatus::Pending,
    }
}

#[cfg(test)]
fn collection_status_str(status: CollectionStatus) -> &'static str {
    match status {
        CollectionStatus::Pending => "pending",
        CollectionStatus::Uploaded => "uploaded",
        CollectionStatus::Verified => "verified",
        CollectionStatus::Rejected => "rejected",
    }
}

fn collection_status_from_str(raw: &str) -> CollectionStatus {
    match raw {
        "uploaded" => CollectionStatus::Uploaded,
        "verified" => CollectionStatus::Verified,
        "rejected" => CollectionStatus::Rejected,
        _ => CollectionStatus::Pending,
    }
}

fn channel_str(channel: ImageChannel) -> &'static str {
    match channel {
        ImageChannel::Dev => "dev",
        ImageChannel::Test => "test",
        ImageChannel::Prod => "prod",
    }
}

fn channel_from_str(raw: &str) -> ImageChannel {
    match raw {
        "test" => ImageChannel::Test,
        "prod" => ImageChannel::Prod,
        _ => ImageChannel::Dev,
    }
}

// ---------------------------------------------------------------------------
// 行模型与映射
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
struct StorageObjectRow {
    id: Uuid,
    bucket: String,
    object_key: String,
    sha256: String,
    size: i64,
    content_type: String,
    created_at: DateTime<Utc>,
}

impl From<StorageObjectRow> for StorageObject {
    fn from(row: StorageObjectRow) -> Self {
        Self {
            id: row.id,
            bucket: row.bucket,
            object_key: row.object_key,
            sha256: row.sha256,
            size: row.size,
            content_type: row.content_type,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ImageArtifactRow {
    id: Uuid,
    name: String,
    version: String,
    kind: String,
    format: String,
    compression: String,
    platform: String,
    manifest_digest: Option<String>,
    total_size: i64,
    status: String,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ImageArtifactRow> for ImageArtifact {
    fn from(row: ImageArtifactRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            version: row.version,
            kind: kind_from_str(&row.kind),
            format: format_from_str(&row.format),
            compression: compression_from_str(&row.compression),
            platform: row.platform,
            manifest_digest: row.manifest_digest,
            total_size: row.total_size,
            status: artifact_status_from_str(&row.status),
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ImageChunkRow {
    manifest_digest: String,
    index: i32,
    object_key: String,
    sha256: String,
    size: i64,
    offset: i64,
}

impl From<ImageChunkRow> for ImageChunk {
    fn from(row: ImageChunkRow) -> Self {
        Self {
            manifest_digest: row.manifest_digest,
            index: row.index,
            object_key: row.object_key,
            sha256: row.sha256,
            size: row.size,
            offset: row.offset,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ImageVerificationRow {
    id: Uuid,
    artifact_id: Uuid,
    ok: bool,
    report: Value,
    /// 用户被删除后为 NULL；领域模型要求 `Uuid`。
    verified_by: Option<Uuid>,
    verified_at: DateTime<Utc>,
}

impl From<ImageVerificationRow> for ImageVerification {
    fn from(row: ImageVerificationRow) -> Self {
        Self {
            id: row.id,
            artifact_id: row.artifact_id,
            ok: row.ok,
            report: row.report,
            verified_by: row.verified_by.unwrap_or_default(),
            verified_at: row.verified_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ImageChannelRowRow {
    id: Uuid,
    artifact_id: Uuid,
    channel: String,
    is_current: bool,
    created_at: DateTime<Utc>,
}

impl From<ImageChannelRowRow> for ImageChannelRow {
    fn from(row: ImageChannelRowRow) -> Self {
        Self {
            id: row.id,
            artifact_id: row.artifact_id,
            channel: channel_from_str(&row.channel),
            is_current: row.is_current,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ImageAssignmentRow {
    id: Uuid,
    image_id: Uuid,
    area_id: Option<Uuid>,
    room_id: Option<Uuid>,
    channel: String,
    rollout_state: String,
    /// 用户被删除后为 NULL；领域模型要求 `Uuid`。
    assigned_by: Option<Uuid>,
    assigned_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ImageAssignmentRow> for ImageAssignment {
    fn from(row: ImageAssignmentRow) -> Self {
        Self {
            id: row.id,
            image_id: row.image_id,
            area_id: row.area_id,
            room_id: row.room_id,
            channel: channel_from_str(&row.channel),
            rollout_state: row.rollout_state,
            assigned_by: row.assigned_by.unwrap_or_default(),
            assigned_at: row.assigned_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct TransferRow {
    id: Uuid,
    direction: String,
    room_id: Uuid,
    image_id: Option<Uuid>,
    manifest_digest: Option<String>,
    status: String,
    progress: i32,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TransferRow> for Transfer {
    fn from(row: TransferRow) -> Self {
        Self {
            id: row.id,
            direction: transfer_direction_from_str(&row.direction),
            room_id: row.room_id,
            image_id: row.image_id,
            manifest_digest: row.manifest_digest,
            status: transfer_status_from_str(&row.status),
            progress: row.progress,
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct CollectionRow {
    id: Uuid,
    area_id: Option<Uuid>,
    room_id: Option<Uuid>,
    kind: String,
    object_key: String,
    sha256: String,
    size: i64,
    status: String,
    created_at: DateTime<Utc>,
}

impl From<CollectionRow> for Collection {
    fn from(row: CollectionRow) -> Self {
        Self {
            id: row.id,
            area_id: row.area_id,
            room_id: row.room_id,
            kind: row.kind,
            object_key: row.object_key,
            sha256: row.sha256,
            size: row.size,
            status: collection_status_from_str(&row.status),
            created_at: row.created_at,
        }
    }
}

/// `assign_room` 的结果：分配记录 + 本次创建的分发任务（§16.3）。
///
/// 事务内同时 upsert `image_assignments` 并插入 `transfers`，两者一起返回，
/// 调用方可用 `transfer.id` 构造 `image.pull` 下行 payload。
#[derive(Debug, Clone)]
pub struct AssignOutcome {
    pub assignment: ImageAssignment,
    pub transfer: Transfer,
}

// ---------------------------------------------------------------------------
// 通用 SQL 片段
// ---------------------------------------------------------------------------

const STORAGE_OBJECT_COLUMNS: &str =
    "id, bucket, object_key, sha256, size, content_type, created_at";

const IMAGE_COLUMNS: &str = "id, name, version, kind, format, compression, platform,
     manifest_digest, total_size, status, created_by, created_at, updated_at";

const CHUNK_COLUMNS: &str = "manifest_digest, index, object_key, sha256, size, \"offset\"";

const VERIFICATION_COLUMNS: &str = "id, artifact_id, ok, report, verified_by, verified_at";

const CHANNEL_COLUMNS: &str = "id, artifact_id, channel, is_current, created_at";

const ASSIGNMENT_COLUMNS: &str = "id, image_id, area_id, room_id, channel,
     rollout_state, assigned_by, assigned_at, updated_at";

const TRANSFER_COLUMNS: &str = "id, direction, room_id, image_id, manifest_digest,
     status, progress, error, created_at, updated_at";

const COLLECTION_COLUMNS: &str = "id, area_id, room_id, kind, object_key, sha256,
     size, status, created_at";

/// `list_images` 的过滤条件（参数顺序：kind、status、name）。
const IMAGE_FILTER: &str = "($1::text IS NULL OR kind = $1)
   AND ($2::text IS NULL OR status = $2)
   AND ($3::text IS NULL OR $3::text = '' OR name ILIKE '%' || $3::text || '%')";

// ---------------------------------------------------------------------------
// 下行投递
// ---------------------------------------------------------------------------

/// 与 `qe-device-pki::messaging::MAX_UNACKED_DOWNLINK` / `schedule` 保持一致：
/// 单台设备未 ACK 的下行消息达到该值即拒绝入队（背压，避免设备失联时写爆库）。
const MAX_UNACKED_DOWNLINK: i64 = 1000;
/// 与 `qe-device-pki::messaging::MAX_MESSAGE_BYTES` 保持一致（64 KiB）。
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_DOWNLINK_KIND_LEN: usize = 64;

/// 下行消息大小校验（纯函数，无 DB，便于单测）：
/// `kind` 1..=64 字节；`payload` ≤ 64 KiB。与 device-pki 的 `/messages` 一致。
fn validate_downlink(kind: &str, payload: &str) -> DbResult<()> {
    if kind.is_empty() || kind.len() > MAX_DOWNLINK_KIND_LEN {
        return Err(DbError::bad_request("kind 长度必须在 1..=64"));
    }
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(DbError::bad_request("payload 超过 65536 字节"));
    }
    Ok(())
}

/// 背压校验（纯函数，无 DB）：单设备未 ACK 队列达到上限时返回 `Conflict`。
fn ensure_downlink_backpressure(unacked: i64) -> DbResult<()> {
    if unacked >= MAX_UNACKED_DOWNLINK {
        return Err(DbError::conflict("设备未 ACK 的下行消息已达上限"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 仓储
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct StorageRepo {
    db: PgPool,
}

impl StorageRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    // ---- 存储对象 ----------------------------------------------------------

    /// 按 `object_key` 幂等登记对象（同一 key 覆盖元数据），返回最终行。
    pub async fn upsert_object(
        &self,
        bucket: &str,
        object_key: &str,
        sha256: &str,
        size: i64,
        content_type: &str,
    ) -> DbResult<StorageObject> {
        let sql = format!(
            "INSERT INTO storage_objects (bucket, object_key, sha256, size, content_type)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (object_key) DO UPDATE
                SET bucket = EXCLUDED.bucket,
                    sha256 = EXCLUDED.sha256,
                    size = EXCLUDED.size,
                    content_type = EXCLUDED.content_type
             RETURNING {STORAGE_OBJECT_COLUMNS}"
        );
        let row = sqlx::query_as::<_, StorageObjectRow>(&sql)
            .bind(bucket)
            .bind(object_key)
            .bind(sha256)
            .bind(size)
            .bind(content_type)
            .fetch_one(&self.db)
            .await?;
        Ok(StorageObject::from(row))
    }

    /// 按内容摘要取最近登记的单个对象。
    pub async fn get_object(&self, sha256: &str) -> DbResult<Option<StorageObject>> {
        let sql = format!(
            "SELECT {STORAGE_OBJECT_COLUMNS}
             FROM storage_objects
             WHERE sha256 = $1
             ORDER BY created_at DESC, id
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, StorageObjectRow>(&sql)
            .bind(sha256)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(StorageObject::from))
    }

    // ---- 镜像制品 ----------------------------------------------------------

    pub async fn create_image(&self, input: &NewImage) -> DbResult<ImageArtifact> {
        let sql = format!(
            "INSERT INTO image_artifacts
                (name, version, kind, format, compression, platform, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING {IMAGE_COLUMNS}"
        );
        let row = sqlx::query_as::<_, ImageArtifactRow>(&sql)
            .bind(&input.name)
            .bind(&input.version)
            .bind(kind_str(input.kind))
            .bind(format_str(input.format))
            .bind(compression_str(input.compression))
            .bind(&input.platform)
            .bind(input.created_by)
            .fetch_one(&self.db)
            .await?;
        Ok(ImageArtifact::from(row))
    }

    pub async fn list_images(
        &self,
        filter: &ImageFilter,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<ImageArtifact>> {
        let offset = (page - 1) * page_size;
        let kind = filter.kind.map(kind_str);
        let status = filter.status.map(artifact_status_str);

        let count_sql = format!("SELECT count(*) FROM image_artifacts WHERE {IMAGE_FILTER}");
        let total: i64 = sqlx::query_scalar(&count_sql)
            .bind(kind)
            .bind(status)
            .bind(filter.name.as_deref())
            .fetch_one(&self.db)
            .await?;

        let sql = format!(
            "SELECT {IMAGE_COLUMNS}
             FROM image_artifacts
             WHERE {IMAGE_FILTER}
             ORDER BY created_at DESC, id
             LIMIT $4 OFFSET $5"
        );
        let rows = sqlx::query_as::<_, ImageArtifactRow>(&sql)
            .bind(kind)
            .bind(status)
            .bind(filter.name.as_deref())
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(ImageArtifact::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get_image(&self, id: Uuid) -> DbResult<Option<ImageArtifact>> {
        let sql = format!("SELECT {IMAGE_COLUMNS} FROM image_artifacts WHERE id = $1");
        let row = sqlx::query_as::<_, ImageArtifactRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(ImageArtifact::from))
    }

    /// manifest 写入 S3 后登记摘要与总大小，状态推进到 `manifest_ready`。
    pub async fn mark_manifest_ready(
        &self,
        id: Uuid,
        manifest_digest: &str,
        total_size: i64,
    ) -> DbResult<Option<ImageArtifact>> {
        let sql = format!(
            "UPDATE image_artifacts
             SET manifest_digest = $2,
                 total_size = $3,
                 status = $4,
                 updated_at = now()
             WHERE id = $1
             RETURNING {IMAGE_COLUMNS}"
        );
        let row = sqlx::query_as::<_, ImageArtifactRow>(&sql)
            .bind(id)
            .bind(manifest_digest)
            .bind(total_size)
            .bind(artifact_status_str(ArtifactStatus::ManifestReady))
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(ImageArtifact::from))
    }

    pub async fn set_status(
        &self,
        id: Uuid,
        status: ArtifactStatus,
    ) -> DbResult<Option<ImageArtifact>> {
        let sql = format!(
            "UPDATE image_artifacts
             SET status = $2, updated_at = now()
             WHERE id = $1
             RETURNING {IMAGE_COLUMNS}"
        );
        let row = sqlx::query_as::<_, ImageArtifactRow>(&sql)
            .bind(id)
            .bind(artifact_status_str(status))
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(ImageArtifact::from))
    }

    /// 全量替换某 manifest 的分片（事务内 DELETE + INSERT）。
    pub async fn replace_chunks(
        &self,
        manifest_digest: &str,
        chunks: &[ImageChunk],
    ) -> DbResult<()> {
        let mut tx = self.db.begin().await?;

        sqlx::query("DELETE FROM image_chunks WHERE manifest_digest = $1")
            .bind(manifest_digest)
            .execute(&mut *tx)
            .await?;

        for chunk in chunks {
            sqlx::query(
                "INSERT INTO image_chunks
                    (manifest_digest, index, object_key, sha256, size, \"offset\")
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(manifest_digest)
            .bind(chunk.index)
            .bind(&chunk.object_key)
            .bind(&chunk.sha256)
            .bind(chunk.size)
            .bind(chunk.offset)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn chunks(&self, manifest_digest: &str) -> DbResult<Vec<ImageChunk>> {
        let sql = format!(
            "SELECT {CHUNK_COLUMNS}
             FROM image_chunks
             WHERE manifest_digest = $1
             ORDER BY index"
        );
        let rows = sqlx::query_as::<_, ImageChunkRow>(&sql)
            .bind(manifest_digest)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(ImageChunk::from).collect())
    }

    /// 某 manifest 的分片数（§16.3：构造 `image.pull` payload 的 `chunk_count`）。
    pub async fn chunk_count(&self, manifest_digest: &str) -> DbResult<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM image_chunks WHERE manifest_digest = $1",
        )
        .bind(manifest_digest)
        .fetch_one(&self.db)
        .await?;
        Ok(count)
    }

    // ---- 验证 / 发布 -------------------------------------------------------

    pub async fn insert_verification(
        &self,
        artifact_id: Uuid,
        ok: bool,
        report: Value,
        verified_by: Uuid,
    ) -> DbResult<ImageVerification> {
        let sql = format!(
            "INSERT INTO image_verifications (artifact_id, ok, report, verified_by)
             VALUES ($1, $2, $3, $4)
             RETURNING {VERIFICATION_COLUMNS}"
        );
        let row = sqlx::query_as::<_, ImageVerificationRow>(&sql)
            .bind(artifact_id)
            .bind(ok)
            .bind(&report)
            .bind(verified_by)
            .fetch_one(&self.db)
            .await?;
        Ok(ImageVerification::from(row))
    }

    pub async fn latest_verification(
        &self,
        artifact_id: Uuid,
    ) -> DbResult<Option<ImageVerification>> {
        let sql = format!(
            "SELECT {VERIFICATION_COLUMNS}
             FROM image_verifications
             WHERE artifact_id = $1
             ORDER BY verified_at DESC, id
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, ImageVerificationRow>(&sql)
            .bind(artifact_id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(ImageVerification::from))
    }

    /// 发布：仅 `verified` 可发布（其它状态返回 `Conflict`），并 upsert 通道当前版本。
    ///
    /// 未找到制品返回 `None`；状态不合法返回 `DbError::Conflict`，便于 API 映射 409。
    pub async fn publish(
        &self,
        id: Uuid,
        channel: ImageChannel,
    ) -> DbResult<Option<ImageArtifact>> {
        let mut tx = self.db.begin().await?;

        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM image_artifacts WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(status) = status else {
            tx.rollback().await?;
            return Ok(None);
        };
        if status != artifact_status_str(ArtifactStatus::Verified) {
            tx.rollback().await?;
            return Err(DbError::conflict("只有已验证（verified）的镜像才能发布"));
        }

        let sql = format!(
            "UPDATE image_artifacts
             SET status = $2, updated_at = now()
             WHERE id = $1
             RETURNING {IMAGE_COLUMNS}"
        );
        let row = sqlx::query_as::<_, ImageArtifactRow>(&sql)
            .bind(id)
            .bind(artifact_status_str(ArtifactStatus::Published))
            .fetch_one(&mut *tx)
            .await?;

        // 同一通道只保留最新发布的当前版本。
        sqlx::query(
            "UPDATE image_channels SET is_current = FALSE
             WHERE channel = $1 AND artifact_id <> $2",
        )
        .bind(channel_str(channel))
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO image_channels (artifact_id, channel, is_current)
             VALUES ($1, $2, TRUE)
             ON CONFLICT (artifact_id, channel) DO UPDATE SET is_current = TRUE",
        )
        .bind(id)
        .bind(channel_str(channel))
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(ImageArtifact::from(row)))
    }

    pub async fn channels(&self, id: Uuid) -> DbResult<Vec<ImageChannelRow>> {
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS}
             FROM image_channels
             WHERE artifact_id = $1
             ORDER BY created_at, id"
        );
        let rows = sqlx::query_as::<_, ImageChannelRowRow>(&sql)
            .bind(id)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(ImageChannelRow::from).collect())
    }

    // ---- 分配 / 分发 -------------------------------------------------------

    /// 分配镜像到房间：镜像必须已发布、房间必须存在；事务内 upsert assignment
    /// 并创建一条 `direction = 'download'` 的 pending transfer。
    ///
    /// 未找到镜像 / 房间返回 `None`；镜像未发布返回 `DbError::Conflict`。
    pub async fn assign_room(
        &self,
        image_id: Uuid,
        room_id: Uuid,
        channel: ImageChannel,
        actor: Uuid,
    ) -> DbResult<Option<AssignOutcome>> {
        let mut tx = self.db.begin().await?;

        let image = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, manifest_digest FROM image_artifacts WHERE id = $1 FOR UPDATE",
        )
        .bind(image_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((status, manifest_digest)) = image else {
            tx.rollback().await?;
            return Ok(None);
        };
        if status != artifact_status_str(ArtifactStatus::Published) {
            tx.rollback().await?;
            return Err(DbError::conflict("镜像尚未发布，无法分配到房间"));
        }

        let room_exists = sqlx::query_scalar::<_, Uuid>("SELECT id FROM rooms WHERE id = $1")
            .bind(room_id)
            .fetch_optional(&mut *tx)
            .await?;
        if room_exists.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        let sql = format!(
            "INSERT INTO image_assignments
                (image_id, room_id, channel, rollout_state, assigned_by, assigned_at, updated_at)
             VALUES ($1, $2, $3, 'pending', $4, now(), now())
             ON CONFLICT (room_id, image_id) DO UPDATE
                SET channel = EXCLUDED.channel,
                    rollout_state = 'pending',
                    assigned_by = EXCLUDED.assigned_by,
                    updated_at = now()
             RETURNING {ASSIGNMENT_COLUMNS}"
        );
        let row = sqlx::query_as::<_, ImageAssignmentRow>(&sql)
            .bind(image_id)
            .bind(room_id)
            .bind(channel_str(channel))
            .bind(actor)
            .fetch_one(&mut *tx)
            .await?;

        let transfer_sql = format!(
            "INSERT INTO transfers
                (direction, room_id, image_id, manifest_digest, status, progress)
             VALUES ('download', $1, $2, $3, 'pending', 0)
             RETURNING {TRANSFER_COLUMNS}"
        );
        let transfer_row = sqlx::query_as::<_, TransferRow>(&transfer_sql)
            .bind(room_id)
            .bind(image_id)
            .bind(manifest_digest)
            .fetch_one(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(Some(AssignOutcome {
            assignment: ImageAssignment::from(row),
            transfer: Transfer::from(transfer_row),
        }))
    }

    pub async fn unassign_room(&self, image_id: Uuid, room_id: Uuid) -> DbResult<bool> {
        let deleted =
            sqlx::query("DELETE FROM image_assignments WHERE image_id = $1 AND room_id = $2")
                .bind(image_id)
                .bind(room_id)
                .execute(&self.db)
                .await?
                .rows_affected();
        Ok(deleted > 0)
    }

    pub async fn list_transfers(
        &self,
        room_id: Option<Uuid>,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Transfer>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM transfers
             WHERE ($1::uuid IS NULL OR room_id = $1)",
        )
        .bind(room_id)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {TRANSFER_COLUMNS}
             FROM transfers
             WHERE ($1::uuid IS NULL OR room_id = $1)
             ORDER BY created_at DESC, id
             LIMIT $2 OFFSET $3"
        );
        let rows = sqlx::query_as::<_, TransferRow>(&sql)
            .bind(room_id)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(Transfer::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get_transfer(&self, id: Uuid) -> DbResult<Option<Transfer>> {
        let sql = format!("SELECT {TRANSFER_COLUMNS} FROM transfers WHERE id = $1");
        let row = sqlx::query_as::<_, TransferRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Transfer::from))
    }

    /// 仅 `failed` / `expired` 可重试：重置为 pending，progress=0，error=NULL。
    pub async fn retry_transfer(&self, id: Uuid) -> DbResult<bool> {
        let updated = sqlx::query(
            "UPDATE transfers
             SET status = 'pending', progress = 0, error = NULL, updated_at = now()
             WHERE id = $1 AND status IN ('failed', 'expired')",
        )
        .bind(id)
        .execute(&self.db)
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    /// 向设备槽位投递下行消息：返回 `Some(serial)` / `None`（无存活设备）。
    ///
    /// 语义与 `schedule::ScheduleEventRepo::enqueue_downlink_for_slot` /
    /// `qe-device-pki::messaging::downlink_publish_to_slot` 一致：
    /// - 选 `status ∈ {issued, active}` 的证书，`active` 优先、`activated_at` 最新；
    /// - 未 ACK 的下行消息达到 1000 条时返回 `Conflict`（背压）；
    /// - 成功插入 `device_downlink` 后返回设备 `serial`（调用方可据此唤醒长连接）。
    pub async fn enqueue_downlink_for_slot(
        &self,
        slot: &str,
        kind: &str,
        payload: &str,
    ) -> DbResult<Option<String>> {
        validate_downlink(kind, payload)?;

        let mut tx = self.db.begin().await?;

        let device = sqlx::query_as::<_, (String, String)>(
            "SELECT serial, device_slot
             FROM device_enrollment
             WHERE device_slot = $1 AND status IN ('issued', 'active')
             ORDER BY (status = 'active') DESC, activated_at DESC NULLS LAST
             LIMIT 1",
        )
        .bind(slot)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((serial, device_slot)) = device else {
            tx.rollback().await?;
            return Ok(None);
        };

        let unacked: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM device_downlink
             WHERE serial = $1 AND delivered_at IS NULL",
        )
        .bind(&serial)
        .fetch_one(&mut *tx)
        .await?;
        if let Err(error) = ensure_downlink_backpressure(unacked) {
            tx.rollback().await?;
            return Err(error);
        }

        sqlx::query(
            "INSERT INTO device_downlink (serial, device_slot, kind, payload)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&serial)
        .bind(&device_slot)
        .bind(kind)
        .bind(payload)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(serial))
    }

    pub async fn list_collections(
        &self,
        room_id: Option<Uuid>,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Collection>> {
        let offset = (page - 1) * page_size;

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM collections
             WHERE ($1::uuid IS NULL OR room_id = $1)",
        )
        .bind(room_id)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {COLLECTION_COLUMNS}
             FROM collections
             WHERE ($1::uuid IS NULL OR room_id = $1)
             ORDER BY created_at DESC, id
             LIMIT $2 OFFSET $3"
        );
        let rows = sqlx::query_as::<_, CollectionRow>(&sql)
            .bind(room_id)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(Collection::from).collect(),
            total,
            page,
            page_size,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_text_mappings_round_trip() {
        for kind in [
            ImageKind::Docker,
            ImageKind::Oci,
            ImageKind::LxcRootfs,
            ImageKind::VmDisk,
        ] {
            assert_eq!(kind_from_str(kind_str(kind)), kind);
        }
        for format in [
            ImageFormat::DockerArchive,
            ImageFormat::OciLayout,
            ImageFormat::Tar,
            ImageFormat::Qcow2,
            ImageFormat::Raw,
        ] {
            assert_eq!(format_from_str(format_str(format)), format);
        }
        for compression in [Compression::None, Compression::Zstd, Compression::Gzip] {
            assert_eq!(
                compression_from_str(compression_str(compression)),
                compression
            );
        }
        for status in [
            ArtifactStatus::Uploaded,
            ArtifactStatus::ManifestReady,
            ArtifactStatus::Verifying,
            ArtifactStatus::Verified,
            ArtifactStatus::Published,
            ArtifactStatus::Deprecated,
            ArtifactStatus::Revoked,
        ] {
            assert_eq!(
                artifact_status_from_str(artifact_status_str(status)),
                status
            );
        }
        for direction in [TransferDirection::Download, TransferDirection::Upload] {
            assert_eq!(
                transfer_direction_from_str(transfer_direction_str(direction)),
                direction
            );
        }
        for status in [
            TransferStatus::Pending,
            TransferStatus::Running,
            TransferStatus::Completed,
            TransferStatus::Failed,
            TransferStatus::Expired,
        ] {
            assert_eq!(
                transfer_status_from_str(transfer_status_str(status)),
                status
            );
        }
        for status in [
            CollectionStatus::Pending,
            CollectionStatus::Uploaded,
            CollectionStatus::Verified,
            CollectionStatus::Rejected,
        ] {
            assert_eq!(
                collection_status_from_str(collection_status_str(status)),
                status
            );
        }
        for channel in [ImageChannel::Dev, ImageChannel::Test, ImageChannel::Prod] {
            assert_eq!(channel_from_str(channel_str(channel)), channel);
        }
    }

    #[test]
    fn downlink_limits_match_contract() {
        // 与 qe-device-pki::messaging 的常量保持一致。
        assert_eq!(MAX_UNACKED_DOWNLINK, 1000);
        assert_eq!(MAX_MESSAGE_BYTES, 64 * 1024);
        assert_eq!(MAX_DOWNLINK_KIND_LEN, 64);
    }

    #[test]
    fn downlink_validation_enforces_kind_and_payload_limits() {
        // kind 1..=64 字节；payload ≤ 64 KiB。
        assert!(validate_downlink("image.pull", "{}").is_ok());
        assert!(validate_downlink(&"k".repeat(MAX_DOWNLINK_KIND_LEN), "{}").is_ok());
        assert!(validate_downlink("", "{}").is_err());
        assert!(validate_downlink(&"k".repeat(MAX_DOWNLINK_KIND_LEN + 1), "{}").is_err());
        assert!(validate_downlink("image.pull", &"x".repeat(MAX_MESSAGE_BYTES)).is_ok());
        assert!(validate_downlink("image.pull", &"x".repeat(MAX_MESSAGE_BYTES + 1)).is_err());

        // 超限归类为 400（bad request），便于 API 映射。
        assert!(matches!(
            validate_downlink("", "{}"),
            Err(DbError::BadRequest(_))
        ));
        assert!(matches!(
            validate_downlink("image.pull", &"x".repeat(MAX_MESSAGE_BYTES + 1)),
            Err(DbError::BadRequest(_))
        ));
    }

    #[test]
    fn downlink_backpressure_conflicts_at_limit() {
        // 999 未 ACK 仍可入队；达到 1000 即 Conflict（背压）。
        assert!(ensure_downlink_backpressure(0).is_ok());
        assert!(ensure_downlink_backpressure(MAX_UNACKED_DOWNLINK - 1).is_ok());
        assert!(matches!(
            ensure_downlink_backpressure(MAX_UNACKED_DOWNLINK),
            Err(DbError::Conflict(_))
        ));
        assert!(matches!(
            ensure_downlink_backpressure(MAX_UNACKED_DOWNLINK + 1),
            Err(DbError::Conflict(_))
        ));
    }

    #[test]
    fn unknown_db_text_falls_back_without_panic() {
        assert_eq!(artifact_status_from_str("bogus"), ArtifactStatus::Uploaded);
        assert_eq!(transfer_status_from_str("bogus"), TransferStatus::Pending);
        assert_eq!(
            collection_status_from_str("bogus"),
            CollectionStatus::Pending
        );
        assert_eq!(channel_from_str("bogus"), ImageChannel::Dev);
        assert_eq!(kind_from_str("bogus"), ImageKind::Docker);
        assert_eq!(format_from_str("bogus"), ImageFormat::DockerArchive);
        assert_eq!(compression_from_str("bogus"), Compression::None);
        assert_eq!(
            transfer_direction_from_str("bogus"),
            TransferDirection::Download
        );
    }
}

/// 镜像中心收尾（2026-11-08）：删除镜像与对象引用清理辅助。
impl StorageRepo {
    /// 删除镜像（含分片/分配/校验/通道等级联行与 p2p seed），返回被删的 artifact。
    ///
    /// `image_verifications` / `image_channels` / `image_assignments` 由外键级联删除；
    /// `image_chunks` / `image_p2p_seeds` 以 manifest_digest 关联，需要显式删除。
    pub async fn delete_image_cascade(&self, id: Uuid) -> DbResult<Option<ImageArtifact>> {
        let Some(artifact) = self.get_image(id).await? else {
            return Ok(None);
        };
        let mut tx = self.db.begin().await?;
        if let Some(digest) = artifact.manifest_digest.as_deref() {
            sqlx::query("DELETE FROM image_p2p_seeds WHERE manifest_digest = $1")
                .bind(digest)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM image_chunks WHERE manifest_digest = $1")
                .bind(digest)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM image_artifacts WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(artifact))
    }

    /// 该 sha256 分片仍被多少个镜像 manifest 引用（用于判断能否删除 S3 对象）。
    pub async fn chunk_reference_count(&self, sha256: &str) -> DbResult<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM image_chunks WHERE sha256 = $1")
            .bind(sha256)
            .fetch_one(&self.db)
            .await?;
        Ok(count)
    }

    /// 删除 `storage_objects` 登记行（对象本体由调用方按引用计数删除）。
    pub async fn delete_storage_object_row(&self, sha256: &str) -> DbResult<()> {
        sqlx::query("DELETE FROM storage_objects WHERE sha256 = $1")
            .bind(sha256)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}
