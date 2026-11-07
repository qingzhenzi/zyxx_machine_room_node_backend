//! `GET /agent/config` 端到端测试（真实 PostgreSQL）。
//!
//! 覆盖契约 §17.3 / §17.6：
//! - 无任何配置行 → 完整默认策略、revision 0；
//! - global 策略对未绑定房间的证书生效；
//! - room patch 覆盖 global（同时验证 area 层）；
//! - 证书没有存活注册记录 → 401。
//!
//! 连接 `DATABASE_URL`（默认本机 `qestudio` 库），迁移由 `sqlx::migrate!` 自动执行。
//! 测试内用 `DB_LOCK` 串行化：`agent_configs` 的 global 行是单例，并发跑会互相覆盖。

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
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
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// 未显式设置 `DATABASE_URL` 时使用的本地开发库。
const DEFAULT_DATABASE_URL: &str = "postgres://qestudio:qestudio@localhost:5432/qestudio";

/// `agent_configs` 只有一个 global 行，测试必须串行执行。
static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Fixture {
    db: PgPool,
    router: Router,
    area_id: Uuid,
    room_id: Uuid,
    bound_serial: String,
    bound_slot: String,
    unbound_serial: String,
    unbound_slot: String,
}

fn app_state(db: PgPool) -> AppState {
    let (sessions, _events) = SessionManager::new(Default::default());
    AppState {
        db,
        ca: Arc::new(Ca::generate_self_signed("Agent Config Test CA").expect("ca")),
        sessions: Arc::new(sessions),
        admin_token: Arc::from("agent-config-admin"),
        server_cert_fp_hex: Arc::from(""),
        cert_status: Arc::new(CertStatusCache::new()),
        admin_limiter: Arc::new(RateLimiter::new(0)),
        dispatcher: Arc::new(Dispatcher::new(std::time::Duration::from_secs(5))),
        mtls_endpoint: Arc::from("https://localhost:1"),
        enroll_endpoint: Arc::from("https://localhost:1"),
        storage: None,
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

fn new_serial() -> String {
    format!("{:032x}", Uuid::new_v4().as_u128())
}

async fn setup() -> Option<Fixture> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned());
    let db = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
    {
        Ok(db) => db,
        Err(error) => {
            eprintln!("skip: 连不上 PostgreSQL（{url}）：{error}");
            return None;
        }
    };
    sqlx::migrate!("../../migrations")
        .run(&db)
        .await
        .expect("migrations");

    // 每次测试从干净配置开始。scoped 行随房间/区域删除，global 行是单例。
    sqlx::query("DELETE FROM agent_configs")
        .execute(&db)
        .await
        .expect("clean agent_configs");

    let suffix = Uuid::new_v4().simple().to_string();
    let area_id: Uuid =
        sqlx::query_scalar("INSERT INTO areas (code, name) VALUES ($1, $2) RETURNING id")
            .bind(format!("agent-config-area-{suffix}"))
            .bind("agent config test area")
            .fetch_one(&db)
            .await
            .expect("insert area");

    let bound_slot = format!("agent-config-bound-{suffix}");
    let room_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rooms (area_id, code, name, device_slot)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(area_id)
    .bind(format!("agent-config-room-{suffix}"))
    .bind("agent config test room")
    .bind(&bound_slot)
    .fetch_one(&db)
    .await
    .expect("insert room");

    let bound_serial = new_serial();
    let unbound_serial = new_serial();
    let unbound_slot = format!("agent-config-unbound-{suffix}");
    for (serial, slot) in [
        (&bound_serial, &bound_slot),
        (&unbound_serial, &unbound_slot),
    ] {
        sqlx::query(
            "INSERT INTO device_enrollment
               (serial, device_slot, subject_cn, csr_sha256, cert_pem, status, issued_at, activated_at)
             VALUES ($1, $2, $2, $3, '', 'active', now(), now())",
        )
        .bind(serial)
        .bind(slot)
        .bind(Vec::<u8>::new())
        .execute(&db)
        .await
        .expect("insert enrollment");
    }

    Some(Fixture {
        router: routes::mtls_router(app_state(db.clone())),
        db,
        area_id,
        room_id,
        bound_serial,
        bound_slot,
        unbound_serial,
        unbound_slot,
    })
}

async fn cleanup(f: &Fixture) {
    let _ = sqlx::query("DELETE FROM agent_configs")
        .execute(&f.db)
        .await;
    for serial in [&f.bound_serial, &f.unbound_serial] {
        let _ = sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
            .bind(serial)
            .execute(&f.db)
            .await;
    }
    let _ = sqlx::query("DELETE FROM rooms WHERE id = $1")
        .bind(f.room_id)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM areas WHERE id = $1")
        .bind(f.area_id)
        .execute(&f.db)
        .await;
}

