use std::{env, fmt::Display, net::SocketAddr, str::FromStr, time::Duration};

use anyhow::{Context, Result, bail};

/// 运行环境。`production` 下会拒绝示例密钥和通配 CORS。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}

impl Environment {
    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

const DEV_JWT_SECRET: &str = "dev-only-change-me-0123456789abcdef0123456789abcdef";
const DEV_ADMIN_PASSWORD: &str = "Admin@12345";

/// 运行期配置，全部来自环境变量（本地开发可由 `.env` 提供）。
#[derive(Clone)]
pub struct Config {
    pub environment: Environment,
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub db_max_connections: u32,
    pub redis_url: String,
    pub jwt_secret: String,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_secs: i64,
    pub request_timeout_secs: u64,
    pub max_body_bytes: usize,
    pub login_max_attempts: u32,
    pub login_window_secs: u64,
    pub login_lock_secs: u64,
    pub login_ip_max_attempts: u32,
    pub login_ip_window_secs: u64,
    pub email_rate_limit_max: u32,
    pub email_rate_limit_window_secs: u64,
    pub announcement_cache_secs: u64,
    pub public_base_url: String,
    pub password_reset_ttl_secs: u64,
    pub email_verification_ttl_secs: u64,
    pub email_driver: String,
    pub smtp_url: String,
    pub smtp_from: String,
    pub cors_allowed_origins: Vec<String>,
    pub run_migrations: bool,
    pub bootstrap_admin_username: Option<String>,
    pub bootstrap_admin_password: Option<String>,
    pub bootstrap_admin_display_name: String,
    pub log_format_json: bool,
    // ---- 课表事件引擎 ----
    pub schedule_engine_enabled: bool,
    pub schedule_tick_secs: u64,
    pub schedule_horizon_days: i64,
    pub schedule_rebuild_interval_secs: u64,
    pub schedule_pubsub_channel: String,
    pub schedule_downlink_notify_channel: String,
    pub schedule_webhook_timeout_secs: u64,
    pub schedule_webhook_max_attempts: u32,
    pub schedule_delivery_batch: i64,
    // ---- 设备 ----
    /// 设备注册服务对外地址，用于拼装注册链接（如 `https://node:4440`）。
    pub public_enroll_endpoint: String,
    // ---- 设备在线状态通知（device.offline / device.online webhook）----
    /// 是否启用在 qe-api 侧扫描 `device_session` 并投递状态变化 webhook。
    pub device_presence_enabled: bool,
    /// 扫描间隔（秒）。
    pub device_presence_tick_secs: u64,
    /// webhook 请求超时（秒）。
    pub device_presence_webhook_timeout_secs: u64,
    /// 是否同时通知 `device.online`（默认只通知 offline）。
    pub device_presence_notify_online: bool,
    /// 房间/区域都未配置 webhook 时的全局回退地址。
    pub device_presence_global_webhook_url: Option<String>,
    /// 单条投递最大尝试次数。
    pub device_presence_webhook_max_attempts: u32,
    /// 每轮领取投递上限。
    pub device_presence_webhook_batch: i64,
    /// 重试退避基数（秒）。
    pub device_presence_retry_base_seconds: u64,
    // ---- 对象存储（S3 兼容）----
    // ---- 机器遥测（room-agent）----
    /// 机器上报 Bearer token；`None` = 上报端点关闭（返回 503）。
    pub machine_agent_token: Option<String>,
    /// 每台机器保留的快照条数。
    pub machine_snapshot_keep: i64,
    /// 未配置 `S3_ENDPOINT` + `S3_BUCKET` 时为 None，存储/镜像接口返回 503。
    pub storage: Option<StorageConfig>,
}

/// S3 兼容存储配置（RustFS / MinIO / Ceph 等）。
#[derive(Clone)]
pub struct StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,
    pub presign_ttl_secs: u64,
    pub signing_key_base64: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let environment = parse_environment()?;
        let bind_addr = env_optional("BIND_ADDR")
            .unwrap_or_else(|| "0.0.0.0:8080".to_owned())
            .parse::<SocketAddr>()
            .context("BIND_ADDR 不是合法的监听地址")?;

        let database_url = env_required("DATABASE_URL")?;
        let redis_url =
            env_optional("REDIS_URL").unwrap_or_else(|| "redis://127.0.0.1:6379".to_owned());

