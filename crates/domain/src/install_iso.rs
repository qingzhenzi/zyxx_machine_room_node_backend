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
//! - [`seed_decision`]：seed 扫描去重/冲突判定。

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
    pub uploader_user_id: Option<Uuid>,
    /// 上传者展示名（LEFT JOIN users；preset/系统导入可为 None）。
    pub uploader_name: Option<String>,
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
    /// 调用方预声明的 sha256（可选，PUT 完成时必须匹配）。
    pub sha256: Option<String>,
    pub is_preset: bool,
    pub uploader_user_id: Option<Uuid>,
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

/// 可见性（契约 B.1）：preset 所有人可见；非 preset 仅 uploader 与 admin。
pub fn is_iso_visible(iso: &InstallIso, viewer_user_id: Uuid, is_admin: bool) -> bool {
    is_admin || iso.is_preset || iso.uploader_user_id == Some(viewer_user_id)
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
            uploader_user_id: uploader,
            uploader_name: None,
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
}
