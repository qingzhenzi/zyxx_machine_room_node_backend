//! 排课工作台：聚合查询与「为房间补默认课表」。
//!
//! 契约见 `docs/design/schedule-board-implementation-contract.md` 第 2 节：
//! - `GET  /admin/schedule-board`（`schedule:read`）：一次返回区域 / 房间 / 课表 / 周模板 /
//!   单日覆盖，供前端排课工作台（A 周网格 / B 资源时间轴 / C 编排矩阵）共享；
//! - `POST /admin/schedules/ensure`（`schedule:manage`）：幂等地为一批房间补默认课表。
//!
//! 性能约定：聚合查询按集合批量取数（areas / rooms / schedules / weekly_slots /
//! exceptions 各 1 条查询，exceptions 的 slots 再 1 条），禁止按房间或课表逐个查（N+1）。

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::extractors::{ApiJson, AuthUser, ClientIp},
    auth::authorization,
    domain::{
        device::{DeviceOnlineState, RoomDevice},
        rbac::permissions,
        schedule::{Area, Room, Schedule, ScheduleException, ScheduleExceptionSlot, WeeklySlot},
        validation,
    },
    error::{AppError, AppResult},
    state::AppState,
};

/// 排课工作台查询的最大日期跨度（`to - from` 的天数，含边界最多 63 天）。
const MAX_BOARD_RANGE_DAYS: i64 = 62;
/// `ensure` 单次最多处理的房间数。
const MAX_ENSURE_ROOMS: usize = 100;
/// `ensure` 未指定名称时使用的默认课表名。
const DEFAULT_ENSURE_NAME: &str = "默认课表";

// ---------------------------------------------------------------------------
// GET /admin/schedule-board：查询参数与响应 DTO
// ---------------------------------------------------------------------------

/// `GET /admin/schedule-board` 查询参数。
///
/// 日期 / UUID 先按字符串解析再手工校验，缺失或格式错误统一返回 `AppError::BadRequest`，
/// 与既有接口的错误格式保持一致。
#[derive(Debug, Deserialize)]
pub struct ScheduleBoardQuery {
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    /// 仅过滤 rooms，不裁剪 areas。
    #[serde(default)]
    pub area_id: Option<String>,
}

/// 区域项：只给前端筛选下拉用。
#[derive(Debug, Serialize)]
pub struct BoardArea {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub timezone: String,
}

/// 房间设备摘要；`None` = 未绑定槽位或无 enrollment。
#[derive(Debug, Serialize)]
pub struct BoardDevice {
    pub serial: String,
    pub status: Option<String>,
    pub online: Option<bool>,
}

/// 房间项：`is_active = false` 的房间不会出现在响应里。
#[derive(Debug, Serialize)]
pub struct BoardRoom {
    pub id: Uuid,
    pub area_id: Uuid,
    pub area_name: String,
    pub code: String,
    pub name: String,
    pub capacity: i32,
    pub device_slot: Option<String>,
    pub is_active: bool,
    pub timezone: String,
    pub device: Option<BoardDevice>,
}

/// 课表项：`is_active = false` 的课表不会出现在响应里，日期不裁剪。
#[derive(Debug, Serialize)]
pub struct BoardSchedule {
    pub id: Uuid,
    pub room_id: Uuid,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_active: bool,
    pub timezone: String,
}

/// 周模板项（调课目标房间用 `room_id` 表示）。
#[derive(Debug, Serialize)]
pub struct BoardWeeklySlot {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub day_of_week: i16,
    pub start_time: String,
    pub end_time: String,
    pub course_name: String,
    pub teacher: String,
    pub room_id: Option<Uuid>,
    pub note: String,
}

/// 单日覆盖的时段项。
#[derive(Debug, Serialize)]
pub struct BoardExceptionSlot {
    pub id: Uuid,
    pub start_time: String,
    pub end_time: String,
    pub course_name: String,
    pub teacher: String,
    pub room_id: Option<Uuid>,
    pub note: String,
}

/// 单日覆盖项：`slots` 按 `start_time` 排序。
#[derive(Debug, Serialize)]
pub struct BoardException {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub exception_date: NaiveDate,
    pub kind: String,
    pub note: String,
    pub slots: Vec<BoardExceptionSlot>,
}

/// `GET /admin/schedule-board` 响应（字段一律 snake_case，缺省值给 `null`/`[]`）。
#[derive(Debug, Serialize)]
pub struct ScheduleBoardResponse {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub areas: Vec<BoardArea>,
    pub rooms: Vec<BoardRoom>,
    pub schedules: Vec<BoardSchedule>,
    pub weekly_slots: Vec<BoardWeeklySlot>,
    pub exceptions: Vec<BoardException>,
}

