-- 工单 P1 Wave1：自动建单规则配置化 + SLA 超时提醒（契约 §2/§4）。

CREATE TABLE ticket_rules (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    signal_type TEXT NOT NULL CHECK (signal_type IN (
        'machine_health','device_offline','schedule_delivery',
        'presence_delivery','transfer','image_verify')),
    enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    priority    TEXT NOT NULL DEFAULT 'normal'
        CHECK (priority IN ('urgent','high','normal','low')),
    scope_type  TEXT NOT NULL DEFAULT 'global'
        CHECK (scope_type IN ('global','area','room')),
    scope_id    UUID,                 -- scope_type=area/room 时必填（应用层校验）
    thresholds  JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ticket_rules_signal_idx ON ticket_rules (signal_type, enabled);

DROP TRIGGER IF EXISTS ticket_rules_set_updated_at ON ticket_rules;
CREATE TRIGGER ticket_rules_set_updated_at
    BEFORE UPDATE ON ticket_rules
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- SLA 超时提醒水位。
ALTER TABLE tickets ADD COLUMN last_reminded_at TIMESTAMPTZ;
CREATE INDEX tickets_reminder_idx ON tickets (due_at)
    WHERE status IN ('open','in_progress') AND due_at IS NOT NULL;

-- 时间线新增 `reminded`（超时提醒已发送）。
ALTER TABLE ticket_events DROP CONSTRAINT IF EXISTS ticket_events_kind_check;
ALTER TABLE ticket_events ADD CONSTRAINT ticket_events_kind_check
    CHECK (kind IN (
        'created', 'claimed', 'assigned', 'status_changed',
        'priority_changed', 'commented', 'occurred', 'updated', 'reminded'
    ));

-- 种子规则（全部 enabled / global / priority=normal；machine_health critical
-- 仍由 worker 映射为 high）。
INSERT INTO ticket_rules (name, signal_type, enabled, priority, scope_type, thresholds) VALUES
    ('机器健康连续异常', 'machine_health',    TRUE, 'normal', 'global', '{"consecutive":2}'::jsonb),
    ('设备离线',         'device_offline',    TRUE, 'normal', 'global', '{"offline_minutes":10}'::jsonb),
    ('课表投递失败',     'schedule_delivery', TRUE, 'normal', 'global', '{"lookback_hours":24}'::jsonb),
    ('在线事件投递失败', 'presence_delivery', TRUE, 'normal', 'global', '{"lookback_hours":24}'::jsonb),
    ('存储分发失败',     'transfer',          TRUE, 'normal', 'global', '{"lookback_hours":24}'::jsonb),
    ('镜像验证失败',     'image_verify',      TRUE, 'normal', 'global', '{"lookback_hours":24}'::jsonb);
