# 排课全局模型（路径 2）Phase A 实现契约

> 状态：**Phase A 已实现（v2.0-alpha，2026-11-02 验收）**。用户已确认需要「学期 / 发布」概念，且教师、班级、课程为独立实体。
> 已知缺口：班级名称暂不写入 legacy 槽位/事件 payload（Phase B 处理）。
> OpenAPI 已在 `openapi.rs` 注册（13 条路径 / 26 个 schema / 5 个 tag），`frontend/src/api/schema.ts` 已重新生成。
> 本文是后端与前端本期实现的唯一契约。设计背景见 `docs/schedule-board-redesign.md` §5（路径 2）。
>
> **本期范围（Phase A）**：主数据（学期/教师/班级/课程）+ 排课计划（草稿/发布）+ 条目编辑 +
> 发布时把计划**桥接写入现有 legacy 课表**（`schedules` / `schedule_weekly_slots`）并重建排课实例，
> 从而不改动事件引擎与现有工作台读模型。后续 Phase B 再把工作台/事件迁移到条目实体。

## 1. 数据库（新增迁移 `migrations/0020_schedule_plan.sql`）

```sql
-- 学期
CREATE TABLE terms (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code text NOT NULL UNIQUE,
  name text NOT NULL,
  start_date date NOT NULL,
  end_date date NOT NULL,
  is_active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (start_date <= end_date)
);

-- 教师 / 班级 / 课程（轻量主数据）
CREATE TABLE teachers (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code text NOT NULL UNIQUE,
  name text NOT NULL,
  phone text NOT NULL DEFAULT '',
  email text NOT NULL DEFAULT '',
  is_active boolean NOT NULL DEFAULT true,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at/updated_at 同上
);
CREATE TABLE classes (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code text NOT NULL UNIQUE,
  name text NOT NULL,
  grade text NOT NULL DEFAULT '',
  is_active boolean NOT NULL DEFAULT true,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at/updated_at 同上
);
CREATE TABLE courses (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code text NOT NULL UNIQUE,
  name text NOT NULL,
  color text NOT NULL DEFAULT '',
  is_active boolean NOT NULL DEFAULT true,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at/updated_at 同上
);

-- 排课计划（草稿/已发布；同一学期只允许一个 published）
CREATE TABLE schedule_plans (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  term_id uuid NOT NULL REFERENCES terms(id) ON DELETE RESTRICT,
  name text NOT NULL,
  status text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','published')),
  version int NOT NULL DEFAULT 1 CHECK (version >= 1),
  published_at timestamptz,
  published_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX schedule_plans_one_published_per_term
  ON schedule_plans (term_id) WHERE status = 'published';

-- 计划条目：一条 = 某房间某天某时段的一节课（主数据用 id 引用，展示用快照名称）
CREATE TABLE plan_entries (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  plan_id uuid NOT NULL REFERENCES schedule_plans(id) ON DELETE CASCADE,
  room_id uuid NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
  day_of_week smallint NOT NULL CHECK (day_of_week BETWEEN 1 AND 7),
  start_time time NOT NULL,
  end_time time NOT NULL,
  course_id uuid NOT NULL REFERENCES courses(id) ON DELETE RESTRICT,
  teacher_id uuid REFERENCES teachers(id) ON DELETE SET NULL,
  class_id uuid REFERENCES classes(id) ON DELETE SET NULL,
  note text NOT NULL DEFAULT '',
  created_at/updated_at 同上,
  CHECK (start_time < end_time)
);
CREATE INDEX plan_entries_plan_idx ON plan_entries (plan_id, room_id, day_of_week, start_time);
CREATE INDEX plan_entries_room_idx ON plan_entries (room_id);

-- 发布桥接：legacy 课表记录来源计划（可空，历史课表为 NULL）
ALTER TABLE schedules ADD COLUMN plan_id uuid REFERENCES schedule_plans(id) ON DELETE SET NULL;
CREATE INDEX schedules_plan_idx ON schedules (plan_id);
```

所有带 `updated_at` 的新表挂 `set_updated_at()` 触发器（沿用 0001 的函数）。
权限不新增：读用 `schedule:read`，写/发布用 `schedule:manage`。

## 2. 后端 API（`/api/v1/admin`，snake_case，统一分页）