/// 直接落一行配置。`revision` 手动指定，模拟已存在的配置历史。
async fn put_global(db: &PgPool, policy: Value, revision: i64) {
    sqlx::query(
        "INSERT INTO agent_configs (scope_type, scope_id, policy, revision)
         VALUES ('global', NULL, $1, $2)
         ON CONFLICT (scope_type) WHERE scope_id IS NULL AND scope_key IS NULL DO UPDATE
         SET policy = EXCLUDED.policy,
             revision = EXCLUDED.revision,
             updated_at = now()",
    )
    .bind(policy)
    .bind(revision)
    .execute(db)
    .await
    .expect("put global config");
}

async fn put_scoped(db: &PgPool, scope_type: &str, scope_id: Uuid, policy: Value, revision: i64) {
    sqlx::query(
        "INSERT INTO agent_configs (scope_type, scope_id, policy, revision)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (scope_type, scope_id) WHERE scope_id IS NOT NULL DO UPDATE
         SET policy = EXCLUDED.policy,
             revision = EXCLUDED.revision,
             updated_at = now()",
    )
    .bind(scope_type)
    .bind(scope_id)
    .bind(policy)
    .bind(revision)
    .execute(db)
    .await
    .expect("put scoped config");
}

/// 直接落一行 device 作用域配置（0017 的 `agent_configs_device_uniq`）。
async fn put_device(db: &PgPool, slot: &str, policy: Value, revision: i64) {
    sqlx::query(
        "INSERT INTO agent_configs (scope_type, scope_id, scope_key, policy, revision)
         VALUES ('device', NULL, $1, $2, $3)
         ON CONFLICT (scope_key) WHERE scope_type = 'device' DO UPDATE
         SET policy = EXCLUDED.policy,
             revision = EXCLUDED.revision,
             updated_at = now()",
    )
    .bind(slot)
    .bind(policy)
    .bind(revision)
    .execute(db)
    .await
    .expect("put device config");
}

async fn get_config(router: &Router, serial: &str, slot: &str) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("GET")
        .uri("/agent/config")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(peer(serial, slot));

    let response = router.clone().oneshot(request).await.expect("route");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn no_config_returns_defaults() {
    let _guard = DB_LOCK.lock().await;
    let Some(f) = setup().await else { return };

    let (status, body) = get_config(&f.router, &f.bound_serial, &f.bound_slot).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], 0);
    assert_eq!(body["policy"]["lxc_auto_start"], false);
    assert_eq!(body["policy"]["poll_interval_secs"], 2);
    assert_eq!(body["policy"]["download_concurrency"], 4);
    assert_eq!(body["policy"]["cache_max_bytes"], 10_737_418_240u64);
    // v1.1：两档遥测间隔默认 fast 10s / full 300s
    assert_eq!(body["policy"]["telemetry_fast_interval_secs"], 10);
    assert_eq!(body["policy"]["telemetry_full_interval_secs"], 300);

    cleanup(&f).await;
}

#[tokio::test]
async fn global_policy_applies_to_unbound_device() {
    let _guard = DB_LOCK.lock().await;
    let Some(f) = setup().await else { return };

    put_global(
        &f.db,
        json!({ "lxc_auto_start": true, "poll_interval_secs": 5 }),
        3,
    )
    .await;

    // 证书绑定的槽位没有对应房间 → 只应用 global。
    let (status, body) = get_config(&f.router, &f.unbound_serial, &f.unbound_slot).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], 3);
    assert_eq!(body["policy"]["lxc_auto_start"], true);
    assert_eq!(body["policy"]["poll_interval_secs"], 5);
    // 未覆盖的字段保持默认值
    assert_eq!(body["policy"]["download_concurrency"], 4);
    assert_eq!(body["policy"]["cache_max_bytes"], 10_737_418_240u64);

    cleanup(&f).await;
}

#[tokio::test]
async fn room_patch_overrides_global_and_area() {
    let _guard = DB_LOCK.lock().await;
    let Some(f) = setup().await else { return };

    put_global(
        &f.db,
        json!({
            "lxc_auto_start": true,
            "poll_interval_secs": 10,
            "download_concurrency": 8
        }),
        2,
    )
    .await;
    put_scoped(
        &f.db,
        "area",
        f.area_id,
        json!({ "download_concurrency": 16, "cache_max_bytes": 2_048 }),
        5,
    )
    .await;
    put_scoped(
        &f.db,
        "room",
        f.room_id,
        json!({ "lxc_auto_start": false, "poll_interval_secs": 3 }),
        7,
    )
    .await;

    let (status, body) = get_config(&f.router, &f.bound_serial, &f.bound_slot).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 取已应用行的最大 revision（room 的 7）
    assert_eq!(body["revision"], 7);
    // room 覆盖 global
    assert_eq!(body["policy"]["lxc_auto_start"], false);
    assert_eq!(body["policy"]["poll_interval_secs"], 3);
    // area 覆盖 global 中未被 room 覆盖的字段
    assert_eq!(body["policy"]["download_concurrency"], 16);
    assert_eq!(body["policy"]["cache_max_bytes"], 2_048);

    cleanup(&f).await;
}

