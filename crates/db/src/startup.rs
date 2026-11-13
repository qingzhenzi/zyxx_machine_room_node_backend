//! 房间启动配置仓储：`room_startup_profiles` 的读写与首次写入时的 transfer 补齐。
//!
//! 契约见 `docs/startup-provisioning.md` §2 / §3：
//! - 每个房间至多一行配置（`room_id` 主键），首次写入 `revision = 1`，之后 `revision + 1`；
//! - `config` 保存**已校验并规范化**的 [`StartupConfig`] JSON，字段名/默认值与契约一致
//!   （`auto_pull` / `auto_start_vm` 缺省 true，`images` / `vms` 缺省空数组）；
//! - 写入时同事务保证每个 `images[].image_id` 在该房间至少有一条
//!   `transfers(direction='download')` 记录（已有则复用最新一条，不重复插入），
//!   供 agent 后续上报 `transfer.progress/completed`；
//! - 同时幂等 upsert `image_assignments(room_id, image_id)`：device-pki
//!   `/resources/presign` 的 `authorize_image` 只放行「已分配到设备所在房间」的镜像，
//!   没有这条分配 agent 自动拉镜像会 403；
//! - `revision` 由 SQL 原子自增，多副本并发写入不会丢更新（最后一次提交的 revision 最大）。
//!
//! 不在此处做业务校验（镜像是否存在/已发布、VM 规格范围等）：那是 API 层的职责，
//! 这里只保证 JSON 能解析成契约形状。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::error::{DbError, DbResult};

/// `images` 数组上限（契约 §2）。
pub const MAX_STARTUP_IMAGES: usize = 8;
/// `vms` 数组上限（契约 §2）。
pub const MAX_STARTUP_VMS: usize = 32;

const fn default_true() -> bool {
    true
}

const fn default_vcpus() -> u32 {
    2
}

const fn default_memory_mib() -> u32 {
    2_048
}

const fn default_disk_gb() -> u32 {
    20
}

/// 启动配置 JSON（契约 §2 `config`）。
///
/// 字段名严格 snake_case；序列化后固定包含全部字段（`base_volume` / `base_path`
/// 为空时写 `null`，与契约示例一致），便于 agent 侧直接按固定形状反序列化。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupConfig {
    /// 本机镜像缺失时是否自动拉取（缺省 true）。
    #[serde(default = "default_true")]
    pub auto_pull: bool,
    /// 是否自动创建/启动 VM（缺省 true；每个 VM 的 `auto_start` 仍需为 true）。
    #[serde(default = "default_true")]
    pub auto_start_vm: bool,
    /// 需要保证在本机的镜像（去重后 0..=8 个）。
    #[serde(default)]
    pub images: Vec<StartupImageRef>,
    /// 需要保证存在并自动启动的 VM（0..=32 个）。
    #[serde(default)]
    pub vms: Vec<StartupVm>,
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            auto_pull: true,
            auto_start_vm: true,
            images: Vec::new(),
            vms: Vec::new(),
        }
    }
}

/// `config.images[]`：只存镜像 id，其余信息在读取时回查 `image_artifacts`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupImageRef {
    pub image_id: Uuid,
}

/// `config.vms[]`：要自动创建/启动的 VM。
///
/// `base_volume` 与 `base_path` 至多一个非空（契约 §2）；两者都为空时由 agent 从
/// 唯一的 `vm_disk` 镜像推导卷名，服务端不代填（契约 §2 末段）。
///
/// 缺省值在反序列化时补齐（`vcpus=2` / `memory_mib=2048` / `disk_gb=20` /
/// `auto_start=true`），因此**落库与下发的 JSON 一定是完整形状**，agent 侧不需要
/// 再处理“字段缺失”。API 层只负责范围校验。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupVm {
    /// `^[A-Za-z0-9._-]{1,64}$`（API 校验）。
    pub name: String,
    /// 1..=64，缺省 2。
    #[serde(default = "default_vcpus")]
    pub vcpus: u32,
    /// 256..=262144，缺省 2048。
    #[serde(default = "default_memory_mib")]
    pub memory_mib: u32,
    /// 1..=2048，缺省 20。
    #[serde(default = "default_disk_gb")]
    pub disk_gb: u32,
    /// libvirt 存储池底包卷名（`<image name>-<version>`）；与 `base_path` 互斥。
    #[serde(default)]
    pub base_volume: Option<String>,
    /// 底包绝对路径；与 `base_volume` 互斥。
    #[serde(default)]
    pub base_path: Option<String>,
    /// PCI 直通 GPU 地址，≤2 个。
    #[serde(default)]
    pub gpu: Vec<String>,
    /// 缺省 true。
    #[serde(default = "default_true")]
    pub auto_start: bool,
}

