//! SQLite database layer for sangha.
//!
//! All timestamps are Unix milliseconds (i64). Mutually exclusive access is
//! achieved via a `parking_lot::Mutex<Connection>` — a single writer model
//! that is correct for the expected concurrency (one daemon, short-lived
//! transactions).

use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{Connection, params};

use crate::config::Config;
use crate::error::{Result, SanghaError};

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub project: String,
    pub branch: Option<String>,
    pub intent: Option<String>,
    pub pid: Option<i64>,
    pub hostname: Option<String>,
    pub started_at: i64,
    pub last_heartbeat: i64,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LockRow {
    pub resource: String,
    pub project: String,
    pub session_id: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
    pub long_op: bool,
    pub ttl_ms: i64,
    pub acquired_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct InboxRow {
    pub id: i64,
    pub project: String,
    pub from_session: String,
    pub from_branch: Option<String>,
    pub message: String,
    pub tags: Option<String>,
    pub created_at: i64,
}

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

pub struct RegisterInput {
    /// `None` = generate a new UUIDv7.
    pub session_id: Option<String>,
    pub project: String,
    pub branch: Option<String>,
    pub intent: Option<String>,
    pub pid: Option<i64>,
    pub hostname: Option<String>,
    pub metadata: Option<String>,
}

pub struct ClaimInput {
    pub resource: String,
    pub project: String,
    pub session_id: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
    pub long_op: bool,
    pub ttl_ms: i64,
    pub expires_at: i64,
}

pub struct ReleaseInput {
    pub resource: String,
    pub project: String,
    pub session_id: String,
    pub force: bool,
}

pub struct BroadcastInput {
    pub project: String,
    pub from_session: String,
    pub from_branch: Option<String>,
    pub message: String,
    pub tags: Option<String>,
}

pub struct ReadInboxInput {
    pub project: String,
    pub session_id: String,
    pub unread_only: bool,
    pub limit: Option<i64>,
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

pub struct HeartbeatResult {
    pub ok: bool,
    pub session_id: String,
    pub ttl_remaining_ms: i64,
    pub reason: Option<String>,
}

pub struct UnregisterResult {
    pub ok: bool,
    pub locks_released: usize,
}

pub struct ClaimResult {
    pub ok: bool,
    pub held_by: Option<HolderInfo>,
    pub lock: Option<LockRow>,
}

pub struct HolderInfo {
    pub session_id: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
    pub expires_at: i64,
}

pub struct ReleaseResult {
    pub ok: bool,
    pub status: String,
}

pub struct ReadInboxResult {
    pub count: i64,
    pub unread_total: i64,
    pub total: i64,
    pub messages: Vec<InboxRow>,
}

// ---------------------------------------------------------------------------
// Db
// ---------------------------------------------------------------------------

pub struct Db {
    conn: Mutex<Connection>,
    config: Arc<Config>,
}

impl Db {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Open the SQLite database at `config.db_path`, creating parent
    /// directories as needed. Applies WAL mode and other PRAGMAs.
    pub fn open(config: &Arc<Config>) -> Result<Self> {
        let path = &config.db_path;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                SanghaError::Internal(format!(
                    "could not create database directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let conn = Connection::open(path)?;
        Self::apply_pragmas(&conn, true)?;
        Ok(Self { conn: Mutex::new(conn), config: Arc::clone(config) })
    }

    /// Open an in-memory database — used in tests. WAL does not apply to
    /// in-memory connections but all other PRAGMAs are set.
    pub fn open_memory(config: &Arc<Config>) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::apply_pragmas(&conn, false)?;
        Ok(Self { conn: Mutex::new(conn), config: Arc::clone(config) })
    }

    fn apply_pragmas(conn: &Connection, wal: bool) -> Result<()> {
        if wal {
            conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        }
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;\
             PRAGMA busy_timeout=5000;",
        )?;
        Ok(())
    }

    /// Execute the embedded migration SQL to create the initial schema.
    pub fn run_migrations(&self) -> Result<()> {
        let sql = include_str!("../migrations/0001_initial.sql");
        self.conn.lock().execute_batch(sql)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn now_ms() -> i64 {
        Utc::now().timestamp_millis()
    }

    // -----------------------------------------------------------------------
    // Pruning
    // -----------------------------------------------------------------------

    /// Delete sessions whose `last_heartbeat` is older than `session_ttl_ms`.
    /// Returns the number of rows deleted.
    pub fn prune_dead_sessions(&self) -> Result<usize> {
        let cutoff = Self::now_ms() - self.config.session_ttl_ms;
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM sessions WHERE last_heartbeat < ?1", params![cutoff])?;
        Ok(n)
    }

    /// Delete resource locks whose `expires_at` is in the past.
    /// Returns the number of rows deleted.
    pub fn prune_expired_locks(&self) -> Result<usize> {
        let now = Self::now_ms();
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM resource_locks WHERE expires_at < ?1", params![now])?;
        Ok(n)
    }

    /// Delete inbox messages older than `inbox_retention_ms`.
    /// Returns the number of rows deleted.
    pub fn prune_old_inbox(&self) -> Result<usize> {
        let cutoff = Self::now_ms() - self.config.inbox_retention_ms;
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM inbox WHERE created_at < ?1", params![cutoff])?;
        Ok(n)
    }

    /// Run all three pruners in sequence.
    pub fn prune_all(&self) -> Result<()> {
        self.prune_dead_sessions()?;
        self.prune_expired_locks()?;
        self.prune_old_inbox()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    /// Register (or re-register) a session. If `input.session_id` is `None` a
    /// new UUIDv7 is generated. On conflict the row is updated in place while
    /// `started_at` is preserved.
    pub fn register_session(&self, input: RegisterInput) -> Result<SessionRow> {
        let now = Self::now_ms();
        let id = input
            .session_id
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions (
                id, project, branch, intent, pid, hostname,
                started_at, last_heartbeat, metadata
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                project        = excluded.project,
                branch         = excluded.branch,
                intent         = excluded.intent,
                pid            = excluded.pid,
                hostname       = excluded.hostname,
                last_heartbeat = excluded.last_heartbeat,
                metadata       = excluded.metadata",
            params![
                id,
                input.project,
                input.branch,
                input.intent,
                input.pid,
                input.hostname,
                now,
                now,
                input.metadata,
            ],
        )?;

        let row = conn.query_row(
            "SELECT id, project, branch, intent, pid, hostname,
                    started_at, last_heartbeat, metadata
             FROM sessions WHERE id = ?1",
            params![id],
            map_session_row,
        )?;
        Ok(row)
    }

    /// Update `last_heartbeat` for the given session.
    pub fn heartbeat(&self, session_id: &str) -> Result<HeartbeatResult> {
        let now = Self::now_ms();
        let conn = self.conn.lock();
        let affected = conn.execute(
            "UPDATE sessions SET last_heartbeat = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        if affected == 0 {
            return Ok(HeartbeatResult {
                ok: false,
                session_id: session_id.to_string(),
                ttl_remaining_ms: 0,
                reason: Some("session not found".to_string()),
            });
        }

        // Heartbeat just refreshed last_heartbeat to `now`, so the remaining
        // TTL is the full configured session_ttl_ms.
        let ttl_remaining_ms = self.config.session_ttl_ms;
        Ok(HeartbeatResult {
            ok: true,
            session_id: session_id.to_string(),
            ttl_remaining_ms,
            reason: None,
        })
    }

    /// Remove a session row. The ON DELETE CASCADE on `resource_locks` ensures
    /// associated locks are also removed; we count them first.
    pub fn unregister_session(&self, session_id: &str) -> Result<UnregisterResult> {
        let conn = self.conn.lock();

        // Count locks that will be released by the cascade.
        let locks_released: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM resource_locks WHERE session_id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize;

        let affected =
            conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;

        Ok(UnregisterResult { ok: affected > 0, locks_released })
    }

    /// List sessions, optionally filtered by project. Prunes dead sessions
    /// first so the caller always gets a fresh view.
    pub fn list_sessions(&self, project: Option<&str>) -> Result<Vec<SessionRow>> {
        // Prune without holding the lock across the query. The brief gap
        // between prune and SELECT is benign — a session that becomes dead in
        // that window simply appears once more and is pruned next call.
        self.prune_dead_sessions()?;

        let conn = self.conn.lock();
        let mut stmt = match project {
            Some(_) => conn.prepare(
                "SELECT id, project, branch, intent, pid, hostname,
                        started_at, last_heartbeat, metadata
                 FROM sessions WHERE project = ?1
                 ORDER BY started_at",
            )?,
            None => conn.prepare(
                "SELECT id, project, branch, intent, pid, hostname,
                        started_at, last_heartbeat, metadata
                 FROM sessions ORDER BY started_at",
            )?,
        };

        let rows: std::result::Result<Vec<SessionRow>, rusqlite::Error> = match project {
            Some(p) => stmt.query_map(params![p], map_session_row)?.collect(),
            None => stmt.query_map([], map_session_row)?.collect(),
        };
        Ok(rows?)
    }

    /// Fetch a single session by id.
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionRow>> {
        let conn = self.conn.lock();
        match conn.query_row(
            "SELECT id, project, branch, intent, pid, hostname,
                    started_at, last_heartbeat, metadata
             FROM sessions WHERE id = ?1",
            params![session_id],
            map_session_row,
        ) {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SanghaError::Db(e)),
        }
    }

    // -----------------------------------------------------------------------
    // Locks
    // -----------------------------------------------------------------------

    /// Attempt to acquire (or renew) a resource lock.
    ///
    /// Logic:
    /// 1. Prune expired locks.
    /// 2. Verify the session exists.
    /// 3. Check for a conflicting holder. Return `ok=false` + `held_by` if
    ///    someone else holds it.
    /// 4. INSERT … ON CONFLICT DO UPDATE — preserves `acquired_at` on renewal
    ///    by the same session.
    pub fn claim_resource(&self, input: ClaimInput) -> Result<ClaimResult> {
        self.prune_expired_locks()?;

        let now = Self::now_ms();
        let conn = self.conn.lock();

        // Check session exists.
        let session_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                params![input.session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !session_exists {
            return Ok(ClaimResult {
                ok: false,
                held_by: None,
                lock: None,
            });
        }

        // Check for a conflicting holder (different session).
        let existing: Option<LockRow> = conn
            .query_row(
                "SELECT resource, project, session_id, branch, reason,
                        long_op, ttl_ms, acquired_at, expires_at
                 FROM resource_locks
                 WHERE project = ?1 AND resource = ?2",
                params![input.project, input.resource],
                map_lock_row,
            )
            .ok();

        if let Some(ref held) = existing
            && held.session_id != input.session_id
        {
            let holder = HolderInfo {
                session_id: held.session_id.clone(),
                branch: held.branch.clone(),
                reason: held.reason.clone(),
                expires_at: held.expires_at,
            };
            return Ok(ClaimResult {
                ok: false,
                held_by: Some(holder),
                lock: Some(held.clone()),
            });
        }

        // Upsert — preserve acquired_at when renewing.
        conn.execute(
            "INSERT INTO resource_locks (
                resource, project, session_id, branch, reason,
                long_op, ttl_ms, acquired_at, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(project, resource) DO UPDATE SET
                session_id  = excluded.session_id,
                branch      = excluded.branch,
                reason      = excluded.reason,
                long_op     = excluded.long_op,
                ttl_ms      = excluded.ttl_ms,
                expires_at  = excluded.expires_at",
            params![
                input.resource,
                input.project,
                input.session_id,
                input.branch,
                input.reason,
                input.long_op as i64,
                input.ttl_ms,
                now,
                input.expires_at,
            ],
        )?;

        let lock = conn.query_row(
            "SELECT resource, project, session_id, branch, reason,
                    long_op, ttl_ms, acquired_at, expires_at
             FROM resource_locks
             WHERE project = ?1 AND resource = ?2",
            params![input.project, input.resource],
            map_lock_row,
        )?;

        Ok(ClaimResult { ok: true, held_by: None, lock: Some(lock) })
    }

    /// Release a resource lock. If `force=true` the `session_id` check is
    /// skipped (admin release).
    pub fn release_resource(&self, input: ReleaseInput) -> Result<ReleaseResult> {
        let conn = self.conn.lock();
        let affected = if input.force {
            conn.execute(
                "DELETE FROM resource_locks WHERE project = ?1 AND resource = ?2",
                params![input.project, input.resource],
            )?
        } else {
            conn.execute(
                "DELETE FROM resource_locks
                 WHERE project = ?1 AND resource = ?2 AND session_id = ?3",
                params![input.project, input.resource, input.session_id],
            )?
        };

        if affected > 0 {
            Ok(ReleaseResult { ok: true, status: "released".to_string() })
        } else {
            // Distinguish "never existed" from "held by someone else".
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM resource_locks
                     WHERE project = ?1 AND resource = ?2",
                    params![input.project, input.resource],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            if exists {
                Ok(ReleaseResult {
                    ok: false,
                    status: "not_owner".to_string(),
                })
            } else {
                Ok(ReleaseResult {
                    ok: false,
                    status: "not_found".to_string(),
                })
            }
        }
    }

    /// List locks, optionally filtered by project. Prunes expired locks first.
    pub fn list_locks(&self, project: Option<&str>) -> Result<Vec<LockRow>> {
        self.prune_expired_locks()?;

        let conn = self.conn.lock();
        let mut stmt = match project {
            Some(_) => conn.prepare(
                "SELECT resource, project, session_id, branch, reason,
                        long_op, ttl_ms, acquired_at, expires_at
                 FROM resource_locks WHERE project = ?1
                 ORDER BY acquired_at",
            )?,
            None => conn.prepare(
                "SELECT resource, project, session_id, branch, reason,
                        long_op, ttl_ms, acquired_at, expires_at
                 FROM resource_locks ORDER BY acquired_at",
            )?,
        };

        let rows: std::result::Result<Vec<LockRow>, rusqlite::Error> = match project {
            Some(p) => stmt.query_map(params![p], map_lock_row)?.collect(),
            None => stmt.query_map([], map_lock_row)?.collect(),
        };
        Ok(rows?)
    }

    /// Extend expiry of all locks owned by `session_id` whose remaining TTL
    /// is less than half their total TTL. Returns the count of rows updated.
    pub fn auto_extend_locks(&self, session_id: &str) -> Result<usize> {
        let now = Self::now_ms();
        let n = self.conn.lock().execute(
            "UPDATE resource_locks
             SET expires_at = ?1 + ttl_ms
             WHERE session_id = ?2
               AND (expires_at - ?1) < (ttl_ms / 2)",
            params![now, session_id],
        )?;
        Ok(n)
    }

    // -----------------------------------------------------------------------
    // Inbox
    // -----------------------------------------------------------------------

    /// Post a message to the inbox. Prunes old messages first.
    pub fn broadcast(&self, input: BroadcastInput) -> Result<InboxRow> {
        self.prune_old_inbox()?;

        let now = Self::now_ms();
        let conn = self.conn.lock();

        conn.execute(
            "INSERT INTO inbox (project, from_session, from_branch, message, tags, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                input.project,
                input.from_session,
                input.from_branch,
                input.message,
                input.tags,
                now,
            ],
        )?;

        let id = conn.last_insert_rowid();
        let row = conn.query_row(
            "SELECT id, project, from_session, from_branch, message, tags, created_at
             FROM inbox WHERE id = ?1",
            params![id],
            map_inbox_row,
        )?;
        Ok(row)
    }

    /// Read inbox messages for a project on behalf of `session_id`.
    ///
    /// - Excludes messages sent by `session_id` itself.
    /// - If `unread_only`, excludes messages already marked read.
    /// - Counts total and unread **before** marking the returned messages read.
    /// - Inserts `inbox_reads` records for every returned message.
    pub fn read_inbox(&self, input: ReadInboxInput) -> Result<ReadInboxResult> {
        const DEFAULT_LIMIT: i64 = 100;
        const MAX_LIMIT: i64 = 1000;
        let now = Self::now_ms();
        let conn = self.conn.lock();
        let limit = input
            .limit
            .map(|n| n.clamp(1, MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);

        // Total messages in project (excluding own).
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM inbox
                 WHERE project = ?1 AND from_session != ?2",
                params![input.project, input.session_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Unread count (excluding own, not yet in inbox_reads for this session).
        let unread_total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM inbox i
                 LEFT JOIN inbox_reads r
                        ON r.session_id = ?1 AND r.message_id = i.id
                 WHERE i.project = ?2
                   AND i.from_session != ?1
                   AND r.message_id IS NULL",
                params![input.session_id, input.project],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Fetch messages.
        let messages: Vec<InboxRow> = if input.unread_only {
            let mut stmt = conn.prepare(
                "SELECT i.id, i.project, i.from_session, i.from_branch,
                        i.message, i.tags, i.created_at
                 FROM inbox i
                 LEFT JOIN inbox_reads r
                        ON r.session_id = ?1 AND r.message_id = i.id
                 WHERE i.project = ?2
                   AND i.from_session != ?1
                   AND r.message_id IS NULL
                 ORDER BY i.created_at DESC
                 LIMIT ?3",
            )?;
            stmt.query_map(params![input.session_id, input.project, limit], map_inbox_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, project, from_session, from_branch,
                        message, tags, created_at
                 FROM inbox
                 WHERE project = ?1 AND from_session != ?2
                 ORDER BY created_at DESC
                 LIMIT ?3",
            )?;
            stmt.query_map(params![input.project, input.session_id, limit], map_inbox_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        // Mark returned messages as read.
        for msg in &messages {
            conn.execute(
                "INSERT OR IGNORE INTO inbox_reads (session_id, message_id, read_at)
                 VALUES (?1, ?2, ?3)",
                params![input.session_id, msg.id, now],
            )?;
        }

        let count = messages.len() as i64;
        Ok(ReadInboxResult { count, unread_total, total, messages })
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        project: row.get(1)?,
        branch: row.get(2)?,
        intent: row.get(3)?,
        pid: row.get(4)?,
        hostname: row.get(5)?,
        started_at: row.get(6)?,
        last_heartbeat: row.get(7)?,
        metadata: row.get(8)?,
    })
}

fn map_lock_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LockRow> {
    let long_op_int: i64 = row.get(5)?;
    Ok(LockRow {
        resource: row.get(0)?,
        project: row.get(1)?,
        session_id: row.get(2)?,
        branch: row.get(3)?,
        reason: row.get(4)?,
        long_op: long_op_int != 0,
        ttl_ms: row.get(6)?,
        acquired_at: row.get(7)?,
        expires_at: row.get(8)?,
    })
}

fn map_inbox_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InboxRow> {
    Ok(InboxRow {
        id: row.get(0)?,
        project: row.get(1)?,
        from_session: row.get(2)?,
        from_branch: row.get(3)?,
        message: row.get(4)?,
        tags: row.get(5)?,
        created_at: row.get(6)?,
    })
}
