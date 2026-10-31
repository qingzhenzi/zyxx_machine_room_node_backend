//! 资源与镜像管理 P0（一）：存储对象、分发/收集任务。
//!
//! 契约见 `docs/storage-design.md` §14.5。S3 对象一律内容寻址（`objects/{sha256}`），
//! 上传由客户端用短 TTL 预签名 PUT 完成；登记对象时服务端重新读取并校验 sha256/size，
//! 校验通过才写 `storage_objects`。
//!
//! 未配置 `S3_ENDPOINT` + `S3_BUCKET`（或 `StorageService` 初始化失败）时，
//! 本模块所有接口返回 503。

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::Page,
        rbac::permissions,
        storage::{Collection, StorageObject, Transfer, TransferStatus, validate_sha256_hex},
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

/// 单个对象大小上限。P0 对象按 16 MiB 分片，服务端登记需要把内容读进内存
/// （`StorageService::get_bytes` 上限 64 MiB），这里与之一致，避免预签名成功后登记才失败。
const MAX_OBJECT_SIZE_BYTES: i64 = 64 * 1024 * 1024;
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
/// `content_type` 只做长度/字符集约束，避免把控制字符写进库。
const MAX_CONTENT_TYPE_LEN: usize = 200;

/// 内容寻址对象键：`objects/{sha256}`。
pub(super) fn object_key(sha256: &str) -> String {
    format!("objects/{sha256}")
}

/// manifest 对象键：`manifests/{manifest_digest}.json`。
pub(super) fn manifest_key(digest: &str) -> String {
    format!("manifests/{digest}.json")
}

/// manifest 签名对象键：`signatures/{manifest_digest}.sig`。
pub(super) fn signature_key(digest: &str) -> String {
    format!("signatures/{digest}.sig")
}

/// 取对象存储服务；未配置/初始化失败返回 503。
pub(super) fn service(state: &AppState) -> AppResult<&Arc<qe_storage::StorageService>> {
    state
        .storage
        .as_ref()
        .ok_or_else(|| AppError::service_unavailable("未配置 S3"))
}

/// 把 S3 适配层的 `anyhow::Error` 映射成 503，并保留服务端日志。
pub(super) fn storage_error(action: &str, error: anyhow::Error) -> AppError {
    tracing::error!(%error, action, "对象存储操作失败");
    AppError::service_unavailable("对象存储暂时不可用")
}

/// 校验对象大小：P0 允许 1 字节 ~ 64 MiB（与 `get_bytes` 的读取上限一致）。
pub(super) fn normalize_object_size(size: i64) -> AppResult<i64> {
    if size <= 0 || size > MAX_OBJECT_SIZE_BYTES {
        return Err(AppError::bad_request(
            "size 需在 1 字节到 64 MiB 之间（P0 对象按 16 MiB 分片；空对象没有意义）",
        ));
    }
    Ok(size)
}

