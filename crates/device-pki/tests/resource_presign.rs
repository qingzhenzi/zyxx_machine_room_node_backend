//! `/resources/presign` 端到端测试（真实 PostgreSQL + 真实 SigV4 预签名）。
//!
//! 授权链路是跨多张表的 SQL（enrollment → rooms → image_artifacts →
//! image_assignments → image_chunks），纯函数单测覆盖不到；这里用与
//! `enroll_flow.rs` 相同的真实库设施验证它。
//!
//! 默认 `#[ignore]`；用 `scripts/test-integration.sh` 或
//! `TEST_DATABASE_URL=... cargo test -p qe-device-pki --test resource_presign -- --ignored` 运行。

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use base64::Engine;
use http_body_util::BodyExt;
use qe_device_pki::{
    ca::Ca,
    cert_status::CertStatusCache,
    messaging::Dispatcher,
    ratelimit::RateLimiter,
    routes::{self, AppState},
    session::SessionManager,
    tls::PeerCert,
};
use qe_storage::{StorageOptions, StorageService};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// 分片总数：验证 64 条上限与 offset 分页。
const CHUNK_COUNT: i32 = 70;

struct Fixture {
    db: PgPool,
    router: Router,
    area_id: Uuid,
    room_id: Uuid,
    serial: String,
    slot: String,
    digest: String,
    image_id: Uuid,
    /// 另一个存在但**未分配**到本房间的镜像
    other_digest: String,
    chunk_shas: Vec<String>,
}

/// 构造 64 位 hex digest（`seed` 本身必须是 hex）。
fn hex_digest(seed: &str) -> String {
    format!("{seed:0<64}")
}

fn storage() -> Arc<StorageService> {
    let key = base64::engine::general_purpose::STANDARD.encode([3u8; 32]);
    Arc::new(
        StorageService::new(StorageOptions {
            endpoint: "http://127.0.0.1:9000".into(),
            region: "us-east-1".into(),
            bucket: "qe".into(),
            access_key: "ak".into(),
            secret_key: "sk".into(),
            path_style: true,
            presign_ttl_secs: 900,
            signing_key_base64: Some(key),
        })
        .expect("storage service"),
    )
}

fn app_state(db: PgPool, storage: Option<Arc<StorageService>>) -> AppState {
    let (sessions, _events) = SessionManager::new(Default::default());
    AppState {
        db,
        ca: Arc::new(Ca::generate_self_signed("Presign Test CA").expect("ca")),
        sessions: Arc::new(sessions),
        admin_token: Arc::from("presign-admin"),
        server_cert_fp_hex: Arc::from(""),
        cert_status: Arc::new(CertStatusCache::new()),
        admin_limiter: Arc::new(RateLimiter::new(0)),
        dispatcher: Arc::new(Dispatcher::new(std::time::Duration::from_secs(5))),
        mtls_endpoint: Arc::from("https://localhost:1"),
        enroll_endpoint: Arc::from("https://localhost:1"),
        storage,
        vnc: Arc::new(qe_device_pki::vnc::VncHub::new(
            qe_device_pki::vnc::VNC_PAIR_TIMEOUT,
        )),
        vnc_internal_token: None,
    }
}

fn peer(serial: &str, slot: &str) -> PeerCert {
    PeerCert {
        der: Vec::new(),
        common_name: Some(slot.to_owned()),
        serial_hex: Some(serial.to_owned()),
    }
}

