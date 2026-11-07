//! 阶段 2：从 VM 导出制作镜像 —— 设备 mTLS 上传预签名 + 登记端点。
//!
//! 契约见 `docs/vm-base-image-and-diff-disk.md` §8.5（2026-11-07 实施定稿）：
//!
//! - `POST /machine/images/presign-upload`：agent 上传前换预签名 PUT URL；
//!   body `{sha256, size_bytes}`，object_key=`objects/<sha256>`，同 sha 已存在时
//!   返回 `already_exists=true` 让 agent 跳过重复上传（内容寻址天然幂等）。
//! - `POST /machine/images/register`：agent 分片上传完成后登记镜像；
//!   服务端逐个 HEAD 核对 part 对象大小、构造单/多分片 manifest、Ed25519 签名，
//!   写 `manifests/<digest>.json` + `signatures/<digest>.sig`，并写入
//!   `image_artifacts`（status=manifest_ready, created_by=NULL）+ `image_chunks`。
//!
//! 之后由教师/管理员走既有 `POST /admin/images/{id}/verify` → `/publish` 完成闭环。

use crate::error::{Error, Result};
use crate::routes::{AppState, resolve_identity};
use crate::tls::PeerCert;
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use qe_db::storage::StorageRepo;
use qe_domain::storage::{
    Compression, ImageChunk, ImageFormat, ImageKind, Manifest, ManifestChunk, ManifestRuntime,
    NewImage, validate_image_name, validate_image_version, validate_manifest, validate_platform,
    validate_sha256_hex,
};
use qe_storage::StorageService;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

/// 单次上传的对象上限：32 GiB（设计 §8.3）。
pub const MAX_UPLOAD_BYTES: u64 = 32 * 1024 * 1024 * 1024;
/// 分片数量上限（设计 §8.5）。
pub const MAX_PARTS: usize = 2048;
/// 单个分片大小上限 64 MiB（与 agent 侧 32 MiB 切片、verify 的 get_bytes 上限对齐）。
pub const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;

/// 对象内容寻址前缀（与 qe-api / P0 存储契约一致）。
pub const OBJECT_PREFIX: &str = "objects";
/// manifest 对象前缀。
pub const MANIFEST_PREFIX: &str = "manifests";
/// manifest 签名前缀。
pub const SIGNATURE_PREFIX: &str = "signatures";

/// 分片对象键：`objects/<sha256>`。
pub fn object_key(sha256: &str) -> String {
    format!("{OBJECT_PREFIX}/{sha256}")
}

/// manifest 对象键：`manifests/<digest>.json`。
pub fn manifest_key(digest: &str) -> String {
    format!("{MANIFEST_PREFIX}/{digest}.json")
}

/// manifest 签名对象键：`signatures/<digest>.sig`。
pub fn signature_key(digest: &str) -> String {
    format!("{SIGNATURE_PREFIX}/{digest}.sig")
}

fn bad_request(message: impl Into<String>) -> Error {
    Error::BadRequest(message.into())
}

fn domain_bad_request(error: qe_domain::error::DomainError) -> Error {
    Error::BadRequest(error.to_string())
}

fn storage_error(action: &str, error: anyhow::Error) -> Error {
    tracing::error!(%error, action, "对象存储操作失败");
    Error::StorageUnavailable
}

fn storage_service(state: &AppState) -> Result<&Arc<StorageService>> {
    state.storage.as_ref().ok_or(Error::StorageUnavailable)
}

