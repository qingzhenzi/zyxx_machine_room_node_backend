//! 工单自动建单 worker（P0 三条规则 + P1 规则配置化）。
//!
//! - 每 tick 读取 `ticket_rules` 中启用的规则，按 `signal_type` 扫描对应信号；
//! - 规则可配置阈值 / 优先级 / 范围（global / area / room），禁用或删除后对应信号不再建单；
//! - 首轮只建立基线，不重放历史问题；机器健康/设备离线按“事件”语义抑制到恢复，
//!   投递/分发/验证类按源行 id 水位去重；
//! - 建单幂等依赖 DB 部分唯一索引 + `ON CONFLICT`，多副本安全，不依赖 Redis 锁；
//! - 同一轮内单条规则失败只记录日志，不影响其它规则；
//! - 每 tick 顺带扫描超时工单，发送邮件提醒并写 `reminded` 时间线事件（配置开关）。

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use uuid::Uuid;

use qe_domain::ticket::{
    InspectionMachineSummary, InspectionPlan, InspectionRun, InspectionRunStatus, NewTicket,
    TicketCategory, TicketPriority, TicketRule, TicketScopeType, TicketSignalType, TicketSource,
};

use crate::{
    config::TicketWorkerConfig,
    db::tickets::{
        DeviceOfflineSignal, FailedDeliverySignal, ImageVerifySignal, MachineHealthSignal,
        PresenceDeliverySignal, TransferSignal,
    },
    state::AppState,
};

/// 超时提醒每 tick 最多处理条数（避免 SMTP 慢拖住 tick）。
const REMINDER_BATCH_LIMIT: usize = 20;

/// worker 进程内状态：首轮基线 + 各信号的“已处理”水位。
#[derive(Default)]
pub struct TicketWorkerTracker {
    /// 第一轮是否已经建立基线。
    pub initialized: bool,
    /// 启动时已经异常、被抑制到恢复为止的 serial（机器健康）。
    pub health_suppressed: HashSet<String>,
    /// 上一轮判定为“离线超过阈值”的 serial（设备离线）。
    pub offline_over_threshold: HashSet<String>,
    /// 已经处理过的失败投递 id（课表推送规则）。
    pub seen_deliveries: HashSet<i64>,
    /// 已经处理过的在线事件投递 id。
    pub seen_presence_deliveries: HashSet<i64>,
    /// 已经处理过的分发任务 id。
    pub seen_transfers: HashSet<Uuid>,
    /// 已经处理过的镜像验证 id。
    pub seen_image_verifications: HashSet<Uuid>,
}

pub fn spawn(state: AppState, config: TicketWorkerConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        let tick_secs = config.tick_secs.max(1);
        let mut tracker = TicketWorkerTracker::default();
        let mut ticker = interval(Duration::from_secs(tick_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            run_tick(&state, &config, &mut tracker).await;
        }
    })
}

/// 执行一轮扫描：规则驱动 + 超时提醒。单条规则失败只记录日志。
pub async fn run_tick(
    state: &AppState,
    config: &TicketWorkerConfig,
    tracker: &mut TicketWorkerTracker,
) {
    let baseline = !tracker.initialized;
    let rules = match state.ticket_rules.list_enabled().await {
        Ok(rules) => rules,
        Err(error) => {
            tracing::warn!(%error, "读取工单自动规则失败，跳过本轮自动建单");
            return;
        }
    };

    // 同一 signal_type 只执行“最具体”的一条规则（room > area > global），
    // 避免全局规则按名称排序遮蔽更具体的范围规则。
    let mut selected: Vec<&TicketRule> = Vec::new();
    for rule in &rules {
        match selected
            .iter()
            .position(|current| current.signal_type == rule.signal_type)
        {
            Some(index) => {
                if rule.scope_type.rank() > selected[index].scope_type.rank() {
                    selected[index] = rule;
                }
            }
            None => selected.push(rule),
        }
    }

    for rule in selected {
        let result = match rule.signal_type {
            TicketSignalType::MachineHealth => {
                machine_health_tick(state, config, tracker, rule, baseline).await
            }
            TicketSignalType::DeviceOffline => {
                device_offline_tick(state, config, tracker, rule, baseline).await
            }
            TicketSignalType::ScheduleDelivery => {
                schedule_delivery_tick(state, config, tracker, rule, baseline).await
            }
            TicketSignalType::PresenceDelivery => {
                presence_delivery_tick(state, config, tracker, rule, baseline).await
            }
            TicketSignalType::Transfer => {
                transfer_tick(state, config, tracker, rule, baseline).await
            }
            TicketSignalType::ImageVerify => {
                image_verify_tick(state, config, tracker, rule, baseline).await
            }
        };
        match result {
            Ok(created) if created > 0 => tracing::info!(
                signal = rule.signal_type.as_str(),
                rule = %rule.name,
                created,
                baseline,
                "自动规则生成工单"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                %error,
                signal = rule.signal_type.as_str(),
                rule = %rule.name,
                "自动规则扫描失败"
            ),
        }
    }

    reminder_tick(state, config).await;

    // 巡检计划：独立于自动规则与首轮基线，claim 到期计划后同步执行；单条失败只记录日志。
    match inspection_tick(state).await {
        Ok(executed) if executed > 0 => {
            tracing::info!(executed, "巡检计划执行完成")
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "巡检计划扫描失败"),
    }

    if baseline {
        tracing::info!(
            health_suppressed = tracker.health_suppressed.len(),
            offline_baseline = tracker.offline_over_threshold.len(),
            failed_deliveries = tracker.seen_deliveries.len(),
            presence_deliveries = tracker.seen_presence_deliveries.len(),
            transfers = tracker.seen_transfers.len(),
            image_verifications = tracker.seen_image_verifications.len(),
            "工单自动建单 worker 已建立基线（不重放历史）"
        );
        tracker.initialized = true;
    }
}

