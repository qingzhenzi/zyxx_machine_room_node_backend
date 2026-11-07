//! 工单（Ticket / Work Order）集成测试：真实 PostgreSQL + Redis。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 或显式
//! `cargo test -p qe-api --test tickets -- --ignored --nocapture` 运行。
//!
//! 覆盖：
//! - 人工建单 / 列表过滤 / 详情（时间线 + 关联上下文）/ 统计；
//! - 状态机（claim → resolve → close → reopen，非法流转 409、缺 resolution 400）；
//! - 权限（viewer 只读、operator 可处理、ticket:manage 才能 PATCH/assign）；
//! - 自动单去重：部分唯一索引 + `occurrence_count`/`occurred` 事件幂等；
//! - worker 的三条规则 SQL（机器健康连续 N 条、设备离线阈值、投递失败聚合）。

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use qe_api::{
    api,
    cache::Cache,
    config::{Config, Environment},
    db::{Db, users::UserRepo},
    state::AppState,
    ticket_worker,
};
use qe_auth::password::hash_password_async;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "TicketIntegration@12345";

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
        public_enroll_endpoint: "https://enroll.test:4440".to_owned(),
        storage: None,
    }
}

/// 初始化一次日志，便于 `--nocapture` 时定位 500 的服务端原因。
fn init_tracing() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info,sqlx=warn".into()),
            )
            .try_init();
    });
}

async fn test_state() -> AppState {
    init_tracing();
    let db = Db::connect(&database_url(), 4)
        .await
        .expect("connect postgres");
    // 多个 ignored 测试并行运行时，sqlx 迁移 advisory lock 可能触发 3s lock_timeout，
    // 这里重试几次（迁移本身幂等）。
    let mut attempts = 0;
    loop {
        match db.migrate().await {
            Ok(()) => break,
            Err(error) if attempts < 20 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(500)).await;
                eprintln!("migrate retry {attempts}: {error}");
            }
            Err(error) => panic!("run migrations: {error}"),
        }
    }
    let cache = Cache::connect(&redis_url()).await.expect("connect redis");
    AppState::new(db, cache, test_config())
}

struct Harness {
    state: AppState,
    app: Router,
    admin_token: String,
    admin_id: Uuid,
    operator_token: String,
    operator_id: Uuid,
    viewer_token: String,
    viewer_id: Uuid,
    suffix: String,
    area_id: Uuid,
    room_id: Uuid,
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
        let admin_name = format!("{prefix}_admin_{suffix}");
        let operator_name = format!("{prefix}_op_{suffix}");
        let viewer_name = format!("{prefix}_viewer_{suffix}");

        let admin = users
            .create_with_roles(
                &admin_name,
                None,
                "Ticket Admin",
                &password_hash,
                &["admin".to_owned()],
            )
            .await
            .expect("create admin");
        // operator 角色由 migrations/0018_tickets.sql 播种。
        let operator = users
            .create_with_roles(
                &operator_name,
                None,
                "值班运维",
                &password_hash,
                &["operator".to_owned()],
            )
            .await
            .expect("create operator");
        let viewer = users
            .create_with_roles(
                &viewer_name,
                None,
                "Ticket Viewer",
                &password_hash,
                &["viewer".to_owned()],
            )
            .await
            .expect("create viewer");

        let admin_token = login(&app, &admin_name, PASSWORD).await;
        let operator_token = login(&app, &operator_name, PASSWORD).await;
        let viewer_token = login(&app, &viewer_name, PASSWORD).await;

        let pool = state.db.pool();
        let area_code = format!("tk_area_{suffix}");
        let area_id: Uuid = sqlx::query_scalar(
            "INSERT INTO areas (code, name, timezone) VALUES ($1, $2, 'Asia/Shanghai') RETURNING id",
        )
        .bind(&area_code)
        .bind(format!("工单测试区域 {suffix}"))
        .fetch_one(pool)
        .await
        .expect("create area");
        let room_code = format!("tk_room_{suffix}");
        let room_id: Uuid = sqlx::query_scalar(
            "INSERT INTO rooms (area_id, code, name) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(area_id)
        .bind(&room_code)
        .bind(format!("工单测试房间 {suffix}"))
        .fetch_one(pool)
        .await
        .expect("create room");

        Self {
            state,
            app,
            admin_token,
            admin_id: admin.id,
            operator_token,
            operator_id: operator.id,
            viewer_token,
            viewer_id: viewer.id,
            suffix,
            area_id,
            room_id,
        }
    }

    fn pool(&self) -> &PgPool {
        self.state.db.pool()
    }

    async fn create_ticket(&self, title: &str, priority: &str) -> Value {
        let (status, body) = send_json(
            &self.app,
            "POST",
            "/api/v1/admin/tickets",
            Some(&self.operator_token),
            Some(json!({
                "title": title,
                "description": "测试描述",
                "category": "other",
                "priority": priority,
                "room_id": self.room_id,
            })),
        )
        .await;
        assert_eq!(status, 201, "{body}");
        body
    }

