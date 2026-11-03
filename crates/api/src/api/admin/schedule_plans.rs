//! 排课全局模型（路径 2 Phase A）管理接口：主数据 + 排课计划 + 发布桥接。
//!
//! 契约见 `docs/design/schedule-plan-implementation-contract.md` §2，挂在
//! `/api/v1/admin`（权限复用 `schedule:read` / `schedule:manage`）：
//!
//! | 方法 | 路径 | 权限 |
//! | --- | --- | --- |
//! | GET/POST | `/terms`、`/teachers`、`/classes`、`/courses` | read / manage |
//! | GET/PATCH/DELETE | `/terms/{id}` 等 | read / manage |
//! | GET/POST | `/schedule-plans` | read / manage |
//! | GET/PATCH/DELETE | `/schedule-plans/{id}` | read / manage |
//! | POST | `/schedule-plans/{id}/clone` | manage |
//! | GET/PUT | `/schedule-plans/{id}/entries` | read / manage |
//! | POST | `/schedule-plans/{id}/publish` | manage |
//!
//! 审计动作：`term.*` / `teacher.*` / `class.*` / `course.*` /
//! `schedule_plan.{create,update,delete,clone,replace_entries,publish}`。
//! 主数据被条目引用 / 已发布计划被删除或修改 → 409；发布后的 legacy 实例重建在
//! 事务外进行，失败只记日志（发布本身已成功）。

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    db::schedule_plan::{DeleteOutcome, PlanDeleteOutcome},
    domain::{
        models::Page,
        rbac::permissions,
        schedule_plan::{
            Class, Course, NewClass, NewCourse, NewPlanEntry, NewTeacher, NewTerm, PlanEntry,
            PlanStatus, SchedulePlan, Teacher, Term, UpdateClass, UpdateCourse, UpdateTeacher,
            UpdateTerm, normalize_color, normalize_email, normalize_metadata, normalize_name,
            normalize_optional_text, validate_plan_entries, validate_term_dates,
        },
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

// ---------------------------------------------------------------------------
// 查询 / 请求 / 响应 DTO
// ---------------------------------------------------------------------------

/// 主数据分页查询参数（学期/教师/班级/课程共用）。
#[derive(Debug, Deserialize)]
pub struct MasterListQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub include_inactive: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTermRequest {
    pub code: String,
    pub name: String,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTermRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub start_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub end_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTeacherRequest {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTeacherRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    /// 缺失 = 不修改；显式 `null` = 重置为 `{}`。
    #[serde(default, deserialize_with = "super::deserialize_nullable")]
    pub metadata: Option<Option<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateClassRequest {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub grade: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClassRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub grade: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    /// 缺失 = 不修改；显式 `null` = 重置为 `{}`。
    #[serde(default, deserialize_with = "super::deserialize_nullable")]
    pub metadata: Option<Option<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCourseRequest {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCourseRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    /// 缺失 = 不修改；显式 `null` = 重置为 `{}`。
    #[serde(default, deserialize_with = "super::deserialize_nullable")]
    pub metadata: Option<Option<Value>>,
}

/// 计划分页查询参数。
#[derive(Debug, Deserialize)]
pub struct PlanListQuery {
    #[serde(default)]
    pub term_id: Option<Uuid>,
    /// `draft` | `published`；非法值 400。
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePlanRequest {
    pub term_id: Uuid,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePlanRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ReplacePlanEntriesRequest {
    #[serde(default)]
    pub entries: Vec<NewPlanEntry>,
}

/// `GET /schedule-plans/{id}`：计划 + 条目（含名称快照）。
#[derive(Debug, Serialize)]
pub struct PlanDetailResponse {
    pub plan: SchedulePlan,
    pub entries: Vec<PlanEntry>,
}

/// `POST /schedule-plans/{id}/publish` 响应（契约 §2.2 的 `PublishResult`）。
#[derive(Debug, Serialize)]
pub struct PublishResultResponse {
    pub plan: SchedulePlan,
    /// 成功重建实例的 legacy 课表数（失败只记日志，不计入）。
    pub schedules_rebuilt: usize,
    /// 写入 legacy 周模板的槽位数。
    pub slots_written: usize,
    /// 重建出的排课实例数。
    pub occurrences: usize,
}

// ---------------------------------------------------------------------------
// 通用辅助
// ---------------------------------------------------------------------------

/// 解析 `status` 查询参数：缺失 / 空白 => `None`；非法值 400。
fn parse_plan_status(raw: Option<&str>) -> AppResult<Option<PlanStatus>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => value
            .parse::<PlanStatus>()
            .map(Some)
            .map_err(AppError::from),
    }
}

/// 分页参数 + 关键词（主数据列表共用）。
fn master_list_params(query: &MasterListQuery) -> AppResult<(i64, i64, String, bool)> {
    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let keyword = validation::normalize_keyword(query.keyword.as_deref())?;
    Ok((
        page,
        page_size,
        keyword,
        query.include_inactive.unwrap_or(false),
    ))
}

/// PATCH 的 metadata 归一：缺失 => 不修改；`null` => `{}`；对象 => 校验。
fn update_metadata(metadata: Option<Option<Value>>) -> AppResult<Option<Value>> {
    match metadata {
        None => Ok(None),
        Some(None) => Ok(Some(json!({}))),
        Some(Some(value)) => Ok(Some(validation::validate_metadata(value)?)),
    }
}

/// 主数据删除结果 → HTTP 状态：不存在 404、被引用 409、成功 204。
fn map_delete_outcome(
    outcome: DeleteOutcome,
    not_found: &str,
    referenced: impl FnOnce(i64) -> String,
) -> AppResult<StatusCode> {
    match outcome {
        DeleteOutcome::Deleted => Ok(StatusCode::NO_CONTENT),
        DeleteOutcome::NotFound => Err(AppError::not_found(not_found)),
        DeleteOutcome::Referenced(count) => Err(AppError::conflict(referenced(count))),
    }
}

// ---------------------------------------------------------------------------
// 学期
// ---------------------------------------------------------------------------

pub async fn term_list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<MasterListQuery>,
) -> AppResult<Json<Page<Term>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let (page, page_size, keyword, include_inactive) = master_list_params(&query)?;
    Ok(Json(
        state
            .terms
            .list(&keyword, include_inactive, page, page_size)
            .await?,
    ))
}

pub async fn term_get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(term_id): Path<Uuid>,
) -> AppResult<Json<Term>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let term = state
        .terms
        .get(term_id)
        .await?
        .ok_or_else(|| AppError::not_found("学期不存在"))?;
    Ok(Json(term))
}

