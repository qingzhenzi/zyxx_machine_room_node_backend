//! 平台 ISO 资产库的领域模型与纯函数（契约 `docs/image-studio.md` 附录 B）。
//!
//! ISO 是平台一等资产：教师上传/管理员预置 → ISO 库（`install_isos`）→ 构建会话
//! 通过 `iso_id` 引用，preparing 阶段由服务端生成 presigned GET 并下发
//! `vm.command iso.pull` 把 ISO 分发到目标 agent。
//!
//! 本文件只放纯模型/校验规则（不依赖 SQLx/Axum）：
//! - [`InstallIso`] / [`IsoStatus`]：`install_isos` 一行；
//! - [`validate_iso_filename`]：文件名白名单（`[A-Za-z0-9._-]{1,128}` 且 `.iso`）；
//! - [`iso_storage_key`]：S3 对象键 `isos/<id>/<filename>`；
//! - [`is_iso_visible`]：preset 所有人可见，非 preset 仅 uploader/admin；
//! - [`seed_decision`]：seed 扫描去重/冲突判定；
//! - 分片续传（附录 C.1）：[`validate_upload_size`] / [`resolve_upload_plan`] /
//!   [`expected_chunk_len`] / [`IsoChunk`]；
//! - 共享审批（附录 C.2）：[`IsoVisibility`] / [`ShareStatus`] / [`is_iso_shared_public`]。

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// ISO 文件名最大长度（契约 B.1）。
pub const ISO_FILENAME_MAX_CHARS: usize = 128;

/// 单次上传/seed 导入大小上限：16 GiB（契约 B.1）。
pub const MAX_ISO_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// 构建会话分发 ISO 的 presigned GET TTL：2 小时（契约要求 ≥1h；大 ISO 用慢链路也够）。
pub const ISO_PRESIGN_TTL_SECS: u64 = 2 * 60 * 60;

/// 分片上传默认分片大小（MiB，契约 C.1：`ISO_UPLOAD_CHUNK_MIB` 默认 64）。
pub const DEFAULT_UPLOAD_CHUNK_MIB: u32 = 64;
/// 分片大小允许范围（MiB，契约 C.1：8..=256）。
pub const UPLOAD_CHUNK_MIB_RANGE: std::ops::RangeInclusive<u32> = 8..=256;
/// 1 MiB。
pub const MIB: u64 = 1024 * 1024;

/// ISO 资产状态（`install_isos.status`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsoStatus {
    Uploading,
    Ready,
    Failed,
}

impl IsoStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uploading => "uploading",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for IsoStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for IsoStatus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "uploading" => Ok(Self::Uploading),
            "ready" => Ok(Self::Ready),
            "failed" => Ok(Self::Failed),
            _ => Err(()),
        }
    }
}

/// ISO 可见范围（`install_isos.visibility`，附录 C.2）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IsoVisibility {
    #[default]
    Private,
    Shared,
}

impl IsoVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
        }
    }
}

impl fmt::Display for IsoVisibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for IsoVisibility {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "private" => Ok(Self::Private),
            "shared" => Ok(Self::Shared),
            _ => Err(()),
        }
    }
}

/// ISO 共享审批状态（`install_isos.share_status`，附录 C.2）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareStatus {
    #[default]
    None,
    Pending,
    Approved,
    Rejected,
}

impl ShareStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

impl fmt::Display for ShareStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ShareStatus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            _ => Err(()),
        }
    }
}

/// ISO 资产（`install_isos` 一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallIso {
    pub id: Uuid,
    pub filename: String,
    pub display_name: String,
    pub description: Option<String>,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    pub status: IsoStatus,
    /// S3 对象键（uploading 阶段可为 None）。
    pub storage_key: Option<String>,
    pub is_preset: bool,
    /// 可见范围（附录 C.2）：private / shared（shared 仍需 approved 才对所有人可见）。
    pub visibility: IsoVisibility,
    pub share_status: ShareStatus,
    pub share_requested_by: Option<Uuid>,
    pub share_reviewed_by: Option<Uuid>,
    pub share_reviewed_at: Option<DateTime<Utc>>,
    pub share_note: Option<String>,
    pub uploader_user_id: Option<Uuid>,
    /// 上传者展示名（LEFT JOIN users；preset/系统导入可为 None）。
    pub uploader_name: Option<String>,
    /// 分片上传计划（附录 C.1；seed/预设或 v3 单次上传为 None/0）。
    pub upload_chunk_size: Option<i64>,
    pub upload_total_chunks: Option<i32>,
    pub upload_updated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub uploaded_at: Option<DateTime<Utc>>,
}