    async fn cleanup(&self) {
        // 工单（事件级联）；自动规则测试可能按 serial 建单，也一并清理。
        sqlx::query("DELETE FROM tickets WHERE title LIKE $1 OR serial = $2")
            .bind(format!("%{}%", self.suffix))
            .bind(format!("tk-serial-{}", self.suffix))
            .execute(self.pool())
            .await
            .expect("cleanup tickets");
        sqlx::query("DELETE FROM ticket_events WHERE note LIKE $1")
            .bind(format!("%{}%", self.suffix))
            .execute(self.pool())
            .await
            .expect("cleanup ticket events");
        sqlx::query("DELETE FROM rooms WHERE id = $1")
            .bind(self.room_id)
            .execute(self.pool())
            .await
            .expect("cleanup room");
        sqlx::query("DELETE FROM areas WHERE id = $1")
            .bind(self.area_id)
            .execute(self.pool())
            .await
            .expect("cleanup area");
        sqlx::query("DELETE FROM audit_logs WHERE actor_id = ANY($1)")
            .bind(vec![self.admin_id, self.operator_id, self.viewer_id])
            .execute(self.pool())
            .await
            .expect("cleanup audit");
        sqlx::query("DELETE FROM users WHERE id = ANY($1)")
            .bind(vec![self.admin_id, self.operator_id, self.viewer_id])
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

/// 创建测试用自定义角色（仅本次测试使用，结束后删除）。
async fn create_test_role(pool: &PgPool, code: &str, permission_codes: &[&str]) {
    sqlx::query(
        "INSERT INTO roles (code, name, description, is_system)
         VALUES ($1, $2, '测试角色', FALSE)
         ON CONFLICT (code) DO NOTHING",
    )
    .bind(code)
    .bind(format!("测试角色 {code}"))
    .execute(pool)
    .await
    .expect("create role");
    let codes: Vec<String> = permission_codes
        .iter()
        .map(|code| (*code).to_owned())
        .collect();
    sqlx::query(
        "INSERT INTO role_permissions (role_id, permission_id)
         SELECT r.id, p.id FROM roles r JOIN permissions p ON p.code = ANY($2)
          WHERE r.code = $1
         ON CONFLICT DO NOTHING",
    )
    .bind(code)
    .bind(&codes)
    .execute(pool)
    .await
    .expect("grant role permissions");
}

/// 删除测试自定义角色（用户删除后会级联 user_roles）。
async fn drop_test_role(pool: &PgPool, code: &str) {
    sqlx::query("DELETE FROM roles WHERE code = $1 AND is_system = FALSE")
        .bind(code)
        .execute(pool)
        .await
        .expect("drop test role");
}

/// 插入 device_enrollment（machine_snapshots 的外键）。
async fn insert_enrollment(pool: &PgPool, serial: &str, device_slot: &str) {
    sqlx::query(
        "INSERT INTO device_enrollment
             (serial, device_slot, subject_cn, csr_sha256, cert_pem, status)
         VALUES ($1, $2, 'test-subject', $3, 'test-cert', 'active')",
    )
    .bind(serial)
    .bind(device_slot)
    .bind(vec![0u8; 32])
    .execute(pool)
    .await
    .expect("insert enrollment");
}

async fn insert_snapshot(
    pool: &PgPool,
    serial: &str,
    device_slot: &str,
    collected_at: DateTime<Utc>,
    health_status: &str,
    issues: Value,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO machine_snapshots
             (serial, device_slot, collected_at, agent_version, hostname,
              health_status, health_score, payload)
         VALUES ($1, $2, $3, '0.1.0', 'tk-host', $4, 42, $5)
         RETURNING id",
    )
    .bind(serial)
    .bind(device_slot)
    .bind(collected_at)
    .bind(health_status)
    .bind(json!({ "health": { "issues": issues } }))
    .fetch_one(pool)
    .await
    .expect("insert snapshot")
}

async fn cleanup_worker_fixtures(pool: &PgPool, serial: &str, room_id: Uuid, area_id: Uuid) {
    sqlx::query("DELETE FROM tickets WHERE serial = $1")
        .bind(serial)
        .execute(pool)
        .await
        .expect("cleanup serial tickets");
    sqlx::query("DELETE FROM machine_snapshots WHERE serial = $1")
        .bind(serial)
        .execute(pool)
        .await
        .expect("cleanup snapshots");
    sqlx::query("DELETE FROM device_session WHERE serial = $1")
        .bind(serial)
        .execute(pool)
        .await
        .expect("cleanup session");
    sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
        .bind(serial)
        .execute(pool)
        .await
        .expect("cleanup enrollment");
    sqlx::query("DELETE FROM rooms WHERE id = $1")
        .bind(room_id)
        .execute(pool)
        .await
        .expect("cleanup room");
    sqlx::query("DELETE FROM areas WHERE id = $1")
        .bind(area_id)
        .execute(pool)
        .await
        .expect("cleanup area");
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn ticket_lifecycle_state_machine_and_permissions() {
    let harness = Harness::new("tk").await;
    let title = format!("打印机无法开机 {}", harness.suffix);

    // ---- 创建（operator 有 ticket:create）----
    let ticket = harness.create_ticket(&title, "high").await;
    let ticket_id = ticket["id"].as_str().expect("id").to_owned();
    assert!(
        ticket["code"].as_str().expect("code").starts_with('T'),
        "{ticket}"
    );
    assert_eq!(ticket["status"], "open");
    assert_eq!(ticket["priority"], "high");
    assert_eq!(ticket["category"], "other");
    assert_eq!(ticket["source"], "manual");
    assert_eq!(ticket["reporter_name"], "值班运维");
    assert!(ticket["due_at"].as_str().is_some(), "应计算 SLA due_at");
    assert_eq!(ticket["room_id"], harness.room_id.to_string());

    // ---- 权限 ----
    let (status, _) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/tickets",
        Some(&harness.viewer_token),
        Some(json!({ "title": format!("viewer 不应能建单 {}", harness.suffix) })),
    )
    .await;
    assert_eq!(status, 403, "viewer 缺少 ticket:create");

    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets?page_size=100",
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "viewer 有 ticket:read：{body}");
    assert!(body["items"].as_array().expect("items").len() <= 100);

