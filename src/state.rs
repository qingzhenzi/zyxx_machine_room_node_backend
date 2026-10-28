use std::sync::Arc;

use sqlx::PgPool;

use crate::{
    auth::jwt::JwtKeys,
    config::Config,
    infra::{announcements::AnnouncementRepo, cache::Cache, roles::RoleRepo, users::UserRepo},
};

/// 所有 handler 共享的应用状态。各 Repo 内部持有连接池克隆，成本很低。
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub cache: Cache,
    pub users: UserRepo,
    pub roles: RoleRepo,
    pub announcements: AnnouncementRepo,
    pub jwt: Arc<JwtKeys>,
    pub config: Arc<Config>,
    /// 用于在 `/healthz` 中区分负载均衡后的不同副本。
    pub instance_id: String,
}

impl AppState {
    pub fn new(db: PgPool, cache: Cache, config: Config) -> Self {
        let jwt = Arc::new(JwtKeys::new(&config.jwt_secret));
        let instance_id =
            std::env::var("HOSTNAME").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
        Self {
            users: UserRepo::new(db.clone()),
            roles: RoleRepo::new(db.clone()),
            announcements: AnnouncementRepo::new(db.clone()),
            db,
            cache,
            jwt,
            config: Arc::new(config),
            instance_id,
        }
    }
}
