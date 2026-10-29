pub mod api;
pub mod auth;
pub mod bootstrap;
pub mod config;
pub mod error;
pub mod state;

// qe-api 作为 HTTP/装配层依赖下层 crate，而不是反过来。
pub use qe_cache as cache;
pub use qe_db as db;
pub use qe_domain as domain;

use anyhow::Context;
use tokio::net::TcpListener;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{cache::Cache, config::Config, db::Db, state::AppState};

/// 启动入口：加载配置 → 连接依赖 → 迁移/引导 → 启动 HTTP 服务。
pub async fn run() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let config = Config::from_env()?;
    init_tracing(config.log_format_json);

    tracing::info!(addr = %config.bind_addr, "正在启动 backend");

    let db = Db::connect(&config.database_url, config.db_max_connections)
        .await
        .context("连接 PostgreSQL 失败")?;

    if config.run_migrations {
        db.migrate().await.context("执行数据库迁移失败")?;
        tracing::info!("数据库迁移完成");
    }

    let cache = Cache::connect(&config.redis_url)
        .await
        .context("连接 Redis 失败")?;

    let state = AppState::new(db, cache, config.clone());
    bootstrap::ensure_bootstrap_admin(&state)
        .await
        .context("初始化引导管理员失败")?;

    let listener = TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("监听 {} 失败", config.bind_addr))?;

    tracing::info!(addr = %config.bind_addr, "backend 已就绪");
    axum::serve(listener, api::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP 服务异常退出")?;

    tracing::info!("backend 已退出");
    Ok(())
}

fn init_tracing(json: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,hyper=warn"));

    if json {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "无法监听 Ctrl+C");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "无法监听 SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("收到退出信号，开始优雅停机");
}
