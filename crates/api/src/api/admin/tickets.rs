//! 工单（Ticket / Work Order）管理接口。
//!
//! 契约见 `docs/design/ticket-implementation-contract.md` §6。挂在 `/api/v1/admin`：
//!
//! | 方法 | 路径 | 权限 |
//! | --- | --- | --- |
//! | GET | `/tickets` | `ticket:read` |
//! | GET | `/tickets/summary` | `ticket:read` |
//! | GET | `/tickets/stats` | `ticket:read` |
//! | GET | `/tickets/assignees` | `ticket:handle` |
//! | POST | `/tickets` | `ticket:create` |
//! | GET | `/tickets/{id}` | `ticket:read` |
//! | PATCH | `/tickets/{id}` | `ticket:manage` |
//! | POST | `/tickets/{id}/claim` | `ticket:handle` |
//! | POST | `/tickets/{id}/assign` | `ticket:manage` |
//! | POST | `/tickets/{id}/transition` | `ticket:handle` |
//! | POST | `/tickets/{id}/comments` | `ticket:handle` |
//!
//! 人工写操作统一写审计（`ticket.create / ticket.assign / ticket.status /
//! ticket.comment`，`resource_type = "ticket"`）；状态机冲突 → 409。

use std::{collections::VecDeque, convert::Infallible};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{
        Response,
        sse::{Event, Sse},
    },
};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        models::Page,
        rbac::permissions,
        ticket::{
            NewTicket, Ticket, TicketCategory, TicketDetail, TicketEvent, TicketExportRow,
            TicketFilter, TicketPriority, TicketSource, TicketStats, TicketStatus,
            TicketStreamEvent, TicketSummary, TransitionInput, validate_root_cause,
        },
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

// ----------------------------------------------------------------------
// Wave 2 B：数据范围（按区域）helper
// ----------------------------------------------------------------------

/// 当前用户的工单数据范围：`None` = 不限制（scope:all 或无授权行）；`Some` = 限定区域。
pub(super) async fn scope_area_ids(
    state: &AppState,
    user_id: Uuid,
) -> AppResult<Option<Vec<Uuid>>> {
    let granted = authorization::permissions_of(state, user_id).await?;
    if granted
        .iter()
        .any(|code| code == permissions::TICKET_SCOPE_ALL)
    {
        return Ok(None);
    }
    let areas = state.ticket_scopes.area_ids_for_user(user_id).await?;
    // 没有任何 scope 行 = 向后兼容，不限制。
    Ok(if areas.is_empty() { None } else { Some(areas) })
}

/// 区域是否在数据范围内（`None` 范围 = 不限制；工单无区域 = 仅不受限用户可见）。
pub(super) fn area_in_scope(scoped: &Option<Vec<Uuid>>, area_id: Option<Uuid>) -> bool {
    match scoped {
        None => true,
        Some(areas) => area_id.is_some_and(|area| areas.contains(&area)),
    }
}

/// 写操作的数据范围校验：范围外 → 403；工单不存在 → 404。
pub(super) async fn ensure_write_scope(
    state: &AppState,
    user_id: Uuid,
    ticket_id: Uuid,
) -> AppResult<()> {
    let scoped = scope_area_ids(state, user_id).await?;
    if scoped.is_none() {
        return Ok(());
    }
    match state.tickets.ticket_area(ticket_id).await? {
        Some(area) => {
            if !area_in_scope(&scoped, Some(area)) {
                Err(AppError::forbidden("无权操作该区域的工单"))
            } else {
                Ok(())
            }
        }
        None => {
            if state.tickets.get_ticket(ticket_id).await?.is_some() {
                Err(AppError::forbidden("无权操作该区域的工单"))
            } else {
                Err(AppError::not_found("工单不存在"))
            }
        }
    }
}

/// 列表查询参数。
#[derive(Debug, Deserialize)]
pub struct TicketQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub assignee_id: Option<Uuid>,
    /// 只看未分配工单（与 `assignee_id` 同时出现时以后者为准）。
    #[serde(default)]
    pub unassigned: Option<bool>,
    /// 只看“分配给我”的工单（`assignee_id = 当前用户`）。
    #[serde(default)]
    pub mine: Option<bool>,
    /// 只看未结束状态（`open/in_progress`）。
    #[serde(default)]
    pub active: Option<bool>,
    /// 只看超时未结束工单。
    #[serde(default)]
    pub overdue: Option<bool>,
    #[serde(default)]
    pub area_id: Option<Uuid>,
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub keyword: Option<String>,
    /// RFC3339 或 `YYYY-MM-DD`（按 UTC 自然日）。
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

