//! 可选外部工具采集：platform-health / platform-diagnose / qeaudit / docker / virsh。
//!
//! 每个工具独立调用、独立超时（默认 15s，可用 `ROOM_AGENT_TOOL_TIMEOUT_SECS` 调整）；
//! 失败/超时只在 `sections.agent.tools.<name>` 里留下
//! `{"available": false, "error": ...}`，绝不向上返回错误，保证内置指标照常上报。
//!
//! 原始 JSON 尽量原样存档（`sections.platform_health` / `diagnose` / `security`），
//! docker/virsh 的文本输出会规整为契约示例中的数组形状。
//!
//! **退出码约定**：`platform-diagnose` / `qeaudit` 用非 0 退出码表达「发现告警/故障」
//! （如 qeaudit 默认 `--fail-on fail` 有 findings 时返回 2）。因此 JSON 类工具只要
//! stdout 能解析成 JSON 就采信数据，并把退出码记录到 `tools.<name>.exit_code`，
//! 不把「有 findings」误判成「工具不可用」。docker/virsh 仍按非 0 退出码判失败。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::collect::disk::COMMAND_TIMEOUT;
use crate::config::Config;
use crate::process::{self, CommandOutput};

/// 外部工具默认超时（15s）。
pub const TOOL_TIMEOUT: Duration = COMMAND_TIMEOUT;

/// 单个外部工具的可用性/耗时/版本，序列化进 `sections.agent.tools`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolStatus {
    /// 是否成功采集。
    pub available: bool,
    /// 是否被 `ROOM_AGENT_DISABLE_*` 主动关闭（关闭时不带 error）。
    #[serde(skip_serializing_if = "is_false")]
    pub disabled: bool,
    /// 是否因当前采集档位（fast）主动跳过。
    #[serde(skip_serializing_if = "is_false")]
    pub skipped: bool,
    /// 命令耗时（毫秒）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 进程退出码（0 不记录；非 0 用于区分「有 findings」与真正失败）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// 工具版本/JSON schema（能从输出里识别时记录）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 失败原因（超时、非 0 退出码、输出无法解析）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 本次数据复用自缓存（重工具按间隔刷新，不每轮都跑）。
    #[serde(skip_serializing_if = "is_false")]
    pub cached: bool,
    /// 缓存年龄（秒），仅在 `cached = true` 时出现。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_secs: Option<u64>,
}

impl ToolStatus {
    fn success(duration: Duration) -> Self {
        Self {
            available: true,
            disabled: false,
            skipped: false,
            duration_ms: Some(duration.as_millis() as u64),
            exit_code: None,
            version: None,
            error: None,
            cached: false,
            age_secs: None,
        }
    }

    /// 采集成功但进程以非 0 退出（findings 语义），记录退出码。
    fn success_with_exit(duration: Duration, exit_code: Option<i32>) -> Self {
        let mut status = Self::success(duration);
        status.exit_code = exit_code.filter(|code| *code != 0);
        status
    }

    fn disabled() -> Self {
        Self {
            available: false,
            disabled: true,
            skipped: false,
            duration_ms: None,
            exit_code: None,
            version: None,
            error: None,
            cached: false,
            age_secs: None,
        }
    }

    /// fast 档主动跳过的外部工具（不启动子进程）。
    fn skipped() -> Self {
        Self {
            available: false,
            disabled: false,
            skipped: true,
            duration_ms: None,
            exit_code: None,
            version: None,
            error: None,
            cached: false,
            age_secs: None,
        }
    }

    fn failure(duration: Duration, error: impl Into<String>) -> Self {
        Self {
            available: false,
            disabled: false,
            skipped: false,
            duration_ms: Some(duration.as_millis() as u64),
            exit_code: None,
            version: None,
            error: Some(error.into()),
            cached: false,
            age_secs: None,
        }
    }

    /// 标记为复用缓存，记录缓存年龄。
    fn with_cached(mut self, age: Duration) -> Self {
        self.cached = true;
        self.age_secs = Some(age.as_secs());
        self
    }

    fn with_version(mut self, version: Option<String>) -> Self {
        self.version = version;
        self
    }

    fn with_exit_code(mut self, exit_code: Option<i32>) -> Self {
        self.exit_code = exit_code.filter(|code| *code != 0);
        self
    }

