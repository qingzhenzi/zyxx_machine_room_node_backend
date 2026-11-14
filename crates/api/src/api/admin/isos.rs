//! 平台 ISO 资产库管理端接口（契约 `docs/image-studio.md` 附录 B.2）。
//!
//! - `GET    /api/v1/admin/isos?status=ready`：列表（preset 所有人可见，非 preset 仅
//!   uploader/admin）；读取权限 `image:read`；
//! - `POST   /api/v1/admin/isos`：登记 `uploading` 记录，返回 `upload_url`；
//! - `PUT    /api/v1/admin/isos/{id}/content`：**原始 body 流式**落临时文件，边写边算
//!   SHA-256（上限 16 GiB），完成后流式上传 S3 并置 `ready`；
//! - `DELETE /api/v1/admin/isos/{id}`：删除记录 + S3 对象；有非终态构建会话引用 → 409；
//! - `POST   /api/v1/admin/isos/seed/scan`：管理员触发 seed 目录扫描（见 `iso_seed`）。
//!
//! v4（附录 C.1/C.2）：
//! - 分片续传上传：`POST /admin/isos`（size_bytes + chunk_size_mib）→
//!   `GET /admin/isos/{id}/upload` / `PUT /admin/isos/{id}/chunks/{index}` /
//!   `POST /admin/isos/{id}/complete` / `DELETE /admin/isos/{id}/upload`；
//!   `PUT /content` 单次上传保留兼容；
//! - 共享审批：`share-request` / `share-cancel` / `share-requests` / `share-review` / `unshare`。
//!
//! 注意：`PUT content` 不挂全局 2 MiB body 限制 / 30s 超时，由 `api/mod.rs` 的独立
//! 长上传路由承载（16 GiB、2 小时）。

use std::time::Duration;

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use qe_domain::{
    install_iso::{
        InstallIso, IsoChunk, IsoStatus, IsoVisibility, MAX_ISO_BYTES, MIB, NewInstallIso,
        ShareStatus, can_request_share, chunk_count, expected_chunk_len, is_iso_shared_public,
        iso_storage_key, missing_chunk_indices, resolve_chunk_size_mib, upload_plan,
        validate_iso_filename, validate_upload_size,
    },
    rbac::is_admin as role_is_admin,
    storage::validate_sha256_hex,
};

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::rbac::permissions,
    error::{AppError, AppResult},
    iso_seed,
    state::AppState,
};

/// ISO 上传 S3 写入超时（4.7GB 在慢链路上可能 >30 分钟）。
const ISO_UPLOAD_S3_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
/// description 最大长度。
const MAX_DESCRIPTION_CHARS: usize = 500;
/// 使用中会话引用（删除保护）返回的 409 文案。
const ISO_IN_USE_MESSAGE: &str = "该 ISO 正被进行中的镜像构建会话使用，请先取消/完成会话";

// ---------------------------------------------------------------------------
// 请求 / 响应
// ---------------------------------------------------------------------------

/// `GET /admin/isos` 查询参数。
#[derive(Debug, Deserialize)]
pub struct IsoListQuery {
    /// `uploading` / `ready` / `failed`；缺省全部。
    #[serde(default)]
    pub status: Option<String>,
}

/// `POST /admin/isos` 请求体（契约 B.2）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateIsoRequest {
    pub filename: String,
    /// 文件总大小（字节，必填，>0 且 ≤16 GiB）。
    pub size_bytes: u64,
    /// 可选：分片大小（MiB，8..=256；缺省 `ISO_UPLOAD_CHUNK_MIB`）。
    #[serde(default)]
    pub chunk_size_mib: Option<u32>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// 可选：调用方预声明 sha256（complete/PUT 完成时必须匹配）。
    #[serde(default)]
    pub sha256: Option<String>,
}

/// POST 返回的分片上传计划。
#[derive(Debug, Clone, Serialize)]
pub struct CreateUploadInfo {
    /// 分片大小（字节）。
    pub chunk_size: i64,
    /// 分片大小（MiB，便于前端展示/计算）。
    pub chunk_size_mib: u32,
    pub total_chunks: i32,
    /// 已收到分片索引（续传时为已上传部分）。
    pub received: Vec<i32>,
}

/// 上传状态响应（`GET /admin/isos/{id}/upload`）。
#[derive(Debug, Clone, Serialize)]
pub struct UploadStatusReply {
    pub status: IsoStatus,
    pub size_bytes: i64,
    pub chunk_size: i64,
    pub chunk_size_mib: u32,
    pub total_chunks: i32,
    pub received: Vec<i32>,
    pub received_bytes: i64,
    pub sha256: Option<String>,
}

/// 分片上传响应。
#[derive(Debug, Clone, Serialize)]
pub struct ChunkUploadReply {
    pub received_count: i64,
    pub received_bytes: i64,
}

/// `POST /admin/isos/{id}/complete` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteIsoRequest {
    #[serde(default)]
    pub sha256: Option<String>,
}

/// `POST /admin/isos/{id}/share-review` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareReviewRequest {
    pub approve: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// complete 缺片时的 409 响应（额外带 `missing` 索引数组）。
#[derive(Debug)]
pub enum CompleteIsoError {
    Missing(Vec<i32>),
    App(AppError),
}

impl From<AppError> for CompleteIsoError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<qe_db::error::DbError> for CompleteIsoError {
    fn from(error: qe_db::error::DbError) -> Self {
        Self::App(AppError::from(error))
    }
}

impl IntoResponse for CompleteIsoError {
    fn into_response(self) -> Response {
        match self {
            Self::Missing(missing) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": {
                        "code": "missing_chunks",
                        "message": format!("缺少 {} 个分片，请先上传", missing.len()),
                        "missing": missing,
                    }
                })),
            )
                .into_response(),
            Self::App(error) => error.into_response(),
        }
    }
}

/// `POST /admin/isos` 响应：上传地址（相对路径，前端拼 control host）。
#[derive(Debug, Serialize)]
pub struct CreateIsoReply {
    pub id: Uuid,
    /// v3 单次 PUT 兼容地址（`PUT /content`）。
    pub upload_url: String,
    /// v4 分片上传计划。
    pub upload: CreateUploadInfo,
}

