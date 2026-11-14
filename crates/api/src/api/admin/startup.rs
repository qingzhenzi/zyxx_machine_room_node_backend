//! 房间启动配置（startup provisioning）管理端接口。
//!
//! 契约见 `docs/startup-provisioning.md` §2 / §3：
//! - `GET  /api/v1/admin/rooms/{room_id}/startup`：读取配置 + 镜像/目标设备摘要；
//! - `PUT  /api/v1/admin/rooms/{room_id}/startup`：严格校验后 upsert（revision+1），
//!   确保每个镜像有 download transfer，并向房间全部设备槽位入队
//!   `startup.update` 下行（尽力唤醒长连接，失败只 warn）。
//!
//! 校验口径完全按契约 §2 表格（字段名 snake_case，错误信息指出字段）：
//! - `images` 0..=8、`image_id` 去重，镜像必须存在且 `published` + manifest 完整；
//! - `vms` 0..=32，`name` 去重，`base_volume` / `base_path` 互斥，`gpu` ≤2；
//! - `vcpus` 1..=64（缺省 2）、`memory_mib` 256..=262144（缺省 2048）、
//!   `disk_gb` 1..=2048（缺省 20）、`auto_start` 缺省 true；
//! - `vms[].network` 可选：`mode` 必填，VLAN 1..=4094、子网必须是 RFC1918 私有
//!   网段且至少 /30，allowlist 不允许 `0.0.0.0/0`、端口必须显式；同一房间的
//!   所有 VM 共享一张房间桥，因此 `network` 配置必须完全一致；
//! - 缺省 `network=null` = 不配网（旧配置语义不变）；
//! - `auto_pull` / `auto_start_vm` 缺省 true。
//!
//! 缺省值由 `qe_db::startup` 的 serde 默认值负责，因此落库与回显的配置总是完整形状。

use std::collections::HashSet;
use std::net::Ipv4Addr;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    db::startup::{
        MAX_STARTUP_IMAGES, MAX_STARTUP_VMS, RoomStartupProfile, StartupAllowService,
        StartupConfig, StartupNetworkMode, StartupPort, StartupVm, StartupVmNetwork,
    },
    domain::{rbac::permissions, storage::ArtifactStatus},
    error::{AppError, AppResult},
    state::AppState,
};

/// `startup.update` 下行 kind（契约 §4.2）。
pub const STARTUP_UPDATE_KIND: &str = "startup.update";

/// GPU 直通地址上限（与 vm.command create 一致）。
const STARTUP_GPU_MAX: usize = 2;
/// VM 规格范围（契约 §2；缺省值由 `qe_db::startup` 补齐）。
const VCPUS_RANGE: (u64, u64) = (1, 64);
const MEMORY_MIB_RANGE: (u64, u64) = (256, 262_144);
const DISK_GB_RANGE: (u64, u64) = (1, 2_048);

/// `PUT /admin/rooms/{room_id}/startup` 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateStartupRequest {
    pub enabled: bool,
    pub config: StartupConfig,
}

/// GET / PUT 响应中的镜像摘要（契约 §3.1）。
#[derive(Debug, Clone, Serialize)]
pub struct StartupImageReply {
    pub image_id: Uuid,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub status: String,
    pub manifest_digest: Option<String>,
    pub total_size: i64,
    /// 该房间该镜像最新一条 download transfer；缺失为 null。
    pub transfer_id: Option<Uuid>,
    /// 该镜像是否已分配到本房间（device-pki presign 授权依赖它；便于排障）。
    pub assigned: bool,
}

/// GET / PUT 响应中的下发目标（房间主设备 + `room_devices` 子机）。
#[derive(Debug, Clone, Serialize)]
pub struct StartupTargetReply {
    pub device_slot: String,
    /// 该槽位最近一次注册的证书 serial（未注册为 null）。
    pub serial: Option<String>,
}

/// GET / PUT 响应。无配置行时：`enabled=false, revision=0, config=默认值`。
#[derive(Debug, Clone, Serialize)]
pub struct StartupReply {
    pub room_id: Uuid,
    pub enabled: bool,
    pub revision: i64,
    pub config: StartupConfig,
    /// 无配置行时为 null。
    pub updated_at: Option<String>,
    pub images: Vec<StartupImageReply>,
    pub targets: Vec<StartupTargetReply>,
}

/// `GET /api/v1/admin/rooms/{room_id}/startup` —— 读取房间启动配置。
///
/// 权限 `vm:control`；房间不存在 404；无配置行返回默认空配置（200）。
pub async fn get(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Path(room_id): Path<Uuid>,
) -> AppResult<Json<StartupReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    if state.rooms.get(room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }

    let profile = state.startup.get(room_id).await?;
    Ok(Json(build_reply(&state, room_id, profile).await?))
}

