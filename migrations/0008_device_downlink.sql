-- 设备下行消息队列（服务端 → 设备，契约 §6）
--
-- 与上行收件箱 device_message_inbox 对称：上行靠 (serial, seq) 去重，
-- 下行靠全局自增 id 做"从哪继续拉"的游标。
--
-- 为什么用全局 BIGSERIAL 而不是每设备 seq：拉取条件是
-- `WHERE serial = $1 AND id > $after`，全局 id 对单台设备天然单调，
-- 不需要额外维护每设备计数器（也就没有计数器竞争与两步写入）。

CREATE TABLE device_downlink (
  id           BIGSERIAL PRIMARY KEY,
  serial       TEXT NOT NULL REFERENCES device_enrollment(serial),
  device_slot  TEXT NOT NULL,
  kind         TEXT NOT NULL,
  payload      TEXT NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- 未 ACK 的才是待投递的；ACK 即删除（审计由上行收件箱与日志承担）
  delivered_at TIMESTAMPTZ
);

-- 设备拉取：按 (serial, id) 走主键序，只看未投递的
CREATE INDEX device_downlink_pending
  ON device_downlink (serial, id)
  WHERE delivered_at IS NULL;

-- 背压：单台设备的未 ACK 队列长度（发布时读它）
CREATE INDEX device_downlink_unacked
  ON device_downlink (serial)
  WHERE delivered_at IS NULL;
