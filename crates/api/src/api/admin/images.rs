//! 资源与镜像管理 P0（二）：镜像制品、manifest、验证、发布与房间分配。
//!
//! 契约见 `docs/storage-design.md` §14.5。镜像由「manifest + 内容寻址分片 + Ed25519 签名 +
//! 验证报告」构成，状态机为 `uploaded → manifest_ready → verified → published`。
//! P0 只支持 `compression = none`，分片按 manifest 顺序逻辑拼接。
//!
//! 未配置 S3 时所有依赖对象存储的接口返回 503；`publish` / 房间分配只依赖数据库，
//! 但镜像必须是已验证/已发布状态，否则返回 409。

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::Page,
        rbac::permissions,
        storage::{
            ArtifactStatus, Compression, ImageArtifact, ImageAssignment, ImageChannel,
            ImageChannelRow, ImageChunk, ImageFilter, ImageFormat, ImageKind, ImageVerification,
            Manifest, ManifestChunk, ManifestRuntime, NewImage, validate_image_name,
            validate_image_version, validate_manifest, validate_platform, validate_sha256_hex,
        },
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

/// verify 时用于格式 magic 检查的 tar 标识偏移（POSIX ustar）。
const TAR_MAGIC_OFFSET: usize = 257;
const TAR_MAGIC: &[u8; 5] = b"ustar";
const QCOW2_MAGIC: &[u8; 4] = b"QFI\xfb";

