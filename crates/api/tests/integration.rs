//! 真实 PostgreSQL + Redis 集成测试。
//!
//! 默认标记为 ignored；使用 `scripts/test-integration.sh` 启动依赖并显式运行。
//! 测试使用独立的 `qestudio_test` 数据库，不碰开发数据。

use qe_api::{
    cache::Cache,
    db::{Db, audit::AuditLogRepo, security::SecurityRepo, users::UserRepo},
    domain::validation,
};
use qe_auth::password::hash_password_async;

fn database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://qestudio:qestudio@127.0.0.1:5432/qestudio_test".into())
}

fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis; run via scripts/test-integration.sh"]
async fn audit_log_and_login_lock_work_against_real_services() {
    let db = Db::connect(&database_url(), 2)
        .await
        .expect("connect postgres");
    db.migrate().await.expect("run migrations");
    let cache = Cache::connect(&redis_url()).await.expect("connect redis");

    let username = format!("it_{}", uuid::Uuid::new_v4().simple());
    let users = UserRepo::new(db.pool().clone());
    let hash = hash_password_async("Integration@12345".to_owned())
        .await
        .expect("hash password");
    let user = users
        .create_with_roles(
            &username,
            None,
            "Integration User",
            &hash,
            &["viewer".to_owned()],
        )
        .await
        .expect("create user");

    let audit = AuditLogRepo::new(db.pool().clone());
    let resource_id = user.id.to_string();
    audit
        .record(
            user.id,
            "integration.test",
            "user",
            Some(&resource_id),
            serde_json::json!({ "source": "integration" }),
            Some("127.0.0.1"),
        )
        .await
        .expect("write audit");

    let page = audit
        .list(Some("integration.test"), Some("user"), 1, 10)
        .await
        .expect("read audit");
    assert!(
        page.items.iter().any(
            |entry| entry.actor_id == Some(user.id) && entry.ip.as_deref() == Some("127.0.0.1")
        )
    );

    cache
        .clear_login_failures(&username)
        .await
        .expect("clear lock");
    for attempt in 0..3 {
        let count = cache
            .record_login_failure(&username, 60, 3, 30)
            .await
            .expect("record failure");
        assert_eq!(count, attempt + 1);
    }
    assert!(
        cache
            .login_retry_after(&username)
            .await
            .expect("ttl")
            .is_some()
    );
    cache
        .clear_login_failures(&username)
        .await
        .expect("clear lock");

    let version = cache
        .bump_announcement_version()
        .await
        .expect("bump version");
    assert!(version >= 1);
    let cache_key = format!("announcements:test:v{version}");
    cache
        .set_json(&cache_key, &serde_json::json!({ "ok": true }), 30)
        .await
        .expect("set cache");
    let cached: Option<serde_json::Value> = cache.get_json(&cache_key).await.expect("get cache");
    assert_eq!(cached, Some(serde_json::json!({ "ok": true })));

    let sanitized =
        validation::normalize_announcement_content("<p>ok</p><script>alert(1)</script>")
            .expect("sanitize");
    assert!(!sanitized.contains("script"));

    let security = SecurityRepo::new(db.pool().clone());
    let session_id = uuid::Uuid::new_v4();
    let token = "integration-refresh-token";
    let token_hash = qe_auth::session::refresh_token_hash(token);
    security
        .create_session_with_id(
            session_id,
            user.id,
            &token_hash,
            "integration-agent",
            Some("127.0.0.1"),
        )
        .await
        .expect("create session");
    assert!(security.session_active(session_id).await.expect("active"));
    assert_eq!(
        security.list_sessions(user.id).await.expect("list").len(),
        1
    );
    assert!(
        security
            .rotate_session_token(session_id, "new-hash")
            .await
            .expect("rotate")
    );
    assert!(
        security
            .revoke_session(user.id, session_id)
            .await
            .expect("revoke")
    );
    assert!(!security.session_active(session_id).await.expect("inactive"));

    cache
        .mark_session_active(session_id, 60)
        .await
        .expect("mark session");
    assert_eq!(
        cache
            .cached_session_active(session_id)
            .await
            .expect("cached"),
        Some(true)
    );
    cache
        .revoke_session_cache(session_id)
        .await
        .expect("revoke cache");
    assert_eq!(
        cache
            .cached_session_active(session_id)
            .await
            .expect("cached"),
        None
    );

    let reset_hash = qe_auth::session::refresh_token_hash("reset-token");
    security
        .create_password_reset(
            user.id,
            &reset_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .expect("create reset");
    assert_eq!(
        security
            .consume_password_reset(&reset_hash)
            .await
            .expect("consume reset"),
        Some(user.id)
    );
    assert_eq!(
        security
            .consume_password_reset(&reset_hash)
            .await
            .expect("consume used reset"),
        None
    );

    sqlx::query("DELETE FROM audit_logs WHERE actor_id = $1")
        .bind(user.id)
        .execute(db.pool())
        .await
        .expect("cleanup audit");
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user.id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
}