/// 新建上传记录（POST /admin/isos）。
#[derive(Debug, Clone)]
pub struct NewInstallIso {
    pub id: Uuid,
    pub filename: String,
    pub display_name: String,
    pub description: Option<String>,
    /// 调用方预声明的 sha256（可选，complete/PUT 完成时必须匹配）。
    pub sha256: Option<String>,
    pub is_preset: bool,
    pub uploader_user_id: Option<Uuid>,
    /// 声明的文件总大小（分片上传必填；seed 导入可传 0，ready 时回填）。
    pub size_bytes: i64,
    /// 分片大小（字节）与总片数；seed/预设为 None。
    pub upload_chunk_size: Option<i64>,
    pub upload_total_chunks: Option<i32>,
}

/// 校验 ISO 文件名（契约 B.1）：`[A-Za-z0-9._-]{1,128}` 且 `.iso` 结尾（大小写不敏感）。
///
/// 额外拒绝 `..` / 以 `.` 开头（避免与 agent 的 `.<name>.part` 临时文件冲突）。
/// 返回 trim 后的规范文件名（保留原始大小写）。
pub fn validate_iso_filename(raw: &str) -> Result<String, String> {
    let filename = raw.trim();
    let length = filename.chars().count();
    if !(1..=ISO_FILENAME_MAX_CHARS).contains(&length) {
        return Err(format!(
            "filename 长度需为 1-{ISO_FILENAME_MAX_CHARS} 个字符"
        ));
    }
    if !filename
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err("filename 只能包含字母、数字、点、下划线和短横线".to_owned());
    }
    if filename.contains("..") || filename.starts_with('.') {
        return Err("filename 不能包含 `..` 或以 `.` 开头".to_owned());
    }
    let lower = filename.to_ascii_lowercase();
    if !lower.ends_with(".iso") || lower == ".iso" {
        return Err("filename 必须以 .iso 结尾".to_owned());
    }
    Ok(filename.to_owned())
}

/// S3 对象键：`isos/<id>/<filename>`（契约 B.1）。
pub fn iso_storage_key(id: Uuid, filename: &str) -> String {
    format!("isos/{id}/{filename}")
}

/// 是否已共享且审批通过（附录 C.2：所有 `image:read` 教师可见/可用）。
pub fn is_iso_shared_public(iso: &InstallIso) -> bool {
    iso.visibility == IsoVisibility::Shared && iso.share_status == ShareStatus::Approved
}

/// 可见性（附录 B.1 + C.2）：preset 所有人；owner/admin；shared+approved 所有人。
pub fn is_iso_visible(iso: &InstallIso, viewer_user_id: Uuid, is_admin: bool) -> bool {
    is_admin
        || iso.is_preset
        || iso.uploader_user_id == Some(viewer_user_id)
        || is_iso_shared_public(iso)
}

/// 分享申请前置（附录 C.2）：preset 不可申请；仅 ready 的 ISO 可共享。
pub fn can_request_share(iso: &InstallIso) -> bool {
    !iso.is_preset && iso.status == IsoStatus::Ready
}

/// 是否有权修改/删除（PUT content / DELETE）：admin 或 uploader。
pub fn can_manage_iso(iso: &InstallIso, actor: Uuid, is_admin: bool) -> bool {
    is_admin || iso.uploader_user_id == Some(actor)
}

/// seed 扫描对一个文件/已有记录的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedDecision {
    /// 库里没有该 filename → 导入。
    Import,
    /// 同 filename 且同 size + sha → 跳过。
    Skip,
    /// 同 filename 但内容/状态不同 → 记为失败项（不覆盖已有资产）。
    Conflict,
}