#[derive(Debug, Deserialize)]
pub struct ImageQuery {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateImageRequest {
    pub name: String,
    pub version: String,
    pub kind: String,
    pub format: String,
    #[serde(default)]
    pub compression: Option<String>,
    pub platform: String,
}

/// `POST /images/{id}/manifest` 的单个分片：sha256 指向已登记的 `objects/{sha256}`。
#[derive(Debug, Deserialize)]
pub struct ManifestPart {
    pub index: i32,
    pub sha256: String,
    pub size: i64,
    pub offset: i64,
}

#[derive(Debug, Deserialize)]
pub struct SubmitManifestRequest {
    pub total_size: i64,
    pub parts: Vec<ManifestPart>,
    /// 运行时信息。支持 §4 的嵌套结构（`docker` / `lxc` / `vm`），也兼容扁平字段。
    #[serde(default)]
    pub runtime: Option<RuntimeInput>,
    #[serde(default)]
    pub annotations: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RuntimeInput {
    #[serde(default)]
    pub docker: Option<DockerRuntimeInput>,
    #[serde(default)]
    pub lxc: Option<LxcRuntimeInput>,
    #[serde(default)]
    pub vm: Option<VmRuntimeInput>,
    // 兼容扁平写法（与 `ManifestRuntime` 字段同名）。
    #[serde(default)]
    pub docker_image_ref: Option<String>,
    #[serde(default)]
    pub lxc_distro: Option<String>,
    #[serde(default)]
    pub lxc_release: Option<String>,
    #[serde(default)]
    pub vm_os: Option<String>,
    #[serde(default)]
    pub vm_disk_size: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DockerRuntimeInput {
    #[serde(default)]
    pub image_ref: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LxcRuntimeInput {
    #[serde(default)]
    pub distro: Option<String>,
    #[serde(default)]
    pub release: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct VmRuntimeInput {
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub disk_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PublishImageRequest {
    pub channel: String,
}

#[derive(Debug, Deserialize)]
pub struct AssignImageRequest {
    /// 缺省 `prod`。
    #[serde(default)]
    pub channel: Option<String>,
}

/// 镜像详情：制品字段平铺 + 分片 + 最近一次验证 + 已发布通道。
#[derive(Debug, Serialize)]
pub struct ImageDetail {
    #[serde(flatten)]
    pub artifact: ImageArtifact,
    pub chunks: Vec<ImageChunk>,
    pub latest_verification: Option<ImageVerification>,
    pub channels: Vec<ImageChannelRow>,
}

/// 提交 manifest 的结果：产物 + digest + 已写入 S3 的 manifest JSON。
#[derive(Debug, Serialize)]
pub struct SubmitManifestReply {
    pub artifact: ImageArtifact,
    pub manifest_digest: String,
    pub manifest: Value,
}

/// 验证结果：更新后的产物 + 本次验证记录（`report` 里逐项列出检查）。
#[derive(Debug, Serialize)]
pub struct VerifyReply {
    pub artifact: ImageArtifact,
    pub verification: ImageVerification,
}

/// `GET /images/{id}/manifest`：manifest JSON + 预签名原始文件下载地址。
#[derive(Debug, Serialize)]
pub struct ManifestReply {
    pub artifact_id: Uuid,
    pub manifest_digest: String,
    pub manifest: Value,
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
}

fn parse_kind(raw: &str) -> AppResult<ImageKind> {
    raw.trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .parse::<ImageKind>()
        .map_err(|_| {
            AppError::bad_request(
                "kind 只能是 docker / oci / lxc_rootfs / vm_disk（也接受 lxc-rootfs / vm-disk）",
            )
        })
}

fn parse_format(raw: &str) -> AppResult<ImageFormat> {
    raw.trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .parse::<ImageFormat>()
        .map_err(|_| {
            AppError::bad_request(
                "format 只能是 docker_archive / oci_layout / tar / qcow2 / raw（也接受 docker-archive / oci-layout）",
            )
        })
}

fn parse_compression(raw: Option<&str>) -> AppResult<Compression> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Compression::None);
    };
    let compression = value
        .parse::<Compression>()
        .map_err(|_| AppError::bad_request("compression 只能是 none / zstd / gzip"))?;
    if compression != Compression::None {
        return Err(AppError::bad_request(
            "P0 仅支持 compression=none，zstd/gzip 将在 P1 开放",
        ));
    }
    Ok(compression)
}

fn parse_status(raw: &str) -> AppResult<ArtifactStatus> {
    raw.trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .parse::<ArtifactStatus>()
        .map_err(|_| {
            AppError::bad_request(
                "status 只能是 uploaded / manifest_ready / verifying / verified / published / deprecated / revoked",
            )
        })
}

fn parse_channel(raw: Option<&str>) -> AppResult<ImageChannel> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(ImageChannel::Prod),
        Some(value) => value
            .parse::<ImageChannel>()
            .map_err(|_| AppError::bad_request("channel 只能是 dev / test / prod")),
    }
}

/// kind / format 组合约束（§14.2）。
fn ensure_kind_format(kind: ImageKind, format: ImageFormat) -> AppResult<()> {
    let compatible = match kind {
        ImageKind::Docker => format == ImageFormat::DockerArchive,
        ImageKind::Oci => format == ImageFormat::OciLayout,
        ImageKind::LxcRootfs => format == ImageFormat::Tar,
        ImageKind::VmDisk => matches!(format, ImageFormat::Qcow2 | ImageFormat::Raw),
    };
    if compatible {
        Ok(())
    } else {
        Err(AppError::bad_request(format!(
            "kind={kind} 与 format={format} 不匹配（docker→docker_archive、oci→oci_layout、lxc_rootfs→tar、vm_disk→qcow2/raw）"
        )))
    }
}

/// 运行时信息：嵌套结构优先，其次扁平字段；文本字段去掉空白并限制长度。
fn normalize_runtime(input: Option<RuntimeInput>) -> AppResult<ManifestRuntime> {
    let input = input.unwrap_or_default();
    let docker = input.docker.unwrap_or_default();
    let lxc = input.lxc.unwrap_or_default();
    let vm = input.vm.unwrap_or_default();

    Ok(ManifestRuntime {
        docker_image_ref: normalize_optional_text(
            docker.image_ref.or(input.docker_image_ref),
            "runtime.docker.image_ref",
        )?,
        lxc_distro: normalize_optional_text(lxc.distro.or(input.lxc_distro), "runtime.lxc.distro")?,
        lxc_release: normalize_optional_text(
            lxc.release.or(input.lxc_release),
            "runtime.lxc.release",
        )?,
        vm_os: normalize_optional_text(vm.os.or(input.vm_os), "runtime.vm.os")?,
        vm_disk_size: vm.disk_size.or(input.vm_disk_size),
    })
}

fn normalize_optional_text(raw: Option<String>, label: &str) -> AppResult<Option<String>> {
    match raw {
        None => Ok(None),
        Some(value) => {
            let value = value.trim().to_owned();
            if value.is_empty() {
                return Ok(None);
            }
            if value.chars().count() > 200 || value.chars().any(char::is_control) {
                return Err(AppError::bad_request(format!(
                    "{label} 不合法（≤200 个字符且不含控制字符）"
                )));
            }
            Ok(Some(value))
        }
    }
}

fn normalize_annotations(raw: Option<Value>) -> AppResult<Value> {
    match raw {
        Some(value) => Ok(validation::validate_metadata(value)?),
        None => Ok(json!({})),
    }
}

/// 校验 parts 的 index/offset 连续性并生成 manifest 分片列表。
///
/// 分片对象必须在 `storage_objects` 注册且大小一致；`object_key` 由 sha256 内容寻址推出。
fn build_chunks(parts: &[ManifestPart], total_size: i64) -> AppResult<Vec<ManifestChunk>> {
    if parts.is_empty() {
        return Err(AppError::bad_request("parts 不能为空"));
    }
    if total_size <= 0 {
        return Err(AppError::bad_request("total_size 必须大于 0"));
    }

    let mut chunks = Vec::with_capacity(parts.len());
    let mut expected_offset = 0_i64;
    for (position, part) in parts.iter().enumerate() {
        if part.index != position as i32 {
            return Err(AppError::bad_request(format!(
                "分片 index 必须从 0 连续递增：第 {position} 个分片的 index 为 {}",
                part.index
            )));
        }
        if part.size <= 0 {
            return Err(AppError::bad_request(format!(
                "分片 {} 的 size 必须大于 0",
                part.index
            )));
        }
        if part.offset != expected_offset {
            return Err(AppError::bad_request(format!(
                "分片 {} 的 offset 应为 {expected_offset}，实际 {}",
                part.index, part.offset
            )));
        }
        let sha256 = validate_sha256_hex(&part.sha256)?;
        expected_offset += part.size;
        chunks.push(ManifestChunk {
            index: part.index,
            object_key: super::storage::object_key(&sha256),
            sha256,
            size: part.size,
            offset: part.offset,
        });
    }
    if expected_offset != total_size {
        return Err(AppError::bad_request(format!(
            "total_size 与分片大小之和不一致：声明 {total_size}，分片合计 {expected_offset}"
        )));
    }
    Ok(chunks)
}

/// 格式 magic 检查（compression=none）：qcow2 `QFI\xfb`；tar 系 offset 257 的 `ustar`；raw 跳过。
fn check_magic(format: ImageFormat, head: &[u8]) -> Result<(), String> {
    match format {
        ImageFormat::Raw => Ok(()),
        ImageFormat::Qcow2 => {
            if head.len() >= QCOW2_MAGIC.len() && &head[..QCOW2_MAGIC.len()] == QCOW2_MAGIC {
                Ok(())
            } else {
                Err("qcow2 魔数不是 QFI\\xfb".to_owned())
            }
        }
        ImageFormat::Tar | ImageFormat::DockerArchive | ImageFormat::OciLayout => {
            let end = TAR_MAGIC_OFFSET + TAR_MAGIC.len();
            if head.len() >= end && &head[TAR_MAGIC_OFFSET..end] == TAR_MAGIC {
                Ok(())
            } else {
                Err(format!("tar 在 offset {TAR_MAGIC_OFFSET} 缺少 ustar 标识"))
            }
        }
    }
}

/// `GET /images`：按 kind / status / name 分页。
pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<ImageQuery>,
) -> AppResult<Json<Page<ImageArtifact>>> {
    authorization::require_permission(&state, id, permissions::IMAGE_READ).await?;
    super::storage::service(&state)?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let name = validation::normalize_keyword(query.name.as_deref())?;
    let filter = ImageFilter {
        kind: query
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(parse_kind)
            .transpose()?,
        status: query
            .status
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(parse_status)
            .transpose()?,
        name: (!name.is_empty()).then_some(name),
    };

    Ok(Json(
        state
            .storage_repo
            .list_images(&filter, page, page_size)
            .await?,
    ))
}

