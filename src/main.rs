use std::sync::Arc;

use clap::{Parser, Subcommand};
use sangha::config::Config;
use sangha::db::Db;

#[derive(Parser)]
#[command(name = "sangha", about = "Session coordination daemon for manas")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the sangha server (default)
    Serve {
        /// Use stdio transport instead of HTTP (for testing / mcpjungle)
        #[arg(long)]
        stdio: bool,
    },
    /// Show active sessions
    Status {
        #[arg(long)]
        project: Option<String>,
        /// Filter to user-scoped sessions
        #[arg(long)]
        user: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show active resource locks
    Locks {
        #[arg(long)]
        project: Option<String>,
        /// Filter to user-scoped locks
        #[arg(long)]
        user: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Clear sessions or force-release locks
    Clear {
        /// Force-release a specific resource lock
        #[arg(long)]
        force_release: Option<String>,
        /// Clear all sessions and locks
        #[arg(long)]
        all: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Print the database path
    Path,
    /// Check daemon and database health
    Health,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    let config = Arc::new(Config::from_env()?);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level)),
        )
        .init();

    match cli.command.unwrap_or(Commands::Status {
        project: None,
        user: false,
        json: false,
    }) {
        Commands::Serve { stdio } => {
            let db = Arc::new(Db::open(&config)?);
            db.run_migrations()?;
            db.prune_all()?;

            if stdio {
                serve_stdio(db, config).await?;
            } else {
                serve_http(db, config).await?;
            }
        }

        Commands::Status { project, user, json } => {
            let db = Arc::new(Db::open(&config)?);
            db.run_migrations()?;

            let effective_project = if user {
                Some(sangha::tools::validate::USER_SCOPE_PROJECT.to_string())
            } else {
                project
            };

            let sessions = db.list_sessions(effective_project.as_deref())?;

            if json {
                let formatted: Vec<serde_json::Value> = sessions
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "session_id": s.id,
                            "project": s.project,
                            "branch": s.branch,
                            "intent": s.intent,
                            "pid": s.pid,
                            "hostname": s.hostname,
                            "started_at": s.started_at,
                            "last_heartbeat": s.last_heartbeat,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&formatted)?);
            } else if sessions.is_empty() {
                println!("No active sessions.");
            } else {
                println!("{} active session(s):\n", sessions.len());
                for s in &sessions {
                    println!("  {} ({})", s.id, s.project);
                    if let Some(b) = &s.branch {
                        println!("    branch: {b}");
                    }
                    if let Some(i) = &s.intent {
                        println!("    intent: {i}");
                    }
                    println!();
                }
            }
        }

        Commands::Locks { project, user, json } => {
            let db = Arc::new(Db::open(&config)?);
            db.run_migrations()?;

            let effective_project = if user {
                Some(sangha::tools::validate::USER_SCOPE_PROJECT.to_string())
            } else {
                project
            };

            let locks = db.list_locks(effective_project.as_deref())?;

            if json {
                let formatted: Vec<serde_json::Value> = locks
                    .iter()
                    .map(|l| {
                        serde_json::json!({
                            "resource": l.resource,
                            "project": l.project,
                            "session_id": l.session_id,
                            "branch": l.branch,
                            "reason": l.reason,
                            "long_op": l.long_op,
                            "acquired_at": l.acquired_at,
                            "expires_at": l.expires_at,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&formatted)?);
            } else if locks.is_empty() {
                println!("No active locks.");
            } else {
                println!("{} active lock(s):\n", locks.len());
                for l in &locks {
                    println!("  {} (held by {})", l.resource, l.session_id);
                    println!("    project: {}", l.project);
                    if let Some(r) = &l.reason {
                        println!("    reason: {r}");
                    }
                    if l.long_op {
                        println!("    long_op: true");
                    }
                    println!();
                }
            }
        }

        Commands::Clear {
            force_release,
            all,
            json,
        } => {
            let db = Arc::new(Db::open(&config)?);
            db.run_migrations()?;

            if let Some(resource) = force_release {
                let locks = db.list_locks(None)?;
                let mut released = 0;
                for lock in &locks {
                    if lock.resource == resource {
                        let input = sangha::db::ReleaseInput {
                            resource: lock.resource.clone(),
                            project: lock.project.clone(),
                            session_id: lock.session_id.clone(),
                            force: true,
                        };
                        db.release_resource(input)?;
                        released += 1;
                    }
                }
                if json {
                    println!("{}", serde_json::json!({ "released": released }));
                } else {
                    println!(
                        "Force-released {released} lock(s) for resource '{resource}'."
                    );
                }
            } else if all {
                let sessions = db.list_sessions(None)?;
                let count = sessions.len();
                for s in &sessions {
                    db.unregister_session(&s.id)?;
                }
                if json {
                    println!("{}", serde_json::json!({ "cleared_sessions": count }));
                } else {
                    println!("Cleared {count} session(s) and their locks.");
                }
            } else {
                eprintln!("Usage: sangha clear --force-release <resource> | --all");
                std::process::exit(1);
            }
        }

        Commands::Path => {
            println!("{}", config.db_path.display());
        }

        Commands::Health => {
            let db_ok = Db::open(&config).is_ok();

            let addr = format!("{}:{}", config.host, config.port);
            let daemon_running = tokio::net::TcpStream::connect(&addr).await.is_ok();

            if let Ok(db) = Db::open(&config) {
                let _ = db.run_migrations();
                let sessions = db.list_sessions(None).unwrap_or_default();
                let locks = db.list_locks(None).unwrap_or_default();

                println!(
                    "daemon:   {}",
                    if daemon_running { "running" } else { "not running" }
                );
                println!("database: {}", if db_ok { "ok" } else { "error" });
                println!("sessions: {}", sessions.len());
                println!("locks:    {}", locks.len());
                println!("db path:  {}", config.db_path.display());
            } else {
                println!(
                    "daemon:   {}",
                    if daemon_running { "running" } else { "not running" }
                );
                println!("database: error");
            }
        }
    }

    Ok(())
}

