//! mTLS `GET /agent/startup`：设备拉取房间启动配置（startup provisioning）。
//!
//! 契约见 `docs/startup-provisioning.md` §4.1：
//! - 身份只来自客户端证书：cert serial → `device_enrollment`（`issued` / `active`）
//!   → `device_slot` → `rooms.device_slot`（主设备）或 `room_devices`（子机）→ room；
//!   无存活注册记录 → 401；
//! - 无配置行 / 配置行 `enabled=false` / 槽位未绑定房间：仍返回 **200**，
//!   `revision=0, enabled=false`，`images=[]`（agent 直接退出本轮 reconcile）；
//! - `images[]` 只返回仍处于 `published` 且 `manifest_digest` 非空的镜像，
//!   字段名与下行 `image.pull` payload 保持一致，agent 可直接复用 `ImagePullJob`；
//! - `transfer_id` 取该房间该镜像最新一条 `direction='download'` 的 transfer
//!   （没有则 `null`，agent 可用本地随机 UUID 上报，服务端查不到即忽略）；
//! - `vms[]` 原样透传配置（`base_volume` 可为 `null`，由 agent 侧推导）。
//!
//! 本 crate 不依赖 `qe-api`，因此用 `sqlx::query_as` 直查；配置 JSON 的形状复用
//! `qe_db::startup::StartupConfig`（与写入端同一 serde 定义，避免两端漂移）。

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_db::{
    image_studio::ImageStudioRepo,
    startup::{RoomStartupProfile, RoomStartupRepo, StartupVm},
};
use qe_domain::image_studio::{
    RoomImageDefault, merge_default_image_ids, synthesized_defaults_revision,
};

use crate::error::{Error, Result};
use crate::routes::AppState;
use crate::tls::PeerCert;

/// 镜像分片内容探测端点（与 `image.pull` payload 的 `presign_endpoint` 一致）。
const PRESIGN_ENDPOINT: &str = "/resources/presign";

/// `GET /agent/startup` 响应中的镜像条目（契约 §4.1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupImage {
    pub image_id: Uuid,
    pub transfer_id: Option<Uuid>,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub format: String,
    pub platform: String,
    pub manifest_digest: String,
    pub total_size: i64,
    pub chunk_count: i64,
    pub presign_endpoint: String,
}

/// `GET /agent/startup` 响应（契约 §4.1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupReply {
    pub revision: i64,
    pub enabled: bool,
    pub auto_pull: bool,
    pub auto_start_vm: bool,
    pub images: Vec<StartupImage>,
    pub vms: Vec<StartupVm>,
}

impl StartupReply {
    /// 无配置 / `enabled=false` 时的标准响应：200 + revision=0 + 空列表。
    fn disabled() -> Self {
        Self {
            revision: 0,
            enabled: false,
            auto_pull: true,
            auto_start_vm: true,
            images: Vec::new(),
            vms: Vec::new(),
        }
    }
}

/// 镜像行的查询结果（JOIN 已经把缺列/未发布的镜像过滤掉）。
#[derive(Debug, Clone, FromRow)]
struct StartupImageRow {
    image_id: Uuid,
    transfer_id: Option<Uuid>,
    name: String,
    version: String,
    kind: String,
    format: String,
    platform: String,
    manifest_digest: String,
    total_size: i64,
    chunk_count: i64,
}

impl StartupImageRow {
    fn into_reply(self) -> StartupImage {
        StartupImage {
            image_id: self.image_id,
            transfer_id: self.transfer_id,
            name: self.name,
            version: self.version,
            kind: self.kind,
            format: self.format,
            platform: self.platform,
            manifest_digest: self.manifest_digest,
            total_size: self.total_size,
            chunk_count: self.chunk_count,
            presign_endpoint: PRESIGN_ENDPOINT.to_owned(),
        }
    }
}

