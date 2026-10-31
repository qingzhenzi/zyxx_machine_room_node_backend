//! 课表事件引擎：触发扫描 + 到期投递。
//!
//! ## 为什么放在 `qe-api`
//!
//! 引擎同时依赖 `qe-db`（规则 / 排课实例 / 事件 / 投递仓储）与 `qe-cache`
//! （Redis 锁与发布频道）。`qe-db` 不能依赖 Redis，`qe-cache` 也不认识
//! PostgreSQL，所以只有装配层适合承载这段跨层逻辑。
//!
//! ## 一致性设计
//!
//! - **事件先落库**：`event_key` 唯一，重叠扫描窗口天然幂等；
//! - **每个目标一行投递**：失败按退避重试，达到上限置 `failed`，
//!   永久不可达（无设备槽位 / 无 webhook 地址）置 `skipped`；
//! - **多副本**：同一轮任务用 Redis `SET NX EX` 锁互斥，抢不到锁直接跳过；
//! - **不拖垮主进程**：引擎自身错误只记日志，下一 tick 继续；
//!   启动时的全量重建失败也不影响 HTTP 服务启动。

use std::{collections::HashMap, time::Duration};

use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, NaiveTime, Utc};
use serde_json::{Map, Value, json};
use tokio::{
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use uuid::Uuid;

use crate::{
    db::schedule::EnabledRule,
    domain::schedule::{
        DeliveryJob, DeliveryStatus, DeliveryTarget, EventRule, NewScheduleEvent, Occurrence,
        TriggerKind, local_date_of, local_rfc3339, local_to_utc,
    },
    state::AppState,
};

/// tick 扫描锁：`max(tick*4, 60)` 秒，与扫描窗口一致。
const TICK_LOCK_KEY: &str = "schedule:engine:tick";
/// 投递锁：ttl 远大于单批投递耗时，避免多副本重复投递。
const DELIVER_LOCK_KEY: &str = "schedule:engine:deliver";
/// 全量重建锁：重建可能跑数十秒，给足 TTL。
const REBUILD_LOCK_KEY: &str = "schedule:engine:rebuild";
const DELIVER_LOCK_TTL_SECS: u64 = 60;
const REBUILD_LOCK_TTL_SECS: u64 = 600;
/// 扫描窗口的最小 lookback：tick 周期很小时也要覆盖到上一次 tick。
const MIN_LOOKBACK_SECS: u64 = 60;
/// 触发扫描需要往前多取一段实例：`room_occupied` / `room_idle` 要判断
/// 「此刻是否有更早开始、仍未结束的实例」。
const OCCURRENCE_LOOKBACK_HOURS: i64 = 12;
const DEFAULT_DOWNLINK_KIND: &str = "schedule.event";
/// 配置最小值为 1 秒；这里只做兜底，避免除零/零超时。
const MIN_WEBHOOK_TIMEOUT_SECS: u64 = 1;

/// 一次触发扫描的结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickSummary {
    pub events_created: usize,
    pub deliveries_created: usize,
}

impl TickSummary {
    fn is_empty(&self) -> bool {
        self.events_created == 0 && self.deliveries_created == 0
    }
}

/// 一批到期投递的处理结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliverySummary {
    pub delivered: usize,
    pub retried: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl DeliverySummary {
    fn is_empty(&self) -> bool {
        self.delivered == 0 && self.retried == 0 && self.failed == 0 && self.skipped == 0
    }
}

/// 启动后台引擎：一次启动重建 + tick 循环（扫描 + 投递）+ 周期重建。
///
/// 返回 tick 循环的 `JoinHandle`，调用方可以忽略（detach）或持有以便关闭。
/// 所有子任务都用 `tokio::spawn`，任何单轮失败只记录日志。
pub fn spawn(state: AppState) -> JoinHandle<()> {
    let tick_secs = state.config.schedule_tick_secs.max(1);
    let rebuild_interval_secs = state.config.schedule_rebuild_interval_secs.max(60);

    // 启动后先做一次全量重建（后台执行，不阻塞 HTTP 启动）。
    spawn_startup_rebuild(state.clone());
    // 之后按固定周期再重建，兜住漏掉的事件与跨天的实例物化。
    spawn_rebuild_loop(state.clone(), rebuild_interval_secs);

    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(tick_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;

            match tick_once(&state, Utc::now()).await {
                Ok(summary) => {
                    if !summary.is_empty() {
                        tracing::info!(
                            events = summary.events_created,
                            deliveries = summary.deliveries_created,
                            "课表事件触发扫描完成"
                        );
                    }
                }
                Err(error) => tracing::error!(%error, "课表事件触发扫描失败"),
            }

            match deliver_once(&state).await {
                Ok(summary) => {
                    if !summary.is_empty() {
                        tracing::info!(
                            delivered = summary.delivered,
                            retried = summary.retried,
                            failed = summary.failed,
                            skipped = summary.skipped,
                            "课表事件投递完成"
                        );
                    }
                }
                Err(error) => tracing::error!(%error, "课表事件投递失败"),
            }
        }
    })
}

fn spawn_startup_rebuild(state: AppState) {
    tokio::spawn(async move {
        match rebuild_all(&state).await {
            Ok(occurrences) => tracing::info!(occurrences, "课表实例全量重建完成"),
            Err(error) => tracing::error!(%error, "课表实例全量重建失败（不影响进程启动）"),
        }
    });
}

fn spawn_rebuild_loop(state: AppState, interval_secs: u64) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // 第一次 tick 立即返回；启动后的首次重建由 `spawn_startup_rebuild` 负责。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match rebuild_all(&state).await {
                Ok(occurrences) => tracing::info!(occurrences, "课表实例周期重建完成"),
                Err(error) => tracing::error!(%error, "课表实例周期重建失败"),
            }
        }
    });
}

