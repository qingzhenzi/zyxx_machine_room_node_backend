//! 内部 CA：加载签名密钥、从 CSR 签发客户端证书、生成 RFC 5280 CRL。
//!
//! rcgen 0.13 的 `Issuer` 是私有类型，因此这里直接持有 CA `Certificate` + `KeyPair`，
//! 用 `signed_by(...)` 的公开重载完成签发。

use crate::error::{Error, Result};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, CertificateRevocationListParams,
    CertificateSigningRequestParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, RevocationReason, RevokedCertParams, SerialNumber,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use time::OffsetDateTime;

pub struct Ca {
    cert: Arc<Certificate>,
    key: Arc<KeyPair>,
    ca_cert_pem: String,
    ca_cert_der: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct IssuedCert {
    pub serial_hex: String,
    pub cert_pem: String,
    pub cert_der: Vec<u8>,
}

/// serial 的**规范 hex**：去掉前导零。
///
/// 为什么必须规范化：DER 编码 INTEGER 时会剥掉前导零（首字节高位为 1 时再补
/// 一个 0x00），所以证书里读出来的 `raw_serial()` 与生成时用的 20 字节在
/// 约 1/256 的情况下不等长。签名时记库的是前者、握手时查缓存的是后者的话，
/// 吊销状态就会在那些证书上永远对不上。两侧统一走这个函数。
pub fn canonical_serial_hex(raw: &[u8]) -> String {
    let trimmed = raw
        .iter()
        .skip_while(|b| **b == 0)
        .copied()
        .collect::<Vec<u8>>();
    // 全零的退化输入也要给出确定结果（空串），避免两侧口径不同
    hex::encode(trimmed)
}

/// 吊销条目，用于 CRL 生成。
#[derive(Debug, Clone)]
pub struct RevokedEntry {
    pub serial_hex: String,
    pub revoked_at: OffsetDateTime,
    pub reason: RevocationReason,
}

impl Ca {
    /// 从 PEM 加载 CA 私钥与证书。
    ///
    /// 生产环境必须用这个入口加载持久化的 CA（否则重启后已签发证书无法验证）。
    pub fn load(key_pem: &str, cert_pem: &str) -> Result<Self> {
        let key = KeyPair::from_pem(key_pem)
            .map_err(|e| Error::BadRequest(format!("invalid CA key PEM: {e}")))?;
        // 从 PEM 重建 Certificate：重新 self_signed 会保留原有 subject/扩展
        let params = CertificateParams::from_ca_cert_pem(cert_pem)
            .map_err(|e| Error::BadRequest(format!("invalid CA cert PEM: {e}")))?;
        let cert = params
            .self_signed(&key)
            .map_err(|e| Error::BadRequest(format!("cannot rebuild CA cert: {e}")))?;
        let ca_cert_der = cert.der().to_vec();
        Ok(Self {
            cert: Arc::new(cert),
            key: Arc::new(key),
            ca_cert_pem: cert_pem.to_owned(),
            ca_cert_der,
        })
    }

    /// 生成新的自签名内部 CA（仅开发/测试；生产用 `load`）。
    pub fn generate_self_signed(common_name: &str) -> Result<Self> {
        let key = KeyPair::generate().map_err(|e| Error::Internal(e.into()))?;
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let now = OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(3650);
        let cert = params
            .self_signed(&key)
            .map_err(|e| Error::Internal(e.into()))?;
        let cert_pem = cert.pem();
        let key_pem = key.serialize_pem();
        Self::load(&key_pem, &cert_pem)
    }

    /// 或者直接用已有的 key + params 构造（跳过 PEM 往返，测试友好）。
    pub fn from_parts(key: KeyPair, params: CertificateParams) -> Result<Self> {
        let cert = params
            .self_signed(&key)
            .map_err(|e| Error::Internal(e.into()))?;
        let ca_cert_pem = cert.pem();
        let ca_cert_der = cert.der().to_vec();
        Ok(Self {
            cert: Arc::new(cert),
            key: Arc::new(key),
            ca_cert_pem,
            ca_cert_der,
        })
    }

    pub fn cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    pub fn cert_der(&self) -> &[u8] {
        &self.ca_cert_der
    }

