//! ISO seed 目录扫描（契约 `docs/image-studio.md` 附录 B.2）。
//!
//! 扫描 `ISO_SEED_DIR`（默认 `/var/lib/qe-control/iso-seeds`）下的 `.iso` 文件：
//! 1. 文件名安全校验（`[A-Za-z0-9._-]{1,128}` 且 `.iso`）；
//! 2. 流式计算 SHA-256（4.7GB 文件也不读入内存）+ 上传 S3（`StorageService::put_file` 流式）；
//! 3. 同 filename 同 size+sha → skipped；同名但内容/状态不同 → failed 项（不覆盖已有资产）；
//! 4. 新文件登记为 `is_preset=true` 的 ready ISO。
//!
//! 启动时由 `spawn_initial_scan` 后台执行一次（失败只 warn，不阻塞启动）；
//! 也可通过 `POST /admin/isos/seed/scan` 手动触发。

use std::path::Path;

use qe_domain::install_iso::{
    MAX_ISO_BYTES, NewInstallIso, SeedDecision, iso_storage_key, seed_decision,
    validate_iso_filename,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::state::AppState;

/// seed 上传 S3 的单次超时（4.7GB 在慢链路上可能 >30 分钟）。
const SEED_UPLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2 * 60 * 60);

/// 流式哈希缓冲区大小（1 MiB）。
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

/// seed 扫描失败项。
#[derive(Debug, Clone, Serialize)]
pub struct SeedFailure {
    pub filename: String,
    pub error: String,
}

/// seed 扫描结果（契约 B.2：`{scanned, imported, skipped, failed:[...]}`）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct SeedScanReply {
    pub scanned: usize,
    pub imported: usize,
    pub skipped: usize,
    pub failed: Vec<SeedFailure>,
}

impl SeedScanReply {
    fn fail(&mut self, filename: impl Into<String>, error: impl Into<String>) {
        self.failed.push(SeedFailure {
            filename: filename.into(),
            error: error.into(),
        });
    }
}

/// 启动时后台执行一次 seed 扫描（不阻塞启动；无 seed 目录/无 S3 只 warn）。
pub fn spawn_initial_scan(state: AppState) {
    tokio::spawn(async move {
        match scan_seed_dir(&state, None).await {
            Ok(reply) => {
                if reply.scanned > 0 {
                    tracing::info!(
                        scanned = reply.scanned,
                        imported = reply.imported,
                        skipped = reply.skipped,
                        failed = reply.failed.len(),
                        "ISO seed 目录扫描完成"
                    );
                } else {
                    tracing::debug!("ISO seed 目录为空或不可读，跳过导入");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "ISO seed 目录扫描失败（不阻塞启动）");
            }
        }
    });
}

/// 扫描 seed 目录并导入预设 ISO。
///
/// 返回 `Err` 只代表目录级失败（数据库不可用等）；单个文件失败记在 `failed[]`。
pub async fn scan_seed_dir(state: &AppState, actor: Option<Uuid>) -> anyhow::Result<SeedScanReply> {
    let mut reply = SeedScanReply::default();
    let dir = state.config.iso_seed_dir.clone();

    let Some(storage) = state.storage.as_ref() else {
        tracing::warn!(
            dir = %dir.display(),
            "对象存储未配置，跳过 ISO seed 扫描（上传/分发接口不可用）"
        );
        return Ok(reply);
    };

    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(dir = %dir.display(), "ISO seed 目录不存在，跳过");
            return Ok(reply);
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "读取 ISO seed 目录失败（{}）: {error}",
                dir.display()
            ));
        }
    };

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| anyhow::anyhow!("遍历 ISO seed 目录失败: {error}"))?
    {
        let path = entry.path();
        if !entry
            .file_type()
            .await
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_iso_filename(filename) {
            continue;
        }
        reply.scanned += 1;
        import_seed_file(state, storage.as_ref(), &path, filename, actor, &mut reply).await;
    }

    Ok(reply)
}