async fn serve_stdio(
    db: Arc<Db>,
    config: Arc<Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::transport::stdio;
    use rmcp::ServiceExt;
    use sangha::mcp::SanghaServer;

    let server = SanghaServer::new(db, config);
    let (stdin, stdout) = stdio();
    let service = server.serve((stdin, stdout)).await?;

    tokio::select! {
        res = service.waiting() => { res?; }
        _ = shutdown_signal() => { tracing::info!("shutdown signal received"); }
    }
    Ok(())
}

async fn serve_http(
    db: Arc<Db>,
    config: Arc<Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    use axum::routing::any_service;
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager,
        tower::{StreamableHttpServerConfig, StreamableHttpService},
    };
    use sangha::mcp::SanghaServer;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    let addr = format!("{}:{}", config.host, config.port);

    // Check if already running.
    if tokio::net::TcpStream::connect(&addr).await.is_ok() {
        eprintln!("sangha already running on {addr}");
        std::process::exit(0);
    }

    // Write PID file next to the database.
    let pid_path = config
        .db_path
        .parent()
        .map(|p| p.join("sangha.pid"))
        .unwrap_or_else(|| std::path::PathBuf::from("sangha.pid"));
    std::fs::write(&pid_path, std::process::id().to_string())?;

    let cancel = CancellationToken::new();
    let session_manager = Arc::new(LocalSessionManager::default());
    let shttp_config =
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.clone());

    let db_clone = db.clone();
    let cfg_clone = config.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(SanghaServer::new(db_clone.clone(), cfg_clone.clone())),
        session_manager,
        shttp_config,
    );

    #[allow(deprecated)]
    let app = axum::Router::new().route("/mcp", any_service(mcp_service));

    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("sangha listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    let _ = std::fs::remove_file(&pid_path);
    tracing::info!("sangha shut down");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
}
