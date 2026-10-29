//! Redis 适配器：刷新令牌与 RBAC 权限缓存。
//!
//! 端口 trait 在 `qe-auth`，`Cache` 在本 crate，所以端口实现放在 `auth_ports`。
//! 本 crate 不依赖 PostgreSQL，也不认识 `AppState`。

mod auth_ports;
mod cache;
mod error;

pub use cache::Cache;
pub use error::{CacheError, CacheResult};
