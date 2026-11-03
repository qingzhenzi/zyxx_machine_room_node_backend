//! 排课全局模型（路径 2 Phase A）仓储：学期 / 教师 / 班级 / 课程 / 排课计划 / 计划条目。
//!
//! 契约见 `docs/design/schedule-plan-implementation-contract.md` §1/§2.3，
//! 表结构见 `migrations/0020_schedule_plan.sql`。
//!
//! 设计要点：
//! - 主数据（[`TermRepo`] / [`TeacherRepo`] / [`ClassRepo`] / [`CourseRepo`]）用 `code`
//!   唯一标识；列表带 `usage`（被计划条目引用数）。删除被引用主数据返回
//!   [`DeleteOutcome::Referenced`]，由 API 映射 409（外键 RESTRICT 是最后防线）。
//! - 计划（[`PlanRepo`]）：`draft` 可编辑、`published` 只读；克隆任意状态可克隆，
//!   `version` 取同学期最大值 +1。
//! - 发布（[`PlanRepo::publish`]）在**一个事务**内：锁 plan 行 → 校验学期启用 →
//!   按房间分组条目、找/建 legacy `schedules`（`plan_id` 标记来源、起止 = 学期）→
//!   全量替换 `schedule_weekly_slots`（课程/教师名称快照、`room_id = NULL`）→
//!   清空本计划此前有、现在无条目的房间槽位 → 同 term 其它 published 降级 draft →
//!   更新计划状态。事务提交后由 API 层复用 `ScheduleRepo::rebuild_occurrences` 重建实例。
//! - 全部使用运行时 `sqlx` 查询（不依赖编译期 DATABASE_URL）。

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::models::Page;
use qe_domain::schedule_plan::{
    Class, Course, NewClass, NewCourse, NewTeacher, NewTerm, PlanEntry, PlanStatus, SchedulePlan,
    Teacher, Term, UpdateClass, UpdateCourse, UpdateTeacher, UpdateTerm, ValidatedPlanEntry,
};

use crate::error::{DbError, DbResult};

/// `TIME` 列对外统一输出 `"HH:MM"`。
fn format_time(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

/// 主数据删除结果：`Referenced(n)` 由 API 映射 409。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
    /// 被引用次数；> 0 时没有删除。
    Referenced(i64),
}

/// 计划删除结果：已发布计划不允许删除（409）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanDeleteOutcome {
    Deleted,
    NotFound,
    Published,
}

/// 发布事务的产出：发布后的计划视图 + 受影响的 legacy 课表（事务外重建实例用）。
#[derive(Debug, Clone)]
pub struct PublishOutcome {
    /// 发布后的计划视图（`status = published`）。
    pub plan: SchedulePlan,
    /// 本次发布影响的 legacy 课表 id（新建 / 更新 / 清空周模板，去重）。
    pub schedule_ids: Vec<Uuid>,
    /// 写入 legacy 周模板的槽位数。
    pub slots_written: usize,
}

