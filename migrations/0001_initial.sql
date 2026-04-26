-- NOTE: PRAGMAs are set per-connection in Db::open(), NOT here.

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT,
    intent TEXT,
    pid INTEGER,
    hostname TEXT,
    started_at INTEGER NOT NULL,
    last_heartbeat INTEGER NOT NULL,
    metadata TEXT
);
CREATE INDEX idx_sessions_project ON sessions(project);
CREATE INDEX idx_sessions_heartbeat ON sessions(last_heartbeat);

CREATE TABLE resource_locks (
    resource TEXT NOT NULL,
    project TEXT NOT NULL,
    session_id TEXT NOT NULL,
    branch TEXT,
    reason TEXT,
    long_op INTEGER NOT NULL DEFAULT 0,
    ttl_ms INTEGER NOT NULL,
    acquired_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (project, resource),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
CREATE INDEX idx_locks_expires ON resource_locks(expires_at);
CREATE INDEX idx_locks_session ON resource_locks(session_id);

CREATE TABLE inbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL,
    from_session TEXT NOT NULL,
    from_branch TEXT,
    message TEXT NOT NULL,
    tags TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_inbox_project ON inbox(project, created_at DESC);

CREATE TABLE inbox_reads (
    session_id TEXT NOT NULL,
    message_id INTEGER NOT NULL,
    read_at INTEGER NOT NULL,
    PRIMARY KEY (session_id, message_id)
);