async fn setup() -> Option<Fixture> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("skip: 未设置 TEST_DATABASE_URL");
            return None;
        }
    };
    let db = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
    {
        Ok(db) => db,
        Err(e) => {
            eprintln!("skip: 连不上 PostgreSQL（{e}）");
            return None;
        }
    };
    sqlx::migrate!("../../migrations")
        .run(&db)
        .await
        .expect("migrations");

    // uuid simple 是 32 位 hex，前缀 a/b 保证 digest 仍是合法 hex 且互不相同
    let suffix = Uuid::new_v4().simple().to_string();
    let slot = format!("presign-{suffix}");
    let serial = format!("{:032x}", Uuid::new_v4().as_u128());
    let digest = hex_digest(&format!("a{suffix}"));
    let other_digest = hex_digest(&format!("b{suffix}"));

    let area_id: Uuid =
        sqlx::query_scalar("INSERT INTO areas (code, name) VALUES ($1, $2) RETURNING id")
            .bind(format!("presign-area-{suffix}"))
            .bind("presign test area")
            .fetch_one(&db)
            .await
            .expect("insert area");

    let room_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rooms (area_id, code, name, device_slot)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(area_id)
    .bind(format!("presign-room-{suffix}"))
    .bind("presign test room")
    .bind(&slot)
    .fetch_one(&db)
    .await
    .expect("insert room");

    let image_id: Uuid = sqlx::query_scalar(
        "INSERT INTO image_artifacts
           (name, version, kind, format, platform, manifest_digest, status)
         VALUES ($1, '1.0.0', 'docker', 'docker_archive', 'linux/amd64', $2, 'published')
         RETURNING id",
    )
    .bind(format!("presign-image-{suffix}"))
    .bind(&digest)
    .fetch_one(&db)
    .await
    .expect("insert image");

    // 另一个镜像：存在但**没有**分配到本房间
    sqlx::query(
        "INSERT INTO image_artifacts
           (name, version, kind, format, platform, manifest_digest, status)
         VALUES ($1, '1.0.0', 'docker', 'docker_archive', 'linux/amd64', $2, 'published')",
    )
    .bind(format!("presign-other-{suffix}"))
    .bind(&other_digest)
    .execute(&db)
    .await
    .expect("insert other image");

    sqlx::query(
        "INSERT INTO image_assignments (image_id, room_id, channel, rollout_state)
         VALUES ($1, $2, 'stable', 'pending')",
    )
    .bind(image_id)
    .bind(room_id)
    .execute(&db)
    .await
    .expect("assign image");

    // active 是“已激活”的存活状态；mTLS 握手与这里都认它
    sqlx::query(
        "INSERT INTO device_enrollment
           (serial, device_slot, subject_cn, csr_sha256, cert_pem, status, issued_at, activated_at)
         VALUES ($1, $2, $2, $3, '', 'active', now(), now())",
    )
    .bind(&serial)
    .bind(&slot)
    .bind(Vec::<u8>::new())
    .execute(&db)
    .await
    .expect("insert enrollment");

    let chunk_shas: Vec<String> = (0..CHUNK_COUNT)
        .map(|index| format!("{index:02x}{}", "a".repeat(62)))
        .collect();
    for (index, sha) in chunk_shas.iter().enumerate() {
        sqlx::query(
            "INSERT INTO image_chunks (manifest_digest, index, object_key, sha256, size, \"offset\")
             VALUES ($1, $2, $3, $3, 1024, $4)",
        )
        .bind(&digest)
        .bind(index as i32)
        .bind(sha)
        .bind((index as i64) * 1024)
        .execute(&db)
        .await
        .expect("insert chunk");
    }

    Some(Fixture {
        router: routes::mtls_router(app_state(db.clone(), Some(storage()))),
        db,
        area_id,
        room_id,
        serial,
        slot,
        digest,
        image_id,
        other_digest,
        chunk_shas,
    })
}

