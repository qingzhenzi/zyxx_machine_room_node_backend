-- 课表（Schedule）：区域 → 房间 → 课表 → 周模板 / 单日覆盖 / 排课实例 / 触发规则 / 事件与投递
--
-- 契约见 docs/schedule-design.md 第 2 节；v2 见第 10 节（IANA 时区：区域 → 房间/课表继承）。
-- 触发器复用 0001_init.sql 里的 set_updated_at()，保证所有带 updated_at 的表在 UPDATE 时自动刷新。

-- 区域：楼栋 / 楼层 / 片区。parent_id 与 metadata 为总管理节点 / 子管理节点预留。
-- v2（§10）：区域携带 IANA 时区；房间 / 课表继承该时区，周模板的 TIME 由该时区解释。
CREATE TABLE areas (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    timezone TEXT NOT NULL DEFAULT 'Asia/Shanghai',
    parent_id UUID REFERENCES areas(id) ON DELETE SET NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX areas_parent_idx ON areas (parent_id);

DROP TRIGGER IF EXISTS areas_set_updated_at ON areas;
CREATE TRIGGER areas_set_updated_at
    BEFORE UPDATE ON areas
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 房间：调度最小单位，可绑定一个设备槽位（device_enrollment.device_slot）。
-- v2（§10）：删除 utc_offset_minutes，时区继承自 areas.timezone。
CREATE TABLE rooms (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    area_id UUID NOT NULL REFERENCES areas(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    capacity INT NOT NULL DEFAULT 0 CHECK (capacity >= 0),
    device_slot TEXT,
    webhook_url TEXT,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX rooms_area_idx ON rooms (area_id);
CREATE UNIQUE INDEX rooms_device_slot_key ON rooms (device_slot) WHERE device_slot IS NOT NULL;

DROP TRIGGER IF EXISTS rooms_set_updated_at ON rooms;
CREATE TRIGGER rooms_set_updated_at
    BEFORE UPDATE ON rooms
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 课表：一个房间的排课计划（可存在多份，通常按学期/周期区分）。
-- v2（§10）：删除 utc_offset_minutes，时区继承自房间 → 区域。
CREATE TABLE schedules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_date <= end_date)
);
CREATE INDEX schedules_room_idx ON schedules (room_id, start_date, end_date);

DROP TRIGGER IF EXISTS schedules_set_updated_at ON schedules;
CREATE TRIGGER schedules_set_updated_at
    BEFORE UPDATE ON schedules
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 默认周循环模板：ISO 星期（周一=1..周日=7）+ 本地时段。
CREATE TABLE schedule_weekly_slots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    day_of_week SMALLINT NOT NULL CHECK (day_of_week BETWEEN 1 AND 7),
    start_time TIME NOT NULL,
    end_time TIME NOT NULL,
    course_name TEXT NOT NULL,
    teacher TEXT NOT NULL DEFAULT '',
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL, -- 调课到其他房间；空 = 课表所属房间
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_time < end_time)
);
CREATE INDEX schedule_weekly_slots_schedule_idx
    ON schedule_weekly_slots (schedule_id, day_of_week, start_time);

-- 单日覆盖：某天停课（cancel）或用自定义时段替换当天周模板（replace）。
CREATE TABLE schedule_exceptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    exception_date DATE NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('cancel', 'replace')),
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (schedule_id, exception_date)
);

DROP TRIGGER IF EXISTS schedule_exceptions_set_updated_at ON schedule_exceptions;
CREATE TRIGGER schedule_exceptions_set_updated_at
    BEFORE UPDATE ON schedule_exceptions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE schedule_exception_slots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    exception_id UUID NOT NULL REFERENCES schedule_exceptions(id) ON DELETE CASCADE,
    start_time TIME NOT NULL,
    end_time TIME NOT NULL,
    course_name TEXT NOT NULL,
    teacher TEXT NOT NULL DEFAULT '',
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
    note TEXT NOT NULL DEFAULT '',
    CHECK (start_time < end_time)
);
CREATE INDEX schedule_exception_slots_exception_idx
    ON schedule_exception_slots (exception_id, start_time);