    fn failed_with(mut self, error: impl Into<String>) -> Self {
        self.available = false;
        self.error = Some(error.into());
        self
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// 一次采集得到的全部外部工具结果。
#[derive(Debug, Default)]
pub struct ToolsOutcome {
    /// platform-health 原始 JSON。
    pub platform_health: Option<Value>,
    /// platform-diagnose 原始 JSON。
    pub diagnose: Option<Value>,
    /// qeaudit 原始 JSON。
    pub security: Option<Value>,
    /// docker 容器列表。
    pub containers: Option<Vec<Value>>,
    /// virsh 虚拟机列表。
    pub vms: Option<Vec<Value>>,
    /// 五个工具的可用性状态（key 与 sections 一一对应）。
    pub statuses: BTreeMap<String, ToolStatus>,
}

/// 重工具（diagnose / qeaudit）的内存缓存，daemon 跨轮复用。
#[derive(Debug, Default)]
pub struct ToolsCache {
    diagnose: Option<CachedTool>,
    security: Option<CachedTool>,
}

#[derive(Debug, Clone)]
struct CachedTool {
    value: Value,
    status: ToolStatus,
    at: Instant,
}

/// 采集全部外部工具（每次全新执行，`--once` / 测试用）。
pub fn collect(config: &Config) -> ToolsOutcome {
    let mut cache = ToolsCache::default();
    collect_with_cache(config, &mut cache)
}

/// fast 档：**不启动任何外部工具**，只在 `sections.agent.tools` 里记录全部被跳过。
///
/// 返回的 outcome 各 section 为空，调用方据此组装与 full 档同构但不含重工具数据的 payload。
pub fn skipped_outcome() -> ToolsOutcome {
    let mut outcome = ToolsOutcome::default();
    for name in [
        "platform_health",
        "platform_diagnose",
        "qeaudit",
        "docker",
        "virsh",
    ] {
        outcome
            .statuses
            .insert(name.to_owned(), ToolStatus::skipped());
    }
    outcome
}

/// 采集全部外部工具；diagnose / qeaudit 命中缓存时直接复用，
/// 避免每轮都跑 4.5s / 20s+ 的重命令，从而把快照周期压到 interval 量级。
///
/// 任何单项失败都不影响其余项。
pub fn collect_with_cache(config: &Config, cache: &mut ToolsCache) -> ToolsOutcome {
    let timeout = Duration::from_secs(config.tool_timeout_secs);
    let mut outcome = ToolsOutcome::default();

    let (status, value) = json_tool(
        config.disable_health,
        &config.platform_health_bin,
        &config.platform_health_args,
        timeout,
    );
    outcome
        .statuses
        .insert("platform_health".to_owned(), status);
    outcome.platform_health = value;

    let (status, value) = json_tool_cached(
        config.disable_diagnose,
        &config.platform_diagnose_bin,
        &config.platform_diagnose_args,
        timeout,
        config.diagnose_interval_secs,
        &mut cache.diagnose,
    );
    outcome
        .statuses
        .insert("platform_diagnose".to_owned(), status);
    outcome.diagnose = value;

    let (status, value) = json_tool_cached(
        config.disable_security,
        &config.qeaudit_bin,
        &config.qeaudit_args,
        timeout,
        config.security_interval_secs,
        &mut cache.security,
    );
    outcome.statuses.insert("qeaudit".to_owned(), status);
    outcome.security = value;

    let docker_args = vec![
        "ps".to_owned(),
        "--format".to_owned(),
        "{{json .}}".to_owned(),
    ];
    let (status, text) = raw_tool(
        config.disable_docker,
        &config.docker_bin,
        &docker_args,
        timeout,
    );
    match text {
        None => {
            outcome.statuses.insert("docker".to_owned(), status);
        }
        Some(text) => match parse_docker_ps(&text) {
            Ok(containers) => {
                outcome.containers = Some(containers);
                outcome.statuses.insert("docker".to_owned(), status);
            }
            Err(error) => {
                outcome
                    .statuses
                    .insert("docker".to_owned(), status.failed_with(error));
            }
        },
    }

    let virsh_args = vec!["list".to_owned(), "--all".to_owned()];
    let (status, text) = raw_tool(config.disable_vms, &config.virsh_bin, &virsh_args, timeout);
    match text {
        None => {
            outcome.statuses.insert("virsh".to_owned(), status);
        }
        Some(text) => {
            let vms = parse_virsh_list(&text);
            outcome.vms = Some(vms);
            outcome.statuses.insert("virsh".to_owned(), status);
        }
    }

    outcome
}

/// 工具执行的三态结果。
enum ToolRun {
    /// 被配置关闭。
    Disabled,
    /// 启动失败或超时（对应工具不可用）。
    Failed(ToolStatus),
    /// 正常拿到进程输出（退出码可能非 0）。
    Output(CommandOutput),
}

/// 运行一个「输出 JSON」的工具。
fn json_tool(
    disabled: bool,
    bin: &str,
    args: &[String],
    timeout: Duration,
) -> (ToolStatus, Option<Value>) {
    match execute(disabled, bin, args, timeout) {
        ToolRun::Disabled => (ToolStatus::disabled(), None),
        ToolRun::Failed(status) => (status, None),
        ToolRun::Output(output) => {
            let duration = output.duration;
            match serde_json::from_str::<Value>(output.stdout.trim()) {
                Ok(value) => {
                    let status = if output.success {
                        ToolStatus::success(duration)
                    } else {
                        // platform-diagnose / qeaudit 用非 0 退出码表达 findings。
                        ToolStatus::success_with_exit(duration, output.code)
                    };
                    let version = tool_version(&value);
                    (status.with_version(version), Some(value))
                }
                Err(json_error) => {
                    if output.success {
                        (
                            ToolStatus::success(duration)
                                .failed_with(format!("输出不是合法 JSON: {json_error}")),
                            None,
                        )
                    } else {
                        // 同时保留退出码/stderr 信息与 JSON 解析错误，便于排障。
                        let status = exit_failure(&output);
                        let mut error = status.error.clone().unwrap_or_default();
                        error.push_str(&format!("；输出不是合法 JSON: {json_error}"));
                        (status.failed_with(error), None)
                    }
                }
            }
        }
    }
}

/// 带缓存的 JSON 工具：缓存年龄小于 `refresh_secs`（且 >0）时直接复用上次结果。
fn json_tool_cached(
    disabled: bool,
    bin: &str,
    args: &[String],
    timeout: Duration,
    refresh_secs: u64,
    cache: &mut Option<CachedTool>,
) -> (ToolStatus, Option<Value>) {
    if disabled {
        return (ToolStatus::disabled(), None);
    }
    if refresh_secs > 0
        && let Some(entry) = cache.as_ref()
    {
        let age = entry.at.elapsed();
        if age < Duration::from_secs(refresh_secs) {
            return (
                entry.status.clone().with_cached(age),
                Some(entry.value.clone()),
            );
        }
    }

    let (status, value) = json_tool(false, bin, args, timeout);
    if let Some(value) = value.as_ref() {
        *cache = Some(CachedTool {
            value: value.clone(),
            status: status.clone(),
            at: Instant::now(),
        });
    }
    (status, value)
}

/// 运行一个「输出文本」的工具；非 0 退出码视为失败。
fn raw_tool(
    disabled: bool,
    bin: &str,
    args: &[String],
    timeout: Duration,
) -> (ToolStatus, Option<String>) {
    match execute(disabled, bin, args, timeout) {
        ToolRun::Disabled => (ToolStatus::disabled(), None),
        ToolRun::Failed(status) => (status, None),
        ToolRun::Output(output) => {
            if output.success {
                (ToolStatus::success(output.duration), Some(output.stdout))
            } else {
                (exit_failure(&output), None)
            }
        }
    }
}

fn execute(disabled: bool, bin: &str, args: &[String], timeout: Duration) -> ToolRun {
    if disabled {
        return ToolRun::Disabled;
    }
    match process::run_with_timeout(bin, args, timeout) {
        Err(error) => ToolRun::Failed(ToolStatus::failure(Duration::ZERO, error)),
        Ok(output) if output.timed_out => ToolRun::Failed(ToolStatus::failure(
            output.duration,
            format!("超时（>{}s）", timeout.as_secs()),
        )),
        Ok(output) => ToolRun::Output(output),
    }
}

fn exit_failure(output: &CommandOutput) -> ToolStatus {
    let code = output
        .code
        .map_or_else(|| "signal".to_owned(), |code| code.to_string());
    let stderr = truncate(output.stderr.trim(), 500);
    let error = if stderr.is_empty() {
        format!("退出码 {code}")
    } else {
        format!("退出码 {code}: {stderr}")
    };
    ToolStatus::failure(output.duration, error).with_exit_code(output.code)
}

fn tool_version(value: &Value) -> Option<String> {
    value
        .get("version")
        .and_then(Value::as_str)
        .or_else(|| value.get("schema").and_then(Value::as_str))
        .filter(|version| !version.trim().is_empty())
        .map(str::to_owned)
}

/// 把 qeaudit 原始 JSON 包装成契约示例的 `{"summary": ..., "raw": ...}` 形状。
///
/// qeaudit 自带 `summary`（total/pass/warn/fail/skip/worst_severity）时直接沿用；
/// 没有时按 `findings[].status` 现场统计，保证控制台总能拿到计数。
pub fn security_section(value: &Value) -> Value {
    let summary = value
        .get("summary")
        .cloned()
        .unwrap_or_else(|| summarize_findings(value));
    json!({ "summary": summary, "raw": value })
}

fn summarize_findings(value: &Value) -> Value {
    let mut pass = 0u64;
    let mut warn = 0u64;
    let mut fail = 0u64;
    let mut skip = 0u64;
    if let Some(findings) = value.get("findings").and_then(Value::as_array) {
        for finding in findings {
            match finding.get("status").and_then(Value::as_str) {
                Some("pass") => pass += 1,
                Some("warn") => warn += 1,
                Some("fail") => fail += 1,
                Some("skip") => skip += 1,
                _ => {}
            }
        }
    }
    json!({
        "total": pass + warn + fail + skip,
        "pass": pass,
        "warn": warn,
        "fail": fail,
        "skip": skip,
    })
}

/// 解析 `docker ps --format '{{json .}}'` 的逐行 JSON。
pub fn parse_docker_ps(stdout: &str) -> Result<Vec<Value>, String> {
    let mut containers = Vec::new();
    for (index, raw_line) in stdout.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|error| format!("第 {} 行不是合法 JSON: {error}", index + 1))?;
        let Some(object) = value.as_object() else {
            return Err(format!("第 {} 行不是 JSON 对象", index + 1));
        };
        let mut container = Map::new();
        if let Some(name) = json_str(object, &["Names", "Name", "names", "name"]) {
            container.insert("name".to_owned(), json!(name));
        }
        if let Some(image) = json_str(object, &["Image", "image"]) {
            container.insert("image".to_owned(), json!(image));
        }
        if let Some(state) = json_str(object, &["State", "state"]) {
            container.insert("state".to_owned(), json!(state));
        }
        if let Some(status) = json_str(object, &["Status", "status"]) {
            container.insert("status".to_owned(), json!(status));
        }
        if let Some(id) = json_str(object, &["ID", "Id", "id"]) {
            container.insert("id".to_owned(), json!(id));
        }
        if !container.is_empty() {
            containers.push(Value::Object(container));
        }
    }
    Ok(containers)
}

