#![recursion_limit = "256"]

pub mod api;
pub mod auth;
pub mod bootstrap;
pub mod config;
pub mod email;
pub mod error;
pub mod scheduler;
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
    let _otel_provider = init_tracing(config.log_format_json);

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

    // 课表事件引擎：后台任务，自身错误只记录日志，不影响 HTTP 服务。
    if config.schedule_engine_enabled {
        // JoinHandle detach：引擎生命周期跟随进程，随进程退出一起结束。
        let _engine = scheduler::spawn(state.clone());
        tracing::info!(tick_secs = config.schedule_tick_secs, "课表事件引擎已启动");
    } else {
        tracing::info!("课表事件引擎已禁用（SCHEDULE_ENGINE_ENABLED=false）");
    }

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

struct TelemetryGuard {
    _tracer: opentelemetry_sdk::trace::SdkTracerProvider,
    _logger: opentelemetry_sdk::logs::SdkLoggerProvider,
}

fn init_tracing(json: bool) -> Option<TelemetryGuard> {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,hyper=warn"));

    let otel = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .and_then(|endpoint| {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint.clone())
                .build()
                .map_err(|error| eprintln!("OTLP span exporter init failed: {error}"))
                .ok()?;
            let resource = opentelemetry_sdk::Resource::builder()
                .with_service_name(
                    std::env::var("OTEL_SERVICE_NAME")
                        .unwrap_or_else(|_| "qestudio-backend".to_owned()),
                )
                .build();
            let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(resource.clone())
                .build();
            let tracer = tracer_provider.tracer("qe-api");

            let logs_enabled = std::env::var("OTEL_LOGS_ENABLED")
                .ok()
                .is_some_and(|value| {
                    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
                });
            let mut logger_builder =
                opentelemetry_sdk::logs::SdkLoggerProvider::builder().with_resource(resource);
            if logs_enabled {
                match opentelemetry_otlp::LogExporter::builder()
                    .with_http()
                    .with_endpoint(endpoint)
                    .build()
                {
                    Ok(exporter) => {
                        logger_builder = logger_builder.with_simple_exporter(exporter);
                    }
                    Err(error) => eprintln!("OTLP log exporter init failed: {error}"),
                }
            }
            let logger_provider = logger_builder.build();
            Some((tracer_provider, tracer, logger_provider))
        });

    if let Some((tracer_provider, tracer, logger_provider)) = otel {
        let log_bridge = opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(
            &logger_provider,
        );
        if json {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
                .with(tracing_opentelemetry::layer().with_tracer(tracer.clone()))
                .with(log_bridge)
                .init();
        } else {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .with(log_bridge)
                .init();
        }
        tracing::info!("OpenTelemetry OTLP trace/log export 已启用");
        return Some(TelemetryGuard {
            _tracer: tracer_provider,
            _logger: logger_provider,
        });
    }

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
    None
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
