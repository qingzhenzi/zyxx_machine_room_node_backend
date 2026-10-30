//! 设备 PKI 客户端 CLI。
//!
//! ```text
//! qe-device-pki-client enroll "https://node:4440/enroll?token=T&slot=S" [--wait] [--trust]
//! qe-device-pki-client confirm [--slot S]
//! qe-device-pki-client ping    [--slot S]
//! qe-device-pki-client status  [--slot S]
//! ```

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use qe_device_pki_client::creds::{CredIssue, Credentials};
use qe_device_pki_client::enroll::{self, EnrollOptions, EnrollOutcome};
use qe_device_pki_client::transport::{
    self, BackoffPolicy, ConnectEvent, OutboxQueue, PendingMessage, TransportConfig,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "qe-device-pki-client", version, about = "设备 PKI 注册与 mTLS 通讯客户端")]
struct Cli {
    /// 覆盖数据目录（默认 ~/.local/share/qe-device-pki-client）
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    /// 节点槽位名
    #[arg(long, global = true)]
    slot: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// 用注册链接完成一次性注册
    Enroll {
        /// 注册链接（含 token 与 slot）
        link: String,
        /// approval 型：等待管理员批准（默认最多 5 分钟）
        #[arg(long)]
        wait: bool,
        /// 首次连接：人工确认服务端证书（TOFU）
        #[arg(long)]
        trust: bool,
        /// 批准等待超时（秒）
        #[arg(long, default_value_t = 300)]
        wait_secs: u64,
    },
    /// 用正式证书完成 mTLS 回连确认（通讯真正打通）
    Confirm,
    /// 续跑一次被中断的 approval 注册（用落盘的 pending 状态，不需要原链接）
    Poll {
        /// 最长等待管理员批准的时间（秒）
        #[arg(long, default_value_t = 300)]
        wait_secs: u64,
    },
    /// 心跳 + 防抖重连演示
    Ping {
        /// 心跳次数（0 = 无限）
        #[arg(long, default_value_t = 3)]
        count: u32,
    },
    /// 查看本地凭证状态
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 依赖树里同时存在 aws-lc-rs 与 ring 时无法自动选 provider，必须显式安装
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let data_dir = match &cli.data_dir {
        Some(d) => d.clone(),
        None => qe_device_pki_client::trust::default_data_dir()?,
    };
    std::fs::create_dir_all(&data_dir).context("创建数据目录")?;