/// 导入单个 seed 文件（失败只记入 `reply.failed`，不中断整轮扫描）。
async fn import_seed_file(
    state: &AppState,
    storage: &qe_storage::StorageService,
    path: &Path,
    filename: &str,
    actor: Option<Uuid>,
    reply: &mut SeedScanReply,
) {
    let filename_owned = filename.to_owned();
    let result: anyhow::Result<Option<()>> = async {
        let filename = validate_iso_filename(filename)
            .map_err(|error| anyhow::anyhow!("文件名不合法: {error}"))?;
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| anyhow::anyhow!("读取文件大小失败: {error}"))?;
        let size_bytes = metadata.len();
        if size_bytes == 0 {
            return Err(anyhow::anyhow!("文件为空"));
        }
        if size_bytes > MAX_ISO_BYTES {
            return Err(anyhow::anyhow!(
                "文件超过 {} GiB 上限",
                MAX_ISO_BYTES / (1024 * 1024 * 1024)
            ));
        }

        // 先算 sha（流式），再按已有记录判断跳过/冲突。
        let sha256 = sha256_file(path)
            .await
            .map_err(|error| anyhow::anyhow!("计算 SHA-256 失败: {error}"))?;
        let existing = state
            .isos
            .find_by_filename(&filename)
            .await
            .map_err(|error| anyhow::anyhow!("查询已有 ISO 失败: {error}"))?;
        let decision = seed_decision(
            existing
                .as_ref()
                .map(|iso| (iso.status, iso.size_bytes, iso.sha256.as_deref())),
            size_bytes as i64,
            &sha256,
        );
        match decision {
            SeedDecision::Skip => return Ok(None),
            SeedDecision::Conflict => {
                let status = existing.map(|iso| iso.status.as_str()).unwrap_or("ready");
                return Err(anyhow::anyhow!(
                    "同名 ISO 已存在（status={status}）且内容/大小不同，已跳过导入"
                ));
            }
            SeedDecision::Import => {}
        }

        let id = Uuid::new_v4();
        let record = state
            .isos
            .create(&NewInstallIso {
                id,
                filename,
                display_name: display_name_for(&filename_owned),
                description: None,
                sha256: Some(sha256.clone()),
                is_preset: true,
                uploader_user_id: actor,
            })
            .await
            .map_err(|error| anyhow::anyhow!("登记 ISO 记录失败: {error}"))?;

        let storage_key = iso_storage_key(id, &record.filename);
        if let Err(error) = storage
            .put_file(
                &storage_key,
                path,
                "application/octet-stream",
                SEED_UPLOAD_TIMEOUT,
            )
            .await
        {
            let _ = state.isos.update_failed(record.id).await;
            return Err(anyhow::anyhow!("上传 S3 失败: {error}"));
        }
        match state
            .isos
            .update_ready(record.id, size_bytes as i64, &sha256, &storage_key)
            .await
        {
            Ok(Some(_)) => Ok(Some(())),
            Ok(None) => Err(anyhow::anyhow!("ISO 记录状态已变化，回填 ready 失败")),
            Err(error) => Err(anyhow::anyhow!("回填 ISO ready 状态失败: {error}")),
        }
    }
    .await;

    match result {
        Ok(Some(())) => reply.imported += 1,
        Ok(None) => reply.skipped += 1,
        Err(error) => reply.fail(filename_owned, format!("{error:#}")),
    }
}

/// 文件名是否 `.iso` 结尾（大小写不敏感）。
pub fn is_iso_filename(filename: &str) -> bool {
    let lower = filename.trim().to_ascii_lowercase();
    lower.ends_with(".iso") && lower.len() > 4
}

/// 默认展示名：去掉 `.iso` 后缀的文件名（保留原始大小写）。
pub fn display_name_for(filename: &str) -> String {
    let trimmed = filename.trim();
    let stem = if trimmed.to_ascii_lowercase().ends_with(".iso") {
        &trimmed[..trimmed.len() - 4]
    } else {
        trimmed
    };
    if stem.is_empty() {
        trimmed.to_owned()
    } else {
        stem.to_owned()
    }
}

/// 流式计算文件 SHA-256（1 MiB 缓冲，不把大文件读入内存）。
pub async fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_filename_detection_and_display_name() {
        assert!(is_iso_filename("Win10_LTSC_2021.iso"));
        assert!(is_iso_filename("A.ISO"));
        assert!(!is_iso_filename("readme.txt"));
        assert!(!is_iso_filename(".iso"));
        assert_eq!(display_name_for("Win10_LTSC_2021.iso"), "Win10_LTSC_2021");
        assert_eq!(display_name_for("A.ISO"), "A");
        assert_eq!(display_name_for("noext"), "noext");
    }

    #[tokio::test]
    async fn sha256_file_is_streamed_and_matches_known_vector() {
        let dir = std::env::temp_dir().join(format!("qe-iso-seed-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("abc.iso");
        tokio::fs::write(&path, b"abc").await.unwrap();
        assert_eq!(
            sha256_file(&path).await.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