    /// 从 CSR DER 签发客户端证书。
    ///
    /// `common_name` 由服务端决定（不信任 CSR 自称的身份）。
    pub fn sign_csr(&self, csr_der: &[u8], common_name: &str, days: i64) -> Result<IssuedCert> {
        let csr_der = rustls::pki_types::CertificateSigningRequestDer::from(csr_der.to_vec());
        let csr = CertificateSigningRequestParams::from_der(&csr_der)
            .map_err(|e| Error::BadRequest(format!("invalid CSR DER: {e}")))?;

        let mut serial_raw = [0u8; 20];
        rand::Rng::fill(&mut rand::rng(), &mut serial_raw);
        serial_raw[0] &= 0x7f; // 正整数
        // 记库用规范口径，与握手时从证书里读出来的 serial 对得上
        let serial_hex = canonical_serial_hex(&serial_raw);

        let mut params = csr.params;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        params.distinguished_name = dn;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.serial_number = Some(SerialNumber::from_slice(&serial_raw));
        let now = OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::minutes(5);
        params.not_after = now + time::Duration::days(days.max(1));

        let cert = params
            .signed_by(&csr.public_key, &self.cert, &self.key)
            .map_err(|e| Error::Internal(e.into()))?;

        Ok(IssuedCert {
            serial_hex,
            cert_pem: cert.pem(),
            cert_der: cert.der().to_vec(),
        })
    }

    /// 生成签名 CRL（RFC 5280）。
    pub fn build_crl(&self, revoked: &[RevokedEntry], valid_days: i64) -> Result<String> {
        let now = OffsetDateTime::now_utc();
        let mut serial_raw = [0u8; 20];
        rand::Rng::fill(&mut rand::rng(), &mut serial_raw);
        serial_raw[0] &= 0x7f;

        // 序列号按 20 字节大端转回字节
        let revoked_certs = revoked
            .iter()
            .map(|r| {
                let serial = hex::decode(&r.serial_hex).unwrap_or_default();
                RevokedCertParams {
                    serial_number: SerialNumber::from_slice(&serial),
                    revocation_time: r.revoked_at,
                    reason_code: Some(r.reason),
                    invalidity_date: None,
                }
            })
            .collect();

        let params = CertificateRevocationListParams {
            this_update: now,
            next_update: now + time::Duration::days(valid_days.max(1)),
            crl_number: SerialNumber::from_slice(&serial_raw),
            issuing_distribution_point: None,
            revoked_certs,
            key_identifier_method: rcgen::KeyIdMethod::Sha256,
        };

        let crl = params
            .signed_by(&self.cert, &self.key)
            .map_err(|e| Error::Internal(e.into()))?;
        crl.pem().map_err(|e| Error::Internal(e.into()))
    }

    /// 证书链 PEM（CA）。
    pub fn chain_pem(&self) -> String {
        self.ca_cert_pem.clone()
    }

    /// 构造 rustls 使用的证书链（leaf + CA）。
    pub fn chain_der(&self, leaf_der: &[u8]) -> Vec<CertificateDer<'static>> {
        vec![
            CertificateDer::from(leaf_der.to_vec()),
            CertificateDer::from(self.ca_cert_der.clone()),
        ]
    }
}

/// 把 `reason` 字符串转成 rcgen 的 RevocationReason。
pub fn parse_reason(s: &str) -> RevocationReason {
    match s {
        "key_compromise" => RevocationReason::KeyCompromise,
        "superseded" => RevocationReason::Superseded,
        "decommission" => RevocationReason::CessationOfOperation,
        "affiliation_changed" => RevocationReason::AffiliationChanged,
        "privilege_withdrawn" => RevocationReason::PrivilegeWithdrawn,
        "ca_compromise" => RevocationReason::CaCompromise,
        _ => RevocationReason::Unspecified,
    }
}

/// PEM → rustls 证书链。
pub fn pem_to_cert_chain(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut rd = std::io::BufReader::new(pem.as_bytes());
    let certs: std::result::Result<Vec<_>, _> = rustls_pemfile::certs(&mut rd).collect();
    certs.map_err(|e| Error::BadRequest(format!("invalid cert PEM: {e}")))
}

