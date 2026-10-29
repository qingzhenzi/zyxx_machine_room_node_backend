use std::sync::Arc;

use crate::{
    auth::jwt::JwtKeys,
    cache::Cache,
    config::Config,
    db::{
        Db, announcements::AnnouncementRepo, audit::AuditLogRepo, roles::RoleRepo,
        security::SecurityRepo, users::UserRepo,
    },
};

/// 所有 handler 共享的应用状态。各 Repo 内部持有连接池克隆，成本很低。
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub cache: Cache,
    pub users: UserRepo,
    pub roles: RoleRepo,
    pub announcements: AnnouncementRepo,
    pub audit: AuditLogRepo,
    pub security: SecurityRepo,
    pub email: crate::email::EmailService,
    pub jwt: Arc<JwtKeys>,
    pub config: Arc<Config>,
    /// 用于在 `/healthz` 中区分负载均衡后的不同副本。
    pub instance_id: String,
}

impl AppState {
    pub fn new(db: Db, cache: Cache, config: Config) -> Self {
        let jwt = Arc::new(JwtKeys::new(&config.jwt_secret));
        let email = crate::email::EmailService::new(&config);
        let instance_id =
            std::env::var("HOSTNAME").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
        Self {
            users: UserRepo::new(db.pool().clone()),
            roles: RoleRepo::new(db.pool().clone()),
            announcements: AnnouncementRepo::new(db.pool().clone()),
            audit: AuditLogRepo::new(db.pool().clone()),
            security: SecurityRepo::new(db.pool().clone()),
            email,
            db,
            cache,
            jwt,
            config: Arc::new(config),
            instance_id,
        }
    }
}
