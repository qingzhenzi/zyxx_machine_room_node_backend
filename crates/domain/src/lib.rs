//! 领域层：纯模型、RBAC 权限点与输入校验。
//!
//! 这里刻意不依赖 Axum、SQLx、Redis。数据库行模型在 `qe-db`，缓存适配器在 `qe-cache`；
//! HTTP 状态码与错误响应在 `backend`。权限匹配算法来自 `qe-auth`，本 crate 只保留
//! 业务侧权限点常量。

pub mod agent_config;
pub mod device;
pub mod error;
pub mod machine;
pub mod models;
pub mod rbac;
pub mod schedule;
pub mod storage;
pub mod validation;