// ---------------------------------------------------------------------------
// POST /machine/images/presign-upload
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PresignUploadRequest {
    /// 待上传对象的 sha256（64 位小写 hex）。
    pub sha256: String,
    /// 对象大小（字节，> 0 且 ≤32 GiB）。
    pub size_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct PresignUploadReply {
    /// 内容寻址 object_key：`objects/<sha256>`。
    pub object_key: String,
    /// 预签名 PUT URL（短 TTL，过期前必须完成上传）。
    pub upload_url: String,
    /// 绝对过期时间（RFC3339）。
    pub expires_at: DateTime<Utc>,
    /// 同 sha 对象是否已存在（true 时 agent 可跳过上传直接 register）。
    pub already_exists: bool,
}

/// 校验上传大小：1 字节 ~ 32 GiB。
pub fn validate_upload_size(size_bytes: i64) -> Result<u64> {
    if size_bytes <= 0 {
        return Err(bad_request("size_bytes 必须大于 0"));
    }
    let size = size_bytes as u64;
    if size > MAX_UPLOAD_BYTES {
        return Err(bad_request("size_bytes 不能超过 32 GiB"));
    }
    Ok(size)
}

pub async fn presign_upload(
    State(st): State<AppState>,
    peer: PeerCert,
    Json(request): Json<PresignUploadRequest>,
) -> Result<Json<PresignUploadReply>> {
    // 只接受存活的已注册设备（与其它 mTLS 业务端点同一套 fail-closed 身份校验）。
    let _ = resolve_identity(&st, &peer).await?;
    let sha256 = validate_sha256_hex(&request.sha256).map_err(domain_bad_request)?;
    validate_upload_size(request.size_bytes)?;

    let service = storage_service(&st)?;
    let object_key = object_key(&sha256);
    let already_exists = service
        .object_exists(&object_key)
        .await
        .map_err(|error| storage_error("object_exists", error))?;
    let presigned = service
        .presign_put(&object_key, Some("application/octet-stream"))
        .map_err(|error| storage_error("presign_put", error))?;

    tracing::info!(
        %object_key,
        size_bytes = request.size_bytes,
        already_exists,
        "设备请求镜像分片上传预签名"
    );

    Ok(Json(PresignUploadReply {
        object_key,
        upload_url: presigned.url,
        expires_at: presigned.expires_at,
        already_exists,
    }))
}

// ---------------------------------------------------------------------------
// POST /machine/images/register
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterPart {
    /// 该分片内容的 sha256（64 位小写 hex）。
    pub sha256: String,
    /// 该分片大小（字节，1..=64 MiB）。
    pub size: i64,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub version: String,
    pub platform: String,
    /// 只接受 `vm_disk`。
    pub kind: String,
    /// 只接受 `qcow2`。
    pub format: String,
    /// 只接受 `none`。
    pub compression: String,
    /// 全部分片大小之和（字节，≤32 GiB）。
    pub total_size: i64,
    /// 分片列表（顺序即 offset 顺序）。
    pub parts: Vec<RegisterPart>,
    /// 可选注解（对象）；服务端会补 `source=vm-export` / `serial`。
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct RegisterReply {
    pub image_id: Uuid,
    pub manifest_digest: String,
    /// 固定 `manifest_ready`（verify/publish 由管理端后续编排）。
    pub status: &'static str,
}

/// 校验 kind/format/compression 三元组：本阶段只接受 vm_disk / qcow2 / none。
pub fn parse_manifest_enums(
    kind: &str,
    format: &str,
    compression: &str,
) -> Result<(ImageKind, ImageFormat, Compression)> {
    let kind = ImageKind::from_str(kind)
        .ok()
        .ok_or_else(|| bad_request("kind 只接受 vm_disk"))?;
    if kind != ImageKind::VmDisk {
        return Err(bad_request("kind 只接受 vm_disk"));
    }
    let format = ImageFormat::from_str(format)
        .ok()
        .ok_or_else(|| bad_request("format 只接受 qcow2"))?;
    if format != ImageFormat::Qcow2 {
        return Err(bad_request("format 只接受 qcow2"));
    }
    let compression = Compression::from_str(compression)
        .ok()
        .ok_or_else(|| bad_request("compression 只接受 none"))?;
    if compression != Compression::None {
        return Err(bad_request("compression 只接受 none"));
    }
    Ok((kind, format, compression))
}

/// 校验分片列表并构造 manifest chunks（offset 按顺序累加）。
pub fn build_chunks(parts: &[RegisterPart], total_size: i64) -> Result<Vec<ManifestChunk>> {
    if parts.is_empty() {
        return Err(bad_request("parts 不能为空"));
    }
    if parts.len() > MAX_PARTS {
        return Err(bad_request(format!("parts 不能超过 {MAX_PARTS} 个")));
    }
    if total_size <= 0 || total_size as u64 > MAX_UPLOAD_BYTES {
        return Err(bad_request("total_size 必须在 1 字节到 32 GiB 之间"));
    }

    let mut chunks = Vec::with_capacity(parts.len());
    let mut expected_offset: i64 = 0;
    let mut seen = std::collections::HashSet::with_capacity(parts.len());
    for (position, part) in parts.iter().enumerate() {
        if part.size <= 0 || part.size as u64 > MAX_PART_BYTES {
            return Err(bad_request(format!(
                "分片 {position} 的 size 必须在 1 字节到 64 MiB 之间"
            )));
        }
        let sha256 = validate_sha256_hex(&part.sha256).map_err(domain_bad_request)?;
        if !seen.insert(sha256.clone()) {
            return Err(bad_request(format!("分片 sha256 不能重复：{sha256}")));
        }
        chunks.push(ManifestChunk {
            index: position as i32,
            object_key: object_key(&sha256),
            sha256,
            size: part.size,
            offset: expected_offset,
        });
        expected_offset += part.size;
    }
    if expected_offset != total_size {
        return Err(bad_request(format!(
            "total_size 与分片大小之和不一致：声明 {total_size}，分片合计 {expected_offset}"
        )));
    }
    Ok(chunks)
}

/// 合并 agent 注解：保留调用方字段，强制覆盖 `source=vm-export` 与 `serial`。
pub fn merge_annotations(
    raw: Option<serde_json::Value>,
    serial: &str,
) -> Result<serde_json::Value> {
    let mut annotations = match raw {
        None | Some(serde_json::Value::Null) => serde_json::Map::new(),
        Some(serde_json::Value::Object(map)) => map,
        Some(_) => return Err(bad_request("annotations 必须是 JSON 对象")),
    };
    annotations.insert(
        "source".to_owned(),
        serde_json::Value::String("vm-export".to_owned()),
    );
    annotations.insert(
        "serial".to_owned(),
        serde_json::Value::String(serial.to_owned()),
    );
    Ok(serde_json::Value::Object(annotations))
}

/// 失败时清理：删除本次写入的 artifact/chunks 行与 manifest/signature 对象（尽力而为）。
async fn cleanup_failed_register(
    db: &PgPool,
    storage: &StorageService,
    image_id: Uuid,
    manifest_digest: &str,
) {
    let _ = sqlx::query("DELETE FROM image_chunks WHERE manifest_digest = $1")
        .bind(manifest_digest)
        .execute(db)
        .await;
    let _ = sqlx::query("DELETE FROM image_artifacts WHERE id = $1")
        .bind(image_id)
        .execute(db)
        .await;
    for key in [
        manifest_key(manifest_digest),
        signature_key(manifest_digest),
    ] {
        if let Err(error) = storage.delete_object(&key).await {
            tracing::warn!(%error, %key, "清理失败的镜像 manifest/signature 对象失败");
        }
    }
}

pub async fn register(
    State(st): State<AppState>,
    peer: PeerCert,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<RegisterReply>> {
    let (device_id, _slot) = resolve_identity(&st, &peer).await?;

    // ---- 纯参数校验（先做：即使对象存储未配置，非法参数也应返回 400 而不是 503） ----
    let name = validate_image_name(&request.name).map_err(domain_bad_request)?;
    let version = validate_image_version(&request.version).map_err(domain_bad_request)?;
    let platform = validate_platform(&request.platform).map_err(domain_bad_request)?;
    let (kind, format, compression) =
        parse_manifest_enums(&request.kind, &request.format, &request.compression)?;
    let chunks = build_chunks(&request.parts, request.total_size)?;
    let serial = peer
        .serial_hex
        .clone()
        .unwrap_or_else(|| device_id.to_string());
    let annotations = merge_annotations(request.annotations.clone(), &serial)?;

    let service = storage_service(&st)?;

    // ---- 对象核对：每个 part 必须存在且 HEAD 大小与声明一致 ----
    for chunk in &chunks {
        let actual = service
            .object_len(&chunk.object_key)
            .await
            .map_err(|error| storage_error("object_len", error))?;
        match actual {
            None => {
                return Err(bad_request(format!(
                    "分片 {} 的对象 {} 不存在，请先上传",
                    chunk.index, chunk.object_key
                )));
            }
            Some(len) if len != chunk.size as u64 => {
                return Err(bad_request(format!(
                    "分片 {} 对象大小不一致：声明 {} 字节，实际 {} 字节",
                    chunk.index, chunk.size, len
                )));
            }
            Some(_) => {}
        }
    }

    // ---- 构造并校验 manifest ----
    let manifest = Manifest {
        schema: 1,
        kind,
        name: name.clone(),
        version: version.clone(),
        platform: platform.clone(),
        format,
        compression,
        total_size: request.total_size,
        chunks: chunks.clone(),
        runtime: ManifestRuntime {
            vm_disk_size: Some(request.total_size),
            ..ManifestRuntime::default()
        },
        annotations,
        created_at: Utc::now(),
        created_by: None,
    };
    validate_manifest(&manifest).map_err(domain_bad_request)?;
    let manifest_json = manifest.to_json().map_err(domain_bad_request)?;
    let manifest_digest = manifest.digest_hex().map_err(domain_bad_request)?;

    // ---- 先占位 artifact（唯一约束 name/version/platform 重复 → 409） ----
    let repo = StorageRepo::new(st.db.clone());
    let artifact = repo
        .create_image(&NewImage {
            name,
            version,
            platform,
            kind,
            format,
            compression,
            created_by: None,
        })
        .await
        .map_err(|error| match error {
            qe_db::error::DbError::Conflict(_) => {
                Error::Conflict("同名/同版本/同平台的镜像已存在".to_owned())
            }
            other => Error::Internal(anyhow::anyhow!("创建镜像记录失败: {other}")),
        })?;
    let image_id = artifact.id;

    // ---- 写 manifest + signature，登记 chunks，推进状态 ----
    let result: Result<()> = async {
        let signature = service
            .sign_manifest(&manifest_json)
            .map_err(|error| storage_error("sign_manifest", error))?;
        service
            .put_bytes(
                &manifest_key(&manifest_digest),
                &manifest_json,
                "application/json",
            )
            .await
            .map_err(|error| storage_error("put_manifest", error))?;
        service
            .put_bytes(
                &signature_key(&manifest_digest),
                &signature,
                "application/octet-stream",
            )
            .await
            .map_err(|error| storage_error("put_signature", error))?;

        let rows: Vec<ImageChunk> = chunks
            .iter()
            .map(|chunk| ImageChunk {
                manifest_digest: manifest_digest.clone(),
                index: chunk.index,
                object_key: chunk.object_key.clone(),
                sha256: chunk.sha256.clone(),
                size: chunk.size,
                offset: chunk.offset,
            })
            .collect();
        repo.replace_chunks(&manifest_digest, &rows)
            .await
            .map_err(|error| Error::Internal(anyhow::anyhow!("登记镜像分片失败: {error}")))?;
        repo.mark_manifest_ready(image_id, &manifest_digest, request.total_size)
            .await
            .map_err(|error| Error::Internal(anyhow::anyhow!("更新镜像状态失败: {error}")))?;
        Ok(())
    }
    .await;

    if let Err(error) = result {
        cleanup_failed_register(&st.db, service, image_id, &manifest_digest).await;
        return Err(error);
    }

    tracing::info!(
        %image_id,
        %manifest_digest,
        %serial,
        chunks = chunks.len(),
        total_size = request.total_size,
        "设备登记 VM 导出镜像完成（manifest_ready）"
    );

    Ok(Json(RegisterReply {
        image_id,
        manifest_digest,
        status: "manifest_ready",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(sha: &str, size: i64) -> RegisterPart {
        RegisterPart {
            sha256: sha.to_owned(),
            size,
        }
    }

    fn sha(fill: char) -> String {
        fill.to_string().repeat(64)
    }

    #[test]
    fn upload_size_is_bounded() {
        assert!(validate_upload_size(0).is_err());
        assert!(validate_upload_size(-1).is_err());
        assert_eq!(validate_upload_size(1).unwrap(), 1);
        assert_eq!(
            validate_upload_size(MAX_UPLOAD_BYTES as i64).unwrap(),
            MAX_UPLOAD_BYTES
        );
        assert!(validate_upload_size(MAX_UPLOAD_BYTES as i64 + 1).is_err());
    }

    #[test]
    fn enums_accept_only_vm_qcow2_none() {
        assert!(parse_manifest_enums("vm_disk", "qcow2", "none").is_ok());
        for (kind, format, compression) in [
            ("docker", "qcow2", "none"),
            ("vm_disk", "raw", "none"),
            ("vm_disk", "qcow2", "zstd"),
            ("", "qcow2", "none"),
        ] {
            assert!(
                parse_manifest_enums(kind, format, compression).is_err(),
                "应拒绝 {kind}/{format}/{compression}"
            );
        }
    }

    #[test]
    fn chunks_offsets_and_total_are_validated() {
        let parts = vec![part(&sha('a'), 3), part(&sha('b'), 5)];
        let chunks = build_chunks(&parts, 8).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].object_key, object_key(&sha('a')));
        assert_eq!(chunks[1].index, 1);
        assert_eq!(chunks[1].offset, 3);
        assert_eq!(chunks[1].size, 5);

        // 合计不一致
        assert!(build_chunks(&parts, 9).is_err());
        // 空分片
        assert!(build_chunks(&[], 8).is_err());
        // 单分片超过 64 MiB
        assert!(build_chunks(&[part(&sha('a'), MAX_PART_BYTES as i64 + 1)], 1).is_err());
        // 非法 sha
        assert!(build_chunks(&[part("xyz", 1)], 1).is_err());
        // 重复 sha
        assert!(build_chunks(&[part(&sha('a'), 1), part(&sha('a'), 1)], 2).is_err());
    }

    #[test]
    fn annotations_force_source_and_serial() {
        let merged = merge_annotations(
            Some(serde_json::json!({"base_volume":"ubuntu-22.04","source":"forged"})),
            "abc123",
        )
        .unwrap();
        assert_eq!(merged["source"], serde_json::json!("vm-export"));
        assert_eq!(merged["serial"], serde_json::json!("abc123"));
        assert_eq!(merged["base_volume"], serde_json::json!("ubuntu-22.04"));

        assert_eq!(
            merge_annotations(None, "abc123").unwrap()["source"],
            serde_json::json!("vm-export")
        );
        assert!(merge_annotations(Some(serde_json::json!("nope")), "abc").is_err());
    }
}
