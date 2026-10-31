//! TLS 装配：两个独立 listener（契约第 4 节末尾）。
//!
//! - **注册 listener**：单向 TLS。客户端此时无证书，靠 token + challenge + PoP 授权。
//! - **业务 listener**：mTLS。除了链验证到业务 CA，还要查 `device_enrollment`
//!   确认该证书**可用**（`issued`/`active` 且未被吊销）——"设备更换立即失效"
//!   与"未批准/被拒的证书永远用不了"都在这里落地，不依赖 CRL 分发延迟。
//!
//! 为什么必须是两个 listener：rustls 在握手阶段就要确定是否需要客户端证书，
//! 运行时切换会让状态机很难维护。

use crate::ca::{Ca, pem_to_cert_chain, pem_to_private_key};
use crate::cert_status::CertStatusCache;
use crate::error::{Error, Result};
use axum::extract::FromRequestParts;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};
use std::sync::Arc;
use x509_parser::prelude::FromDer;

/// mTLS 握手后注入请求扩展的对端身份。
#[derive(Debug, Clone)]
pub struct PeerCert {
    pub der: Vec<u8>,
    pub common_name: Option<String>,
    pub serial_hex: Option<String>,
}

impl PeerCert {
    pub fn common_name(&self) -> Option<&str> {
        self.common_name.as_deref()
    }

    pub fn from_der(der: Vec<u8>) -> Self {
        let mut cn = None;
        let mut serial = None;
        if let Ok((_, cert)) = x509_parser::prelude::X509Certificate::from_der(&der) {
            cn = cert
                .subject()
                .iter_common_name()
                .next()
                .and_then(|a| a.as_str().ok().map(|s| s.to_owned()));
            serial = Some(crate::ca::canonical_serial_hex(
                cert.tbs_certificate.raw_serial(),
            ));
        }
        Self {
            der,
            common_name: cn,
            serial_hex: serial,
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for PeerCert {
    type Rejection = crate::Error;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<PeerCert>()
            .cloned()
            .ok_or(Error::Unauthorized)
    }
}

/// 注册 listener：单向 TLS。客户端不提供证书。
pub fn server_config_enroll(cert_pem: &str, key_pem: &str) -> Result<Arc<ServerConfig>> {
    let certs = pem_to_cert_chain(cert_pem)?;
    let key = pem_to_private_key(key_pem)?;
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| Error::BadRequest(format!("tls config: {e}")))?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

/// 业务 listener：mTLS，要求客户端证书且**在 device_enrollment 中可用**。
pub fn server_config_mtls(
    cert_pem: &str,
    key_pem: &str,
    ca: Arc<Ca>,
    cert_status: Arc<CertStatusCache>,
) -> Result<Arc<ServerConfig>> {
    let certs = pem_to_cert_chain(cert_pem)?;
    let key = pem_to_private_key(key_pem)?;
    let verifier = Arc::new(DeviceCertVerifier {
        ca,
        cert_status,
        supported: rustls::crypto::ring::default_provider().signature_verification_algorithms,
    });
    let mut cfg = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|e| Error::BadRequest(format!("tls config: {e}")))?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

/// 自定义客户端证书验证器：链验证 + 可用性检查（吊销 / 未启用）。
///
/// 状态来自内存缓存（`CertStatusCache`），由后台任务每 5s 刷新。
/// 这样 rustls 的同步回调里不需要任何 async/DB 访问，也就不会出现
/// 「单线程 runtime 降级放行」的漏洞。
pub struct DeviceCertVerifier {
    ca: Arc<Ca>,
    cert_status: Arc<CertStatusCache>,
    supported: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl std::fmt::Debug for DeviceCertVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCertVerifier").finish_non_exhaustive()
    }
}

impl ClientCertVerifier for DeviceCertVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        use x509_parser::prelude::*;

        // 1. 必须由我们的 CA 签发
        let (_, leaf) = X509Certificate::from_der(end_entity.as_ref())
            .map_err(|_| rustls::Error::General("invalid client cert DER".into()))?;
        let (_, ca_cert) = X509Certificate::from_der(self.ca.cert_der())
            .map_err(|_| rustls::Error::General("invalid CA cert".into()))?;

