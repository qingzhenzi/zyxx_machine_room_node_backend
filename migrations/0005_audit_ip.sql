-- 审计日志补充客户端 IP。
ALTER TABLE audit_logs ADD COLUMN IF NOT EXISTS ip TEXT;
