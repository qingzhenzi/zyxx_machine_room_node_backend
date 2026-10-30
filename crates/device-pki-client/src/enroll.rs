//! 注册流程（契约第 3、4 节）。
//!
//! ```text
//! 解析链接 → POST /enroll/challenge → 生成密钥 + CSR + PoP
//!          → POST /enroll → auto: 直接落盘 / approval: 轮询后一次性下载 → 落盘
//! ```
//!
//! 注册成功后建议立即 `POST /enroll/confirm`（走 mTLS），那才算通讯真正打通。

use crate::creds::Credentials;
use crate::error::{Error, Result};
use crate::transcript::{self, POP_ALGORITHM};
use crate::trust::{Pin, TrustOutcome, TrustStore};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// 从注册链接解析出的上下文。
#[derive(Debug, Clone)]
pub struct EnrollTarget {
    pub endpoint: String,
    pub token: String,
    pub slot: String,
    pub pin: Option<Pin>,
}

impl EnrollTarget {
    /// 解析 `https://host:port/enroll?token=T&slot=S[&pin=sha256/...]`
    pub fn parse(link: &str) -> Result<Self> {
        let (scheme, rest) = link
            .split_once("://")
            .ok_or_else(|| Error::InvalidLink("缺少 scheme".into()))?;
        if scheme != "https" && scheme != "http" {
            return Err(Error::InvalidLink(format!("不支持的 scheme: {scheme}")));
        }
        let (authority, path) = rest
            .split_once('/')
            .ok_or_else(|| Error::InvalidLink("缺少路径".into()))?;
        let query = path
            .split_once('?')
            .map(|(_, q)| q)
            .ok_or_else(|| Error::InvalidLink("缺少查询参数".into()))?;

        let mut token = None;
        let mut slot = None;
        let mut pin = None;
        for pair in query.split('&') {
            let (k, v) = match pair.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            let v = percent_decode(v);
            match k {
                "token" => token = Some(v),
                "slot" => slot = Some(v),
                "pin" => pin = Some(Pin::parse(&v)?),
                _ => {}
            }
        }

        Ok(Self {
            endpoint: format!("{scheme}://{authority}"),
            token: token.ok_or_else(|| Error::InvalidLink("缺少 token".into()))?,
            slot: slot.ok_or_else(|| Error::InvalidLink("缺少 slot".into()))?,
            pin,
        })
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, Serialize)]
struct ClientInfo {
    hostname: String,
    machine_id: String,
}

#[derive(Debug, Serialize)]
struct ChallengeRequest<'a> {
    token: &'a str,
    slot: &'a str,
    client_info: ClientInfo,
}

#[derive(Debug, Deserialize)]
struct ChallengeReply {
    challenge_id: String,
    challenge: String,
    ttl_secs: u64,
    kind: String,
}

#[derive(Debug, Serialize)]
struct EnrollRequest<'a> {
    challenge_id: &'a str,
    slot: &'a str,
    token: &'a str,
    csr_der: String,
    pop_signature: String,
    pop_algorithm: &'a str,
    tls_cert_seen_fp: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum EnrollReply {
    Issued {
        cert_pem: String,
        ca_chain_pem: String,
        serial: String,
        #[serde(default)]
        mtls_endpoint: Option<String>,
    },
    Pending {
        enroll_id: String,
        poll_after_secs: u64,
    },
}

#[derive(Debug, Deserialize)]
struct StatusReply {
    status: String,
    cert_pem: Option<String>,
    ca_chain_pem: Option<String>,
    serial: Option<String>,
    poll_after_secs: Option<u64>,
}

#[derive(Debug)]
pub enum EnrollOutcome {
    Issued(Credentials),
    PendingApproval { enroll_id: String },
}

/// 注册选项。
#[derive(Debug, Clone)]
pub struct EnrollOptions {
    /// approval 型最长等待多久（None = 不等待，直接返回 pending）
    pub approval_wait: Option<Duration>,
    /// 用户已确认信任（TOFU 场景由 CLI 传入）
    pub trust_confirmed: bool,
    /// 数据目录覆盖（测试用）
    pub data_dir: Option<std::path::PathBuf>,
}