### 2.1 主数据 CRUD

每类资源同一套（示例以 `teachers` 为例，`classes` / `courses` / `terms` 同理）：

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| GET | `/teachers?page=&page_size=&keyword=&include_inactive=` | schedule:read | 分页列表 |
| POST | `/teachers` | schedule:manage | 创建 `{code,name,phone?,email?,is_active?,metadata?}` |
| GET | `/teachers/{id}` | schedule:read | 详情 |
| PATCH | `/teachers/{id}` | schedule:manage | 局部更新；`metadata` 可 null |
| DELETE | `/teachers/{id}` | schedule:manage | 被条目引用时 409 |

- `terms` 字段：`{code,name,start_date,end_date,is_active}`（无 metadata）；
- `classes`：`{code,name,grade,is_active,metadata}`；
- `courses`：`{code,name,color,is_active,metadata}`；
- code 复用 `validation::normalize_code`；name 1..200；日期 `start_date <= end_date`；唯一冲突 409；引用冲突 409。
- 列表项额外返回 `usage`（被条目引用数）便于 UI 提示，可选。

### 2.2 排课计划

| 方法 | 路径 | 权限 | 说明 |
| --- | --- | --- | --- |
| GET | `/schedule-plans?term_id=&status=&page=&page_size=` | schedule:read | 分页；`status` = `draft`\|`published` |
| POST | `/schedule-plans` | schedule:manage | `{term_id,name}` 创建草稿（version=1） |
| GET | `/schedule-plans/{id}` | schedule:read | 计划 + 条目（含名称快照） |
| PATCH | `/schedule-plans/{id}` | schedule:manage | `{name}`；仅 draft |
| DELETE | `/schedule-plans/{id}` | schedule:manage | 仅 draft；published → 409 |
| POST | `/schedule-plans/{id}/clone` | schedule:manage | 复制为新草稿（version=max+1，名称加「副本」），任意状态可克隆 |
| GET | `/schedule-plans/{id}/entries` | schedule:read | 条目列表 |
| PUT | `/schedule-plans/{id}/entries` | schedule:manage | 全量替换 `{entries:[NewPlanEntry]}`；仅 draft |
| POST | `/schedule-plans/{id}/publish` | schedule:manage | 发布：桥接写 legacy 课表 + 重建实例；仅 draft |

`PlanView`：`{id, term_id, term_name, name, status, version, published_at, published_by, created_by,
entry_count, course_count, room_count, created_at, updated_at}`。

`PlanEntry`：`{id, plan_id, room_id, room_code, room_name, area_id, area_name, day_of_week,
start_time:"HH:MM", end_time, course_id, course_code, course_name, teacher_id, teacher_name,
class_id, class_name, note}`。

`NewPlanEntry`：`{room_id, day_of_week, start_time, end_time, course_id, teacher_id?, class_id?, note?}`。

条目校验：`max 2000`；room 启用；course 启用；teacher/class 若给出需启用；时间合法；
**同一房间同一天时段不得重叠**（前端也会校验，后端必须校验 400）。

`PublishResult`：`{plan: PlanView, schedules_rebuilt: n, slots_written: n, occurrences: n}`。

### 2.3 发布语义（桥接 legacy 课表）

在**一个事务**内：
1. 校验计划为 draft（已发布 → 409）、学期存在且启用；
2. 按房间分组条目：
   - 找到 `schedules.plan_id = plan.id AND room_id = 房间` 的记录：更新 `name = plan.name`、
     `start_date/end_date = term 起止`、`is_active = true`；
   - 否则新建 `schedules`（room_id, name=plan.name, term 起止, is_active=true, plan_id=plan.id, created_by=当前用户）；
   - 用条目全量替换该课表 `schedule_weekly_slots`：`day_of_week/start_time/end_time` 照抄，
     `course_name = courses.name`、`teacher = teachers.name（空则 ''）`、`room_id = NULL`、
     `note = 条目 note`（班级信息本期不写入 legacy 槽位，见 §4 已知限制）；
