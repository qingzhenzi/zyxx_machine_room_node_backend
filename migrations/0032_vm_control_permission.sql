-- VM 远程控制：虚拟机操作权限（list / start / shutdown / reboot / destroy / screenshot）。
--
-- 控制台「机器详情 → 虚拟机」面板与
--   POST /api/v1/admin/machines/{serial}/vm/commands
-- 使用该权限。命令通过 device_downlink（kind=vm.command）下发到 room-agent，
-- 结果回写 machine_command_results（表由 0031 创建）。
--
-- 幂等：permission 用 ON CONFLICT DO NOTHING；授权用 JOIN + ON CONFLICT DO NOTHING，
-- 重复执行（含 api 启动时的 bootstrap 补授权）不会报错。

INSERT INTO permissions (code, name, description) VALUES
    ('vm:control', '控制虚拟机', '对机器上的虚拟机执行启动/关机/重启/销毁/截图等操作')
ON CONFLICT (code) DO NOTHING;

-- admin 自动补齐（bootstrap 每次启动也会补一次；这里保证未跑 bootstrap 的库也能直接用）。
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code = 'vm:control'
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;
