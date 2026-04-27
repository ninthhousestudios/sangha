//! Small shared helpers.

/// Format a Unix-millisecond timestamp as an RFC 3339 string. Falls back to
/// the raw integer if the value is out of range for `DateTime`.
pub fn format_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ms.to_string())
}
