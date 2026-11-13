//! 镜像制作工作台仓储（迁移 0039，契约 `docs/image-studio.md` §2 / §3）。
//!
//! 三块职责：
//! 1. `image_build_sessions`：会话创建/读取/active 列表/幂等状态推进/超时回收；
//! 2. `room_image_defaults`：房间默认底包（+ 可选增量包）读取与 upsert；
//! 3. `ensure_image_assigned`：内部保证「镜像→房间」分配 + download transfer，
//!    复用 `storage::StorageRepo::assign_room` 的 SQL 语义但**不经过 API 权限校验**
//!    （默认底包 PUT / 构建会话 preparing 阶段由服务端自己触发）。
//!
//! 状态推进用 `UPDATE ... WHERE state = ANY($from)` 的 CAS 语义：多副本并发 GET
//! 时只有第一个请求能推进状态，其它请求拿到 `None` 后重新读取即可（幂等）。

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use qe_domain::{
    image_studio::{
        BuildSession, BuildState, NewBuildSession, RoomImageDefault as DomainRoomImageDefault,
    },
    storage::{ArtifactStatus, Transfer},
};

use crate::error::{DbError, DbResult};

/// 终态集合（与 `BuildState::is_terminal` 一致；SQL 片段用）。
const TERMINAL_STATES: [&str; 4] = ["done", "failed", "cancelled", "expired"];

const SESSION_COLUMNS: &str = "id, user_id, room_id, machine_serial, build_vm_name,
     base_image_id, delta_image_id, base_transfer_id, name, version, platform,
     vcpus, memory_mib, disk_gb, description, state, error,
     create_request_id, shutdown_request_id, destroy_request_id, make_request_id, image_id,
     created_at, updated_at, finished_at";

/// `expire_stale_sessions` 的 `UPDATE ... FROM stale` 需要带表别名的列。
const SESSION_COLUMNS_QUALIFIED: &str = "s.id, s.user_id, s.room_id, s.machine_serial,
     s.build_vm_name, s.base_image_id, s.delta_image_id, s.base_transfer_id, s.name,
     s.version, s.platform, s.vcpus, s.memory_mib, s.disk_gb, s.description, s.state,
     s.error, s.create_request_id, s.shutdown_request_id, s.destroy_request_id,
     s.make_request_id, s.image_id, s.created_at, s.updated_at, s.finished_at";

const TRANSFER_COLUMNS: &str = "id, direction, room_id, image_id, manifest_digest,
     status, progress, error, created_at, updated_at";

const ROOM_DEFAULT_COLUMNS: &str = "room_id, base_image_id, delta_image_id, updated_by, updated_at";

fn state_from_str(raw: &str) -> BuildState {
    raw.parse().unwrap_or(BuildState::Failed)
}

fn transfer_status_from_str(raw: &str) -> qe_domain::storage::TransferStatus {
    raw.parse()
        .unwrap_or(qe_domain::storage::TransferStatus::Pending)
}

fn transfer_direction_from_str(raw: &str) -> qe_domain::storage::TransferDirection {
    match raw {
        "upload" => qe_domain::storage::TransferDirection::Upload,
        _ => qe_domain::storage::TransferDirection::Download,
    }
}

