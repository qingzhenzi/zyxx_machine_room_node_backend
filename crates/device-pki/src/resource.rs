//! `POST /resources/presign` —— 设备换取对象存储的短期下载 URL（契约 §16.2）。
//!
//! 大文件不走 device-pki 的消息通道：设备拿着 mTLS 身份来这里换预签名 URL，
//! 再直连 S3 下载分片。授权链路 **fail-closed**：
//!
//! ```text
//! serial → device_enrollment（issued/active）→ device_slot
//!        → rooms.device_slot → room_id
//!        → image_artifacts.manifest_digest → image_assignments (image_id, room_id)
//! ```
//!
//! 任何一步缺失都拒绝（403/404），绝不"查不到就放行"；对象存储未配置时
//! 直接 503，连鉴权都不做——此时服务端没有任何可签发的 URL。
//!
//! 取键规则：
//! - `kind=manifest`  → `manifests/{digest}.json`
//! - `kind=signature` → `signatures/{digest}.sig`
//! - `kind=chunk`     → `objects/{sha256}`，`index` 起按 index 升序取
//!   `count`（clamp 1..=64）个

use crate::error::{Error, Result};
use crate::routes::AppState;
use crate::tls::PeerCert;
use axum::Json;
use axum::extract::State;
use qe_storage::{StorageOptions, StorageService};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// 单次最多返回的分片数：一个请求签几百个 URL 只会放大泄漏面与延迟。
pub const MAX_CHUNKS_PER_REQUEST: i64 = 64;

/// 分片内容的默认内容寻址前缀（与 P0 存储契约一致）。
const OBJECT_PREFIX: &str = "objects";

/// `POST /resources/presign` 的 `kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Manifest,
    Signature,
    Chunk,
}

impl ResourceKind {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "manifest" => Ok(Self::Manifest),
            "signature" => Ok(Self::Signature),
            "chunk" => Ok(Self::Chunk),
            other => Err(Error::BadRequest(format!(
                "kind 必须是 manifest|signature|chunk，收到 {other}"
            ))),
        }
    }

    /// 固定键（manifest / signature）；分片的键来自 `image_chunks`。
    pub fn fixed_key(self, digest: &str) -> Option<String> {
        match self {
            Self::Manifest => Some(format!("manifests/{digest}.json")),
            Self::Signature => Some(format!("signatures/{digest}.sig")),
            Self::Chunk => None,
        }
    }
}

/// 分片的内容寻址键。
pub fn chunk_object_key(sha256: &str) -> String {
    format!("{OBJECT_PREFIX}/{sha256}")
}

/// 校验并规范化 manifest digest（小写 64 位 hex）。
///
/// 只接受规范形式：digest 直接拼进对象键，放任任意字符串等于把键空间
/// 交给调用方（`../` 之类的键在 S3 里不会穿越，但会让授权校验和审计对不上）。
pub fn normalize_digest(raw: &str) -> Result<String> {
    let digest = raw.trim();
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::BadRequest(
            "manifest_digest 必须是 64 位十六进制".into(),
        ));
    }
    Ok(digest.to_ascii_lowercase())
}

/// `count` clamp 到 1..=64；`index` 不允许为负。
pub fn clamp_count(count: i64) -> i64 {
    count.clamp(1, MAX_CHUNKS_PER_REQUEST)
}

/// 起始分片下标（负值按 0 处理）。
pub fn clamp_index(index: i32) -> i32 {
    index.max(0)
}

