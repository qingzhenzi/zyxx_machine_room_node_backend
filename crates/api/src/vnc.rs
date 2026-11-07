//! noVNC 阶段 2：qe-api 侧的 VNC 会话存储与内部中继 URL 构造。
//!
//! - `POST /admin/machines/{serial}/vnc/session` 生成 `session_id + ticket`（TTL 60s、一次性），
//!   绑定 serial/vm/password；
//! - `GET /admin/machines/{serial}/vnc/ws?session=&ticket=` 校验 ticket 后升级 WS，
//!   再连接 device-pki 内部明文中继 `/internal/vnc/{session}?token=...`。
//!
//! 设计见 `docs/novnc-design.md` §5。

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// ticket 有效期（一次性）。
pub const VNC_TICKET_TTL_SECS: i64 = 60;
/// 允许的 device-pki 内部地址前缀（http/https/ws/wss）。
const INTERNAL_ENDPOINT_SCHEMES: [&str; 4] = ["http://", "https://", "ws://", "wss://"];

/// 一次 VNC 控制台会话（内存，单副本；ticket 一次性）。
#[derive(Debug, Clone)]
pub struct VncSession {
    pub serial: String,
    pub vm: String,
    pub password: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

struct StoredSession {
    ticket: String,
    session: VncSession,
}

/// 进程内 VNC 会话表（session_id → ticket + 会话信息）。
#[derive(Default)]
pub struct VncSessionStore {
    inner: Mutex<HashMap<String, StoredSession>>,
}

impl VncSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session_id: &str, ticket: &str, session: VncSession) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.insert(
            session_id.to_owned(),
            StoredSession {
                ticket: ticket.to_owned(),
                session,
            },
        );
    }

    /// 校验并**消费**一次性 ticket；成功返回会话（之后同 ticket 不可再用）。
    ///
    /// 校验维度：session_id 存在、ticket 匹配（常量时间）、serial 与路径一致、未过期。
    pub fn consume(&self, serial: &str, session_id: &str, ticket: &str) -> Option<VncSession> {
        let now = Utc::now();
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 顺手清理过期会话，避免内存无限增长。
        map.retain(|_, stored| stored.session.expires_at > now);

        let stored = map.get(session_id)?;
        if !constant_time_eq(stored.ticket.as_bytes(), ticket.as_bytes()) {
            return None;
        }
        if !stored.session.serial.eq_ignore_ascii_case(serial) {
            return None;
        }
        let stored = map.remove(session_id)?;
        Some(stored.session)
    }

    /// 当前会话数量（测试/排障用）。
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// 新的 session_id（32 位小写 hex，满足 device-pki `[A-Za-z0-9_-]{16,64}` 约束）。
pub fn new_session_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// 新的 64 位 hex ticket（两个 UUID 拼接，32 字节随机）。
pub fn new_ticket() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// 会话过期时间。
pub fn session_expiry(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::seconds(VNC_TICKET_TTL_SECS)
}

/// 构造 device-pki 内部中继 WS URL：`{ws_base}/internal/vnc/{session}?token={token}`。
///
/// `base` 支持 `http(s)://` 与 `ws(s)://`（http→ws、https→wss 自动转换）。
pub fn internal_ws_url(base: &str, session_id: &str, token: &str) -> AppResult<String> {
    let base = base.trim().trim_end_matches('/');
    if token.is_empty() {
        return Err(AppError::service_unavailable(
            "未配置 DEVICE_PKI_INTERNAL_TOKEN，noVNC 代理不可用",
        ));
    }
    if session_id.is_empty() || session_id.len() > 64 {
        return Err(AppError::bad_request("session 不合法"));
    }
    let scheme_ok = INTERNAL_ENDPOINT_SCHEMES
        .iter()
        .any(|scheme| base.starts_with(scheme));
    if !scheme_ok {
        return Err(AppError::internal(
            "DEVICE_PKI_INTERNAL_URL 必须是 http(s):// 或 ws(s)://",
        ));
    }
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };
    Ok(format!("{ws_base}/internal/vnc/{session_id}?token={token}"))
}

/// 常量时间比较（ticket 不应因比较耗时泄漏前缀）。
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(serial: &str) -> VncSession {
        let now = Utc::now();
        VncSession {
            serial: serial.to_owned(),
            vm: "debian12".to_owned(),
            password: "secret".to_owned(),
            created_at: now,
            expires_at: session_expiry(now),
        }
    }

    #[test]
    fn ticket_is_64_hex_and_session_id_is_32_hex() {
        let ticket = new_ticket();
        assert_eq!(ticket.len(), 64);
        assert!(ticket.chars().all(|ch| ch.is_ascii_hexdigit()));
        let session_id = new_session_id();
        assert_eq!(session_id.len(), 32);
        assert!(session_id.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn store_consumes_ticket_exactly_once() {
        let store = VncSessionStore::new();
        store.insert("session-1", "ticket-1", session("serial-1"));
        let consumed = store.consume("serial-1", "session-1", "ticket-1");
        assert!(consumed.is_some());
        assert_eq!(consumed.unwrap().password, "secret");
        // 一次性：第二次不可用
        assert!(store.consume("serial-1", "session-1", "ticket-1").is_none());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn store_rejects_wrong_ticket_serial_and_expired_session() {
        let store = VncSessionStore::new();
        store.insert("session-1", "ticket-1", session("serial-1"));
        assert!(store.consume("serial-1", "session-1", "wrong").is_none());
        assert!(store.consume("serial-2", "session-1", "ticket-1").is_none());
        assert!(
            store
                .consume("serial-1", "session-missing", "ticket-1")
                .is_none()
        );
        // 未消费的仍然有效（serial 大小写不敏感）
        assert!(store.consume("SERIAL-1", "session-1", "ticket-1").is_some());

        let mut expired = session("serial-1");
        expired.expires_at = Utc::now() - Duration::seconds(1);
        store.insert("session-2", "ticket-2", expired);
        assert!(store.consume("serial-1", "session-2", "ticket-2").is_none());
    }

    #[test]
    fn internal_ws_url_converts_schemes() {
        assert_eq!(
            internal_ws_url("http://127.0.0.1:4442", "abc", "tok").unwrap(),
            "ws://127.0.0.1:4442/internal/vnc/abc?token=tok"
        );
        assert_eq!(
            internal_ws_url("https://pki:4442/", "abc", "tok").unwrap(),
            "wss://pki:4442/internal/vnc/abc?token=tok"
        );
        assert_eq!(
            internal_ws_url("ws://pki:4442", "abc", "tok").unwrap(),
            "ws://pki:4442/internal/vnc/abc?token=tok"
        );
        assert!(internal_ws_url("http://127.0.0.1:4442", "abc", "").is_err());
        assert!(internal_ws_url("ftp://pki", "abc", "tok").is_err());
    }
}