/// ISO 列表/详情响应（不暴露内部 `storage_key`）。
#[derive(Debug, Clone, Serialize)]
pub struct IsoReply {
    pub id: Uuid,
    pub filename: String,
    pub display_name: String,
    pub description: Option<String>,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    pub status: IsoStatus,
    pub is_preset: bool,
    // ---- v4 共享审批（附录 C.2）----
    pub visibility: IsoVisibility,
    pub share_status: ShareStatus,
    pub is_shared: bool,
    pub share_note: Option<String>,
    pub share_requested_by: Option<Uuid>,
    pub share_reviewed_by: Option<Uuid>,
    pub share_reviewed_at: Option<DateTime<Utc>>,
    pub uploader_user_id: Option<Uuid>,
    pub uploader_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub uploaded_at: Option<DateTime<Utc>>,
}

impl From<InstallIso> for IsoReply {
    fn from(iso: InstallIso) -> Self {
        let is_shared = is_iso_shared_public(&iso);
        Self {
            id: iso.id,
            filename: iso.filename,
            display_name: iso.display_name,
            description: iso.description,
            size_bytes: iso.size_bytes,
            sha256: iso.sha256,
            status: iso.status,
            is_preset: iso.is_preset,
            visibility: iso.visibility,
            share_status: iso.share_status,
            is_shared,
            share_note: iso.share_note,
            share_requested_by: iso.share_requested_by,
            share_reviewed_by: iso.share_reviewed_by,
            share_reviewed_at: iso.share_reviewed_at,
            uploader_user_id: iso.uploader_user_id,
            uploader_name: iso.uploader_name,
            created_at: iso.created_at,
            updated_at: iso.updated_at,
            uploaded_at: iso.uploaded_at,
        }
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

async fn actor_is_admin(state: &AppState, user_id: Uuid) -> AppResult<bool> {
    Ok(role_is_admin(&state.users.roles_of(user_id).await?))
}

fn parse_status_filter(raw: Option<&str>) -> AppResult<Option<IsoStatus>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => value
            .parse::<IsoStatus>()
            .map(Some)
            .map_err(|_| AppError::bad_request("status 只能是 uploading / ready / failed")),
    }
}

fn normalize_optional_text(
    raw: Option<String>,
    max_chars: usize,
    field: &str,
) -> AppResult<Option<String>> {
    match raw {
        None => Ok(None),
        Some(value) => {
            let value = value.trim().to_owned();
            if value.is_empty() {
                return Ok(None);
            }
            if value.chars().count() > max_chars || value.chars().any(char::is_control) {
                return Err(AppError::bad_request(format!(
                    "{field} 不能超过 {max_chars} 个字符且不含控制字符"
                )));
            }
            Ok(Some(value))
        }
    }
}

/// 上传专用临时目录：`<DATA_DIR>/iso-uploads`。
fn upload_temp_dir(state: &AppState) -> std::path::PathBuf {
    state.config.data_dir.join("iso-uploads")
}

fn upload_temp_path(state: &AppState, id: Uuid) -> std::path::PathBuf {
    upload_temp_dir(state).join(format!("{id}.part"))
}

async fn cleanup_temp(path: &std::path::Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), "清理 ISO 上传临时文件失败");
    }
}

/// 上传失败收尾：置 failed + 清理临时文件（尽力而为，不覆盖原始错误）。
async fn fail_upload(state: &AppState, id: Uuid, temp_path: &std::path::Path) {
    if let Err(error) = state.isos.update_failed(id).await {
        tracing::warn!(%error, %id, "标记 ISO 上传失败状态失败");
    }
    cleanup_temp(temp_path).await;
}

/// 分片目录：`<DATA_DIR>/iso-uploads/<iso_id>/`。
fn chunk_dir(state: &AppState, iso_id: Uuid) -> std::path::PathBuf {
    upload_temp_dir(state).join(iso_id.to_string())
}

/// 单个分片文件：`<iso_id>/<index>.part`。
fn chunk_path(state: &AppState, iso_id: Uuid, index: i32) -> std::path::PathBuf {
    chunk_dir(state, iso_id).join(format!("{index}.part"))
}

/// complete 拼接临时文件（成功/失败后都清理）。
fn complete_temp_path(state: &AppState, iso_id: Uuid) -> std::path::PathBuf {
    upload_temp_dir(state).join(format!("{iso_id}.complete.part"))
}

async fn cleanup_dir(path: &std::path::Path) {
    if let Err(error) = tokio::fs::remove_dir_all(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), "清理 ISO 分片目录失败");
    }
}

/// 解析 ISO 的上传计划（chunk_size 字节 / total_chunks）。
///
/// 老记录（v3 单次 PUT / seed preset）没有 plan 列：按当前配置默认值与 size 推导。
struct ResolvedPlan {
    chunk_size: u64,
    chunk_size_mib: u32,
    total_chunks: i32,
}

fn resolve_plan(state: &AppState, iso: &InstallIso) -> ResolvedPlan {
    let default_mib = state.config.iso_upload_chunk_mib.max(1);
    let chunk_size = iso
        .upload_chunk_size
        .filter(|value| *value > 0)
        .map(|value| value as u64)
        .unwrap_or_else(|| u64::from(default_mib) * MIB);
    let chunk_size_mib = u32::try_from(chunk_size / MIB)
        .ok()
        .filter(|mib| *mib > 0)
        .unwrap_or(default_mib);
    let total_chunks = iso
        .upload_total_chunks
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            i32::try_from(chunk_count(iso.size_bytes.max(0) as u64, chunk_size)).unwrap_or(i32::MAX)
        });
    ResolvedPlan {
        chunk_size,
        chunk_size_mib,
        total_chunks,
    }
}

