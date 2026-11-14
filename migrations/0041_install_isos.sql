-- 0041: 平台 ISO 资产库（image-studio 附录 B，v3）。
--
-- 把安装 ISO 变成平台一等资产：上传/预设 → ISO 库 → 构建会话自动分发到目标 agent。
-- 校验/上传流程见 docs/image-studio.md 附录 B：
--   - `install_isos`：ISO 资产元数据（filename 唯一，lower(filename) 唯一索引）；
--   - `image_build_sessions` 增加 `iso_id`（引用资产库）与 `iso_pull_request_id`
--     （preparing 阶段 `vm.command iso.pull` 的 request_id）。
--
-- 幂等：全部 IF NOT EXISTS，可重复执行。回滚 SQL 见文件末尾注释。

CREATE TABLE IF NOT EXISTS install_isos (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 安全文件名：目标机 ISO 目录最终文件名；`lower(filename)` 唯一。
    filename TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT,
    -- 上传阶段先记 0，ready 时回填实际大小。
    size_bytes BIGINT NOT NULL DEFAULT 0,
    sha256 TEXT,
    -- uploading → ready / failed
    status TEXT NOT NULL DEFAULT 'uploading',
    -- S3 对象键：isos/<id>/<filename>；uploading 阶段可为 NULL。
    storage_key TEXT,
    -- 系统预设（seed 目录扫描 / 管理员上传）：所有教师可见。
    is_preset BOOLEAN NOT NULL DEFAULT false,
    uploader_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    uploaded_at TIMESTAMPTZ
);

-- 文件名大小写不敏感唯一：不同大小写视为同名 ISO。
CREATE UNIQUE INDEX IF NOT EXISTS install_isos_filename_key
    ON install_isos (lower(filename));

CREATE INDEX IF NOT EXISTS install_isos_status_idx
    ON install_isos (status, created_at DESC);
CREATE INDEX IF NOT EXISTS install_isos_uploader_idx
    ON install_isos (uploader_user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS install_isos_preset_idx
    ON install_isos (is_preset, created_at DESC);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'install_isos_status_check'
    ) THEN
        ALTER TABLE install_isos
            ADD CONSTRAINT install_isos_status_check
            CHECK (status IN ('uploading', 'ready', 'failed'));
    END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- 构建会话引用 ISO 资产（iso_id）与分发命令 id
-- ---------------------------------------------------------------------------

ALTER TABLE image_build_sessions
    ADD COLUMN IF NOT EXISTS iso_id UUID REFERENCES install_isos(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS iso_pull_request_id UUID;

-- 按 iso_id 查未终态会话（删除保护）用。
CREATE INDEX IF NOT EXISTS image_build_sessions_iso_id_idx
    ON image_build_sessions (iso_id, state)
    WHERE iso_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 回滚 SQL（手动执行）：
--
-- BEGIN;
-- DROP INDEX IF EXISTS image_build_sessions_iso_id_idx;
-- ALTER TABLE image_build_sessions
--     DROP COLUMN IF EXISTS iso_pull_request_id,
--     DROP COLUMN IF EXISTS iso_id;
-- DROP TABLE IF EXISTS install_isos;
-- COMMIT;
-- ---------------------------------------------------------------------------
