# sangha v0.1.0 Code Review

**Reviewer**: Kimi-k2.6 (opencode)
**Date**: 2026-04-26
**Commit**: HEAD
**Scope**: All 22 indexed files (15 src, 7 test), ~5000 LOC

---

## Summary

sangha is a well-architected session coordination daemon for Claude Code sessions. The codebase is compact (~5000 LOC), thoroughly tested (48+ tests including 7 e2e tests that exercise the full MCP/HTTP stack), and follows consistent conventions: one file per tool, `parking_lot::Mutex` for synchronous DB access, UUIDv7 session IDs, Unix-millisecond timestamps, and the `tool / constraint / next_action` error triplet pattern throughout.

The build passes, all tests pass, and `cargo clippy` is clean. The two main structural liabilities are a god-function `main()` (CC=28) and some cross-layer type duplication. There are also a few semantic inconsistencies and a latent panic path in a public API.

---

## Critical Findings

### C1. God function `main()` — CC=28, 205 lines

`src/main.rs:63-267`

The `main()` function handles six subcommands inline, each with its own DB open/migration, formatting, and I/O logic. This is the only file flagged as critical by the health report (health 4.6/10).

**Impact**: Changes to one subcommand require understanding all six. The cyclomatic complexity (28) makes regressions likely. The function also duplicates the `effective_project` resolution logic across `Status`, `Locks`, and `Clear` arms.

**Recommendation**: Extract each `Commands::*` arm into a named function:

```rust
fn run_serve(db: Arc<Db>, config: Arc<Config>, stdio: bool) -> Result<(), Box<dyn Error>> { ... }
fn run_status(db: &Db, project: Option<String>, user: bool, json: bool) -> Result<(), Box<dyn Error>> { ... }
fn run_locks(db: &Db, project: Option<String>, user: bool, json: bool) -> Result<(), Box<dyn Error>> { ... }
fn run_clear(db: &Db, force_release: Option<String>, all: bool, json: bool) -> Result<(), Box<dyn Error>> { ... }
fn run_path(config: &Config) { ... }
fn run_health(config: &Config) -> impl Future { ... }
```

This alone should drop `main()` CC from 28 to ~5.

### C2. `unwrap()` panic path in public API

`src/identity.rs:33`

```rust
let existing = self.session_id.get().unwrap();
```

`Identity::bind()` is a public method called from the MCP handler. While `OnceCell::set` returns `Err(T)` when already initialized, and the `get()` on the next line should logically always succeed, `unwrap()` on a public API is a latent crash if the implementation ever changes (e.g., if `OnceCell` semantics shift or the type is swapped for a different primitive).

**Recommendation**: Replace with an `expect` explaining the invariant, or restructure to avoid the `get()` call entirely:

```rust
Err(_) => {
    if let Some(existing) = self.session_id.get() {
        if existing == &session_id {
            // Idempotent path — but also check project equality (see H4)
            return Ok(());
        }
        // ... error
    }
    // Invariant violation: set failed but get returns None
    return Err(SanghaError::Internal(...));
}
```

### C3. Duplicate `HolderInfo` types across layers

`src/db.rs:129-134` and `src/tools/locks.rs:49-55`

Two independent `HolderInfo` structs with identical fields except `expires_at` type (`i64` vs `String`). The locks handler manually maps one to the other. If a field is added to one but not the other, behavior silently diverges.

**Recommendation**: Define a single `HolderInfo` in the DB layer and let the handler format it for output, or use a shared domain type. The current manual mapping is a maintenance trap.

---

## High-Priority Findings

### H1. `heartbeat()` returns configured TTL, not computed remaining TTL

`src/db.rs:326`

```rust
let ttl_remaining_ms = self.config.session_ttl_ms;
```

After a successful heartbeat, `ttl_remaining_ms` is set to the full configured TTL. Since heartbeat updates `last_heartbeat` to `now`, this value is technically correct, but the name implies a runtime computation. A reader might expect something like:

```rust
let ttl_remaining_ms = self.config.session_ttl_ms; // correct because heartbeat resets last_heartbeat
```

**Recommendation**: Add an explicit comment, or rename to `ttl_ms` to avoid the implication of a dynamic calculation.

### H2. Pruning is lazy (read-triggered) with no background cleanup

`src/db.rs:357`, `src/db.rs:550`, `src/db.rs:594`

Dead sessions, expired locks, and old inbox messages are only pruned when `list_sessions`, `list_locks`, or `broadcast` are called. `prune_all()` runs once at startup (`main.rs:84`). If the daemon runs for days without these read paths being hit, stale data accumulates.

