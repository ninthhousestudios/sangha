//! Actionable errors (mirrors chitta's pattern).
//!
//! Every [`SanghaError`] variant carries enough context to build a JSON-RPC
//! error `data` object that names the tool, the violated constraint, and a
//! next action the caller can take.

use serde::Serialize;
use thiserror::Error;

/// Canonical JSON-RPC `data` payload for every error returned to the wire.
/// Three fields are always populated: `tool`, `constraint`, `next_action`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ErrorData {
    pub tool: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
    pub constraint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received: Option<serde_json::Value>,
    pub next_action: String,
}

/// JSON-RPC error codes per the MCP / JSON-RPC 2.0 spec.
pub mod codes {
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

pub type Result<T> = std::result::Result<T, SanghaError>;

#[derive(Debug, Error)]
pub enum SanghaError {
    #[error("missing config: {name}")]
    MissingConfig { name: &'static str, next_action: &'static str },

    #[error("[{tool}] invalid argument `{argument}`: {constraint}")]
    InvalidArgument {
        tool: &'static str,
        argument: &'static str,
        constraint: String,
        received: Option<String>,
        next_action: String,
    },

    #[error("[{tool}] not found: {kind}")]
    NotFound {
        tool: &'static str,
        kind: String,
        next_action: String,
    },

    #[error("[{tool}] identity error: {message}")]
    IdentityError {
        tool: &'static str,
        message: String,
        next_action: String,
    },

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl SanghaError {
    /// JSON-RPC error code to use on the wire.
    pub fn code(&self) -> i32 {
        match self {
            Self::MissingConfig { .. }
            | Self::InvalidArgument { .. }
            | Self::NotFound { .. }
            | Self::IdentityError { .. } => codes::INVALID_PARAMS,
            Self::Db(_) | Self::Internal(_) => codes::INTERNAL_ERROR,
        }
    }

    /// Short human-readable message for the JSON-RPC `message` field.
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// Structured payload for the JSON-RPC `data` field. Every variant
    /// populates `tool`, `constraint`, `next_action`.
    pub fn data(&self) -> ErrorData {
        match self {
            Self::MissingConfig { name, next_action } => ErrorData {
                tool: "startup",
                argument: Some((*name).to_string()),
                constraint: format!("environment variable `{name}` must be set"),
                received: None,
                next_action: (*next_action).to_string(),
            },
            Self::InvalidArgument {
                tool,
                argument,
                constraint,
                received,
                next_action,
            } => ErrorData {
                tool,
                argument: Some((*argument).to_string()),
                constraint: constraint.clone(),
                received: received.as_deref().map(|s| serde_json::Value::String(s.to_string())),
                next_action: next_action.clone(),
            },
            Self::NotFound { tool, kind, next_action } => ErrorData {
                tool,
                argument: None,
                constraint: format!("{kind} exists"),
                received: None,
                next_action: next_action.clone(),
            },
            Self::IdentityError { tool, message, next_action } => ErrorData {
                tool,
                argument: None,
                constraint: "caller identity is resolvable".to_string(),
                received: Some(serde_json::json!({ "message": message })),
                next_action: next_action.clone(),
            },
            Self::Db(e) => ErrorData {
                tool: "database",
                argument: None,
                constraint: "database query succeeds".to_string(),
                received: Some(serde_json::json!({ "message": e.to_string() })),
                next_action: db_next_action(e),
            },
            Self::Internal(msg) => ErrorData {
                tool: "server",
                argument: None,
                constraint: "server completes the request without an internal fault".to_string(),
                received: Some(serde_json::json!({ "message": msg })),
                next_action: "Report this as a bug; include server logs.".to_string(),
            },
        }
    }
}

fn db_next_action(e: &rusqlite::Error) -> String {
    match e {
        rusqlite::Error::SqliteFailure(_, _) => {
            "The database rejected the query (constraint, permission, or schema issue). \
             Inspect the message, correct the input or schema, and retry."
                .to_string()
        }
        rusqlite::Error::QueryReturnedNoRows => {
            "The expected row was absent. If the caller passed an id, verify it; \
             otherwise report as a bug."
                .to_string()
        }
        _ => {
            "Retry the request. If the error repeats, check server logs for detail and \
             report as a bug if the cause is unclear."
                .to_string()
        }
    }
}
