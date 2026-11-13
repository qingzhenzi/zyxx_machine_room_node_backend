//! 教师端镜像制作工作台（Image Studio v1）服务端接口。
//!
//! 契约见 `docs/image-studio.md` §3：
//! - `GET/PUT /admin/rooms/{room_id}/image-default`：房间默认底包（+ 可选增量包），
//!   写入前校验镜像 `published` 且对当前用户可见，并内部保证 assignment + transfer
//!   + `image.pull` 下发；
//! - `POST /admin/image-builds`：创建构建会话（state=preparing），确保底包已分配、
//!   记录 `base_transfer_id`、必要时触发 `image.pull`；
//! - `GET /admin/image-builds/{id}`：**幂等推进状态机**（preparing → creating →
//!   ready → finishing → making → done/failed），返回契约 §3.2.1 形状；
//! - `GET /admin/image-builds?active=1`：当前用户未终态会话（教师端恢复）；
//! - `POST /admin/image-builds/{id}/finish`：仅 ready，置 finishing 并发优雅关机；
//! - `POST /admin/image-builds/{id}/cancel`：destroy + delete（remove_disks）+ 删除
//!   未使用的 draft，置 cancelled。
//!
//! VM 命令统一走 `super::machines` 的内部入队 helper（`machine_command_results`
//! pending + `device_downlink`），`image.make` 复用现有「draft + 制作工单」身份桥，
//! 保证产出镜像 owner=会话创建者。

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use qe_db::image_studio::SessionUpdate;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        image_studio::{
            BuildSession, BuildState, base_volume_name, build_vm_name,
            preparing_outcome, session_steps, shutdown_grace_expired,
        },
        rbac::permissions,
        storage::{
            ArtifactStatus, ImageArtifact, ImageArtifactView, ImageEventActorKind, NewImageEvent,
            Transfer, can_read_image, validate_image_name, validate_image_version,
            validate_platform,
        },
    },
    error::{AppError, AppResult},
    state::AppState,
};

/// 构建 VM 规格默认值（与 `vm.command create` 的默认值一致）。
const BUILD_VCPUS_DEFAULT: u64 = 2;
const BUILD_MEMORY_MIB_DEFAULT: u64 = 2_048;
const BUILD_DISK_GB_DEFAULT: u64 = 20;
/// 构建 VM 规格范围（与 `vm.command create` 一致）。
const BUILD_VCPUS_RANGE: (u64, u64) = (1, 64);
const BUILD_MEMORY_MIB_RANGE: (u64, u64) = (256, 262_144);
const BUILD_DISK_GB_RANGE: (u64, u64) = (1, 2_048);
/// platform 缺省值（与 agent 侧默认一致）。
const BUILD_PLATFORM_DEFAULT: &str = "linux/amd64";

// ---------------------------------------------------------------------------
// 请求 / 响应
// ---------------------------------------------------------------------------

/// `GET /admin/image-builds?active=1` 查询参数。
///
/// 用字符串解析而不是 `bool`：`serde_urlencoded` 只接受 `true/false`，
/// 而契约/前端传的是 `active=1`。
#[derive(Debug, Deserialize)]
pub struct ListBuildSessionsQuery {
    /// 当前仅支持 `active=1`（缺省也是 active）。
    #[serde(default)]
    pub active: Option<String>,
}

/// `active` 查询参数解析：`1/true` = active（缺省）；`0/false` 明确拒绝。
fn parse_active_filter(raw: Option<&str>) -> AppResult<bool> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("1") | Some("true") | Some("TRUE") => Ok(true),
        Some("0") | Some("false") | Some("FALSE") => Ok(false),
        Some(_) => Err(AppError::bad_request("active 只能是 1 / 0 / true / false")),
    }
}

/// `PUT /admin/rooms/{room_id}/image-default` 请求体（契约 §3.1）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateImageDefaultRequest {
    pub base_image_id: Uuid,
    #[serde(default)]
    pub delta_image_id: Option<Uuid>,
}

/// `GET / PUT /admin/rooms/{room_id}/image-default` 响应（契约 §3.1）。
#[derive(Debug, Serialize)]
pub struct ImageDefaultReply {
    pub base_image: Option<ImageArtifactView>,
    pub delta_image: Option<ImageArtifactView>,
}

/// `POST /admin/image-builds` 请求体（契约 §3.2）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBuildSessionRequest {
    pub room_id: Uuid,
    pub machine_serial: String,
    pub base_image_id: Uuid,
    #[serde(default)]
    pub delta_image_id: Option<Uuid>,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub vcpus: Option<u32>,
    #[serde(default)]
    pub memory_mib: Option<u32>,
    #[serde(default)]
    pub disk_gb: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
}

