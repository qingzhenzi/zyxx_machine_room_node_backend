//! 教师端镜像制作工作台（Image Studio v2：含 ISO 安装 / Windows）服务端接口。
//!
//! 契约见 `docs/image-studio.md` §3 与附录 A（ISO 安装/Windows）：
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
            BuildSession, BuildState, DiskBus, NicMode, SourceMode, base_volume_name,
            build_vm_name, iso_check_outcome, preparing_outcome, session_steps,
            shutdown_grace_expired, validate_iso_name,
        },
        install_iso::{ISO_PRESIGN_TTL_SECS, is_iso_usable},
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

/// 来源模式：缺省 `base`；只接受 `base` / `iso`。
fn parse_source_mode(raw: Option<&str>) -> AppResult<SourceMode> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(SourceMode::Base),
        Some(value) => value
            .parse::<SourceMode>()
            .map_err(|_| AppError::bad_request("source 只能是 base / iso")),
    }
}

/// 磁盘总线：未传时按来源模式取默认值（base→virtio、iso→sata）。
fn parse_disk_bus(raw: Option<&str>, source_mode: SourceMode) -> AppResult<DiskBus> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(source_mode.default_disk_bus()),
        Some(value) => value
            .parse::<DiskBus>()
            .map_err(|_| AppError::bad_request("disk_bus 只能是 virtio / sata")),
    }
}

/// 网卡模式：未传时 `none`（离线安装，Windows 避免强制微软账号）。
fn parse_nic_mode(raw: Option<&str>) -> AppResult<NicMode> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(NicMode::None),
        Some(value) => value
            .parse::<NicMode>()
            .map_err(|_| AppError::bad_request("nic 只能是 none / user / default")),
    }
}

/// ISO 来源二选一校验（契约 B.3）：`iso_id`（平台库）与 `iso_name`（目标机已有）
/// 互斥；iso 模式必须给其一，base 模式都不允许。
fn validate_iso_source_selection(
    source_mode: SourceMode,
    iso_id: Option<Uuid>,
    iso_name: Option<&str>,
) -> AppResult<()> {
    let has_name = iso_name
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    match source_mode {
        SourceMode::Base => {
            if iso_id.is_some() || has_name {
                return Err(AppError::bad_request(
                    "source=base 不支持 iso_id / iso_name（仅 source=iso 使用）",
                ));
            }
            Ok(())
        }
        SourceMode::Iso => {
            if iso_id.is_some() && has_name {
                return Err(AppError::bad_request(
                    "iso_id 与 iso_name 只能二选一（推荐使用平台 ISO 库 iso_id）",
                ));
            }
            if iso_id.is_none() && !has_name {
                return Err(AppError::bad_request(
                    "source=iso 需要 iso_id（平台 ISO 库）或 iso_name（目标机已有 ISO）",
                ));
            }
            Ok(())
        }
    }
}

/// 创建 VM 成功后进入的状态：iso 进入 `installing`（等教师安装），base 进入 `ready`。
pub(crate) fn state_after_create(source_mode: SourceMode) -> BuildState {
    match source_mode {
        SourceMode::Base => BuildState::Ready,
        SourceMode::Iso => BuildState::Installing,
    }
}

/// 是否允许 `POST /eject-iso`：仅 ISO 模式且处于 `installing`。
pub(crate) fn can_eject_iso(source_mode: SourceMode, state: BuildState) -> bool {
    source_mode == SourceMode::Iso && state == BuildState::Installing
}

