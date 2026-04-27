//! Contract tests for error types, config defaults, and validation.

use sangha::error::{ErrorData, SanghaError, codes};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn assert_contract(data: &ErrorData) {
    assert!(!data.tool.is_empty(), "tool must be non-empty; got empty string");
    assert!(
        !data.constraint.is_empty(),
        "constraint must be non-empty for tool={}", data.tool
    );
    assert!(
        !data.next_action.is_empty(),
        "next_action must be non-empty for tool={}", data.tool
    );
}

// ─── every_error_variant_populates_contract ──────────────────────────────────

/// Exhaustive match — if a new variant is added without wiring `.data()` this
/// test either fails to compile (on the `variants` vec) or fails at runtime on
/// an empty field.
#[test]
fn every_error_variant_populates_contract() {
    let variants: Vec<SanghaError> = vec![
        SanghaError::MissingConfig {
            name: "SANGHA_DB_PATH",
            next_action: "set it",
        },
        SanghaError::InvalidArgument {
            tool: "register_session",
            argument: "session_id",
            constraint: "must be non-empty".to_string(),
            received: Some(String::new()),
            next_action: "Pass a non-empty session_id.".to_string(),
        },
        SanghaError::NotFound {
            tool: "get_session",
            kind: "session".to_string(),
            next_action: "Verify the session_id.".to_string(),
        },
        SanghaError::IdentityError {
            tool: "acquire_lock",
            message: "no identity".to_string(),
            next_action: "Register a session first.".to_string(),
        },
        SanghaError::Internal("unexpected state".to_string()),
        SanghaError::Db(rusqlite::Error::QueryReturnedNoRows),
        SanghaError::Db(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some("UNIQUE constraint failed".to_string()),
        )),
        SanghaError::Db(rusqlite::Error::InvalidPath("bad".into())),
    ];

    for e in &variants {
        let d = e.data();
        assert_contract(&d);
        let code = e.code();
        assert!(
            code == codes::INVALID_PARAMS || code == codes::INTERNAL_ERROR,
            "code {code} for {e:?} must be INVALID_PARAMS or INTERNAL_ERROR"
        );
    }
}

// ─── code routing ─────────────────────────────────────────────────────────────

#[test]
fn missing_config_is_invalid_params() {
    let e = SanghaError::MissingConfig {
        name: "SANGHA_DB_PATH",
        next_action: "Set SANGHA_DB_PATH in the environment.",
    };
    assert_eq!(e.code(), codes::INVALID_PARAMS);
    assert_contract(&e.data());
}

#[test]
fn invalid_argument_is_invalid_params() {
    let e = SanghaError::InvalidArgument {
        tool: "register_session",
        argument: "agent_id",
        constraint: "must be non-empty".to_string(),
        received: None,
        next_action: "Pass a non-empty agent_id.".to_string(),
    };
    assert_eq!(e.code(), codes::INVALID_PARAMS);
    assert_contract(&e.data());
}

#[test]
fn not_found_is_invalid_params() {
    let e = SanghaError::NotFound {
        tool: "get_session",
        kind: "session".to_string(),
        next_action: "Verify the id.".to_string(),
    };
    assert_eq!(e.code(), codes::INVALID_PARAMS);
    assert_contract(&e.data());
}

#[test]
fn identity_error_is_invalid_params() {
    let e = SanghaError::IdentityError {
        tool: "acquire_lock",
        message: "no token".to_string(),
        next_action: "Register first.".to_string(),
    };
    assert_eq!(e.code(), codes::INVALID_PARAMS);
    assert_contract(&e.data());
}

#[test]
fn internal_is_internal_error_code() {
    let e = SanghaError::Internal("boom".to_string());
    assert_eq!(e.code(), codes::INTERNAL_ERROR);
    assert_contract(&e.data());
}

// ─── error_data_serializes ────────────────────────────────────────────────────

#[test]
fn error_data_serializes() {
    let variants: Vec<SanghaError> = vec![
        SanghaError::MissingConfig {
            name: "SANGHA_DB_PATH",
            next_action: "set it",
        },
        SanghaError::InvalidArgument {
            tool: "t",
            argument: "a",
            constraint: "c".to_string(),
            received: Some("bad".to_string()),
            next_action: "fix it".to_string(),
        },
        SanghaError::NotFound {
            tool: "t",
            kind: "session".to_string(),
            next_action: "verify id".to_string(),
        },
        SanghaError::IdentityError {
            tool: "t",
            message: "no id".to_string(),
            next_action: "register first".to_string(),
        },
        SanghaError::Internal("oops".to_string()),
    ];

    for e in &variants {
        let data = e.data();
        let json = serde_json::to_value(&data).expect("data must serialize");
        assert!(json.get("tool").is_some(), "tool key missing for {:?}", e);
        assert!(json.get("constraint").is_some(), "constraint key missing for {:?}", e);
        assert!(json.get("next_action").is_some(), "next_action key missing for {:?}", e);
    }
}