**Impact**: For the expected workload (one daemon, a few sessions, frequent heartbeats) this is low risk. But abandoned sessions or frequent short-lived sessions could bloat the DB.

**Recommendation**: Add an optional background prune task (e.g., `tokio::time::interval` every `session_ttl_ms / 2`), or document that lazy pruning is by design.

### H3. Mutex gap between prune and query

`src/db.rs:357-381`

```rust
pub fn list_sessions(&self, project: Option<&str>) -> Result<Vec<SessionRow>> {
    self.prune_dead_sessions()?;    // acquires & releases mutex
    let conn = self.conn.lock();     // re-acquires mutex
    ...
}
```

Between releasing the lock after prune and re-acquiring it for the query, another task could insert a session that would be immediately eligible for pruning. This is a benign race — the session survives this call and is pruned on the next — but worth noting.

**Recommendation**: Document as a benign race. Holding the lock across both operations would hurt throughput for marginal correctness gain.

### H4. `Identity::bind()` ignores project mismatch on re-bind

`src/identity.rs:24-48`

```rust
Ok(()) => {
    let _ = self.project.set(project);  // silently drops error
    Ok(())
}
Err(_) => {
    let existing = self.session_id.get().unwrap();
    if existing == &session_id {
        Ok(())  // idempotent — but project is NOT checked
    } else { ... }
}
```

Two issues:
1. `let _ = self.project.set(project)` silently ignores a set failure (impossible in current code, but the pattern is unsafe).
2. In the idempotent path, if the same `session_id` is passed with a *different* `project`, the method returns `Ok(())` while `project` retains its old value. This is semantically wrong — a caller could observe stale project state.

**Recommendation**: Check project equality in the idempotent path:

```rust
if existing == &session_id {
    if self.project.get().map(|p| p == &project).unwrap_or(false) {
        Ok(())
    } else {
        Err(SanghaError::IdentityError { ... })
    }
} else { ... }
```

### H5. `session_recreated` field is dead code

`src/db.rs:126`, `src/tools/locks.rs:79`

`ClaimResult.session_recreated` is always `false` in the current implementation. The handler layer strips `false` via `if result.session_recreated { Some(true) } else { None }`, so the field never appears in output. The `ClaimInput` struct carries `session_id` but `claim_resource` never creates a session — it just returns `ok=false` if the session is missing.

**Recommendation**: Either implement auto-session-creation on claim, or remove the field and simplify the types. Leaving dead fields confuses readers.

### H6. `gethostname` crate is unused

`Cargo.toml:35`

The `gethostname` dependency is listed but never referenced in source. `cargo-udeps` would flag this. Removing it reduces build time and attack surface.

---

## Medium-Priority Findings

### M1. `format_ms()` duplicated in three tool files

`src/tools/presence.rs:185-189`, `src/tools/locks.rs:209-213`, `src/tools/inbox.rs:146-150`

Identical function body in all three files. Extract to a shared module (e.g., `src/tools/util.rs` or `src/util.rs`).

### M2. Duplicated list-query pattern in `db.rs`

`src/db.rs:357-381` vs `src/db.rs:550-573`

`list_sessions` and `list_locks` share the same structure: prune → lock → prepare statement → query_map → collect. The project filter logic is also duplicated in the CLI (`main.rs` Status/Locks arms) and the handlers (`presence.rs`, `locks.rs`).

**Recommendation**: A generic helper or macro could reduce duplication, though the current explicit form is readable.

### M3. `#[allow(deprecated)]` on `any_service` without migration comment

`src/main.rs:330-331`

```rust
#[allow(deprecated)]
let app = axum::Router::new().route("/mcp", any_service(mcp_service));
```

No comment explains what the replacement API is or when to migrate. This will become a build warning when the deprecated API is removed.

### M4. Config silently falls back on bad env values

`src/config.rs:80-94`

`parse_env_or` prints `eprintln!` and falls back to default. At startup tracing isn't initialized yet, so `eprintln!` is understandable — but if stderr is redirected, the warning is invisible. More importantly, silent degradation violates the "fail fast" principle for configuration.

**Recommendation**: Consider making invalid env vars a hard error rather than silently degrading.

### M5. PID file not cleaned up on crash

`src/main.rs:309-315`, `src/main.rs:340`

The PID file is written on startup and removed only after graceful shutdown. If the process crashes (OOM, signal 9), the stale PID file remains. The daemon checks for already-running via TCP connect, so the stale file is harmless — but untidy.