-- 物化的排课实例：周模板 × 日期（或单日覆盖）展开后的具体一节课。
CREATE TABLE schedule_occurrences (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id UUID NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, -- 实际上课房间（可能调课）
    occurrence_date DATE NOT NULL,
    start_at TIMESTAMPTZ NOT NULL,
    end_at TIMESTAMPTZ NOT NULL,
    course_name TEXT NOT NULL,
    teacher TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL CHECK (source IN ('weekly', 'custom')),
    source_slot_id UUID NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (start_at < end_at),
    UNIQUE (schedule_id, source, source_slot_id, occurrence_date)
);
CREATE INDEX schedule_occurrences_room_time_idx ON schedule_occurrences (room_id, start_at);
CREATE INDEX schedule_occurrences_schedule_time_idx ON schedule_occurrences (schedule_id, start_at);
CREATE INDEX schedule_occurrences_time_idx ON schedule_occurrences (start_at, end_at);

-- 触发规则：按房间配置。config 为 JSONB（targets / webhook_url / downlink_kind / payload）。
CREATE TABLE schedule_event_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    trigger_kind TEXT NOT NULL CHECK (trigger_kind IN (
        'occurrence_start', 'occurrence_end', 'occurrence_start_before',
        'daily_time', 'room_occupied', 'room_idle'
    )),
    offset_minutes INT NOT NULL DEFAULT 0 CHECK (offset_minutes BETWEEN 0 AND 1440),
    at_time TIME,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (trigger_kind <> 'daily_time' OR at_time IS NOT NULL)
);
CREATE INDEX schedule_event_rules_room_idx
    ON schedule_event_rules (room_id) WHERE enabled;

DROP TRIGGER IF EXISTS schedule_event_rules_set_updated_at ON schedule_event_rules;
CREATE TRIGGER schedule_event_rules_set_updated_at
    BEFORE UPDATE ON schedule_event_rules
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 事件：event_key 唯一，保证多副本 / 重算时幂等。
CREATE TABLE schedule_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_key TEXT NOT NULL UNIQUE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    -- v2（§10）：事件生成时所属区域的 IANA 时区快照（展示 / 审计用）。
    timezone TEXT NOT NULL,
    schedule_id UUID REFERENCES schedules(id) ON DELETE SET NULL,
    occurrence_id UUID REFERENCES schedule_occurrences(id) ON DELETE SET NULL,
    rule_id UUID REFERENCES schedule_event_rules(id) ON DELETE SET NULL,
    event_kind TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    triggered_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX schedule_events_room_time_idx ON schedule_events (room_id, triggered_at DESC);
CREATE INDEX schedule_events_created_idx ON schedule_events (created_at DESC);

-- 投递：每个目标一行；重试由 next_attempt_at 驱动。
CREATE TABLE schedule_event_deliveries (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES schedule_events(id) ON DELETE CASCADE,
    target TEXT NOT NULL CHECK (target IN ('pubsub', 'downlink', 'webhook')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'delivered', 'failed', 'skipped')),
    attempts INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    delivered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (event_id, target)
);
CREATE INDEX schedule_event_deliveries_pending_idx
    ON schedule_event_deliveries (next_attempt_at) WHERE status = 'pending';

-- 新权限
INSERT INTO permissions (code, name, description) VALUES
    ('area:read',           '查看区域',     '查看区域列表与详情'),
    ('area:manage',         '管理区域',     '创建、修改、删除区域'),
    ('room:read',           '查看房间',     '查看房间列表与详情'),
    ('room:manage',         '管理房间',     '创建、修改、删除房间与触发规则'),
    ('schedule:read',       '查看课表',     '查看课表、周模板、单日覆盖与排课实例'),
    ('schedule:manage',     '管理课表',     '创建、修改、删除课表与触发规则'),
    ('schedule:event:read', '查看课表事件', '查看触发事件与投递状态'),
    ('schedule:event:manage','管理课表事件','重试或重放事件投递')
ON CONFLICT (code) DO NOTHING;

-- 系统管理员自动补齐（bootstrap 启动时也会补，这里保证未跑 bootstrap 的库也可用）
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

-- viewer / editor 获得只读权限
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r
JOIN permissions p ON p.code IN ('area:read', 'room:read', 'schedule:read', 'schedule:event:read')
WHERE r.code IN ('viewer', 'editor')
ON CONFLICT DO NOTHING;
