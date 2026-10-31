//! 设备在线事件（P3）集成测试：真实 PostgreSQL。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 或显式
//! `cargo test -p qe-api --test device_presence -- --ignored --nocapture` 运行。
//!
//! 覆盖：
//! - `GET /admin/device-events`：倒序、`after_id` 增量、limit/after_id 校验、权限；
//! - `GET /admin/device-events/stream`：SSE 补发 `device.presence` 事件；
//! - `GET /admin/device-presence-deliveries`：按 status 过滤与参数校验；
//! - 区域 create/update 的 `webhook_url`：缺失不修改、`null` 清空、非法 URL 400。

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
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
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "DevicePresenceIntegration@12345";

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
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
        // 不启动 presence worker：测试直接写事件/投递表，避免后台任务竞态。
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

struct Harness {
    state: AppState,
    app: Router,
    admin_token: String,
    admin_id: Uuid,
    viewer_token: String,
    viewer_id: Uuid,
    suffix: String,
}

impl Harness {
    async fn new(prefix: &str) -> Self {
        let state = test_state().await;
        let app: Router = api::router(state.clone());
        let users = UserRepo::new(state.db.pool().clone());
        let password_hash = hash_password_async(PASSWORD.to_owned())
            .await
            .expect("hash password");
        let suffix = Uuid::new_v4().simple().to_string()[..8].to_owned();
        let admin_name = format!("{prefix}_{suffix}");
        let viewer_name = format!("{prefix}v_{suffix}");
        let admin = users
            .create_with_roles(
                &admin_name,
                None,
                "Presence Admin",
                &password_hash,
                &["admin".to_owned()],
            )
            .await
            .expect("create admin");
        let viewer = users
            .create_with_roles(
                &viewer_name,
                None,
                "Presence Viewer",
                &password_hash,
                &["viewer".to_owned()],
            )
            .await
            .expect("create viewer");
        let admin_token = login(&app, &admin_name, PASSWORD).await;
        let viewer_token = login(&app, &viewer_name, PASSWORD).await;
        Self {
            state,
            app,
            admin_token,
            admin_id: admin.id,
            viewer_token,
            viewer_id: viewer.id,
            suffix,
        }
    }

    fn pool(&self) -> &PgPool {
        self.state.db.pool()
    }