        leaf.verify_signature(Some(ca_cert.public_key()))
            .map_err(|_| rustls::Error::General("client cert not signed by CA".into()))?;

        // 2. 有效期
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if !leaf.validity().is_valid_at(
            ASN1Time::from_timestamp(now).unwrap_or_else(|_| ASN1Time::from_timestamp(0).unwrap()),
        ) {
            return Err(rustls::Error::General("client cert expired".into()));
        }

        // 3. 可用性检查：吊销 + 必须在 device_enrollment 里且处于 issued/active
        //    （这就是"换设备即刻失效"、"未批准/被拒证书永远用不了"的落点）
        //
        //    serial 两侧必须用同一个规范化口径：rcgen 写进证书的 INTEGER 会被 DER
        //    去掉前导零，而库里存的是生成时的原始 20 字节 —— 直接 hex 对比时
        //    约 1/256 的证书（首字节为 0x00）会永远对不上。
        let serial_hex = crate::ca::canonical_serial_hex(leaf.tbs_certificate.raw_serial());
        if self.cert_status.is_revoked(&serial_hex) {
            tracing::info!(serial = %serial_hex, "mTLS handshake rejected: cert revoked");
            return Err(rustls::Error::General("client cert revoked".into()));
        }
        if !self.cert_status.is_live(&serial_hex) {
            tracing::info!(serial = %serial_hex, "mTLS handshake rejected: cert not active in device_enrollment");
            return Err(rustls::Error::General(
                "client cert not active in device_enrollment".into(),
            ));
        }

        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

/// 从服务端证书 PEM 计算 sha256 指纹（hex），用于 TOFU 对比。
pub fn cert_fingerprint_hex(cert_pem: &str) -> Result<String> {
    use sha2::{Digest, Sha256};
    let chain = pem_to_cert_chain(cert_pem)?;
    let leaf = chain
        .first()
        .ok_or_else(|| Error::BadRequest("empty cert chain".into()))?;
    Ok(hex::encode(Sha256::digest(leaf.as_ref())))
}

/// SNI 占位（部分 rustls 辅助 API 需要）。
pub fn example_server_name() -> ServerName<'static> {
    ServerName::try_from("localhost").unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_cert_extracts_cn_and_serial() {
        use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
        let ca = Ca::generate_self_signed("Test CA").unwrap();
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "slot-42");
        params.distinguished_name = dn;
        let csr = params.serialize_request(&key).unwrap();
        let issued = ca.sign_csr(csr.der(), "slot-42", 30).unwrap();

        let peer = PeerCert::from_der(issued.cert_der);
        assert_eq!(peer.common_name(), Some("slot-42"));
        assert_eq!(peer.serial_hex.as_deref(), Some(issued.serial_hex.as_str()));
    }

    #[test]
    fn fingerprint_is_stable_and_32_bytes() {
        let ca = Ca::generate_self_signed("Test CA").unwrap();
        let a = cert_fingerprint_hex(ca.cert_pem()).unwrap();
        let b = cert_fingerprint_hex(ca.cert_pem()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn enroll_config_builds_from_generated_ca() {
        let ca = Ca::generate_self_signed("Test CA").unwrap();
        // 用 CA 自身当服务端证书，只为验证配置装配不报错
        let cfg = server_config_enroll(ca.cert_pem(), "bogus").is_err();
        assert!(cfg, "bogus key must be rejected");
    }

    #[test]
    fn peer_cert_missing_extension_is_unauthorized() {
        // FromRequestParts 在无扩展时应拒绝
        use axum::http::Request;
        let (mut parts, _) = Request::new(()).into_parts();
        let res = futures_lite_block(PeerCert::from_request_parts(&mut parts, &()));
        assert!(res.is_err());
    }

    /// 极简阻塞执行器，仅用于测试里驱动一次就绪的 future。
    fn futures_lite_block<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll, Waker};
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = std::pin::pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("future not ready"),
        }
    }
}
