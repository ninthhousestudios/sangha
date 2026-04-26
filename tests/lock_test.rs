mod common;

use std::sync::Arc;

use sangha::config::Config;
use sangha::db::{Db, RegisterInput};
use sangha::tools::locks::{
    handle_claim, handle_list, handle_release, ClaimArgs, LockListArgs, ReleaseArgs,
};
use sangha::tools::presence::handle_heartbeat;

// ---------------------------------------------------------------------------
// Helper: register a session and return (db, session_id, project)
// ---------------------------------------------------------------------------

async fn register_session(db: &Arc<Db>, project: &str) -> String {
    let input = RegisterInput {
        session_id: None,
        project: project.to_string(),
        branch: None,
        intent: None,
        pid: None,
        hostname: None,
        metadata: None,
    };
    db.register_session(input).expect("register session").id
}

fn default_config() -> Arc<Config> {
    common::test_config()
}

// ---------------------------------------------------------------------------
// 1. test_claim_granted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_claim_granted() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("claim-granted");
    let sid = register_session(&db, &project).await;

    let out = handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "file.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("claim ok");

    assert!(out.ok);
    assert!(out.lock.is_some());
    assert!(out.held_by.is_none());
}

// ---------------------------------------------------------------------------
// 2. test_claim_conflict
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_claim_conflict() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("claim-conflict");

    let sid_a = register_session(&db, &project).await;
    let sid_b = register_session(&db, &project).await;

    handle_claim(
        &db,
        &cfg,
        &sid_a,
        &project,
        ClaimArgs {
            resource: "shared.rs".to_string(),
            scope: None,
            reason: Some("editing".to_string()),
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("A claims ok");

    let out_b = handle_claim(
        &db,
        &cfg,
        &sid_b,
        &project,
        ClaimArgs {
            resource: "shared.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("B claim returns result");

    assert!(!out_b.ok);
    assert!(out_b.held_by.is_some());
    assert_eq!(out_b.held_by.unwrap().session_id, sid_a);
}

// ---------------------------------------------------------------------------
// 3. test_same_session_renewal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_same_session_renewal() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("renewal");
    let sid = register_session(&db, &project).await;

    let first = handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "lib.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("first claim ok");

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let second = handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "lib.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("renewal ok");

    assert!(second.ok);
    // acquired_at should be preserved (not bumped on renewal).
    let first_lock = first.lock.unwrap();
    let second_lock = second.lock.unwrap();
    assert_eq!(first_lock.acquired_at, second_lock.acquired_at);
}

// ---------------------------------------------------------------------------
// 4. test_project_scoping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_project_scoping() {
    let db = common::test_db();
    let cfg = default_config();
    let project_a = common::unique_project("scope-a");
    let project_b = common::unique_project("scope-b");

    let sid_a = register_session(&db, &project_a).await;
    let sid_b = register_session(&db, &project_b).await;

    let out_a = handle_claim(
        &db,
        &cfg,
        &sid_a,
        &project_a,
        ClaimArgs {
            resource: "main.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("A claims ok");

    let out_b = handle_claim(
        &db,
        &cfg,
        &sid_b,
        &project_b,
        ClaimArgs {
            resource: "main.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("B claims ok");

    assert!(out_a.ok, "A should succeed");
    assert!(out_b.ok, "B in different project should also succeed");
}

// ---------------------------------------------------------------------------
// 5. test_user_scope_lock
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_user_scope_lock() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("user-scope");
    let sid = register_session(&db, &project).await;

    let out = handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "global-res".to_string(),
            scope: Some("user".to_string()),
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("user scope claim ok");

    assert!(out.ok);
    // The lock should be stored under __user__ project.
    let lock = out.lock.unwrap();
    assert_eq!(lock.project, sangha::tools::validate::USER_SCOPE_PROJECT);
}

// ---------------------------------------------------------------------------
// 6. test_release_by_owner
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_release_by_owner() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("release-owner");
    let sid = register_session(&db, &project).await;

    handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "owned.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("claim ok");

    let out = handle_release(
        &db,
        &sid,
        &project,
        ReleaseArgs { resource: "owned.rs".to_string(), scope: None, force: None },
    )
    .await
    .expect("release ok");

    assert!(out.ok);
    assert_eq!(out.status, "released");
}

// ---------------------------------------------------------------------------
// 7. test_release_by_non_owner
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_release_by_non_owner() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("release-nonowner");

    let sid_a = register_session(&db, &project).await;
    let sid_b = register_session(&db, &project).await;

    handle_claim(
        &db,
        &cfg,
        &sid_a,
        &project,
        ClaimArgs {
            resource: "locked.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("A claims");

    let out = handle_release(
        &db,
        &sid_b,
        &project,
        ReleaseArgs { resource: "locked.rs".to_string(), scope: None, force: None },
    )
    .await
    .expect("release returns result");

    assert!(!out.ok);
    assert_eq!(out.status, "not_owner");
}

// ---------------------------------------------------------------------------
// 8. test_release_not_held
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_release_not_held() {
    let db = common::test_db();
    let project = common::unique_project("release-not-held");
    let sid = register_session(&db, &project).await;

    let out = handle_release(
        &db,
        &sid,
        &project,
        ReleaseArgs { resource: "nonexistent.rs".to_string(), scope: None, force: None },
    )
    .await
    .expect("release returns result");

    assert!(!out.ok);
    assert_eq!(out.status, "not_found");
}

// ---------------------------------------------------------------------------
// 9. test_expired_lock_pruned
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_expired_lock_pruned() {
    let short_cfg = Arc::new(Config {
        db_path: std::path::PathBuf::from(":memory:"),
        host: "127.0.0.1".to_string(),
        port: 0,
        session_ttl_ms: 600_000,
        lock_default_ttl_ms: 50, // very short
        lock_long_op_ttl_ms: 1_800_000,
        lock_max_ttl_ms: 86_400_000,
        inbox_retention_ms: 86_400_000,
        log_level: "info".to_string(),
    });
    let db = Arc::new(Db::open_memory(&short_cfg).expect("open db"));
    db.run_migrations().expect("migrate");

    let project = common::unique_project("expired-lock");
    let sid_a = register_session(&db, &project).await;

    let out_a = handle_claim(
        &db,
        &short_cfg,
        &sid_a,
        &project,
        ClaimArgs {
            resource: "expiring.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("A claims ok");
    assert!(out_a.ok);

    // Wait for the lock to expire.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // A new session should now be able to claim.
    let sid_b = register_session(&db, &project).await;
    let out_b = handle_claim(
        &db,
        &short_cfg,
        &sid_b,
        &project,
        ClaimArgs {
            resource: "expiring.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("B claims after expiry");
    assert!(out_b.ok, "expired lock should have been pruned");
}

// ---------------------------------------------------------------------------
// 10. test_fk_cascade
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fk_cascade() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("fk-cascade");
    let sid = register_session(&db, &project).await;

    handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "cascade.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("claim ok");

    // Unregister session — cascade should remove the lock.
    db.unregister_session(&sid).expect("unregister ok");

    let list = handle_list(&db, LockListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");

    assert!(
        list.locks.is_empty(),
        "lock should be gone after session unregistered (cascade)"
    );
}

// ---------------------------------------------------------------------------
// 11. test_auto_extend_on_heartbeat
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_auto_extend_on_heartbeat() {
    // Use a short TTL so we can reach half-expired quickly.
    let short_cfg = Arc::new(Config {
        db_path: std::path::PathBuf::from(":memory:"),
        host: "127.0.0.1".to_string(),
        port: 0,
        session_ttl_ms: 600_000,
        lock_default_ttl_ms: 200, // 200 ms TTL
        lock_long_op_ttl_ms: 1_800_000,
        lock_max_ttl_ms: 86_400_000,
        inbox_retention_ms: 86_400_000,
        log_level: "info".to_string(),
    });
    let db = Arc::new(Db::open_memory(&short_cfg).expect("open db"));
    db.run_migrations().expect("migrate");

    let project = common::unique_project("auto-extend");
    let sid = register_session(&db, &project).await;

    let before = handle_claim(
        &db,
        &short_cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "extending.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("claim ok");
    assert!(before.ok);

    let expires_before = before.expires_at.clone().unwrap();

    // Sleep past the halfway point (>100 ms).
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;

    // Heartbeat triggers auto_extend_locks.
    handle_heartbeat(&db, &sid).await.expect("heartbeat ok");

    // Fetch the lock directly to check expires_at advanced.
    let list = handle_list(&db, LockListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");

    assert_eq!(list.locks.len(), 1);
    assert!(
        list.locks[0].expires_at > expires_before,
        "expires_at should have advanced after heartbeat auto-extend"
    );
}

// ---------------------------------------------------------------------------
// 12. test_no_extend_fresh_lock
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_no_extend_fresh_lock() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("no-extend");
    let sid = register_session(&db, &project).await;

    let out = handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "fresh.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("claim ok");
    assert!(out.ok);
    let expires_before = out.expires_at.clone().unwrap();

    // Call auto_extend immediately — lock is fresh, should not extend.
    let extended = db.auto_extend_locks(&sid).expect("auto_extend ok");
    assert_eq!(extended, 0, "fresh lock should not be extended");

    let list = handle_list(&db, LockListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");
    assert_eq!(list.locks[0].expires_at, expires_before);
}

// ---------------------------------------------------------------------------
// 13. test_long_op_ttl
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_long_op_ttl() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("long-op");
    let sid = register_session(&db, &project).await;

    let out = handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "long-op.rs".to_string(),
            scope: None,
            reason: None,
            long_op: Some(true),
            ttl_sec: None,
        },
    )
    .await
    .expect("claim ok");

    assert!(out.ok);
    let lock = out.lock.unwrap();
    // long_op flag should be stored.
    assert!(lock.long_op);
    // The lock should have a TTL of lock_long_op_ttl_ms (1_800_000 ms = 1800 s).
    // We verify by checking the lock row ttl_ms stored in list output.
    // Indirect check: acquired_at and expires_at should differ by ~1_800_000 ms.
    // Parse both timestamps and compare.
    let acq = chrono::DateTime::parse_from_rfc3339(&lock.acquired_at)
        .expect("parse acquired_at")
        .timestamp_millis();
    let exp = chrono::DateTime::parse_from_rfc3339(&lock.expires_at)
        .expect("parse expires_at")
        .timestamp_millis();
    let diff = exp - acq;
    assert!(
        diff >= 1_790_000 && diff <= 1_810_000,
        "long_op TTL diff should be ~1_800_000 ms, got {diff}"
    );
}

// ---------------------------------------------------------------------------
// 14. test_max_ttl_clamped
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_max_ttl_clamped() {
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("max-ttl");
    let sid = register_session(&db, &project).await;

    // ttl_sec = 1 day + 1 second (exceeds lock_max_ttl_ms = 86_400_000 ms)
    let too_long_sec = 86_401i64;

    let result = handle_claim(
        &db,
        &cfg,
        &sid,
        &project,
        ClaimArgs {
            resource: "max-ttl.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: Some(too_long_sec),
        },
    )
    .await;

    assert!(result.is_err(), "should return InvalidArgument for TTL exceeding max");
}

// ---------------------------------------------------------------------------
// 15. test_claim_auto_creates_session (documents current behavior)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_claim_without_session_fails() {
    // Documents current DB behavior: claiming without a registered session returns ok=false.
    let db = common::test_db();
    let cfg = default_config();
    let project = common::unique_project("no-session");
    let fake_sid = "00000000-0000-0000-0000-000000000000";

    let out = handle_claim(
        &db,
        &cfg,
        fake_sid,
        &project,
        ClaimArgs {
            resource: "orphan.rs".to_string(),
            scope: None,
            reason: None,
            long_op: None,
            ttl_sec: None,
        },
    )
    .await
    .expect("handler returns Ok with ok=false");

    assert!(!out.ok, "claiming without a registered session should return ok=false");
}