fn threshold(rule: &TicketRule, key: &str, fallback: i64) -> i64 {
    rule.threshold_i64(key).unwrap_or(fallback).max(1)
}

fn lookback(rule: &TicketRule, config: &TicketWorkerConfig) -> i64 {
    threshold(rule, "lookback_hours", config.lookback_hours)
}

async fn insert_plans(state: &AppState, signal: &str, plans: Vec<NewTicket>) -> usize {
    let mut created = 0;
    for plan in plans {
        match state.tickets.insert_ticket(&plan).await {
            Ok(ticket) => {
                created += 1;
                if ticket.occurrence_count > 1 {
                    tracing::info!(
                        rule = signal,
                        code = %ticket.code,
                        occurrence = ticket.occurrence_count,
                        "自动工单重复发生已累加"
                    );
                } else {
                    tracing::info!(
                        rule = signal,
                        code = %ticket.code,
                        occurrence = ticket.occurrence_count,
                        "自动工单已创建"
                    );
                }
            }
            Err(error) => tracing::warn!(%error, rule = signal, "自动建单失败"),
        }
    }
    created
}

async fn machine_health_tick(
    state: &AppState,
    config: &TicketWorkerConfig,
    tracker: &mut TicketWorkerTracker,
    rule: &TicketRule,
    baseline: bool,
) -> anyhow::Result<usize> {
    let consecutive = threshold(rule, "consecutive", config.health_consecutive);
    let signals: Vec<MachineHealthSignal> = state
        .tickets
        .machine_health_candidates(consecutive)
        .await?
        .into_iter()
        .filter(|signal| rule.matches_scope(signal.area_id, signal.room_id))
        .collect();
    let plans = plan_machine_health(tracker, signals, baseline, rule.priority);
    Ok(insert_plans(state, "machine_health", plans).await)
}

async fn device_offline_tick(
    state: &AppState,
    config: &TicketWorkerConfig,
    tracker: &mut TicketWorkerTracker,
    rule: &TicketRule,
    baseline: bool,
) -> anyhow::Result<usize> {
    let minutes = threshold(rule, "offline_minutes", config.device_offline_minutes);
    let signals: Vec<DeviceOfflineSignal> = state
        .tickets
        .device_offline_candidates(minutes)
        .await?
        .into_iter()
        .filter(|signal| rule.matches_scope(Some(signal.area_id), Some(signal.room_id)))
        .collect();
    let plans = plan_device_offline(tracker, signals, baseline, rule.priority);
    Ok(insert_plans(state, "device_offline", plans).await)
}

async fn schedule_delivery_tick(
    state: &AppState,
    config: &TicketWorkerConfig,
    tracker: &mut TicketWorkerTracker,
    rule: &TicketRule,
    baseline: bool,
) -> anyhow::Result<usize> {
    let signals: Vec<FailedDeliverySignal> = state
        .tickets
        .failed_delivery_candidates(lookback(rule, config))
        .await?
        .into_iter()
        .filter(|signal| rule.matches_scope(Some(signal.area_id), Some(signal.room_id)))
        .collect();
    let plans = plan_schedule_delivery(tracker, signals, baseline, rule.priority);
    Ok(insert_plans(state, "schedule_delivery", plans).await)
}

async fn presence_delivery_tick(
    state: &AppState,
    config: &TicketWorkerConfig,
    tracker: &mut TicketWorkerTracker,
    rule: &TicketRule,
    baseline: bool,
) -> anyhow::Result<usize> {
    let signals: Vec<PresenceDeliverySignal> = state
        .tickets
        .presence_delivery_candidates(lookback(rule, config))
        .await?
        .into_iter()
        .filter(|signal| rule.matches_scope(signal.area_id, signal.room_id))
        .collect();
    let plans = plan_presence_delivery(tracker, signals, baseline, rule.priority);
    Ok(insert_plans(state, "presence_delivery", plans).await)
}

