-- 0036: 镜像所有权与命名空间（多教师镜像管理 MVP，契约见
-- docs/image-build-lifecycle.md §2 / docs/image-lifecycle-multiteacher.md §4）。
--
-- 目标：
-- - 新增 image_namespaces（内置 public + 每个已有用户的个人命名空间）；
-- - image_artifacts 增加归属/可见范围/来源/软删除列，并回填：
--   旧数据 visibility='public'、owner_user_id=created_by（NULL → admin），
--   namespace_id = owner 的个人命名空间（owner 为空时退到 public）；
-- - 唯一约束从全局 UNIQUE(name,version,platform) 改为
--   UNIQUE(namespace_id,name,version,platform)，允许不同教师同名同版本；
--   deprecated / revoked 的旧镜像保留在索引里（同一命名空间仍不能重复创建，
--   与“同名预检排除 deprecated/revoked”不冲突：预检放行但插入会撞唯一索引——
--   这是刻意保守取舍，MVP 阶段先保证不会出现同命名空间同名的两条记录）。
--
-- 幂等：全部使用 IF NOT EXISTS / ON CONFLICT / 条件 DO 块，可重复执行。
-- 回滚 SQL 见文件末尾注释（生产回滚前先备份 image_artifacts）。

-- ---------------------------------------------------------------------------
-- 1. 命名空间
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS image_namespaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- personal：每个用户一个；public：内置公共库；team/area/room 预留给后续共享。
    kind TEXT NOT NULL CHECK (kind IN ('personal', 'public', 'team', 'area', 'room')),
    -- personal 命名空间必须属于某个用户；public 为空。
    owner_user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    -- 规范化后的唯一 slug（个人空间取 username 的规范化形式）。
    slug TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS image_namespaces_owner_idx
    ON image_namespaces (owner_user_id, kind);

-- 内置公共命名空间（幂等；created_at 固定给一个早时间，排序时排在最前）。
INSERT INTO image_namespaces (kind, owner_user_id, slug, display_name, created_at)
VALUES ('public', NULL, 'public', '公共镜像', '1970-01-01T00:00:00Z')
ON CONFLICT (slug) DO NOTHING;

-- 为每个已有用户建个人命名空间：slug = 规范化 username，冲突时追加 -2、-3…
WITH candidates AS (
    SELECT u.id AS user_id,
           u.username,
           -- 规范化：小写；只保留 [a-z0-9._-]，其它字符替换为 '-'。
           -- 与 qe_db::storage::normalize_namespace_slug 的口径保持一致。
           lower(regexp_replace(u.username, '[^A-Za-z0-9._-]+', '-', 'g')) AS raw_slug
    FROM users u
), normalized AS (
    SELECT user_id,
           username,
           -- 去掉首尾的 '-' / '.'，空或保留 slug 时退回该用户 id 的前 8 位。
           NULLIF(
               trim(both '-.' FROM
                    CASE WHEN raw_slug IN ('', 'public') THEN '' ELSE raw_slug END),
               ''
           ) AS slug
    FROM candidates
), numbered AS (
    SELECT user_id,
           username,
           COALESCE(slug, 'u-' || substr(replace(user_id::text, '-', ''), 1, 8)) AS base_slug,
           row_number() OVER (
               PARTITION BY COALESCE(slug, 'u-' || substr(replace(user_id::text, '-', ''), 1, 8))
               ORDER BY user_id
           ) AS seq
    FROM normalized
)
INSERT INTO image_namespaces (kind, owner_user_id, slug, display_name)
SELECT 'personal',
       user_id,
       CASE WHEN seq = 1 THEN base_slug ELSE base_slug || '-' || seq END,
       username
FROM numbered
ON CONFLICT (slug) DO NOTHING;

-- 兜底：slug 冲突（用户名撞上已有 slug）导致上面跳过、或迁移后新建的用户，
-- 保证每个用户至少有一个个人命名空间。
INSERT INTO image_namespaces (kind, owner_user_id, slug, display_name)
SELECT 'personal',
       u.id,
       'u-' || substr(replace(u.id::text, '-', ''), 1, 12),
       u.username
FROM users u
WHERE NOT EXISTS (
    SELECT 1 FROM image_namespaces n
    WHERE n.owner_user_id = u.id AND n.kind = 'personal'
)
ON CONFLICT (slug) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 2. image_artifacts 归属 / 可见范围 / 来源列
-- ---------------------------------------------------------------------------