/// 统计查询参数（缺省近 30 天）。
#[derive(Debug, Deserialize)]
pub struct TicketStatsQuery {
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTicketRequest {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<TicketCategory>,
    #[serde(default)]
    pub priority: Option<TicketPriority>,
    #[serde(default)]
    pub room_id: Option<Uuid>,
    #[serde(default)]
    pub area_id: Option<Uuid>,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub device_slot: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTicketRequest {
    #[serde(default)]
    pub title: Option<String>,
    /// 缺失 = 不修改；`null` = 清空。
    #[serde(default, deserialize_with = "super::deserialize_nullable")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub priority: Option<TicketPriority>,
    #[serde(default)]
    pub category: Option<TicketCategory>,
}

#[derive(Debug, Deserialize)]
pub struct AssignTicketRequest {
    /// 缺失 = 非法（400）；显式 `null` = 释放认领。
    #[serde(default, deserialize_with = "super::deserialize_nullable")]
    pub assignee_id: Option<Option<Uuid>>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionTicketRequest {
    pub to_status: TicketStatus,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// 根因分类（resolve/close 时写入；允许 resolved→closed 补充）。
    #[serde(default)]
    pub root_cause: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommentTicketRequest {
    /// 契约字段为 `body`；兼容前端早期写法的 `note`。
    #[serde(alias = "note")]
    pub body: String,
}

const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 10_000;
const MAX_NOTE_CHARS: usize = 2_000;

fn normalize_title(raw: &str) -> AppResult<String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(AppError::bad_request("工单标题不能为空"));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError::bad_request(format!(
            "工单标题不能超过 {MAX_TITLE_CHARS} 个字符"
        )));
    }
    Ok(title.to_owned())
}

fn normalize_description(raw: &str) -> AppResult<String> {
    let description = raw.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(AppError::bad_request(format!(
            "工单描述不能超过 {MAX_DESCRIPTION_CHARS} 个字符"
        )));
    }
    Ok(description.to_owned())
}

fn normalize_note_text(raw: &str, label: &str) -> AppResult<String> {
    let note = raw.trim();
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(AppError::bad_request(format!(
            "{label}不能超过 {MAX_NOTE_CHARS} 个字符"
        )));
    }
    Ok(note.to_owned())
}

fn normalize_serial(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(serial) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if serial.chars().count() > 128 {
        return Err(AppError::bad_request("设备序列号不能超过 128 个字符"));
    }
    Ok(Some(serial.to_owned()))
}

/// 解析时间参数：RFC3339 或 `YYYY-MM-DD`；日期形式按 UTC 自然日边界处理。
fn parse_time_param(raw: Option<&str>, end_of_day: bool) -> AppResult<Option<DateTime<Utc>>> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Ok(instant) = DateTime::parse_from_rfc3339(value) {
        return Ok(Some(instant.with_timezone(&Utc)));
    }
    if let Ok(date) = value.parse::<NaiveDate>() {
        let start = date
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| Utc.from_local_datetime(&naive).single())
            .ok_or_else(|| AppError::bad_request("时间参数不合法"))?;
        return Ok(Some(if end_of_day {
            start + Duration::days(1)
        } else {
            start
        }));
    }
    Err(AppError::bad_request(format!(
        "时间参数需为 RFC3339 或 YYYY-MM-DD：{value}"
    )))
}

fn parse_optional_enum<T: std::str::FromStr<Err = crate::domain::error::DomainError>>(
    raw: Option<&str>,
    label: &str,
) -> AppResult<Option<T>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => value
            .parse::<T>()
            .map(Some)
            .map_err(|_| AppError::bad_request(format!("{label} 取值不合法：{value}"))),
    }
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<TicketQuery>,
) -> AppResult<Json<Page<Ticket>>> {
    authorization::require_permission(&state, id, permissions::TICKET_READ).await?;

    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    let from = parse_time_param(query.from.as_deref(), false)?;
    let to = parse_time_param(query.to.as_deref(), true)?;
    if let (Some(from), Some(to)) = (from, to)
        && from >= to
    {
        return Err(AppError::bad_request("开始时间必须早于结束时间"));
    }

    let filter = TicketFilter {
        status: parse_optional_enum(query.status.as_deref(), "status")?,
        category: parse_optional_enum(query.category.as_deref(), "category")?,
        priority: parse_optional_enum(query.priority.as_deref(), "priority")?,
        source: parse_optional_enum(query.source.as_deref(), "source")?,
        assignee_id: query.assignee_id,
        unassigned: query.unassigned.unwrap_or(false),
        mine: query.mine.unwrap_or(false).then_some(id),
        active: query.active.unwrap_or(false),
        overdue: query.overdue.unwrap_or(false),
        area_id: query.area_id,
        room_id: query.room_id,
        serial: query
            .serial
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
        keyword: (!keyword.is_empty()).then_some(keyword),
        from,
        to,
        scoped_area_ids: scope_area_ids(&state, id).await?,
        page,
        page_size,
    };
    Ok(Json(state.tickets.list(&filter).await?))
}

