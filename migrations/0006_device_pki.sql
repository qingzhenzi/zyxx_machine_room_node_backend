-- 设备注册与 mTLS 通讯（docs/device-pki-protocol.md 第 5 节）

CREATE TABLE device_bootstrap_token (
  token_hash   BYTEA PRIMARY KEY,
  kind         TEXT NOT NULL CHECK (kind IN ('auto','approval')),
  device_slot  TEXT NOT NULL,
  created_by   TEXT NOT NULL,
  expires_at   TIMESTAMPTZ NOT NULL,
  used_at      TIMESTAMPTZ
);

CREATE TABLE device_enroll_challenge (
  challenge_id UUID PRIMARY KEY,
  challenge    BYTEA NOT NULL,
  token_hash   BYTEA NOT NULL REFERENCES device_bootstrap_token,
  device_slot  TEXT NOT NULL,
  issued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at   TIMESTAMPTZ NOT NULL,
  consumed_at  TIMESTAMPTZ
);

CREATE TABLE device_enrollment (
  serial        TEXT PRIMARY KEY,
  device_slot   TEXT NOT NULL,
  subject_cn    TEXT NOT NULL,
  csr_sha256    BYTEA NOT NULL,
  cert_pem      TEXT NOT NULL,
  status        TEXT NOT NULL CHECK (status IN ('pending','issued','active','rejected','revoked')),
  client_info   JSONB,
  enroll_id     UUID,
  downloaded_at TIMESTAMPTZ,
  issued_at     TIMESTAMPTZ,
  activated_at  TIMESTAMPTZ,
  revoked_at    TIMESTAMPTZ,
  revoke_reason TEXT
);

-- 同一 slot 只允许一个非终态证书（部分唯一索引）
CREATE UNIQUE INDEX device_slot_one_live ON device_enrollment (device_slot)
  WHERE status IN ('pending','issued','active');

-- 防抖会话持久化（契约第 6 节）
CREATE TABLE device_session (
  serial        TEXT PRIMARY KEY REFERENCES device_enrollment(serial),
  device_slot   TEXT NOT NULL,
  last_seen_seq BIGINT NOT NULL DEFAULT 0,
  state         TEXT NOT NULL DEFAULT 'online' CHECK (state IN ('online','offline')),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