async fn transfer_tick(
    state: &AppState,
    config: &TicketWorkerConfig,
    tracker: &mut TicketWorkerTracker,
    rule: &TicketRule,
    baseline: bool,
) -> anyhow::Result<usize> {
    let signals: Vec<TransferSignal> = state
        .tickets
        .transfer_candidates(lookback(rule, config))
        .await?
        .into_iter()
        .filter(|signal| rule.matches_scope(Some(signal.area_id), Some(signal.room_id)))
        .collect();
    let plans = plan_transfer(tracker, signals, baseline, rule.priority);
    Ok(insert_plans(state, "transfer", plans).await)
}

async fn image_verify_tick(
    state: &AppState,
    config: &TicketWorkerConfig,
    tracker: &mut TicketWorkerTracker,
    rule: &TicketRule,
    baseline: bool,
) -> anyhow::Result<usize> {
    let signals: Vec<ImageVerifySignal> = state
        .tickets
        .image_verify_candidates(lookback(rule, config))
        .await?
        .into_iter()
        .filter(|_| rule.matches_scope(None, None))
        .collect();
    let plans = plan_image_verify(tracker, signals, baseline, rule.priority);
    Ok(insert_plans(state, "image_verify", plans).await)
}

/// 超时提醒：已过 SLA 且超过提醒间隔的未结束工单 → 邮件 + `reminded` 事件。
async fn reminder_tick(state: &AppState, config: &TicketWorkerConfig) {
    if !config.email_enabled {
        return;
    }
    let tickets = match state
        .tickets
        .overdue_tickets(config.overdue_remind_hours)
        .await
    {
        Ok(tickets) => tickets,
        Err(error) => {
            tracing::warn!(%error, "扫描超时工单失败");
            return;
        }
    };
    for ticket in tickets.into_iter().take(REMINDER_BATCH_LIMIT) {
        // 没有收件人时跳过并保留水位，后续补上邮箱后还能再提醒。
        let Some(email) = ticket.notify_email.as_deref() else {
            continue;
        };
        // 先原子抢占（多副本只有一个副本能更新成功），再发信 + 写 reminded。
        match state
            .tickets
            .claim_reminder(ticket.id, config.overdue_remind_hours)
            .await
        {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                tracing::warn!(%error, ticket_id = %ticket.id, "抢占超时提醒失败");
                continue;
            }
        }
        state
            .email
            .send_ticket_overdue(
                email,
                &ticket.code,
                &ticket.title,
                &ticket.id.to_string(),
                &ticket.due_at.to_rfc3339(),
            )
            .await;
    }
}

// ----------------------------------------------------------------------
// 纯规划函数（与数据库解耦，便于单元测试）
// ----------------------------------------------------------------------

fn machine_health_ticket(signal: MachineHealthSignal, priority: TicketPriority) -> NewTicket {
    let effective = if signal.health_status == "critical" {
        TicketPriority::High
    } else {
        priority
    };
    let issues = payload_health_issues(&signal.payload);
    let issue_text = describe_health_issues(&issues);
    let description = if issue_text.is_empty() {
        format!(
            "机器 {} 连续上报非 ok 健康状态（当前：{}）。",
            signal.serial, signal.health_status
        )
    } else {
        format!(
            "机器 {} 连续上报非 ok 健康状态（当前：{}）。\n问题：\n{}",
            signal.serial, signal.health_status, issue_text
        )
    };
    NewTicket {
        title: format!(
            "机器健康检查 {}：{}",
            signal.health_status, signal.device_slot
        ),
        description,
        priority: effective,
        category: TicketCategory::MachineHealth,
        source: TicketSource::Auto,
        source_type: Some("machine_snapshot".to_owned()),
        source_id: Some(signal.snapshot_id.to_string()),
        dedupe_key: Some(format!("machine_health:{}", signal.serial)),
        area_id: signal.area_id,
        room_id: signal.room_id,
        serial: Some(signal.serial.clone()),
        device_slot: Some(signal.device_slot.clone()),
        reporter_id: None,
        metadata: json!({
            "serial": signal.serial,
            "device_slot": signal.device_slot,
            "snapshot_id": signal.snapshot_id,
            "health_status": signal.health_status,
            "health_score": signal.health_score,
            "collected_at": signal.collected_at,
            "issues": issues,
        }),
    }
}

fn device_offline_ticket(signal: DeviceOfflineSignal, priority: TicketPriority) -> NewTicket {
    let description = format!(
        "设备 {}（{}）离线超过阈值。\n房间：{} {}\n最后会话更新：{}",
        signal.serial,
        signal.device_slot,
        signal.room_code,
        signal.room_name,
        signal.session_updated_at.to_rfc3339()
    );
    NewTicket {
        title: format!("设备离线：{} / {}", signal.device_slot, signal.serial),
        description,
        priority,
        category: TicketCategory::DeviceOffline,
        source: TicketSource::Auto,
        source_type: Some("device_presence".to_owned()),
        source_id: Some(signal.session_updated_at.to_rfc3339()),
        dedupe_key: Some(format!("device_offline:{}", signal.serial)),
        area_id: Some(signal.area_id),
        room_id: Some(signal.room_id),
        serial: Some(signal.serial.clone()),
        device_slot: Some(signal.device_slot.clone()),
        reporter_id: None,
        metadata: json!({
            "serial": signal.serial,
            "device_slot": signal.device_slot,
            "room_id": signal.room_id,
            "room_code": signal.room_code,
            "room_name": signal.room_name,
            "area_id": signal.area_id,
            "area_name": signal.area_name,
            "session_updated_at": signal.session_updated_at,
        }),
    }
}