impl Default for EnrollOptions {
    fn default() -> Self {
        Self {
            approval_wait: Some(Duration::from_secs(300)),
            trust_confirmed: false,
            data_dir: None,
        }
    }
}

/// 执行完整注册。
pub async fn enroll(target: &EnrollTarget, opts: &EnrollOptions) -> Result<EnrollOutcome> {
    let http = build_http_client(target)?;

    // 0. 先探一次服务端身份（拿 tls_cert_seen_fp）
    let trust = verify_server_identity(target, &http, opts).await?;

    // 1. 请求挑战值
    let client_info = collect_client_info();
    let challenge_url = format!("{}/enroll/challenge", target.endpoint);
    let resp = http
        .post(&challenge_url)
        .json(&ChallengeRequest {
            token: &target.token,
            slot: &target.slot,
            client_info,
        })
        .send()
        .await
        .map_err(|e| Error::Transport(e.to_string()))?;
    let resp = check_status(resp).await?;
    let chal: ChallengeReply = resp
        .json()
        .await
        .map_err(|e| Error::Transport(e.to_string()))?;

    // 2. 生成密钥 + CSR + PoP
    let key = transcript::DeviceKey::generate()?;
    let csr_der = build_csr(&target.slot, &key)?;
    let csr_sha = transcript::sha256(&csr_der);
    let token_hash = transcript::sha256(target.token.as_bytes());
    let challenge_bytes = hex::decode(&chal.challenge)
        .map_err(|e| Error::Unexpected(format!("challenge 不是合法 hex: {e}")))?;
    let fp_bytes = hex::decode(&trust.seen_fp).unwrap_or_default();

    let msg = transcript::canonical_transcript(
        &challenge_bytes,
        &token_hash,
        &target.slot,
        &csr_sha,
        &fp_bytes,
    );
    let pop_sig = key.sign(&msg)?;

    // 3. 提交注册
    let b64 = base64::engine::general_purpose::STANDARD;
    let enroll_url = format!("{}/enroll", target.endpoint);
    let resp = http
        .post(&enroll_url)
        .json(&EnrollRequest {
            challenge_id: &chal.challenge_id,
            slot: &target.slot,
            token: &target.token,
            csr_der: b64.encode(&csr_der),
            pop_signature: b64.encode(&pop_sig),
            pop_algorithm: POP_ALGORITHM,
            tls_cert_seen_fp: trust.seen_fp.clone(),
        })
        .send()
        .await
        .map_err(|e| Error::Transport(e.to_string()))?;
    let resp = check_status(resp).await?;
    let reply: EnrollReply = resp
        .json()
        .await
        .map_err(|e| Error::Transport(e.to_string()))?;

    match reply {
        EnrollReply::Issued {
            cert_pem,
            ca_chain_pem,
            serial,
            mtls_endpoint,
        } => {
            let creds = persist(
                target,
                &key,
                cert_pem,
                ca_chain_pem,
                serial,
                mtls_endpoint,
                opts,
            )?;
            Ok(EnrollOutcome::Issued(creds))
        }
        EnrollReply::Pending {
            enroll_id,
            poll_after_secs,
        } => match opts.approval_wait {
            None => Ok(EnrollOutcome::PendingApproval { enroll_id }),
            Some(max_wait) => {
                let (cert_pem, ca_chain_pem, serial) =
                    poll_until_approved(&http, target, &enroll_id, poll_after_secs, max_wait)
                        .await?;
                // approval 型下载路径没有单独的 mtls_endpoint 字段，
                // 用注册端点主机名 + 服务端约定端口无法推断，交给 persist 回退
                let creds = persist(
                    target,
                    &key,
                    cert_pem,
                    ca_chain_pem,
                    serial,
                    None,
                    opts,
                )?;
                Ok(EnrollOutcome::Issued(creds))
            }
        },
    }
}