/// `GET /images/{id}`：详情 + 分片 + 最近验证 + 通道。
pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(image_id): Path<Uuid>,
) -> AppResult<Json<ImageDetail>> {
    authorization::require_permission(&state, id, permissions::IMAGE_READ).await?;
    super::storage::service(&state)?;

    let artifact = state
        .storage_repo
        .get_image(image_id)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;
    let chunks = match artifact.manifest_digest.as_deref() {
        Some(digest) => state.storage_repo.chunks(digest).await?,
        None => Vec::new(),
    };
    let latest_verification = state.storage_repo.latest_verification(image_id).await?;
    let channels = state.storage_repo.channels(image_id).await?;

    Ok(Json(ImageDetail {
        artifact,
        chunks,
        latest_verification,
        channels,
    }))
}

/// `POST /images`：创建 artifact（`uploaded`），同名同版本同平台唯一（409）。
pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateImageRequest>,
) -> AppResult<(StatusCode, Json<ImageArtifact>)> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    super::storage::service(&state)?;

    let kind = parse_kind(&request.kind)?;
    let format = parse_format(&request.format)?;
    ensure_kind_format(kind, format)?;
    let input = NewImage {
        name: validate_image_name(&request.name)?,
        version: validate_image_version(&request.version)?,
        kind,
        format,
        compression: parse_compression(request.compression.as_deref())?,
        platform: validate_platform(&request.platform)?,
        created_by: Some(id),
    };
    let artifact = state.storage_repo.create_image(&input).await?;

    super::audit::record_or_log(
        &state,
        id,
        "image.create",
        "image",
        Some(artifact.id.to_string()),
        json!({
            "name": artifact.name,
            "version": artifact.version,
            "kind": artifact.kind,
            "format": artifact.format,
            "platform": artifact.platform,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(artifact)))
}

