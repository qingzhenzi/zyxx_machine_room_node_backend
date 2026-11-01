-- P1：一个房间多台机器（主节点 + 子机）。
--
-- 事实来源约定：
-- - 主机器：`rooms.device_slot`（保持既有口径：课表/镜像下发/房间设备摘要不变）；
-- - 子机器：`room_devices`；一台设备最多属于一个房间。
-- - 不建到 `device_enrollment` 的外键：槽位在签发注册 token 时已存在，
--   设备完成注册后才有 enrollment 行（与 `device_bootstrap_token` / `rooms.device_slot` 一致）。
--
-- 两个触发器保证「同一槽位不会同时是某房间的主设备、又是另一房间的子设备」：
-- 1. 槽位升格为房间主设备时，自动从 `room_devices` 移除；
-- 2. 已是某房间主设备的槽位不允许再插入 `room_devices`。

CREATE TABLE room_devices (
    device_slot TEXT PRIMARY KEY,
    room_id     UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    label       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT room_devices_slot_not_empty CHECK (device_slot <> '')
);

CREATE INDEX room_devices_room_idx ON room_devices (room_id);

DROP TRIGGER IF EXISTS room_devices_set_updated_at ON room_devices;
CREATE TRIGGER room_devices_set_updated_at
    BEFORE UPDATE ON room_devices
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE OR REPLACE FUNCTION rooms_clear_room_devices()
RETURNS trigger AS $$
BEGIN
    IF NEW.device_slot IS NOT NULL THEN
        DELETE FROM room_devices WHERE device_slot = NEW.device_slot;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS rooms_clear_room_devices ON rooms;
CREATE TRIGGER rooms_clear_room_devices
    BEFORE INSERT OR UPDATE OF device_slot ON rooms
    FOR EACH ROW EXECUTE FUNCTION rooms_clear_room_devices();

CREATE OR REPLACE FUNCTION room_devices_reject_primary()
RETURNS trigger AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM rooms WHERE device_slot = NEW.device_slot) THEN
        RAISE EXCEPTION 'device_slot % 已是房间主设备，不能同时作为子机', NEW.device_slot;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS room_devices_reject_primary ON room_devices;
CREATE TRIGGER room_devices_reject_primary
    BEFORE INSERT OR UPDATE OF device_slot ON room_devices
    FOR EACH ROW EXECUTE FUNCTION room_devices_reject_primary();