pub async fn term_create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateTermRequest>,
) -> AppResult<(StatusCode, Json<Term>)> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    validate_term_dates(request.start_date, request.end_date)?;
    let input = NewTerm {
        code: validation::normalize_code(&request.code, "学期编码")?,
        name: normalize_name(&request.name, "学期名称")?,
        start_date: request.start_date,
        end_date: request.end_date,
        is_active: request.is_active.unwrap_or(true),
    };
    let term = state.terms.create(&input).await?;
    super::audit::record_or_log(
        &state,
        id,
        "term.create",
        "term",
        Some(term.id.to_string()),
        json!({
            "code": term.code,
            "name": term.name,
            "start_date": term.start_date,
            "end_date": term.end_date,
        }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(term)))
}

pub async fn term_update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(term_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateTermRequest>,
) -> AppResult<Json<Term>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    if request.code.is_none()
        && request.name.is_none()
        && request.start_date.is_none()
        && request.end_date.is_none()
        && request.is_active.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }
    // 只改单个日期时，需结合库里的另一端日期校验区间。
    let existing = state
        .terms
        .get(term_id)
        .await?
        .ok_or_else(|| AppError::not_found("学期不存在"))?;
    validate_term_dates(
        request.start_date.unwrap_or(existing.start_date),
        request.end_date.unwrap_or(existing.end_date),
    )?;

    let input = UpdateTerm {
        code: request
            .code
            .as_deref()
            .map(|code| validation::normalize_code(code, "学期编码"))
            .transpose()?,
        name: request
            .name
            .as_deref()
            .map(|name| normalize_name(name, "学期名称"))
            .transpose()?,
        start_date: request.start_date,
        end_date: request.end_date,
        is_active: request.is_active,
    };
    let term = state
        .terms
        .update(term_id, &input)
        .await?
        .ok_or_else(|| AppError::not_found("学期不存在"))?;
    super::audit::record_or_log(
        &state,
        id,
        "term.update",
        "term",
        Some(term.id.to_string()),
        json!({
            "code": term.code,
            "name": term.name,
            "start_date": term.start_date,
            "end_date": term.end_date,
            "is_active": term.is_active,
        }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(term))
}

