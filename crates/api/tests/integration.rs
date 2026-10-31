//! 真实 PostgreSQL + Redis 集成测试。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 启动依赖并显式运行。
//! 测试使用独立的 `qestudio_test` 数据库，不碰开发数据。

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use qe_api::{
    api,
    cache::Cache,
    config::{Config, Environment},
    db::{Db, audit::AuditLogRepo, security::SecurityRepo, users::UserRepo},
    domain::validation,
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

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn audit_log_and_login_lock_work_against_real_services() {
    let db = Db::connect(&database_url(), 2)
        .await
        .expect("connect postgres");
    db.migrate().await.expect("run migrations");
    let cache = Cache::connect(&redis_url()).await.expect("connect redis");

    let username = format!("it_{}", uuid::Uuid::new_v4().simple());
    let users = UserRepo::new(db.pool().clone());
    let hash = hash_password_async("Integration@12345".to_owned())
        .await
        .expect("hash password");
    let user = users
        .create_with_roles(
            &username,
            None,
            "Integration User",
            &hash,
            &["viewer".to_owned()],
        )
        .await
        .expect("create user");

    let audit = AuditLogRepo::new(db.pool().clone());
    let resource_id = user.id.to_string();
    audit
        .record(
            user.id,
            "integration.test",
            "user",
            Some(&resource_id),
            serde_json::json!({ "source": "integration" }),
            Some("127.0.0.1"),
        )
        .await
        .expect("write audit");

    let page = audit
        .list(Some("integration.test"), Some("user"), 1, 10)
        .await
        .expect("read audit");
    assert!(
        page.items.iter().any(
            |entry| entry.actor_id == Some(user.id) && entry.ip.as_deref() == Some("127.0.0.1")
        )
    );

    cache
        .clear_login_failures(&username)
        .await
        .expect("clear lock");
    for attempt in 0..3 {
        let count = cache
            .record_login_failure(&username, 60, 3, 30)
            .await
            .expect("record failure");
        assert_eq!(count, attempt + 1);
    }
    assert!(
        cache
            .login_retry_after(&username)
            .await
            .expect("ttl")
            .is_some()
    );
    cache
        .clear_login_failures(&username)
        .await
        .expect("clear lock");

    let rate_key = format!("it:rate:{}", uuid::Uuid::new_v4().simple());
    assert!(
        cache
            .check_rate_limit(&rate_key, 2, 60)
            .await
            .expect("rate 1")
            .is_none()
    );
    assert!(
        cache
            .check_rate_limit(&rate_key, 2, 60)
            .await
            .expect("rate 2")
            .is_none()
    );
    assert!(
        cache
            .check_rate_limit(&rate_key, 2, 60)
            .await
            .expect("rate 3")
            .is_some()
    );
    cache.del(&rate_key).await.expect("cleanup rate");

    let version = cache
        .bump_announcement_version()
        .await
        .expect("bump version");
    assert!(version >= 1);
    let cache_key = format!("announcements:test:v{version}");
    cache
        .set_json(&cache_key, &serde_json::json!({ "ok": true }), 30)
        .await
        .expect("set cache");
    let cached: Option<serde_json::Value> = cache.get_json(&cache_key).await.expect("get cache");
    assert_eq!(cached, Some(serde_json::json!({ "ok": true })));

    let sanitized =
        validation::normalize_announcement_content("<p>ok</p><script>alert(1)</script>")
            .expect("sanitize");
    assert!(!sanitized.contains("script"));

    let security = SecurityRepo::new(db.pool().clone());
    let session_id = uuid::Uuid::new_v4();
    let token = "integration-refresh-token";
    let token_hash = qe_auth::session::refresh_token_hash(token);
    security
        .create_session_with_id(
            session_id,
            user.id,
            &token_hash,
            "integration-agent",
            Some("127.0.0.1"),
        )
        .await
        .expect("create session");
    assert!(security.session_active(session_id).await.expect("active"));
    assert_eq!(
        security.list_sessions(user.id).await.expect("list").len(),
        1
    );
    assert!(
        security
            .rotate_session_token(session_id, "new-hash")
            .await
            .expect("rotate")
    );
    assert!(
        security
            .revoke_session(user.id, session_id)
            .await
            .expect("revoke")
    );
    assert!(!security.session_active(session_id).await.expect("inactive"));

    cache
        .mark_session_active(session_id, 60)
        .await
        .expect("mark session");
    assert_eq!(
        cache
            .cached_session_active(session_id)
            .await
            .expect("cached"),
        Some(true)
    );
    cache
        .revoke_session_cache(session_id)
        .await
        .expect("revoke cache");
    assert_eq!(
        cache
            .cached_session_active(session_id)
            .await
            .expect("cached"),
        None
    );

    let reset_hash = qe_auth::session::refresh_token_hash("reset-token");
    security
        .create_password_reset(
            user.id,
            &reset_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .expect("create reset");
    assert_eq!(
        security
            .consume_password_reset(&reset_hash)
            .await
            .expect("consume reset"),
        Some(user.id)
    );
    assert_eq!(
        security
            .consume_password_reset(&reset_hash)
            .await
            .expect("consume used reset"),
        None
    );

    sqlx::query("DELETE FROM audit_logs WHERE actor_id = $1")
        .bind(user.id)
        .execute(db.pool())
        .await
        .expect("cleanup audit");
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user.id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
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
        public_enroll_endpoint: "https://localhost:4440".to_owned(),
        storage: None,
    }
}

async fn send_json(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
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
    let status = response.status();
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

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn http_auth_flow_rbac_and_security_headers() {
    let db = Db::connect(&database_url(), 4)
        .await
        .expect("connect postgres");
    db.migrate().await.expect("run migrations");
    let cache = Cache::connect(&redis_url()).await.expect("connect redis");
    let app = api::router(AppState::new(db.clone(), cache.clone(), test_config()));

    let users = UserRepo::new(db.pool().clone());
    let password = "HttpIntegration@12345";
    let password_hash = hash_password_async(password.to_owned())
        .await
        .expect("hash password");
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let short = &suffix[..8];
    let viewer_name = format!("hv_{short}");
    let admin_name = format!("ha_{short}");
    let email = format!("viewer_{short}@example.com");
    let viewer = users
        .create_with_roles(
            &viewer_name,
            Some(&email),
            "HTTP Viewer",
            &password_hash,
            &["viewer".to_owned()],
        )
        .await
        .expect("create viewer");
    let admin = users
        .create_with_roles(
            &admin_name,
            None,
            "HTTP Admin",
            &password_hash,
            &["admin".to_owned()],
        )
        .await
        .expect("create admin");

    // 基础安全响应头与统一 404。
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("health request"),
        )
        .await
        .expect("health response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        response
            .headers()
            .get("referrer-policy")
            .and_then(|value| value.to_str().ok()),
        Some("no-referrer")
    );

    let (status, body) = send_json(&app, "GET", "/missing", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    let (status, body) = send_json(&app, "POST", "/healthz", None, None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["error"]["code"], "method_not_allowed");

    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "username": viewer_name, "password": password })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let access = body["access_token"].as_str().expect("access").to_owned();
    let refresh = body["refresh_token"].as_str().expect("refresh").to_owned();

    let (status, body) = send_json(&app, "GET", "/api/v1/auth/me", Some(&access), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["username"], viewer_name);

    let (status, body) = send_json(&app, "GET", "/api/v1/admin/users", Some(&access), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "forbidden");

    // refresh 一次性轮换：旧 token 第二次使用必须失败。
    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({ "refresh_token": refresh })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let refreshed_access = body["access_token"].as_str().expect("access").to_owned();
    let (status, _) = send_json(
        &app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({ "refresh_token": refresh.clone() })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!refreshed_access.is_empty());

    // 管理员可以访问管理接口，撤销全部会话后 access token 立即失效。
    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "username": admin_name, "password": password })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let admin_access = body["access_token"]
        .as_str()
        .expect("admin access")
        .to_owned();
    let (status, body) = send_json(
        &app,
        "GET",
        "/api/v1/admin/users",
        Some(&admin_access),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = send_json(
        &app,
        "DELETE",
        "/api/v1/auth/sessions",
        Some(&admin_access),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send_json(&app, "GET", "/api/v1/auth/me", Some(&admin_access), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 找回密码按邮箱限流；连续第 6 次应返回 429。
    for attempt in 1..=6 {
        let (status, body) = send_json(
            &app,
            "POST",
            "/api/v1/auth/password/forgot",
            None,
            Some(json!({ "email": email })),
        )
        .await;
        if attempt < 6 {
            assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        } else {
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        }
    }

    sqlx::query("DELETE FROM audit_logs WHERE actor_id = ANY($1)")
        .bind(vec![viewer.id, admin.id])
        .execute(db.pool())
        .await
        .expect("cleanup audit");
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![viewer.id, admin.id])
        .execute(db.pool())
        .await
        .expect("cleanup users");
}
