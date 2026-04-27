use crate::error::{Result, SanghaError};

/// Connection-bound session identity.
///
/// Each HTTP connection (= one CC session) gets its own `Identity` instance.
/// The first `session_register` call binds the identity for the lifetime of
/// that connection. All subsequent tool calls derive session_id and project
/// from this struct.
pub struct Identity {
    session_id: tokio::sync::OnceCell<String>,
    project: tokio::sync::OnceCell<String>,
}

impl Identity {
    pub fn new() -> Self {
        Self {
            session_id: tokio::sync::OnceCell::new(),
            project: tokio::sync::OnceCell::new(),
        }
    }

    /// Bind session_id and project. Idempotent if the same values are passed.
    /// Returns IdentityError if called with different values.
    pub fn bind(&self, session_id: String, project: String) -> Result<()> {
        match self.session_id.set(session_id.clone()) {
            Ok(()) => {
                // First bind — also set project
                let _ = self.project.set(project);
                Ok(())
            }
            Err(_) => {
                // Already set — check if same values (idempotent on both)
                if let Some(existing) = self.session_id.get()
                    && existing != &session_id
                {
                    return Err(SanghaError::IdentityError {
                        tool: "session_register",
                        message: format!(
                            "connection already bound to session {existing}, cannot rebind to {session_id}"
                        ),
                        next_action: "Use the existing session or open a new HTTP connection."
                            .to_string(),
                    });
                }
                if let Some(existing_project) = self.project.get()
                    && existing_project != &project
                {
                    return Err(SanghaError::IdentityError {
                        tool: "session_register",
                        message: format!(
                            "connection already bound to project {existing_project}, cannot rebind to {project}"
                        ),
                        next_action: "Use the existing session or open a new HTTP connection."
                            .to_string(),
                    });
                }
                Ok(())
            }
        }
    }

    /// Get the bound session_id. Errors if not yet bound.
    pub fn session_id(&self) -> Result<&str> {
        self.session_id.get().map(|s| s.as_str()).ok_or_else(|| SanghaError::IdentityError {
            tool: "identity",
            message: "no session bound to this connection".to_string(),
            next_action: "Call session_register first to bind a session to this connection."
                .to_string(),
        })
    }

    /// Get the bound project. Errors if not yet bound.
    pub fn project(&self) -> Result<&str> {
        self.project.get().map(|s| s.as_str()).ok_or_else(|| SanghaError::IdentityError {
            tool: "identity",
            message: "no session bound to this connection".to_string(),
            next_action: "Call session_register first to bind a session to this connection."
                .to_string(),
        })
    }

    /// Whether this identity has been bound.
    pub fn is_bound(&self) -> bool {
        self.session_id.get().is_some()
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::new()
    }
}
