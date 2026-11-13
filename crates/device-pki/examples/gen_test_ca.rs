//! 生成测试用 CA 与服务端 TLS 证书（仅本地联调用，勿用于生产）。
//!
//! 默认 SAN 为 `localhost` + `127.0.0.1`；跨机/局域网联调时可用环境变量
//! `DEVICE_PKI_EXTRA_SANS`（逗号分隔的 IP 或域名）或命令行追加参数补 SAN，例如：
//! `DEVICE_PKI_EXTRA_SANS="192.168.8.141,kerno.local" bash scripts/dev-pki-certs.sh`

use anyhow::Result;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType,
};
use std::net::IpAddr;

fn main() -> Result<()> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "certs".into());
    std::fs::create_dir_all(&out)?;

    // 额外 SAN：环境变量 + 命令行参数（追加在默认的 localhost/127.0.0.1 之后）。
    let mut extra_sans: Vec<String> = std::env::var("DEVICE_PKI_EXTRA_SANS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    extra_sans.extend(std::env::args().skip(2));
    if !extra_sans.is_empty() {
        println!("额外 SAN：{}", extra_sans.join(", "));
    }

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
        let mut sans = vec![
            SanType::DnsName("localhost".try_into()?),
            SanType::IpAddress(IpAddr::from([127, 0, 0, 1])),
        ];
        for raw in &extra_sans {
            match raw.parse::<IpAddr>() {
                Ok(ip) => sans.push(SanType::IpAddress(ip)),
                Err(_) => sans.push(SanType::DnsName(raw.as_str().try_into()?)),
            }
        }
        sp.subject_alt_names = sans;
        let cert = sp.signed_by(&skey, &ca_cert, &ca_key)?;
        std::fs::write(format!("{out}/{name}.pem"), cert.pem())?;
        std::fs::write(format!("{out}/{name}-key.pem"), skey.serialize_pem())?;
    }

    println!("证书已生成到 {out}");
    Ok(())
}
