//! 总控阶段 2：多控制节点联邦（MVP）。
//!
//! 契约见 `docs/federation.md`：
//! - `GET /admin/federation/summary`：本节点计数摘要；支持 `x-federation-token`（env
//!   `FEDERATION_TOKEN`）与 admin JWT 二选一，供其他控制节点拉取；
//! - `GET/POST /admin/federation/nodes`、`DELETE /nodes/{id}`、`POST /nodes/{id}/refresh`：
//!   联邦节点注册/删除/刷新（reqwest 拉远端 summary，5s 超时；成功 online、失败 offline，写审计）；
//! - `GET /admin/federation/overview`：本节点 + enabled 节点聚合 KPI。
//!
//! MVP 不做跨节点下发；节点 token 用于拉取只读摘要。

use std::time::Duration;

use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use qe_db::federation::{FederationNode, LocalSummary, NewFederationNode};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::rbac::permissions,
    error::{AppError, AppResult},
    state::AppState,
};

/// 拉取远端 summary 的超时。
const FEDERATION_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// 节点视为「在线」的最近刷新窗口（超过则该节点按 offline 展示）。
const NODE_ONLINE_TTL_SECS: i64 = 300;
/// 联邦 token 请求头。
const FEDERATION_TOKEN_HEADER: &str = "x-federation-token";
/// 节点名最大长度。
const NODE_NAME_MAX_CHARS: usize = 64;
/// token 最大长度（防止超大字段）。
const NODE_TOKEN_MAX_CHARS: usize = 512;

// ---------------------------------------------------------------------------
// 鉴权：x-federation-token 或 admin JWT
// ---------------------------------------------------------------------------

/// `/federation/summary` 的两种调用方。
#[derive(Debug, Clone, Copy)]
pub enum FederationAuth {
    /// 其他控制节点用 `x-federation-token` 拉取（只读）。
    Federated,
    /// 管理端用户 JWT。
    Admin(Uuid),
}

impl FromRequestParts<AppState> for FederationAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(header) = parts.headers.get(FEDERATION_TOKEN_HEADER) {
            let provided = header
                .to_str()
                .map_err(|_| AppError::unauthorized("x-federation-token 不是合法字符串"))?
                .trim();
            let expected = state
                .config
                .federation_token
                .as_deref()
                .ok_or_else(|| AppError::unauthorized("本节点未配置 FEDERATION_TOKEN"))?;
            if constant_time_eq(provided.as_bytes(), expected.trim().as_bytes()) {
                return Ok(Self::Federated);
            }
            return Err(AppError::unauthorized("x-federation-token 无效"));
        }
        let AuthUser { id } = AuthUser::from_request_parts(parts, state).await?;
        Ok(Self::Admin(id))
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// 请求/响应模型
// ---------------------------------------------------------------------------

/// `GET /admin/federation/summary` 响应（远端拉取后原样缓存）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationSummaryReply {
    /// 节点展示名（默认 `HOSTNAME`）。
    pub node_name: String,
    pub generated_at: DateTime<Utc>,
    pub counts: LocalSummary,
}

