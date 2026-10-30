-- 设备上行消息收件箱（契约 §6）
--
-- 为什么需要它：客户端是 at-least-once（没收到 ACK 就重发），服务端必须能
-- 1) 按 (serial, seq) 去重（跨进程重启也要有效，光靠内存水位不够）
-- 2) 记住"已处理 / 处理失败"，让失败的重发能自愈
-- 3) 留下审计轨迹（谁、什么时候、什么载荷、失败原因）

CREATE TABLE device_message_inbox (
  serial       TEXT NOT NULL REFERENCES device_enrollment(serial),
  seq          BIGINT NOT NULL CHECK (seq > 0),
  device_slot  TEXT NOT NULL,
  kind         TEXT NOT NULL,
  payload      TEXT NOT NULL,
  attempts     INT NOT NULL DEFAULT 0,
  received_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  processed_at TIMESTAMPTZ,
  last_error   TEXT,
  PRIMARY KEY (serial, seq)
);

-- 未处理成功的消息：运维/重发都靠它定位
CREATE INDEX device_message_inbox_unprocessed
  ON device_message_inbox (received_at)
  WHERE processed_at IS NULL;