pub async fn term_delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(term_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let status = map_delete_outcome(
        state.terms.delete(term_id).await?,
        "学期不存在",
        |count| format!("学期仍被 {count} 个排课计划引用，请先删除这些计划"),
    )?;
    super::audit::record_or_log(
        &state,
        id,
        "term.delete",
        "term",
        Some(term_id.to_string()),
        json!({}),
        ip.as_deref(),
    )
    .await;
    Ok(status)
}

// ---------------------------------------------------------------------------
// 教师
// ---------------------------------------------------------------------------

pub async fn teacher_list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<MasterListQuery>,
) -> AppResult<Json<Page<Teacher>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let (page, page_size, keyword, include_inactive) = master_list_params(&query)?;
    Ok(Json(
        state
            .teachers
            .list(&keyword, include_inactive, page, page_size)
            .await?,
    ))
}

pub async fn teacher_get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(teacher_id): Path<Uuid>,
) -> AppResult<Json<Teacher>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let teacher = state
        .teachers
        .get(teacher_id)
        .await?
        .ok_or_else(|| AppError::not_found("教师不存在"))?;
    Ok(Json(teacher))
}

pub async fn teacher_create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateTeacherRequest>,
) -> AppResult<(StatusCode, Json<Teacher>)> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let input = NewTeacher {
        code: validation::normalize_code(&request.code, "教师编码")?,
        name: normalize_name(&request.name, "教师名称")?,
        phone: normalize_optional_text(request.phone.as_deref().unwrap_or(""), "教师手机号", 32)?,
        email: normalize_email(request.email.as_deref().unwrap_or(""))?,
        is_active: request.is_active.unwrap_or(true),
        metadata: normalize_metadata(request.metadata)?,
    };
    let teacher = state.teachers.create(&input).await?;
    super::audit::record_or_log(
        &state,
        id,
        "teacher.create",
        "teacher",
        Some(teacher.id.to_string()),
        json!({ "code": teacher.code, "name": teacher.name }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(teacher)))
}

pub async fn teacher_update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(teacher_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateTeacherRequest>,
) -> AppResult<Json<Teacher>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    if request.code.is_none()
        && request.name.is_none()
        && request.phone.is_none()
        && request.email.is_none()
        && request.is_active.is_none()
        && request.metadata.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }
    let input = UpdateTeacher {
        code: request
            .code
            .as_deref()
            .map(|code| validation::normalize_code(code, "教师编码"))
            .transpose()?,
        name: request
            .name
            .as_deref()
            .map(|name| normalize_name(name, "教师名称"))
            .transpose()?,
        phone: request
            .phone
            .as_deref()
            .map(|phone| normalize_optional_text(phone, "教师手机号", 32))
            .transpose()?,
        email: request.email.as_deref().map(normalize_email).transpose()?,
        is_active: request.is_active,
        metadata: update_metadata(request.metadata)?,
    };
    let teacher = state
        .teachers
        .update(teacher_id, &input)
        .await?
        .ok_or_else(|| AppError::not_found("教师不存在"))?;
    super::audit::record_or_log(
        &state,
        id,
        "teacher.update",
        "teacher",
        Some(teacher.id.to_string()),
        json!({ "code": teacher.code, "name": teacher.name, "is_active": teacher.is_active }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(teacher))
}

pub async fn teacher_delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(teacher_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let status = map_delete_outcome(
        state.teachers.delete(teacher_id).await?,
        "教师不存在",
        |count| format!("教师仍被 {count} 条计划条目引用，请先移除引用"),
    )?;
    super::audit::record_or_log(
        &state,
        id,
        "teacher.delete",
        "teacher",
        Some(teacher_id.to_string()),
        json!({}),
        ip.as_deref(),
    )
    .await;
    Ok(status)
}

// ---------------------------------------------------------------------------
// 班级
// ---------------------------------------------------------------------------

pub async fn class_list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<MasterListQuery>,
) -> AppResult<Json<Page<Class>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let (page, page_size, keyword, include_inactive) = master_list_params(&query)?;
    Ok(Json(
        state
            .classes
            .list(&keyword, include_inactive, page, page_size)
            .await?,
    ))
}

pub async fn class_get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(class_id): Path<Uuid>,
) -> AppResult<Json<Class>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let class = state
        .classes
        .get(class_id)
        .await?
        .ok_or_else(|| AppError::not_found("班级不存在"))?;
    Ok(Json(class))
}

