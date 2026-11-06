-- 0028：device_id 回填（0027 只做加列；回填与索引调整独立成迁移，避免已应用版本的 checksum 冲突）

-- 回填：为现有数据生成 device_id
UPDATE device_enrollment
SET device_id = COALESCE(device_id, gen_random_uuid())
WHERE device_id IS NULL;

-- token 必须复用同 slot 已有 enrollment 的 device_id，否则客户端用旧 token 注册时
-- 会签出与该 slot enrollment 不一致的 device_id（证书 CN 与 DB identity 分叉）。
UPDATE device_bootstrap_token t
SET device_id = e.device_id
FROM (
    SELECT DISTINCT ON (device_slot) device_slot, device_id
    FROM device_enrollment
    WHERE device_id IS NOT NULL
    ORDER BY device_slot,
             (status IN ('issued','active')) DESC,
             activated_at DESC NULLS LAST,
             issued_at DESC NULLS LAST
) e
WHERE t.device_id IS NULL AND t.device_slot = e.device_slot;

-- 没有任何 enrollment 的 token（典型是刚签发、还没用到）才分配新 id。
UPDATE device_bootstrap_token
SET device_id = gen_random_uuid()
WHERE device_id IS NULL;

UPDATE device_enroll_challenge
SET device_id = COALESCE(device_id, (
    SELECT device_id FROM device_bootstrap_token t WHERE t.token_hash = device_enroll_challenge.token_hash LIMIT 1
))
WHERE device_id IS NULL;

UPDATE room_devices rd
SET device_id = COALESCE(rd.device_id, (
    SELECT e.device_id FROM device_enrollment e WHERE e.device_slot = rd.device_slot AND e.device_id IS NOT NULL ORDER BY e.issued_at DESC NULLS LAST LIMIT 1
))
WHERE rd.device_id IS NULL;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'device_session') THEN
        UPDATE device_session ds
        SET device_id = COALESCE(ds.device_id, (
            SELECT e.device_id FROM device_enrollment e WHERE e.device_slot = ds.device_slot AND e.device_id IS NOT NULL ORDER BY e.issued_at DESC NULLS LAST LIMIT 1
        ))
        WHERE ds.device_id IS NULL;
    END IF;
END $$;

UPDATE device_message_inbox mi
SET device_id = COALESCE(mi.device_id, (
    SELECT e.device_id FROM device_enrollment e WHERE e.device_slot = mi.device_slot AND e.device_id IS NOT NULL ORDER BY e.issued_at DESC NULLS LAST LIMIT 1
))
WHERE mi.device_id IS NULL;

UPDATE device_downlink dl
SET device_id = COALESCE(dl.device_id, (
    SELECT e.device_id FROM device_enrollment e WHERE e.device_slot = dl.device_slot AND e.device_id IS NOT NULL ORDER BY e.issued_at DESC NULLS LAST LIMIT 1
))
WHERE dl.device_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS device_enrollment_device_id_uniq ON device_enrollment (device_id);
CREATE UNIQUE INDEX IF NOT EXISTS device_bootstrap_token_device_id_uniq ON device_bootstrap_token (device_id);
CREATE UNIQUE INDEX IF NOT EXISTS room_devices_device_id_uniq ON room_devices (device_id) WHERE device_id IS NOT NULL;
