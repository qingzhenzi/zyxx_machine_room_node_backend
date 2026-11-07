//! WebSocket 隧道（mTLS）端到端测试：证明「这条连接真的能建立」，
//! 并且**建立它本身就完成注册激活**（`issued → active`）。
//!
//! 这里起的是**真实 TCP + rustls** 的 mTLS listener（不是 tower::oneshot），
//! 因为 WebSocket 升级、客户端证书注入、ALPN 这些都只有在真实连接上才成立。
//!
//! 默认 `#[ignore]`；用 `scripts/test-integration.sh` 启动 PostgreSQL 后运行。

use futures_util::{SinkExt, StreamExt};
use qe_device_pki::{
    ca::Ca,
    cert_status::CertStatusCache,
    ratelimit::RateLimiter,
    routes::{self, AppState},
    session::SessionManager,
    tls,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;

const ADMIN_TOKEN: &str = "integration-admin-token";

/// 生成一套 CA + 服务端证书（带 SAN，客户端要做主机名校验）。
fn make_ca_and_server_cert() -> (rcgen::Certificate, rcgen::KeyPair, String, String) {
    use rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType,
    };
    use std::net::IpAddr;

    let ca_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "WS Test CA");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(365);
    let ca_cert = params.self_signed(&ca_key).unwrap();

    let server_key = KeyPair::generate().unwrap();
    let mut sp = CertificateParams::default();
    let mut sd = DistinguishedName::new();
    sd.push(DnType::CommonName, "localhost");
    sp.distinguished_name = sd;
    sp.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into().unwrap()),
        SanType::IpAddress(IpAddr::from([127, 0, 0, 1])),
    ];
    sp.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = sp.signed_by(&server_key, &ca_cert, &ca_key).unwrap();

    (
        ca_cert,
        ca_key,
        server_cert.pem(),
        server_key.serialize_pem(),
    )
}

/// 起一个真实的 mTLS listener（与 `main.rs` 的装配一致：注入 PeerCert + 对端地址）。
async fn spawn_mtls_listener(
    listener: tokio::net::TcpListener,
    cfg: Arc<rustls::ServerConfig>,
    app: axum::Router,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => return,
            };
            let acceptor = acceptor.clone();
            let app = app.clone();
            tokio::spawn(async move {
                let Ok(tls_stream) = acceptor.accept(stream).await else {
                    return;
                };
                let peer_cert = tls_stream
                    .get_ref()
                    .1
                    .peer_certificates()
                    .and_then(|c| c.first().map(|d| tls::PeerCert::from_der(d.to_vec())));
                let svc = app.clone();
                let io = hyper_util::rt::TokioIo::new(tls_stream);
                let svc = hyper_util::service::TowerToHyperService::new(svc.layer(
                    axum::middleware::from_fn(
                        move |mut req: axum::extract::Request, next: axum::middleware::Next| {
                            let pc = peer_cert.clone();
                            async move {
                                if let Some(pc) = pc {
                                    req.extensions_mut().insert(pc);
                                }
                                req.extensions_mut().insert(peer_addr);
                                next.run(req).await
                            }
                        },
                    ),
                ));
                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection_with_upgrades(io, svc)
                .await;
            });
        }
    })
}

/// 设备侧：TCP + mTLS（带设备证书）+ WebSocket 握手。
async fn dial(
    addr: SocketAddr,
    ca_pem: &str,
    device_cert_pem: &str,
    device_key_pem: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut roots = rustls::RootCertStore::empty();
    for der in qe_device_pki::ca::pem_to_cert_chain(ca_pem).unwrap() {
        roots.add(der).unwrap();
    }
    let chain = qe_device_pki::ca::pem_to_cert_chain(device_cert_pem).unwrap();
    let key = qe_device_pki::ca::pem_to_private_key(device_key_pem).unwrap();
    let mut client_cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .unwrap();
    client_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];

    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_cfg));
    let tcp = TcpStream::connect(addr).await.expect("connect");
    let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let tls_stream = connector.connect(server_name, tcp).await.expect("tls");

    let request = format!("wss://localhost:{}/channel", addr.port())
        .into_client_request()
        .unwrap();
    let (ws, _) = tokio_tungstenite::client_async(request, tls_stream)
        .await
        .expect("websocket handshake");
    ws
}

