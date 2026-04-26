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

    match cli.command.unwrap_or(Commands::Serve { stdio: false }) {
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
