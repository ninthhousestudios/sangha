use std::sync::Arc;

use sangha::config::Config;
use sangha::db::Db;

pub fn test_config() -> Arc<Config> {
    Arc::new(Config {
        db_path: std::path::PathBuf::from(":memory:"),
        host: "127.0.0.1".to_string(),
        port: 0,
        session_ttl_ms: 600_000,
        lock_default_ttl_ms: 600_000,
        lock_long_op_ttl_ms: 1_800_000,
        lock_max_ttl_ms: 86_400_000,
        inbox_retention_ms: 86_400_000,
        log_level: "info".to_string(),
    })
}

pub fn test_db() -> Arc<Db> {
    let config = test_config();
    let db = Db::open_memory(&config).expect("open in-memory db");
    db.run_migrations().expect("run migrations");
    Arc::new(db)
}

pub fn unique_project(name: &str) -> String {
    format!("{}-{}", name, uuid::Uuid::now_v7())
}
