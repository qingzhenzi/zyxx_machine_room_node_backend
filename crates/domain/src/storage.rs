//! 资源与镜像管理的领域模型与校验。
//!
//! 契约见 `docs/storage-design.md` §14.2。镜像用「manifest + 内容寻址分片」描述：
//! `manifest_sha256`（[`Manifest::digest_hex`]）是镜像的权威标识，`version` 只用于
//! 展示与排序。P0 只接受 `compression = none`，zstd/gzip 字段保留给 P1。

use std::{collections::HashSet, fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{DomainError, DomainResult};

/// 镜像类型，对应 `image_artifacts.kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    Docker,
    Oci,
    LxcRootfs,
    VmDisk,
}

impl ImageKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Oci => "oci",
            Self::LxcRootfs => "lxc_rootfs",
            Self::VmDisk => "vm_disk",
        }
    }
}

impl fmt::Display for ImageKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ImageKind {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "docker" => Ok(Self::Docker),
            "oci" => Ok(Self::Oci),
            "lxc_rootfs" => Ok(Self::LxcRootfs),
            "vm_disk" => Ok(Self::VmDisk),
            _ => Err(()),
        }
    }
}

/// 镜像产物格式，对应 `image_artifacts.format`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    DockerArchive,
    OciLayout,
    Tar,
    Qcow2,
    Raw,
}

impl ImageFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DockerArchive => "docker_archive",
            Self::OciLayout => "oci_layout",
            Self::Tar => "tar",
            Self::Qcow2 => "qcow2",
            Self::Raw => "raw",
        }
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ImageFormat {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "docker_archive" => Ok(Self::DockerArchive),
            "oci_layout" => Ok(Self::OciLayout),
            "tar" => Ok(Self::Tar),
            "qcow2" => Ok(Self::Qcow2),
            "raw" => Ok(Self::Raw),
            _ => Err(()),
        }
    }
}

/// 压缩算法。P0 只接受 [`Compression::None`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    None,
    Zstd,
    Gzip,
}

impl Compression {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Zstd => "zstd",
            Self::Gzip => "gzip",
        }
    }
}

impl fmt::Display for Compression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Compression {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "zstd" => Ok(Self::Zstd),
            "gzip" => Ok(Self::Gzip),
            _ => Err(()),
        }
    }
}

/// 镜像产物状态机：`uploaded → manifest_ready → verifying → verified → published → deprecated/revoked`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Uploaded,
    ManifestReady,
    Verifying,
    Verified,
    Published,
    Deprecated,
    Revoked,
}

impl ArtifactStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uploaded => "uploaded",
            Self::ManifestReady => "manifest_ready",
            Self::Verifying => "verifying",
            Self::Verified => "verified",
            Self::Published => "published",
            Self::Deprecated => "deprecated",
            Self::Revoked => "revoked",
        }
    }
}

impl fmt::Display for ArtifactStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ArtifactStatus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "uploaded" => Ok(Self::Uploaded),
            "manifest_ready" => Ok(Self::ManifestReady),
            "verifying" => Ok(Self::Verifying),
            "verified" => Ok(Self::Verified),
            "published" => Ok(Self::Published),
            "deprecated" => Ok(Self::Deprecated),
            "revoked" => Ok(Self::Revoked),
            _ => Err(()),
        }
    }
}

/// 传输方向：`download` 分发到房间，`upload` 从房间收集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Download,
    Upload,
}

impl TransferDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Upload => "upload",
        }
    }
}

impl fmt::Display for TransferDirection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TransferDirection {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "download" => Ok(Self::Download),
            "upload" => Ok(Self::Upload),
            _ => Err(()),
        }
    }
}

/// 传输状态机：`pending → running → completed / failed / expired`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Expired,
}

impl TransferStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

impl fmt::Display for TransferStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for TransferStatus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "expired" => Ok(Self::Expired),
            _ => Err(()),
        }
    }
}

/// 收集状态机：`pending → uploaded → verified / rejected`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionStatus {
    Pending,
    Uploaded,
    Verified,
    Rejected,
}

impl CollectionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Uploaded => "uploaded",
            Self::Verified => "verified",
            Self::Rejected => "rejected",
        }
    }
}

