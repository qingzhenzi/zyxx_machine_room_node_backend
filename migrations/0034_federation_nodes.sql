-- 总控阶段 2：「多控制节点联邦」注册表（MVP）。
--
-- 每个控制节点保留一份其他控制节点的只读摘要缓存；总控页把本节点 + 各联邦节点
-- 的计数聚合展示。MVP 不做跨节点下发，只做注册/刷新/只读聚合。
--
-- 安全说明：token 用于本节点主动拉取远端 `GET /admin/federation/summary`
-- （远端以 `x-federation-token` 鉴权），MVP 明文存库；生产应改为密文/外部 Secret。

CREATE TABLE IF NOT EXISTS federation_nodes (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name           TEXT NOT NULL UNIQUE,
    base_url       TEXT NOT NULL,
    token          TEXT NOT NULL,
    enabled        BOOLEAN NOT NULL DEFAULT TRUE,
    last_seen_at   TIMESTAMPTZ,
    last_status    TEXT CHECK (last_status IS NULL OR last_status IN ('online', 'offline')),
    cached_summary JSONB,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS federation_nodes_enabled_idx ON federation_nodes (enabled);

DROP TRIGGER IF EXISTS federation_nodes_set_updated_at ON federation_nodes;
CREATE TRIGGER federation_nodes_set_updated_at
    BEFORE UPDATE ON federation_nodes
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