#[derive(Debug, Deserialize)]
pub struct PresignRequest {
    pub manifest_digest: String,
    /// "manifest" | "signature" | "chunk"
    pub kind: String,
    /// `kind=chunk` 的起始分片下标，默认 0
    #[serde(default)]
    pub index: Option<i32>,
    /// `kind=chunk` 的单次数量，默认 64，clamp 1..=64
    #[serde(default)]
    pub count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PresignedObject {
    pub key: String,
    pub url: String,
    /// 绝对过期时间（RFC3339），客户端可据此提前续签
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct PresignReply {
    pub urls: Vec<PresignedObject>,
    /// manifest 验签用的 Ed25519 公钥（私钥只在本机，不下发）
    pub public_key_base64: String,
}

/// `POST /resources/presign`（mTLS listener）。
///
/// 顺序有讲究：**先看对象存储是否可用**，再校验证书/房间/分配。
/// 存储没配置时任何授权结论都没有意义，直接 503 让客户端按可重试处理。
pub async fn presign(
    State(st): State<AppState>,
    peer: PeerCert,
    Json(req): Json<PresignRequest>,
) -> Result<Json<PresignReply>> {
    let storage = st.storage.as_ref().ok_or(Error::StorageUnavailable)?;

    let digest = normalize_digest(&req.manifest_digest)?;
    let kind = ResourceKind::parse(&req.kind)?;

    // 身份只取自 mTLS 证书，不信任请求体
    let serial = peer.serial_hex.as_deref().ok_or(Error::Unauthorized)?;
    let room_id = resolve_room_for_serial(&st.db, serial).await?;
    // 镜像必须真实存在且已分配到该房间；失败在函数内区分 404 / 403
    authorize_image(&st.db, &digest, room_id).await?;

    let keys = match kind {
        ResourceKind::Chunk => {
            let index = clamp_index(req.index.unwrap_or(0));
            let count = clamp_count(req.count.unwrap_or(MAX_CHUNKS_PER_REQUEST));
            chunk_keys(&st.db, &digest, index, count).await?
        }
        // 固定键不会失败：`fixed_key` 对这两种 kind 恒为 Some
        _ => vec![kind.fixed_key(&digest).unwrap_or_default()],
    };

    let mut urls = Vec::with_capacity(keys.len());
    for key in keys {
        let signed = storage
            .presign_get(&key)
            .map_err(|e| Error::Internal(e.context(format!("预签名对象 {key} 失败"))))?;
        urls.push(PresignedObject {
            key,
            url: signed.url,
            expires_at: signed.expires_at,
        });
    }

    Ok(Json(PresignReply {
        urls,
        public_key_base64: storage.public_key_base64(),
    }))
}

/// serial → 存活 enrollment → `device_slot` → `rooms.id`。
///
/// 证书状态以库里的 `issued`/`active` 为准（与 mTLS 握手的允许范围一致），
/// 而不是相信证书本身——吊销后即使握手缓存还没刷新，这里也不会放行。
pub async fn resolve_room_for_serial(pool: &PgPool, serial: &str) -> Result<Uuid> {
    let slot: Option<String> = sqlx::query_scalar(
        "SELECT device_slot FROM device_enrollment
         WHERE serial = $1 AND status IN ('issued','active')
         LIMIT 1",
    )
    .bind(serial)
    .fetch_optional(pool)
    .await?;
    let slot = slot.ok_or_else(|| Error::Forbidden("设备证书不在 issued/active 状态".into()))?;

    let room: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM rooms WHERE device_slot = $1 LIMIT 1")
            .bind(&slot)
            .fetch_optional(pool)
            .await?;
    room.ok_or_else(|| Error::Forbidden(format!("设备槽位 {slot} 未绑定房间")))
}

/// manifest digest → 该房间必须存在 `(image_id, room_id)` 分配（fail-closed）。
///
/// 用 `image_assignments JOIN image_artifacts.manifest_digest` 判定，而不是先
/// 任意挑一行 artifact：同一个 digest 可能被多个 artifact 行引用（唯一约束是
/// name+version+platform），挑到未分配的那行会误判。
///
/// - 制品不存在 → 404（digest 未知，多半是客户端拿错了 manifest）
/// - 制品存在但没分配到该房间 → 403（明确的越权/未分配）
pub async fn authorize_image(pool: &PgPool, digest: &str, room_id: Uuid) -> Result<()> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM image_artifacts WHERE manifest_digest = $1)",
    )
    .bind(digest)
    .fetch_one(pool)
    .await?;
    if !exists {
        return Err(Error::NotFound);
    }

    let assigned: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM image_artifacts a
            JOIN image_assignments ia ON ia.image_id = a.id
            WHERE a.manifest_digest = $1 AND ia.room_id = $2
         )",
    )
    .bind(digest)
    .bind(room_id)
    .fetch_one(pool)
    .await?;
    if !assigned {
        return Err(Error::Forbidden("镜像未分配到该设备所在房间".into()));
    }
    Ok(())
}

