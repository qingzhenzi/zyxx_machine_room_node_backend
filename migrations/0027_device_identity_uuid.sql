-- 设备全局唯一身份：用 device_id（UUID）替代以 device_slot 为全局身份的假设
--
-- 目标：
-- 1. 为 device_enrollment 加 device_id（全局唯一）
-- 2. 为 bootstrap token / challenge 加 device_id
-- 3. 为 room_devices 改用 device_id 关联，保留 display_name（原 device_slot 降级为展示名）
-- 4. rooms.device_slot 保留但降级为主设备展示名/绑定名；后续逐步过渡到 device_id
-- 5. downlink/inbox/session 等表加 device_id 便于迁移

-- 1) device_bootstrap_token
ALTER TABLE device_bootstrap_token
    ADD COLUMN IF NOT EXISTS device_id UUID;
CREATE UNIQUE INDEX IF NOT EXISTS device_bootstrap_token_device_id_uniq ON device_bootstrap_token (device_id) WHERE device_id IS NOT NULL;

-- 2) device_enroll_challenge
ALTER TABLE device_enroll_challenge
    ADD COLUMN IF NOT EXISTS device_id UUID;
CREATE INDEX IF NOT EXISTS device_enroll_challenge_device_id_idx ON device_enroll_challenge (device_id);

-- 3) device_enrollment
ALTER TABLE device_enrollment
    ADD COLUMN IF NOT EXISTS device_id UUID;
CREATE UNIQUE INDEX IF NOT EXISTS device_enrollment_device_id_uniq ON device_enrollment (device_id) WHERE device_id IS NOT NULL;
-- 便于按 slot 查找（迁移过渡期）
CREATE INDEX IF NOT EXISTS device_enrollment_device_slot_idx ON device_enrollment (device_slot);

-- 4) room_devices: 改用 device_id 关联
-- 保留旧结构，新增列后再迁移数据
ALTER TABLE room_devices
    ADD COLUMN IF NOT EXISTS device_id UUID,
    ADD COLUMN IF NOT EXISTS display_name TEXT;
-- 用现有 device_slot 填充 display_name（便于展示）
UPDATE room_devices SET display_name = COALESCE(display_name, device_slot) WHERE display_name IS NULL;
-- 若有历史数据，尝试按 enrollment 映射（迁移完成后再补充）
CREATE UNIQUE INDEX IF NOT EXISTS room_devices_device_id_uniq ON room_devices (device_id) WHERE device_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS room_devices_room_id_idx ON room_devices (room_id);

-- 5) rooms 保留 device_slot，仅补充注释语义（实际降级由应用层处理）
COMMENT ON COLUMN rooms.device_slot IS 'deprecated_as_identity: room primary device display/legacy binding; prefer device_id';

-- 6) 消息/会话相关表加 device_id（渐进替换）
ALTER TABLE device_message_inbox
    ADD COLUMN IF NOT EXISTS device_id UUID;
ALTER TABLE device_downlink
    ADD COLUMN IF NOT EXISTS device_id UUID;
-- device_session 可能在其他迁移中定义，按需加列
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'device_session') THEN
        ALTER TABLE device_session ADD COLUMN IF NOT EXISTS device_id UUID;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS device_message_inbox_device_id_idx ON device_message_inbox (device_id);
CREATE INDEX IF NOT EXISTS device_downlink_device_id_idx ON device_downlink (device_id);
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'device_session') THEN
        CREATE INDEX IF NOT EXISTS device_session_device_id_idx ON device_session (device_id);
    END IF;
END $$;


