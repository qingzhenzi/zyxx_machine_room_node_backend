//! P2P 镜像分发：种子登记与 peer 查询（mTLS 设备入口）。
//!
//! 设备成功拉取 / 组装镜像后把自己的 qep2p blob ticket 登记到 device-pki；
//! 同一内网的其它设备按 manifest digest 查询可用 seed，直连下载后再按 manifest
//! 的 chunk 列表校验 sha256。契约见 `room_agent/docs/p2p-image-distribution.md`。
//!
//! 安全语义：
//! - 身份来自设备证书（`PeerCert` 的 serial 即 `device_enrollment.serial`）；
//! - ticket 即能力：拿到 ticket 的内网设备可以直接下载该 blob，因此只在内网
//!   暴露 mTLS listener，且 seed 有 TTL（默认 3600s，可配 60..=86400）；
//! - 查询排除请求者自己，只返回未过期 seed，按 `updated_at` 倒序，最多 20 条。

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::routes::{AppState, resolve_identity};
use crate::tls::PeerCert;

/// 种子默认有效期（秒）。
pub const DEFAULT_SEED_TTL_SECS: i64 = 3600;
/// 种子有效期下限（秒）。
pub const MIN_SEED_TTL_SECS: i64 = 60;
/// 种子有效期上限（秒）。
pub const MAX_SEED_TTL_SECS: i64 = 86_400;
/// peers 查询最多返回条数。
pub const PEER_LIMIT: i64 = 20;
/// blob ticket 最大长度（防滥用/超大请求）。
const MAX_TICKET_LEN: usize = 8192;
/// size_bytes 上限：1 TiB（与缓存上限同量级）。
const MAX_SIZE_BYTES: i64 = 1 << 40;

/// `POST /machine/p2p/seed` 请求体。
#[derive(Debug, Clone, Deserialize)]
pub struct SeedRequest {
    /// 镜像 digest：`sha256:<64 hex>` 或 64 位 hex（服务端归一化为小写 hex）。
    pub manifest_digest: String,
    /// qep2p BlobTicket 字符串（ticket 即下载能力）。
    pub blob_ticket: String,
    /// 镜像产物总字节数（= manifest.total_size）。
    pub size_bytes: i64,
}

/// `POST /machine/p2p/seed` 响应体。
#[derive(Debug, Serialize)]
pub struct SeedReply {
    pub ok: bool,
    /// RFC3339 时间戳。
    pub expires_at: String,
}

/// `GET /machine/p2p/peers` 查询参数。
#[derive(Debug, Default, Deserialize)]
pub struct PeersQuery {
    /// 镜像 digest（必填）。
    pub digest: Option<String>,
}

/// 单个 peer seed。
#[derive(Debug, Serialize)]
pub struct PeerInfo {
    pub serial: String,
    pub blob_ticket: String,
    pub size_bytes: i64,
    /// RFC3339 时间戳。
    pub updated_at: String,
}

/// `GET /machine/p2p/peers` 响应体。
#[derive(Debug, Serialize)]
pub struct PeersReply {
    pub peers: Vec<PeerInfo>,
}

/// 归一化 manifest digest：接受 `sha256:` 前缀，统一存 64 位小写 hex。
pub fn normalize_digest(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let hex = trimmed.strip_prefix("sha256:").unwrap_or(trimmed);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::BadRequest(
            "manifest_digest 必须是 sha256:<64 hex> 或 64 位 hex".into(),
        ));
    }
    Ok(hex.to_ascii_lowercase())
}

/// 校验 size_bytes：必须为正且不超过 1 TiB。
pub fn validate_size_bytes(size_bytes: i64) -> Result<i64> {
    if size_bytes <= 0 {
        return Err(Error::BadRequest(format!(
            "size_bytes 必须 > 0，实际 {size_bytes}"
        )));
    }
    if size_bytes > MAX_SIZE_BYTES {
        return Err(Error::BadRequest(format!(
            "size_bytes 超过上限 1 TiB，实际 {size_bytes}"
        )));
    }
    Ok(size_bytes)
}

/// 校验并裁剪 blob ticket。
pub fn validate_ticket(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::BadRequest("blob_ticket 不能为空".into()));
    }
    if trimmed.len() > MAX_TICKET_LEN {
        return Err(Error::BadRequest(format!(
            "blob_ticket 超过 {MAX_TICKET_LEN} 字节"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(Error::BadRequest("blob_ticket 不能包含控制字符".into()));
    }
    Ok(trimmed.to_owned())
}

/// 解析 TTL（纯函数，便于单测）：越界/非法回退默认值。
pub fn parse_seed_ttl(raw: Option<&str>) -> i64 {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| (MIN_SEED_TTL_SECS..=MAX_SEED_TTL_SECS).contains(value))
        .unwrap_or(DEFAULT_SEED_TTL_SECS)
}

/// 从环境变量读取 TTL（`P2P_SEED_TTL_SECS`，默认 3600s，范围 60..=86400）。
pub fn seed_ttl_from_env() -> i64 {
    let raw = std::env::var("P2P_SEED_TTL_SECS").ok();
    parse_seed_ttl(raw.as_deref())
}

/// `POST /machine/p2p/seed` —— 登记/刷新本设备的镜像 seed。
pub async fn seed(
    State(st): State<AppState>,
    peer: PeerCert,
    Json(req): Json<SeedRequest>,
) -> Result<Json<SeedReply>> {
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;
    // 与其它 mTLS 业务路由一致：先确认证书对应 issued/active 设备。
    let _ = resolve_identity(&st, &peer).await?;

    let digest = normalize_digest(&req.manifest_digest)?;
    let ticket = validate_ticket(&req.blob_ticket)?;
    let size_bytes = validate_size_bytes(req.size_bytes)?;
    let ttl_secs = seed_ttl_from_env();

    let expires_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "INSERT INTO image_p2p_seeds
             (manifest_digest, serial, blob_ticket, size_bytes, updated_at, expires_at)
         VALUES ($1, $2, $3, $4, now(), now() + ($5::bigint * INTERVAL '1 second'))
         ON CONFLICT (manifest_digest, serial) DO UPDATE SET
             blob_ticket = EXCLUDED.blob_ticket,
             size_bytes  = EXCLUDED.size_bytes,
             updated_at  = now(),
             expires_at  = EXCLUDED.expires_at
         RETURNING expires_at",
    )
    .bind(&digest)
    .bind(&serial)
    .bind(&ticket)
    .bind(size_bytes)
    .bind(ttl_secs)
    .fetch_one(&st.db)
    .await?;

    tracing::info!(
        serial = %serial,
        digest = %digest,
        size_bytes,
        ttl_secs,
        "P2P 镜像 seed 已登记"
    );
    Ok(Json(SeedReply {
        ok: true,
        expires_at: expires_at.to_rfc3339(),
    }))
}

