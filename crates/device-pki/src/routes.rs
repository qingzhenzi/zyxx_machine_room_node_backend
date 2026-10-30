//! HTTP 端点（契约第 4 节）。

use crate::ca::{parse_reason, Ca};
use crate::challenge;
use crate::error::{Error, Result};
use crate::pop::{self, SignatureAlgorithm};
use crate::session::{SessionManager, SessionState};
use crate::token::{self, TokenKind};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

/// 客户端证书有效期（天）。一次性颁发 + 持久化场景下不建议过短。
pub const CLIENT_CERT_DAYS: i64 = 365;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub ca: Arc<Ca>,
    pub sessions: Arc<SessionManager>,
    /// 管理员调用 /revoke 的 bearer token
    pub admin_token: Arc<str>,
    /// 服务端自己的证书 sha256（hex），用于 TOFU 指纹对比
    pub server_cert_fp_hex: Arc<str>,
}

// ---------------- /enroll/challenge ----------------

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientInfo {
    pub hostname: Option<String>,
    pub machine_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeRequest {
    pub token: String,
    pub slot: String,
    pub client_info: Option<ClientInfo>,
}

#[derive(Debug, Serialize)]
pub struct ChallengeReply {
    pub challenge_id: Uuid,
    pub challenge: String,
    pub ttl_secs: u64,
    pub kind: String,
}

/// `POST /enroll/challenge` —— 下发挑战值。
///
/// approval 型在这里就创建 pending 注册单，把 client_info 存进去供管理员审核。
pub async fn enroll_challenge(
    State(st): State<AppState>,
    Json(req): Json<ChallengeRequest>,
) -> Result<Json<ChallengeReply>> {
    let (token_row, kind) = token::verify(&st.db, &req.token, &req.slot).await?;

    if kind == TokenKind::Approval && req.client_info.is_none() {
        return Err(Error::BadRequest(
            "approval token requires client_info".into(),
        ));
    }

    // slot 冲突检查：同一 slot 只允许一个存活证书
    let live: Option<(String,)> = sqlx::query_as(
        "SELECT serial FROM device_enrollment
         WHERE device_slot = $1 AND status IN ('pending','issued','active') LIMIT 1",
    )
    .bind(&req.slot)
    .fetch_optional(&st.db)
    .await?;
    if live.is_some() {
        return Err(Error::SlotConflict);
    }

    let issued = challenge::issue(&st.db, &token_row.token_hash, &req.slot).await?;

    // approval 型：此处落一条 pending，供 GET /enroll/{id} 轮询
    let mut enroll_id = None;
    if kind == TokenKind::Approval {
        let id = Uuid::new_v4();
        let info = serde_json::to_value(req.client_info.as_ref()).unwrap_or_default();
        sqlx::query(
            "INSERT INTO device_enrollment
               (serial, device_slot, subject_cn, csr_sha256, cert_pem, status, client_info, enroll_id)
             VALUES ($1, $2, $3, $4, '', 'pending', $5, $6)",
        )
        .bind(format!("pending-{id}"))
        .bind(&req.slot)
        .bind(&req.slot)
        .bind(Vec::<u8>::new())
        .bind(info)
        .bind(id)
        .execute(&st.db)
        .await?;
        enroll_id = Some(id);
    }

    let reply = ChallengeReply {
        challenge_id: issued.challenge_id,
        challenge: hex::encode(issued.challenge),
        ttl_secs: issued.ttl_secs,
        kind: kind.as_str().to_owned(),
    };
    let _ = enroll_id;
    Ok(Json(reply))
}

// ---------------- /enroll ----------------

#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    pub challenge_id: Uuid,
    pub slot: String,
    pub token: String,
    /// base64(DER)
    pub csr_der: String,
    /// base64(signature)
    pub pop_signature: String,
    /// "ecdsa-p256-sha256" | "ed25519"
    pub pop_algorithm: String,
    /// 客户端看到的服务端证书 sha256（hex）
    pub tls_cert_seen_fp: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EnrollReply {
    Issued {
        cert_pem: String,
        ca_chain_pem: String,
        serial: String,
        /// 客户端调用 confirm 时用
        confirm_hint: String,
    },
    Pending {
        enroll_id: Uuid,
        poll_after_secs: u64,
    },
}

pub async fn enroll(
    State(st): State<AppState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<EnrollReply>> {
    let b64 = base64::engine::general_purpose::STANDARD;

    let csr_der = b64
        .decode(req.csr_der.as_bytes())
        .map_err(|e| Error::BadRequest(format!("csr_der not base64: {e}")))?;
    let pop_sig = b64
        .decode(req.pop_signature.as_bytes())
        .map_err(|e| Error::BadRequest(format!("pop_signature not base64: {e}")))?;
    let alg = SignatureAlgorithm::from_label(&req.pop_algorithm)?;

    let (token_row, kind) = token::verify(&st.db, &req.token, &req.slot).await?;
    let chal = challenge::verify(
        &st.db,
        req.challenge_id,
        &token_row.token_hash,
        &req.slot,
    )
    .await?;

    // TOFU / pin 指纹对比：客户端看到的服务端证书必须与服务端自述一致
    if !st.server_cert_fp_hex.is_empty()
        && !req.tls_cert_seen_fp.eq_ignore_ascii_case(&st.server_cert_fp_hex)
    {
        return Err(Error::FpMismatch);
    }

    // PoP 验签：公钥取自 CSR
    let csr_for_pubkey = req.csr_der.clone();
    let csr_sha = pop::sha256(&csr_der);
    let transcript = pop::canonical_transcript(
        &chal.challenge,
        &token_row.token_hash,
        &req.slot,
        &csr_sha,
        &hex::decode(&req.tls_cert_seen_fp).unwrap_or_default(),
    );
    let public_key = extract_csr_public_key(&csr_der)?;
    pop::verify(&public_key, alg, &transcript, &pop_sig)?;
    let _ = csr_for_pubkey;

    // 原子消费 challenge + token
    let mut tx = st.db.begin().await?;
    if !challenge::consume(&mut *tx, req.challenge_id).await? {
        return Err(Error::ChallengeConsumed);
    }
    if !token::consume(&mut *tx, &token_row.token_hash).await? {
        return Err(Error::TokenInvalid);
    }
    tx.commit().await?;

    // 先吊销同 slot 旧证书（换设备场景），再签发新的
    sqlx::query(
        "UPDATE device_enrollment SET status = 'revoked', revoked_at = now(),
                revoke_reason = 'superseded'
         WHERE device_slot = $1 AND status IN ('issued','active')",
    )
    .bind(&req.slot)
    .execute(&st.db)
    .await?;

    let issued = st.ca.sign_csr(&csr_der, &req.slot, CLIENT_CERT_DAYS)?;

    match kind {
        TokenKind::Auto => {
            sqlx::query(
                "INSERT INTO device_enrollment
                   (serial, device_slot, subject_cn, csr_sha256, cert_pem, status, issued_at, downloaded_at)
                 VALUES ($1, $2, $3, $4, $5, 'issued', now(), now())
                 ON CONFLICT (serial) DO UPDATE SET cert_pem = EXCLUDED.cert_pem",
            )
            .bind(&issued.serial_hex)
            .bind(&req.slot)
            .bind(&req.slot)
            .bind(csr_sha.as_slice())
            .bind(&issued.cert_pem)
            .execute(&st.db)
            .await?;

            Ok(Json(EnrollReply::Issued {
                cert_pem: issued.cert_pem,
                ca_chain_pem: st.ca.chain_pem(),
                serial: issued.serial_hex,
                confirm_hint: "POST /enroll/confirm over mTLS to activate".into(),
            }))
        }
        TokenKind::Approval => {
            // 更新 pending 单，等管理员批准
            let row: Option<(Uuid,)> = sqlx::query_as(
                "SELECT enroll_id FROM device_enrollment
                 WHERE device_slot = $1 AND status = 'pending' AND enroll_id IS NOT NULL
                 ORDER BY issued_at NULLS LAST LIMIT 1",
            )
            .bind(&req.slot)
            .fetch_optional(&st.db)
            .await?;
            let enroll_id = row.map(|(id,)| id).unwrap_or_else(Uuid::new_v4);

            sqlx::query(
                "UPDATE device_enrollment
                 SET serial = $1, csr_sha256 = $2, cert_pem = $3, enroll_id = $4
                 WHERE device_slot = $5 AND status = 'pending'",
            )
            .bind(&issued.serial_hex)
            .bind(csr_sha.as_slice())
            .bind(&issued.cert_pem)
            .bind(enroll_id)
            .bind(&req.slot)
            .execute(&st.db)
            .await?;

            Ok(Json(EnrollReply::Pending {
                enroll_id,
                poll_after_secs: 5,
            }))
        }
    }
}

/// 从 CSR DER 提取 SubjectPublicKeyInfo。
fn extract_csr_public_key(csr_der: &[u8]) -> Result<Vec<u8>> {
    use x509_parser::prelude::*;
    let (_, csr) = x509_parser::certification_request::X509CertificationRequest::from_der(csr_der)
        .map_err(|e| Error::BadRequest(format!("invalid CSR: {e}")))?;
    Ok(csr
        .certification_request_info
        .subject_pki
        .raw
        .to_vec())
}

// ---------------- 管理员审批 ----------------

#[derive(Debug, Deserialize)]
pub struct ApprovalDecision {
    pub approve: bool,
    pub note: Option<String>,
}

/// `POST /admin/enroll/{id}/decide` —— 管理员批准/拒绝（需 Bearer）。
pub async fn decide_enrollment(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<ApprovalDecision>,
) -> Result<Json<serde_json::Value>> {
    require_admin(&st, &headers)?;
    let status = if req.approve { "issued" } else { "rejected" };
    let res = sqlx::query(
        "UPDATE device_enrollment SET status = $1,
             issued_at = CASE WHEN $1 = 'issued' THEN now() ELSE issued_at END
         WHERE enroll_id = $2 AND status = 'pending'",
    )
    .bind(status)
    .bind(id)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(Error::EnrollmentNotFound);
    }
    let _ = req.note;
    Ok(Json(serde_json::json!({ "status": status })))
}

// ---------------- /enroll/{id} 轮询 ----------------

#[derive(Debug, Serialize)]
pub struct EnrollStatusReply {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_pem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_chain_pem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_after_secs: Option<u64>,
}

/// `GET /enroll/{id}` —— approval 型轮询；approved 后**一次性**下载证书。
pub async fn enroll_status(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EnrollStatusReply>> {
    let row: Option<(String, Option<String>, String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            "SELECT status, cert_pem, serial, downloaded_at
             FROM device_enrollment WHERE enroll_id = $1",
        )
        .bind(id)
        .fetch_optional(&st.db)
        .await?;
    let (status, cert_pem, serial, downloaded_at) =
        row.ok_or(Error::EnrollmentNotFound)?;

    match status.as_str() {
        "pending" => Ok(Json(EnrollStatusReply {
            status: "pending".into(),
            cert_pem: None,
            ca_chain_pem: None,
            serial: None,
            poll_after_secs: Some(5),
        })),
        "rejected" => Ok(Json(EnrollStatusReply {
            status: "rejected".into(),
            cert_pem: None,
            ca_chain_pem: None,
            serial: None,
            poll_after_secs: None,
        })),
        "issued" | "active" => {
            if downloaded_at.is_some() {
                return Err(Error::AlreadyDownloaded);
            }
            // 一次性：标记已下载
            let res = sqlx::query(
                "UPDATE device_enrollment SET downloaded_at = now()
                 WHERE enroll_id = $1 AND downloaded_at IS NULL",
            )
            .bind(id)
            .execute(&st.db)
            .await?;
            if res.rows_affected() == 0 {
                return Err(Error::AlreadyDownloaded);
            }
            Ok(Json(EnrollStatusReply {
                status: "approved".into(),
                cert_pem,
                ca_chain_pem: Some(st.ca.chain_pem()),
                serial: Some(serial),
                poll_after_secs: None,
            }))
        }
        "revoked" => Err(Error::ApprovalRejected),
        _ => Err(Error::Internal(anyhow::anyhow!("unexpected status {status}"))),
    }
}

// ---------------- /enroll/confirm（mTLS listener） ----------------

#[derive(Debug, Deserialize)]
pub struct ConfirmRequest {
    pub slot: String,
}

#[derive(Debug, Serialize)]
pub struct ConfirmReply {
    pub status: String,
    pub serial: String,
    pub mtls: String,
}

/// `POST /enroll/confirm` —— 客户端拿到证书后回连验证，mTLS 成功才把登记推进到 active。
pub async fn confirm(
    State(st): State<AppState>,
    Extension(peer): Extension<crate::tls::PeerCert>,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<ConfirmReply>> {
    // 身份来自 mTLS 证书中的 CN，而不是请求体
    let subject = peer.common_name().ok_or(Error::Unauthorized)?;
    if subject != req.slot {
        return Err(Error::Unauthorized);
    }

    let res = sqlx::query(
        "UPDATE device_enrollment SET status = 'active', activated_at = now()
         WHERE device_slot = $1 AND status = 'issued'",
    )
    .bind(&req.slot)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        // 已经是 active（重复 confirm）视为幂等成功
        let exists: Option<(String,)> = sqlx::query_as(
            "SELECT serial FROM device_enrollment
             WHERE device_slot = $1 AND status = 'active' LIMIT 1",
        )
        .bind(&req.slot)
        .fetch_optional(&st.db)
        .await?;
        let (serial,) = exists.ok_or(Error::EnrollmentNotFound)?;
        return Ok(Json(ConfirmReply {
            status: "active".into(),
            serial,
            mtls: "ok".into(),
        }));
    }

    let (serial,): (String,) = sqlx::query_as(
        "SELECT serial FROM device_enrollment WHERE device_slot = $1 AND status = 'active'",
    )
    .bind(&req.slot)
    .fetch_one(&st.db)
    .await?;

    st.sessions
        .upsert(&serial, &req.slot, SessionState::Online, 0);

    Ok(Json(ConfirmReply {
        status: "active".into(),
        serial,
        mtls: "ok".into(),
    }))
}

// ---------------- 吊销 ----------------

#[derive(Debug, Deserialize)]
pub struct RevokeRequest {
    pub serial: String,
    pub reason: Option<String>,
}

/// `POST /revoke` —— 管理员吊销（需 Bearer）。
pub async fn revoke(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<RevokeRequest>,
) -> Result<Json<serde_json::Value>> {
    require_admin(&st, &headers)?;
    let reason = req.reason.unwrap_or_else(|| "unspecified".into());
    let res = sqlx::query(
        "UPDATE device_enrollment SET status = 'revoked', revoked_at = now(), revoke_reason = $1
         WHERE serial = $2 AND revoked_at IS NULL",
    )
    .bind(&reason)
    .bind(&req.serial)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(Error::EnrollmentNotFound);
    }
    Ok(Json(serde_json::json!({ "status": "revoked", "serial": req.serial })))
}

// ---------------- CRL ----------------

pub async fn crl(State(st): State<AppState>) -> Result<axum::response::Response> {
    let rows: Vec<(String, chrono::DateTime<chrono::Utc>, Option<String>)> = sqlx::query_as(
        "SELECT serial, revoked_at, revoke_reason FROM device_enrollment
         WHERE revoked_at IS NOT NULL ORDER BY revoked_at",
    )
    .fetch_all(&st.db)
    .await?;

    let entries: Vec<crate::ca::RevokedEntry> = rows
        .into_iter()
        .map(|(serial, at, reason)| crate::ca::RevokedEntry {
            serial_hex: serial,
            revoked_at: OffsetDateTime::from_unix_timestamp(at.timestamp()).unwrap_or_else(|_| {
                OffsetDateTime::now_utc()
            }),
            reason: parse_reason(reason.as_deref().unwrap_or("unspecified")),
        })
        .collect();

    let pem = st.ca.build_crl(&entries, 7)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/pkix-crl")],
        pem,
    )
        .into_response())
}

