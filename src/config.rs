//! Environment-driven configuration.
//!
//! Loaded once at startup via [`Config::from_env`]. No file formats, no
//! runtime reconfiguration — restart to change settings.

use std::path::PathBuf;
use std::str::FromStr;

use crate::error::{Result, SanghaError};

#[derive(Debug, Clone)]
pub struct Config {
    pub db_path: PathBuf,
    pub host: String,
    pub port: u16,
    /// Session TTL stored as milliseconds.
    pub session_ttl_ms: i64,
    /// Default advisory lock TTL in milliseconds.
    pub lock_default_ttl_ms: i64,
    /// Long-operation advisory lock TTL in milliseconds.
    pub lock_long_op_ttl_ms: i64,
    /// Maximum advisory lock TTL in milliseconds.
    pub lock_max_ttl_ms: i64,
    /// Inbox message retention in milliseconds.
    pub inbox_retention_ms: i64,
    pub log_level: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let db_path = std::env::var("SANGHA_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_db_path());

        let host =
            std::env::var("SANGHA_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

        let port: u16 = parse_env_or("SANGHA_PORT", 3200);

        // Parse all the _SEC env vars (seconds) and convert to ms.
        let session_ttl_ms: i64 = parse_env_or::<i64>("SANGHA_SESSION_TTL_SEC", 1800) * 1000;
        let lock_default_ttl_ms: i64 =
            parse_env_or::<i64>("SANGHA_LOCK_TTL_SEC", 600) * 1000;
        let lock_long_op_ttl_ms: i64 =
            parse_env_or::<i64>("SANGHA_LOCK_LONG_OP_TTL_SEC", 1800) * 1000;
        let lock_max_ttl_ms: i64 =
            parse_env_or::<i64>("SANGHA_LOCK_MAX_TTL_SEC", 86400) * 1000;
        let inbox_retention_ms: i64 =
            parse_env_or::<i64>("SANGHA_INBOX_RETENTION_SEC", 86400) * 1000;

        let log_level =
            std::env::var("SANGHA_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        Ok(Self {
            db_path,
            host,
            port,
            session_ttl_ms,
            lock_default_ttl_ms,
            lock_long_op_ttl_ms,
            lock_max_ttl_ms,
            inbox_retention_ms,
            log_level,
        })
    }
}

fn default_db_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".sangha/state.db");
        p
    } else {
        PathBuf::from(".sangha/state.db")
    }
}

/// Parse an env var as `T`, returning `default` on missing or unparseable values.
/// Never panics.
fn parse_env_or<T: FromStr + std::fmt::Display>(name: &str, default: T) -> T {
    match std::env::var(name) {
        Err(_) => default,
        Ok(v) => match v.parse() {
            Ok(parsed) => parsed,
            Err(_) => {
                eprintln!(
                    "WARNING: {name}={v:?} is not a valid {ty} — using default {default}",
                    ty = std::any::type_name::<T>(),
                );
                default
            }
        },
    }
}

// Silence unused-import warning when SanghaError isn't used directly in this file.
const _: fn() = || {
    let _: Option<SanghaError> = None;
};
