pub mod announcements;
pub mod roles;
pub mod users;

use axum::{
    Router,
    routing::{delete as delete_route, get, patch as patch_route, put},
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
}