use axum::response::IntoResponse;

// ---------------- 工具 ----------------

fn require_admin(st: &AppState, headers: &axum::http::HeaderMap) -> Result<()> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Unauthorized)?;
    let token = value
        .strip_prefix("Bearer ")
        .ok_or(Error::Unauthorized)?;
    // 常量时间比较
    let a = Sha256::digest(token.as_bytes());
    let b = Sha256::digest(st.admin_token.as_bytes());
    let eq = a
        .iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y));
    if eq != 0 {
        return Err(Error::Unauthorized);
    }
    Ok(())
}

/// 注册端点路由（单向 TLS listener）。
pub fn enroll_router(st: AppState) -> Router {
    Router::new()
        .route("/enroll/challenge", post(enroll_challenge))
        .route("/enroll", post(enroll))
        .route("/enroll/{id}", get(enroll_status))
        .route("/admin/enroll/{id}/decide", post(decide_enrollment))
        .route("/crl", get(crl))
        .with_state(st)
}

/// mTLS 端点路由。
pub fn mtls_router(st: AppState) -> Router {
    Router::new()
        .route("/enroll/confirm", post(confirm))
        .route("/revoke", post(revoke))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csr_pubkey_extraction_rejects_garbage() {
        assert!(extract_csr_public_key(b"nope").is_err());
    }

    #[test]
    fn csr_pubkey_extraction_works() {
        use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "dev");
        params.distinguished_name = dn;
        let csr = params.serialize_request(&key).unwrap();
        let pk = extract_csr_public_key(csr.der()).unwrap();
        assert!(!pk.is_empty());
        assert_eq!(pk, key.public_key_der());
    }

    #[tokio::test]
    async fn admin_token_compare_rejects_wrong() {
        use axum::http::{HeaderMap, HeaderValue};
        let st = AppState {
            db: PgPool::connect_lazy("postgres://localhost/none").unwrap(),
            ca: Arc::new(Ca::generate_self_signed("t").unwrap()),
            sessions: Arc::new(SessionManager::new(Default::default()).0),
            admin_token: Arc::from("secret"),
            server_cert_fp_hex: Arc::from(""),
        };
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert!(require_admin(&st, &h).is_err());

        let mut ok = HeaderMap::new();
        ok.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        assert!(require_admin(&st, &ok).is_ok());

        assert!(require_admin(&st, &HeaderMap::new()).is_err());
    }
}
