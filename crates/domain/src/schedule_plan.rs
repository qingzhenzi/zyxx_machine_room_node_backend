//! 排课全局模型（路径 2 Phase A）：学期 / 教师 / 班级 / 课程 / 排课计划 / 计划条目。
//!
//! 契约见 `docs/design/schedule-plan-implementation-contract.md` §1/§2。本模块只保留
//! 纯模型与输入校验（不依赖 SQLx/HTTP/Redis），数据库行模型与发布事务在 `qe-db`，
//! HTTP 状态码与错误响应在 `qe-api`。
//!
//! 设计要点：
//! - 主数据（[`Term`] / [`Teacher`] / [`Class`] / [`Course`]）用 `code` 唯一标识，
//!   列表额外带 `usage`（被计划条目引用数）便于前端提示删除影响；
//! - 计划（[`SchedulePlan`]）归属一个学期，`draft` 可编辑、`published` 只读；
//! - 条目（[`PlanEntry`]）按「模板周」维护（ISO 星期 + 本地时段），发布时桥接写入
//!   legacy 课表；校验包含「同一房间同一天时段不重叠」。

use std::{collections::BTreeMap, fmt, str::FromStr};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    error::{DomainError, DomainResult},
    validation,
};

/// 单个计划一次全量替换的条目上限（契约 §2.2）。
pub const MAX_PLAN_ENTRIES: usize = 2000;

// ---------------------------------------------------------------------------
// 通用校验
// ---------------------------------------------------------------------------

/// 规范化主数据名称：去空白，长度 1..=200。
pub fn normalize_name(raw: &str, label: &str) -> DomainResult<String> {
    let name = raw.trim().to_owned();
    let length = name.chars().count();
    if !(1..=200).contains(&length) {
        return Err(DomainError::bad_request(format!(
            "{label}长度需为 1-200 个字符"
        )));
    }
    Ok(name)
}

/// 规范化可选文本：去空白，最长 `max` 字符；空字符串保留为空（库列 NOT NULL DEFAULT ''）。
pub fn normalize_optional_text(raw: &str, label: &str, max: usize) -> DomainResult<String> {
    let value = raw.trim().to_owned();
    if value.chars().count() > max {
        return Err(DomainError::bad_request(format!(
            "{label}不能超过 {max} 个字符"
        )));
    }
    Ok(value)
}

/// 规范化教师邮箱：空字符串 => 空字符串；否则校验后转小写。
pub fn normalize_email(raw: &str) -> DomainResult<String> {
    Ok(validation::normalize_optional_email(raw)?.unwrap_or_default())
}

/// 规范化课程色值：空字符串 = 由前端按课程名取色；最长 32 字符。
pub fn normalize_color(raw: &str) -> DomainResult<String> {
    normalize_optional_text(raw, "课程颜色", 32)
}

/// 校验元数据并归一为对象；`None` => `{}`。
pub fn normalize_metadata(metadata: Option<Value>) -> DomainResult<Value> {
    match metadata {
        Some(value) => validation::validate_metadata(value),
        None => Ok(serde_json::json!({})),
    }
}

/// 校验学期日期区间：`start_date <= end_date`。
pub fn validate_term_dates(start_date: NaiveDate, end_date: NaiveDate) -> DomainResult<()> {
    validation::validate_date_range(start_date, end_date)
}

// ---------------------------------------------------------------------------
// 学期
// ---------------------------------------------------------------------------