// ---------------------------------------------------------------------------
// 行模型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
struct BuildSessionRow {
    id: Uuid,
    user_id: Uuid,
    room_id: Uuid,
    machine_serial: String,
    build_vm_name: String,
    base_image_id: Uuid,
    delta_image_id: Option<Uuid>,
    base_transfer_id: Option<Uuid>,
    name: String,
    version: String,
    platform: String,
    vcpus: i32,
    memory_mib: i32,
    disk_gb: i32,
    description: Option<String>,
    state: String,
    error: Option<String>,
    create_request_id: Option<Uuid>,
    shutdown_request_id: Option<Uuid>,
    destroy_request_id: Option<Uuid>,
    make_request_id: Option<Uuid>,
    image_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

impl From<BuildSessionRow> for BuildSession {
    fn from(row: BuildSessionRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            room_id: row.room_id,
            machine_serial: row.machine_serial,
            build_vm_name: row.build_vm_name,
            base_image_id: row.base_image_id,
            delta_image_id: row.delta_image_id,
            base_transfer_id: row.base_transfer_id,
            name: row.name,
            version: row.version,
            platform: row.platform,
            vcpus: row.vcpus,
            memory_mib: row.memory_mib,
            disk_gb: row.disk_gb,
            description: row.description,
            state: state_from_str(&row.state),
            error: row.error,
            create_request_id: row.create_request_id,
            shutdown_request_id: row.shutdown_request_id,
            destroy_request_id: row.destroy_request_id,
            make_request_id: row.make_request_id,
            image_id: row.image_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            finished_at: row.finished_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct TransferRow {
    id: Uuid,
    direction: String,
    room_id: Uuid,
    image_id: Option<Uuid>,
    manifest_digest: Option<String>,
    status: String,
    progress: i32,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TransferRow> for Transfer {
    fn from(row: TransferRow) -> Self {
        Self {
            id: row.id,
            direction: transfer_direction_from_str(&row.direction),
            room_id: row.room_id,
            image_id: row.image_id,
            manifest_digest: row.manifest_digest,
            status: transfer_status_from_str(&row.status),
            progress: row.progress,
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct RoomDefaultRow {
    room_id: Uuid,
    base_image_id: Uuid,
    delta_image_id: Option<Uuid>,
    updated_by: Option<Uuid>,
    updated_at: DateTime<Utc>,
}

impl From<RoomDefaultRow> for DomainRoomImageDefault {
    fn from(row: RoomDefaultRow) -> Self {
        Self {
            room_id: row.room_id,
            base_image_id: row.base_image_id,
            delta_image_id: row.delta_image_id,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        }
    }
}

/// 幂等状态推进的补丁：`error` 每次覆盖（成功推进传 `None` 会清空旧错误），
/// 其余 id 字段 `Some` 才写入（`COALESCE`）。
#[derive(Debug, Clone)]
pub struct SessionUpdate {
    pub from_states: Vec<BuildState>,
    pub to_state: BuildState,
    pub error: Option<String>,
    pub create_request_id: Option<Uuid>,
    pub shutdown_request_id: Option<Uuid>,
    pub destroy_request_id: Option<Uuid>,
    pub make_request_id: Option<Uuid>,
    pub image_id: Option<Uuid>,
}

impl SessionUpdate {
    /// 从 `from_states` 之一推进到 `to_state`（CAS；不匹配则返回 `None`）。
    pub fn new(from_states: &[BuildState], to_state: BuildState) -> Self {
        Self {
            from_states: from_states.to_vec(),
            to_state,
            error: None,
            create_request_id: None,
            shutdown_request_id: None,
            destroy_request_id: None,
            make_request_id: None,
            image_id: None,
        }
    }

    /// 失败推进：写入 error 文案。
    pub fn failed(from_states: &[BuildState], error: impl Into<String>) -> Self {
        let mut update = Self::new(from_states, BuildState::Failed);
        update.error = Some(error.into());
        update
    }

    pub fn with_create_request(mut self, request_id: Uuid) -> Self {
        self.create_request_id = Some(request_id);
        self
    }

    pub fn with_shutdown_request(mut self, request_id: Uuid) -> Self {
        self.shutdown_request_id = Some(request_id);
        self
    }

    pub fn with_destroy_request(mut self, request_id: Uuid) -> Self {
        self.destroy_request_id = Some(request_id);
        self
    }

    pub fn with_make_request(mut self, request_id: Uuid) -> Self {
        self.make_request_id = Some(request_id);
        self
    }

    pub fn with_image_id(mut self, image_id: Uuid) -> Self {
        self.image_id = Some(image_id);
        self
    }
}

/// `ensure_image_assigned` 的结果：本次（或已存在的）download transfer。
#[derive(Debug, Clone)]
pub struct EnsureAssignedOutcome {
    pub transfer: Transfer,
}

/// 镜像制作工作台 + 房间默认底包仓储。
#[derive(Clone)]
pub struct ImageStudioRepo {
    db: PgPool,
}

impl ImageStudioRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    // ---- 会话 --------------------------------------------------------------

    /// 创建会话（`state = preparing`，`base_transfer_id` 可空）。
    pub async fn create_session(&self, input: &NewBuildSession) -> DbResult<BuildSession> {
        let sql = format!(
            "INSERT INTO image_build_sessions
                (id, user_id, room_id, machine_serial, build_vm_name, base_image_id,
                 delta_image_id, base_transfer_id, name, version, platform,
                 vcpus, memory_mib, disk_gb, description, state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, 'preparing')
             RETURNING {SESSION_COLUMNS}"
        );
        let row = sqlx::query_as::<_, BuildSessionRow>(&sql)
            .bind(input.id)
            .bind(input.user_id)
            .bind(input.room_id)
            .bind(&input.machine_serial)
            .bind(&input.build_vm_name)
            .bind(input.base_image_id)
            .bind(input.delta_image_id)
            .bind(input.base_transfer_id)
            .bind(&input.name)
            .bind(&input.version)
            .bind(&input.platform)
            .bind(input.vcpus)
            .bind(input.memory_mib)
            .bind(input.disk_gb)
            .bind(input.description.as_deref())
            .fetch_one(&self.db)
            .await?;
        Ok(BuildSession::from(row))
    }

    pub async fn get_session(&self, id: Uuid) -> DbResult<Option<BuildSession>> {
        let sql = format!("SELECT {SESSION_COLUMNS} FROM image_build_sessions WHERE id = $1");
        let row = sqlx::query_as::<_, BuildSessionRow>(&sql)
            .bind(id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(BuildSession::from))
    }

    /// 当前用户「进行中的制作」（非终态，最新在前）。
    pub async fn list_active_sessions(&self, user_id: Uuid) -> DbResult<Vec<BuildSession>> {
        let sql = format!(
            "SELECT {SESSION_COLUMNS}
               FROM image_build_sessions
              WHERE user_id = $1
                AND state NOT IN ('done', 'failed', 'cancelled', 'expired')
              ORDER BY created_at DESC, id DESC
              LIMIT 20"
        );
        let rows = sqlx::query_as::<_, BuildSessionRow>(&sql)
            .bind(user_id)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(BuildSession::from).collect())
    }

    /// 当前用户 + 同名同版本同平台的进行中会话（创建前 409 预检用）。
    pub async fn find_active_session_by_name(
        &self,
        user_id: Uuid,
        name: &str,
        version: &str,
        platform: &str,
    ) -> DbResult<Option<BuildSession>> {
        let sql = format!(
            "SELECT {SESSION_COLUMNS}
               FROM image_build_sessions
              WHERE user_id = $1
                AND name = $2
                AND version = $3
                AND platform = $4
                AND state NOT IN ('done', 'failed', 'cancelled', 'expired')
              ORDER BY created_at DESC, id DESC
              LIMIT 1"
        );
        let row = sqlx::query_as::<_, BuildSessionRow>(&sql)
            .bind(user_id)
            .bind(name)
            .bind(version)
            .bind(platform)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(BuildSession::from))
    }

    /// 幂等状态推进：仅当当前状态属于 `update.from_states` 时更新，返回新行；
    /// 状态已被其它请求推进过时返回 `None`（调用方重新读取即可）。
    pub async fn update_session(
        &self,
        id: Uuid,
        update: &SessionUpdate,
    ) -> DbResult<Option<BuildSession>> {
        let from_states: Vec<String> = update
            .from_states
            .iter()
            .map(|state| state.as_str().to_owned())
            .collect();
        let terminal_states: Vec<String> = TERMINAL_STATES
            .iter()
            .map(|state| (*state).to_owned())
            .collect();
        let sql = format!(
            "UPDATE image_build_sessions
                SET state = $2,
                    error = $3,
                    create_request_id = COALESCE($4, create_request_id),
                    shutdown_request_id = COALESCE($5, shutdown_request_id),
                    destroy_request_id = COALESCE($6, destroy_request_id),
                    make_request_id = COALESCE($7, make_request_id),
                    image_id = COALESCE($8, image_id),
                    updated_at = now(),
                    finished_at = CASE
                        WHEN $2 = ANY($9::text[]) THEN COALESCE(finished_at, now())
                        ELSE finished_at
                    END
              WHERE id = $1 AND state = ANY($10::text[])
              RETURNING {SESSION_COLUMNS}"
        );
        let row = sqlx::query_as::<_, BuildSessionRow>(&sql)
            .bind(id)
            .bind(update.to_state.as_str())
            .bind(update.error.as_deref())
            .bind(update.create_request_id)
            .bind(update.shutdown_request_id)
            .bind(update.destroy_request_id)
            .bind(update.make_request_id)
            .bind(update.image_id)
            .bind(terminal_states.as_slice())
            .bind(from_states.as_slice())
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(BuildSession::from))
    }

    /// 超时回收：非终态且 `updated_at < now() - older_than_minutes` → `expired`。
    ///
    /// 返回被标记的会话，调用方负责尽力回收构建 VM（destroy + delete）。
    /// `FOR UPDATE SKIP LOCKED` 保证多副本并发扫描不重复处理同一行。
    pub async fn expire_stale_sessions(
        &self,
        older_than_minutes: i64,
    ) -> DbResult<Vec<BuildSession>> {
        let older_than_minutes = i32::try_from(older_than_minutes).unwrap_or(i32::MAX);
        let sql = format!(
            "WITH stale AS (
                 SELECT id
                   FROM image_build_sessions
                  WHERE state NOT IN ('done', 'failed', 'cancelled', 'expired')
                    AND updated_at < now() - make_interval(mins => $1::int)
                  ORDER BY updated_at
                  LIMIT 20
                    FOR UPDATE SKIP LOCKED
             )
             UPDATE image_build_sessions s
                SET state = 'expired',
                    error = COALESCE(s.error, '制作会话超过 2 小时未完成，已自动回收'),
                    finished_at = now(),
                    updated_at = now()
               FROM stale
              WHERE s.id = stale.id
              RETURNING {SESSION_COLUMNS_QUALIFIED}"
        );
        let rows = sqlx::query_as::<_, BuildSessionRow>(&sql)
            .bind(older_than_minutes)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.into_iter().map(BuildSession::from).collect())
    }