/// `GET /machine/p2p/peers?digest=...` —— 查询可用的 peer seed。
pub async fn peers(
    State(st): State<AppState>,
    peer: PeerCert,
    Query(query): Query<PeersQuery>,
) -> Result<Json<PeersReply>> {
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;
    let _ = resolve_identity(&st, &peer).await?;

    let raw_digest = query
        .digest
        .as_deref()
        .ok_or_else(|| Error::BadRequest("缺少 digest 查询参数".into()))?;
    let digest = normalize_digest(raw_digest)?;

    let rows: Vec<(String, String, i64, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT serial, blob_ticket, size_bytes, updated_at
           FROM image_p2p_seeds
          WHERE manifest_digest = $1
            AND expires_at > now()
            AND serial <> $2
          ORDER BY updated_at DESC
          LIMIT $3",
    )
    .bind(&digest)
    .bind(&serial)
    .bind(PEER_LIMIT)
    .fetch_all(&st.db)
    .await?;

    let peers = rows
        .into_iter()
        .map(|(serial, blob_ticket, size_bytes, updated_at)| PeerInfo {
            serial,
            blob_ticket,
            size_bytes,
            updated_at: updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(PeersReply { peers }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_accepts_prefixed_or_hex_and_normalizes() {
        let hex = "a".repeat(64);
        assert_eq!(normalize_digest(&hex).unwrap(), hex);
        assert_eq!(normalize_digest(&format!("sha256:{hex}")).unwrap(), hex);
        let upper = "A".repeat(64);
        assert_eq!(normalize_digest(&upper).unwrap(), "a".repeat(64));
        assert_eq!(normalize_digest(&format!(" sha256:{hex} ")).unwrap(), hex);
    }

    #[test]
    fn digest_rejects_invalid() {
        assert!(normalize_digest("").is_err());
        assert!(normalize_digest("sha256:xyz").is_err());
        assert!(normalize_digest(&"g".repeat(64)).is_err());
        assert!(normalize_digest(&"a".repeat(63)).is_err());
        assert!(normalize_digest(&format!("md5:{hex}", hex = "a".repeat(64))).is_err());
    }

    #[test]
    fn size_bytes_bounds() {
        assert_eq!(validate_size_bytes(1).unwrap(), 1);
        assert_eq!(validate_size_bytes(1 << 40).unwrap(), 1 << 40);
        assert!(validate_size_bytes(0).is_err());
        assert!(validate_size_bytes(-1).is_err());
        assert!(validate_size_bytes((1 << 40) + 1).is_err());
    }

    #[test]
    fn ticket_validation_trims_and_rejects() {
        assert_eq!(validate_ticket("  ticket  ").unwrap(), "ticket");
        assert!(validate_ticket("").is_err());
        assert!(validate_ticket("   ").is_err());
        assert!(validate_ticket(&"x".repeat(MAX_TICKET_LEN + 1)).is_err());
        assert!(validate_ticket("ticket\u{0}").is_err());
    }

    #[test]
    fn seed_ttl_defaults_and_bounds() {
        assert_eq!(parse_seed_ttl(None), DEFAULT_SEED_TTL_SECS);
        assert_eq!(parse_seed_ttl(Some("not-a-number")), DEFAULT_SEED_TTL_SECS);
        assert_eq!(parse_seed_ttl(Some("59")), DEFAULT_SEED_TTL_SECS);
        assert_eq!(parse_seed_ttl(Some("60")), 60);
        assert_eq!(parse_seed_ttl(Some("86400")), 86_400);
        assert_eq!(parse_seed_ttl(Some("86401")), DEFAULT_SEED_TTL_SECS);
    }

    #[test]
    fn seed_request_requires_all_fields() {
        let ok: SeedRequest = serde_json::from_str(
            r#"{"manifest_digest":"sha256:aaaa","blob_ticket":"t","size_bytes":1}"#,
        )
        .unwrap();
        assert_eq!(ok.size_bytes, 1);
        // 缺 blob_ticket / size_bytes 都应反序列化失败（路由层会回 422/400）。
        assert!(serde_json::from_str::<SeedRequest>(r#"{"manifest_digest":"sha256:aa"}"#).is_err());
    }

    #[test]
    fn peers_query_deserializes() {
        let q: PeersQuery = serde_json::from_str(r#"{"digest":"sha256:abc"}"#).unwrap();
        assert_eq!(q.digest.as_deref(), Some("sha256:abc"));
        let empty: PeersQuery = serde_json::from_str("{}").unwrap();
        assert!(empty.digest.is_none());
    }
}