/// `finish` 的前置条件（返回 409 文案的 Err 表示不允许）。
///
/// 契约 A.2：仅 `ready`；iso 模式必须先完成 eject（`media_ejected=true`）。
pub(crate) fn finish_gate(session: &BuildSession) -> Result<(), String> {
    if !session.state.can_finish() {
        return Err(format!("仅 ready 状态可结束制作（当前 {}）", session.state));
    }
    if session.source_mode == SourceMode::Iso && !session.media_ejected {
        return Err("请先完成『安装完成，弹出 ISO』（退出安装介质）再结束制作".to_owned());
    }
    Ok(())
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
    /// `base`（默认，底包差分盘）/ `iso`（用机器上的安装 ISO 建安装 VM）。
    #[serde(default)]
    pub source: Option<String>,
    /// source=base 必填；source=iso 时忽略。
    #[serde(default)]
    pub base_image_id: Option<Uuid>,
    /// source=iso 且使用平台 ISO 资产库时必填（与 iso_name 二选一）。
    #[serde(default)]
    pub iso_id: Option<Uuid>,
    /// source=iso 且使用目标机已有 ISO 时必填（不允许路径分隔符）。
    #[serde(default)]
    pub iso_name: Option<String>,
    /// iso 模式磁盘总线；缺省 base→virtio、iso→sata。
    #[serde(default)]
    pub disk_bus: Option<String>,
    /// 网卡模式 none/user/default；缺省 none（离线安装）。
    #[serde(default)]
    pub nic: Option<String>,
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
    /// `base` / `iso`（契约附录 A.3.2）。
    pub source_mode: SourceMode,
    pub iso_name: Option<String>,
    /// iso 资产模式（v3）引用的 ISO 库记录 id。
    pub iso_id: Option<Uuid>,
    pub disk_bus: DiskBus,
    pub nic: NicMode,
    /// iso 模式：安装完成后是否已弹出 ISO（finish 前必须为 true）。
    pub media_ejected: bool,
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
            source_mode: session.source_mode,
            iso_name: session.iso_name,
            iso_id: session.iso_id,
            disk_bus: session.disk_bus,
            nic: session.nic,
            media_ejected: session.media_ejected,
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
    let base_image = match session.base_image_id {
        Some(base_image_id) => load_image_view(state, base_image_id).await?,
        None => None,
    };
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

/// base 模式 `vm.command create` 的 args（纯函数，便于单测）。
///
/// `disk_bus` 总是显式下发（sata 用于 Windows 底包复用，契约 A.6.3）；
/// `nic` 仅在显式指定时下发，保持 v1 行为（agent 缺省不挂网卡）。
pub(crate) fn base_create_vm_args(session: &BuildSession, base_volume: &str) -> Value {
    let mut args = json!({
        "name": session.build_vm_name,
        "vcpus": session.vcpus,
        "memory_mib": session.memory_mib,
        "disk_gb": session.disk_gb,
        "base_volume": base_volume,
        "disk_bus": session.disk_bus.as_str(),
        "start": true,
    });
    if session.nic != NicMode::None {
        args["nic"] = json!(session.nic.as_str());
    }
    args
}

/// iso 模式 `vm.command create` 的 args（纯函数，便于单测）。
///
/// 契约 A.3.2：注入 `iso_name / disk_bus / nic / start=true`，由 agent 挂 CD-ROM
/// 并按 cdrom 优先启动。
pub(crate) fn iso_create_vm_args(session: &BuildSession, iso_name: &str) -> Value {
    json!({
        "name": session.build_vm_name,
        "vcpus": session.vcpus,
        "memory_mib": session.memory_mib,
        "disk_gb": session.disk_gb,
        "iso_name": iso_name,
        "disk_bus": session.disk_bus.as_str(),
        "nic": session.nic.as_str(),
        "start": true,
    })
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
        BuildState::Installing => Ok(session),
        BuildState::Ejecting => advance_ejecting(state, session).await,
        BuildState::Finishing => advance_finishing(state, session).await,
        BuildState::Making => advance_making(state, session).await,
        _ => Ok(session),
    }
}

/// preparing：按来源模式分流。
///
/// - base：等待底包 transfer 完成 → 发 `create`（差分盘 backing=底包）；
/// - iso：等待 `iso.list` 校验结果 → ISO 存在后发 `create`（挂 CD-ROM）。
async fn advance_preparing(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    match session.source_mode {
        SourceMode::Base => advance_preparing_base(state, session).await,
        SourceMode::Iso => advance_preparing_iso(state, session).await,
    }
}

