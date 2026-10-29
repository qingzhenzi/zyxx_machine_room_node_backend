pub mod admin;
pub mod announcements;
pub mod auth;
pub mod extractors;
pub mod openapi;
pub mod trace;
pub mod validation;

use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    middleware,
    routing::{get, post},
};
use serde_json::{Value, json};
use tower::ServiceBuilder;
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
};

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let cors = build_cors(&state);
    let timeout = state.config.request_timeout();
    let max_body_bytes = state.config.max_body_bytes;

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api-docs/openapi.json", get(openapi::json_spec))
        .route("/docs", get(openapi::ui))
        .nest("/api/v1", api_v1())
        .layer(
            ServiceBuilder::new()
                // 追踪中间件放在最外层，确保错误响应/panic 响应也带 trace 头。
                .layer(middleware::from_fn(trace::trace_middleware))
                .layer(CatchPanicLayer::new())
                .layer(RequestBodyLimitLayer::new(max_body_bytes))
                .layer(CompressionLayer::new())
                // Cors 需要其内层响应体实现 Default，因此必须放在 Compression 之内。
                .layer(cors)
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    timeout,
                )),
        )
        .with_state(state)
}

fn api_v1() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(auth::login))
        .route("/auth/refresh", post(auth::refresh))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/password/forgot", post(auth::forgot_password))
        .route("/auth/password/reset", post(auth::reset_password))
        .route(
            "/auth/email/verification",
            post(auth::request_email_verification),
        )
        .route("/auth/email/verify", post(auth::verify_email))
        .route(
            "/auth/sessions",
            get(auth::list_sessions).delete(auth::revoke_all_sessions),
        )
        .route(
            "/auth/sessions/{id}",
            axum::routing::delete(auth::revoke_session),
        )
        .route("/announcements", get(announcements::list))
        .route("/announcements/{id}", get(announcements::get))
        .nest("/admin", admin::router())
}

fn build_cors(state: &AppState) -> CorsLayer {
    let allow_origin = if state
        .config
        .cors_allowed_origins
        .iter()
        .any(|origin| origin == "*")
    {
        AllowOrigin::any()
    } else {
        let origins = state
            .config
            .cors_allowed_origins
            .iter()
            .filter_map(|origin| match origin.parse::<HeaderValue>() {
                Ok(value) => Some(value),
                Err(_) => {
                    tracing::warn!(%origin, "忽略非法的 CORS 允许来源");
                    None
                }
            })
            .collect::<Vec<_>>();
        AllowOrigin::list(origins)
    };

    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("x-trace-id"),
            HeaderName::from_static("traceparent"),
        ])
        // 浏览器端需要读取追踪 ID，才能把它带进错误上报。
        .expose_headers([
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("x-trace-id"),
        ])
        .max_age(Duration::from_secs(3600))
}

/// 存活探针：进程还在就返回 200，不依赖外部组件。
/// `instance` 用于验证 nginx 后面的多个副本确实在轮转。
async fn healthz(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "status": "ok", "instance": state.instance_id }))
}

/// 就绪探针：数据库与 Redis 都可用才返回 200，供负载均衡摘流。
async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let database_ok = state.db.ping().await.is_ok();
    let cache_ok = state.cache.ping().await.is_ok();
    let healthy = database_ok && cache_ok;

    (
        if healthy {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(json!({
            "status": if healthy { "ok" } else { "degraded" },
            "instance": state.instance_id,
            "database": database_ok,
            "cache": cache_ok,
        })),
    )
}
