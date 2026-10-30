//! 本地凭证持久化：证书、私钥、CA 链，全部 0600。

use crate::error::{Error, Result};
use crate::trust::{default_data_dir, write_private};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Credentials {
    pub device_slot: String,
    pub serial: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub ca_chain_pem: String,
    pub endpoint: String,
}

impl Credentials {
    /// 默认落盘目录：`<data_dir>/creds/<slot>/`
    pub fn dir_for(slot: &str) -> Result<PathBuf> {
        Ok(default_data_dir()?.join("creds").join(sanitize(slot)))
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        write_private(&dir.join("cert.pem"), self.cert_pem.as_bytes())?;
        write_private(&dir.join("key.pem"), self.key_pem.as_bytes())?;
        write_private(&dir.join("ca-chain.pem"), self.ca_chain_pem.as_bytes())?;
        let meta = serde_json::json!({
            "device_slot": self.device_slot,
            "serial": self.serial,
            "endpoint": self.endpoint,
        });
        write_private(
            &dir.join("meta.json"),
            serde_json::to_string_pretty(&meta)?.as_bytes(),
        )?;
        Ok(())
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let cert_pem = read_required(&dir.join("cert.pem"))?;
        let key_pem = read_required(&dir.join("key.pem"))?;
        let ca_chain_pem = read_required(&dir.join("ca-chain.pem"))?;
        let meta_raw = read_required(&dir.join("meta.json"))?;
        let meta: serde_json::Value = serde_json::from_str(&meta_raw)?;
        Ok(Self {
            device_slot: meta["device_slot"].as_str().unwrap_or_default().to_owned(),
            serial: meta["serial"].as_str().unwrap_or_default().to_owned(),
            cert_pem,
            key_pem,
            ca_chain_pem,
            endpoint: meta["endpoint"].as_str().unwrap_or_default().to_owned(),
        })
    }

    pub fn exists(dir: &Path) -> bool {
        dir.join("cert.pem").exists() && dir.join("key.pem").exists()
    }

    /// 私钥 PEM 用 PKCS#8 包装，便于 rustls 直接解析。
    pub fn cert_chain_der(&self) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
        let mut rd = std::io::BufReader::new(self.cert_pem.as_bytes());
        let certs: std::result::Result<Vec<_>, _> = rustls_pemfile::certs(&mut rd).collect();
        certs.map_err(|e| Error::Storage(format!("parse cert.pem: {e}")))
    }

    pub fn private_key_der(&self) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
        let mut rd = std::io::BufReader::new(self.key_pem.as_bytes());
        rustls_pemfile::private_key(&mut rd)
            .map_err(|e| Error::Storage(format!("parse key.pem: {e}")))?
            .ok_or_else(|| Error::Storage("key.pem 中没有私钥".into()))
    }

    pub fn ca_roots(&self) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
        let mut rd = std::io::BufReader::new(self.ca_chain_pem.as_bytes());
        let certs: std::result::Result<Vec<_>, _> = rustls_pemfile::certs(&mut rd).collect();
        certs.map_err(|e| Error::Storage(format!("parse ca-chain.pem: {e}")))
    }
}

fn read_required(p: &Path) -> Result<String> {
    std::fs::read_to_string(p).map_err(|e| {
        Error::Storage(format!("读取 {} 失败: {e}（是否已完成注册？）", p.display()))
    })
}

/// 把 slot 名转成安全目录名。
fn sanitize(slot: &str) -> String {
    slot.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Credentials {
        Credentials {
            device_slot: "slot-1".into(),
            serial: "abcd".into(),
            cert_pem: "-----BEGIN CERTIFICATE-----\nX\n-----END CERTIFICATE-----\n".into(),
            key_pem: "-----BEGIN PRIVATE KEY-----\nY\n-----END PRIVATE KEY-----\n".into(),
            ca_chain_pem: "-----BEGIN CERTIFICATE-----\nZ\n-----END CERTIFICATE-----\n".into(),
            endpoint: "node:4440".into(),
        }
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let creds = sample();
        creds.save(dir.path()).unwrap();
        assert!(Credentials::exists(dir.path()));
        let loaded = Credentials::load(dir.path()).unwrap();
        assert_eq!(loaded.device_slot, "slot-1");
        assert_eq!(loaded.serial, "abcd");
        assert_eq!(loaded.cert_pem, creds.cert_pem);
        assert_eq!(loaded.endpoint, "node:4440");
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        sample().save(dir.path()).unwrap();
        for name in ["cert.pem", "key.pem", "ca-chain.pem", "meta.json"] {
            let mode = std::fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name} 应为 0600，实际 {mode:o}");
        }
        let dir_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn load_missing_errors_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let err = Credentials::load(dir.path()).unwrap_err();
        assert!(matches!(err, Error::Storage(_)));
        assert!(!Credentials::exists(dir.path()));
    }

    #[test]
    fn slot_sanitized_for_path() {
        assert_eq!(sanitize("a/b c"), "a_b_c");
        assert_eq!(sanitize("slot-1"), "slot-1");
    }
}