// ---------------------------------------------------------------------------
// 行模型与映射
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
struct TermRow {
    id: Uuid,
    code: String,
    name: String,
    start_date: NaiveDate,
    end_date: NaiveDate,
    is_active: bool,
    usage: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TermRow> for Term {
    fn from(row: TermRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            start_date: row.start_date,
            end_date: row.end_date,
            is_active: row.is_active,
            usage: row.usage,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct TeacherRow {
    id: Uuid,
    code: String,
    name: String,
    phone: String,
    email: String,
    is_active: bool,
    metadata: Value,
    usage: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TeacherRow> for Teacher {
    fn from(row: TeacherRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            phone: row.phone,
            email: row.email,
            is_active: row.is_active,
            metadata: row.metadata,
            usage: row.usage,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ClassRow {
    id: Uuid,
    code: String,
    name: String,
    grade: String,
    is_active: bool,
    metadata: Value,
    usage: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ClassRow> for Class {
    fn from(row: ClassRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            grade: row.grade,
            is_active: row.is_active,
            metadata: row.metadata,
            usage: row.usage,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct CourseRow {
    id: Uuid,
    code: String,
    name: String,
    color: String,
    is_active: bool,
    metadata: Value,
    usage: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<CourseRow> for Course {
    fn from(row: CourseRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            color: row.color,
            is_active: row.is_active,
            metadata: row.metadata,
            usage: row.usage,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct PlanRow {
    id: Uuid,
    term_id: Uuid,
    term_name: String,
    name: String,
    status: String,
    version: i32,
    published_at: Option<DateTime<Utc>>,
    published_by: Option<Uuid>,
    created_by: Option<Uuid>,
    entry_count: i64,
    course_count: i64,
    room_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<PlanRow> for SchedulePlan {
    fn from(row: PlanRow) -> Self {
        Self {
            id: row.id,
            term_id: row.term_id,
            term_name: row.term_name,
            name: row.name,
            // 数据库 CHECK 约束保证枚举合法；解析失败退化为 draft 而不是 panic。
            status: row.status.parse().unwrap_or(PlanStatus::Draft),
            version: row.version,
            published_at: row.published_at,
            published_by: row.published_by,
            created_by: row.created_by,
            entry_count: row.entry_count,
            course_count: row.course_count,
            room_count: row.room_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct EntryRow {
    id: Uuid,
    plan_id: Uuid,
    room_id: Uuid,
    room_code: String,
    room_name: String,
    area_id: Uuid,
    area_name: String,
    day_of_week: i16,
    start_time: NaiveTime,
    end_time: NaiveTime,
    course_id: Uuid,
    course_code: String,
    course_name: String,
    teacher_id: Option<Uuid>,
    teacher_name: Option<String>,
    class_id: Option<Uuid>,
    class_name: Option<String>,
    note: String,
}

impl From<EntryRow> for PlanEntry {
    fn from(row: EntryRow) -> Self {
        Self {
            id: row.id,
            plan_id: row.plan_id,
            room_id: row.room_id,
            room_code: row.room_code,
            room_name: row.room_name,
            area_id: row.area_id,
            area_name: row.area_name,
            day_of_week: row.day_of_week,
            start_time: format_time(row.start_time),
            end_time: format_time(row.end_time),
            course_id: row.course_id,
            course_code: row.course_code,
            course_name: row.course_name,
            teacher_id: row.teacher_id,
            teacher_name: row.teacher_name,
            class_id: row.class_id,
            class_name: row.class_name,
            note: row.note,
        }
    }
}

// ---------------------------------------------------------------------------
// 通用 SQL 片段
// ---------------------------------------------------------------------------

const TERM_COLUMNS: &str = "tm.id, tm.code, tm.name, tm.start_date, tm.end_date, tm.is_active,
     (SELECT count(*) FROM schedule_plans p WHERE p.term_id = tm.id) AS usage,
     tm.created_at, tm.updated_at";

const TEACHER_COLUMNS: &str = "t.id, t.code, t.name, t.phone, t.email, t.is_active, t.metadata,
     (SELECT count(*) FROM plan_entries e WHERE e.teacher_id = t.id) AS usage,
     t.created_at, t.updated_at";

const CLASS_COLUMNS: &str = "cl.id, cl.code, cl.name, cl.grade, cl.is_active, cl.metadata,
     (SELECT count(*) FROM plan_entries e WHERE e.class_id = cl.id) AS usage,
     cl.created_at, cl.updated_at";

const COURSE_COLUMNS: &str = "co.id, co.code, co.name, co.color, co.is_active, co.metadata,
     (SELECT count(*) FROM plan_entries e WHERE e.course_id = co.id) AS usage,
     co.created_at, co.updated_at";

const PLAN_COLUMNS: &str = "p.id, p.term_id, tm.name AS term_name, p.name, p.status, p.version,
     p.published_at, p.published_by, p.created_by,
     (SELECT count(*) FROM plan_entries e WHERE e.plan_id = p.id) AS entry_count,
     (SELECT count(DISTINCT e.course_id) FROM plan_entries e WHERE e.plan_id = p.id) AS course_count,
     (SELECT count(DISTINCT e.room_id) FROM plan_entries e WHERE e.plan_id = p.id) AS room_count,
     p.created_at, p.updated_at";

const PLAN_FROM: &str = "FROM schedule_plans p
     JOIN terms tm ON tm.id = p.term_id";

const ENTRY_COLUMNS: &str = "e.id, e.plan_id, e.room_id, r.code AS room_code, r.name AS room_name,
     r.area_id, a.name AS area_name, e.day_of_week, e.start_time, e.end_time,
     e.course_id, co.code AS course_code, co.name AS course_name,
     e.teacher_id, t.name AS teacher_name, e.class_id, cl.name AS class_name, e.note";

const ENTRY_FROM: &str = "FROM plan_entries e
     JOIN rooms r ON r.id = e.room_id
     JOIN areas a ON a.id = r.area_id
     JOIN courses co ON co.id = e.course_id
     LEFT JOIN teachers t ON t.id = e.teacher_id
     LEFT JOIN classes cl ON cl.id = e.class_id";

/// 删除被引用主数据的通用流程：先数引用，被引用则不删；删除影响 0 行视为不存在。
async fn delete_master_row(
    db: &PgPool,
    delete_sql: &str,
    reference_sql: &str,
    id: Uuid,
) -> DbResult<DeleteOutcome> {
    let references: i64 = sqlx::query_scalar(reference_sql)
        .bind(id)
        .fetch_one(db)
        .await?;
    if references > 0 {
        return Ok(DeleteOutcome::Referenced(references));
    }
    let deleted = sqlx::query(delete_sql)
        .bind(id)
        .execute(db)
        .await?
        .rows_affected();
    Ok(if deleted > 0 {
        DeleteOutcome::Deleted
    } else {
        DeleteOutcome::NotFound
    })
}

// ---------------------------------------------------------------------------
// 学期
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TermRepo {
    db: PgPool,
}

impl TermRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        keyword: &str,
        include_inactive: bool,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Term>> {
        let offset = (page - 1) * page_size;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM terms tm
             WHERE ($1 = '' OR tm.code ILIKE '%' || $1 || '%' OR tm.name ILIKE '%' || $1 || '%')
               AND ($2 OR tm.is_active)",
        )
        .bind(keyword)
        .bind(include_inactive)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {TERM_COLUMNS}
             FROM terms tm
             WHERE ($1 = '' OR tm.code ILIKE '%' || $1 || '%' OR tm.name ILIKE '%' || $1 || '%')
               AND ($2 OR tm.is_active)
             ORDER BY tm.start_date DESC, tm.code
             LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query_as::<_, TermRow>(&sql)
            .bind(keyword)
            .bind(include_inactive)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;
        Ok(Page::new(
            rows.into_iter().map(Term::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<Term>> {
        let sql = format!("SELECT {TERM_COLUMNS} FROM terms tm WHERE tm.id = $1");
        let row = sqlx::query_as::<_, TermRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Term::from))
    }

    /// 创建学期；`code` 唯一冲突由数据库报 `Conflict`（409）。
    pub async fn create(&self, input: &NewTerm) -> DbResult<Term> {
        let sql = format!(
            "WITH inserted AS (
                INSERT INTO terms (code, name, start_date, end_date, is_active)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING *
             )
             SELECT {TERM_COLUMNS} FROM inserted tm"
        );
        let row = sqlx::query_as::<_, TermRow>(&sql)
            .bind(&input.code)
            .bind(&input.name)
            .bind(input.start_date)
            .bind(input.end_date)
            .bind(input.is_active)
            .fetch_one(&self.db)
            .await?;
        Ok(Term::from(row))
    }

    pub async fn update(&self, id: Uuid, input: &UpdateTerm) -> DbResult<Option<Term>> {
        let sql = format!(
            "WITH updated AS (
                UPDATE terms SET
                    code = COALESCE($2, code),
                    name = COALESCE($3, name),
                    start_date = COALESCE($4, start_date),
                    end_date = COALESCE($5, end_date),
                    is_active = COALESCE($6, is_active)
                WHERE id = $1
                RETURNING *
             )
             SELECT {TERM_COLUMNS} FROM updated tm"
        );
        let row = sqlx::query_as::<_, TermRow>(&sql)
            .bind(id)
            .bind(&input.code)
            .bind(&input.name)
            .bind(input.start_date)
            .bind(input.end_date)
            .bind(input.is_active)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Term::from))
    }

    /// 删除学期；有计划引用时返回 [`DeleteOutcome::Referenced`]（API 映射 409）。
    pub async fn delete(&self, id: Uuid) -> DbResult<DeleteOutcome> {
        delete_master_row(
            &self.db,
            "DELETE FROM terms WHERE id = $1",
            "SELECT count(*) FROM schedule_plans WHERE term_id = $1",
            id,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// 教师
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TeacherRepo {
    db: PgPool,
}

impl TeacherRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        keyword: &str,
        include_inactive: bool,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Teacher>> {
        let offset = (page - 1) * page_size;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM teachers t
             WHERE ($1 = '' OR t.code ILIKE '%' || $1 || '%' OR t.name ILIKE '%' || $1 || '%')
               AND ($2 OR t.is_active)",
        )
        .bind(keyword)
        .bind(include_inactive)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {TEACHER_COLUMNS}
             FROM teachers t
             WHERE ($1 = '' OR t.code ILIKE '%' || $1 || '%' OR t.name ILIKE '%' || $1 || '%')
               AND ($2 OR t.is_active)
             ORDER BY t.code
             LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query_as::<_, TeacherRow>(&sql)
            .bind(keyword)
            .bind(include_inactive)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;
        Ok(Page::new(
            rows.into_iter().map(Teacher::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<Teacher>> {
        let sql = format!("SELECT {TEACHER_COLUMNS} FROM teachers t WHERE t.id = $1");
        let row = sqlx::query_as::<_, TeacherRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Teacher::from))
    }

    pub async fn create(&self, input: &NewTeacher) -> DbResult<Teacher> {
        let sql = format!(
            "WITH inserted AS (
                INSERT INTO teachers (code, name, phone, email, is_active, metadata)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING *
             )
             SELECT {TEACHER_COLUMNS} FROM inserted t"
        );
        let row = sqlx::query_as::<_, TeacherRow>(&sql)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.phone)
            .bind(&input.email)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_one(&self.db)
            .await?;
        Ok(Teacher::from(row))
    }

    pub async fn update(&self, id: Uuid, input: &UpdateTeacher) -> DbResult<Option<Teacher>> {
        let sql = format!(
            "WITH updated AS (
                UPDATE teachers SET
                    code = COALESCE($2, code),
                    name = COALESCE($3, name),
                    phone = COALESCE($4, phone),
                    email = COALESCE($5, email),
                    is_active = COALESCE($6, is_active),
                    metadata = COALESCE($7, metadata)
                WHERE id = $1
                RETURNING *
             )
             SELECT {TEACHER_COLUMNS} FROM updated t"
        );
        let row = sqlx::query_as::<_, TeacherRow>(&sql)
            .bind(id)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.phone)
            .bind(&input.email)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Teacher::from))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<DeleteOutcome> {
        delete_master_row(
            &self.db,
            "DELETE FROM teachers WHERE id = $1",
            "SELECT count(*) FROM plan_entries WHERE teacher_id = $1",
            id,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// 班级
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ClassRepo {
    db: PgPool,
}

impl ClassRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        keyword: &str,
        include_inactive: bool,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Class>> {
        let offset = (page - 1) * page_size;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM classes cl
             WHERE ($1 = '' OR cl.code ILIKE '%' || $1 || '%' OR cl.name ILIKE '%' || $1 || '%')
               AND ($2 OR cl.is_active)",
        )
        .bind(keyword)
        .bind(include_inactive)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {CLASS_COLUMNS}
             FROM classes cl
             WHERE ($1 = '' OR cl.code ILIKE '%' || $1 || '%' OR cl.name ILIKE '%' || $1 || '%')
               AND ($2 OR cl.is_active)
             ORDER BY cl.code
             LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query_as::<_, ClassRow>(&sql)
            .bind(keyword)
            .bind(include_inactive)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;
        Ok(Page::new(
            rows.into_iter().map(Class::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<Class>> {
        let sql = format!("SELECT {CLASS_COLUMNS} FROM classes cl WHERE cl.id = $1");
        let row = sqlx::query_as::<_, ClassRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Class::from))
    }

    pub async fn create(&self, input: &NewClass) -> DbResult<Class> {
        let sql = format!(
            "WITH inserted AS (
                INSERT INTO classes (code, name, grade, is_active, metadata)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING *
             )
             SELECT {CLASS_COLUMNS} FROM inserted cl"
        );
        let row = sqlx::query_as::<_, ClassRow>(&sql)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.grade)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_one(&self.db)
            .await?;
        Ok(Class::from(row))
    }

    pub async fn update(&self, id: Uuid, input: &UpdateClass) -> DbResult<Option<Class>> {
        let sql = format!(
            "WITH updated AS (
                UPDATE classes SET
                    code = COALESCE($2, code),
                    name = COALESCE($3, name),
                    grade = COALESCE($4, grade),
                    is_active = COALESCE($5, is_active),
                    metadata = COALESCE($6, metadata)
                WHERE id = $1
                RETURNING *
             )
             SELECT {CLASS_COLUMNS} FROM updated cl"
        );
        let row = sqlx::query_as::<_, ClassRow>(&sql)
            .bind(id)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.grade)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Class::from))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<DeleteOutcome> {
        delete_master_row(
            &self.db,
            "DELETE FROM classes WHERE id = $1",
            "SELECT count(*) FROM plan_entries WHERE class_id = $1",
            id,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// 课程
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CourseRepo {
    db: PgPool,
}

impl CourseRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        keyword: &str,
        include_inactive: bool,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<Course>> {
        let offset = (page - 1) * page_size;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM courses co
             WHERE ($1 = '' OR co.code ILIKE '%' || $1 || '%' OR co.name ILIKE '%' || $1 || '%')
               AND ($2 OR co.is_active)",
        )
        .bind(keyword)
        .bind(include_inactive)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {COURSE_COLUMNS}
             FROM courses co
             WHERE ($1 = '' OR co.code ILIKE '%' || $1 || '%' OR co.name ILIKE '%' || $1 || '%')
               AND ($2 OR co.is_active)
             ORDER BY co.code
             LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query_as::<_, CourseRow>(&sql)
            .bind(keyword)
            .bind(include_inactive)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;
        Ok(Page::new(
            rows.into_iter().map(Course::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<Course>> {
        let sql = format!("SELECT {COURSE_COLUMNS} FROM courses co WHERE co.id = $1");
        let row = sqlx::query_as::<_, CourseRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Course::from))
    }

    pub async fn create(&self, input: &NewCourse) -> DbResult<Course> {
        let sql = format!(
            "WITH inserted AS (
                INSERT INTO courses (code, name, color, is_active, metadata)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING *
             )
             SELECT {COURSE_COLUMNS} FROM inserted co"
        );
        let row = sqlx::query_as::<_, CourseRow>(&sql)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.color)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_one(&self.db)
            .await?;
        Ok(Course::from(row))
    }

    pub async fn update(&self, id: Uuid, input: &UpdateCourse) -> DbResult<Option<Course>> {
        let sql = format!(
            "WITH updated AS (
                UPDATE courses SET
                    code = COALESCE($2, code),
                    name = COALESCE($3, name),
                    color = COALESCE($4, color),
                    is_active = COALESCE($5, is_active),
                    metadata = COALESCE($6, metadata)
                WHERE id = $1
                RETURNING *
             )
             SELECT {COURSE_COLUMNS} FROM updated co"
        );
        let row = sqlx::query_as::<_, CourseRow>(&sql)
            .bind(id)
            .bind(&input.code)
            .bind(&input.name)
            .bind(&input.color)
            .bind(input.is_active)
            .bind(&input.metadata)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(Course::from))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<DeleteOutcome> {
        delete_master_row(
            &self.db,
            "DELETE FROM courses WHERE id = $1",
            "SELECT count(*) FROM plan_entries WHERE course_id = $1",
            id,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// 排课计划与条目
// ---------------------------------------------------------------------------

/// 发布时读取的条目（含课程/教师名称快照，桥接 legacy 周模板用）。
#[derive(Debug, Clone, FromRow)]
struct PublishEntryRow {
    room_id: Uuid,
    day_of_week: i16,
    start_time: NaiveTime,
    end_time: NaiveTime,
    course_name: String,
    teacher_name: Option<String>,
    note: String,
}

#[derive(Clone)]
pub struct PlanRepo {
    db: PgPool,
}

impl PlanRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn list(
        &self,
        term_id: Option<Uuid>,
        status: Option<PlanStatus>,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<SchedulePlan>> {
        let offset = (page - 1) * page_size;
        let status_text = status.map(PlanStatus::as_str);

        let total: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM schedule_plans p
             WHERE ($1::uuid IS NULL OR p.term_id = $1)
               AND ($2::text IS NULL OR p.status = $2)",
        )
        .bind(term_id)
        .bind(status_text)
        .fetch_one(&self.db)
        .await?;

        let sql = format!(
            "SELECT {PLAN_COLUMNS}
             {PLAN_FROM}
             WHERE ($1::uuid IS NULL OR p.term_id = $1)
               AND ($2::text IS NULL OR p.status = $2)
             ORDER BY p.created_at DESC, p.id
             LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query_as::<_, PlanRow>(&sql)
            .bind(term_id)
            .bind(status_text)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;
        Ok(Page::new(
            rows.into_iter().map(SchedulePlan::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<SchedulePlan>> {
        let sql = format!("SELECT {PLAN_COLUMNS} {PLAN_FROM} WHERE p.id = $1");
        let row = sqlx::query_as::<_, PlanRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(SchedulePlan::from))
    }

    /// 创建草稿计划（`version = 1`）；学期必须存在且启用。
    pub async fn create(
        &self,
        term_id: Uuid,
        name: &str,
        created_by: Option<Uuid>,
    ) -> DbResult<SchedulePlan> {
        let mut tx = self.db.begin().await?;
        let term = sqlx::query_as::<_, (bool,)>("SELECT is_active FROM terms WHERE id = $1")
            .bind(term_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some((is_active,)) = term else {
            tx.rollback().await?;
            return Err(DbError::not_found("学期不存在"));
        };
        if !is_active {
            tx.rollback().await?;
            return Err(DbError::bad_request("学期已停用，无法创建计划"));
        }
        let plan_id: Uuid = sqlx::query_scalar(
            "INSERT INTO schedule_plans (term_id, name, created_by)
             VALUES ($1, $2, $3)
             RETURNING id",
        )
        .bind(term_id)
        .bind(name)
        .bind(created_by)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get(plan_id)
            .await?
            .ok_or_else(|| DbError::internal("创建排课计划后读取失败"))
    }

    /// 重命名计划；只有草稿可改（已发布 → `Conflict`，API 映射 409）。
    pub async fn update_name(&self, id: Uuid, name: &str) -> DbResult<Option<SchedulePlan>> {
        let mut tx = self.db.begin().await?;
        let status =
            sqlx::query_scalar::<_, String>("SELECT status FROM schedule_plans WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(status) = status else {
            tx.rollback().await?;
            return Ok(None);
        };
        if status == PlanStatus::Published.as_str() {
            tx.rollback().await?;
            return Err(DbError::conflict("已发布的计划不可修改，请克隆为新版本"));
        }
        sqlx::query("UPDATE schedule_plans SET name = $2 WHERE id = $1")
            .bind(id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get(id).await
    }

    /// 删除计划：仅草稿；已发布 → [`PlanDeleteOutcome::Published`]（API 映射 409）。
    ///
    /// 计划条目级联删除；该计划发布产生的 legacy 课表保留（`schedules.plan_id` 置 NULL）。
    pub async fn delete(&self, id: Uuid) -> DbResult<PlanDeleteOutcome> {
        let status =
            sqlx::query_scalar::<_, String>("SELECT status FROM schedule_plans WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.db)
                .await?;
        let Some(status) = status else {
            return Ok(PlanDeleteOutcome::NotFound);
        };
        if status == PlanStatus::Published.as_str() {
            return Ok(PlanDeleteOutcome::Published);
        }
        sqlx::query("DELETE FROM schedule_plans WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(PlanDeleteOutcome::Deleted)
    }

    /// 克隆任意状态的计划为新草稿：`version = 同学期最大值 + 1`，名称加「副本」，复制全部条目。
    pub async fn clone(
        &self,
        id: Uuid,
        created_by: Option<Uuid>,
    ) -> DbResult<Option<SchedulePlan>> {
        let mut tx = self.db.begin().await?;
        let source = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT term_id, name FROM schedule_plans WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((term_id, source_name)) = source else {
            tx.rollback().await?;
            return Ok(None);
        };

        let version: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM schedule_plans WHERE term_id = $1",
        )
        .bind(term_id)
        .fetch_one(&mut *tx)
        .await?;
        // 名称加「副本」，总长控制在 200 字符内（按字符截断，避免打断多字节字符）。
        let suffix = " 副本";
        let mut name: String = source_name
            .chars()
            .take(200 - suffix.chars().count())
            .collect();
        name.push_str(suffix);

        let new_id: Uuid = sqlx::query_scalar(
            "INSERT INTO schedule_plans (term_id, name, version, created_by)
             VALUES ($1, $2, $3, $4)
             RETURNING id",
        )
        .bind(term_id)
        .bind(&name)
        .bind(version)
        .bind(created_by)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO plan_entries
                 (plan_id, room_id, day_of_week, start_time, end_time,
                  course_id, teacher_id, class_id, note)
             SELECT $1, room_id, day_of_week, start_time, end_time,
                    course_id, teacher_id, class_id, note
             FROM plan_entries
             WHERE plan_id = $2",
        )
        .bind(new_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get(new_id).await
    }

    /// 条目列表（房间 → 星期 → 开始时间排序）。
    pub async fn entries(&self, plan_id: Uuid) -> DbResult<Vec<PlanEntry>> {
        let sql = format!(
            "SELECT {ENTRY_COLUMNS}
             {ENTRY_FROM}
             WHERE e.plan_id = $1
             ORDER BY a.name, r.code, e.day_of_week, e.start_time, e.id"
        );
        let rows = sqlx::query_as::<_, EntryRow>(&sql)
            .bind(plan_id)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(PlanEntry::from).collect())
    }

    /// 全量替换计划条目（仅草稿）：校验条目引用的房间/课程/教师/班级存在且启用。
    pub async fn replace_entries(
        &self,
        plan_id: Uuid,
        entries: &[ValidatedPlanEntry],
    ) -> DbResult<()> {
        let mut tx = self.db.begin().await?;
        let status =
            sqlx::query_scalar::<_, String>("SELECT status FROM schedule_plans WHERE id = $1")
                .bind(plan_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(status) = status else {
            tx.rollback().await?;
            return Err(DbError::not_found("排课计划不存在"));
        };
        if status == PlanStatus::Published.as_str() {
            tx.rollback().await?;
            return Err(DbError::conflict(
                "已发布的计划不可修改条目，请克隆为新版本",
            ));
        }

        // 批量校验引用（一次一条查询，避免按条目 N+1）。
        let room_ids: Vec<Uuid> = entries.iter().map(|entry| entry.room_id).collect();
        let course_ids: Vec<Uuid> = entries.iter().map(|entry| entry.course_id).collect();
        let teacher_ids: Vec<Uuid> = entries
            .iter()
            .filter_map(|entry| entry.teacher_id)
            .collect();
        let class_ids: Vec<Uuid> = entries.iter().filter_map(|entry| entry.class_id).collect();
        ensure_active_ids(
            &mut tx,
            "SELECT id FROM rooms WHERE id = ANY($1) AND is_active",
            &room_ids,
            "房间",
        )
        .await?;
        ensure_active_ids(
            &mut tx,
            "SELECT id FROM courses WHERE id = ANY($1) AND is_active",
            &course_ids,
            "课程",
        )
        .await?;
        ensure_active_ids(
            &mut tx,
            "SELECT id FROM teachers WHERE id = ANY($1) AND is_active",
            &teacher_ids,
            "教师",
        )
        .await?;
        ensure_active_ids(
            &mut tx,
            "SELECT id FROM classes WHERE id = ANY($1) AND is_active",
            &class_ids,
            "班级",
        )
        .await?;

        sqlx::query("DELETE FROM plan_entries WHERE plan_id = $1")
            .bind(plan_id)
            .execute(&mut *tx)
            .await?;
        for entry in entries {
            sqlx::query(
                "INSERT INTO plan_entries
                     (plan_id, room_id, day_of_week, start_time, end_time,
                      course_id, teacher_id, class_id, note)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(plan_id)
            .bind(entry.room_id)
            .bind(entry.day_of_week)
            .bind(entry.start_time)
            .bind(entry.end_time)
            .bind(entry.course_id)
            .bind(entry.teacher_id)
            .bind(entry.class_id)
            .bind(&entry.note)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// 发布计划：见模块文档与契约 §2.3。返回受影响 legacy 课表，供调用方重建实例。
    pub async fn publish(&self, plan_id: Uuid, actor_id: Option<Uuid>) -> DbResult<PublishOutcome> {
        let mut tx = self.db.begin().await?;

        // 锁计划行：并发重复发布会在此排队，后到者看到 published 状态后返回 409。
        let plan = sqlx::query_as::<_, (String, String, String, NaiveDate, NaiveDate, bool)>(
            "SELECT p.status, p.name, tm.name, tm.start_date, tm.end_date, tm.is_active
             FROM schedule_plans p
             JOIN terms tm ON tm.id = p.term_id
             WHERE p.id = $1
             FOR UPDATE OF p",
        )
        .bind(plan_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((status, plan_name, _term_name, term_start, term_end, term_active)) = plan else {
            tx.rollback().await?;
            return Err(DbError::not_found("排课计划不存在"));
        };
        if status == PlanStatus::Published.as_str() {
            tx.rollback().await?;
            return Err(DbError::conflict("计划已发布，请克隆为新版本后再发布"));
        }
        if !term_active {
            tx.rollback().await?;
            return Err(DbError::bad_request("学期已停用，无法发布"));
        }

        // 条目按房间分组（时间顺序）。
        let entry_sql = "SELECT e.room_id, e.day_of_week, e.start_time, e.end_time,
                    co.name AS course_name, t.name AS teacher_name, e.note
             FROM plan_entries e
             JOIN courses co ON co.id = e.course_id
             LEFT JOIN teachers t ON t.id = e.teacher_id
             WHERE e.plan_id = $1
             ORDER BY e.room_id, e.day_of_week, e.start_time, e.id";
        let entries = sqlx::query_as::<_, PublishEntryRow>(entry_sql)
            .bind(plan_id)
            .fetch_all(&mut *tx)
            .await?;
        let mut entries_by_room: BTreeMap<Uuid, Vec<PublishEntryRow>> = BTreeMap::new();
        for entry in entries {
            entries_by_room
                .entry(entry.room_id)
                .or_default()
                .push(entry);
        }

        // 本计划此前发布产生的 legacy 课表（room_id -> schedule_id）。
        let existing: Vec<(Uuid, Uuid)> =
            sqlx::query_as("SELECT id, room_id FROM schedules WHERE plan_id = $1 ORDER BY id")
                .bind(plan_id)
                .fetch_all(&mut *tx)
                .await?;
        // 同 term 之前已发布计划产生的 legacy 课表：本次发布应**接管**它们（同一房间复用课表行），
        // 否则会被另一份 active 课表重复展示（同一房间出现两套周模板）。
        let previous: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT s.id, s.room_id
             FROM schedules s
             JOIN schedule_plans p ON p.id = s.plan_id
             WHERE p.term_id = (SELECT term_id FROM schedule_plans WHERE id = $1)
               AND p.id <> $1
               AND p.status = 'published'
             ORDER BY s.id",
        )
        .bind(plan_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut existing_by_room: HashMap<Uuid, Uuid> = HashMap::new();
        // 本计划自己的课表优先，其次接管此前 published 计划的课表。
        for (schedule_id, room_id) in &previous {
            existing_by_room.entry(*room_id).or_insert(*schedule_id);
        }
        for (schedule_id, room_id) in &existing {
            existing_by_room.insert(*room_id, *schedule_id);
        }
        // 还没有任何计划课表的房间：优先接管该房间现有的 legacy 启用课表（与学期有交集），
        // 避免发布后同一房间出现「计划课表 + 历史课表」两套 active 周模板。
        let room_ids: Vec<Uuid> = entries_by_room.keys().copied().collect();
        if !room_ids.is_empty() {
            let legacy: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT DISTINCT ON (room_id) id, room_id
                 FROM schedules
                 WHERE plan_id IS NULL AND is_active
                   AND room_id = ANY($1)
                   AND start_date <= $2 AND end_date >= $3
                 ORDER BY room_id, updated_at DESC, id",
            )
            .bind(&room_ids)
            .bind(term_end)
            .bind(term_start)
            .fetch_all(&mut *tx)
            .await?;
            for (schedule_id, room_id) in &legacy {
                existing_by_room.entry(*room_id).or_insert(*schedule_id);
            }
        }

        // 同 term 其它 published 计划降级为 draft（唯一索引只拦新发布的）。
        sqlx::query(
            "UPDATE schedule_plans SET status = 'draft'
             WHERE term_id = (SELECT term_id FROM schedule_plans WHERE id = $1)
               AND status = 'published'
               AND id <> $1",
        )
        .bind(plan_id)
        .execute(&mut *tx)
        .await?;

        let mut schedule_ids: Vec<Uuid> = Vec::new();
        let mut slots_written = 0usize;

        for (room_id, room_entries) in &entries_by_room {
            let schedule_id = match existing_by_room.get(room_id) {
                Some(schedule_id) => {
                    // 更新既有桥接课表：名称 = 计划名、起止 = 学期、启用。
                    sqlx::query(
                        "UPDATE schedules
                         SET name = $2, start_date = $3, end_date = $4,
                             is_active = TRUE, plan_id = $5
                         WHERE id = $1",
                    )
                    .bind(*schedule_id)
                    .bind(&plan_name)
                    .bind(term_start)
                    .bind(term_end)
                    .bind(plan_id)
                    .execute(&mut *tx)
                    .await?;
                    *schedule_id
                }
                None => {
                    sqlx::query_scalar(
                        "INSERT INTO schedules
                         (room_id, name, start_date, end_date, is_active, plan_id, created_by)
                     VALUES ($1, $2, $3, $4, TRUE, $5, $6)
                     RETURNING id",
                    )
                    .bind(*room_id)
                    .bind(&plan_name)
                    .bind(term_start)
                    .bind(term_end)
                    .bind(plan_id)
                    .bind(actor_id)
                    .fetch_one(&mut *tx)
                    .await?
                }
            };

            // 全量替换该课表的周模板：课程/教师名称快照、room_id = NULL（班级本期不入槽位）。
            sqlx::query("DELETE FROM schedule_weekly_slots WHERE schedule_id = $1")
                .bind(schedule_id)
                .execute(&mut *tx)
                .await?;
            for entry in room_entries {
                sqlx::query(
                    "INSERT INTO schedule_weekly_slots
                         (schedule_id, day_of_week, start_time, end_time,
                          course_name, teacher, room_id, note)
                     VALUES ($1, $2, $3, $4, $5, $6, NULL, $7)",
                )
                .bind(schedule_id)
                .bind(entry.day_of_week)
                .bind(entry.start_time)
                .bind(entry.end_time)
                .bind(&entry.course_name)
                .bind(entry.teacher_name.as_deref().unwrap_or(""))
                .bind(&entry.note)
                .execute(&mut *tx)
                .await?;
                slots_written += 1;
            }
            schedule_ids.push(schedule_id);
        }

        // 本次发布未覆盖的旧课表（本计划此前的行 + 被接管计划遗留的房间）：
        // 清空周模板并停用，避免同一房间同时存在两套 active 周模板。
        let mut leftovers: Vec<Uuid> = existing.iter().map(|(id, _)| *id).collect();
        leftovers.extend(previous.iter().map(|(id, _)| *id));
        leftovers.sort_unstable();
        leftovers.dedup();
        for schedule_id in leftovers {
            if schedule_ids.contains(&schedule_id) {
                continue;
            }
            sqlx::query("DELETE FROM schedule_weekly_slots WHERE schedule_id = $1")
                .bind(schedule_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE schedules SET is_active = FALSE WHERE id = $1")
                .bind(schedule_id)
                .execute(&mut *tx)
                .await?;
            schedule_ids.push(schedule_id);
        }
        schedule_ids.sort_unstable();
        schedule_ids.dedup();

        // 发布当前计划（唯一索引此时不会冲突：同 term 其它 published 已降级）。
        sqlx::query(
            "UPDATE schedule_plans
             SET status = 'published', published_at = now(), published_by = $2
             WHERE id = $1",
        )
        .bind(plan_id)
        .bind(actor_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        let plan = self
            .get(plan_id)
            .await?
            .ok_or_else(|| DbError::internal("发布后读取排课计划失败"))?;
        Ok(PublishOutcome {
            plan,
            schedule_ids,
            slots_written,
        })
    }
}

/// 批量校验引用 id 均存在且启用（去重后一次查询；空集合直接通过）。
async fn ensure_active_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sql: &str,
    ids: &[Uuid],
    label: &str,
) -> DbResult<()> {
    let requested: HashSet<Uuid> = ids.iter().copied().collect();
    if requested.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = requested.into_iter().collect();
    let found: Vec<Uuid> = sqlx::query_scalar(sql)
        .bind(&ids)
        .fetch_all(&mut **tx)
        .await?;
    let found: HashSet<Uuid> = found.into_iter().collect();
    if let Some(missing) = ids.iter().find(|id| !found.contains(id)) {
        return Err(DbError::bad_request(format!(
            "{label}不存在或已停用：{missing}"
        )));
    }
    Ok(())
}