/// 会话响应（契约 §3.2.1，教师端直接消费）。
#[derive(Debug, Serialize)]
pub struct BuildSessionReply {
    pub id: Uuid,
    pub state: BuildState,
    pub error: Option<String>,
    pub room_id: Uuid,
    pub machine_serial: String,
    pub build_vm_name: String,
    pub base_image: Option<ImageArtifactView>,
    pub delta_image: Option<ImageArtifactView>,
    pub name: String,
    pub version: String,
    pub platform: String,
    pub vcpus: i32,
    pub memory_mib: i32,
    pub disk_gb: i32,
    pub description: Option<String>,
    pub steps: Vec<crate::domain::image_studio::BuildStep>,
    pub base_transfer: Option<Transfer>,
    pub image_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl BuildSessionReply {
    fn from_parts(
        session: BuildSession,
        base_image: Option<ImageArtifactView>,
        delta_image: Option<ImageArtifactView>,
        base_transfer: Option<Transfer>,
    ) -> Self {
        let steps = session_steps(&session, base_transfer.as_ref());
        Self {
            id: session.id,
            state: session.state,
            error: session.error,
            room_id: session.room_id,
            machine_serial: session.machine_serial,
            build_vm_name: session.build_vm_name,
            base_image,
            delta_image,
            name: session.name,
            version: session.version,
            platform: session.platform,
            vcpus: session.vcpus,
            memory_mib: session.memory_mib,
            disk_gb: session.disk_gb,
            description: session.description,
            steps,
            base_transfer,
            image_id: session.image_id,
            created_at: session.created_at,
            updated_at: session.updated_at,
            finished_at: session.finished_at,
        }
    }
}

// ---------------------------------------------------------------------------
// 通用内部 helper
// ---------------------------------------------------------------------------

/// 加载镜像视图（不存在返回 `None`）。
async fn load_image_view(state: &AppState, id: Uuid) -> AppResult<Option<ImageArtifactView>> {
    Ok(state.storage_repo.get_image_view(id).await?)
}

/// 校验镜像存在、未删除、对当前用户可见且已发布（含 manifest），返回制品。
async fn require_visible_published_image(
    state: &AppState,
    actor: Uuid,
    image_id: Uuid,
    field: &str,
) -> AppResult<ImageArtifact> {
    let artifact = state
        .storage_repo
        .get_image(image_id)
        .await?
        .ok_or_else(|| AppError::bad_request(format!("{field} 引用的镜像不存在（{image_id}）")))?;
    if artifact.deleted_at.is_some() {
        return Err(AppError::bad_request(format!(
            "{field} 引用的镜像已被删除（{image_id}）"
        )));
    }
    let is_admin = super::images::actor_is_admin(state, actor).await?;
    if !can_read_image(&artifact, actor, is_admin) {
        return Err(AppError::forbidden(format!(
            "{field} 引用的镜像不可见（仅 owner / 公共镜像可用）"
        )));
    }
    if artifact.status != ArtifactStatus::Published {
        return Err(AppError::bad_request(format!(
            "{field} 引用的镜像未发布（status={}，需为 published）",
            artifact.status
        )));
    }
    let manifest_ok = artifact
        .manifest_digest
        .as_deref()
        .map(str::trim)
        .is_some_and(|digest| !digest.is_empty());
    if !manifest_ok {
        return Err(AppError::bad_request(format!(
            "{field} 引用的镜像缺少 manifest_digest，无法拉取/构建"
        )));
    }
    Ok(artifact)
}

/// 确保镜像分配给房间、有可等待 transfer；需要拉取时投递 `image.pull`（尽力而为）。
async fn ensure_assigned_and_pull(
    state: &AppState,
    room_id: Uuid,
    image_id: Uuid,
    actor: Uuid,
    target_slot: Option<&str>,
) -> AppResult<Transfer> {
    let outcome = state
        .image_studio
        .ensure_image_assigned(room_id, image_id, actor)
        .await?
        .ok_or_else(|| AppError::not_found("镜像或房间不存在，无法分配"))?;
    if outcome.transfer.status != crate::domain::storage::TransferStatus::Completed {
        match target_slot {
            // 构建会话的目标可能是 room_devices 子机：image.pull 必须投给目标槽位。
            Some(slot) => {
                super::images::dispatch_image_pull_to_slot(
                    state,
                    slot,
                    image_id,
                    outcome.transfer.id,
                )
                .await;
            }
            // 房间默认底包：投给房间主设备槽位。
            None => {
                super::images::dispatch_image_pull(state, room_id, image_id, outcome.transfer.id)
                    .await;
            }
        }
    }
    Ok(outcome.transfer)
}

/// 会话回复：回查底包/增量包视图与 transfer。
async fn session_reply(state: &AppState, session: BuildSession) -> AppResult<BuildSessionReply> {
    let base_transfer = match session.base_transfer_id {
        Some(transfer_id) => state.storage_repo.get_transfer(transfer_id).await?,
        None => None,
    };
    let base_image = load_image_view(state, session.base_image_id).await?;
    let delta_image = match session.delta_image_id {
        Some(delta_id) => load_image_view(state, delta_id).await?,
        None => None,
    };
    Ok(BuildSessionReply::from_parts(
        session,
        base_image,
        delta_image,
        base_transfer,
    ))
}

/// 读取会话；不存在 404。
async fn load_session(state: &AppState, id: Uuid) -> AppResult<BuildSession> {
    state
        .image_studio
        .get_session(id)
        .await?
        .ok_or_else(|| AppError::not_found("制作会话不存在"))
}

/// 读取会话并校验 owner/admin。
async fn load_owned_session(state: &AppState, actor: Uuid, id: Uuid) -> AppResult<BuildSession> {
    let session = load_session(state, id).await?;
    let is_admin = super::images::actor_is_admin(state, actor).await?;
    if !session.visible_to(actor, is_admin) {
        return Err(AppError::forbidden("无权查看或操作他人的制作会话"));
    }
    Ok(session)
}

/// 重新读取会话（CAS 失败/状态被并发推进后）。
async fn reload_session(state: &AppState, id: Uuid) -> AppResult<BuildSession> {
    load_session(state, id).await
}

/// 一次命令结果查询的归一化结果。
#[derive(Debug, Clone)]
enum CommandOutcome {
    /// 尚未回写（或 pending 行暂时缺失）。
    Pending,
    Ok(Option<Value>),
    Failed(String),
}

/// 查询 `machine_command_results` 的归一化结果。
async fn command_outcome(state: &AppState, request_id: Uuid) -> AppResult<CommandOutcome> {
    let row: Option<(String, Option<Value>, Option<String>)> = sqlx::query_as(
        "SELECT status, result, error
           FROM machine_command_results
          WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_optional(state.db.pool())
    .await
    .map_err(|error| AppError::internal(format!("查询 VM 命令结果失败: {error}")))?;
    let Some((status, result, error)) = row else {
        return Ok(CommandOutcome::Pending);
    };
    match status.as_str() {
        "pending" => Ok(CommandOutcome::Pending),
        "ok" => Ok(CommandOutcome::Ok(result)),
        other => Ok(CommandOutcome::Failed(
            error.unwrap_or_else(|| format!("命令执行失败（{other}）")),
        )),
    }
}

/// 把会话推进为 failed 并返回最新行（CAS 失败时重新读取）。
async fn transition_failed(
    state: &AppState,
    session: &BuildSession,
    error: impl Into<String>,
) -> AppResult<BuildSession> {
    let update = SessionUpdate::failed(&[session.state], error);
    let updated = match state
        .image_studio
        .update_session(session.id, &update)
        .await?
    {
        Some(updated) => updated,
        None => reload_session(state, session.id).await?,
    };
    // 契约 §2：失败时 VM 尽力回收（已下发过 create 才有 VM；draft 保留供人工排查）。
    if session.create_request_id.is_some()
        && let Ok(Some(slot)) = machine_slot(state, &session.machine_serial).await
    {
        for (op, args) in [
            ("destroy", json!({ "name": session.build_vm_name })),
            (
                "delete",
                json!({ "name": session.build_vm_name, "remove_disks": true }),
            ),
        ] {
            if let Err(error) =
                super::machines::enqueue_internal_vm_command(state, &slot, op, args).await
            {
                tracing::warn!(
                    session_id = %session.id,
                    %op,
                    %error,
                    "失败会话：VM 回收命令下发失败（尽力而为）"
                );
            }
        }
    }
    Ok(updated)
}

/// 设备槽位：会话目标机器的 `device_slot`（机器摘要不存在返回 `None`）。
async fn machine_slot(state: &AppState, serial: &str) -> AppResult<Option<String>> {
    Ok(state
        .machines
        .summary(serial)
        .await?
        .map(|machine| machine.device_slot))
}

/// 向会话目标设备下发一条 vm.command，返回 request_id。
async fn enqueue_session_vm_command(
    state: &AppState,
    serial: &str,
    op: &str,
    args: Value,
) -> AppResult<Uuid> {
    let slot = machine_slot(state, serial)
        .await?
        .ok_or_else(|| AppError::conflict("构建目标机器不存在或未注册，无法下发命令"))?;
    super::machines::enqueue_internal_vm_command(state, &slot, op, args).await
}

/// 记录会话审计事件（失败只记日志）。
async fn record_session_audit(
    state: &AppState,
    actor: Uuid,
    session: &BuildSession,
    action: &str,
    detail: Value,
    ip: Option<&str>,
) {
    super::audit::record_or_log(
        state,
        actor,
        action,
        "image_build_session",
        Some(session.id.to_string()),
        detail,
        ip,
    )
    .await;
}

/// 写一条镜像事件（用于默认底包变更；失败只 warn）。
async fn record_image_event(
    state: &AppState,
    actor: Uuid,
    image_id: Uuid,
    event: &str,
    payload: Value,
) {
    if let Err(error) = state
        .storage_repo
        .insert_image_event(&NewImageEvent {
            image_id,
            actor_kind: ImageEventActorKind::User,
            actor_user_id: Some(actor),
            actor_serial: None,
            event: event.to_owned(),
            payload,
        })
        .await
    {
        tracing::warn!(%error, %image_id, event, "写入镜像事件失败（忽略）");
    }
}

// ---------------------------------------------------------------------------
// 状态机推进（GET 时幂等执行）
// ---------------------------------------------------------------------------

/// 依据当前状态幂等推进会话，返回推进后的最新会话。
pub(crate) async fn advance_session(
    state: &AppState,
    session: BuildSession,
) -> AppResult<BuildSession> {
    match session.state {
        BuildState::Preparing => advance_preparing(state, session).await,
        BuildState::Creating => advance_creating(state, session).await,
        BuildState::Finishing => advance_finishing(state, session).await,
        BuildState::Making => advance_making(state, session).await,
        _ => Ok(session),
    }
}

/// preparing：base transfer completed → 发 `vm.command create`（差分盘 backing=底包）
/// 并进入 creating；failed/expired → failed；pending/running → 等待。
async fn advance_preparing(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    let Some(transfer_id) = session.base_transfer_id else {
        return transition_failed(state, &session, "缺少基础镜像传输记录，请取消后重试").await;
    };
    let Some(transfer) = state.storage_repo.get_transfer(transfer_id).await? else {
        return transition_failed(state, &session, "基础镜像传输记录不存在，请取消后重试").await;
    };
    match preparing_outcome(transfer.status, transfer.error.as_deref()) {
        None => Ok(session),
        Some(Err(error)) => transition_failed(state, &session, error).await,
        Some(Ok(())) => {
            let base = match state.storage_repo.get_image(session.base_image_id).await? {
                Some(base) => base,
                None => {
                    return transition_failed(state, &session, "基础镜像不存在（可能已被删除）")
                        .await;
                }
            };
            let args = json!({
                "name": session.build_vm_name,
                "vcpus": session.vcpus,
                "memory_mib": session.memory_mib,
                "disk_gb": session.disk_gb,
                "base_volume": base_volume_name(&base.name, &base.version),
                "start": true,
            });
            let request_id =
                match enqueue_session_vm_command(state, &session.machine_serial, "create", args)
                    .await
                {
                    Ok(request_id) => request_id,
                    Err(error) => {
                        return transition_failed(state, &session, error.to_string()).await;
                    }
                };
            let update = SessionUpdate::new(&[BuildState::Preparing], BuildState::Creating)
                .with_create_request(request_id);
            match state
                .image_studio
                .update_session(session.id, &update)
                .await?
            {
                Some(updated) => Ok(updated),
                None => reload_session(state, session.id).await,
            }
        }
    }
}

/// creating：创建 VM 命令 ok → ready；error/timeout → failed。
async fn advance_creating(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    let Some(request_id) = session.create_request_id else {
        return Ok(session);
    };
    match command_outcome(state, request_id).await? {
        CommandOutcome::Pending => Ok(session),
        CommandOutcome::Ok(_) => {
            let update = SessionUpdate::new(&[BuildState::Creating], BuildState::Ready);
            match state
                .image_studio
                .update_session(session.id, &update)
                .await?
            {
                Some(updated) => Ok(updated),
                None => reload_session(state, session.id).await,
            }
        }
        CommandOutcome::Failed(error) => transition_failed(state, &session, error).await,
    }
}

/// 构建完成/失败后尽力删除构建 VM（undefine + 删差分盘；底包卷保留）。
/// 失败只 warn：会话状态已经终态，回收由后台 worker 兜底。
async fn cleanup_build_vm(state: &AppState, session: &BuildSession) {
    let args = json!({ "name": session.build_vm_name, "remove_disks": true });
    match enqueue_session_vm_command(state, &session.machine_serial, "delete", args).await {
        Ok(request_id) => tracing::info!(
            session_id = %session.id,
            vm = %session.build_vm_name,
            %request_id,
            "构建完成，已下发删除构建 VM"
        ),
        Err(error) => tracing::warn!(
            session_id = %session.id,
            vm = %session.build_vm_name,
            %error,
            "构建完成，删除构建 VM 下发失败（可手动清理）"
        ),
    }
}

/// finishing：关机/强制 destroy 完成 → 发 `image.make` 进入 making；
/// 优雅关机失败或超过 60s 宽限期 → 自动 destroy；destroy 失败 → failed。
async fn advance_finishing(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    let shutdown_outcome = match session.shutdown_request_id {
        Some(request_id) => Some(command_outcome(state, request_id).await?),
        None => None,
    };
    let destroy_outcome = match session.destroy_request_id {
        Some(request_id) => Some(command_outcome(state, request_id).await?),
        None => None,
    };

    // 注意：`shutdown` 命令返回 ok 只表示 ACPI 关机请求已下发，**不代表 VM 已关闭**
    // （无 acpid/guest 不响应时 VM 仍在运行，直接 image.make 会被 agent 拒绝）。
    // 因此只有 `destroy` 命令有终态后才认为 VM 已不在运行：
    // - destroy ok：强制断电成功；
    // - destroy 失败：多半是“域已不在运行”（优雅关机已生效），交给 image.make 做最终校验。
    let destroy_terminal = matches!(
        destroy_outcome,
        Some(CommandOutcome::Ok(_)) | Some(CommandOutcome::Failed(_))
    );
    if destroy_terminal {
        return enqueue_image_make(state, session).await;
    }
    if let Some(CommandOutcome::Failed(error)) = &shutdown_outcome {
        tracing::warn!(
            session_id = %session.id,
            %error,
            "finishing: 优雅关机失败，改为强制关机"
        );
    }

    // 优雅关机失败（或没有发出 shutdown）→ 立即 destroy；仍在 pending 时等宽限期。
    let should_destroy = match &shutdown_outcome {
        Some(CommandOutcome::Failed(_)) => session.destroy_request_id.is_none(),
        None => session.destroy_request_id.is_none(),
        Some(CommandOutcome::Ok(_)) | Some(CommandOutcome::Pending) => {
            session.destroy_request_id.is_none()
                && shutdown_grace_expired(session.updated_at, Utc::now())
        }
    };
    if should_destroy {
        let args = json!({ "name": session.build_vm_name });
        let request_id =
            match enqueue_session_vm_command(state, &session.machine_serial, "destroy", args).await
            {
                Ok(request_id) => request_id,
                Err(error) => {
                    // destroy 无法下发（设备离线等）：等宽限/超时回收，避免会话瞬间失败。
                    tracing::warn!(
                        session_id = %session.id,
                        %error,
                        "finishing: destroy 下发失败，等待后台回收"
                    );
                    return Ok(session);
                }
            };
        let update = SessionUpdate::new(&[BuildState::Finishing], BuildState::Finishing)
            .with_destroy_request(request_id);
        match state
            .image_studio
            .update_session(session.id, &update)
            .await?
        {
            Some(updated) => Ok(updated),
            None => reload_session(state, session.id).await,
        }
    } else {
        Ok(session)
    }
}

/// 发送 `image.make`（复用 draft + 制作工单身份桥），成功进入 making 并记录 image_id。
async fn enqueue_image_make(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    let mut args = json!({
        "vm_name": session.build_vm_name,
        "name": session.name,
        "version": session.version,
        "platform": session.platform,
    });
    if let Some(description) = session.description.as_deref()
        && !description.trim().is_empty()
    {
        args["description"] = json!(description);
    }
    let slot = match machine_slot(state, &session.machine_serial).await? {
        Some(slot) => slot,
        None => {
            return transition_failed(state, &session, "构建目标机器不存在或未注册").await;
        }
    };
    let prepared =
        match super::machines::enqueue_image_make_for_session(state, session.user_id, &slot, args)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => return transition_failed(state, &session, error.to_string()).await,
        };
    let update = SessionUpdate::new(&[BuildState::Finishing], BuildState::Making)
        .with_make_request(prepared.request_id)
        .with_image_id(prepared.image_id);
    match state
        .image_studio
        .update_session(session.id, &update)
        .await?
    {
        Some(updated) => Ok(updated),
        None => {
            // 并发 GET 已推进：丢弃本次多创建的 draft/工单，避免留下孤儿。
            if let Err(error) = state
                .storage_repo
                .delete_image_cascade(prepared.image_id)
                .await
            {
                tracing::warn!(%error, image_id = %prepared.image_id, "清理并发 image.make draft 失败");
            }
            reload_session(state, session.id).await
        }
    }
}

/// making：image.make ok → done（image_id 已在 making 时记录）；error/timeout → failed。
async fn advance_making(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    let Some(request_id) = session.make_request_id else {
        return Ok(session);
    };
    match command_outcome(state, request_id).await? {
        CommandOutcome::Pending => Ok(session),
        CommandOutcome::Ok(data) => {
            let image_id = session.image_id.or_else(|| {
                data.as_ref()
                    .and_then(|data| data.get("image_id"))
                    .and_then(|value| value.as_str())
                    .and_then(|raw| raw.parse::<Uuid>().ok())
            });
            let mut update = SessionUpdate::new(&[BuildState::Making], BuildState::Done);
            if let Some(image_id) = image_id {
                update = update.with_image_id(image_id);
            }
            match state
                .image_studio
                .update_session(session.id, &update)
                .await?
            {
                Some(updated) => {
                    // 镜像已上传完成，构建 VM 不再需要：尽力回收（不阻塞完成态）。
                    cleanup_build_vm(state, &updated).await;
                    Ok(updated)
                }
                None => reload_session(state, session.id).await,
            }
        }
        CommandOutcome::Failed(error) => transition_failed(state, &session, error).await,
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers：房间默认底包
// ---------------------------------------------------------------------------

/// `GET /api/v1/admin/rooms/{room_id}/image-default` —— 读取房间默认底包。
pub async fn get_image_default(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(room_id): Path<Uuid>,
) -> AppResult<Json<ImageDefaultReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    if state.rooms.get(room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }
    let default = state.image_studio.get_room_default(room_id).await?;
    let (base_image, delta_image) = match default {
        Some(default) => (
            load_image_view(&state, default.base_image_id).await?,
            match default.delta_image_id {
                Some(delta_id) => load_image_view(&state, delta_id).await?,
                None => None,
            },
        ),
        None => (None, None),
    };
    Ok(Json(ImageDefaultReply {
        base_image,
        delta_image,
    }))
}

/// `PUT /api/v1/admin/rooms/{room_id}/image-default` —— 设置房间默认底包。
///
/// 校验镜像 published 且对当前用户可见；写 `image_assignments` + transfer，
/// 必要时投递 `image.pull`（agent 也会在启动配置里拉取，这里是即时补偿）。
pub async fn update_image_default(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateImageDefaultRequest>,
) -> AppResult<Json<ImageDefaultReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    if state.rooms.get(room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }
    if request.base_image_id == Uuid::nil() {
        return Err(AppError::bad_request("base_image_id 不能为空"));
    }
    let base =
        require_visible_published_image(&state, id, request.base_image_id, "base_image_id").await?;
    let delta = match request.delta_image_id {
        Some(delta_id) => {
            if delta_id == base.id {
                return Err(AppError::bad_request(
                    "delta_image_id 不能与 base_image_id 相同",
                ));
            }
            Some(require_visible_published_image(&state, id, delta_id, "delta_image_id").await?)
        }
        None => None,
    };

    // 底包 + 增量包都要能在目标房间拉取：保证 assignment/transfer 并尽力触发 pull。
    ensure_assigned_and_pull(&state, room_id, base.id, id, None).await?;
    if let Some(delta) = &delta {
        ensure_assigned_and_pull(&state, room_id, delta.id, id, None).await?;
    }

    let default = state
        .image_studio
        .upsert_room_default(room_id, base.id, delta.as_ref().map(|image| image.id), id)
        .await?;

    record_image_event(
        &state,
        id,
        base.id,
        "room.default.update",
        json!({
            "room_id": room_id,
            "base_image_id": base.id,
            "delta_image_id": delta.as_ref().map(|image| image.id),
        }),
    )
    .await;
    super::audit::record_or_log(
        &state,
        id,
        "room.image_default.update",
        "room",
        Some(room_id.to_string()),
        json!({
            "base_image_id": base.id,
            "delta_image_id": delta.as_ref().map(|image| image.id),
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(ImageDefaultReply {
        base_image: load_image_view(&state, default.base_image_id).await?,
        delta_image: match default.delta_image_id {
            Some(delta_id) => load_image_view(&state, delta_id).await?,
            None => None,
        },
    }))
}

// ---------------------------------------------------------------------------
// HTTP handlers：构建会话
// ---------------------------------------------------------------------------

/// `POST /api/v1/admin/image-builds` —— 创建镜像构建会话。
///
/// 校验顺序：权限 → 房间/机器归属+在线+存活证书 → 底包可见/已发布 → 同名预检 409
/// → 确保分配与 transfer → 写会话（preparing）→ 必要时 image.pull → 返回会话。
pub async fn create_build_session(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<CreateBuildSessionRequest>,
) -> AppResult<(StatusCode, Json<BuildSessionReply>)> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    authorization::require_permission(&state, id, permissions::IMAGE_MANAGE).await?;

    // ---- 目标房间/机器 ----
    if state.rooms.get(request.room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }
    let serial = request.machine_serial.trim();
    if serial.is_empty() {
        return Err(AppError::bad_request("machine_serial 不能为空"));
    }
    let machine = state
        .machines
        .summary(serial)
        .await?
        .ok_or_else(|| AppError::not_found("构建目标机器不存在"))?;
    if machine.room_id != Some(request.room_id) {
        return Err(AppError::bad_request(
            "构建目标机器不属于该房间（room.device_slot / room_devices）",
        ));
    }
    if !matches!(machine.status.as_str(), "issued" | "active") {
        return Err(AppError::conflict(
            "构建目标机器没有存活证书（设备需先在控制台注册）",
        ));
    }
    if machine.online.as_deref() != Some("online") {
        return Err(AppError::conflict(
            "构建目标机器当前离线，请等待 agent 上线后重试",
        ));
    }

    // ---- 基础镜像 ----
    let base =
        require_visible_published_image(&state, id, request.base_image_id, "base_image_id").await?;
    if base.kind != crate::domain::storage::ImageKind::VmDisk {
        return Err(AppError::bad_request(
            "base_image_id 必须是 vm_disk 类型的系统底包（agent 以 <name>-<version> 卷名创建差分盘）",
        ));
    }
    let delta = match request.delta_image_id {
        Some(delta_id) => {
            Some(require_visible_published_image(&state, id, delta_id, "delta_image_id").await?)
        }
        None => None,
    };

    // ---- 镜像元数据校验（与 image.make 的下发校验一致） ----
    let name = request.name.trim().to_owned();
    let name =
        validate_image_name(&name).map_err(|error| AppError::bad_request(error.to_string()))?;
    let version = request.version.trim().to_owned();
    let version = validate_image_version(&version)
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    let platform = match request.platform.as_deref().map(str::trim) {
        Some(platform) if !platform.is_empty() => {
            validate_platform(platform).map_err(|error| AppError::bad_request(error.to_string()))?
        }
        _ => BUILD_PLATFORM_DEFAULT.to_owned(),
    };
    let vcpus = super::machines::check_range(
        u64::from(request.vcpus.unwrap_or(BUILD_VCPUS_DEFAULT as u32)),
        "vcpus",
        BUILD_VCPUS_RANGE,
    )?;
    let memory_mib = super::machines::check_range(
        u64::from(
            request
                .memory_mib
                .unwrap_or(BUILD_MEMORY_MIB_DEFAULT as u32),
        ),
        "memory_mib",
        BUILD_MEMORY_MIB_RANGE,
    )?;
    let disk_gb = super::machines::check_range(
        u64::from(request.disk_gb.unwrap_or(BUILD_DISK_GB_DEFAULT as u32)),
        "disk_gb",
        BUILD_DISK_GB_RANGE,
    )?;
    let description = request
        .description
        .map(|description| description.trim().to_owned())
        .filter(|description| !description.is_empty());
    if let Some(description) = description.as_deref()
        && description.chars().count() > 255
    {
        return Err(AppError::bad_request("description 不能超过 255 个字符"));
    }

    // ---- 同名预检（契约 §4.1 / §3.2：在创建 VM 之前 409） ----
    let namespace = state.storage_repo.ensure_personal_namespace(id).await?;
    if let Some(existing) = state
        .storage_repo
        .find_name_conflict(namespace.id, &name, &version, &platform)
        .await?
        && existing.status.blocks_image_name()
    {
        return Err(AppError::conflict(format!(
            "image_name_taken：@{} 下 {}:{}（{}）已存在，请更换版本",
            namespace.slug, name, version, platform
        )));
    }

    // 同一教师已有一个进行中的同名会话 → 409（避免同一镜像名重复建 VM/重复上传）。
    if let Some(active) = state
        .image_studio
        .find_active_session_by_name(id, &name, &version, &platform)
        .await?
    {
        return Err(AppError::conflict(format!(
            "该名称版本已有一个进行中的制作会话（{}，state={}），请先完成或取消",
            active.id, active.state
        )));
    }

    // ---- 确保底包分配 + transfer + pull ----
    let target_slot = machine.device_slot.clone();
    let transfer = ensure_assigned_and_pull(
        &state,
        request.room_id,
        base.id,
        id,
        Some(target_slot.as_str()),
    )
    .await?;
    if let Some(delta) = &delta {
        ensure_assigned_and_pull(
            &state,
            request.room_id,
            delta.id,
            id,
            Some(target_slot.as_str()),
        )
        .await?;
    }

    // ---- 写会话 ----
    let session_id = Uuid::new_v4();
    let new_session = crate::domain::image_studio::NewBuildSession {
        id: session_id,
        user_id: id,
        room_id: request.room_id,
        machine_serial: serial.to_owned(),
        build_vm_name: build_vm_name(session_id),
        base_image_id: base.id,
        delta_image_id: delta.as_ref().map(|image| image.id),
        base_transfer_id: Some(transfer.id),
        name,
        version,
        platform,
        vcpus: vcpus as i32,
        memory_mib: memory_mib as i32,
        disk_gb: disk_gb as i32,
        description,
    };
    let session = state.image_studio.create_session(&new_session).await?;

    record_session_audit(
        &state,
        id,
        &session,
        "image.build.session.create",
        json!({
            "room_id": session.room_id,
            "machine_serial": session.machine_serial,
            "base_image_id": session.base_image_id,
            "base_transfer_id": session.base_transfer_id,
            "build_vm_name": session.build_vm_name,
            "name": session.name,
            "version": session.version,
            "platform": session.platform,
        }),
        ip.as_deref(),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(session_reply(&state, session).await?),
    ))
}

/// `GET /api/v1/admin/image-builds/{id}` —— 读取（并幂等推进）构建会话。
pub async fn get_build_session(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(session_id): Path<Uuid>,
) -> AppResult<Json<BuildSessionReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    let session = load_owned_session(&state, id, session_id).await?;
    let previous_state = session.state;
    let session = advance_session(&state, session).await?;
    // 只在本次请求真正推进到 failed/done 时记一条审计（避免前端轮询刷屏）。
    if session.state != previous_state {
        match session.state {
            BuildState::Failed => {
                record_session_audit(
                    &state,
                    id,
                    &session,
                    "image.build.session.failed",
                    json!({ "error": session.error, "state": session.state.as_str() }),
                    None,
                )
                .await;
            }
            BuildState::Done => {
                record_session_audit(
                    &state,
                    id,
                    &session,
                    "image.build.session.done",
                    json!({ "image_id": session.image_id, "state": session.state.as_str() }),
                    None,
                )
                .await;
            }
            _ => {}
        }
    }
    Ok(Json(session_reply(&state, session).await?))
}