fn persist(
    target: &EnrollTarget,
    key: &transcript::DeviceKey,
    cert_pem: String,
    ca_chain_pem: String,
    serial: String,
    mtls_endpoint: Option<String>,
    opts: &EnrollOptions,
) -> Result<Credentials> {
    let key_pem = key_to_pem(key)?;
    let creds = Credentials {
        device_slot: target.slot.clone(),
        serial,
        cert_pem,
        key_pem,
        ca_chain_pem,
        endpoint: target.endpoint.clone(),
        // 服务端返回的 mTLS 端点优先；缺失时退化为注册端点（并打警告）
        mtls_endpoint: match mtls_endpoint {
            Some(e) if !e.is_empty() => e,
            _ => {
                tracing::warn!(
                    "服务端未返回 mtls_endpoint，回退到注册端点 —— \
                     confirm 很可能失败，请检查服务端 PUBLIC_MTLS_ENDPOINT 配置"
                );
                target.endpoint.clone()
            }
        },
    };
    let dir = match &opts.data_dir {
        Some(d) => d.join("creds").join(&target.slot),
        None => Credentials::dir_for(&target.slot)?,
    };
    creds.save(&dir)?;
    Ok(creds)
}

async fn poll_until_approved(
    http: &reqwest::Client,
    target: &EnrollTarget,
    enroll_id: &str,
    poll_after_secs: u64,
    max_wait: Duration,
) -> Result<(String, String, String)> {
    let deadline = std::time::Instant::now() + max_wait;
    let interval = Duration::from_secs(poll_after_secs.max(1));
    let url = format!("{}/enroll/{enroll_id}", target.endpoint);

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(Error::PendingApproval(enroll_id.to_owned()));
        }
        tokio::time::sleep(interval).await;

        let resp = http
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::GONE {
            return Err(Error::AlreadyDownloaded);
        }
        let resp = check_status(resp).await?;
        let st: StatusReply = resp
            .json()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        match st.status.as_str() {
            "pending" => continue,
            "rejected" => return Err(Error::ApprovalRejected),
            "approved" => {
                let cert = st
                    .cert_pem
                    .ok_or_else(|| Error::Unexpected("approved 但无 cert_pem".into()))?;
                let ca = st
                    .ca_chain_pem
                    .ok_or_else(|| Error::Unexpected("approved 但无 ca_chain_pem".into()))?;
                let serial = st
                    .serial
                    .ok_or_else(|| Error::Unexpected("approved 但无 serial".into()))?;
                return Ok((cert, ca, serial));
            }
            other => return Err(Error::Unexpected(format!("未知状态 {other}"))),
        }
    }
}

/// 构建 HTTP 客户端。接受自签/internal CA 服务端证书（信任由 pin/TOFU 承担）。
fn build_http_client(_target: &EnrollTarget) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // 身份校验见 verify_server_identity
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Transport(e.to_string()))
}

/// 服务端身份校验：pin → TOFU 记录 → 人工确认。
async fn verify_server_identity(
    target: &EnrollTarget,
    http: &reqwest::Client,
    opts: &EnrollOptions,
) -> Result<TrustOutcome> {
    // 主动拉一次 /crl 的连接证书指纹。reqwest 不直接暴露 TLS 对端证书，
    // 这里用 `Endpoint` 探测：失败时降级为"无指纹"（服务端会放过空指纹）。
    let endpoint_key = target.endpoint.clone();
    let store_path = match &opts.data_dir {
        Some(d) => d.join("trust-store.json"),
        None => TrustStore::path()?,
    };
    let mut store = TrustStore::load(&store_path)?;

    let seen = probe_server_cert(http, &target.endpoint).await?;

    let outcome = match seen {
        Some(der) => store.check(&endpoint_key, target.pin.as_ref(), &der)?,
        None => TrustOutcome {
            seen_fp: String::new(),
            needs_confirmation: target.pin.is_none(),
            reason: "无法获取服务端证书（将跳过指纹绑定）".into(),
        },
    };

    if outcome.needs_confirmation && !opts.trust_confirmed {
        return Err(Error::TrustRejected);
    }
    if outcome.needs_confirmation && opts.trust_confirmed && !outcome.seen_fp.is_empty() {
        store.remember(&endpoint_key, &outcome.seen_fp);
        store.save(&store_path)?;
    }
    Ok(outcome)
}

