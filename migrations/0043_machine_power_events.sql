-- 0043: 机器电源事件（关机 / 重启）与最后一次事件查询。
--
-- agent 在主机 shutdown/reboot 收尾完成后通过 device-pki mTLS
-- `POST /agent/power-event` 上报；即使上报失败，agent 本地也会写
-- `$DEVICE_DATA_DIR/last-shutdown.json`（reported=false），下次启动补报。
--
-- 设计要点：
-- - 一个 serial 多次事件按时间保留；管理台/详情只取最新一条；
-- - action 只记录 shutdown / reboot；
-- - graceful 表示「无 destroy、无清理错误、网络清理成功」；
-- - details 保存 agent 的完整收尾明细（VM 列表、网络、错误等），便于排障；
-- - created_at 用服务端时间，避免设备时钟漂移影响排序。

CREATE TABLE IF NOT EXISTS machine_power_events (
    id            BIGSERIAL PRIMARY KEY,
    serial        TEXT NOT NULL REFERENCES device_enrollment(serial) ON DELETE CASCADE,
    device_slot   TEXT NOT NULL,
    action        TEXT NOT NULL,
    graceful      BOOLEAN NOT NULL,
    requested_by  TEXT,
    agent_version TEXT,
    source        TEXT NOT NULL DEFAULT 'agent',
    note          TEXT,
    details       JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT machine_power_events_action_check
        CHECK (action IN ('shutdown', 'reboot'))
);

-- 详情页/列表「最近一次电源事件」：按 serial + 时间倒序取一条。
CREATE INDEX IF NOT EXISTS machine_power_events_serial_created_idx
    ON machine_power_events (serial, created_at DESC, id DESC);

-- 管理端按时间排查全房间关机/重启（可选）。
CREATE INDEX IF NOT EXISTS machine_power_events_created_idx
    ON machine_power_events (created_at DESC);

-- 回滚 SQL（手动执行）：
--
-- BEGIN;
-- DROP INDEX IF EXISTS machine_power_events_created_idx;
-- DROP INDEX IF EXISTS machine_power_events_serial_created_idx;
-- DROP TABLE IF EXISTS machine_power_events;
-- COMMIT;