fn schedule_delivery_ticket(fresh: &[FailedDeliverySignal], priority: TicketPriority) -> NewTicket {
    let first = &fresh[0];
    let latest = fresh
        .iter()
        .max_by_key(|signal| (signal.triggered_at, signal.delivery_id))
        .unwrap_or(first);
    let errors: Vec<String> = fresh
        .iter()
        .filter_map(|signal| signal.last_error.clone())
        .take(10)
        .collect();
    let mut description = format!(
        "房间 {} {} 的课表事件投递失败（目标：{}，本地日期：{}），本轮新增 {} 次失败。",
        first.room_code,
        first.room_name,
        first.target,
        first.local_date,
        fresh.len()
    );
    describe_errors(&mut description, &errors);
    NewTicket {
        title: format!(
            "课表推送失败：{} / {} / {}",
            first.room_code, first.target, first.local_date
        ),
        description,
        priority,
        category: TicketCategory::DeliveryFailed,
        source: TicketSource::Auto,
        source_type: Some("schedule_delivery".to_owned()),
        source_id: Some(latest.event_id.to_string()),
        dedupe_key: Some(format!(
            "schedule_delivery:{}:{}:{}",
            first.room_id, first.target, first.local_date
        )),
        area_id: Some(first.area_id),
        room_id: Some(first.room_id),
        serial: None,
        device_slot: None,
        reporter_id: None,
        metadata: json!({
            "room_id": first.room_id,
            "room_code": first.room_code,
            "room_name": first.room_name,
            "area_id": first.area_id,
            "target": first.target,
            "local_date": first.local_date,
            "event_id": latest.event_id,
            "delivery_id": latest.delivery_id,
            "failed_count": fresh.len(),
            "attempts": latest.attempts,
            "errors": errors,
        }),
    }
}

fn presence_delivery_ticket(
    fresh: &[PresenceDeliverySignal],
    target: &str,
    local_date: &str,
    priority: TicketPriority,
) -> NewTicket {
    let first = &fresh[0];
    let latest = fresh
        .iter()
        .max_by_key(|signal| (signal.occurred_at, signal.delivery_id))
        .unwrap_or(first);
    let errors: Vec<String> = fresh
        .iter()
        .filter_map(|signal| signal.last_error.clone())
        .take(10)
        .collect();
    let mut description = format!(
        "设备在线事件 webhook 投递失败（目标：{target}，本地日期：{local_date}），本轮新增 {} 次失败。",
        fresh.len()
    );
    describe_errors(&mut description, &errors);
    NewTicket {
        title: format!("在线事件推送失败：{target} / {local_date}"),
        description,
        priority,
        category: TicketCategory::DeliveryFailed,
        source: TicketSource::Auto,
        source_type: Some("presence_delivery".to_owned()),
        source_id: Some(latest.delivery_id.to_string()),
        dedupe_key: Some(format!("presence_delivery:{target}:{local_date}")),
        area_id: latest.area_id,
        room_id: latest.room_id,
        serial: Some(latest.serial.clone()),
        device_slot: None,
        reporter_id: None,
        metadata: json!({
            "target": target,
            "local_date": local_date,
            "event_id": latest.event_id,
            "delivery_id": latest.delivery_id,
            "serial": latest.serial,
            "failed_count": fresh.len(),
            "errors": errors,
        }),
    }
}

fn transfer_ticket(signal: &TransferSignal, priority: TicketPriority) -> NewTicket {
    let description = format!(
        "房间 {} {} 的存储分发任务进入 {} 终态。\n错误：{}",
        signal.room_code,
        signal.room_name,
        signal.status,
        signal.error.as_deref().unwrap_or("（无）")
    );
    NewTicket {
        title: format!(
            "分发失败：{} / {}",
            signal.room_code,
            &signal.transfer_id.to_string()[..8]
        ),
        description,
        priority,
        category: TicketCategory::DistributionFailed,
        source: TicketSource::Auto,
        source_type: Some("transfer".to_owned()),
        source_id: Some(signal.transfer_id.to_string()),
        dedupe_key: Some(format!("transfer:{}", signal.transfer_id)),
        area_id: Some(signal.area_id),
        room_id: Some(signal.room_id),
        serial: None,
        device_slot: None,
        reporter_id: None,
        metadata: json!({
            "transfer_id": signal.transfer_id,
            "status": signal.status,
            "error": signal.error,
            "room_id": signal.room_id,
            "room_code": signal.room_code,
            "updated_at": signal.updated_at,
        }),
    }
}

