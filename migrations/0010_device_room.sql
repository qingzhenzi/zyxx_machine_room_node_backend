-- v3：设备接入区域 / 房间（device PKI ↔ 房间列表）
--
-- 契约见 docs/schedule-design.md 第 11 节。本迁移**只新增权限**，无表结构变更：
-- `rooms.device_slot` 与部分唯一索引 `rooms_device_slot_key` 已在 0009_schedule.sql 定义。
--
-- 设备本身仍由 device_enrollment / device_session / device_bootstrap_token 管理
-- （见 0006_device_pki.sql）；绑定的唯一事实来源是
-- `rooms.device_slot` ↔ `device_enrollment.device_slot`。

-- 新权限
INSERT INTO permissions (code, name, description) VALUES
    ('device:read',   '查看设备', '查看设备列表、注册状态与房间绑定'),
    ('device:manage', '管理设备', '绑定/解绑设备到房间、生成设备注册 token')
ON CONFLICT (code) DO NOTHING;

-- 系统管理员自动补齐（bootstrap 启动时也会补，这里保证未跑 bootstrap 的库也可用）
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

-- viewer / editor 获得只读权限
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r
JOIN permissions p ON p.code = 'device:read'
WHERE r.code IN ('viewer', 'editor')
ON CONFLICT DO NOTHING;