impl fmt::Display for CollectionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CollectionStatus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "uploaded" => Ok(Self::Uploaded),
            "verified" => Ok(Self::Verified),
            "rejected" => Ok(Self::Rejected),
            _ => Err(()),
        }
    }
}

/// 镜像发布通道：`dev / test / prod`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageChannel {
    Dev,
    Test,
    Prod,
}

impl ImageChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Test => "test",
            Self::Prod => "prod",
        }
    }
}

impl fmt::Display for ImageChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ImageChannel {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "dev" => Ok(Self::Dev),
            "test" => Ok(Self::Test),
            "prod" => Ok(Self::Prod),
            _ => Err(()),
        }
    }
}

/// manifest 中的单个分片。内容寻址：`sha256` 同时决定对象 key（`objects/{sha256}`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestChunk {
    pub index: i32,
    pub object_key: String,
    pub sha256: String,
    pub size: i64,
    pub offset: i64,
}

/// manifest 中与运行时相关的可选信息（Docker / LXC / VM）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRuntime {
    pub docker_image_ref: Option<String>,
    pub lxc_distro: Option<String>,
    pub lxc_release: Option<String>,
    pub vm_os: Option<String>,
    pub vm_disk_size: Option<i64>,
}

/// 镜像清单。字段顺序即 JSON 字段顺序，[`Manifest::digest_hex`] 对序列化后的字节取 sha256。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: i32,
    pub kind: ImageKind,
    pub name: String,
    pub version: String,
    pub platform: String,
    pub format: ImageFormat,
    pub compression: Compression,
    pub total_size: i64,
    pub chunks: Vec<ManifestChunk>,
    pub runtime: ManifestRuntime,
    pub annotations: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
}

impl Manifest {
    /// 序列化为 manifest JSON（字段顺序固定，供签名与内容寻址使用）。
    pub fn to_json(&self) -> DomainResult<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|error| DomainError::internal(format!("manifest 序列化失败：{error}")))
    }

    /// `sha256(manifest_json)` 的小写 hex，即镜像 digest。
    pub fn digest_hex(&self) -> DomainResult<String> {
        Ok(sha256_hex(&self.to_json()?))
    }
}

/// S3 对象登记（`storage_objects`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageObject {
    pub id: Uuid,
    pub bucket: String,
    pub object_key: String,
    pub sha256: String,
    pub size: i64,
    pub content_type: String,
    pub created_at: DateTime<Utc>,
}

/// 镜像产物（`image_artifacts`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageArtifact {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub kind: ImageKind,
    pub format: ImageFormat,
    pub compression: Compression,
    pub platform: String,
    pub manifest_digest: Option<String>,
    pub total_size: i64,
    pub status: ArtifactStatus,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 镜像分片（`image_chunks`），由 manifest 落库而来。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageChunk {
    pub manifest_digest: String,
    pub index: i32,
    pub object_key: String,
    pub sha256: String,
    pub size: i64,
    pub offset: i64,
}

/// 镜像验证报告（`image_verifications`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageVerification {
    pub id: Uuid,
    pub artifact_id: Uuid,
    pub ok: bool,
    pub report: serde_json::Value,
    pub verified_by: Uuid,
    pub verified_at: DateTime<Utc>,
}

/// 镜像通道行（`image_channels`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageChannelRow {
    pub id: Uuid,
    pub artifact_id: Uuid,
    pub channel: ImageChannel,
    pub is_current: bool,
    pub created_at: DateTime<Utc>,
}

/// 镜像到房间/区域的分配（`image_assignments`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAssignment {
    pub id: Uuid,
    pub image_id: Uuid,
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub channel: ImageChannel,
    pub rollout_state: String,
    pub assigned_by: Uuid,
    pub assigned_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 分发/收集传输任务（`transfers`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub id: Uuid,
    pub direction: TransferDirection,
    pub room_id: Uuid,
    pub image_id: Option<Uuid>,
    pub manifest_digest: Option<String>,
    pub status: TransferStatus,
    pub progress: i32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 收集上来的产物（`collections`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: Uuid,
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub kind: String,
    pub object_key: String,
    pub sha256: String,
    pub size: i64,
    pub status: CollectionStatus,
    pub created_at: DateTime<Utc>,
}

