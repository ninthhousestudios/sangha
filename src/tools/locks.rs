//! Handler functions for resource lock tools.
//!
//! Each handler validates input, delegates to [`Db`], and formats output for
//! the MCP response layer.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::db::{ClaimInput, Db, ReleaseInput};
use crate::error::Result;
use crate::tools::validate;
use crate::ttl;
use crate::util::format_ms;

// ---------------------------------------------------------------------------
// Args types (deserialized from MCP tool input)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaimArgs {
    pub resource: String,
    pub scope: Option<String>,
    pub reason: Option<String>,
    pub long_op: Option<bool>,
    /// TTL in seconds (converted to ms internally).
    pub ttl_sec: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReleaseArgs {
    pub resource: String,
    pub scope: Option<String>,
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LockListArgs {
    pub project: Option<String>,
    pub scope: Option<String>,
}

// ---------------------------------------------------------------------------
// Output types (serialized to MCP tool response)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct HolderInfo {
    pub session_id: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
    /// ISO 8601 / RFC 3339 string.
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct FormattedLock {
    pub resource: String,
    pub project: String,
    pub session_id: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
    pub long_op: bool,
    /// ISO 8601 / RFC 3339 string.
    pub acquired_at: String,
    /// ISO 8601 / RFC 3339 string.
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct ClaimOutput {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_by: Option<HolderInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<FormattedLock>,
}

#[derive(Debug, Serialize)]
pub struct ReleaseOutput {
    pub ok: bool,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct LockListOutput {
    pub locks: Vec<FormattedLock>,
    pub as_of: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Attempt to claim (or renew) an advisory resource lock.
pub async fn handle_claim(
    db: &Arc<Db>,
    config: &Config,
    session_id: &str,
    session_project: &str,
    args: ClaimArgs,
) -> Result<ClaimOutput> {
    validate::non_empty("resource_claim", "resource", &args.resource)?;
    validate::max_len("resource_claim", "resource", &args.resource, validate::MAX_RESOURCE)?;
    if let Some(ref r) = args.reason {
        validate::max_len("resource_claim", "reason", r, validate::MAX_REASON)?;
    }

    let parsed_scope = validate::scope("resource_claim", args.scope.as_deref())?;
    let effective_project = validate::resolve_project(parsed_scope, session_project);

    let long_op = args.long_op.unwrap_or(false);

    // Convert ttl_sec → ms, validate against max.
    let ttl_ms_opt: Option<i64> = args.ttl_sec.map(|s| s * 1000);
    let validated_ms =
        validate::ttl_in_range("resource_claim", "ttl_sec", ttl_ms_opt, config.lock_max_ttl_ms)?;
    // ttl_in_range returns 0 for None — treat that as "use default"
    let requested_ms = if validated_ms > 0 { Some(validated_ms) } else { None };

    let effective_ttl = ttl::resolve_ttl(requested_ms, long_op, config);

    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = now + effective_ttl;

    // Inherit the branch from the session row so locks carry it for display.
    let branch = db.get_session(session_id)?.and_then(|s| s.branch);

    let input = ClaimInput {
        resource: args.resource,
        project: effective_project.to_string(),
        session_id: session_id.to_string(),
        branch,
        reason: args.reason,
        long_op,
        ttl_ms: effective_ttl,
        expires_at,
    };

    let result = db.claim_resource(input)?;

    if result.ok {
        Ok(ClaimOutput {
            ok: true,
            held_by: None,
            lock: result.lock.map(lock_to_formatted),
        })
    } else {
        let held_by = result.held_by.map(|h| HolderInfo {
            session_id: h.session_id,
            branch: h.branch,
            reason: h.reason,
            expires_at: format_ms(h.expires_at),
        });
        Ok(ClaimOutput {
            ok: false,
            held_by,
            lock: result.lock.map(lock_to_formatted),
        })
    }
}

/// Release an advisory resource lock.
pub async fn handle_release(
    db: &Arc<Db>,
    session_id: &str,
    session_project: &str,
    args: ReleaseArgs,
) -> Result<ReleaseOutput> {
    validate::non_empty("resource_release", "resource", &args.resource)?;
    validate::max_len("resource_release", "resource", &args.resource, validate::MAX_RESOURCE)?;

    let parsed_scope = validate::scope("resource_release", args.scope.as_deref())?;
    let effective_project = validate::resolve_project(parsed_scope, session_project);

    let input = ReleaseInput {
        resource: args.resource,
        project: effective_project.to_string(),
        session_id: session_id.to_string(),
        force: args.force.unwrap_or(false),
    };

    let result = db.release_resource(input)?;
    Ok(ReleaseOutput { ok: result.ok, status: result.status })
}

/// List active resource locks.
pub async fn handle_list(db: &Arc<Db>, args: LockListArgs) -> Result<LockListOutput> {
    let parsed_scope = validate::scope("resource_list", args.scope.as_deref())?;

    let effective_project: Option<String> = match parsed_scope {
        validate::Scope::User => Some(validate::USER_SCOPE_PROJECT.to_string()),
        validate::Scope::Project => args.project,
    };

    let rows = db.list_locks(effective_project.as_deref())?;
    let locks = rows.into_iter().map(lock_to_formatted).collect();
    let as_of = chrono::Utc::now().to_rfc3339();

    Ok(LockListOutput { locks, as_of })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lock_to_formatted(row: crate::db::LockRow) -> FormattedLock {
    FormattedLock {
        resource: row.resource,
        project: row.project,
        session_id: row.session_id,
        branch: row.branch,
        reason: row.reason,
        long_op: row.long_op,
        acquired_at: format_ms(row.acquired_at),
        expires_at: format_ms(row.expires_at),
    }
}
