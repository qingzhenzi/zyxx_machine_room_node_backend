-- 排课全局模型（路径 2 Phase A）：学期 / 教师 / 班级 / 课程 / 排课计划 / 计划条目。
--
-- 契约见 docs/design/schedule-plan-implementation-contract.md §1；
-- 设计背景见 docs/schedule-board-redesign.md §5（路径 2）。
--
-- 设计要点：
-- - 主数据（terms/teachers/classes/courses）只做轻量编码 + 名称，条目用 id 引用；
-- - plan_entries 是「模板周」条目（day_of_week + 本地时段），发布时桥接写入
--   legacy 的 schedules / schedule_weekly_slots，从而不改动事件引擎与工作台读模型；
-- - 同一学期只允许一个 published 计划（部分唯一索引），发布时先把同 term 其它
--   published 降级为 draft；
-- - schedules.plan_id 标记 legacy 课表来自哪份计划（历史课表为 NULL），
--   删除计划时置 NULL（保留历史课表，不做级联删除）。

-- 学期：主数据，计划必须归属一个学期。
CREATE TABLE terms (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_date <= end_date)
);

DROP TRIGGER IF EXISTS terms_set_updated_at ON terms;
CREATE TRIGGER terms_set_updated_at
    BEFORE UPDATE ON terms
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 教师（轻量主数据）。
CREATE TABLE teachers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    phone TEXT NOT NULL DEFAULT '',
    email TEXT NOT NULL DEFAULT '',
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DROP TRIGGER IF EXISTS teachers_set_updated_at ON teachers;
CREATE TRIGGER teachers_set_updated_at
    BEFORE UPDATE ON teachers
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 班级（轻量主数据）。
CREATE TABLE classes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    grade TEXT NOT NULL DEFAULT '',
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DROP TRIGGER IF EXISTS classes_set_updated_at ON classes;
CREATE TRIGGER classes_set_updated_at
    BEFORE UPDATE ON classes
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 课程（轻量主数据；color 供前端课程色板使用，空 = 前端按名称取色）。
CREATE TABLE courses (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '',
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DROP TRIGGER IF EXISTS courses_set_updated_at ON courses;
CREATE TRIGGER courses_set_updated_at
    BEFORE UPDATE ON courses
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 排课计划：草稿 / 已发布；同一学期只允许一个 published。
CREATE TABLE schedule_plans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    term_id UUID NOT NULL REFERENCES terms(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published')),
    version INT NOT NULL DEFAULT 1 CHECK (version >= 1),
    published_at TIMESTAMPTZ,
    published_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX schedule_plans_one_published_per_term
    ON schedule_plans (term_id) WHERE status = 'published';

DROP TRIGGER IF EXISTS schedule_plans_set_updated_at ON schedule_plans;
CREATE TRIGGER schedule_plans_set_updated_at
    BEFORE UPDATE ON schedule_plans
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 计划条目：一条 = 某房间某天某时段的一节课（主数据用 id 引用，展示用快照名称）。
CREATE TABLE plan_entries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_id UUID NOT NULL REFERENCES schedule_plans(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    day_of_week SMALLINT NOT NULL CHECK (day_of_week BETWEEN 1 AND 7),
    start_time TIME NOT NULL,
    end_time TIME NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE RESTRICT,
    teacher_id UUID REFERENCES teachers(id) ON DELETE SET NULL,
    class_id UUID REFERENCES classes(id) ON DELETE SET NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_time < end_time)
);
CREATE INDEX plan_entries_plan_idx ON plan_entries (plan_id, room_id, day_of_week, start_time);
CREATE INDEX plan_entries_room_idx ON plan_entries (room_id);

DROP TRIGGER IF EXISTS plan_entries_set_updated_at ON plan_entries;
CREATE TRIGGER plan_entries_set_updated_at
    BEFORE UPDATE ON plan_entries
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 发布桥接：legacy 课表记录来源计划（可空，历史 / 工作台手工课表为 NULL）。
ALTER TABLE schedules ADD COLUMN plan_id uuid REFERENCES schedule_plans(id) ON DELETE SET NULL;
CREATE INDEX schedules_plan_idx ON schedules (plan_id);