/// 创建镜像产物的输入。
#[derive(Debug, Clone)]
pub struct NewImage {
    pub name: String,
    pub version: String,
    pub kind: ImageKind,
    pub format: ImageFormat,
    pub compression: Compression,
    pub platform: String,
    pub created_by: Option<Uuid>,
}

/// 镜像列表过滤条件。
#[derive(Debug, Clone, Default)]
pub struct ImageFilter {
    pub kind: Option<ImageKind>,
    pub status: Option<ArtifactStatus>,
    pub name: Option<String>,
}

/// 校验镜像名称：1..=64，只允许小写字母、数字与 `- _ .`，返回去空白后的名称。
pub fn validate_image_name(raw: &str) -> DomainResult<String> {
    let name = raw.trim();
    let length = name.chars().count();
    if !(1..=64).contains(&length) {
        return Err(DomainError::bad_request("镜像名称长度需为 1-64 个字符"));
    }
    if !name.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_' | '.')
    }) {
        return Err(DomainError::bad_request(
            "镜像名称只能包含小写字母、数字、短横线、下划线和点",
        ));
    }
    Ok(name.to_owned())
}

/// 校验镜像版本：1..=64，只允许字母、数字与 `.-_+`，返回去空白后的版本。
pub fn validate_image_version(raw: &str) -> DomainResult<String> {
    let version = raw.trim();
    let length = version.chars().count();
    if !(1..=64).contains(&length) {
        return Err(DomainError::bad_request("镜像版本长度需为 1-64 个字符"));
    }
    if !version.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
    }) {
        return Err(DomainError::bad_request(
            "镜像版本只能包含字母、数字、点、短横线、下划线和加号",
        ));
    }
    Ok(version.to_owned())
}

/// 校验平台：`{os}/{arch}`，两段各 1..=32，只允许字母、数字与 `-_.`。
pub fn validate_platform(raw: &str) -> DomainResult<String> {
    let platform = raw.trim();
    let Some((os, arch)) = platform.split_once('/') else {
        return Err(DomainError::bad_request(
            "平台格式应为 {os}/{arch}（如 linux/amd64）",
        ));
    };
    if os.is_empty() || arch.is_empty() || arch.contains('/') {
        return Err(DomainError::bad_request(
            "平台格式应为 {os}/{arch}（如 linux/amd64）",
        ));
    }
    if !(1..=32).contains(&os.chars().count()) || !(1..=32).contains(&arch.chars().count()) {
        return Err(DomainError::bad_request(
            "平台的操作系统与架构长度需为 1-32 个字符",
        ));
    }
    for part in [os, arch] {
        if !part.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        }) {
            return Err(DomainError::bad_request(
                "平台只能包含字母、数字、短横线、下划线和点",
            ));
        }
    }
    Ok(platform.to_owned())
}

/// 校验 sha256：必须为 64 位小写十六进制字符串，返回去空白后的摘要。
pub fn validate_sha256_hex(raw: &str) -> DomainResult<String> {
    let digest = raw.trim();
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(DomainError::bad_request(
            "sha256 必须为 64 位小写十六进制字符串",
        ));
    }
    Ok(digest.to_owned())
}

/// 校验 S3 对象 key：1..=512，不允许 `..`，只允许字母、数字与 `-_. /`。
pub fn validate_object_key(raw: &str) -> DomainResult<String> {
    let key = raw.trim();
    let length = key.chars().count();
    if !(1..=512).contains(&length) {
        return Err(DomainError::bad_request("对象 key 长度需为 1-512 个字符"));
    }
    if key.contains("..") {
        return Err(DomainError::bad_request("对象 key 不能包含 .."));
    }
    if !key.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
    }) {
        return Err(DomainError::bad_request(
            "对象 key 只能包含字母、数字、短横线、下划线、点和斜杠",
        ));
    }
    Ok(key.to_owned())
}