/// `POST /images/{id}/manifest`：提交分片清单。
///
/// 校验每个分片对应的对象已登记且大小一致 → 生成 manifest JSON → Ed25519 签名 →
/// 写 `manifests/{digest}.json` / `signatures/{digest}.sig` → 落 `image_chunks` →
/// 状态置 `manifest_ready`。
pub async fn submit_manifest(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(image_id): Path<Uuid>,
    ApiJson(request): ApiJson<SubmitManifestRequest>,
) -> AppResult<Json<SubmitManifestReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;

    let artifact = state
        .storage_repo
        .get_image(image_id)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;
    if matches!(
        artifact.status,
        ArtifactStatus::Published | ArtifactStatus::Deprecated | ArtifactStatus::Revoked
    ) {
        return Err(AppError::conflict(
            "已发布/已废弃/已吊销的镜像不能再提交 manifest",
        ));
    }
    if artifact.compression != Compression::None {
        return Err(AppError::bad_request(
            "P0 仅支持 compression=none，压缩镜像的打分片校验将在 P1 开放",
        ));
    }

    let chunks = build_chunks(&request.parts, request.total_size)?;
    for chunk in &chunks {
        let object = state
            .storage_repo
            .get_object(&chunk.sha256)
            .await?
            .ok_or_else(|| {
                AppError::bad_request(format!(
                    "分片 {} 的对象 {} 尚未登记，请先调用 POST /storage/objects",
                    chunk.index, chunk.sha256
                ))
            })?;
        if object.size != chunk.size {
            return Err(AppError::bad_request(format!(
                "分片 {} 大小与已登记对象不一致：清单 {} 字节，对象 {} 字节",
                chunk.index, chunk.size, object.size
            )));
        }
    }

    let manifest = Manifest {
        schema: 1,
        kind: artifact.kind,
        name: artifact.name.clone(),
        version: artifact.version.clone(),
        platform: artifact.platform.clone(),
        format: artifact.format,
        compression: artifact.compression,
        total_size: request.total_size,
        chunks: chunks.clone(),
        runtime: normalize_runtime(request.runtime)?,
        annotations: normalize_annotations(request.annotations)?,
        created_at: Utc::now(),
        created_by: Some(id),
    };
    validate_manifest(&manifest)?;

    let manifest_json = manifest.to_json()?;
    let manifest_digest = manifest.digest_hex()?;
    let service = super::storage::service(&state)?;
    let signature = service
        .sign_manifest(&manifest_json)
        .map_err(|error| super::storage::storage_error("sign_manifest", error))?;
    service
        .put_bytes(
            &super::storage::manifest_key(&manifest_digest),
            &manifest_json,
            "application/json",
        )
        .await
        .map_err(|error| super::storage::storage_error("put_manifest", error))?;
    service
        .put_bytes(
            &super::storage::signature_key(&manifest_digest),
            &signature,
            "application/octet-stream",
        )
        .await
        .map_err(|error| super::storage::storage_error("put_signature", error))?;

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
    state
        .storage_repo
        .replace_chunks(&manifest_digest, &rows)
        .await?;
    let artifact = state
        .storage_repo
        .mark_manifest_ready(image_id, &manifest_digest, request.total_size)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;

    let manifest_value: Value = serde_json::from_slice(&manifest_json)
        .map_err(|error| AppError::internal(format!("manifest 序列化结果不可解析: {error}")))?;

    Ok(Json(SubmitManifestReply {
        artifact,
        manifest_digest,
        manifest: manifest_value,
    }))
}