/// `PUT /api/v1/admin/rooms/{room_id}/startup` —— 保存房间启动配置。
///
/// 行为（契约 §3.2）：房间不存在 404 → 校验 config 400 → upsert（revision+1）
/// → 确保镜像 transfer → 向所有设备槽位入队 `startup.update` → 返回与 GET 相同结构。
pub async fn update(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    Path(room_id): Path<Uuid>,
    ApiJson(request): ApiJson<UpdateStartupRequest>,
) -> AppResult<Json<StartupReply>> {
    authorization::require_permission(&state, id, permissions::VM_CONTROL).await?;
    if state.rooms.get(room_id).await?.is_none() {
        return Err(AppError::not_found("房间不存在"));
    }

    let config = validate_config(request.config)?;
    validate_published_images(&state, &config).await?;

    let profile = state
        .startup
        .upsert(room_id, request.enabled, &config, id)
        .await?;
    let revision = profile.revision;

    super::audit::record_or_log(
        &state,
        id,
        "room.startup.update",
        "room",
        Some(room_id.to_string()),
        json!({
            "enabled": profile.enabled,
            "revision": revision,
            "images": config.images.iter().map(|image| image.image_id).collect::<Vec<_>>(),
            "vms": config.vms.iter().map(|vm| &vm.name).collect::<Vec<_>>(),
        }),
        ip.as_deref(),
    )
    .await;

    dispatch_startup_update(&state, room_id, revision).await;

    Ok(Json(build_reply(&state, room_id, Some(profile)).await?))
}

/// 校验并规范化配置；返回写完缺省值、去重后的配置。
///
/// 纯函数（不查库），便于单测；镜像存在性/发布状态由 [`validate_published_images`] 校验。
fn validate_config(config: StartupConfig) -> AppResult<StartupConfig> {
    if config.images.len() > MAX_STARTUP_IMAGES {
        return Err(AppError::bad_request(format!(
            "config.images 最多 {MAX_STARTUP_IMAGES} 个（当前 {}）",
            config.images.len()
        )));
    }
    if config.vms.len() > MAX_STARTUP_VMS {
        return Err(AppError::bad_request(format!(
            "config.vms 最多 {MAX_STARTUP_VMS} 个（当前 {}）",
            config.vms.len()
        )));
    }

    // images 去重（保留首次出现）。
    let mut seen_images = HashSet::new();
    let mut images = Vec::with_capacity(config.images.len());
    for image in config.images {
        if seen_images.insert(image.image_id) {
            images.push(image);
        }
    }

    // vms 去重 + 逐字段校验（保留首次出现）。
    let mut seen_names = HashSet::new();
    let mut vms = Vec::with_capacity(config.vms.len());
    let mut room_network: Option<StartupVmNetwork> = None;
    for (index, vm) in config.vms.into_iter().enumerate() {
        let vm = validate_vm(vm, index)?;
        // 同房间所有 VM 共用一张桥（network_name 由 room_id/vlan 决定），
        // 配置不一致会让每轮 reconcile 互相覆盖，因此写入前就拒绝。
        if let Some(network) = vm.network.as_ref() {
            match &room_network {
                None => room_network = Some(network.clone()),
                Some(expected) if expected != network => {
                    return Err(AppError::bad_request(format!(
                        "config.vms[{index}].network 与同房间其它 VM 不一致：\
                         所有 VM 共享一张房间桥，mode/vlan_id/subnet/allowlist 必须完全相同"
                    )));
                }
                Some(_) => {}
            }
        }
        if seen_names.insert(vm.name.clone()) {
            vms.push(vm);
        }
    }

    Ok(StartupConfig {
        auto_pull: config.auto_pull,
        auto_start_vm: config.auto_start_vm,
        images,
        vms,
    })
}

/// 校验单个 VM 配置（字段错误信息带 `config.vms[{index}]` 前缀）。
fn validate_vm(vm: StartupVm, index: usize) -> AppResult<StartupVm> {
    let prefix = format!("config.vms[{index}]");
    let name = super::machines::validate_name_token(&vm.name, &format!("{prefix}.name"), 64)?;
    let vcpus = check_count(vm.vcpus, &format!("{prefix}.vcpus"), VCPUS_RANGE)?;
    let memory_mib = check_count(
        vm.memory_mib,
        &format!("{prefix}.memory_mib"),
        MEMORY_MIB_RANGE,
    )?;
    let disk_gb = check_count(vm.disk_gb, &format!("{prefix}.disk_gb"), DISK_GB_RANGE)?;

    let base_volume = vm
        .base_volume
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            super::machines::validate_name_token(value, &format!("{prefix}.base_volume"), 128)
        })
        .transpose()?;
    let base_path = vm
        .base_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| super::machines::validate_base_path(value))
        .transpose()?;
    if base_volume.is_some() && base_path.is_some() {
        return Err(AppError::bad_request(format!(
            "{prefix}.base_volume 与 {prefix}.base_path 只能提供一个"
        )));
    }

    if vm.gpu.len() > STARTUP_GPU_MAX {
        return Err(AppError::bad_request(format!(
            "{prefix}.gpu 最多 {STARTUP_GPU_MAX} 个（当前 {}）",
            vm.gpu.len()
        )));
    }
    let mut gpu = Vec::with_capacity(vm.gpu.len());
    for (gpu_index, raw) in vm.gpu.iter().enumerate() {
        gpu.push(super::machines::validate_gpu_address(raw).map_err(|_| {
            AppError::bad_request(format!(
                "{prefix}.gpu[{gpu_index}] 非法（{raw}），应为 dddd:bb:dd.f"
            ))
        })?);
    }

    let network = validate_vm_network(vm.network, &prefix)?;

    Ok(StartupVm {
        name,
        vcpus,
        memory_mib,
        disk_gb,
        base_volume,
        base_path,
        gpu,
        auto_start: vm.auto_start,
        network,
    })
}