/// 联邦节点视图（不含 token）。
#[derive(Debug, Clone, Serialize)]
pub struct FederationNodeView {
    pub id: Uuid,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    /// `online` / `offline` / `unknown`（结合最近刷新时间计算）。
    pub status: &'static str,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub summary: Option<LocalSummary>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 聚合 KPI。
#[derive(Debug, Clone, Default, Serialize)]
pub struct FederationTotals {
    pub nodes_total: i64,
    pub nodes_online: i64,
    pub nodes_offline: i64,
    pub rooms: i64,
    pub areas: i64,
    pub machines: i64,
    pub online_machines: i64,
    pub open_tickets: i64,
    pub images: i64,
}

/// `GET /admin/federation/overview` 响应。
#[derive(Debug, Clone, Serialize)]
pub struct FederationOverview {
    pub node_name: String,
    pub generated_at: DateTime<Utc>,
    pub local: LocalSummary,
    pub nodes: Vec<FederationNodeView>,
    pub totals: FederationTotals,
}

#[derive(Debug, Deserialize)]
pub struct CreateFederationNodeRequest {
    pub name: String,
    pub base_url: String,
    pub token: String,
}

// ---------------------------------------------------------------------------
// 纯函数（单测覆盖）
// ---------------------------------------------------------------------------

/// 校验节点名：1..=64 字符，不允许控制字符。
fn validate_node_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    let length = name.chars().count();
    if !(1..=NODE_NAME_MAX_CHARS).contains(&length) {
        return Err(AppError::bad_request(format!(
            "节点名称长度需为 1-{NODE_NAME_MAX_CHARS} 个字符"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::bad_request("节点名称不能包含控制字符"));
    }
    Ok(name.to_owned())
}

/// 规范化 `base_url`：只接受 http/https、无 userinfo/query/fragment/路径，去掉尾部 `/`。
pub fn normalize_base_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("base_url 不能为空"));
    }
    let url = Url::parse(trimmed)
        .map_err(|error| AppError::bad_request(format!("base_url 不是合法 URL（{error}）")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::bad_request("base_url 只支持 http/https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::bad_request("base_url 不能包含用户名或密码"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::bad_request("base_url 不能包含 query 或 fragment"));
    }
    if !matches!(url.path(), "" | "/") {
        return Err(AppError::bad_request(
            "base_url 不能包含路径（只需 http://host:port）",
        ));
    }
    Ok(trimmed.trim_end_matches('/').to_owned())
}

/// 节点状态判定：最近一次刷新 online 且在 5 分钟内 → online；否则 offline/unknown。
pub fn effective_node_status(
    last_status: Option<&str>,
    last_seen_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> &'static str {
    match (last_status, last_seen_at) {
        (Some("online"), Some(seen))
            if now - seen <= chrono::Duration::seconds(NODE_ONLINE_TTL_SECS) =>
        {
            "online"
        }
        (Some("online"), _) | (Some("offline"), _) => "offline",
        _ => "unknown",
    }
}

/// 从远端 summary 响应（或旧缓存）解析计数。
pub fn parse_cached_summary(value: &Value) -> Option<LocalSummary> {
    if let Ok(reply) = serde_json::from_value::<FederationSummaryReply>(value.clone()) {
        return Some(reply.counts);
    }
    serde_json::from_value::<LocalSummary>(value.clone()).ok()
}

/// 聚合：本地节点 + 各联邦节点（只把 online 节点且能解析出摘要的计数计入合计）。
pub fn aggregate_totals(
    local: &LocalSummary,
    nodes: &[(&'static str, Option<LocalSummary>)],
) -> FederationTotals {
    let mut totals = FederationTotals {
        // 本节点始终在线并计入合计；nodes 仅为远端节点。
        nodes_total: nodes.len() as i64 + 1,
        nodes_online: 1,
        rooms: local.rooms,
        areas: local.areas,
        machines: local.machines,
        online_machines: local.online_machines,
        open_tickets: local.open_tickets,
        images: local.images,
        ..FederationTotals::default()
    };
    for (status, summary) in nodes {
        match *status {
            "online" => {
                totals.nodes_online += 1;
                if let Some(summary) = summary {
                    totals.rooms += summary.rooms;
                    totals.areas += summary.areas;
                    totals.machines += summary.machines;
                    totals.online_machines += summary.online_machines;
                    totals.open_tickets += summary.open_tickets;
                    totals.images += summary.images;
                }
            }
            "offline" => totals.nodes_offline += 1,
            _ => {}
        }
    }
    totals
}

fn node_view(node: &FederationNode, now: DateTime<Utc>) -> FederationNodeView {
    FederationNodeView {
        id: node.id,
        name: node.name.clone(),
        base_url: node.base_url.clone(),
        enabled: node.enabled,
        status: effective_node_status(node.last_status.as_deref(), node.last_seen_at, now),
        last_seen_at: node.last_seen_at,
        last_status: node.last_status.clone(),
        summary: node.cached_summary.as_ref().and_then(parse_cached_summary),
        created_at: node.created_at,
        updated_at: node.updated_at,
    }
}

fn node_name() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "control-node".to_owned())
}

/// 拉取远端 `/admin/federation/summary`；失败返回可读错误（调用方标记 offline）。
async fn fetch_remote_summary(node: &FederationNode) -> Result<FederationSummaryReply, String> {
    let url = format!(
        "{}/api/v1/admin/federation/summary",
        node.base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(FEDERATION_FETCH_TIMEOUT)
        .build()
        .map_err(|error| format!("构建 HTTP 客户端失败: {error}"))?;
    let response = client
        .get(&url)
        .header(FEDERATION_TOKEN_HEADER, &node.token)
        .send()
        .await
        .map_err(|error| format!("请求远端失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("远端返回 HTTP {}", response.status().as_u16()));
    }
    response
        .json::<FederationSummaryReply>()
        .await
        .map_err(|error| format!("解析远端 summary 失败: {error}"))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/admin/federation/summary` —— 本节点计数摘要（联邦拉取 / 控制台也能看）。
pub async fn summary(
    State(state): State<AppState>,
    _auth: FederationAuth,
) -> AppResult<Json<FederationSummaryReply>> {
    let counts = state.federation.local_summary().await?;
    Ok(Json(FederationSummaryReply {
        node_name: node_name(),
        generated_at: Utc::now(),
        counts,
    }))
}

/// `GET /api/v1/admin/federation/nodes` —— 全部联邦节点（token 不回显）。
pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<FederationNodeView>>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;
    let now = Utc::now();
    let nodes = state.federation.list().await?;
    Ok(Json(
        nodes.iter().map(|node| node_view(node, now)).collect(),
    ))
}

/// `POST /api/v1/admin/federation/nodes` —— 注册一个联邦控制节点。
pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateFederationNodeRequest>,
) -> AppResult<(StatusCode, Json<FederationNodeView>)> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;

    let name = validate_node_name(&request.name)?;
    let base_url = normalize_base_url(&request.base_url)?;
    let token = request.token.trim();
    if token.is_empty() {
        return Err(AppError::bad_request("token 不能为空"));
    }
    if token.chars().count() > NODE_TOKEN_MAX_CHARS {
        return Err(AppError::bad_request(format!(
            "token 不能超过 {NODE_TOKEN_MAX_CHARS} 个字符"
        )));
    }

    let node = state
        .federation
        .create(&NewFederationNode {
            name,
            base_url,
            token: token.to_owned(),
        })
        .await
        .map_err(|error| match error {
            qe_db::error::DbError::Conflict(_) => AppError::conflict("同名联邦节点已存在"),
            other => AppError::from(other),
        })?;

    super::audit::record_or_log(
        &state,
        id,
        "federation.node.create",
        "federation_node",
        Some(node.id.to_string()),
        serde_json::json!({ "name": node.name, "base_url": node.base_url }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(node_view(&node, Utc::now()))))
}

/// `DELETE /api/v1/admin/federation/nodes/{id}`。
pub async fn delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(node_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;
    let node = state
        .federation
        .get(node_id)
        .await?
        .ok_or_else(|| AppError::not_found("联邦节点不存在"))?;
    if !state.federation.delete(node_id).await? {
        return Err(AppError::not_found("联邦节点不存在"));
    }
    super::audit::record_or_log(
        &state,
        id,
        "federation.node.delete",
        "federation_node",
        Some(node_id.to_string()),
        serde_json::json!({ "name": node.name, "base_url": node.base_url }),
        ip.as_deref(),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/admin/federation/nodes/{id}/refresh` —— 拉取远端 summary。
///
/// 成功：last_status=online、last_seen_at=now、cached_summary=远端摘要；
/// 失败：last_status=offline（保留上次缓存），返回 502 并写审计。
pub async fn refresh(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(node_id): Path<Uuid>,
) -> AppResult<Json<FederationNodeView>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;
    let node = state
        .federation
        .get(node_id)
        .await?
        .ok_or_else(|| AppError::not_found("联邦节点不存在"))?;

    match fetch_remote_summary(&node).await {
        Ok(remote) => {
            let cached = serde_json::to_value(&remote)
                .map_err(|error| AppError::internal(format!("序列化节点摘要失败: {error}")))?;
            let updated = state
                .federation
                .mark_online(node_id, &cached)
                .await?
                .ok_or_else(|| AppError::not_found("联邦节点不存在"))?;
            super::audit::record_or_log(
                &state,
                id,
                "federation.node.refresh",
                "federation_node",
                Some(node_id.to_string()),
                serde_json::json!({ "status": "online", "counts": remote.counts }),
                ip.as_deref(),
            )
            .await;
            Ok(Json(node_view(&updated, Utc::now())))
        }
        Err(error) => {
            state.federation.mark_offline(node_id).await?;
            super::audit::record_or_log(
                &state,
                id,
                "federation.node.refresh",
                "federation_node",
                Some(node_id.to_string()),
                serde_json::json!({ "status": "offline", "error": error }),
                ip.as_deref(),
            )
            .await;
            Err(AppError::bad_gateway(format!("联邦节点刷新失败：{error}")))
        }
    }
}

/// `GET /api/v1/admin/federation/overview` —— 本节点 + enabled 节点聚合总览。
pub async fn overview(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<FederationOverview>> {
    authorization::require_permission(&state, id, permissions::DEVICE_READ).await?;
    let now = Utc::now();
    let local = state.federation.local_summary().await?;
    let nodes: Vec<FederationNodeView> = state
        .federation
        .list()
        .await?
        .into_iter()
        .filter(|node| node.enabled)
        .map(|node| node_view(&node, now))
        .collect();
    let aggregate_inputs: Vec<(&'static str, Option<LocalSummary>)> = nodes
        .iter()
        .map(|node| (node.status, node.summary))
        .collect();
    let totals = aggregate_totals(&local, &aggregate_inputs);

    Ok(Json(FederationOverview {
        node_name: node_name(),
        generated_at: now,
        local,
        nodes,
        totals,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    #[test]
    fn base_url_normalization() {
        assert_eq!(
            normalize_base_url("http://127.0.0.1:8081/").unwrap(),
            "http://127.0.0.1:8081"
        );
        assert_eq!(
            normalize_base_url(" https://node.example.com ").unwrap(),
            "https://node.example.com"
        );
        for bad in [
            "ftp://node",
            "http://user:pass@node",
            "http://node/path",
            "http://node?x=1",
            "http://node#frag",
            "",
        ] {
            assert!(normalize_base_url(bad).is_err(), "应拒绝 base_url={bad}");
        }
    }

    #[test]
    fn node_name_validation() {
        assert!(validate_node_name("node-b").is_ok());
        assert!(validate_node_name("机房 A · 控制节点").is_ok());
        assert!(validate_node_name("   ").is_err());
        assert!(validate_node_name(&"x".repeat(65)).is_err());
        assert!(validate_node_name("bad\nname").is_err());
    }

    #[test]
    fn effective_status_online_offline_unknown() {
        let now = Utc::now();
        assert_eq!(
            effective_node_status(Some("online"), Some(now - ChronoDuration::seconds(10)), now),
            "online"
        );
        assert_eq!(
            effective_node_status(
                Some("online"),
                Some(now - ChronoDuration::seconds(600)),
                now
            ),
            "offline"
        );
        assert_eq!(
            effective_node_status(Some("offline"), Some(now), now),
            "offline"
        );
        assert_eq!(effective_node_status(None, None, now), "unknown");
    }

    #[test]
    fn aggregate_only_counts_online_nodes_for_kpi() {
        let local = LocalSummary {
            rooms: 10,
            areas: 2,
            machines: 5,
            online_machines: 3,
            open_tickets: 1,
            images: 4,
        };
        let remote = LocalSummary {
            rooms: 7,
            areas: 1,
            machines: 2,
            online_machines: 2,
            open_tickets: 3,
            images: 1,
        };
        let totals = aggregate_totals(
            &local,
            &[
                ("online", Some(remote)),
                ("offline", None),
                ("unknown", None),
            ],
        );
        // nodes_total 含本节点（1）+ 远端 3；本节点始终在线。
        assert_eq!(totals.nodes_total, 4);
        assert_eq!(totals.nodes_online, 2);
        assert_eq!(totals.nodes_offline, 1);
        assert_eq!(totals.rooms, 17);
        assert_eq!(totals.machines, 7);
        assert_eq!(totals.online_machines, 5);
        assert_eq!(totals.open_tickets, 4);
        assert_eq!(totals.images, 5);
    }

    #[test]
    fn parse_cached_summary_accepts_reply_and_bare_counts() {
        let counts = LocalSummary {
            rooms: 1,
            areas: 1,
            machines: 2,
            online_machines: 1,
            open_tickets: 0,
            images: 3,
        };
        let reply = FederationSummaryReply {
            node_name: "node-b".to_owned(),
            generated_at: Utc::now(),
            counts,
        };
        let reply_value = serde_json::to_value(&reply).unwrap();
        assert_eq!(parse_cached_summary(&reply_value), Some(counts));
        let bare = serde_json::to_value(counts).unwrap();
        assert_eq!(parse_cached_summary(&bare), Some(counts));
        assert_eq!(parse_cached_summary(&serde_json::json!({"nope": 1})), None);
    }

    #[test]
    fn constant_time_token_compare() {
        assert!(constant_time_eq(b"token", b"token"));
        assert!(!constant_time_eq(b"token", b"tokeN"));
        assert!(!constant_time_eq(b"token", b"token-longer"));
    }
}