        let jwt_secret = env_required("JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            bail!("JWT_SECRET 至少需要 32 个字符，建议使用 `openssl rand -hex 32` 生成");
        }

        let access_token_ttl_secs = env_parse("ACCESS_TOKEN_TTL_SECONDS", 15 * 60_i64)?;
        if access_token_ttl_secs <= 0 || access_token_ttl_secs > 24 * 60 * 60 {
            bail!("ACCESS_TOKEN_TTL_SECONDS 必须在 1 秒到 24 小时之间");
        }

        let refresh_token_ttl_secs = env_parse("REFRESH_TOKEN_TTL_SECONDS", 7 * 24 * 60 * 60_i64)?;
        if refresh_token_ttl_secs <= access_token_ttl_secs {
            bail!("REFRESH_TOKEN_TTL_SECONDS 必须大于 ACCESS_TOKEN_TTL_SECONDS");
        }

        let cors_allowed_origins = env_optional("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|| "http://localhost:5173".to_owned())
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        let bootstrap_admin_username = env_optional("BOOTSTRAP_ADMIN_USERNAME");
        let bootstrap_admin_password = env_optional("BOOTSTRAP_ADMIN_PASSWORD");
        if bootstrap_admin_password.is_some() && bootstrap_admin_username.is_none() {
            bail!("设置了 BOOTSTRAP_ADMIN_PASSWORD 但缺少 BOOTSTRAP_ADMIN_USERNAME");
        }

        let db_max_connections = env_parse("DB_MAX_CONNECTIONS", 10_u32)?;
        if db_max_connections == 0 {
            bail!("DB_MAX_CONNECTIONS 必须大于 0");
        }

        let login_max_attempts = env_parse("LOGIN_MAX_ATTEMPTS", 5_u32)?;
        let login_window_secs = env_parse("LOGIN_WINDOW_SECONDS", 300_u64)?;
        let login_lock_secs = env_parse("LOGIN_LOCK_SECONDS", 900_u64)?;
        let login_ip_max_attempts = env_parse("LOGIN_IP_MAX_ATTEMPTS", 60_u32)?;
        let login_ip_window_secs = env_parse("LOGIN_IP_WINDOW_SECONDS", 300_u64)?;
        let email_rate_limit_max = env_parse("EMAIL_RATE_LIMIT_MAX", 5_u32)?;
        let email_rate_limit_window_secs = env_parse("EMAIL_RATE_LIMIT_WINDOW_SECONDS", 900_u64)?;
        let announcement_cache_secs = env_parse("ANNOUNCEMENT_CACHE_SECONDS", 30_u64)?;
        let public_base_url = env_optional("PUBLIC_BASE_URL")
            .unwrap_or_else(|| "http://localhost:5173".to_owned())
            .trim_end_matches('/')
            .to_owned();
        let password_reset_ttl_secs = env_parse("PASSWORD_RESET_TTL_SECONDS", 1800_u64)?;
        let email_verification_ttl_secs = env_parse("EMAIL_VERIFICATION_TTL_SECONDS", 86_400_u64)?;
        let email_driver = env_optional("EMAIL_DRIVER").unwrap_or_else(|| "log".to_owned());
        let smtp_url =
            env_optional("SMTP_URL").unwrap_or_else(|| "smtp://127.0.0.1:1025".to_owned());
        let smtp_from = env_optional("SMTP_FROM")
            .unwrap_or_else(|| "QEStudio <no-reply@qestudio.local>".to_owned());
        if login_max_attempts == 0 || login_window_secs == 0 || login_lock_secs == 0 {
            bail!("LOGIN_MAX_ATTEMPTS / LOGIN_WINDOW_SECONDS / LOGIN_LOCK_SECONDS 必须大于 0");
        }
        if login_ip_max_attempts == 0 || login_ip_window_secs == 0 {
            bail!("LOGIN_IP_MAX_ATTEMPTS / LOGIN_IP_WINDOW_SECONDS 必须大于 0");
        }
        if email_rate_limit_max == 0 || email_rate_limit_window_secs == 0 {
            bail!("EMAIL_RATE_LIMIT_MAX / EMAIL_RATE_LIMIT_WINDOW_SECONDS 必须大于 0");
        }

        let schedule_tick_secs = env_parse("SCHEDULE_TICK_SECONDS", 15_u64)?;
        if !(5..=300).contains(&schedule_tick_secs) {
            bail!("SCHEDULE_TICK_SECONDS 需在 5-300 秒之间");
        }
        let schedule_horizon_days = env_parse("SCHEDULE_HORIZON_DAYS", 30_i64)?;
        if !(1..=366).contains(&schedule_horizon_days) {
            bail!("SCHEDULE_HORIZON_DAYS 需在 1-366 天之间");
        }
        let schedule_rebuild_interval_secs =
            env_parse("SCHEDULE_REBUILD_INTERVAL_SECONDS", 3600_u64)?;
        if !(60..=86_400).contains(&schedule_rebuild_interval_secs) {
            bail!("SCHEDULE_REBUILD_INTERVAL_SECONDS 需在 60-86400 秒之间");
        }
        let schedule_webhook_timeout_secs = env_parse("SCHEDULE_WEBHOOK_TIMEOUT_SECONDS", 5_u64)?;
        if !(1..=60).contains(&schedule_webhook_timeout_secs) {
            bail!("SCHEDULE_WEBHOOK_TIMEOUT_SECONDS 需在 1-60 秒之间");
        }
        let schedule_webhook_max_attempts = env_parse("SCHEDULE_WEBHOOK_MAX_ATTEMPTS", 5_u32)?;
        if !(1..=20).contains(&schedule_webhook_max_attempts) {
            bail!("SCHEDULE_WEBHOOK_MAX_ATTEMPTS 需在 1-20 之间");
        }
        let device_presence_enabled = env_parse_bool("DEVICE_PRESENCE_ENABLED", true)?;
        let device_presence_tick_secs = env_parse("DEVICE_PRESENCE_TICK_SECONDS", 5_u64)?;
        if !(2..=60).contains(&device_presence_tick_secs) {
            bail!("DEVICE_PRESENCE_TICK_SECONDS 需在 2-60 秒之间");
        }
        let device_presence_webhook_timeout_secs =
            env_parse("DEVICE_PRESENCE_WEBHOOK_TIMEOUT_SECONDS", 5_u64)?;
        if !(1..=60).contains(&device_presence_webhook_timeout_secs) {
            bail!("DEVICE_PRESENCE_WEBHOOK_TIMEOUT_SECONDS 需在 1-60 秒之间");
        }
        let device_presence_notify_online = env_parse_bool("DEVICE_PRESENCE_NOTIFY_ONLINE", false)?;
        let device_presence_global_webhook_url = env_optional("DEVICE_PRESENCE_GLOBAL_WEBHOOK_URL");
        let device_presence_webhook_max_attempts =
            env_parse("DEVICE_PRESENCE_WEBHOOK_MAX_ATTEMPTS", 5_u32)?;
        if !(1..=20).contains(&device_presence_webhook_max_attempts) {
            bail!("DEVICE_PRESENCE_WEBHOOK_MAX_ATTEMPTS 需在 1-20 之间");
        }
        let device_presence_webhook_batch = env_parse("DEVICE_PRESENCE_WEBHOOK_BATCH", 20_i64)?;
        if !(1..=200).contains(&device_presence_webhook_batch) {
            bail!("DEVICE_PRESENCE_WEBHOOK_BATCH 需在 1-200 之间");
        }
        let device_presence_retry_base_seconds =
            env_parse("DEVICE_PRESENCE_RETRY_BASE_SECONDS", 5_u64)?;
        if !(1..=60).contains(&device_presence_retry_base_seconds) {
            bail!("DEVICE_PRESENCE_RETRY_BASE_SECONDS 需在 1-60 之间");
        }
        let schedule_delivery_batch = env_parse("SCHEDULE_DELIVERY_BATCH", 50_i64)?;
        if !(1..=500).contains(&schedule_delivery_batch) {
            bail!("SCHEDULE_DELIVERY_BATCH 需在 1-500 之间");
        }

        let machine_agent_token =
            env_optional("MACHINE_AGENT_TOKEN").filter(|token| !token.is_empty());
        if let Some(token) = machine_agent_token.as_deref()
            && token.len() < 16
        {
            bail!("MACHINE_AGENT_TOKEN 至少需要 16 个字符");
        }
        let machine_snapshot_keep = env_parse("MACHINE_SNAPSHOT_KEEP", 100_i64)?;
        if !(10..=1000).contains(&machine_snapshot_keep) {
            bail!("MACHINE_SNAPSHOT_KEEP 需在 10-1000 之间");
        }

        let storage = parse_storage_config()?;

        reject_insecure_production_defaults(
            environment,
            &jwt_secret,
            bootstrap_admin_password.as_deref(),
            &database_url,
            &cors_allowed_origins,
        )?;

        Ok(Self {
            environment,
            bind_addr,
            database_url,
            db_max_connections,
            redis_url,
            jwt_secret,
            access_token_ttl_secs,
            refresh_token_ttl_secs,
            request_timeout_secs: env_parse("REQUEST_TIMEOUT_SECONDS", 30_u64)?,
            max_body_bytes: env_parse("MAX_BODY_BYTES", 2 * 1024 * 1024_usize)?,
            login_max_attempts,
            login_window_secs,
            login_lock_secs,
            login_ip_max_attempts,
            login_ip_window_secs,
            email_rate_limit_max,
            email_rate_limit_window_secs,
            announcement_cache_secs,
            public_base_url,
            password_reset_ttl_secs,
            email_verification_ttl_secs,
            email_driver,
            smtp_url,
            smtp_from,
            cors_allowed_origins,
            run_migrations: env_parse_bool("RUN_MIGRATIONS", true)?,
            bootstrap_admin_username,
            bootstrap_admin_password,
            bootstrap_admin_display_name: env_optional("BOOTSTRAP_ADMIN_DISPLAY_NAME")
                .unwrap_or_else(|| "Administrator".to_owned()),
            log_format_json: env_optional("LOG_FORMAT")
                .is_some_and(|value| value.eq_ignore_ascii_case("json")),
            schedule_engine_enabled: env_parse_bool("SCHEDULE_ENGINE_ENABLED", true)?,
            schedule_tick_secs,
            schedule_horizon_days,
            schedule_rebuild_interval_secs,
            schedule_pubsub_channel: env_optional("SCHEDULE_PUBSUB_CHANNEL")
                .unwrap_or_else(|| "qe-schedule:events".to_owned()),
            schedule_downlink_notify_channel: env_optional("SCHEDULE_DOWNLINK_NOTIFY_CHANNEL")
                .unwrap_or_else(|| "qe-device-pki:downlink".to_owned()),
            schedule_webhook_timeout_secs,
            schedule_webhook_max_attempts,
            schedule_delivery_batch,
            device_presence_enabled,
            device_presence_tick_secs,
            device_presence_webhook_timeout_secs,
            device_presence_notify_online,
            device_presence_global_webhook_url,
            device_presence_webhook_max_attempts,
            device_presence_webhook_batch,
            device_presence_retry_base_seconds,
            machine_agent_token,
            machine_snapshot_keep,
            public_enroll_endpoint: env_optional("PUBLIC_ENROLL_ENDPOINT")
                .unwrap_or_else(|| "https://localhost:4440".to_owned())
                .trim_end_matches('/')
                .to_owned(),
            storage,
        })
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }

    /// 读取工单自动建单 worker 的配置。
    ///
    /// 这些开关只影响后台 worker，因此独立成 [`TicketWorkerConfig`]（不进入核心
    /// `Config` 结构体），避免为了新增后台任务改动所有既有 `Config` 构造点。
    /// 解析/范围校验与其它 worker 配置保持一致：启动时失败即退出。
    pub fn ticket_worker_config(&self) -> Result<TicketWorkerConfig> {
        TicketWorkerConfig::from_env()
    }

    /// 工单邮件通知开关（`TICKET_EMAIL_ENABLED`，默认 true）。
    /// 供 API handler 做转派通知；worker 侧直接使用 [`TicketWorkerConfig`]。
    pub fn ticket_email_enabled(&self) -> bool {
        env_parse_bool("TICKET_EMAIL_ENABLED", true).unwrap_or(true)
    }
}

/// 工单自动建单 worker 配置（环境变量见 `.env.example`）。
#[derive(Clone, Debug)]
pub struct TicketWorkerConfig {
    /// `TICKETS_AUTO_ENABLED`，默认 `true`。
    pub auto_enabled: bool,
    /// `TICKET_TICK_SECONDS`，默认 60（10..=600）。
    pub tick_secs: u64,
    /// `TICKET_DEVICE_OFFLINE_MINUTES`，默认 10（1..=1440）。
    pub device_offline_minutes: i64,
    /// `TICKET_HEALTH_CONSECUTIVE`，默认 2（1..=10）。
    pub health_consecutive: i64,
    /// `TICKET_LOOKBACK_HOURS`，默认 24（1..=168）。
    pub lookback_hours: i64,
    /// `TICKET_EMAIL_ENABLED`，默认 true（无 SMTP 时走 log driver）。
    pub email_enabled: bool,
    /// `TICKET_OVERDUE_REMIND_HOURS`，默认 24（1..=168）。
    pub overdue_remind_hours: i64,
}