/// 校验并规范化 `config.vms[].network`。
///
/// 规则与 `qe-vm-net::VmNetworkProfile::validate` 对齐（agent 侧仍会再校验一次）：
/// - 缺省 / `mode="none"`：不配网；`none` 时不允许再带 vlan/subnet/allowlist；
/// - `vlan_id` 1..=4094；
/// - `subnet` 必须是 RFC1918、前缀 <=30，并归一为网络地址；
/// - `allow_internet` 只允许 `mode="nat"`；
/// - allowlist：`cidr` 不能是 `0.0.0.0/0`、不能与自身子网重叠、端口必须显式且合法；
/// - `proto` 归一为小写 `tcp`/`udp`/`any`，缺省 `tcp`。
fn validate_vm_network(
    network: Option<StartupVmNetwork>,
    prefix: &str,
) -> AppResult<Option<StartupVmNetwork>> {
    let Some(mut network) = network else {
        return Ok(None);
    };
    let field = format!("{prefix}.network");

    if let Some(vlan) = network.vlan_id
        && !(1..=4094).contains(&vlan)
    {
        return Err(AppError::bad_request(format!(
            "{field}.vlan_id 越界: {vlan}（允许 1..=4094）"
        )));
    }

    if network.mode == StartupNetworkMode::None {
        if network.allow_internet {
            return Err(AppError::bad_request(format!(
                "{field}.allow_internet 不能与 mode=none 组合（没有网络出口）"
            )));
        }
        if network.vlan_id.is_some()
            || network
                .subnet
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            || !network.allow_services.is_empty()
        {
            return Err(AppError::bad_request(format!(
                "{field} 的 mode=none 不创建网络，不能同时配置 vlan_id/subnet/allow_services"
            )));
        }
        // 空字符串归一成 null，避免回显 `"subnet": ""` 这种歧义形状。
        network.subnet = None;
        return Ok(Some(network));
    }

    if network.allow_internet && network.mode != StartupNetworkMode::Nat {
        return Err(AppError::bad_request(format!(
            "{field}.allow_internet 需要 mode=nat（isolated 没有出口）"
        )));
    }

    // 子网归一：显式给出时必须私有、至少 /30；缺省由 agent 按 room_id 派生。
    let subnet = match network
        .subnet
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => None,
        Some(raw) => {
            let (addr, len) = parse_cidr(raw).ok_or_else(|| {
                AppError::bad_request(format!(
                    "{field}.subnet 非法: {raw:?}（示例 10.77.104.0/24）"
                ))
            })?;
            if len > 30 {
                return Err(AppError::bad_request(format!(
                    "{field}.subnet {raw:?} 太小（/ {len}），至少需要 /30 才能放网关与 DHCP"
                )));
            }
            let net = mask_ipv4(addr, len);
            if !is_private_ipv4(net) || !is_private_ipv4(broadcast_ipv4(net, len)) {
                return Err(AppError::bad_request(format!(
                    "{field}.subnet {raw:?} 不是 RFC1918 私有网段（10/8、172.16/12、192.168/16）"
                )));
            }
            Some(format!("{net}/{len}"))
        }
    };

    let mut allow_services = Vec::with_capacity(network.allow_services.len());
    for (index, service) in network.allow_services.into_iter().enumerate() {
        allow_services.push(validate_allow_service(
            service,
            subnet.as_deref(),
            &format!("{field}.allow_services[{index}]"),
        )?);
    }

    network.subnet = subnet;
    network.allow_services = allow_services;
    Ok(Some(network))
}