### M6. `read_inbox` defaults `limit` to `i64::MAX`

`src/db.rs:632`

```rust
let limit = input.limit.unwrap_or(i64::MAX);
```

If a caller omits `limit` and there are millions of messages, this loads them all into memory. The handler layer doesn't cap it either.

**Recommendation**: Add a configurable default limit (e.g., 100) and a hard cap (e.g., 1000) in the handler layer.

### M7. Lock `branch` field is always `None`

`src/tools/locks.rs:130`

```rust
branch: None,
```

`ClaimInput.branch` is hard-coded to `None` in `handle_claim`, even though the session may have a branch. The DB schema supports it and `map_lock_row` reads it, but the tool layer never populates it. This appears to be an incomplete feature.

**Recommendation**: Either populate from the session row, or remove the column from the schema if it's not needed.

### M8. E2E test has a TOCTOU port race

`tests/e2e.rs:30-32`

```rust
let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
let port = listener.local_addr().unwrap().port();
drop(listener);
```

Between `drop(listener)` and `child.spawn()`, another process could grab the port. The daemon does its own `TcpListener::bind`, so the worst case is a test failure — not a security issue — but it's flaky under load.

**Recommendation**: Pass `port: 0` to the daemon and have it report its bound port to a known file, or use a unix socket for tests.

---

## Low-Priority / Style Findings

### L1. `EmptyArgs` could be a unit struct

`src/mcp.rs:22-23`

```rust
pub struct EmptyArgs {}
```

Could be `pub struct EmptyArgs;` — marginally more idiomatic. The braced form may be required by rmcp's `Parameters<T>` derive, so verify before changing.

### L2. CLI default subcommand is `Status`, not `Serve`

`src/main.rs:76-80`

```rust
match cli.command.unwrap_or(Commands::Status { ... })
```

The `Serve` variant comment says "Start the sangha server (default)", but the actual CLI default is `Status`. This contradicts the comment and is surprising for a daemon.

### L3. `long_op as i64` is less explicit than `i64::from`

`src/db.rs:485`

```rust
input.long_op as i64,
```

`i64::from(input.long_op)` would be more explicit about the intent (boolean to integer mapping).

### L4. `unwrap_or_default()` on JSON serialization in error path

`src/mcp.rs:216`

```rust
Some(serde_json::to_value(data).unwrap_or_default()),
```

If serializing `ErrorData` fails (extremely unlikely), the client receives an empty `data` object with no actionable fields. Since `ErrorData` contains only simple serializable types, this is theoretical — but an `expect` or `map_err` would be cleaner.

### L5. `inbox` table uses `AUTOINCREMENT`

`migrations/0001_initial.sql:34`

`INTEGER PRIMARY KEY AUTOINCREMENT` prevents rowid reuse. For a frequently-pruned message queue, plain `INTEGER PRIMARY KEY` (which still auto-increments) would allow reuse and is slightly more efficient. `AUTOINCREMENT` guarantees monotonicity across vacuum, which may be desirable.

### L6. Test `Config` construction duplicated

Multiple test files construct `Config` manually with the same field values. `tests/common/mod.rs` provides `test_config()` but some tests (`session_test.rs:244-254`, `lock_test.rs:361-371`, `inbox_test.rs:310-319`) construct custom configs inline.

**Recommendation**: Add a `test_config_with_ttl()` helper or use a builder pattern.

### L7. Async helper that doesn't await

`tests/lock_test.rs:16-27`

```rust
async fn register_session(db: &Arc<Db>, project: &str) -> String {
    let input = RegisterInput { ... };
    db.register_session(input).expect("register session").id
}
```

This function is `async` but contains no `.await`. It could be synchronous. This is harmless but misleading.

---

## Positive Observations

1. **Error contract pattern** — The `tool / constraint / next_action` triplet on every error variant is excellent. `tests/contract.rs` exhaustively checks every variant, catching regressions when new variants are added.

2. **Connection-scoped identity** — `Identity` with `OnceCell` is a clean solution for per-connection session state without global mutable state. The `SanghaServer::clone()` correctly creates a fresh `Identity` for each connection.

3. **Pragmatic lazy pruning** — While it has the gap noted in H2, the lazy pruning avoids background-task complexity. Given expected scale (single daemon, few sessions), this is a reasonable trade-off.

4. **Scope abstraction** — `validate::Scope` + `USER_SCOPE_PROJECT` sentinel elegantly handles project-scoped vs user-scoped resources without extra tables or joins.

