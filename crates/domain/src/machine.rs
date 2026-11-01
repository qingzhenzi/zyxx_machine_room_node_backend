//! 机器遥测上报契约（v1）：请求体、字段校验与目标快照。
//!
//! 两个入口共用本模块，保证校验口径完全一致：
//! - `qe-api` 的 `POST /api/v1/machine/report`（fleet Bearer token，过渡期）；
//! - `qe-device-pki` mTLS listener 的 `POST /machine/report`（默认入口，身份来自设备证书）。
//!
//! `device_slot` 在 mTLS 入口可以省略（以证书 CN 为准），因此这里用 `Option`；
//! Bearer 入口的 handler 负责强制要求它存在。
//!
//! `tier` 用来区分两档采集：`fast`（指标/健康，10s 级）与 `full`（含外部工具，分钟级）。

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DomainError, DomainResult};

/// 契约版本；不认识的版本直接 400，避免静默丢字段。
pub const SCHEMA_VERSION: i32 = 1;
/// 上报体上限（全局 body limit 之下再做一次语义限制）。
pub const MAX_REPORT_BYTES: usize = 1024 * 1024;
/// 允许的采集时间超前量（客户端时钟偏差）。
const MAX_CLOCK_SKEW_SECS: i64 = 300;

/// 采集档位：`fast` 只跑内置采集；`full` 额外跑外部工具（platform-health 等）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MachineReportTier {
    Fast,
    #[default]
    Full,
}

impl MachineReportTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Full => "full",
        }
    }

    /// 未知值返回 `None`（调用方决定是 400 还是回退默认）。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fast" => Some(Self::Fast),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// 健康结论；`status` 只接受 ok / warning / critical。
#[derive(Debug, Deserialize, Serialize)]
pub struct MachineHealthReport {
    pub status: String,
    #[serde(default)]
    pub score: Option<i32>,
    #[serde(default)]
    pub issues: Option<Value>,
}

/// 列表页冗余指标。
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct MachineMetrics {
    #[serde(default)]
    pub cpu_usage_percent: Option<f64>,
    #[serde(default)]
    pub memory_usage_percent: Option<f64>,
    #[serde(default)]
    pub disk_usage_percent: Option<f64>,
    #[serde(default)]
    pub uptime_secs: Option<i64>,
}

/// `POST /machine/report` 请求体（契约 v1）。
#[derive(Debug, Deserialize, Serialize)]
pub struct MachineReportRequest {
    #[serde(default)]
    pub schema: Option<i32>,
    /// mTLS 入口可省略（身份来自证书），Bearer 入口必填。
    #[serde(default)]
    pub device_slot: Option<String>,
    pub agent_version: String,
    #[serde(default)]
    pub collected_at: Option<DateTime<Utc>>,
    pub hostname: String,
    #[serde(default)]
    pub os_name: Option<String>,
    #[serde(default)]
    pub kernel_version: Option<String>,
    pub health: MachineHealthReport,
    #[serde(default)]
    pub metrics: Option<MachineMetrics>,
    /// 采集档位；缺省 `full`，兼容旧的 room-agent。
    #[serde(default)]
    pub tier: Option<MachineReportTier>,
    /// 各采集域的原始 JSON（platform-health / diagnose / security / containers / vms / agent）。
    #[serde(default)]
    pub sections: Option<Value>,
}

/// 校验并规范化后的上报，交给仓储层落库。
#[derive(Debug, Clone)]
pub struct ValidatedMachineReport {
    pub device_slot: Option<String>,
    pub tier: MachineReportTier,
    pub collected_at: DateTime<Utc>,
    pub agent_version: String,
    pub hostname: String,
    pub os_name: Option<String>,
    pub kernel_version: Option<String>,
    pub health_status: String,
    pub health_score: Option<i32>,
    pub cpu_usage_percent: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub disk_usage_percent: Option<f64>,
    pub uptime_secs: Option<i64>,
    /// 完整上报 JSON（原样存档；调用方可补上解析后的 device_slot）。
    pub payload: Value,
}

impl MachineReportRequest {
    /// 校验契约版本、字段范围、时钟偏差与体积，并生成落库用的扁平字段。
    pub fn validate(self) -> DomainResult<ValidatedMachineReport> {
        if self.schema.unwrap_or(SCHEMA_VERSION) != SCHEMA_VERSION {
            return Err(DomainError::bad_request(format!(
                "不支持的 schema 版本，期望 {SCHEMA_VERSION}"
            )));
        }

        let device_slot = self
            .device_slot
            .as_deref()
            .map(|raw| normalize_text(raw, "device_slot", 128))
            .transpose()?;
        let agent_version = normalize_text(&self.agent_version, "agent_version", 64)?;
        let hostname = normalize_text(&self.hostname, "hostname", 255)?;
        let os_name = normalize_optional(self.os_name.clone(), 255);
        let kernel_version = normalize_optional(self.kernel_version.clone(), 128);

        let health_status = self.health.status.trim().to_ascii_lowercase();
        if !matches!(health_status.as_str(), "ok" | "warning" | "critical") {
            return Err(DomainError::bad_request(
                "health.status 只能是 ok / warning / critical",
            ));
        }
        let health_score = match self.health.score {
            None => None,
            Some(score) if (0..=100).contains(&score) => Some(score),
            Some(_) => {
                return Err(DomainError::bad_request("health.score 需为 0-100 的整数"));
            }
        };

        let metrics = self.metrics.as_ref();
        let cpu_usage_percent = validate_percent(
            metrics.and_then(|metrics| metrics.cpu_usage_percent),
            "metrics.cpu_usage_percent",
        )?;
        let memory_usage_percent = validate_percent(
            metrics.and_then(|metrics| metrics.memory_usage_percent),
            "metrics.memory_usage_percent",
        )?;
        let disk_usage_percent = validate_percent(
            metrics.and_then(|metrics| metrics.disk_usage_percent),
            "metrics.disk_usage_percent",
        )?;
        let uptime_secs = metrics
            .and_then(|metrics| metrics.uptime_secs)
            .map(|value| {
                if value < 0 {
                    Err(DomainError::bad_request("metrics.uptime_secs 不能为负数"))
                } else {
                    Ok(value)
                }
            })
            .transpose()?;

        let now = Utc::now();
        let collected_at = self.collected_at.unwrap_or(now);
        if collected_at > now + Duration::seconds(MAX_CLOCK_SKEW_SECS) {
            return Err(DomainError::bad_request(
                "collected_at 超出允许的时钟偏差（未来 5 分钟以上）",
            ));
        }

        let tier = self.tier.unwrap_or_default();
        let payload = serde_json::to_value(&self)
            .map_err(|error| DomainError::bad_request(format!("上报体不是合法 JSON: {error}")))?;
        if serde_json::to_vec(&payload)
            .map(|bytes| bytes.len())
            .unwrap_or(0)
            > MAX_REPORT_BYTES
        {
            return Err(DomainError::bad_request("上报体超过 1 MiB 限制"));
        }

        Ok(ValidatedMachineReport {
            device_slot,
            tier,
            collected_at,
            agent_version,
            hostname,
            os_name,
            kernel_version,
            health_status,
            health_score,
            cpu_usage_percent,
            memory_usage_percent,
            disk_usage_percent,
            uptime_secs,
            payload,
        })
    }
}