impl From<Area> for BoardArea {
    fn from(area: Area) -> Self {
        Self {
            id: area.id,
            code: area.code,
            name: area.name,
            timezone: area.timezone,
        }
    }
}

/// 由房间领域摘要构造工作台设备摘要。
///
/// 槽位已分配但没有任何注册记录（`serial = None`）时视为无 enrollment，返回 `None`；
/// `online` 拿不到（无会话 / 无遥测快照）时为 `null`。
fn board_device(device: Option<RoomDevice>) -> Option<BoardDevice> {
    let device = device?;
    let serial = device.serial?;
    Some(BoardDevice {
        serial,
        status: device.status.map(|status| status.as_str().to_owned()),
        online: device
            .online
            .map(|online| online == DeviceOnlineState::Online),
    })
}

impl From<Room> for BoardRoom {
    fn from(room: Room) -> Self {
        Self {
            device: board_device(room.device),
            id: room.id,
            area_id: room.area_id,
            area_name: room.area_name,
            code: room.code,
            name: room.name,
            capacity: room.capacity,
            device_slot: room.device_slot,
            is_active: room.is_active,
            timezone: room.timezone,
        }
    }
}

impl From<Schedule> for BoardSchedule {
    fn from(schedule: Schedule) -> Self {
        Self {
            id: schedule.id,
            room_id: schedule.room_id,
            name: schedule.name,
            start_date: schedule.start_date,
            end_date: schedule.end_date,
            is_active: schedule.is_active,
            timezone: schedule.timezone,
        }
    }
}

impl From<WeeklySlot> for BoardWeeklySlot {
    fn from(slot: WeeklySlot) -> Self {
        Self {
            id: slot.id,
            schedule_id: slot.schedule_id,
            day_of_week: slot.day_of_week,
            start_time: slot.start_time,
            end_time: slot.end_time,
            course_name: slot.course_name,
            teacher: slot.teacher,
            room_id: slot.room_id,
            note: slot.note,
        }
    }
}

impl From<ScheduleExceptionSlot> for BoardExceptionSlot {
    fn from(slot: ScheduleExceptionSlot) -> Self {
        Self {
            id: slot.id,
            start_time: slot.start_time,
            end_time: slot.end_time,
            course_name: slot.course_name,
            teacher: slot.teacher,
            room_id: slot.room_id,
            note: slot.note,
        }
    }
}

impl From<ScheduleException> for BoardException {
    fn from(exception: ScheduleException) -> Self {
        Self {
            id: exception.id,
            schedule_id: exception.schedule_id,
            exception_date: exception.exception_date,
            kind: exception.kind.as_str().to_owned(),
            note: exception.note,
            slots: exception
                .slots
                .into_iter()
                .map(BoardExceptionSlot::from)
                .collect(),
        }
    }
}

/// 解析必填日期参数（`YYYY-MM-DD`）。
fn parse_required_date(raw: Option<&str>, label: &str) -> AppResult<NaiveDate> {
    let value = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::bad_request(format!("缺少查询参数 {label}")))?;
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| AppError::bad_request(format!("{label} 日期格式需为 YYYY-MM-DD")))
}

/// 解析可选 UUID 查询参数：缺失或空白视为 `None`。
fn parse_optional_uuid(raw: Option<&str>, label: &str) -> AppResult<Option<Uuid>> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    Uuid::parse_str(value)
        .map(Some)
        .map_err(|_| AppError::bad_request(format!("{label} 需为合法的 UUID")))
}

/// `GET /admin/schedule-board`：排课工作台聚合数据。
pub async fn board(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    Query(query): Query<ScheduleBoardQuery>,
) -> AppResult<Json<ScheduleBoardResponse>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_READ).await?;

    let from = parse_required_date(query.from.as_deref(), "from")?;
    let to = parse_required_date(query.to.as_deref(), "to")?;
    let area_id = parse_optional_uuid(query.area_id.as_deref(), "area_id")?;
    if to < from {
        return Err(AppError::bad_request("to 不能早于 from"));
    }
    if (to - from).num_days() > MAX_BOARD_RANGE_DAYS {
        return Err(AppError::bad_request(format!(
            "查询跨度不能超过 {MAX_BOARD_RANGE_DAYS} 天"
        )));
    }

    // 5 类数据各 1 条（exceptions 的 slots 再 1 条）查询，禁止 N+1。
    let areas = state.areas.board_areas().await?;
    let rooms = state.rooms.board_rooms(area_id).await?;

    let room_ids: Vec<Uuid> = rooms.iter().map(|room| room.id).collect();
    let schedules = state.schedules.board_schedules(&room_ids).await?;

    let schedule_ids: Vec<Uuid> = schedules.iter().map(|schedule| schedule.id).collect();
    let weekly_slots = state.schedules.board_weekly_slots(&schedule_ids).await?;
    let exceptions = state
        .schedules
        .board_exceptions(&schedule_ids, from, to)
        .await?;

    Ok(Json(ScheduleBoardResponse {
        from,
        to,
        areas: areas.into_iter().map(BoardArea::from).collect(),
        rooms: rooms.into_iter().map(BoardRoom::from).collect(),
        schedules: schedules.into_iter().map(BoardSchedule::from).collect(),
        weekly_slots: weekly_slots
            .into_iter()
            .map(BoardWeeklySlot::from)
            .collect(),
        exceptions: exceptions.into_iter().map(BoardException::from).collect(),
    }))
}