5. **Comprehensive test coverage** — 48+ tests spanning unit, integration, contract, and e2e. The e2e tests spawn actual daemon subprocesses and exercise the full MCP/JSON-RPC/HTTP stack — unusually thorough for v0.1.0.

6. **Migration hygiene** — PRAGMAs are correctly deferred to per-connection setup (`Db::open()`), and the migration SQL explicitly documents this.

7. **FK cascade cleanup** — `ON DELETE CASCADE` on `resource_locks.session_id` ensures locks are cleaned up when sessions are removed.

8. **Lock renewal preserves `acquired_at`** — The `ON CONFLICT DO UPDATE` on `resource_locks` intentionally omits `acquired_at`, so renewal by the same session preserves the original claim time. This is a subtle but correct design choice.

9. **Clean lib+bin split** — `src/lib.rs` exports modules; `src/main.rs` is purely the CLI/daemon entry point. This enables both embedding and standalone use.

---

## Architectural Notes

### Async handlers wrapping sync DB

Tool handlers are `async fn` but call `db.method()` synchronously. The `CLAUDE.md` documents this as intentional: "DB calls are sync (parking_lot::Mutex), called directly from async handlers." This is fine for the expected concurrency (single daemon, short transactions) but blocks the tokio runtime if any query takes more than a few milliseconds. If this becomes a concern, `spawn_blocking` is the escape hatch.

### Single-process assumption

The design assumes one sangha process per database. The TCP-port check (`main.rs:304`) enforces this. Horizontal scaling would require replacing SQLite or adding a connection pooler.

### Soft failures vs hard errors in claim

`db.claim_resource()` returns `ClaimResult { ok: false, ... }` when the session doesn't exist or another session holds the lock. This is a *soft failure* — the caller gets a JSON `ok: false` rather than an error response. This design choice is consistent (all business-logic failures are soft), but it means callers must always check the `ok` field.

---

## Metrics

| Metric | Value |
|--------|-------|
| Source files | 15 |
| Test files | 7 |
| LOC (src) | ~4,989 |
| Symbols indexed | 421 |
| Import edges | 24 |
| Health (avg, unhealthy files) | 6.2 / 10 |
| God functions | 1 (`main`, CC=28) |
| Long param lists | 4 |
| Clone groups | 10 (17 duplicate symbols) |
| Security findings | 1 (low, unwrap in export) |
| Test count | ~48 |
| Build | Clean |
| Clippy | Clean |

---

## Priority Summary

| ID | Severity | Summary | File |
|----|----------|---------|------|
| C1 | Critical | God function `main()` CC=28 | `src/main.rs` |
| C2 | Critical | `unwrap()` in public API `Identity::bind` | `src/identity.rs:33` |
| C3 | Critical | Duplicate `HolderInfo` across layers | `src/db.rs`, `src/tools/locks.rs` |
| H1 | High | `heartbeat()` TTL naming is misleading | `src/db.rs:326` |
| H2 | High | No background pruning | `src/db.rs` |
| H3 | High | Mutex gap between prune and query | `src/db.rs:357` |
| H4 | High | `Identity::bind` ignores project mismatch | `src/identity.rs:28` |
| H5 | High | `session_recreated` is dead code | `src/db.rs:126` |
| H6 | High | `gethostname` crate unused | `Cargo.toml:35` |
| M1 | Medium | `format_ms` duplicated 3x | `tools/*.rs` |
| M2 | Medium | `list_sessions`/`list_locks` pattern duplication | `src/db.rs` |
| M3 | Medium | Deprecated API without tracking comment | `src/main.rs:330` |
| M4 | Medium | Silent config fallback on bad env | `src/config.rs` |
| M5 | Medium | Stale PID file on crash | `src/main.rs` |
| M6 | Medium | `read_inbox` limit defaults to `i64::MAX` | `src/db.rs:632` |
| M7 | Medium | Lock `branch` always `None` | `src/tools/locks.rs:130` |
| M8 | Medium | E2E port TOCTOU race | `tests/e2e.rs:30` |
| L1 | Low | `EmptyArgs` braced vs unit | `src/mcp.rs` |
| L2 | Low | Default subcommand is `Status` | `src/main.rs` |
| L3 | Low | `long_op as i64` less explicit | `src/db.rs:485` |
| L4 | Low | `unwrap_or_default` in error path | `src/mcp.rs:216` |
| L5 | Low | `AUTOINCREMENT` vs plain PK | `migrations` |
| L6 | Low | Test `Config` duplication | `tests/*.rs` |
| L7 | Low | Async helper without await | `tests/lock_test.rs` |
