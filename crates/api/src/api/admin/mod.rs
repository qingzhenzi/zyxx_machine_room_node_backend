pub mod agent_configs;
pub mod announcements;
pub mod areas;
pub mod audit;
pub mod devices;
pub mod images;
pub mod roles;
pub mod rooms;
pub mod schedule_events;
pub mod schedules;
pub mod storage;
pub mod users;

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
        .route("/rooms/{id}/occurrences", get(schedules::room_occurrences))
        .route(
            "/rooms/{id}/event-rules",
            get(schedule_events::list_event_rules).put(schedule_events::replace_event_rules),
        )
        .route("/schedules", get(schedules::list).post(schedules::create))
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
        .route("/images/{id}", get(images::get))
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
        // ---- 节点配置（P2）----
        .route(
            "/agent-configs",
            get(agent_configs::list).put(agent_configs::update),
        )
        .route("/agent-configs/effective", get(agent_configs::effective))
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
