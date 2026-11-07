-- 「教师端」阶段 1：console 内置 teacher 角色。
--
-- 不新建用户体系：由 admin 在「角色权限」页面把 teacher 角色授予教师账号即可。
-- 权限范围（只读为主 + 教学相关操作）：
--   area:read, room:read, schedule:read, schedule:event:read,
--   device:read, device:wake, vm:control,
--   image:read, image:manage, storage:read,
--   ticket:read, ticket:create, ticket:comment, ticket:claim, ticket:transition
--
-- 幂等：角色/授权都用 ON CONFLICT DO NOTHING；重复执行不会报错。
-- 注意：不要动 admin 的权限（admin 由 0002 的 CROSS JOIN 与 bootstrap 自动全量授权）。

INSERT INTO roles (code, name, description, is_system) VALUES
    ('teacher', '教师', '教师工作台：查看区域/房间/课表/设备，管理镜像与工单，控制虚拟机与网络唤醒', FALSE)
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code IN (
    'area:read',
    'room:read',
    'schedule:read',
    'schedule:event:read',
    'device:read',
    'device:wake',
    'vm:control',
    'image:read',
    'image:manage',
    'storage:read',
    'ticket:read',
    'ticket:create',
    'ticket:comment',
    'ticket:claim',
    'ticket:transition'
)
WHERE r.code = 'teacher'
ON CONFLICT DO NOTHING;
