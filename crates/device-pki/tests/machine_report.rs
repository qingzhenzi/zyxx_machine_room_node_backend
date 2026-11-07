//! mTLS 机器遥测上报端点（`POST /machine/report`）端到端测试（真实 PostgreSQL）。
//!
//! 覆盖：
//! - 身份来自 `PeerCert`（serial + CN=slot），请求体可省略 `device_slot`；
//! - 快照按 `tier` 落库（fast / full）；
//! - 请求体 `device_slot` 与证书不一致时 400；
//! - 上报会刷新 mTLS 会话活动（不会因为只上报、不发心跳被判定离线）。
//!
//! 默认 `#[ignore]`；用 `scripts/test-integration.sh` 或
//! `TEST_DATABASE_URL=... cargo test -p qe-device-pki --test machine_report -- --ignored` 运行。

use std::sync::Arc;
use std::time::Duration;

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
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
        ca: Arc::new(Ca::generate_self_signed("Machine Report Test CA").expect("ca")),
        sessions: Arc::new(sessions),
        admin_token: Arc::from("machine-report-admin"),
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
    }
}

fn peer(serial: &str, slot: &str) -> PeerCert {
    PeerCert {
        der: Vec::new(),
        common_name: Some(slot.to_owned()),
        serial_hex: Some(serial.to_owned()),
    }
}

async fn post_report(app: &Router, peer: PeerCert, body: Value) -> (u16, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/machine/report")
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
    let serial = format!("mr-serial-{suffix}");
    let slot = format!("mr-slot-{suffix}");
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

fn report_body() -> Value {
    json!({
        "schema": 1,
        "agent_version": "0.1.0",
        "hostname": "node-mr",
        "health": { "status": "ok", "score": 98 },
        "metrics": { "cpu_usage_percent": 1.5, "memory_usage_percent": 2.5 },
        "sections": { "agent": { "builtin_collect_duration_ms": 3 } }
    })
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn mtls_report_uses_cert_identity_and_persists_tier() {
    let Some((db, serial, slot, device_id)) = setup().await else {
        return;
    };
    let state = app_state(db.clone());
    let sessions = Arc::clone(&state.sessions);
    let app = routes::mtls_router(state);

    // 1) 旧证书（CN=slot）：请求体不带 device_slot，身份取证书 + DB 里的 device_id。
    let (status, body) = post_report(&app, peer(&serial, &slot), report_body()).await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["serial"], serial);
    assert_eq!(body["device_slot"], slot);
    assert_eq!(body["device_id"], device_id.to_string());

    let (tier, payload_slot): (String, String) = sqlx::query_as(
        "SELECT tier, payload->>'device_slot' FROM machine_snapshots WHERE serial = $1",
    )
    .bind(&serial)
    .fetch_one(&db)
    .await
    .expect("snapshot row");
    assert_eq!(tier, "full");
    assert_eq!(payload_slot, slot);
    assert!(
        sessions.last_seen_seq(&serial).is_some(),
        "上报应刷新会话活动"
    );

    // 2) fast 档上报：只带指标。
    let mut fast = report_body();
    fast["tier"] = json!("fast");
    let (status, body) = post_report(&app, peer(&serial, &slot), fast).await;
    assert_eq!(status, 202, "{body}");
    let fast_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM machine_snapshots WHERE serial = $1 AND tier = 'fast'",
    )
    .bind(&serial)
    .fetch_one(&db)
    .await
    .expect("fast count");
    assert_eq!(fast_rows, 1);

    // 3) body 里的 device_slot 与证书不一致：400。
    let mut mismatched = report_body();
    mismatched["device_slot"] = json!("other-slot");
    let (status, body) = post_report(&app, peer(&serial, &slot), mismatched).await;
    assert_eq!(status, 400, "{body}");

    // 清理：快照 + 设备记录。
    sqlx::query("DELETE FROM machine_snapshots WHERE serial = $1")
        .bind(&serial)
        .execute(&db)
        .await
        .expect("cleanup snapshots");
    sqlx::query("DELETE FROM device_session WHERE serial = $1")
        .bind(&serial)
        .execute(&db)
        .await
        .expect("cleanup session");
    sqlx::query("DELETE FROM device_enrollment WHERE serial = $1")
        .bind(&serial)
        .execute(&db)
        .await
        .expect("cleanup enrollment");
}

/// 新证书：CN 就是 device_id（UUID），slot 需要从 DB 解析。
#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn mtls_report_accepts_device_id_cn() {
    let Some((db, serial, slot, device_id)) = setup().await else {
        return;
    };
    let state = app_state(db.clone());
    let app = routes::mtls_router(state);

    // CN 用 device_id，而不是 slot。
    let (status, body) =
        post_report(&app, peer(&serial, &device_id.to_string()), report_body()).await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["serial"], serial);
    assert_eq!(body["device_id"], device_id.to_string());
    assert_eq!(body["device_slot"], slot, "slot 应从 DB 解析出来");

    // 请求体若带正确的 device_slot，也能通过；带错误值仍然 400。
    let mut ok_body = report_body();
    ok_body["device_slot"] = json!(slot);
    let (status, _) = post_report(&app, peer(&serial, &device_id.to_string()), ok_body).await;
    assert_eq!(status, 202);

    let mut bad = report_body();
    bad["device_slot"] = json!("other-slot");
    let (status, body) = post_report(&app, peer(&serial, &device_id.to_string()), bad).await;
    assert_eq!(status, 400, "{body}");

    for sql in [
        "DELETE FROM machine_snapshots WHERE serial = $1",
        "DELETE FROM device_session WHERE serial = $1",
        "DELETE FROM device_enrollment WHERE serial = $1",
    ] {
        sqlx::query(sql)
            .bind(&serial)
            .execute(&db)
            .await
            .expect("cleanup");
    }
}