ALTER TABLE image_artifacts
    ADD COLUMN IF NOT EXISTS namespace_id UUID REFERENCES image_namespaces(id),
    ADD COLUMN IF NOT EXISTS owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    -- 旧数据默认 public（历史上所有人都能看/操作）。
    ADD COLUMN IF NOT EXISTS visibility TEXT NOT NULL DEFAULT 'public',
    ADD COLUMN IF NOT EXISTS source_kind TEXT,
    ADD COLUMN IF NOT EXISTS source_room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS source_machine_serial TEXT,
    ADD COLUMN IF NOT EXISTS source_vm_name TEXT,
    ADD COLUMN IF NOT EXISTS description TEXT,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- 可见范围：MVP 只用 private/public，room/area 预留（M4 共享）。
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'image_artifacts_visibility_check'
    ) THEN
        ALTER TABLE image_artifacts
            ADD CONSTRAINT image_artifacts_visibility_check
            CHECK (visibility IN ('private', 'room', 'area', 'public'))
            NOT VALID;
        ALTER TABLE image_artifacts VALIDATE CONSTRAINT image_artifacts_visibility_check;
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS image_artifacts_owner_idx
    ON image_artifacts (owner_user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS image_artifacts_namespace_idx
    ON image_artifacts (namespace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS image_artifacts_visibility_idx
    ON image_artifacts (visibility, created_at DESC);

-- ---------------------------------------------------------------------------
-- 3. 回填（旧数据 owner=created_by，NULL → 管理员；visibility=public）
-- ---------------------------------------------------------------------------

UPDATE image_artifacts a
SET owner_user_id = COALESCE(
        a.created_by,
        (
            SELECT u.id
            FROM users u
            JOIN user_roles ur ON ur.user_id = u.id
            JOIN roles r ON r.id = ur.role_id
            WHERE r.code = 'admin' AND u.is_active
            ORDER BY u.created_at, u.id
            LIMIT 1
        )
    )
WHERE a.owner_user_id IS NULL;

UPDATE image_artifacts a
SET namespace_id = COALESCE(
        (
            SELECT n.id
            FROM image_namespaces n
            WHERE n.owner_user_id = a.owner_user_id AND n.kind = 'personal'
            LIMIT 1
        ),
        (
            SELECT n.id
            FROM image_namespaces n
            WHERE n.kind = 'public'
            ORDER BY n.created_at, n.id
            LIMIT 1
        )
    )
WHERE a.namespace_id IS NULL;

-- visibility 列 NOT NULL DEFAULT 'public'：旧行自动取默认值，无需再 UPDATE。

-- 回填完成后固定 NOT NULL（只在确实没有空值时执行，保证幂等且不因脏数据卡死）。
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM image_artifacts WHERE namespace_id IS NULL) THEN
        ALTER TABLE image_artifacts ALTER COLUMN namespace_id SET NOT NULL;
    END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- 4. 唯一约束替换：全局 (name,version,platform) → (namespace_id,name,version,platform)
-- ---------------------------------------------------------------------------

-- 先建新索引（旧约束还在，避免出现“无约束窗口”）；再删旧约束。
CREATE UNIQUE INDEX IF NOT EXISTS image_artifacts_ns_name_version_platform
    ON image_artifacts (namespace_id, name, version, platform);

ALTER TABLE image_artifacts
    DROP CONSTRAINT IF EXISTS image_artifacts_name_version_platform_key;

-- 旧数据里若存在同名冲突（理论上不会，因为旧约束是全局唯一），
-- 新索引创建会失败；此时保留旧约束并回滚本次迁移即可（见下方回滚 SQL）。

-- ---------------------------------------------------------------------------
-- 回滚 SQL（手动执行；先备份 image_artifacts）：
--
-- BEGIN;
-- ALTER TABLE image_artifacts
--     ADD CONSTRAINT image_artifacts_name_version_platform_key
--     UNIQUE (name, version, platform);
-- DROP INDEX IF EXISTS image_artifacts_ns_name_version_platform;
-- ALTER TABLE image_artifacts
--     DROP COLUMN IF EXISTS deleted_at,
--     DROP COLUMN IF EXISTS description,
--     DROP COLUMN IF EXISTS source_vm_name,
--     DROP COLUMN IF EXISTS source_machine_serial,
--     DROP COLUMN IF EXISTS source_room_id,
--     DROP COLUMN IF EXISTS source_kind,
--     DROP COLUMN IF EXISTS visibility,
--     DROP COLUMN IF EXISTS owner_user_id,
--     DROP COLUMN IF EXISTS namespace_id;
-- DROP TABLE IF EXISTS image_namespaces;
-- COMMIT;
-- ---------------------------------------------------------------------------
