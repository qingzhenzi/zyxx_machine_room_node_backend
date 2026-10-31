//! agent 配置：环境变量解析（契约 §16.5）。
//!
//! 必填项只有两项（`DEVICE_PKI_MTLS_URL`、`DEVICE_SLOT`），其余全部有默认值，
//! 便于在开发机上直接跑。解析逻辑同时提供 [`AgentConfig::from_lookup`]，
//! 让测试无需改写进程环境即可断言默认值与缺失项。

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

/// `CACHE_MAX_BYTES` 默认 10 GiB。
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// `DOWNLOAD_CONCURRENCY` 默认 4。
pub const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 4;

/// agent 运行配置（契约 §16.4 的 `AgentConfig` + §16.5 的 `LIBVIRT_POOL`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    /// device-pki mTLS 端点，如 `https://node:4441`。
    pub mtls_endpoint: String,
    /// 设备槽位（与证书 CN 一致）。
    pub slot: String,
    /// 客户端状态/凭证目录。
    pub data_dir: PathBuf,
    /// 分片缓存目录。
    pub cache_dir: PathBuf,
    /// 缓存上限（字节）。
    pub cache_max_bytes: u64,
    /// 并发分片数。
    pub download_concurrency: usize,
    /// `docker` 可执行文件路径。
    pub docker_bin: String,
    /// `ctr` 可执行文件路径。
    pub ctr_bin: String,
    /// `incus` 可执行文件路径（为空时 plain LXC 兜底）。
    pub incus_bin: String,
    /// `virsh` 可执行文件路径。
    pub virsh_bin: String,
    /// libvirt storage pool 名。
    pub libvirt_pool: String,
    /// plain LXC 根目录。
    pub lxc_rootfs_dir: PathBuf,
}

impl AgentConfig {
    /// 从进程环境读取配置。
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// 用给定的查找函数解析配置（测试用）。
    ///
    /// 空白值视同未设置，这样 systemd 的 `Environment=FOO=` 不会静默覆盖默认值。
    pub fn from_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let get = |key: &str| -> Option<String> {
            lookup(key)
                .map(|raw| raw.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        // INCUS_BIN 是唯一允许"显式置空"的变量：空值 = 禁用 Incus、走 plain LXC 兜底
        // （契约 §16.4）。因此这里不做空白过滤。
        let incus_bin = lookup("INCUS_BIN")
            .map(|raw| raw.trim().to_owned())
            .unwrap_or_else(|| "incus".to_owned());

        let mtls_endpoint = required(&get, "DEVICE_PKI_MTLS_URL")?;
        if !(mtls_endpoint.starts_with("https://") || mtls_endpoint.starts_with("http://")) {
            bail!("DEVICE_PKI_MTLS_URL 必须是以 http:// 或 https:// 开头的 URL");
        }
        let slot = required(&get, "DEVICE_SLOT")?;

        let cache_max_bytes = match get("CACHE_MAX_BYTES") {
            Some(raw) => raw
                .parse::<u64>()
                .with_context(|| format!("CACHE_MAX_BYTES 不是合法字节数: `{raw}`"))?,
            None => DEFAULT_CACHE_MAX_BYTES,
        };
        let download_concurrency = match get("DOWNLOAD_CONCURRENCY") {
            Some(raw) => raw
                .parse::<usize>()
                .with_context(|| format!("DOWNLOAD_CONCURRENCY 不是合法整数: `{raw}`"))?,
            None => DEFAULT_DOWNLOAD_CONCURRENCY,
        };
        if download_concurrency == 0 {
            bail!("DOWNLOAD_CONCURRENCY 必须 >= 1");
        }

        Ok(Self {
            mtls_endpoint,
            slot,
            data_dir: path_or(&get, "DEVICE_DATA_DIR", "./data"),
            cache_dir: path_or(&get, "CACHE_DIR", "./cache"),
            cache_max_bytes,
            download_concurrency,
            docker_bin: string_or(&get, "DOCKER_BIN", "docker"),
            ctr_bin: string_or(&get, "CTR_BIN", "ctr"),
            incus_bin,
            virsh_bin: string_or(&get, "VIRSH_BIN", "virsh"),
            libvirt_pool: string_or(&get, "LIBVIRT_POOL", "default"),
            lxc_rootfs_dir: path_or(&get, "LXC_ROOTFS_DIR", "/var/lib/lxc"),
        })
    }
}

fn required<F>(get: &F, key: &str) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    match get(key) {
        Some(value) => Ok(value),
        None => bail!("缺少必填环境变量 {key}"),
    }
}

fn string_or<F>(get: &F, key: &str, default: &str) -> String
where
    F: Fn(&str) -> Option<String>,
{
    get(key).unwrap_or_else(|| default.to_owned())
}

