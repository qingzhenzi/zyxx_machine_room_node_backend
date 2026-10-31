//! v3 设备接入区域 / 房间：设备分页、房间绑定、槽位分配与 bootstrap token。
//!
//! 契约见 `docs/schedule-design.md` 第 11.3 节。设备事实来自 `device_enrollment`
//! / `device_session` / `device_bootstrap_token`（见 `migrations/0006_device_pki.sql`）；
//! 房间绑定的唯一事实来源是 `rooms.device_slot` ↔ `device_enrollment.device_slot`。
//!
//! 约定：
//! - 行模型（`FromRow`）只出现在本文件，映射到 `qe_domain::device`；
//! - bootstrap token 明文只在签发时返回一次，库中只存 SHA-256（与
//!   `qe-device-pki::token` 语义一致，但不依赖该 crate）；
//! - `bind_to_room` / `ensure_room_slot` 在事务内完成，避免并发绑定留下中间态。

use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::{
    device::{
        BootstrapTokenKind, DeviceFilter, DeviceInfo, DeviceOnlineState, DeviceStatus,
        IssuedBootstrapToken,
    },
    models::Page,
    schedule::Room,
};

use crate::error::DbResult;

/// `rooms.device_slot` 的最大长度（与 `qe_domain::validation::normalize_device_slot` 一致）。
const MAX_DEVICE_SLOT_LEN: usize = 128;
/// 槽位冲突时追加的 id 片段长度。
const SHORT_ID_LEN: usize = 8;

const DEVICE_COLUMNS: &str = "de.serial, de.device_slot, de.status,
     ds.state AS device_online, ds.updated_at AS device_last_seen_at,
     (de.downloaded_at IS NOT NULL) AS downloaded,
     de.client_info, de.issued_at, de.activated_at, de.revoked_at,
     r.id AS room_id, r.code AS room_code, r.name AS room_name,
     a.id AS area_id, a.name AS area_name";

const DEVICE_JOINS: &str = "LEFT JOIN rooms r ON r.device_slot = de.device_slot
     LEFT JOIN areas a ON a.id = r.area_id
     LEFT JOIN device_session ds ON ds.serial = de.serial";

/// 过滤条件（注释里的 `$n` 与 `list` 中的参数顺序一致）。
const DEVICE_FILTER: &str = "($1::uuid IS NULL OR a.id = $1)
       AND ($2::uuid IS NULL OR r.id = $2)
       AND ($3::text IS NULL OR de.status = $3)
       AND ($4::boolean IS NULL OR (r.id IS NOT NULL) = $4::boolean)
       AND ($5::text IS NULL OR $5::text = ''
            OR de.serial ILIKE '%' || $5::text || '%'
            OR de.device_slot ILIKE '%' || $5::text || '%'
            OR r.code ILIKE '%' || $5::text || '%'
            OR r.name ILIKE '%' || $5::text || '%')";