/// v3 ISO 资产分发（幂等）：生成 presigned GET（TTL≥1h）→ 入队 `vm.command iso.pull`
/// → 记录 `iso_pull_request_id`（状态仍 `preparing`）。GET 也会调用它做失败重试。
async fn start_iso_distribution(
    state: &AppState,
    session: BuildSession,
) -> AppResult<BuildSession> {
    if session.iso_pull_request_id.is_some() {
        return Ok(session);
    }
    let Some(iso_id) = session.iso_id else {
        return Ok(session);
    };
    let iso = state
        .isos
        .get(iso_id)
        .await?
        .ok_or_else(|| AppError::not_found("ISO 资产不存在"))?;
    if !is_iso_usable(&iso) {
        return transition_failed(
            state,
            &session,
            format!("ISO 资产未就绪（status={}）", iso.status),
        )
        .await;
    }
    let Some(storage_key) = iso.storage_key.as_deref() else {
        return transition_failed(state, &session, "ISO 资产缺少存储对象").await;
    };
    // 未配置对象存储 → 503（POST 已在创建会话前断言过一次；GET 重试时同样可能遇到）。
    let storage = super::storage::service(state)?;
    let presigned = storage
        .presign_get_with_ttl(storage_key, ISO_PRESIGN_TTL_SECS)
        .map_err(|error| AppError::internal(format!("生成 ISO 下载链接失败: {error}")))?;
    let args = json!({
        "name": iso.filename,
        "url": presigned.url,
        "sha256": iso.sha256,
        "size_bytes": iso.size_bytes,
    });
    let request_id =
        enqueue_session_vm_command(state, &session.machine_serial, "iso.pull", args).await?;
    let update = SessionUpdate::new(&[BuildState::Preparing], BuildState::Preparing)
        .with_iso_pull_request(request_id);
    match state
        .image_studio
        .update_session(session.id, &update)
        .await?
    {
        Some(updated) => Ok(updated),
        None => reload_session(state, session.id).await,
    }
}

