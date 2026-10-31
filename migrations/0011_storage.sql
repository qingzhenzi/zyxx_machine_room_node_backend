-- 对象存储与镜像管理（P0）
--
-- 契约见 docs/storage-design.md §14.3。所有 S3 对象内容寻址（objects/{sha256}），
-- 镜像 manifest 存 manifests/{sha256}.json，Ed25519 签名存 signatures/{sha256}.sig。
-- P0 只接受 compression = 'none'（zstd/gzip 字段保留，manifest 校验由 API 拒绝）。

-- 已注册的 S3 对象：object_key 唯一；sha256 为内容寻址摘要（小写 hex）。
CREATE TABLE storage_objects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bucket TEXT NOT NULL,
    object_key TEXT NOT NULL UNIQUE,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    size BIGINT NOT NULL CHECK (size >= 0),
    content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX storage_objects_sha256_idx ON storage_objects (sha256);

-- 镜像制品：(name, version, platform) 唯一；manifest_digest 在 manifest 就绪后写入。
CREATE TABLE image_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    kind TEXT NOT NULL,
    format TEXT NOT NULL,
    compression TEXT NOT NULL DEFAULT 'none',
    platform TEXT NOT NULL,
    manifest_digest TEXT,
    total_size BIGINT NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'uploaded',
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (name, version, platform)
);
CREATE INDEX image_artifacts_status_idx ON image_artifacts (status, created_at DESC);

-- manifest 分片：按 (manifest_digest, index) 唯一，"offset" 为逻辑拼接偏移。
-- 注："offset" 是 SQL 保留字，列名必须加引号（契约 §14.3 的字段名保持不变）。
CREATE TABLE image_chunks (
    manifest_digest TEXT NOT NULL,
    index INT NOT NULL,
    object_key TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size BIGINT NOT NULL,
    "offset" BIGINT NOT NULL,
    PRIMARY KEY (manifest_digest, index)
);

-- 验证记录：每次 verify 追加一行，保留历史报告。
CREATE TABLE image_verifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
    ok BOOLEAN NOT NULL,
    report JSONB NOT NULL DEFAULT '{}'::jsonb,
    verified_by UUID REFERENCES users(id) ON DELETE SET NULL,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX image_verifications_artifact_idx ON image_verifications (artifact_id, verified_at DESC);

-- 发布通道：同一制品在每个通道最多一行；is_current 表示该通道当前生效版本。
CREATE TABLE image_channels (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    is_current BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (artifact_id, channel)
);

-- 镜像分配：房间维度的分发目标（area_id 预留给区域级滚动）。
CREATE TABLE image_assignments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    image_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
    area_id UUID REFERENCES areas(id) ON DELETE CASCADE,
    room_id UUID REFERENCES rooms(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    rollout_state TEXT NOT NULL DEFAULT 'pending',
    assigned_by UUID REFERENCES users(id) ON DELETE SET NULL,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (room_id, image_id)
);

-- 分发 / 收集任务：分配镜像时创建 direction='download' 的 pending 任务。
CREATE TABLE transfers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    direction TEXT NOT NULL,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    image_id UUID REFERENCES image_artifacts(id) ON DELETE SET NULL,
    manifest_digest TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    progress INT NOT NULL DEFAULT 0,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX transfers_room_idx ON transfers (room_id, created_at DESC);

-- 资源收集：节点回传的文件登记（P0 只落库，不下载）。
CREATE TABLE collections (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    area_id UUID REFERENCES areas(id) ON DELETE SET NULL,
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    object_key TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 权限：admin 全量；viewer / editor 只读
INSERT INTO permissions (code, name, description) VALUES
 ('storage:read','查看存储资源','查看存储对象、分发/收集任务'),
 ('storage:manage','管理存储资源','上传注册对象、管理分发/收集任务'),
 ('image:read','查看镜像','查看镜像、清单与验证报告'),
 ('image:manage','管理镜像','创建/验证/发布/分配镜像')
ON CONFLICT (code) DO NOTHING;
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p WHERE r.code='admin' ON CONFLICT DO NOTHING;
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r JOIN permissions p ON p.code IN ('storage:read','image:read')
WHERE r.code IN ('viewer','editor') ON CONFLICT DO NOTHING;
