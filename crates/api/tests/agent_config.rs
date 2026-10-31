//! 节点配置管理（P2）集成测试：真实 PostgreSQL。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 启动依赖并显式运行。
//! 覆盖：area/room 配置写入、global 列表、global→area→room 合并、权限 403、
//! 目标不存在 404、二次下发写入 `device_downlink.kind = 'config.update'`。

use axum::Router;
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
use uuid::Uuid;

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

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn agent_config_scope_merge_and_dispatch() {
    let state = test_state().await;
    let app: Router = api::router(state.clone());
    let users = UserRepo::new(state.db.pool().clone());
    let password = "AgentConfigIntegration@12345";
    let password_hash = hash_password_async(password.to_owned())
        .await
        .expect("hash password");
    let suffix = &Uuid::new_v4().simple().to_string()[..8];
    let admin_name = format!("ac_{suffix}");
    let viewer_name = format!("acv_{suffix}");
    let admin = users
        .create_with_roles(
            &admin_name,
            None,
            "Agent Config Admin",
            &password_hash,
            &["admin".to_owned()],
        )
        .await
        .expect("create admin");
    let viewer = users
        .create_with_roles(
            &viewer_name,
            None,
            "Agent Config Viewer",
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
        Some(json!({ "code": format!("acarea_{suffix}"), "name": "节点配置测试区域" })),
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
            "code": format!("acroom_{suffix}"),
            "name": "节点配置测试房间"
        })),
    )
    .await;
    assert_eq!(status, 201, "{room}");
    let room_id = room["id"].as_str().expect("room id").to_owned();

    let mut created_config_ids: Vec<Uuid> = Vec::new();

    // 1. area 作用域写入（房间尚未绑定设备槽位，不下发）
    let (status, area_config) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "area",
            "scope_id": area_id,
            "policy": { "poll_interval_secs": 5 }
        })),
    )
    .await;
    assert_eq!(status, 200, "{area_config}");
    assert_eq!(area_config["config"]["scope_type"], "area", "{area_config}");
    assert_eq!(area_config["config"]["scope_id"], area_id, "{area_config}");
    assert_eq!(area_config["pushed_rooms"], 0, "{area_config}");
    created_config_ids.push(
        area_config["config"]["id"]
            .as_str()
            .expect("area config id")
            .parse()
            .expect("area config uuid"),
    );

    // 2. room 作用域写入
    let (status, room_config) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "room",
            "scope_id": room_id,
            "policy": { "lxc_auto_start": true, "download_concurrency": 8 }
        })),
    )
    .await;
    assert_eq!(status, 200, "{room_config}");
    assert_eq!(
        room_config["config"]["policy"]["lxc_auto_start"], true,
        "{room_config}"
    );
    assert_eq!(room_config["pushed_rooms"], 0, "{room_config}");
    created_config_ids.push(
        room_config["config"]["id"]
            .as_str()
            .expect("room config id")
            .parse()
            .expect("room config uuid"),
    );

    // 3. global 写入 → GET 列表
    let (status, global_config) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "global",
            "policy": { "cache_max_bytes": 123456789 }
        })),
    )
    .await;
    assert_eq!(status, 200, "{global_config}");
    assert_eq!(
        global_config["config"]["scope_type"], "global",
        "{global_config}"
    );
    assert_eq!(
        global_config["config"]["scope_id"],
        Value::Null,
        "{global_config}"
    );
    let global_config_id: Uuid = global_config["config"]["id"]
        .as_str()
        .expect("global config id")
        .parse()
        .expect("global config uuid");
    created_config_ids.push(global_config_id);

    let (status, list) = send_json(
        &app,
        "GET",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{list}");
    let total = list["total"].as_i64().expect("list total");
    let items = list["items"].as_array().expect("list items");
    assert!(total >= 3, "{list}");
    assert_eq!(total as usize, items.len(), "{list}");
    let global_item = items
        .iter()
        .find(|item| item["scope_type"] == "global")
        .unwrap_or_else(|| panic!("global config missing: {list}"));
    assert_eq!(
        global_item["policy"]["cache_max_bytes"], 123456789,
        "{list}"
    );

    // 4. effective：global → area → room 合并
    let (status, effective) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/agent-configs/effective?room_id={room_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{effective}");
    assert_eq!(effective["room_id"], room_id, "{effective}");
    let policy = &effective["policy"];
    assert_eq!(policy["lxc_auto_start"], true, "{effective}");
    assert_eq!(policy["poll_interval_secs"], 5, "{effective}");
    assert_eq!(policy["download_concurrency"], 8, "{effective}");
    assert_eq!(policy["cache_max_bytes"], 123456789, "{effective}");
    assert!(
        effective["revision"].as_i64().expect("revision") >= 1,
        "{effective}"
    );

    // 5. 权限：viewer 可读（device:read），不可写（device:manage）
    let (status, viewer_list) = send_json(
        &app,
        "GET",
        "/api/v1/admin/agent-configs",
        Some(&viewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{viewer_list}");
    let (status, denied) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&viewer_token),
        Some(json!({
            "scope_type": "global",
            "policy": { "poll_interval_secs": 9 }
        })),
    )
    .await;
    assert_eq!(status, 403, "{denied}");

    // 6. 作用域参数与目标存在性
    let (status, body) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "global",
            "scope_id": room_id,
            "policy": {}
        })),
    )
    .await;
    assert_eq!(status, 400, "global + scope_id = {status} {body}");

    let (status, body) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "room",
            "policy": { "poll_interval_secs": 3 }
        })),
    )
    .await;
    assert_eq!(status, 400, "room without scope_id = {status} {body}");

    let missing_room = Uuid::new_v4().to_string();
    let (status, body) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "room",
            "scope_id": missing_room,
            "policy": { "poll_interval_secs": 3 }
        })),
    )
    .await;
    assert_eq!(status, 404, "missing room = {status} {body}");

    let missing_area = Uuid::new_v4().to_string();
    let (status, body) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "area",
            "scope_id": missing_area,
            "policy": { "poll_interval_secs": 3 }
        })),
    )
    .await;
    assert_eq!(status, 404, "missing area = {status} {body}");

    let (status, body) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "room",
            "scope_id": room_id,
            "policy": { "poll_interval_secs": 0 }
        })),
    )
    .await;
    assert_eq!(status, 400, "out-of-range policy = {status} {body}");

    // 7. 绑定设备槽位 + 插入 active 注册 → 再写 room 配置应下发 config.update
    let device_slot = format!("ac-slot-{suffix}");
    let device_serial = format!("ac-serial-{suffix}");
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

    let (status, pushed) = send_json(
        &app,
        "PUT",
        "/api/v1/admin/agent-configs",
        Some(&admin_token),
        Some(json!({
            "scope_type": "room",
            "scope_id": room_id,
            "policy": {
                "lxc_auto_start": true,
                "download_concurrency": 8,
                "poll_interval_secs": 7
            }
        })),
    )
    .await;
    assert_eq!(status, 200, "{pushed}");
    assert_eq!(pushed["pushed_rooms"], 1, "{pushed}");
    let room_revision = pushed["config"]["revision"]
        .as_i64()
        .expect("room revision");
    assert!(room_revision >= 2, "{pushed}");

    let (downlink_kind, downlink_payload): (String, String) = sqlx::query_as(
        "SELECT kind, payload FROM device_downlink
         WHERE serial = $1
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(&device_serial)
    .fetch_one(state.db.pool())
    .await
    .expect("config.update downlink row");
    assert_eq!(downlink_kind, "config.update");
    let payload: Value = serde_json::from_str(&downlink_payload).expect("config.update payload");
    assert_eq!(payload["revision"], room_revision, "{payload}");
    assert_eq!(payload["policy"]["lxc_auto_start"], true, "{payload}");
    assert_eq!(payload["policy"]["poll_interval_secs"], 7, "{payload}");
    assert_eq!(payload["policy"]["download_concurrency"], 8, "{payload}");
    assert_eq!(payload["policy"]["cache_max_bytes"], 123456789, "{payload}");

    // 8. 下发后的 effective：room 覆盖立即反映
    let (status, effective) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/agent-configs/effective?room_id={room_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{effective}");
    assert_eq!(effective["policy"]["poll_interval_secs"], 7, "{effective}");
    assert_eq!(effective["revision"], room_revision, "{effective}");

    let (status, body) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/agent-configs/effective?room_id={missing_room}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, 404, "effective missing room = {status} {body}");

    // 9. 审计：至少 4 次成功写入
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE actor_id = $1 AND action = 'agent_config.update'",
    )
    .bind(admin.id)
    .fetch_one(state.db.pool())
    .await
    .expect("agent_config.update audit count");
    assert!(audit_count >= 4, "audit_count = {audit_count}");

    // 清理（先删配置与设备行，再删房间/区域/用户）
    for config_id in created_config_ids {
        sqlx::query("DELETE FROM agent_configs WHERE id = $1")
            .bind(config_id)
            .execute(state.db.pool())
            .await
            .expect("cleanup agent config");
    }
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
    let (status, _) = send_json(
        &app,
        "DELETE",
        &format!("/api/v1/admin/rooms/{room_id}"),
        Some(&admin_token),
        None,
    )
    .await;
    assert!(status == 200 || status == 204);
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
