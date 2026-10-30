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
        })
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
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