// ---------------------------------------------------------------------------
// POST /admin/schedules/ensure：幂等补默认课表
// ---------------------------------------------------------------------------

/// `POST /admin/schedules/ensure` 请求体。
#[derive(Debug, Deserialize)]
pub struct EnsureSchedulesRequest {
    pub room_ids: Vec<Uuid>,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    /// 缺省 `默认课表`。
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /admin/schedules/ensure` 响应：新建 / 已存在（字段沿用 `GET /schedules` 的课表结构）。
#[derive(Debug, Serialize)]
pub struct EnsureSchedulesResponse {
    pub created: Vec<Schedule>,
    pub existing: Vec<Schedule>,
}

/// `POST /admin/schedules/ensure`：幂等地为一批房间补默认课表。
///
/// 每个房间：已存在启用且与 `[start_date, end_date]` 有交集的课表 → `existing`；
/// 否则新建一份 → `created`。同参数重复调用不产生重复课表。
pub async fn ensure(
    State(state): State<AppState>,
    AuthUser { id }: AuthUser,
    ClientIp(ip): ClientIp,
    ApiJson(request): ApiJson<EnsureSchedulesRequest>,
) -> AppResult<Json<EnsureSchedulesResponse>> {
    authorization::require_permission(&state, id, permissions::SCHEDULE_MANAGE).await?;

    let EnsureSchedulesRequest {
        room_ids: requested_room_ids,
        start_date,
        end_date,
        name,
    } = request;

    // 去重并保持请求顺序；契约要求 1..=100 个房间。
    let mut seen = HashSet::with_capacity(requested_room_ids.len());
    let mut room_ids = Vec::with_capacity(requested_room_ids.len());
    for room_id in requested_room_ids {
        if seen.insert(room_id) {
            room_ids.push(room_id);
        }
    }
    if room_ids.is_empty() {
        return Err(AppError::bad_request("room_ids 不能为空"));
    }
    if room_ids.len() > MAX_ENSURE_ROOMS {
        return Err(AppError::bad_request(format!(
            "单次最多为 {MAX_ENSURE_ROOMS} 个房间补建课表"
        )));
    }

    validation::validate_date_range(start_date, end_date)?;
    let name = match name.as_deref() {
        Some(raw) => validation::normalize_schedule_name(raw)?,
        None => DEFAULT_ENSURE_NAME.to_owned(),
    };

    // 房间存在性 / 启用状态在仓储层事务内校验（返回 404 / 400），并保证幂等。
    let outcome = state
        .schedules
        .ensure_for_rooms(&room_ids, start_date, end_date, &name, Some(id))
        .await?;
    let created = outcome.created.len();
    let existing = outcome.existing.len();

    super::audit::record_or_log(
        &state,
        id,
        "schedule.ensure",
        "schedule",
        None,
        serde_json::json!({
            "created": created,
            "existing": existing,
            "room_ids": room_ids,
        }),
        ip.as_deref(),
    )
    .await;

    Ok(Json(EnsureSchedulesResponse {
        created: outcome.created,
        existing: outcome.existing,
    }))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::domain::device::{DeviceStatus, RoomDevice};

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("测试日期合法")
    }

    fn room_device(
        serial: Option<&str>,
        status: Option<DeviceStatus>,
        online: Option<DeviceOnlineState>,
    ) -> RoomDevice {
        RoomDevice {
            device_slot: "room-501".to_owned(),
            serial: serial.map(str::to_owned),
            status,
            online,
            last_seen_at: None,
            pending_token: false,
        }
    }

    /// 设备摘要：未绑定 / 无 enrollment → `null`；在线状态取不到 → `online = null`。
    #[test]
    fn board_device_follows_null_semantics() {
        assert!(board_device(None).is_none());
        assert!(board_device(Some(room_device(None, None, None))).is_none());

        let online = board_device(Some(room_device(
            Some("2e4d"),
            Some(DeviceStatus::Active),
            Some(DeviceOnlineState::Online),
        )))
        .expect("已注册设备应有摘要");
        assert_eq!(online.serial, "2e4d");
        assert_eq!(online.status.as_deref(), Some("active"));
        assert_eq!(online.online, Some(true));

        let unknown =
            board_device(Some(room_device(Some("2e4d"), None, None))).expect("已注册设备应有摘要");
        assert_eq!(unknown.online, None);
        assert_eq!(unknown.status, None);
    }

    /// 响应字段一律 snake_case；空的 `weekly_slots` / `exceptions` 不能省略成缺键。
    #[test]
    fn board_response_keeps_empty_arrays() {
        let response = ScheduleBoardResponse {
            from: date(2026, 11, 2),
            to: date(2026, 11, 8),
            areas: vec![BoardArea {
                id: Uuid::nil(),
                code: "p3".to_owned(),
                name: "P3 区域".to_owned(),
                timezone: "Asia/Shanghai".to_owned(),
            }],
            rooms: vec![BoardRoom {
                id: Uuid::nil(),
                area_id: Uuid::nil(),
                area_name: "P3 区域".to_owned(),
                code: "p3-room".to_owned(),
                name: "P3 房间".to_owned(),
                capacity: 50,
                device_slot: None,
                is_active: true,
                timezone: "Asia/Shanghai".to_owned(),
                device: None,
            }],
            schedules: Vec::new(),
            weekly_slots: Vec::new(),
            exceptions: Vec::new(),
        };
        let value = serde_json::to_value(&response).expect("响应可序列化");
        assert_eq!(value["from"], "2026-11-02");
        assert_eq!(value["to"], "2026-11-08");
        assert!(value["weekly_slots"].as_array().expect("数组").is_empty());
        assert!(value["exceptions"].as_array().expect("数组").is_empty());
        assert!(value["rooms"][0]["device"].is_null());
        assert_eq!(value["rooms"][0]["area_name"], "P3 区域");
        assert_eq!(value["areas"][0]["code"], "p3");
    }

    /// 周模板 DTO 只保留契约字段（不带 `room_name` / `created_at`）。
    #[test]
    fn weekly_slot_dto_matches_contract_fields() {
        let slot = BoardWeeklySlot::from(WeeklySlot {
            id: Uuid::nil(),
            schedule_id: Uuid::nil(),
            day_of_week: 1,
            start_time: "08:00".to_owned(),
            end_time: "09:40".to_owned(),
            course_name: "高等数学".to_owned(),
            teacher: "张伟".to_owned(),
            room_id: None,
            room_name: Some("别的房间".to_owned()),
            note: String::new(),
            created_at: Utc::now(),
        });
        let value = serde_json::to_value(&slot).expect("周模板可序列化");
        let object = value.as_object().expect("对象");
        assert_eq!(
            object.len(),
            9,
            "契约字段：id/schedule_id/day_of_week/start_time/end_time/course_name/teacher/room_id/note"
        );
        assert!(object.get("room_name").is_none());
        assert!(object.get("created_at").is_none());
        assert!(value["room_id"].is_null());
    }

    /// 单日覆盖 DTO：kind 输出小写，slots 只保留契约字段。
    #[test]
    fn exception_dto_matches_contract_fields() {
        let exception = BoardException::from(ScheduleException {
            id: Uuid::nil(),
            schedule_id: Uuid::nil(),
            exception_date: date(2026, 11, 3),
            kind: crate::domain::schedule::ScheduleExceptionKind::Cancel,
            note: String::new(),
            slots: vec![ScheduleExceptionSlot {
                id: Uuid::nil(),
                exception_id: Uuid::nil(),
                start_time: "08:00".to_owned(),
                end_time: "09:40".to_owned(),
                course_name: "高等数学".to_owned(),
                teacher: "张伟".to_owned(),
                room_id: None,
                room_name: None,
                note: String::new(),
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let value = serde_json::to_value(&exception).expect("单日覆盖可序列化");
        assert_eq!(value["kind"], "cancel");
        assert_eq!(value["exception_date"], "2026-11-03");
        assert_eq!(value["slots"][0]["start_time"], "08:00");
        assert!(value["slots"][0].get("exception_id").is_none());
        assert!(value["slots"][0].get("room_name").is_none());
    }

    /// 查询参数手工解析：缺失 / 格式错误都映射为 400，空白按缺失处理。
    #[test]
    fn query_parsing_maps_invalid_input_to_bad_request() {
        assert_eq!(
            parse_required_date(Some("2026-11-02"), "from").expect("合法日期"),
            date(2026, 11, 2)
        );
        assert!(parse_required_date(None, "from").is_err());
        assert!(parse_required_date(Some("  "), "from").is_err());
        assert!(parse_required_date(Some("2026-13-01"), "from").is_err());
        assert!(
            parse_optional_uuid(None, "area_id")
                .expect("缺省")
                .is_none()
        );
        assert!(
            parse_optional_uuid(Some(" "), "area_id")
                .expect("空白")
                .is_none()
        );
        assert!(parse_optional_uuid(Some("not-a-uuid"), "area_id").is_err());
    }
}
