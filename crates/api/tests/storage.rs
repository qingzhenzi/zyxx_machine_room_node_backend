//! 资源/镜像管理（P0）集成测试：真实 PostgreSQL + 进程内 fake S3。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 启动依赖并显式运行。

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::{Path as AxumPath, State as AxumState},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::put as put_route,
};
use http_body_util::BodyExt;
use qe_api::{
    api,
    cache::Cache,
    config::{Config, Environment, StorageConfig},
    db::{Db, users::UserRepo},
    state::AppState,
};
use qe_auth::password::hash_password_async;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tower::ServiceExt;

const SIGNING_KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

type FakeS3Store = Arc<Mutex<HashMap<String, Vec<u8>>>>;

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test".into())
}

fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

fn test_config(s3_endpoint: String) -> Config {
    Config {
        environment: Environment::Development,
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        database_url: database_url(),
        db_max_connections: 4,
        redis_url: redis_url(),
        jwt_secret: "integration-secret-integration-secret-1234".to_owned(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 3600,
        request_timeout_secs: 30,
        max_body_bytes: 2 * 1024 * 1024,
        login_max_attempts: 5,
        login_window_secs: 300,
        login_lock_secs: 900,
        login_ip_max_attempts: 60,
        login_ip_window_secs: 300,
        email_rate_limit_max: 5,
        email_rate_limit_window_secs: 900,
        announcement_cache_secs: 30,
        public_base_url: "http://localhost:5173".to_owned(),
        password_reset_ttl_secs: 1800,
        email_verification_ttl_secs: 86_400,
        email_driver: "log".to_owned(),
        smtp_url: "smtp://127.0.0.1:1025".to_owned(),
        smtp_from: "QEStudio <no-reply@qestudio.local>".to_owned(),
        cors_allowed_origins: vec!["http://localhost:5173".to_owned()],
        run_migrations: false,
        bootstrap_admin_username: None,
        bootstrap_admin_password: None,
        bootstrap_admin_display_name: "Administrator".to_owned(),
        log_format_json: false,
        schedule_engine_enabled: false,
        schedule_tick_secs: 15,
        schedule_horizon_days: 30,
        schedule_rebuild_interval_secs: 3600,
        schedule_pubsub_channel: "qe-schedule:events".to_owned(),
        schedule_downlink_notify_channel: "qe-device-pki:downlink".to_owned(),
        schedule_webhook_timeout_secs: 5,
        schedule_webhook_max_attempts: 5,
        schedule_delivery_batch: 50,
        machine_agent_token: None,
        machine_snapshot_keep: 100,
        device_presence_enabled: false,
        device_presence_tick_secs: 5,
        device_presence_webhook_timeout_secs: 5,
        device_presence_notify_online: false,
        device_presence_global_webhook_url: None,
        device_presence_webhook_max_attempts: 5,
        device_presence_webhook_batch: 20,
        device_presence_retry_base_seconds: 5,
        device_pki_internal_url: "http://127.0.0.1:4442".to_owned(),
        device_pki_internal_token: None,
        federation_token: None,
        public_enroll_endpoint: "https://enroll.test:4440".to_owned(),
        storage: Some(StorageConfig {
            endpoint: s3_endpoint,
            region: "us-east-1".to_owned(),
            bucket: "test-bucket".to_owned(),
            access_key: "test-access".to_owned(),
            secret_key: "test-secret".to_owned(),
            path_style: true,
            presign_ttl_secs: 900,
            signing_key_base64: Some(SIGNING_KEY_B64.to_owned()),
        }),
    }
}

async fn test_state(s3_endpoint: String) -> AppState {
    let db = Db::connect(&database_url(), 4)
        .await
        .expect("connect postgres");
    db.migrate().await.expect("run migrations");
    let cache = Cache::connect(&redis_url()).await.expect("connect redis");
    AppState::new(db, cache, test_config(s3_endpoint))
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn image_upload_verify_publish_and_assign_flow() {
    let (store, endpoint) = spawn_fake_s3().await;
    let state = test_state(endpoint).await;
    let app: Router = api::router(state.clone());
    let users = UserRepo::new(state.db.pool().clone());
    let password = "StorageIntegration@12345";
    let password_hash = hash_password_async(password.to_owned())
        .await
        .expect("hash password");
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let admin_name = format!("ia_{suffix}");
    let viewer_name = format!("iv_{suffix}");
    let admin = users
        .create_with_roles(
            &admin_name,
            None,
            "Image Admin",
            &password_hash,
            &["admin".to_owned()],
        )
        .await
        .expect("create admin");
    let viewer = users
        .create_with_roles(
            &viewer_name,
            None,
            "Image Viewer",
            &password_hash,
            &["viewer".to_owned()],
        )
        .await
        .expect("create viewer");
    let admin_token = login(&app, &admin_name, password).await;
    let viewer_token = login(&app, &viewer_name, password).await;

    // 区域 + 房间
    let (status, area) = send_json(
        &app,
        "POST",
        "/api/v1/admin/areas",
        Some(&admin_token),
        Some(json!({ "code": format!("sarea_{suffix}"), "name": "存储测试区域" })),
    )
    .await;
    assert_eq!(status, 201, "{area}");
    let area_id = area["id"].as_str().expect("area id").to_owned();
    let (status, room) = send_json(
        &app,
        "POST",
        "/api/v1/admin/rooms",
        Some(&admin_token),
        Some(json!({
            "area_id": area_id,
            "code": format!("sroom_{suffix}"),
            "name": "存储测试房间"
        })),
    )
    .await;
    assert_eq!(status, 201, "{room}");
    let room_id = room["id"].as_str().expect("room id").to_owned();

    // 创建镜像（vm_disk + qcow2，P0 compression=none）
    let (status, image) = send_json(
        &app,
        "POST",
        "/api/v1/admin/images",
        Some(&admin_token),
        Some(json!({
            "name": "win11-base",
            "version": "2026.10.31-1",
            "kind": "vm_disk",
            "format": "qcow2",
            "compression": "none",
            "platform": "linux/amd64"
        })),
    )
    .await;
    assert!(status == 200 || status == 201, "{image}");
    let image_id = image["id"]
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| panic!("image id missing: {image}"));

    // 两个 chunk：qcow2 magic + 填充
    let mut chunk0 = b"QFI\xfb".to_vec();
    chunk0.extend_from_slice(&[0x11; 1016]);
    let chunk1 = vec![0x22u8; 2048];
    let chunks = [chunk0, chunk1];
    let hashes: Vec<String> = chunks.iter().map(|chunk| sha256_hex(chunk)).collect();

    // presign → PUT → 注册
    let mut total_size = 0i64;
    for (index, (chunk, sha)) in chunks.iter().zip(hashes.iter()).enumerate() {
        let size = chunk.len() as i64;
        total_size += size;
        let (status, presigned) = send_json(
            &app,
            "POST",
            "/api/v1/admin/storage/objects/presign",
            Some(&admin_token),
            Some(
                json!({ "sha256": sha, "size": size, "content_type": "application/octet-stream" }),
            ),
        )
        .await;
        assert_eq!(status, 200, "{presigned}");
        let upload_url = presigned["upload_url"]
            .as_str()
            .or_else(|| presigned["url"].as_str())
            .unwrap_or_else(|| panic!("upload url missing: {presigned}"));
        let response = reqwest::Client::new()
            .put(upload_url)
            .body(chunk.clone())
            .send()
            .await
            .expect("put chunk");
        assert!(response.status().is_success(), "upload chunk {index}");

        let (status, object) = send_json(
            &app,
            "POST",
            "/api/v1/admin/storage/objects",
            Some(&admin_token),
            Some(
                json!({ "sha256": sha, "size": size, "content_type": "application/octet-stream" }),
            ),
        )
        .await;
        assert!(status == 200 || status == 201, "{object}");
        assert_eq!(object["sha256"], *sha);
    }
    // 服务端确实把 chunk 读回来校验过：fake S3 中应有对象
    {
        let objects = store.lock().await;
        assert!(objects.contains_key(&format!("test-bucket/objects/{}", hashes[0])));
    }

    // 提交 manifest
    let (status, manifest_reply) = send_json(
        &app,
        "POST",
        &format!("/api/v1/admin/images/{image_id}/manifest"),
        Some(&admin_token),
        Some(json!({
            "total_size": total_size,
            "parts": [
                { "index": 0, "sha256": hashes[0], "size": chunks[0].len(), "offset": 0 },
                { "index": 1, "sha256": hashes[1], "size": chunks[1].len(), "offset": chunks[0].len() }
            ],
            "runtime": { "vm_os": "windows-11", "vm_disk_size": total_size },
            "annotations": { "source": "integration" }
        })),
    )
    .await;
    assert_eq!(status, 200, "{manifest_reply}");

    let (status, image_detail) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/images/{image_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{image_detail}");
    let manifest_digest: String =
        sqlx::query_scalar("SELECT manifest_digest FROM image_artifacts WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&image_id).expect("image uuid"))
            .fetch_one(state.db.pool())
            .await
            .expect("manifest digest");
    assert_eq!(manifest_digest.len(), 64);
    let chunk_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM image_chunks WHERE manifest_digest = $1")
            .bind(&manifest_digest)
            .fetch_one(state.db.pool())
            .await
            .expect("chunk count");
    assert_eq!(chunk_count, 2);
    // manifest 与签名都写入了 S3
    {
        let objects = store.lock().await;
        assert!(objects.contains_key(&format!("test-bucket/manifests/{manifest_digest}.json")));
        assert!(objects.contains_key(&format!("test-bucket/signatures/{manifest_digest}.sig")));
    }

    // 验证 → 发布
    let (status, verify) = send_json(
        &app,
        "POST",
        &format!("/api/v1/admin/images/{image_id}/verify"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{verify}");
    let image_status: String =
        sqlx::query_scalar("SELECT status FROM image_artifacts WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&image_id).expect("image uuid"))
            .fetch_one(state.db.pool())
            .await
            .expect("image status");
    assert_eq!(image_status, "verified", "verify = {verify}");

    let (status, published) = send_json(
        &app,
        "POST",
        &format!("/api/v1/admin/images/{image_id}/publish"),
        Some(&admin_token),
        Some(json!({ "channel": "dev" })),
    )
    .await;
    assert_eq!(status, 200, "{published}");
    let image_status: String =
        sqlx::query_scalar("SELECT status FROM image_artifacts WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&image_id).expect("image uuid"))
            .fetch_one(state.db.pool())
            .await
            .expect("image status");
    assert_eq!(image_status, "published");

    // manifest 下载地址可访问且内容一致
    let (status, manifest_download) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/images/{image_id}/manifest"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{manifest_download}");
    let download_url = manifest_download["download_url"]
        .as_str()
        .or_else(|| manifest_download["url"].as_str())
        .unwrap_or_else(|| panic!("manifest download url missing: {manifest_download}"));
    let downloaded = reqwest::get(download_url)
        .await
        .expect("get manifest")
        .bytes()
        .await
        .expect("manifest bytes");
    let stored_manifest = store
        .lock()
        .await
        .get(&format!("test-bucket/manifests/{manifest_digest}.json"))
        .cloned()
        .expect("manifest in fake s3");
    assert_eq!(downloaded.as_ref(), stored_manifest.as_slice());

    // 房间绑定设备槽位 + 插入 active 注册与在线会话：分配后应写入 image.pull 下行。
    let device_slot = format!("slot-{suffix}");
    let device_serial = format!("serial-{suffix}");
    let (status, bound) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{room_id}/device"),
        Some(&admin_token),
        Some(json!({ "device_slot": device_slot })),
    )
    .await;
    assert_eq!(status, 200, "{bound}");
    assert_eq!(bound["device_slot"], device_slot, "{bound}");
    sqlx::query(
        "INSERT INTO device_enrollment
            (serial, device_slot, subject_cn, csr_sha256, cert_pem, status, activated_at)
         VALUES ($1, $2, $3, $4, $5, 'active', now())",
    )
    .bind(&device_serial)
    .bind(&device_slot)
    .bind(format!("cn-{suffix}"))
    .bind(vec![0u8; 32])
    .bind("-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----")
    .execute(state.db.pool())
    .await
    .expect("insert active device enrollment");
    sqlx::query(
        "INSERT INTO device_session (serial, device_slot, state) VALUES ($1, $2, 'online')",
    )
    .bind(&device_serial)
    .bind(&device_slot)
    .execute(state.db.pool())
    .await
    .expect("insert online device session");

    // 分配到房间 → 产生 transfer
    let (status, assignment) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{room_id}/images/{image_id}"),
        Some(&admin_token),
        Some(json!({ "channel": "dev" })),
    )
    .await;
    assert_eq!(status, 200, "{assignment}");
    let (status, transfers) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/transfers?room_id={room_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{transfers}");
    assert_eq!(transfers["total"], 1, "{transfers}");
    assert_eq!(transfers["items"][0]["direction"], "download");

    // 分配后应投递 image.pull（§16.1 字段）到设备的 device_downlink。
    let (downlink_kind, downlink_payload): (String, String) = sqlx::query_as(
        "SELECT kind, payload FROM device_downlink
         WHERE serial = $1
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(&device_serial)
    .fetch_one(state.db.pool())
    .await
    .expect("image.pull downlink row");
    assert_eq!(downlink_kind, "image.pull");
    let payload: Value = serde_json::from_str(&downlink_payload).expect("image.pull payload json");
    assert_eq!(
        payload["transfer_id"], transfers["items"][0]["id"],
        "{payload}"
    );
    assert_eq!(payload["image_id"], image_id, "{payload}");
    assert_eq!(payload["name"], "win11-base", "{payload}");
    assert_eq!(payload["version"], "2026.10.31-1", "{payload}");
    assert_eq!(payload["kind"], "vm_disk", "{payload}");
    assert_eq!(payload["format"], "qcow2", "{payload}");
    assert_eq!(payload["platform"], "linux/amd64", "{payload}");
    assert_eq!(payload["manifest_digest"], manifest_digest, "{payload}");
    assert_eq!(payload["total_size"], total_size, "{payload}");
    assert_eq!(payload["chunk_count"], 2, "{payload}");
    assert_eq!(
        payload["presign_endpoint"], "/resources/presign",
        "{payload}"
    );

    // 负例：未验证即可发布 → 非 2xx（400/409）
    let (status, second) = send_json(
        &app,
        "POST",
        "/api/v1/admin/images",
        Some(&admin_token),
        Some(json!({
            "name": "unverified",
            "version": "1",
            "kind": "docker",
            "format": "docker_archive",
            "compression": "none",
            "platform": "linux/amd64"
        })),
    )
    .await;
    assert!(status == 200 || status == 201, "{second}");
    let second_id = second["id"].as_str().expect("second image id").to_owned();
    let (status, body) = send_json(
        &app,
        "POST",
        &format!("/api/v1/admin/images/{second_id}/publish"),
        Some(&admin_token),
        Some(json!({ "channel": "dev" })),
    )
    .await;
    assert!(
        status == 400 || status == 409,
        "unverified publish = {status} {body}"
    );

    // 负例：注册不存在的对象 → 非 2xx
    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/admin/storage/objects",
        Some(&admin_token),
        Some(json!({
            "sha256": sha256_hex(b"not-uploaded"),
            "size": 11,
            "content_type": "application/octet-stream"
        })),
    )
    .await;
    assert!(status >= 400, "missing object = {status} {body}");

    // 负例：viewer 无管理权限
    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/admin/images",
        Some(&viewer_token),
        Some(json!({
            "name": "nope",
            "version": "1",
            "kind": "docker",
            "format": "docker_archive",
            "compression": "none",
            "platform": "linux/amd64"
        })),
    )
    .await;
    assert_eq!(status, 403, "{body}");

    // 清理
    for table in [
        "image_assignments",
        "transfers",
        "image_channels",
        "image_verifications",
        "image_chunks",
        "image_artifacts",
        "storage_objects",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(state.db.pool())
            .await
            .expect("cleanup storage table");
    }
    // 清理设备行（外键顺序：下行 → 会话 → 注册）。
    for statement in [
        "DELETE FROM device_downlink WHERE serial = $1",
        "DELETE FROM device_session WHERE serial = $1",
        "DELETE FROM device_enrollment WHERE serial = $1",
    ] {
        sqlx::query(statement)
            .bind(&device_serial)
            .execute(state.db.pool())
            .await
            .expect("cleanup device rows");
    }
    for room in [&room_id] {
        let (status, _) = send_json(
            &app,
            "DELETE",
            &format!("/api/v1/admin/rooms/{room}"),
            Some(&admin_token),
            None,
        )
        .await;
        assert!(status == 200 || status == 204);
    }
    let (status, _) = send_json(
        &app,
        "DELETE",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert!(status == 200 || status == 204);
    sqlx::query("DELETE FROM audit_logs WHERE actor_id = ANY($1)")
        .bind(vec![admin.id, viewer.id])
        .execute(state.db.pool())
        .await
        .expect("cleanup audit");
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![admin.id, viewer.id])
        .execute(state.db.pool())
        .await
        .expect("cleanup users");
}

async fn spawn_fake_s3() -> (FakeS3Store, String) {
    let store: FakeS3Store = Arc::new(Mutex::new(HashMap::new()));

    async fn put_object(
        AxumState(store): AxumState<FakeS3Store>,
        AxumPath(key): AxumPath<String>,
        body: Bytes,
    ) -> StatusCode {
        store.lock().await.insert(key, body.to_vec());
        StatusCode::OK
    }

    async fn get_object(
        AxumState(store): AxumState<FakeS3Store>,
        AxumPath(key): AxumPath<String>,
    ) -> Response {
        match store.lock().await.get(&key).cloned() {
            Some(bytes) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/octet-stream")],
                bytes,
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    let app = Router::new()
        .route("/{*key}", put_route(put_object).get(get_object))
        .with_state(store.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake s3");
    let addr = listener.local_addr().expect("fake s3 addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve fake s3");
    });
    (store, format!("http://{addr}"))
}

async fn login(app: &Router, username: &str, password: &str) -> String {
    let (status, body) = send_json(
        app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "username": username, "password": password })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

async fn send_json(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    use axum::body::Body;
    use axum::http::Request;
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(body) => Body::from(body.to_string()),
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("response");
    let status = response.status().as_u16();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}
