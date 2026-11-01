-- P2：agent_configs 支持 device 作用域（机器级覆盖，key 为 device_slot）。
--
-- 背景：0012 的 `agent_configs.scope_id` 是 UUID（area / room 用），而设备槽位
-- `device_slot` 是 TEXT（`device_enrollment.device_slot` / `rooms.device_slot` /
-- `room_devices.device_slot`）。因此新增 `scope_key` 专门存 device 的 key，
-- 其余作用域 `scope_key` 必须为 NULL。
--
-- 作用域四态：
--   global : scope_id IS NULL     AND scope_key IS NULL
--   area   : scope_id IS NOT NULL AND scope_key IS NULL
--   room   : scope_id IS NOT NULL AND scope_key IS NULL
--   device : scope_id IS NULL     AND scope_key NOT NULL（device_slot）
--
-- 合并顺序：global → area → room → device；device 层随机器不同而不同，
-- room 通过 `rooms.device_slot`（主设备）或 `room_devices`（子机）反查。
--
-- 向后兼容：已有行 `scope_key` 自动为 NULL，全部仍满足新约束。

ALTER TABLE agent_configs ADD COLUMN scope_key TEXT;

-- 0012 的列级 CHECK（scope_type）与表级 CHECK（scope_id 搭配）都需要替换。
ALTER TABLE agent_configs DROP CONSTRAINT IF EXISTS agent_configs_scope_type_check;
ALTER TABLE agent_configs ADD CONSTRAINT agent_configs_scope_type_check
    CHECK (scope_type IN ('global', 'area', 'room', 'device'));

ALTER TABLE agent_configs DROP CONSTRAINT IF EXISTS agent_configs_check;
ALTER TABLE agent_configs DROP CONSTRAINT IF EXISTS agent_configs_scope_check;
ALTER TABLE agent_configs ADD CONSTRAINT agent_configs_scope_check CHECK (
       (scope_type = 'global' AND scope_id IS NULL AND scope_key IS NULL)
    OR (scope_type IN ('area', 'room') AND scope_id IS NOT NULL AND scope_key IS NULL)
    OR (scope_type = 'device' AND scope_id IS NULL AND scope_key IS NOT NULL AND scope_key <> '')
);

-- global 唯一索引：从「scope_id IS NULL」收紧为「scope_id / scope_key 均为 NULL」，
-- 否则所有 device 行（scope_id 为 NULL）会落在同一个部分唯一索引里互相冲突。
DROP INDEX IF EXISTS agent_configs_global_uniq;
CREATE UNIQUE INDEX agent_configs_global_uniq
    ON agent_configs (scope_type)
    WHERE scope_id IS NULL AND scope_key IS NULL;

-- scoped 唯一索引（0012 已建：(scope_type, scope_id) WHERE scope_id IS NOT NULL）
-- 对 area / room 保持不变；device 行 scope_id 为 NULL，不参与该索引。

-- device 唯一索引：一个设备槽位最多一行 device 配置。
CREATE UNIQUE INDEX agent_configs_device_uniq
    ON agent_configs (scope_key)
    WHERE scope_type = 'device';