/// PEM → rustls 私钥。
pub fn pem_to_private_key(pem: &str) -> Result<PrivateKeyDer<'static>> {
    let mut rd = std::io::BufReader::new(pem.as_bytes());
    rustls_pemfile::private_key(&mut rd)
        .map_err(|e| Error::BadRequest(format!("invalid key PEM: {e}")))?
        .ok_or_else(|| Error::BadRequest("no private key found in PEM".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::BasicConstraints as RcgenBasicConstraints;
    use rcgen::{CertificateParams as CsrParams, KeyPair as CsrKey};
    use x509_parser::prelude::{FromDer, X509Certificate};

    fn make_csr(cn: &str) -> Vec<u8> {
        let key = CsrKey::generate().unwrap();
        let mut params = CsrParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, cn);
        params.distinguished_name = dn;
        let csr = params.serialize_request(&key).unwrap();
        csr.der().to_vec()
    }

    #[test]
    fn self_signed_ca_roundtrip() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        assert!(ca.cert_pem().contains("BEGIN CERTIFICATE"));
        assert!(!ca.cert_der().is_empty());
    }

    #[test]
    fn ca_reload_from_pem_is_stable() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Reload CA");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(RcgenBasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params.self_signed(&key).unwrap();
        let reloaded = Ca::load(&key.serialize_pem(), &cert.pem()).unwrap();
        assert_eq!(reloaded.cert_pem(), cert.pem());
        drop(ca);
    }

    #[test]
    fn sign_csr_issues_client_auth_cert() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        let csr_der = make_csr("ignored-cn");
        let issued = ca.sign_csr(&csr_der, "slot-1", 365).unwrap();
        assert!(issued.cert_pem.contains("BEGIN CERTIFICATE"));
        assert_eq!(issued.serial_hex.len(), 40);

        let (_, cert) = X509Certificate::from_der(&issued.cert_der).unwrap();
        assert_eq!(cert.subject().to_string(), "CN=slot-1");
        let eku = cert.extended_key_usage().unwrap().expect("eku present");
        assert!(eku.value.client_auth);
        // 验证确实由该 CA 签发
        let (_, ca_cert) = X509Certificate::from_der(ca.cert_der()).unwrap();
        assert!(cert.verify_signature(Some(ca_cert.public_key())).is_ok());
    }

    #[test]
    fn sign_csr_rejects_garbage() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        assert!(ca.sign_csr(b"not-a-csr", "x", 30).is_err());
    }

    #[test]
    fn serial_is_positive_and_unique() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        let a = ca.sign_csr(&make_csr("a"), "a", 30).unwrap();
        let b = ca.sign_csr(&make_csr("a"), "a", 30).unwrap();
        assert_ne!(a.serial_hex, b.serial_hex);
        let first = u8::from_str_radix(&a.serial_hex[..2], 16).unwrap();
        assert!(first & 0x80 == 0);
    }

    #[test]
    fn canonical_serial_strips_leading_zeros() {
        assert_eq!(canonical_serial_hex(&[0x00, 0x00, 0xab]), "ab");
        assert_eq!(canonical_serial_hex(&[0x7f, 0x00]), "7f00");
        assert_eq!(canonical_serial_hex(&[0x00]), "");
        assert_eq!(canonical_serial_hex(&[]), "");
    }

    /// 签发时记下的 serial 必须和证书里读出来的完全一致 —— 否则吊销/可用性
    /// 检查在某些证书上（DER 剥掉前导零的那约 1/256）会永远对不上。
    #[test]
    fn recorded_serial_matches_certificate_bytes() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        for _ in 0..64 {
            let issued = ca.sign_csr(&make_csr("a"), "a", 30).unwrap();
            let (_, cert) = X509Certificate::from_der(&issued.cert_der).unwrap();
            assert_eq!(
                canonical_serial_hex(cert.tbs_certificate.raw_serial()),
                issued.serial_hex
            );
        }
    }

    #[test]
    fn crl_contains_revoked_serials() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        let now = OffsetDateTime::now_utc();
        let entries = vec![
            RevokedEntry {
                serial_hex: "01aa".into(),
                revoked_at: now,
                reason: RevocationReason::Superseded,
            },
            RevokedEntry {
                serial_hex: "02bb".into(),
                revoked_at: now,
                reason: RevocationReason::CessationOfOperation,
            },
        ];
        let crl_pem = ca.build_crl(&entries, 7).unwrap();
        assert!(crl_pem.contains("BEGIN X509 CRL"));
        assert!(crl_pem.contains("END X509 CRL"));
    }

    #[test]
    fn crl_empty_is_valid() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        let crl_pem = ca.build_crl(&[], 7).unwrap();
        assert!(crl_pem.contains("BEGIN X509 CRL"));
    }

    #[test]
    fn reason_parsing() {
        assert!(matches!(
            parse_reason("superseded"),
            RevocationReason::Superseded
        ));
        assert!(matches!(
            parse_reason("unknown-thing"),
            RevocationReason::Unspecified
        ));
    }

    #[test]
    fn pem_helpers_work() {
        let ca = Ca::generate_self_signed("Test Device CA").unwrap();
        let chain = pem_to_cert_chain(ca.cert_pem()).unwrap();
        assert_eq!(chain.len(), 1);
    }
}
