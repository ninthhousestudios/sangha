mod common;

use std::sync::Arc;

use sangha::config::Config;
use sangha::db::{Db, RegisterInput};
use sangha::tools::inbox::{handle_broadcast, handle_read_inbox, BroadcastArgs, ReadInboxArgs};

// ---------------------------------------------------------------------------
// Helper: register a session and return its id
// ---------------------------------------------------------------------------

fn register_session(db: &Arc<Db>, project: &str) -> String {
    db.register_session(RegisterInput {
        session_id: None,
        project: project.to_string(),
        branch: None,
        intent: None,
        pid: None,
        hostname: None,
        metadata: None,
    })
    .expect("register session")
    .id
}

// ---------------------------------------------------------------------------
// 1. test_broadcast_and_read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_broadcast_and_read() {
    let db = common::test_db();
    let project = common::unique_project("broadcast-read");

    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs { message: "hello from A".to_string(), tags: None, scope: None },
    )
    .await
    .expect("broadcast ok");

    let out = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: None, scope: None },
    )
    .await
    .expect("read ok");

    assert_eq!(out.count, 1);
    assert_eq!(out.messages[0].message, "hello from A");
    assert_eq!(out.messages[0].from_session, sid_a);
}

// ---------------------------------------------------------------------------
// 2. test_excludes_own_messages
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_excludes_own_messages() {
    let db = common::test_db();
    let project = common::unique_project("own-msgs");

    let sid_a = register_session(&db, &project);

    handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs { message: "self-broadcast".to_string(), tags: None, scope: None },
    )
    .await
    .expect("broadcast ok");

    let out = handle_read_inbox(
        &db,
        &sid_a,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: None, scope: None },
    )
    .await
    .expect("read ok");

    assert_eq!(out.count, 0, "sender should not see their own messages");
    assert_eq!(out.total, 0);
}

// ---------------------------------------------------------------------------
// 3. test_unread_only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unread_only() {
    let db = common::test_db();
    let project = common::unique_project("unread-only");

    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs { message: "mark me read".to_string(), tags: None, scope: None },
    )
    .await
    .expect("broadcast ok");

    // First read — marks the message as read.
    let first = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: None, scope: None },
    )
    .await
    .expect("first read ok");
    assert_eq!(first.count, 1);

    // Second read with unread_only=true — should be empty now.
    let second = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: None, scope: None },
    )
    .await
    .expect("second read ok");
    assert_eq!(second.count, 0, "message should be marked read after first read");
    assert_eq!(second.unread_total, 0);
}

// ---------------------------------------------------------------------------
// 4. test_read_all
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_all() {
    let db = common::test_db();
    let project = common::unique_project("read-all");

    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs { message: "persistent message".to_string(), tags: None, scope: None },
    )
    .await
    .expect("broadcast ok");

    // Mark as read.
    handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: None, scope: None },
    )
    .await
    .expect("first read ok");

    // Read with unread_only=false — should still return the message.
    let out = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(false), limit: None, scope: None },
    )
    .await
    .expect("read all ok");

    assert_eq!(out.count, 1, "unread_only=false should return already-read messages");
    assert_eq!(out.messages[0].message, "persistent message");
}

// ---------------------------------------------------------------------------
// 5. test_project_scoping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_project_scoping() {
    let db = common::test_db();
    let project_a = common::unique_project("proj-scope-a");
    let project_b = common::unique_project("proj-scope-b");

    let sid_a = register_session(&db, &project_a);
    let sid_b = register_session(&db, &project_b);

    handle_broadcast(
        &db,
        &sid_a,
        &project_a,
        BroadcastArgs { message: "only for project A".to_string(), tags: None, scope: None },
    )
    .await
    .expect("broadcast ok");

    let out = handle_read_inbox(
        &db,
        &sid_b,
        &project_b,
        ReadInboxArgs { unread_only: Some(true), limit: None, scope: None },
    )
    .await
    .expect("read ok");

    assert_eq!(out.count, 0, "session in project B should not see project A messages");
    assert_eq!(out.total, 0);
}

