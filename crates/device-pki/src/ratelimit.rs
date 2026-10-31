//! 管理端限速（按对端 IP 的固定窗口）。
//!
//! `/admin/...` 用的是静态 `ADMIN_TOKEN` bearer：没有账号锁定、没有 MFA、
//! 没有失败计数，唯一的防线就是「不能无限试」。这里给管理端点套一层
//! 每 IP 每分钟的请求预算。
//!
//! 只统计管理面：注册面（challenge/enroll）靠 256 bit 随机 token 自身的熵，
//! 暴力猜解不可行；给它限速反而会误伤重试的正常设备。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 默认预算：每 IP 每分钟 30 次管理请求。
pub const DEFAULT_LIMIT: u32 = 30;
/// 窗口长度。
pub const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
struct Window {
    start: Instant,
    count: u32,
}

/// 固定窗口限速器。`limit = 0` 表示不限速（显式关闭）。
#[derive(Debug)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, Window>>,
    limit: u32,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}

impl RateLimiter {
    pub fn new(limit: u32) -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            limit,
        }
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// 记一次请求。`Ok(())` 放行；`Err(retry_after_secs)` 表示本窗口已超限。
    pub fn check(&self, key: &str) -> std::result::Result<(), u64> {
        if self.limit == 0 {
            return Ok(());
        }
        let now = Instant::now();
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());

        // 蠕虫/扫描器会造出海量来源，别让这张表无界增长
        if map.len() > 10_000 {
            map.retain(|_, w| now.duration_since(w.start) < WINDOW);
        }

        let win = map.entry(key.to_owned()).or_insert(Window {
            start: now,
            count: 0,
        });
        if now.duration_since(win.start) >= WINDOW {
            *win = Window {
                start: now,
                count: 0,
            };
        }
        win.count += 1;

        if win.count > self.limit {
            let elapsed = now.duration_since(win.start);
            let retry = WINDOW.saturating_sub(elapsed).as_secs() + 1;
            return Err(retry);
        }
        Ok(())
    }
}

/// 限速 key：优先用对端地址。没有对端地址（单测里 oneshot 打路由）时
/// 用固定值，避免"看不到地址就绕过限速"。
pub fn key_for(addr: Option<&SocketAddr>) -> String {
    match addr {
        Some(a) => a.ip().to_string(),
        None => "unknown".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(limit: u32) -> RateLimiter {
        RateLimiter::new(limit)
    }

    #[test]
    fn allows_up_to_limit_then_rejects() {
        let l = limiter(3);
        assert!(l.check("1.2.3.4").is_ok());
        assert!(l.check("1.2.3.4").is_ok());
        assert!(l.check("1.2.3.4").is_ok());
        let err = l.check("1.2.3.4").expect_err("第 4 次应被限");
        assert!(err >= 1 && err <= WINDOW.as_secs() + 1, "retry={err}");
    }

    #[test]
    fn budget_is_per_key() {
        let l = limiter(1);
        assert!(l.check("1.1.1.1").is_ok());
        assert!(l.check("2.2.2.2").is_ok(), "不同 IP 各有各的预算");
        assert!(l.check("1.1.1.1").is_err());
    }

    #[test]
    fn zero_limit_disables() {
        let l = limiter(0);
        for _ in 0..1000 {
            assert!(l.check("x").is_ok());
        }
        assert!(l.windows.lock().unwrap().is_empty(), "不限速时不应记账");
    }

    #[test]
    fn expired_window_resets() {
        let l = limiter(1);
        assert!(l.check("k").is_ok());
        assert!(l.check("k").is_err());
        // 手动把窗口拨到过期，验证重置逻辑
        {
            let mut map = l.windows.lock().unwrap();
            let w = map.get_mut("k").unwrap();
            w.start -= WINDOW;
        }
        assert!(l.check("k").is_ok(), "新窗口应重新计数");
    }

    #[test]
    fn key_for_uses_ip_not_port() {
        let a: SocketAddr = "203.0.113.9:54321".parse().unwrap();
        assert_eq!(key_for(Some(&a)), "203.0.113.9");
        assert_eq!(key_for(None), "unknown");
    }
}
