-- 工单 Wave3：巡检计划（inspection plans / runs）。
--
-- 计划按 scope（global/area/room）+ interval_hours 周期执行：worker 用
-- `FOR UPDATE SKIP LOCKED` 原子领取到期计划并前移 next_run_at，多副本安全。
-- 每次执行产生一条 inspection_runs；有异常机器时自动建一张 machine_health 工单
-- 并在 run 上关联 ticket_id。

INSERT INTO permissions (code, name, description) VALUES
    ('ticket:inspection:manage', '管理巡检计划', '巡检计划的读取、增删改与手动执行')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

CREATE TABLE inspection_plans (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name           TEXT NOT NULL,
    scope_type     TEXT NOT NULL DEFAULT 'global'
        CHECK (scope_type IN ('global', 'area', 'room')),
    scope_id       UUID,
    interval_hours INT NOT NULL DEFAULT 24 CHECK (interval_hours BETWEEN 1 AND 168),
    enabled        BOOLEAN NOT NULL DEFAULT TRUE,
    created_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    last_run_at    TIMESTAMPTZ,
    next_run_at    TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX inspection_plans_due_idx ON inspection_plans (enabled, next_run_at);

DROP TRIGGER IF EXISTS inspection_plans_set_updated_at ON inspection_plans;
CREATE TRIGGER inspection_plans_set_updated_at
    BEFORE UPDATE ON inspection_plans
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE inspection_runs (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    plan_id          UUID NOT NULL REFERENCES inspection_plans(id) ON DELETE CASCADE,
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at      TIMESTAMPTZ,
    status           TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'ok', 'issues', 'failed')),
    checked_machines INT NOT NULL DEFAULT 0,
    issue_machines   INT NOT NULL DEFAULT 0,
    ticket_id        UUID REFERENCES tickets(id) ON DELETE SET NULL,
    summary          JSONB NOT NULL DEFAULT '{}'::jsonb,
    error            TEXT
);
CREATE INDEX inspection_runs_plan_idx ON inspection_runs (plan_id, started_at DESC);