impl TicketWorkerConfig {
    pub fn from_env() -> Result<Self> {
        let auto_enabled = env_parse_bool("TICKETS_AUTO_ENABLED", true)?;
        let tick_secs = env_parse("TICKET_TICK_SECONDS", 60_u64)?;
        if !(10..=600).contains(&tick_secs) {
            bail!("TICKET_TICK_SECONDS 需在 10-600 秒之间");
        }
        let device_offline_minutes = env_parse("TICKET_DEVICE_OFFLINE_MINUTES", 10_i64)?;
        if !(1..=1440).contains(&device_offline_minutes) {
            bail!("TICKET_DEVICE_OFFLINE_MINUTES 需在 1-1440 分钟之间");
        }
        let health_consecutive = env_parse("TICKET_HEALTH_CONSECUTIVE", 2_i64)?;
        if !(1..=10).contains(&health_consecutive) {
            bail!("TICKET_HEALTH_CONSECUTIVE 需在 1-10 之间");
        }
        let lookback_hours = env_parse("TICKET_LOOKBACK_HOURS", 24_i64)?;
        if !(1..=168).contains(&lookback_hours) {
            bail!("TICKET_LOOKBACK_HOURS 需在 1-168 小时之间");
        }
        let email_enabled = env_parse_bool("TICKET_EMAIL_ENABLED", true)?;
        let overdue_remind_hours = env_parse("TICKET_OVERDUE_REMIND_HOURS", 24_i64)?;
        if !(1..=168).contains(&overdue_remind_hours) {
            bail!("TICKET_OVERDUE_REMIND_HOURS 需在 1-168 小时之间");
        }
        Ok(Self {
            auto_enabled,
            tick_secs,
            device_offline_minutes,
            health_consecutive,
            lookback_hours,
            email_enabled,
            overdue_remind_hours,
        })
    }
}

