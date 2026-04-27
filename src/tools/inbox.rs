//! Handler functions for inbox broadcast tools.
//!
//! Each handler validates input, delegates to [`Db`], and formats output for
//! the MCP response layer.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db::{BroadcastInput, Db, ReadInboxInput};
use crate::error::Result;
use crate::tools::validate;
use crate::util::format_ms;

// ---------------------------------------------------------------------------
// Args types (deserialized from MCP tool input)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BroadcastArgs {
    pub message: String,
    pub tags: Option<Vec<String>>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadInboxArgs {
    pub unread_only: Option<bool>,
    pub limit: Option<i64>,
    pub scope: Option<String>,
}

// ---------------------------------------------------------------------------
// Output types (serialized to MCP tool response)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FormattedInbox {
    pub id: i64,
    pub from_session: String,
    pub from_branch: Option<String>,
    pub message: String,
    pub tags: Option<Vec<String>>,
    /// ISO 8601 / RFC 3339 string.
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct BroadcastOutput {
    pub ok: bool,
    pub message_id: i64,
}

#[derive(Debug, Serialize)]
pub struct ReadInboxOutput {
    pub count: i64,
    pub unread_total: i64,
    pub total: i64,
    pub messages: Vec<FormattedInbox>,
    /// ISO 8601 timestamp of when the inbox was read.
    pub as_of: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Broadcast a message to all other sessions in the same project (or user scope).
pub async fn handle_broadcast(
    db: &Arc<Db>,
    session_id: &str,
    session_project: &str,
    args: BroadcastArgs,
) -> Result<BroadcastOutput> {
    validate::non_empty("inbox_broadcast", "message", &args.message)?;

    let parsed_scope = validate::scope("inbox_broadcast", args.scope.as_deref())?;
    let effective_project = validate::resolve_project(parsed_scope, session_project);

    let tags_json: Option<String> =
        args.tags.map(|t| serde_json::to_string(&t).unwrap_or_default());

    let from_branch = db
        .get_session(session_id)?
        .and_then(|s| s.branch);

    let input = BroadcastInput {
        project: effective_project.to_string(),
        from_session: session_id.to_string(),
        from_branch,
        message: args.message,
        tags: tags_json,
    };

    let row = db.broadcast(input)?;
    Ok(BroadcastOutput { ok: true, message_id: row.id })
}

/// Read inbox messages for the current session's project (or user scope).
pub async fn handle_read_inbox(
    db: &Arc<Db>,
    session_id: &str,
    session_project: &str,
    args: ReadInboxArgs,
) -> Result<ReadInboxOutput> {
    let parsed_scope = validate::scope("inbox_read", args.scope.as_deref())?;
    let effective_project = validate::resolve_project(parsed_scope, session_project);

    let input = ReadInboxInput {
        project: effective_project.to_string(),
        session_id: session_id.to_string(),
        unread_only: args.unread_only.unwrap_or(true),
        limit: args.limit,
    };

    let result = db.read_inbox(input)?;

    let messages = result
        .messages
        .into_iter()
        .map(|row| FormattedInbox {
            id: row.id,
            from_session: row.from_session,
            from_branch: row.from_branch,
            message: row.message,
            tags: row.tags.as_deref().and_then(|s| serde_json::from_str(s).ok()),
            created_at: format_ms(row.created_at),
        })
        .collect();

    let as_of = chrono::Utc::now().to_rfc3339();

    Ok(ReadInboxOutput {
        count: result.count,
        unread_total: result.unread_total,
        total: result.total,
        messages,
        as_of,
    })
}

