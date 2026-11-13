//! 镜像构建会话后台回收（契约 `docs/image-studio.md` §3.2）。
//!
//! 教师关掉页面/机器掉线后，构建会话可能停在非终态。本 worker 每 60s 扫描一次：
//! `state NOT IN (done, failed, cancelled, expired)` 且 `updated_at < now()-2h`
//! → 置 `expired`，并尽力回收构建 VM（destroy + delete remove_disks=true）、
//! 清理尚未消费的 draft 镜像，写 `image_events(actor_kind=system)`。
//!
//! 多副本安全：`expire_stale_sessions` 用 `FOR UPDATE SKIP LOCKED` + CAS，同一行
//! 只会被一个副本回收；VM 回收命令是幂等语义（destroy/delete 不存在的 VM 由 agent
//! 返回错误，只 warn）。

use std::time::Duration;

use tokio::time::{MissedTickBehavior, interval};

use qe_domain::image_studio::{BuildSession, SESSION_STALE_HOURS};

use crate::{
    domain::storage::{ArtifactStatus, ImageEventActorKind, NewImageEvent},
    state::AppState,
};

/// 扫描周期：60s。
const REAPER_TICK: Duration = Duration::from_secs(60);

/// 启动后台构建会话回收（生命周期跟随进程）。
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(REAPER_TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match state
                .image_studio
                .expire_stale_sessions(SESSION_STALE_HOURS * 60)
                .await
            {
                Ok(sessions) if sessions.is_empty() => {}
                Ok(sessions) => {
                    tracing::info!(expired = sessions.len(), "镜像构建会话超时回收完成");
                    for session in sessions {
                        reap_session(&state, session).await;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "镜像构建会话超时回收失败，下轮重试");
                }
            }
        }
    })
}

/// 回收一个已置 `expired` 的会话：销毁/删除构建 VM + 清理 draft + 写事件。
async fn reap_session(state: &AppState, session: BuildSession) {
    if let Ok(Some(machine)) = state.machines.summary(&session.machine_serial).await {
        let slot = machine.device_slot;
        for (op, args) in [
            (
                "destroy",
                serde_json::json!({ "name": session.build_vm_name }),
            ),
            (
                "delete",
                serde_json::json!({ "name": session.build_vm_name, "remove_disks": true }),
            ),
        ] {
            if let Err(error) =
                crate::api::admin::machines::enqueue_internal_vm_command(state, &slot, op, args)
                    .await
            {
                tracing::warn!(
                    session_id = %session.id,
                    %op,
                    %error,
                    "超时回收：vm.command 下发失败（尽力而为，下轮不再重试）"
                );
            }
        }
    } else {
        tracing::warn!(
            session_id = %session.id,
            serial = %session.machine_serial,
            "超时回收：机器摘要不可用，跳过 VM 回收"
        );
    }

    if let Some(image_id) = session.image_id
        && let Ok(Some(artifact)) = state.storage_repo.get_image(image_id).await
        && artifact.status == ArtifactStatus::Draft
    {
        if let Err(error) = state.storage_repo.delete_image_cascade(image_id).await {
            tracing::warn!(session_id = %session.id, %error, "超时回收：清理 draft 镜像失败");
        }
    }

    if let Some(image_id) = session.image_id {
        let event = NewImageEvent {
            image_id,
            actor_kind: ImageEventActorKind::System,
            actor_user_id: None,
            actor_serial: Some(session.machine_serial.clone()),
            event: "build.session.expired".to_owned(),
            payload: serde_json::json!({
                "session_id": session.id,
                "user_id": session.user_id,
                "room_id": session.room_id,
                "build_vm_name": session.build_vm_name,
            }),
        };
        if let Err(error) = state.storage_repo.insert_image_event(&event).await {
            tracing::warn!(%error, session_id = %session.id, "写入 build.session.expired 事件失败（忽略）");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_window_matches_two_hours() {
        // 契约 §3.2：>2h 非终态会话回收；worker 用小时数换算分钟。
        assert_eq!(SESSION_STALE_HOURS, 2);
        assert_eq!(SESSION_STALE_HOURS * 60, 120);
        assert_eq!(REAPER_TICK, Duration::from_secs(60));
    }
}
