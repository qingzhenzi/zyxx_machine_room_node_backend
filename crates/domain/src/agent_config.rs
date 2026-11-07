//! P2 节点配置管理的领域模型与校验。
//!
//! 契约见 `docs/storage-design.md` §17.1 与 `docs/machine-telemetry.md` v1.1。配置按
//! 作用域存储增量 patch：**global → area → room → device** 依次合并；device 作用域
//! 用设备槽位（`scope_key`）作为 key，一机一行覆盖。本地专属字段（mTLS 端点、slot、
//! 路径、可执行文件、libvirt pool）永不接受远端覆盖，因此不在 [`AgentPolicy`] 里。

use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{DomainError, DomainResult};

/// 轮询间隔默认值（秒）。
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 2;
/// 轮询间隔取值范围（秒）。
pub const POLL_INTERVAL_SECS_RANGE: std::ops::RangeInclusive<u64> = 1..=60;
/// 下载并发默认值。
pub const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 4;
/// 下载并发取值范围。
pub const DOWNLOAD_CONCURRENCY_RANGE: std::ops::RangeInclusive<usize> = 1..=32;
/// 缓存上限默认值：10 GiB。
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// 缓存上限最大值：1 TiB。
pub const MAX_CACHE_MAX_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
/// 机器遥测 fast 档上报间隔默认值（秒）。
pub const DEFAULT_TELEMETRY_FAST_INTERVAL_SECS: u64 = 10;
/// 机器遥测 fast 档上报间隔范围（秒）。
pub const TELEMETRY_FAST_INTERVAL_SECS_RANGE: std::ops::RangeInclusive<u64> = 1..=60;
/// 机器遥测 full 档上报间隔默认值（秒）。
pub const DEFAULT_TELEMETRY_FULL_INTERVAL_SECS: u64 = 300;
/// 机器遥测 full 档上报间隔范围（秒）。
pub const TELEMETRY_FULL_INTERVAL_SECS_RANGE: std::ops::RangeInclusive<u64> = 60..=3600;

/// 节点运行时策略（合并后的完整策略）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPolicy {
    pub lxc_auto_start: bool,
    pub poll_interval_secs: u64,
    pub download_concurrency: usize,
    pub cache_max_bytes: u64,
    /// 机器遥测 fast 档间隔（秒）：只采内置指标，不含外部工具。
    pub telemetry_fast_interval_secs: u64,
    /// 机器遥测 full 档间隔（秒）：额外包含 platform-health / diagnose / qeaudit 等。
    pub telemetry_full_interval_secs: u64,
}

impl Default for AgentPolicy {
    fn default() -> Self {
        Self {
            lxc_auto_start: false,
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            download_concurrency: DEFAULT_DOWNLOAD_CONCURRENCY,
            cache_max_bytes: DEFAULT_CACHE_MAX_BYTES,
            telemetry_fast_interval_secs: DEFAULT_TELEMETRY_FAST_INTERVAL_SECS,
            telemetry_full_interval_secs: DEFAULT_TELEMETRY_FULL_INTERVAL_SECS,
        }
    }
}

impl AgentPolicy {
    /// 校验完整策略的所有字段范围（含 `full >= fast`）。
    ///
    /// 与 [`validate_policy_patch`] 的区别：patch 只校验显式字段，完整策略还要
    /// 保证 fast/full 两档之间的大小关系（否则远端策略下发后会出现 fast 比 full 还慢）。
    pub fn validate(&self) -> DomainResult<()> {
        if !POLL_INTERVAL_SECS_RANGE.contains(&self.poll_interval_secs) {
            return Err(DomainError::bad_request(format!(
                "poll_interval_secs 必须在 {}..={} 之间（当前 {}）",
                POLL_INTERVAL_SECS_RANGE.start(),
                POLL_INTERVAL_SECS_RANGE.end(),
                self.poll_interval_secs
            )));
        }
        if !DOWNLOAD_CONCURRENCY_RANGE.contains(&self.download_concurrency) {
            return Err(DomainError::bad_request(format!(
                "download_concurrency 必须在 {}..={} 之间（当前 {}）",
                DOWNLOAD_CONCURRENCY_RANGE.start(),
                DOWNLOAD_CONCURRENCY_RANGE.end(),
                self.download_concurrency
            )));
        }
        if self.cache_max_bytes > MAX_CACHE_MAX_BYTES {
            return Err(DomainError::bad_request(format!(
                "cache_max_bytes 必须在 0..={MAX_CACHE_MAX_BYTES} 之间（当前 {}）",
                self.cache_max_bytes
            )));
        }
        validate_telemetry_ranges(
            self.telemetry_fast_interval_secs,
            self.telemetry_full_interval_secs,
        )
    }
}