/// 学期（主数据）。
#[derive(Debug, Clone, Serialize)]
pub struct Term {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_active: bool,
    /// 引用该学期的计划数（列表展示用）。
    pub usage: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTerm {
    pub code: String,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_active: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateTerm {
    pub code: Option<String>,
    pub name: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub is_active: Option<bool>,
}

// ---------------------------------------------------------------------------
// 教师
// ---------------------------------------------------------------------------

/// 教师（轻量主数据）。
#[derive(Debug, Clone, Serialize)]
pub struct Teacher {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub phone: String,
    pub email: String,
    pub is_active: bool,
    pub metadata: Value,
    /// 被计划条目引用数（列表展示用）。
    pub usage: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTeacher {
    pub code: String,
    pub name: String,
    pub phone: String,
    pub email: String,
    pub is_active: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateTeacher {
    pub code: Option<String>,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub is_active: Option<bool>,
    /// `Some({})` = 重置为空对象（接口层把显式 `null` 归一为 `{}`）。
    pub metadata: Option<Value>,
}

// ---------------------------------------------------------------------------
// 班级
// ---------------------------------------------------------------------------

/// 班级（轻量主数据）。
#[derive(Debug, Clone, Serialize)]
pub struct Class {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub grade: String,
    pub is_active: bool,
    pub metadata: Value,
    /// 被计划条目引用数（列表展示用）。
    pub usage: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewClass {
    pub code: String,
    pub name: String,
    pub grade: String,
    pub is_active: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateClass {
    pub code: Option<String>,
    pub name: Option<String>,
    pub grade: Option<String>,
    pub is_active: Option<bool>,
    pub metadata: Option<Value>,
}

// ---------------------------------------------------------------------------
// 课程
// ---------------------------------------------------------------------------

/// 课程（轻量主数据）。
#[derive(Debug, Clone, Serialize)]
pub struct Course {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    /// 前端课程色值（如 `#3a5bd9`）；空 = 前端按课程名取色。
    pub color: String,
    pub is_active: bool,
    pub metadata: Value,
    /// 被计划条目引用数（列表展示用）。
    pub usage: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewCourse {
    pub code: String,
    pub name: String,
    pub color: String,
    pub is_active: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateCourse {
    pub code: Option<String>,
    pub name: Option<String>,
    pub color: Option<String>,
    pub is_active: Option<bool>,
    pub metadata: Option<Value>,
}

// ---------------------------------------------------------------------------
// 排课计划
// ---------------------------------------------------------------------------

/// 计划状态：`draft` 草稿可编辑 / `published` 已发布只读。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Draft,
    Published,
}

impl PlanStatus {
    pub const ALL: [Self; 2] = [Self::Draft, Self::Published];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }

    /// 中文文案（审计 / 提示用）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "草稿",
            Self::Published => "已发布",
        }
    }
}

impl fmt::Display for PlanStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PlanStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "draft" => Ok(Self::Draft),
            "published" => Ok(Self::Published),
            other => Err(DomainError::bad_request(format!(
                "非法的排课计划状态：{other}（可选 draft / published）"
            ))),
        }
    }
}

/// 计划视图（契约 §2.2 的 `PlanView`）。
#[derive(Debug, Clone, Serialize)]
pub struct SchedulePlan {
    pub id: Uuid,
    pub term_id: Uuid,
    pub term_name: String,
    pub name: String,
    pub status: PlanStatus,
    pub version: i32,
    pub published_at: Option<DateTime<Utc>>,
    pub published_by: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub entry_count: i64,
    pub course_count: i64,
    pub room_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 计划条目视图（契约 §2.2 的 `PlanEntry`，含房间/课程/教师/班级名称快照）。
#[derive(Debug, Clone, Serialize)]
pub struct PlanEntry {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub room_id: Uuid,
    pub room_code: String,
    pub room_name: String,
    pub area_id: Uuid,
    pub area_name: String,
    pub day_of_week: i16,
    pub start_time: String,
    pub end_time: String,
    pub course_id: Uuid,
    pub course_code: String,
    pub course_name: String,
    pub teacher_id: Option<Uuid>,
    pub teacher_name: Option<String>,
    pub class_id: Option<Uuid>,
    pub class_name: Option<String>,
    pub note: String,
}

/// 条目全量替换的输入（契约 §2.2 的 `NewPlanEntry`）。
#[derive(Debug, Clone, Deserialize)]
pub struct NewPlanEntry {
    pub room_id: Uuid,
    pub day_of_week: i16,
    pub start_time: String,
    pub end_time: String,
    pub course_id: Uuid,
    #[serde(default)]
    pub teacher_id: Option<Uuid>,
    #[serde(default)]
    pub class_id: Option<Uuid>,
    #[serde(default)]
    pub note: String,
}

/// 校验后的条目（时间已解析，可直接写库）。
#[derive(Debug, Clone)]
pub struct ValidatedPlanEntry {
    pub room_id: Uuid,
    pub day_of_week: i16,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub course_id: Uuid,
    pub teacher_id: Option<Uuid>,
    pub class_id: Option<Uuid>,
    pub note: String,
}

/// 校验并规范化一批条目：数量 ≤ [`MAX_PLAN_ENTRIES`]、星期/时间合法、
/// 备注规范化，且**同一房间同一天时段不得重叠**。
pub fn validate_plan_entries(entries: &[NewPlanEntry]) -> DomainResult<Vec<ValidatedPlanEntry>> {
    if entries.len() > MAX_PLAN_ENTRIES {
        return Err(DomainError::bad_request(format!(
            "单个计划最多 {MAX_PLAN_ENTRIES} 条条目"
        )));
    }

    let mut validated = Vec::with_capacity(entries.len());
    for entry in entries {
        if !(1..=7).contains(&entry.day_of_week) {
            return Err(DomainError::bad_request(
                "星期需为 1-7（ISO：周一=1 .. 周日=7）",
            ));
        }
        let start_time = validation::parse_time(&entry.start_time)?;
        let end_time = validation::parse_time(&entry.end_time)?;
        if start_time >= end_time {
            return Err(DomainError::bad_request("开始时间必须早于结束时间"));
        }
        validated.push(ValidatedPlanEntry {
            room_id: entry.room_id,
            day_of_week: entry.day_of_week,
            start_time,
            end_time,
            course_id: entry.course_id,
            teacher_id: entry.teacher_id,
            class_id: entry.class_id,
            note: validation::normalize_note(&entry.note)?,
        });
    }

    check_room_time_overlaps(&validated)?;
    Ok(validated)
}

/// 同一房间同一天的时段不得重叠（含完全相同的时间段）。
///
/// 按 `(room_id, day_of_week)` 分组后按开始时间排序，相邻条目 `prev.end > next.start`
/// 即视为重叠；空条目与单条目直接通过。
pub fn check_room_time_overlaps(entries: &[ValidatedPlanEntry]) -> DomainResult<()> {
    let mut groups: BTreeMap<(Uuid, i16), Vec<(NaiveTime, NaiveTime)>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry((entry.room_id, entry.day_of_week))
            .or_default()
            .push((entry.start_time, entry.end_time));
    }
    for ((_, day_of_week), mut windows) in groups {
        if windows.len() < 2 {
            continue;
        }
        windows.sort_unstable();
        for pair in windows.windows(2) {
            let (previous_start, previous_end) = pair[0];
            let (next_start, next_end) = pair[1];
            if previous_end > next_start {
                return Err(DomainError::bad_request(format!(
                    "同一房间同一天（周{day_of_week}）的时段重叠：{}~{} 与 {}~{}",
                    format_hh_mm(previous_start),
                    format_hh_mm(previous_end),
                    format_hh_mm(next_start),
                    format_hh_mm(next_end),
                )));
            }
        }
    }
    Ok(())
}

/// `HH:MM` 文案（校验错误信息用；入库时间已在 db 层格式化）。
fn format_hh_mm(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(room_id: Uuid, day: i16, start: &str, end: &str) -> NewPlanEntry {
        NewPlanEntry {
            room_id,
            day_of_week: day,
            start_time: start.to_owned(),
            end_time: end.to_owned(),
            course_id: Uuid::nil(),
            teacher_id: None,
            class_id: None,
            note: String::new(),
        }
    }

    #[test]
    fn plan_status_round_trips() {
        assert_eq!("draft".parse::<PlanStatus>().unwrap(), PlanStatus::Draft);
        assert_eq!(
            "published".parse::<PlanStatus>().unwrap(),
            PlanStatus::Published
        );
        assert!("archived".parse::<PlanStatus>().is_err());
        assert_eq!(PlanStatus::Published.as_str(), "published");
    }

    /// 同一房间同一天重叠 → 400；不同房间 / 不同天 / 首尾相接不算重叠。
    #[test]
    fn entries_overlap_is_rejected_per_room_and_day() {
        let room_a = Uuid::new_v4();
        let room_b = Uuid::new_v4();

        // 首尾相接（08:00-09:40 + 09:40-11:20）合法。
        let adjacent = vec![
            entry(room_a, 1, "08:00", "09:40"),
            entry(room_a, 1, "09:40", "11:20"),
        ];
        assert!(validate_plan_entries(&adjacent).is_ok());

        // 不同房间 / 不同星期的相同时间合法。
        let other_scope = vec![
            entry(room_a, 1, "08:00", "09:40"),
            entry(room_b, 1, "08:00", "09:40"),
            entry(room_a, 2, "08:00", "09:40"),
        ];
        assert!(validate_plan_entries(&other_scope).is_ok());

        let overlap = vec![
            entry(room_a, 1, "08:00", "10:00"),
            entry(room_a, 1, "09:30", "11:00"),
        ];
        let error = validate_plan_entries(&overlap).expect_err("重叠应被拒绝");
        assert!(error.to_string().contains("时段重叠"));
    }

    /// 时间 / 星期非法 → 400；条目数与备注超限 → 400。
    #[test]
    fn entries_validate_time_and_limits() {
        let room = Uuid::new_v4();
        assert!(validate_plan_entries(&[entry(room, 0, "08:00", "09:00")]).is_err());
        assert!(validate_plan_entries(&[entry(room, 8, "08:00", "09:00")]).is_err());
        assert!(validate_plan_entries(&[entry(room, 1, "09:00", "08:00")]).is_err());
        assert!(validate_plan_entries(&[entry(room, 1, "25:00", "26:00")]).is_err());
        assert!(validate_plan_entries(&[]).is_ok());
    }

    #[test]
    fn term_dates_and_metadata_are_validated() {
        let start = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2027, 1, 31).unwrap();
        assert!(validate_term_dates(start, end).is_ok());
        assert!(validate_term_dates(end, start).is_err());
        assert_eq!(normalize_metadata(None).unwrap(), serde_json::json!({}));
        assert!(normalize_metadata(Some(serde_json::json!([1]))).is_err());
        assert_eq!(
            normalize_email(" Teacher@Example.COM ").unwrap(),
            "teacher@example.com"
        );
        assert_eq!(normalize_email(" ").unwrap(), "");
    }
}
