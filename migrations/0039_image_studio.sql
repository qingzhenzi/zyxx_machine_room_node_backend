-- 0039: 教师端镜像制作工作台（Image Studio v1，契约 docs/image-studio.md §2）。
--
-- 两张表：
--   1. room_image_defaults：房间默认底包（+ 可选增量包）。`/agent/startup` 会把
--      默认底包/增量包一并追加进 images[]，保证 agent 连上后自动拉取；
--   2. image_build_sessions：服务端统一创建/回收的构建会话。构建 VM 由会话状态机
--      幂等推进（preparing → creating → ready → finishing → making → done），
--      教师端关闭后仍可从「进行中的制作」恢复。
--
-- 幂等：CREATE TABLE / INDEX 均带 IF NOT EXISTS，可重复执行。回滚 SQL 见文件末尾注释。

CREATE TABLE IF NOT EXISTS room_image_defaults (
    room_id UUID PRIMARY KEY REFERENCES rooms(id) ON DELETE CASCADE,
    -- 底包：房间内构建/新建 VM 的默认基础镜像（必须已发布）。
    base_image_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE RESTRICT,
    -- 增量包：本期只存储 + 在 /agent/startup 一并下发拉取，应用逻辑留待 P3。
    delta_image_id UUID REFERENCES image_artifacts(id) ON DELETE SET NULL,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS room_image_defaults_base_idx
    ON room_image_defaults (base_image_id);
CREATE INDEX IF NOT EXISTS room_image_defaults_delta_idx
    ON room_image_defaults (delta_image_id)
    WHERE delta_image_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS image_build_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 会话创建者（教师）：会话仅本人（或 admin）可见/结束/取消。
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    machine_serial TEXT NOT NULL,
    -- 构建 VM 名：qe-build-<8hex>（服务端生成，唯一性由会话 id 派生）。
    build_vm_name TEXT NOT NULL,
    base_image_id UUID NOT NULL REFERENCES image_artifacts(id),
    delta_image_id UUID REFERENCES image_artifacts(id),
    -- 等待的 image.pull transfer（preparing 阶段轮询它）。
    base_transfer_id UUID,
    -- 目标镜像元数据（image.make 时原样下发，保证与工单快照一致）。
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    platform TEXT NOT NULL,
    vcpus INT NOT NULL,
    memory_mib INT NOT NULL,
    disk_gb INT NOT NULL,
    description TEXT,
    -- 状态机：preparing | creating | ready | finishing | making | done |
    --         failed | cancelled | expired（见契约 §2）。
    state TEXT NOT NULL CHECK (state IN (
        'preparing', 'creating', 'ready', 'finishing', 'making',
        'done', 'failed', 'cancelled', 'expired'
    )),
    error TEXT,
    create_request_id UUID,
    shutdown_request_id UUID,
    destroy_request_id UUID,
    make_request_id UUID,
    -- image.make 产出的镜像（draft → manifest_ready → published）。
    image_id UUID REFERENCES image_artifacts(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS image_build_sessions_user_idx
    ON image_build_sessions (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS image_build_sessions_active_idx
    ON image_build_sessions (state)
    WHERE state NOT IN ('done', 'failed', 'cancelled', 'expired');
CREATE INDEX IF NOT EXISTS image_build_sessions_machine_idx
    ON image_build_sessions (machine_serial, created_at DESC);

-- ---------------------------------------------------------------------------
-- 回滚 SQL（手动执行；先停止 qe-api 的会话回收 worker）：
--
-- BEGIN;
-- DROP TABLE IF EXISTS image_build_sessions;
-- DROP TABLE IF EXISTS room_image_defaults;
-- COMMIT;
-- ---------------------------------------------------------------------------