fn normalize_text(value: &str, label: &str, max_len: usize) -> DomainResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::bad_request(format!("{label}不能为空")));
    }
    if trimmed.chars().count() > max_len {
        return Err(DomainError::bad_request(format!(
            "{label}长度不能超过 {max_len} 个字符"
        )));
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional(value: Option<String>, max_len: usize) -> Option<String> {
    value
        .map(|raw| raw.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(max_len).collect())
}

fn validate_percent(value: Option<f64>, label: &str) -> DomainResult<Option<f64>> {
    match value {
        None => Ok(None),
        Some(value) if (0.0..=100.0).contains(&value) && value.is_finite() => Ok(Some(value)),
        Some(_) => Err(DomainError::bad_request(format!(
            "{label}需为 0-100 的百分比"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(value: serde_json::Value) -> MachineReportRequest {
        serde_json::from_value(value).expect("请求体")
    }

    fn minimal() -> serde_json::Value {
        json!({
            "schema": 1,
            "device_slot": "room-101",
            "agent_version": "0.1.0",
            "hostname": "node-1",
            "health": { "status": "ok" }
        })
    }

    #[test]
    fn percent_validation_rejects_out_of_range_and_nan() {
        assert_eq!(validate_percent(None, "cpu").unwrap(), None);
        assert_eq!(validate_percent(Some(0.0), "cpu").unwrap(), Some(0.0));
        assert_eq!(validate_percent(Some(100.0), "cpu").unwrap(), Some(100.0));
        assert!(validate_percent(Some(-1.0), "cpu").is_err());
        assert!(validate_percent(Some(101.0), "cpu").is_err());
        assert!(validate_percent(Some(f64::NAN), "cpu").is_err());
        assert!(validate_percent(Some(f64::INFINITY), "cpu").is_err());
    }

    #[test]
    fn text_normalization_trims_and_limits() {
        assert_eq!(
            normalize_text("  room-101  ", "device_slot", 128).unwrap(),
            "room-101"
        );
        assert!(normalize_text("   ", "device_slot", 128).is_err());
        assert!(normalize_text(&"x".repeat(129), "device_slot", 128).is_err());
        assert_eq!(normalize_optional(Some("  ".to_owned()), 32), None);
        assert_eq!(
            normalize_optional(Some(" Debian 13 ".to_owned()), 32).unwrap(),
            "Debian 13"
        );
    }

    #[test]
    fn tier_defaults_to_full_and_parses_known_values() {
        assert_eq!(
            MachineReportTier::parse("FAST"),
            Some(MachineReportTier::Fast)
        );
        assert_eq!(
            MachineReportTier::parse("full"),
            Some(MachineReportTier::Full)
        );
        assert_eq!(MachineReportTier::parse("slow"), None);
        assert_eq!(MachineReportTier::default(), MachineReportTier::Full);
    }

    #[test]
    fn validate_accepts_minimal_without_slot_and_defaults_tier() {
        let validated = request(minimal()).validate().expect("validate");
        assert_eq!(validated.device_slot.as_deref(), Some("room-101"));
        assert_eq!(validated.tier, MachineReportTier::Full);
        assert!(validated.health_score.is_none());
        assert!(validated.payload.is_object());
    }

    #[test]
    fn validate_rejects_unknown_schema_and_bad_health() {
        let mut body = minimal();
        body["schema"] = json!(99);
        assert!(request(body).validate().is_err());

        let mut body = minimal();
        body["health"]["status"] = json!("bad");
        assert!(request(body).validate().is_err());
    }

    #[test]
    fn validate_rejects_future_collected_at_and_negative_uptime() {
        let mut body = minimal();
        body["collected_at"] = json!((Utc::now() + Duration::hours(1)).to_rfc3339());
        assert!(request(body).validate().is_err());

        let mut body = minimal();
        body["metrics"] = json!({ "uptime_secs": -1 });
        assert!(request(body).validate().is_err());
    }

    #[test]
    fn validate_rejects_oversized_payload() {
        let mut body = minimal();
        body["sections"] = json!({ "huge": "x".repeat(MAX_REPORT_BYTES) });
        assert!(request(body).validate().is_err());
    }
}
