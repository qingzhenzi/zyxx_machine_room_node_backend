//! 主机电源事件上报（mTLS）：`POST /agent/power-event`。
//!
//! agent 在主机 shutdown/reboot 收尾完成后调用；身份完全来自设备证书：
//! - `serial` 取证书 serial；
//! - `device_slot` / `device_id` 由 `routes::resolve_identity` 从 DB 解析；
//! - 请求体不能指定其它 serial。
//!
//! 该接口是「尽力上报」：agent 失败时本地保留 `last-shutdown.json` 并下次补报，
//! 所以这里不做重试/队列，只做校验 + 落库。单条记录详情上限 64 KiB，
//! 避免被错误 agent 写入超大 JSON。

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use qe_db::machine::{MachineRepo, NewMachinePowerEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::routes::{AppState, resolve_identity};

/// `details` 最大字节数。
pub const MAX_DETAILS_BYTES: usize = 64 * 1024;
/// `note` 最大字符数。
pub const MAX_NOTE_CHARS: usize = 512;
/// `requested_by` 最大字符数。
pub const MAX_REQUESTED_BY_CHARS: usize = 128;
/// `agent_version` 最大字符数。
pub const MAX_AGENT_VERSION_CHARS: usize = 64;

/// 请求体。
#[derive(Debug, Clone, Deserialize)]
pub struct PowerEventRequest {
    /// 契约版本；缺省 1。当前只接受 1。
    #[serde(default)]
    pub schema: Option<u32>,
    /// `shutdown` / `poweroff` / `reboot` / `restart`（大小写不敏感）。
    pub action: String,
    pub graceful: bool,
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub agent_version: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// agent 本地事件时间；仅作为 details 保留，排序用服务端时间。
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub details: Option<Value>,
}

/// 校验后的规范化事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPowerEvent {
    pub action: String,
    pub graceful: bool,
    pub requested_by: Option<String>,
    pub agent_version: Option<String>,
    pub note: Option<String>,
    pub details: Value,
}

/// 规范化 action。
pub fn normalize_action(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "shutdown" | "poweroff" | "power_off" | "halt" => Some("shutdown"),
        "reboot" | "restart" => Some("reboot"),
        _ => None,
    }
}

fn clean_text(raw: Option<String>, max_chars: usize, field: &str) -> Result<Option<String>> {
    let Some(value) = raw else {
        return Ok(None);
    };
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > max_chars {
        return Err(Error::BadRequest(format!(
            "{field} 不能超过 {max_chars} 个字符"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(Error::BadRequest(format!("{field} 不能包含控制字符")));
    }
    Ok(Some(value))
}

/// 纯校验函数（单测不需要 DB）。
pub fn validate(request: PowerEventRequest) -> Result<ValidatedPowerEvent> {
    if let Some(schema) = request.schema
        && schema != 1
    {
        return Err(Error::BadRequest(format!(
            "不支持的 schema {schema}（当前只支持 1）"
        )));
    }
    let action = normalize_action(&request.action)
        .ok_or_else(|| Error::BadRequest("action 只能是 shutdown/reboot".to_owned()))?;

    let requested_by = clean_text(request.requested_by, MAX_REQUESTED_BY_CHARS, "requested_by")?;
    let agent_version = clean_text(
        request.agent_version,
        MAX_AGENT_VERSION_CHARS,
        "agent_version",
    )?;
    let note = clean_text(request.note, MAX_NOTE_CHARS, "note")?;

    let mut details = request.details.unwrap_or_else(|| json!({}));
    if !details.is_object() {
        return Err(Error::BadRequest("details 必须是 JSON object".to_owned()));
    }
    if let Some(at) = request.at {
        if let Some(object) = details.as_object_mut() {
            object.entry("agent_at").or_insert_with(|| json!(at));
        }
    }
    let size = serde_json::to_vec(&details)
        .map_err(|error| Error::BadRequest(format!("details 无法序列化：{error}")))?
        .len();
    if size > MAX_DETAILS_BYTES {
        return Err(Error::BadRequest(format!(
            "details 过大（{size} 字节，上限 {MAX_DETAILS_BYTES}）"
        )));
    }

    Ok(ValidatedPowerEvent {
        action: action.to_owned(),
        graceful: request.graceful,
        requested_by,
        agent_version,
        note,
        details,
    })
}

/// `POST /agent/power-event`。
pub async fn report(
    State(st): State<AppState>,
    peer: crate::tls::PeerCert,
    Json(request): Json<PowerEventRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;
    let (device_id, device_slot) = resolve_identity(&st, &peer).await?;
    let event = validate(request)?;

    let stored = MachineRepo::new(st.db.clone())
        .record_power_event(&NewMachinePowerEvent {
            serial: serial.clone(),
            device_slot: device_slot.clone(),
            action: event.action.clone(),
            graceful: event.graceful,
            requested_by: event.requested_by.or_else(|| Some("agent".to_owned())),
            agent_version: event.agent_version,
            source: "agent".to_owned(),
            note: event.note,
            details: event.details,
        })
        .await
        .map_err(|error| Error::Internal(anyhow::anyhow!("写入电源事件失败: {error}")))?;

    tracing::info!(
        serial = %stored.serial,
        device_id = %device_id,
        slot = %stored.device_slot,
        action = %stored.action,
        graceful = stored.graceful,
        "收到机器电源事件（mTLS）"
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "accepted",
            "id": stored.id,
            "serial": stored.serial,
            "device_id": device_id.to_string(),
            "device_slot": stored.device_slot,
            "action": stored.action,
            "graceful": stored.graceful,
            "created_at": stored.created_at,
        })),
    ))
}

