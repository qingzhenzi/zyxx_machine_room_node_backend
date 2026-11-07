-- WOL（Wake-on-LAN）：设备网络唤醒权限。
--
-- 控制台「机器详情 → 网络唤醒」调用
--   POST /api/v1/admin/machines/{serial}/wake
-- 需要 device:wake；属于电源控制，比 device:read 更高，默认只授系统管理员
-- （operator 等业务角色由管理员在角色页按需勾选）。
--
-- 幂等：permission 用 ON CONFLICT DO NOTHING；授权用 CROSS JOIN + ON CONFLICT DO NOTHING，
-- 重复执行（含 api 启动时的 bootstrap 补授权）不会报错。

INSERT INTO permissions (code, name, description) VALUES
    ('device:wake', '设备唤醒', '向机器发送 Wake-on-LAN magic packet（局域网唤醒）')
ON CONFLICT (code) DO NOTHING;

-- admin 自动补齐（bootstrap 每次启动也会补一次；这里保证未跑 bootstrap 的库也能直接用）。
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code = 'device:wake'
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;
