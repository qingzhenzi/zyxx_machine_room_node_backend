-- 0031: 机器命令结果（Phase1 VM 远程控制 / 后续 GPU 等命令的结果回传）
--
-- 控制面下发命令时先落一行 pending（kind 如 'vm.command'，payload 为原始下发
-- JSON）；agent 执行完通过 device-pki 的 mTLS 路由
-- `POST /machine/command/result` 回传结果：成功 -> ok，失败 -> error，
-- 超时 -> timeout。row 只能由所属 serial 更新；已终态再次上报按幂等 no-op 处理。

CREATE TABLE IF NOT EXISTS machine_command_results (
    request_id  UUID PRIMARY KEY,
    serial      TEXT NOT NULL REFERENCES device_enrollment(serial) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    payload     JSONB NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending', -- pending/ok/error/timeout
    result      JSONB,
    error       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 管理端按设备倒序查最近命令结果。
CREATE INDEX IF NOT EXISTS machine_command_results_serial_created_idx
    ON machine_command_results (serial, created_at DESC);
