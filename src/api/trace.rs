use std::time::Instant;

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use tracing::Instrument;
use uuid::Uuid;

/// 响应头中返回给客户端的追踪标识。
pub const TRACE_ID_HEADER: &str = "x-trace-id";
pub const REQUEST_ID_HEADER: &str = "x-request-id";

const TRACEPARENT_HEADER: &str = "traceparent";
const MAX_REQUEST_ID_LEN: usize = 128;

/// 当前请求的追踪上下文，挂在 request extensions 上，后续 handler 或下游 HTTP 调用可复用。
#[derive(Debug, Clone)]
pub struct TraceContext {
    /// W3C trace-id：32 位十六进制。没有接入 OpenTelemetry 时，它同时充当请求级追踪 ID。
    pub trace_id: String,
    /// 边缘代理（nginx）或本服务生成的请求 ID，用于日志关联。
    pub request_id: String,
}

impl TraceContext {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let trace_id = headers
            .get(TRACEPARENT_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(trace_id_from_traceparent)
            .or_else(|| {
                headers
                    .get(TRACE_ID_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(normalize_trace_id)
            })
            .unwrap_or_else(generate_trace_id);

        let request_id = headers
            .get(REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= MAX_REQUEST_ID_LEN)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        Self {
            trace_id,
            request_id,
        }
    }
}

/// 最外层中间件：
/// 1. 解析/生成 trace_id 与 request_id；
/// 2. 建立 `http_request` span，让 handler 内所有日志都带这两个字段；
/// 3. 请求结束后记录状态码/耗时，并把 ID 写回响应头。
pub async fn trace_middleware(mut request: Request, next: Next) -> Response {
    let context = TraceContext::from_headers(request.headers());
    request.extensions_mut().insert(context.clone());

    let method = request.method().clone();
    let uri = request.uri().clone();
    let span = tracing::info_span!(
        "http_request",
        trace_id = %context.trace_id,
        request_id = %context.request_id,
        method = %method,
        uri = %uri,
    );

    let started = Instant::now();
    let mut response = next.run(request).instrument(span.clone()).await;
    let status = response.status().as_u16();
    let latency_ms = started.elapsed().as_millis() as u64;

    tracing::info!(parent: &span, status, latency_ms, "request completed");

    insert_header(response.headers_mut(), TRACE_ID_HEADER, &context.trace_id);
    insert_header(
        response.headers_mut(),
        REQUEST_ID_HEADER,
        &context.request_id,
    );
    response
}

fn insert_header(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(HeaderName::from_static(name), value);
    }
}

fn generate_trace_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// 从 W3C `traceparent: 00-<32hex trace-id>-<16hex parent-id>-<2hex flags>` 中提取 trace-id。
fn trace_id_from_traceparent(value: &str) -> Option<String> {
    let mut parts = value.split('-');
    let version = parts.next()?;
    let trace_id = parts.next()?;
    let parent_id = parts.next()?;
    let flags = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if version.len() != 2 || !is_hex(version) || version.eq_ignore_ascii_case("ff") {
        return None;
    }
    if parent_id.len() != 16 || !is_hex(parent_id) {
        return None;
    }
    if flags.len() != 2 || !is_hex(flags) {
        return None;
    }
    normalize_trace_id(trace_id)
}

fn normalize_trace_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() != 32 || !is_hex(value) {
        return None;
    }
    if value.bytes().all(|byte| byte == b'0') {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn is_hex(value: &str) -> bool {
    value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        headers
    }

    #[test]
    fn parses_w3c_traceparent() {
        let headers = headers(&[(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )]);
        let context = TraceContext::from_headers(&headers);
        assert_eq!(context.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn accepts_x_trace_id_and_lowercases_it() {
        let headers = headers(&[("x-trace-id", "4BF92F3577B34DA6A3CE929D0E0E4736")]);
        let context = TraceContext::from_headers(&headers);
        assert_eq!(context.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn rejects_invalid_or_zero_trace_ids() {
        for value in [
            "not-a-trace-id",
            "00000000000000000000000000000000",
            "4bf92f3577b34da6a3ce929d0e0e473",
        ] {
            let headers = headers(&[("x-trace-id", value)]);
            let context = TraceContext::from_headers(&headers);
            assert_ne!(context.trace_id, value);
            assert_eq!(context.trace_id.len(), 32);
        }
    }

    #[test]
    fn generates_trace_id_when_missing() {
        let context = TraceContext::from_headers(&HeaderMap::new());
        assert_eq!(context.trace_id.len(), 32);
        assert!(
            context
                .trace_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        assert!(!context.request_id.is_empty());
    }

    #[test]
    fn preserves_request_id_from_nginx() {
        let headers = headers(&[("x-request-id", "req-123")]);
        let context = TraceContext::from_headers(&headers);
        assert_eq!(context.request_id, "req-123");
    }
}
