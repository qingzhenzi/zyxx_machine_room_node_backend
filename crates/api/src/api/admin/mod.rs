pub mod agent_configs;
pub mod announcements;
pub mod areas;
pub mod audit;
pub mod device_events;
pub mod devices;
pub mod federation;
pub mod images;
pub mod machines;
pub mod roles;
pub mod rooms;
pub mod schedule_board;
pub mod schedule_events;
pub mod schedule_plans;
pub mod schedules;
pub mod storage;
pub mod ticket_attachments;
pub mod ticket_inspections;
pub mod ticket_rules;
pub mod ticket_scopes;
pub mod tickets;
pub mod users;
pub mod vnc;

use axum::{
    Router,
    routing::{delete as delete_route, get, patch as patch_route, post, put},
};

use crate::state::AppState;

/// `/api/v1/admin` 下的全部接口。每个 handler 自行做权限校验。
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/announcements",
            get(announcements::list).post(announcements::create),
        )
        .route(
            "/announcements/{id}",
            patch_route(announcements::update).delete(announcements::delete),
        )
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", patch_route(users::update))
        .route("/users/{id}/roles", put(users::set_roles))
        .route("/roles", get(roles::list).post(roles::create))
        .route("/roles/{id}", delete_route(roles::delete))
        .route("/roles/{id}/permissions", put(roles::set_permissions))
        .route("/permissions", get(roles::list_permissions))
        .route("/audit-logs", get(audit::list))
        // ---- 课表与事件 ----
        .route("/areas", get(areas::list).post(areas::create))
        .route(
            "/areas/{id}",
            get(areas::get).patch(areas::update).delete(areas::delete),
        )
        .route("/rooms", get(rooms::list).post(rooms::create))
        .route(
            "/rooms/{id}",
            get(rooms::get).patch(rooms::update).delete(rooms::delete),
        )
        // ---- 设备 —— 区域/房间（v3）----
        .route("/devices", get(devices::list))
        .route("/devices/{serial}", get(devices::get))
        .route(
            "/rooms/{id}/device",
            get(devices::get_room_device).put(devices::bind_room_device),
        )
        .route(
            "/rooms/{id}/device-token",
            post(devices::issue_device_token),
        )
        // ---- 一个房间多台机器（P1）：主设备之外再加子机 ----
        .route("/rooms/{id}/devices", get(devices::list_room_machines))
        .route(
            "/rooms/{id}/devices/token",
            post(devices::issue_machine_token),
        )
        .route(
            "/rooms/{id}/devices/{slot}",
            delete_route(devices::remove_room_machine),
        )
        .route("/rooms/{id}/occurrences", get(schedules::room_occurrences))
        .route(
            "/rooms/{id}/event-rules",
            get(schedule_events::list_event_rules).put(schedule_events::replace_event_rules),
        )
        .route("/schedules", get(schedules::list).post(schedules::create))
        // 静态段 `/schedules/ensure` 必须先于 `/schedules/{id}` 的匹配语义注册：
        // axum 的 matchit 会优先匹配静态段，这里按可读性放在同组。
        .route("/schedules/ensure", post(schedule_board::ensure))
        .route("/schedule-board", get(schedule_board::board))
        .route(
            "/schedules/{id}",
            get(schedules::get)
                .patch(schedules::update)
                .delete(schedules::delete),
        )
        .route(
            "/schedules/{id}/weekly-slots",
            get(schedules::weekly_slots).put(schedules::replace_weekly_slots),
        )
        .route("/schedules/{id}/exceptions", get(schedules::exceptions))
        .route(
            "/schedules/{id}/exceptions/{date}",
            put(schedules::upsert_exception).delete(schedules::delete_exception),
        )
        .route("/schedules/{id}/occurrences", get(schedules::occurrences))
        .route("/schedules/{id}/rebuild", post(schedules::rebuild))
        // ---- 排课全局模型（路径 2 Phase A）：主数据 + 排课计划 ----
        .route(
            "/terms",
            get(schedule_plans::term_list).post(schedule_plans::term_create),
        )
        .route(
            "/terms/{id}",
            get(schedule_plans::term_get)
                .patch(schedule_plans::term_update)
                .delete(schedule_plans::term_delete),
        )
        .route(
            "/teachers",
            get(schedule_plans::teacher_list).post(schedule_plans::teacher_create),
        )
        .route(
            "/teachers/{id}",
            get(schedule_plans::teacher_get)
                .patch(schedule_plans::teacher_update)
                .delete(schedule_plans::teacher_delete),
        )
        .route(
            "/classes",
            get(schedule_plans::class_list).post(schedule_plans::class_create),
        )
        .route(
            "/classes/{id}",
            get(schedule_plans::class_get)
                .patch(schedule_plans::class_update)
                .delete(schedule_plans::class_delete),
        )
        .route(
            "/courses",
            get(schedule_plans::course_list).post(schedule_plans::course_create),
        )
        .route(
            "/courses/{id}",
            get(schedule_plans::course_get)
                .patch(schedule_plans::course_update)
                .delete(schedule_plans::course_delete),
        )
        .route(
            "/schedule-plans",
            get(schedule_plans::plan_list).post(schedule_plans::plan_create),
        )
        .route(
            "/schedule-plans/{id}",
            get(schedule_plans::plan_get)
                .patch(schedule_plans::plan_update)
                .delete(schedule_plans::plan_delete),
        )
        .route(
            "/schedule-plans/{id}/clone",
            post(schedule_plans::plan_clone),
        )
        .route(
            "/schedule-plans/{id}/entries",
            get(schedule_plans::plan_entries).put(schedule_plans::plan_replace_entries),
        )
        .route(
            "/schedule-plans/{id}/publish",
            post(schedule_plans::plan_publish),
        )
        .route("/schedule-events", get(schedule_events::list_events))
        .route("/schedule-events/{id}", get(schedule_events::get_event))
        .route(
            "/schedule-events/{id}/replay",
            post(schedule_events::replay_event),
        )
        .route(
            "/schedule-events/{id}/deliveries/{delivery_id}/retry",
            post(schedule_events::retry_delivery),
        )
        // ---- 资源与镜像（P0）----
        .route("/storage/objects/presign", post(storage::presign_object))
        .route("/storage/objects", post(storage::register_object))
        .route("/storage/objects/{sha256}", get(storage::get_object))
        .route("/transfers", get(storage::list_transfers))
        .route("/transfers/{id}/retry", post(storage::retry_transfer))
        .route("/collections", get(storage::list_collections))
        .route("/images", get(images::list).post(images::create))
        .route("/images/{id}", get(images::get).delete(images::remove))
        .route("/images/{id}/deprecate", post(images::deprecate))
        .route(
            "/images/{id}/manifest",
            get(images::get_manifest).post(images::submit_manifest),
        )
        .route("/images/{id}/verify", post(images::verify))
        .route("/images/{id}/publish", post(images::publish))
        .route(
            "/rooms/{id}/images/{image_id}",
            put(images::assign_room).delete(images::unassign_room),
        )
        // ---- 机器总览（room-agent 上报）----
        .route("/machines", get(machines::list))
        // 必须在 `/machines/{serial}` 之前注册静态路径，否则会被动态段吞掉。
        .route("/machines/stream", get(machines::stream))
        .route("/machines/{serial}", get(machines::get))
        .route("/machines/{serial}/snapshots", get(machines::snapshots))
        .route(
            "/machines/{serial}/telemetry/report",
            post(machines::request_telemetry_report),
        )
        .route(
            "/machines/{serial}/agent/poweroff",
            post(machines::create_agent_poweroff),
        )
        // WOL：需要 device:wake（迁移 0030 播种；默认仅 admin 角色）。
        .route("/machines/{serial}/wake", post(machines::wake))
        // VM 远程控制（kind=vm.command 经 device_downlink 下发；权限 vm:control，迁移 0032）。
        .route(
            "/machines/{serial}/vm/commands",
            post(machines::create_vm_command),
        )
        .route(
            "/machines/{serial}/vm/commands/{request_id}",
            get(machines::get_vm_command),
        )
        // 容器生命周期（Incus 后端；权限同 vm:control，实验环境控制）。
        .route(
            "/machines/{serial}/container/commands",
            post(machines::create_container_command),
        )
        .route(
            "/machines/{serial}/container/commands/{request_id}",
            get(machines::get_vm_command),
        )
        // 教师批量下发实验环境（room 级；逐槽位 create，权限 vm:control）。
        .route(
            "/rooms/{room_id}/vm/environments",
            post(machines::create_room_vm_environment),
        )
        // noVNC 阶段 2：VNC 控制台会话 + 浏览器 WS 代理（vm:control）。
        .route("/machines/{serial}/vnc/session", post(vnc::create_session))
        .route("/machines/{serial}/vnc/ws", get(vnc::ws_proxy))
        // ---- 总控阶段 2：多控制节点联邦（注册/刷新/只读聚合）----
        .route("/federation/summary", get(federation::summary))
        .route("/federation/overview", get(federation::overview))
        .route(
            "/federation/nodes",
            get(federation::list).post(federation::create),
        )
        .route(
            "/federation/nodes/{id}",
            axum::routing::delete(federation::delete),
        )
        .route("/federation/nodes/{id}/refresh", post(federation::refresh))
        // ---- 节点配置（P2）----
        .route(
            "/agent-configs",
            get(agent_configs::list).put(agent_configs::update),
        )
        .route("/agent-configs/effective", get(agent_configs::effective))
        // ---- 设备在线事件与可靠投递（P3）----
        .route("/device-events", get(device_events::list))
        .route("/device-events/stream", get(device_events::stream))
        .route(
            "/device-presence-deliveries",
            get(device_events::list_deliveries),
        )
        // ---- 工单（P0）----
        // 静态段必须在 /tickets/{id} 之前注册（axum matchit 静态优先，这里保持可读顺序）。
        .route("/tickets", get(tickets::list).post(tickets::create))
        .route("/tickets/summary", get(tickets::summary))
        .route("/tickets/stats", get(tickets::stats))
        .route("/tickets/assignees", get(tickets::assignees))
        .route("/tickets/export", get(tickets::export))
        .route("/tickets/stream", get(tickets::stream))
        .route("/tickets/{id}", get(tickets::get).patch(tickets::update))
        .route("/tickets/{id}/claim", post(tickets::claim))
        .route("/tickets/{id}/assign", post(tickets::assign))
        .route("/tickets/{id}/transition", post(tickets::transition))
        .route("/tickets/{id}/comments", post(tickets::comments))
        // ---- 工单自动规则（P1）----
        .route(
            "/ticket-rules",
            get(ticket_rules::list).post(ticket_rules::create),
        )
        .route(
            "/ticket-rules/{id}",
            patch_route(ticket_rules::update).delete(ticket_rules::delete),
        )
        // ---- 工单数据范围（P1 Wave2）----
        .route("/ticket-scopes", get(ticket_scopes::list))
        .route("/ticket-scopes/areas", get(ticket_scopes::areas))
        .route("/ticket-scopes/{user_id}", put(ticket_scopes::replace))
        // ---- 工单附件（P1 Wave2）----
        .route(
            "/tickets/{id}/attachments",
            get(ticket_attachments::list).post(ticket_attachments::register),
        )
        .route(
            "/tickets/{id}/attachments/upload",
            post(ticket_attachments::upload),
        )
        .route(
            "/tickets/{id}/attachments/presign",
            post(ticket_attachments::presign),
        )
        .route(
            "/tickets/{id}/attachments/{attachment_id}/download",
            get(ticket_attachments::download),
        )
        .route(
            "/tickets/{id}/attachments/{attachment_id}",
            delete_route(ticket_attachments::delete),
        )
        // ---- 巡检计划（Wave 3）----
        .route(
            "/inspection-plans",
            get(ticket_inspections::list).post(ticket_inspections::create),
        )
        .route(
            "/inspection-plans/{id}",
            patch_route(ticket_inspections::update).delete(ticket_inspections::delete),
        )
        .route("/inspection-plans/{id}/run", post(ticket_inspections::run))
        .route("/inspection-runs", get(ticket_inspections::runs))
}

/// 反序列化 `Option<Option<T>>`：区分“字段缺失”（`None`）与显式 `null`（`Some(None)`）。
///
/// serde 默认会把显式 `null` 也解析成外层 `None`，导致 PATCH 的“清空”语义失效；
/// 所有可空 PATCH 字段都应使用 `#[serde(default, deserialize_with = "deserialize_nullable")]`。
pub(super) fn deserialize_nullable<'de, T, D>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Option::<T>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 路由注册在构造时就会检测冲突（同路径同方法、重叠的动态段等），
    /// 这里确保课表相关路由与既有路由可以共存。
    #[test]
    fn router_has_no_route_conflicts() {
        let _router = router();
    }
}
