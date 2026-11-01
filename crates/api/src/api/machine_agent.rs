//! 机器遥测上报端点（room-agent → qe-api，过渡期的 Bearer 入口）。
//!
//! 默认入口是 `qe-device-pki` mTLS listener 上的 `POST /machine/report`；本端点保留给
//! 尚未切 mTLS 的旧 room-agent，`MACHINE_AGENT_TOKEN` 未配置时关闭（503）。
//!
//! 请求体、字段校验与 tier 语义都在 `qe_domain::machine`，与 mTLS 入口完全共用。
//! 身份口径：本入口只上报 `device_slot`，服务端在 `device_enrollment` 中解析 `serial`
//! （只接受 `issued` / `active`），客户端无法伪造任意 serial。

use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, header, request::Parts},
};
use serde_json::{Value, json};

use qe_db::machine::NewMachineSnapshot;
use qe_domain::machine::MachineReportRequest;

use crate::{
    api::extractors::ApiJson,
    error::{AppError, AppResult},
    state::AppState,
};

/// 常量时间比较，避免 token 逐字节比较泄漏长度/前缀信息。
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// 机器上报鉴权提取器：在 JSON body 解析之前完成 token 校验，
/// 这样无效 token 不会因为 body 格式错误而得到 400（避免把校验顺序暴露给未授权调用方）。
pub struct MachineAgentAuth;

impl FromRequestParts<AppState> for MachineAgentAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        require_agent_token(state, &parts.headers)?;
        Ok(Self)
    }
}

fn require_agent_token(state: &AppState, headers: &axum::http::HeaderMap) -> AppResult<()> {
    let expected = state.config.machine_agent_token.as_deref().ok_or_else(|| {
        AppError::service_unavailable("机器上报未启用（服务端未配置 MACHINE_AGENT_TOKEN）")
    })?;

    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::unauthorized("缺少 Authorization 请求头"))?;
    let provided = raw
        .split_once(' ')
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, token)| token.trim())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| AppError::unauthorized("Authorization 需为 `Bearer <token>` 格式"))?;

    if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(AppError::unauthorized("机器上报 token 无效"));
    }
    Ok(())
}

/// `POST /api/v1/machine/report`（Bearer 入口）。
pub async fn report(
    State(state): State<AppState>,
    _auth: MachineAgentAuth,
    ApiJson(request): ApiJson<MachineReportRequest>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let validated = request.validate()?;
    let device_slot = validated
        .device_slot
        .clone()
        .ok_or_else(|| AppError::bad_request("device_slot 不能为空"))?;

    let enrollment = state
        .machines
        .resolve_enrollment(&device_slot)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "槽位 {device_slot} 未找到已注册设备（需要 issued/active 状态）"
            ))
        })?;

    // payload 原样存档，但以服务端解析出的槽位为准（防止 body 与证书/库不一致）。
    let mut payload = validated.payload;
    payload["device_slot"] = json!(enrollment.device_slot);

    let snapshot = NewMachineSnapshot {
        serial: enrollment.serial.clone(),
        device_slot: enrollment.device_slot.clone(),
        tier: validated.tier,
        collected_at: validated.collected_at,
        agent_version: validated.agent_version,
        hostname: validated.hostname,
        os_name: validated.os_name,
        kernel_version: validated.kernel_version,
        health_status: validated.health_status,
        health_score: validated.health_score,
        cpu_usage_percent: validated.cpu_usage_percent,
        memory_usage_percent: validated.memory_usage_percent,
        disk_usage_percent: validated.disk_usage_percent,
        uptime_secs: validated.uptime_secs,
        payload,
    };

    let meta = state
        .machines
        .record(&snapshot, state.config.machine_snapshot_keep)
        .await?;

    tracing::info!(
        serial = %meta.serial,
        slot = %meta.device_slot,
        tier = validated.tier.as_str(),
        health = %meta.health_status,
        score = ?meta.health_score,
        collected_at = %meta.collected_at,
        "机器快照已上报"
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "snapshot_id": meta.id,
            "serial": meta.serial,
            "device_slot": meta.device_slot,
            "received_at": meta.received_at,
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_compares_content_and_length() {
        assert!(constant_time_eq(b"secret-token-0123", b"secret-token-0123"));
        assert!(!constant_time_eq(
            b"secret-token-0123",
            b"secret-token-0124"
        ));
        assert!(!constant_time_eq(b"short", b"longer-value"));
    }
}