3. 该 plan 之前发布过、但本次已无条目的房间：把对应 legacy 课表的周模板清空（保留课表行）；
4. 发布前把同 term 其它 `published` 计划改为 `draft`（唯一索引只会拦新 published）；
5. 更新 `plan.status = published`、`published_at = now()`、`published_by`；
6. 事务提交后，对受影响的 legacy 课表调用 `ScheduleRepo::rebuild_occurrences(schedule_id, horizon)`；
   任何重建失败只记 `tracing::error!`，不影响发布结果（返回里带 `occurrences` 总数）。
7. 审计：`schedule_plan.publish` detail `{plan_id, term_id, schedules, slots, occurrences}`。

**并发**：发布时对 plan 行 `SELECT ... FOR UPDATE`（防重复发布）；legacy 课表更新沿用既有 `FOR UPDATE`。

## 3. 前端 Phase A（新页面 `/schedules/plans`「排课计划」）

- 侧栏「课表与事件」新增入口「排课计划」（icon 可用 `calendar`/`layers`），路由
  `schedules/plans`（name `schedule-plans`，`meta.activeName = 'schedule-plans'`，权限 `schedule:read`）。
- 页面结构（`frontend/src/views/SchedulePlansView.vue` + `frontend/src/features/schedulePlans/`）：
  1. **学期条**：选择学期（下拉）、新建/编辑/删除学期（弹窗），显示起止日期；
  2. **主数据**：三个 tab（课程 / 教师 / 班级），表格 + 新建/编辑/删除（弹窗），字段按 §2.1；
  3. **计划区**（选中学期后）：计划列表（名称、状态徽标 草稿/已发布、版本、条目数、更新时间），
     操作：打开编辑、克隆、删除（仅草稿）、发布（仅草稿，ConfirmDialog 二次确认，提示会覆盖 legacy 课表）；
     新建计划（弹窗：名称）；
  4. **计划编辑器**：显示条目表（按房间→星期→时间排序；列：房间/星期/时间/课程/教师/班级/备注/操作），
     新增/编辑/删除条目（弹窗：房间下拉按区域分组、星期、起止时间、课程/教师/班级下拉），
     底部「保存草稿」（PUT entries）+「发布」；已发布计划只读，显示「克隆为新版本」；
  5. 加载/空/错误态、权限降级（无 manage 只读）、toast 与 ConfirmDialog 沿用控制台规范。
- API 类型手写在 `frontend/src/features/schedulePlans/types.ts`（**不改自动生成的 `api/schema.ts`**），
  `api/index.ts` 增加 `listTerms/createTerm/updateTerm/deleteTerm`、teachers/classes/courses CRUD、
  `listSchedulePlans/getSchedulePlan/createSchedulePlan/updateSchedulePlan/deleteSchedulePlan/cloneSchedulePlan/
  schedulePlanEntries/replaceSchedulePlanEntries/publishSchedulePlan`。
- 发布后跳转/提示：成功 toast「已发布 · N 张课表 / M 节课程」，并提供「去排课工作台查看」链接（`/schedules`）。
- 进入页面默认选中年份包含今天的 active 学期；没有学期则引导创建。

## 4. 已知限制（Phase A）

- 发布是**单向桥接**：legacy 工作台（`/schedules`）仍编辑 legacy 周模板，之后再发布会覆盖这些改动；
  本期 UI 在发布确认框里明确提示「发布会覆盖排课工作台对该学期课表的修改」。
- 班级/课程/教师信息在 legacy 槽位里只保留课程名与教师名，班级不进入事件 payload（Phase B 处理）。
- 计划条目按「模板周」维护（day_of_week + 时间），单日调课本期仍走 legacy 工作台的 exception 机制。
- 删除被计划条目引用的课程/教师/班级返回 409；学期删除同样 409（有计划时）。

## 5. 验收

- `cargo check -p qe-api --all-targets`、`cargo test -p qe-api --lib`、`cargo fmt --check` 通过；
- 临时实例（18080）真实库跑通：主数据 CRUD、计划创建/条目全量替换、发布 → legacy `schedules` 出现
  `plan_id` 标记与正确 weekly slots、排课实例重建、重复发布 409、同 term 旧 published 被降级；
- `vue-tsc --noEmit`、`vitest run` 通过；页面在 1440×900 下完整可用；
- 发布后的课在 `/schedules` 工作台可见（区域/房间正确、周模板正确、事件链路不变）。