/// 某个作用域保存的增量 patch；`None` 表示该字段不覆盖下层配置。
///
/// 数据库 `agent_configs.policy` 以 JSONB 保存它的序列化结果，而不是完整
/// [`AgentPolicy`] —— 这样未显式设置的字段（含未来新增字段）可以继续继承。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPolicyPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lxc_auto_start: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_concurrency: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_max_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_fast_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_full_interval_secs: Option<u64>,
}

impl AgentPolicyPatch {
    /// 把 patch 中已设置的字段写入 `policy`（未设置的字段保持不变）。
    pub fn apply_to(&self, policy: &mut AgentPolicy) {
        if let Some(value) = self.lxc_auto_start {
            policy.lxc_auto_start = value;
        }
        if let Some(value) = self.poll_interval_secs {
            policy.poll_interval_secs = value;
        }
        if let Some(value) = self.download_concurrency {
            policy.download_concurrency = value;
        }
        if let Some(value) = self.cache_max_bytes {
            policy.cache_max_bytes = value;
        }
        if let Some(value) = self.telemetry_fast_interval_secs {
            policy.telemetry_fast_interval_secs = value;
        }
        if let Some(value) = self.telemetry_full_interval_secs {
            policy.telemetry_full_interval_secs = value;
        }
    }

    /// 以 `base` 为底应用本 patch，返回新策略。
    pub fn applied(&self, mut base: AgentPolicy) -> AgentPolicy {
        self.apply_to(&mut base);
        base
    }
}

/// 配置作用域，对应 `agent_configs.scope_type`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigScopeType {
    Global,
    Area,
    Room,
    /// 设备级覆盖：`scope_key` 为 `device_slot`，只作用于该机器。
    Device,
}

impl ConfigScopeType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Area => "area",
            Self::Room => "room",
            Self::Device => "device",
        }
    }
}

impl FromStr for ConfigScopeType {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "global" => Ok(Self::Global),
            "area" => Ok(Self::Area),
            "room" => Ok(Self::Room),
            "device" => Ok(Self::Device),
            _ => Err(()),
        }
    }
}

/// 某个作用域当前保存的配置行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: Uuid,
    pub scope_type: ConfigScopeType,
    /// area / room 作用域的目标 UUID；global / device 为 NULL。
    pub scope_id: Option<Uuid>,
    /// device 作用域的 key（`device_slot`）；其余作用域为 NULL。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_key: Option<String>,
    /// 增量 patch，数据库以 JSONB 保存。
    pub policy: AgentPolicyPatch,
    pub revision: i64,
    pub updated_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 某房间合并后的生效策略与已应用配置行的最大 revision（无行时为 0）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveAgentPolicy {
    pub policy: AgentPolicy,
    pub revision: i64,
}

/// 校验 patch 中各字段范围；中文错误信息。
///
/// 未设置（`None`）的字段不校验；结果保证可安全写入 `agent_configs`。
pub fn validate_policy_patch(patch: &AgentPolicyPatch) -> DomainResult<()> {
    if let Some(value) = patch.poll_interval_secs
        && !POLL_INTERVAL_SECS_RANGE.contains(&value)
    {
        return Err(DomainError::bad_request(format!(
            "poll_interval_secs 必须在 {}..={} 之间（当前 {value}）",
            POLL_INTERVAL_SECS_RANGE.start(),
            POLL_INTERVAL_SECS_RANGE.end()
        )));
    }
    if let Some(value) = patch.download_concurrency
        && !DOWNLOAD_CONCURRENCY_RANGE.contains(&value)
    {
        return Err(DomainError::bad_request(format!(
            "download_concurrency 必须在 {}..={} 之间（当前 {value}）",
            DOWNLOAD_CONCURRENCY_RANGE.start(),
            DOWNLOAD_CONCURRENCY_RANGE.end()
        )));
    }
    if let Some(value) = patch.cache_max_bytes
        && value > MAX_CACHE_MAX_BYTES
    {
        return Err(DomainError::bad_request(format!(
            "cache_max_bytes 必须在 0..={MAX_CACHE_MAX_BYTES} 之间（当前 {value}）"
        )));
    }
    if let Some(value) = patch.telemetry_fast_interval_secs
        && !TELEMETRY_FAST_INTERVAL_SECS_RANGE.contains(&value)
    {
        return Err(DomainError::bad_request(format!(
            "telemetry_fast_interval_secs 必须在 {}..={} 之间（当前 {value}）",
            TELEMETRY_FAST_INTERVAL_SECS_RANGE.start(),
            TELEMETRY_FAST_INTERVAL_SECS_RANGE.end()
        )));
    }
    if let Some(value) = patch.telemetry_full_interval_secs
        && !TELEMETRY_FULL_INTERVAL_SECS_RANGE.contains(&value)
    {
        return Err(DomainError::bad_request(format!(
            "telemetry_full_interval_secs 必须在 {}..={} 之间（当前 {value}）",
            TELEMETRY_FULL_INTERVAL_SECS_RANGE.start(),
            TELEMETRY_FULL_INTERVAL_SECS_RANGE.end()
        )));
    }
    // 同一 patch 同时改两档时立刻校验顺序；只改一档时无法判断（另一档来自下层，
    // 可能出现短暂非法状态），完整顺序由 `AgentPolicy::validate` 在合并后兜底。
    if let (Some(fast), Some(full)) = (
        patch.telemetry_fast_interval_secs,
        patch.telemetry_full_interval_secs,
    ) {
        validate_telemetry_intervals(fast, full)?;
    }
    Ok(())
}