/// SHA-256 摘要（库中 `device_bootstrap_token.token_hash` 存该值）。
fn hash_token(token: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

/// 生成 32 字节随机 token，以 hex（64 字符）作为明文返回。
fn random_token() -> String {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    hex::encode(raw)
}

/// 库里的 `device_enrollment.status` → 领域枚举；未知值退化为 `Pending`（与
/// `schedule.rs` 处理库数据异常的风格一致，不 panic）。
fn status_from_str(raw: &str) -> DeviceStatus {
    raw.parse::<DeviceStatus>().unwrap_or(DeviceStatus::Pending)
}

/// `device_session.state` → 领域枚举；没有会话（设备从未上线）时为 `None`。
fn online_from_str(raw: Option<&str>) -> Option<DeviceOnlineState> {
    raw.and_then(|state| state.parse::<DeviceOnlineState>().ok())
}

/// 房间 id 的前 8 位 hex，用作槽位冲突后缀（`Uuid::simple()` 长度足够）。
fn short_id(id: Uuid) -> String {
    id.simple().to_string().chars().take(SHORT_ID_LEN).collect()
}

/// 用房间 code 生成槽位候选：去空白并截断到 128；code 为空时退化为 `room-{id8}`。
fn slot_from_code(code: &str, id: Uuid) -> String {
    let base: String = code
        .trim()
        .chars()
        .take(MAX_DEVICE_SLOT_LEN)
        .collect::<String>();
    if base.is_empty() {
        format!("room-{}", short_id(id))
    } else {
        base
    }
}

/// 冲突时的候选：`{code}-{id8}`，总长仍不超过 128。
fn slot_with_suffix(code: &str, id: Uuid) -> String {
    let short = short_id(id);
    let max_base = MAX_DEVICE_SLOT_LEN.saturating_sub(short.len() + 1);
    let base: String = code.trim().chars().take(max_base).collect::<String>();
    if base.is_empty() {
        format!("room-{short}")
    } else {
        format!("{base}-{short}")
    }
}

#[derive(Debug, Clone, FromRow)]
struct DeviceInfoRow {
    serial: String,
    device_slot: String,
    status: String,
    device_online: Option<String>,
    device_last_seen_at: Option<DateTime<Utc>>,
    downloaded: bool,
    client_info: Option<Value>,
    issued_at: Option<DateTime<Utc>>,
    activated_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    room_id: Option<Uuid>,
    room_code: Option<String>,
    room_name: Option<String>,
    area_id: Option<Uuid>,
    area_name: Option<String>,
}

impl From<DeviceInfoRow> for DeviceInfo {
    fn from(row: DeviceInfoRow) -> Self {
        Self {
            serial: row.serial,
            device_slot: row.device_slot,
            status: status_from_str(&row.status),
            online: online_from_str(row.device_online.as_deref()),
            last_seen_at: row.device_last_seen_at,
            downloaded: row.downloaded,
            client_info: row.client_info,
            issued_at: row.issued_at,
            activated_at: row.activated_at,
            revoked_at: row.revoked_at,
            room_id: row.room_id,
            room_code: row.room_code,
            room_name: row.room_name,
            area_id: row.area_id,
            area_name: row.area_name,
        }
    }
}

#[derive(Clone)]
pub struct DeviceRepo {
    db: PgPool,
}

impl DeviceRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// 设备分页：按 `issued_at DESC NULLS LAST, serial` 排序，支持区域 / 房间 / 状态 /
    /// 是否已绑定 / 关键字（serial、device_slot、房间 code/name）过滤。
    pub async fn list(
        &self,
        filter: &DeviceFilter,
        page: i64,
        page_size: i64,
    ) -> DbResult<Page<DeviceInfo>> {
        let offset = (page - 1) * page_size;
        let status = filter.status.map(DeviceStatus::as_str);

        let count_sql = format!(
            "SELECT count(*)
             FROM device_enrollment de
             {DEVICE_JOINS}
             WHERE {DEVICE_FILTER}"
        );
        let total: i64 = sqlx::query_scalar(&count_sql)
            .bind(filter.area_id)
            .bind(filter.room_id)
            .bind(status)
            .bind(filter.assigned)
            .bind(filter.keyword.as_deref())
            .fetch_one(&self.db)
            .await?;

        let sql = format!(
            "SELECT {DEVICE_COLUMNS}
             FROM device_enrollment de
             {DEVICE_JOINS}
             WHERE {DEVICE_FILTER}
             ORDER BY de.issued_at DESC NULLS LAST, de.serial
             LIMIT $6 OFFSET $7"
        );
        let rows = sqlx::query_as::<_, DeviceInfoRow>(&sql)
            .bind(filter.area_id)
            .bind(filter.room_id)
            .bind(status)
            .bind(filter.assigned)
            .bind(filter.keyword.as_deref())
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.db)
            .await?;

        Ok(Page::new(
            rows.into_iter().map(DeviceInfo::from).collect(),
            total,
            page,
            page_size,
        ))
    }

    pub async fn get(&self, serial: &str) -> DbResult<Option<DeviceInfo>> {
        let sql = format!(
            "SELECT {DEVICE_COLUMNS}
             FROM device_enrollment de
             {DEVICE_JOINS}
             WHERE de.serial = $1"
        );
        let row = sqlx::query_as::<_, DeviceInfoRow>(&sql)
            .bind(serial)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(DeviceInfo::from))
    }

    /// 绑定 / 迁移 / 解绑。`Some(slot)`：若该槽位已绑定到其它房间，事务内先清空旧房间
    /// 再绑定本房间；`None`：清空本房间绑定。房间不存在返回 `None`。
    ///
    /// 槽位唯一性由 `rooms_device_slot_key` 部分唯一索引兜底，并发冲突映射为
    /// `DbError::Conflict`（API 层 409）。
    pub async fn bind_to_room(
        &self,
        room_id: Uuid,
        device_slot: Option<&str>,
    ) -> DbResult<Option<Room>> {
        let mut tx = self.db.begin().await?;

        let exists = sqlx::query_scalar::<_, Uuid>("SELECT id FROM rooms WHERE id = $1")
            .bind(room_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        match device_slot {
            Some(slot) => {
                // 迁移：先释放旧房间的同名槽位，再绑到本房间（顺序不能反，否则撞唯一索引）。
                sqlx::query(
                    "UPDATE rooms SET device_slot = NULL WHERE device_slot = $1 AND id <> $2",
                )
                .bind(slot)
                .bind(room_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query("UPDATE rooms SET device_slot = $1 WHERE id = $2")
                    .bind(slot)
                    .bind(room_id)
                    .execute(&mut *tx)
                    .await?;
            }
            None => {
                sqlx::query("UPDATE rooms SET device_slot = NULL WHERE id = $1")
                    .bind(room_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        let room = crate::schedule::room_on(&mut tx, room_id).await?;
        tx.commit().await?;
        Ok(room)
    }

    /// 确保房间有 `device_slot`：已有则原样返回；否则用房间 `code` 生成候选，
    /// 被占用时追加房间 id 的前 8 位（总长 ≤ 128）。房间不存在返回 `None`。
    pub async fn ensure_room_slot(&self, room_id: Uuid) -> DbResult<Option<String>> {
        let mut tx = self.db.begin().await?;

        let row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT code, device_slot FROM rooms WHERE id = $1 FOR UPDATE",
        )
        .bind(room_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((code, device_slot)) = row else {
            tx.rollback().await?;
            return Ok(None);
        };
        if let Some(slot) = device_slot {
            tx.commit().await?;
            return Ok(Some(slot));
        }

        let mut candidate = slot_from_code(&code, room_id);
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM rooms WHERE device_slot = $1 AND id <> $2
             )",
        )
        .bind(&candidate)
        .bind(room_id)
        .fetch_one(&mut *tx)
        .await?;
        if taken {
            candidate = slot_with_suffix(&code, room_id);
        }

        sqlx::query("UPDATE rooms SET device_slot = $1 WHERE id = $2")
            .bind(&candidate)
            .bind(room_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(candidate))
    }

    /// 生成一次性 bootstrap token：32 字节随机 → hex 明文返回，SHA-256 存
    /// `device_bootstrap_token.token_hash`；TTL 由调用方校验（契约 §11.2 为 60..=86400 秒）。
    pub async fn issue_bootstrap_token(
        &self,
        slot: &str,
        kind: BootstrapTokenKind,
        created_by: &str,
        ttl_secs: i64,
    ) -> DbResult<IssuedBootstrapToken> {
        let token = random_token();
        let token_hash = hash_token(&token);
        let expires_at = Utc::now() + Duration::seconds(ttl_secs);

        sqlx::query(
            "INSERT INTO device_bootstrap_token
                (token_hash, kind, device_slot, created_by, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token_hash.as_slice())
        .bind(kind.as_str())
        .bind(slot)
        .bind(created_by)
        .bind(expires_at)
        .execute(&self.db)
        .await?;

        Ok(IssuedBootstrapToken { token, expires_at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hash_is_sha256_and_deterministic() {
        let expected: [u8; 32] = Sha256::digest(b"deadbeef").into();
        assert_eq!(hash_token("deadbeef"), expected);
        assert_eq!(hash_token("deadbeef"), hash_token("deadbeef"));
        assert_ne!(hash_token("deadbeef"), hash_token("deadbeee"));
    }

    #[test]
    fn random_token_is_64_hex_chars() {
        let token = random_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash_token(&token).len(), 32);
        assert_ne!(token, random_token());
    }

    #[test]
    fn status_and_online_mapping() {
        for status in [
            DeviceStatus::Pending,
            DeviceStatus::Issued,
            DeviceStatus::Active,
            DeviceStatus::Rejected,
            DeviceStatus::Revoked,
        ] {
            assert_eq!(status_from_str(status.as_str()), status);
        }
        // 库数据异常时退回 pending，而不是 panic。
        assert_eq!(status_from_str("unknown"), DeviceStatus::Pending);

        assert_eq!(
            online_from_str(Some("online")),
            Some(DeviceOnlineState::Online)
        );
        assert_eq!(
            online_from_str(Some("offline")),
            Some(DeviceOnlineState::Offline)
        );
        assert_eq!(online_from_str(None), None);
        assert_eq!(online_from_str(Some("busy")), None);
    }

    #[test]
    fn device_info_maps_room_and_area() {
        let id = Uuid::nil();
        let row = DeviceInfoRow {
            serial: "SN-1".to_owned(),
            device_slot: "node-101".to_owned(),
            status: "active".to_owned(),
            device_online: Some("offline".to_owned()),
            device_last_seen_at: None,
            downloaded: true,
            client_info: Some(serde_json::json!({ "os": "linux" })),
            issued_at: None,
            activated_at: None,
            revoked_at: None,
            room_id: Some(id),
            room_code: Some("room-101".to_owned()),
            room_name: Some("101 教室".to_owned()),
            area_id: Some(id),
            area_name: Some("A 栋".to_owned()),
        };
        let info = DeviceInfo::from(row);
        assert_eq!(info.status, DeviceStatus::Active);
        assert_eq!(info.online, Some(DeviceOnlineState::Offline));
        assert!(info.downloaded);
        assert_eq!(info.room_id, Some(id));
        assert_eq!(info.area_name.as_deref(), Some("A 栋"));
    }

    #[test]
    fn slot_candidates_respect_length_limit() {
        let id = Uuid::nil();
        let long_code = "a".repeat(200);

        let plain = slot_from_code(&long_code, id);
        assert_eq!(plain.chars().count(), MAX_DEVICE_SLOT_LEN);
        assert_eq!(slot_from_code("node-101", id), "node-101");
        assert_eq!(
            slot_from_code("  node-101  ", id),
            "node-101",
            "槽位候选应去首尾空白"
        );

        let suffixed = slot_with_suffix(&long_code, id);
        assert!(suffixed.chars().count() <= MAX_DEVICE_SLOT_LEN);
        assert!(suffixed.ends_with(&short_id(id)));
        assert_eq!(
            slot_with_suffix("node-101", id),
            format!("node-101-{}", short_id(id))
        );
    }
}
