-- 0042: ISO 分片续传上传 + 共享审批（image-studio 附录 C.1 / C.2，v4）。
--
-- 1. `install_isos` 增加分片上传进度列（upload_chunk_size / upload_total_chunks /
--    upload_updated_at）与共享审批列（visibility / share_status / share_*）。
-- 2. 新增 `install_iso_chunks`：每个分片的大小/SHA/接收时间，主键 (iso_id, chunk_index)；
--    分片文件本体在 `<DATA_DIR>/iso-uploads/<iso_id>/<index>.part`（不入库 S3）。
--
-- 幂等：ADD COLUMN IF NOT EXISTS + 命名约束条件创建 + CREATE TABLE/INDEX IF NOT EXISTS。

-- ---------------------------------------------------------------------------
-- 1. 分片上传进度
-- ---------------------------------------------------------------------------

ALTER TABLE install_isos
    ADD COLUMN IF NOT EXISTS upload_chunk_size BIGINT,
    ADD COLUMN IF NOT EXISTS upload_total_chunks INT,
    ADD COLUMN IF NOT EXISTS upload_updated_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS install_iso_chunks (
    iso_id UUID NOT NULL REFERENCES install_isos(id) ON DELETE CASCADE,
    chunk_index INT NOT NULL CHECK (chunk_index >= 0),
    size_bytes BIGINT NOT NULL CHECK (size_bytes > 0),
    sha256 TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (iso_id, chunk_index)
);

-- ---------------------------------------------------------------------------
-- 2. 共享与审批
-- ---------------------------------------------------------------------------

ALTER TABLE install_isos
    ADD COLUMN IF NOT EXISTS visibility TEXT NOT NULL DEFAULT 'private',
    ADD COLUMN IF NOT EXISTS share_status TEXT NOT NULL DEFAULT 'none',
    ADD COLUMN IF NOT EXISTS share_requested_by UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS share_reviewed_by UUID REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS share_reviewed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS share_note TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'install_isos_visibility_check'
    ) THEN
        ALTER TABLE install_isos
            ADD CONSTRAINT install_isos_visibility_check
            CHECK (visibility IN ('private', 'shared'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'install_isos_share_status_check'
    ) THEN
        ALTER TABLE install_isos
            ADD CONSTRAINT install_isos_share_status_check
            CHECK (share_status IN ('none', 'pending', 'approved', 'rejected'));
    END IF;
END
$$;

-- 审批列表按状态过滤（通常只看 pending）。
CREATE INDEX IF NOT EXISTS install_isos_share_status_idx
    ON install_isos (share_status, updated_at DESC)
    WHERE share_status <> 'none';

-- 列表可见性：preset / owner / admin / shared+approved。
CREATE INDEX IF NOT EXISTS install_isos_visibility_idx
    ON install_isos (visibility, share_status)
    WHERE visibility = 'shared' AND share_status = 'approved';

-- ---------------------------------------------------------------------------
-- 回滚 SQL（手动执行）：
--
-- BEGIN;
-- DROP TABLE IF EXISTS install_iso_chunks;
-- DROP INDEX IF EXISTS install_isos_visibility_idx;
-- DROP INDEX IF EXISTS install_isos_share_status_idx;
-- ALTER TABLE install_isos
--     DROP CONSTRAINT IF EXISTS install_isos_share_status_check,
--     DROP CONSTRAINT IF EXISTS install_isos_visibility_check,
--     DROP COLUMN IF EXISTS share_note,
--     DROP COLUMN IF EXISTS share_reviewed_at,
--     DROP COLUMN IF EXISTS share_reviewed_by,
--     DROP COLUMN IF EXISTS share_requested_by,
--     DROP COLUMN IF EXISTS share_status,
--     DROP COLUMN IF EXISTS visibility,
--     DROP COLUMN IF EXISTS upload_updated_at,
--     DROP COLUMN IF EXISTS upload_total_chunks,
--     DROP COLUMN IF EXISTS upload_chunk_size;
-- COMMIT;
-- ---------------------------------------------------------------------------