/// `GET /agent/startup` —— 设备按证书身份拉取启动配置。
///
/// 401：证书没有 client serial，或 serial 没有 `issued`/`active` 注册记录；
/// 500：数据库错误或库里存了非法配置 JSON（写入端已校验，出现即为数据损坏）。
pub async fn get_startup(State(st): State<AppState>, peer: PeerCert) -> Result<Json<StartupReply>> {
    let serial = peer.serial_hex.clone().ok_or(Error::Unauthorized)?;

    let slot = sqlx::query_scalar::<_, String>(
        "SELECT device_slot FROM device_enrollment
          WHERE serial = $1 AND status IN ('issued','active')
          LIMIT 1",
    )
    .bind(&serial)
    .fetch_optional(&st.db)
    .await?
    .ok_or(Error::Unauthorized)?;

    // 槽位 → 房间：主设备看 rooms.device_slot，子机看 room_devices（迁移 0015）。
    let room_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT r.id
           FROM rooms r
          WHERE r.device_slot = $1
         UNION ALL
         SELECT rd.room_id
           FROM room_devices rd
          WHERE rd.device_slot = $1
          LIMIT 1",
    )
    .bind(&slot)
    .fetch_optional(&st.db)
    .await?;

    let Some(room_id) = room_id else {
        // 槽位还没绑定房间：正常情况（刚注册、还没分配），不是错误。
        tracing::debug!(%serial, %slot, "startup: 设备槽位未绑定房间，返回空配置");
        return Ok(Json(StartupReply::disabled()));
    };

    // 房间默认底包（image-studio.md §3.1）：即使没有 startup profile 也要下发，
    // 保证 agent 连上后自动拉取默认底包/增量包。
    let defaults = ImageStudioRepo::new(st.db.clone())
        .get_room_default(room_id)
        .await?;
    let profile = RoomStartupRepo::new(st.db.clone()).get(room_id).await?;
    let effective = effective_startup(profile.as_ref(), defaults.as_ref());
    if !effective.enabled {
        tracing::debug!(%serial, %slot, %room_id, "startup: 房间无启用配置，返回空配置");
        return Ok(Json(StartupReply::disabled()));
    }

    let images = if effective.image_ids.is_empty() {
        Vec::new()
    } else {
        load_published_images(&st.db, room_id, &effective.image_ids).await?
    };

    // 周期 reconcile 每 60s 一次，成功路径用 debug 避免把日志刷满。
    tracing::debug!(
        %serial,
        %slot,
        %room_id,
        revision = effective.revision,
        images = images.len(),
        vms = effective.vms.len(),
        from_defaults = defaults.is_some(),
        "startup 配置下发"
    );

    Ok(Json(StartupReply {
        revision: effective.revision,
        enabled: true,
        auto_pull: effective.auto_pull,
        auto_start_vm: effective.auto_start_vm,
        images,
        vms: effective.vms,
    }))
}

/// 生效的启动配置：startup profile 与房间默认底包合成后的纯数据（便于单测）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveStartup {
    enabled: bool,
    revision: i64,
    auto_pull: bool,
    auto_start_vm: bool,
    image_ids: Vec<Uuid>,
    vms: Vec<StartupVm>,
}

impl EffectiveStartup {
    fn disabled() -> Self {
        Self {
            enabled: false,
            revision: 0,
            auto_pull: true,
            auto_start_vm: true,
            image_ids: Vec::new(),
            vms: Vec::new(),
        }
    }
}