/// 校验 fast/full 两档间隔范围与顺序（完整策略用）。
fn validate_telemetry_ranges(fast: u64, full: u64) -> DomainResult<()> {
    if !TELEMETRY_FAST_INTERVAL_SECS_RANGE.contains(&fast) {
        return Err(DomainError::bad_request(format!(
            "telemetry_fast_interval_secs 必须在 {}..={} 之间（当前 {fast}）",
            TELEMETRY_FAST_INTERVAL_SECS_RANGE.start(),
            TELEMETRY_FAST_INTERVAL_SECS_RANGE.end()
        )));
    }
    if !TELEMETRY_FULL_INTERVAL_SECS_RANGE.contains(&full) {
        return Err(DomainError::bad_request(format!(
            "telemetry_full_interval_secs 必须在 {}..={} 之间（当前 {full}）",
            TELEMETRY_FULL_INTERVAL_SECS_RANGE.start(),
            TELEMETRY_FULL_INTERVAL_SECS_RANGE.end()
        )));
    }
    validate_telemetry_intervals(fast, full)
}

/// 校验 fast/full 两档间隔顺序：full 不能小于 fast。
fn validate_telemetry_intervals(fast: u64, full: u64) -> DomainResult<()> {
    if full < fast {
        return Err(DomainError::bad_request(format!(
            "telemetry_full_interval_secs（{full}）不能小于 telemetry_fast_interval_secs（{fast}）"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_matches_contract() {
        let policy = AgentPolicy::default();
        assert!(!policy.lxc_auto_start);
        assert_eq!(policy.poll_interval_secs, 2);
        assert_eq!(policy.download_concurrency, 4);
        assert_eq!(policy.cache_max_bytes, 10 * 1024 * 1024 * 1024);
        assert_eq!(policy.telemetry_fast_interval_secs, 10);
        assert_eq!(policy.telemetry_full_interval_secs, 300);
    }

    #[test]
    fn patch_only_overrides_present_fields() {
        let mut policy = AgentPolicy::default();
        let patch = AgentPolicyPatch {
            lxc_auto_start: Some(true),
            poll_interval_secs: Some(9),
            download_concurrency: None,
            cache_max_bytes: None,
            telemetry_fast_interval_secs: Some(20),
            telemetry_full_interval_secs: None,
        };
        patch.apply_to(&mut policy);
        assert!(policy.lxc_auto_start);
        assert_eq!(policy.poll_interval_secs, 9);
        assert_eq!(policy.download_concurrency, DEFAULT_DOWNLOAD_CONCURRENCY);
        assert_eq!(policy.cache_max_bytes, DEFAULT_CACHE_MAX_BYTES);
        assert_eq!(policy.telemetry_fast_interval_secs, 20);
        assert_eq!(
            policy.telemetry_full_interval_secs,
            DEFAULT_TELEMETRY_FULL_INTERVAL_SECS
        );

        let applied = patch.applied(AgentPolicy::default());
        assert_eq!(applied, policy);
    }

    #[test]
    fn patch_deserializes_from_partial_json() {
        let patch: AgentPolicyPatch = serde_json::from_str(r#"{"poll_interval_secs":7}"#).unwrap();
        assert_eq!(patch.poll_interval_secs, Some(7));
        assert_eq!(patch.lxc_auto_start, None);

        let empty: AgentPolicyPatch = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.poll_interval_secs, None);
        assert_eq!(
            serde_json::to_value(&empty).unwrap(),
            serde_json::json!({}),
            "空 patch 不应把 None 序列化进 JSONB"
        );
    }

    #[test]
    fn policy_deserializes_with_defaults() {
        let policy: AgentPolicy = serde_json::from_str(r#"{"lxc_auto_start":true}"#).unwrap();
        assert!(policy.lxc_auto_start);
        assert_eq!(policy.poll_interval_secs, DEFAULT_POLL_INTERVAL_SECS);
    }

    #[test]
    fn validate_accepts_boundary_values() {
        assert!(
            validate_policy_patch(&AgentPolicyPatch {
                poll_interval_secs: Some(1),
                download_concurrency: Some(32),
                cache_max_bytes: Some(0),
                telemetry_fast_interval_secs: Some(1),
                telemetry_full_interval_secs: Some(60),
                ..Default::default()
            })
            .is_ok()
        );
        assert!(
            validate_policy_patch(&AgentPolicyPatch {
                poll_interval_secs: Some(60),
                download_concurrency: Some(1),
                cache_max_bytes: Some(MAX_CACHE_MAX_BYTES),
                telemetry_fast_interval_secs: Some(60),
                telemetry_full_interval_secs: Some(3600),
                ..Default::default()
            })
            .is_ok()
        );
        assert!(validate_policy_patch(&AgentPolicyPatch::default()).is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range_values() {
        for patch in [
            AgentPolicyPatch {
                poll_interval_secs: Some(0),
                ..Default::default()
            },
            AgentPolicyPatch {
                poll_interval_secs: Some(61),
                ..Default::default()
            },
            AgentPolicyPatch {
                download_concurrency: Some(0),
                ..Default::default()
            },
            AgentPolicyPatch {
                download_concurrency: Some(33),
                ..Default::default()
            },
            AgentPolicyPatch {
                cache_max_bytes: Some(MAX_CACHE_MAX_BYTES + 1),
                ..Default::default()
            },
            AgentPolicyPatch {
                telemetry_fast_interval_secs: Some(0),
                ..Default::default()
            },
            AgentPolicyPatch {
                telemetry_fast_interval_secs: Some(61),
                ..Default::default()
            },
            AgentPolicyPatch {
                telemetry_full_interval_secs: Some(59),
                ..Default::default()
            },
            AgentPolicyPatch {
                telemetry_full_interval_secs: Some(3601),
                ..Default::default()
            },
        ] {
            assert!(validate_policy_patch(&patch).is_err());
        }
    }

    #[test]
    fn validate_rejects_full_slower_than_fast() {
        // 当前范围（fast ≤ 60 ≤ full）下 full < fast 不可达，但顺序校验仍作为兜底保留。
        assert!(validate_telemetry_intervals(60, 60).is_ok());
        assert!(validate_telemetry_intervals(10, 300).is_ok());
        let error = validate_telemetry_intervals(60, 59).unwrap_err();
        assert!(error.to_string().contains("不能小于"), "{error}");

        // patch 同时设置两档时 60/60 合法（边界）；full=59 先被范围校验拦截。
        let patch = AgentPolicyPatch {
            telemetry_fast_interval_secs: Some(60),
            telemetry_full_interval_secs: Some(60),
            ..Default::default()
        };
        assert!(validate_policy_patch(&patch).is_ok());
        let patch = AgentPolicyPatch {
            telemetry_fast_interval_secs: Some(60),
            telemetry_full_interval_secs: Some(59),
            ..Default::default()
        };
        assert!(validate_policy_patch(&patch).is_err());
    }

    #[test]
    fn complete_policy_validate_checks_order_and_ranges() {
        assert!(AgentPolicy::default().validate().is_ok());

        let mut policy = AgentPolicy {
            telemetry_fast_interval_secs: 60,
            telemetry_full_interval_secs: 60,
            ..Default::default()
        };
        assert!(policy.validate().is_ok());

        // 越界（同时顺序非法）必须被拒绝
        policy.telemetry_full_interval_secs = 59;
        assert!(policy.validate().is_err());

        policy.telemetry_full_interval_secs = 300;
        policy.telemetry_fast_interval_secs = 0;
        assert!(policy.validate().is_err());
    }

    #[test]
    fn scope_type_roundtrip() {
        for scope in [
            ConfigScopeType::Global,
            ConfigScopeType::Area,
            ConfigScopeType::Room,
            ConfigScopeType::Device,
        ] {
            assert_eq!(scope.as_str().parse::<ConfigScopeType>(), Ok(scope));
        }
        assert_eq!(
            serde_json::to_string(&ConfigScopeType::Room).unwrap(),
            "\"room\""
        );
        assert_eq!(
            serde_json::to_string(&ConfigScopeType::Device).unwrap(),
            "\"device\""
        );
        assert!("unknown".parse::<ConfigScopeType>().is_err());
    }
}