fn image_verify_ticket(signal: &ImageVerifySignal, priority: TicketPriority) -> NewTicket {
    let description = format!(
        "镜像 {}:{} 验证失败（verification {}）。\n请查看验证报告后重新上传/验证。",
        signal.image_name,
        signal.image_version,
        &signal.verification_id.to_string()[..8]
    );
    NewTicket {
        title: format!(
            "镜像验证失败：{}:{}",
            signal.image_name, signal.image_version
        ),
        description,
        priority,
        category: TicketCategory::DistributionFailed,
        source: TicketSource::Auto,
        source_type: Some("image_verify".to_owned()),
        source_id: Some(signal.verification_id.to_string()),
        dedupe_key: Some(format!("image_verify:{}", signal.artifact_id)),
        area_id: None,
        room_id: None,
        serial: None,
        device_slot: None,
        reporter_id: None,
        metadata: json!({
            "artifact_id": signal.artifact_id,
            "verification_id": signal.verification_id,
            "image_name": signal.image_name,
            "image_version": signal.image_version,
            "report": signal.report,
            "verified_at": signal.verified_at,
        }),
    }
}

/// 机器健康：首轮把已异常 serial 记为抑制基线；之后只对“新出现”的异常建单，
/// 恢复后解除抑制。
fn plan_machine_health(
    tracker: &mut TicketWorkerTracker,
    signals: Vec<MachineHealthSignal>,
    baseline: bool,
    priority: TicketPriority,
) -> Vec<NewTicket> {
    let current: HashSet<String> = signals.iter().map(|item| item.serial.clone()).collect();
    if baseline {
        tracker.health_suppressed = current;
        return Vec::new();
    }
    tracker
        .health_suppressed
        .retain(|serial| current.contains(serial));

    let mut plans = Vec::new();
    for signal in signals {
        if tracker.health_suppressed.contains(&signal.serial) {
            continue;
        }
        tracker.health_suppressed.insert(signal.serial.clone());
        plans.push(machine_health_ticket(signal, priority));
    }
    plans
}

/// 设备离线：首轮记录基线；之后只对“新判定离线”的 serial 建单，恢复后移除。
fn plan_device_offline(
    tracker: &mut TicketWorkerTracker,
    signals: Vec<DeviceOfflineSignal>,
    baseline: bool,
    priority: TicketPriority,
) -> Vec<NewTicket> {
    let current: HashSet<String> = signals.iter().map(|item| item.serial.clone()).collect();
    if baseline {
        tracker.offline_over_threshold = current;
        return Vec::new();
    }
    let mut plans = Vec::new();
    for signal in &signals {
        if tracker.offline_over_threshold.insert(signal.serial.clone()) {
            plans.push(device_offline_ticket(signal.clone(), priority));
        }
    }
    tracker
        .offline_over_threshold
        .retain(|serial| current.contains(serial));
    plans
}

/// 课表投递失败：按 `(room_id, target, local_date)` 聚合；只处理本轮新出现的投递。
fn plan_schedule_delivery(
    tracker: &mut TicketWorkerTracker,
    signals: Vec<FailedDeliverySignal>,
    baseline: bool,
    priority: TicketPriority,
) -> Vec<NewTicket> {
    let current_ids: HashSet<i64> = signals.iter().map(|signal| signal.delivery_id).collect();
    if baseline {
        tracker.seen_deliveries = current_ids;
        return Vec::new();
    }
    let previous = std::mem::take(&mut tracker.seen_deliveries);

    let mut groups: HashMap<(Uuid, String, String), Vec<FailedDeliverySignal>> = HashMap::new();
    for signal in signals {
        let key = (
            signal.room_id,
            signal.target.clone(),
            signal.local_date.clone(),
        );
        groups.entry(key).or_default().push(signal);
    }
    let mut plans = Vec::new();
    for (_, group) in groups {
        let fresh: Vec<FailedDeliverySignal> = group
            .iter()
            .filter(|signal| !previous.contains(&signal.delivery_id))
            .cloned()
            .collect();
        if !fresh.is_empty() {
            plans.push(schedule_delivery_ticket(&fresh, priority));
        }
    }
    tracker.seen_deliveries = current_ids;
    plans
}

/// 在线事件投递失败：按 `(target, local_date)` 聚合；只处理本轮新出现的投递。
fn plan_presence_delivery(
    tracker: &mut TicketWorkerTracker,
    signals: Vec<PresenceDeliverySignal>,
    baseline: bool,
    priority: TicketPriority,
) -> Vec<NewTicket> {
    let current_ids: HashSet<i64> = signals.iter().map(|signal| signal.delivery_id).collect();
    if baseline {
        tracker.seen_presence_deliveries = current_ids;
        return Vec::new();
    }
    let previous = std::mem::take(&mut tracker.seen_presence_deliveries);

    let mut groups: HashMap<(String, String), Vec<PresenceDeliverySignal>> = HashMap::new();
    for signal in signals {
        groups
            .entry((signal.target.clone(), signal.local_date.clone()))
            .or_default()
            .push(signal);
    }
    let mut plans = Vec::new();
    for ((target, local_date), group) in groups {
        let fresh: Vec<PresenceDeliverySignal> = group
            .iter()
            .filter(|signal| !previous.contains(&signal.delivery_id))
            .cloned()
            .collect();
        if !fresh.is_empty() {
            plans.push(presence_delivery_ticket(
                &fresh,
                &target,
                &local_date,
                priority,
            ));
        }
    }
    tracker.seen_presence_deliveries = current_ids;
    plans
}

