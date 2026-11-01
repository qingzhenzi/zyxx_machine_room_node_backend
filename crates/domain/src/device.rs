//! v3 设备接入区域/房间的领域模型。
//!
//! 契约见 `docs/schedule-design.md` 第 11.2 节。设备本身仍由 `device_enrollment`
//! 管理，房间只持有设备槽位（`rooms.device_slot`）；本模块只提供只读摘要与查询
//! 条件，不承担绑定/状态变更逻辑（那些在 `qe-db`）。

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 设备注册状态，对应 `device_enrollment.status`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    Pending,
    Issued,
    Active,
    Rejected,
    Revoked,
}

impl DeviceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Issued => "issued",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
        }
    }
}

impl fmt::Display for DeviceStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DeviceStatus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "issued" => Ok(Self::Issued),
            "active" => Ok(Self::Active),
            "rejected" => Ok(Self::Rejected),
            "revoked" => Ok(Self::Revoked),
            _ => Err(()),
        }
    }
}

/// 设备在线状态，对应 `device_session.state`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceOnlineState {
    Online,
    Offline,
}

impl DeviceOnlineState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
        }
    }
}

impl fmt::Display for DeviceOnlineState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DeviceOnlineState {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "online" => Ok(Self::Online),
            "offline" => Ok(Self::Offline),
            _ => Err(()),
        }
    }
}

/// 房间上展示的设备摘要（`Room.device`）。`None` 表示房间未绑定设备槽位。
#[derive(Debug, Clone, Serialize)]
pub struct RoomDevice {
    pub device_slot: String,
    pub serial: Option<String>, // None = 已分配槽位但尚未注册
    pub status: Option<DeviceStatus>,
    pub online: Option<DeviceOnlineState>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub pending_token: bool, // 存在未使用且未过期的注册 token
}

/// 房间内的机器条目（`GET /admin/rooms/{id}/devices`）。
///
/// 主设备来自 `rooms.device_slot`，子设备来自 `room_devices`；
/// `serial` / `status` 为 `None` 表示槽位已分配但设备尚未注册。
#[derive(Debug, Clone, Serialize)]
pub struct RoomMachine {
    pub device_slot: String,
    /// `true` = 房间主设备（保留既有下发口径），`false` = 子机。
    pub is_primary: bool,
    pub label: Option<String>,
    pub serial: Option<String>,
    pub status: Option<DeviceStatus>,
    pub online: Option<DeviceOnlineState>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub pending_token: bool,
    pub created_at: Option<DateTime<Utc>>,
}

/// 设备列表条目（`GET /admin/devices`）。
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub serial: String,
    pub device_slot: String,
    pub status: DeviceStatus,
    pub online: Option<DeviceOnlineState>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub downloaded: bool,
    pub client_info: Option<serde_json::Value>,
    pub issued_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub room_id: Option<Uuid>,
    pub room_code: Option<String>,
    pub room_name: Option<String>,
    pub area_id: Option<Uuid>,
    pub area_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceFilter {
    pub area_id: Option<Uuid>,
    pub room_id: Option<Uuid>,
    pub status: Option<DeviceStatus>,
    pub assigned: Option<bool>, // true=已绑定房间，false=未绑定
    pub keyword: Option<String>,
}

/// 设备注册 token 类型：`auto` 自动注册 / `approval` 需审批。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapTokenKind {
    Auto,
    Approval,
}

impl BootstrapTokenKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Approval => "approval",
        }
    }
}

impl fmt::Display for BootstrapTokenKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BootstrapTokenKind {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "approval" => Ok(Self::Approval),
            _ => Err(()),
        }
    }
}

/// 已签发的注册 token：明文只返回一次，库中只存 SHA-256。
#[derive(Debug, Clone)]
pub struct IssuedBootstrapToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_status_json_and_from_str_round_trip() {
        for status in [
            DeviceStatus::Pending,
            DeviceStatus::Issued,
            DeviceStatus::Active,
            DeviceStatus::Rejected,
            DeviceStatus::Revoked,
        ] {
            let json = serde_json::to_string(&status).expect("序列化");
            assert_eq!(json, format!("\"{}\"", status.as_str()));
            let parsed: DeviceStatus = serde_json::from_str(&json).expect("反序列化");
            assert_eq!(parsed, status);
            assert_eq!(status.as_str().parse::<DeviceStatus>(), Ok(status));
        }
        // 大小写与首尾空白容错；非法值返回 Err(())。
        assert_eq!(
            " Pending ".parse::<DeviceStatus>(),
            Ok(DeviceStatus::Pending)
        );
        assert_eq!("UNKNOWN".parse::<DeviceStatus>(), Err(()));
        assert_eq!("".parse::<DeviceStatus>(), Err(()));
    }

    #[test]
    fn device_online_state_json_and_from_str_round_trip() {
        for state in [DeviceOnlineState::Online, DeviceOnlineState::Offline] {
            let json = serde_json::to_string(&state).expect("序列化");
            assert_eq!(json, format!("\"{}\"", state.as_str()));
            let parsed: DeviceOnlineState = serde_json::from_str(&json).expect("反序列化");
            assert_eq!(parsed, state);
            assert_eq!(state.as_str().parse::<DeviceOnlineState>(), Ok(state));
        }
        assert_eq!(
            " Online ".parse::<DeviceOnlineState>(),
            Ok(DeviceOnlineState::Online)
        );
        assert_eq!("busy".parse::<DeviceOnlineState>(), Err(()));
    }

    #[test]
    fn bootstrap_token_kind_json_and_from_str_round_trip() {
        for kind in [BootstrapTokenKind::Auto, BootstrapTokenKind::Approval] {
            let json = serde_json::to_string(&kind).expect("序列化");
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            let parsed: BootstrapTokenKind = serde_json::from_str(&json).expect("反序列化");
            assert_eq!(parsed, kind);
            assert_eq!(kind.as_str().parse::<BootstrapTokenKind>(), Ok(kind));
        }
        assert_eq!(
            "Approval".parse::<BootstrapTokenKind>(),
            Ok(BootstrapTokenKind::Approval)
        );
        assert_eq!("manual".parse::<BootstrapTokenKind>(), Err(()));
    }

    #[test]
    fn room_device_serializes_snake_case_fields() {
        let device = RoomDevice {
            device_slot: "node-101".to_owned(),
            serial: None,
            status: None,
            online: None,
            last_seen_at: None,
            pending_token: true,
        };
        assert_eq!(
            serde_json::to_value(&device).expect("序列化"),
            serde_json::json!({
                "device_slot": "node-101",
                "serial": null,
                "status": null,
                "online": null,
                "last_seen_at": null,
                "pending_token": true,
            })
        );
    }

    #[test]
    fn device_filter_defaults_to_unfiltered() {
        let filter = DeviceFilter::default();
        assert!(filter.area_id.is_none());
        assert!(filter.room_id.is_none());
        assert!(filter.status.is_none());
        assert!(filter.assigned.is_none());
        assert!(filter.keyword.is_none());
    }
}
