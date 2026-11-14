pub mod admin;
pub mod announcements;
pub mod auth;
pub mod extractors;
pub mod machine_agent;
pub mod openapi;
pub mod trace;
pub mod validation;

use std::{any::Any, time::Duration};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde_json::{Value, json};
use tower::ServiceBuilder;
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
};

use crate::{error::AppError, state::AppState};

pub fn router(state: AppState) -> Router {
    // 长上传路由单独构建（16 GiB / 2h），不能进入下面的全局 body 限制/超时层。
    let iso_upload = iso_upload_router(state.clone());
    let cors = build_cors(&state);
    let timeout = state.config.request_timeout();
    let max_body_bytes = state.config.max_body_bytes;
    let production = state.config.environment.is_production();

    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api-docs/openapi.json", get(openapi::json_spec))
        .route("/docs", get(openapi::ui))
        .nest("/api/v1", api_v1())
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(
            ServiceBuilder::new()
                // 追踪中间件放在最外层，确保错误响应/panic 响应也带 trace 头。
                .layer(middleware::from_fn(trace::trace_middleware))
                .layer(middleware::from_fn(move |request, next| {
                    security_headers(request, next, production)
                }))
                .layer(CatchPanicLayer::custom(handle_panic))
                .layer(DefaultBodyLimit::max(max_body_bytes))
                .layer(RequestBodyLimitLayer::new(max_body_bytes))
                .layer(CompressionLayer::new())
                // Cors 需要其内层响应体实现 Default，因此必须放在 Compression 之内。
                .layer(cors)
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    timeout,
                )),
        )
        .with_state(state);
    router.merge(iso_upload)
}

/// ISO 内容上传专用路由：`PUT /api/v1/admin/isos/{id}/content`。
///
/// 大 ISO（4.7GB / 上限 16 GiB）需要远大于普通接口的 body 上限与超时，因此**不**
/// 经过主路由的 `DefaultBodyLimit`（2 MiB）与 `TimeoutLayer`（30s）；其余安全头/
/// CORS/追踪/禁缓存能力保持一致。
fn iso_upload_router(state: AppState) -> Router {
    use qe_domain::install_iso::MAX_ISO_BYTES;

    // 多留 1 MiB：超限时让 handler 的流式计数先触发 413（语义更明确），
    // 而不是被 body limit 层直接拒绝。
    let max_body_bytes = usize::try_from(MAX_ISO_BYTES)
        .unwrap_or(usize::MAX)
        .saturating_add(1024 * 1024);
    let upload_timeout = Duration::from_secs(2 * 60 * 60);
    let cors = build_cors(&state);
    let production = state.config.environment.is_production();
    Router::new()
        .route(
            "/api/v1/admin/isos/{id}/content",
            put(admin::isos::upload_content),
        )
        .layer(
            ServiceBuilder::new()
                .layer(middleware::from_fn(trace::trace_middleware))
                .layer(middleware::from_fn(move |request, next| {
                    security_headers(request, next, production)
                }))
                .layer(SetResponseHeaderLayer::overriding(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("no-store"),
                ))
                .layer(DefaultBodyLimit::max(max_body_bytes))
                .layer(cors)
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    upload_timeout,
                )),
        )
        .with_state(state)
}

/// 统一给所有响应（含 404/405/panic）加基础安全头；HSTS 只在 production 下发送。
async fn security_headers(request: Request, next: Next, production: bool) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );
    if production {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

fn handle_panic(error: Box<dyn Any + Send + 'static>) -> Response {
    let message = error
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| error.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic");
    tracing::error!(panic = %message, "request panicked");
    AppError::internal("panic").into_response()
}

async fn not_found() -> Response {
    AppError::not_found("接口不存在").into_response()
}

async fn method_not_allowed() -> Response {
    AppError::method_not_allowed("该接口不支持此 HTTP 方法").into_response()
}

fn api_v1() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth_routes())
        .route("/announcements", get(announcements::list))
        .route("/announcements/{id}", get(announcements::get))
        // 机器遥测上报（room-agent 的 fleet token 认证在 handler 内完成）
        .route("/machine/report", post(machine_agent::report))
        // 管理接口可能包含用户/审计数据，禁止中间缓存。
        .nest(
            "/admin",
            admin::router().layer(SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            )),
        )
}

fn auth_routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(auth::login))
        .route("/refresh", post(auth::refresh))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/password/forgot", post(auth::forgot_password))
        .route("/password/reset", post(auth::reset_password))
        .route(
            "/email/verification",
            post(auth::request_email_verification),
        )
        .route("/email/verify", post(auth::verify_email))
        .route(
            "/sessions",
            get(auth::list_sessions).delete(auth::revoke_all_sessions),
        )
        .route(
            "/sessions/{id}",
            axum::routing::delete(auth::revoke_session),
        )
        // 令牌响应不能被浏览器/代理缓存。
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
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
