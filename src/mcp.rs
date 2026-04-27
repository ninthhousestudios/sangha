//! MCP server wiring — maps rmcp tool dispatch to handler functions.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use rmcp::handler::server::router::tool::ToolRouter;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::Config;
use crate::db::Db;
use crate::error::SanghaError;
use crate::identity::Identity;
use crate::tools::{inbox, locks, presence};

// ---------------------------------------------------------------------------
// Empty args for tools that take no client input
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmptyArgs {}

// ---------------------------------------------------------------------------
// SanghaServer
// ---------------------------------------------------------------------------

pub struct SanghaServer {
    db: Arc<Db>,
    identity: Identity,
    config: Arc<Config>,
    tool_router: ToolRouter<Self>,
}

impl Clone for SanghaServer {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            identity: Identity::new(),
            config: Arc::clone(&self.config),
            tool_router: Self::tool_router(),
        }
    }
}

impl SanghaServer {
    pub fn new(db: Arc<Db>, config: Arc<Config>) -> Self {
        Self {
            db,
            identity: Identity::new(),
            config,
            tool_router: Self::tool_router(),
        }
    }
}

impl Drop for SanghaServer {
    fn drop(&mut self) {
        if let Ok(sid) = self.identity.session_id() {
            let _ = self.db.unregister_session(sid);
        }
    }
}

// ---------------------------------------------------------------------------
// Tool methods
// ---------------------------------------------------------------------------

#[tool_router(router = tool_router)]
impl SanghaServer {
    #[tool(description = "Register or re-register a session on this connection. \
        Call this first — all other tools require a bound session.")]
    pub async fn session_register(
        &self,
        Parameters(args): Parameters<presence::RegisterArgs>,
    ) -> Result<String, ErrorData> {
        let existing_id = self.identity.session_id().ok().map(|s| s.to_string());
        let project = args.project.clone();

        let out = presence::handle_register(&self.db, existing_id.as_deref(), args)
            .await
            .map_err(sangha_to_rmcp)?;

        self.identity
            .bind(out.session_id.clone(), project)
            .map_err(sangha_to_rmcp)?;

        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "Send a heartbeat for the current session. \
        Extends the session TTL and auto-renews any locks past half-life.")]
    pub async fn session_heartbeat(
        &self,
        #[allow(unused_variables)]
        Parameters(_args): Parameters<EmptyArgs>,
    ) -> Result<String, ErrorData> {
        let sid = self.identity.session_id().map_err(sangha_to_rmcp)?;
        let out = presence::handle_heartbeat(&self.db, sid)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "Unregister the current session. \
        Releases all held locks via cascade.")]
    pub async fn session_unregister(
        &self,
        #[allow(unused_variables)]
        Parameters(_args): Parameters<EmptyArgs>,
    ) -> Result<String, ErrorData> {
        let sid = self.identity.session_id().map_err(sangha_to_rmcp)?;
        let out = presence::handle_unregister(&self.db, sid)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "List active sessions, optionally filtered by project or scope.")]
    pub async fn session_list(
        &self,
        Parameters(args): Parameters<presence::ListArgs>,
    ) -> Result<String, ErrorData> {
        let out = presence::handle_list(&self.db, args)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "Claim (or renew) an advisory resource lock. \
        Returns held_by info if another session holds the resource.")]
    pub async fn resource_claim(
        &self,
        Parameters(args): Parameters<locks::ClaimArgs>,
    ) -> Result<String, ErrorData> {
        let sid = self.identity.session_id().map_err(sangha_to_rmcp)?;
        let proj = self.identity.project().map_err(sangha_to_rmcp)?;
        let out = locks::handle_claim(&self.db, &self.config, sid, proj, args)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "Release an advisory resource lock.")]
    pub async fn resource_release(
        &self,
        Parameters(args): Parameters<locks::ReleaseArgs>,
    ) -> Result<String, ErrorData> {
        let sid = self.identity.session_id().map_err(sangha_to_rmcp)?;
        let proj = self.identity.project().map_err(sangha_to_rmcp)?;
        let out = locks::handle_release(&self.db, sid, proj, args)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "List active resource locks, optionally filtered by project or scope.")]
    pub async fn resource_list(
        &self,
        Parameters(args): Parameters<locks::LockListArgs>,
    ) -> Result<String, ErrorData> {
        let out = locks::handle_list(&self.db, args)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "Broadcast a message to all other sessions in the same project.")]
    pub async fn broadcast(
        &self,
        Parameters(args): Parameters<inbox::BroadcastArgs>,
    ) -> Result<String, ErrorData> {
        let sid = self.identity.session_id().map_err(sangha_to_rmcp)?;
        let proj = self.identity.project().map_err(sangha_to_rmcp)?;
        let out = inbox::handle_broadcast(&self.db, sid, proj, args)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }

    #[tool(description = "Read inbox messages for the current session's project. \
        Marks returned messages as read.")]
    pub async fn read_inbox(
        &self,
        Parameters(args): Parameters<inbox::ReadInboxArgs>,
    ) -> Result<String, ErrorData> {
        let sid = self.identity.session_id().map_err(sangha_to_rmcp)?;
        let proj = self.identity.project().map_err(sangha_to_rmcp)?;
        let out = inbox::handle_read_inbox(&self.db, sid, proj, args)
            .await
            .map_err(sangha_to_rmcp)?;
        serde_json::to_string_pretty(&out).map_err(json_to_rmcp)
    }
}

// ---------------------------------------------------------------------------
// ServerHandler
// ---------------------------------------------------------------------------

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SanghaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "sangha v0.1.0 — session coordination daemon. \
                 Tools: session_register, session_heartbeat, session_unregister, \
                 session_list, resource_claim, resource_release, resource_list, \
                 broadcast, read_inbox. \
                 Call session_register first to bind this connection to a session.",
            )
    }
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn sangha_to_rmcp(e: SanghaError) -> ErrorData {
    let data = e.data();
    ErrorData::new(
        rmcp::model::ErrorCode(e.code()),
        e.message(),
        Some(serde_json::to_value(data).unwrap_or_default()),
    )
}

fn json_to_rmcp(e: serde_json::Error) -> ErrorData {
    ErrorData::new(
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        format!("JSON serialization failed: {e}"),
        None,
    )
}