async fn call_presign(router: &Router, peer: PeerCert, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/resources/presign")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    req.extensions_mut().insert(peer);

    let resp = router.clone().oneshot(req).await.expect("route");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn cleanup(f: &Fixture) {
    for sql in [
        "DELETE FROM image_assignments WHERE image_id = $1",
        "DELETE FROM image_artifacts WHERE id = $1",
    ] {
        let _ = sqlx::query(sql).bind(f.image_id).execute(&f.db).await;
    }
    let _ = sqlx::query("DELETE FROM image_chunks WHERE manifest_digest = $1")
        .bind(&f.digest)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM image_chunks WHERE manifest_digest = $1")
        .bind(&f.other_digest)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM image_artifacts WHERE manifest_digest = $1")
        .bind(&f.other_digest)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
        .bind(&f.serial)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM rooms WHERE id = $1")
        .bind(f.room_id)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM areas WHERE id = $1")
        .bind(f.area_id)
        .execute(&f.db)
        .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn presign_manifest_signature_and_chunk_paging() {
    let Some(f) = setup().await else { return };
    let peer = peer(&f.serial, &f.slot);
    let router = f.router.clone();

    // manifest：固定键
    let (status, body) = call_presign(
        &router,
        peer.clone(),
        json!({ "manifest_digest": f.digest, "kind": "manifest" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let urls = body["urls"].as_array().expect("urls");
    assert_eq!(urls.len(), 1);
    assert_eq!(
        urls[0]["key"].as_str().unwrap(),
        format!("manifests/{}.json", f.digest)
    );
    assert!(urls[0]["url"].as_str().unwrap().starts_with("http://"));
    assert!(urls[0]["expires_at"].is_string());
    assert_eq!(
        body["public_key_base64"].as_str().map(str::len),
        Some(44),
        "Ed25519 公钥 base64 应为 44 字符"
    );

    // signature：固定键
    let (status, body) = call_presign(
        &router,
        peer.clone(),
        json!({ "manifest_digest": f.digest, "kind": "signature" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["urls"][0]["key"].as_str().unwrap(),
        format!("signatures/{}.sig", f.digest)
    );

    // chunk：缺省 count=64，按 index 升序
    let (status, body) = call_presign(
        &router,
        peer.clone(),
        json!({ "manifest_digest": f.digest, "kind": "chunk" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let urls = body["urls"].as_array().expect("urls");
    assert_eq!(urls.len(), 64, "默认上限 64");
    for (i, item) in urls.iter().enumerate() {
        assert_eq!(
            item["key"].as_str().unwrap(),
            format!("objects/{}", f.chunk_shas[i])
        );
    }

    // 剩余 6 个
    let (status, body) = call_presign(
        &router,
        peer.clone(),
        json!({ "manifest_digest": f.digest, "kind": "chunk", "index": 64, "count": 64 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let urls = body["urls"].as_array().expect("urls");
    assert_eq!(urls.len(), 6);
    assert_eq!(
        urls[0]["key"].as_str().unwrap(),
        format!("objects/{}", f.chunk_shas[64])
    );

    // count 越界被 clamp 到上限
    let (status, body) = call_presign(
        &router,
        peer.clone(),
        json!({ "manifest_digest": f.digest, "kind": "chunk", "index": 0, "count": 10_000 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["urls"].as_array().unwrap().len(), 64);

    // index 越界 → 404
    let (status, body) = call_presign(
        &router,
        peer,
        json!({ "manifest_digest": f.digest, "kind": "chunk", "index": 999 }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "not_found");

    cleanup(&f).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn presign_rejects_unauthorized_devices_and_images() {
    let Some(f) = setup().await else { return };
    let router = f.router.clone();

    // 未知设备 serial → 403（不能签任何 URL）
    let (status, body) = call_presign(
        &router,
        peer(&format!("{:032x}", 7), "some-other-slot"),
        json!({ "manifest_digest": f.digest, "kind": "manifest" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "forbidden");

    // 镜像存在但没分配到该房间 → 403
    let (status, body) = call_presign(
        &router,
        peer(&f.serial, &f.slot),
        json!({ "manifest_digest": f.other_digest, "kind": "manifest" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "forbidden");

    // digest 根本不存在 → 404
    let (status, body) = call_presign(
        &router,
        peer(&f.serial, &f.slot),
        json!({ "manifest_digest": hex_digest("c"), "kind": "manifest" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "not_found");

    // 吊销后的证书：即使握手缓存还没刷新，也不能再签 URL
    sqlx::query(
        "UPDATE device_enrollment SET status = 'revoked', revoked_at = now() WHERE serial = $1",
    )
    .bind(&f.serial)
    .execute(&f.db)
    .await
    .expect("revoke");
    let (status, body) = call_presign(
        &router,
        peer(&f.serial, &f.slot),
        json!({ "manifest_digest": f.digest, "kind": "chunk" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    cleanup(&f).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn presign_requires_object_storage_configuration() {
    let Some(f) = setup().await else { return };

    // 同一个库、同样的授权数据，但存储未配置 → 503
    let bare = routes::mtls_router(app_state(f.db.clone(), None));
    let (status, body) = call_presign(
        &bare,
        peer(&f.serial, &f.slot),
        json!({ "manifest_digest": f.digest, "kind": "manifest" }),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "storage_unavailable");

    cleanup(&f).await;
}