/// 从 `image_chunks` 取 `index >= index` 的 `count` 个分片键（index 升序）。
///
/// 没有匹配分片 → 404：说明请求的 index 超出了 manifest 的分片数。
pub async fn chunk_keys(
    pool: &PgPool,
    digest: &str,
    index: i32,
    count: i64,
) -> Result<Vec<String>> {
    let shas: Vec<String> = sqlx::query_scalar(
        "SELECT sha256 FROM image_chunks
         WHERE manifest_digest = $1 AND index >= $2
         ORDER BY index ASC
         LIMIT $3",
    )
    .bind(digest)
    .bind(index)
    .bind(clamp_count(count))
    .fetch_all(pool)
    .await?;

    if shas.is_empty() {
        return Err(Error::NotFound);
    }
    Ok(shas.iter().map(|sha| chunk_object_key(sha)).collect())
}

// ---------------- 配置：从环境变量构建存储服务 ----------------

/// 从环境变量构建 S3 适配器；未配置或构建失败返回 `None`（端点回 503）。
///
/// 与 `crates/api` 使用同一套变量：`S3_ENDPOINT` 与 `S3_BUCKET` 必须同时设置。
pub fn build_storage_from_env() -> Option<Arc<StorageService>> {
    match storage_options_from(|key| std::env::var(key).ok()) {
        Ok(Some(options)) => {
            let bucket = options.bucket.clone();
            let endpoint = options.endpoint.clone();
            match StorageService::new(options) {
                Ok(service) => {
                    tracing::info!(bucket = %bucket, endpoint = %endpoint, "对象存储已启用（/resources/presign）");
                    Some(Arc::new(service))
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "初始化对象存储失败，/resources/presign 将返回 503"
                    );
                    None
                }
            }
        }
        Ok(None) => {
            tracing::warn!("未配置 S3_ENDPOINT/S3_BUCKET，/resources/presign 将返回 503");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "S3 配置无效，/resources/presign 将返回 503");
            None
        }
    }
}