/// `GET /api/v1/admin/image-builds?active=1` —— 当前用户未终态会话。
pub async fn list_build_sessions(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<ListBuildSessionsQuery>,
) -> AppResult<Json<Vec<BuildSessionReply>>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    if !parse_active_filter(query.active.as_deref())? {
        return Err(AppError::bad_request(
            "当前仅支持 active=1（未结束的构建会话）",
        ));
    }
    let sessions = state.image_studio.list_active_sessions(id).await?;
    let mut replies = Vec::with_capacity(sessions.len());
    for session in sessions {
        // 恢复列表也顺手推进（教师端可能只轮询列表页）。
        let session = advance_session(&state, session).await?;
        replies.push(session_reply(&state, session).await?);
    }
    Ok(Json(replies))
}

/// `POST /api/v1/admin/image-builds/{id}/finish` —— 结束制作（仅 ready）。
pub async fn finish_build_session(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(session_id): Path<Uuid>,
) -> AppResult<Json<BuildSessionReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    let session = load_owned_session(&state, id, session_id).await?;
    if !session.state.can_finish() {
        return Err(AppError::conflict(format!(
            "仅 ready 状态可结束制作（当前 {}）",
            session.state
        )));
    }
    let request_id = enqueue_session_vm_command(
        &state,
        &session.machine_serial,
        "shutdown",
        json!({ "name": session.build_vm_name }),
    )
    .await?;
    let update = SessionUpdate::new(&[BuildState::Ready], BuildState::Finishing)
        .with_shutdown_request(request_id);
    let session = match state
        .image_studio
        .update_session(session.id, &update)
        .await?
    {
        Some(updated) => updated,
        None => {
            return Err(AppError::conflict("会话状态已被其它请求改变，请刷新后重试"));
        }
    };
    record_session_audit(
        &state,
        id,
        &session,
        "image.build.session.finish",
        json!({ "shutdown_request_id": request_id }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(session_reply(&state, session).await?))
}

