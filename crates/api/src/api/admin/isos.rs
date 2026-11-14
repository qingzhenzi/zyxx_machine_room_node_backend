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
//! 注意：`PUT content` 不挂全局 2 MiB body 限制 / 30s 超时，由 `api/mod.rs` 的独立
//! 长上传路由承载（16 GiB、2 小时）。

use std::time::Duration;

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use qe_domain::{
    install_iso::{
        InstallIso, IsoStatus, MAX_ISO_BYTES, NewInstallIso, iso_storage_key, validate_iso_filename,
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
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// 可选：调用方预声明 sha256（PUT 完成时必须匹配）。
    #[serde(default)]
    pub sha256: Option<String>,
}

/// `POST /admin/isos` 响应：上传地址（相对路径，前端拼 control host）。
#[derive(Debug, Serialize)]
pub struct CreateIsoReply {
    pub id: Uuid,
    pub upload_url: String,
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
    pub uploader_user_id: Option<Uuid>,
    pub uploader_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub uploaded_at: Option<DateTime<Utc>>,
}

impl From<InstallIso> for IsoReply {
    fn from(iso: InstallIso) -> Self {
        Self {
            id: iso.id,
            filename: iso.filename,
            display_name: iso.display_name,
            description: iso.description,
            size_bytes: iso.size_bytes,
            sha256: iso.sha256,
            status: iso.status,
            is_preset: iso.is_preset,
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

/// `POST /api/v1/admin/isos` —— 登记 ISO 上传（先建 `uploading` 记录）。
pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateIsoRequest>,
) -> AppResult<(StatusCode, Json<CreateIsoReply>)> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;

    let filename = validate_iso_filename(&request.filename).map_err(AppError::bad_request)?;
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

    // 文件名大小写不敏感唯一：同名（无论状态）直接 409。
    if let Some(existing) = state.isos.find_by_filename(&filename).await? {
        return Err(AppError::conflict(format!(
            "同名 ISO 已存在（{}，status={}）",
            existing.filename, existing.status
        )));
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
        })
        .await?;
    record_iso_audit(&state, id, &iso, "iso.create", ip.as_deref()).await;

    Ok((
        StatusCode::CREATED,
        Json(CreateIsoReply {
            id: iso.id,
            upload_url: format!("/api/v1/admin/isos/{}/content", iso.id),
        }),
    ))
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
            "sha256": "a".repeat(64),
        }))
        .expect("request");
        assert_eq!(request.filename, "Win10_LTSC_2021.iso");
        assert!(request.display_name.is_none());
        assert!(
            serde_json::from_value::<CreateIsoRequest>(serde_json::json!({
                "filename": "a.iso",
                "extra": 1,
            }))
            .is_err()
        );
    }

    #[test]
    fn iso_reply_never_exposes_storage_key() {
        let iso = InstallIso {
            id: Uuid::new_v4(),
            filename: "Win10_LTSC_2021.iso".to_owned(),
            display_name: "Win10".to_owned(),
            description: None,
            size_bytes: 5_368_709_120,
            sha256: Some("a".repeat(64)),
            status: IsoStatus::Ready,
            storage_key: Some("isos/x/Win10.iso".to_owned()),
            is_preset: true,
            uploader_user_id: None,
            uploader_name: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            uploaded_at: Some(Utc::now()),
        };
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
            "created_at",
            "updated_at",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["status"], serde_json::json!("ready"));
    }
}