async fn next_frame(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>,
) -> serde_json::Value {
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("等一帧")
        .expect("连接不应关闭");
    let Message::Text(text) = msg.expect("帧") else {
        panic!("应为文本帧");
    };
    serde_json::from_str(&text).expect("JSON 帧")
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn ws_channel_connects_and_activates_enrollment() {
    // rustls 需要显式 provider（sqlx 用 aws-lc-rs、本 crate 用 ring，特征不唯一）
    let _ = rustls::crypto::ring::default_provider().install_default();

    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("skip: 未设置 TEST_DATABASE_URL");
            return;
        }
    };
    let db = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
    {
        Ok(db) => db,
        Err(e) => {
            eprintln!("skip: 连不上 PostgreSQL（{e}）");
            return;
        }
    };
    sqlx::migrate!("../../migrations")
        .run(&db)
        .await
        .expect("migrate");

    let slot = format!("ws-{}", uuid::Uuid::new_v4().simple());
    let cleanup = |db: sqlx::PgPool, slot: String| async move {
        for sql in [
            "DELETE FROM device_downlink WHERE device_slot = $1",
            "DELETE FROM device_message_inbox WHERE device_slot = $1",
            "DELETE FROM device_session WHERE device_slot = $1",
            "DELETE FROM device_enroll_challenge WHERE device_slot = $1",
            "DELETE FROM device_enrollment WHERE device_slot = $1",
            "DELETE FROM device_bootstrap_token WHERE device_slot = $1",
        ] {
            let _ = sqlx::query(sql).bind(&slot).execute(&db).await;
        }
    };

    // ---- CA 与服务端证书 ----
    let (ca_cert, ca_key, server_cert_pem, server_key_pem) = make_ca_and_server_cert();
    let ca_pem = ca_cert.pem();
    let ca_key_pem = ca_key.serialize_pem();
    let ca = Arc::new(Ca::load(&ca_key_pem, &ca_pem).expect("load ca"));
    let fp = tls::cert_fingerprint_hex(&ca_pem).unwrap();

    // ---- 服务端状态与两个路由 ----
    let (sessions, _ev) = SessionManager::new(Default::default());
    let cert_status = Arc::new(CertStatusCache::new());
    let state = AppState {
        db: db.clone(),
        ca,
        sessions: Arc::new(sessions),
        admin_token: Arc::from(ADMIN_TOKEN),
        server_cert_fp_hex: Arc::from(fp.as_str()),
        cert_status: Arc::clone(&cert_status),
        admin_limiter: Arc::new(RateLimiter::new(0)),
        dispatcher: Arc::new(qe_device_pki::messaging::Dispatcher::new(
            std::time::Duration::from_secs(5),
        )),
        mtls_endpoint: Arc::from("https://localhost:1"), // 测试里不用它
        enroll_endpoint: Arc::from("https://localhost:1"),
        // 预签名端点未配置 S3：本测试不覆盖对象存储
        storage: None,
        vnc: Arc::new(qe_device_pki::vnc::VncHub::new(
            qe_device_pki::vnc::VNC_PAIR_TIMEOUT,
        )),
        vnc_internal_token: None,
    };
    let enroll_app = routes::enroll_router(state.clone());
    let mtls_app = routes::mtls_router(state.clone());

    // 业务方订阅：证明帧协议真的走到分发器（而不是只做去重）
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen_h = Arc::clone(&seen);
    state
        .dispatcher
        .subscribe("heartbeat", move |m: qe_device_pki::messaging::Message| {
            let seen_h = Arc::clone(&seen_h);
            async move {
                seen_h.lock().unwrap().push(m.payload.clone());
                Ok(())
            }
        });

    // ---- 走真实注册流程拿到设备证书（auto 型）----
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let post = |uri: &str, body: serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let admin = |mut req: Request<Body>| {
        req.headers_mut().insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {ADMIN_TOKEN}")).unwrap(),
        );
        req
    };
    let call = |req: Request<Body>| {
        let app = enroll_app.clone();
        async move {
            let resp = app.oneshot(req).await.expect("route");
            let status = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let v: serde_json::Value =
                serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, v)
        }
    };

    let (status, body) = call(admin(post(
        "/admin/tokens",
        serde_json::json!({"kind": "auto", "slot": slot}),
    )))
    .await;
    assert_eq!(status, StatusCode::OK, "发 token: {body}");
    let token = body["token"].as_str().unwrap().to_owned();

    let (_, body) = call(post(
        "/enroll/challenge",
        serde_json::json!({"token": token, "slot": slot}),
    ))
    .await;
    let challenge_id = body["challenge_id"].as_str().unwrap().to_owned();
    let challenge = body["challenge"].as_str().unwrap().to_owned();

    // 设备密钥：CSR 用它，之后客户端身份也用它
    let device_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::default();
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, slot.as_str());
    params.distinguished_name = dn;
    let csr = params.serialize_request(&device_key).unwrap();
    let csr_der = csr.der().to_vec();

    // PoP：transcript 用服务端自述的指纹
    let fp_bytes = hex::decode(&fp).unwrap();
    let msg = qe_device_pki::pop::canonical_transcript(
        &hex::decode(&challenge).unwrap(),
        &qe_device_pki::pop::sha256(token.as_bytes()),
        &slot,
        &qe_device_pki::pop::sha256(&csr_der),
        &fp_bytes,
    );
    let ring_key = ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &device_key.serialize_der(),
        &ring::rand::SystemRandom::new(),
    )
    .unwrap();
    let sig = ring_key
        .sign(&ring::rand::SystemRandom::new(), &msg)
        .unwrap()
        .as_ref()
        .to_vec();

    let (status, body) = call(post(
        "/enroll",
        serde_json::json!({
            "challenge_id": challenge_id,
            "slot": slot,
            "token": token,
            "csr_der": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &csr_der),
            "pop_signature": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &sig),
            "pop_algorithm": qe_device_pki::pop::POP_ALGORITHM,
            "tls_cert_seen_fp": fp,
        }),
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "enroll: {body}");
    let device_cert_pem = body["cert_pem"].as_str().unwrap().to_owned();
    assert_eq!(body["status"], "issued");

    // 注册单此刻必须还是 issued（还没激活）
    let (before,): (String,) =
        sqlx::query_as("SELECT status FROM device_enrollment WHERE device_slot = $1")
            .bind(&slot)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(before, "issued");

    // fail-closed：握手前必须把证书可用性加载进来
    cert_status.reload(&db).await.expect("reload cert status");

    // ---- 起真实 mTLS listener ----
    let mtls_cfg = tls::server_config_mtls(
        &server_cert_pem,
        &server_key_pem,
        state.ca.clone(),
        Arc::clone(&cert_status),
    )
    .expect("mtls config");
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let listener_addr = listener.local_addr().expect("local addr");
    let handle = spawn_mtls_listener(listener, mtls_cfg, mtls_app).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // ---- 设备连上来 ----
    let mut ws = dial(
        listener_addr,
        &ca_pem,
        &device_cert_pem,
        &device_key.serialize_pem(),
    )
    .await;

    // 首帧必须是 ready —— 它就是"注册已激活"的回执
    let ready = next_frame(&mut ws).await;
    assert_eq!(ready["type"], "ready", "{ready}");
    assert_eq!(ready["status"], "active");
    assert_eq!(ready["serial"].as_str().unwrap().len(), 40);

    let (after,): (String,) =
        sqlx::query_as("SELECT status FROM device_enrollment WHERE device_slot = $1")
            .bind(&slot)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, "active", "建连本身必须把 issued 推进到 active");
    let (cert_pem_after,): (String,) =
        sqlx::query_as("SELECT cert_pem FROM device_enrollment WHERE device_slot = $1")
            .bind(&slot)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(cert_pem_after.is_empty(), "激活后 DB 不应再持有证书 PEM");

    // ---- 上行：帧协议走的是与 HTTP 同一条处理路径 ----
    ws.send(Message::Text(
        serde_json::json!({"type":"msg","seq":1,"kind":"heartbeat","payload":"{}"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let ack = next_frame(&mut ws).await;
    assert_eq!(ack["type"], "ack_msg", "{ack}");
    assert_eq!(ack["seq"], 1);
    assert_eq!(ack["status"], "ok");
    let (inbox_status,): (Option<String>,) = sqlx::query_as(
        "SELECT processed_at::text FROM device_message_inbox
         WHERE device_slot = $1 AND seq = 1",
    )
    .bind(&slot)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(inbox_status.is_some(), "上行应已进收件箱并处理");
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["{}".to_string()],
        "帧必须真的分发到业务 handler"
    );

    // ---- 下行：业务方投递 → 长连接立即收到（不用等轮询）----
    let serial = ready["serial"].as_str().unwrap().to_owned();
    qe_device_pki::messaging::downlink_publish(&db, &serial, &slot, "config", r#"{"mode":"a"}"#)
        .await
        .expect("publish");
    let down = next_frame(&mut ws).await;
    assert_eq!(down["type"], "down", "{down}");
    assert_eq!(down["kind"], "config");
    assert_eq!(down["payload"], r#"{"mode":"a"}"#);
    let down_id = down["id"].as_i64().unwrap();

    // 处理成功 → ack；服务端标记已投递
    ws.send(Message::Text(
        serde_json::json!({"type":"ack","up_to": down_id})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let (unacked,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM device_downlink WHERE device_slot = $1 AND delivered_at IS NULL",
    )
    .bind(&slot)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(unacked, 0, "ack 后不应还有未投递消息");

    // ---- 服务端关闭连接 ----
    ws.close(None).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    handle.abort();

    cleanup(db, slot).await;
}