    // ---- 房间默认底包 ------------------------------------------------------

    pub async fn get_room_default(
        &self,
        room_id: Uuid,
    ) -> DbResult<Option<DomainRoomImageDefault>> {
        let sql =
            format!("SELECT {ROOM_DEFAULT_COLUMNS} FROM room_image_defaults WHERE room_id = $1");
        let row = sqlx::query_as::<_, RoomDefaultRow>(&sql)
            .bind(room_id)
            .fetch_optional(&self.db)
            .await?;
        Ok(row.map(DomainRoomImageDefault::from))
    }

    /// upsert 房间默认底包（`updated_by`/`updated_at` 刷新；同事务不校验镜像状态，
    /// 由 API 层在写入前校验 published/可见）。
    pub async fn upsert_room_default(
        &self,
        room_id: Uuid,
        base_image_id: Uuid,
        delta_image_id: Option<Uuid>,
        actor: Uuid,
    ) -> DbResult<DomainRoomImageDefault> {
        let sql = format!(
            "INSERT INTO room_image_defaults
                (room_id, base_image_id, delta_image_id, updated_by, updated_at)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (room_id) DO UPDATE
                SET base_image_id = EXCLUDED.base_image_id,
                    delta_image_id = EXCLUDED.delta_image_id,
                    updated_by = EXCLUDED.updated_by,
                    updated_at = now()
             RETURNING {ROOM_DEFAULT_COLUMNS}"
        );
        let row = sqlx::query_as::<_, RoomDefaultRow>(&sql)
            .bind(room_id)
            .bind(base_image_id)
            .bind(delta_image_id)
            .bind(actor)
            .fetch_one(&self.db)
            .await?;
        Ok(DomainRoomImageDefault::from(row))
    }