/// 校验 manifest 完整性：schema、分片连续性/大小/总大小、类型-格式组合、压缩与去重。
pub fn validate_manifest(manifest: &Manifest) -> DomainResult<()> {
    if manifest.schema != 1 {
        return Err(DomainError::bad_request("manifest schema 必须为 1"));
    }
    validate_kind_format(manifest.kind, manifest.format)?;
    if manifest.compression != Compression::None {
        return Err(DomainError::bad_request("P0 仅支持 compression=none"));
    }
    if manifest.chunks.is_empty() {
        return Err(DomainError::bad_request("manifest 分片不能为空"));
    }

    let mut expected_offset: i64 = 0;
    let mut digests: HashSet<&str> = HashSet::with_capacity(manifest.chunks.len());
    for (position, chunk) in manifest.chunks.iter().enumerate() {
        if chunk.index != position as i32 {
            return Err(DomainError::bad_request(
                "manifest 分片 index 必须从 0 开始连续递增",
            ));
        }
        if chunk.size <= 0 {
            return Err(DomainError::bad_request(format!(
                "manifest 分片 {} 的大小必须大于 0",
                chunk.index
            )));
        }
        if chunk.offset != expected_offset {
            return Err(DomainError::bad_request(format!(
                "manifest 分片 {} 的 offset 必须为 {expected_offset}",
                chunk.index
            )));
        }
        if !digests.insert(chunk.sha256.as_str()) {
            return Err(DomainError::bad_request(format!(
                "manifest 分片 sha256 不能重复：{}",
                chunk.sha256
            )));
        }
        expected_offset = expected_offset
            .checked_add(chunk.size)
            .ok_or_else(|| DomainError::bad_request("manifest 分片总大小溢出"))?;
    }
    if manifest.total_size != expected_offset {
        return Err(DomainError::bad_request(format!(
            "manifest total_size 必须等于分片大小之和（{expected_offset}）"
        )));
    }
    Ok(())
}