    /// 删除测试用户及其审计日志；事件/投递/区域由各测试自行清理。
    async fn cleanup_users(&self) {
        sqlx::query("DELETE FROM audit_logs WHERE actor_id = ANY($1)")
            .bind(vec![self.admin_id, self.viewer_id])
            .execute(self.pool())
            .await
            .expect("cleanup audit");
        sqlx::query("DELETE FROM users WHERE id = ANY($1)")
            .bind(vec![self.admin_id, self.viewer_id])
            .execute(self.pool())
            .await
            .expect("cleanup users");
    }
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
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
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

/// 插入一条事件（绕过 worker，只验证 API 层）。
async fn insert_event(
    pool: &PgPool,
    serial: &str,
    device_slot: &str,
    previous_state: &str,
    current_state: &str,
    source_updated_at: chrono::DateTime<chrono::Utc>,
    payload: Value,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO device_presence_events
             (serial, device_slot, room_id, area_id, previous_state, current_state,
              source_updated_at, payload)
         VALUES ($1, $2, NULL, NULL, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(serial)
    .bind(device_slot)
    .bind(previous_state)
    .bind(current_state)
    .bind(source_updated_at)
    .bind(payload)
    .fetch_one(pool)
    .await
    .expect("insert presence event")
}

async fn cleanup_events(pool: &PgPool, serial: &str) {
    sqlx::query(
        "DELETE FROM device_presence_deliveries
          WHERE event_id IN (SELECT id FROM device_presence_events WHERE serial = $1)",
    )
    .bind(serial)
    .execute(pool)
    .await
    .expect("cleanup deliveries");
    sqlx::query("DELETE FROM device_presence_events WHERE serial = $1")
        .bind(serial)
        .execute(pool)
        .await
        .expect("cleanup events");
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn events_and_deliveries_are_queryable_and_streamable() {
    let harness = Harness::new("dp").await;
    let serial = format!("dp_evt_{}", harness.suffix);
    let device_slot = format!("dp-slot-{}", harness.suffix);
    cleanup_events(harness.pool(), &serial).await;

    let now = chrono::Utc::now();
    let first = insert_event(
        harness.pool(),
        &serial,
        &device_slot,
        "online",
        "offline",
        now,
        json!({ "marker": serial, "room_name": "测试房间" }),
    )
    .await;
    let second = insert_event(
        harness.pool(),
        &serial,
        &device_slot,
        "offline",
        "online",
        now + chrono::Duration::seconds(1),
        json!({ "marker": serial }),
    )
    .await;
    assert!(second > first);

    // 列表：id 倒序，total = 本次返回条数。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        &format!(
            "/api/v1/admin/device-events?limit=200&after_id={}",
            first - 1
        ),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(body["total"].as_u64(), Some(items.len() as u64));
    assert_eq!(items.len(), 2, "{body}");
    assert_eq!(items[0]["id"].as_i64(), Some(second));
    assert_eq!(items[1]["id"].as_i64(), Some(first));
    assert_eq!(items[0]["serial"], serial);
    assert_eq!(items[0]["device_slot"], device_slot);
    assert_eq!(items[0]["previous_state"], "offline");
    assert_eq!(items[0]["current_state"], "online");
    assert_eq!(items[0]["payload"]["marker"], serial);
    assert_eq!(items[1]["previous_state"], "online");
    assert_eq!(items[1]["current_state"], "offline");
    assert!(items[0]["occurred_at"].is_string());

    // after_id 增量：只返回 id > after_id 的事件。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/device-events?after_id={first}"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["id"].as_i64(), Some(second));

    // 参数校验 + 权限。
    for uri in [
        "/api/v1/admin/device-events?limit=0".to_owned(),
        "/api/v1/admin/device-events?limit=201".to_owned(),
        "/api/v1/admin/device-events?after_id=-1".to_owned(),
    ] {
        let (status, body) =
            send_json(&harness.app, "GET", &uri, Some(&harness.admin_token), None).await;
        assert_eq!(status, 400, "{uri}: {body}");
    }
    // viewer 角色有 device:read；未认证请求为 401。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-events",
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-events",
        None,
        None,
    )
    .await;
    assert_eq!(status, 401, "{body}");

    // 三条不同状态的投递。
    for (suffix, delivery_status) in [
        ("pending", "pending"),
        ("delivered", "delivered"),
        ("failed", "failed"),
    ] {
        sqlx::query(
            "INSERT INTO device_presence_deliveries (event_id, target_url, status)
             VALUES ($1, $2, $3)
             ON CONFLICT (event_id, target_url) DO NOTHING",
        )
        .bind(second)
        .bind(format!("https://hooks.test/{serial}/{suffix}"))
        .bind(delivery_status)
        .execute(harness.pool())
        .await
        .expect("insert delivery");
    }
    let pending_url = format!("https://hooks.test/{serial}/pending");

    // 不传 status：至少包含我们的 3 条；返回结构 total = items.len()。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-presence-deliveries?limit=200",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(body["total"].as_u64(), Some(items.len() as u64));
    for expected in ["pending", "delivered", "failed"] {
        let url = format!("https://hooks.test/{serial}/{expected}");
        assert!(
            items.iter().any(|item| item["target_url"] == url
                && item["status"] == expected
                && item["event_id"].as_i64() == Some(second)),
            "missing delivery {expected}: {body}"
        );
    }

    // status 过滤：返回的每一行都必须是该状态，且包含我们的 pending 行。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-presence-deliveries?status=pending&limit=200",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    assert!(
        items.iter().all(|item| item["status"] == "pending"),
        "{body}"
    );
    assert!(
        items.iter().any(|item| item["target_url"] == pending_url),
        "{body}"
    );

    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-presence-deliveries?status=skipped",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-presence-deliveries?status=",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-presence-deliveries?limit=201",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/device-presence-deliveries",
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // SSE：after_id = first 后应立即补发第二条事件（按 id 正序）。
    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/admin/device-events/stream?after_id={first}"
                ))
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", harness.admin_token),
                )
                .header(header::ACCEPT, "text/event-stream")
                .body(Body::empty())
                .expect("sse request"),
        )
        .await
        .expect("sse response");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        content_type.starts_with("text/event-stream"),
        "content-type = {content_type}"
    );

    let mut body = response.into_body();
    let mut received = String::new();
    for _ in 0..10 {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("SSE frame timeout");
        let Some(frame) = frame else { break };
        let frame = frame.expect("SSE frame");
        if let Ok(data) = frame.into_data() {
            received.push_str(&String::from_utf8_lossy(&data));
            if received.contains("event: device.presence") {
                break;
            }
        }
    }
    assert!(
        received.contains("event: device.presence"),
        "missing device.presence frame: {received}"
    );
    assert!(received.contains(&serial), "missing serial: {received}");
    assert!(
        received.contains(&format!("id: {second}")),
        "missing event id: {received}"
    );

    // 清理：投递 → 事件 → 用户（审计）。
    cleanup_events(harness.pool(), &serial).await;
    harness.cleanup_users().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn area_webhook_url_create_update_semantics() {
    let harness = Harness::new("dpa").await;
    let code = format!("dpa_{}", harness.suffix);
    let code_empty = format!("dpae_{}", harness.suffix);
    let hook = "https://hooks.test/area-presence";
    let hook2 = "https://hooks.test/area-presence-v2";

    // create：合法 URL 往返。
    let (status, area) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/areas",
        Some(&harness.admin_token),
        Some(json!({
            "code": code,
            "name": "设备事件区域",
            "webhook_url": hook
        })),
    )
    .await;
    assert_eq!(status, 201, "{area}");
    let area_id = area["id"].as_str().expect("area id").to_owned();
    assert_eq!(area["webhook_url"], hook);

    // create：非法 URL 400。
    let (status, body) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/areas",
        Some(&harness.admin_token),
        Some(json!({
            "code": format!("dpab_{}", harness.suffix),
            "name": "非法 webhook 区域",
            "webhook_url": "ftp://hooks.test/nope"
        })),
    )
    .await;
    assert_eq!(status, 400, "{body}");

    // create：空字符串归一为 null。
    let (status, area_empty) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/areas",
        Some(&harness.admin_token),
        Some(json!({
            "code": code_empty,
            "name": "空 webhook 区域",
            "webhook_url": ""
        })),
    )
    .await;
    assert_eq!(status, 201, "{area_empty}");
    assert!(area_empty["webhook_url"].is_null(), "{area_empty}");
    let area_empty_id = area_empty["id"].as_str().expect("area id").to_owned();

    // PATCH 缺失 webhook_url：不修改。
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        Some(json!({ "name": "设备事件区域（改名）" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["webhook_url"], hook, "{body}");

    // PATCH null：清空。
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        Some(json!({ "webhook_url": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["webhook_url"].is_null(), "{body}");

    // 用 GET 确认持久化结果。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["webhook_url"].is_null(), "{body}");

    // 重新设置。
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        Some(json!({ "webhook_url": hook2 })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["webhook_url"], hook2);

    // 非法 URL：400 且保持原值。
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        Some(json!({ "webhook_url": "ftp://hooks.test/nope" })),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["webhook_url"], hook2, "{body}");

    // 空字符串等价于清空。
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        Some(json!({ "webhook_url": "" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["webhook_url"].is_null(), "{body}");

    // 没有任何字段 → 400。
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.admin_token),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400, "{body}");

    // viewer 无 area:manage。
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&harness.viewer_token),
        Some(json!({ "webhook_url": hook })),
    )
    .await;
    assert_eq!(status, 403, "{body}");

    // 清理区域（无房间，直接删）。
    for id in [&area_id, &area_empty_id] {
        let (status, body) = send_json(
            &harness.app,
            "DELETE",
            &format!("/api/v1/admin/areas/{id}"),
            Some(&harness.admin_token),
            None,
        )
        .await;
        assert!(status == 200 || status == 204, "{body}");
    }
    harness.cleanup_users().await;
}