pub async fn class_create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateClassRequest>,
) -> AppResult<(StatusCode, Json<Class>)> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let input = NewClass {
        code: validation::normalize_code(&request.code, "班级编码")?,
        name: normalize_name(&request.name, "班级名称")?,
        grade: normalize_optional_text(request.grade.as_deref().unwrap_or(""), "年级", 64)?,
        is_active: request.is_active.unwrap_or(true),
        metadata: normalize_metadata(request.metadata)?,
    };
    let class = state.classes.create(&input).await?;
    super::audit::record_or_log(
        &state,
        id,
        "class.create",
        "class",
        Some(class.id.to_string()),
        json!({ "code": class.code, "name": class.name, "grade": class.grade }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(class)))
}

pub async fn class_update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(class_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateClassRequest>,
) -> AppResult<Json<Class>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    if request.code.is_none()
        && request.name.is_none()
        && request.grade.is_none()
        && request.is_active.is_none()
        && request.metadata.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }
    let input = UpdateClass {
        code: request
            .code
            .as_deref()
            .map(|code| validation::normalize_code(code, "班级编码"))
            .transpose()?,
        name: request
            .name
            .as_deref()
            .map(|name| normalize_name(name, "班级名称"))
            .transpose()?,
        grade: request
            .grade
            .as_deref()
            .map(|grade| normalize_optional_text(grade, "年级", 64))
            .transpose()?,
        is_active: request.is_active,
        metadata: update_metadata(request.metadata)?,
    };
    let class = state
        .classes
        .update(class_id, &input)
        .await?
        .ok_or_else(|| AppError::not_found("班级不存在"))?;
    super::audit::record_or_log(
        &state,
        id,
        "class.update",
        "class",
        Some(class.id.to_string()),
        json!({ "code": class.code, "name": class.name, "is_active": class.is_active }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(class))
}

pub async fn class_delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(class_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let status = map_delete_outcome(
        state.classes.delete(class_id).await?,
        "班级不存在",
        |count| format!("班级仍被 {count} 条计划条目引用，请先移除引用"),
    )?;
    super::audit::record_or_log(
        &state,
        id,
        "class.delete",
        "class",
        Some(class_id.to_string()),
        json!({}),
        ip.as_deref(),
    )
    .await;
    Ok(status)
}

// ---------------------------------------------------------------------------
// 课程
// ---------------------------------------------------------------------------

pub async fn course_list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<MasterListQuery>,
) -> AppResult<Json<Page<Course>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let (page, page_size, keyword, include_inactive) = master_list_params(&query)?;
    Ok(Json(
        state
            .courses
            .list(&keyword, include_inactive, page, page_size)
            .await?,
    ))
}

pub async fn course_get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(course_id): Path<Uuid>,
) -> AppResult<Json<Course>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let course = state
        .courses
        .get(course_id)
        .await?
        .ok_or_else(|| AppError::not_found("课程不存在"))?;
    Ok(Json(course))
}

pub async fn course_create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateCourseRequest>,
) -> AppResult<(StatusCode, Json<Course>)> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let input = NewCourse {
        code: validation::normalize_code(&request.code, "课程编码")?,
        name: normalize_name(&request.name, "课程名称")?,
        color: normalize_color(request.color.as_deref().unwrap_or(""))?,
        is_active: request.is_active.unwrap_or(true),
        metadata: normalize_metadata(request.metadata)?,
    };
    let course = state.courses.create(&input).await?;
    super::audit::record_or_log(
        &state,
        id,
        "course.create",
        "course",
        Some(course.id.to_string()),
        json!({ "code": course.code, "name": course.name, "color": course.color }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(course)))
}

pub async fn course_update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(course_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateCourseRequest>,
) -> AppResult<Json<Course>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    if request.code.is_none()
        && request.name.is_none()
        && request.color.is_none()
        && request.is_active.is_none()
        && request.metadata.is_none()
    {
        return Err(AppError::bad_request("没有需要更新的字段"));
    }
    let input = UpdateCourse {
        code: request
            .code
            .as_deref()
            .map(|code| validation::normalize_code(code, "课程编码"))
            .transpose()?,
        name: request
            .name
            .as_deref()
            .map(|name| normalize_name(name, "课程名称"))
            .transpose()?,
        color: request.color.as_deref().map(normalize_color).transpose()?,
        is_active: request.is_active,
        metadata: update_metadata(request.metadata)?,
    };
    let course = state
        .courses
        .update(course_id, &input)
        .await?
        .ok_or_else(|| AppError::not_found("课程不存在"))?;
    super::audit::record_or_log(
        &state,
        id,
        "course.update",
        "course",
        Some(course.id.to_string()),
        json!({ "code": course.code, "name": course.name, "is_active": course.is_active }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(course))
}