/// 合成规则（契约 image-studio.md §3.1 + startup-provisioning.md §4.1）：
///
/// 1. 有 startup profile 且 `enabled=true`：revision/auto_*/vms 取 profile，`images`
///    额外并入房间默认底包/增量包（按 image_id 去重）；
/// 2. 有 profile 但 `enabled=false`：尊重显式禁用，返回空配置（不下发默认底包）；
/// 3. 无 profile、有默认底包：`enabled=true`，revision 由默认底包 `updated_at` 派生
///    （恒 >0，默认底包变更时变化），auto 全开、vms 为空；
/// 4. 都没有：空配置（200 + revision=0）。
fn effective_startup(
    profile: Option<&RoomStartupProfile>,
    defaults: Option<&RoomImageDefault>,
) -> EffectiveStartup {
    match profile {
        Some(profile) if profile.enabled => {
            let configured: Vec<Uuid> = profile
                .config
                .images
                .iter()
                .map(|image| image.image_id)
                .collect();
            EffectiveStartup {
                enabled: true,
                revision: profile.revision,
                auto_pull: profile.config.auto_pull,
                auto_start_vm: profile.config.auto_start_vm,
                image_ids: merge_default_image_ids(&configured, defaults),
                vms: profile.config.vms.clone(),
            }
        }
        Some(_) => EffectiveStartup::disabled(),
        None => match defaults {
            Some(defaults) => EffectiveStartup {
                enabled: true,
                revision: synthesized_defaults_revision(defaults.updated_at),
                auto_pull: true,
                auto_start_vm: true,
                image_ids: merge_default_image_ids(&[], Some(defaults)),
                vms: Vec::new(),
            },
            None => EffectiveStartup::disabled(),
        },
    }
}

