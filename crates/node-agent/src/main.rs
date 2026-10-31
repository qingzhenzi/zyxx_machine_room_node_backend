//! 房间节点 agent 入口（P1）。
//!
//! 启动流程：读环境变量 → 加载设备证书 → `confirm`（已激活可忽略错误）→ 注册
//! `image.pull` handler → 每 2s `downlink::poll_once`；Ctrl-C 优雅退出。
//!
//! 导入是重操作，handler 内用 `tokio::sync::Mutex` 串行执行，避免两个镜像同时
//! `docker load` / `virsh vol-upload`。

#![deny(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use qe_device_pki_client::creds::{self, Credentials};
use qe_device_pki_client::downlink::{self, Dispatcher};
use qe_device_pki_client::transport::{self, DownlinkMessage, OutboxQueue, TransportConfig};
use qe_node_agent::cache::ObjectCache;
use qe_node_agent::config::AgentConfig;
use qe_node_agent::importer::ImporterSet;
use qe_node_agent::job::{ImagePullJob, ensure_staging_root};
use qe_node_agent::presign::MtlsPresignClient;
use qe_node_agent::pull::{HttpObjectFetcher, ObjectFetcher, Puller};
use qe_node_agent::report::{MtlsReporter, Reporter};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

/// 下行轮询间隔（契约 §16.4）。
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// 单次拉取的下行消息条数。
const DOWNLINK_BATCH: i64 = 20;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    if let Err(error) = run().await {
        error!(error = %format!("{error:#}"), "qe-node-agent 退出");
        return Err(error);
    }
    info!("qe-node-agent 已退出");
    Ok(())
}