/// 以 `now` 为基准跑一次触发扫描。
///
/// 抢不到 Redis 锁（其它副本正在跑）时返回空结果，不报错。
pub async fn tick_once(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<TickSummary> {
    let ttl = tick_lock_ttl(state.config.schedule_tick_secs);
    if !acquire_lock(state, TICK_LOCK_KEY, ttl).await? {
        tracing::debug!("其它副本正在进行课表触发扫描，跳过本轮");
        return Ok(TickSummary::default());
    }
    let result = scan_once(state, now).await;
    release_lock(state, TICK_LOCK_KEY).await;
    result
}

async fn scan_once(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<TickSummary> {
    let lookback = lookback_secs(state.config.schedule_tick_secs);
    let window_start = now - ChronoDuration::seconds(lookback);
    let window_end = now;

    let rules = state
        .event_rules
        .enabled_rules()
        .await
        .context("加载启用中的触发规则失败")?;
    if rules.is_empty() {
        return Ok(TickSummary::default());
    }

    // 实例窗口：向后覆盖 `occurrence_start_before` 的最大提前量（校验上限 1440 分钟）。
    let max_offset_minutes = rules
        .iter()
        .filter(|enabled| enabled.rule.trigger_kind == TriggerKind::OccurrenceStartBefore)
        .map(|enabled| i64::from(enabled.rule.offset_minutes.clamp(0, 1440)))
        .max()
        .unwrap_or(0);
    let fetch_from = window_start - ChronoDuration::hours(OCCURRENCE_LOOKBACK_HOURS);
    let fetch_to = window_end + ChronoDuration::minutes(max_offset_minutes);
    let occurrences = state
        .schedules
        .occurrences_overlapping(fetch_from, fetch_to)
        .await
        .context("加载窗口内的排课实例失败")?;

    let mut occurrences_by_room: HashMap<Uuid, Vec<&Occurrence>> = HashMap::new();
    for occurrence in &occurrences {
        occurrences_by_room
            .entry(occurrence.room_id)
            .or_default()
            .push(occurrence);
    }

    let mut summary = TickSummary::default();
    for enabled in &rules {
        let resolved = resolve_rule_config(&enabled.rule.config);
        let targets = resolve_targets(&resolved, enabled);

        if enabled.rule.trigger_kind == TriggerKind::DailyTime {
            for hit in daily_hits(
                &enabled.rule,
                enabled.timezone.as_str(),
                window_start,
                window_end,
            ) {
                let triggered = TriggeredEvent::for_daily(
                    &enabled.rule,
                    enabled,
                    hit.triggered_at,
                    hit.local_date,
                );
                record_event(
                    state,
                    &mut summary,
                    triggered.to_new_event(&resolved),
                    &targets,
                )
                .await;
            }
            continue;
        }

        let room_occurrences: &[&Occurrence] = occurrences_by_room
            .get(&enabled.rule.room_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for occurrence in room_occurrences {
            let Some(triggered_at) =
                occurrence_trigger_instant(enabled.rule.trigger_kind, &enabled.rule, occurrence)
            else {
                continue;
            };
            if !is_in_window(triggered_at, window_start, window_end) {
                continue;
            }
            if !room_activity_allows(enabled.rule.trigger_kind, occurrence, room_occurrences) {
                continue;
            }
            let triggered = TriggeredEvent::for_occurrence(
                &enabled.rule,
                enabled,
                occurrence,
                enabled.rule.trigger_kind,
                triggered_at,
            );
            record_event(
                state,
                &mut summary,
                triggered.to_new_event(&resolved),
                &targets,
            )
            .await;
        }
    }

    Ok(summary)
}

/// 写入事件（幂等）并统计；失败只记日志，不打断整轮扫描。
async fn record_event(
    state: &AppState,
    summary: &mut TickSummary,
    new_event: NewScheduleEvent,
    targets: &ResolvedTargets,
) {
    let event_key = new_event.event_key.clone();
    match state
        .schedule_events
        .insert_if_absent(&new_event, &targets.targets)
        .await
    {
        Ok(Some(event)) => {
            summary.events_created += 1;
            summary.deliveries_created += targets.targets.len();

            if !targets.immediate_skips.is_empty() {
                match mark_immediate_skips(state, event.id, &targets.immediate_skips).await {
                    Ok(()) => {}
                    Err(error) => {
                        tracing::error!(%error, event_id = %event.id, "标记跳过投递失败");
                    }
                }
            }
        }
        Ok(None) => tracing::trace!(event_key = %event_key, "事件已存在，跳过"),
        Err(error) => tracing::error!(%error, event_key = %event_key, "写入课表事件失败"),
    }
}

/// 把触发瞬间就注定无法投递的目标直接置 `skipped`（契约 §6.1），
/// 避免每个 tick 都重复尝试。
async fn mark_immediate_skips(
    state: &AppState,
    event_id: Uuid,
    skips: &[(DeliveryTarget, &'static str)],
) -> anyhow::Result<()> {
    let deliveries = state
        .schedule_events
        .deliveries_for(event_id)
        .await
        .context("加载事件的投递记录失败")?;
    for delivery in deliveries {
        if delivery.status != DeliveryStatus::Pending {
            continue;
        }
        if let Some((_, reason)) = skips.iter().find(|(target, _)| *target == delivery.target) {
            state
                .schedule_events
                .mark_skipped(delivery.id, reason)
                .await?;
        }
    }
    Ok(())
}

/// 处理一批到期的投递。抢不到锁时返回空结果。
pub async fn deliver_once(state: &AppState) -> anyhow::Result<DeliverySummary> {
    if !acquire_lock(state, DELIVER_LOCK_KEY, DELIVER_LOCK_TTL_SECS).await? {
        tracing::debug!("其它副本正在进行课表事件投递，跳过本轮");
        return Ok(DeliverySummary::default());
    }
    let result = deliver_batch(state).await;
    release_lock(state, DELIVER_LOCK_KEY).await;
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryOutcome {
    Delivered,
    Retried,
    Failed,
    Skipped,
}

async fn deliver_batch(state: &AppState) -> anyhow::Result<DeliverySummary> {
    let batch = state.config.schedule_delivery_batch.max(1);
    let jobs = state
        .schedule_events
        .due_deliveries(batch)
        .await
        .context("加载到期投递失败")?;
    if jobs.is_empty() {
        return Ok(DeliverySummary::default());
    }

    let client = build_webhook_client(state)?;
    let mut summary = DeliverySummary::default();
    for job in jobs {
        match deliver_job(state, &client, &job).await {
            Ok(DeliveryOutcome::Delivered) => summary.delivered += 1,
            Ok(DeliveryOutcome::Retried) => summary.retried += 1,
            Ok(DeliveryOutcome::Failed) => summary.failed += 1,
            Ok(DeliveryOutcome::Skipped) => summary.skipped += 1,
            Err(error) => {
                tracing::error!(%error, delivery_id = job.delivery_id, "处理投递失败");
            }
        }
    }
    Ok(summary)
}

async fn deliver_job(
    state: &AppState,
    client: &reqwest::Client,
    job: &DeliveryJob,
) -> anyhow::Result<DeliveryOutcome> {
    match job.target {
        DeliveryTarget::Pubsub => deliver_pubsub(state, job).await,
        DeliveryTarget::Downlink => deliver_downlink(state, job).await,
        DeliveryTarget::Webhook => deliver_webhook(state, client, job).await,
    }
}

async fn deliver_pubsub(state: &AppState, job: &DeliveryJob) -> anyhow::Result<DeliveryOutcome> {
    let payload = serde_json::to_string(&job.payload).context("序列化事件 payload 失败")?;
    match state
        .cache
        .publish(&state.config.schedule_pubsub_channel, &payload)
        .await
    {
        Ok(()) => {
            state
                .schedule_events
                .mark_delivered(job.delivery_id)
                .await?;
            Ok(DeliveryOutcome::Delivered)
        }
        Err(error) => retry_or_fail(state, job, format!("Redis 发布失败：{error}")).await,
    }
}

async fn deliver_downlink(state: &AppState, job: &DeliveryJob) -> anyhow::Result<DeliveryOutcome> {
    let Some(slot) = job
        .device_slot
        .as_deref()
        .map(str::trim)
        .filter(|slot| !slot.is_empty())
    else {
        // 房间压根没绑设备槽位：重试也不会成功，直接跳过。
        state
            .schedule_events
            .mark_skipped(job.delivery_id, "房间未绑定设备槽位")
            .await?;
        return Ok(DeliveryOutcome::Skipped);
    };

    let kind = downlink_kind(&job.payload);
    let payload = serde_json::to_string(&job.payload).context("序列化下行 payload 失败")?;
    match state
        .schedule_events
        .enqueue_downlink_for_slot(slot, kind, &payload)
        .await
    {
        Ok(Some(serial)) => {
            // 唤醒长连接（跨进程尽力而为；失败不影响投递结果，还有兜底轮询）。
            if let Err(error) = state
                .cache
                .publish(&state.config.schedule_downlink_notify_channel, &serial)
                .await
            {
                tracing::warn!(%error, serial = %serial, "发布设备下行通知失败");
            }
            state
                .schedule_events
                .mark_delivered(job.delivery_id)
                .await?;
            Ok(DeliveryOutcome::Delivered)
        }
        Ok(None) => {
            // 槽位绑了设备槽位但此刻没有存活证书：设备可能稍后上线，重试。
            retry_or_fail(state, job, "没有可用设备，等待设备上线".to_owned()).await
        }
        Err(error) => retry_or_fail(state, job, format!("设备下行入队失败：{error}")).await,
    }
}

async fn deliver_webhook(
    state: &AppState,
    client: &reqwest::Client,
    job: &DeliveryJob,
) -> anyhow::Result<DeliveryOutcome> {
    let Some(url) = job
        .webhook_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        state
            .schedule_events
            .mark_skipped(job.delivery_id, "房间未配置 webhook 地址")
            .await?;
        return Ok(DeliveryOutcome::Skipped);
    };

    let response = client
        .post(url)
        .header("X-QE-Event", job.event_kind.as_str())
        .header("X-QE-Event-Id", job.event_id.to_string())
        .header("X-QE-Delivery-Id", job.delivery_id.to_string())
        .json(&job.payload)
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => {
            state
                .schedule_events
                .mark_delivered(job.delivery_id)
                .await?;
            Ok(DeliveryOutcome::Delivered)
        }
        Ok(response) => {
            retry_or_fail(
                state,
                job,
                format!("webhook 返回 HTTP {}", response.status()),
            )
            .await
        }
        Err(error) => retry_or_fail(state, job, format!("webhook 请求失败：{error}")).await,
    }
}

/// 失败后按契约退避重试；达到上限置 `failed`。
async fn retry_or_fail(
    state: &AppState,
    job: &DeliveryJob,
    error: String,
) -> anyhow::Result<DeliveryOutcome> {
    let max_attempts = state.config.schedule_webhook_max_attempts.max(1) as i32;
    let next_attempts = job.attempts.saturating_add(1);

    if next_attempts >= max_attempts {
        state
            .schedule_events
            .mark_failed(job.delivery_id, next_attempts, &error)
            .await?;
        tracing::warn!(
            delivery_id = job.delivery_id,
            attempts = next_attempts,
            %error,
            "投递达到最大尝试次数，标记失败"
        );
        return Ok(DeliveryOutcome::Failed);
    }

    let delay_secs = retry_backoff_secs(next_attempts);
    let next_attempt_at = Utc::now() + ChronoDuration::seconds(delay_secs);
    state
        .schedule_events
        .mark_retry(job.delivery_id, next_attempts, next_attempt_at, &error)
        .await?;
    tracing::debug!(
        delivery_id = job.delivery_id,
        attempts = next_attempts,
        delay_secs,
        %error,
        "投递失败，已安排重试"
    );
    Ok(DeliveryOutcome::Retried)
}

fn build_webhook_client(state: &AppState) -> anyhow::Result<reqwest::Client> {
    let timeout = Duration::from_secs(
        state
            .config
            .schedule_webhook_timeout_secs
            .max(MIN_WEBHOOK_TIMEOUT_SECS),
    );
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("qe-schedule-engine/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("构建 webhook HTTP 客户端失败")
}

/// 全量重建所有课表的未来排课实例，返回写入条数。
pub async fn rebuild_all(state: &AppState) -> anyhow::Result<usize> {
    if !acquire_lock(state, REBUILD_LOCK_KEY, REBUILD_LOCK_TTL_SECS).await? {
        tracing::debug!("其它副本正在进行课表实例全量重建，跳过本轮");
        return Ok(0);
    }
    let result = state
        .schedules
        .rebuild_all_occurrences(state.config.schedule_horizon_days)
        .await
        .context("全量重建课表实例失败");
    release_lock(state, REBUILD_LOCK_KEY).await;
    result
}

async fn acquire_lock(state: &AppState, key: &str, ttl_secs: u64) -> anyhow::Result<bool> {
    state
        .cache
        .try_lock(key, ttl_secs)
        .await
        .with_context(|| format!("抢占 Redis 锁 {key} 失败"))
}

async fn release_lock(state: &AppState, key: &str) {
    if let Err(error) = state.cache.unlock(key).await {
        tracing::warn!(%error, lock = key, "释放 Redis 锁失败（TTL 到期后自动释放）");
    }
}

// ---------------------------------------------------------------------------
// 纯逻辑（可单测，不依赖 DB / Redis）
// ---------------------------------------------------------------------------

/// 扫描窗口 lookback：`max(60s, tick*4)`。
fn lookback_secs(tick_secs: u64) -> i64 {
    tick_secs.saturating_mul(4).max(MIN_LOOKBACK_SECS) as i64
}

/// tick 锁 TTL：与扫描窗口一致；窗口重叠由 `event_key` 幂等兜底。
fn tick_lock_ttl(tick_secs: u64) -> u64 {
    tick_secs.saturating_mul(4).max(MIN_LOOKBACK_SECS)
}

/// 触发瞬间是否落在扫描窗口 `(window_start, window_end]` 内。
fn is_in_window(
    instant: DateTime<Utc>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> bool {
    instant > window_start && instant <= window_end
}

/// 解析规则的 `at_time`（`"HH:MM"`，也接受 `"HH:MM:SS"`）。
fn parse_at_time(raw: &str) -> Option<NaiveTime> {
    let raw = raw.trim();
    NaiveTime::parse_from_str(raw, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(raw, "%H:%M:%S"))
        .ok()
}

/// 一次每日定时触发的候选。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DailyHit {
    local_date: NaiveDate,
    triggered_at: DateTime<Utc>,
}

/// 计算窗口内命中的每日定时事件。
///
/// `at_time` 是**本地钟面时间**，按房间所属区域的 IANA 时区解释（契约 §10.5）：
/// 窗口跨本地午夜时两天都判断；夏令时跳变导致的「不存在的本地时刻」当天不触发，
/// 重复出现的本地时刻取较早的那个（由 `qe_domain::schedule::local_to_utc` 决定）。
fn daily_hits(
    rule: &EventRule,
    timezone: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<DailyHit> {
    let Some(at_time) = rule.at_time.as_deref().and_then(parse_at_time) else {
        tracing::warn!(rule_id = %rule.id, "每日定时规则缺少合法的 at_time，跳过");
        return Vec::new();
    };

    let Ok(mut local_date) = local_date_of(window_start, timezone) else {
        tracing::warn!(rule_id = %rule.id, timezone, "时区非法，跳过每日定时规则");
        return Vec::new();
    };
    let Ok(last_local_date) = local_date_of(window_end, timezone) else {
        tracing::warn!(rule_id = %rule.id, timezone, "时区非法，跳过每日定时规则");
        return Vec::new();
    };

    let mut hits = Vec::new();
    while local_date <= last_local_date {
        match local_to_utc(local_date, at_time, timezone) {
            Ok(triggered_at) => {
                if is_in_window(triggered_at, window_start, window_end) {
                    hits.push(DailyHit {
                        local_date,
                        triggered_at,
                    });
                }
            }
            Err(error) => {
                // 夏令时「跳过」的本地时刻（如 America/New_York 03-08 02:30）当天不存在。
                tracing::warn!(
                    rule_id = %rule.id,
                    timezone,
                    %local_date,
                    %error,
                    "本地时刻在该时区中不存在，跳过该日"
                );
            }
        }
        let Some(next) = local_date.succ_opt() else {
            break;
        };
        local_date = next;
    }
    hits
}

/// 实例类规则的触发瞬间。`daily_time` 不针对实例，返回 `None`。
fn occurrence_trigger_instant(
    kind: TriggerKind,
    rule: &EventRule,
    occurrence: &Occurrence,
) -> Option<DateTime<Utc>> {
    match kind {
        TriggerKind::OccurrenceStart | TriggerKind::RoomOccupied => Some(occurrence.start_at),
        TriggerKind::OccurrenceEnd | TriggerKind::RoomIdle => Some(occurrence.end_at),
        TriggerKind::OccurrenceStartBefore => {
            Some(occurrence.start_at - ChronoDuration::minutes(i64::from(rule.offset_minutes)))
        }
        TriggerKind::DailyTime => None,
    }
}

/// 双排课去重：`room_occupied` 要求此刻没有「更早开始、仍未结束」的其它实例；
/// `room_idle` 要求此刻没有「其它仍在进行」的实例。背靠背（一个结束、下一个开始）
/// 不算占用。
fn room_activity_allows(
    kind: TriggerKind,
    occurrence: &Occurrence,
    room_occurrences: &[&Occurrence],
) -> bool {
    match kind {
        TriggerKind::RoomOccupied => {
            let start_at = occurrence.start_at;
            !room_occurrences.iter().any(|other| {
                other.id != occurrence.id && other.start_at < start_at && other.end_at > start_at
            })
        }
        TriggerKind::RoomIdle => {
            let end_at = occurrence.end_at;
            !room_occurrences.iter().any(|other| {
                other.id != occurrence.id && other.start_at < end_at && other.end_at > end_at
            })
        }
        _ => true,
    }
}

/// 实例类事件键：`occ:{rule_id}:{occurrence_id}:{trigger_kind}`。
fn occurrence_event_key(rule_id: Uuid, occurrence_id: Uuid, kind: TriggerKind) -> String {
    format!("occ:{rule_id}:{occurrence_id}:{}", kind.as_str())
}

/// 每日定时事件键：`daily:{rule_id}:{local_date}`。
fn daily_event_key(rule_id: Uuid, local_date: NaiveDate) -> String {
    format!("daily:{rule_id}:{}", local_date.format("%Y-%m-%d"))
}

/// 事件标题（与契约 §6.1 文案一致）。
fn event_title(kind: TriggerKind, course_name: &str, offset_minutes: i32) -> String {
    match kind {
        TriggerKind::OccurrenceStart => format!("课程开始：{course_name}"),
        TriggerKind::OccurrenceEnd => format!("课程结束：{course_name}"),
        TriggerKind::OccurrenceStartBefore => {
            format!("上课提醒（{offset_minutes} 分钟）：{course_name}")
        }
        TriggerKind::DailyTime => "每日事件".to_owned(),
        TriggerKind::RoomOccupied => format!("房间占用：{course_name}"),
        TriggerKind::RoomIdle => "房间空闲".to_owned(),
    }
}

/// 从规则 config 解析出的投递目标配置（契约 §6.1）。
#[derive(Debug, Clone, PartialEq)]
struct ResolvedConfig {
    targets: Vec<DeliveryTarget>,
    downlink_kind: String,
    webhook_url: Option<String>,
    extra_payload: Option<Value>,
}

fn default_targets() -> Vec<DeliveryTarget> {
    vec![
        DeliveryTarget::Pubsub,
        DeliveryTarget::Downlink,
        DeliveryTarget::Webhook,
    ]
}

fn resolve_rule_config(config: &Value) -> ResolvedConfig {
    let targets = match config.get("targets") {
        None | Some(Value::Null) => default_targets(),
        Some(Value::Array(items)) => {
            let mut targets = Vec::new();
            for item in items {
                let Some(raw) = item.as_str() else {
                    tracing::warn!(value = %item, "config.targets 含非字符串元素，已忽略");
                    continue;
                };
                match raw.trim().parse::<DeliveryTarget>() {
                    Ok(target) => {
                        if !targets.contains(&target) {
                            targets.push(target);
                        }
                    }
                    Err(()) => {
                        tracing::warn!(target = raw, "config.targets 含未知投递目标，已忽略");
                    }
                }
            }
            targets
        }
        Some(other) => {
            tracing::warn!(value = %other, "config.targets 不是数组，使用默认目标");
            default_targets()
        }
    };

    let downlink_kind = config
        .get("downlink_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or(DEFAULT_DOWNLINK_KIND)
        .to_owned();
    let webhook_url = config
        .get("webhook_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned);
    let extra_payload = config
        .get("payload")
        .filter(|payload| payload.is_object())
        .cloned();

    ResolvedConfig {
        targets,
        downlink_kind,
        webhook_url,
        extra_payload,
    }
}

/// 规则解析出的投递目标 + 立刻就能判定为不可达的目标。
#[derive(Debug, Clone)]
struct ResolvedTargets {
    targets: Vec<DeliveryTarget>,
    immediate_skips: Vec<(DeliveryTarget, &'static str)>,
}

fn resolve_targets(resolved: &ResolvedConfig, enabled: &EnabledRule) -> ResolvedTargets {
    let has_device_slot = enabled
        .device_slot
        .as_deref()
        .is_some_and(|slot| !slot.trim().is_empty());
    let has_webhook = resolved.webhook_url.is_some()
        || enabled
            .webhook_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty());

    let mut immediate_skips = Vec::new();
    for target in &resolved.targets {
        match target {
            DeliveryTarget::Downlink if !has_device_slot => {
                immediate_skips.push((DeliveryTarget::Downlink, "房间未绑定设备槽位"))
            }
            DeliveryTarget::Webhook if !has_webhook => {
                immediate_skips.push((DeliveryTarget::Webhook, "房间未配置 webhook 地址"));
            }
            _ => {}
        }
    }

    ResolvedTargets {
        targets: resolved.targets.clone(),
        immediate_skips,
    }
}

/// 下行消息 kind：优先取事件 payload 里的 `downlink_kind`，否则用默认值。
fn downlink_kind(payload: &Value) -> &str {
    payload
        .get("downlink_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or(DEFAULT_DOWNLINK_KIND)
}

/// 重试退避秒数：`min(5 * 2^attempts, 300)`。
fn retry_backoff_secs(attempts: i32) -> i64 {
    (5_i64 * 2_i64.pow(attempts.clamp(0, 16) as u32)).min(300)
}

/// 一次命中的触发（实例或每日定时），负责生成事件键 / 标题 / payload。
struct TriggeredEvent<'a> {
    rule: &'a EventRule,
    enabled: &'a EnabledRule,
    occurrence: Option<&'a Occurrence>,
    kind: TriggerKind,
    triggered_at: DateTime<Utc>,
    event_key: String,
}

impl<'a> TriggeredEvent<'a> {
    fn for_occurrence(
        rule: &'a EventRule,
        enabled: &'a EnabledRule,
        occurrence: &'a Occurrence,
        kind: TriggerKind,
        triggered_at: DateTime<Utc>,
    ) -> Self {
        Self {
            rule,
            enabled,
            occurrence: Some(occurrence),
            kind,
            triggered_at,
            event_key: occurrence_event_key(rule.id, occurrence.id, kind),
        }
    }

    fn for_daily(
        rule: &'a EventRule,
        enabled: &'a EnabledRule,
        triggered_at: DateTime<Utc>,
        local_date: NaiveDate,
    ) -> Self {
        Self {
            rule,
            enabled,
            occurrence: None,
            kind: TriggerKind::DailyTime,
            triggered_at,
            event_key: daily_event_key(rule.id, local_date),
        }
    }

    fn course_name(&self) -> &str {
        self.occurrence
            .map_or("", |occurrence| occurrence.course_name.as_str())
    }

    fn title(&self) -> String {
        event_title(self.kind, self.course_name(), self.rule.offset_minutes)
    }

    /// 基础 payload，再浅合并 `config.payload`。
    ///
    /// 时间字段双份：`*_at` 是 UTC（存库/投递用），`*_local` 是带偏移的本地时间
    /// （展示用，由 `qe_domain::schedule::local_rfc3339` 生成）。
    fn payload(&self, resolved: &ResolvedConfig) -> Value {
        let timezone = self.enabled.timezone.as_str();
        let mut payload = Map::new();
        payload.insert("event_kind".to_owned(), json!(self.kind.as_str()));
        payload.insert("room_id".to_owned(), json!(self.rule.room_id));
        payload.insert("room_code".to_owned(), json!(self.enabled.room_code));
        payload.insert("room_name".to_owned(), json!(self.enabled.room_name));
        payload.insert("timezone".to_owned(), json!(timezone));
        payload.insert(
            "schedule_id".to_owned(),
            self.occurrence
                .map_or(Value::Null, |occurrence| json!(occurrence.schedule_id)),
        );
        payload.insert(
            "occurrence_id".to_owned(),
            self.occurrence
                .map_or(Value::Null, |occurrence| json!(occurrence.id)),
        );
        payload.insert(
            "course_name".to_owned(),
            self.occurrence
                .map_or(Value::Null, |occurrence| json!(occurrence.course_name)),
        );
        payload.insert(
            "teacher".to_owned(),
            self.occurrence
                .map_or(Value::Null, |occurrence| json!(occurrence.teacher)),
        );
        payload.insert(
            "start_at".to_owned(),
            self.occurrence
                .map_or(Value::Null, |occurrence| json!(occurrence.start_at)),
        );
        payload.insert(
            "start_local".to_owned(),
            self.occurrence.map_or(Value::Null, |occurrence| {
                json!(local_rfc3339(occurrence.start_at, timezone))
            }),
        );
        payload.insert(
            "end_at".to_owned(),
            self.occurrence
                .map_or(Value::Null, |occurrence| json!(occurrence.end_at)),
        );
        payload.insert(
            "end_local".to_owned(),
            self.occurrence.map_or(Value::Null, |occurrence| {
                json!(local_rfc3339(occurrence.end_at, timezone))
            }),
        );
        payload.insert("triggered_at".to_owned(), json!(self.triggered_at));
        payload.insert(
            "triggered_at_local".to_owned(),
            json!(local_rfc3339(self.triggered_at, timezone)),
        );
        payload.insert(
            "downlink_kind".to_owned(),
            json!(resolved.downlink_kind.as_str()),
        );

        if let Some(Value::Object(extra)) = &resolved.extra_payload {
            for (key, value) in extra {
                payload.insert(key.clone(), value.clone());
            }
        }

        Value::Object(payload)
    }

    fn to_new_event(&self, resolved: &ResolvedConfig) -> NewScheduleEvent {
        NewScheduleEvent {
            event_key: self.event_key.clone(),
            room_id: self.rule.room_id,
            schedule_id: self.occurrence.map(|occurrence| occurrence.schedule_id),
            occurrence_id: self.occurrence.map(|occurrence| occurrence.id),
            rule_id: Some(self.rule.id),
            event_kind: self.kind.as_str().to_owned(),
            title: self.title(),
            payload: self.payload(resolved),
            triggered_at: self.triggered_at,
            timezone: self.enabled.timezone.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::schedule::OccurrenceSource;

    fn ts(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .expect("测试时间戳合法")
            .with_timezone(&Utc)
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("测试日期合法")
    }

    fn event_rule(kind: TriggerKind, offset_minutes: i32) -> EventRule {
        EventRule {
            id: Uuid::from_u128(1),
            room_id: Uuid::from_u128(2),
            trigger_kind: kind,
            offset_minutes,
            at_time: None,
            enabled: true,
            config: json!({}),
            created_at: ts("2026-01-01T00:00:00Z"),
            updated_at: ts("2026-01-01T00:00:00Z"),
        }
    }

    fn enabled_rule(rule: EventRule) -> EnabledRule {
        EnabledRule {
            rule,
            room_code: "room-101".to_owned(),
            room_name: "101 教室".to_owned(),
            device_slot: Some("slot-101".to_owned()),
            webhook_url: Some("https://example.com/hook".to_owned()),
            timezone: "Asia/Shanghai".to_owned(),
        }
    }

    fn occurrence(id: u128, start: DateTime<Utc>, end: DateTime<Utc>) -> Occurrence {
        Occurrence {
            id: Uuid::from_u128(id),
            schedule_id: Uuid::from_u128(100),
            room_id: Uuid::from_u128(2),
            occurrence_date: start.date_naive(),
            start_at: start,
            end_at: end,
            course_name: "高等数学".to_owned(),
            teacher: "张老师".to_owned(),
            source: OccurrenceSource::Weekly,
            source_slot_id: Uuid::from_u128(200),
            note: String::new(),
            timezone: "Asia/Shanghai".to_owned(),
        }
    }

    #[test]
    fn lookback_and_lock_ttl_follow_tick_seconds() {
        assert_eq!(lookback_secs(5), 60);
        assert_eq!(lookback_secs(15), 60);
        assert_eq!(lookback_secs(300), 1200);
        assert_eq!(tick_lock_ttl(5), 60);
        assert_eq!(tick_lock_ttl(15), 60);
        assert_eq!(tick_lock_ttl(300), 1200);
    }

    #[test]
    fn window_is_left_open_right_closed() {
        let start = ts("2026-01-01T00:00:00Z");
        let end = ts("2026-01-01T00:01:00Z");
        assert!(!is_in_window(start, start, end), "窗口左边界不含");
        assert!(is_in_window(ts("2026-01-01T00:00:01Z"), start, end));
        assert!(is_in_window(end, start, end), "窗口右边界含");
        assert!(!is_in_window(ts("2026-01-01T00:01:01Z"), start, end));
    }

    #[test]
    fn at_time_accepts_hh_mm_and_hh_mm_ss() {
        assert_eq!(parse_at_time("08:30"), NaiveTime::from_hms_opt(8, 30, 0));
        assert_eq!(
            parse_at_time(" 08:30:15 "),
            NaiveTime::from_hms_opt(8, 30, 15)
        );
        assert_eq!(parse_at_time("25:00"), None);
        assert_eq!(parse_at_time("not-a-time"), None);
    }

    #[test]
    fn daily_hits_use_iana_timezone() {
        // 窗口 UTC 23:59:30 → 00:00:30；Asia/Shanghai 下本地是 07:59:30 → 08:00:30，
        // at_time 08:00 落在窗口内，触发点是 UTC 00:00:00。
        let mut rule = event_rule(TriggerKind::DailyTime, 0);
        rule.at_time = Some("08:00".to_owned());
        let hits = daily_hits(
            &rule,
            "Asia/Shanghai",
            ts("2026-01-01T23:59:30Z"),
            ts("2026-01-02T00:00:30Z"),
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].local_date, day(2026, 1, 2));
        assert_eq!(hits[0].triggered_at, ts("2026-01-02T00:00:00Z"));
    }

    #[test]
    fn daily_hits_use_iana_timezone_across_dst() {
        // America/New_York 2026-11-01 结束夏令时：01:30 本地出现两次，
        // 契约要求取较早的（EDT，UTC-4）→ 05:30Z；窗口按本地日期覆盖该日。
        let mut rule = event_rule(TriggerKind::DailyTime, 0);
        rule.at_time = Some("01:30".to_owned());
        let hits = daily_hits(
            &rule,
            "America/New_York",
            ts("2026-11-01T05:00:00Z"),
            ts("2026-11-01T06:00:00Z"),
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].local_date, day(2026, 11, 1));
        assert_eq!(hits[0].triggered_at, ts("2026-11-01T05:30:00Z"));
    }

    #[test]
    fn daily_hits_skip_nonexistent_dst_local_time() {
        // America/New_York 2026-03-08 02:00→03:00：02:30 本地不存在，当天不触发。
        let mut rule = event_rule(TriggerKind::DailyTime, 0);
        rule.at_time = Some("02:30".to_owned());
        let hits = daily_hits(
            &rule,
            "America/New_York",
            ts("2026-03-08T05:00:00Z"),
            ts("2026-03-08T08:00:00Z"),
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn daily_hits_skip_invalid_timezone() {
        let mut rule = event_rule(TriggerKind::DailyTime, 0);
        rule.at_time = Some("08:00".to_owned());
        let hits = daily_hits(
            &rule,
            "Not/A_Timezone",
            ts("2026-01-01T23:59:00Z"),
            ts("2026-01-02T00:01:00Z"),
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn daily_hits_check_every_local_date_in_window() {
        // 跨两个本地日期的 48h 窗口，at_time=00:00 应命中中间的两个日期。
        let mut rule = event_rule(TriggerKind::DailyTime, 0);
        rule.at_time = Some("00:00".to_owned());
        let hits = daily_hits(
            &rule,
            "UTC",
            ts("2026-01-01T12:00:00Z"),
            ts("2026-01-03T12:00:00Z"),
        );
        assert_eq!(
            hits.iter().map(|hit| hit.local_date).collect::<Vec<_>>(),
            vec![day(2026, 1, 2), day(2026, 1, 3)]
        );
    }

    #[test]
    fn daily_hits_skip_rule_without_at_time() {
        let rule = event_rule(TriggerKind::DailyTime, 0);
        let hits = daily_hits(
            &rule,
            "Asia/Shanghai",
            ts("2026-01-01T00:00:00Z"),
            ts("2026-01-01T00:01:00Z"),
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn occurrence_trigger_instants_follow_contract() {
        let rule = event_rule(TriggerKind::OccurrenceStartBefore, 30);
        let occ = occurrence(10, ts("2026-01-02T01:00:00Z"), ts("2026-01-02T02:00:00Z"));
        assert_eq!(
            occurrence_trigger_instant(TriggerKind::OccurrenceStart, &rule, &occ),
            Some(ts("2026-01-02T01:00:00Z"))
        );
        assert_eq!(
            occurrence_trigger_instant(TriggerKind::OccurrenceEnd, &rule, &occ),
            Some(ts("2026-01-02T02:00:00Z"))
        );
        assert_eq!(
            occurrence_trigger_instant(TriggerKind::OccurrenceStartBefore, &rule, &occ),
            Some(ts("2026-01-02T00:30:00Z")),
            "提前 offset 分钟触发"
        );
        assert_eq!(
            occurrence_trigger_instant(TriggerKind::RoomOccupied, &rule, &occ),
            Some(ts("2026-01-02T01:00:00Z"))
        );
        assert_eq!(
            occurrence_trigger_instant(TriggerKind::RoomIdle, &rule, &occ),
            Some(ts("2026-01-02T02:00:00Z"))
        );
        assert_eq!(
            occurrence_trigger_instant(TriggerKind::DailyTime, &rule, &occ),
            None
        );
    }

    #[test]
    fn room_occupied_dedupes_double_booking() {
        let first = occurrence(10, ts("2026-01-02T01:00:00Z"), ts("2026-01-02T02:00:00Z"));
        let second = occurrence(11, ts("2026-01-02T01:30:00Z"), ts("2026-01-02T02:30:00Z"));
        let room = [&first, &second];
        assert!(
            room_activity_allows(TriggerKind::RoomOccupied, &first, &room),
            "第一个实例开始时应触发占用"
        );
        assert!(
            !room_activity_allows(TriggerKind::RoomOccupied, &second, &room),
            "第二个实例开始时房间已被占用，不应重复触发"
        );
    }

    #[test]
    fn room_idle_requires_no_ongoing_occurrence() {
        let earlier = occurrence(10, ts("2026-01-02T01:00:00Z"), ts("2026-01-02T02:00:00Z"));
        let overlapping = occurrence(11, ts("2026-01-02T01:30:00Z"), ts("2026-01-02T02:30:00Z"));
        let back_to_back = occurrence(12, ts("2026-01-02T02:00:00Z"), ts("2026-01-02T02:30:00Z"));

        assert!(
            !room_activity_allows(TriggerKind::RoomIdle, &earlier, &[&earlier, &overlapping]),
            "还有一个实例在进行中，不应触发空闲"
        );
        assert!(
            room_activity_allows(TriggerKind::RoomIdle, &earlier, &[&earlier, &back_to_back]),
            "背靠背排课不算占用"
        );
    }

    #[test]
    fn event_keys_follow_contract() {
        let rule_id = Uuid::from_u128(1);
        let occurrence_id = Uuid::from_u128(2);
        assert_eq!(
            occurrence_event_key(rule_id, occurrence_id, TriggerKind::OccurrenceStart),
            format!("occ:{rule_id}:{occurrence_id}:occurrence_start")
        );
        assert_eq!(
            daily_event_key(rule_id, day(2026, 1, 2)),
            format!("daily:{rule_id}:2026-01-02")
        );
    }

    #[test]
    fn titles_follow_contract() {
        assert_eq!(
            event_title(TriggerKind::OccurrenceStart, "高数", 0),
            "课程开始：高数"
        );
        assert_eq!(
            event_title(TriggerKind::OccurrenceEnd, "高数", 0),
            "课程结束：高数"
        );
        assert_eq!(
            event_title(TriggerKind::OccurrenceStartBefore, "高数", 15),
            "上课提醒（15 分钟）：高数"
        );
        assert_eq!(event_title(TriggerKind::DailyTime, "", 0), "每日事件");
        assert_eq!(
            event_title(TriggerKind::RoomOccupied, "高数", 0),
            "房间占用：高数"
        );
        assert_eq!(event_title(TriggerKind::RoomIdle, "", 0), "房间空闲");
    }

    #[test]
    fn rule_config_defaults_to_all_targets() {
        let resolved = resolve_rule_config(&json!({}));
        assert_eq!(resolved.targets, default_targets());
        assert_eq!(resolved.downlink_kind, "schedule.event");
        assert_eq!(resolved.webhook_url, None);
        assert_eq!(resolved.extra_payload, None);
    }

    #[test]
    fn rule_config_reads_targets_kind_and_payload() {
        let resolved = resolve_rule_config(&json!({
            "targets": ["pubsub", "unknown-target", "webhook", "pubsub"],
            "downlink_kind": "custom.kind",
            "webhook_url": "https://hook.example.com/x",
            "payload": { "teacher": "李老师" }
        }));
        assert_eq!(
            resolved.targets,
            vec![DeliveryTarget::Pubsub, DeliveryTarget::Webhook]
        );
        assert_eq!(resolved.downlink_kind, "custom.kind");
        assert_eq!(
            resolved.webhook_url.as_deref(),
            Some("https://hook.example.com/x")
        );
        assert_eq!(resolved.extra_payload, Some(json!({ "teacher": "李老师" })));
    }

    #[test]
    fn rule_config_ignores_invalid_targets_value() {
        let resolved = resolve_rule_config(&json!({ "targets": "pubsub" }));
        assert_eq!(resolved.targets, default_targets());
    }

    #[test]
    fn immediate_skips_cover_missing_slot_and_webhook() {
        let mut enabled = enabled_rule(event_rule(TriggerKind::OccurrenceStart, 0));
        enabled.device_slot = None;
        enabled.webhook_url = None;
        let resolved = resolve_rule_config(&json!({}));
        let targets = resolve_targets(&resolved, &enabled);
        assert_eq!(targets.targets.len(), 3);
        assert!(
            targets
                .immediate_skips
                .contains(&(DeliveryTarget::Downlink, "房间未绑定设备槽位"))
        );
        assert!(
            targets
                .immediate_skips
                .contains(&(DeliveryTarget::Webhook, "房间未配置 webhook 地址"))
        );
    }

    #[test]
    fn immediate_skips_honor_config_webhook_url() {
        let mut enabled = enabled_rule(event_rule(TriggerKind::OccurrenceStart, 0));
        enabled.webhook_url = None;
        let resolved = resolve_rule_config(&json!({ "webhook_url": "https://hook.example.com/x" }));
        let targets = resolve_targets(&resolved, &enabled);
        assert!(targets.immediate_skips.is_empty());
    }

    #[test]
    fn occurrence_event_payload_includes_base_fields() {
        let rule = event_rule(TriggerKind::OccurrenceStart, 0);
        let enabled = enabled_rule(rule.clone());
        let occ = occurrence(10, ts("2026-01-02T01:00:00Z"), ts("2026-01-02T02:00:00Z"));
        let resolved = resolve_rule_config(&json!({}));
        let triggered = TriggeredEvent::for_occurrence(
            &enabled.rule,
            &enabled,
            &occ,
            TriggerKind::OccurrenceStart,
            occ.start_at,
        );
        let new_event = triggered.to_new_event(&resolved);

        assert_eq!(
            new_event.event_key,
            format!("occ:{}:{}:occurrence_start", rule.id, occ.id)
        );
        assert_eq!(new_event.event_kind, "occurrence_start");
        assert_eq!(new_event.schedule_id, Some(occ.schedule_id));
        assert_eq!(new_event.occurrence_id, Some(occ.id));
        assert_eq!(new_event.rule_id, Some(rule.id));
        assert_eq!(new_event.title, "课程开始：高等数学");
        assert_eq!(new_event.timezone, "Asia/Shanghai");

        let payload = &new_event.payload;
        assert_eq!(payload["event_kind"], "occurrence_start");
        assert_eq!(payload["room_code"], "room-101");
        assert_eq!(payload["room_name"], "101 教室");
        assert_eq!(payload["timezone"], "Asia/Shanghai");
        assert_eq!(payload["schedule_id"], json!(occ.schedule_id));
        assert_eq!(payload["occurrence_id"], json!(occ.id));
        assert_eq!(payload["course_name"], "高等数学");
        assert_eq!(payload["teacher"], "张老师");
        // UTC 字段保留
        assert_eq!(payload["start_at"], json!(occ.start_at));
        assert_eq!(payload["end_at"], json!(occ.end_at));
        assert_eq!(payload["triggered_at"], json!(occ.start_at));
        // 本地时间字段带偏移：01:00Z → 09:00+08:00
        assert_eq!(payload["start_local"], "2026-01-02T09:00:00+08:00");
        assert_eq!(payload["end_local"], "2026-01-02T10:00:00+08:00");
        assert_eq!(payload["triggered_at_local"], "2026-01-02T09:00:00+08:00");
        assert_eq!(payload["downlink_kind"], "schedule.event");
    }

    #[test]
    fn config_payload_shallow_merges_into_event_payload() {
        let enabled = enabled_rule(event_rule(TriggerKind::OccurrenceStart, 0));
        let occ = occurrence(10, ts("2026-01-02T01:00:00Z"), ts("2026-01-02T02:00:00Z"));
        let resolved = resolve_rule_config(&json!({
            "payload": { "course_name": "覆盖课程名", "extra": 1 }
        }));
        let triggered = TriggeredEvent::for_occurrence(
            &enabled.rule,
            &enabled,
            &occ,
            TriggerKind::OccurrenceStart,
            occ.start_at,
        );
        let payload = triggered.payload(&resolved);
        assert_eq!(payload["course_name"], "覆盖课程名");
        assert_eq!(payload["extra"], 1);
        // 基础字段仍保留
        assert_eq!(payload["room_code"], "room-101");
    }

    #[test]
    fn daily_event_payload_has_null_occurrence_fields() {
        let enabled = enabled_rule(event_rule(TriggerKind::DailyTime, 0));
        let triggered = TriggeredEvent::for_daily(
            &enabled.rule,
            &enabled,
            ts("2026-01-02T00:00:00Z"),
            day(2026, 1, 2),
        );
        let resolved = resolve_rule_config(&json!({}));
        let new_event = triggered.to_new_event(&resolved);

        assert_eq!(
            new_event.event_key,
            format!("daily:{}:2026-01-02", enabled.rule.id)
        );
        assert_eq!(new_event.event_kind, "daily_time");
        assert_eq!(new_event.title, "每日事件");
        assert_eq!(new_event.schedule_id, None);
        assert_eq!(new_event.occurrence_id, None);
        assert_eq!(new_event.timezone, "Asia/Shanghai");
        assert!(new_event.payload["course_name"].is_null());
        assert!(new_event.payload["start_at"].is_null());
        assert!(new_event.payload["start_local"].is_null());
        assert!(new_event.payload["end_at"].is_null());
        assert!(new_event.payload["end_local"].is_null());
        // 每日事件没有实例，但触发时刻仍有本地时间
        assert_eq!(new_event.payload["timezone"], "Asia/Shanghai");
        assert_eq!(
            new_event.payload["triggered_at_local"],
            "2026-01-02T08:00:00+08:00"
        );
        assert_eq!(new_event.payload["downlink_kind"], "schedule.event");
    }

    #[test]
    fn downlink_kind_prefers_payload_value() {
        assert_eq!(
            downlink_kind(&json!({ "downlink_kind": "custom.kind" })),
            "custom.kind"
        );
        assert_eq!(downlink_kind(&json!({})), DEFAULT_DOWNLINK_KIND);
        assert_eq!(
            downlink_kind(&json!({ "downlink_kind": "  " })),
            DEFAULT_DOWNLINK_KIND
        );
    }

    #[test]
    fn retry_backoff_is_capped() {
        assert_eq!(retry_backoff_secs(0), 5);
        assert_eq!(retry_backoff_secs(1), 10);
        assert_eq!(retry_backoff_secs(2), 20);
        assert_eq!(retry_backoff_secs(5), 160);
        assert_eq!(retry_backoff_secs(6), 300);
        assert_eq!(retry_backoff_secs(20), 300);
    }

    #[test]
    fn payload_local_time_uses_rule_timezone() {
        let mut enabled = enabled_rule(event_rule(TriggerKind::DailyTime, 0));
        enabled.timezone = "America/New_York".to_owned();
        let triggered = TriggeredEvent::for_daily(
            &enabled.rule,
            &enabled,
            ts("2026-01-02T01:00:00Z"),
            day(2026, 1, 2),
        );
        let resolved = resolve_rule_config(&json!({}));
        let new_event = triggered.to_new_event(&resolved);

        assert_eq!(new_event.timezone, "America/New_York");
        assert_eq!(new_event.payload["timezone"], "America/New_York");
        // 2026-01-02T01:00Z = 纽约 2026-01-01T20:00:00-05:00（EST）
        assert_eq!(
            new_event.payload["triggered_at_local"],
            "2026-01-01T20:00:00-05:00"
        );
    }
}
