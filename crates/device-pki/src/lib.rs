//! 设备 PKI：一次性 token 注册、证书签发/吊销、mTLS 防抖会话。
//!
//! 协议契约见仓库根 `docs/device-pki-protocol.md`。
//!
//! 两个 listener：
//! - 注册 listener（单向 TLS）：`POST /enroll/challenge`、`POST /enroll`、
//!   `GET /enroll/{id}`、`POST /admin/tokens`、`GET /admin/enrollments`、
//!   `DELETE /admin/enrollments/{id}`、`POST /admin/enroll/{id}/decide`、`GET /crl`
//! - 业务 listener（mTLS）：`POST /enroll/confirm`、`POST /messages`、
//!   `GET /agent/config`（节点配置，契约 §17.3）、
//!   `POST /resources/presign`（对象存储预签名，契约 §16.2）、
//!   `POST /machine/p2p/seed` / `GET /machine/p2p/peers`（P2P 镜像分发种子）、
//!   `POST /machine/command/result`（机器命令结果回传）、
//!   `POST /machine/images/presign-upload` / `POST /machine/images/register`（VM 导出镜像上传/登记，
//!   契约 `docs/vm-base-image-and-diff-disk.md` §8.5）、
//!   `GET /vnc/{serial}`（noVNC agent 隧道）、`POST /revoke`
//! - 内部明文 listener（`INTERNAL_ADDR`，默认 127.0.0.1:4442）：`GET /internal/vnc/{session}`（qe-api 中继）

pub mod agent_config;
pub mod ca;
pub mod cert_status;
pub mod challenge;
pub mod channel;
pub mod command;
pub mod error;
pub mod images;
pub mod messaging;
#[cfg(feature = "redis-notify")]
pub mod notify_redis;
pub mod p2p;
pub mod pop;
pub mod ratelimit;
pub mod resource;
pub mod routes;
pub mod session;
pub mod tls;
pub mod token;
pub mod transfer;
pub mod vnc;

pub use error::{Error, Result};