/// 纯解析：`get(key)` 返回环境变量值。独立出来便于单测（不碰进程环境）。
///
/// 未设置 `S3_ENDPOINT` 与 `S3_BUCKET` → `Ok(None)`（对象存储整体禁用）；
/// 只设置其中一个、或数值/布尔值非法 → `Err`。
pub fn storage_options_from(
    get: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Option<StorageOptions>> {
    let env = |key: &str| {
        get(key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };

    let endpoint = env("S3_ENDPOINT");
    let bucket = env("S3_BUCKET");
    let (endpoint, bucket) = match (endpoint, bucket) {
        (None, None) => return Ok(None),
        (Some(endpoint), Some(bucket)) => (endpoint, bucket),
        _ => anyhow::bail!("S3_ENDPOINT 与 S3_BUCKET 必须同时设置（或都不设置以禁用对象存储）"),
    };

    let presign_ttl_secs = match env("STORAGE_PRESIGN_TTL_SECONDS") {
        Some(raw) => {
            let ttl: u64 = raw.parse().map_err(|error| {
                anyhow::anyhow!("STORAGE_PRESIGN_TTL_SECONDS 的值 {raw:?} 无法解析: {error}")
            })?;
            // 提前校验，避免 StorageService::new 报错时只剩一句含糊的初始化失败
            if !(60..=86_400).contains(&ttl) {
                anyhow::bail!("STORAGE_PRESIGN_TTL_SECONDS 需在 60-86400 秒之间，收到 {ttl}");
            }
            ttl
        }
        None => 900,
    };

    let path_style = match env("S3_PATH_STYLE") {
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => anyhow::bail!("S3_PATH_STYLE 的值 {raw:?} 不是合法布尔值"),
        },
        None => true,
    };

    Ok(Some(StorageOptions {
        endpoint,
        region: env("S3_REGION").unwrap_or_else(|| "us-east-1".to_owned()),
        bucket,
        access_key: env("S3_ACCESS_KEY").unwrap_or_default(),
        secret_key: env("S3_SECRET_KEY").unwrap_or_default(),
        path_style,
        presign_ttl_secs,
        signing_key_base64: env("STORAGE_SIGNING_KEY_BASE64"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ca::Ca;
    use crate::cert_status::CertStatusCache;
    use crate::ratelimit::RateLimiter;
    use crate::routes::AppState;
    use crate::session::SessionManager;
    use base64::Engine;

    fn digest() -> String {
        "ab".repeat(32)
    }

    fn state(storage: Option<Arc<StorageService>>) -> AppState {
        AppState {
            // 连接懒建立：本模块的单测都不碰数据库
            db: PgPool::connect_lazy("postgres://localhost/none").unwrap(),
            ca: Arc::new(Ca::generate_self_signed("t").unwrap()),
            sessions: Arc::new(SessionManager::new(Default::default()).0),
            admin_token: Arc::from("secret"),
            server_cert_fp_hex: Arc::from(""),
            cert_status: Arc::new(CertStatusCache::new()),
            admin_limiter: Arc::new(RateLimiter::new(0)),
            dispatcher: Arc::new(crate::messaging::Dispatcher::new(
                std::time::Duration::from_secs(1),
            )),
            mtls_endpoint: Arc::from("https://mtls.test:4441"),
            enroll_endpoint: Arc::from("https://enroll.test:4440"),
            storage,
        }
    }

    fn storage() -> Arc<StorageService> {
        Arc::new(
            StorageService::new(StorageOptions {
                endpoint: "http://127.0.0.1:9000".into(),
                region: "us-east-1".into(),
                bucket: "qe".into(),
                access_key: "ak".into(),
                secret_key: "sk".into(),
                path_style: true,
                presign_ttl_secs: 900,
                signing_key_base64: Some(
                    base64::engine::general_purpose::STANDARD.encode([7u8; 32]),
                ),
            })
            .expect("storage"),
        )
    }

    fn peer() -> PeerCert {
        PeerCert {
            der: Vec::new(),
            common_name: Some("slot-1".into()),
            serial_hex: Some("01ab".into()),
        }
    }

    fn request(kind: &str) -> PresignRequest {
        PresignRequest {
            manifest_digest: digest(),
            kind: kind.into(),
            index: None,
            count: None,
        }
    }

    #[test]
    fn digest_validation_is_strict_and_normalizes_case() {
        assert_eq!(
            normalize_digest(&digest().to_uppercase()).unwrap(),
            digest()
        );
        assert!(normalize_digest("short").is_err());
        assert!(normalize_digest(&"zz".repeat(32)).is_err());
        assert!(normalize_digest(&("ab".repeat(31) + "/")).is_err());
    }

    #[test]
    fn fixed_keys_follow_storage_layout() {
        let digest = digest();
        assert_eq!(
            ResourceKind::Manifest.fixed_key(&digest).unwrap(),
            format!("manifests/{digest}.json")
        );
        assert_eq!(
            ResourceKind::Signature.fixed_key(&digest).unwrap(),
            format!("signatures/{digest}.sig")
        );
        assert!(ResourceKind::Chunk.fixed_key(&digest).is_none());
        assert_eq!(chunk_object_key("deadbeef"), "objects/deadbeef".to_owned());
    }

    #[test]
    fn kind_parse_rejects_unknown() {
        assert_eq!(
            ResourceKind::parse("manifest").unwrap(),
            ResourceKind::Manifest
        );
        assert_eq!(
            ResourceKind::parse("signature").unwrap(),
            ResourceKind::Signature
        );
        assert_eq!(ResourceKind::parse("chunk").unwrap(), ResourceKind::Chunk);
        let err = ResourceKind::parse("bucket").unwrap_err();
        assert_eq!(err.code(), "bad_request");
    }

    #[test]
    fn count_is_clamped_to_1_through_64() {
        assert_eq!(clamp_count(0), 1);
        assert_eq!(clamp_count(-5), 1);
        assert_eq!(clamp_count(1), 1);
        assert_eq!(clamp_count(63), 63);
        assert_eq!(clamp_count(64), 64);
        assert_eq!(clamp_count(65), MAX_CHUNKS_PER_REQUEST);
        assert_eq!(clamp_count(i64::MAX), MAX_CHUNKS_PER_REQUEST);
    }

    #[test]
    fn index_is_clamped_to_zero() {
        assert_eq!(clamp_index(-1), 0);
        assert_eq!(clamp_index(0), 0);
        assert_eq!(clamp_index(7), 7);
    }

    /// 没有对象存储时必须在**碰数据库之前**就拒绝（503），
    /// 否则配置缺失会被误报成"设备未授权"，现场很难定位。
    #[tokio::test]
    async fn missing_storage_returns_503_without_database() {
        let st = state(None);
        let err = presign(State(st), peer(), Json(request("manifest")))
            .await
            .expect_err("未配置存储必须拒绝");
        assert_eq!(err.code(), "storage_unavailable");
    }

    /// 请求格式错误同样是 400，且不需要数据库。
    #[tokio::test]
    async fn invalid_digest_is_bad_request_before_database() {
        let st = state(Some(storage()));
        let mut req = request("manifest");
        req.manifest_digest = "nope".into();
        let err = presign(State(st), peer(), Json(req)).await.unwrap_err();
        assert_eq!(err.code(), "bad_request");
    }

    #[tokio::test]
    async fn invalid_kind_is_bad_request_before_database() {
        let st = state(Some(storage()));
        let err = presign(State(st), peer(), Json(request("blob")))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "bad_request");
    }

    /// 用一张静态表模拟环境变量（`storage_options_from` 的 `get` 参数）。
    fn lookup<'a>(
        vars: &'a [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn storage_options_are_disabled_only_when_both_are_missing() {
        assert!(
            storage_options_from(|_: &str| None::<String>)
                .unwrap()
                .is_none()
        );

        let vars = [("S3_ENDPOINT", "http://127.0.0.1:9000")];
        let err = storage_options_from(lookup(&vars)).unwrap_err();
        assert!(err.to_string().contains("同时设置"), "{err}");

        let vars = [("S3_BUCKET", "qe")];
        assert!(storage_options_from(lookup(&vars)).is_err());
    }

    #[test]
    fn storage_options_use_api_compatible_defaults() {
        let vars = [
            ("S3_ENDPOINT", "http://127.0.0.1:9000/"),
            ("S3_BUCKET", "qe"),
        ];
        let options = storage_options_from(lookup(&vars))
            .unwrap()
            .expect("configured");
        assert_eq!(options.endpoint, "http://127.0.0.1:9000/");
        assert_eq!(options.bucket, "qe");
        assert_eq!(options.region, "us-east-1");
        assert!(options.path_style);
        assert_eq!(options.presign_ttl_secs, 900);
        assert!(options.signing_key_base64.is_none());
        // StorageService::new 会自己 trim 结尾斜杠
        assert!(StorageService::new(options).is_ok());
    }

    #[test]
    fn storage_options_honour_explicit_values() {
        let vars = [
            ("S3_ENDPOINT", "https://s3.example.com"),
            ("S3_BUCKET", "images"),
            ("S3_REGION", "cn-east-1"),
            ("S3_ACCESS_KEY", "ak"),
            ("S3_SECRET_KEY", "sk"),
            ("S3_PATH_STYLE", "false"),
            ("STORAGE_PRESIGN_TTL_SECONDS", "120"),
            ("STORAGE_SIGNING_KEY_BASE64", "c2VlZA=="),
        ];
        let options = storage_options_from(lookup(&vars))
            .unwrap()
            .expect("configured");
        assert_eq!(options.endpoint, "https://s3.example.com");
        assert_eq!(options.region, "cn-east-1");
        assert_eq!(options.access_key, "ak");
        assert_eq!(options.secret_key, "sk");
        assert!(!options.path_style);
        assert_eq!(options.presign_ttl_secs, 120);
        assert_eq!(options.signing_key_base64.as_deref(), Some("c2VlZA=="));
    }

    #[test]
    fn storage_options_reject_bad_ttl_and_path_style() {
        let check = |extra: (&'static str, &'static str)| {
            let vars = [("S3_ENDPOINT", "http://h:9000"), ("S3_BUCKET", "qe"), extra];
            storage_options_from(lookup(&vars))
        };

        assert!(
            check(("STORAGE_PRESIGN_TTL_SECONDS", "59"))
                .unwrap_err()
                .to_string()
                .contains("60-86400")
        );
        assert!(
            check(("STORAGE_PRESIGN_TTL_SECONDS", "86401"))
                .unwrap_err()
                .to_string()
                .contains("60-86400")
        );
        assert!(check(("STORAGE_PRESIGN_TTL_SECONDS", "abc")).is_err());
        assert!(check(("S3_PATH_STYLE", "maybe")).is_err());
        assert!(!check(("S3_PATH_STYLE", "off")).unwrap().unwrap().path_style);
    }
}
