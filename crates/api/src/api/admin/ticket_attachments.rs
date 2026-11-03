//! 工单附件接口（Wave 2 C）：预签名上传、登记、列表（下载预签名）、删除。
//!
//! 契约：附件大小 1..=20MiB；sha256 为 64 位 hex；content-type 允许
//! image/*、application/pdf、text/plain、application/zip、application/gzip、
//! application/octet-stream；对象 key 固定为 `tickets/{ticket_id}/{uuid}/{filename}`。
//! 未配置对象存储时：presign → 503；list 的 `download_url` 为 `null`。

use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use qe_domain::ticket::{TicketAttachment, sanitize_attachment_filename, validate_attachment_meta};
use qe_storage::StorageService;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::rbac::permissions,
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct PresignAttachmentRequest {
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct AttachmentPresignReply {
    pub object_key: String,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterAttachmentRequest {
    pub object_key: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
}

/// 代理上传：浏览器把文件直接 POST 给后端（避免对象存储 CORS 配置依赖）。
#[derive(Debug, Deserialize)]
pub struct UploadAttachmentQuery {
    pub filename: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

fn storage_service(state: &AppState) -> AppResult<&Arc<StorageService>> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| AppError::service_unavailable("未配置对象存储，附件功能不可用"))
}

fn storage_error(action: &str, error: anyhow::Error) -> AppError {
    tracing::error!(%error, action, "工单附件对象存储操作失败");
    AppError::service_unavailable("对象存储暂时不可用")
}

/// 上传/登记/删除都需要 ticket:attachment 或 ticket:handle。
async fn require_attachment_write(state: &AppState, user_id: Uuid) -> AppResult<()> {
    authorization::require_any_permission(
        state,
        user_id,
        &[permissions::TICKET_ATTACHMENT, permissions::TICKET_HANDLE],
    )
    .await?;
    Ok(())
}

/// 上传前校验：工单存在 + 数据范围 + 元数据合法。
async fn ensure_upload_target(
    state: &AppState,
    user_id: Uuid,
    ticket_id: Uuid,
    filename: &str,
    content_type: &str,
    size: i64,
    sha256: &str,
) -> AppResult<String> {
    if state.tickets.get_ticket(ticket_id).await?.is_none() {
        return Err(AppError::not_found("工单不存在"));
    }
    super::tickets::ensure_write_scope(state, user_id, ticket_id).await?;
    validate_attachment_meta(filename, content_type, size, sha256)?;
    Ok(sanitize_attachment_filename(filename)?)
}

pub async fn presign(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(ticket_id): Path<Uuid>,
    ApiJson(request): ApiJson<PresignAttachmentRequest>,
) -> AppResult<Json<AttachmentPresignReply>> {
    require_attachment_write(&state, id).await?;
    let filename = ensure_upload_target(
        &state,
        id,
        ticket_id,
        &request.filename,
        &request.content_type,
        request.size,
        &request.sha256,
    )
    .await?;
    let service = storage_service(&state)?;
    let object_key = format!("tickets/{ticket_id}/{}/{}", Uuid::new_v4(), filename);
    let presigned = service
        .presign_put(&object_key, Some(request.content_type.trim()))
        .map_err(|error| storage_error("attachment_presign_put", error))?;
    Ok(Json(AttachmentPresignReply {
        object_key,
        upload_url: presigned.url,
        expires_at: presigned.expires_at,
    }))
}

/// 后端代理上传：`POST /admin/tickets/{id}/attachments/upload?filename=...`。
///
/// 请求体是文件原始字节（≤20MiB，受 `MAX_BODY_BYTES` 限制）；服务端计算 sha256 后
/// 直接写入对象存储并登记附件。前端同源调用，无需对象存储 CORS。
pub async fn upload(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(ticket_id): Path<Uuid>,
    Query(query): Query<UploadAttachmentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<(StatusCode, Json<TicketAttachment>)> {
    require_attachment_write(&state, id).await?;

    let header_content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim())
        .filter(|value| !value.is_empty());
    let content_type = query
        .content_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(header_content_type)
        .unwrap_or("application/octet-stream")
        .to_owned();

    if body.is_empty() {
        return Err(AppError::bad_request("附件内容不能为空"));
    }
    let size = body.len() as i64;
    let sha256 = hex::encode(Sha256::digest(&body));
    let filename = ensure_upload_target(
        &state,
        id,
        ticket_id,
        &query.filename,
        &content_type,
        size,
        &sha256,
    )
    .await?;

    let service = storage_service(&state)?;
    let object_key = format!("tickets/{ticket_id}/{}/{}", Uuid::new_v4(), filename);
    service
        .put_bytes(&object_key, &body, &content_type)
        .await
        .map_err(|error| storage_error("attachment_proxy_put", error))?;

    let mut attachment = state
        .ticket_attachments
        .insert(
            ticket_id,
            object_key.trim(),
            query.filename.trim(),
            content_type.trim(),
            size,
            sha256.as_str(),
            id,
        )
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "ticket_attachment.create",
        "ticket_attachment",
        Some(attachment.id.to_string()),
        json!({
            "ticket_id": ticket_id,
            "object_key": attachment.object_key,
            "filename": attachment.filename,
            "size": attachment.size,
            "mode": "proxy_upload",
        }),
        ip.as_deref(),
    )
    .await;
    attachment.download_url = service
        .presign_get(&attachment.object_key)
        .ok()
        .map(|presigned| presigned.url);
    Ok((StatusCode::CREATED, Json(attachment)))
}

