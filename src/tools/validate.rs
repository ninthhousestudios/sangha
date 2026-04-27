//! Shared argument validation for all sangha tools.
//!
//! Each function returns an [`InvalidArgument`](crate::error::SanghaError::InvalidArgument)
//! with a populated `constraint` + `next_action` on failure.

use crate::error::{Result, SanghaError};

/// Return `InvalidArgument` if `val` is empty.
pub fn non_empty(tool: &'static str, arg: &'static str, val: &str) -> Result<()> {
    if val.is_empty() {
        return Err(SanghaError::InvalidArgument {
            tool,
            argument: arg,
            constraint: "must be non-empty".to_string(),
            received: Some(String::new()),
            next_action: format!("Pass a non-empty value for `{arg}`."),
        });
    }
    Ok(())
}

/// Return `InvalidArgument` if `val.len() > max`.
pub fn max_len(tool: &'static str, arg: &'static str, val: &str, max: usize) -> Result<()> {
    if val.len() > max {
        return Err(SanghaError::InvalidArgument {
            tool,
            argument: arg,
            constraint: format!("length <= {max} bytes"),
            received: Some(val.len().to_string()),
            next_action: format!("Shorten `{arg}` to at most {max} bytes."),
        });
    }
    Ok(())
}

/// Return `InvalidArgument` if `val <= 0`.
pub fn positive_i64(tool: &'static str, arg: &'static str, val: i64) -> Result<()> {
    if val <= 0 {
        return Err(SanghaError::InvalidArgument {
            tool,
            argument: arg,
            constraint: "> 0".to_string(),
            received: Some(val.to_string()),
            next_action: format!("Pass a positive integer for `{arg}`."),
        });
    }
    Ok(())
}

/// Validate an optional TTL (milliseconds).
///
/// - `None`  → `Ok(0)` (caller should use its own default)
/// - `Some(v)` where v <= 0 → `InvalidArgument`
/// - `Some(v)` where v > max_ms → `InvalidArgument`
/// - otherwise → `Ok(v)`
pub fn ttl_in_range(
    tool: &'static str,
    arg: &'static str,
    val: Option<i64>,
    max_ms: i64,
) -> Result<i64> {
    match val {
        None => Ok(0),
        Some(v) => {
            if v <= 0 {
                return Err(SanghaError::InvalidArgument {
                    tool,
                    argument: arg,
                    constraint: "> 0".to_string(),
                    received: Some(v.to_string()),
                    next_action: format!(
                        "Pass a positive value for `{arg}`, or omit to use the default."
                    ),
                });
            }
            if v > max_ms {
                return Err(SanghaError::InvalidArgument {
                    tool,
                    argument: arg,
                    constraint: format!("<= {max_ms} ms"),
                    received: Some(v.to_string()),
                    next_action: format!(
                        "Pass `{arg}` <= {max_ms} ms, or omit to use the default."
                    ),
                });
            }
            Ok(v)
        }
    }
}

/// Scope of a resource: tied to a project or to the current user across projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    User,
}

/// Sentinel project identifier used when scope is `User`.
pub const USER_SCOPE_PROJECT: &str = "__user__";

/// Parse an optional scope string (`"project"` | `"user"`, case-insensitive).
/// `None` defaults to `Project`.
pub fn scope(tool: &'static str, val: Option<&str>) -> Result<Scope> {
    match val {
        None | Some("") => Ok(Scope::Project),
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "project" => Ok(Scope::Project),
            "user" => Ok(Scope::User),
            _ => Err(SanghaError::InvalidArgument {
                tool,
                argument: "scope",
                constraint: r#"one of "project" or "user""#.to_string(),
                received: Some(s.to_string()),
                next_action: r#"Pass scope="project" (default) or scope="user"."#.to_string(),
            }),
        },
    }
}

/// Return the effective project key for a given scope.
///
/// - `Project` → `session_project` (whatever project this session belongs to)
/// - `User`    → [`USER_SCOPE_PROJECT`] sentinel
pub fn resolve_project(scope: Scope, session_project: &str) -> &str {
    match scope {
        Scope::Project => session_project,
        Scope::User => USER_SCOPE_PROJECT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_ok() {
        assert!(non_empty("t", "name", "hello").is_ok());
    }

    #[test]
    fn non_empty_err() {
        assert!(non_empty("t", "name", "").is_err());
    }

    #[test]
    fn max_len_ok() {
        assert!(max_len("t", "x", "abc", 10).is_ok());
        assert!(max_len("t", "x", "abc", 3).is_ok());
    }

    #[test]
    fn max_len_err() {
        assert!(max_len("t", "x", "abcd", 3).is_err());
    }

    #[test]
    fn positive_i64_ok() {
        assert!(positive_i64("t", "n", 1).is_ok());
    }

    #[test]
    fn positive_i64_err() {
        assert!(positive_i64("t", "n", 0).is_err());
        assert!(positive_i64("t", "n", -5).is_err());
    }

    #[test]
    fn ttl_none_returns_zero() {
        assert_eq!(ttl_in_range("t", "ttl", None, 10_000).unwrap(), 0);
    }

    #[test]
    fn ttl_valid() {
        assert_eq!(ttl_in_range("t", "ttl", Some(5000), 10_000).unwrap(), 5000);
    }

    #[test]
    fn ttl_zero_err() {
        assert!(ttl_in_range("t", "ttl", Some(0), 10_000).is_err());
    }

    #[test]
    fn ttl_negative_err() {
        assert!(ttl_in_range("t", "ttl", Some(-1), 10_000).is_err());
    }

    #[test]
    fn ttl_over_max_err() {
        assert!(ttl_in_range("t", "ttl", Some(10_001), 10_000).is_err());
    }

    #[test]
    fn ttl_at_max_ok() {
        assert_eq!(ttl_in_range("t", "ttl", Some(10_000), 10_000).unwrap(), 10_000);
    }

    #[test]
    fn scope_defaults_to_project() {
        assert_eq!(scope("t", None).unwrap(), Scope::Project);
        assert_eq!(scope("t", Some("")).unwrap(), Scope::Project);
    }

    #[test]
    fn scope_project_variants() {
        assert_eq!(scope("t", Some("project")).unwrap(), Scope::Project);
        assert_eq!(scope("t", Some("PROJECT")).unwrap(), Scope::Project);
        assert_eq!(scope("t", Some("Project")).unwrap(), Scope::Project);
    }

    #[test]
    fn scope_user_variants() {
        assert_eq!(scope("t", Some("user")).unwrap(), Scope::User);
        assert_eq!(scope("t", Some("USER")).unwrap(), Scope::User);
    }

    #[test]
    fn scope_invalid_err() {
        assert!(scope("t", Some("global")).is_err());
    }

    #[test]
    fn resolve_project_project_scope() {
        assert_eq!(resolve_project(Scope::Project, "my-project"), "my-project");
    }

    #[test]
    fn resolve_project_user_scope() {
        assert_eq!(resolve_project(Scope::User, "my-project"), USER_SCOPE_PROJECT);
    }
}
