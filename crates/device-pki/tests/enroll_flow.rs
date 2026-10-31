//! 注册流程**端到端**集成测试（真实 PostgreSQL，HTTP 层）。
//!
//! 之前的测试全是纯函数级（PoP / CA / CRL / 会话防抖），状态机里的坑——
//! pending 单卡死 slot、token 被烧掉后无法恢复、审批后拿不到 mtls_endpoint、
//! 激活后证书仍留在库里——一个都覆盖不到。这里用 tower::ServiceExt::oneshot
//! 直接打路由，不启 TLS 监听（TLS 装配另有单测）。
//!
//! 默认 `#[ignore]`；用 `scripts/test-integration.sh` 启动 PostgreSQL 后运行。
//! 未设置 `TEST_DATABASE_URL` 或连不上库时自动跳过。

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
    pop,
    ratelimit::RateLimiter,
    routes::{self, AppState},
    session::SessionManager,
    tls,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "integration-admin-token";

/// 测试用状态 + 两个 listener 的路由（注册端 / mTLS 端）。
/// 默认不限速：限速行为由 `admin_rate_limit_kicks_in` 单独用低预算验证。
async fn setup() -> Option<(AppState, Router, Router)> {
    setup_with_limit(0).await
}

async fn setup_with_limit(admin_limit: u32) -> Option<(AppState, Router, Router)> {
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
        .expect("run migrations");

    let ca = Arc::new(Ca::generate_self_signed("Integration Device CA").expect("ca"));
    // 注册端点的"证书"在测试里就是 CA 自己，只为得到一个确定的指纹值
    let fp = tls::cert_fingerprint_hex(ca.cert_pem()).expect("fingerprint");
    let (sessions, _rx) = SessionManager::new(Default::default());

    let state = AppState {
        db,
        ca,
        sessions: Arc::new(sessions),
        admin_token: Arc::from(ADMIN_TOKEN),
        server_cert_fp_hex: Arc::from(fp.as_str()),
        cert_status: Arc::new(CertStatusCache::new()),
        admin_limiter: Arc::new(RateLimiter::new(admin_limit)),
        dispatcher: Arc::new(qe_device_pki::messaging::Dispatcher::new(
            std::time::Duration::from_secs(5),
        )),
        mtls_endpoint: Arc::from("https://mtls.integration.test:4441"),
        enroll_endpoint: Arc::from("https://enroll.integration.test:4440"),
        // 预签名端点未配置 S3：本测试不覆盖对象存储
        storage: None,
    };
    Some((
        state.clone(),
        routes::enroll_router(state.clone()),
        routes::mtls_router(state),
    ))
}

/// 每个用例用独立 slot，并在结束时清理，避免污染共享库。
async fn cleanup(db: &sqlx::PgPool, slot: &str) {
    for sql in [
        "DELETE FROM device_session WHERE device_slot = $1",
        "DELETE FROM device_enroll_challenge WHERE device_slot = $1",
        "DELETE FROM device_enrollment WHERE device_slot = $1",
        "DELETE FROM device_bootstrap_token WHERE device_slot = $1",
    ] {
        let _ = sqlx::query(sql).bind(slot).execute(db).await;
    }
}

fn unique_slot(tag: &str) -> String {
    format!("it-{tag}-{}", uuid::Uuid::new_v4().simple())
}

async fn call(router: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = router.clone().oneshot(req).await.expect("route");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn with_admin(req: Request<Body>) -> Request<Body> {
    let mut req = req;
    req.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&format!("Bearer {ADMIN_TOKEN}")).unwrap(),
    );
    req
}

/// 造一把 P-256 设备密钥 + CSR，并提供对任意 transcript 签名的能力。
struct Device {
    csr_der: Vec<u8>,
    key: ring::signature::EcdsaKeyPair,
}

fn make_device(slot: &str) -> Device {
    use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

    let rc_key = KeyPair::generate().expect("rcgen key");
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, slot);
    params.distinguished_name = dn;
    let csr = params.serialize_request(&rc_key).expect("csr");

    let pkcs8 = rc_key.serialize_der();
    let key = ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &pkcs8,
        &ring::rand::SystemRandom::new(),
    )
    .expect("ring key");
    Device {
        csr_der: csr.der().to_vec(),
        key,
    }
}

impl Device {
    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.key
            .sign(&ring::rand::SystemRandom::new(), msg)
            .expect("sign")
            .as_ref()
            .to_vec()
    }
}

