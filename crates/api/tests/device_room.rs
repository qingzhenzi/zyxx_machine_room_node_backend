//! 设备接入区域/房间集成测试：真实 PostgreSQL + Redis。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 启动依赖并显式运行。

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use qe_api::{
    api,
    cache::Cache,
    config::{Config, Environment},
    db::{Db, users::UserRepo},
    state::AppState,
};
use qe_auth::password::hash_password_async;
use serde_json::{Value, json};
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test".into())
}

fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

fn test_config() -> Config {
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
        device_presence_enabled: false,
        device_presence_tick_secs: 5,
        device_presence_webhook_timeout_secs: 5,
        device_presence_notify_online: false,
        device_presence_global_webhook_url: None,
        device_presence_webhook_max_attempts: 5,
        device_presence_webhook_batch: 20,
        device_presence_retry_base_seconds: 5,
        public_enroll_endpoint: "https://enroll.test:4440".to_owned(),
        storage: None,
    }
}

async fn test_state() -> AppState {
    let db = Db::connect(&database_url(), 4)
        .await
        .expect("connect postgres");
    db.migrate().await.expect("run migrations");
    let cache = Cache::connect(&redis_url()).await.expect("connect redis");
    AppState::new(db, cache, test_config())
}

fn unique_code(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn device_can_be_added_moved_and_removed_from_rooms() {
    let state = test_state().await;
    let app: Router = api::router(state.clone());
    let users = UserRepo::new(state.db.pool().clone());
    let password = "DeviceIntegration@12345";
    let password_hash = hash_password_async(password.to_owned())
        .await
        .expect("hash password");
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let admin_name = format!("da_{suffix}");
    let viewer_name = format!("dv_{suffix}");
    let admin = users
        .create_with_roles(
            &admin_name,
            None,
            "Device Admin",
            &password_hash,
            &["admin".to_owned()],
        )
        .await
        .expect("create admin");
    let viewer = users
        .create_with_roles(
            &viewer_name,
            None,
            "Device Viewer",
            &password_hash,
            &["viewer".to_owned()],
        )
        .await
        .expect("create viewer");
    let token = login(&app, &admin_name, password).await;
    let viewer_token = login(&app, &viewer_name, password).await;

    // 区域 + 两个房间
    let (status, area) = send_json(
        &app,
        "POST",
        "/api/v1/admin/areas",
        Some(&token),
        Some(json!({ "code": unique_code("darea"), "name": "设备测试区域" })),
    )
    .await;
    assert_eq!(status, 201, "{area}");
    let area_id = area["id"].as_str().expect("area id").to_owned();

    let room_code = unique_code("droom");
    let (status, room) = send_json(
        &app,
        "POST",
        "/api/v1/admin/rooms",
        Some(&token),
        Some(json!({
            "area_id": area_id,
            "code": room_code,
            "name": "设备测试房间 1",
            "capacity": 30
        })),
    )
    .await;
    assert_eq!(status, 201, "{room}");
    let room_id = room["id"].as_str().expect("room id").to_owned();
    assert!(room["device"].is_null(), "新房间不应有设备：{room}");

    let (status, room2) = send_json(
        &app,
        "POST",
        "/api/v1/admin/rooms",
        Some(&token),
        Some(json!({
            "area_id": area_id,
            "code": unique_code("droom"),
            "name": "设备测试房间 2"
        })),
    )
    .await;
    assert_eq!(status, 201, "{room2}");
    let room2_id = room2["id"].as_str().expect("room2 id").to_owned();

    // 绑定设备槽位（尚未注册）
    let slot = unique_code("slot");
    let (status, body) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{room_id}/device"),
        Some(&token),
        Some(json!({ "device_slot": slot })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["device_slot"], slot);
    assert_eq!(body["device"]["device_slot"], slot);
    assert!(body["device"]["serial"].is_null(), "{body}");
    assert_eq!(body["device"]["pending_token"], false);

    // 从房间签发注册 token
    let (status, issued) = send_json(
        &app,
        "POST",
        &format!("/api/v1/admin/rooms/{room_id}/device-token"),
        Some(&token),
        Some(json!({ "kind": "auto", "ttl_secs": 600 })),
    )
    .await;
    assert_eq!(status, 200, "{issued}");
    let issued_token = issued["token"].as_str().expect("token").to_owned();
    assert_eq!(issued_token.len(), 64, "{issued}");
    assert_eq!(issued["kind"], "auto");
    assert_eq!(issued["device_slot"], slot);
    let link = issued["link"].as_str().expect("link");
    assert!(
        link.starts_with("https://enroll.test:4440/enroll?"),
        "{link}"
    );
    assert!(link.contains("token=") && link.contains("slot="), "{link}");

    // token 已落库
    let token_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM device_bootstrap_token
         WHERE device_slot = $1 AND used_at IS NULL AND expires_at > now()",
    )
    .bind(&slot)
    .fetch_one(state.db.pool())
    .await
    .expect("count bootstrap token");
    assert_eq!(token_rows, 1);

    // 房间摘要显示等待注册
    let (status, device) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/rooms/{room_id}/device"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{device}");
    assert_eq!(device["pending_token"], true, "{device}");
    assert!(device["serial"].is_null(), "{device}");

    // viewer 不能管理设备
    let (status, body) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{room_id}/device"),
        Some(&viewer_token),
        Some(json!({ "device_slot": null })),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = send_json(
        &app,
        "POST",
        &format!("/api/v1/admin/rooms/{room_id}/device-token"),
        Some(&viewer_token),
        Some(json!({ "kind": "auto" })),
    )
    .await;
    assert_eq!(status, 403, "{body}");

    // 模拟设备注册完成：写入 device_enrollment + device_session，并消费 token。
    let serial = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO device_enrollment
            (serial, device_slot, subject_cn, csr_sha256, cert_pem, status, issued_at, activated_at)
         VALUES ($1, $2, $3, $4, $5, 'active', now(), now())",
    )
    .bind(&serial)
    .bind(&slot)
    .bind(&slot)
    .bind(vec![7u8; 32])
    .bind("-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n")
    .execute(state.db.pool())
    .await
    .expect("insert enrollment");
    sqlx::query(
        "INSERT INTO device_session (serial, device_slot, last_seen_seq, state, updated_at)
         VALUES ($1, $2, 1, 'online', now())",
    )
    .bind(&serial)
    .bind(&slot)
    .execute(state.db.pool())
    .await
    .expect("insert session");
    sqlx::query("UPDATE device_bootstrap_token SET used_at = now() WHERE device_slot = $1")
        .bind(&slot)
        .execute(state.db.pool())
        .await
        .expect("consume token");

    // 房间详情带出设备摘要
    let (status, body) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/rooms/{room_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["device"]["serial"], serial, "{body}");
    assert_eq!(body["device"]["status"], "active");
    assert_eq!(body["device"]["online"], "online");
    assert!(body["device"]["last_seen_at"].is_string(), "{body}");
    assert_eq!(body["device"]["pending_token"], false);

    // 设备列表可按区域/房间/关键字过滤
    let (status, devices) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/devices?area_id={area_id}&room_id={room_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{devices}");
    assert_eq!(devices["total"], 1, "{devices}");
    let item = &devices["items"][0];
    assert_eq!(item["serial"], serial);
    assert_eq!(item["device_slot"], slot);
    assert_eq!(item["status"], "active");
    assert_eq!(item["online"], "online");
    assert_eq!(item["room_id"], room_id);
    assert_eq!(item["area_id"], area_id);

    let (status, by_keyword) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/devices?keyword={serial}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{by_keyword}");
    assert_eq!(by_keyword["total"], 1, "{by_keyword}");

    let (status, detail) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/devices/{serial}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(detail["serial"], serial);

    // 迁移到房间 2：房间 1 自动解绑
    let (status, body) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{room2_id}/device"),
        Some(&token),
        Some(json!({ "device_slot": slot })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["device"]["serial"], serial, "{body}");

    let (status, body) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/rooms/{room_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["device"].is_null(), "旧房间应已解绑：{body}");

    // 解绑
    let (status, body) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{room2_id}/device"),
        Some(&token),
        Some(json!({ "device_slot": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["device"].is_null(), "{body}");

    let (status, unbound) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/devices?keyword={serial}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{unbound}");
    assert_eq!(unbound["items"][0]["room_id"], Value::Null, "{unbound}");

    // 404：房间不存在
    let missing = uuid::Uuid::new_v4();
    let (status, body) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{missing}/device"),
        Some(&token),
        Some(json!({ "device_slot": slot })),
    )
    .await;
    assert_eq!(status, 404, "{body}");

    // 清理
    for room in [&room_id, &room2_id] {
        let (status, _) = send_json(
            &app,
            "DELETE",
            &format!("/api/v1/admin/rooms/{room}"),
            Some(&token),
            None,
        )
        .await;
        assert!(status == 200 || status == 204, "delete room = {status}");
    }
    let (status, _) = send_json(
        &app,
        "DELETE",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&token),
        None,
    )
    .await;
    assert!(status == 200 || status == 204, "delete area = {status}");

    sqlx::query("DELETE FROM device_session WHERE serial = $1")
        .bind(&serial)
        .execute(state.db.pool())
        .await
        .expect("cleanup session");
    sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
        .bind(&serial)
        .execute(state.db.pool())
        .await
        .expect("cleanup enrollment");
    sqlx::query("DELETE FROM device_bootstrap_token WHERE device_slot = $1")
        .bind(&slot)
        .execute(state.db.pool())
        .await
        .expect("cleanup token");
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