/// 探测服务端证书 DER。纯 Rust TLS 栈不暴露对端证书时返回 None。
async fn probe_server_cert(
    _http: &reqwest::Client,
    endpoint: &str,
) -> Result<Option<Vec<u8>>> {
    // 用 tokio-rustls 直连一次，从连接里读对端证书。
    let host_port = endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .to_owned();

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    // 主动接受自签证书：这里只取证书做指纹比对，信任判定在 TrustStore
    cfg.dangerous()
        .set_certificate_verifier(std::sync::Arc::new(AcceptAnyCert));
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(cfg));

    let server_name = match host_port.split(':').next() {
        Some(h) => rustls::pki_types::ServerName::try_from(h.to_owned())
            .map_err(|e| Error::Transport(format!("invalid host: {e}")))?,
        None => return Ok(None),
    };

    let tcp = match tokio::net::TcpStream::connect(&host_port).await {
        Ok(t) => t,
        Err(e) => return Err(Error::Transport(format!("connect {host_port}: {e}"))),
    };
    let tls = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| Error::Transport(format!("tls handshake: {e}")))?;
    let der = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|c| c.first())
        .map(|c| c.as_ref().to_vec());
    Ok(der)
}

/// 接受任意证书的 verifier —— 仅用于"先取证书再比对指纹"的场景。
#[derive(Debug)]
struct AcceptAnyCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
    {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
    {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let code = resp
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v["code"].as_str().map(|s| s.to_owned()))
        .unwrap_or_default();
    Err(match code.as_str() {
        "token_invalid" => Error::TokenInvalid,
        "slot_conflict" => Error::SlotConflict,
        "challenge_expired" => Error::ChallengeExpired,
        "challenge_consumed" => Error::ChallengeConsumed,
        "challenge_mismatch" => Error::ChallengeMismatch,
        "pop_invalid" => Error::PopRejected,
        "fp_mismatch" => Error::FingerprintMismatch,
        "approval_rejected" => Error::ApprovalRejected,
        "already_downloaded" => Error::AlreadyDownloaded,
        other => Error::Unexpected(format!("HTTP {status} code={other}")),
    })
}

fn collect_client_info() -> ClientInfo {
    ClientInfo {
        hostname: hostname(),
        machine_id: machine_id(),
    }
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "unknown".into())
}

fn machine_id() -> String {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// 用 rcgen 生成 CSR（复用 transcript 里的 P-256 私钥）。
fn build_csr(slot: &str, key: &transcript::DeviceKey) -> Result<Vec<u8>> {
    use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

    // 把 PKCS#8 交给 rcgen
    let key_pem = key_to_pem(key)?;
    let rc_key = KeyPair::from_pem(&key_pem)
        .map_err(|e| Error::Crypto(format!("rcgen load key: {e}")))?;

    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, slot);
    params.distinguished_name = dn;

    let csr = params
        .serialize_request(&rc_key)
        .map_err(|e| Error::Crypto(format!("build CSR: {e}")))?;
    Ok(csr.der().to_vec())
}

/// PKCS#8 → PEM。
fn key_to_pem(key: &transcript::DeviceKey) -> Result<String> {
    use base64::engine::general_purpose::STANDARD;
    let b64 = STANDARD.encode(key.pkcs8());
    let mut out = String::from("-----BEGIN PRIVATE KEY-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END PRIVATE KEY-----\n");
    Ok(out)
}

/// 便捷入口：解析链接 + 注册。
pub async fn enroll_link(link: &str, opts: &EnrollOptions) -> Result<EnrollOutcome> {
    let target = EnrollTarget::parse(link)?;
    enroll(&target, opts).await
}

