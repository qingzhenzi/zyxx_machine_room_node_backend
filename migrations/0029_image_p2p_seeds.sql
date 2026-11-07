-- 0029: P2P 镜像种子登记（agent 间内网分发）
--
-- 设备成功拉取 / 组装镜像后，把自己的 qep2p blob ticket 登记到 device-pki；
-- 同一内网的其它设备按 manifest digest 查询可用 seed，直连下载后再按 manifest
-- 的 chunk 列表逐片校验 sha256，校验通过才导入。seed 有 TTL，过期即不可再被查询到。
--
-- 约定：
--   * manifest_digest 统一归一化为 64 位小写 hex 存储（接受 "sha256:" 前缀输入）；
--   * (manifest_digest, serial) 唯一，upsert 刷新 ticket / size / expires_at；
--   * serial 引用 device_enrollment，设备删除时 seed 级联清理。

CREATE TABLE IF NOT EXISTS image_p2p_seeds (
    id              BIGSERIAL PRIMARY KEY,
    manifest_digest TEXT NOT NULL,
    serial          TEXT NOT NULL REFERENCES device_enrollment(serial) ON DELETE CASCADE,
    blob_ticket     TEXT NOT NULL,
    size_bytes      BIGINT NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    UNIQUE (manifest_digest, serial)
);

-- 查询路径：WHERE manifest_digest = $1 AND expires_at > now() ORDER BY updated_at DESC
CREATE INDEX IF NOT EXISTS image_p2p_seeds_digest_expires_idx
    ON image_p2p_seeds (manifest_digest, expires_at);
