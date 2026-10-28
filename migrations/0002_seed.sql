-- 初始权限与角色
-- 权限编码约定：<资源>:<动作>

INSERT INTO permissions (code, name, description) VALUES
    ('announcement:read',    '查看公告',     '查看已发布的公告'),
    ('announcement:write',   '编辑公告',     '创建与编辑公告草稿'),
    ('announcement:publish', '发布公告',     '发布、下架或归档公告'),
    ('announcement:delete',  '删除公告',     '删除公告'),
    ('user:manage',          '管理用户',     '创建、禁用用户并分配角色'),
    ('role:manage',          '管理角色权限', '创建角色并调整角色权限')
ON CONFLICT (code) DO NOTHING;

INSERT INTO roles (code, name, description, is_system) VALUES
    ('admin',  '管理员', '拥有全部权限的系统角色', TRUE),
    ('editor', '编辑',   '可维护并发布公告',       FALSE),
    ('viewer', '访客',   '只读后台账号，可查看公告（含草稿）', FALSE)
ON CONFLICT (code) DO NOTHING;

-- admin：全部权限
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

-- editor：公告读写与发布
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code IN (
    'announcement:read',
    'announcement:write',
    'announcement:publish'
)
WHERE r.code = 'editor'
ON CONFLICT DO NOTHING;

-- viewer：只读
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.code = 'announcement:read'
WHERE r.code = 'viewer'
ON CONFLICT DO NOTHING;
