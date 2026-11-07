//! 设备命令结果回传：`POST /machine/command/result`（mTLS 设备入口）。
//!
//! 控制面下发 `vm.command`（以及后续 GPU/维护命令）时在
//! `machine_command_results` 落一行 `pending`；agent 执行完回传
//! `{request_id, ok, data, error}`：
//!
//! - `ok=true` → `status=ok`，`data` 写入 `result`；
//! - `ok=false` → `status=error`，`error` 写入错误列（超时由 agent 上报为
//!   `ok=false` + "超时" 文案；服务端调度侧也可把长期 pending 标 timeout）；
//! - 未知 request_id → 404；
//! - 已终态再次上报 → 200 幂等 no-op（不覆盖第一次结果）。

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::routes::{AppState, resolve_identity};
use crate::tls::PeerCert;

/// 初始状态。
pub const STATUS_PENDING: &str = "pending";

/// `POST /machine/command/result` 请求体。
#[derive(Debug, Clone, Deserialize)]
pub struct CommandResultRequest {
    pub request_id: Uuid,
    pub ok: bool,
    /// 成功时的业务数据（原样存 JSONB）。
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    /// 失败原因。
    #[serde(default)]
    pub error: Option<String>,
}

/// `POST /machine/command/result` 响应体。
#[derive(Debug, Serialize)]
pub struct CommandResultReply {
    pub ok: bool,
    /// `ok` / `error` / `noop`（已终态，幂等）。
    pub status: String,
}

/// 由 agent 的 `ok` 决定终态。
pub fn terminal_status(ok: bool) -> &'static str {
    if ok { "ok" } else { "error" }
}

/// 是否为终态（已终态再上报按 no-op 处理）。
pub fn is_terminal(status: &str) -> bool {
    matches!(status, "ok" | "error" | "timeout")
}

/// `POST /machine/command/result` —— 回传命令执行结果。
pub async fn report_result(
    State(st): State<AppState>,
    peer: PeerCert,
    Json(req): Json<CommandResultRequest>,
) -> Result<Json<CommandResultReply>> {
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;
    // 与其它 mTLS 业务路由一致：证书必须对应 issued/active 设备。
    let _ = resolve_identity(&st, &peer).await?;
    let status = terminal_status(req.ok);

    // 先查归属与当前状态：未知/别人的 request_id 一律 404（不泄露存在性）。
    let existing: Option<(String, String)> =
        sqlx::query_as("SELECT status, serial FROM machine_command_results WHERE request_id = $1")
            .bind(req.request_id)
            .fetch_optional(&st.db)
            .await?;
    let Some((current_status, owner_serial)) = existing else {
        return Err(Error::NotFound);
    };
    if owner_serial != serial {
        return Err(Error::NotFound);
    }
    if is_terminal(&current_status) {
        return Ok(Json(CommandResultReply {
            ok: true,
            status: "noop".to_owned(),
        }));
    }

    let updated = sqlx::query(
        "UPDATE machine_command_results
            SET status = $3, result = $4, error = $5, updated_at = now()
          WHERE request_id = $1 AND serial = $2 AND status = 'pending'",
    )
    .bind(req.request_id)
    .bind(&serial)
    .bind(status)
    .bind(req.data.clone())
    .bind(req.error.clone())
    .execute(&st.db)
    .await?;

    if updated.rows_affected() == 0 {
        // 并发下另一个请求已完成（或状态被调度器改成 timeout）→ 幂等 no-op。
        return Ok(Json(CommandResultReply {
            ok: true,
            status: "noop".to_owned(),
        }));
    }

    tracing::info!(
        request_id = %req.request_id,
        serial = %serial,
        status,
        "机器命令结果已回传"
    );
    Ok(Json(CommandResultReply {
        ok: true,
        status: status.to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ok_to_terminal_status() {
        assert_eq!(terminal_status(true), "ok");
        assert_eq!(terminal_status(false), "error");
        assert_eq!(STATUS_PENDING, "pending");
    }

    #[test]
    fn terminal_detection() {
        for terminal in ["ok", "error", "timeout"] {
            assert!(is_terminal(terminal), "{terminal}");
        }
        for pending in ["pending", "", "running"] {
            assert!(!is_terminal(pending), "{pending}");
        }
    }

    #[test]
    fn request_requires_request_id_and_ok() {
        let ok: CommandResultRequest = serde_json::from_str(
            r#"{"request_id":"0b6f9d4e-4a41-4b6e-9d0b-4f8b0f3c2a11","ok":true,
                "data":{"vms":[]},"error":null}"#,
        )
        .unwrap();
        assert!(ok.ok);
        assert_eq!(ok.data.unwrap()["vms"], serde_json::json!([]));
        assert!(ok.error.is_none());

        // 缺 request_id / ok 都解析失败（路由层回 422/400）
        assert!(serde_json::from_str::<CommandResultRequest>(r#"{"ok":true}"#).is_err());
        assert!(
            serde_json::from_str::<CommandResultRequest>(
                r#"{"request_id":"0b6f9d4e-4a41-4b6e-9d0b-4f8b0f3c2a11"}"#
            )
            .is_err()
        );
        // 非法 uuid
        assert!(
            serde_json::from_str::<CommandResultRequest>(r#"{"request_id":"nope","ok":true}"#)
                .is_err()
        );
    }

    #[test]
    fn failure_body_keeps_error_text() {
        let request: CommandResultRequest = serde_json::from_str(
            r#"{"request_id":"0b6f9d4e-4a41-4b6e-9d0b-4f8b0f3c2a11","ok":false,
                "data":null,"error":"命令执行超时（30s）"}"#,
        )
        .unwrap();
        assert!(!request.ok);
        assert_eq!(request.error.as_deref(), Some("命令执行超时（30s）"));
        assert_eq!(terminal_status(request.ok), "error");
    }
}