pub async fn register(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(ticket_id): Path<Uuid>,
    ApiJson(request): ApiJson<RegisterAttachmentRequest>,
) -> AppResult<(StatusCode, Json<TicketAttachment>)> {
    require_attachment_write(&state, id).await?;
    let _filename = ensure_upload_target(
        &state,
        id,
        ticket_id,
        &request.filename,
        &request.content_type,
        request.size,
        &request.sha256,
    )
    .await?;
    let expected_prefix = format!("tickets/{ticket_id}/");
    if !request.object_key.starts_with(&expected_prefix) {
        return Err(AppError::bad_request(format!(
            "object_key 必须以 {expected_prefix} 开头"
        )));
    }
    if let Some(service) = state.storage.as_ref() {
        let exists = service
            .object_exists(request.object_key.trim())
            .await
            .map_err(|error| storage_error("attachment_object_exists", error))?;
        if !exists {
            return Err(AppError::bad_request("对象不存在，请先上传附件"));
        }
    }
    let attachment = state
        .ticket_attachments
        .insert(
            ticket_id,
            request.object_key.trim(),
            request.filename.trim(),
            request.content_type.trim(),
            request.size,
            request.sha256.trim().to_ascii_lowercase().as_str(),
            id,
        )
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "ticket_attachment.create",
        "ticket_attachment",
        Some(attachment.id.to_string()),
        json!({
            "ticket_id": ticket_id,
            "object_key": attachment.object_key,
            "filename": attachment.filename,
            "size": attachment.size,
        }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(attachment)))
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(ticket_id): Path<Uuid>,
) -> AppResult<Json<Vec<TicketAttachment>>> {
    authorization::require_permission(&state, id, permissions::TICKET_READ).await?;

    if state.tickets.get_ticket(ticket_id).await?.is_none() {
        return Err(AppError::not_found("工单不存在"));
    }
    let scoped = super::tickets::scope_area_ids(&state, id).await?;
    let area_id = state.tickets.ticket_area(ticket_id).await?;
    if !super::tickets::area_in_scope(&scoped, area_id) {
        return Err(AppError::not_found("工单不存在"));
    }

    let mut attachments = state.ticket_attachments.list(ticket_id).await?;
    if let Some(service) = state.storage.as_ref() {
        for attachment in &mut attachments {
            attachment.download_url = service
                .presign_get(&attachment.object_key)
                .ok()
                .map(|presigned| presigned.url);
        }
    }
    Ok(Json(attachments))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path((ticket_id, attachment_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    authorization::require_any_permission(
        &state,
        id,
        &[
            permissions::TICKET_ATTACHMENT,
            permissions::TICKET_HANDLE,
            permissions::TICKET_MANAGE,
        ],
    )
    .await?;

    if state.tickets.get_ticket(ticket_id).await?.is_none() {
        return Err(AppError::not_found("工单不存在"));
    }
    super::tickets::ensure_write_scope(&state, id, ticket_id).await?;

    let attachment = state
        .ticket_attachments
        .get(attachment_id)
        .await?
        .filter(|attachment| attachment.ticket_id == ticket_id)
        .ok_or_else(|| AppError::not_found("附件不存在"))?;

    // 上传者本人或拥有 ticket:manage 的人可以删除。
    let is_uploader = attachment.uploaded_by == Some(id);
    if !is_uploader {
        let granted = authorization::permissions_of(&state, id).await?;
        if !granted
            .iter()
            .any(|code| code == permissions::TICKET_MANAGE)
        {
            return Err(AppError::forbidden(
                "只有上传者或拥有 ticket:manage 的人可以删除附件",
            ));
        }
    }

    state
        .ticket_attachments
        .delete(ticket_id, attachment_id)
        .await?;
    // 元数据删除后尽力清理对象存储；失败只记日志，不影响接口结果。
    if let Some(service) = state.storage.as_ref()
        && let Err(error) = service.delete_object(&attachment.object_key).await
    {
        tracing::warn!(%error, ticket_id = %ticket_id, attachment_id = %attachment_id, "删除附件对象失败");
    }
    super::audit::record_or_log(
        &state,
        id,
        "ticket_attachment.delete",
        "ticket_attachment",
        Some(attachment_id.to_string()),
        json!({ "ticket_id": ticket_id, "filename": attachment.filename }),
        ip.as_deref(),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// 后端代理下载：`GET /admin/tickets/{id}/attachments/{attachment_id}/download`。
///
/// 权限 `ticket:read` + 工单数据范围；返回 `Content-Disposition: attachment`，
/// 避免浏览器直接预览，也避免前端依赖对象存储 CORS。
pub async fn download(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path((ticket_id, attachment_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Response> {
    authorization::require_permission(&state, id, permissions::TICKET_READ).await?;
    if state.tickets.get_ticket(ticket_id).await?.is_none() {
        return Err(AppError::not_found("工单不存在"));
    }
    let scoped = super::tickets::scope_area_ids(&state, id).await?;
    let area_id = state.tickets.ticket_area(ticket_id).await?;
    if !super::tickets::area_in_scope(&scoped, area_id) {
        return Err(AppError::not_found("工单不存在"));
    }
    let attachment = state
        .ticket_attachments
        .get(attachment_id)
        .await?
        .filter(|attachment| attachment.ticket_id == ticket_id)
        .ok_or_else(|| AppError::not_found("附件不存在"))?;
    let service = storage_service(&state)?;
    let bytes = service
        .get_bytes(&attachment.object_key)
        .await
        .map_err(|error| storage_error("attachment_download_get", error))?;

    let content_type = HeaderValue::from_str(&attachment.content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    let disposition = format!(
        "attachment; filename*=UTF-8''{}",
        rfc5987_encode(&attachment.filename)
    );
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition)
                .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
        )
        .header(header::CONTENT_LENGTH, attachment.size.to_string())
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|_| AppError::internal("构造下载响应失败"))?;
    Ok(response)
}

fn rfc5987_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}