    match cli.cmd {
        Cmd::Enroll {
            link,
            wait,
            trust,
            wait_secs,
        } => {
            // 链接里含一次性 token，走 argv 会留在 shell history / ps 里。
            // 传 `-` 表示从 stdin 读（配合脚本或剪贴板管道）。
            let link = if link == "-" {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .context("从 stdin 读取注册链接")?;
                buf.trim().to_owned()
            } else {
                link
            };
            println!("注册链接: {}", enroll::link_summary(&link)?);
            let opts = EnrollOptions {
                approval_wait: if wait {
                    Some(Duration::from_secs(wait_secs))
                } else {
                    None
                },
                trust_confirmed: trust,
                data_dir: Some(data_dir.clone()),
            };

            match enroll::enroll_link(&link, &opts).await? {
                EnrollOutcome::Issued(creds) => {
                    println!("✓ 注册成功");
                    println!("  slot   : {}", creds.device_slot);
                    println!("  serial : {}", creds.serial);
                    let dir = enroll::creds_dir(&creds.device_slot, Some(&data_dir))?;
                    println!("  凭证   : {}", dir.display());
                    println!();
                    println!("下一步：`qe-device-pki-client confirm --slot {}` 完成 mTLS 验证", creds.device_slot);
                }
                EnrollOutcome::PendingApproval { enroll_id } => {
                    let slot = enroll::EnrollTarget::parse(&link)
                        .map(|t| t.slot)
                        .unwrap_or_else(|_| "<slot>".into());
                    println!("◷ 等待管理员批准，enroll_id = {enroll_id}");
                    println!("  私钥与注册单已落盘（pending/），批准或中断后续跑即可：");
                    println!("    qe-device-pki-client poll --slot {slot} --wait-secs 300");
                }
            }
        }
        Cmd::Confirm => {
            let cfg = load_transport(&data_dir, cli.slot.as_deref())?;
            println!("→ mTLS confirm {} ...", cfg.mtls_endpoint);
            let reply = transport::confirm(&cfg).await?;
            println!("✓ 通讯已打通");
            println!("  status : {}", reply.status);
            println!("  serial : {}", reply.serial);
            println!("  mtls   : {}", reply.mtls);
        }
        Cmd::Poll { wait_secs } => {
            let slot = match cli.slot.clone() {
                Some(s) => s,
                None => find_any_pending(&data_dir)?,
            };
            let opts = EnrollOptions {
                approval_wait: Some(Duration::from_secs(wait_secs)),
                trust_confirmed: false,
                data_dir: Some(data_dir.clone()),
            };
            println!("→ 续跑 {slot} 的审批轮询（最长 {wait_secs}s）");
            let creds = enroll::resume_poll(&slot, Some(&data_dir), &opts).await?;
            println!("✓ 注册成功（续跑）");
            println!("  slot   : {}", creds.device_slot);
            println!("  serial : {}", creds.serial);
            println!("  凭证   : {}", enroll::creds_dir(&creds.device_slot, Some(&data_dir))?.display());
            println!("下一步：`qe-device-pki-client confirm --slot {}` 完成 mTLS 验证", creds.device_slot);
        }
        Cmd::Ping { count } => {
            let cfg = load_transport(&data_dir, cli.slot.as_deref())?;
            let outbox = Arc::new(OutboxQueue::open(&data_dir.join("outbox"))?);
            let (tx, mut rx) = tokio::sync::mpsc::channel::<ConnectEvent>(64);

            let log_task = tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    match ev {
                        ConnectEvent::Connected => println!("● 已连接"),
                        ConnectEvent::Disconnected { reason, attempt } => {
                            println!("○ 断开（第 {attempt} 次退避）: {reason}")
                        }
                        ConnectEvent::Replaying { count } => {
                            println!("↻ 重放未 ACK 消息 {count} 条")
                        }
                        ConnectEvent::Replayed { count } => println!("✓ 重放完成 {count} 条"),
                    }
                }
            });

            // 用 confirm 作为连通性探针；每次成功后 run_with_reconnect 会按心跳间隔再探
            let cfg_all = Arc::new(cfg);
            let heartbeat = cfg_all.heartbeat;
            let iterations = if count == 0 { u64::MAX } else { count as u64 };
            let mut done = 0u64;

            // 生产者：每次心跳入队一条消息（先落盘，发送由重连循环负责 —— 契约 §6 的
            // "写 sled 后才发送，收到 ACK 才删除"）
            let producer_outbox = Arc::clone(&outbox);
            let producer = tokio::spawn(async move {
                let mut ticker = tokio::time::interval(heartbeat);
                loop {
                    ticker.tick().await;
                    let seq = match producer_outbox.next_seq() {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("✗ 取 seq 失败: {e}");
                            continue;
                        }
                    };
                    let epoch = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let msg = PendingMessage {
                        seq,
                        kind: "heartbeat".into(),
                        payload: serde_json::json!({ "at": epoch }).to_string(),
                    };
                    if let Err(e) = producer_outbox.enqueue(&msg) {
                        eprintln!("✗ 消息入队失败: {e}");
                    }
                }
            });

            let probe_cfg = Arc::clone(&cfg_all);
            let send_cfg = Arc::clone(&cfg_all);
            let handle = tokio::spawn(transport::run_with_reconnect(
                BackoffPolicy::default(),
                Arc::clone(&outbox),
                move || {
                    let c = Arc::clone(&probe_cfg);
                    // 不在这里 sleep：run_with_reconnect 已按心跳间隔 pacing，
                    // 再塞一个 std::thread::sleep 只会阻塞 tokio worker 线程
                    async move { transport::confirm(&c).await.map(|_| ()) }
                },
                move |msg: PendingMessage| {
                    let c = Arc::clone(&send_cfg);
                    async move {
                        let ack = transport::send_message(&c, &msg).await?;
                        if ack.duplicate {
                            tracing::debug!(seq = ack.seq, "服务端判为重复（重放幂等，正常）");
                        }
                        Ok(())
                    }
                },
                Some(tx),
            ));

            // 心跳：按间隔打印，达到 count 次退出
            let mut ticker = tokio::time::interval(heartbeat);
            loop {
                ticker.tick().await;
                if done >= iterations {
                    break;
                }
                done += 1;
                println!("♥ 心跳 #{done}（待 ACK {} 条）", outbox.len());
            }
            handle.abort();
            producer.abort();
            log_task.abort();
            println!("✓ 心跳结束，未 ACK 消息剩余 {} 条", outbox.len());
        }
        Cmd::Status => {
            let creds_dir = match cli.slot.as_deref() {
                Some(s) => enroll::creds_dir(s, Some(&data_dir))?,
                None => find_any_creds(&data_dir)?,
            };
            if !Credentials::exists(&creds_dir) {
                let issue = CredIssue::NotEnrolled;
                println!("○ {}", issue);
                println!();
                println!("{}", issue.advice());
                std::process::exit(2);
            }
            let creds = Credentials::load(&creds_dir)?;
            println!("✓ 已注册");
            println!("  slot     : {}", creds.device_slot);
            println!("  serial   : {}", creds.serial);
            println!("  endpoint : {}", creds.endpoint);
            println!("  目录     : {}", creds_dir.display());

            use x509_parser::prelude::*;
            if let Some(der) = creds.cert_chain_der()?.first()
                && let Ok((_, cert)) = X509Certificate::from_der(der.as_ref())
            {
                println!(
                    "  有效期   : {} → {}",
                    cert.validity().not_before,
                    cert.validity().not_after
                );
            }

            // 启动自检：PEM 可解析 / 私钥与证书匹配 / 未过期
            match creds.verify() {
                Ok(()) => println!("  状态     : 可用"),
                Err(issue) => {
                    println!("  状态     : ✗ {issue}");
                    println!();
                    println!("{}", issue.advice());
                    std::process::exit(2);
                }
            }
        }
    }

    Ok(())
}