    // ---- 内部：分配 + transfer ---------------------------------------------

    /// 保证镜像已分配到房间并存在一条可等待的 download transfer（内部方法，无权限校验）。
    ///
    /// - 镜像不存在 / 已软删除 / 房间不存在 → `Ok(None)`；
    /// - 镜像未 `published` → `Err(Conflict)`（契约只允许分发已发布镜像）；
    /// - 已有 `pending|running|completed` 的 download transfer 时复用最新一条；
    ///   只有 `failed/expired` 或从未创建时才新建 pending transfer。
    pub async fn ensure_image_assigned(
        &self,
        room_id: Uuid,
        image_id: Uuid,
        actor: Uuid,
    ) -> DbResult<Option<EnsureAssignedOutcome>> {
        let mut tx = self.db.begin().await?;

        let image: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT status, manifest_digest
               FROM image_artifacts
              WHERE id = $1 AND deleted_at IS NULL
              FOR UPDATE",
        )
        .bind(image_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((status, manifest_digest)) = image else {
            tx.rollback().await?;
            return Ok(None);
        };
        if status != ArtifactStatus::Published.as_str() {
            tx.rollback().await?;
            return Err(DbError::conflict(format!(
                "镜像尚未发布（status={status}，需为 published），不能分配或拉取"
            )));
        }

        let room_exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM rooms WHERE id = $1")
            .bind(room_id)
            .fetch_optional(&mut *tx)
            .await?;
        if room_exists.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }

        // 与 storage::assign_room / qe_db::startup 相同的幂等 upsert；已存在的分配
        // 保留 channel，只刷新状态/操作人（channel='prod'：房间默认底包走生产通道）。
        sqlx::query(
            "INSERT INTO image_assignments
                 (image_id, room_id, channel, rollout_state, assigned_by, assigned_at, updated_at)
             VALUES ($1, $2, 'prod', 'pending', $3, now(), now())
             ON CONFLICT (room_id, image_id) DO UPDATE
                SET rollout_state = 'pending',
                    assigned_by = EXCLUDED.assigned_by,
                    updated_at = now()",
        )
        .bind(image_id)
        .bind(room_id)
        .bind(actor)
        .execute(&mut *tx)
        .await?;

        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id
               FROM transfers
              WHERE room_id = $1
                AND image_id = $2
                AND direction = 'download'
                AND status IN ('pending', 'running', 'completed')
              ORDER BY created_at DESC, id DESC
              LIMIT 1",
        )
        .bind(room_id)
        .bind(image_id)
        .fetch_optional(&mut *tx)
        .await?;

        let transfer_sql = format!("SELECT {TRANSFER_COLUMNS} FROM transfers WHERE id = $1");
        let row = match existing {
            Some(transfer_id) => {
                sqlx::query_as::<_, TransferRow>(&transfer_sql)
                    .bind(transfer_id)
                    .fetch_one(&mut *tx)
                    .await?
            }
            None => {
                let insert = format!(
                    "INSERT INTO transfers
                         (direction, room_id, image_id, manifest_digest, status, progress)
                     VALUES ('download', $1, $2, $3, 'pending', 0)
                     RETURNING {TRANSFER_COLUMNS}"
                );
                sqlx::query_as::<_, TransferRow>(&insert)
                    .bind(room_id)
                    .bind(image_id)
                    .bind(manifest_digest)
                    .fetch_one(&mut *tx)
                    .await?
            }
        };

        tx.commit().await?;
        Ok(Some(EnsureAssignedOutcome {
            transfer: Transfer::from(row),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qe_domain::storage::TransferStatus;

    fn transfer_row(status: &str, progress: i32) -> TransferRow {
        let now = Utc::now();
        TransferRow {
            id: Uuid::new_v4(),
            direction: "download".to_owned(),
            room_id: Uuid::new_v4(),
            image_id: Some(Uuid::new_v4()),
            manifest_digest: Some("sha256:abc".to_owned()),
            status: status.to_owned(),
            progress,
            error: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn session_update_builders_patch_expected_columns() {
        let ready = BuildState::Ready;
        let update = SessionUpdate::new(&[ready], BuildState::Finishing)
            .with_shutdown_request(Uuid::new_v4());
        assert_eq!(update.from_states, vec![BuildState::Ready]);
        assert_eq!(update.to_state, BuildState::Finishing);
        assert!(update.shutdown_request_id.is_some());
        assert!(update.error.is_none());

        let failed = SessionUpdate::failed(&[BuildState::Preparing], "transfer 失败");
        assert_eq!(failed.to_state, BuildState::Failed);
        assert_eq!(failed.error.as_deref(), Some("transfer 失败"));

        let making = SessionUpdate::new(&[BuildState::Finishing], BuildState::Making)
            .with_make_request(Uuid::new_v4())
            .with_image_id(Uuid::new_v4());
        assert!(making.make_request_id.is_some());
        assert!(making.image_id.is_some());
    }

    #[test]
    fn unknown_state_text_falls_back_without_panic() {
        assert_eq!(state_from_str("bogus"), BuildState::Failed);
        assert_eq!(state_from_str("ready"), BuildState::Ready);
        assert_eq!(transfer_status_from_str("bogus"), TransferStatus::Pending);
        assert_eq!(
            transfer_status_from_str("completed"),
            TransferStatus::Completed
        );
    }

    #[test]
    fn transfer_row_maps_to_domain() {
        let transfer = Transfer::from(transfer_row("running", 42));
        assert_eq!(transfer.status, TransferStatus::Running);
        assert_eq!(transfer.progress, 42);
        assert_eq!(
            transfer.direction,
            qe_domain::storage::TransferDirection::Download
        );
    }

    #[test]
    fn terminal_states_match_domain_helper() {
        for state in [
            BuildState::Done,
            BuildState::Failed,
            BuildState::Cancelled,
            BuildState::Expired,
        ] {
            assert!(TERMINAL_STATES.contains(&state.as_str()));
            assert!(state.is_terminal());
        }
        for state in [
            BuildState::Preparing,
            BuildState::Creating,
            BuildState::Ready,
            BuildState::Finishing,
            BuildState::Making,
        ] {
            assert!(!TERMINAL_STATES.contains(&state.as_str()));
            assert!(!state.is_terminal());
        }
    }
}
