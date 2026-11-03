-- 工单（Ticket / Work Order）P0：表、事件时间线、权限与值班运维角色。
--
-- 契约见 docs/design/ticket-implementation-contract.md §2 与
-- 设计见 docs/design/ticket-system-analysis.md §4。
--
-- 设计要点：
-- - 工单主键 UUID（路由/通知稳定），`code` 人类可读 `T-YYYYMMDD-####`，
--   流水来自全局序列 `tickets_code_seq`；日期按关联区域时区（缺省 Asia/Shanghai）；
-- - 自动单幂等键 `dedupe_key`：仅“未结束”状态（open/in_progress）唯一，
--   关闭/取消后可再开新单（区分“这次发生”）；
-- - `ticket_events` 是处理轨迹事实来源（创建的 created、认领 claimed、转派 assigned、
--   状态 status_changed、优先级 priority_changed、评论 commented、重复发生 occurred）；
-- - 重复发生事件的 `(ticket_id, source_type, source_id)` 部分唯一，保证同一源行只
--   追加一次，`occurrence_count` 因此不会每个 tick 都膨胀；
-- - 自动单不写 audit_logs（无 actor），人工管理动作由 API 侧写审计。

CREATE SEQUENCE IF NOT EXISTS tickets_code_seq;

CREATE TABLE tickets (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code             TEXT NOT NULL UNIQUE,
    title            TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    status           TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open','in_progress','resolved','closed','cancelled')),
    priority         TEXT NOT NULL DEFAULT 'normal'
        CHECK (priority IN ('urgent','high','normal','low')),
    category         TEXT NOT NULL DEFAULT 'other'
        CHECK (category IN ('machine_health','device_offline','delivery_failed',
                            'distribution_failed','security','other')),
    source           TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('auto','manual')),
    -- 精确信号来源（如 machine_snapshot + 快照 id、schedule_delivery + 投递 id）
    source_type      TEXT,
    source_id        TEXT,
    -- 自动单幂等键；手工单为 NULL
    dedupe_key       TEXT,
    area_id          UUID REFERENCES areas(id) ON DELETE SET NULL,
    room_id          UUID REFERENCES rooms(id) ON DELETE SET NULL,
    serial           TEXT,
    device_slot      TEXT,
    reporter_id      UUID REFERENCES users(id) ON DELETE SET NULL,
    assignee_id      UUID REFERENCES users(id) ON DELETE SET NULL,
    due_at           TIMESTAMPTZ,
    acknowledged_at  TIMESTAMPTZ,
    resolved_at      TIMESTAMPTZ,
    closed_at        TIMESTAMPTZ,
    resolution       TEXT,
    occurrence_count INT NOT NULL DEFAULT 1,
    last_seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- 建单时的信号快照（健康问题列表、投递错误、事件 payload 等）。
    -- 源表会被裁剪（machine_snapshots 每台只留最近 N 条），工单里的快照是永久证据。
    metadata         JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- 已解决/已关闭必须有解决时间；已关闭必须有关闭时间。
    CHECK (status NOT IN ('resolved','closed') OR resolved_at IS NOT NULL),
    CHECK (status <> 'closed' OR closed_at IS NOT NULL)
);

-- 同一 dedupe_key 在“未结束”状态下最多一张；关闭/取消后可再开新单。
CREATE UNIQUE INDEX tickets_dedupe_active ON tickets (dedupe_key)
    WHERE dedupe_key IS NOT NULL AND status IN ('open','in_progress');
CREATE INDEX tickets_status_updated_idx ON tickets (status, updated_at DESC);
CREATE INDEX tickets_assignee_idx       ON tickets (assignee_id, status);
CREATE INDEX tickets_room_idx           ON tickets (room_id, created_at DESC);
CREATE INDEX tickets_category_idx       ON tickets (category, priority, status);
CREATE INDEX tickets_serial_idx         ON tickets (serial, created_at DESC);
CREATE INDEX tickets_created_idx        ON tickets (created_at DESC);
CREATE INDEX tickets_due_idx            ON tickets (due_at)
    WHERE status IN ('open','in_progress');

DROP TRIGGER IF EXISTS tickets_set_updated_at ON tickets;
CREATE TRIGGER tickets_set_updated_at
    BEFORE UPDATE ON tickets
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- 处理轨迹（append-only）。
CREATE TABLE ticket_events (
    id          BIGSERIAL PRIMARY KEY,
    ticket_id   UUID NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN
        ('created','claimed','assigned','status_changed','priority_changed','commented','occurred')),
    actor_id    UUID REFERENCES users(id) ON DELETE SET NULL,
    from_status TEXT,
    to_status   TEXT,
    note        TEXT NOT NULL DEFAULT '',
    payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
    source_type TEXT,
    source_id   TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ticket_events_ticket_idx ON ticket_events (ticket_id, id);
-- 同一源行（如某条失败投递）只追加一次 occurred 事件，多副本安全。
CREATE UNIQUE INDEX ticket_events_occurred_key
    ON ticket_events (ticket_id, source_type, source_id)
    WHERE kind = 'occurred' AND source_id IS NOT NULL;

-- ---- 权限：ticket:read / create / handle / manage ----
INSERT INTO permissions (code, name, description) VALUES
    ('ticket:read',   '查看工单', '查看工单列表、详情与统计'),
    ('ticket:create', '创建工单', '手工上报工单'),
    ('ticket:handle', '处理工单', '认领、评论、开始/解决/关闭/重开/取消'),
    ('ticket:manage', '管理工单', '转派给他人、修改标题/描述/优先级/分类')
ON CONFLICT (code) DO NOTHING;

-- 系统管理员自动补齐（bootstrap 启动时也会补，这里保证未跑 bootstrap 的库也可用）。
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

-- 值班运维角色：迁移播种一次，之后由管理员在角色页维护（is_system = FALSE 便于调整）。
INSERT INTO roles (code, name, description, is_system) VALUES
    ('operator', '值班运维', '处理工单，并查看设备/区域/房间', FALSE)
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code IN (
    'ticket:read', 'ticket:create', 'ticket:handle',
    'device:read', 'area:read', 'room:read'
)
WHERE r.code = 'operator'
ON CONFLICT DO NOTHING;

-- viewer / editor 只读：能看工单但不能改。
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code = 'ticket:read'
WHERE r.code IN ('viewer', 'editor')
ON CONFLICT DO NOTHING;
