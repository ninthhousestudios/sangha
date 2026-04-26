mod common;

use std::sync::Arc;

use sangha::config::Config;
use sangha::db::Db;
use sangha::tools::presence::{
    handle_heartbeat, handle_list, handle_register, handle_unregister, ListArgs, RegisterArgs,
};

// ---------------------------------------------------------------------------
// 1. register then list shows the session
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_and_list() {
    let db = common::test_db();
    let project = common::unique_project("register-and-list");

    let out = handle_register(
        &db,
        None,
        RegisterArgs { project: project.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("register ok");

    let list = handle_list(&db, ListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");

    assert_eq!(list.sessions.len(), 1);
    assert_eq!(list.sessions[0].session_id, out.session_id);
    assert_eq!(list.sessions[0].project, project);
}

// ---------------------------------------------------------------------------
// 2. re-register same session_id preserves started_at
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_upsert() {
    let db = common::test_db();
    let project = common::unique_project("register-upsert");

    let first = handle_register(
        &db,
        None,
        RegisterArgs {
            project: project.clone(),
            branch: Some("main".to_string()),
            intent: None,
            metadata: None,
        },
    )
    .await
    .expect("first register ok");

    // Brief pause so clock advances.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let second = handle_register(
        &db,
        Some(&first.session_id),
        RegisterArgs {
            project: project.clone(),
            branch: Some("feature".to_string()),
            intent: Some("new intent".to_string()),
            metadata: None,
        },
    )
    .await
    .expect("re-register ok");

    // Same session_id.
    assert_eq!(first.session_id, second.session_id);

    // started_at must be preserved (upsert).
    let list = handle_list(&db, ListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");
    assert_eq!(list.sessions.len(), 1);
    // Branch and intent should reflect the second registration.
    assert_eq!(list.sessions[0].branch.as_deref(), Some("feature"));
    assert_eq!(list.sessions[0].intent.as_deref(), Some("new intent"));
}

// ---------------------------------------------------------------------------
// 3. second session on same project sees the first in `others`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_returns_others() {
    let db = common::test_db();
    let project = common::unique_project("register-others");

    let first = handle_register(
        &db,
        None,
        RegisterArgs { project: project.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("first ok");

    let second = handle_register(
        &db,
        None,
        RegisterArgs { project: project.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("second ok");

    // The second registration should report the first session in `others`.
    assert_eq!(second.others.len(), 1);
    assert_eq!(second.others[0].session_id, first.session_id);

    // The first registration had no peers.
    assert!(first.others.is_empty());
}

// ---------------------------------------------------------------------------
// 4. sessions in project A are not visible from project B
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_project_scoping() {
    let db = common::test_db();
    let project_a = common::unique_project("scope-a");
    let project_b = common::unique_project("scope-b");

    handle_register(
        &db,
        None,
        RegisterArgs { project: project_a.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("register in A");

    let list_b = handle_list(&db, ListArgs { project: Some(project_b.clone()), scope: None })
        .await
        .expect("list B");

    assert!(
        list_b.sessions.is_empty(),
        "project B should have no sessions, got {:?}",
        list_b.sessions.len()
    );

    let list_a = handle_list(&db, ListArgs { project: Some(project_a.clone()), scope: None })
        .await
        .expect("list A");
    assert_eq!(list_a.sessions.len(), 1);
}

// ---------------------------------------------------------------------------
// 5. heartbeat advances last_heartbeat
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_heartbeat_updates_timestamp() {
    let db = common::test_db();
    let project = common::unique_project("hb-ts");

    let reg = handle_register(
        &db,
        None,
        RegisterArgs { project: project.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("register ok");

    // Grab initial last_heartbeat.
    let before = db
        .get_session(&reg.session_id)
        .expect("get ok")
        .expect("session exists")
        .last_heartbeat;

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let hb_out = handle_heartbeat(&db, &reg.session_id).await.expect("heartbeat ok");
    assert_eq!(hb_out.session_id, reg.session_id);
    assert!(hb_out.ttl_remaining_sec > 0);

    let after = db
        .get_session(&reg.session_id)
        .expect("get ok")
        .expect("session exists")
        .last_heartbeat;

    assert!(after > before, "last_heartbeat should advance after heartbeat");
}

// ---------------------------------------------------------------------------
// 6. heartbeat on unknown session does not panic; result.ok is false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_heartbeat_unknown_session() {
    let db = common::test_db();

    // The DB returns ok=false for unknown sessions — the handler propagates it.
    let out = handle_heartbeat(&db, "00000000-0000-0000-0000-000000000000")
        .await
        .expect("handler should not error");

    // ttl_remaining_sec is 0 when the session is not found.
    assert_eq!(out.ttl_remaining_sec, 0);
}

// ---------------------------------------------------------------------------
// 7. unregister removes the session from the list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unregister() {
    let db = common::test_db();
    let project = common::unique_project("unregister");

    let reg = handle_register(
        &db,
        None,
        RegisterArgs { project: project.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("register ok");

    let unreg = handle_unregister(&db, &reg.session_id).await.expect("unregister ok");
    assert!(unreg.ok);

    let list = handle_list(&db, ListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");
    assert!(list.sessions.is_empty(), "session should be gone after unregister");
}

// ---------------------------------------------------------------------------
// 8. dead sessions are pruned after TTL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_ttl_pruning() {
    // Config with a very short session TTL (50 ms).
    let short_ttl_config = Arc::new(Config {
        db_path: std::path::PathBuf::from(":memory:"),
        host: "127.0.0.1".to_string(),
        port: 0,
        session_ttl_ms: 50,
        lock_default_ttl_ms: 600_000,
        lock_long_op_ttl_ms: 1_800_000,
        lock_max_ttl_ms: 86_400_000,
        inbox_retention_ms: 86_400_000,
        log_level: "info".to_string(),
    });
    let db = Arc::new(Db::open_memory(&short_ttl_config).expect("open db"));
    db.run_migrations().expect("migrate");

    let project = common::unique_project("ttl-prune");

    handle_register(
        &db,
        None,
        RegisterArgs { project: project.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("register ok");

    // Wait long enough for the session to expire.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // list_sessions prunes first, so the expired session should be gone.
    let list = handle_list(&db, ListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");

    assert!(
        list.sessions.is_empty(),
        "expired session should have been pruned, got {}",
        list.sessions.len()
    );
}

// ---------------------------------------------------------------------------
// 9. live sessions survive pruning
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_not_pruned_within_ttl() {
    let db = common::test_db(); // TTL = 600 s
    let project = common::unique_project("ttl-survive");

    let reg = handle_register(
        &db,
        None,
        RegisterArgs { project: project.clone(), branch: None, intent: None, metadata: None },
    )
    .await
    .expect("register ok");

    // Prune explicitly — nothing should be removed.
    db.prune_dead_sessions().expect("prune ok");

    let list = handle_list(&db, ListArgs { project: Some(project.clone()), scope: None })
        .await
        .expect("list ok");

    assert_eq!(list.sessions.len(), 1);
    assert_eq!(list.sessions[0].session_id, reg.session_id);
}