/// 数据库中的一行启动配置。
#[derive(Debug, Clone, FromRow)]
struct RoomStartupProfileRow {
    room_id: Uuid,
    enabled: bool,
    config: Value,
    revision: i64,
    updated_by: Option<Uuid>,
    updated_at: DateTime<Utc>,
}

impl RoomStartupProfileRow {
    fn into_domain(self) -> DbResult<RoomStartupProfile> {
        let config = serde_json::from_value::<StartupConfig>(self.config).map_err(|error| {
            DbError::internal(format!(
                "room_startup_profiles.config 不是合法启动配置: {error}"
            ))
        })?;
        Ok(RoomStartupProfile {
            room_id: self.room_id,
            enabled: self.enabled,
            config,
            revision: self.revision,
            updated_by: self.updated_by,
            updated_at: self.updated_at,
        })
    }
}

/// 房间启动配置（领域对象）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomStartupProfile {
    pub room_id: Uuid,
    pub enabled: bool,
    pub config: StartupConfig,
    pub revision: i64,
    pub updated_by: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

/// 启动配置仓储。
#[derive(Clone)]
pub struct RoomStartupRepo {
    db: PgPool,
}

const PROFILE_COLUMNS: &str = "room_id, enabled, config, revision, updated_by, updated_at";

impl RoomStartupRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 读取房间启动配置；从未配置过返回 `None`（API 侧对应 revision=0/enabled=false）。
    pub async fn get(&self, room_id: Uuid) -> DbResult<Option<RoomStartupProfile>> {
        let sql = format!("SELECT {PROFILE_COLUMNS} FROM room_startup_profiles WHERE room_id = $1");
        let row = sqlx::query_as::<_, RoomStartupProfileRow>(&sql)
            .bind(room_id)
            .fetch_optional(&self.db)
            .await?;
        row.map(RoomStartupProfileRow::into_domain).transpose()
    }

    /// 写入（或整体替换）房间启动配置：首次 `revision = 1`，之后 `revision + 1`。
    ///
    /// 同一事务内为每个引用镜像补齐：
    /// 1. 一条 `transfers(direction='download', status='pending')`（已有 download transfer
    ///    时复用最新一条，不新建），供 agent 上报 transfer 进度；
    /// 2. 一条 `image_assignments(room_id, image_id)` 幂等 upsert（`channel='prod'`），
    ///    供 device-pki `/resources/presign` 的 `authorize_image` 放行——否则自动拉镜像
    ///    会在预签名阶段 403（镜像未分配到设备所在房间）。已存在的分配不覆盖 channel，
    ///    只刷新 `rollout_state='pending'` / `assigned_by` / `updated_at`。
    ///
    /// 调用方需先确认房间存在（否则外键错误会变成 400）。
    pub async fn upsert(
        &self,
        room_id: Uuid,
        enabled: bool,
        config: &StartupConfig,
        actor: Uuid,
    ) -> DbResult<RoomStartupProfile> {
        let config_json = serde_json::to_value(config)
            .map_err(|error| DbError::internal(format!("序列化启动配置失败: {error}")))?;

        let mut tx = self.db.begin().await?;
        let sql = format!(
            "INSERT INTO room_startup_profiles (room_id, enabled, config, revision, updated_by)
             VALUES ($1, $2, $3, 1, $4)
             ON CONFLICT (room_id) DO UPDATE
             SET enabled = EXCLUDED.enabled,
                 config = EXCLUDED.config,
                 revision = room_startup_profiles.revision + 1,
                 updated_by = EXCLUDED.updated_by,
                 updated_at = now()
             RETURNING {PROFILE_COLUMNS}"
        );
        let row = sqlx::query_as::<_, RoomStartupProfileRow>(&sql)
            .bind(room_id)
            .bind(enabled)
            .bind(config_json)
            .bind(actor)
            .fetch_one(&mut *tx)
            .await?;

        for image in &config.images {
            sqlx::query(
                "INSERT INTO transfers (direction, room_id, image_id, manifest_digest, status, progress)
                 SELECT 'download', $1, $2,
                        (SELECT manifest_digest FROM image_artifacts WHERE id = $2),
                        'pending', 0
                  WHERE NOT EXISTS (
                      SELECT 1 FROM transfers
                       WHERE room_id = $1 AND image_id = $2 AND direction = 'download'
                  )",
            )
            .bind(room_id)
            .bind(image.image_id)
            .execute(&mut *tx)
            .await?;

            // presign 授权（device-pki resource::authorize_image）依赖这张表；
            // 与 assign_room 相同的幂等 upsert。已存在的分配保留原 channel。
            sqlx::query(
                "INSERT INTO image_assignments
                     (image_id, room_id, channel, rollout_state, assigned_by, assigned_at, updated_at)
                 VALUES ($2, $1, 'prod', 'pending', $3, now(), now())
                 ON CONFLICT (room_id, image_id) DO UPDATE
                    SET rollout_state = 'pending',
                        assigned_by = EXCLUDED.assigned_by,
                        updated_at = now()",
            )
            .bind(room_id)
            .bind(image.image_id)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        row.into_domain()
    }

    /// 房间是否已分配该镜像（`image_assignments` 存在即 true）。
    ///
    /// 管理端 GET/PUT 响应用它标出 `assigned`，便于排障「配置里有镜像但 agent 预签名 403」。
    pub async fn is_assigned(&self, room_id: Uuid, image_id: Uuid) -> DbResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM image_assignments
                  WHERE room_id = $1 AND image_id = $2
             )",
        )
        .bind(room_id)
        .bind(image_id)
        .fetch_one(&self.db)
        .await
        .map_err(DbError::from)
    }

    /// 房间 + 镜像的最新一条 download transfer id（GET /agent/startup 下发 `transfer_id` 用）。
    pub async fn latest_download_transfer_id(
        &self,
        room_id: Uuid,
        image_id: Uuid,
    ) -> DbResult<Option<Uuid>> {
        sqlx::query_scalar(
            "SELECT id FROM transfers
              WHERE room_id = $1 AND image_id = $2 AND direction = 'download'
              ORDER BY created_at DESC, id DESC
              LIMIT 1",
        )
        .bind(room_id)
        .bind(image_id)
        .fetch_optional(&self.db)
        .await
        .map_err(DbError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_contract_defaults() {
        let config = StartupConfig::default();
        assert!(config.auto_pull);
        assert!(config.auto_start_vm);
        assert!(config.images.is_empty());
        assert!(config.vms.is_empty());
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "auto_pull": true,
                "auto_start_vm": true,
                "images": [],
                "vms": [],
            })
        );
    }

    #[test]
    fn config_deserializes_contract_example() {
        // 契约 §2 的示例：字段名（snake_case）与可空 base_path 必须原样接受。
        let raw = serde_json::json!({
            "auto_pull": true,
            "auto_start_vm": true,
            "images": [{ "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b" }],
            "vms": [
                {
                    "name": "lab-104",
                    "vcpus": 2,
                    "memory_mib": 2048,
                    "disk_gb": 20,
                    "base_volume": "real-debian-13-0.1.1",
                    "base_path": null,
                    "gpu": [],
                    "auto_start": true
                }
            ]
        });
        let config: StartupConfig = serde_json::from_value(raw).expect("deserialize");
        assert_eq!(config.images.len(), 1);
        assert_eq!(
            config.images[0].image_id,
            Uuid::parse_str("204cb75d-ac1b-4492-b8b3-808d2270070b").unwrap()
        );
        assert_eq!(config.vms.len(), 1);
        let vm = &config.vms[0];
        assert_eq!(vm.name, "lab-104");
        assert_eq!(vm.vcpus, 2);
        assert_eq!(vm.memory_mib, 2048);
        assert_eq!(vm.disk_gb, 20);
        assert_eq!(vm.base_volume.as_deref(), Some("real-debian-13-0.1.1"));
        assert_eq!(vm.base_path, None);
        assert!(vm.auto_start);

        // 序列化后字段名仍然是 snake_case，且空的可选字段写 null（agent 可直接解析）。
        let value = serde_json::to_value(&config).expect("serialize");
        assert!(value["vms"][0].get("base_path").is_some());
        assert!(value["vms"][0]["base_path"].is_null());
        assert!(value.get("auto_start_vm").is_some());
    }

    #[test]
    fn config_defaults_apply_when_fields_missing() {
        // 仅给 vms/images，其它字段走缺省：auto_pull / auto_start_vm = true。
        let config: StartupConfig =
            serde_json::from_value(serde_json::json!({ "vms": [], "images": [] })).expect("parse");
        assert!(config.auto_pull);
        assert!(config.auto_start_vm);

        // VM 的可选字段可以整体缺省：vcpus/memory_mib/disk_gb/auto_start 按契约补默认值，
        // base_volume/base_path/gpu 为空。
        let config: StartupConfig = serde_json::from_value(serde_json::json!({
            "vms": [{ "name": "lab-1" }]
        }))
        .expect("parse");
        let vm = &config.vms[0];
        assert_eq!(vm.vcpus, 2);
        assert_eq!(vm.memory_mib, 2_048);
        assert_eq!(vm.disk_gb, 20);
        assert!(vm.auto_start);
        assert_eq!(vm.base_volume, None);
        assert_eq!(vm.base_path, None);
        assert!(vm.gpu.is_empty());
    }

    #[test]
    fn config_rejects_unknown_fields() {
        // 协作者拼错字段名时必须报错，而不是被静默忽略（避免“保存成功但配置没生效”）。
        assert!(
            serde_json::from_value::<StartupConfig>(serde_json::json!({ "autopull": true }))
                .is_err()
        );
        assert!(
            serde_json::from_value::<StartupConfig>(serde_json::json!({
                "vms": [{ "name": "lab-1", "vcpus": 1, "memory_mib": 256, "disk_gb": 1,
                          "auto_start": true, "gpus": [] }]
            }))
            .is_err()
        );
    }

    #[test]
    fn profile_row_maps_config_and_revision() {
        let room_id = Uuid::new_v4();
        let actor = Uuid::new_v4();
        let now = Utc::now();
        let row = RoomStartupProfileRow {
            room_id,
            enabled: true,
            config: serde_json::json!({
                "auto_pull": false,
                "auto_start_vm": true,
                "images": [{ "image_id": "204cb75d-ac1b-4492-b8b3-808d2270070b" }],
                "vms": []
            }),
            revision: 7,
            updated_by: Some(actor),
            updated_at: now,
        };
        let profile = row.into_domain().expect("profile");
        assert_eq!(profile.room_id, room_id);
        assert!(profile.enabled);
        assert!(!profile.config.auto_pull);
        assert_eq!(profile.revision, 7);
        assert_eq!(profile.updated_by, Some(actor));
        assert_eq!(profile.updated_at, now);
    }

    #[test]
    fn profile_row_rejects_corrupt_config() {
        let row = RoomStartupProfileRow {
            room_id: Uuid::new_v4(),
            enabled: true,
            config: serde_json::json!({ "auto_pull": "yes" }),
            revision: 1,
            updated_by: None,
            updated_at: Utc::now(),
        };
        assert!(matches!(row.into_domain(), Err(DbError::Internal(_))));
    }

    #[test]
    fn limits_match_contract() {
        assert_eq!(MAX_STARTUP_IMAGES, 8);
        assert_eq!(MAX_STARTUP_VMS, 32);
    }
}