pub async fn course_delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(course_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let status = map_delete_outcome(
        state.courses.delete(course_id).await?,
        "课程不存在",
        |count| format!("课程仍被 {count} 条计划条目引用，请先移除引用"),
    )?;
    super::audit::record_or_log(
        &state,
        id,
        "course.delete",
        "course",
        Some(course_id.to_string()),
        json!({}),
        ip.as_deref(),
    )
    .await;
    Ok(status)
}

// ---------------------------------------------------------------------------
// 排课计划
// ---------------------------------------------------------------------------

pub async fn plan_list(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<PlanListQuery>,
) -> AppResult<Json<Page<SchedulePlan>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let (page, page_size) = validation::page_params(query.page, query.page_size)?;
    let status = parse_plan_status(query.status.as_deref())?;
    Ok(Json(
        state
            .schedule_plans
            .list(query.term_id, status, page, page_size)
            .await?,
    ))
}

pub async fn plan_get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(plan_id): Path<Uuid>,
) -> AppResult<Json<PlanDetailResponse>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    let plan = state
        .schedule_plans
        .get(plan_id)
        .await?
        .ok_or_else(|| AppError::not_found("排课计划不存在"))?;
    let entries = state.schedule_plans.entries(plan_id).await?;
    Ok(Json(PlanDetailResponse { plan, entries }))
}

pub async fn plan_create(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreatePlanRequest>,
) -> AppResult<(StatusCode, Json<SchedulePlan>)> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let name = validation::normalize_schedule_name(&request.name)?;
    let plan = state
        .schedule_plans
        .create(request.term_id, &name, Some(id))
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "schedule_plan.create",
        "schedule_plan",
        Some(plan.id.to_string()),
        json!({ "term_id": plan.term_id, "name": plan.name, "version": plan.version }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(plan)))
}

pub async fn plan_update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdatePlanRequest>,
) -> AppResult<Json<SchedulePlan>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let name = validation::normalize_schedule_name(&request.name)?;
    let plan = state
        .schedule_plans
        .update_name(plan_id, &name)
        .await?
        .ok_or_else(|| AppError::not_found("排课计划不存在"))?;
    super::audit::record_or_log(
        &state,
        id,
        "schedule_plan.update",
        "schedule_plan",
        Some(plan.id.to_string()),
        json!({ "name": plan.name }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(plan))
}

pub async fn plan_delete(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    match state.schedule_plans.delete(plan_id).await? {
        PlanDeleteOutcome::Deleted => {
            super::audit::record_or_log(
                &state,
                id,
                "schedule_plan.delete",
                "schedule_plan",
                Some(plan_id.to_string()),
                json!({}),
                ip.as_deref(),
            )
            .await;
            Ok(StatusCode::NO_CONTENT)
        }
        PlanDeleteOutcome::NotFound => Err(AppError::not_found("排课计划不存在")),
        PlanDeleteOutcome::Published => Err(AppError::conflict(
            "已发布的计划不可删除，请保留或先发布其它计划将其降级为草稿",
        )),
    }
}

pub async fn plan_clone(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<SchedulePlan>)> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    let plan = state
        .schedule_plans
        .clone(plan_id, Some(id))
        .await?
        .ok_or_else(|| AppError::not_found("排课计划不存在"))?;
    super::audit::record_or_log(
        &state,
        id,
        "schedule_plan.clone",
        "schedule_plan",
        Some(plan.id.to_string()),
        json!({
            "source_plan_id": plan_id,
            "term_id": plan.term_id,
            "name": plan.name,
            "version": plan.version,
        }),
        ip.as_deref(),
    )
    .await;
    Ok((StatusCode::CREATED, Json(plan)))
}

pub async fn plan_entries(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(plan_id): Path<Uuid>,
) -> AppResult<Json<Vec<PlanEntry>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;
    if state.schedule_plans.get(plan_id).await?.is_none() {
        return Err(AppError::not_found("排课计划不存在"));
    }
    Ok(Json(state.schedule_plans.entries(plan_id).await?))
}