/// 按配置顺序之外的稳定顺序返回「已发布且 manifest 完整」的镜像条目。
///
/// `transfer_id` 取该房间该镜像最新一条 download transfer；多副本下同一时刻取到的
/// 可能是不同 transfer，但都指向同一 manifest，agent 上报任一都能关联。
async fn load_published_images(
    db: &PgPool,
    room_id: Uuid,
    image_ids: &[Uuid],
) -> Result<Vec<StartupImage>> {
    let rows = sqlx::query_as::<_, StartupImageRow>(
        "SELECT ia.id AS image_id,
                t.id AS transfer_id,
                ia.name,
                ia.version,
                ia.kind,
                ia.format,
                ia.platform,
                COALESCE(ia.manifest_digest, '') AS manifest_digest,
                ia.total_size,
                (SELECT count(*) FROM image_chunks c
                  WHERE c.manifest_digest = ia.manifest_digest) AS chunk_count
           FROM image_artifacts ia
           LEFT JOIN LATERAL (
                SELECT tr.id
                  FROM transfers tr
                 WHERE tr.room_id = $1
                   AND tr.image_id = ia.id
                   AND tr.direction = 'download'
                 ORDER BY tr.created_at DESC, tr.id DESC
                 LIMIT 1
           ) t ON TRUE
          WHERE ia.id = ANY($2::uuid[])
            AND ia.status = 'published'
            AND ia.manifest_digest IS NOT NULL
            AND btrim(ia.manifest_digest) <> ''
          ORDER BY ia.name, ia.version, ia.id",
    )
    .bind(room_id)
    .bind(image_ids)
    .fetch_all(db)
    .await?;

    let images: Vec<StartupImage> = rows.into_iter().map(StartupImageRow::into_reply).collect();
    if images.len() != image_ids.len() {
        // 配置引用但未发布/已删除的镜像不下发（契约 §4.1：只包含已发布且 manifest 完整的）。
        tracing::warn!(
            %room_id,
            configured = image_ids.len(),
            published = images.len(),
            "startup: 部分配置镜像未发布或不存在，已从下发中过滤"
        );
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qe_db::startup::StartupConfig;

    fn image_row() -> StartupImageRow {
        StartupImageRow {
            image_id: Uuid::new_v4(),
            transfer_id: Some(Uuid::new_v4()),
            name: "real-debian-13".to_owned(),
            version: "0.1.1".to_owned(),
            kind: "vm_disk".to_owned(),
            format: "qcow2".to_owned(),
            platform: "debian/amd64".to_owned(),
            manifest_digest: "sha256:abc".to_owned(),
            total_size: 2_148_073_472,
            chunk_count: 65,
        }
    }

    #[test]
    fn disabled_reply_is_200_shape_with_revision_zero() {
        let reply = StartupReply::disabled();
        assert_eq!(reply.revision, 0);
        assert!(!reply.enabled);
        assert!(reply.auto_pull);
        assert!(reply.auto_start_vm);
        assert!(reply.images.is_empty());
        assert!(reply.vms.is_empty());

        let value = serde_json::to_value(&reply).expect("serialize");
        assert_eq!(value["revision"], serde_json::json!(0));
        assert_eq!(value["enabled"], serde_json::json!(false));
        assert_eq!(value["auto_pull"], serde_json::json!(true));
        assert_eq!(value["auto_start_vm"], serde_json::json!(true));
        assert_eq!(value["images"], serde_json::json!([]));
        assert_eq!(value["vms"], serde_json::json!([]));
    }

    #[test]
    fn image_row_maps_to_pull_payload_field_names() {
        let row = image_row();
        let image_id = row.image_id;
        let transfer_id = row.transfer_id;
        let image = row.into_reply();
        assert_eq!(image.image_id, image_id);
        assert_eq!(image.transfer_id, transfer_id);
        assert_eq!(image.presign_endpoint, "/resources/presign");

        let value = serde_json::to_value(&image).expect("serialize");
        // 字段名与 image.pull payload 保持一致（agent 可直接转换）。
        for key in [
            "image_id",
            "transfer_id",
            "name",
            "version",
            "kind",
            "format",
            "platform",
            "manifest_digest",
            "total_size",
            "chunk_count",
            "presign_endpoint",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["manifest_digest"], serde_json::json!("sha256:abc"));
        assert_eq!(value["total_size"], serde_json::json!(2_148_073_472_i64));
    }

    #[test]
    fn image_without_transfer_serializes_null_transfer_id() {
        let mut row = image_row();
        row.transfer_id = None;
        let value = serde_json::to_value(row.into_reply()).expect("serialize");
        assert!(value["transfer_id"].is_null());
    }

    #[test]
    fn reply_echoes_config_vms_and_flags() {
        // 契约 §4.1 的响应：enabled=true 时回显 auto_* 与 vms（base_volume 可为 null）。
        let config: StartupConfig = serde_json::from_value(serde_json::json!({
            "auto_pull": false,
            "auto_start_vm": true,
            "images": [{ "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b" }],
            "vms": [{
                "name": "lab-104",
                "vcpus": 2,
                "memory_mib": 2048,
                "disk_gb": 20,
                "base_volume": null,
                "base_path": null,
                "gpu": [],
                "auto_start": true
            }]
        }))
        .expect("config");

        let reply = StartupReply {
            revision: 3,
            enabled: true,
            auto_pull: config.auto_pull,
            auto_start_vm: config.auto_start_vm,
            images: vec![image_row().into_reply()],
            vms: config.vms,
        };
        assert!(!reply.auto_pull);
        assert!(reply.auto_start_vm);

        let value = serde_json::to_value(&reply).expect("serialize");
        assert_eq!(value["revision"], serde_json::json!(3));
        assert_eq!(value["enabled"], serde_json::json!(true));
        assert_eq!(value["vms"][0]["name"], serde_json::json!("lab-104"));
        assert!(value["vms"][0]["base_volume"].is_null());
        assert_eq!(value["images"].as_array().unwrap().len(), 1);
        // 契约里的下划线字段名不能被写成 camelCase。
        assert!(value.get("autoStartVm").is_none());
        assert!(value.get("auto_start_vm").is_some());
    }

    fn profile(enabled: bool, revision: i64, images: Vec<Uuid>) -> RoomStartupProfile {
        RoomStartupProfile {
            room_id: Uuid::new_v4(),
            enabled,
            config: StartupConfig {
                auto_pull: false,
                auto_start_vm: true,
                images: images
                    .into_iter()
                    .map(|image_id| qe_db::startup::StartupImageRef { image_id })
                    .collect(),
                vms: vec![StartupVm {
                    name: "lab-104".to_owned(),
                    vcpus: 2,
                    memory_mib: 2_048,
                    disk_gb: 20,
                    base_volume: None,
                    base_path: None,
                    gpu: Vec::new(),
                    auto_start: true,
                }],
            },
            revision,
            updated_by: Some(Uuid::new_v4()),
            updated_at: chrono::Utc::now(),
        }
    }

    fn defaults(
        base: Uuid,
        delta: Option<Uuid>,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> RoomImageDefault {
        RoomImageDefault {
            room_id: Uuid::new_v4(),
            base_image_id: base,
            delta_image_id: delta,
            updated_by: None,
            updated_at,
        }
    }

    #[test]
    fn effective_startup_defaults_only_is_enabled_with_positive_revision() {
        let base = Uuid::new_v4();
        let delta = Uuid::new_v4();
        let updated_at = chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let effective = effective_startup(None, Some(&defaults(base, Some(delta), updated_at)));
        assert!(effective.enabled);
        assert!(effective.revision > 0);
        assert_eq!(effective.revision, 1_700_000_000);
        assert!(effective.auto_pull);
        assert!(effective.auto_start_vm);
        assert_eq!(effective.image_ids, vec![base, delta]);
        assert!(effective.vms.is_empty());
    }

    #[test]
    fn effective_startup_merges_profile_images_with_defaults() {
        let configured = Uuid::new_v4();
        let base = configured; // 与默认底包重复 → 去重
        let delta = Uuid::new_v4();
        let profile = profile(true, 5, vec![configured]);
        let effective = effective_startup(
            Some(&profile),
            Some(&defaults(base, Some(delta), chrono::Utc::now())),
        );
        assert!(effective.enabled);
        assert_eq!(effective.revision, 5);
        assert!(!effective.auto_pull);
        assert!(effective.auto_start_vm);
        assert_eq!(effective.image_ids, vec![configured, delta]);
        assert_eq!(effective.vms.len(), 1);
    }

    #[test]
    fn effective_startup_respects_disabled_profile_and_empty_room() {
        // 显式 disabled：即使有默认底包也不下发（尊重用户禁用）。
        let disabled = profile(false, 3, Vec::new());
        let effective = effective_startup(
            Some(&disabled),
            Some(&defaults(Uuid::new_v4(), None, chrono::Utc::now())),
        );
        assert!(!effective.enabled);
        assert_eq!(effective.revision, 0);
        assert!(effective.image_ids.is_empty());

        // 无 profile 无默认底包：空配置。
        let empty = effective_startup(None, None);
        assert!(!empty.enabled);
        assert_eq!(empty.revision, 0);
        assert!(empty.image_ids.is_empty());
    }

    #[test]
    fn default_image_merge_dedupes_preserving_order() {
        let base = Uuid::new_v4();
        let delta = Uuid::new_v4();
        let default = defaults(base, Some(delta), chrono::Utc::now());
        // 配置里已有 base，合并后不重复追加。
        assert_eq!(
            merge_default_image_ids(&[base], Some(&default)),
            vec![base, delta]
        );
        assert_eq!(
            merge_default_image_ids(&[], Some(&default)),
            vec![base, delta]
        );
        assert!(merge_default_image_ids(&[], None).is_empty());
    }

    #[test]
    fn config_from_db_deserializes_with_defaults() {
        // 库里若有历史/手工写入的缺省字段，serde 默认值保证仍可下发。
        let config: StartupConfig = serde_json::from_value(serde_json::json!({
            "images": [],
            "vms": [{ "name": "lab-1" }]
        }))
        .expect("config");
        assert!(config.auto_pull);
        assert!(config.auto_start_vm);
        assert_eq!(config.vms[0].vcpus, 2);
        assert_eq!(config.vms[0].memory_mib, 2_048);
        assert_eq!(config.vms[0].disk_gb, 20);
        assert!(config.vms[0].auto_start);
    }
}