/// 校验一条 allowlist；返回归一化后的条目。
fn validate_allow_service(
    service: StartupAllowService,
    own_subnet: Option<&str>,
    field: &str,
) -> AppResult<StartupAllowService> {
    let raw_cidr = service.cidr.trim();
    let (addr, len) = parse_cidr(raw_cidr).ok_or_else(|| {
        AppError::bad_request(format!(
            "{field}.cidr 非法: {raw_cidr:?}（示例 192.168.8.231/32 或 192.168.8.0/24）"
        ))
    })?;
    if len == 0 {
        return Err(AppError::bad_request(format!(
            "{field}.cidr 不能用 0.0.0.0/0（等于放行所有目标）；请写具体 CIDR/IP"
        )));
    }
    let target = mask_ipv4(addr, len);
    if let Some(own) = own_subnet.and_then(parse_cidr) {
        let own_net = mask_ipv4(own.0, own.1);
        if overlaps(target, len, own_net, own.1) {
            return Err(AppError::bad_request(format!(
                "{field}.cidr {raw_cidr:?} 与 VM 自身子网 {own_net}/{} 重叠：这等于放行横向访问",
                own.1
            )));
        }
    }

    let proto = match service
        .proto
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => "tcp".to_owned(),
        Some(raw) => match raw.to_ascii_lowercase().as_str() {
            "tcp" => "tcp".to_owned(),
            "udp" => "udp".to_owned(),
            "any" => "any".to_owned(),
            other => {
                return Err(AppError::bad_request(format!(
                    "{field}.proto 不支持: {other:?}（允许 tcp|udp|any）"
                )));
            }
        },
    };

    if service.ports.is_empty() {
        return Err(AppError::bad_request(format!(
            "{field}.ports 不能为空（必须显式列出端口，避免误放行全部端口）"
        )));
    }
    let mut ports = Vec::with_capacity(service.ports.len());
    for (index, port) in service.ports.into_iter().enumerate() {
        ports.push(
            normalize_port(port).map_err(|reason| {
                AppError::bad_request(format!("{field}.ports[{index}] {reason}"))
            })?,
        );
    }

    Ok(StartupAllowService {
        cidr: format!("{target}/{len}"),
        proto: Some(proto),
        ports,
    })
}

/// 归一端口：数字保持数字，范围字符串解析后重新格式化（拒绝 0 与反向范围）。
fn normalize_port(port: StartupPort) -> Result<StartupPort, String> {
    match port {
        StartupPort::Port(0) => Err("端口 0 不合法".to_owned()),
        StartupPort::Port(port) => Ok(StartupPort::Port(port)),
        StartupPort::Text(raw) => {
            let raw = raw.trim();
            let (start, end) = match raw.split_once('-') {
                Some((start, end)) => (start.trim(), Some(end.trim())),
                None => (raw, None),
            };
            let start: u16 = start
                .parse()
                .map_err(|_| format!("非法: {raw:?}（示例 8080 或 8000-8100）"))?;
            let end: u16 = match end {
                Some(end) => end
                    .parse()
                    .map_err(|_| format!("非法: {raw:?}（示例 8080 或 8000-8100）"))?,
                None => start,
            };
            if start == 0 || end == 0 {
                return Err(format!("端口 0 不合法: {raw:?}"));
            }
            if start > end {
                return Err(format!("端口段起始大于结束: {raw:?}"));
            }
            if start == end {
                Ok(StartupPort::Port(start))
            } else {
                Ok(StartupPort::Text(format!("{start}-{end}")))
            }
        }
    }
}

/// 解析 `a.b.c.d[/len]`（缺省 /32）。
fn parse_cidr(raw: &str) -> Option<(Ipv4Addr, u8)> {
    let raw = raw.trim();
    let (addr, len) = match raw.split_once('/') {
        Some((addr, len)) => (addr.trim(), Some(len.trim())),
        None => (raw, None),
    };
    let addr: Ipv4Addr = addr.parse().ok()?;
    let len: u8 = match len {
        Some(len) => len.parse().ok()?,
        None => 32,
    };
    (len <= 32).then_some((addr, len))
}

fn mask_ipv4(addr: Ipv4Addr, len: u8) -> Ipv4Addr {
    let mask = if len == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(len))
    };
    Ipv4Addr::from(u32::from(addr) & mask)
}

fn broadcast_ipv4(net: Ipv4Addr, len: u8) -> Ipv4Addr {
    let mask = if len == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(len))
    };
    Ipv4Addr::from(u32::from(net) | !mask)
}

fn overlaps(left: Ipv4Addr, left_len: u8, right: Ipv4Addr, right_len: u8) -> bool {
    // 取较短前缀（更宽的网段）比较：10.0.0.0/8 与 10.1.0.0/16 必须判为重叠。
    let len = left_len.min(right_len);
    let mask = if len == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(len))
    };
    (u32::from(left) & mask) == (u32::from(right) & mask)
}

fn is_private_ipv4(addr: Ipv4Addr) -> bool {
    let octets = addr.octets();
    octets[0] == 10
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 168)
}

/// 校验 u32 计数在契约范围内（返回原值；serde 已排除 >u32::MAX 的输入）。
fn check_count(value: u32, field: &str, range: (u64, u64)) -> AppResult<u32> {
    let value = super::machines::check_range(u64::from(value), field, range)?;
    // check_range 已保证 1..=262144，回写 u32 不会截断。
    Ok(value as u32)
}