/// `POST /images/{id}/verify`：同步验证（分片 sha256/size、总大小、格式 magic、manifest 签名）。
///
/// 检查结果逐项写入 `image_verifications.report`；全部通过置 `verified`，
/// 否则回到 `manifest_ready`（`report.ok = false`），修复分片后可重新提交 manifest。
pub async fn verify(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(image_id): Path<Uuid>,
) -> AppResult<Json<VerifyReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;

    let service = super::storage::service(&state)?;
    let artifact = state
        .storage_repo
        .get_image(image_id)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;
    let manifest_digest = artifact
        .manifest_digest
        .clone()
        .ok_or_else(|| AppError::conflict("镜像尚未提交 manifest，无法验证"))?;
    if matches!(
        artifact.status,
        ArtifactStatus::Published | ArtifactStatus::Deprecated | ArtifactStatus::Revoked
    ) {
        return Err(AppError::conflict(
            "已发布/已废弃/已吊销的镜像不能重新验证，请发布新版本",
        ));
    }
    if artifact.compression != Compression::None {
        return Err(AppError::bad_request(
            "P0 仅支持 compression=none，压缩镜像请在 P1 验证",
        ));
    }
    let chunks = state.storage_repo.chunks(&manifest_digest).await?;
    if chunks.is_empty() {
        return Err(AppError::conflict("镜像没有分片数据，无法验证"));
    }

    let mut checks: Vec<Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut total_size = 0_i64;
    let mut first_chunk: Option<Vec<u8>> = None;

    for chunk in &chunks {
        match service.get_bytes(&chunk.object_key).await {
            Ok(bytes) => {
                let actual_sha256 = qe_storage::sha256_hex(&bytes);
                checks.push(json!({
                    "name": format!("chunk.{}.sha256", chunk.index),
                    "ok": actual_sha256 == chunk.sha256,
                    "detail": format!("期望 {}，实际 {}", chunk.sha256, actual_sha256),
                }));
                let actual_size = bytes.len() as i64;
                checks.push(json!({
                    "name": format!("chunk.{}.size", chunk.index),
                    "ok": actual_size == chunk.size,
                    "detail": format!("期望 {} 字节，实际 {} 字节", chunk.size, actual_size),
                }));
                total_size += actual_size;
                if chunk.index == 0 {
                    first_chunk = Some(bytes);
                }
            }
            Err(error) => {
                tracing::error!(%error, index = chunk.index, "验证镜像时读取分片失败");
                errors.push(format!("读取分片 {} 失败", chunk.index));
                checks.push(json!({
                    "name": format!("chunk.{}.sha256", chunk.index),
                    "ok": false,
                    "detail": "读取对象失败",
                }));
                checks.push(json!({
                    "name": format!("chunk.{}.size", chunk.index),
                    "ok": false,
                    "detail": "读取对象失败",
                }));
            }
        }
    }

    checks.push(json!({
        "name": "total_size",
        "ok": total_size == artifact.total_size,
        "detail": format!("期望 {} 字节，实际 {} 字节", artifact.total_size, total_size),
    }));

    match first_chunk.as_deref() {
        Some(head) => match check_magic(artifact.format, head) {
            Ok(()) => checks.push(json!({
                "name": "magic",
                "ok": true,
                "detail": if artifact.format == ImageFormat::Raw {
                    "raw 跳过 magic 校验".to_owned()
                } else {
                    format!("{} 格式 magic 校验通过", artifact.format)
                },
            })),
            Err(detail) => checks.push(json!({
                "name": "magic",
                "ok": false,
                "detail": detail,
            })),
        },
        None => checks.push(json!({
            "name": "magic",
            "ok": false,
            "detail": "缺少 index=0 的分片，无法校验格式",
        })),
    }

    let manifest_key = super::storage::manifest_key(&manifest_digest);
    match service.get_bytes(&manifest_key).await {
        Ok(manifest_json) => {
            let actual_digest = qe_storage::sha256_hex(&manifest_json);
            checks.push(json!({
                "name": "manifest.digest",
                "ok": actual_digest == manifest_digest,
                "detail": format!("期望 {manifest_digest}，实际 {actual_digest}"),
            }));
            match service
                .get_bytes(&super::storage::signature_key(&manifest_digest))
                .await
            {
                Ok(signature) => {
                    let signature_ok = service.verify_manifest(&manifest_json, &signature);
                    checks.push(json!({
                        "name": "manifest.signature",
                        "ok": signature_ok,
                        "detail": if signature_ok { "Ed25519 验签通过".to_owned() } else { "Ed25519 验签失败".to_owned() },
                    }));
                }
                Err(error) => {
                    tracing::error!(%error, "验证镜像时读取 manifest 签名失败");
                    errors.push("读取 manifest 签名失败".to_owned());
                    checks.push(json!({
                        "name": "manifest.signature",
                        "ok": false,
                        "detail": "读取签名对象失败",
                    }));
                }
            }
        }
        Err(error) => {
            tracing::error!(%error, "验证镜像时读取 manifest 失败");
            errors.push("读取 manifest 失败".to_owned());
            checks.push(json!({
                "name": "manifest.digest",
                "ok": false,
                "detail": "读取 manifest 对象失败",
            }));
            checks.push(json!({
                "name": "manifest.signature",
                "ok": false,
                "detail": "manifest 不可读，无法验签",
            }));
        }
    }

    let ok = errors.is_empty() && checks.iter().all(|check| check["ok"] == Value::Bool(true));
    let report = json!({
        "manifest_digest": manifest_digest,
        "checks": checks,
        "errors": errors,
    });
    let verification = state
        .storage_repo
        .insert_verification(image_id, ok, report, id)
        .await?;
    let status = if ok {
        ArtifactStatus::Verified
    } else {
        ArtifactStatus::ManifestReady
    };
    let artifact = state
        .storage_repo
        .set_status(image_id, status)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "image.verify",
        "image",
        Some(image_id.to_string()),
        json!({
            "manifest_digest": manifest_digest,
            "ok": ok,
            "status": artifact.status,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(VerifyReply {
        artifact,
        verification,
    }))
}

