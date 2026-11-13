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
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_db::startup::{StartupConfig, StartupVm};

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

/// 配置行（只需要下发用到的列）。
#[derive(Debug, Clone, FromRow)]
struct ProfileRow {
    enabled: bool,
    config: Value,
    revision: i64,
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

    let profile = load_profile(&st.db, room_id).await?;
    let Some(profile) = profile else {
        tracing::debug!(%serial, %slot, %room_id, "startup: 房间无启动配置，返回空配置");
        return Ok(Json(StartupReply::disabled()));
    };
    if !profile.enabled {
        tracing::debug!(%serial, %slot, %room_id, revision = profile.revision, "startup: 配置已禁用，返回空配置");
        return Ok(Json(StartupReply::disabled()));
    }

    let config: StartupConfig = serde_json::from_value(profile.config).map_err(|error| {
        Error::Internal(anyhow::anyhow!(
            "room_startup_profiles.config 不是合法启动配置（room_id={room_id}）: {error}"
        ))
    })?;

    let image_ids: Vec<Uuid> = config.images.iter().map(|image| image.image_id).collect();
    let images = if image_ids.is_empty() {
        Vec::new()
    } else {
        load_published_images(&st.db, room_id, &image_ids).await?
    };

    // 周期 reconcile 每 60s 一次，成功路径用 debug 避免把日志刷满。
    tracing::debug!(
        %serial,
        %slot,
        %room_id,
        revision = profile.revision,
        images = images.len(),
        vms = config.vms.len(),
        "startup 配置下发"
    );

    Ok(Json(StartupReply {
        revision: profile.revision,
        enabled: true,
        auto_pull: config.auto_pull,
        auto_start_vm: config.auto_start_vm,
        images,
        vms: config.vms,
    }))
}

/// 读取房间配置行；无行返回 `None`。
async fn load_profile(db: &PgPool, room_id: Uuid) -> Result<Option<ProfileRow>> {
    let row = sqlx::query_as::<_, ProfileRow>(
        "SELECT enabled, config, revision
           FROM room_startup_profiles
          WHERE room_id = $1",
    )
    .bind(room_id)
    .fetch_optional(db)
    .await?;
    Ok(row)
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