// ---------------------------------------------------------------------------
// 6. test_user_scope_broadcast
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_user_scope_broadcast() {
    let db = common::test_db();
    let project_a = common::unique_project("user-scope-a");
    let project_b = common::unique_project("user-scope-b");

    let sid_a = register_session(&db, &project_a);
    let sid_b = register_session(&db, &project_b);

    // Broadcast with user scope — goes into __user__ project.
    handle_broadcast(
        &db,
        &sid_a,
        &project_a,
        BroadcastArgs {
            message: "cross-project message".to_string(),
            tags: None,
            scope: Some("user".to_string()),
        },
    )
    .await
    .expect("broadcast ok");

    // Read with user scope from a different project — should see it.
    let out = handle_read_inbox(
        &db,
        &sid_b,
        &project_b,
        ReadInboxArgs {
            unread_only: Some(true),
            limit: None,
            scope: Some("user".to_string()),
        },
    )
    .await
    .expect("read ok");

    assert_eq!(out.count, 1, "user-scoped message should be visible across projects");
    assert_eq!(out.messages[0].message, "cross-project message");
}

// ---------------------------------------------------------------------------
// 7. test_tags_preserved
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tags_preserved() {
    let db = common::test_db();
    let project = common::unique_project("tags");

    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs {
            message: "tagged message".to_string(),
            tags: Some(vec!["important".to_string(), "handoff".to_string()]),
            scope: None,
        },
    )
    .await
    .expect("broadcast ok");

    let out = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: None, scope: None },
    )
    .await
    .expect("read ok");

    assert_eq!(out.count, 1);
    let tags = out.messages[0].tags.as_ref().expect("tags present");
    assert_eq!(tags, &vec!["important".to_string(), "handoff".to_string()]);
}

// ---------------------------------------------------------------------------
// 8. test_retention_pruning
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_retention_pruning() {
    let short_cfg = Arc::new(Config {
        db_path: std::path::PathBuf::from(":memory:"),
        host: "127.0.0.1".to_string(),
        port: 0,
        session_ttl_ms: 600_000,
        lock_default_ttl_ms: 600_000,
        lock_long_op_ttl_ms: 1_800_000,
        lock_max_ttl_ms: 86_400_000,
        inbox_retention_ms: 50, // very short retention
        log_level: "info".to_string(),
    });
    let db = Arc::new(Db::open_memory(&short_cfg).expect("open db"));
    db.run_migrations().expect("migrate");

    let project = common::unique_project("retention");
    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    // Broadcast first message.
    let first = handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs { message: "old message".to_string(), tags: None, scope: None },
    )
    .await
    .expect("first broadcast ok");

    // Wait past retention window.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Broadcast second message — this triggers prune_old_inbox internally.
    handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs { message: "new message".to_string(), tags: None, scope: None },
    )
    .await
    .expect("second broadcast ok");

    // Read inbox — old message should be pruned, only new one visible.
    let out = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(false), limit: None, scope: None },
    )
    .await
    .expect("read ok");

    assert_eq!(out.count, 1, "old message should have been pruned");
    assert_eq!(out.messages[0].message, "new message");
    // Verify the old message id is gone.
    assert!(
        out.messages.iter().all(|m| m.id != first.message_id),
        "old message id should not appear"
    );
}

// ---------------------------------------------------------------------------
// 9. test_counts_before_marking
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_counts_before_marking() {
    let db = common::test_db();
    let project = common::unique_project("counts");

    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    // Session A broadcasts 3 messages.
    for i in 1..=3 {
        handle_broadcast(
            &db,
            &sid_a,
            &project,
            BroadcastArgs {
                message: format!("message {i}"),
                tags: None,
                scope: None,
            },
        )
        .await
        .expect("broadcast ok");
    }

    // Session B reads with limit=1.
    let out = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: Some(1), scope: None },
    )
    .await
    .expect("read ok");

    assert_eq!(out.count, 1, "should return only 1 message due to limit");
    assert_eq!(out.unread_total, 3, "unread_total should reflect all 3 before marking");
    assert_eq!(out.total, 3, "total should be 3");
}