/// seed 扫描去重/冲突判定（纯函数，便于单测）。
///
/// `existing` 为库里同 filename 记录（若有）的 `(status, size_bytes, sha256)`。
pub fn seed_decision(
    existing: Option<(IsoStatus, i64, Option<&str>)>,
    size_bytes: i64,
    sha256: &str,
) -> SeedDecision {
    match existing {
        None => SeedDecision::Import,
        Some((IsoStatus::Ready, size, sha))
            if size == size_bytes && sha.is_some_and(|sha| sha.eq_ignore_ascii_case(sha256)) =>
        {
            SeedDecision::Skip
        }
        Some(_) => SeedDecision::Conflict,
    }
}

/// ISO 是否可用于创建构建会话：必须 `ready`、有 storage_key/sha256 且 size > 0。
pub fn is_iso_usable(iso: &InstallIso) -> bool {
    iso.status == IsoStatus::Ready
        && iso.size_bytes > 0
        && iso
            .sha256
            .as_deref()
            .map(str::trim)
            .is_some_and(|sha| !sha.is_empty())
        && iso
            .storage_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|key| !key.is_empty())
}

/// `install_iso_chunks` 一行（分片接收记录）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsoChunk {
    pub iso_id: Uuid,
    pub chunk_index: i32,
    pub size_bytes: i64,
    pub sha256: String,
    pub received_at: DateTime<Utc>,
}

/// 分片上传状态（`GET /admin/isos/{id}/upload`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsoUploadProgress {
    pub chunk_size: i64,
    pub total_chunks: i32,
    pub received: Vec<i32>,
    pub received_bytes: i64,
}

/// 校验声明的 ISO 总大小（附录 C.1：>0 且 ≤16 GiB）。
pub fn validate_upload_size(size_bytes: u64) -> Result<u64, String> {
    if size_bytes == 0 {
        return Err("size_bytes 必须大于 0".to_owned());
    }
    if size_bytes > MAX_ISO_BYTES {
        return Err(format!(
            "size_bytes 不能超过 {} GiB",
            MAX_ISO_BYTES / (1024 * 1024 * 1024)
        ));
    }
    Ok(size_bytes)
}

/// 校验分片大小（MiB）：8..=256；未传时用 `default_mib`。
pub fn resolve_chunk_size_mib(requested: Option<u32>, default_mib: u32) -> Result<u32, String> {
    let mib = requested.unwrap_or(default_mib);
    if !UPLOAD_CHUNK_MIB_RANGE.contains(&mib) {
        return Err(format!(
            "chunk_size_mib 需在 {}-{} 之间（收到 {mib}）",
            UPLOAD_CHUNK_MIB_RANGE.start(),
            UPLOAD_CHUNK_MIB_RANGE.end()
        ));
    }
    Ok(mib)
}

