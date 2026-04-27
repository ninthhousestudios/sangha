//! Handler functions for session presence tools.
//!
//! Each handler validates input, delegates to [`Db`], and formats output for
//! the MCP response layer. Handlers are async to fit the MCP server's async
//! call convention; the DB itself is sync (no spawn_blocking at this layer).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db::{Db, RegisterInput};
use crate::error::{Result, SanghaError};
use crate::tools::validate;
use crate::util::format_ms;

// ---------------------------------------------------------------------------
// Args types (deserialized from MCP tool input)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterArgs {
    /// Project path (required — daemon has no cwd).
    pub project: String,
    pub branch: Option<String>,
    pub intent: Option<String>,
    pub metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    pub project: Option<String>,
    pub scope: Option<String>,
}

// ---------------------------------------------------------------------------
// Output types (serialized to MCP tool response)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub project: String,
    pub branch: Option<String>,
    pub intent: Option<String>,
    pub started_at: i64,
}

#[derive(Debug, Serialize)]
pub struct RegisterOutput {
    pub session_id: String,
    pub others: Vec<SessionSummary>,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatOutput {
    pub session_id: String,
    pub ttl_remaining_sec: i64,
}

#[derive(Debug, Serialize)]
pub struct UnregisterOutput {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct FormattedSession {
    pub session_id: String,
    pub project: String,
    pub branch: Option<String>,
    pub intent: Option<String>,
    pub pid: Option<i64>,
    pub hostname: Option<String>,
    /// ISO 8601 / RFC 3339 string.
    pub started_at: String,
    /// ISO 8601 / RFC 3339 string.
    pub last_heartbeat: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ListOutput {
    pub sessions: Vec<FormattedSession>,
    /// ISO 8601 timestamp of when the list was assembled.
    pub as_of: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Register (or re-register) a session.
///
/// Validates that `project` is non-empty. Builds a [`RegisterInput`] and
/// delegates to [`Db::register_session`]. Returns the new `session_id` plus a
/// list of *other* currently-active sessions on the same project.
pub async fn handle_register(
    db: &Arc<Db>,
    session_id: Option<&str>,
    args: RegisterArgs,
) -> Result<RegisterOutput> {
    validate::non_empty("session_register", "project", &args.project)?;
    validate::max_len("session_register", "project", &args.project, validate::MAX_PROJECT)?;
    if let Some(ref b) = args.branch {
        validate::max_len("session_register", "branch", b, validate::MAX_BRANCH)?;
    }
    if let Some(ref i) = args.intent {
        validate::max_len("session_register", "intent", i, validate::MAX_INTENT)?;
    }

    // Serialise metadata to a JSON string for storage.
    let metadata_str: Option<String> = args
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| SanghaError::Internal(e.to_string()))?;
    if let Some(ref m) = metadata_str {
        validate::max_len("session_register", "metadata", m, validate::MAX_METADATA)?;
    }

    let input = RegisterInput {
        session_id: session_id.map(|s| s.to_string()),
        project: args.project.clone(),
        branch: args.branch,
        intent: args.intent,
        pid: None,
        hostname: None,
        metadata: metadata_str,
    };

    let row = db.register_session(input)?;
    let registered_id = row.id.clone();

    // Collect other sessions on the same project, excluding self.
    let all = db.list_sessions(Some(&args.project))?;
    let others = all
        .into_iter()
        .filter(|s| s.id != registered_id)
        .map(|s| SessionSummary {
            session_id: s.id,
            project: s.project,
            branch: s.branch,
            intent: s.intent,
            started_at: s.started_at,
        })
        .collect();

    Ok(RegisterOutput { session_id: registered_id, others })
}

/// Bump the heartbeat for an existing session.
///
/// Delegates to [`Db::heartbeat`], then auto-extends any locks held by the
/// session via [`Db::auto_extend_locks`]. Returns the session id and the
/// remaining TTL in whole seconds.
pub async fn handle_heartbeat(db: &Arc<Db>, session_id: &str) -> Result<HeartbeatOutput> {
    let result = db.heartbeat(session_id)?;
    db.auto_extend_locks(session_id)?;
    Ok(HeartbeatOutput {
        session_id: result.session_id,
        ttl_remaining_sec: result.ttl_remaining_ms / 1000,
    })
}

/// Unregister a session. Associated locks are removed via cascade.
pub async fn handle_unregister(db: &Arc<Db>, session_id: &str) -> Result<UnregisterOutput> {
    let result = db.unregister_session(session_id)?;
    Ok(UnregisterOutput { ok: result.ok })
}

/// List active sessions.
///
/// Parses the optional `scope` argument, resolves the effective project, and
/// returns all sessions visible under that scope. If neither `scope` nor
/// `project` is provided the list is global (all projects).
pub async fn handle_list(db: &Arc<Db>, args: ListArgs) -> Result<ListOutput> {
    let parsed_scope = validate::scope("session_list", args.scope.as_deref())?;

    // Determine the effective project filter.
    let effective_project: Option<String> = match parsed_scope {
        validate::Scope::User => Some(validate::USER_SCOPE_PROJECT.to_string()),
        validate::Scope::Project => args.project,
    };

    let rows = db.list_sessions(effective_project.as_deref())?;

    let sessions = rows.into_iter().map(session_to_formatted).collect();
    let as_of = chrono::Utc::now().to_rfc3339();

    Ok(ListOutput { sessions, as_of })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn session_to_formatted(row: crate::db::SessionRow) -> FormattedSession {
    // Parse the stored JSON string back into a Value for output.
    let metadata = row
        .metadata
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    FormattedSession {
        session_id: row.id,
        project: row.project,
        branch: row.branch,
        intent: row.intent,
        pid: row.pid,
        hostname: row.hostname,
        started_at: format_ms(row.started_at),
        last_heartbeat: format_ms(row.last_heartbeat),
        metadata,
    }
}