/// 走完 challenge → enroll 的请求构造（auto 型）。
fn enroll_request(
    token: &str,
    slot: &str,
    challenge_id: &str,
    challenge_hex: &str,
    device: &Device,
    seen_fp: &str,
) -> Value {
    let b64 = base64::engine::general_purpose::STANDARD;
    let challenge = hex::decode(challenge_hex).expect("challenge hex");
    let csr_sha = pop::sha256(&device.csr_der);
    let fp_bytes = hex::decode(seen_fp).expect("fp hex");
    let msg = pop::canonical_transcript(
        &challenge,
        &pop::sha256(token.as_bytes()),
        slot,
        &csr_sha,
        &fp_bytes,
    );
    json!({
        "challenge_id": challenge_id,
        "slot": slot,
        "token": token,
        "csr_der": b64.encode(&device.csr_der),
        "pop_signature": b64.encode(device.sign(&msg)),
        "pop_algorithm": pop::POP_ALGORITHM,
        "tls_cert_seen_fp": seen_fp,
    })
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn auto_enrollment_end_to_end() {
    let Some((state, enroll_app, _mtls_app)) = setup().await else {
        return;
    };
    let db = state.db.clone();
    let slot = unique_slot("auto");
    let fp = state.server_cert_fp_hex.to_string();

    // 1. 管理端发注册链接
    let (status, body) = call(
        &enroll_app,
        with_admin(post_json(
            "/admin/tokens",
            json!({"kind": "auto", "slot": slot}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create token: {body}");
    let token = body["token"].as_str().expect("token").to_owned();
    assert!(body["link"].as_str().unwrap().contains(&token));

    // 2. 不带 Bearer 的管理请求必须被拒
    let (status, _) = call(
        &enroll_app,
        post_json("/admin/tokens", json!({"kind": "auto", "slot": slot})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 3. challenge
    let (status, body) = call(
        &enroll_app,
        post_json(
            "/enroll/challenge",
            json!({"token": token, "slot": slot, "client_info": {"hostname": "it", "machine_id": "m"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "challenge: {body}");
    let challenge_id = body["challenge_id"].as_str().unwrap().to_owned();
    let challenge = body["challenge"].as_str().unwrap().to_owned();
    assert_eq!(body["kind"], "auto");

    // 4. enroll（PoP + TOFU 指纹）
    let device = make_device(&slot);
    let (status, body) = call(
        &enroll_app,
        post_json(
            "/enroll",
            enroll_request(&token, &slot, &challenge_id, &challenge, &device, &fp),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "enroll: {body}");
    assert_eq!(body["status"], "issued");
    assert!(
        body["cert_pem"]
            .as_str()
            .unwrap()
            .contains("BEGIN CERTIFICATE")
    );
    // approval 之外的路径也必须给出 mTLS 端点，否则客户端 confirm 无从下手
    assert_eq!(body["mtls_endpoint"], "https://mtls.integration.test:4441");
    let serial = body["serial"].as_str().unwrap().to_owned();

    // 5. token 一次性：再来一次必须失败
    let (status, _) = call(
        &enroll_app,
        post_json("/enroll/challenge", json!({"token": token, "slot": slot})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "token 应已被消费");

    // 6. 同 slot 已有存活证书 → 后到者 409
    let (status, body) = call(
        &enroll_app,
        with_admin(post_json(
            "/admin/tokens",
            json!({"kind": "auto", "slot": slot}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token2 = body["token"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &enroll_app,
        post_json("/enroll/challenge", json!({"token": token2, "slot": slot})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "slot 已有 issued 证书");

    // 7. 证书确实落库、serial 一致
    let (status_in_db,): (String,) =
        sqlx::query_as("SELECT status FROM device_enrollment WHERE serial = $1")
            .bind(&serial)
            .fetch_one(&db)
            .await
            .expect("row");
    assert_eq!(status_in_db, "issued");

    cleanup(&db, &slot).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn approval_flow_survives_retries_and_delivers_mtls_endpoint() {
    let Some((state, enroll_app, mtls_app)) = setup().await else {
        return;
    };
    let db = state.db.clone();
    let slot = unique_slot("approval");
    let fp = state.server_cert_fp_hex.to_string();

    let (status, body) = call(
        &enroll_app,
        with_admin(post_json(
            "/admin/tokens",
            json!({"kind": "approval", "slot": slot}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create approval token: {body}");
    let token = body["token"].as_str().unwrap().to_owned();

    // challenge：服务端此时就应给出 enroll_id（客户端要立刻落盘）
    let (status, body) = call(
        &enroll_app,
        post_json(
            "/enroll/challenge",
            json!({"token": token, "slot": slot, "client_info": {"hostname": "it", "machine_id": "m"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "challenge: {body}");
    let challenge_id = body["challenge_id"].as_str().unwrap().to_owned();
    let challenge = body["challenge"].as_str().unwrap().to_owned();
    let enroll_id = body["enroll_id"]
        .as_str()
        .expect("challenge 应返回 enroll_id")
        .to_owned();

    // 关键：客户端在 challenge 之后、enroll 之前掉线并重试 challenge，
    // 不能被自己刚创建的 pending 单挡成 409（否则这台设备永久注册不上）
    let (status, body) = call(
        &enroll_app,
        post_json(
            "/enroll/challenge",
            json!({"token": token, "slot": slot, "client_info": {"hostname": "it", "machine_id": "m"}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "pending 单应可复用: {body}");
    assert_eq!(
        body["enroll_id"].as_str().unwrap(),
        enroll_id,
        "复用时 enroll_id 不应改变"
    );

    let device = make_device(&slot);
    let (status, body) = call(
        &enroll_app,
        post_json(
            "/enroll",
            enroll_request(&token, &slot, &challenge_id, &challenge, &device, &fp),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "enroll: {body}");
    assert_eq!(body["status"], "pending");
    assert_eq!(body["enroll_id"].as_str().unwrap(), enroll_id);

    // 未审批：轮询只能看到 pending
    let (status, body) = call(
        &enroll_app,
        Request::builder()
            .uri(format!("/enroll/{enroll_id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "pending");
    assert!(body["cert_pem"].is_null());

    // 管理员审批（需要 Bearer）
    let (status, _) = call(
        &enroll_app,
        Request::builder()
            .method("POST")
            .uri(format!("/admin/enroll/{enroll_id}/decide"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"approve": true}).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "无 Bearer 应被拒");

    let (status, body) = call(
        &enroll_app,
        with_admin(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/enroll/{enroll_id}/decide"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"approve": true, "note": "ok"}).to_string(),
                ))
                .unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "decide: {body}");
    assert_eq!(body["status"], "issued");

    // 批准后必须能拿到证书，**并且带上 mTLS 端点**（否则 confirm 必然 404）
    let (status, body) = call(
        &enroll_app,
        Request::builder()
            .uri(format!("/enroll/{enroll_id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "download: {body}");
    assert_eq!(body["status"], "approved");
    let cert_pem = body["cert_pem"].as_str().expect("cert").to_owned();
    assert_eq!(
        body["mtls_endpoint"].as_str().unwrap(),
        "https://mtls.integration.test:4441",
        "approval 路径也必须下发 mtls_endpoint"
    );

    // 激活前允许重复下载（响应在路上丢了还能再取）
    let (status, body) = call(
        &enroll_app,
        Request::builder()
            .uri(format!("/enroll/{enroll_id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "issued 期间应可重取: {body}");

    // confirm（mTLS 路由；对端证书由测试注入）
    let cert_der = {
        let mut rd = std::io::BufReader::new(cert_pem.as_bytes());
        rustls_pemfile::certs(&mut rd)
            .collect::<Result<Vec<_>, _>>()
            .expect("parse cert")[0]
            .to_vec()
    };
    let mut req = Request::builder()
        .method("POST")
        .uri("/enroll/confirm")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"slot": slot}).to_string()))
        .unwrap();
    req.extensions_mut()
        .insert(qe_device_pki::tls::PeerCert::from_der(cert_der));
    let (status, body) = call(&mtls_app, req).await;
    assert_eq!(status, StatusCode::OK, "confirm: {body}");
    assert_eq!(body["status"], "active");

    // 激活后：DB 里的证书被清空，再取 410
    let (pem_in_db,): (String,) =
        sqlx::query_as("SELECT cert_pem FROM device_enrollment WHERE device_slot = $1")
            .bind(&slot)
            .fetch_one(&db)
            .await
            .expect("row");
    assert!(pem_in_db.is_empty(), "激活后 DB 不应再持有证书 PEM");
    let (status, _) = call(
        &enroll_app,
        Request::builder()
            .uri(format!("/enroll/{enroll_id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::GONE, "激活后再取应为 410");

    cleanup(&db, &slot).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn rejecting_an_enrollment_revokes_the_signed_certificate() {
    let Some((state, enroll_app, _mtls_app)) = setup().await else {
        return;
    };
    let db = state.db.clone();
    let slot = unique_slot("reject");
    let fp = state.server_cert_fp_hex.to_string();

    let (_, body) = call(
        &enroll_app,
        with_admin(post_json(
            "/admin/tokens",
            json!({"kind": "approval", "slot": slot}),
        )),
    )
    .await;
    let token = body["token"].as_str().unwrap().to_owned();

    let (_, body) = call(
        &enroll_app,
        post_json(
            "/enroll/challenge",
            json!({"token": token, "slot": slot, "client_info": {"hostname": "it"}}),
        ),
    )
    .await;
    let challenge_id = body["challenge_id"].as_str().unwrap().to_owned();
    let challenge = body["challenge"].as_str().unwrap().to_owned();
    let enroll_id = body["enroll_id"].as_str().unwrap().to_owned();

    let device = make_device(&slot);
    let (status, _) = call(
        &enroll_app,
        post_json(
            "/enroll",
            enroll_request(&token, &slot, &challenge_id, &challenge, &device, &fp),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 管理员拒绝：证书在 enroll 阶段就签好了，必须**同时**吊销，
    // 否则一张 100 年有效的证书会以 rejected 身份留在库里
    let (status, body) = call(
        &enroll_app,
        with_admin(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/enroll/{enroll_id}/decide"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"approve": false, "note": "unknown device"}).to_string(),
                ))
                .unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "decide: {body}");

    let (revoked_at, reason): (Option<chrono::DateTime<chrono::Utc>>, Option<String>) =
        sqlx::query_as(
            "SELECT revoked_at, revoke_reason FROM device_enrollment WHERE device_slot = $1",
        )
        .bind(&slot)
        .fetch_one(&db)
        .await
        .expect("row");
    let revoked_at = revoked_at.expect("拒绝必须写 revoked_at");
    let age = (chrono::Utc::now() - revoked_at).num_seconds();
    assert!(
        age.abs() < 30,
        "revoked_at 应该是刚刚写入的，实际 {age}s 前"
    );
    assert_eq!(reason.as_deref(), Some("rejected"));

    // 轮询看到 rejected，且拿不到证书
    let (status, body) = call(
        &enroll_app,
        Request::builder()
            .uri(format!("/enroll/{enroll_id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rejected 轮询: {body}");
    assert_eq!(body["status"], "rejected");
    assert!(body["cert_pem"].is_null());

    cleanup(&db, &slot).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn slot_name_validation_and_cleanup_endpoint() {
    let Some((state, enroll_app, _)) = setup().await else {
        return;
    };
    let db = state.db.clone();
    let slot = unique_slot("cleanup");

    // slot 白名单：带路径分隔符的 slot 必须被拒（否则注册链接可被注入）
    let (status, body) = call(
        &enroll_app,
        with_admin(post_json(
            "/admin/tokens",
            json!({"kind": "auto", "slot": "../etc/passwd"}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "非法 slot: {body}");

    // 建一个 approval 注册单，然后用管理接口清理掉，slot 才能继续用
    let (_, body) = call(
        &enroll_app,
        with_admin(post_json(
            "/admin/tokens",
            json!({"kind": "approval", "slot": slot}),
        )),
    )
    .await;
    let token = body["token"].as_str().unwrap().to_owned();
    let (_, body) = call(
        &enroll_app,
        post_json(
            "/enroll/challenge",
            json!({"token": token, "slot": slot, "client_info": {"hostname": "it"}}),
        ),
    )
    .await;
    let enroll_id = body["enroll_id"].as_str().unwrap().to_owned();

    let (status, _) = call(
        &enroll_app,
        Request::builder()
            .method("DELETE")
            .uri(format!("/admin/enrollments/{enroll_id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "无 Bearer 应被拒");

    let (status, body) = call(
        &enroll_app,
        with_admin(
            Request::builder()
                .method("DELETE")
                .uri(format!("/admin/enrollments/{enroll_id}"))
                .body(Body::empty())
                .unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清理: {body}");

    let (left,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM device_enrollment WHERE enroll_id = $1::uuid")
            .bind(&enroll_id)
            .fetch_one(&db)
            .await
            .expect("count");
    assert_eq!(left, 0, "清理后注册单应已删除");

    cleanup(&db, &slot).await;
}

// ---------------- 限速与上行消息 ----------------

fn with_ip(addr: std::net::SocketAddr, mut req: Request<Body>) -> Request<Body> {
    req.extensions_mut().insert(addr);
    req
}

fn pem_first_der(pem: &str) -> Vec<u8> {
    let mut rd = std::io::BufReader::new(pem.as_bytes());
    rustls_pemfile::certs(&mut rd)
        .collect::<Result<Vec<_>, _>>()
        .expect("parse cert")[0]
        .to_vec()
}

/// 走完 auto 注册，返回 `(cert_pem, serial)`。
async fn auto_enroll(state: &AppState, app: &Router, slot: &str) -> (String, String) {
    let fp = state.server_cert_fp_hex.to_string();
    let (_, body) = call(
        app,
        with_admin(post_json(
            "/admin/tokens",
            json!({"kind": "auto", "slot": slot}),
        )),
    )
    .await;
    let token = body["token"].as_str().unwrap().to_owned();
    let (_, body) = call(
        app,
        post_json("/enroll/challenge", json!({"token": token, "slot": slot})),
    )
    .await;
    let challenge_id = body["challenge_id"].as_str().unwrap().to_owned();
    let challenge = body["challenge"].as_str().unwrap().to_owned();
    let device = make_device(slot);
    let (_, body) = call(
        app,
        post_json(
            "/enroll",
            enroll_request(&token, slot, &challenge_id, &challenge, &device, &fp),
        ),
    )
    .await;
    assert_eq!(body["status"], "issued", "auto 注册应直接签发: {body}");
    (
        body["cert_pem"].as_str().unwrap().to_owned(),
        body["serial"].as_str().unwrap().to_owned(),
    )
}

/// ADMIN_TOKEN 是静态 bearer，没有账号锁定可言 —— 限速是唯一防线。
#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn admin_endpoints_are_rate_limited_per_ip() {
    let Some((_state, app, _)) = setup_with_limit(2).await else {
        return;
    };
    let ip_a: std::net::SocketAddr = "203.0.113.7:40000".parse().unwrap();
    let ip_b: std::net::SocketAddr = "198.51.100.7:40001".parse().unwrap();

    let list_req = || {
        with_admin(
            Request::builder()
                .uri("/admin/enrollments")
                .body(Body::empty())
                .unwrap(),
        )
    };

    for i in 0..2 {
        let (status, body) = call(&app, with_ip(ip_a, list_req())).await;
        assert_eq!(status, StatusCode::OK, "第 {} 次应放行: {body}", i + 1);
    }

    let resp = app
        .clone()
        .oneshot(with_ip(ip_a, list_req()))
        .await
        .expect("route");
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "第 3 次应被限"
    );
    let retry_after = resp
        .headers()
        .get(axum::http::header::RETRY_AFTER)
        .is_some();
    assert!(retry_after, "429 应带 Retry-After");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], "rate_limited", "{body}");

    // 换个 IP 不受影响：预算是按 IP 记的
    let (status, _) = call(&app, with_ip(ip_b, list_req())).await;
    assert_eq!(status, StatusCode::OK, "另一个 IP 应仍有完整预算");
}

/// 上行消息按 `(serial, seq)` 去重，重放必须幂等，水位要落库。
#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn messages_are_deduped_by_serial_and_seq() {
    let Some((state, enroll_app, mtls_app)) = setup().await else {
        return;
    };
    let db = state.db.clone();
    let slot = unique_slot("msg");
    let (cert_pem, serial) = auto_enroll(&state, &enroll_app, &slot).await;
    let cert_der = pem_first_der(&cert_pem);

    let build = |seq: i64, payload: String| {
        let mut req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"seq": seq, "kind": "heartbeat", "payload": payload}).to_string(),
            ))
            .unwrap();
        req.extensions_mut()
            .insert(qe_device_pki::tls::PeerCert::from_der(cert_der.clone()));
        req
    };

    let (status, body) = call(&mtls_app, build(1, "{}".into())).await;
    assert_eq!(status, StatusCode::OK, "首条消息: {body}");
    assert_eq!(body["duplicate"], false);

    let (status, body) = call(&mtls_app, build(1, "{}".into())).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "重放仍应是 2xx，客户端才敢重试: {body}"
    );
    assert_eq!(body["duplicate"], true, "同一 seq 必须被判为重复");

    let (status, body) = call(&mtls_app, build(2, "{}".into())).await;
    assert_eq!(status, StatusCode::OK, "新 seq: {body}");
    assert_eq!(body["duplicate"], false);

    let (status, _) = call(&mtls_app, build(0, "{}".into())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "seq 必须为正");

    let (status, _) = call(
        &mtls_app,
        build(3, "x".repeat(routes::MAX_MESSAGE_BYTES + 1)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "超大载荷应被拒");

    // 水位落库：重启后仍能去重（契约 §6 的"重启可恢复"）
    let (watermark,): (i64,) =
        sqlx::query_as("SELECT last_seen_seq FROM device_session WHERE serial = $1")
            .bind(&serial)
            .fetch_one(&db)
            .await
            .expect("session row");
    assert_eq!(watermark, 2, "水位应停在最新 seq");

    // 没有 mTLS 证书（没插 PeerCert 扩展）直接调用必须 401
    let (status, _) = call(
        &mtls_app,
        Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"seq": 9, "kind": "x", "payload": ""}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "身份必须来自 mTLS 证书");

    cleanup(&db, &slot).await;
}

/// 处理语义：**成功才推水位**，失败能被重发自愈，没有订阅方要明确记档。
#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn message_processing_is_at_least_once_with_self_healing() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let Some((state, enroll_app, mtls_app)) = setup().await else {
        return;
    };
    let db = state.db.clone();
    let slot = unique_slot("proc");
    let (cert_pem, serial) = auto_enroll(&state, &enroll_app, &slot).await;
    let cert_der = pem_first_der(&cert_pem);

    // 第一次故意失败，之后成功 —— 验证重发能自愈
    let attempts = Arc::new(AtomicUsize::new(0));
    let a = Arc::clone(&attempts);
    state.dispatcher.subscribe("telemetry", move |_m| {
        let a = Arc::clone(&a);
        async move {
            if a.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(qe_device_pki::Error::Internal(anyhow::anyhow!(
                    "第一次故意失败"
                )));
            }
            Ok(())
        }
    });

    let build = {
        let cert_der = cert_der.clone();
        move |seq: i64, kind: &str| {
            let mut req = Request::builder()
                .method("POST")
                .uri("/messages")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"seq": seq, "kind": kind, "payload": "{}"}).to_string(),
                ))
                .unwrap();
            req.extensions_mut()
                .insert(qe_device_pki::tls::PeerCert::from_der(cert_der.clone()));
            req
        }
    };

    // 1) 第一次投递：handler 失败 → 5xx，且不推水位
    let (status, body) = call(&mtls_app, build(1, "telemetry")).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "失败应是 5xx: {body}"
    );
    let (attempts_db, processed_at, last_error): (i32, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT attempts, processed_at::text, last_error
             FROM device_message_inbox WHERE serial = $1 AND seq = 1",
        )
        .bind(&serial)
        .fetch_one(&db)
        .await
        .expect("inbox row");
    assert_eq!(attempts_db, 1);
    assert!(processed_at.is_none(), "失败不能算处理成功");
    assert!(last_error.unwrap().contains("故意失败"), "要留下失败原因");
    let watermark: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(last_seen_seq), 0) FROM device_session WHERE serial = $1",
    )
    .bind(&serial)
    .fetch_one(&db)
    .await
    .expect("watermark");
    assert!(watermark < 1, "失败时不能推水位，实际 {watermark}");

    // 2) 客户端重发同一条 → 自愈成功
    let (status, body) = call(&mtls_app, build(1, "telemetry")).await;
    assert_eq!(status, StatusCode::OK, "重发应成功: {body}");
    assert_eq!(body["duplicate"], false);
    let (attempts_db, processed_at, _): (i32, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT attempts, processed_at::text, last_error
         FROM device_message_inbox WHERE serial = $1 AND seq = 1",
    )
    .bind(&serial)
    .fetch_one(&db)
    .await
    .expect("inbox row");
    assert_eq!(attempts_db, 2);
    assert!(processed_at.is_some(), "成功后必须落 processed_at");
    let (watermark,): (i64,) =
        sqlx::query_as("SELECT last_seen_seq FROM device_session WHERE serial = $1")
            .bind(&serial)
            .fetch_one(&db)
            .await
            .expect("session row");
    assert_eq!(watermark, 1, "成功后才推水位");

    // 3) 再次重发 → 幂等 ACK，handler 不再跑
    let hits_before = attempts.load(Ordering::SeqCst);
    let (status, body) = call(&mtls_app, build(1, "telemetry")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["duplicate"], true);
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        hits_before,
        "已处理过的消息不该再触发 handler"
    );

    // 4) 没有订阅方的 kind：明确记档，不假装成功也不无限重试
    let (status, body) = call(&mtls_app, build(2, "no-such-kind")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "no_handler");
    let (processed_at, last_error): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT processed_at::text, last_error
         FROM device_message_inbox WHERE serial = $1 AND seq = 2",
    )
    .bind(&serial)
    .fetch_one(&db)
    .await
    .expect("inbox row");
    assert!(processed_at.is_some(), "no_handler 也要记档为终态");
    assert!(last_error.unwrap().contains("handler"), "要写明原因");

    cleanup(&db, &slot).await;
}

/// 下行通道：业务方投递 → 设备拉取 → 处理成功才 ACK（拉取不等于投递）。
#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn downlink_queue_is_pull_then_ack() {
    use qe_device_pki::messaging;

    let Some((state, enroll_app, mtls_app)) = setup().await else {
        return;
    };
    let db = state.db.clone();
    let slot = unique_slot("dl");
    let (cert_pem, serial) = auto_enroll(&state, &enroll_app, &slot).await;
    let cert_der = pem_first_der(&cert_pem);

    // 业务方按 slot 投递（自动解析当前 active 的 serial）
    let id = messaging::downlink_publish_to_slot(&db, &slot, "config", r#"{"mode":"a"}"#)
        .await
        .expect("publish")
        .expect("active 设备应能收到");
    let id2 = messaging::downlink_publish(&db, &serial, &slot, "config", r#"{"mode":"b"}"#)
        .await
        .expect("publish");

    let pull = |after: i64| {
        let mut req = Request::builder()
            .uri(format!("/messages/downlink?after={after}"))
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(qe_device_pki::tls::PeerCert::from_der(cert_der.clone()));
        req
    };
    let ack = |up_to: i64| {
        let mut req = Request::builder()
            .method("POST")
            .uri("/messages/downlink/ack")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"up_to": up_to}).to_string()))
            .unwrap();
        req.extensions_mut()
            .insert(qe_device_pki::tls::PeerCert::from_der(cert_der.clone()));
        req
    };

    // 拉取：按 id 升序，且"拉取不等于投递"
    let (status, body) = call(&mtls_app, pull(0)).await;
    assert_eq!(status, StatusCode::OK, "pull: {body}");
    let msgs = body["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["id"], id);
    assert_eq!(msgs[0]["kind"], "config");
    assert_eq!(msgs[0]["payload"], r#"{"mode":"a"}"#);
    assert_eq!(body["next_after"], id2);

    // 没 ACK 之前，再拉一次还会拿到同一批（至少一次）
    let (status, body) = call(&mtls_app, pull(0)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["messages"].as_array().unwrap().len(),
        2,
        "未 ACK 时重复拉取必须仍然拿到"
    );

    // 处理成功后 ACK
    let (status, body) = call(&mtls_app, ack(id2)).await;
    assert_eq!(status, StatusCode::OK, "ack: {body}");
    assert_eq!(body["acked"], 2);

    // ACK 之后拉不到旧消息；从旧游标再拉也是空（幂等）
    let (status, body) = call(&mtls_app, pull(id2)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["messages"].as_array().unwrap().is_empty(), "{body}");
    let (status, body) = call(&mtls_app, ack(id2)).await;
    assert_eq!(status, StatusCode::OK, "重复 ACK 幂等: {body}");
    assert_eq!(body["acked"], 0);

    // 向不存在的 slot 投递 → None（而不是报错）
    let nobody = unique_slot("nobody");
    assert!(
        messaging::downlink_publish_to_slot(&db, &nobody, "x", "{}")
            .await
            .unwrap()
            .is_none(),
        "没有 active 设备时应返回 None"
    );

    // 无 mTLS 证书拉取 → 401
    let (status, _) = call(
        &mtls_app,
        Request::builder()
            .uri("/messages/downlink?after=0")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    cleanup(&db, &slot).await;
}