/// 供 CLI 显示链接摘要（不泄漏 token）。
pub fn link_summary(link: &str) -> Result<String> {
    let t = EnrollTarget::parse(link)?;
    Ok(format!(
        "endpoint={} slot={} pin={} token={}...",
        t.endpoint,
        t.slot,
        t.pin.is_some(),
        &t.token.chars().take(6).collect::<String>()
    ))
}

/// 落盘目录解析（CLI 用）。
pub fn creds_dir(slot: &str, data_dir: Option<&Path>) -> Result<std::path::PathBuf> {
    match data_dir {
        Some(d) => Ok(d.join("creds").join(slot)),
        None => Credentials::dir_for(slot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_link_full() {
        let t = EnrollTarget::parse(
            "https://node.example:4440/enroll?token=abc123&slot=slot-1&pin=sha256/aa",
        );
        // pin 太短应报错
        assert!(t.is_err());

        let t = EnrollTarget::parse("https://node.example:4440/enroll?token=abc123&slot=slot-1")
            .unwrap();
        assert_eq!(t.endpoint, "https://node.example:4440");
        assert_eq!(t.token, "abc123");
        assert_eq!(t.slot, "slot-1");
        assert!(t.pin.is_none());
    }

    #[test]
    fn parse_link_with_valid_pin() {
        let pin_hex = "cd".repeat(32);
        let t = EnrollTarget::parse(&format!(
            "https://node:4440/enroll?token=t&slot=s&pin=sha256/{pin_hex}"
        ))
        .unwrap();
        assert_eq!(t.pin, Some(Pin::CertSha256(pin_hex)));
    }

    #[test]
    fn parse_link_rejects_bad_input() {
        assert!(EnrollTarget::parse("no-scheme").is_err());
        assert!(EnrollTarget::parse("ftp://x/y?token=a&slot=b").is_err());
        assert!(EnrollTarget::parse("https://x/enroll?slot=b").is_err());
        assert!(EnrollTarget::parse("https://x/enroll?token=a").is_err());
        assert!(EnrollTarget::parse("https://x/enroll").is_err());
    }

    #[test]
    fn percent_decoding_works() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn link_summary_hides_token() {
        let s = link_summary("https://n:1/enroll?token=supersecrettoken&slot=s").unwrap();
        assert!(s.contains("slot=s"));
        assert!(!s.contains("supersecrettoken"));
        assert!(s.contains("supert..."[..6].to_string().as_str()) || s.contains("supers..."));
    }

    #[test]
    fn csr_built_with_cn_equal_slot() {
        use x509_parser::prelude::*;
        let key = transcript::DeviceKey::generate().unwrap();
        let csr_der = build_csr("slot-7", &key).unwrap();
        let (_, csr) = X509CertificationRequest::from_der(&csr_der).unwrap();
        let info = &csr.certification_request_info;
        let cn = info.subject.iter_common_name().next().unwrap();
        assert_eq!(cn.as_str().unwrap(), "slot-7");
    }

    #[test]
    fn csr_public_key_is_full_spki_containing_raw_key() {
        let key = transcript::DeviceKey::generate().unwrap();
        let csr_der = build_csr("s", &key).unwrap();
        use x509_parser::prelude::*;
        let (_, csr) = X509CertificationRequest::from_der(&csr_der).unwrap();
        let spki = csr.certification_request_info.subject_pki.raw;
        // CSR 里是完整 SPKI（含 AlgorithmIdentifier），ring 给的是裸公钥位
        assert!(spki.len() > key.public_key_der().len());
        assert!(
            spki.ends_with(key.public_key_der()),
            "SPKI 应包含裸公钥作为结尾"
        );
    }

    #[test]
    fn key_to_pem_shape() {
        let key = transcript::DeviceKey::generate().unwrap();
        let pem = key_to_pem(&key).unwrap();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert!(pem.ends_with("-----END PRIVATE KEY-----\n"));
        // rcgen 应该能解析回去
        assert!(rcgen::KeyPair::from_pem(&pem).is_ok());
    }
}