fn load_transport(data_dir: &std::path::Path, slot: Option<&str>) -> Result<TransportConfig> {
    let creds_dir = match slot {
        Some(s) => qe_device_pki_client::creds::locate(s, Some(data_dir))?,
        None => find_any_creds(data_dir)?,
    };
    let creds = Credentials::load(&creds_dir).with_context(|| {
        format!("加载凭证失败（{}）", creds_dir.display())
    })?;
    // 用前自检：损坏的凭证必须在这里就报清楚，而不是等 mTLS 握手含糊失败
    if let Err(issue) = creds.verify() {
        anyhow::bail!("凭证不可用：{issue}\n\n{}", issue.advice());
    }
    Ok(TransportConfig::new(creds, data_dir.to_path_buf()))
}

/// 未指定 slot 时找出唯一的待续跑注册（`pending/` 下的落盘状态）。
fn find_any_pending(data_dir: &std::path::Path) -> Result<String> {
    let base = data_dir.join("pending");
    let mut found = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                found.push(p.file_stem().unwrap_or_default().to_string_lossy().into_owned());
            }
        }
    }
    match found.len() {
        0 => anyhow::bail!(
            "没有待续跑的注册（{} 下为空）。请用注册链接重新 enroll",
            base.display()
        ),
        1 => Ok(found.remove(0)),
        _ => anyhow::bail!("发现多个待续跑注册，请用 --slot 指定：{}", found.join(", ")),
    }
}

/// 未指定 slot 时找出唯一的凭证目录。
fn find_any_creds(data_dir: &std::path::Path) -> Result<PathBuf> {
    let base = data_dir.join("creds");
    let mut found = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() && Credentials::exists(&p) {
                found.push(p);
            }
        }
    }
    match found.len() {
        0 => anyhow::bail!("未找到任何已注册凭证（{} 下为空），请先执行 enroll", base.display()),
        1 => Ok(found.remove(0)),
        _ => anyhow::bail!(
            "发现多个凭证目录，请用 --slot 指定：{}",
            found
                .iter()
                .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
