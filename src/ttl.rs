//! Pure TTL helper functions — no I/O, no side effects.

use crate::config::Config;

/// Return the default TTL for a lock based on the `long_op` flag.
pub fn default_ttl(long_op: bool, config: &Config) -> i64 {
    if long_op {
        config.lock_long_op_ttl_ms
    } else {
        config.lock_default_ttl_ms
    }
}

/// Resolve the effective TTL in milliseconds.
///
/// - `None` → use `default_ttl(long_op, config)`
/// - `Some(v)` where v > 0 → use `v` (caller already validated via `validate::ttl_in_range`)
pub fn resolve_ttl(requested: Option<i64>, long_op: bool, config: &Config) -> i64 {
    match requested {
        Some(v) if v > 0 => v,
        _ => default_ttl(long_op, config),
    }
}

/// Return true if a lock is more than halfway to expiry.
pub fn is_half_expired(expires_at: i64, ttl_ms: i64, now: i64) -> bool {
    let remaining = expires_at - now;
    remaining < (ttl_ms / 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::config::Config;

    fn test_config() -> Config {
        Config {
            db_path: PathBuf::from(":memory:"),
            host: "127.0.0.1".to_string(),
            port: 0,
            session_ttl_ms: 600_000,
            lock_default_ttl_ms: 600_000,
            lock_long_op_ttl_ms: 1_800_000,
            lock_max_ttl_ms: 86_400_000,
            inbox_retention_ms: 86_400_000,
            log_level: "info".to_string(),
        }
    }

    #[test]
    fn default_ttl_short_op() {
        let cfg = test_config();
        assert_eq!(default_ttl(false, &cfg), 600_000);
    }

    #[test]
    fn default_ttl_long_op() {
        let cfg = test_config();
        assert_eq!(default_ttl(true, &cfg), 1_800_000);
    }

    #[test]
    fn resolve_ttl_uses_requested_when_positive() {
        let cfg = test_config();
        assert_eq!(resolve_ttl(Some(12_000), false, &cfg), 12_000);
        assert_eq!(resolve_ttl(Some(99_999), true, &cfg), 99_999);
    }

    #[test]
    fn resolve_ttl_falls_back_on_none() {
        let cfg = test_config();
        assert_eq!(resolve_ttl(None, false, &cfg), 600_000);
        assert_eq!(resolve_ttl(None, true, &cfg), 1_800_000);
    }

    #[test]
    fn resolve_ttl_falls_back_on_zero_or_negative() {
        let cfg = test_config();
        assert_eq!(resolve_ttl(Some(0), false, &cfg), 600_000);
        assert_eq!(resolve_ttl(Some(-1), false, &cfg), 600_000);
    }

    #[test]
    fn is_half_expired_true_when_past_halfway() {
        // ttl=1000, expires_at=1000, now=600 → remaining=400 < 500 → true
        assert!(is_half_expired(1000, 1000, 600));
    }

    #[test]
    fn is_half_expired_false_when_fresh() {
        // ttl=1000, expires_at=1000, now=100 → remaining=900 >= 500 → false
        assert!(!is_half_expired(1000, 1000, 100));
    }

    #[test]
    fn is_half_expired_exactly_at_half() {
        // ttl=1000, expires_at=1000, now=500 → remaining=500, NOT < 500 → false
        assert!(!is_half_expired(1000, 1000, 500));
    }
}
