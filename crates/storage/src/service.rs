//! S3 兼容对象存储适配：SigV4 预签名、对象读写与 manifest 签名。

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, TimeDelta, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use reqwest::Url;
use reqwest::header::CONTENT_TYPE;
use uuid::Uuid;

use crate::base64;
use crate::sigv4::{self, QueryPresign};

/// 服务端 `get_bytes` 允许的最大对象大小（P0 对象都很小）。
const MAX_OBJECT_BYTES: usize = 64 * 1024 * 1024;
const MIN_PRESIGN_TTL_SECS: u64 = 60;
const MAX_PRESIGN_TTL_SECS: u64 = 86_400;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// S3 兼容存储配置（与 `docs/storage-design.md` §14.1 一致）。
#[derive(Debug, Clone)]
pub struct StorageOptions {
    /// 例如 `http://127.0.0.1:9000`，不允许携带路径/query。
    pub endpoint: String,
    /// 默认 `us-east-1`。
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    /// RustFS / MinIO 用 `true`。
    pub path_style: bool,
    /// 60..=86400 秒。
    pub presign_ttl_secs: u64,
    /// 32 字节 Ed25519 seed 的 base64；`None` = 启动时随机生成（仅开发）。
    pub signing_key_base64: Option<String>,
}

/// 预签名结果：URL 与绝对过期时间。
#[derive(Debug, Clone)]
pub struct PresignedUrl {
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

/// S3 客户端 + Ed25519 manifest 签名器。
pub struct StorageService {
    client: reqwest::Client,
    options: StorageOptions,
    scheme: String,
    /// 基础 `Host`（含非默认端口），不含 bucket。
    host: String,
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl StorageService {
    pub fn new(options: StorageOptions) -> Result<Self> {
        let mut options = options;
        options.endpoint = options.endpoint.trim().trim_end_matches('/').to_owned();
        let endpoint = Url::parse(&options.endpoint).context("S3 endpoint 不是合法 URL")?;
        match endpoint.scheme() {
            "http" | "https" => {}
            other => bail!("S3 endpoint 仅支持 http/https，收到 {other}"),
        }
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            bail!("S3 endpoint 不能包含用户名或密码");
        }
        if endpoint.query().is_some() || endpoint.fragment().is_some() {
            bail!("S3 endpoint 不能包含 query 或 fragment");
        }
        if !matches!(endpoint.path(), "" | "/") {
            bail!("S3 endpoint 不能包含路径，请使用 http://host:port 形式");
        }

        let bucket = options.bucket.trim();
        if bucket.is_empty() {
            bail!("S3 bucket 不能为空");
        }
        let region = options.region.trim();
        if region.is_empty() {
            bail!("S3 region 不能为空");
        }
        if !(MIN_PRESIGN_TTL_SECS..=MAX_PRESIGN_TTL_SECS).contains(&options.presign_ttl_secs) {
            bail!(
                "presign_ttl_secs 需在 {MIN_PRESIGN_TTL_SECS}-{MAX_PRESIGN_TTL_SECS} 秒之间，收到 {}",
                options.presign_ttl_secs
            );
        }

        let scheme = endpoint.scheme().to_owned();
        let host = endpoint_host(&endpoint)?;
        let signing_key = load_signing_key(options.signing_key_base64.as_deref())?;
        let verifying_key = signing_key.verifying_key();
        options.bucket = bucket.to_owned();
        options.region = region.to_owned();

        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("构建 S3 HTTP 客户端失败")?;

        Ok(Self {
            client,
            options,
            scheme,
            host,
            signing_key,
            verifying_key,
        })
    }

    pub fn bucket(&self) -> &str {
        &self.options.bucket
    }

    /// Ed25519 公钥的 base64（32 字节，标准字母表 + padding）。
    pub fn public_key_base64(&self) -> String {
        base64::encode(self.verifying_key.as_bytes())
    }

    pub fn presign_get(&self, object_key: &str) -> Result<PresignedUrl> {
        self.presign_get_at(object_key, Utc::now())
    }

