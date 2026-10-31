//! 上行 `transfer.progress` / `transfer.completed` 端到端测试（真实 PostgreSQL）。
//!
//! 走真实 `Dispatcher`（与 `/messages` / WebSocket 隧道同一条路径），
//! 断言 `transfers` 与 `image_assignments.rollout_state` 的契约状态迁移（§16.1）。
//!
//! 默认 `#[ignore]`；用 `scripts/test-integration.sh` 或
//! `TEST_DATABASE_URL=... cargo test -p qe-device-pki --test transfer_flow -- --ignored` 运行。

use qe_device_pki::messaging::{Dispatch, Dispatcher, Message};
use qe_device_pki::transfer::{self, KIND_COMPLETED, KIND_PROGRESS};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

struct Fixture {
    db: PgPool,
    dispatcher: Dispatcher,
    area_id: Uuid,
    room_id: Uuid,
    image_id: Uuid,
    transfer_id: Uuid,
}

async fn setup() -> Option<Fixture> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("skip: 未设置 TEST_DATABASE_URL");
            return None;
        }
    };
    let db = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
    {
        Ok(db) => db,
        Err(e) => {
            eprintln!("skip: 连不上 PostgreSQL（{e}）");
            return None;
        }
    };
    sqlx::migrate!("../../migrations")
        .run(&db)
        .await
        .expect("migrations");

    let suffix = Uuid::new_v4().simple().to_string();
    let area_id: Uuid =
        sqlx::query_scalar("INSERT INTO areas (code, name) VALUES ($1, $2) RETURNING id")
            .bind(format!("transfer-area-{suffix}"))
            .bind("transfer test area")
            .fetch_one(&db)
            .await
            .expect("insert area");
    let room_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rooms (area_id, code, name) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(area_id)
    .bind(format!("transfer-room-{suffix}"))
    .bind("transfer test room")
    .fetch_one(&db)
    .await
    .expect("insert room");
    let image_id: Uuid = sqlx::query_scalar(
        "INSERT INTO image_artifacts
           (name, version, kind, format, platform, manifest_digest, status)
         VALUES ($1, '1.0.0', 'docker', 'docker_archive', 'linux/amd64', $2, 'published')
         RETURNING id",
    )
    .bind(format!("transfer-image-{suffix}"))
    .bind("ab".repeat(32))
    .fetch_one(&db)
    .await
    .expect("insert image");
    sqlx::query(
        "INSERT INTO image_assignments (image_id, room_id, channel, rollout_state)
         VALUES ($1, $2, 'stable', 'pending')",
    )
    .bind(image_id)
    .bind(room_id)
    .execute(&db)
    .await
    .expect("insert assignment");
    let transfer_id: Uuid = sqlx::query_scalar(
        "INSERT INTO transfers (direction, room_id, image_id, manifest_digest, status, progress)
         VALUES ('download', $1, $2, $3, 'pending', 0) RETURNING id",
    )
    .bind(room_id)
    .bind(image_id)
    .bind("ab".repeat(32))
    .fetch_one(&db)
    .await
    .expect("insert transfer");

    let dispatcher = Dispatcher::new(Duration::from_secs(5));
    transfer::install(&dispatcher, db.clone());

    Some(Fixture {
        db,
        dispatcher,
        area_id,
        room_id,
        image_id,
        transfer_id,
    })
}

async fn cleanup(f: &Fixture) {
    let _ = sqlx::query("DELETE FROM transfers WHERE id = $1")
        .bind(f.transfer_id)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM image_assignments WHERE image_id = $1")
        .bind(f.image_id)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM image_artifacts WHERE id = $1")
        .bind(f.image_id)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM rooms WHERE id = $1")
        .bind(f.room_id)
        .execute(&f.db)
        .await;
    let _ = sqlx::query("DELETE FROM areas WHERE id = $1")
        .bind(f.area_id)
        .execute(&f.db)
        .await;
}

fn message(kind: &str, payload: serde_json::Value) -> Message {
    Message {
        serial: "transfer-test-serial".into(),
        slot: "transfer-test-slot".into(),
        seq: 1,
        kind: kind.into(),
        payload: payload.to_string(),
        received_at: chrono::Utc::now(),
    }
}