fn parse_environment() -> Result<Environment> {
    match env_optional("APP_ENV") {
        None => Ok(Environment::Development),
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "development" | "dev" | "local" => Ok(Environment::Development),
            "production" | "prod" => Ok(Environment::Production),
            _ => bail!("APP_ENV 只能是 development 或 production"),
        },
    }
}

/// 解析 S3 配置：`S3_ENDPOINT` 与 `S3_BUCKET` 同时存在才启用。
fn parse_storage_config() -> Result<Option<StorageConfig>> {
    let endpoint = env_optional("S3_ENDPOINT");
    let bucket = env_optional("S3_BUCKET");
    let storage = match (endpoint, bucket) {
        (None, None) => None,
        (Some(endpoint), Some(bucket)) => {
            let presign_ttl_secs = env_parse("STORAGE_PRESIGN_TTL_SECONDS", 900_u64)?;
            if !(60..=86_400).contains(&presign_ttl_secs) {
                bail!("STORAGE_PRESIGN_TTL_SECONDS 需在 60-86400 秒之间");
            }
            Some(StorageConfig {
                endpoint: endpoint.trim_end_matches('/').to_owned(),
                region: env_optional("S3_REGION").unwrap_or_else(|| "us-east-1".to_owned()),
                bucket,
                access_key: env_optional("S3_ACCESS_KEY").unwrap_or_default(),
                secret_key: env_optional("S3_SECRET_KEY").unwrap_or_default(),
                path_style: env_parse_bool("S3_PATH_STYLE", true)?,
                presign_ttl_secs,
                signing_key_base64: env_optional("STORAGE_SIGNING_KEY_BASE64"),
            })
        }
        _ => bail!("S3_ENDPOINT 与 S3_BUCKET 必须同时设置（或都不设置以禁用对象存储）"),
    };
    Ok(storage)
}