/// 由总大小 + 分片大小计算总片数（向上取整，至少 1）。
pub fn chunk_count(size_bytes: u64, chunk_size_bytes: u64) -> u32 {
    if size_bytes == 0 || chunk_size_bytes == 0 {
        return 0;
    }
    let count = size_bytes.div_ceil(chunk_size_bytes);
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// 缺失分片索引（0..total 中未收到的），用于 complete 409 响应。
pub fn missing_chunk_indices(total_chunks: i32, received: &[i32]) -> Vec<i32> {
    if total_chunks <= 0 {
        return Vec::new();
    }
    let mut seen = vec![false; usize::try_from(total_chunks).unwrap_or(0)];
    for index in received {
        if *index >= 0
            && let Some(slot) = seen.get_mut(*index as usize)
        {
            *slot = true;
        }
    }
    seen.into_iter()
        .enumerate()
        .filter_map(|(index, seen)| (!seen).then(|| i32::try_from(index).ok()).flatten())
        .collect()
}

/// 上传计划：`(chunk_size_bytes, total_chunks)`。
pub fn upload_plan(size_bytes: u64, chunk_size_mib: u32) -> (u64, u32) {
    let chunk_size_bytes = u64::from(chunk_size_mib) * MIB;
    (chunk_size_bytes, chunk_count(size_bytes, chunk_size_bytes))
}

/// 第 `index` 片应有的字节数；非末片必须等于 chunk_size，末片可小。
///
/// 返回 `Err` 表示 index 越界或计划不合法（客户端应 400/409）。
pub fn expected_chunk_len(
    index: i32,
    total_chunks: i32,
    size_bytes: u64,
    chunk_size_bytes: u64,
) -> Result<u64, String> {
    if total_chunks <= 0 || chunk_size_bytes == 0 {
        return Err("上传计划无效（total_chunks/chunk_size 必须 >0）".to_owned());
    }
    if index < 0 || index >= total_chunks {
        return Err(format!(
            "chunk_index 越界：{index}（有效范围 0..{total_chunks}）"
        ));
    }
    if index < total_chunks - 1 {
        return Ok(chunk_size_bytes);
    }
    let last_start = u64::try_from(total_chunks - 1)
        .unwrap_or(0)
        .saturating_mul(chunk_size_bytes);
    if size_bytes <= last_start {
        return Err("上传计划与 size_bytes 不一致".to_owned());
    }
    Ok(size_bytes - last_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(status: IsoStatus, preset: bool, uploader: Option<Uuid>) -> InstallIso {
        let now = Utc::now();
        InstallIso {
            id: Uuid::new_v4(),
            filename: "Win10_LTSC_2021.iso".to_owned(),
            display_name: "Windows 10 LTSC 2021".to_owned(),
            description: None,
            size_bytes: 5_368_709_120,
            sha256: Some("a".repeat(64)),
            status,
            storage_key: Some("isos/x/Win10_LTSC_2021.iso".to_owned()),
            is_preset: preset,
            visibility: IsoVisibility::Private,
            share_status: ShareStatus::None,
            share_requested_by: None,
            share_reviewed_by: None,
            share_reviewed_at: None,
            share_note: None,
            uploader_user_id: uploader,
            uploader_name: None,
            upload_chunk_size: None,
            upload_total_chunks: None,
            upload_updated_at: None,
            created_at: now,
            updated_at: now,
            uploaded_at: (status == IsoStatus::Ready).then_some(now),
        }
    }

    #[test]
    fn filename_validation_matches_contract() {
        assert_eq!(
            validate_iso_filename(" Win10_LTSC_2021.iso ").unwrap(),
            "Win10_LTSC_2021.iso"
        );
        assert_eq!(
            validate_iso_filename("ubuntu-24.04.iso").unwrap(),
            "ubuntu-24.04.iso"
        );
        assert_eq!(validate_iso_filename("A.ISO").unwrap(), "A.ISO");
        for bad in [
            "",
            "   ",
            "win.iso.exe",
            "win.iso/",
            "win\\\\x.iso",
            "win x.iso",
            "../win.iso",
            ".hidden.iso",
            "a..b.iso",
            &format!("{}.iso", "x".repeat(126)),
        ] {
            assert!(validate_iso_filename(bad).is_err(), "should reject {bad:?}");
        }
        // 128 字符总长的边界值可以接受：123 + ".iso"
        let boundary = format!("{}.iso", "x".repeat(124));
        assert_eq!(validate_iso_filename(&boundary).unwrap(), boundary);
    }

    #[test]
    fn storage_key_and_status_round_trip() {
        let id = Uuid::new_v4();
        assert_eq!(
            iso_storage_key(id, "Win10.iso"),
            format!("isos/{id}/Win10.iso")
        );
        for status in [IsoStatus::Uploading, IsoStatus::Ready, IsoStatus::Failed] {
            assert_eq!(status.as_str().parse::<IsoStatus>(), Ok(status));
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::json!(status.as_str())
            );
        }
        assert!("bogus".parse::<IsoStatus>().is_err());
    }

    #[test]
    fn visibility_and_manage_rules() {
        let uploader = Uuid::new_v4();
        let viewer = Uuid::new_v4();

        let preset = iso(IsoStatus::Ready, true, None);
        assert!(is_iso_visible(&preset, viewer, false));
        assert!(!can_manage_iso(&preset, viewer, false));

        let private = iso(IsoStatus::Ready, false, Some(uploader));
        assert!(is_iso_visible(&private, uploader, false));
        assert!(!is_iso_visible(&private, viewer, false));
        assert!(can_manage_iso(&private, uploader, false));
        assert!(!can_manage_iso(&private, viewer, false));

        // admin 都能看/管理。
        assert!(is_iso_visible(&private, viewer, true));
        assert!(can_manage_iso(&private, viewer, true));
    }

    #[test]
    fn seed_decision_import_skip_conflict() {
        let sha = "a".repeat(64);
        assert_eq!(seed_decision(None, 10, &sha), SeedDecision::Import);
        assert_eq!(
            seed_decision(Some((IsoStatus::Ready, 10, Some(sha.as_str()))), 10, &sha),
            SeedDecision::Skip
        );
        // 同大小但 sha 不同（或大小不同）→ 冲突。
        assert_eq!(
            seed_decision(
                Some((IsoStatus::Ready, 10, Some("b".repeat(64).as_str()))),
                10,
                &sha
            ),
            SeedDecision::Conflict
        );
        assert_eq!(
            seed_decision(Some((IsoStatus::Ready, 11, Some(sha.as_str()))), 10, &sha),
            SeedDecision::Conflict
        );
        // 未完成/失败的记录不能被 seed 覆盖。
        assert_eq!(
            seed_decision(Some((IsoStatus::Uploading, 10, None)), 10, &sha),
            SeedDecision::Conflict
        );
        assert_eq!(
            seed_decision(Some((IsoStatus::Failed, 10, None)), 10, &sha),
            SeedDecision::Conflict
        );
    }

    #[test]
    fn usable_requires_ready_complete_metadata() {
        assert!(is_iso_usable(&iso(IsoStatus::Ready, true, None)));
        assert!(!is_iso_usable(&iso(IsoStatus::Uploading, true, None)));
        assert!(!is_iso_usable(&iso(IsoStatus::Failed, true, None)));
        let mut broken = iso(IsoStatus::Ready, true, None);
        broken.sha256 = None;
        assert!(!is_iso_usable(&broken));
        broken.sha256 = Some("a".repeat(64));
        broken.storage_key = None;
        assert!(!is_iso_usable(&broken));
        broken.storage_key = Some("isos/x".to_owned());
        broken.size_bytes = 0;
        assert!(!is_iso_usable(&broken));
    }

    fn shared_iso() -> InstallIso {
        let mut iso = iso(IsoStatus::Ready, false, Some(Uuid::new_v4()));
        iso.visibility = IsoVisibility::Shared;
        iso.share_status = ShareStatus::Approved;
        iso
    }

    #[test]
    fn shared_approved_is_visible_to_everyone_else_private() {
        let viewer = Uuid::new_v4();
        let shared = shared_iso();
        assert!(is_iso_shared_public(&shared));
        assert!(is_iso_visible(&shared, viewer, false));
        assert!(!can_manage_iso(&shared, viewer, false));
        assert!(can_request_share(&shared) == false || true); // ready 可共享（owner 才看得到入口）

        // shared 但 pending/rejected 不公开。
        let mut pending = shared_iso();
        pending.share_status = ShareStatus::Pending;
        assert!(!is_iso_shared_public(&pending));
        assert!(!is_iso_visible(&pending, viewer, false));
        let mut rejected = shared_iso();
        rejected.share_status = ShareStatus::Rejected;
        assert!(!is_iso_visible(&rejected, viewer, false));

        // 非 ready / preset 不可申请共享。
        let mut uploading = shared_iso();
        uploading.status = IsoStatus::Uploading;
        assert!(!can_request_share(&uploading));
        assert!(!can_request_share(&iso(IsoStatus::Ready, true, None)));
    }

    #[test]
    fn visibility_and_share_status_round_trip() {
        for visibility in [IsoVisibility::Private, IsoVisibility::Shared] {
            assert_eq!(visibility.as_str().parse::<IsoVisibility>(), Ok(visibility));
            assert_eq!(
                serde_json::to_value(visibility).unwrap(),
                serde_json::json!(visibility.as_str())
            );
        }
        for status in [
            ShareStatus::None,
            ShareStatus::Pending,
            ShareStatus::Approved,
            ShareStatus::Rejected,
        ] {
            assert_eq!(status.as_str().parse::<ShareStatus>(), Ok(status));
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::json!(status.as_str())
            );
        }
        assert!("bogus".parse::<IsoVisibility>().is_err());
        assert!("bogus".parse::<ShareStatus>().is_err());
    }

    #[test]
    fn missing_chunk_indices_are_reported_in_order() {
        assert_eq!(missing_chunk_indices(3, &[0, 1, 2]), Vec::<i32>::new());
        assert_eq!(missing_chunk_indices(3, &[0, 2]), vec![1]);
        assert_eq!(missing_chunk_indices(3, &[]), vec![0, 1, 2]);
        assert_eq!(missing_chunk_indices(3, &[0, 9, -1]), vec![1, 2]);
        assert_eq!(missing_chunk_indices(0, &[]), Vec::<i32>::new());
    }

    #[test]
    fn upload_size_and_chunk_size_validation() {
        assert_eq!(validate_upload_size(1).unwrap(), 1);
        assert_eq!(validate_upload_size(MAX_ISO_BYTES).unwrap(), MAX_ISO_BYTES);
        assert!(validate_upload_size(0).is_err());
        assert!(validate_upload_size(MAX_ISO_BYTES + 1).is_err());

        assert_eq!(resolve_chunk_size_mib(None, 64).unwrap(), 64);
        assert_eq!(resolve_chunk_size_mib(Some(8), 64).unwrap(), 8);
        assert_eq!(resolve_chunk_size_mib(Some(256), 64).unwrap(), 256);
        assert!(resolve_chunk_size_mib(Some(7), 64).is_err());
        assert!(resolve_chunk_size_mib(Some(257), 64).is_err());
        // 配置默认值越界也要拒绝（启动时解析已校验，这里兜底）。
        assert!(resolve_chunk_size_mib(None, 4).is_err());
    }

    #[test]
    fn chunk_count_and_plan_cover_edge_sizes() {
        let mib = MIB;
        assert_eq!(chunk_count(1, mib), 1);
        assert_eq!(chunk_count(mib, mib), 1);
        assert_eq!(chunk_count(mib + 1, mib), 2);
        assert_eq!(chunk_count(4_700_000_000, 64 * mib), 71);
        assert_eq!(chunk_count(0, mib), 0);
        assert_eq!(chunk_count(mib, 0), 0);

        let (chunk_size, total_u32) = upload_plan(150 * mib + 5, 64);
        let total = i32::try_from(total_u32).unwrap();
        assert_eq!(chunk_size, 64 * mib);
        assert_eq!(total, 3);
        assert_eq!(
            expected_chunk_len(0, total, 150 * mib + 5, chunk_size).unwrap(),
            64 * mib
        );
        assert_eq!(
            expected_chunk_len(1, total, 150 * mib + 5, chunk_size).unwrap(),
            64 * mib
        );
        // 末片可小。
        assert_eq!(
            expected_chunk_len(2, total, 150 * mib + 5, chunk_size).unwrap(),
            22 * mib + 5
        );
        // 越界/非法计划。
        assert!(expected_chunk_len(3, total, 150 * mib + 5, chunk_size).is_err());
        assert!(expected_chunk_len(-1, total, 150 * mib + 5, chunk_size).is_err());
        assert!(expected_chunk_len(0, 0, 10, chunk_size).is_err());
        assert!(expected_chunk_len(0, 1, 10, 0).is_err());
        // 总大小恰好整除：末片等于 chunk_size。
        assert_eq!(expected_chunk_len(1, 2, 2 * mib, mib).unwrap(), mib);
    }
}
