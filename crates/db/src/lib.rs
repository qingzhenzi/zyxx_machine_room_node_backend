//! PostgreSQL 适配器：连接池、迁移、SQLx 行模型与仓储。
//!
//! 业务实体来自 `qe-domain`；HTTP 错误映射在 `backend`。本 crate 只负责 SQL 与
//! 行到实体的转换，以及 `qe-auth` 权限来源端口的实现。

pub mod agent_config;
pub mod announcements;
pub mod audit;
pub mod auth_ports;
mod db;
pub mod device;
pub mod error;
pub mod roles;
pub mod rows;
pub mod schedule;
pub mod security;
pub mod storage;
pub mod users;

pub use db::Db;
pub use error::{DbError, DbResult};