    // ---- 列表 / 过滤 ----
    let (status, body) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/tickets?keyword={}", harness.suffix),
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["total"], 1, "{body}");
    assert_eq!(body["items"][0]["id"], ticket_id.as_str());

    // keyword 里的 LIKE 通配符必须被转义：`%` 不应匹配全部工单（QA #10）。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets?keyword=%25",
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["total"], 0, "keyword=% 不应匹配全部工单：{body}");

    // ---- 详情：时间线 + 关联上下文 ----
    let detail_uri = format!("/api/v1/admin/tickets/{ticket_id}");
    let (status, detail) = send_json(
        &harness.app,
        "GET",
        &detail_uri,
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    let events = detail["events"].as_array().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "created");
    assert_eq!(events[0]["actor_name"], "值班运维");
    assert_eq!(detail["context"]["room_id"], harness.room_id.to_string());
    assert_eq!(detail["context"]["area_id"], harness.area_id.to_string());
    assert!(detail["context"]["room_name"].as_str().is_some());
    assert!(detail["context"]["online"].is_null() || detail["context"]["online"].is_boolean());

    // ---- 认领：open → in_progress ----
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/claim"),
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "in_progress");
    assert_eq!(body["assignee_id"], harness.operator_id.to_string());
    assert!(body["acknowledged_at"].as_str().is_some());

    // 已被他人认领 → 409
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/claim"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 409, "{body}");

    // ---- 认领幂等：自己重复认领返回 200 ----
    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/claim"),
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200);

    // ---- 解决必须带 resolution ----
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({ "to_status": "resolved" })),
    )
    .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({
            "to_status": "resolved",
            "resolution": "已更换电源线",
            "root_cause": "hardware",
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "resolved");
    assert_eq!(body["resolution"], "已更换电源线");
    assert_eq!(body["root_cause"], "hardware");
    assert!(body["resolved_at"].as_str().is_some());

    // 根因分类非法 → 400（任何流转都会先校验）。
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({ "to_status": "resolved", "resolution": "x", "root_cause": "bogus" })),
    )
    .await;
    assert_eq!(status, 400, "非法根因应 400：{body}");

    // 非法流转 resolved → in_progress → 409
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({ "to_status": "in_progress" })),
    )
    .await;
    assert_eq!(status, 409, "{body}");

    // ---- 关闭 → 重开 ----
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({ "to_status": "closed" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "closed");
    assert!(body["closed_at"].as_str().is_some());

    // mine = assignee_id = 当前用户（不限制状态；未结束过滤交给 active）。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets?mine=true",
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["total"], 1,
        "已关闭但分配给自己的工单也应在 mine 中：{body}"
    );
    assert_eq!(body["items"][0]["id"], ticket_id.as_str());

    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({ "to_status": "open" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "open");
    assert!(body["resolved_at"].is_null(), "重开应清空 resolved_at");
    assert!(body["closed_at"].is_null(), "重开应清空 closed_at");

    // ---- PATCH 内容（ticket:manage）----
    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &detail_uri,
        Some(&harness.operator_token),
        Some(json!({ "title": "无权限修改" })),
    )
    .await;
    assert_eq!(status, 403, "operator 没有 ticket:manage：{body}");

    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &detail_uri,
        Some(&harness.admin_token),
        Some(json!({ "title": format!("更新后的标题 {}", harness.suffix), "priority": "urgent" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["priority"], "urgent");
    assert_eq!(
        body["title"],
        format!("更新后的标题 {}", harness.suffix).as_str()
    );

    // PATCH 仅改标题/描述也必须写时间线：updated 事件（QA #1）。
    let (status, detail) = send_json(
        &harness.app,
        "GET",
        &detail_uri,
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    let events = detail["events"].as_array().expect("events");
    let updated_event = events
        .iter()
        .find(|event| event["kind"] == "updated")
        .expect("PATCH 标题/优先级后应有 updated 事件");
    let changed_fields = updated_event["payload"]["changed_fields"]
        .as_array()
        .expect("changed_fields");
    assert!(
        changed_fields.iter().any(|field| field == "title"),
        "{updated_event}"
    );
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "priority_changed"),
        "优先级变更应保持 priority_changed 事件"
    );
    assert_eq!(updated_event["actor_name"], "Ticket Admin");

    // ---- 转派：目标必须拥有 ticket:handle ----
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/assign"),
        Some(&harness.admin_token),
        Some(json!({ "assignee_id": harness.viewer_id })),
    )
    .await;
    assert_eq!(status, 400, "viewer 没有 ticket:handle：{body}");

    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/assign"),
        Some(&harness.admin_token),
        Some(json!({ "assignee_id": harness.operator_id })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["assignee_id"], harness.operator_id.to_string());

    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/assign"),
        Some(&harness.admin_token),
        Some(json!({ "assignee_id": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["assignee_id"].is_null());

    // ---- assign 空 body / 缺少 assignee_id → 400（QA #7）----
    let conflict_title = format!("冲突测试 {}", harness.suffix);
    let conflict_ticket = harness.create_ticket(&conflict_title, "normal").await;
    let conflict_id = conflict_ticket["id"].as_str().expect("id").to_owned();

    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{conflict_id}/assign"),
        Some(&harness.admin_token),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 400, "缺少 assignee_id 不应静默释放：{body}");

    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{conflict_id}/assign"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 400, "空 body 应返回 400");

    // 转派给 admin 后，operator 用 transition 开始处理 → 409（QA #6）。
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{conflict_id}/assign"),
        Some(&harness.admin_token),
        Some(json!({ "assignee_id": harness.admin_id })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{conflict_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({ "to_status": "in_progress" })),
    )
    .await;
    assert_eq!(status, 409, "已转派给他人的工单不能被他人开始处理：{body}");
    // 显式 null 释放认领后才允许开始处理。
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{conflict_id}/assign"),
        Some(&harness.admin_token),
        Some(json!({ "assignee_id": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{conflict_id}/transition"),
        Some(&harness.operator_token),
        Some(json!({ "to_status": "in_progress" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["assignee_id"], harness.operator_id.to_string());

    // ---- 评论 ----
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/comments"),
        Some(&harness.operator_token),
        Some(json!({ "body": "已到现场，正在排查" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["kind"], "commented");
    assert_eq!(body["actor_name"], "值班运维");

    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/comments"),
        Some(&harness.viewer_token),
        Some(json!({ "body": "viewer 不应能评论" })),
    )
    .await;
    assert_eq!(status, 403);

    // ---- 我的待办 / 未分配过滤 ----
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets?mine=true&active=true",
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let items = body["items"].as_array().expect("items");
    // 主工单已释放 assignee，不在 mine 中；冲突测试单在办、属于 operator，应在 mine 中。
    assert!(
        items.iter().all(|item| item["id"] != ticket_id.as_str()),
        "已释放的工单不应出现在 mine=true&active=true：{body}"
    );
    assert!(
        items.iter().any(|item| item["id"] == conflict_id.as_str()),
        "在办的冲突测试单应出现在 mine 中：{body}"
    );

    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets?unassigned=true&active=true",
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // ---- 卡片计数 ----
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets/summary",
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    for status_key in ["open", "in_progress", "resolved", "closed", "cancelled"] {
        assert!(body["by_status"].get(status_key).is_some(), "{body}");
    }

    // ---- 统计（默认近 30 天）----
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets/stats",
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["totals"]["created"].as_i64().unwrap_or(0) >= 1,
        "{body}"
    );
    assert!(body["by_day"].is_array());
    assert!(body["by_category"].is_array());
    assert!(body["top_rooms"].is_array());

    // ---- 转派候选人（ticket:handle）----
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets/assignees",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let operator_id = harness.operator_id.to_string();
    let ids: Vec<&str> = body
        .as_array()
        .expect("assignees")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&operator_id.as_str()));

    // ---- 404 ----
    let (status, _) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/tickets/{}", Uuid::new_v4()),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 404);

    // ---- 审计 ----
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs
          WHERE actor_id = $1 AND resource_type = 'ticket'
            AND action IN ('ticket.create','ticket.status','ticket.assign','ticket.comment')",
    )
    .bind(harness.operator_id)
    .fetch_one(harness.pool())
    .await
    .expect("count audits");
    assert!(audits >= 4, "人工写操作应写审计，实际 {audits}");

    // transition 审计必须记录真实旧状态（QA #3）：最近一次是 closed→open 重开。
    let (from_status, to_status): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT detail->>'from_status', detail->>'to_status'
           FROM audit_logs
          WHERE actor_id = $1 AND action = 'ticket.status'
            AND resource_id = $2 AND detail->>'action' = 'transition'
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(harness.operator_id)
    .bind(&ticket_id)
    .fetch_one(harness.pool())
    .await
    .expect("transition audit");
    assert_eq!(
        from_status.as_deref(),
        Some("closed"),
        "from_status 应为旧状态"
    );
    assert_eq!(to_status.as_deref(), Some("open"));

    harness.cleanup().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn ticket_dedupe_bumps_occurrence_once_per_source() {
    let harness = Harness::new("tkd").await;
    let dedupe_key = format!("machine_health:tk-serial-{}", harness.suffix);
    let serial = format!("tk-serial-{}", harness.suffix);

    let new_ticket = |snapshot_id: i64| qe_api::domain::ticket::NewTicket {
        title: format!("机器健康异常 {}", harness.suffix),
        description: "dedupe 测试".to_owned(),
        priority: qe_api::domain::ticket::TicketPriority::High,
        category: qe_api::domain::ticket::TicketCategory::MachineHealth,
        source: qe_api::domain::ticket::TicketSource::Auto,
        source_type: Some("machine_snapshot".to_owned()),
        source_id: Some(snapshot_id.to_string()),
        dedupe_key: Some(dedupe_key.clone()),
        area_id: Some(harness.area_id),
        room_id: Some(harness.room_id),
        serial: Some(serial.clone()),
        device_slot: None,
        reporter_id: None,
        metadata: json!({ "snapshot_id": snapshot_id, "health_status": "critical" }),
    };

    let first = harness
        .state
        .tickets
        .insert_ticket(&new_ticket(1))
        .await
        .expect("first insert");
    assert_eq!(first.occurrence_count, 1);

    // QA #2：created 事件已记录首条源行，同一 source_id 再 bump（模拟多副本第二副本）
    // 不应把 occurrence 计成 2。
    let bumped_same_source = harness
        .state
        .tickets
        .bump_occurrence(&dedupe_key, "machine_snapshot", "1", &json!({}))
        .await
        .expect("bump same source");
    assert!(
        bumped_same_source.is_none(),
        "同一源行已被 created 记录，bump 应返回 None"
    );
    let after_replica = harness
        .state
        .tickets
        .insert_ticket(&new_ticket(1))
        .await
        .expect("second replica insert");
    assert_eq!(
        after_replica.occurrence_count, 1,
        "多副本对同一源行重复建单不应双计"
    );

    // 同一 dedupe_key、不同 source_id → 累加重复发生。
    let second = harness
        .state
        .tickets
        .insert_ticket(&new_ticket(2))
        .await
        .expect("second insert");
    assert_eq!(second.id, first.id, "不应新建第二张未结束工单");
    assert_eq!(second.occurrence_count, 2);
    assert_eq!(second.metadata["snapshot_id"], 2);

    // 同一 source_id 再次 bump / 重复上报 → 幂等，不再累加。
    let bumped_again = harness
        .state
        .tickets
        .bump_occurrence(&dedupe_key, "machine_snapshot", "2", &json!({}))
        .await
        .expect("bump occurred source");
    assert!(bumped_again.is_none(), "已记录的源行不应再次累加");
    let third = harness
        .state
        .tickets
        .insert_ticket(&new_ticket(2))
        .await
        .expect("third insert");
    assert_eq!(third.occurrence_count, 2, "同一源行只追加一次 occurred");

    let (ticket, events) = harness
        .state
        .tickets
        .get(first.id)
        .await
        .expect("get")
        .expect("ticket");
    assert_eq!(ticket.occurrence_count, 2);
    let occurred = events
        .iter()
        .filter(|event| event.kind == qe_api::domain::ticket::TicketEventKind::Occurred)
        .count();
    assert_eq!(
        occurred, 1,
        "首条源行记录为 created 事件；第二条新源行追加 1 条 occurred（同一源行幂等）"
    );

    // 取消后同一 dedupe_key 可以再开新单（部分唯一索引只约束未结束状态）。
    harness
        .state
        .tickets
        .add_comment(first.id, harness.operator_id, "关闭前备注")
        .await
        .expect("comment");
    harness
        .state
        .tickets
        .transition(
            first.id,
            &qe_api::domain::ticket::TransitionInput {
                to_status: qe_api::domain::ticket::TicketStatus::Cancelled,
                resolution: None,
                root_cause: None,
                note: Some("重复问题已合并".to_owned()),
            },
            harness.operator_id,
            Utc::now(),
        )
        .await
        .expect("cancel");
    let reopened = harness
        .state
        .tickets
        .insert_ticket(&new_ticket(3))
        .await
        .expect("insert after close");
    assert_ne!(reopened.id, first.id, "关闭/取消后同 dedupe_key 应开新单");
    assert_eq!(reopened.occurrence_count, 1);

    harness.cleanup().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn worker_signal_queries_detect_machine_offline_and_delivery_failures() {
    let harness = Harness::new("tkw").await;
    let pool = harness.pool();
    let serial = format!("tk-serial-{}", harness.suffix);
    let device_slot = format!("tk-slot-{}", harness.suffix);
    let area_code = format!("tk_w_area_{}", harness.suffix);
    let room_code = format!("tk_w_room_{}", harness.suffix);
    // 先清理上次失败留下的同 serial 残留（避免 FK/唯一约束冲突）。
    for statement in [
        "DELETE FROM tickets WHERE serial = $1",
        "DELETE FROM machine_snapshots WHERE serial = $1",
        "DELETE FROM device_session WHERE serial = $1",
        "DELETE FROM device_enrollment WHERE serial = $1",
    ] {
        sqlx::query(statement)
            .bind(&serial)
            .execute(pool)
            .await
            .expect("cleanup worker residue");
    }

    let area_id: Uuid = sqlx::query_scalar(
        "INSERT INTO areas (code, name, timezone) VALUES ($1, $2, 'Asia/Shanghai') RETURNING id",
    )
    .bind(&area_code)
    .bind(format!("工单规则区域 {}", harness.suffix))
    .fetch_one(pool)
    .await
    .expect("create area");
    let room_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rooms (area_id, code, name, device_slot) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(area_id)
    .bind(&room_code)
    .bind(format!("工单规则房间 {}", harness.suffix))
    .bind(&device_slot)
    .fetch_one(pool)
    .await
    .expect("create room");

    insert_enrollment(pool, &serial, &device_slot).await;

    // ---- 机器健康规则：最近连续 2 条非 ok 才命中；ok 会重置 ----
    let now = Utc::now();
    insert_snapshot(
        pool,
        &serial,
        &device_slot,
        now - chrono::Duration::seconds(120),
        "critical",
        json!([{ "severity": "critical", "code": "disk", "message": "磁盘满" }]),
    )
    .await;
    insert_snapshot(
        pool,
        &serial,
        &device_slot,
        now - chrono::Duration::seconds(60),
        "warning",
        json!([{ "severity": "warning", "code": "cpu", "message": "CPU 高" }]),
    )
    .await;
    let candidates = harness
        .state
        .tickets
        .machine_health_candidates(2)
        .await
        .expect("machine candidates");
    let signal = candidates
        .iter()
        .find(|signal| signal.serial == serial)
        .expect("应命中最近连续 2 条非 ok");
    assert_eq!(signal.health_status, "warning");
    assert_eq!(signal.room_id, Some(room_id), "应按槽位映射到房间");

    // 最新一条恢复 ok → 不再是候选。
    insert_snapshot(pool, &serial, &device_slot, now, "ok", json!([])).await;
    let candidates = harness
        .state
        .tickets
        .machine_health_candidates(2)
        .await
        .expect("machine candidates");
    assert!(!candidates.iter().any(|signal| signal.serial == serial));

    // ---- 设备离线规则 ----
    sqlx::query(
        "INSERT INTO device_session (serial, device_slot, state, updated_at)
         VALUES ($1, $2, 'offline', now() - interval '30 minutes')
         ON CONFLICT (serial) DO UPDATE SET state = 'offline',
             updated_at = now() - interval '30 minutes'",
    )
    .bind(&serial)
    .bind(&device_slot)
    .execute(pool)
    .await
    .expect("insert session");
    let offline = harness
        .state
        .tickets
        .device_offline_candidates(10)
        .await
        .expect("offline candidates");
    let offline_signal = offline
        .iter()
        .find(|signal| signal.serial == serial)
        .expect("离线超过 10 分钟应命中");
    assert_eq!(offline_signal.room_id, room_id);
    assert_eq!(offline_signal.room_code, room_code);

    // ---- 推送失败规则（按房间 + 目标 + 本地日期聚合）----
    let event_key = format!("tk-event-{}", harness.suffix);
    let event_id: Uuid = sqlx::query_scalar(
        "INSERT INTO schedule_events
             (event_key, room_id, timezone, event_kind, title, payload, triggered_at)
         VALUES ($1, $2, 'Asia/Shanghai', 'test', '推送失败测试', '{}'::jsonb, now())
         RETURNING id",
    )
    .bind(&event_key)
    .bind(room_id)
    .fetch_one(pool)
    .await
    .expect("insert schedule event");
    sqlx::query(
        "INSERT INTO schedule_event_deliveries
             (event_id, target, status, attempts, last_error)
         VALUES ($1, 'webhook', 'failed', 5, 'connection refused')",
    )
    .bind(event_id)
    .execute(pool)
    .await
    .expect("insert failed delivery");
    let failed = harness
        .state
        .tickets
        .failed_delivery_candidates(24)
        .await
        .expect("delivery candidates");
    let delivery = failed
        .iter()
        .find(|signal| signal.event_id == event_id)
        .expect("失败投递应命中");
    assert_eq!(delivery.target, "webhook");
    assert_eq!(delivery.room_code, room_code);
    assert!(
        delivery.local_date.chars().count() == 10,
        "本地日期应为 YYYY-MM-DD：{}",
        delivery.local_date
    );

    // ---- run_tick 首轮只建基线（不重放历史）----
    let config = qe_api::config::TicketWorkerConfig {
        auto_enabled: true,
        tick_secs: 60,
        device_offline_minutes: 10,
        health_consecutive: 2,
        lookback_hours: 24,
        email_enabled: false,
        overdue_remind_hours: 24,
    };
    let mut tracker = ticket_worker::TicketWorkerTracker::default();
    ticket_worker::run_tick(&harness.state, &config, &mut tracker).await;
    assert!(tracker.initialized, "首轮后应建立基线");
    let tickets_before: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE serial = $1")
        .bind(&serial)
        .fetch_one(pool)
        .await
        .expect("count tickets");
    assert_eq!(tickets_before, 0, "首轮不应重放历史问题");

    // 清理
    sqlx::query("DELETE FROM schedule_event_deliveries WHERE event_id = $1")
        .bind(event_id)
        .execute(pool)
        .await
        .expect("cleanup deliveries");
    sqlx::query("DELETE FROM schedule_events WHERE id = $1")
        .bind(event_id)
        .execute(pool)
        .await
        .expect("cleanup event");
    cleanup_worker_fixtures(pool, &serial, room_id, area_id).await;
    harness.cleanup().await;
}

#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
#[tokio::test]
async fn p1_rbac_rules_export_and_ticket_only_role() {
    let harness = Harness::new("tk_p1").await;
    let users = UserRepo::new(harness.pool().clone());
    let password_hash = hash_password_async(PASSWORD.to_owned())
        .await
        .expect("hash password");
    let handler_name = format!("tk_p1_handler_{}", harness.suffix);
    let handler_email = format!("{handler_name}@example.com");
    let handler = users
        .create_with_roles(
            &handler_name,
            Some(&handler_email),
            "工单处理员",
            &password_hash,
            &["ticket_operator".to_owned()],
        )
        .await
        .expect("create ticket_operator");
    let handler_token = login(&harness.app, &handler_name, PASSWORD).await;

    // ---- ticket-only 角色：有工单权限、没有 device/area/room 权限 ----
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/auth/me",
        Some(&handler_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let permissions: Vec<&str> = body["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(permissions.contains(&"ticket:read"));
    assert!(permissions.contains(&"ticket:claim"));
    assert!(!permissions.contains(&"device:read"));
    assert!(!permissions.contains(&"room:read"));

    let title = format!("P1 工单处理员测试 {}", harness.suffix);
    let (status, ticket) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/tickets",
        Some(&handler_token),
        Some(json!({ "title": title, "priority": "normal", "room_id": harness.room_id })),
    )
    .await;
    assert_eq!(status, 201, "{ticket}");
    let ticket_id = ticket["id"].as_str().expect("id").to_owned();

    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/claim"),
        Some(&handler_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/comments"),
        Some(&handler_token),
        Some(json!({ "body": "p1 处理员评论" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/transition"),
        Some(&handler_token),
        Some(json!({ "to_status": "resolved", "resolution": "p1 处理完成" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // 没有 update/assign 权限：PATCH 与转派应 403。
    let (status, _) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/tickets/{ticket_id}"),
        Some(&handler_token),
        Some(json!({ "title": "越权修改" })),
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/assign"),
        Some(&handler_token),
        Some(json!({ "assignee_id": handler.id })),
    )
    .await;
    assert_eq!(status, 403);

    // ---- 规则 CRUD / 权限 / 阈值校验 ----
    let (status, _) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/ticket-rules",
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 403, "viewer 不应能查看规则");
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/ticket-rules",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["total"].as_i64().unwrap_or(0) >= 6, "{body}");

    // 多段权限码（P1 细粒度）必须能授予自定义角色（QA Wave2 P2 回归）。
    let multi_role = format!("tk_multi_{}", harness.suffix);
    let (status, body) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/roles",
        Some(&harness.admin_token),
        Some(json!({
            "code": multi_role,
            "name": "多段权限测试",
            "permission_codes": ["ticket:rule:manage", "ticket:scope:all"]
        })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert!(body["permissions"].as_array().is_some());
    let (status, _) = send_json(
        &harness.app,
        "DELETE",
        &format!(
            "/api/v1/admin/roles/{}",
            body["id"].as_str().expect("role id")
        ),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 204);

    let (status, rule) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/ticket-rules",
        Some(&harness.admin_token),
        Some(json!({
            "name": format!("P1 规则 {}", harness.suffix),
            "signal_type": "machine_health",
            "priority": "high",
            "thresholds": { "consecutive": 3 }
        })),
    )
    .await;
    assert_eq!(status, 201, "{rule}");
    let rule_id = rule["id"].as_str().expect("rule id").to_owned();
    assert_eq!(rule["thresholds"]["consecutive"], 3);

    let (status, body) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/ticket-rules",
        Some(&harness.admin_token),
        Some(json!({
            "name": format!("坏规则 {}", harness.suffix),
            "signal_type": "machine_health",
            "thresholds": { "consecutive": 999 }
        })),
    )
    .await;
    assert_eq!(status, 400, "越界阈值应 400：{body}");

    let (status, body) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/ticket-rules/{rule_id}"),
        Some(&harness.admin_token),
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["enabled"], false);

    let (status, _) = send_json(
        &harness.app,
        "DELETE",
        &format!("/api/v1/admin/ticket-rules/{rule_id}"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 204);

    // ---- CSV 导出：BOM + 中文表头 + 命中筛选 ----
    let request = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/admin/tickets/export?keyword={}",
            harness.suffix
        ))
        .header(header::AUTHORIZATION, format!("Bearer {handler_token}"))
        .body(Body::empty())
        .expect("request");
    let response = harness.app.clone().oneshot(request).await.expect("export");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/csv; charset=utf-8")
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let csv = String::from_utf8_lossy(&bytes);
    assert!(csv.starts_with('\u{feff}'), "CSV 应带 UTF-8 BOM");
    assert!(csv.contains("编号,标题,状态"), "{csv}");
    assert!(csv.contains(&title), "导出应包含本次工单：{csv}");

    // 清理本用例额外创建的用户/工单。
    sqlx::query("DELETE FROM tickets WHERE id = $1")
        .bind(Uuid::parse_str(&ticket_id).expect("uuid"))
        .execute(harness.pool())
        .await
        .expect("cleanup p1 ticket");
    sqlx::query("DELETE FROM audit_logs WHERE actor_id = $1")
        .bind(handler.id)
        .execute(harness.pool())
        .await
        .expect("cleanup p1 audit");
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(handler.id)
        .execute(harness.pool())
        .await
        .expect("cleanup p1 user");
    harness.cleanup().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn wave2_data_scopes_limit_reads_and_writes() {
    let harness = Harness::new("tks").await;
    let pool = harness.pool();
    let suffix = harness.suffix.clone();

    // 第二个区域 + 房间（用来验证范围外不可见/不可写）。
    let area_b: Uuid = sqlx::query_scalar(
        "INSERT INTO areas (code, name, timezone) VALUES ($1, $2, 'Asia/Shanghai') RETURNING id",
    )
    .bind(format!("tks_area_b_{suffix}"))
    .bind(format!("范围外区域 {suffix}"))
    .fetch_one(pool)
    .await
    .expect("create area b");
    let room_b: Uuid = sqlx::query_scalar(
        "INSERT INTO rooms (area_id, code, name) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(area_b)
    .bind(format!("tks_room_b_{suffix}"))
    .bind(format!("范围外房间 {suffix}"))
    .fetch_one(pool)
    .await
    .expect("create room b");

    // 区域 A 工单（harness.create_ticket 用 harness 房间）。
    let ticket_a = harness
        .create_ticket(&format!("范围内工单 {suffix}"), "normal")
        .await;
    let ticket_a_id = ticket_a["id"].as_str().expect("id").to_owned();
    // 区域 B 工单。
    let (status, ticket_b) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/tickets",
        Some(&harness.operator_token),
        Some(json!({
            "title": format!("范围外工单 {suffix}"),
            "category": "other",
            "priority": "normal",
            "room_id": room_b,
        })),
    )
    .await;
    assert_eq!(status, 201, "{ticket_b}");
    let ticket_b_id = ticket_b["id"].as_str().expect("id").to_owned();

    // 两个自定义用户：scoped（仅区域 A）与 scope:all。
    let users = UserRepo::new(pool.clone());
    let password_hash = hash_password_async(PASSWORD.to_owned())
        .await
        .expect("hash password");
    let scoped_role = format!("tks_scoped_{suffix}");
    let all_role = format!("tks_all_{suffix}");
    create_test_role(
        pool,
        &scoped_role,
        &[
            "ticket:read",
            "ticket:claim",
            "ticket:comment",
            "ticket:transition",
        ],
    )
    .await;
    create_test_role(pool, &all_role, &["ticket:read", "ticket:scope:all"]).await;
    let scoped_name = format!("tks_scoped_user_{suffix}");
    let all_name = format!("tks_all_user_{suffix}");
    let scoped = users
        .create_with_roles(
            &scoped_name,
            None,
            "范围内用户",
            &password_hash,
            &[scoped_role.clone()],
        )
        .await
        .expect("create scoped user");
    let all = users
        .create_with_roles(
            &all_name,
            None,
            "全部区域用户",
            &password_hash,
            &[all_role.clone()],
        )
        .await
        .expect("create scope:all user");
    let scoped_token = login(&harness.app, &scoped_name, PASSWORD).await;
    let all_token = login(&harness.app, &all_name, PASSWORD).await;

    // admin 设置 scoped 用户仅能访问区域 A。
    let (status, body) = send_json(
        &harness.app,
        "PUT",
        &format!("/api/v1/admin/ticket-scopes/{}", scoped.id),
        Some(&harness.admin_token),
        Some(json!({ "area_ids": [harness.area_id] })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["area_ids"], json!([harness.area_id]));

    // 范围列表包含该用户。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/ticket-scopes",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body.as_array()
            .expect("scopes")
            .iter()
            .any(|row| row["user_id"] == scoped.id.to_string()
                && row["area_ids"] == json!([harness.area_id])),
        "{body}"
    );

    // operator（只有工单处理权限，没有 scope:manage/manage）不能管理范围。
    let (status, _) = send_json(
        &harness.app,
        "PUT",
        &format!("/api/v1/admin/ticket-scopes/{}", scoped.id),
        Some(&harness.operator_token),
        Some(json!({ "area_ids": [] })),
    )
    .await;
    assert_eq!(status, 403);

    // 不存在的区域 → 404。
    let (status, _) = send_json(
        &harness.app,
        "PUT",
        &format!("/api/v1/admin/ticket-scopes/{}", scoped.id),
        Some(&harness.admin_token),
        Some(json!({ "area_ids": [Uuid::new_v4()] })),
    )
    .await;
    assert_eq!(status, 404);

    // 有范围的用户：只看得到区域 A 的工单。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets?keyword=&page_size=100",
        Some(&scoped_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&ticket_a_id.as_str()), "{body}");
    assert!(
        !ids.contains(&ticket_b_id.as_str()),
        "范围外工单不应可见：{body}"
    );

    // 范围外详情 → 404；范围内详情 → 200。
    let (status, _) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/tickets/{ticket_b_id}"),
        Some(&scoped_token),
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/tickets/{ticket_a_id}"),
        Some(&scoped_token),
        None,
    )
    .await;
    assert_eq!(status, 200);

    // 范围外写操作 → 403（claim/transition/comment 都校验）。
    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_b_id}/claim"),
        Some(&scoped_token),
        None,
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_b_id}/comments"),
        Some(&scoped_token),
        Some(json!({ "body": "越权评论" })),
    )
    .await;
    assert_eq!(status, 403);

    // summary/stats 也按范围过滤。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets/summary",
        Some(&scoped_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["by_status"]["open"], 1,
        "范围内应只有 1 张 open：{body}"
    );

    // scope:all 用户能看到两个区域的工单。
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/tickets?page_size=100",
        Some(&all_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&ticket_a_id.as_str()) && ids.contains(&ticket_b_id.as_str()));

    // 清理：范围行随用户删除级联。
    sqlx::query("DELETE FROM audit_logs WHERE actor_id = ANY($1)")
        .bind(vec![scoped.id, all.id])
        .execute(pool)
        .await
        .expect("cleanup audit");
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![scoped.id, all.id])
        .execute(pool)
        .await
        .expect("cleanup users");
    drop_test_role(pool, &scoped_role).await;
    drop_test_role(pool, &all_role).await;
    sqlx::query("DELETE FROM rooms WHERE id = $1")
        .bind(room_b)
        .execute(pool)
        .await
        .expect("cleanup room b");
    sqlx::query("DELETE FROM areas WHERE id = $1")
        .bind(area_b)
        .execute(pool)
        .await
        .expect("cleanup area b");
    harness.cleanup().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn wave2_attachments_register_list_and_delete() {
    let harness = Harness::new("tka").await;
    let suffix = harness.suffix.clone();
    let ticket = harness
        .create_ticket(&format!("附件工单 {suffix}"), "normal")
        .await;
    let ticket_id = ticket["id"].as_str().expect("id").to_owned();
    let sha = "b".repeat(64);

    // 测试配置未启用对象存储：presign → 503。
    let (status, body) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments/presign"),
        Some(&harness.admin_token),
        Some(json!({
            "filename": "note.txt",
            "content_type": "text/plain",
            "size": 10,
            "sha256": sha,
        })),
    )
    .await;
    assert_eq!(status, 503, "未配置存储时 presign 应 503：{body}");

    // 参数校验先于存储检查：非法大小/sha/content-type → 400。
    for payload in [
        json!({ "filename": "note.txt", "content_type": "text/plain", "size": 0, "sha256": sha }),
        json!({ "filename": "note.txt", "content_type": "application/exe", "size": 10, "sha256": sha }),
        json!({ "filename": "note.txt", "content_type": "text/plain", "size": 10, "sha256": "abc" }),
    ] {
        let (status, body) = send_json(
            &harness.app,
            "POST",
            &format!("/api/v1/admin/tickets/{ticket_id}/attachments/presign"),
            Some(&harness.admin_token),
            Some(payload),
        )
        .await;
        assert_eq!(status, 400, "{body}");
    }

    // 登记（元数据）不依赖对象存储。
    let object_key = format!("tickets/{ticket_id}/{}/note.txt", Uuid::new_v4());
    let register_payload = json!({
        "object_key": object_key,
        "filename": "note.txt",
        "content_type": "text/plain",
        "size": 10,
        "sha256": sha,
    });
    let (status, attachment) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments"),
        Some(&harness.admin_token),
        Some(register_payload.clone()),
    )
    .await;
    assert_eq!(status, 201, "{attachment}");
    let attachment_id = attachment["id"].as_str().expect("id").to_owned();
    assert_eq!(attachment["filename"], "note.txt");
    assert_eq!(attachment["size"], 10);

    // object_key 必须属于该工单。
    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments"),
        Some(&harness.admin_token),
        Some(json!({
            "object_key": format!("tickets/{}/x/note.txt", Uuid::new_v4()),
            "filename": "note.txt",
            "content_type": "text/plain",
            "size": 10,
            "sha256": sha,
        })),
    )
    .await;
    assert_eq!(status, 400);

    // 重复 object_key → 409。
    let (status, _) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments"),
        Some(&harness.admin_token),
        Some(register_payload),
    )
    .await;
    assert_eq!(status, 409);

    // 列表：ticket:read 可见，未配置存储时 download_url = null。
    let (status, list) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments"),
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    let rows = list.as_array().expect("attachments");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], attachment_id.as_str());
    assert!(rows[0]["download_url"].is_null());
    assert_eq!(rows[0]["uploaded_by_name"], "Ticket Admin");

    // viewer 没有 ticket:attachment/handle/manage → 删除 403。
    let (status, _) = send_json(
        &harness.app,
        "DELETE",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments/{attachment_id}"),
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 403);

    // 上传者（admin）删除 → 204；再删 → 404。
    let (status, _) = send_json(
        &harness.app,
        "DELETE",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments/{attachment_id}"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = send_json(
        &harness.app,
        "DELETE",
        &format!("/api/v1/admin/tickets/{ticket_id}/attachments/{attachment_id}"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 404);

    harness.cleanup().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn wave3_inspection_plans_crud_run_and_atomic_claim() {
    let harness = Harness::new("tki").await;
    let pool = harness.pool().clone();
    let suffix = harness.suffix.clone();

    // ---- 权限：operator 没有 ticket:inspection:manage / ticket:manage → 403 ----
    let (status, _) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/inspection-plans",
        Some(&harness.operator_token),
        None,
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/inspection-plans",
        Some(&harness.operator_token),
        Some(json!({ "name": "越权计划", "interval_hours": 24 })),
    )
    .await;
    assert_eq!(status, 403);

    // ---- 参数校验（admin）----
    for payload in [
        json!({ "name": "", "interval_hours": 24 }),
        json!({ "name": "周期非法", "interval_hours": 0 }),
        json!({ "name": "周期过大", "interval_hours": 169 }),
        json!({ "name": "缺 scope_id", "scope_type": "area", "interval_hours": 24 }),
    ] {
        let (status, body) = send_json(
            &harness.app,
            "POST",
            "/api/v1/admin/inspection-plans",
            Some(&harness.admin_token),
            Some(payload),
        )
        .await;
        assert_eq!(status, 400, "{body}");
    }
    let (status, _) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/inspection-plans",
        Some(&harness.admin_token),
        Some(json!({
            "name": "范围不存在",
            "scope_type": "area",
            "scope_id": Uuid::new_v4(),
            "interval_hours": 24,
        })),
    )
    .await;
    assert_eq!(status, 404);

    // ---- 创建 / 列表 / PATCH ----
    let plan_name = format!("巡检计划 {suffix}");
    let (status, plan) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/inspection-plans",
        Some(&harness.admin_token),
        Some(json!({
            "name": plan_name,
            "scope_type": "global",
            "interval_hours": 2,
            "enabled": true,
        })),
    )
    .await;
    assert_eq!(status, 201, "{plan}");
    let plan_id = plan["id"].as_str().expect("plan id").to_owned();
    assert_eq!(plan["name"], plan_name.as_str());
    assert_eq!(plan["interval_hours"], 2);
    assert!(plan["last_run"].is_null());

    let updated_name = format!("巡检计划更新 {suffix}");
    let (status, plan) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/inspection-plans/{plan_id}"),
        Some(&harness.admin_token),
        Some(json!({ "name": updated_name, "interval_hours": 3, "enabled": false })),
    )
    .await;
    assert_eq!(status, 200, "{plan}");
    assert_eq!(plan["name"], updated_name.as_str());
    assert_eq!(plan["interval_hours"], 3);
    assert_eq!(plan["enabled"], false);

    let (status, list) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/inspection-plans",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    assert!(
        list.as_array()
            .expect("plans")
            .iter()
            .any(|row| row["id"] == plan_id.as_str()),
        "{list}"
    );

    // ---- 构造机器快照 fixture（critical）----
    let serial = format!("tk-serial-{suffix}");
    let device_slot = format!("tk-slot-{suffix}");
    insert_enrollment(&pool, &serial, &device_slot).await;
    insert_snapshot(
        &pool,
        &serial,
        &device_slot,
        Utc::now() - chrono::Duration::seconds(30),
        "critical",
        json!([{ "severity": "critical", "code": "disk", "message": "磁盘使用率过高" }]),
    )
    .await;

    // 重新启用后手动执行一次（global scope = 全部 enrollment）。
    let (status, _) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/inspection-plans/{plan_id}"),
        Some(&harness.admin_token),
        Some(json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, 200);

    let (status, run) = send_json(
        &harness.app,
        "POST",
        &format!("/api/v1/admin/inspection-plans/{plan_id}/run"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{run}");
    assert_eq!(run["status"], "issues", "{run}");
    assert_eq!(run["plan_id"], plan_id.as_str());
    let checked = run["checked_machines"].as_i64().unwrap_or(0);
    let issues = run["issue_machines"].as_i64().unwrap_or(0);
    assert!(checked >= 1, "{run}");
    assert!(issues >= 1, "{run}");
    assert!(run["finished_at"].as_str().is_some());
    assert!(
        !run["summary"]["issues"]
            .as_array()
            .expect("issues")
            .is_empty()
    );
    let run_id = run["id"].as_str().expect("run id").to_owned();
    let ticket_id = run["ticket_id"]
        .as_str()
        .expect("有异常时应关联工单")
        .to_owned();

    // 自动工单内容。
    let (status, detail) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/tickets/{ticket_id}"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    let ticket = &detail["ticket"];
    assert_eq!(ticket["category"], "machine_health");
    assert_eq!(ticket["priority"], "high", "存在 critical 时应高优先级");
    assert_eq!(ticket["source"], "auto");
    assert_eq!(ticket["source_type"], "inspection");
    assert_eq!(ticket["source_id"], run_id.as_str());
    assert_eq!(
        ticket["dedupe_key"],
        format!("inspection:{plan_id}:{run_id}").as_str()
    );
    assert!(
        ticket["title"]
            .as_str()
            .unwrap_or_default()
            .contains("巡检报告")
    );

    // 执行历史。
    let (status, runs) = send_json(
        &harness.app,
        "GET",
        &format!("/api/v1/admin/inspection-runs?plan_id={plan_id}&limit=10"),
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{runs}");
    assert!(
        runs.as_array()
            .expect("runs")
            .iter()
            .any(|row| row["id"] == run_id.as_str() && row["status"] == "issues"),
        "{runs}"
    );
    // limit 校验。
    let (status, _) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/inspection-runs?limit=101",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 400);

    // ---- 禁用计划不被 claim；启用后并发 claim 只有一次命中 ----
    let plan_uuid = Uuid::parse_str(&plan_id).expect("uuid");
    let (status, _) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/inspection-plans/{plan_id}"),
        Some(&harness.admin_token),
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, 200);
    let disabled_claims = harness
        .state
        .ticket_inspections
        .claim_due_plans(20)
        .await
        .expect("claim disabled");
    assert!(
        !disabled_claims.iter().any(|plan| plan.id == plan_uuid),
        "禁用计划不应被 claim"
    );

    let (status, _) = send_json(
        &harness.app,
        "PATCH",
        &format!("/api/v1/admin/inspection-plans/{plan_id}"),
        Some(&harness.admin_token),
        Some(json!({ "enabled": true, "interval_hours": 24 })),
    )
    .await;
    assert_eq!(status, 200);
    // 手动执行会把 next_run_at 推到下一个周期；这里显式置为到期，验证并发 claim 只命中一次。
    sqlx::query("UPDATE inspection_plans SET next_run_at = now() WHERE id = $1")
        .bind(plan_uuid)
        .execute(harness.pool())
        .await
        .expect("force due");
    let (first, second) = tokio::join!(
        harness.state.ticket_inspections.claim_due_plans(20),
        harness.state.ticket_inspections.claim_due_plans(20)
    );
    let first = first.expect("claim a");
    let second = second.expect("claim b");
    let hits = first
        .iter()
        .chain(second.iter())
        .filter(|plan| plan.id == plan_uuid)
        .count();
    assert_eq!(hits, 1, "多副本并发 claim 同一计划只应命中一次");
    let third = harness
        .state
        .ticket_inspections
        .claim_due_plans(20)
        .await
        .expect("claim c");
    assert!(
        !third.iter().any(|plan| plan.id == plan_uuid),
        "claim 后 next_run_at 已前移，不应再次到期"
    );

    // ---- worker 每 tick 执行到期计划 ----
    let (status, worker_plan) = send_json(
        &harness.app,
        "POST",
        "/api/v1/admin/inspection-plans",
        Some(&harness.admin_token),
        Some(json!({
            "name": format!("worker 巡检 {suffix}"),
            "scope_type": "global",
            "interval_hours": 1,
            "enabled": true,
        })),
    )
    .await;
    assert_eq!(status, 201, "{worker_plan}");
    let worker_plan_id = worker_plan["id"].as_str().expect("id").to_owned();
    let config = qe_api::config::TicketWorkerConfig {
        auto_enabled: true,
        tick_secs: 60,
        device_offline_minutes: 10,
        health_consecutive: 2,
        lookback_hours: 24,
        email_enabled: false,
        overdue_remind_hours: 24,
    };
    let mut tracker = ticket_worker::TicketWorkerTracker::default();
    ticket_worker::run_tick(&harness.state, &config, &mut tracker).await;
    let worker_runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM inspection_runs WHERE plan_id = $1::uuid")
            .bind(&worker_plan_id)
            .fetch_one(&pool)
            .await
            .expect("count worker runs");
    assert!(worker_runs >= 1, "worker tick 应执行到期计划");

    // ---- 清理 ----
    sqlx::query("DELETE FROM inspection_plans WHERE name LIKE $1")
        .bind(format!("%{suffix}%"))
        .execute(&pool)
        .await
        .expect("cleanup plans");
    sqlx::query("DELETE FROM machine_snapshots WHERE serial = $1")
        .bind(&serial)
        .execute(&pool)
        .await
        .expect("cleanup snapshots");
    sqlx::query("DELETE FROM device_session WHERE serial = $1")
        .bind(&serial)
        .execute(&pool)
        .await
        .expect("cleanup session");
    sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
        .bind(&serial)
        .execute(&pool)
        .await
        .expect("cleanup enrollment");
    harness.cleanup().await;
}

/// Wave2 P3：CSV 导出最多 5000 行（超过按 5000 截断），且带 UTF-8 BOM。
#[tokio::test]
#[ignore = "需要真实 PostgreSQL/Redis"]
async fn csv_export_caps_at_5000_rows() {
    let harness = Harness::new("csvcap").await;
    let suffix = harness.suffix.clone();
    let prefix = format!("csvcap-{suffix}");
    let code_prefix = format!("TCSV{}", &suffix[..suffix.len().min(8)]);
    sqlx::query(
        "INSERT INTO tickets (code, title)
         SELECT $1 || g::text, $2 || '-' || g::text
           FROM generate_series(1, 5001) AS g",
    )
    .bind(&code_prefix)
    .bind(&prefix)
    .execute(harness.pool())
    .await
    .expect("bulk insert tickets");

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/admin/tickets/export?keyword={prefix}"))
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", harness.admin_token),
        )
        .body(Body::empty())
        .expect("request");
    let response = harness
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("export response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "CSV 应带 UTF-8 BOM");
    let text = String::from_utf8_lossy(&bytes[3..]);
    let lines = text.lines().filter(|line| !line.is_empty()).count();
    // 1 行表头 + 最多 5000 行数据。
    assert_eq!(lines, 5001, "CSV 行数应被截断为 5000 条数据");

    sqlx::query("DELETE FROM tickets WHERE title LIKE $1")
        .bind(format!("{prefix}%"))
        .execute(harness.pool())
        .await
        .expect("cleanup csv tickets");
}

/// Wave1 P3：超时提醒抢占是原子的，两个并发 claim 只有一次成功。
#[tokio::test]
#[ignore = "需要真实 PostgreSQL/Redis"]
async fn reminder_claim_is_atomic_across_replicas() {
    let harness = Harness::new("remindatomic").await;
    let ticket = harness.create_ticket("并发提醒-原子抢占", "normal").await;
    let ticket_id = ticket["id"].as_str().expect("id").to_owned();
    sqlx::query("UPDATE tickets SET due_at = now() - interval '1 hour' WHERE id = $1")
        .bind(Uuid::parse_str(&ticket_id).expect("uuid"))
        .execute(harness.pool())
        .await
        .expect("set due");
    let id = Uuid::parse_str(&ticket_id).expect("uuid");
    let (first, second) = tokio::join!(
        harness.state.tickets.claim_reminder(id, 24),
        harness.state.tickets.claim_reminder(id, 24)
    );
    let hits = usize::from(first.expect("claim a")) + usize::from(second.expect("claim b"));
    assert_eq!(hits, 1, "同一提醒窗口内只允许一个副本抢占成功");
    let third = harness
        .state
        .tickets
        .claim_reminder(id, 24)
        .await
        .expect("claim c");
    assert!(!third, "窗口内已提醒过，不应再次抢占");
}

/// Wave2 P3：范围配置区域选项不要求 area:read，但需要 scope 管理权限。
#[tokio::test]
#[ignore = "需要真实 PostgreSQL/Redis"]
async fn scope_area_options_require_scope_manage_not_area_read() {
    let harness = Harness::new("scopearea").await;
    let (status, body) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/ticket-scopes/areas",
        Some(&harness.admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.as_array().is_some(), "{body}");
    let (status, _) = send_json(
        &harness.app,
        "GET",
        "/api/v1/admin/ticket-scopes/areas",
        Some(&harness.viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 403);
}