    pub fn presign_put(
        &self,
        object_key: &str,
        content_type: Option<&str>,
    ) -> Result<PresignedUrl> {
        if let Some(value) = content_type {
            // 预签名只覆盖 `host`，content-type 由上传方作为请求头携带；
            // 这里至少挡住空值与 CRLF 注入。
            if value.trim().is_empty() || value.contains('\r') || value.contains('\n') {
                bail!("content-type 非法");
            }
        }
        self.presign("PUT", object_key, Utc::now())
    }

    fn presign_get_at(&self, object_key: &str, timestamp: DateTime<Utc>) -> Result<PresignedUrl> {
        self.presign("GET", object_key, timestamp)
    }

    fn presign(
        &self,
        method: &str,
        object_key: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<PresignedUrl> {
        let path = self.object_path(object_key)?;
        let host = self.object_host();
        let url = sigv4::presign_url(&QueryPresign {
            scheme: &self.scheme,
            host: &host,
            path: &path,
            method,
            access_key: &self.options.access_key,
            secret_key: &self.options.secret_key,
            region: &self.options.region,
            timestamp,
            expires_secs: self.options.presign_ttl_secs,
        });
        Ok(PresignedUrl {
            url,
            expires_at: timestamp + TimeDelta::seconds(self.options.presign_ttl_secs as i64),
        })
    }

    /// 服务端读取：内部用预签名 GET + reqwest，返回完整字节（上限 64 MiB）。
    pub async fn get_bytes(&self, object_key: &str) -> Result<Vec<u8>> {
        let presigned = self.presign_get(object_key)?;
        let mut response = self
            .client
            .get(&presigned.url)
            .send()
            .await
            .map_err(|error| transport_error("请求 S3 对象失败", object_key, error))?;
        let status = response.status();
        if !status.is_success() {
            bail!("读取 S3 对象失败：{object_key}（HTTP {}）", status.as_u16());
        }
        if let Some(length) = response.content_length()
            && length > MAX_OBJECT_BYTES as u64
        {
            bail!("S3 对象超过 64 MiB 上限：{object_key}");
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| transport_error("读取 S3 响应体失败", object_key, error))?
        {
            if body.len() + chunk.len() > MAX_OBJECT_BYTES {
                bail!("S3 对象超过 64 MiB 上限：{object_key}");
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// 服务端写入：内部用预签名 PUT + reqwest，并携带 content-type。
    pub async fn put_bytes(
        &self,
        object_key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<()> {
        let presigned = self.presign_put(object_key, Some(content_type))?;
        let response = self
            .client
            .put(&presigned.url)
            .header(CONTENT_TYPE, content_type)
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|error| transport_error("写入 S3 对象失败", object_key, error))?;
        let status = response.status();
        if !status.is_success() {
            bail!("写入 S3 对象失败：{object_key}（HTTP {}）", status.as_u16());
        }
        Ok(())
    }

    /// Ed25519 对 manifest 原始字节签名，返回 64 字节。
    pub fn sign_manifest(&self, manifest_json: &[u8]) -> Result<Vec<u8>> {
        let signature: Signature = self.signing_key.sign(manifest_json);
        Ok(signature.to_bytes().to_vec())
    }

    /// 校验 manifest 签名；长度错误、格式错误或签名不匹配都返回 `false`。
    pub fn verify_manifest(&self, manifest_json: &[u8], signature: &[u8]) -> bool {
        let Ok(signature) = Signature::from_slice(signature) else {
            return false;
        };
        self.verifying_key
            .verify_strict(manifest_json, &signature)
            .is_ok()
    }

    /// 预签名 URL 中的 Host 头：path-style 用 endpoint 本身，否则用 `{bucket}.{host}`。
    fn object_host(&self) -> String {
        if self.options.path_style {
            self.host.clone()
        } else {
            format!("{}.{}", self.options.bucket, self.host)
        }
    }

    fn object_path(&self, object_key: &str) -> Result<String> {
        let object_key = object_key.trim();
        if object_key.is_empty() {
            bail!("object key 不能为空");
        }
        if object_key.starts_with('/') {
            bail!("object key 不能以 / 开头");
        }
        let encoded_key = sigv4::encode_path(object_key);
        Ok(if self.options.path_style {
            format!(
                "/{}/{}",
                sigv4::encode_component(&self.options.bucket),
                encoded_key
            )
        } else {
            format!("/{encoded_key}")
        })
    }
}

/// 解析 endpoint 的 host（IPv6 补方括号），保留非默认端口。
fn endpoint_host(endpoint: &Url) -> Result<String> {
    let host = endpoint
        .host_str()
        .filter(|host| !host.is_empty())
        .context("S3 endpoint 缺少主机名")?;
    let host = if host.starts_with('[') {
        host.to_owned()
    } else if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(match endpoint.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn load_signing_key(encoded: Option<&str>) -> Result<SigningKey> {
    match encoded.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            let seed =
                base64::decode(value).context("STORAGE_SIGNING_KEY_BASE64 不是合法的 base64")?;
            let seed: [u8; 32] = seed.as_slice().try_into().map_err(|_| {
                anyhow!(
                    "STORAGE_SIGNING_KEY_BASE64 必须是 32 字节 Ed25519 seed，收到 {} 字节",
                    seed.len()
                )
            })?;
            Ok(SigningKey::from_bytes(&seed))
        }
        // 开发默认：两次 UUIDv4（OS CSPRNG）拼出 32 字节 seed；生产应配置固定密钥。
        None => {
            let mut seed = [0u8; 32];
            seed[..16].copy_from_slice(Uuid::new_v4().as_bytes());
            seed[16..].copy_from_slice(Uuid::new_v4().as_bytes());
            Ok(SigningKey::from_bytes(&seed))
        }
    }
}

/// 去掉错误里的预签名 URL（带签名参数），避免泄漏到日志。
fn transport_error(context: &str, object_key: &str, error: reqwest::Error) -> anyhow::Error {
    anyhow!("{context}：{object_key}（{}）", error.without_url())
}

#[cfg(test)]
mod tests {
    use super::{StorageOptions, StorageService, base64};
    use chrono::{TimeZone, Utc};

    fn options() -> StorageOptions {
        StorageOptions {
            endpoint: "http://127.0.0.1:9000".to_owned(),
            region: "us-east-1".to_owned(),
            bucket: "qe".to_owned(),
            access_key: "test-access".to_owned(),
            secret_key: "test-secret".to_owned(),
            path_style: true,
            presign_ttl_secs: 900,
            signing_key_base64: None,
        }
    }

    fn build_service(options: StorageOptions) -> StorageService {
        StorageService::new(options).expect("测试配置应合法")
    }

    #[test]
    fn aws_reference_presigned_url_via_service() {
        let service = build_service(StorageOptions {
            endpoint: "https://s3.amazonaws.com".to_owned(),
            bucket: "examplebucket".to_owned(),
            access_key: "AKIAIOSFODNN7EXAMPLE".to_owned(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
            path_style: false,
            presign_ttl_secs: 86_400,
            ..options()
        });
        let timestamp = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let presigned = service.presign_get_at("test.txt", timestamp).unwrap();
        assert_eq!(
            presigned.url,
            "https://examplebucket.s3.amazonaws.com/test.txt?\
             X-Amz-Algorithm=AWS4-HMAC-SHA256&\
             X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&\
             X-Amz-Date=20130524T000000Z&\
             X-Amz-Expires=86400&\
             X-Amz-SignedHeaders=host&\
             X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
        assert_eq!(
            presigned.expires_at,
            timestamp + chrono::TimeDelta::seconds(86_400)
        );
    }

    #[test]
    fn path_style_url_shape() {
        let service = build_service(options());
        let timestamp = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let presigned = service.presign_get_at("objects/abc123", timestamp).unwrap();
        assert!(
            presigned.url.starts_with(
                "http://127.0.0.1:9000/qe/objects/abc123?\
                 X-Amz-Algorithm=AWS4-HMAC-SHA256&\
                 X-Amz-Credential=test-access%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&\
                 X-Amz-Date=20130524T000000Z&\
                 X-Amz-Expires=900&\
                 X-Amz-SignedHeaders=host&\
                 X-Amz-Signature="
            ),
            "实际 URL：{}",
            presigned.url
        );
    }

    #[test]
    fn ipv6_endpoint_keeps_brackets_in_host() {
        let service = build_service(StorageOptions {
            endpoint: "http://[::1]:9000".to_owned(),
            ..options()
        });
        let timestamp = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let presigned = service.presign_get_at("objects/x", timestamp).unwrap();
        assert!(
            presigned.url.starts_with("http://[::1]:9000/qe/objects/x?"),
            "实际 URL：{}",
            presigned.url
        );
    }

    #[test]
    fn service_is_send_and_sync_for_arc_sharing() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<StorageService>();
        assert_send_sync::<StorageOptions>();
    }

    #[test]
    fn virtual_host_url_shape() {
        let service = build_service(StorageOptions {
            endpoint: "https://s3.example.com".to_owned(),
            path_style: false,
            ..options()
        });
        let timestamp = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let presigned = service.presign_get_at("objects/abc", timestamp).unwrap();
        assert!(
            presigned
                .url
                .starts_with("https://qe.s3.example.com/objects/abc?"),
            "实际 URL：{}",
            presigned.url
        );
        assert!(!presigned.url.contains("/qe/objects/abc"));
    }

    #[test]
    fn presign_put_keeps_host_only_signature_and_encodes_key() {
        let service = build_service(options());
        let presigned = service
            .presign_put("objects/a b/中文.txt", Some("application/octet-stream"))
            .unwrap();
        assert!(presigned.url.contains("X-Amz-SignedHeaders=host"));
        assert!(
            presigned
                .url
                .contains("/qe/objects/a%20b/%E4%B8%AD%E6%96%87.txt?")
        );
        assert!(
            service
                .presign_put("objects/x", Some("bad\r\nheader"))
                .is_err()
        );
    }

    #[test]
    fn presign_rejects_invalid_object_keys() {
        let service = build_service(options());
        assert!(service.presign_get(" ").is_err());
        assert!(service.presign_get("/absolute").is_err());
    }

    #[test]
    fn new_validates_required_fields() {
        assert!(
            StorageService::new(StorageOptions {
                endpoint: "not a url".to_owned(),
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                endpoint: "ftp://127.0.0.1:9000".to_owned(),
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                endpoint: "http://127.0.0.1:9000/prefix".to_owned(),
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                bucket: "  ".to_owned(),
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                region: String::new(),
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                presign_ttl_secs: 59,
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                presign_ttl_secs: 86_401,
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                signing_key_base64: Some("not-base64!!".to_owned()),
                ..options()
            })
            .is_err()
        );
        assert!(
            StorageService::new(StorageOptions {
                signing_key_base64: Some(base64::encode(&[1u8; 31])),
                ..options()
            })
            .is_err()
        );
    }

    #[test]
    fn signing_key_from_base64_is_deterministic_and_shared() {
        let seed = [7u8; 32];
        let configured = StorageOptions {
            signing_key_base64: Some(base64::encode(&seed)),
            ..options()
        };
        let first = build_service(configured.clone());
        let second = build_service(configured);
        assert_eq!(first.public_key_base64(), second.public_key_base64());
        assert_eq!(first.bucket(), "qe");

        let signature = first.sign_manifest(b"{\"schema\":1}").unwrap();
        assert!(second.verify_manifest(b"{\"schema\":1}", &signature));

        let ephemeral = build_service(options());
        assert_ne!(ephemeral.public_key_base64(), first.public_key_base64());
    }

    #[test]
    fn sign_and_verify_roundtrip_rejects_tampering() {
        let service = build_service(options());
        let manifest = br#"{"schema":1,"name":"demo","chunks":[]}"#;
        let signature = service.sign_manifest(manifest).unwrap();
        assert_eq!(signature.len(), 64);
        assert!(service.verify_manifest(manifest, &signature));

        // 内容被篡改
        assert!(!service.verify_manifest(b"{\"schema\":1}", &signature));
        // 签名被篡改
        let mut tampered = signature.clone();
        tampered[0] ^= 0xff;
        assert!(!service.verify_manifest(manifest, &tampered));
        // 长度不对
        assert!(!service.verify_manifest(manifest, &signature[..63]));
        assert!(!service.verify_manifest(manifest, &[]));

        // 另一个密钥签的 manifest 不应通过
        let other = build_service(StorageOptions {
            signing_key_base64: Some(base64::encode(&[9u8; 32])),
            ..options()
        });
        let other_signature = other.sign_manifest(manifest).unwrap();
        assert!(!service.verify_manifest(manifest, &other_signature));
    }
}
