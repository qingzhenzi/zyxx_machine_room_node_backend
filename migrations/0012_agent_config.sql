-- P2：节点配置管理（docs/storage-design.md 第 17 节）
--
-- 配置按作用域存储：global（唯一，scope_id 为空）→ area → room。
-- 每个作用域只存**增量 patch**（AgentPolicyPatch 的 JSONB），生效策略由
-- global → area(room.area_id) → room 依次合并得到；revision 在每次 upsert 时 +1。
CREATE TABLE agent_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scope_type TEXT NOT NULL CHECK (scope_type IN ('global','area','room')),
    scope_id UUID,
    policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((scope_type = 'global' AND scope_id IS NULL)
        OR (scope_type <> 'global' AND scope_id IS NOT NULL))
);
CREATE UNIQUE INDEX agent_configs_global_uniq ON agent_configs (scope_type) WHERE scope_id IS NULL;
CREATE UNIQUE INDEX agent_configs_scoped_uniq ON agent_configs (scope_type, scope_id) WHERE scope_id IS NOT NULL;
