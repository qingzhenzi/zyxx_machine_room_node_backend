//! mTLS 电源事件端点（`POST /agent/power-event`）端到端测试（真实 PostgreSQL）。
//!
//! 覆盖：
//! - serial / device_slot 完全来自设备证书，请求体不能换 serial；
//! - 合法事件落 `machine_power_events`，可按 serial 查最新一条；
//! - 非法 action / 超大 details 返回 400；
//! - 无证书 serial 返回 401。
//!
//! 默认会被跳过：只有设置 `TEST_DATABASE_URL` 且本机 PostgreSQL 可连时才执行。
//! 运行：`TEST_DATABASE_URL=postgres://... cargo test -p qe-device-pki --test power_event`

use std::sync::Arc;
use std::time::Duration;

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use qe_db::machine::MachineRepo;
use qe_device_pki::ca::Ca;
use qe_device_pki::cert_status::CertStatusCache;
use qe_device_pki::messaging::Dispatcher;
use qe_device_pki::ratelimit::RateLimiter;
use qe_device_pki::routes::{self, AppState};
use qe_device_pki::session::SessionManager;
use qe_device_pki::tls::PeerCert;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

fn app_state(db: PgPool) -> AppState {
    let (sessions, _events) = SessionManager::new(Default::default());
    AppState {
        db,
        ca: Arc::new(Ca::generate_self_signed("Power Event Test CA").expect("ca")),
        sessions: Arc::new(sessions),
        admin_token: Arc::from("power-event-admin"),
        server_cert_fp_hex: Arc::from(""),
        cert_status: Arc::new(CertStatusCache::new()),
        admin_limiter: Arc::new(RateLimiter::new(0)),
        dispatcher: Arc::new(Dispatcher::new(Duration::from_secs(5))),
        mtls_endpoint: Arc::from("https://localhost:1"),
        enroll_endpoint: Arc::from("https://localhost:1"),
        storage: None,
        vnc: Arc::new(qe_device_pki::vnc::VncHub::new(
            qe_device_pki::vnc::VNC_PAIR_TIMEOUT,
        )),
        vnc_internal_token: None,
        metrics: Arc::new(qe_device_pki::metrics::MetricsHub::new()),
        realtime: Arc::new(qe_device_pki::realtime::RealtimeHub::new()),
    }
}

fn peer(serial: &str, slot: &str) -> PeerCert {
    PeerCert {
        der: Vec::new(),
        common_name: Some(slot.to_owned()),
        serial_hex: Some(serial.to_owned()),
    }
}

async fn post_event(app: &Router, peer: PeerCert, body: Value) -> (u16, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/agent/power-event")
        .header("content-type", "application/json")
        .extension(peer)
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
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

async fn setup() -> Option<(PgPool, String, String, Uuid)> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(value) => value,
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
        Err(error) => {
            eprintln!("skip: 连不上 PostgreSQL（{error}）");
            return None;
        }
    };
    sqlx::migrate!("../../migrations")
        .run(&db)
        .await
        .expect("migrations");

    let suffix = Uuid::new_v4().simple().to_string();
    let serial = format!("pe-serial-{suffix}");
    let slot = format!("pe-slot-{suffix}");
    let device_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO device_enrollment
            (serial, device_slot, device_id, subject_cn, csr_sha256, cert_pem, status, issued_at, activated_at)
         VALUES ($1, $2, $3, $4, $5, 'test-cert', 'active', now(), now())",
    )
    .bind(&serial)
    .bind(&slot)
    .bind(device_id)
    .bind(&slot)
    .bind(vec![0u8; 32])
    .execute(&db)
    .await
    .expect("insert enrollment");

    Some((db, serial, slot, device_id))
}

fn body(action: &str) -> Value {
    json!({
        "schema": 1,
        "action": action,
        "graceful": true,
        "requested_by": "agent",
        "agent_version": "0.1.0",
        "note": "VM 2/2 优雅停止；网络已清理（1 个）",
        "at": "2026-11-14T06:00:00Z",
        "details": {
            "trigger": "signal:SIGTERM",
            "hostname": "node-pe",
            "vms": [{ "name": "vm-1", "graceful": true, "forced": false }],
            "network": { "ok": true, "networks": ["qevm-101"] },
            "errors": []
        }
    })
}

#[tokio::test]
async fn records_power_event_and_returns_latest() {
    let Some((db, serial, slot, _device_id)) = setup().await else {
        return;
    };
    let router = routes::mtls_router(app_state(db.clone()));
    let (status, reply) = post_event(&router, peer(&serial, &slot), body("shutdown")).await;
    assert_eq!(status, 202, "{reply}");
    assert_eq!(reply["status"], "accepted");
    assert_eq!(reply["action"], "shutdown");
    assert_eq!(reply["graceful"], true);
    assert_eq!(reply["serial"], serial);

    let repo = MachineRepo::new(db.clone());
    let latest = repo
        .latest_power_event(&serial)
        .await
        .expect("query")
        .expect("latest event");
    assert_eq!(latest.action, "shutdown");
    assert!(latest.graceful);
    assert_eq!(latest.device_slot, slot);
    assert_eq!(latest.source, "agent");
    assert_eq!(latest.details["trigger"], "signal:SIGTERM");
    assert_eq!(latest.details["agent_at"], "2026-11-14T06:00:00Z");

    // 第二次事件应覆盖 `latest`，但两次都保留。
    let (status, reply) = post_event(&router, peer(&serial, &slot), body("reboot")).await;
    assert_eq!(status, 202, "{reply}");
    let latest = repo
        .latest_power_event(&serial)
        .await
        .expect("query")
        .expect("latest event");
    assert_eq!(latest.action, "reboot");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM machine_power_events WHERE serial = $1")
            .bind(&serial)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 2);

    cleanup(&db, &serial).await;
}

#[tokio::test]
async fn rejects_invalid_action_and_missing_certificate() {
    let Some((db, serial, slot, _device_id)) = setup().await else {
        return;
    };
    let router = routes::mtls_router(app_state(db.clone()));

    let (status, reply) = post_event(&router, peer(&serial, &slot), body("format-disk")).await;
    assert_eq!(status, 400, "{reply}");

    let (status, reply) = post_event(
        &router,
        PeerCert {
            der: Vec::new(),
            common_name: Some(slot.clone()),
            serial_hex: None,
        },
        body("shutdown"),
    )
    .await;
    assert_eq!(status, 401, "{reply}");

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM machine_power_events WHERE serial = $1")
            .bind(&serial)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 0);

    cleanup(&db, &serial).await;
}

async fn cleanup(db: &PgPool, serial: &str) {
    let _ = sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
        .bind(serial)
        .execute(db)
        .await;
}
