//! 设备 PKI 服务端二进制：同时起注册 listener（单向 TLS）与业务 listener（mTLS）。

use anyhow::{Context, Result};
use qe_device_pki::ca::Ca;
use qe_device_pki::revocation::{self, RevocationCache};
use qe_device_pki::routes::{self, AppState};
use qe_device_pki::session::{DebounceConfig, SessionManager};
use qe_device_pki::tls;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 同时被 sqlx(aws-lc-rs) 与 ring 引入时无法自动选 provider，必须显式安装
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls CryptoProvider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,qe_device_pki=debug".into()),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL required")?;
    let enroll_addr = std::env::var("ENROLL_ADDR").unwrap_or_else(|_| "0.0.0.0:4440".into());
    let mtls_addr = std::env::var("MTLS_ADDR").unwrap_or_else(|_| "0.0.0.0:4441".into());
    let enroll_cert_path = std::env::var("ENROLL_TLS_CERT").context("ENROLL_TLS_CERT required")?;
    let enroll_key_path = std::env::var("ENROLL_TLS_KEY").context("ENROLL_TLS_KEY required")?;
    let mtls_cert_path = std::env::var("MTLS_TLS_CERT").context("MTLS_TLS_CERT required")?;
    let mtls_key_path = std::env::var("MTLS_TLS_KEY").context("MTLS_TLS_KEY required")?;
    let admin_token = std::env::var("ADMIN_TOKEN").context("ADMIN_TOKEN required")?;
    // 对外公布的 mTLS 地址（客户端拿到证书后连这个，不是注册地址）
    let public_mtls = std::env::var("PUBLIC_MTLS_ENDPOINT")
        .unwrap_or_else(|_| format!("https://{mtls_addr}"));

    // 环境变量给的是文件路径，这里读成 PEM 内容
    let enroll_cert = std::fs::read_to_string(&enroll_cert_path)
        .with_context(|| format!("读取 {enroll_cert_path} 失败"))?;
    let enroll_key = std::fs::read_to_string(&enroll_key_path)
        .with_context(|| format!("读取 {enroll_key_path} 失败"))?;
    let mtls_cert = std::fs::read_to_string(&mtls_cert_path)
        .with_context(|| format!("读取 {mtls_cert_path} 失败"))?;
    let mtls_key = std::fs::read_to_string(&mtls_key_path)
        .with_context(|| format!("读取 {mtls_key_path} 失败"))?;

    let ca_key_pem = std::fs::read_to_string(
        std::env::var("CA_KEY").unwrap_or_else(|_| "certs/ca-key.pem".into()),
    )
    .context("read CA key")?;
    let ca_cert_pem = std::fs::read_to_string(
        std::env::var("CA_CERT").unwrap_or_else(|_| "certs/ca.pem".into()),
    )
    .context("read CA cert")?;
    let ca = Arc::new(Ca::load(&ca_key_pem, &ca_cert_pem)?);

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context("connect database")?;

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("run migrations")?;

    // 吊销状态：启动时先加载一次全量，之后每 5s 刷新
    let revocations = Arc::new(RevocationCache::new());
    match revocations.reload(&db).await {
        Ok(n) => tracing::info!(revoked = n, "revocation cache loaded"),
        Err(e) => tracing::warn!(error = %e, "initial revocation load failed"),
    }
    let _revocation_refresher = revocation::spawn_refresher(
        Arc::clone(&revocations),
        db.clone(),
        revocation::DEFAULT_REFRESH_INTERVAL,
    );

    let (sessions, mut events) = SessionManager::new(DebounceConfig::default());
    let sessions = Arc::new(sessions);

    // 会话状态变更落库（防抖合并后的最终状态）
    let db_for_events = db.clone();
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            tracing::info!(
                serial = %ev.serial,
                slot = %ev.device_slot,
                state = ev.state.as_str(),
                seq = ev.last_seen_seq,
                "session state committed"
            );
            let _ = sqlx::query(
                "INSERT INTO device_session (serial, device_slot, last_seen_seq, state, updated_at)
                 VALUES ($1, $2, $3, $4, now())
                 ON CONFLICT (serial) DO UPDATE SET
                   last_seen_seq = EXCLUDED.last_seen_seq,
                   state = EXCLUDED.state,
                   updated_at = now()",
            )
            .bind(&ev.serial)
            .bind(&ev.device_slot)
            .bind(ev.last_seen_seq)
            .bind(ev.state.as_str())
            .execute(&db_for_events)
            .await;
        }
    });

    let enroll_fp = tls::cert_fingerprint_hex(&enroll_cert)?;

    let state = AppState {
        db: db.clone(),
        ca: Arc::clone(&ca),
        sessions,
        admin_token: Arc::from(admin_token.as_str()),
        server_cert_fp_hex: Arc::from(enroll_fp.as_str()),
        revocations: Arc::clone(&revocations),
        mtls_endpoint: Arc::from(public_mtls.as_str()),
    };

    let enroll_cfg = tls::server_config_enroll(&enroll_cert, &enroll_key)?;
    let mtls_cfg =
        tls::server_config_mtls(&mtls_cert, &mtls_key, Arc::clone(&ca), revocations)?;

    let enroll_app = routes::enroll_router(state.clone());
    let mtls_app = routes::mtls_router(state);

    let enroll_listener = tokio::net::TcpListener::bind(&enroll_addr).await?;
    let mtls_listener = tokio::net::TcpListener::bind(&mtls_addr).await?;
    tracing::info!("enroll listener (one-way TLS) on {enroll_addr}");
    tracing::info!("mtls listener (client cert required) on {mtls_addr}");

    let enroll_task = axum_server_with_tls(enroll_listener, enroll_cfg, enroll_app);
    let mtls_task = axum_server_with_tls(mtls_listener, mtls_cfg, mtls_app);

    tokio::try_join!(enroll_task, mtls_task)?;
    Ok(())
}

/// 用 tokio-rustls 手工 accept 循环服务 axum，并注入 mTLS 对端身份。
async fn axum_server_with_tls(
    listener: tokio::net::TcpListener,
    config: Arc<tokio_rustls::rustls::ServerConfig>,
    app: axum::Router,
) -> Result<()> {
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("accept failed: {e}");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!("tls handshake failed from {peer_addr}: {e}");
                    return;
                }
            };

            // 提取客户端证书（mTLS listener 才有）
            let peer_cert = tls_stream
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|c| c.first().map(|d| qe_device_pki::tls::PeerCert::from_der(d.to_vec())));

            let svc = app.clone();
            let io = hyper_util::rt::TokioIo::new(tls_stream);
            let svc = hyper_util::service::TowerToHyperService::new(
                svc.layer(axum::middleware::from_fn(
                    move |mut req: axum::extract::Request, next: axum::middleware::Next| {
                        let pc = peer_cert.clone();
                        async move {
                            if let Some(pc) = pc {
                                req.extensions_mut().insert(pc);
                            }
                            next.run(req).await
                        }
                    },
                )),
            );
            if let Err(e) = hyper_util::server::conn::auto::Builder::new(
                hyper_util::rt::TokioExecutor::new(),
            )
            .serve_connection_with_upgrades(io, svc)
            .await
            {
                tracing::debug!("connection error from {peer_addr}: {e}");
            }
        });
    }
}