/// 规范化 `content_type`：缺省 `application/octet-stream`。
pub(super) fn normalize_content_type(raw: Option<&str>) -> AppResult<String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(DEFAULT_CONTENT_TYPE.to_owned()),
        Some(value) => {
            if value.len() > MAX_CONTENT_TYPE_LEN
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            {
                return Err(AppError::bad_request(
                    "content_type 不合法（≤200 个可见 ASCII 字符）",
                ));
            }
            Ok(value.to_owned())
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PresignObjectRequest {
    pub sha256: String,
    pub size: i64,
    #[serde(default)]
    pub content_type: Option<String>,
}

/// `POST /storage/objects/presign`：返回预签名 PUT 地址（客户端直传 S3）。
#[derive(Debug, Serialize)]
pub struct PresignObjectReply {
    pub bucket: String,
    pub object_key: String,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

/// `GET /storage/objects/{sha256}`：对象详情 + 预签名下载地址。
#[derive(Debug, Serialize)]
pub struct StorageObjectReply {
    pub object: StorageObject,
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct TransferQuery {
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CollectionQuery {
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

fn object_target(request: &PresignObjectRequest) -> AppResult<(String, String, i64, String)> {
    let sha256 = validate_sha256_hex(&request.sha256)?;
    let size = normalize_object_size(request.size)?;
    let content_type = normalize_content_type(request.content_type.as_deref())?;
    let key = object_key(&sha256);
    Ok((key, sha256, size, content_type))
}

/// `POST /storage/objects/presign`：校验 sha256/size 后返回预签名 PUT。
pub async fn presign_object(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ApiJson(request): ApiJson<PresignObjectRequest>,
) -> AppResult<Json<PresignObjectReply>> {
    authorization::require_permission(&state, id, permissions::STORAGE_MANAGE).await?;

    let (key, _sha256, _size, content_type) = object_target(&request)?;
    let service = service(&state)?;
    let presigned = service
        .presign_put(&key, Some(&content_type))
        .map_err(|error| storage_error("presign_put", error))?;

    Ok(Json(PresignObjectReply {
        bucket: service.bucket().to_owned(),
        object_key: key,
        upload_url: presigned.url,
        expires_at: presigned.expires_at,
    }))
}

/// `POST /storage/objects`：登记已上传对象。
///
/// 服务端从 S3 读回内容并重算 sha256/size，只有完全匹配才落库；任何不一致返回 400。
pub async fn register_object(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<PresignObjectRequest>,
) -> AppResult<(StatusCode, Json<StorageObject>)> {
    authorization::require_permission(&state, id, permissions::STORAGE_MANAGE).await?;

    let (key, sha256, size, content_type) = object_target(&request)?;
    let service = service(&state)?;

    let bytes = service
        .get_bytes(&key)
        .await
        .map_err(|error| storage_error("get_bytes", error))?;
    let actual_size = bytes.len() as i64;
    if actual_size != size {
        return Err(AppError::bad_request(format!(
            "对象大小不一致：声明 {size} 字节，S3 中实际 {actual_size} 字节"
        )));
    }
    let actual_sha256 = qe_storage::sha256_hex(&bytes);
    if actual_sha256 != sha256 {
        return Err(AppError::bad_request(format!(
            "对象 sha256 不一致：声明 {sha256}，实际 {actual_sha256}"
        )));
    }

    let object = state
        .storage_repo
        .upsert_object(service.bucket(), &key, &sha256, actual_size, &content_type)
        .await?;

    super::audit::record_or_log(
        &state,
        id,
        "storage.object.register",
        "storage_object",
        Some(object.id.to_string()),
        json!({
            "object_key": object.object_key,
            "sha256": object.sha256,
            "size": object.size,
            "content_type": object.content_type,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(object)))
}

/// `GET /storage/objects/{sha256}`：对象详情 + 预签名下载地址；未登记 404。
pub async fn get_object(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(sha256): Path<String>,
) -> AppResult<Json<StorageObjectReply>> {
    authorization::require_permission(&state, id, permissions::STORAGE_READ).await?;

    let sha256 = validate_sha256_hex(&sha256)?;
    let object = state
        .storage_repo
        .get_object(&sha256)
        .await?
        .ok_or_else(|| AppError::not_found("存储对象不存在"))?;
    let service = service(&state)?;
    let presigned = service
        .presign_get(&object.object_key)
        .map_err(|error| storage_error("presign_get", error))?;

    Ok(Json(StorageObjectReply {
        object,
        download_url: presigned.url,
        expires_at: presigned.expires_at,
    }))
}

/// `GET /transfers?room_id=&page=&page_size=`：分发/收集任务分页。
pub async fn list_transfers(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<TransferQuery>,
) -> AppResult<Json<Page<Transfer>>> {
    authorization::require_permission(&state, id, permissions::STORAGE_READ).await?;
    service(&state)?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    Ok(Json(
        state
            .storage_repo
            .list_transfers(query.room_id, page, page_size)
            .await?,
    ))
}

/// `POST /transfers/{id}/retry`：把 failed/expired 的任务重置为 pending。
pub async fn retry_transfer(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(transfer_id): Path<Uuid>,
) -> AppResult<Json<Transfer>> {
    authorization::require_permission(&state, id, permissions::STORAGE_MANAGE).await?;
    service(&state)?;

    let transfer = state
        .storage_repo
        .get_transfer(transfer_id)
        .await?
        .ok_or_else(|| AppError::not_found("分发任务不存在"))?;
    if !matches!(
        transfer.status,
        TransferStatus::Failed | TransferStatus::Expired
    ) {
        return Err(AppError::conflict(
            "只有 failed / expired 状态的分发任务可以重试",
        ));
    }
    if !state.storage_repo.retry_transfer(transfer_id).await? {
        return Err(AppError::conflict("任务状态已变化，重试失败"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "transfer.retry",
        "transfer",
        Some(transfer_id.to_string()),
        json!({
            "room_id": transfer.room_id,
            "manifest_digest": transfer.manifest_digest,
            "previous_status": transfer.status,
        }),
        ip.as_deref(),
    )
    .await;

    let transfer = state
        .storage_repo
        .get_transfer(transfer_id)
        .await?
        .ok_or_else(|| AppError::not_found("分发任务不存在"))?;
    Ok(Json(transfer))
}

/// `GET /collections?room_id=&page=&page_size=`：收集登记分页。
pub async fn list_collections(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<CollectionQuery>,
) -> AppResult<Json<Page<Collection>>> {
    authorization::require_permission(&state, id, permissions::STORAGE_READ).await?;
    service(&state)?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    Ok(Json(
        state
            .storage_repo
            .list_collections(query.room_id, page, page_size)
            .await?,
    ))
}
