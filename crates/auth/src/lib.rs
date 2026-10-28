//! QEStudio 的认证原语。
//!
//! 这个 crate 只依赖算法与存储抽象，**不认识** Axum、PostgreSQL、业务实体和
//! `AppState`。存储侧通过 [`session::TokenStore`] 与 [`permission::PermissionCache`]、
//! [`permission::PermissionSource`] 三个端口接入，具体实现留在 `backend`。
//!
//! 这样做的目的不是「为了拆而拆」，而是让认证代码无法反向依赖业务代码：
//! 哪天真要把它独立成服务，替换端口实现即可，不需要重写内部逻辑。
//!
//! 不在这里的东西：HTTP extractor、`IntoResponse`、用户仓储、RBAC 权限点常量。
//! 权限点是业务约定（`announcement:write` 这类），属于后端；这里只负责判定。

// `async fn` in trait 在这里是安全的：所有实现都在同进程、单线程切换，
// 且端口只用作泛型参数，不做 trait object。
#![allow(async_fn_in_trait)]

pub mod error;
pub mod jwt;
pub mod password;
pub mod permission;
pub mod session;

pub use error::{AuthError, AuthResult};