pub async fn plan_replace_entries(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
    ApiJson(request): ApiJson<ReplacePlanEntriesRequest>,
) -> AppResult<Json<Vec<PlanEntry>>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;
    if state.schedule_plans.get(plan_id).await?.is_none() {
        return Err(AppError::not_found("排课计划不存在"));
    }
    // 纯校验（时间/星期/同房间同日不重叠）在领域层；引用房间/课程/教师/班级的
    // 存在性与启用状态由仓储层在同一事务内批量校验。
    let entries = validate_plan_entries(&request.entries)?;
    state
        .schedule_plans
        .replace_entries(plan_id, &entries)
        .await?;
    super::audit::record_or_log(
        &state,
        id,
        "schedule_plan.replace_entries",
        "schedule_plan",
        Some(plan_id.to_string()),
        json!({ "count": entries.len() }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(state.schedule_plans.entries(plan_id).await?))
}

pub async fn plan_publish(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(plan_id): Path<Uuid>,
) -> AppResult<Json<PublishResultResponse>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    // 事务内完成校验 + 桥接写 legacy 课表；重复发布 / 学期停用由仓储层返回 409/400。
    let outcome = state.schedule_plans.publish(plan_id, Some(id)).await?;

    // 事务外重建受影响课表的未来实例：失败只记日志，发布本身已提交。
    let mut schedules_rebuilt = 0usize;
    let mut occurrences = 0usize;
    for schedule_id in &outcome.schedule_ids {
        match state
            .schedules
            .rebuild_occurrences(*schedule_id, state.config.schedule_horizon_days)
            .await
        {
            Ok(count) => {
                schedules_rebuilt += 1;
                occurrences += count;
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    %schedule_id,
                    %plan_id,
                    "发布后重建排课实例失败，可稍后调用 /schedules/{id}/rebuild 补齐"
                );
            }
        }
    }

    super::audit::record_or_log(
        &state,
        id,
        "schedule_plan.publish",
        "schedule_plan",
        Some(plan_id.to_string()),
        json!({
            "plan_id": outcome.plan.id,
            "term_id": outcome.plan.term_id,
            "schedules": outcome.schedule_ids.len(),
            "slots": outcome.slots_written,
            "occurrences": occurrences,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(PublishResultResponse {
        plan: outcome.plan,
        schedules_rebuilt,
        slots_written: outcome.slots_written,
        occurrences,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PATCH 主数据的 metadata：缺失 = 不修改，显式 null = 重置为 `{}`。
    #[test]
    fn update_requests_distinguish_missing_and_null_metadata() {
        let missing: UpdateTeacherRequest = serde_json::from_value(json!({})).unwrap();
        assert!(missing.metadata.is_none());

        let cleared: UpdateTeacherRequest =
            serde_json::from_value(json!({ "metadata": null })).unwrap();
        assert_eq!(cleared.metadata, Some(None));

        let set: UpdateTeacherRequest =
            serde_json::from_value(json!({ "metadata": { "tag": "x" } })).unwrap();
        assert_eq!(set.metadata, Some(Some(json!({ "tag": "x" }))));

        let class_cleared: UpdateClassRequest =
            serde_json::from_value(json!({ "metadata": null })).unwrap();
        assert_eq!(class_cleared.metadata, Some(None));

        let course_missing: UpdateCourseRequest = serde_json::from_value(json!({})).unwrap();
        assert!(course_missing.metadata.is_none());

        assert_eq!(update_metadata(Some(None)).unwrap(), Some(json!({})));
        assert!(update_metadata(Some(Some(json!([1, 2])))).is_err());
    }

    /// 计划状态筛选：缺失 / 空白 = 不过滤；非法值 400。
    #[test]
    fn plan_status_filter_parsing() {
        assert_eq!(parse_plan_status(None).unwrap(), None);
        assert_eq!(parse_plan_status(Some("  ")).unwrap(), None);
        assert_eq!(
            parse_plan_status(Some("published")).unwrap(),
            Some(PlanStatus::Published)
        );
        assert!(parse_plan_status(Some("archived")).is_err());
    }

    /// 条目请求：可选字段可省略，缺省值为 None / 空备注。
    #[test]
    fn replace_entries_request_defaults() {
        let request: ReplacePlanEntriesRequest = serde_json::from_value(json!({
            "entries": [
                {
                    "room_id": "00000000-0000-0000-0000-000000000001",
                    "day_of_week": 1,
                    "start_time": "08:00",
                    "end_time": "09:40",
                    "course_id": "00000000-0000-0000-0000-000000000002"
                }
            ]
        }))
        .unwrap();
        let entry = &request.entries[0];
        assert_eq!(entry.day_of_week, 1);
        assert!(entry.teacher_id.is_none());
        assert!(entry.class_id.is_none());
        assert_eq!(entry.note, "");
    }
}