async fn run() -> Result<()> {
    let config = AgentConfig::from_env().context("读取 agent 配置失败")?;
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("创建数据目录失败: {}", config.data_dir.display()))?;
    std::fs::create_dir_all(&config.cache_dir)
        .with_context(|| format!("创建缓存目录失败: {}", config.cache_dir.display()))?;

    info!(
        slot = %config.slot,
        endpoint = %config.mtls_endpoint,
        data_dir = %config.data_dir.display(),
        cache_dir = %config.cache_dir.display(),
        cache_max_bytes = config.cache_max_bytes,
        download_concurrency = config.download_concurrency,
        "启动 qe-node-agent"
    );

    // 1. 设备凭证（复用 device-pki-client 的目录布局：<data_dir>/creds/<slot>）
    let creds_dir =
        creds::locate(&config.slot, Some(&config.data_dir)).context("定位设备凭证目录失败")?;
    let mut credentials = Credentials::load(&creds_dir)
        .with_context(|| format!("加载设备凭证失败: {}", creds_dir.display()))?;
    // 以环境变量为准（证书 meta 里的端点可能是注册时的旧地址）。
    credentials.mtls_endpoint = config.mtls_endpoint.clone();
    if let Err(issue) = credentials.verify() {
        warn!(
            slot = %config.slot,
            issue = %issue,
            advice = %issue.advice(),
            "设备凭证自检未通过，仍尝试启动"
        );
    }

    // 2. mTLS confirm。服务端对已 active 的证书可能返回错误，这不是致命问题。
    let confirm_config = transport_config(&credentials, &config);
    match transport::confirm(&confirm_config).await {
        Ok(reply) => info!(
            status = %reply.status,
            serial = %reply.serial,
            mtls = %reply.mtls,
            "mTLS 通讯已确认"
        ),
        Err(error) => warn!(
            error = %error,
            "mTLS confirm 失败（若证书已激活可忽略），继续启动下行轮询"
        ),
    }

    // 3. 上行 outbox（持久化，发送成功后删除）
    let outbox = Arc::new(
        OutboxQueue::open(&config.data_dir.join("outbox"))
            .map_err(|error| anyhow::anyhow!("打开上行 outbox 失败: {error}"))?,
    );
    let reporter: Arc<dyn Reporter> = Arc::new(MtlsReporter::new(
        transport_config(&credentials, &config),
        Arc::clone(&outbox),
    ));

    // 4. 预签名 + 对象下载 + 内容寻址缓存
    let presign = Arc::new(
        MtlsPresignClient::new(&transport_config(&credentials, &config))
            .context("构建预签名 mTLS 客户端失败")?,
    );
    let fetcher: Arc<dyn ObjectFetcher> =
        Arc::new(HttpObjectFetcher::new().context("构建对象下载客户端失败")?);
    let cache = Arc::new(ObjectCache::new(&config.cache_dir, config.cache_max_bytes));
    let puller = Puller::new(presign, fetcher, cache, config.download_concurrency);

    // 5. 导入适配器 + job
    let staging_root = config.data_dir.join("staging");
    ensure_staging_root(&staging_root)?;
    let job = Arc::new(ImagePullJob::new(
        puller,
        ImporterSet::builtin(&config),
        reporter,
        &staging_root,
    ));

    // 6. 下行分发：image.pull 串行执行
    let dispatcher = Arc::new(Dispatcher::new());
    let import_lock = Arc::new(tokio::sync::Mutex::new(()));
    {
        let job = Arc::clone(&job);
        let import_lock = Arc::clone(&import_lock);
        dispatcher.subscribe("image.pull", move |message: DownlinkMessage| {
            let job = Arc::clone(&job);
            let import_lock = Arc::clone(&import_lock);
            async move {
                // 导入必须串行：docker load / incus image import / virsh vol-upload
                // 同时跑会争抢同一份运行时状态。
                let _guard = import_lock.lock().await;
                match job.run(&message.payload).await {
                    Ok(outcome) if outcome.ok => info!(
                        transfer_id = %outcome.transfer_id,
                        imported_ref = ?outcome.imported_ref,
                        "image.pull 处理成功"
                    ),
                    Ok(outcome) => warn!(
                        transfer_id = %outcome.transfer_id,
                        error = ?outcome.error,
                        "image.pull 失败，已上报 transfer.completed(ok=false)"
                    ),
                    Err(error) => {
                        // payload 无法解析：ack 掉避免毒消息无限重投，错误留给运维排查。
                        error!(
                            id = message.id,
                            error = %format!("{error:#}"),
                            "image.pull payload 解析失败，跳过该消息"
                        );
                    }
                }
                Ok(())
            }
        });
    }

    // 7. 下行轮询循环 + Ctrl-C
    let downlink_config = transport_config(&credentials, &config);
    let state_file = downlink::state_path(&config.slot, Some(&config.data_dir))
        .context("定位下行游标文件失败")?;
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // interval 的第一次 tick 立即就绪：启动后马上拉一次，之后按 2s 周期。

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("收到 Ctrl-C，停止轮询并退出");
                break;
            }
            _ = ticker.tick() => {
                match downlink::poll_once(
                    &downlink_config,
                    &dispatcher,
                    &state_file,
                    DOWNLINK_BATCH,
                )
                .await
                {
                    Ok(0) => debug!("没有新的下行消息"),
                    Ok(count) => info!(count, "处理下行消息"),
                    Err(error) => warn!(
                        error = %error,
                        "轮询下行失败，{:?} 后重试", POLL_INTERVAL
                    ),
                }
            }
        }
    }

    // 未送达的上行消息已持久化在 outbox，下次启动或下一次上报时重放。
    drop(outbox);
    Ok(())
}

/// 构造一份 mTLS 传输配置（`TransportConfig` 不实现 Clone，按需构造）。
fn transport_config(credentials: &Credentials, config: &AgentConfig) -> TransportConfig {
    let mut credentials = credentials.clone();
    credentials.mtls_endpoint = config.mtls_endpoint.clone();
    TransportConfig::new(credentials, config.data_dir.clone())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,qe_node_agent=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false)
        .compact()
        .init();
}
