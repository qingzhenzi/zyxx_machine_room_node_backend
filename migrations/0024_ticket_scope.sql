-- 工单 Wave 2 B：按区域的数据范围（ticket_scopes）。
--
-- 语义（契约）：
-- - 拥有 ticket:scope:all 的用户不受限制；
-- - 有 ticket_scopes 行的用户只能看到/操作这些区域的工单；
-- - 没有任何 scope 行的用户默认不受限制（向后兼容）。

INSERT INTO permissions (code, name, description) VALUES
    ('ticket:scope:manage', '管理工单数据范围', '维护用户可访问的工单区域'),
    ('ticket:scope:all',    '查看全部区域工单', '不受区域数据范围限制')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

CREATE TABLE ticket_scopes (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    area_id UUID NOT NULL REFERENCES areas(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, area_id)
);
CREATE INDEX ticket_scopes_area_idx ON ticket_scopes (area_id);