/// 存储分发失败：一任务一单；只处理新出现的终态。
fn plan_transfer(
    tracker: &mut TicketWorkerTracker,
    signals: Vec<TransferSignal>,
    baseline: bool,
    priority: TicketPriority,
) -> Vec<NewTicket> {
    let current_ids: HashSet<Uuid> = signals.iter().map(|signal| signal.transfer_id).collect();
    if baseline {
        tracker.seen_transfers = current_ids;
        return Vec::new();
    }
    let previous = std::mem::take(&mut tracker.seen_transfers);
    let plans = signals
        .iter()
        .filter(|signal| !previous.contains(&signal.transfer_id))
        .map(|signal| transfer_ticket(signal, priority))
        .collect();
    tracker.seen_transfers = current_ids;
    plans
}

/// 镜像验证失败：一 artifact 一单；只处理新出现的失败验证。
fn plan_image_verify(
    tracker: &mut TicketWorkerTracker,
    signals: Vec<ImageVerifySignal>,
    baseline: bool,
    priority: TicketPriority,
) -> Vec<NewTicket> {
    let current_ids: HashSet<Uuid> = signals
        .iter()
        .map(|signal| signal.verification_id)
        .collect();
    if baseline {
        tracker.seen_image_verifications = current_ids;
        return Vec::new();
    }
    let previous = std::mem::take(&mut tracker.seen_image_verifications);
    let plans = signals
        .iter()
        .filter(|signal| !previous.contains(&signal.verification_id))
        .map(|signal| image_verify_ticket(signal, priority))
        .collect();
    tracker.seen_image_verifications = current_ids;
    plans
}

