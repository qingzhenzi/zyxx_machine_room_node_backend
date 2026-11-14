-- 0040: ISO 安装 / Windows 支持（image-studio 附录 A，2026-11-14）。
--
-- 扩展 image_build_sessions：
--   - source_mode：base（底包差分盘，v1 行为）/ iso（挂载机器上的安装 ISO 建安装 VM）；
--   - iso_name / disk_bus / nic：ISO 模式创建 VM 的参数（iso_name 只存文件名）；
--   - media_ejected：安装完成后是否已弹出 ISO（finish 前必须为 true）；
--   - iso_check_request_id / eject_request_id：preparing/ejecting 阶段轮询的命令 id；
--   - base_image_id 改为可空：iso 模式没有底包。
--   - state CHECK 增加 'installing' / 'ejecting'。
--
-- 幂等：ADD COLUMN IF NOT EXISTS + DROP/ADD CONSTRAINT 条件判断，可重复执行。
-- 回滚 SQL 见文件末尾注释。

-- ---------------------------------------------------------------------------
-- 1. 新列
-- ---------------------------------------------------------------------------

ALTER TABLE image_build_sessions
    ADD COLUMN IF NOT EXISTS source_mode TEXT NOT NULL DEFAULT 'base',
    ADD COLUMN IF NOT EXISTS iso_name TEXT,
    ADD COLUMN IF NOT EXISTS disk_bus TEXT NOT NULL DEFAULT 'virtio',
    ADD COLUMN IF NOT EXISTS nic TEXT NOT NULL DEFAULT 'none',
    ADD COLUMN IF NOT EXISTS media_ejected BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS iso_check_request_id UUID,
    ADD COLUMN IF NOT EXISTS eject_request_id UUID;

-- iso 模式没有底包；base 模式仍由 API 层保证非空。
ALTER TABLE image_build_sessions
    ALTER COLUMN base_image_id DROP NOT NULL;

-- ---------------------------------------------------------------------------
-- 2. CHECK 约束（命名 + 条件创建，保证幂等；ADD COLUMN 的默认值已符合约束）
-- ---------------------------------------------------------------------------

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'image_build_sessions_source_mode_check'
    ) THEN
        ALTER TABLE image_build_sessions
            ADD CONSTRAINT image_build_sessions_source_mode_check
            CHECK (source_mode IN ('base', 'iso'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'image_build_sessions_disk_bus_check'
    ) THEN
        ALTER TABLE image_build_sessions
            ADD CONSTRAINT image_build_sessions_disk_bus_check
            CHECK (disk_bus IN ('virtio', 'sata'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'image_build_sessions_nic_check'
    ) THEN
        ALTER TABLE image_build_sessions
            ADD CONSTRAINT image_build_sessions_nic_check
            CHECK (nic IN ('none', 'user', 'default'));
    END IF;
END
$$;

-- state CHECK：0039 建表时的匿名约束名为 image_build_sessions_state_check；
-- 先删旧约束再建包含 installing/ejecting 的新约束（DROP IF EXISTS 保证幂等）。
ALTER TABLE image_build_sessions
    DROP CONSTRAINT IF EXISTS image_build_sessions_state_check;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'image_build_sessions_state_v2_check'
    ) THEN
        ALTER TABLE image_build_sessions
            ADD CONSTRAINT image_build_sessions_state_v2_check
            CHECK (state IN (
                'preparing', 'creating', 'installing', 'ejecting', 'ready',
                'finishing', 'making', 'done', 'failed', 'cancelled', 'expired'
            ));
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS image_build_sessions_iso_idx
    ON image_build_sessions (source_mode, created_at DESC);

-- ---------------------------------------------------------------------------
-- 回滚 SQL（手动执行；先停止 qe-api 的会话回收 worker）：
--
-- BEGIN;
-- ALTER TABLE image_build_sessions
--     DROP CONSTRAINT IF EXISTS image_build_sessions_state_v2_check;
-- ALTER TABLE image_build_sessions
--     ADD CONSTRAINT image_build_sessions_state_check
--     CHECK (state IN ('preparing','creating','ready','finishing','making',
--                      'done','failed','cancelled','expired'));
-- DROP INDEX IF EXISTS image_build_sessions_iso_idx;
-- ALTER TABLE image_build_sessions
--     DROP CONSTRAINT IF EXISTS image_build_sessions_nic_check,
--     DROP CONSTRAINT IF EXISTS image_build_sessions_disk_bus_check,
--     DROP CONSTRAINT IF EXISTS image_build_sessions_source_mode_check,
--     DROP COLUMN IF EXISTS eject_request_id,
--     DROP COLUMN IF EXISTS iso_check_request_id,
--     DROP COLUMN IF EXISTS media_ejected,
--     DROP COLUMN IF EXISTS nic,
--     DROP COLUMN IF EXISTS disk_bus,
--     DROP COLUMN IF EXISTS iso_name,
--     DROP COLUMN IF EXISTS source_mode;
-- -- iso 会话无法回收为 base（base_image_id 为空），回滚前先删除它们。
-- DELETE FROM image_build_sessions WHERE base_image_id IS NULL;
-- ALTER TABLE image_build_sessions ALTER COLUMN base_image_id SET NOT NULL;
-- COMMIT;
-- ---------------------------------------------------------------------------