/// 组装上传状态（含已收分片）。
fn build_upload_status(
    iso: &InstallIso,
    plan: &ResolvedPlan,
    chunks: &[IsoChunk],
) -> UploadStatusReply {
    let received: Vec<i32> = chunks.iter().map(|chunk| chunk.chunk_index).collect();
    let received_bytes: i64 = chunks.iter().map(|chunk| chunk.size_bytes).sum();
    UploadStatusReply {
        status: iso.status,
        size_bytes: iso.size_bytes,
        chunk_size: i64::try_from(plan.chunk_size).unwrap_or(i64::MAX),
        chunk_size_mib: plan.chunk_size_mib,
        total_chunks: plan.total_chunks,
        received,
        received_bytes,
        sha256: iso.sha256.clone(),
    }
}

async fn record_iso_audit(
    state: &AppState,
    actor: Uuid,
    ion: &InstallIso,
    action: &str,
    ip: Option<&str>,
) {
    super::audit::record_or_log(
        state,
        actor,
        action,
        "install_iso",
        Some(ion.id.to_string()),
        serde_json::json!({
            "filename": ion.filename,
            "size_bytes": ion.size_bytes,
            "sha256": ion.sha256,
            "is_preset": ion.is_preset,
        }),
        ip,
    )
    .await;
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/admin/isos` —— ISO 库列表（preset 所有人可见，非 preset 仅 uploader/admin）。
pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<IsoListQuery>,
) -> AppResult<Json<Vec<IsoReply>>> {
    authorization::require_permission(&state, id, permissions::IMAGE_READ).await?;
    let status = parse_status_filter(query.status.as_deref())?;
    let is_admin = actor_is_admin(&state, id).await?;
    let items = state.isos.list(id, is_admin, status).await?;
    Ok(Json(items.into_iter().map(IsoReply::from).collect()))
}

/// `POST /api/v1/admin/isos` —— 登记 ISO 上传（分片计划；同名 uploading 幂等续传）。
pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateIsoRequest>,
) -> AppResult<(StatusCode, Json<CreateIsoReply>)> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;

    let filename = validate_iso_filename(&request.filename).map_err(AppError::bad_request)?;
    let size_bytes = validate_upload_size(request.size_bytes).map_err(AppError::bad_request)?;
    let chunk_size_mib =
        resolve_chunk_size_mib(request.chunk_size_mib, state.config.iso_upload_chunk_mib)
            .map_err(AppError::bad_request)?;
    let (chunk_size_bytes, total_chunks) = upload_plan(size_bytes, chunk_size_mib);
    let total_chunks_i32 = i32::try_from(total_chunks).unwrap_or(i32::MAX);

    let display_name = match normalize_optional_text(request.display_name, 128, "display_name")? {
        Some(display_name) => display_name,
        None => iso_seed::display_name_for(&filename),
    };
    let description =
        normalize_optional_text(request.description, MAX_DESCRIPTION_CHARS, "description")?;
    let sha256 = match request
        .sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(sha256) => Some(
            validate_sha256_hex(&sha256.trim().to_ascii_lowercase())
                .map_err(|error| AppError::bad_request(format!("sha256 格式非法: {error}")))?,
        ),
        None => None,
    };

    // 同名（大小写不敏感）处理（契约 C.1）：
    // - 同 owner + 同 size + uploading → 幂等返回原记录（续传）；
    // - ready → 409；其它（他人/preset/大小不同/failed）→ 409。
    if let Some(existing) = state.isos.find_by_filename(&filename).await? {
        let same_owner = existing.uploader_user_id == Some(id);
        let upload_url = format!("/api/v1/admin/isos/{}/content", existing.id);
        if !existing.is_preset && same_owner && existing.status == IsoStatus::Uploading {
            if existing.size_bytes != size_bytes as i64 {
                return Err(AppError::conflict(format!(
                    "同名 ISO 正在上传中，但声明大小不同（已有 {} 字节，本次 {} 字节）；如需重传请先 DELETE /admin/isos/{}/upload",
                    existing.size_bytes, size_bytes, existing.id
                )));
            }
            let plan = resolve_plan(&state, &existing);
            let chunks = state.isos.list_chunks(existing.id).await?;
            let received: Vec<i32> = chunks.iter().map(|chunk| chunk.chunk_index).collect();
            return Ok((
                StatusCode::OK,
                Json(CreateIsoReply {
                    id: existing.id,
                    upload_url,
                    upload: CreateUploadInfo {
                        chunk_size: i64::try_from(plan.chunk_size).unwrap_or(i64::MAX),
                        chunk_size_mib: plan.chunk_size_mib,
                        total_chunks: plan.total_chunks,
                        received,
                    },
                }),
            ));
        }
        return Err(AppError::conflict(if existing.status == IsoStatus::Ready {
            format!(
                "同名 ISO 已存在且已就绪（{}）；可直接使用，无需重复上传",
                existing.filename
            )
        } else {
            format!(
                "同名 ISO 已存在（{}，status={}）；请先 DELETE /admin/isos/{}/upload 再重试",
                existing.filename, existing.status, existing.id
            )
        }));
    }

    let iso = state
        .isos
        .create(&NewInstallIso {
            id: Uuid::new_v4(),
            filename,
            display_name,
            description,
            sha256,
            is_preset: false,
            uploader_user_id: Some(id),
            size_bytes: i64::try_from(size_bytes).unwrap_or(i64::MAX),
            upload_chunk_size: Some(i64::try_from(chunk_size_bytes).unwrap_or(i64::MAX)),
            upload_total_chunks: Some(total_chunks_i32),
        })
        .await?;
    record_iso_audit(&state, id, &iso, "iso.create", ip.as_deref()).await;

    Ok((
        StatusCode::CREATED,
        Json(CreateIsoReply {
            id: iso.id,
            upload_url: format!("/api/v1/admin/isos/{}/content", iso.id),
            upload: CreateUploadInfo {
                chunk_size: i64::try_from(chunk_size_bytes).unwrap_or(i64::MAX),
                chunk_size_mib,
                total_chunks: total_chunks_i32,
                received: Vec::new(),
            },
        }),
    ))
}

/// 读取 ISO 并校验 owner/admin；preset 不允许上传/中止（契约 C.1）。
async fn load_uploadable_iso(state: &AppState, actor: Uuid, iso_id: Uuid) -> AppResult<InstallIso> {
    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    let is_admin = actor_is_admin(state, actor).await?;
    if !qe_domain::install_iso::can_manage_iso(&iso, actor, is_admin) {
        return Err(AppError::forbidden("无权操作他人的 ISO"));
    }
    if iso.is_preset {
        return Err(AppError::conflict("系统预设 ISO 不允许修改上传状态"));
    }
    Ok(iso)
}

/// `GET /api/v1/admin/isos/{id}/upload` —— 分片上传状态（owner/admin）。
pub async fn get_upload(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(iso_id): Path<Uuid>,
) -> AppResult<Json<UploadStatusReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = load_uploadable_iso(&state, id, iso_id).await?;
    let plan = resolve_plan(&state, &iso);
    let chunks = state.isos.list_chunks(iso_id).await?;
    Ok(Json(build_upload_status(&iso, &plan, &chunks)))
}

/// 从请求头/query 提取可选的期望分片 SHA-256。
fn expected_chunk_sha(headers: &HeaderMap, query_sha: Option<&str>) -> AppResult<Option<String>> {
    let header_sha = headers
        .get("x-chunk-sha256")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let raw = header_sha.or(query_sha.map(str::trim).filter(|value| !value.is_empty()));
    match raw {
        None => Ok(None),
        Some(value) => validate_sha256_hex(&value.to_ascii_lowercase())
            .map(Some)
            .map_err(|error| AppError::bad_request(format!("分片 sha256 格式非法: {error}"))),
    }
}

/// `POST /api/v1/admin/isos/{id}/complete` 查询参数（可替代 body 的 sha256）。
#[derive(Debug, Deserialize)]
pub struct ChunkQuery {
    #[serde(default)]
    pub sha256: Option<String>,
}

/// `PUT /api/v1/admin/isos/{id}/chunks/{index}` —— 上传单个分片（原始 body）。
///
/// 允许 `uploading` / `failed`（失败保留分片可续传）；重复上传同 index 幂等覆盖。
pub async fn upload_chunk(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path((iso_id, index)): Path<(Uuid, i32)>,
    Query(query): Query<ChunkQuery>,
    headers: HeaderMap,
    body: Body,
) -> AppResult<Json<ChunkUploadReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = load_uploadable_iso(&state, id, iso_id).await?;
    if !matches!(iso.status, IsoStatus::Uploading | IsoStatus::Failed) {
        return Err(AppError::conflict(format!(
            "仅 uploading/failed 可上传分片（当前 {}）",
            iso.status
        )));
    }
    let plan = resolve_plan(&state, &iso);
    let expected_len = expected_chunk_len(
        index,
        plan.total_chunks,
        iso.size_bytes.max(0) as u64,
        plan.chunk_size,
    )
    .map_err(AppError::bad_request)?;
    let expected_sha = expected_chunk_sha(&headers, query.sha256.as_deref())?;
    if expected_len > MAX_ISO_BYTES {
        return Err(AppError::payload_too_large("分片大小超过上限"));
    }

    let dir = chunk_dir(&state, iso_id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| AppError::internal(format!("创建分片目录失败: {error}")))?;
    let target = chunk_path(&state, iso_id, index);
    let temp = dir.join(format!("{index}.part.tmp"));
    cleanup_temp(&temp).await;

    let mut file = tokio::fs::File::create(&temp)
        .await
        .map_err(|error| AppError::internal(format!("创建分片临时文件失败: {error}")))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                cleanup_temp(&temp).await;
                return Err(AppError::bad_request(format!("读取分片数据失败: {error}")));
            }
        };
        received = received.saturating_add(chunk.len() as u64);
        if received > expected_len {
            cleanup_temp(&temp).await;
            return Err(AppError::bad_request(format!(
                "分片 {index} 超过预期长度 {expected_len} 字节"
            )));
        }
        hasher.update(&chunk);
        if let Err(error) = file.write_all(&chunk).await {
            cleanup_temp(&temp).await;
            return Err(AppError::internal(format!("写入分片文件失败: {error}")));
        }
    }
    if let Err(error) = file.flush().await {
        cleanup_temp(&temp).await;
        return Err(AppError::internal(format!("刷写分片文件失败: {error}")));
    }
    drop(file);

    if received != expected_len {
        cleanup_temp(&temp).await;
        return Err(AppError::bad_request(format!(
            "分片 {index} 长度不符：期望 {expected_len} 字节，实际 {received} 字节"
        )));
    }
    let sha256 = hex::encode(hasher.finalize());
    if let Some(expected) = expected_sha.as_deref()
        && !expected.eq_ignore_ascii_case(&sha256)
    {
        cleanup_temp(&temp).await;
        return Err(AppError::bad_request(format!(
            "分片 {index} SHA-256 与请求声明值不一致"
        )));
    }
    tokio::fs::rename(&temp, &target)
        .await
        .map_err(|error| AppError::internal(format!("落盘分片文件失败: {error}")))?;
    state
        .isos
        .upsert_chunk(
            iso_id,
            index,
            i64::try_from(received).unwrap_or(i64::MAX),
            &sha256,
        )
        .await?;
    let (received_count, received_bytes) = state.isos.received_summary(iso_id).await?;
    Ok(Json(ChunkUploadReply {
        received_count,
        received_bytes,
    }))
}

/// `POST /api/v1/admin/isos/{id}/complete` —— 合并分片、整体校验并上传 S3。
///
/// 缺片 → 409 + `missing` 索引；失败置 failed 但保留分片供续传。
pub async fn complete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
    ApiJson(request): ApiJson<CompleteIsoRequest>,
) -> Result<Json<IsoReply>, CompleteIsoError> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = load_uploadable_iso(&state, id, iso_id).await?;
    if !matches!(iso.status, IsoStatus::Uploading | IsoStatus::Failed) {
        return Err(AppError::conflict(format!(
            "仅 uploading/failed 可完成上传（当前 {}）",
            iso.status
        ))
        .into());
    }
    let body_sha = match request
        .sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => Some(
            validate_sha256_hex(&raw.to_ascii_lowercase())
                .map_err(|error| AppError::bad_request(format!("sha256 格式非法: {error}")))?,
        ),
        None => None,
    };
    let plan = resolve_plan(&state, &iso);
    let chunks = state.isos.list_chunks(iso_id).await?;
    let received: Vec<i32> = chunks.iter().map(|chunk| chunk.chunk_index).collect();
    let missing = missing_chunk_indices(plan.total_chunks, &received);
    if !missing.is_empty() {
        return Err(CompleteIsoError::Missing(missing));
    }
    let size_sum: i64 = chunks.iter().map(|chunk| chunk.size_bytes).sum();
    if size_sum != iso.size_bytes {
        return Err(AppError::conflict(format!(
            "分片大小之和（{size_sum}）与声明 size_bytes（{}）不一致",
            iso.size_bytes
        ))
        .into());
    }

    // 按序流式拼接 + 整体 SHA-256（不一次性读入内存）。
    let final_temp = complete_temp_path(&state, iso_id);
    cleanup_temp(&final_temp).await;
    let dir = chunk_dir(&state, iso_id);
    let mut output = tokio::fs::File::create(&final_temp)
        .await
        .map_err(|error| {
            CompleteIsoError::App(AppError::internal(format!("创建合并临时文件失败: {error}")))
        })?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut buffer = vec![0u8; 1024 * 1024];
    for chunk in &chunks {
        let part = dir.join(format!("{}.part", chunk.chunk_index));
        let mut input = match tokio::fs::File::open(&part).await {
            Ok(input) => input,
            Err(error) => {
                cleanup_temp(&final_temp).await;
                let _ = state.isos.update_failed(iso_id).await;
                return Err(AppError::internal(format!(
                    "读取分片文件失败（{}）: {error}",
                    part.display()
                ))
                .into());
            }
        };
        loop {
            let read = match input.read(&mut buffer).await {
                Ok(read) => read,
                Err(error) => {
                    cleanup_temp(&final_temp).await;
                    let _ = state.isos.update_failed(iso_id).await;
                    return Err(AppError::internal(format!("读取分片数据失败: {error}")).into());
                }
            };
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > MAX_ISO_BYTES {
                cleanup_temp(&final_temp).await;
                let _ = state.isos.update_failed(iso_id).await;
                return Err(AppError::payload_too_large("合并后大小超过上限").into());
            }
            hasher.update(&buffer[..read]);
            if let Err(error) = output.write_all(&buffer[..read]).await {
                cleanup_temp(&final_temp).await;
                let _ = state.isos.update_failed(iso_id).await;
                return Err(AppError::internal(format!("写入合并文件失败: {error}")).into());
            }
        }
    }
    if let Err(error) = output.flush().await {
        cleanup_temp(&final_temp).await;
        let _ = state.isos.update_failed(iso_id).await;
        return Err(AppError::internal(format!("刷写合并文件失败: {error}")).into());
    }
    drop(output);

    let sha256 = hex::encode(hasher.finalize());
    let expected_sha = body_sha.or_else(|| iso.sha256.clone());
    if let Some(expected) = expected_sha.as_deref()
        && !expected.eq_ignore_ascii_case(&sha256)
    {
        cleanup_temp(&final_temp).await;
        let _ = state.isos.update_failed(iso_id).await;
        return Err(
            AppError::bad_request("ISO 整体 SHA-256 与声明值不一致，请检查/重传分片").into(),
        );
    }

    let storage = super::storage::service(&state)?.clone();
    let storage_key = iso_storage_key(iso_id, &iso.filename);
    if let Err(error) = storage
        .put_file(
            &storage_key,
            &final_temp,
            "application/octet-stream",
            ISO_UPLOAD_S3_TIMEOUT,
        )
        .await
    {
        cleanup_temp(&final_temp).await;
        let _ = state.isos.update_failed(iso_id).await;
        return Err(AppError::bad_gateway(format!("上传 ISO 到对象存储失败: {error}")).into());
    }
    cleanup_temp(&final_temp).await;

    let updated = state
        .isos
        .update_ready(
            iso_id,
            i64::try_from(total).unwrap_or(i64::MAX),
            &sha256,
            &storage_key,
        )
        .await?
        .ok_or_else(|| {
            CompleteIsoError::App(AppError::conflict(
                "ISO 状态已变化（可能被并发完成/删除），请刷新",
            ))
        })?;
    // ready 后清理分片（记录里保留 upload_chunk/total 便于展示）。
    state.isos.delete_chunks(iso_id).await?;
    cleanup_dir(&dir).await;
    record_iso_audit(&state, id, &updated, "iso.upload_ready", ip.as_deref()).await;
    Ok(Json(IsoReply::from(updated)))
}

/// `DELETE /api/v1/admin/isos/{id}/upload` —— 中止分片上传并释放文件名。
///
/// 契约 C.1 允许「记录 failed 或删除记录」；这里直接删除记录（chunk 行 FK 级联）
/// 并清理分片目录，让同名可以立即重新上传。
pub async fn abort_upload(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = load_uploadable_iso(&state, id, iso_id).await?;
    if !matches!(iso.status, IsoStatus::Uploading | IsoStatus::Failed) {
        return Err(AppError::conflict(format!(
            "仅 uploading/failed 可中止上传（当前 {}）；已就绪请用 DELETE /admin/isos/{iso_id}",
            iso.status
        )));
    }
    state.isos.delete_chunks(iso_id).await?;
    cleanup_dir(&chunk_dir(&state, iso_id)).await;
    cleanup_temp(&complete_temp_path(&state, iso_id)).await;
    state.isos.delete(iso_id).await?;
    record_iso_audit(&state, id, &iso, "iso.upload_abort", ip.as_deref()).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/v1/admin/isos/{id}/content` —— 原始 body 流式上传（不经过 ApiJson）。
///
/// 边收边写临时文件并计算 SHA-256：4.7GB ISO 也不会一次性读入内存。
/// 完成后 `put_file` 流式上传 S3，置 ready 返回记录；任一步失败置 failed 并清理临时文件。
pub async fn upload_content(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
    body: Body,
) -> AppResult<Json<IsoReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;

    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    let is_admin = actor_is_admin(&state, id).await?;
    if !qe_domain::install_iso::can_manage_iso(&iso, id, is_admin) {
        return Err(AppError::forbidden("无权上传他人的 ISO"));
    }
    if iso.status != IsoStatus::Uploading {
        return Err(AppError::conflict(format!(
            "仅 uploading 状态可上传内容（当前 {}）",
            iso.status
        )));
    }
    let storage = super::storage::service(&state)?.clone();

    let temp_dir = upload_temp_dir(&state);
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|error| AppError::internal(format!("创建 ISO 上传目录失败: {error}")))?;
    let temp_path = upload_temp_path(&state, iso_id);
    cleanup_temp(&temp_path).await;

    let mut file = tokio::fs::File::create(&temp_path)
        .await
        .map_err(|error| AppError::internal(format!("创建 ISO 临时文件失败: {error}")))?;
    let expected_sha = iso.sha256.clone();
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                fail_upload(&state, iso_id, &temp_path).await;
                return Err(AppError::bad_request(format!(
                    "读取 ISO 上传数据失败: {error}"
                )));
            }
        };
        size += chunk.len() as u64;
        if size > MAX_ISO_BYTES {
            fail_upload(&state, iso_id, &temp_path).await;
            return Err(AppError::payload_too_large(format!(
                "ISO 超过 {} GiB 上限",
                MAX_ISO_BYTES / (1024 * 1024 * 1024)
            )));
        }
        hasher.update(&chunk);
        if let Err(error) = file.write_all(&chunk).await {
            fail_upload(&state, iso_id, &temp_path).await;
            return Err(AppError::internal(format!(
                "写入 ISO 临时文件失败: {error}"
            )));
        }
    }
    if let Err(error) = file.flush().await {
        fail_upload(&state, iso_id, &temp_path).await;
        return Err(AppError::internal(format!(
            "刷写 ISO 临时文件失败: {error}"
        )));
    }
    drop(file);

    if size == 0 {
        fail_upload(&state, iso_id, &temp_path).await;
        return Err(AppError::bad_request("ISO 内容为空"));
    }
    if iso.size_bytes > 0 && size != iso.size_bytes as u64 {
        fail_upload(&state, iso_id, &temp_path).await;
        return Err(AppError::bad_request(format!(
            "ISO 大小与 POST 声明不一致（声明 {} 字节，实际 {size} 字节）",
            iso.size_bytes
        )));
    }
    let sha256 = hex::encode(hasher.finalize());
    if let Some(expected) = expected_sha.as_deref()
        && !expected.eq_ignore_ascii_case(&sha256)
    {
        fail_upload(&state, iso_id, &temp_path).await;
        return Err(AppError::bad_request(
            "ISO 内容 SHA-256 与预声明值不一致，请重新上传",
        ));
    }

    let storage_key = iso_storage_key(iso_id, &iso.filename);
    if let Err(error) = storage
        .put_file(
            &storage_key,
            &temp_path,
            "application/octet-stream",
            ISO_UPLOAD_S3_TIMEOUT,
        )
        .await
    {
        fail_upload(&state, iso_id, &temp_path).await;
        return Err(AppError::bad_gateway(format!(
            "上传 ISO 到对象存储失败: {error}"
        )));
    }
    cleanup_temp(&temp_path).await;

    let updated = state
        .isos
        .update_ready(iso_id, size as i64, &sha256, &storage_key)
        .await?
        .ok_or_else(|| AppError::conflict("ISO 状态已变化（可能被并发上传/删除），请刷新"))?;
    // 单次 PUT 完成后不需要保留分片（可能来自之前中断的分片上传）。
    state.isos.delete_chunks(iso_id).await?;
    cleanup_dir(&chunk_dir(&state, iso_id)).await;
    record_iso_audit(&state, id, &updated, "iso.upload_ready", ip.as_deref()).await;
    Ok(Json(IsoReply::from(updated)))
}