pub async fn summary(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<TicketSummary>> {
    authorization::require_permission(&state, id, permissions::TICKET_READ).await?;
    let scoped_area_ids = scope_area_ids(&state, id).await?;
    Ok(Json(
        state
            .tickets
            .summary(id, scoped_area_ids.as_deref())
            .await?,
    ))
}

pub async fn stats(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<TicketStatsQuery>,
) -> AppResult<Json<TicketStats>> {
    authorization::require_permission(&state, id, permissions::TICKET_READ).await?;

    let to = parse_time_param(query.to.as_deref(), true)?.unwrap_or_else(Utc::now);
    let from =
        parse_time_param(query.from.as_deref(), false)?.unwrap_or_else(|| to - Duration::days(30));
    if from >= to {
        return Err(AppError::bad_request("开始时间必须早于结束时间"));
    }
    let scoped_area_ids = scope_area_ids(&state, id).await?;
    Ok(Json(
        state
            .tickets
            .stats(from, to, scoped_area_ids.as_deref())
            .await?,
    ))
}

pub async fn assignees(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
) -> AppResult<Json<Vec<crate::domain::ticket::TicketAssignee>>> {
    authorization::require_any_permission(
        &state,
        id,
        &[
            permissions::TICKET_HANDLE,
            permissions::TICKET_ASSIGN,
            permissions::TICKET_MANAGE,
        ],
    )
    .await?;
    Ok(Json(state.tickets.assignees().await?))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateTicketRequest>,
) -> AppResult<(StatusCode, Json<Ticket>)> {
    authorization::require_permission(&state, id, permissions::TICKET_CREATE).await?;

    let title = normalize_title(&request.title)?;
    let description = normalize_description(request.description.as_deref().unwrap_or(""))?;

    // 房间存在时自动带出区域（工单详情/统计都按区域时区聚合）。
    let room = match request.room_id {
        Some(room_id) => Some(
            state
                .rooms
                .get(room_id)
                .await?
                .ok_or_else(|| AppError::not_found("房间不存在"))?,
        ),
        None => None,
    };
    let mut area_id = request.area_id;
    if let Some(room) = room.as_ref() {
        match area_id {
            Some(explicit) if explicit != room.area_id => {
                return Err(AppError::bad_request("房间不属于指定区域"));
            }
            _ => area_id = Some(room.area_id),
        }
    } else if let Some(area) = area_id {
        if state.areas.get(area).await?.is_none() {
            return Err(AppError::not_found("区域不存在"));
        }
    }
    let device_slot = match request.device_slot.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => Some(validation::normalize_device_slot(raw)?),
    };
    let serial = normalize_serial(request.serial.as_deref())?;

    let ticket = state
        .tickets
        .insert_ticket(&NewTicket {
            title: title.clone(),
            description,
            priority: request.priority.unwrap_or(TicketPriority::Normal),
            category: request.category.unwrap_or(TicketCategory::Other),
            source: TicketSource::Manual,
            source_type: None,
            source_id: None,
            dedupe_key: None,
            area_id,
            room_id: request.room_id,
            serial,
            device_slot,
            reporter_id: Some(id),
            metadata: json!({}),
        })
        .await?;

    super::audit::record_or_log(
        &state,
        id,
        "ticket.create",
        "ticket",
        Some(ticket.id.to_string()),
        json!({
            "code": ticket.code,
            "title": ticket.title,
            "priority": ticket.priority.as_str(),
            "category": ticket.category.as_str(),
            "room_id": ticket.room_id,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(ticket)))
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(ticket_id): Path<Uuid>,
) -> AppResult<Json<TicketDetail>> {
    authorization::require_permission(&state, id, permissions::TICKET_READ).await?;

    let scoped_area_ids = scope_area_ids(&state, id).await?;
    let Some((ticket, events)) = state.tickets.get(ticket_id).await? else {
        return Err(AppError::not_found("工单不存在"));
    };
    let area_id = state.tickets.ticket_area(ticket_id).await?;
    if !area_in_scope(&scoped_area_ids, area_id) {
        // 范围外按不存在处理，避免泄露越权信息。
        return Err(AppError::not_found("工单不存在"));
    }
    let context = state.tickets.context(&ticket).await?;
    Ok(Json(TicketDetail {
        ticket,
        events,
        context,
    }))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(ticket_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateTicketRequest>,
) -> AppResult<Json<Ticket>> {
    authorization::require_any_permission(
        &state,
        id,
        &[permissions::TICKET_UPDATE, permissions::TICKET_MANAGE],
    )
    .await?;
    ensure_write_scope(&state, id, ticket_id).await?;

    let title = match request.title.as_deref() {
        Some(raw) => Some(normalize_title(raw)?),
        None => None,
    };
    let description = match request.description {
        Some(Some(raw)) => Some(normalize_description(&raw)?),
        Some(None) => Some(String::new()),
        None => None,
    };
    let ticket = state
        .tickets
        .update_content(
            ticket_id,
            id,
            title.as_deref(),
            description.as_deref(),
            request.priority,
            request.category,
        )
        .await?;

    super::audit::record_or_log(
        &state,
        id,
        // PATCH 内容修改使用独立的审计 action（契约审计动作新增 ticket.update）。
        "ticket.update",
        "ticket",
        Some(ticket.id.to_string()),
        json!({
            "action": "update_content",
            "title": title,
            "priority": request.priority.map(|value| value.as_str()),
            "category": request.category.map(|value| value.as_str()),
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(ticket))
}

pub async fn claim(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(ticket_id): Path<Uuid>,
) -> AppResult<Json<Ticket>> {
    authorization::require_any_permission(
        &state,
        id,
        &[permissions::TICKET_CLAIM, permissions::TICKET_HANDLE],
    )
    .await?;
    ensure_write_scope(&state, id, ticket_id).await?;

    let ticket = state.tickets.claim(ticket_id, id, Utc::now()).await?;
    super::audit::record_or_log(
        &state,
        id,
        "ticket.status",
        "ticket",
        Some(ticket.id.to_string()),
        json!({ "action": "claim", "to_status": ticket.status.as_str() }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(ticket))
}

pub async fn assign(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(ticket_id): Path<Uuid>,
    ApiJson(request): ApiJson<AssignTicketRequest>,
) -> AppResult<Json<Ticket>> {
    authorization::require_any_permission(
        &state,
        id,
        &[permissions::TICKET_ASSIGN, permissions::TICKET_MANAGE],
    )
    .await?;
    ensure_write_scope(&state, id, ticket_id).await?;

    // 空 body / 缺少 assignee_id 不再等价于“释放认领”，必须显式传 null。
    let Some(assignee_id) = request.assignee_id else {
        return Err(AppError::bad_request(
            "assignee_id 必填：传用户 id 表示转派，显式传 null 表示释放认领",
        ));
    };
    let ticket = state
        .tickets
        .assign(ticket_id, assignee_id, id, Utc::now())
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "ticket.assign",
        "ticket",
        Some(ticket.id.to_string()),
        json!({ "assignee_id": assignee_id }),
        ip.as_deref(),
    )
    .await;

    // 转派邮件通知（best-effort，不阻塞响应；无 SMTP 时 EmailService 退化为日志）。
    if state.config.ticket_email_enabled()
        && let Some(assignee_id) = assignee_id
        && let Ok(Some(user)) = state.users.find_by_id(assignee_id).await
        && let Some(email) = user.email.clone()
    {
        let email_service = state.email.clone();
        let code = ticket.code.clone();
        let title = ticket.title.clone();
        let ticket_id = ticket.id.to_string();
        tokio::spawn(async move {
            email_service
                .send_ticket_assigned(&email, &code, &title, &ticket_id)
                .await;
        });
    }

    Ok(Json(ticket))
}

pub async fn transition(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(ticket_id): Path<Uuid>,
    ApiJson(request): ApiJson<TransitionTicketRequest>,
) -> AppResult<Json<Ticket>> {
    // 开始处理（→ in_progress）按“认领”授权；其它流转按“流转”授权，均兼容旧 ticket:handle。
    let required: &[&str] = if request.to_status == TicketStatus::InProgress {
        &[permissions::TICKET_CLAIM, permissions::TICKET_HANDLE]
    } else {
        &[permissions::TICKET_TRANSITION, permissions::TICKET_HANDLE]
    };
    authorization::require_any_permission(&state, id, required).await?;
    ensure_write_scope(&state, id, ticket_id).await?;

    let resolution = request
        .resolution
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let note = match request.note.as_deref() {
        Some(raw) => Some(normalize_note_text(raw, "处理说明")?),
        None => None,
    };
    let root_cause = validate_root_cause(request.root_cause.as_deref())?;
    // 审计需要真实的旧状态；在变更前读取一次（状态机本身仍由仓储事务内校验）。
    let before = state
        .tickets
        .get_ticket(ticket_id)
        .await?
        .ok_or_else(|| AppError::not_found("工单不存在"))?;
    let ticket = state
        .tickets
        .transition(
            ticket_id,
            &TransitionInput {
                to_status: request.to_status,
                resolution,
                note: note.filter(|value| !value.is_empty()),
                root_cause: root_cause.clone(),
            },
            id,
            Utc::now(),
        )
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "ticket.status",
        "ticket",
        Some(ticket.id.to_string()),
        json!({
            "action": "transition",
            "from_status": before.status.as_str(),
            "to_status": request.to_status.as_str(),
            "root_cause": root_cause,
        }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(ticket))
}

pub async fn comments(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(ticket_id): Path<Uuid>,
    ApiJson(request): ApiJson<CommentTicketRequest>,
) -> AppResult<(StatusCode, Json<TicketEvent>)> {
    authorization::require_any_permission(
        &state,
        id,
        &[permissions::TICKET_COMMENT, permissions::TICKET_HANDLE],
    )
    .await?;
    ensure_write_scope(&state, id, ticket_id).await?;

    let body = request.body.trim();
    if body.is_empty() {
        return Err(AppError::bad_request("评论内容不能为空"));
    }
    if body.chars().count() > MAX_NOTE_CHARS {
        return Err(AppError::bad_request(format!(
            "评论内容不能超过 {MAX_NOTE_CHARS} 个字符"
        )));
    }
    let event = state.tickets.add_comment(ticket_id, id, body).await?;
    super::audit::record_or_log(
        &state,
        id,
        "ticket.comment",
        "ticket",
        Some(ticket_id.to_string()),
        json!({ "event_id": event.id }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(event)))
}

// ----------------------------------------------------------------------
// P1：CSV 导出 / SSE 实时流
// ----------------------------------------------------------------------

/// 导出上限（契约 §6）。
const EXPORT_MAX_ROWS: i64 = 5000;
/// SSE 首次补发条数。
const TICKET_STREAM_BACKLOG_LIMIT: i64 = 200;
/// SSE 每轮增量条数。
const TICKET_STREAM_POLL_LIMIT: i64 = 100;
/// SSE 增量查询间隔。
const TICKET_STREAM_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// SSE 心跳间隔。
const TICKET_STREAM_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, Deserialize)]
pub struct TicketStreamQuery {
    /// 只接收 `ticket_events.id > after_event_id` 的事件（断线续传）。
    #[serde(default)]
    pub after_event_id: Option<i64>,
}

/// 导出 CSV：筛选参数与列表一致，不分页，最多 5000 行。
pub async fn export(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<TicketQuery>,
) -> AppResult<Response> {
    authorization::require_any_permission(
        &state,
        id,
        &[permissions::TICKET_EXPORT, permissions::TICKET_READ],
    )
    .await?;

    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    let from = parse_time_param(query.from.as_deref(), false)?;
    let to = parse_time_param(query.to.as_deref(), true)?;
    if let (Some(from), Some(to)) = (from, to)
        && from >= to
    {
        return Err(AppError::bad_request("开始时间必须早于结束时间"));
    }
    let filter = TicketFilter {
        status: parse_optional_enum(query.status.as_deref(), "status")?,
        category: parse_optional_enum(query.category.as_deref(), "category")?,
        priority: parse_optional_enum(query.priority.as_deref(), "priority")?,
        source: parse_optional_enum(query.source.as_deref(), "source")?,
        assignee_id: query.assignee_id,
        unassigned: query.unassigned.unwrap_or(false),
        mine: query.mine.unwrap_or(false).then_some(id),
        active: query.active.unwrap_or(false),
        overdue: query.overdue.unwrap_or(false),
        area_id: query.area_id,
        room_id: query.room_id,
        serial: query
            .serial
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
        keyword: (!keyword.is_empty()).then_some(keyword),
        from,
        to,
        scoped_area_ids: scope_area_ids(&state, id).await?,
        page: 1,
        page_size: 20,
    };
    let rows = state.tickets.export_rows(&filter, EXPORT_MAX_ROWS).await?;
    let csv = tickets_to_csv(&rows);
    let filename = format!("tickets-{}.csv", Utc::now().format("%Y%m%d-%H%M"));

    let mut response = Response::new(axum::body::Body::from(csv.into_bytes()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(value) =
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
    {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

fn tickets_to_csv(rows: &[TicketExportRow]) -> String {
    let mut output = String::from('\u{feff}');
    output.push_str(
        "编号,标题,状态,优先级,分类,来源,区域,房间,设备槽位,序列号,上报人,处理人,创建时间,解决时间,关闭时间,解决说明\n",
    );
    for row in rows {
        let fields = [
            csv_field(&row.code),
            csv_field(&row.title),
            csv_field(row.status.label()),
            csv_field(row.priority.label()),
            csv_field(row.category.label()),
            csv_field(row.source.label()),
            csv_field(row.area_name.as_deref().unwrap_or("")),
            csv_field(row.room_name.as_deref().unwrap_or("")),
            csv_field(row.device_slot.as_deref().unwrap_or("")),
            csv_field(row.serial.as_deref().unwrap_or("")),
            csv_field(row.reporter_name.as_deref().unwrap_or("")),
            csv_field(row.assignee_name.as_deref().unwrap_or("")),
            csv_field(&row.created_at.to_rfc3339()),
            csv_field(
                &row.resolved_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_default(),
            ),
            csv_field(
                &row.closed_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_default(),
            ),
            csv_field(row.resolution.as_deref().unwrap_or("")),
        ];
        output.push_str(&fields.join(","));
        output.push('\n');
    }
    output
}

fn csv_field(value: &str) -> String {
    // 防止 CSV 公式注入：以 = + - @ 或制表符/回车开头的值加前缀单引号。
    let mut value = value.to_owned();
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        value.insert(0, '\'');
    }
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

/// 工单事件 SSE：先补发 `after_event_id` 之后的事件，再每 2 秒查库增量（多副本安全）。
pub async fn stream(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<TicketStreamQuery>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    authorization::require_permission(&state, id, permissions::TICKET_READ).await?;

    let after_id = query.after_event_id.unwrap_or(0);
    if after_id < 0 {
        return Err(AppError::bad_request("after_event_id 不能为负数"));
    }

    let initial = TicketStreamState {
        state,
        last_id: after_id,
        pending: VecDeque::new(),
        backlog_loaded: false,
        last_heartbeat: Instant::now(),
    };
    let stream = stream::unfold(initial, |mut state| async move {
        loop {
            if let Some(event) = state.pending.pop_front() {
                state.last_id = event.id;
                let frame = Event::default()
                    .event("ticket.event")
                    .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned()));
                return Some((Ok(frame), state));
            }

            if !state.backlog_loaded {
                state.backlog_loaded = true;
                match state
                    .state
                    .tickets
                    .stream_events_after(state.last_id, TICKET_STREAM_BACKLOG_LIMIT)
                    .await
                {
                    Ok(events) => {
                        state.enqueue(events);
                        continue;
                    }
                    Err(error) => tracing::warn!(%error, "SSE 补发工单事件失败，稍后重试"),
                }
            }

            tokio::time::sleep(TICKET_STREAM_POLL_INTERVAL).await;

            if state.last_heartbeat.elapsed() >= TICKET_STREAM_HEARTBEAT_INTERVAL {
                state.last_heartbeat = Instant::now();
                return Some((Ok(Event::default().event("heartbeat").data("{}")), state));
            }

            match state
                .state
                .tickets
                .stream_events_after(state.last_id, TICKET_STREAM_POLL_LIMIT)
                .await
            {
                Ok(events) => state.enqueue(events),
                Err(error) => tracing::warn!(%error, "SSE 增量查询工单事件失败"),
            }
        }
    });

    Ok(Sse::new(stream))
}

struct TicketStreamState {
    state: AppState,
    last_id: i64,
    pending: VecDeque<TicketStreamEvent>,
    backlog_loaded: bool,
    last_heartbeat: Instant,
}

impl TicketStreamState {
    fn enqueue(&mut self, mut events: Vec<TicketStreamEvent>) {
        events.sort_by_key(|event| event.id);
        self.pending.extend(events);
    }
}