/// `POST /api/v1/admin/image-builds/{id}/cancel` —— 取消制作并回收构建 VM。
///
/// destroy + delete(remove_disks=true) 都尽力而为；删除尚未消费的 draft 镜像，
/// 最后把会话置 `cancelled`（终态幂等：已 cancelled/expired 直接返回当前状态）。
pub async fn cancel_build_session(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(session_id): Path<Uuid>,
) -> AppResult<Json<BuildSessionReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    let session = load_owned_session(&state, id, session_id).await?;
    if matches!(session.state, BuildState::Cancelled | BuildState::Expired) {
        return Ok(Json(session_reply(&state, session).await?));
    }
    if !session.state.can_cancel() {
        return Err(AppError::conflict(format!(
            "会话已结束（{}），无法取消",
            session.state
        )));
    }
    if let Some(slot) = machine_slot(&state, &session.machine_serial).await? {
        let destroy = super::machines::enqueue_internal_vm_command(
            &state,
            &slot,
            "destroy",
            json!({ "name": session.build_vm_name }),
        )
        .await;
        if let Err(error) = destroy {
            tracing::warn!(session_id = %session.id, %error, "取消会话：destroy 下发失败（继续 delete）");
        }
        let delete = super::machines::enqueue_internal_vm_command(
            &state,
            &slot,
            "delete",
            json!({ "name": session.build_vm_name, "remove_disks": true }),
        )
        .await;
        if let Err(error) = delete {
            tracing::warn!(session_id = %session.id, %error, "取消会话：delete 下发失败");
        }
    }

    // 删除尚未消费的 draft 镜像（image.make 已启动但还没 register 的情况）。
    if let Some(image_id) = session.image_id
        && let Ok(Some(artifact)) = state.storage_repo.get_image(image_id).await
        && artifact.status == ArtifactStatus::Draft
        && let Err(error) = state.storage_repo.delete_image_cascade(image_id).await
    {
        tracing::warn!(session_id = %session.id, %error, "取消会话：清理 draft 镜像失败");
    }

    let update = SessionUpdate::new(&[session.state], BuildState::Cancelled);
    let session = match state
        .image_studio
        .update_session(session.id, &update)
        .await?
    {
        Some(updated) => updated,
        None => reload_session(&state, session.id).await?,
    };
    record_session_audit(
        &state,
        id,
        &session,
        "image.build.session.cancel",
        json!({ "state": session.state.as_str() }),
        ip.as_deref(),
    )
    .await;
    Ok(Json(session_reply(&state, session).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_rejects_unknown_fields_and_requires_core_fields() {
        let ok: CreateBuildSessionRequest = serde_json::from_value(json!({
            "room_id": Uuid::nil(),
            "machine_serial": "0638",
            "base_image_id": Uuid::nil(),
            "name": "lab-base",
            "version": "1.0.0",
        }))
        .expect("request");
        assert_eq!(ok.platform, None);
        assert_eq!(ok.vcpus, None);

        assert!(
            serde_json::from_value::<CreateBuildSessionRequest>(json!({
                "room_id": Uuid::nil(),
                "machine_serial": "0638",
                "base_image_id": Uuid::nil(),
                "name": "lab-base",
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CreateBuildSessionRequest>(json!({
                "room_id": Uuid::nil(),
                "machine_serial": "0638",
                "base_image_id": Uuid::nil(),
                "name": "lab-base",
                "version": "1.0.0",
                "extra": 1
            }))
            .is_err()
        );
    }

    #[test]
    fn active_query_accepts_contract_form() {
        // 前端传 `?active=1`：不能用 bool（serde_urlencoded 只认 true/false）。
        assert!(parse_active_filter(None).unwrap());
        assert!(parse_active_filter(Some("1")).unwrap());
        assert!(parse_active_filter(Some("true")).unwrap());
        assert!(!parse_active_filter(Some("0")).unwrap());
        assert!(!parse_active_filter(Some("false")).unwrap());
        assert!(parse_active_filter(Some("yes")).is_err());
    }

    #[test]
    fn build_spec_defaults_match_vm_create_defaults() {
        assert_eq!(BUILD_VCPUS_DEFAULT, 2);
        assert_eq!(BUILD_MEMORY_MIB_DEFAULT, 2_048);
        assert_eq!(BUILD_DISK_GB_DEFAULT, 20);
        assert_eq!(BUILD_PLATFORM_DEFAULT, "linux/amd64");
        assert!(BUILD_VCPUS_RANGE.0 <= BUILD_VCPUS_DEFAULT);
        assert!(BUILD_VCPUS_DEFAULT <= BUILD_VCPUS_RANGE.1);
        assert!(BUILD_MEMORY_MIB_RANGE.0 <= BUILD_MEMORY_MIB_DEFAULT);
        assert!(BUILD_MEMORY_MIB_DEFAULT <= BUILD_MEMORY_MIB_RANGE.1);
        assert!(BUILD_DISK_GB_RANGE.0 <= BUILD_DISK_GB_DEFAULT);
        assert!(BUILD_DISK_GB_DEFAULT <= BUILD_DISK_GB_RANGE.1);
    }

    #[test]
    fn image_default_request_requires_base_and_accepts_delta() {
        let request: UpdateImageDefaultRequest = serde_json::from_value(json!({
            "base_image_id": Uuid::nil(),
        }))
        .expect("request");
        assert!(request.delta_image_id.is_none());
        assert!(
            serde_json::from_value::<UpdateImageDefaultRequest>(json!({
                "base_image_id": Uuid::nil(),
                "delta_image_id": Uuid::nil(),
                "extra": 1
            }))
            .is_err()
        );
    }

    #[test]
    fn session_reply_matches_contract_shape() {
        let now = Utc::now();
        let session = BuildSession {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            machine_serial: "dev-b104".to_owned(),
            build_vm_name: "qe-build-a1b2c3d4".to_owned(),
            base_image_id: Uuid::new_v4(),
            delta_image_id: None,
            base_transfer_id: None,
            name: "lab-base".to_owned(),
            version: "1.0.0".to_owned(),
            platform: "linux/amd64".to_owned(),
            vcpus: 2,
            memory_mib: 2_048,
            disk_gb: 40,
            description: Some("build".to_owned()),
            state: BuildState::Preparing,
            error: None,
            create_request_id: None,
            shutdown_request_id: None,
            destroy_request_id: None,
            make_request_id: None,
            image_id: None,
            created_at: now,
            updated_at: now,
            finished_at: None,
        };
        let reply = BuildSessionReply::from_parts(session, None, None, None);
        let value = serde_json::to_value(&reply).expect("serialize");
        for key in [
            "id",
            "state",
            "error",
            "room_id",
            "machine_serial",
            "build_vm_name",
            "base_image",
            "name",
            "version",
            "platform",
            "vcpus",
            "memory_mib",
            "disk_gb",
            "description",
            "steps",
            "base_transfer",
            "image_id",
            "created_at",
            "updated_at",
            "finished_at",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["state"], json!("preparing"));
        assert!(value["image_id"].is_null());
        assert!(value["base_transfer"].is_null());
        assert_eq!(value["steps"].as_array().unwrap().len(), 5);
        assert_eq!(value["steps"][0]["key"], json!("base_pull"));
        assert_eq!(value["steps"][0]["status"], json!("pending"));
        for step in value["steps"].as_array().unwrap() {
            assert!(matches!(
                step["status"].as_str(),
                Some("ok" | "running" | "pending" | "error")
            ));
        }
    }

    #[test]
    fn build_state_serializes_snake_case_in_reply() {
        for (state, expected) in [
            (BuildState::Preparing, "preparing"),
            (BuildState::Creating, "creating"),
            (BuildState::Ready, "ready"),
            (BuildState::Finishing, "finishing"),
            (BuildState::Making, "making"),
            (BuildState::Done, "done"),
            (BuildState::Failed, "failed"),
            (BuildState::Cancelled, "cancelled"),
            (BuildState::Expired, "expired"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(expected));
        }
    }
}