fn payload_health_issues(payload: &Value) -> Vec<Value> {
    payload
        .pointer("/health/issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn describe_health_issues(issues: &[Value]) -> String {
    let mut lines = String::new();
    for issue in issues.iter().take(10) {
        let severity = issue
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("warning");
        let code = issue
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let message = issue
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("（无描述）");
        lines.push_str(&format!("- [{severity}] {code}: {message}\n"));
    }
    lines
}

fn describe_errors(description: &mut String, errors: &[String]) {
    if errors.is_empty() {
        return;
    }
    description.push_str("\n最近错误：\n");
    for error in errors {
        description.push_str("- ");
        description.push_str(error);
        description.push('\n');
    }
}

// ----------------------------------------------------------------------
// Wave 3：巡检计划（inspection plans）
// ----------------------------------------------------------------------

/// 每 tick 最多执行的巡检计划数。
const INSPECTION_BATCH_LIMIT: i64 = 20;

/// 领取并执行到期计划；返回实际执行条数。
async fn inspection_tick(state: &AppState) -> anyhow::Result<usize> {
    let plans = state
        .ticket_inspections
        .claim_due_plans(INSPECTION_BATCH_LIMIT)
        .await?;
    let mut executed = 0;
    for plan in &plans {
        match run_plan(state, plan).await {
            Ok(run) => {
                executed += 1;
                tracing::info!(
                    plan = %plan.name,
                    run_id = %run.id,
                    status = run.status.as_str(),
                    checked = run.checked_machines,
                    issues = run.issue_machines,
                    "巡检计划已执行"
                );
            }
            Err(error) => tracing::warn!(%error, plan = %plan.name, "巡检计划执行失败"),
        }
    }
    Ok(executed)
}

/// 执行一次巡检计划（worker 到期执行与 API 手动执行共用）。
///
/// 流程：建 running run → 查询 scope 内机器摘要 → 统计异常 → 有异常则建工单并关联
/// run → finish_run。查询/建单失败时把 run 置 `failed` 并记录 error，不向上抛错
/// （保证运维能看到失败记录，且不阻塞其它计划）。
pub async fn run_plan(state: &AppState, plan: &InspectionPlan) -> anyhow::Result<InspectionRun> {
    let repo = &state.ticket_inspections;
    let run = repo.insert_run(plan.id).await?;

    let machines = match repo
        .machine_summary_in_scope(plan.scope_type, plan.scope_id)
        .await
    {
        Ok(machines) => machines,
        Err(error) => {
            return fail_run(state, run, plan, &error.to_string()).await;
        }
    };

    let checked = machines
        .iter()
        .filter(|machine| machine.health_status.is_some())
        .count() as i64;
    let issues: Vec<&InspectionMachineSummary> = machines
        .iter()
        .filter(|machine| {
            machine
                .health_status
                .as_deref()
                .is_some_and(|status| status != "ok")
        })
        .collect();
    let issue_count = issues.len() as i64;
    let has_critical = issues
        .iter()
        .any(|machine| machine.health_status.as_deref() == Some("critical"));
    let summary = inspection_summary(plan, checked, issue_count, &issues);

    if issue_count == 0 {
        return Ok(repo
            .finish_run(
                run.id,
                plan.id,
                InspectionRunStatus::Ok,
                checked,
                0,
                None,
                &summary,
                None,
            )
            .await?);
    }

    let ticket = match state
        .tickets
        .insert_ticket(&inspection_ticket(
            plan,
            &run,
            checked,
            &issues,
            has_critical,
        ))
        .await
    {
        Ok(ticket) => ticket,
        Err(error) => {
            return fail_run(state, run, plan, &error.to_string()).await;
        }
    };
    let run = repo
        .finish_run(
            run.id,
            plan.id,
            InspectionRunStatus::Issues,
            checked,
            issue_count,
            Some(ticket.id),
            &summary,
            None,
        )
        .await?;
    Ok(run)
}

async fn fail_run(
    state: &AppState,
    run: InspectionRun,
    plan: &InspectionPlan,
    error: &str,
) -> anyhow::Result<InspectionRun> {
    let summary = json!({
        "plan_id": plan.id,
        "plan_name": plan.name,
        "error": error,
    });
    Ok(state
        .ticket_inspections
        .finish_run(
            run.id,
            plan.id,
            InspectionRunStatus::Failed,
            0,
            0,
            None,
            &summary,
            Some(error),
        )
        .await?)
}

fn inspection_summary(
    plan: &InspectionPlan,
    checked: i64,
    issue_count: i64,
    issues: &[&InspectionMachineSummary],
) -> Value {
    json!({
        "plan_id": plan.id,
        "plan_name": plan.name,
        "scope_type": plan.scope_type.as_str(),
        "scope_id": plan.scope_id,
        "checked_machines": checked,
        "issue_machines": issue_count,
        "issues": issues
            .iter()
            .map(|machine| json!({
                "serial": machine.serial,
                "device_slot": machine.device_slot,
                "room_id": machine.room_id,
                "room_name": machine.room_name,
                "area_id": machine.area_id,
                "health_status": machine.health_status,
                "health_score": machine.health_score,
                "issues": machine.issues,
                "collected_at": machine.collected_at,
            }))
            .collect::<Vec<_>>(),
    })
}

fn inspection_ticket(
    plan: &InspectionPlan,
    run: &InspectionRun,
    checked: i64,
    issues: &[&InspectionMachineSummary],
    has_critical: bool,
) -> NewTicket {
    let mut description = format!(
        "巡检计划「{}」检查了 {} 台机器，其中 {} 台异常。",
        plan.name,
        checked,
        issues.len()
    );
    for machine in issues.iter().take(20) {
        description.push_str(&format!(
            "\n- {}（{}）：{}",
            machine.serial,
            machine.room_name.as_deref().unwrap_or("未绑定房间"),
            machine.health_status.as_deref().unwrap_or("unknown")
        ));
        for issue in machine.issues.iter().take(3) {
            if let Some(message) = issue.get("message").and_then(Value::as_str) {
                description.push_str(&format!("；{message}"));
            }
        }
    }

    let (area_id, room_id) = match plan.scope_type {
        TicketScopeType::Global => (None, None),
        TicketScopeType::Area => (plan.scope_id, None),
        TicketScopeType::Room => (
            issues.first().and_then(|machine| machine.area_id),
            plan.scope_id,
        ),
    };
    let summary = inspection_summary(plan, checked, issues.len() as i64, issues);
    let mut metadata = summary;
    if let Some(object) = metadata.as_object_mut() {
        object.insert("run_id".to_owned(), json!(run.id));
        object.insert("ticket_source".to_owned(), json!("inspection"));
    }

    NewTicket {
        title: format!("巡检报告：{} — {} 台异常", plan.name, issues.len()),
        description,
        priority: if has_critical {
            TicketPriority::High
        } else {
            TicketPriority::Normal
        },
        category: TicketCategory::MachineHealth,
        source: TicketSource::Auto,
        source_type: Some("inspection".to_owned()),
        source_id: Some(run.id.to_string()),
        dedupe_key: Some(format!("inspection:{}:{}", plan.id, run.id)),
        area_id,
        room_id,
        serial: None,
        device_slot: None,
        reporter_id: None,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn machine_signal(serial: &str, snapshot_id: i64) -> MachineHealthSignal {
        MachineHealthSignal {
            serial: serial.to_owned(),
            device_slot: format!("slot-{serial}"),
            snapshot_id,
            health_status: "warning".to_owned(),
            health_score: Some(80),
            collected_at: Utc::now(),
            payload: json!({}),
            room_id: None,
            area_id: None,
        }
    }

    fn offline_signal(serial: &str) -> DeviceOfflineSignal {
        DeviceOfflineSignal {
            serial: serial.to_owned(),
            device_slot: format!("slot-{serial}"),
            session_updated_at: Utc::now(),
            room_id: Uuid::nil(),
            room_code: "room".to_owned(),
            room_name: "房间".to_owned(),
            area_id: Uuid::nil(),
            area_name: "区域".to_owned(),
        }
    }

    fn delivery_signal(delivery_id: i64, room: Uuid, target: &str) -> FailedDeliverySignal {
        FailedDeliverySignal {
            delivery_id,
            event_id: Uuid::nil(),
            room_id: room,
            room_code: "room".to_owned(),
            room_name: "房间".to_owned(),
            area_id: Uuid::nil(),
            target: target.to_owned(),
            attempts: 5,
            last_error: Some("timeout".to_owned()),
            triggered_at: Utc::now(),
            payload: json!({}),
            local_date: "2026-11-02".to_owned(),
        }
    }

    #[test]
    fn machine_health_baseline_is_not_replayed() {
        let mut tracker = TicketWorkerTracker::default();
        let signals = vec![machine_signal("s1", 1), machine_signal("s2", 2)];
        assert!(
            plan_machine_health(&mut tracker, signals, true, TicketPriority::Normal).is_empty()
        );
        assert_eq!(tracker.health_suppressed.len(), 2);

        let plans = plan_machine_health(
            &mut tracker,
            vec![machine_signal("s1", 3), machine_signal("s2", 4)],
            false,
            TicketPriority::Normal,
        );
        assert!(plans.is_empty(), "已在基线中的 serial 不应重复建单");
    }

    #[test]
    fn device_offline_baseline_and_recovery() {
        let mut tracker = TicketWorkerTracker::default();
        assert!(
            plan_device_offline(
                &mut tracker,
                vec![offline_signal("s1")],
                true,
                TicketPriority::Normal
            )
            .is_empty()
        );
        assert!(
            plan_device_offline(
                &mut tracker,
                vec![offline_signal("s1")],
                false,
                TicketPriority::Normal
            )
            .is_empty()
        );
        assert!(
            plan_device_offline(&mut tracker, vec![], false, TicketPriority::Normal).is_empty()
        );
        let plans = plan_device_offline(
            &mut tracker,
            vec![offline_signal("s1")],
            false,
            TicketPriority::Normal,
        );
        assert_eq!(plans.len(), 1);
    }

    #[test]
    fn delivery_failures_group_by_room_target_and_day() {
        let mut tracker = TicketWorkerTracker::default();
        let room = Uuid::new_v4();
        let first = vec![
            delivery_signal(1, room, "https://a"),
            delivery_signal(2, room, "https://a"),
        ];
        assert!(
            plan_schedule_delivery(&mut tracker, first, true, TicketPriority::Normal).is_empty()
        );
        assert_eq!(tracker.seen_deliveries.len(), 2);

        let second = delivery_signal(3, room, "https://a");
        let plans =
            plan_schedule_delivery(&mut tracker, vec![second], false, TicketPriority::Normal);
        assert_eq!(plans.len(), 1);
        assert_eq!(
            plans[0].dedupe_key.as_deref(),
            Some(format!("schedule_delivery:{room}:https://a:2026-11-02").as_str())
        );
    }

    #[test]
    fn presence_delivery_groups_and_honours_priority() {
        let mut tracker = TicketWorkerTracker::default();
        let signal = PresenceDeliverySignal {
            delivery_id: 7,
            event_id: 3,
            serial: "s1".to_owned(),
            target: "https://hook".to_owned(),
            last_error: Some("500".to_owned()),
            occurred_at: Utc::now(),
            room_id: None,
            area_id: None,
            local_date: "2026-11-02".to_owned(),
        };
        assert!(
            plan_presence_delivery(&mut tracker, vec![], true, TicketPriority::High).is_empty()
        );
        let plans = plan_presence_delivery(&mut tracker, vec![signal], false, TicketPriority::High);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].priority, TicketPriority::High);
        assert_eq!(
            plans[0].dedupe_key.as_deref(),
            Some("presence_delivery:https://hook:2026-11-02")
        );
    }

    #[test]
    fn transfer_and_image_verify_only_plan_new_rows() {
        let mut tracker = TicketWorkerTracker::default();
        let transfer = TransferSignal {
            transfer_id: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            room_code: "room".to_owned(),
            room_name: "房间".to_owned(),
            area_id: Uuid::new_v4(),
            area_name: "区域".to_owned(),
            status: "failed".to_owned(),
            error: Some("disk full".to_owned()),
            updated_at: Utc::now(),
            local_date: "2026-11-02".to_owned(),
        };
        assert!(plan_transfer(&mut tracker, vec![], true, TicketPriority::Normal).is_empty());
        let plans = plan_transfer(&mut tracker, vec![transfer], false, TicketPriority::Normal);
        assert_eq!(plans.len(), 1);

        let verify = ImageVerifySignal {
            verification_id: Uuid::new_v4(),
            artifact_id: Uuid::new_v4(),
            image_name: "ubuntu".to_owned(),
            image_version: "24.04".to_owned(),
            report: json!({ "checks": [] }),
            verified_at: Utc::now(),
        };
        assert!(plan_image_verify(&mut tracker, vec![], true, TicketPriority::Normal).is_empty());
        let plans = plan_image_verify(&mut tracker, vec![verify], false, TicketPriority::Normal);
        assert_eq!(plans.len(), 1);
    }
}