// ---------------------------------------------------------------------------
// 10. cursor ordering — newest first, follow-up returns next (P3 #25)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cursor_ordering() {
    let db = common::test_db();
    let project = common::unique_project("cursor-order");

    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    for i in 1..=3 {
        handle_broadcast(
            &db,
            &sid_a,
            &project,
            BroadcastArgs {
                message: format!("msg-{i}"),
                tags: None,
                scope: None,
            },
        )
        .await
        .expect("broadcast ok");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    // Read limit=1 — should get the newest (msg-3) due to ORDER BY created_at DESC.
    let first_read = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: Some(1), scope: None },
    )
    .await
    .expect("first read ok");

    assert_eq!(first_read.count, 1);
    assert_eq!(first_read.messages[0].message, "msg-3");

    // Follow-up read — msg-3 is now read, should get msg-2.
    let second_read = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(true), limit: Some(1), scope: None },
    )
    .await
    .expect("second read ok");

    assert_eq!(second_read.count, 1);
    assert_eq!(second_read.messages[0].message, "msg-2");

    // Unread total should have decremented.
    assert_eq!(second_read.unread_total, 2, "unread should be 2 after first read of 1");
}

// ---------------------------------------------------------------------------
// 11. limit clamping — 0 and huge values (P3 #44)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_limit_clamping() {
    let db = common::test_db();
    let project = common::unique_project("limit-clamp");

    let sid_a = register_session(&db, &project);
    let sid_b = register_session(&db, &project);

    handle_broadcast(
        &db,
        &sid_a,
        &project,
        BroadcastArgs { message: "hello".to_string(), tags: None, scope: None },
    )
    .await
    .expect("broadcast ok");

    // limit=0 should be clamped to 1.
    let out_zero = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(false), limit: Some(0), scope: None },
    )
    .await
    .expect("read ok");
    assert_eq!(out_zero.count, 1, "limit=0 should be clamped to 1");

    // limit=99999 should be clamped to MAX_LIMIT (1000).
    let out_huge = handle_read_inbox(
        &db,
        &sid_b,
        &project,
        ReadInboxArgs { unread_only: Some(false), limit: Some(99999), scope: None },
    )
    .await
    .expect("read ok");
    assert_eq!(out_huge.count, 1, "should still return the 1 message");
}

// ---------------------------------------------------------------------------
// 12. empty message rejected by validation (P1 #5 — handler→validator)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_broadcast_empty_message_rejected() {
    let db = common::test_db();
    let project = common::unique_project("empty-msg");

    let sid = register_session(&db, &project);

    let result = handle_broadcast(
        &db,
        &sid,
        &project,
        BroadcastArgs { message: String::new(), tags: None, scope: None },
    )
    .await;

    assert!(result.is_err(), "empty message should be rejected by validation");
}

// ---------------------------------------------------------------------------
// 13. prune_old_inbox returns count (P3 #45)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_prune_old_inbox_returns_count() {
    let short_retention = std::sync::Arc::new(Config {
        db_path: std::path::PathBuf::from(":memory:"),
        host: "127.0.0.1".to_string(),
        port: 0,
        session_ttl_ms: 600_000,
        lock_default_ttl_ms: 600_000,
        lock_long_op_ttl_ms: 1_800_000,
        lock_max_ttl_ms: 86_400_000,
        inbox_retention_ms: 50,
        log_level: "info".to_string(),
    });
    let db = std::sync::Arc::new(
        sangha::db::Db::open_memory(&short_retention).expect("open db"),
    );
    db.run_migrations().expect("migrate");

    let project = common::unique_project("prune-inbox-count");
    let sid = register_session(&db, &project);

    handle_broadcast(
        &db,
        &sid,
        &project,
        BroadcastArgs { message: "old msg".to_string(), tags: None, scope: None },
    )
    .await
    .expect("broadcast ok");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let pruned = db.prune_old_inbox().expect("prune ok");
    assert_eq!(pruned, 1, "should have pruned exactly 1 old inbox message");
}