/// 兼容字段命名：OpenAPI / 文档示例用的响应（仅测试/文档引用）。
#[derive(Debug, Serialize)]
pub struct PowerEventReply {
    pub status: String,
    pub id: i64,
    pub serial: String,
    pub action: String,
    pub graceful: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(action: &str) -> PowerEventRequest {
        PowerEventRequest {
            schema: Some(1),
            action: action.to_owned(),
            graceful: true,
            requested_by: Some("agent".to_owned()),
            agent_version: Some("0.1.0".to_owned()),
            note: Some("ok".to_owned()),
            at: None,
            details: Some(json!({"trigger": "signal:SIGTERM"})),
        }
    }

    #[test]
    fn normalizes_actions() {
        assert_eq!(normalize_action("poweroff"), Some("shutdown"));
        assert_eq!(normalize_action("RESTART"), Some("reboot"));
        assert_eq!(normalize_action("sleep"), None);
        assert!(validate(request("shutdown")).is_ok());
        assert!(validate(request("reboot")).is_ok());
        assert!(validate(request("format-disk")).is_err());
    }

    #[test]
    fn rejects_bad_schema_and_details() {
        let mut bad = request("shutdown");
        bad.schema = Some(2);
        assert!(validate(bad).is_err());

        let mut bad = request("shutdown");
        bad.details = Some(json!("not-an-object"));
        assert!(validate(bad).is_err());

        let mut big = request("shutdown");
        big.details = Some(json!({"blob": "x".repeat(MAX_DETAILS_BYTES)}));
        assert!(validate(big).is_err());
    }

    #[test]
    fn keeps_agent_at_in_details() {
        let mut with_at = request("reboot");
        with_at.at = Some(Utc::now());
        let event = validate(with_at).unwrap();
        assert!(event.details.get("agent_at").is_some());
    }

    #[test]
    fn trims_text_and_rejects_control_chars() {
        let mut with_note = request("shutdown");
        with_note.note = Some("  replaced disk  ".to_owned());
        assert_eq!(
            validate(with_note).unwrap().note.as_deref(),
            Some("replaced disk")
        );

        let mut bad = request("shutdown");
        bad.requested_by = Some("bad\nname".to_owned());
        assert!(validate(bad).is_err());
    }
}