#[tokio::test]
async fn missing_or_revoked_enrollment_is_unauthorized() {
    let _guard = DB_LOCK.lock().await;
    let Some(f) = setup().await else { return };

    // database 里没有这个 serial
    let (status, body) = get_config(&f.router, &new_serial(), "unknown-slot").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["code"], "unauthorized");

    // 已吊销：即使握手缓存未刷新，handler 也必须拒绝
    sqlx::query(
        "UPDATE device_enrollment SET status = 'revoked', revoked_at = now() WHERE serial = $1",
    )
    .bind(&f.bound_serial)
    .execute(&f.db)
    .await
    .expect("revoke");
    let (status, body) = get_config(&f.router, &f.bound_serial, &f.bound_slot).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    cleanup(&f).await;
}

/// v1.1：device 作用域覆盖 room 层（证书 CN = device_slot）。
#[tokio::test]
async fn device_patch_overrides_room_layer() {
    let _guard = DB_LOCK.lock().await;
    let Some(f) = setup().await else { return };

    put_global(
        &f.db,
        json!({ "poll_interval_secs": 10, "cache_max_bytes": 4_096 }),
        2,
    )
    .await;
    put_scoped(
        &f.db,
        "room",
        f.room_id,
        json!({ "lxc_auto_start": false, "poll_interval_secs": 3 }),
        7,
    )
    .await;
    put_device(
        &f.db,
        &f.bound_slot,
        json!({
            "lxc_auto_start": true,
            "telemetry_fast_interval_secs": 20,
            "telemetry_full_interval_secs": 900
        }),
        9,
    )
    .await;

    let (status, body) = get_config(&f.router, &f.bound_serial, &f.bound_slot).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], 9, "{body}");
    // device 覆盖 room
    assert_eq!(body["policy"]["lxc_auto_start"], true, "{body}");
    assert_eq!(body["policy"]["telemetry_fast_interval_secs"], 20, "{body}");
    assert_eq!(
        body["policy"]["telemetry_full_interval_secs"], 900,
        "{body}"
    );
    // room 层仍生效，global 未被 room 覆盖的字段保留
    assert_eq!(body["policy"]["poll_interval_secs"], 3, "{body}");
    assert_eq!(body["policy"]["cache_max_bytes"], 4_096, "{body}");

    // 同房间外的槽位（无房间）只应用 global，不受该 device 行影响
    let (status, body) = get_config(&f.router, &f.unbound_serial, &f.unbound_slot).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], 2, "{body}");
    assert_eq!(body["policy"]["lxc_auto_start"], false, "{body}");
    assert_eq!(body["policy"]["poll_interval_secs"], 10, "{body}");

    cleanup(&f).await;
}

/// P1：子机（`room_devices`）也能解析到房间层 + device 层。
#[tokio::test]
async fn child_device_resolves_room_via_room_devices() {
    let _guard = DB_LOCK.lock().await;
    let Some(f) = setup().await else { return };

    let child_slot = format!(
        "agent-config-child-{}",
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let child_serial = new_serial();
    sqlx::query("INSERT INTO room_devices (device_slot, room_id, label) VALUES ($1, $2, $3)")
        .bind(&child_slot)
        .bind(f.room_id)
        .bind("child")
        .execute(&f.db)
        .await
        .expect("insert room_devices");
    sqlx::query(
        "INSERT INTO device_enrollment
           (serial, device_slot, subject_cn, csr_sha256, cert_pem, status, issued_at, activated_at)
         VALUES ($1, $2, $2, $3, '', 'active', now(), now())",
    )
    .bind(&child_serial)
    .bind(&child_slot)
    .bind(Vec::<u8>::new())
    .execute(&f.db)
    .await
    .expect("insert child enrollment");

    put_global(&f.db, json!({ "poll_interval_secs": 10 }), 1).await;
    put_scoped(
        &f.db,
        "area",
        f.area_id,
        json!({ "download_concurrency": 16 }),
        2,
    )
    .await;
    put_scoped(
        &f.db,
        "room",
        f.room_id,
        json!({ "cache_max_bytes": 2_048 }),
        3,
    )
    .await;
    put_device(
        &f.db,
        &child_slot,
        json!({ "telemetry_fast_interval_secs": 30 }),
        4,
    )
    .await;

    let (status, body) = get_config(&f.router, &child_serial, &child_slot).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], 4, "{body}");
    assert_eq!(body["policy"]["poll_interval_secs"], 10, "{body}");
    assert_eq!(body["policy"]["download_concurrency"], 16, "{body}");
    assert_eq!(body["policy"]["cache_max_bytes"], 2_048, "{body}");
    assert_eq!(body["policy"]["telemetry_fast_interval_secs"], 30, "{body}");

    cleanup(&f).await;
    let _ = sqlx::query("DELETE FROM room_devices WHERE device_slot = $1")
        .bind(&child_slot)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
        .bind(&child_serial)
        .execute(&f.db)
        .await;
}
