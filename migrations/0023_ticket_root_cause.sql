-- 工单 Wave 2 A：根因分类（resolve/close 时写入）。
ALTER TABLE tickets ADD COLUMN root_cause TEXT;

ALTER TABLE tickets ADD CONSTRAINT tickets_root_cause_check
    CHECK (root_cause IS NULL OR root_cause IN (
        'hardware', 'network', 'software', 'config', 'usage', 'other'
    ));