#[test]
fn error_data_invalid_argument_has_argument_field() {
    let e = SanghaError::InvalidArgument {
        tool: "register_session",
        argument: "session_id",
        constraint: "non-empty".to_string(),
        received: Some("".to_string()),
        next_action: "pass non-empty".to_string(),
    };
    let json = serde_json::to_value(e.data()).unwrap();
    assert_eq!(json["tool"], "register_session");
    assert_eq!(json["argument"], "session_id");
    assert!(json.get("received").is_some());
}

#[test]
fn error_data_roundtrip() {
    let data = ErrorData {
        tool: "register_session",
        argument: Some("agent_id".to_string()),
        constraint: "must be non-empty".to_string(),
        received: Some(serde_json::json!("")),
        next_action: "Pass a non-empty agent_id.".to_string(),
    };
    // Serialize → deserialize → re-serialize and check field stability.
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["tool"], "register_session");
    assert_eq!(json["argument"], "agent_id");
    assert_eq!(json["constraint"], "must be non-empty");
    assert_eq!(json["next_action"], "Pass a non-empty agent_id.");
    // received is present because it's Some
    assert!(json.get("received").is_some());
}

#[test]
fn error_data_skips_none_fields() {
    let data = ErrorData {
        tool: "server",
        argument: None,
        constraint: "no fault".to_string(),
        received: None,
        next_action: "retry".to_string(),
    };
    let json = serde_json::to_value(&data).unwrap();
    assert!(json.get("argument").is_none(), "argument should be absent when None");
    assert!(json.get("received").is_none(), "received should be absent when None");
}

// ─── config_defaults_are_sane ─────────────────────────────────────────────────

use sangha::config::Config;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_clean_env<R>(f: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let vars = [
        "SANGHA_DB_PATH",
        "SANGHA_HOST",
        "SANGHA_PORT",
        "SANGHA_SESSION_TTL_SEC",
        "SANGHA_LOCK_TTL_SEC",
        "SANGHA_LOCK_LONG_OP_TTL_SEC",
        "SANGHA_LOCK_MAX_TTL_SEC",
        "SANGHA_INBOX_RETENTION_SEC",
        "SANGHA_LOG_LEVEL",
    ];

    let prior: Vec<(String, Option<String>)> =
        vars.iter().map(|k| (k.to_string(), std::env::var(k).ok())).collect();

    unsafe {
        for k in &vars {
            std::env::remove_var(k);
        }
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

    unsafe {
        for (k, v) in &prior {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }

    match result {
        Ok(r) => r,
        Err(p) => std::panic::resume_unwind(p),
    }
}

#[test]
fn config_defaults_are_sane() {
    with_clean_env(|| {
        let cfg = Config::from_env().expect("from_env should succeed with no env set");

        // host
        assert_eq!(cfg.host, "127.0.0.1");
        // port
        assert_eq!(cfg.port, 3200);
        // log level
        assert_eq!(cfg.log_level, "info");

        // TTLs — stored as ms so 600s → 600_000ms
        assert_eq!(cfg.session_ttl_ms, 1800 * 1000);
        assert_eq!(cfg.lock_default_ttl_ms, 600 * 1000);
        assert_eq!(cfg.lock_long_op_ttl_ms, 1800 * 1000);
        assert_eq!(cfg.lock_max_ttl_ms, 86400 * 1000);
        assert_eq!(cfg.inbox_retention_ms, 86400 * 1000);

        // db path includes .sangha/state.db
        let path_str = cfg.db_path.to_string_lossy();
        assert!(
            path_str.contains(".sangha") && path_str.ends_with("state.db"),
            "unexpected db_path: {path_str}"
        );
    });
}

#[test]
fn config_env_override_port() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prior = std::env::var("SANGHA_PORT").ok();

    unsafe { std::env::set_var("SANGHA_PORT", "4321") };
    let cfg = Config::from_env().unwrap();
    assert_eq!(cfg.port, 4321);

    unsafe {
        match prior {
            Some(v) => std::env::set_var("SANGHA_PORT", v),
            None => std::env::remove_var("SANGHA_PORT"),
        }
    }
}

#[test]
fn config_bad_env_value_falls_back_to_default() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prior = std::env::var("SANGHA_PORT").ok();

    unsafe { std::env::set_var("SANGHA_PORT", "not_a_number") };
    let cfg = Config::from_env().unwrap();
    // Should fall back to default port 3200, not panic.
    assert_eq!(cfg.port, 3200);

    unsafe {
        match prior {
            Some(v) => std::env::set_var("SANGHA_PORT", v),
            None => std::env::remove_var("SANGHA_PORT"),
        }
    }
}
