-- 0038: 镜像制作工单与镜像事件（契约见 docs/image-build-lifecycle.md §4）。
--
-- 背景：设备 register 只有设备证书身份，没有教师身份。控制面在下发 image.make
-- 之前先落「draft 镜像 + 制作工单」，把工单 id 注入下行 args；设备 register 时
-- 回传 build_request_id，服务端据此把同一 artifact 从 draft 推进到 manifest_ready，
-- 从而让 GUI 制作的镜像天然有 owner，且同名冲突能在上传前 409。
--
-- 幂等：全部使用 IF NOT EXISTS，可重复执行。回滚 SQL 见文件末尾注释。

CREATE TABLE IF NOT EXISTS image_build_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 预创建的 draft artifact；删除镜像时级联删除工单。
    image_id UUID NOT NULL REFERENCES image_artifacts(id) ON DELETE CASCADE,
    machine_serial TEXT NOT NULL,
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
    -- 快照（register 时必须与之一致，防止设备侧改参数）。
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    platform TEXT NOT NULL,
    -- pending → consumed / failed / expired
    state TEXT NOT NULL DEFAULT 'pending',
    -- 工单有效期（默认 now()+30min，由 API 写入）。
    expires_at TIMESTAMPTZ NOT NULL,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    consumed_at TIMESTAMPTZ,
    failed_reason TEXT
);

-- 同一设备 + 同名同版本同平台同时只能有一个 pending 工单（并发下发防重）。
CREATE UNIQUE INDEX IF NOT EXISTS image_build_requests_pending_uniq
    ON image_build_requests (machine_serial, name, version, platform)
    WHERE state = 'pending';

CREATE INDEX IF NOT EXISTS image_build_requests_image_idx
    ON image_build_requests (image_id, created_at DESC);
CREATE INDEX IF NOT EXISTS image_build_requests_state_expires_idx
    ON image_build_requests (state, expires_at);

-- 镜像审计事件：build.requested / register / verify / publish / deprecate / delete / assign…
-- actor_kind=user（控制面 API）、device（mTLS 设备回调）、system（定时清理）。
CREATE TABLE IF NOT EXISTS image_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    image_id UUID REFERENCES image_artifacts(id) ON DELETE CASCADE,
    actor_kind TEXT NOT NULL,
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    actor_serial TEXT,
    event TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS image_events_image_idx
    ON image_events (image_id, created_at DESC);
CREATE INDEX IF NOT EXISTS image_events_event_idx
    ON image_events (event, created_at DESC);

-- ---------------------------------------------------------------------------
-- 回滚 SQL（手动执行）：
--
-- BEGIN;
-- DROP TABLE IF EXISTS image_events;
-- DROP TABLE IF EXISTS image_build_requests;
-- COMMIT;
-- ---------------------------------------------------------------------------