/// 校验配置引用的镜像都存在、`published` 且 manifest 完整（契约 §2 / §3.2 步骤 1）。
async fn validate_published_images(state: &AppState, config: &StartupConfig) -> AppResult<()> {
    for (index, image) in config.images.iter().enumerate() {
        let field = format!("config.images[{index}].image_id");
        let artifact = state
            .storage_repo
            .get_image(image.image_id)
            .await?
            .ok_or_else(|| {
                AppError::bad_request(format!("{field} 引用的镜像不存在（{}）", image.image_id))
            })?;
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
                "{field} 引用的镜像缺少 manifest_digest"
            )));
        }
    }
    Ok(())
}

/// 组装 GET / PUT 响应：配置行（或默认值）+ 镜像摘要 + 目标设备。
async fn build_reply(
    state: &AppState,
    room_id: Uuid,
    profile: Option<RoomStartupProfile>,
) -> AppResult<StartupReply> {
    let (enabled, revision, config, updated_at) = match profile {
        Some(profile) => (
            profile.enabled,
            profile.revision,
            profile.config,
            Some(profile.updated_at.to_rfc3339()),
        ),
        None => (false, 0, StartupConfig::default(), None),
    };

    let images = load_images(state, room_id, &config).await?;
    let targets = load_targets(state, room_id).await?;

    Ok(StartupReply {
        room_id,
        enabled,
        revision,
        config,
        updated_at,
        images,
        targets,
    })
}

/// 回查配置引用的镜像并拼出摘要；镜像已被删除时跳过并 warn（配置行保留原 id）。
async fn load_images(
    state: &AppState,
    room_id: Uuid,
    config: &StartupConfig,
) -> AppResult<Vec<StartupImageReply>> {
    let mut images = Vec::with_capacity(config.images.len());
    for image in &config.images {
        let Some(artifact) = state.storage_repo.get_image(image.image_id).await? else {
            tracing::warn!(
                %room_id,
                image_id = %image.image_id,
                "启动配置引用的镜像已不存在，响应中跳过"
            );
            continue;
        };
        let transfer_id = state
            .startup
            .latest_download_transfer_id(room_id, artifact.id)
            .await?;
        let assigned = state.startup.is_assigned(room_id, artifact.id).await?;
        images.push(StartupImageReply {
            image_id: artifact.id,
            name: artifact.name,
            version: artifact.version,
            kind: artifact.kind.as_str().to_owned(),
            status: artifact.status.as_str().to_owned(),
            manifest_digest: artifact.manifest_digest,
            total_size: artifact.total_size,
            transfer_id,
            assigned,
        });
    }
    Ok(images)
}

/// 目标设备：房间主设备槽位（`rooms.device_slot`）+ `room_devices` 子机。
async fn load_targets(state: &AppState, room_id: Uuid) -> AppResult<Vec<StartupTargetReply>> {
    let machines = state
        .devices
        .list_room_machines(room_id)
        .await?
        .unwrap_or_default();
    Ok(machines
        .into_iter()
        .map(|machine| StartupTargetReply {
            device_slot: machine.device_slot,
            serial: machine.serial,
        })
        .collect())
}

