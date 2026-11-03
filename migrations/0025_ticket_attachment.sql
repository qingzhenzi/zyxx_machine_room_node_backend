-- 工单 Wave 2 C：附件（对象存储预签名上传 + 登记）。
INSERT INTO permissions (code, name, description) VALUES
    ('ticket:attachment', '工单附件', '上传/登记/删除工单附件')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
WHERE r.code = 'admin'
ON CONFLICT DO NOTHING;

CREATE TABLE ticket_attachments (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    ticket_id    UUID NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    object_key   TEXT NOT NULL UNIQUE,
    filename     TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size         BIGINT NOT NULL CHECK (size > 0),
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    uploaded_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ticket_attachments_ticket_idx ON ticket_attachments (ticket_id, created_at DESC);
