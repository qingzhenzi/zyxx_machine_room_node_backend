-- 工单 P1 Wave1：独立细粒度 RBAC（契约 docs/design/ticket-p1-contract.md §1）。
--
-- 保留 P0 的 ticket:read / create / handle / manage（向后兼容），新增 7 个可独立授权的
-- 动作权限；API 侧用 `require_any_permission(新权限, 旧权限)` 做 OR 兼容判定。
-- `ticket_operator` 角色证明工单权限可以独立授权：只给工单权限，不捆绑
-- device:read / area:read / room:read（工单详情上下文由 ticket 接口自带）。

INSERT INTO permissions (code, name, description) VALUES
    ('ticket:claim',      '认领工单', '认领工单 / 开始处理（transition 到 in_progress）'),
    ('ticket:comment',    '评论工单', '在工单时间线追加评论'),
    ('ticket:transition', '流转工单', '解决 / 关闭 / 重开 / 取消'),
    ('ticket:assign',     '转派工单', '转派或释放处理人'),
    ('ticket:update',     '编辑工单', '修改标题 / 描述 / 优先级 / 分类'),
    ('ticket:rule:manage','管理工单规则', '自动建单规则的读取与增删改'),
    ('ticket:export',     '导出工单', '导出工单 CSV')
ON CONFLICT (code) DO NOTHING;

-- 系统管理员全量授权（bootstrap 启动时也会补）。
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

-- 值班运维：移除粗粒度 ticket:handle，改为 claim/comment/transition 三个独立权限；
-- read/create 与 device/area/room read 保持不变。
DELETE FROM role_permissions rp
USING roles r, permissions p
WHERE rp.role_id = r.id
  AND rp.permission_id = p.id
  AND r.code = 'operator'
  AND p.code = 'ticket:handle';

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code IN ('ticket:claim', 'ticket:comment', 'ticket:transition')
WHERE r.code = 'operator'
ON CONFLICT DO NOTHING;

-- 工单处理员：只给工单权限（无任何 device/area/room 权限）。
INSERT INTO roles (code, name, description, is_system) VALUES
    ('ticket_operator', '工单处理员', '只处理工单，不捆绑设备/区域/房间权限', FALSE)
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code IN (
    'ticket:read', 'ticket:create', 'ticket:claim', 'ticket:comment', 'ticket:transition'
)
WHERE r.code = 'ticket_operator'
ON CONFLICT DO NOTHING;
