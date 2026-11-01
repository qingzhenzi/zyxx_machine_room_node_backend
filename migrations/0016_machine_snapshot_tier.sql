-- P2：机器遥测分两档采集（fast / full）。
--
-- - `fast`：内置 /proc、/sys 指标 + 健康分，10s 级，不上外部工具；
-- - `full`：额外包含 platform-health / diagnose / qeaudit / docker / virsh 等分段，分钟级。
--
-- 表结构只加一列，旧数据默认 `full`，旧上报（不带 tier）也按 full 处理。

ALTER TABLE machine_snapshots
    ADD COLUMN tier TEXT NOT NULL DEFAULT 'full';

ALTER TABLE machine_snapshots
    ADD CONSTRAINT machine_snapshots_tier_check CHECK (tier IN ('fast', 'full'));

CREATE INDEX machine_snapshots_serial_tier_collected
    ON machine_snapshots (serial, tier, collected_at DESC);