/// `POST /images/{id}/publish`：仅 `verified` 可发布，写通道（同通道旧版本下线）。
pub async fn publish(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(image_id): Path<Uuid>,
    ApiJson(request): ApiJson<PublishImageRequest>,
) -> AppResult<Json<ImageArtifact>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    super::storage::service(&state)?;

    let channel = parse_channel(Some(&request.channel))?;
    let artifact = state
        .storage_repo
        .get_image(image_id)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;
    if artifact.status != ArtifactStatus::Verified {
        return Err(AppError::conflict("仅 verified 状态的镜像可以发布"));
    }

    let artifact = state
        .storage_repo
        .publish(image_id, channel)
        .await?
        .ok_or_else(|| AppError::conflict("镜像状态已变化，发布失败"))?;

    super::audit::record_or_log(
        &state,
        id,
        "image.publish",
        "image",
        Some(image_id.to_string()),
        json!({
            "channel": channel,
            "manifest_digest": artifact.manifest_digest,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(artifact))
}

/// `GET /images/{id}/manifest`：manifest JSON + 预签名原始下载地址。
pub async fn get_manifest(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(image_id): Path<Uuid>,
) -> AppResult<Json<ManifestReply>> {
    authorization::require_permission(&state, id, permissions::IMAGE_READ).await?;

    let artifact = state
        .storage_repo
        .get_image(image_id)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;
    let manifest_digest = artifact
        .manifest_digest
        .clone()
        .ok_or_else(|| AppError::conflict("镜像尚未提交 manifest"))?;
    let service = super::storage::service(&state)?;
    let manifest_key = super::storage::manifest_key(&manifest_digest);
    let bytes = service
        .get_bytes(&manifest_key)
        .await
        .map_err(|error| super::storage::storage_error("get_manifest", error))?;
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::internal(format!("manifest 对象不是合法 JSON: {error}")))?;
    let presigned = service
        .presign_get(&manifest_key)
        .map_err(|error| super::storage::storage_error("presign_manifest", error))?;

    Ok(Json(ManifestReply {
        artifact_id: artifact.id,
        manifest_digest,
        manifest,
        download_url: presigned.url,
        expires_at: presigned.expires_at,
    }))
}

