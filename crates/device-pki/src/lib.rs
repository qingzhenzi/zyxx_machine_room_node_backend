//! 设备 PKI：一次性 token 注册、证书签发/吊销、mTLS 防抖会话。
//!
//! 协议契约见仓库根 `docs/device-pki-protocol.md`。
//!
//! 两个 listener：
//! - 注册 listener（单向 TLS）：`POST /enroll/challenge`、`POST /enroll`、
//!   `GET /enroll/{id}`、`POST /admin/tokens`、`GET /admin/enrollments`、
//!   `DELETE /admin/enrollments/{id}`、`POST /admin/enroll/{id}/decide`、`GET /crl`
//! - 业务 listener（mTLS）：`POST /enroll/confirm`、`POST /messages`、`POST /revoke`

pub mod ca;
pub mod cert_status;
pub mod challenge;
pub mod error;
pub mod messaging;
pub mod pop;
pub mod ratelimit;
pub mod routes;
pub mod session;
pub mod tls;
pub mod token;

pub use error::{Error, Result};