/// `preparing`（iso 资产）：确保 iso.pull 已下发 → 轮询结果 → ok 后进入创建 VM。
///
/// 分发是幂等的：目标机已有同名且 sha 匹配的 ISO 时 agent 返回 `skipped=true`（仍为 ok），
/// 会话直接进入创建 VM。超过 30 分钟未完成 → failed（契约 B.3）。
async fn advance_preparing_iso_asset(
    state: &AppState,
    session: BuildSession,
) -> AppResult<BuildSession> {
    let Some(request_id) = session.iso_pull_request_id else {
        return start_iso_distribution(state, session).await;
    };
    match command_outcome(state, request_id).await? {
        CommandOutcome::Pending => {
            if Utc::now() - session.updated_at > chrono::Duration::minutes(30) {
                return transition_failed(
                    state,
                    &session,
                    "ISO 分发超时（>30 分钟），请检查目标机网络或稍后重试",
                )
                .await;
            }
            Ok(session)
        }
        CommandOutcome::Ok(_) => {
            let iso_name = session.iso_name.clone().unwrap_or_default();
            let args = iso_create_vm_args(&session, &iso_name);
            let create_request_id =
                match enqueue_session_vm_command(state, &session.machine_serial, "create", args)
                    .await
                {
                    Ok(request_id) => request_id,
                    Err(error) => {
                        return transition_failed(state, &session, error.to_string()).await;
                    }
                };
            let update = SessionUpdate::new(&[BuildState::Preparing], BuildState::Creating)
                .with_create_request(create_request_id);
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

/// `preparing`（iso）：轮询 `iso.list`；ISO 存在 → 发 `create`（iso_name/disk_bus/nic/
/// start=true）并进入 `creating`；ISO 不存在/命令失败 → `failed`。
async fn advance_preparing_iso(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    // v3：平台 ISO 库资产 → 先分发（iso.pull）再创建 VM；v2 的 iso_name 路径保持原逻辑。
    if session.iso_id.is_some() {
        return advance_preparing_iso_asset(state, session).await;
    }
    let iso_name = session.iso_name.clone().unwrap_or_default();
    let Some(request_id) = session.iso_check_request_id else {
        // 兜底：v1 数据/人工写入的 iso 会话没有校验 id，直接补发一次 iso.list。
        let request_id =
            match enqueue_session_vm_command(state, &session.machine_serial, "iso.list", json!({}))
                .await
            {
                Ok(request_id) => request_id,
                Err(error) => return transition_failed(state, &session, error.to_string()).await,
            };
        let update = SessionUpdate::new(&[BuildState::Preparing], BuildState::Preparing)
            .with_iso_check_request(request_id);
        return match state
            .image_studio
            .update_session(session.id, &update)
            .await?
        {
            Some(updated) => Ok(updated),
            None => reload_session(state, session.id).await,
        };
    };

    let (status, data, error) = match command_outcome(state, request_id).await? {
        CommandOutcome::Pending => return Ok(session),
        CommandOutcome::Ok(data) => ("ok", data, None),
        CommandOutcome::Failed(error) => ("error", None, Some(error)),
    };
    match iso_check_outcome(status, data.as_ref(), error.as_deref(), &iso_name) {
        None => Ok(session),
        Some(Err(error)) => transition_failed(state, &session, error).await,
        Some(Ok(())) => {
            let args = iso_create_vm_args(&session, &iso_name);
            let create_request_id =
                match enqueue_session_vm_command(state, &session.machine_serial, "create", args)
                    .await
                {
                    Ok(request_id) => request_id,
                    Err(error) => {
                        return transition_failed(state, &session, error.to_string()).await;
                    }
                };
            let update = SessionUpdate::new(&[BuildState::Preparing], BuildState::Creating)
                .with_create_request(create_request_id);
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

/// preparing（base）：base transfer completed → 发 `vm.command create`（差分盘 backing=底包）
/// 并进入 creating；failed/expired → failed；pending/running → 等待。
async fn advance_preparing_base(
    state: &AppState,
    session: BuildSession,
) -> AppResult<BuildSession> {
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
            let Some(base_image_id) = session.base_image_id else {
                return transition_failed(state, &session, "base 模式缺少基础镜像，请取消后重试")
                    .await;
            };
            let base = match state.storage_repo.get_image(base_image_id).await? {
                Some(base) => base,
                None => {
                    return transition_failed(state, &session, "基础镜像不存在（可能已被删除）")
                        .await;
                }
            };
            let args = base_create_vm_args(&session, &base_volume_name(&base.name, &base.version));
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
            let target = state_after_create(session.source_mode);
            let update = SessionUpdate::new(&[BuildState::Creating], target);
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

/// ejecting：`vm.command eject` ok → ready（`media_ejected=true`）；失败 → failed。
async fn advance_ejecting(state: &AppState, session: BuildSession) -> AppResult<BuildSession> {
    let Some(request_id) = session.eject_request_id else {
        return Ok(session);
    };
    match command_outcome(state, request_id).await? {
        CommandOutcome::Pending => Ok(session),
        CommandOutcome::Ok(_) => {
            let update = SessionUpdate::new(&[BuildState::Ejecting], BuildState::Ready)
                .with_media_ejected(true);
            match state
                .image_studio
                .update_session(session.id, &update)
                .await?
            {
                Some(updated) => Ok(updated),
                None => reload_session(state, session.id).await,
            }
        }
        CommandOutcome::Failed(error) => {
            transition_failed(state, &session, format!("弹出 ISO 失败：{error}")).await
        }
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

    // ---- 来源模式 + 基础镜像 / ISO（契约附录 A.3.2） ----
    let source_mode = parse_source_mode(request.source.as_deref())?;
    let disk_bus = parse_disk_bus(request.disk_bus.as_deref(), source_mode)?;
    let nic = parse_nic_mode(request.nic.as_deref())?;
    let base = match source_mode {
        SourceMode::Base => {
            let base_image_id = request.base_image_id.ok_or_else(|| {
                AppError::bad_request("source=base 时 base_image_id 必填（已发布底包）")
            })?;
            let base =
                require_visible_published_image(&state, id, base_image_id, "base_image_id").await?;
            if base.kind != crate::domain::storage::ImageKind::VmDisk {
                return Err(AppError::bad_request(
                    "base_image_id 必须是 vm_disk 类型的系统底包（agent 以 <name>-<version> 卷名创建差分盘）",
                ));
            }
            Some(base)
        }
        // iso 模式没有底包（迁移 0040：base_image_id 可空）；忽略客户端传来的 base_image_id。
        SourceMode::Iso => None,
    };
    validate_iso_source_selection(source_mode, request.iso_id, request.iso_name.as_deref())?;
    // iso_id：查平台 ISO 库（ready + 可见），filename 取库记录；iso_name：兼容 v2。
    let iso_asset = match (source_mode, request.iso_id) {
        (SourceMode::Iso, Some(iso_id)) => {
            let iso = state
                .isos
                .get(iso_id)
                .await?
                .ok_or_else(|| AppError::not_found("ISO 资产不存在"))?;
            let is_admin = super::images::actor_is_admin(&state, id).await?;
            if !crate::domain::install_iso::is_iso_visible(&iso, id, is_admin) {
                return Err(AppError::forbidden("无权使用他人的私有 ISO"));
            }
            if !is_iso_usable(&iso) {
                return Err(AppError::conflict(format!(
                    "ISO 资产未就绪（status={}，需为 ready）",
                    iso.status
                )));
            }
            // 提前断言对象存储可用，避免创建会话后才发现无法生成下载链接。
            super::storage::service(&state)?;
            Some(iso)
        }
        _ => None,
    };
    let iso_name = match source_mode {
        SourceMode::Base => None,
        SourceMode::Iso => match &iso_asset {
            Some(iso) => Some(iso.filename.clone()),
            None => {
                let raw = request
                    .iso_name
                    .as_deref()
                    .ok_or_else(|| AppError::bad_request("source=iso 时 iso_name 必填"))?;
                Some(validate_iso_name(raw).map_err(AppError::bad_request)?)
            }
        },
    };
    if source_mode == SourceMode::Iso && request.delta_image_id.is_some() {
        return Err(AppError::bad_request("source=iso 不支持 delta_image_id"));
    }
    let delta = match request.delta_image_id {
        Some(delta_id) if source_mode == SourceMode::Base => {
            Some(require_visible_published_image(&state, id, delta_id, "delta_image_id").await?)
        }
        _ => None,
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

    // ---- 来源相关的准备：base 确保底包分配/拉取；iso 先校验目标机上的 ISO 存在 ----
    let target_slot = machine.device_slot.clone();
    let (base_image_id, delta_image_id, base_transfer_id, iso_check_request_id) = match source_mode
    {
        SourceMode::Base => {
            let base = base
                .as_ref()
                .ok_or_else(|| AppError::internal("base 模式缺少已校验底包"))?;
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
            (
                Some(base.id),
                delta.as_ref().map(|image| image.id),
                Some(transfer.id),
                None,
            )
        }
        SourceMode::Iso => match &iso_asset {
            // v3：平台 ISO 库。preparing 首步生成 presigned GET 并下发 iso.pull，
            // 由会话创建后的 start_iso_distribution 完成（GET 幂等重试）。
            Some(_) => (None, None, None, None),
            // v2 兼容：确认目标机 ISO 目录里存在该 ISO（记录 request_id，GET 时轮询）。
            None => {
                let request_id =
                    enqueue_session_vm_command(&state, serial, "iso.list", json!({})).await?;
                (None, None, None, Some(request_id))
            }
        },
    };

    // ---- 写会话 ----
    let session_id = Uuid::new_v4();
    let new_session = crate::domain::image_studio::NewBuildSession {
        id: session_id,
        user_id: id,
        room_id: request.room_id,
        machine_serial: serial.to_owned(),
        build_vm_name: build_vm_name(session_id),
        source_mode,
        iso_name: iso_name.clone(),
        iso_id: iso_asset.as_ref().map(|iso| iso.id),
        disk_bus,
        nic,
        base_image_id,
        delta_image_id,
        base_transfer_id,
        iso_check_request_id,
        name,
        version,
        platform,
        vcpus: vcpus as i32,
        memory_mib: memory_mib as i32,
        disk_gb: disk_gb as i32,
        description,
    };
    let session = state.image_studio.create_session(&new_session).await?;
    // v3：平台 ISO 库会话 → 立即生成 presigned GET 并下发 iso.pull（失败由 GET 重试）。
    let session = if session.iso_id.is_some() {
        start_iso_distribution(&state, session).await?
    } else {
        session
    };

    record_session_audit(
        &state,
        id,
        &session,
        "image.build.session.create",
        json!({
            "room_id": session.room_id,
            "machine_serial": session.machine_serial,
            "source_mode": session.source_mode.as_str(),
            "iso_name": session.iso_name,
            "iso_id": session.iso_id,
            "disk_bus": session.disk_bus.as_str(),
            "nic": session.nic.as_str(),
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
    if let Err(reason) = finish_gate(&session) {
        return Err(AppError::conflict(reason));
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

/// `POST /api/v1/admin/image-builds/{id}/eject-iso` —— ISO 安装完成后弹出安装介质。
///
/// 仅 iso 模式 + `installing` 状态可用（契约 A.3.2）：置 `ejecting` 并下发
/// `vm.command eject`，命令 ok 后由 GET 状态机推进到 `ready`（`media_ejected=true`）。
pub async fn eject_build_session_iso(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(session_id): Path<Uuid>,
) -> AppResult<Json<BuildSessionReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    let session = load_owned_session(&state, id, session_id).await?;
    if session.source_mode != SourceMode::Iso {
        return Err(AppError::conflict("仅 source=iso 的安装会话支持弹出 ISO"));
    }
    if !can_eject_iso(session.source_mode, session.state) {
        return Err(AppError::conflict(format!(
            "仅 installing 状态可弹出 ISO（当前 {}）",
            session.state
        )));
    }
    let request_id = enqueue_session_vm_command(
        &state,
        &session.machine_serial,
        "eject",
        json!({ "name": session.build_vm_name }),
    )
    .await?;
    let update = SessionUpdate::new(&[BuildState::Installing], BuildState::Ejecting)
        .with_eject_request(request_id);
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
        "image.build.session.eject_iso",
        json!({ "eject_request_id": request_id }),
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
    fn create_request_accepts_iso_source_without_base_image() {
        let iso: CreateBuildSessionRequest = serde_json::from_value(json!({
            "room_id": Uuid::nil(),
            "machine_serial": "dev-b104",
            "source": "iso",
            "iso_name": "Win10_LTSC_2021.iso",
            "disk_bus": "sata",
            "nic": "none",
            "name": "win10-ltsc",
            "version": "0.1.0",
            "platform": "windows/amd64",
            "vcpus": 4,
            "memory_mib": 4096,
            "disk_gb": 40,
        }))
        .expect("iso request");
        assert_eq!(iso.source.as_deref(), Some("iso"));
        assert!(iso.base_image_id.is_none());
        assert_eq!(iso.iso_name.as_deref(), Some("Win10_LTSC_2021.iso"));
        assert_eq!(iso.disk_bus.as_deref(), Some("sata"));

        // v3：iso_id（平台 ISO 库）也可用，且与 iso_name 二选一。
        let iso_asset: CreateBuildSessionRequest = serde_json::from_value(json!({
            "room_id": Uuid::nil(),
            "machine_serial": "dev-b104",
            "source": "iso",
            "iso_id": Uuid::nil(),
            "name": "win10-ltsc",
            "version": "0.2.0",
        }))
        .expect("iso_id request");
        assert_eq!(iso_asset.iso_id, Some(Uuid::nil()));
        assert!(iso_asset.iso_name.is_none());
    }

    #[test]
    fn iso_source_selection_requires_exactly_one() {
        // base：两个都不能给。
        assert!(validate_iso_source_selection(SourceMode::Base, None, None).is_ok());
        assert!(
            validate_iso_source_selection(SourceMode::Base, Some(Uuid::new_v4()), None).is_err()
        );
        assert!(validate_iso_source_selection(SourceMode::Base, None, Some("a.iso")).is_err());
        // iso：必须二选一，同时给报错。
        let iso_id = Uuid::new_v4();
        assert!(validate_iso_source_selection(SourceMode::Iso, Some(iso_id), None).is_ok());
        assert!(validate_iso_source_selection(SourceMode::Iso, None, Some("a.iso")).is_ok());
        assert!(
            validate_iso_source_selection(SourceMode::Iso, Some(iso_id), Some("a.iso")).is_err()
        );
        assert!(validate_iso_source_selection(SourceMode::Iso, None, None).is_err());
    }

    #[test]
    fn source_disk_bus_and_nic_parsing_defaults_and_errors() {
        assert_eq!(parse_source_mode(None).unwrap(), SourceMode::Base);
        assert_eq!(parse_source_mode(Some(" iso ")).unwrap(), SourceMode::Iso);
        assert!(parse_source_mode(Some("docker")).is_err());

        // disk_bus 缺省按来源模式：base→virtio、iso→sata。
        assert_eq!(
            parse_disk_bus(None, SourceMode::Base).unwrap(),
            DiskBus::Virtio
        );
        assert_eq!(
            parse_disk_bus(None, SourceMode::Iso).unwrap(),
            DiskBus::Sata
        );
        assert_eq!(
            parse_disk_bus(Some("sata"), SourceMode::Base).unwrap(),
            DiskBus::Sata
        );
        assert!(parse_disk_bus(Some("scsi"), SourceMode::Base).is_err());

        assert_eq!(parse_nic_mode(None).unwrap(), NicMode::None);
        assert_eq!(parse_nic_mode(Some("user")).unwrap(), NicMode::User);
        assert_eq!(parse_nic_mode(Some("default")).unwrap(), NicMode::Default);
        assert!(parse_nic_mode(Some("bridge")).is_err());
    }

    fn session_with(source_mode: SourceMode, state: BuildState) -> BuildSession {
        let now = Utc::now();
        BuildSession {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            room_id: Uuid::new_v4(),
            machine_serial: "dev-b104".to_owned(),
            build_vm_name: "qe-build-a1b2c3d4".to_owned(),
            source_mode,
            iso_name: (source_mode == SourceMode::Iso).then(|| "Win10_LTSC_2021.iso".to_owned()),
            iso_id: None,
            iso_pull_request_id: None,
            disk_bus: source_mode.default_disk_bus(),
            nic: NicMode::None,
            media_ejected: false,
            iso_check_request_id: None,
            eject_request_id: None,
            base_image_id: (source_mode == SourceMode::Base).then(Uuid::new_v4),
            delta_image_id: None,
            base_transfer_id: None,
            name: "lab-base".to_owned(),
            version: "1.0.0".to_owned(),
            platform: "linux/amd64".to_owned(),
            vcpus: 2,
            memory_mib: 2_048,
            disk_gb: 40,
            description: None,
            state,
            error: None,
            create_request_id: None,
            shutdown_request_id: None,
            destroy_request_id: None,
            make_request_id: None,
            image_id: None,
            created_at: now,
            updated_at: now,
            finished_at: None,
        }
    }

    #[test]
    fn iso_state_machine_transitions() {
        // 创建 VM 成功后的下一状态：base→ready，iso→installing。
        assert_eq!(state_after_create(SourceMode::Base), BuildState::Ready);
        assert_eq!(state_after_create(SourceMode::Iso), BuildState::Installing);
        // eject 仅 iso + installing。
        assert!(can_eject_iso(SourceMode::Iso, BuildState::Installing));
        assert!(!can_eject_iso(SourceMode::Iso, BuildState::Ready));
        assert!(!can_eject_iso(SourceMode::Base, BuildState::Installing));
        // iso 全路径上的状态都应是非终态（可继续推进/取消）。
        for state in [
            BuildState::Preparing,
            BuildState::Creating,
            BuildState::Installing,
            BuildState::Ejecting,
            BuildState::Ready,
            BuildState::Finishing,
            BuildState::Making,
        ] {
            assert!(!state.is_terminal(), "{state}");
        }
    }

    #[test]
    fn iso_asset_session_steps_flow() {
        // iso_id 会话：7 步含 iso_distribute；preparing 未发 iso.pull 时校验中。
        let mut session = session_with(SourceMode::Iso, BuildState::Preparing);
        session.iso_id = Some(Uuid::new_v4());
        let steps = crate::domain::image_studio::session_steps(&session, None);
        let keys: Vec<&str> = steps.iter().map(|step| step.key.as_str()).collect();
        assert_eq!(
            keys,
            crate::domain::image_studio::ISO_ID_STEP_KEYS,
            "iso_id 会话应有 7 步"
        );
        // 已发 iso.pull → 分发中。
        session.iso_pull_request_id = Some(Uuid::new_v4());
        let steps = crate::domain::image_studio::session_steps(&session, None);
        assert_eq!(steps[1].key, "iso_distribute");
        assert_eq!(
            steps[1].status,
            crate::domain::image_studio::StepStatus::Running
        );
    }

    #[test]
    fn create_vm_args_include_iso_and_disk_bus() {
        // base + Windows 底包：disk_bus=sata，未指定 nic 时不注入。
        let mut base = session_with(SourceMode::Base, BuildState::Preparing);
        base.disk_bus = DiskBus::Sata;
        let args = base_create_vm_args(&base, "win10-ltsc-0.1.0");
        assert_eq!(args["base_volume"], json!("win10-ltsc-0.1.0"));
        assert_eq!(args["disk_bus"], json!("sata"));
        assert_eq!(args["start"], json!(true));
        assert!(args.get("nic").is_none());
        assert!(args.get("iso_name").is_none());

        // base + 显式网卡。
        base.nic = NicMode::User;
        assert_eq!(base_create_vm_args(&base, "v")["nic"], json!("user"));

        // iso：注入 iso_name/disk_bus/nic/start。
        let mut iso = session_with(SourceMode::Iso, BuildState::Preparing);
        iso.disk_bus = DiskBus::Sata;
        iso.nic = NicMode::None;
        let args = iso_create_vm_args(&iso, "Win10_LTSC_2021.iso");
        assert_eq!(args["iso_name"], json!("Win10_LTSC_2021.iso"));
        assert_eq!(args["disk_bus"], json!("sata"));
        assert_eq!(args["nic"], json!("none"));
        assert_eq!(args["start"], json!(true));
        assert!(args.get("base_volume").is_none());
    }

    #[test]
    fn finish_gate_enforces_ready_and_iso_ejected() {
        // base：ready 允许。
        assert!(finish_gate(&session_with(SourceMode::Base, BuildState::Ready)).is_ok());
        // 非 ready 拒绝。
        assert!(finish_gate(&session_with(SourceMode::Base, BuildState::Creating)).is_err());
        assert!(finish_gate(&session_with(SourceMode::Iso, BuildState::Installing)).is_err());
        // iso ready 但未 eject → 拒绝（契约 A.2）。
        let mut iso_ready = session_with(SourceMode::Iso, BuildState::Ready);
        let error = finish_gate(&iso_ready).unwrap_err();
        assert!(error.contains("弹出 ISO"), "{error}");
        iso_ready.media_ejected = true;
        assert!(finish_gate(&iso_ready).is_ok());
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
        let mut session = session_with(SourceMode::Base, BuildState::Preparing);
        session.description = Some("build".to_owned());
        let reply = BuildSessionReply::from_parts(session, None, None, None);
        let value = serde_json::to_value(&reply).expect("serialize");
        for key in [
            "id",
            "state",
            "error",
            "room_id",
            "machine_serial",
            "build_vm_name",
            "source_mode",
            "iso_name",
            "iso_id",
            "disk_bus",
            "nic",
            "media_ejected",
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
        assert_eq!(value["source_mode"], json!("base"));
        assert_eq!(value["disk_bus"], json!("virtio"));
        assert_eq!(value["nic"], json!("none"));
        assert_eq!(value["media_ejected"], json!(false));
        assert!(value["iso_name"].is_null());
        assert!(value["iso_id"].is_null());
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