/// `PUT /rooms/{id}/images/{image_id}`：把已发布镜像分配到房间并创建分发任务。
pub async fn assign_room(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path((room_id, image_id)): Path<(Uuid, Uuid)>,
    ApiJson(request): ApiJson<AssignImageRequest>,
) -> AppResult<Json<ImageAssignment>> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    super::storage::service(&state)?;

    let channel = parse_channel(request.channel.as_deref())?;
    let artifact = state
        .storage_repo
        .get_image(image_id)
        .await?
        .ok_or_else(|| AppError::not_found("镜像不存在"))?;
    if artifact.status != ArtifactStatus::Published {
        return Err(AppError::conflict(
            "仅 published 状态的镜像可以分配到房间，请先发布通道",
        ));
    }

    let assignment = state
        .storage_repo
        .assign_room(image_id, room_id, channel, id)
        .await?
        .ok_or_else(|| AppError::not_found("房间不存在"))?;

    super::audit::record_or_log(
        &state,
        id,
        "image.assign",
        "image",
        Some(image_id.to_string()),
        json!({
            "room_id": room_id,
            "channel": channel,
            "manifest_digest": artifact.manifest_digest,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(assignment))
}

/// `DELETE /rooms/{id}/images/{image_id}`：取消房间分配（不删除镜像本身）。
pub async fn unassign_room(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path((room_id, image_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;
    super::storage::service(&state)?;

    if !state.storage_repo.unassign_room(image_id, room_id).await? {
        return Err(AppError::not_found("该房间未分配此镜像"));
    }

    super::audit::record_or_log(
        &state,
        id,
        "image.unassign",
        "image",
        Some(image_id.to_string()),
        json!({ "room_id": room_id }),
        ip.as_deref(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