fn reject_insecure_production_defaults(
    environment: Environment,
    jwt_secret: &str,
    admin_password: Option<&str>,
    database_url: &str,
    cors_allowed_origins: &[String],
) -> Result<()> {
    if !environment.is_production() {
        return Ok(());
    }
    if jwt_secret == DEV_JWT_SECRET || jwt_secret.starts_with("dev-only-") {
        bail!("生产环境不能使用示例 JWT_SECRET，请用 `openssl rand -hex 32` 生成");
    }
    if admin_password == Some(DEV_ADMIN_PASSWORD) {
        bail!("生产环境不能使用示例 BOOTSTRAP_ADMIN_PASSWORD，请改为强密码");
    }
    if database_url_uses_default_password(database_url) {
        bail!("生产环境不能使用与用户名相同的数据库密码");
    }
    if cors_allowed_origins.iter().any(|origin| origin == "*") {
        bail!("生产环境不能把 CORS_ALLOWED_ORIGINS 设为 *");
    }
    Ok(())
}

/// 识别 `postgres://user:user@host` 这类示例连接串（用户名与密码相同）。
fn database_url_uses_default_password(database_url: &str) -> bool {
    let Some(authority) = database_url.split("://").nth(1) else {
        return false;
    };
    let Some(userinfo) = authority.split('@').next() else {
        return false;
    };
    match userinfo.split_once(':') {
        Some((user, password)) => !user.is_empty() && user == password,
        None => false,
    }
}

fn env_required(key: &str) -> Result<String> {
    env_optional(key).with_context(|| format!("缺少必需的环境变量 {key}"))
}

fn env_optional(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: Display,
{
    match env_optional(key) {
        Some(raw) => raw
            .parse::<T>()
            .map_err(|error| anyhow::anyhow!("环境变量 {key} 的值 {raw:?} 无法解析: {error}")),
        None => Ok(default),
    }
}

fn env_parse_bool(key: &str, default: bool) -> Result<bool> {
    match env_optional(key) {
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => bail!("环境变量 {key} 的值 {raw:?} 不是合法布尔值"),
        },
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origins(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn production_rejects_dev_jwt_secret() {
        let result = reject_insecure_production_defaults(
            Environment::Production,
            DEV_JWT_SECRET,
            Some("Long-enough-password-123"),
            "postgres://app:strong-db-password@db:5432/app",
            &origins(&["https://app.example.com"]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn production_rejects_dev_admin_password() {
        let result = reject_insecure_production_defaults(
            Environment::Production,
            "0123456789abcdef0123456789abcdef",
            Some(DEV_ADMIN_PASSWORD),
            "postgres://app:strong-db-password@db:5432/app",
            &origins(&["https://app.example.com"]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn production_rejects_wildcard_cors() {
        let result = reject_insecure_production_defaults(
            Environment::Production,
            "0123456789abcdef0123456789abcdef",
            Some("Long-enough-password-123"),
            "postgres://app:strong-db-password@db:5432/app",
            &origins(&["*"]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn production_accepts_explicit_settings() {
        let result = reject_insecure_production_defaults(
            Environment::Production,
            "0123456789abcdef0123456789abcdef",
            Some("Long-enough-password-123"),
            "postgres://app:strong-db-password@db:5432/app",
            &origins(&["https://app.example.com"]),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn production_rejects_default_database_password() {
        let result = reject_insecure_production_defaults(
            Environment::Production,
            "0123456789abcdef0123456789abcdef",
            Some("Long-enough-password-123"),
            "postgres://acme:acme@db:5432/acme",
            &origins(&["https://app.example.com"]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn development_allows_examples() {
        let result = reject_insecure_production_defaults(
            Environment::Development,
            DEV_JWT_SECRET,
            Some(DEV_ADMIN_PASSWORD),
            "postgres://acme:acme@db:5432/acme",
            &origins(&["*"]),
        );
        assert!(result.is_ok());
    }
}