/// 解析 `virsh list --all` 表格（进程执行时设置了 `LC_ALL=C`，表头为英文）。
pub fn parse_virsh_list(stdout: &str) -> Vec<Value> {
    let mut vms = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        // 表头 `Id Name State` 与 `------` 分隔线。
        if trimmed.is_empty() || trimmed.chars().all(|ch| ch == '-') {
            continue;
        }
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        let Some(first) = fields.first() else {
            continue;
        };
        if *first == "Id" || *first == "ID" {
            continue;
        }
        // 行首必须是数字 id 或 `-`（关闭的域）。
        if !(first.chars().all(|ch| ch.is_ascii_digit()) || *first == "-") {
            continue;
        }
        if fields.len() < 2 {
            continue;
        }
        let name = fields[1];
        let state = fields[2..].join(" ");
        let mut vm = Map::new();
        vm.insert("name".to_owned(), json!(name));
        if !state.is_empty() {
            vm.insert("state".to_owned(), json!(state));
        }
        if let Ok(id) = first.parse::<u64>() {
            vm.insert("id".to_owned(), json!(id));
        }
        vms.push(Value::Object(vm));
    }
    vms
}

fn json_str<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn truncate(text: &str, limit: usize) -> String {
    let mut result: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn config_with(pairs: &[(&str, &str)]) -> Config {
        let mut map: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        map.entry("DEVICE_PKI_MTLS_URL".to_owned())
            .or_insert_with(|| "https://node:4441".to_owned());
        map.entry("DEVICE_SLOT".to_owned())
            .or_insert_with(|| "room-101".to_owned());
        Config::from_lookup(&Cli::default(), |key| map.get(key).cloned()).unwrap()
    }

    fn disabled_config() -> Config {
        config_with(&[
            ("ROOM_AGENT_DISABLE_HEALTH", "true"),
            ("ROOM_AGENT_DISABLE_DIAGNOSE", "true"),
            ("ROOM_AGENT_DISABLE_SECURITY", "true"),
            ("ROOM_AGENT_DISABLE_DOCKER", "true"),
            ("ROOM_AGENT_DISABLE_VMS", "true"),
        ])
    }

    #[test]
    fn tool_status_serialization_shapes() {
        let failure = ToolStatus::failure(Duration::from_millis(12), "boom");
        let value = serde_json::to_value(&failure).unwrap();
        assert_eq!(value["available"], json!(false));
        assert_eq!(value["error"], json!("boom"));
        assert_eq!(value["duration_ms"], json!(12));
        assert!(value.get("disabled").is_none());
        assert!(value.get("version").is_none());
        assert!(value.get("exit_code").is_none());

        let disabled = serde_json::to_value(ToolStatus::disabled()).unwrap();
        assert_eq!(disabled, json!({"available": false, "disabled": true}));

        let success = serde_json::to_value(
            ToolStatus::success(Duration::from_millis(5)).with_version(Some("v1".to_owned())),
        )
        .unwrap();
        assert_eq!(success["available"], json!(true));
        assert_eq!(success["version"], json!("v1"));
        assert!(success.get("error").is_none());

        let findings = serde_json::to_value(ToolStatus::success_with_exit(
            Duration::from_millis(9),
            Some(2),
        ))
        .unwrap();
        assert_eq!(findings["available"], json!(true));
        assert_eq!(findings["exit_code"], json!(2));
    }

    #[test]
    fn disabled_tools_are_recorded_without_errors() {
        let outcome = collect(&disabled_config());
        assert_eq!(outcome.statuses.len(), 5);
        for name in [
            "platform_health",
            "platform_diagnose",
            "qeaudit",
            "docker",
            "virsh",
        ] {
            let status = outcome
                .statuses
                .get(name)
                .unwrap_or_else(|| panic!("缺少 {name}"));
            assert!(!status.available, "{name}");
            assert!(status.disabled, "{name}");
            assert!(status.error.is_none(), "{name}");
        }
        assert!(outcome.platform_health.is_none());
        assert!(outcome.diagnose.is_none());
        assert!(outcome.security.is_none());
        assert!(outcome.containers.is_none());
        assert!(outcome.vms.is_none());
    }

    #[test]
    fn missing_binary_is_tolerated_and_recorded() {
        let config = config_with(&[
            (
                "PLATFORM_HEALTH_BIN",
                "/nonexistent/qe-room-agent-platform-health",
            ),
            ("ROOM_AGENT_DISABLE_DIAGNOSE", "true"),
            ("ROOM_AGENT_DISABLE_SECURITY", "true"),
            ("ROOM_AGENT_DISABLE_DOCKER", "true"),
            ("ROOM_AGENT_DISABLE_VMS", "true"),
        ]);
        let outcome = collect(&config);
        let status = &outcome.statuses["platform_health"];
        assert!(!status.available);
        assert!(!status.disabled);
        assert!(
            status
                .error
                .as_deref()
                .is_some_and(|error| error.contains("启动")),
            "{status:?}"
        );
        assert!(outcome.platform_health.is_none());
        // 其余工具被禁用，仍然全量记录。
        assert_eq!(outcome.statuses.len(), 5);
    }

    #[test]
    fn non_json_output_is_recorded_as_error() {
        let (status, value) = json_tool(true, "unused", &[], TOOL_TIMEOUT);
        assert!(status.disabled);
        assert!(value.is_none());

        let config = config_with(&[
            ("PLATFORM_HEALTH_BIN", "echo"),
            ("PLATFORM_HEALTH_ARGS", "not-json"),
            ("ROOM_AGENT_DISABLE_DIAGNOSE", "true"),
            ("ROOM_AGENT_DISABLE_SECURITY", "true"),
            ("ROOM_AGENT_DISABLE_DOCKER", "true"),
            ("ROOM_AGENT_DISABLE_VMS", "true"),
        ]);
        let outcome = collect(&config);
        let status = &outcome.statuses["platform_health"];
        assert!(!status.available);
        assert!(
            status
                .error
                .as_deref()
                .is_some_and(|error| error.contains("不是合法 JSON")),
            "{status:?}"
        );
    }

    #[test]
    fn json_tool_parses_success_and_version() {
        let (status, value) = json_tool(
            false,
            "sh",
            &args(&[
                "-c",
                "printf '{\"schema\":\"x/v1\",\"health\":{\"status\":\"ok\"}}'",
            ]),
            TOOL_TIMEOUT,
        );
        assert!(status.available);
        assert!(status.exit_code.is_none());
        assert_eq!(status.version.as_deref(), Some("x/v1"));
        assert_eq!(value.unwrap()["health"]["status"], json!("ok"));
    }

    #[test]
    fn json_tool_accepts_findings_exit_code() {
        // qeaudit / platform-diagnose 用退出码 2 表达 findings，JSON 仍然有效。
        let (status, value) = json_tool(
            false,
            "sh",
            &args(&["-c", "printf '{\"summary\":{\"fail\":1}}'; exit 2"]),
            TOOL_TIMEOUT,
        );
        assert!(status.available);
        assert_eq!(status.exit_code, Some(2));
        assert_eq!(value.unwrap()["summary"]["fail"], json!(1));
    }

    #[test]
    fn json_tool_reports_nonzero_exit_without_json() {
        let (status, value) = json_tool(
            false,
            "sh",
            &args(&["-c", "echo boom >&2; exit 2"]),
            TOOL_TIMEOUT,
        );
        assert!(!status.available);
        assert_eq!(status.exit_code, Some(2));
        let error = status.error.unwrap();
        assert!(error.contains("退出码 2"), "{error}");
        assert!(error.contains("boom"), "{error}");
        assert!(value.is_none());
    }

    #[test]
    fn security_section_keeps_qeaudit_summary() {
        let value = json!({
            "tool": "qeaudit",
            "version": "0.1.0",
            "summary": {"total": 44, "pass": 18, "warn": 5, "fail": 12, "skip": 9},
            "findings": []
        });
        let section = security_section(&value);
        assert_eq!(section["summary"]["pass"], json!(18));
        assert_eq!(section["summary"]["fail"], json!(12));
        assert_eq!(section["raw"]["version"], json!("0.1.0"));
    }

    #[test]
    fn security_section_derives_summary_when_missing() {
        let value = json!({
            "findings": [
                {"id": "A", "status": "pass"},
                {"id": "B", "status": "warn"},
                {"id": "C", "status": "fail"},
                {"id": "D", "status": "skip"},
                {"id": "E", "status": "pass"}
            ]
        });
        let summary = &security_section(&value)["summary"];
        assert_eq!(summary["total"], json!(5));
        assert_eq!(summary["pass"], json!(2));
        assert_eq!(summary["warn"], json!(1));
        assert_eq!(summary["fail"], json!(1));
        assert_eq!(summary["skip"], json!(1));
    }

    #[test]
    fn parses_docker_ps_lines() {
        let stdout = concat!(
            r#"{"Command":"\"/entrypoint.sh\"","ID":"62a5ce3a7c46","Image":"rustfs/rustfs:latest","Names":"qestudio-rustfs-1","State":"running","Status":"Up 2 hours"}"#,
            "\n",
            r#"{"ID":"abc","Image":"nginx:1.27","Names":"web","State":"exited","Status":"Exited (0) 3 hours ago"}"#,
            "\n",
        );
        let containers = parse_docker_ps(stdout).unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0]["name"], json!("qestudio-rustfs-1"));
        assert_eq!(containers[0]["image"], json!("rustfs/rustfs:latest"));
        assert_eq!(containers[0]["state"], json!("running"));
        assert_eq!(containers[0]["status"], json!("Up 2 hours"));
        assert_eq!(containers[0]["id"], json!("62a5ce3a7c46"));
        assert_eq!(containers[1]["name"], json!("web"));
    }

    #[test]
    fn docker_ps_rejects_malformed_lines() {
        let err = parse_docker_ps("not-json\n").unwrap_err();
        assert!(err.contains("不是合法 JSON"), "{err}");
        let err = parse_docker_ps("[]\n").unwrap_err();
        assert!(err.contains("不是 JSON 对象"), "{err}");
    }

    #[test]
    fn parses_virsh_list_table() {
        let stdout = "\
 Id   Name      State
--------------------------
 1    debian12  running
 -    win11     shut off
";
        let vms = parse_virsh_list(stdout);
        assert_eq!(vms.len(), 2);
        assert_eq!(vms[0]["name"], json!("debian12"));
        assert_eq!(vms[0]["state"], json!("running"));
        assert_eq!(vms[0]["id"], json!(1));
        assert_eq!(vms[1]["name"], json!("win11"));
        assert_eq!(vms[1]["state"], json!("shut off"));
        assert!(vms[1].get("id").is_none());
    }

    #[test]
    fn empty_virsh_list_is_fine() {
        let stdout = "\
 Id   Name   State
--------------------
";
        assert!(parse_virsh_list(stdout).is_empty());
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("温度传感器过高", 2), "温度…");
    }
}