/// 校验镜像类型与产物格式的合法组合。
fn validate_kind_format(kind: ImageKind, format: ImageFormat) -> DomainResult<()> {
    let expected = match kind {
        ImageKind::Docker => "docker_archive",
        ImageKind::Oci => "oci_layout",
        ImageKind::LxcRootfs => "tar",
        ImageKind::VmDisk => "qcow2 或 raw",
    };
    let matches = match kind {
        ImageKind::Docker => format == ImageFormat::DockerArchive,
        ImageKind::Oci => format == ImageFormat::OciLayout,
        ImageKind::LxcRootfs => format == ImageFormat::Tar,
        ImageKind::VmDisk => matches!(format, ImageFormat::Qcow2 | ImageFormat::Raw),
    };
    if !matches {
        return Err(DomainError::bad_request(format!(
            "镜像类型 {} 只能使用格式 {expected}，当前为 {}",
            kind.as_str(),
            format.as_str()
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(HEX[usize::from(byte >> 4)] as char);
        hex.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    hex
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn chunk(index: i32, offset: i64, size: i64, sha256: &str) -> ManifestChunk {
        ManifestChunk {
            index,
            object_key: format!("objects/{sha256}"),
            sha256: sha256.to_owned(),
            size,
            offset,
        }
    }

    fn sample_manifest() -> Manifest {
        let first = digest(0xab);
        let second = digest(0xcd);
        Manifest {
            schema: 1,
            kind: ImageKind::Docker,
            name: "web-console".to_owned(),
            version: "2026.10.31-1".to_owned(),
            platform: "linux/amd64".to_owned(),
            format: ImageFormat::DockerArchive,
            compression: Compression::None,
            total_size: 32,
            chunks: vec![chunk(0, 0, 16, &first), chunk(1, 16, 16, &second)],
            runtime: ManifestRuntime {
                docker_image_ref: Some("web-console:2026.10.31-1".to_owned()),
                ..Default::default()
            },
            annotations: json!({}),
            created_at: Utc.with_ymd_and_hms(2026, 10, 31, 5, 0, 0).unwrap(),
            created_by: None,
        }
    }

    #[test]
    fn image_kind_and_format_use_snake_case() {
        let kinds = [
            (ImageKind::Docker, "\"docker\""),
            (ImageKind::Oci, "\"oci\""),
            (ImageKind::LxcRootfs, "\"lxc_rootfs\""),
            (ImageKind::VmDisk, "\"vm_disk\""),
        ];
        for (kind, expected) in kinds {
            assert_eq!(serde_json::to_string(&kind).unwrap(), expected);
            let parsed: ImageKind = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, kind);
            assert_eq!(kind.as_str().parse::<ImageKind>(), Ok(kind));
        }

        let formats = [
            (ImageFormat::DockerArchive, "\"docker_archive\""),
            (ImageFormat::OciLayout, "\"oci_layout\""),
            (ImageFormat::Tar, "\"tar\""),
            (ImageFormat::Qcow2, "\"qcow2\""),
            (ImageFormat::Raw, "\"raw\""),
        ];
        for (format, expected) in formats {
            assert_eq!(serde_json::to_string(&format).unwrap(), expected);
            let parsed: ImageFormat = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, format);
            assert_eq!(format.as_str().parse::<ImageFormat>(), Ok(format));
        }

        assert_eq!("LXC_ROOTFS".parse::<ImageKind>(), Ok(ImageKind::LxcRootfs));
        assert_eq!("vm-disk".parse::<ImageKind>(), Err(()));
        assert_eq!("docker-archive".parse::<ImageFormat>(), Err(()));
    }

    #[test]
    fn status_and_channel_enums_use_contract_case() {
        let compressions = [
            (Compression::None, "\"none\""),
            (Compression::Zstd, "\"zstd\""),
            (Compression::Gzip, "\"gzip\""),
        ];
        for (value, expected) in compressions {
            assert_eq!(serde_json::to_string(&value).unwrap(), expected);
            let parsed: Compression = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, value);
        }

        let statuses = [
            (ArtifactStatus::Uploaded, "\"uploaded\""),
            (ArtifactStatus::ManifestReady, "\"manifest_ready\""),
            (ArtifactStatus::Verifying, "\"verifying\""),
            (ArtifactStatus::Verified, "\"verified\""),
            (ArtifactStatus::Published, "\"published\""),
            (ArtifactStatus::Deprecated, "\"deprecated\""),
            (ArtifactStatus::Revoked, "\"revoked\""),
        ];
        for (status, expected) in statuses {
            assert_eq!(serde_json::to_string(&status).unwrap(), expected);
            let parsed: ArtifactStatus = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, status);
            assert_eq!(status.as_str().parse::<ArtifactStatus>(), Ok(status));
        }

        let directions = [
            (TransferDirection::Download, "\"download\""),
            (TransferDirection::Upload, "\"upload\""),
        ];
        for (direction, expected) in directions {
            assert_eq!(serde_json::to_string(&direction).unwrap(), expected);
            let parsed: TransferDirection = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, direction);
        }

        let transfer_statuses = [
            (TransferStatus::Pending, "\"pending\""),
            (TransferStatus::Running, "\"running\""),
            (TransferStatus::Completed, "\"completed\""),
            (TransferStatus::Failed, "\"failed\""),
            (TransferStatus::Expired, "\"expired\""),
        ];
        for (status, expected) in transfer_statuses {
            assert_eq!(serde_json::to_string(&status).unwrap(), expected);
            let parsed: TransferStatus = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, status);
        }

        let collection_statuses = [
            (CollectionStatus::Pending, "\"pending\""),
            (CollectionStatus::Uploaded, "\"uploaded\""),
            (CollectionStatus::Verified, "\"verified\""),
            (CollectionStatus::Rejected, "\"rejected\""),
        ];
        for (status, expected) in collection_statuses {
            assert_eq!(serde_json::to_string(&status).unwrap(), expected);
            let parsed: CollectionStatus = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, status);
        }

        let channels = [
            (ImageChannel::Dev, "\"dev\""),
            (ImageChannel::Test, "\"test\""),
            (ImageChannel::Prod, "\"prod\""),
        ];
        for (channel, expected) in channels {
            assert_eq!(serde_json::to_string(&channel).unwrap(), expected);
            let parsed: ImageChannel = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, channel);
            assert_eq!(channel.as_str().parse::<ImageChannel>(), Ok(channel));
        }
        assert_eq!("PROD".parse::<ImageChannel>(), Ok(ImageChannel::Prod));
        assert_eq!("staging".parse::<ImageChannel>(), Err(()));
    }

    #[test]
    fn manifest_json_uses_contract_field_names() {
        let value: serde_json::Value =
            serde_json::from_slice(&sample_manifest().to_json().unwrap()).unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(value["kind"], "docker");
        assert_eq!(value["name"], "web-console");
        assert_eq!(value["version"], "2026.10.31-1");
        assert_eq!(value["platform"], "linux/amd64");
        assert_eq!(value["format"], "docker_archive");
        assert_eq!(value["compression"], "none");
        assert_eq!(value["total_size"], 32);
        assert_eq!(
            value["runtime"]["docker_image_ref"],
            "web-console:2026.10.31-1"
        );
        assert_eq!(value["runtime"]["lxc_distro"], serde_json::Value::Null);
        assert_eq!(value["chunks"][0]["index"], 0);
        assert_eq!(
            value["chunks"][0]["object_key"],
            format!("objects/{}", digest(0xab))
        );
        assert_eq!(value["chunks"][0]["size"], 16);
        assert_eq!(value["chunks"][0]["offset"], 0);
        assert_eq!(value["chunks"][1]["offset"], 16);
        assert_eq!(value["created_at"], "2026-10-31T05:00:00Z");
        assert_eq!(value["created_by"], serde_json::Value::Null);
    }

    #[test]
    fn manifest_json_and_digest_are_stable() {
        let manifest = sample_manifest();
        let json_a = manifest.to_json().unwrap();
        let json_b = manifest.to_json().unwrap();
        assert_eq!(json_a, json_b);

        let digest_hex = manifest.digest_hex().unwrap();
        assert_eq!(digest_hex.len(), 64);
        assert!(
            digest_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        );
        assert_eq!(digest_hex, sha256_hex(&json_a));

        // 字段变化必须导致 digest 变化。
        let mut changed = manifest.clone();
        changed.version = "2026.10.31-2".to_owned();
        assert_ne!(changed.digest_hex().unwrap(), digest_hex);
    }

    #[test]
    fn validate_manifest_accepts_valid_manifests() {
        assert!(validate_manifest(&sample_manifest()).is_ok());

        let mut vm = sample_manifest();
        vm.kind = ImageKind::VmDisk;
        vm.format = ImageFormat::Raw;
        assert!(validate_manifest(&vm).is_ok());

        let mut lxc = sample_manifest();
        lxc.kind = ImageKind::LxcRootfs;
        lxc.format = ImageFormat::Tar;
        assert!(validate_manifest(&lxc).is_ok());

        let mut oci = sample_manifest();
        oci.kind = ImageKind::Oci;
        oci.format = ImageFormat::OciLayout;
        assert!(validate_manifest(&oci).is_ok());
    }

    #[test]
    fn validate_manifest_rejects_bad_schema_and_empty_chunks() {
        let mut manifest = sample_manifest();
        manifest.schema = 2;
        assert!(validate_manifest(&manifest).is_err());

        let mut manifest = sample_manifest();
        manifest.chunks.clear();
        manifest.total_size = 0;
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn validate_manifest_rejects_non_contiguous_chunks() {
        let mut index_gap = sample_manifest();
        index_gap.chunks[1].index = 2;
        assert!(validate_manifest(&index_gap).is_err());

        let mut offset_gap = sample_manifest();
        offset_gap.chunks[1].offset = 32;
        assert!(validate_manifest(&offset_gap).is_err());

        let mut first_offset = sample_manifest();
        first_offset.chunks[0].offset = 8;
        assert!(validate_manifest(&first_offset).is_err());
    }

    #[test]
    fn validate_manifest_rejects_bad_sizes_and_total() {
        let mut zero_size = sample_manifest();
        zero_size.chunks[0].size = 0;
        assert!(validate_manifest(&zero_size).is_err());

        let mut total_mismatch = sample_manifest();
        total_mismatch.total_size = 31;
        assert!(validate_manifest(&total_mismatch).is_err());
    }

    #[test]
    fn validate_manifest_rejects_duplicate_digests() {
        let mut duplicate = sample_manifest();
        let repeated = duplicate.chunks[0].sha256.clone();
        duplicate.chunks[1].sha256 = repeated;
        assert!(validate_manifest(&duplicate).is_err());
    }

    #[test]
    fn validate_manifest_rejects_bad_kind_format_pairs() {
        for (kind, format) in [
            (ImageKind::Docker, ImageFormat::Tar),
            (ImageKind::Docker, ImageFormat::Raw),
            (ImageKind::Oci, ImageFormat::DockerArchive),
            (ImageKind::LxcRootfs, ImageFormat::Qcow2),
            (ImageKind::VmDisk, ImageFormat::Tar),
            (ImageKind::VmDisk, ImageFormat::DockerArchive),
        ] {
            let mut manifest = sample_manifest();
            manifest.kind = kind;
            manifest.format = format;
            assert!(
                validate_manifest(&manifest).is_err(),
                "应拒绝组合：{} + {}",
                kind.as_str(),
                format.as_str()
            );
        }
    }

    #[test]
    fn validate_manifest_rejects_p0_compression() {
        for compression in [Compression::Zstd, Compression::Gzip] {
            let mut manifest = sample_manifest();
            manifest.compression = compression;
            assert!(validate_manifest(&manifest).is_err());
        }
    }

    #[test]
    fn image_name_is_validated_and_trimmed() {
        assert_eq!(validate_image_name("web-console").unwrap(), "web-console");
        assert_eq!(
            validate_image_name(" ubuntu_24.04 ").unwrap(),
            "ubuntu_24.04"
        );
        assert_eq!(
            validate_image_name(&"a".repeat(64)).unwrap(),
            "a".repeat(64)
        );
        assert!(validate_image_name("").is_err());
        assert!(validate_image_name(&"a".repeat(65)).is_err());
        assert!(validate_image_name("Web-Console").is_err());
        assert!(validate_image_name("web console").is_err());
        assert!(validate_image_name("web/console").is_err());
        assert!(validate_image_name("镜像").is_err());
    }

    #[test]
    fn image_version_is_validated_and_trimmed() {
        assert_eq!(
            validate_image_version("2026.10.31-1").unwrap(),
            "2026.10.31-1"
        );
        assert_eq!(
            validate_image_version(" 1.0.0+build ").unwrap(),
            "1.0.0+build"
        );
        assert_eq!(
            validate_image_version(&"v".repeat(64)).unwrap(),
            "v".repeat(64)
        );
        assert!(validate_image_version("").is_err());
        assert!(validate_image_version(&"v".repeat(65)).is_err());
        assert!(validate_image_version("1 0").is_err());
        assert!(validate_image_version("版本").is_err());
    }

    #[test]
    fn platform_is_validated() {
        assert_eq!(validate_platform("linux/amd64").unwrap(), "linux/amd64");
        assert_eq!(validate_platform(" linux/arm64 ").unwrap(), "linux/arm64");
        assert!(validate_platform("linux").is_err());
        assert!(validate_platform("/amd64").is_err());
        assert!(validate_platform("linux/").is_err());
        assert!(validate_platform("linux/amd64/arm").is_err());
        assert!(validate_platform("linux/amd 64").is_err());
        assert!(validate_platform(&format!("{}/{}", "a".repeat(32), "b".repeat(32))).is_ok());
        assert!(validate_platform(&format!("{}/{}", "a".repeat(33), "amd64")).is_err());
    }

    #[test]
    fn sha256_hex_is_validated() {
        let valid = "a".repeat(64);
        assert_eq!(validate_sha256_hex(&valid).unwrap(), valid);
        assert_eq!(validate_sha256_hex(&format!(" {valid} ")).unwrap(), valid);
        assert!(validate_sha256_hex(&"a".repeat(63)).is_err());
        assert!(validate_sha256_hex(&"a".repeat(65)).is_err());
        assert!(validate_sha256_hex(&"A".repeat(64)).is_err());
        assert!(validate_sha256_hex(&"g".repeat(64)).is_err());
        assert!(validate_sha256_hex("").is_err());
    }

    #[test]
    fn object_key_is_validated() {
        assert_eq!(
            validate_object_key("objects/abcdef").unwrap(),
            "objects/abcdef"
        );
        assert_eq!(
            validate_object_key(&"a".repeat(512)).unwrap(),
            "a".repeat(512)
        );
        assert!(validate_object_key(&"a".repeat(513)).is_err());
        assert!(validate_object_key("").is_err());
        assert!(validate_object_key("objects/../etc/passwd").is_err());
        assert!(validate_object_key("objects/..").is_err());
        assert!(validate_object_key("objects/a b").is_err());
        assert!(validate_object_key("objects/a?b").is_err());
        assert!(validate_object_key("objects/镜像").is_err());
    }
}
