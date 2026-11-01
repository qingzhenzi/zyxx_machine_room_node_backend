use std::sync::Arc;

use qe_storage::{StorageOptions, StorageService};

use crate::{
    auth::jwt::JwtKeys,
    cache::Cache,
    config::{Config, StorageConfig},
    db::{
        Db,
        agent_config::AgentConfigRepo,
        announcements::AnnouncementRepo,
        audit::AuditLogRepo,
        device::DeviceRepo,
        machine::MachineRepo,
        presence::{PresenceEventRecord, PresenceRepo},
        roles::RoleRepo,
        schedule::{AreaRepo, EventRuleRepo, RoomRepo, ScheduleEventRepo, ScheduleRepo},
        security::SecurityRepo,
        storage::StorageRepo,
        users::UserRepo,
    },
};

/// 本地事件广播缓冲：慢消费者落后超过该数量会收到 `Lagged`，SSE 侧应回查数据库。
const EVENT_CHANNEL_CAPACITY: usize = 256;

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
    // ---- 课表与事件 ----
    pub areas: AreaRepo,
    pub rooms: RoomRepo,
    pub schedules: ScheduleRepo,
    pub event_rules: EventRuleRepo,
    pub schedule_events: ScheduleEventRepo,
    // ---- 设备 ----
    pub devices: DeviceRepo,
    // ---- 机器遥测（room-agent 上报）----
    pub machines: MachineRepo,
    // ---- 设备在线事件与可靠投递（P3）----
    pub presence: PresenceRepo,
    /// 事件落库后的本地广播（SSE 实时推送；跨副本仍以查库为准）。
    pub presence_events: tokio::sync::broadcast::Sender<PresenceEventRecord>,
    // ---- 节点配置（P2：global → area → room）----
    pub agent_configs: AgentConfigRepo,
    // ---- 资源与镜像（S3 兼容对象存储）----
    /// 未配置 `S3_ENDPOINT` + `S3_BUCKET`（或初始化失败）时为 `None`，
    /// 存储/镜像接口统一返回 503。
    pub storage: Option<Arc<StorageService>>,
    pub storage_repo: StorageRepo,
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
        let storage = build_storage(config.storage.as_ref());
        let (presence_events, _) = tokio::sync::broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            users: UserRepo::new(db.pool().clone()),
            roles: RoleRepo::new(db.pool().clone()),
            announcements: AnnouncementRepo::new(db.pool().clone()),
            audit: AuditLogRepo::new(db.pool().clone()),
            security: SecurityRepo::new(db.pool().clone()),
            areas: AreaRepo::new(db.pool().clone()),
            rooms: RoomRepo::new(db.pool().clone()),
            schedules: ScheduleRepo::new(db.pool().clone()),
            event_rules: EventRuleRepo::new(db.pool().clone()),
            schedule_events: ScheduleEventRepo::new(db.pool().clone()),
            devices: DeviceRepo::new(db.pool().clone()),
            machines: MachineRepo::new(db.pool().clone()),
            presence: PresenceRepo::new(db.pool().clone()),
            presence_events,
            agent_configs: AgentConfigRepo::new(db.pool().clone()),
            storage,
            storage_repo: StorageRepo::new(db.pool().clone()),
            email,
            db,
            cache,
            jwt,
            config: Arc::new(config),
            instance_id,
        }
    }
}

/// 由配置构建 S3 适配器。
///
/// 初始化失败（端点非法、签名密钥格式错误等）只记录错误并返回 `None`，
/// 让 API 进程仍能启动；相关接口会返回 503，便于在不影响其它功能的情况下修复配置。
fn build_storage(storage: Option<&StorageConfig>) -> Option<Arc<StorageService>> {
    let config = storage?;
    let options = StorageOptions {
        endpoint: config.endpoint.clone(),
        region: config.region.clone(),
        bucket: config.bucket.clone(),
        access_key: config.access_key.clone(),
        secret_key: config.secret_key.clone(),
        path_style: config.path_style,
        presign_ttl_secs: config.presign_ttl_secs,
        signing_key_base64: config.signing_key_base64.clone(),
    };
    match StorageService::new(options) {
        Ok(service) => {
            tracing::info!(
                bucket = %config.bucket,
                endpoint = %config.endpoint,
                "对象存储已启用"
            );
            Some(Arc::new(service))
        }
        Err(error) => {
            tracing::error!(%error, "初始化对象存储失败，存储/镜像接口将返回 503");
            None
        }
    }
}
