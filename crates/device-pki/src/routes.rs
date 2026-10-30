//! HTTP 端点（契约第 4 节）。

use crate::ca::{parse_reason, Ca};
use crate::challenge;
use crate::error::{Error, Result};
use crate::pop::{self, SignatureAlgorithm};
use crate::session::{Dedup, SessionManager, SessionState};
use crate::token::{self, TokenKind};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

/// 客户端证书有效期（天）：100 年。
///
/// 为什么这么长：这些证书只用于 mTLS 身份认证，生命周期由**主动吊销**管理，
/// 而不是由有效期管理。短证书 + 自动续期方案需要一整套续期链路、到期告警、
/// 续期失败兜底；长证书 + 实时吊销用更少的活动部件达到同样效果。
///
/// 真正的失效点是服务端 mTLS 握手时的吊销检查（见 `tls.rs`），
/// 吊销延迟 ≈ 吊销缓存刷新间隔（5s），与证书有效期无关。
pub const CLIENT_CERT_DAYS: i64 = 36500;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub ca: Arc<Ca>,
    pub sessions: Arc<SessionManager>,
    /// 管理员调用 /revoke 的 bearer token
    pub admin_token: Arc<str>,
    /// 服务端自己的证书 sha256（hex），用于 TOFU 指纹对比
    pub server_cert_fp_hex: Arc<str>,
    /// 证书可用性内存缓存（mTLS 握手查它：是否 issued/active、是否被吊销）
    pub cert_status: Arc<crate::cert_status::CertStatusCache>,
    /// 管理端限速器（静态 bearer 没有账号锁定，只能靠"不能无限试"）
    pub admin_limiter: Arc<crate::ratelimit::RateLimiter>,
    /// 对外公布的 mTLS 端点，随注册响应下发给客户端
    pub mtls_endpoint: Arc<str>,
    /// 对外公布的注册端点，用于拼接注册链接（管理端发 token 时用）
    pub enroll_endpoint: Arc<str>,
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
    /// approval 型：注册单 id，客户端应立刻持久化，便于中断后续跑
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enroll_id: Option<Uuid>,
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

    // slot 冲突检查：同一 slot 只允许一个存活证书。
    //
    // 例外：approval 型的 `pending` 单**可复用**。否则客户端在 challenge 之后、
    // enroll 之前网络抖动一次（或进程重启），就会被自己刚创建的 pending 单挡成 409，
    // 而没有任何清理入口，这台设备永久注册不上。
    let live: Option<(String, String)> = sqlx::query_as(
        "SELECT serial, status FROM device_enrollment
         WHERE device_slot = $1 AND status IN ('pending','issued','active') LIMIT 1",
    )
    .bind(&req.slot)
    .fetch_optional(&st.db)
    .await?;
    let reusable_pending = matches!(&live, Some((_, status)) if status == "pending" && kind == TokenKind::Approval);
    if live.is_some() && !reusable_pending {
        return Err(Error::SlotConflict);
    }

    let issued = challenge::issue(&st.db, &token_row.token_hash, &req.slot).await?;

    // approval 型：落一条 pending（或复用已有的），供 GET /enroll/{id} 轮询
    let mut enroll_id = None;
    if kind == TokenKind::Approval {
        let info = serde_json::to_value(req.client_info.as_ref()).unwrap_or_default();
        if reusable_pending {
            let row: Option<(Uuid,)> = sqlx::query_as(
                "UPDATE device_enrollment SET client_info = $1
                 WHERE device_slot = $2 AND status = 'pending' AND enroll_id IS NOT NULL
                 RETURNING enroll_id",
            )
            .bind(info.clone())
            .bind(&req.slot)
            .fetch_optional(&st.db)
            .await?;
            enroll_id = row.map(|(id,)| id);
        }
        if enroll_id.is_none() {
            let id = Uuid::new_v4();
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
    }

    let reply = ChallengeReply {
        challenge_id: issued.challenge_id,
        challenge: hex::encode(issued.challenge),
        ttl_secs: issued.ttl_secs,
        kind: kind.as_str().to_owned(),
        enroll_id,
    };
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
        /// 客户端后续 mTLS 通讯应连这个地址（与注册端点不同）
        mtls_endpoint: String,
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

    // TOFU / pin 指纹对比：客户端看到的服务端证书必须与服务端自述一致。
    //
    // 这里**不允许空值绕过**：早期实现写成 `!server_fp.is_empty() && ...`，
    // 只要服务端没配置指纹，客户端传什么都放过，等于 TOFU 防中间人完全失效。
    if st.server_cert_fp_hex.is_empty() {
        return Err(Error::Internal(anyhow::anyhow!(
            "server_cert_fp_hex 未配置，拒绝在无法校验 TOFU 指纹的情况下签发证书"
        )));
    }
    if req.tls_cert_seen_fp.len() != 64
        || !req.tls_cert_seen_fp.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(Error::FpMismatch);
    }
    if !req.tls_cert_seen_fp.eq_ignore_ascii_case(&st.server_cert_fp_hex) {
        tracing::warn!(
            seen = %req.tls_cert_seen_fp,
            expected = %st.server_cert_fp_hex,
            slot = %req.slot,
            "TOFU fingerprint mismatch — possible MITM"
        );
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

    // 先签发（纯计算，无副作用），再把「消费 challenge + 消费 token + 顶掉旧证书 +
    // 落库」放进**同一个事务**：任何一步失败都整体回滚，token 不会被烧掉，
    // 客户端可以拿同一张链接重试。之前消费事务与落库分离，落库失败 = 链接作废。
    let issued = st.ca.sign_csr(&csr_der, &req.slot, CLIENT_CERT_DAYS)?;

    let mut tx = st.db.begin().await?;
    if !challenge::consume(&mut *tx, req.challenge_id).await? {
        return Err(Error::ChallengeConsumed);
    }
    if !token::consume(&mut *tx, &token_row.token_hash).await? {
        return Err(Error::TokenInvalid);
    }

    // 同 slot 旧证书顶掉（换设备场景 / 竞态窗口）。返回旧 serial，
    // 提交后再立即写入吊销缓存 —— 不等 5s 刷新，否则旧设备在这个窗口内仍能完成握手。
    let superseded: Vec<(String,)> = sqlx::query_as(
        "UPDATE device_enrollment SET status = 'revoked', revoked_at = now(),
                revoke_reason = 'superseded'
         WHERE device_slot = $1 AND status IN ('issued','active')
         RETURNING serial",
    )
    .bind(&req.slot)
    .fetch_all(&mut *tx)
    .await?;

    let csr_sha = pop::sha256(&csr_der);
    let rows_affected = match kind {
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
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        TokenKind::Approval => {
            // 更新 pending 单，等管理员批准
            let row: Option<(Uuid,)> = sqlx::query_as(
                "SELECT enroll_id FROM device_enrollment
                 WHERE device_slot = $1 AND status = 'pending' AND enroll_id IS NOT NULL
                 ORDER BY issued_at NULLS LAST LIMIT 1",
            )
            .bind(&req.slot)
            .fetch_optional(&mut *tx)
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
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
    };
    if rows_affected == 0 {
        // pending 单被管理员并发改掉（拒绝/删除）：回滚，token 保留
        return Err(Error::EnrollmentNotFound);
    }
    tx.commit().await?;

    for (old_serial,) in &superseded {
        st.cert_status.mark_revoked(old_serial);
        tracing::info!(serial = %old_serial, slot = %req.slot, "superseded old cert, revoked immediately");
    }

    match kind {
        TokenKind::Auto => Ok(Json(EnrollReply::Issued {
            cert_pem: issued.cert_pem,
            ca_chain_pem: st.ca.chain_pem(),
            serial: issued.serial_hex,
            mtls_endpoint: st.mtls_endpoint.to_string(),
            confirm_hint: "POST /enroll/confirm over mTLS to activate".into(),
        })),
        TokenKind::Approval => {
            let enroll_id: Uuid = sqlx::query_scalar(
                "SELECT enroll_id FROM device_enrollment
                 WHERE device_slot = $1 AND status = 'pending' LIMIT 1",
            )
            .bind(&req.slot)
            .fetch_one(&st.db)
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
///
/// 拒绝时**立即吊销**该注册单在 enroll 阶段就已签好的证书：只改状态的话，
/// 一张 100 年有效、由本 CA 签发的证书会以 `rejected` 身份留在库里，
/// 而 mTLS 握手只查吊销缓存，PEM 一旦泄露就是永久可用凭证。
pub async fn decide_enrollment(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<ApprovalDecision>,
) -> Result<Json<serde_json::Value>> {
    require_admin(&st, &headers)?;
    let status = if req.approve { "issued" } else { "rejected" };
    let serial: Option<String> = sqlx::query_scalar(
        "UPDATE device_enrollment SET status = $1,
              issued_at = CASE WHEN $1 = 'issued' THEN now() ELSE issued_at END,
              revoked_at = CASE WHEN $1 = 'rejected' THEN now() ELSE revoked_at END,
              revoke_reason = CASE WHEN $1 = 'rejected' THEN 'rejected' ELSE revoke_reason END
         WHERE enroll_id = $2 AND status = 'pending'
         RETURNING serial",
    )
    .bind(status)
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let serial = serial.ok_or(Error::EnrollmentNotFound)?;
    if let Some(note) = req.note.as_deref() {
        tracing::info!(enroll_id = %id, note, "approval decision note");
    }
    if !req.approve {
        // 立即生效，不等 5s 后台刷新
        st.cert_status.mark_revoked(&serial);
        tracing::info!(enroll_id = %id, serial = %serial, "enrollment rejected, cert revoked");
    }
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
    /// 拿到证书后应连的 mTLS 端点（与注册端点不同）。
    /// 缺了它 approval 型客户端只能拿注册端点去 confirm，必然 404。
    pub mtls_endpoint: String,
}

/// `GET /enroll/{id}` —— approval 型轮询；approved 后下载证书。
///
/// 可取性边界：`issued`（未激活）期间**可以重复下载**——服务端在响应体真正
/// 送达前就已置位 `downloaded_at`，若响应在网络中丢失，一次性语义会让设备
/// 永久拿不到证书且 token 已消费、无路可走。改为以 `confirm`（激活）为界：
/// 激活时 `cert_pem` 被清空，之后再取才是 410，DB 也不再长期持有私钥等价物。
pub async fn enroll_status(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EnrollStatusReply>> {
    let row: Option<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT status, cert_pem, serial FROM device_enrollment WHERE enroll_id = $1",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let (status, cert_pem, serial) = row.ok_or(Error::EnrollmentNotFound)?;

    let mtls_endpoint = st.mtls_endpoint.to_string();
    match status.as_str() {
        "pending" => Ok(Json(EnrollStatusReply {
            status: "pending".into(),
            cert_pem: None,
            ca_chain_pem: None,
            serial: None,
            poll_after_secs: Some(5),
            mtls_endpoint,
        })),
        "rejected" => Ok(Json(EnrollStatusReply {
            status: "rejected".into(),
            cert_pem: None,
            ca_chain_pem: None,
            serial: None,
            poll_after_secs: None,
            mtls_endpoint,
        })),
        "issued" => {
            // 记录首次下载时间（尽力而为，不作为闸门）
            let _ = sqlx::query(
                "UPDATE device_enrollment SET downloaded_at = COALESCE(downloaded_at, now())
                 WHERE enroll_id = $1",
            )
            .bind(id)
            .execute(&st.db)
            .await;
            let cert_pem = cert_pem.ok_or(Error::AlreadyDownloaded)?;
            Ok(Json(EnrollStatusReply {
                status: "approved".into(),
                cert_pem: Some(cert_pem),
                ca_chain_pem: Some(st.ca.chain_pem()),
                serial: Some(serial),
                poll_after_secs: None,
                mtls_endpoint,
            }))
        }
        // 已激活：证书已交付并被使用，DB 侧已清空
        "active" => Err(Error::AlreadyDownloaded),
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
    peer: crate::tls::PeerCert,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<ConfirmReply>> {
    // 身份来自 mTLS 证书中的 CN，而不是请求体
    let subject = peer.common_name().ok_or(Error::Unauthorized)?;
    if subject != req.slot {
        return Err(Error::Unauthorized);
    }

    let res = sqlx::query(
        "UPDATE device_enrollment SET status = 'active', activated_at = now(),
                -- 证书已交付并被使用，DB 不再长期持有它（见 enroll_status 的可取性边界）
                cert_pem = ''
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
    // 立即生效，不等后台刷新
    st.cert_status.mark_revoked(&req.serial);
    tracing::info!(serial = %req.serial, reason = %reason, "cert revoked");
    Ok(Json(serde_json::json!({ "status": "revoked", "serial": req.serial })))
}

// ---------------- 管理端：签发注册链接 / 注册单管理 ----------------

/// slot 合法字符：与客户端 `sanitize()` 的白名单一致，同时避免注册链接被注入。
fn validate_slot(slot: &str) -> Result<()> {
    if slot.is_empty() || slot.len() > 128 {
        return Err(Error::BadRequest("slot 长度必须在 1..=128".into()));
    }
    if !slot
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(Error::BadRequest(
            "slot 只允许字母/数字/-/_/.".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    /// "auto" | "approval"
    pub kind: String,
    pub slot: String,
    /// 有效期（秒），默认 900（契约第 3 节 15 min）
    pub ttl_secs: Option<i64>,
    /// 记入 `device_bootstrap_token.created_by`
    pub created_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateTokenReply {
    pub token: String,
    pub kind: String,
    pub slot: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// 明文注册链接，**只在这一次响应里出现**（库中只存 SHA-256）
    pub link: String,
}

/// `POST /admin/tokens` —— 生成一次性注册链接（需 Bearer）。
pub async fn create_token(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<CreateTokenReply>> {
    require_admin(&st, &headers)?;
    let kind = TokenKind::parse(&req.kind)?;
    let slot = req.slot.trim();
    validate_slot(slot)?;
    let ttl = chrono::Duration::seconds(req.ttl_secs.unwrap_or(900).clamp(60, 86_400));
    let created_by = req.created_by.unwrap_or_else(|| "admin".to_owned());
    let issued = token::issue(&st.db, kind, slot, &created_by, ttl).await?;
    let link = token::enroll_link(&st.enroll_endpoint, &issued.token, slot);
    tracing::info!(slot, kind = kind.as_str(), created_by, "bootstrap token issued");
    Ok(Json(CreateTokenReply {
        token: issued.token,
        kind: kind.as_str().to_owned(),
        slot: slot.to_owned(),
        expires_at: issued.expires_at,
        link,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// 按状态过滤：pending / issued / active / rejected / revoked
    pub status: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct EnrollmentSummary {
    pub enroll_id: Option<Uuid>,
    pub serial: String,
    pub device_slot: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_info: Option<serde_json::Value>,
    pub downloaded: bool,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub activated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// `GET /admin/enrollments` —— 注册单列表（需 Bearer）。
///
/// 审批流此前缺"列出来"这一步：管理员必须先知道 enroll_id 才能 decide，
/// 而 enroll_id 只在客户端手里，等于没有审批入口。
pub async fn list_enrollments(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(q): axum::extract::Query<ListQuery>,
) -> Result<Json<Vec<EnrollmentSummary>>> {
    require_admin(&st, &headers)?;
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    type Row = (
        Option<Uuid>,
        String,
        String,
        String,
        Option<serde_json::Value>,
        bool,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let rows: Vec<Row> = if let Some(status) = q.status {
            sqlx::query_as(
                "SELECT enroll_id, serial, device_slot, status, client_info,
                        downloaded_at IS NOT NULL, issued_at, activated_at, revoked_at
                 FROM device_enrollment WHERE status = $1
                 ORDER BY issued_at DESC NULLS LAST LIMIT $2",
            )
            .bind(status)
            .bind(limit)
            .fetch_all(&st.db)
            .await?
        } else {
            sqlx::query_as(
                "SELECT enroll_id, serial, device_slot, status, client_info,
                        downloaded_at IS NOT NULL, issued_at, activated_at, revoked_at
                 FROM device_enrollment
                 ORDER BY issued_at DESC NULLS LAST LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&st.db)
            .await?
        };
    Ok(Json(
        rows.into_iter()
            .map(
                |(enroll_id, serial, device_slot, status, client_info, downloaded, issued_at, activated_at, revoked_at)| {
                    EnrollmentSummary {
                        enroll_id,
                        serial,
                        device_slot,
                        status,
                        client_info,
                        downloaded,
                        issued_at,
                        activated_at,
                        revoked_at,
                    }
                },
            )
            .collect(),
    ))
}

/// `DELETE /admin/enrollments/{id}` —— 清理终态注册单（需 Bearer）。
///
/// 只允许删 `pending`（卡死的半截注册）与 `rejected`；`issued/active` 要走
/// `/revoke` 才能同时吊销证书，`revoked` 保留作吊销审计。
pub async fn delete_enrollment(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>> {
    require_admin(&st, &headers)?;
    let res = sqlx::query(
        "DELETE FROM device_enrollment
         WHERE enroll_id = $1 AND status IN ('pending','rejected')",
    )
    .bind(id)
    .execute(&st.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(Error::EnrollmentNotFound);
    }
    tracing::info!(enroll_id = %id, "enrollment cleaned up");
    Ok(Json(serde_json::json!({ "status": "deleted" })))
}

// ---------------- /messages（mTLS listener） ----------------

/// 单条消息最大载荷。防抖会话层不是消息队列，超大载荷应走对象存储。
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    /// 单调递增，从 1 开始；服务端按 `(serial, seq)` 去重
    pub seq: i64,
    pub kind: String,
    pub payload: String,
}

#[derive(Debug, Serialize)]
pub struct MessageReply {
    pub status: String,
    pub seq: i64,
    pub duplicate: bool,
}

/// `POST /messages` —— 设备上行消息（mTLS listener）。
///
/// **幂等**：重复的 `seq` 返回 `duplicate: true` 但仍是 2xx，这样客户端重放
/// 不会因为"怕重复触发副作用"而不敢重试。
pub async fn receive_message(
    State(st): State<AppState>,
    peer: crate::tls::PeerCert,
    Json(req): Json<MessageRequest>,
) -> Result<Json<MessageReply>> {
    // 身份来自 mTLS 证书，不信任请求体
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;
    let slot = peer.common_name().ok_or(Error::Unauthorized)?.to_owned();

    if req.seq <= 0 {
        return Err(Error::BadRequest("seq 必须为正整数".into()));
    }
    if req.kind.is_empty() || req.kind.len() > 64 {
        return Err(Error::BadRequest("kind 长度必须在 1..=64".into()));
    }
    if req.payload.len() > MAX_MESSAGE_BYTES {
        return Err(Error::BadRequest(format!(
            "payload 超过 {MAX_MESSAGE_BYTES} 字节"
        )));
    }

    let duplicate = match st.sessions.last_seen_seq(&serial) {
        Some(_) => st.sessions.observe(&serial, req.seq) == Dedup::Duplicate,
        None => {
            // 未知会话（重启后、或该设备还没 confirm 过）：先登记再按新消息处理
            st.sessions.upsert(&serial, &slot, SessionState::Online, req.seq);
            false
        }
    };

    if !duplicate {
        // 水位落库：重启后仍能按 (serial, seq) 去重（契约 §6）
        sqlx::query(
            "INSERT INTO device_session (serial, device_slot, last_seen_seq, state, updated_at)
             VALUES ($1, $2, $3, 'online', now())
             ON CONFLICT (serial) DO UPDATE SET
               last_seen_seq = GREATEST(device_session.last_seen_seq, EXCLUDED.last_seen_seq),
               state = 'online',
               updated_at = now()",
        )
        .bind(&serial)
        .bind(&slot)
        .bind(req.seq)
        .execute(&st.db)
        .await?;
        // 上行活动也算在线：若此前被判 offline，这里会排一次防抖后的 online 翻转
        st.sessions.mark_seen(&serial).await;
    }

    Ok(Json(MessageReply {
        status: if duplicate { "duplicate" } else { "ok" }.into(),
        seq: req.seq,
        duplicate,
    }))
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

/// 管理端限速中间件。
///
/// `/admin/...` 的鉴权是**静态** bearer：没有账号锁定、没有失败计数，
/// 唯一的防线就是「不能无限试」。按对端 IP 给每分钟预算，超了回 429 + Retry-After。
pub async fn admin_rate_limit(
    State(st): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response> {
    let addr = req.extensions().get::<std::net::SocketAddr>().copied();
    let key = crate::ratelimit::key_for(addr.as_ref());
    if let Err(retry) = st.admin_limiter.check(&key) {
        tracing::warn!(ip = %key, retry_after = retry, "admin rate limited");
        return Err(Error::RateLimited(retry));
    }
    Ok(next.run(req).await)
}

/// 注册端点路由（单向 TLS listener）。
pub fn enroll_router(st: AppState) -> Router {
    // 只给管理面限速：注册面靠 256 bit 随机 token 自身的熵，
    // 暴力猜解不可行，给它限速反而会误伤重试的正常设备。
    let admin = Router::new()
        .route("/admin/tokens", post(create_token))
        .route("/admin/enrollments", get(list_enrollments))
        .route(
            "/admin/enrollments/{id}",
            axum::routing::delete(delete_enrollment),
        )
        .route("/admin/enroll/{id}/decide", post(decide_enrollment))
        .layer(axum::middleware::from_fn_with_state(
            st.clone(),
            admin_rate_limit,
        ));

    Router::new()
        .route("/enroll/challenge", post(enroll_challenge))
        .route("/enroll", post(enroll))
        .route("/enroll/{id}", get(enroll_status))
        .route("/crl", get(crl))
        .merge(admin)
        .with_state(st)
}

/// mTLS 端点路由。
pub fn mtls_router(st: AppState) -> Router {
    Router::new()
        .route("/enroll/confirm", post(confirm))
        .route("/messages", post(receive_message))
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
            cert_status: Arc::new(crate::cert_status::CertStatusCache::new()),
            admin_limiter: Arc::new(crate::ratelimit::RateLimiter::new(0)),
            mtls_endpoint: Arc::from("https://mtls.test:4441"),
            enroll_endpoint: Arc::from("https://enroll.test:4440"),
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