async fn transfer_state(db: &PgPool, id: Uuid) -> (String, i32, Option<String>) {
    sqlx::query_as("SELECT status, progress, error FROM transfers WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .expect("transfer row")
}

async fn rollout_state(db: &PgPool, image_id: Uuid, room_id: Uuid) -> String {
    sqlx::query_scalar(
        "SELECT rollout_state FROM image_assignments WHERE image_id = $1 AND room_id = $2",
    )
    .bind(image_id)
    .bind(room_id)
    .fetch_one(db)
    .await
    .expect("assignment row")
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn progress_then_completed_success_updates_transfer_and_assignment() {
    let Some(f) = setup().await else { return };

    // progress
    let out = f
        .dispatcher
        .dispatch(message(
            KIND_PROGRESS,
            json!({
                "transfer_id": f.transfer_id,
                "phase": "download",
                "progress": 42,
                "bytes_done": 1024,
                "error": null
            }),
        ))
        .await
        .expect("dispatch progress");
    assert_eq!(out, Dispatch::Processed);
    let (status, progress, error) = transfer_state(&f.db, f.transfer_id).await;
    assert_eq!(status, "running");
    assert_eq!(progress, 42);
    assert!(error.is_none());
    assert_eq!(rollout_state(&f.db, f.image_id, f.room_id).await, "pending");

    // completed ok=true
    let out = f
        .dispatcher
        .dispatch(message(
            KIND_COMPLETED,
            json!({
                "transfer_id": f.transfer_id,
                "ok": true,
                "manifest_digest": "ab".repeat(32),
                "verified": true,
                "imported_ref": "docker:img:1",
                "error": null
            }),
        ))
        .await
        .expect("dispatch completed");
    assert_eq!(out, Dispatch::Processed);
    let (status, progress, error) = transfer_state(&f.db, f.transfer_id).await;
    assert_eq!(status, "completed");
    assert_eq!(progress, 100);
    assert!(error.is_none());
    assert_eq!(
        rollout_state(&f.db, f.image_id, f.room_id).await,
        "deployed"
    );

    cleanup(&f).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn completed_failure_marks_transfer_and_rollout_failed() {
    let Some(f) = setup().await else { return };

    // 先有一个已知进度，失败时不应被清零
    f.dispatcher
        .dispatch(message(
            KIND_PROGRESS,
            json!({ "transfer_id": f.transfer_id, "progress": 60 }),
        ))
        .await
        .expect("dispatch progress");

    let out = f
        .dispatcher
        .dispatch(message(
            KIND_COMPLETED,
            json!({
                "transfer_id": f.transfer_id,
                "ok": false,
                "verified": false,
                "error": "qcow2 magic mismatch"
            }),
        ))
        .await
        .expect("dispatch completed");
    assert_eq!(out, Dispatch::Processed);

    let (status, progress, error) = transfer_state(&f.db, f.transfer_id).await;
    assert_eq!(status, "failed");
    assert_eq!(progress, 60, "失败不能把已知进度清零");
    assert_eq!(error.as_deref(), Some("qcow2 magic mismatch"));
    assert_eq!(rollout_state(&f.db, f.image_id, f.room_id).await, "failed");

    cleanup(&f).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; run via scripts/test-integration.sh"]
async fn unknown_transfer_is_idempotent_noop_and_bad_phase_fails() {
    let Some(f) = setup().await else { return };

    // 已被清理的 transfer：no-op 成功（否则设备 outbox 会被永久卡住）
    let out = f
        .dispatcher
        .dispatch(message(
            KIND_COMPLETED,
            json!({ "transfer_id": Uuid::new_v4(), "ok": true }),
        ))
        .await
        .expect("unknown transfer must not fail");
    assert_eq!(out, Dispatch::Processed);

    // 未知 phase 是客户端 bug：必须冒泡成 5xx，让设备侧尽早暴露
    let err = f
        .dispatcher
        .dispatch(message(
            KIND_PROGRESS,
            json!({ "transfer_id": f.transfer_id, "phase": "install", "progress": 1 }),
        ))
        .await
        .expect_err("unknown phase must fail");
    assert_eq!(err.code(), "bad_request");

    // 失败后状态没被改
    let (status, progress, _) = transfer_state(&f.db, f.transfer_id).await;
    assert_eq!(status, "pending");
    assert_eq!(progress, 0);

    cleanup(&f).await;
}
