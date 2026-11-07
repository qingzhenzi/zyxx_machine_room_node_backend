//! 课表功能集成测试：真实 PostgreSQL + Redis。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 启动依赖并显式运行。
//! 测试使用独立的 `qestudio_test` 数据库，不碰开发数据。

use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::post};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use http_body_util::BodyExt;
use qe_api::{
    api,
    cache::Cache,
    config::{Config, Environment},
    db::{
        Db,
        schedule::{AreaRepo, EventRuleRepo, RoomRepo, ScheduleEventRepo, ScheduleRepo},
        users::UserRepo,
    },
    domain::{
        models::Page,
        schedule::{
            DeliveryStatus, DeliveryTarget, EventFilter, NewArea, NewRoom, NewSchedule,
            ScheduleEvent, ScheduleExceptionKind, TriggerKind, ValidatedEventRule,
            ValidatedExceptionSlot, ValidatedWeeklySlot,
        },
        validation,
    },
    state::AppState,
};
use qe_auth::password::hash_password_async;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test".into())
}

fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

fn test_config() -> Config {
    Config {
        environment: Environment::Development,
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        database_url: database_url(),
        db_max_connections: 4,
        redis_url: redis_url(),
        jwt_secret: "integration-secret-integration-secret-1234".to_owned(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 3600,
        request_timeout_secs: 30,
        max_body_bytes: 2 * 1024 * 1024,
        login_max_attempts: 5,
        login_window_secs: 300,
        login_lock_secs: 900,
        login_ip_max_attempts: 60,
        login_ip_window_secs: 300,
        email_rate_limit_max: 5,
        email_rate_limit_window_secs: 900,
        announcement_cache_secs: 30,
        public_base_url: "http://localhost:5173".to_owned(),
        password_reset_ttl_secs: 1800,
        email_verification_ttl_secs: 86_400,
        email_driver: "log".to_owned(),
        smtp_url: "smtp://127.0.0.1:1025".to_owned(),
        smtp_from: "QEStudio <no-reply@qestudio.local>".to_owned(),
        cors_allowed_origins: vec!["http://localhost:5173".to_owned()],
        run_migrations: false,
        bootstrap_admin_username: None,
        bootstrap_admin_password: None,
        bootstrap_admin_display_name: "Administrator".to_owned(),
        log_format_json: false,
        schedule_engine_enabled: false,
        schedule_tick_secs: 15,
        schedule_horizon_days: 30,
        schedule_rebuild_interval_secs: 3600,
        schedule_pubsub_channel: "qe-schedule:events".to_owned(),
        schedule_downlink_notify_channel: "qe-device-pki:downlink".to_owned(),
        schedule_webhook_timeout_secs: 5,
        schedule_webhook_max_attempts: 5,
        schedule_delivery_batch: 50,
        machine_agent_token: None,
        machine_snapshot_keep: 100,
        device_presence_enabled: false,
        device_presence_tick_secs: 5,
        device_presence_webhook_timeout_secs: 5,
        device_presence_notify_online: false,
        device_presence_global_webhook_url: None,
        device_presence_webhook_max_attempts: 5,
        device_presence_webhook_batch: 20,
        device_presence_retry_base_seconds: 5,
        device_pki_internal_url: "http://127.0.0.1:4442".to_owned(),
        device_pki_internal_token: None,
        public_enroll_endpoint: "https://localhost:4440".to_owned(),
        storage: None,
    }
}

async fn test_state() -> AppState {
    let db = Db::connect(&database_url(), 4)
        .await
        .expect("connect postgres");
    db.migrate().await.expect("run migrations");
    let cache = Cache::connect(&redis_url()).await.expect("connect redis");
    AppState::new(db, cache, test_config())
}

fn unique_code(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

fn time_str(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

fn local_of(instant: DateTime<Utc>, offset_minutes: i32) -> DateTime<Utc> {
    instant + Duration::minutes(offset_minutes as i64)
}

fn utc_date(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
}

async fn cleanup_area_room(
    areas: &AreaRepo,
    rooms: &RoomRepo,
    area_id: uuid::Uuid,
    room_id: uuid::Uuid,
) {
    rooms.delete(room_id).await.expect("cleanup room");
    areas.delete(area_id).await.expect("cleanup area");
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn weekly_template_exceptions_and_occurrences() {
    let state = test_state().await;
    let areas = AreaRepo::new(state.db.pool().clone());
    let rooms = RoomRepo::new(state.db.pool().clone());
    let schedules = ScheduleRepo::new(state.db.pool().clone());

    let area = areas
        .create(&NewArea {
            code: unique_code("area"),
            name: "展开测试区域".to_owned(),
            description: String::new(),
            parent_id: None,
            is_active: true,
            metadata: json!({}),
            timezone: "Asia/Shanghai".to_owned(),
            webhook_url: None,
        })
        .await
        .expect("create area");
    let room = rooms
        .create(&NewRoom {
            area_id: area.id,
            code: unique_code("room"),
            name: "展开测试房间".to_owned(),
            capacity: 30,
            device_slot: None,
            webhook_url: None,
            is_active: true,
            metadata: json!({}),
        })
        .await
        .expect("create room");

    // 选最近的周一作为起点，保证课表落在物化窗口内（默认 30 天）。
    let today = local_of(Utc::now(), 480).date_naive();
    let days_until_monday = (8 - today.weekday().number_from_monday()) % 7;
    let start = today + Duration::days(i64::from(days_until_monday));
    let end = start + Duration::days(13); // 两个完整教学周
    let schedule = schedules
        .create(&NewSchedule {
            room_id: room.id,
            name: "集成测试课表".to_owned(),
            start_date: start,
            end_date: end,
            is_active: true,
            created_by: None,
        })
        .await
        .expect("create schedule");

    let slots = vec![
        ValidatedWeeklySlot {
            day_of_week: 1, // 周一
            start_time: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(9, 40, 0).unwrap(),
            course_name: "数学".to_owned(),
            teacher: "张老师".to_owned(),
            room_id: None,
            note: String::new(),
        },
        ValidatedWeeklySlot {
            day_of_week: 3, // 周三
            start_time: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(11, 40, 0).unwrap(),
            course_name: "英语".to_owned(),
            teacher: "李老师".to_owned(),
            room_id: None,
            note: String::new(),
        },
        ValidatedWeeklySlot {
            day_of_week: 7, // 周日
            start_time: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(15, 40, 0).unwrap(),
            course_name: "体育".to_owned(),
            teacher: "王老师".to_owned(),
            room_id: None,
            note: String::new(),
        },
    ];
    let stored = schedules
        .replace_weekly_slots(schedule.id, &slots)
        .await
        .expect("replace weekly slots")
        .expect("schedule exists");
    assert_eq!(stored.len(), 3);

    let cancel_date = start + Duration::days(2); // 第一个周三停课
    schedules
        .upsert_exception(
            schedule.id,
            cancel_date,
            ScheduleExceptionKind::Cancel,
            "临时停课",
            &[],
        )
        .await
        .expect("cancel exception");
    let replace_date = start + Duration::days(7); // 第二个周一改成 09:00 一节课
    schedules
        .upsert_exception(
            schedule.id,
            replace_date,
            ScheduleExceptionKind::Replace,
            "调课",
            &[ValidatedExceptionSlot {
                start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                end_time: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                course_name: "调课-物理".to_owned(),
                teacher: "赵老师".to_owned(),
                room_id: None,
                note: String::new(),
            }],
        )
        .await
        .expect("replace exception");

    let written = schedules
        .rebuild_occurrences(schedule.id, 30)
        .await
        .expect("rebuild occurrences");
    assert!(written >= 5, "written = {written}");

    let from = utc_date(start);
    let to = utc_date(end) + Duration::days(1);
    let occurrences = schedules
        .occurrences(schedule.id, from, to)
        .await
        .expect("list occurrences");
    assert_eq!(occurrences.len(), 5, "occurrences = {occurrences:#?}");
    assert!(
        occurrences
            .iter()
            .all(|occurrence| occurrence.timezone == "Asia/Shanghai"),
        "occurrences 时区 = {:?}",
        occurrences
            .iter()
            .map(|occurrence| occurrence.timezone.as_str())
            .collect::<Vec<_>>()
    );

    let courses: Vec<&str> = occurrences
        .iter()
        .map(|occurrence| occurrence.course_name.as_str())
        .collect();
    assert!(courses.contains(&"数学"));
    assert!(courses.contains(&"英语"));
    assert!(courses.contains(&"体育"));
    assert!(courses.contains(&"调课-物理"));

    // 停课日不应有实例。
    assert!(
        !occurrences
            .iter()
            .any(|occurrence| occurrence.occurrence_date == cancel_date)
    );
    // 调课日只保留自定义时段（本地 09:00 = UTC 01:00）。
    let replaced: Vec<_> = occurrences
        .iter()
        .filter(|occurrence| occurrence.occurrence_date == replace_date)
        .collect();
    assert_eq!(replaced.len(), 1);
    assert_eq!(
        local_of(replaced[0].start_at, 480)
            .format("%H:%M")
            .to_string(),
        "09:00"
    );

    let room_occurrences = schedules
        .room_occurrences(room.id, from, to)
        .await
        .expect("room occurrences");
    assert_eq!(room_occurrences.len(), 5);

    schedules
        .delete(schedule.id)
        .await
        .expect("delete schedule");
    cleanup_area_room(&areas, &rooms, area.id, room.id).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn engine_triggers_events_and_webhook_delivery() {
    let state = test_state().await;
    let areas = AreaRepo::new(state.db.pool().clone());
    let rooms = RoomRepo::new(state.db.pool().clone());
    let schedules = ScheduleRepo::new(state.db.pool().clone());
    let rules = EventRuleRepo::new(state.db.pool().clone());
    let events = ScheduleEventRepo::new(state.db.pool().clone());

    let (received, hook_url) = spawn_webhook_sink().await;

    let area = areas
        .create(&NewArea {
            code: unique_code("area"),
            name: "引擎测试区域".to_owned(),
            description: String::new(),
            parent_id: None,
            is_active: true,
            metadata: json!({}),
            timezone: "Asia/Shanghai".to_owned(),
            webhook_url: None,
        })
        .await
        .expect("create area");
    let room = rooms
        .create(&NewRoom {
            area_id: area.id,
            code: unique_code("room"),
            name: "引擎测试房间".to_owned(),
            capacity: 20,
            device_slot: None,
            webhook_url: Some(hook_url.clone()),
            is_active: true,
            metadata: json!({}),
        })
        .await
        .expect("create room");

    // 固定用「次日 10:00（本地）」作为上课时间：既在未来、可被 rebuild 物化，
    // 又保证本地日期与 UTC 日期一致（本地 00:00-08:00 的实例会落在 UTC 前一天，
    // 用 utc_date(start_date) 作查询下界会漏查，测试会在临近午夜时随机失败）。
    let next_day = local_of(Utc::now(), 480).date_naive() + Duration::days(1);
    let start_local =
        Utc.from_utc_datetime(&next_day.and_hms_opt(10, 0, 0).expect("10:00 是合法时间"));
    let start_time = start_local.time();
    let end_time = start_time + Duration::minutes(45);
    let start_date = start_local.date_naive();
    let end_date = start_date + Duration::days(2);
    let schedule = schedules
        .create(&NewSchedule {
            room_id: room.id,
            name: "引擎测试课表".to_owned(),
            start_date,
            end_date,
            is_active: true,
            created_by: None,
        })
        .await
        .expect("create schedule");
    schedules
        .replace_weekly_slots(
            schedule.id,
            &[ValidatedWeeklySlot {
                day_of_week: start_date.weekday().number_from_monday() as i16,
                start_time,
                end_time,
                course_name: "引擎数学".to_owned(),
                teacher: "引擎老师".to_owned(),
                room_id: None,
                note: String::new(),
            }],
        )
        .await
        .expect("weekly slots");
    schedules
        .rebuild_occurrences(schedule.id, 30)
        .await
        .expect("rebuild");

    let occurrence = schedules
        .occurrences(
            schedule.id,
            utc_date(start_date),
            utc_date(end_date + Duration::days(1)),
        )
        .await
        .expect("occurrences")
        .into_iter()
        .next()
        .expect("occurrence");

    // 每日定时取「上课后 1 分钟」（本地时间），tick 用对应的真实 UTC 时刻。
    let daily_local = local_of(occurrence.start_at, 480) + Duration::minutes(1);
    let daily_at = time_str(daily_local.time());
    let daily_trigger = occurrence.start_at + Duration::minutes(1);
    rules
        .replace_for_room(
            room.id,
            &[
                ValidatedEventRule {
                    trigger_kind: TriggerKind::OccurrenceStart,
                    offset_minutes: 0,
                    at_time: None,
                    enabled: true,
                    config: json!({ "targets": ["pubsub", "downlink", "webhook"] }),
                },
                ValidatedEventRule {
                    trigger_kind: TriggerKind::OccurrenceStartBefore,
                    offset_minutes: 2,
                    at_time: None,
                    enabled: true,
                    config: json!({ "targets": ["pubsub"] }),
                },
                ValidatedEventRule {
                    trigger_kind: TriggerKind::RoomOccupied,
                    offset_minutes: 0,
                    at_time: None,
                    enabled: true,
                    config: json!({ "targets": ["pubsub"] }),
                },
                ValidatedEventRule {
                    trigger_kind: TriggerKind::DailyTime,
                    offset_minutes: 0,
                    at_time: Some(validation::parse_time(&daily_at).unwrap()),
                    enabled: true,
                    config: json!({ "targets": ["pubsub"] }),
                },
            ],
        )
        .await
        .expect("replace rules");

    // 1) 开始前 2 分钟；2) 开始瞬间（start + occupied）；3) 每日定时。
    qe_api::scheduler::tick_once(
        &state,
        occurrence.start_at - Duration::minutes(2) + Duration::seconds(5),
    )
    .await
    .expect("tick before");
    qe_api::scheduler::tick_once(&state, occurrence.start_at + Duration::seconds(5))
        .await
        .expect("tick start");
    qe_api::scheduler::tick_once(&state, daily_trigger + Duration::seconds(5))
        .await
        .expect("tick daily");

    let page: Page<ScheduleEvent> = events
        .list(
            &EventFilter {
                room_id: Some(room.id),
                ..Default::default()
            },
            1,
            100,
        )
        .await
        .expect("list events");
    let kinds: Vec<&str> = page
        .items
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(kinds.contains(&"occurrence_start"), "kinds = {kinds:?}");
    assert!(kinds.contains(&"occurrence_start_before"));
    assert!(kinds.contains(&"room_occupied"));
    assert!(kinds.contains(&"daily_time"));
    assert!(
        page.items
            .iter()
            .all(|event| event.timezone == "Asia/Shanghai"),
        "事件时区 = {:?}",
        page.items
            .iter()
            .map(|event| event.timezone.as_str())
            .collect::<Vec<_>>()
    );

    let start_event = page
        .items
        .iter()
        .find(|event| event.event_kind == "occurrence_start")
        .expect("start event");
    assert_eq!(start_event.timezone, "Asia/Shanghai");
    assert_eq!(start_event.payload["timezone"], "Asia/Shanghai");
    assert!(
        start_event.payload["start_local"]
            .as_str()
            .is_some_and(|value| value.ends_with("+08:00")),
        "payload = {}",
        start_event.payload
    );
    let deliveries = events
        .deliveries_for(start_event.id)
        .await
        .expect("deliveries");
    assert_eq!(deliveries.len(), 3);
    assert!(
        deliveries
            .iter()
            .any(|delivery| delivery.target == DeliveryTarget::Downlink)
    );

    // 投递一轮：pubsub 成功，downlink 无设备 => skipped，webhook 应打到本地服务。
    for _ in 0..3 {
        qe_api::scheduler::deliver_once(&state)
            .await
            .expect("deliver");
    }
    assert!(!received.lock().await.is_empty(), "webhook 没有收到事件");
    let deliveries = events
        .deliveries_for(start_event.id)
        .await
        .expect("deliveries after");
    assert!(deliveries.iter().all(|delivery| {
        delivery.status == DeliveryStatus::Delivered || delivery.status == DeliveryStatus::Skipped
    }));

    // 重放：投递重置为 pending，再投递一次。
    let reset = events.replay_event(start_event.id).await.expect("replay");
    assert!(reset >= 1);
    qe_api::scheduler::deliver_once(&state)
        .await
        .expect("deliver replay");
    let deliveries = events
        .deliveries_for(start_event.id)
        .await
        .expect("deliveries replayed");
    assert!(deliveries.iter().any(|delivery| {
        delivery.target == DeliveryTarget::Webhook && delivery.status == DeliveryStatus::Delivered
    }));

    schedules
        .delete(schedule.id)
        .await
        .expect("delete schedule");
    cleanup_area_room(&areas, &rooms, area.id, room.id).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn admin_http_api_crud_and_rbac() {
    let state = test_state().await;
    let app = api::router(state.clone());

    let users = UserRepo::new(state.db.pool().clone());
    let password = "ScheduleIntegration@12345";
    let password_hash = hash_password_async(password.to_owned())
        .await
        .expect("hash password");
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let admin_name = format!("sa_{suffix}");
    let viewer_name = format!("sv_{suffix}");
    let admin = users
        .create_with_roles(
            &admin_name,
            None,
            "Schedule Admin",
            &password_hash,
            &["admin".to_owned()],
        )
        .await
        .expect("create admin");
    let viewer = users
        .create_with_roles(
            &viewer_name,
            None,
            "Schedule Viewer",
            &password_hash,
            &["viewer".to_owned()],
        )
        .await
        .expect("create viewer");

    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "username": admin_name, "password": password })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let token = body["access_token"].as_str().expect("access").to_owned();

    let area_code = unique_code("httparea");
    let (status, area) = send_json(
        &app,
        "POST",
        "/api/v1/admin/areas",
        Some(&token),
        Some(json!({ "code": area_code, "name": "HTTP 区域", "timezone": "Asia/Shanghai" })),
    )
    .await;
    assert_eq!(status, 201, "{area}");
    let area_id = area["id"].as_str().expect("area id").to_owned();

    let (status, rooms_page) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/rooms?area_id={area_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{rooms_page}");
    assert_eq!(rooms_page["total"], 0);

    let (status, room) = send_json(
        &app,
        "POST",
        "/api/v1/admin/rooms",
        Some(&token),
        Some(json!({
            "area_id": area_id,
            "code": unique_code("httproom"),
            "name": "HTTP 房间",
            "capacity": 42
        })),
    )
    .await;
    assert_eq!(status, 201, "{room}");
    let room_id = room["id"].as_str().expect("room id").to_owned();

    let today = local_of(Utc::now(), 480).date_naive();
    let tomorrow = today + Duration::days(1);
    let (status, schedule) = send_json(
        &app,
        "POST",
        "/api/v1/admin/schedules",
        Some(&token),
        Some(json!({
            "room_id": room_id,
            "name": "HTTP 课表",
            "start_date": tomorrow.to_string(),
            "end_date": (tomorrow + Duration::days(6)).to_string()
        })),
    )
    .await;
    assert_eq!(status, 201, "{schedule}");
    assert_eq!(schedule["timezone"], "Asia/Shanghai");
    let schedule_id = schedule["id"].as_str().expect("schedule id").to_owned();

    let (status, body) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/schedules/{schedule_id}/weekly-slots"),
        Some(&token),
        Some(json!({
            "slots": [{
                "day_of_week": tomorrow.weekday().number_from_monday(),
                "start_time": "08:00",
                "end_time": "09:40",
                "course_name": "HTTP 数学",
                "teacher": "老师"
            }]
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let slot_count = body["slots"]
        .as_array()
        .or_else(|| body.as_array())
        .map(Vec::len)
        .unwrap_or(0);
    assert_eq!(slot_count, 1, "weekly slots body = {body}");

    let (status, body) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/schedules/{schedule_id}/occurrences?from={tomorrow}&to={tomorrow}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let items = body.as_array().expect("occurrences array");
    assert_eq!(items.len(), 1, "occurrences = {body}");
    assert_eq!(items[0]["timezone"], "Asia/Shanghai");
    assert!(
        items[0]["start_at"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z')),
        "start_at 应为 UTC：{}",
        items[0]["start_at"]
    );
    let start_local = items[0]["start_local"].as_str().expect("start_local");
    assert!(start_local.ends_with("+08:00"), "{start_local}");
    assert!(start_local.contains("08:00:00"), "{start_local}");

    // 区域时区变更：课表继承区域时区并重建实例（Asia/Tokyo +09:00）。
    let (status, body) = send_json(
        &app,
        "PATCH",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&token),
        Some(json!({ "timezone": "Asia/Tokyo" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["timezone"], "Asia/Tokyo");

    let (status, body) = send_json(
        &app,
        "GET",
        &format!(
            "/api/v1/admin/schedules/{schedule_id}/occurrences?from={today}&to={}",
            tomorrow + Duration::days(2)
        ),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let tokyo = body
        .as_array()
        .expect("occurrences array after tz change")
        .iter()
        .find(|item| item["timezone"] == "Asia/Tokyo")
        .expect("重建后的课表应继承 Asia/Tokyo");
    assert!(
        tokyo["start_local"]
            .as_str()
            .is_some_and(|value| value.ends_with("+09:00")),
        "{tokyo}"
    );

    let (status, rules) = send_json(
        &app,
        "PUT",
        &format!("/api/v1/admin/rooms/{room_id}/event-rules"),
        Some(&token),
        Some(json!({
            "rules": [{
                "trigger_kind": "occurrence_start",
                "offset_minutes": 0,
                "config": { "targets": ["pubsub"] }
            }]
        })),
    )
    .await;
    assert_eq!(status, 200, "{rules}");

    let (status, events_page) = send_json(
        &app,
        "GET",
        &format!("/api/v1/admin/schedule-events?room_id={room_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{events_page}");

    // viewer 无写权限。
    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "username": viewer_name, "password": password })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let viewer_token = body["access_token"]
        .as_str()
        .expect("viewer access")
        .to_owned();
    let (status, body) = send_json(
        &app,
        "POST",
        "/api/v1/admin/areas",
        Some(&viewer_token),
        Some(json!({ "code": unique_code("nope"), "name": "不应创建" })),
    )
    .await;
    assert_eq!(status, 403, "{body}");

    // 清理（房间级联课表/实例/规则/事件）。
    let (status, _) = send_json(
        &app,
        "DELETE",
        &format!("/api/v1/admin/rooms/{room_id}"),
        Some(&token),
        None,
    )
    .await;
    assert!(status == 204 || status == 200, "delete room = {status}");
    let (status, _) = send_json(
        &app,
        "DELETE",
        &format!("/api/v1/admin/areas/{area_id}"),
        Some(&token),
        None,
    )
    .await;
    assert!(status == 204 || status == 200, "delete area = {status}");

    sqlx::query("DELETE FROM audit_logs WHERE actor_id = ANY($1)")
        .bind(vec![admin.id, viewer.id])
        .execute(state.db.pool())
        .await
        .expect("cleanup audit");
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![admin.id, viewer.id])
        .execute(state.db.pool())
        .await
        .expect("cleanup users");
}

async fn spawn_webhook_sink() -> (Arc<Mutex<Vec<Value>>>, String) {
    let received = Arc::new(Mutex::new(Vec::new()));
    async fn sink(
        State(received): State<Arc<Mutex<Vec<Value>>>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        received.lock().await.push(body);
        Json(json!({ "ok": true }))
    }
    let app = Router::new()
        .route("/hook", post(sink))
        .with_state(received.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind webhook sink");
    let addr = listener.local_addr().expect("sink addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve webhook sink");
    });
    (received, format!("http://{addr}/hook"))
}

async fn send_json(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    use axum::body::Body;
    use axum::http::Request;
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(body) => Body::from(body.to_string()),
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("response");
    let status = response.status().as_u16();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}
