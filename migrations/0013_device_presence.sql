-- P3：设备在线事件持久化、可靠投递与实时推送（docs/storage-design.md 第 18 节）
--
-- 1. 区域级 webhook：投递目标按 rooms.webhook_url → areas.webhook_url →
--    DEVICE_PRESENCE_GLOBAL_WEBHOOK_URL 回退；
-- 2. 事件表：唯一约束 (serial, current_state, source_updated_at) 保证多副本扫描
--    同一翻转只落一行；
-- 3. 投递表：唯一约束 (event_id, target_url) 保证同一事件对同一目标只建一次投递，
--    worker 用 FOR UPDATE SKIP LOCKED 领取到期行，多副本不会重复投递。

ALTER TABLE areas ADD COLUMN webhook_url TEXT;

CREATE TABLE device_presence_events (
    id BIGSERIAL PRIMARY KEY,
    serial TEXT NOT NULL,
    device_slot TEXT NOT NULL,
    room_id UUID,
    area_id UUID,
    previous_state TEXT NOT NULL,
    current_state TEXT NOT NULL CHECK (current_state IN ('online','offline')),
    source_updated_at TIMESTAMPTZ NOT NULL,   -- 触发翻转的 device_session.updated_at
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    UNIQUE (serial, current_state, source_updated_at)   -- 多副本幂等
);
CREATE INDEX device_presence_events_id_desc ON device_presence_events (id DESC);

CREATE TABLE device_presence_deliveries (
    id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES device_presence_events(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','delivered','failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (event_id, target_url)                       -- 多副本不重复建投递
);
CREATE INDEX device_presence_deliveries_due
    ON device_presence_deliveries (next_attempt_at)
    WHERE status = 'pending';
