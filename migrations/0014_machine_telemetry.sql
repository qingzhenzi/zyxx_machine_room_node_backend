-- 机器遥测快照（room-agent 周期上报，契约见 docs/machine-telemetry.md）
--
-- 设计要点：
-- - 一台机器 = 一个 device_enrollment（serial 唯一），快照按 (serial, collected_at) 幂等；
-- - payload 原样保存完整上报 JSON（含 platform-health / diagnose / security 等分段），
--   列表用的少量指标做列级冗余，避免扫 JSONB；
-- - 保留策略由 qe-api 的 MACHINE_SNAPSHOT_KEEP 控制（插入时裁剪旧行）。

CREATE TABLE machine_snapshots (
  id                    BIGSERIAL PRIMARY KEY,
  serial                TEXT NOT NULL REFERENCES device_enrollment(serial) ON DELETE CASCADE,
  device_slot           TEXT NOT NULL,
  collected_at          TIMESTAMPTZ NOT NULL,
  received_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
  agent_version         TEXT NOT NULL,
  hostname              TEXT NOT NULL,
  os_name               TEXT,
  kernel_version        TEXT,
  health_status         TEXT NOT NULL,
  health_score          INT,
  cpu_usage_percent     DOUBLE PRECISION,
  memory_usage_percent  DOUBLE PRECISION,
  disk_usage_percent    DOUBLE PRECISION,
  uptime_secs           BIGINT,
  payload               JSONB NOT NULL,
  CONSTRAINT machine_snapshots_health_status_check
    CHECK (health_status IN ('ok', 'warning', 'critical')),
  CONSTRAINT machine_snapshots_health_score_check
    CHECK (health_score IS NULL OR (health_score >= 0 AND health_score <= 100)),
  CONSTRAINT machine_snapshots_collected_at_unique UNIQUE (serial, collected_at)
);

CREATE INDEX machine_snapshots_serial_collected
  ON machine_snapshots (serial, collected_at DESC);

CREATE INDEX machine_snapshots_collected
  ON machine_snapshots (collected_at DESC);