/// `DELETE /api/v1/admin/isos/{id}` —— 删除记录 + S3 对象（使用中 409）。
pub async fn remove(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    let is_admin = actor_is_admin(&state, id).await?;
    if !qe_domain::install_iso::can_manage_iso(&iso, id, is_admin) {
        return Err(AppError::forbidden("无权删除他人的 ISO"));
    }
    if iso.is_preset {
        return Err(AppError::conflict("系统预设 ISO 不允许删除"));
    }
    if state.isos.count_active_sessions(iso_id).await? > 0 {
        return Err(AppError::conflict(ISO_IN_USE_MESSAGE));
    }

    if let Some(storage_key) = iso.storage_key.as_deref() {
        let storage = super::storage::service(&state)?;
        if let Err(error) = storage.delete_object(storage_key).await {
            tracing::warn!(%error, %iso_id, "删除 ISO 的 S3 对象失败（继续删除记录）");
        }
    }
    state.isos.delete(iso_id).await?;
    // 清理本地上传中间态（分片目录 / 单次 PUT 临时文件）。
    state.isos.delete_chunks(iso_id).await.ok();
    cleanup_dir(&chunk_dir(&state, iso_id)).await;
    cleanup_temp(&upload_temp_path(&state, iso_id)).await;
    cleanup_temp(&complete_temp_path(&state, iso_id)).await;
    record_iso_audit(&state, id, &iso, "iso.delete", ip.as_deref()).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/admin/isos/seed/scan` —— 管理员触发 seed 目录扫描。
pub async fn seed_scan(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
) -> AppResult<Json<iso_seed::SeedScanReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    if !actor_is_admin(&state, id).await? {
        return Err(AppError::forbidden("仅管理员可触发 ISO seed 扫描"));
    }
    let reply = iso_seed::scan_seed_dir(&state, Some(id))
        .await
        .map_err(|error| AppError::internal(format!("ISO seed 扫描失败: {error:#}")))?;
    super::audit::record_or_log(
        &state,
        id,
        "iso.seed_scan",
        "install_iso",
        None,
        serde_json::json!({
            "scanned": reply.scanned,
            "imported": reply.imported,
            "skipped": reply.skipped,
            "failed": reply.failed,
        }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(reply))
}

// ---------------------------------------------------------------------------
// 共享与审批（附录 C.2）
// ---------------------------------------------------------------------------

/// `GET /admin/isos/share-requests?status=pending` 查询参数。
#[derive(Debug, Deserialize)]
pub struct ShareRequestQuery {
    /// pending / approved / rejected / none / all（缺省 pending）。
    #[serde(default)]
    pub status: Option<String>,
}

fn parse_share_status_filter(raw: Option<&str>) -> AppResult<Option<ShareStatus>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(Some(ShareStatus::Pending)),
        Some("all") => Ok(None),
        Some(value) => value.parse::<ShareStatus>().map(Some).map_err(|_| {
            AppError::bad_request("status 只能是 pending / approved / rejected / none / all")
        }),
    }
}

/// `POST /api/v1/admin/isos/{id}/share-request` —— owner/admin 申请共享（pending 幂等）。
pub async fn share_request(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
) -> AppResult<Json<IsoReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    let is_admin = actor_is_admin(&state, id).await?;
    if !qe_domain::install_iso::can_manage_iso(&iso, id, is_admin) {
        return Err(AppError::forbidden("无权申请共享他人的 ISO"));
    }
    if iso.is_preset {
        return Err(AppError::conflict(
            "系统预设 ISO 无需申请共享（所有人可见）",
        ));
    }
    if !can_request_share(&iso) {
        return Err(AppError::conflict("仅 ready 状态的 ISO 可申请共享"));
    }
    // pending / approved：幂等返回当前状态。
    if matches!(
        iso.share_status,
        ShareStatus::Pending | ShareStatus::Approved
    ) {
        return Ok(Json(IsoReply::from(iso)));
    }
    let updated = state
        .isos
        .request_share(iso_id, id)
        .await?
        .ok_or_else(|| AppError::conflict("当前共享状态不允许申请（请刷新后重试）"))?;
    record_iso_audit(&state, id, &updated, "iso.share_request", ip.as_deref()).await;
    Ok(Json(IsoReply::from(updated)))
}

/// `POST /api/v1/admin/isos/{id}/share-cancel` —— owner/admin 取消 pending 申请。
pub async fn share_cancel(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
) -> AppResult<Json<IsoReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    let is_admin = actor_is_admin(&state, id).await?;
    if !qe_domain::install_iso::can_manage_iso(&iso, id, is_admin) {
        return Err(AppError::forbidden("无权操作他人的 ISO 共享状态"));
    }
    if iso.is_preset {
        return Err(AppError::conflict("系统预设 ISO 没有共享申请"));
    }
    let updated = state
        .isos
        .cancel_share(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    record_iso_audit(&state, id, &updated, "iso.share_cancel", ip.as_deref()).await;
    Ok(Json(IsoReply::from(updated)))
}

/// `GET /api/v1/admin/isos/share-requests?status=pending` —— 管理员审批列表。
pub async fn share_requests(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<ShareRequestQuery>,
) -> AppResult<Json<Vec<IsoReply>>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    if !actor_is_admin(&state, id).await? {
        return Err(AppError::forbidden("仅管理员可查看 ISO 共享审批列表"));
    }
    let status = parse_share_status_filter(query.status.as_deref())?;
    let items = state.isos.list_share_requests(status).await?;
    Ok(Json(items.into_iter().map(IsoReply::from).collect()))
}

/// `POST /api/v1/admin/isos/{id}/share-review` —— 管理员审批共享申请。
pub async fn share_review(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
    ApiJson(request): ApiJson<ShareReviewRequest>,
) -> AppResult<Json<IsoReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    if !actor_is_admin(&state, id).await? {
        return Err(AppError::forbidden("仅管理员可审批 ISO 共享申请"));
    }
    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    if iso.is_preset {
        return Err(AppError::conflict("系统预设 ISO 无需审批"));
    }
    let note = normalize_optional_text(request.note, 500, "note")?;
    let updated = state
        .isos
        .review_share(iso_id, request.approve, id, note.as_deref())
        .await?
        .ok_or_else(|| AppError::conflict("仅 pending 状态的共享申请可审批"))?;
    record_iso_audit(
        &state,
        id,
        &updated,
        if request.approve {
            "iso.share_approve"
        } else {
            "iso.share_reject"
        },
        ip.as_deref(),
    )
    .await;
    Ok(Json(IsoReply::from(updated)))
}

/// `POST /api/v1/admin/isos/{id}/unshare` —— owner/admin 撤销共享。
pub async fn unshare(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(iso_id): Path<Uuid>,
) -> AppResult<Json<IsoReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    let is_admin = actor_is_admin(&state, id).await?;
    if !qe_domain::install_iso::can_manage_iso(&iso, id, is_admin) {
        return Err(AppError::forbidden("无权撤销他人的 ISO 共享"));
    }
    if iso.is_preset {
        return Err(AppError::conflict("系统预设 ISO 无需撤销共享"));
    }
    let updated = state
        .isos
        .unshare(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 不存在"))?;
    record_iso_audit(&state, id, &updated, "iso.unshare", ip.as_deref()).await;
    Ok(Json(IsoReply::from(updated)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_filter_parsing() {
        assert_eq!(parse_status_filter(None).unwrap(), None);
        assert_eq!(parse_status_filter(Some(" ")).unwrap(), None);
        assert_eq!(
            parse_status_filter(Some("ready")).unwrap(),
            Some(IsoStatus::Ready)
        );
        assert!(parse_status_filter(Some("bogus")).is_err());
    }

    #[test]
    fn create_request_parses_and_rejects_unknown_fields() {
        let request: CreateIsoRequest = serde_json::from_value(serde_json::json!({
            "filename": "Win10_LTSC_2021.iso",
            "size_bytes": 5_368_709_120_u64,
            "sha256": "a".repeat(64),
        }))
        .expect("request");
        assert_eq!(request.filename, "Win10_LTSC_2021.iso");
        assert_eq!(request.size_bytes, 5_368_709_120);
        assert!(request.chunk_size_mib.is_none());
        assert!(
            serde_json::from_value::<CreateIsoRequest>(serde_json::json!({
                "filename": "a.iso",
                "extra": 1,
            }))
            .is_err()
        );
        // size_bytes 必填（契约 C.1）。
        assert!(
            serde_json::from_value::<CreateIsoRequest>(serde_json::json!({
                "filename": "a.iso",
            }))
            .is_err()
        );
    }

    fn sample_iso() -> InstallIso {
        let now = Utc::now();
        InstallIso {
            id: Uuid::new_v4(),
            filename: "Win10_LTSC_2021.iso".to_owned(),
            display_name: "Win10".to_owned(),
            description: None,
            size_bytes: 5_368_709_120,
            sha256: Some("a".repeat(64)),
            status: IsoStatus::Ready,
            storage_key: Some("isos/x/Win10.iso".to_owned()),
            is_preset: true,
            visibility: IsoVisibility::Private,
            share_status: ShareStatus::None,
            share_requested_by: None,
            share_reviewed_by: None,
            share_reviewed_at: None,
            share_note: None,
            uploader_user_id: None,
            uploader_name: None,
            upload_chunk_size: Some(64 * 1024 * 1024),
            upload_total_chunks: Some(80),
            upload_updated_at: Some(now),
            created_at: now,
            updated_at: now,
            uploaded_at: Some(now),
        }
    }

    #[test]
    fn iso_reply_never_exposes_storage_key() {
        let iso = sample_iso();
        let value = serde_json::to_value(IsoReply::from(iso)).expect("serialize");
        assert!(value.get("storage_key").is_none());
        for key in [
            "id",
            "filename",
            "display_name",
            "size_bytes",
            "sha256",
            "status",
            "is_preset",
            "visibility",
            "share_status",
            "is_shared",
            "share_note",
            "uploader_name",
            "created_at",
            "updated_at",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["status"], serde_json::json!("ready"));
        assert_eq!(value["visibility"], serde_json::json!("private"));
        assert_eq!(value["share_status"], serde_json::json!("none"));
        assert_eq!(value["is_shared"], serde_json::json!(false));
    }

    #[test]
    fn shared_reply_marks_is_shared() {
        let mut iso = sample_iso();
        iso.is_preset = false;
        iso.visibility = IsoVisibility::Shared;
        iso.share_status = ShareStatus::Approved;
        let value = serde_json::to_value(IsoReply::from(iso)).expect("serialize");
        assert_eq!(value["visibility"], serde_json::json!("shared"));
        assert_eq!(value["share_status"], serde_json::json!("approved"));
        assert_eq!(value["is_shared"], serde_json::json!(true));
    }

    #[test]
    fn share_status_filter_parsing() {
        assert_eq!(
            parse_share_status_filter(None).unwrap(),
            Some(ShareStatus::Pending)
        );
        assert_eq!(
            parse_share_status_filter(Some("approved")).unwrap(),
            Some(ShareStatus::Approved)
        );
        assert_eq!(parse_share_status_filter(Some("all")).unwrap(), None);
        assert!(parse_share_status_filter(Some("bogus")).is_err());
    }

    #[test]
    fn chunk_sha_header_and_query_are_validated() {
        let sha = "a".repeat(64);
        let mut headers = HeaderMap::new();
        assert_eq!(expected_chunk_sha(&headers, None).unwrap(), None);
        assert_eq!(
            expected_chunk_sha(&headers, Some(sha.as_str())).unwrap(),
            Some(sha.clone())
        );
        headers.insert("x-chunk-sha256", sha.parse().unwrap());
        assert_eq!(
            expected_chunk_sha(&headers, None).unwrap(),
            Some(sha.clone())
        );
        // 大小写自动归一。
        headers.insert("x-chunk-sha256", "A".repeat(64).parse().unwrap());
        assert_eq!(
            expected_chunk_sha(&headers, None).unwrap(),
            Some(sha.clone())
        );
        headers.insert("x-chunk-sha256", "not-hex".parse().unwrap());
        assert!(expected_chunk_sha(&headers, None).is_err());
    }

    #[test]
    fn upload_status_reflects_received_chunks() {
        let iso = sample_iso();
        let plan = ResolvedPlan {
            chunk_size: 64 * 1024 * 1024,
            chunk_size_mib: 64,
            total_chunks: 80,
        };
        let now = Utc::now();
        let chunks = vec![
            IsoChunk {
                iso_id: iso.id,
                chunk_index: 0,
                size_bytes: 64 * 1024 * 1024,
                sha256: "b".repeat(64),
                received_at: now,
            },
            IsoChunk {
                iso_id: iso.id,
                chunk_index: 3,
                size_bytes: 64 * 1024 * 1024,
                sha256: "c".repeat(64),
                received_at: now,
            },
        ];
        let status = build_upload_status(&iso, &plan, &chunks);
        assert_eq!(status.status, IsoStatus::Ready);
        assert_eq!(status.received, vec![0, 3]);
        assert_eq!(status.received_bytes, 128 * 1024 * 1024);
        assert_eq!(status.chunk_size_mib, 64);
        assert_eq!(status.total_chunks, 80);
        assert_eq!(status.sha256.as_deref(), Some("a".repeat(64).as_str()));
        let value = serde_json::to_value(&status).expect("serialize");
        for key in [
            "status",
            "size_bytes",
            "chunk_size",
            "chunk_size_mib",
            "total_chunks",
            "received",
            "received_bytes",
            "sha256",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn complete_request_and_share_review_parse() {
        let complete: CompleteIsoRequest =
            serde_json::from_value(serde_json::json!({})).expect("complete");
        assert!(complete.sha256.is_none());
        let review: ShareReviewRequest = serde_json::from_value(serde_json::json!({
            "approve": false,
            "note": "请补充说明",
        }))
        .expect("review");
        assert!(!review.approve);
        assert_eq!(review.note.as_deref(), Some("请补充说明"));
        assert!(
            serde_json::from_value::<ShareReviewRequest>(serde_json::json!({
                "approve": true, "extra": 1
            }))
            .is_err()
        );
    }
}
