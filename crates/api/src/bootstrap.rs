use crate::{
    auth::{authorization, password},
    domain::{rbac::roles, validation},
    error::AppResult,
    state::AppState,
};

/// 启动时保证：
/// 1. 系统管理员角色拥有当前全部权限；
/// 2. 如果配置了引导管理员且账号不存在，则创建之（幂等）。
pub async fn ensure_bootstrap_admin(state: &AppState) -> AppResult<()> {
    let granted = state.roles.grant_all_permissions_to_admin().await?;
    if granted > 0 {
        tracing::info!(granted, "已为系统管理员角色补充新权限");
    }
    // 迁移（如 0018_tickets.sql）里已经给 admin 全量授权时 `granted == 0`，
    // 但其它副本/旧进程可能仍持有 5 分钟 TTL 的旧权限缓存（比如只有 21 项权限），
    // 新权限会在缓存过期前一直 403。这里每次进程启动都 bump 一次权限代际，
    // 让新权限立即生效；多副本滚动部署时每个副本各失效一次，代价可接受。
    authorization::bump_permission_version(state).await;

    let Some(username) = state.config.bootstrap_admin_username.clone() else {
        return Ok(());
    };
    let Some(password) = state.config.bootstrap_admin_password.clone() else {
        tracing::warn!(
            "设置了 BOOTSTRAP_ADMIN_USERNAME 但未设置 BOOTSTRAP_ADMIN_PASSWORD，跳过管理员初始化"
        );
        return Ok(());
    };

    let username = validation::normalize_username(&username)?;
    validation::validate_password(&password)?;
    let display_name =
        validation::normalize_display_name(&state.config.bootstrap_admin_display_name)?;

    if state.users.find_by_username(&username).await?.is_some() {
        tracing::info!(%username, "引导管理员已存在，跳过创建");
        return Ok(());
    }

    let password_hash = password::hash_password_async(password).await?;
    let admin_role = vec![roles::ADMIN.to_owned()];

    match state
        .users
        .create_with_roles(&username, None, &display_name, &password_hash, &admin_role)
        .await
    {
        Ok(user) => {
            tracing::warn!(
                %username,
                user_id = %user.id,
                "已创建引导管理员，请立即修改默认密码"
            );
        }
        // 多副本同时启动时另一个副本可能已经创建成功。
        Err(crate::db::DbError::Conflict(_)) => {
            tracing::info!(%username, "引导管理员已被其他副本创建，跳过");
        }
        Err(error) => return Err(error.into()),
    }

    Ok(())
}