fn path_or<F>(get: &F, key: &str, default: &str) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    PathBuf::from(get(key).unwrap_or_else(|| default.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn minimal() -> Vec<(&'static str, &'static str)> {
        vec![
            ("DEVICE_PKI_MTLS_URL", "https://node:4441"),
            ("DEVICE_SLOT", "slot-1"),
        ]
    }

    #[test]
    fn defaults_apply_when_only_required_are_set() {
        let config = AgentConfig::from_lookup(lookup_from(&minimal())).unwrap();

        assert_eq!(config.mtls_endpoint, "https://node:4441");
        assert_eq!(config.slot, "slot-1");
        assert_eq!(config.data_dir, PathBuf::from("./data"));
        assert_eq!(config.cache_dir, PathBuf::from("./cache"));
        assert_eq!(config.cache_max_bytes, 10_737_418_240);
        assert_eq!(config.cache_max_bytes, DEFAULT_CACHE_MAX_BYTES);
        assert_eq!(config.download_concurrency, 4);
        assert_eq!(config.docker_bin, "docker");
        assert_eq!(config.ctr_bin, "ctr");
        assert_eq!(config.incus_bin, "incus");
        assert_eq!(config.virsh_bin, "virsh");
        assert_eq!(config.libvirt_pool, "default");
        assert_eq!(config.lxc_rootfs_dir, PathBuf::from("/var/lib/lxc"));
    }

    #[test]
    fn overrides_are_parsed() {
        let mut pairs = minimal();
        pairs.extend([
            ("DEVICE_DATA_DIR", "/var/lib/qe"),
            ("CACHE_DIR", "/var/cache/qe"),
            ("CACHE_MAX_BYTES", "1024"),
            ("DOWNLOAD_CONCURRENCY", "8"),
            ("DOCKER_BIN", "/usr/local/bin/docker"),
            ("CTR_BIN", "/usr/local/bin/ctr"),
            ("INCUS_BIN", ""),
            ("VIRSH_BIN", "/usr/bin/virsh"),
            ("LIBVIRT_POOL", "images"),
            ("LXC_ROOTFS_DIR", "/srv/lxc"),
        ]);
        let config = AgentConfig::from_lookup(lookup_from(&pairs)).unwrap();

        assert_eq!(config.data_dir, PathBuf::from("/var/lib/qe"));
        assert_eq!(config.cache_dir, PathBuf::from("/var/cache/qe"));
        assert_eq!(config.cache_max_bytes, 1024);
        assert_eq!(config.download_concurrency, 8);
        assert_eq!(config.docker_bin, "/usr/local/bin/docker");
        assert_eq!(config.ctr_bin, "/usr/local/bin/ctr");
        // 显式置空 = 禁用 Incus（plain LXC 兜底），其余空白值仍视同未设置
        assert_eq!(config.incus_bin, "");
        assert_eq!(config.virsh_bin, "/usr/bin/virsh");
        assert_eq!(config.libvirt_pool, "images");
        assert_eq!(config.lxc_rootfs_dir, PathBuf::from("/srv/lxc"));
    }

    #[test]
    fn missing_mtls_endpoint_is_rejected() {
        let err = AgentConfig::from_lookup(lookup_from(&[("DEVICE_SLOT", "slot-1")]))
            .expect_err("缺少 DEVICE_PKI_MTLS_URL 必须报错");
        assert!(err.to_string().contains("DEVICE_PKI_MTLS_URL"), "{err}");
    }

    #[test]
    fn missing_slot_is_rejected() {
        let err =
            AgentConfig::from_lookup(lookup_from(&[("DEVICE_PKI_MTLS_URL", "https://node:4441")]))
                .expect_err("缺少 DEVICE_SLOT 必须报错");
        assert!(err.to_string().contains("DEVICE_SLOT"), "{err}");
    }

    #[test]
    fn blank_required_value_counts_as_missing() {
        let err = AgentConfig::from_lookup(lookup_from(&[
            ("DEVICE_PKI_MTLS_URL", "   "),
            ("DEVICE_SLOT", "slot-1"),
        ]))
        .expect_err("空白值应视为未设置");
        assert!(err.to_string().contains("DEVICE_PKI_MTLS_URL"), "{err}");
    }

    #[test]
    fn malformed_numbers_are_rejected_with_context() {
        let mut pairs = minimal();
        pairs.push(("CACHE_MAX_BYTES", "10GB"));
        let err = AgentConfig::from_lookup(lookup_from(&pairs)).unwrap_err();
        assert!(err.to_string().contains("CACHE_MAX_BYTES"), "{err}");

        let mut pairs = minimal();
        pairs.push(("DOWNLOAD_CONCURRENCY", "many"));
        let err = AgentConfig::from_lookup(lookup_from(&pairs)).unwrap_err();
        assert!(err.to_string().contains("DOWNLOAD_CONCURRENCY"), "{err}");

        let mut pairs = minimal();
        pairs.push(("DOWNLOAD_CONCURRENCY", "0"));
        let err = AgentConfig::from_lookup(lookup_from(&pairs)).unwrap_err();
        assert!(err.to_string().contains("DOWNLOAD_CONCURRENCY"), "{err}");
    }

    #[test]
    fn mtls_endpoint_must_be_a_url() {
        let err = AgentConfig::from_lookup(lookup_from(&[
            ("DEVICE_PKI_MTLS_URL", "node:4441"),
            ("DEVICE_SLOT", "slot-1"),
        ]))
        .expect_err("缺少 scheme 的端点必须拒绝");
        assert!(err.to_string().contains("http"), "{err}");
    }

    #[test]
    fn values_are_trimmed() {
        let config = AgentConfig::from_lookup(lookup_from(&[
            ("DEVICE_PKI_MTLS_URL", "  https://node:4441  "),
            ("DEVICE_SLOT", " slot-1\t"),
        ]))
        .unwrap();
        assert_eq!(config.mtls_endpoint, "https://node:4441");
        assert_eq!(config.slot, "slot-1");
    }
}
