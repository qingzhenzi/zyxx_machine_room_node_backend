//! 生成测试用 CA 与服务端 TLS 证书（仅本地联调用，勿用于生产）。

use anyhow::Result;
use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType};
use std::net::IpAddr;

fn main() -> Result<()> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "certs".into());
    std::fs::create_dir_all(&out)?;

    // 1. 内部 CA（设备证书签发者）
    let ca_key = KeyPair::generate()?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Local Test Device CA");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(3650);
    let ca_cert = params.self_signed(&ca_key)?;
    std::fs::write(format!("{out}/ca.pem"), ca_cert.pem())?;
    std::fs::write(format!("{out}/ca-key.pem"), ca_key.serialize_pem())?;

    // 2. 服务端 TLS 证书（注册端点 + mTLS 端点各一张）
    for name in ["enroll", "mtls"] {
        let skey = KeyPair::generate()?;
        let mut sp = CertificateParams::default();
        let mut sdn = DistinguishedName::new();
        sdn.push(DnType::CommonName, "localhost");
        sp.distinguished_name = sdn;
        sp.subject_alt_names = vec![
            SanType::DnsName("localhost".try_into()?),
            SanType::IpAddress(IpAddr::from([127, 0, 0, 1])),
        ];
        let cert = sp.signed_by(&skey, &ca_cert, &ca_key)?;
        std::fs::write(format!("{out}/{name}.pem"), cert.pem())?;
        std::fs::write(format!("{out}/{name}-key.pem"), skey.serialize_pem())?;
    }

    println!("证书已生成到 {out}");
    Ok(())
}