/// 向房间所有设备槽位入队 `startup.update {"revision":N}` 并尽力唤醒长连接。
///
/// 尽力而为：没有绑定槽位 / 槽位无存活证书 / 入队失败 / Redis 通知失败都只记 warn，
/// 不影响 PUT 写入结果（agent 的周期 reconcile 是兜底）。
async fn dispatch_startup_update(state: &AppState, room_id: Uuid, revision: i64) {
    let slots = match state.devices.list_room_machines(room_id).await {
        Ok(Some(machines)) => machines
            .into_iter()
            .map(|machine| machine.device_slot)
            .collect::<Vec<_>>(),
        Ok(None) => {
            tracing::warn!(%room_id, "房间不存在，跳过 startup.update 下发");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, %room_id, "读取房间设备失败，跳过 startup.update 下发");
            return;
        }
    };
    if slots.is_empty() {
        tracing::warn!(%room_id, revision, "房间未绑定设备槽位，startup.update 未下发");
        return;
    }

    let payload = json!({ "revision": revision }).to_string();
    for slot in slots {
        match state
            .storage_repo
            .enqueue_downlink_for_slot(&slot, STARTUP_UPDATE_KIND, &payload)
            .await
        {
            Ok(Some(serial)) => {
                if let Err(error) = state
                    .cache
                    .publish(&state.config.schedule_downlink_notify_channel, &serial)
                    .await
                {
                    tracing::warn!(%error, %serial, %slot, "发布设备下行通知失败，startup.update 已入队");
                }
            }
            Ok(None) => {
                tracing::warn!(%room_id, %slot, revision, "设备槽位当前没有存活设备，startup.update 未下发");
            }
            Err(error) => {
                tracing::warn!(%error, %room_id, %slot, revision, "startup.update 入队失败");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qe_db::startup::StartupImageRef;

    fn config_from_json(value: serde_json::Value) -> StartupConfig {
        serde_json::from_value(value).expect("config json")
    }

    #[test]
    fn request_requires_enabled_and_config() {
        let ok: UpdateStartupRequest = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "config": { "auto_pull": true, "auto_start_vm": true, "images": [], "vms": [] }
        }))
        .expect("request");
        assert!(ok.enabled);

        assert!(
            serde_json::from_value::<UpdateStartupRequest>(serde_json::json!({ "config": {} }))
                .is_err()
        );
        assert!(
            serde_json::from_value::<UpdateStartupRequest>(serde_json::json!({ "enabled": true }))
                .is_err()
        );
        // 未知字段（拼写错误）直接 400，而不是被静默忽略。
        assert!(
            serde_json::from_value::<UpdateStartupRequest>(serde_json::json!({
                "enabled": true,
                "config": {},
                "extra": 1
            }))
            .is_err()
        );
    }

    #[test]
    fn validate_config_applies_and_keeps_defaults() {
        let config = validate_config(StartupConfig::default()).expect("valid");
        assert!(config.auto_pull);
        assert!(config.auto_start_vm);
        assert!(config.images.is_empty());
        assert!(config.vms.is_empty());

        let config = validate_config(config_from_json(serde_json::json!({
            "images": [
                { "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b" },
                { "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b" }
            ],
            "vms": [
                { "name": "lab-104" },
                { "name": "lab-105", "vcpus": 4, "memory_mib": 4096, "disk_gb": 40,
                  "base_volume": "real-debian-13-0.1.1", "auto_start": false }
            ]
        })))
        .expect("valid");
        // images / vms 去重；VM 缺省值在 serde 层已补齐。
        assert_eq!(config.images.len(), 1);
        assert_eq!(config.vms.len(), 2);
        assert_eq!(config.vms[0].vcpus, 2);
        assert_eq!(config.vms[1].vcpus, 4);
        assert_eq!(config.vms[1].memory_mib, 4096);
        assert_eq!(config.vms[1].disk_gb, 40);
        assert!(!config.vms[1].auto_start);
    }

    #[test]
    fn validate_config_rejects_out_of_range_fields() {
        // images 上限 8。
        let images = (0..9)
            .map(|_| json!({ "image_id": Uuid::new_v4() }))
            .collect::<Vec<_>>();
        let config = config_from_json(json!({ "images": images }));
        let error = validate_config(config).unwrap_err();
        assert!(error.to_string().contains("config.images"));

        // vms 上限 32。
        let vms = (0..33)
            .map(|index| json!({ "name": format!("lab-{index}") }))
            .collect::<Vec<_>>();
        let error = validate_config(config_from_json(json!({ "vms": vms }))).unwrap_err();
        assert!(error.to_string().contains("config.vms"));

        // vcpus / memory_mib / disk_gb 范围。
        for (field, value) in [("vcpus", 0), ("vcpus", 65), ("memory_mib", 255)] {
            let mut vm = json!({ "name": "lab-1" });
            vm[field] = json!(value);
            let error = validate_config(config_from_json(json!({ "vms": [vm] }))).unwrap_err();
            assert!(error.to_string().contains(field), "field={field}");
        }
        let mut vm = json!({ "name": "lab-1" });
        vm["disk_gb"] = json!(0);
        assert!(validate_config(config_from_json(json!({ "vms": [vm] }))).is_err());

        // 缺省值在范围内（vcpus=2 等），不应被误判。
        assert!(validate_config(config_from_json(json!({ "vms": [{ "name": "lab-1" }] }))).is_ok());
    }

    #[test]
    fn validate_config_rejects_bad_names_and_paths() {
        // name 字符集 / 长度。
        for name in ["", "lab 1", "lab/1", &"a".repeat(65)] {
            let error = validate_config(config_from_json(json!({ "vms": [{ "name": name }] })))
                .unwrap_err();
            assert!(
                error.to_string().contains("config.vms[0].name"),
                "name={name}"
            );
        }

        // base_volume 与 base_path 互斥。
        let error = validate_config(config_from_json(json!({
            "vms": [{ "name": "lab-1", "base_volume": "vol-1", "base_path": "/var/lib/x.qcow2" }]
        })))
        .unwrap_err();
        assert!(error.to_string().contains("base_volume"));

        // base_path 必须绝对路径。
        let error = validate_config(config_from_json(json!({
            "vms": [{ "name": "lab-1", "base_path": "relative/path.qcow2" }]
        })))
        .unwrap_err();
        assert!(error.to_string().contains("base_path"), "error={error}");

        // base_volume 正则（不允许空格/斜杠）。
        assert!(
            validate_config(config_from_json(json!({
                "vms": [{ "name": "lab-1", "base_volume": "bad volume" }]
            })))
            .is_err()
        );

        // 空字符串等价于未提供（不触发互斥）。
        assert!(
            validate_config(config_from_json(json!({
                "vms": [{ "name": "lab-1", "base_volume": "  ", "base_path": "/abs/x.qcow2" }]
            })))
            .is_ok()
        );
    }

    #[test]
    fn validate_config_rejects_bad_gpu() {
        let error = validate_config(config_from_json(json!({
            "vms": [{ "name": "lab-1", "gpu": ["0000:01:00.0", "0000:02:00.0", "0000:03:00.0"] }]
        })))
        .unwrap_err();
        assert!(error.to_string().contains("config.vms[0].gpu"));

        assert!(
            validate_config(config_from_json(json!({
                "vms": [{ "name": "lab-1", "gpu": ["0000:01:00.0", "0000:02:00.1"] }]
            })))
            .is_ok()
        );
        assert!(
            validate_config(config_from_json(json!({
                "vms": [{ "name": "lab-1", "gpu": ["0000:01:00.9"] }]
            })))
            .is_err()
        );
    }

    #[test]
    fn validate_config_accepts_and_normalizes_vm_network() {
        let config = validate_config(config_from_json(json!({
            "vms": [{
                "name": "lab-104",
                "network": {
                    "mode": "isolated",
                    "vlan_id": 104,
                    "subnet": "10.77.104.9/24",
                    "allow_services": [{
                        "cidr": "192.168.8.231",
                        "ports": [8080, "8000-8100", "9000-9000"]
                    }]
                }
            }]
        })))
        .expect("valid network");
        let network = config.vms[0].network.as_ref().expect("network");
        assert_eq!(network.mode, StartupNetworkMode::Isolated);
        assert_eq!(network.vlan_id, Some(104));
        // 子网按前缀归一，IP 不带前缀时归一为 /32。
        assert_eq!(network.subnet.as_deref(), Some("10.77.104.0/24"));
        assert_eq!(network.allow_services[0].cidr.as_str(), "192.168.8.231/32");
        // proto 缺省 tcp；单端口范围归一成数字。
        assert_eq!(network.allow_services[0].proto.as_deref(), Some("tcp"));
        assert_eq!(
            network.allow_services[0].ports,
            vec![
                StartupPort::Port(8080),
                StartupPort::Text("8000-8100".into()),
                StartupPort::Port(9000),
            ]
        );
        assert!(!network.allow_internet);
        assert!(!network.allow_same_room);

        // GET/PUT 回显：落库/响应里的 network 是归一化后的完整形状。
        let value = serde_json::to_value(&config).expect("serialize");
        assert_eq!(value["vms"][0]["network"]["mode"], json!("isolated"));
        assert_eq!(
            value["vms"][0]["network"]["subnet"],
            json!("10.77.104.0/24")
        );
        assert_eq!(
            value["vms"][0]["network"]["allow_services"][0]["proto"],
            json!("tcp")
        );

        // nat + allow_internet 合法。
        assert!(
            validate_config(config_from_json(json!({
                "vms": [{
                    "name": "lab-1",
                    "network": { "mode": "nat", "allow_internet": true }
                }]
            })))
            .is_ok()
        );

        // mode=none 等价不配网（不允许再带网络字段）。
        assert!(
            validate_config(config_from_json(json!({
                "vms": [{ "name": "lab-1", "network": { "mode": "none" } }]
            })))
            .is_ok()
        );
    }

    #[test]
    fn validate_config_rejects_bad_vm_network() {
        let cases = [
            (
                json!({ "mode": "isolated", "vlan_id": 0 }),
                "network.vlan_id",
            ),
            (
                json!({ "mode": "isolated", "vlan_id": 4095 }),
                "network.vlan_id",
            ),
            (
                json!({ "mode": "isolated", "subnet": "10.77.1.0/31" }),
                "network.subnet",
            ),
            (
                json!({ "mode": "isolated", "subnet": "8.8.8.0/24" }),
                "network.subnet",
            ),
            (
                json!({ "mode": "isolated", "subnet": "not-a-cidr" }),
                "network.subnet",
            ),
            (
                json!({ "mode": "isolated", "allow_internet": true }),
                "allow_internet",
            ),
            (json!({ "mode": "none", "vlan_id": 1 }), "mode=none"),
            (
                json!({ "mode": "isolated", "allow_services": [{ "cidr": "192.168.8.1", "ports": [] }] }),
                "network.allow_services[0].ports",
            ),
            (
                json!({ "mode": "isolated", "allow_services": [{ "cidr": "0.0.0.0/0", "ports": [80] }] }),
                "0.0.0.0/0",
            ),
            (
                json!({ "mode": "isolated", "allow_services": [{ "cidr": "192.168.8.1", "proto": "sctp", "ports": [80] }] }),
                "network.allow_services[0].proto",
            ),
            (
                json!({ "mode": "isolated", "allow_services": [{ "cidr": "192.168.8.1", "ports": [0] }] }),
                "network.allow_services[0].ports[0]",
            ),
            (
                json!({ "mode": "isolated", "subnet": "10.77.1.0/24",
                        "allow_services": [{ "cidr": "10.77.1.0/24", "ports": [80] }] }),
                "重叠",
            ),
        ];
        for (network, expected) in cases {
            let error = validate_config(config_from_json(json!({
                "vms": [{ "name": "lab-1", "network": network }]
            })))
            .expect_err("必须拒绝");
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?}, error={error}"
            );
        }
    }

    #[test]
    fn validate_config_rejects_inconsistent_room_network() {
        // 同房间共享一张桥：不同 VM 的网络配置必须一致。
        let error = validate_config(config_from_json(json!({
            "vms": [
                { "name": "lab-1", "network": { "mode": "isolated", "vlan_id": 101 } },
                { "name": "lab-2", "network": { "mode": "nat", "vlan_id": 102 } }
            ]
        })))
        .unwrap_err();
        assert!(
            error.to_string().contains("config.vms[1].network"),
            "{error}"
        );

        // 两台 VM 配置相同则允许。
        assert!(
            validate_config(config_from_json(json!({
                "vms": [
                    { "name": "lab-1", "network": { "mode": "isolated", "vlan_id": 101 } },
                    { "name": "lab-2", "network": { "mode": "isolated", "vlan_id": 101 } }
                ]
            })))
            .is_ok()
        );
    }

    #[test]
    fn validate_config_keeps_legacy_vm_without_network() {
        let config = validate_config(config_from_json(json!({
            "vms": [{ "name": "lab-1" }]
        })))
        .expect("legacy config");
        assert!(config.vms[0].network.is_none());

        // 回显/下发 JSON 里 network 为 null，agent 侧 serde default 可解析。
        let value = serde_json::to_value(&config).expect("serialize");
        assert!(value["vms"][0]["network"].is_null());
    }

    #[test]
    fn reply_serializes_contract_field_names() {
        let image_id = Uuid::new_v4();
        let transfer_id = Uuid::new_v4();
        let reply = StartupReply {
            room_id: Uuid::nil(),
            enabled: true,
            revision: 3,
            config: StartupConfig::default(),
            updated_at: Some(
                chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0)
                    .unwrap()
                    .to_rfc3339(),
            ),
            images: vec![StartupImageReply {
                image_id,
                name: "real-debian-13".to_owned(),
                version: "0.1.1".to_owned(),
                kind: "vm_disk".to_owned(),
                status: "published".to_owned(),
                manifest_digest: Some("sha256:abc".to_owned()),
                total_size: 2_148_073_472,
                transfer_id: Some(transfer_id),
                assigned: true,
            }],
            targets: vec![StartupTargetReply {
                device_slot: "dev-b104".to_owned(),
                serial: Some("0638".to_owned()),
            }],
        };
        let value = serde_json::to_value(&reply).expect("serialize");
        assert_eq!(value["room_id"], json!(Uuid::nil()));
        assert!(value.get("revision").is_some());
        assert_eq!(value["images"][0]["image_id"], json!(image_id));
        assert_eq!(value["images"][0]["manifest_digest"], json!("sha256:abc"));
        assert_eq!(value["images"][0]["transfer_id"], json!(transfer_id));
        assert_eq!(value["images"][0]["assigned"], json!(true));
        assert_eq!(value["targets"][0]["device_slot"], json!("dev-b104"));
        assert_eq!(value["config"]["auto_pull"], json!(true));
        // updated_at 用 RFC3339 字符串（契约示例为 +00:00 形式）。
        assert!(value["updated_at"].as_str().unwrap().contains("+00:00"));

        // 无配置行时 updated_at 为 null、revision=0。
        let empty = StartupReply {
            room_id: Uuid::nil(),
            enabled: false,
            revision: 0,
            config: StartupConfig::default(),
            updated_at: None,
            images: Vec::new(),
            targets: Vec::new(),
        };
        let value = serde_json::to_value(&empty).expect("serialize");
        assert!(value["updated_at"].is_null());
        assert_eq!(value["revision"], json!(0));
        assert_eq!(value["enabled"], json!(false));
    }

    #[test]
    fn image_ref_helper_roundtrips() {
        let image_id = Uuid::new_v4();
        let config = StartupConfig {
            auto_pull: true,
            auto_start_vm: true,
            images: vec![StartupImageRef { image_id }],
            vms: Vec::new(),
        };
        let value = serde_json::to_value(&config).expect("serialize");
        assert_eq!(value["images"][0]["image_id"], json!(image_id));
        assert_eq!(
            serde_json::from_value::<StartupConfig>(value)
                .expect("parse")
                .images[0]
                .image_id,
            image_id
        );
    }
}
