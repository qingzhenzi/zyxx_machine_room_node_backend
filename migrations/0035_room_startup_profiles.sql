-- 房间启动配置（startup provisioning）：契约见 docs/startup-provisioning.md §2。
--
-- 房间里「设备启动后自动准备什么环境」的声明式配置：镜像清单 + 要自动创建/启动的 VM。
-- agent 通过 mTLS `GET /agent/startup` 拉取本表内容（enabled=true 时），按配置自动
-- 拉镜像、建/启 VM；管理端通过 `PUT /admin/rooms/{room_id}/startup` 写入。
--
-- 约定：
-- - 每个房间至多一行（room_id 主键），首次写入 revision=1，之后 revision+1；
-- - config 保存**已校验并规范化**的 JSON（见 qe_db::startup::StartupConfig），
--   不做列级约束，避免配置形状演进时改表；
-- - 删除房间级联删除配置；updated_by 只是审计线索，用户删除后置空。
CREATE TABLE IF NOT EXISTS room_startup_profiles (
    room_id UUID PRIMARY KEY REFERENCES rooms(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
