# sangha v0.1.0 Code Review

**Reviewer**: GLM-5.1 (opencode)
**Date**: 2026-04-26
**Commit**: HEAD
**Scope**: All 22 indexed files (15 src, 7 test), ~5000 LOC

---

## Summary

sangha is a well-structured session coordination daemon. The codebase follows its stated conventions consistently: one file per tool, actionable error triplets, parking_lot::Mutex for sync DB, UUIDv7 session IDs, and Unix-millisecond timestamps throughout. The error contract pattern (`tool` / `constraint` / `next_action`) is a standout design choice — every error a client receives is immediately actionable.

The main liabilities are a god-function `main()`, a panic-risk `unwrap()` in a public API, several duplicate types across layers, and a few semantic inaccuracies in return values. None of these are data-corruption risks, but they are maintenance and reliability concerns.

---

## Critical Findings

### C1. God function `main()` — CC=28, 205 lines

`src/main.rs:63-267`

The `main()` function handles six subcommands inline, each with its own DB open, migration, and formatting logic. This is the top refactor target per the health report (4.6/10).

**Impact**: Any change to one subcommand requires understanding all six. The cyclomatic complexity makes it easy to introduce regressions.

**Recommendation**: Extract each `Commands::*` arm into its own function. A minimal refactor:

```
fn run_serve(db, config, stdio) -> Result<...>
fn run_status(db, project, user, json) -> Result<...>
fn run_locks(db, project, user, json) -> Result<...>
fn run_clear(db, force_release, all, json) -> Result<...>
fn run_path(config) -> Result<...>
fn run_health(config) -> Result<...>
```

This alone should drop CC from 28 to ~5 per function.

### C2. `unwrap()` panic in exported function

`src/identity.rs:33`

```rust
let existing = self.session_id.get().unwrap();
```

`Identity::bind()` is a public method called from the MCP handler. The `OnceCell::get()` call after `set()` fails (i.e., the `Err(_)` branch) should logically always return `Some`, but `unwrap()` on a public API path is a latent crash if the implementation ever changes. This was flagged by the security scanner.

**Recommendation**: Replace with `expect("OnceCell::set failed but value absent — invariant violation")` at minimum, or restructure to avoid the unwrap:

```rust
Err(already_set) => {
    let existing = already_set; // AlreadyCell gives back the value that failed to set
    if existing == session_id { ... }
}
```

(Note: `OnceCell::set` returns `Err(T)` containing the value that *was not* set, so the existing value is available via `self.session_id.get()` — but the returned `Err(already)` actually contains the *new* value, not the old one. The current code is correct but fragile.)

### C3. Duplicate `HolderInfo` type across layers

`src/db.rs:129-134` and `src/tools/locks.rs:49-55`

Two independent `HolderInfo` structs with identical fields except the `expires_at` type (`i64` vs `String`). The `locks` handler manually maps one to the other. If a field is added to one but not the other, behavior silently diverges.

**Recommendation**: Either have the DB layer return the formatted type directly, or define a shared type. The current mapping is not wrong — it's just a maintenance trap.

---

## High-Priority Findings

### H1. `heartbeat()` returns the *configured* TTL, not the *remaining* TTL

`src/db.rs:326-332`

```rust
let ttl_remaining_ms = self.config.session_ttl_ms;
```

After a heartbeat, `ttl_remaining_ms` is set to the full session TTL (e.g., 600,000 ms). But the field name says "remaining" and the handler converts it to `ttl_remaining_sec` for the client. This is semantically misleading — a heartbeat issued 5 seconds before expiry would still report 600 seconds remaining.

The handler layer (`presence.rs:149`) divides by 1000 and returns it as-is, so clients get an inaccurate picture.

**Recommendation**: Either rename to `ttl_ms` / `ttl_sec` (indicating the full TTL), or compute actual remaining time:

```rust
let remaining = self.config.session_ttl_ms; // correct for now since heartbeat resets last_heartbeat
```

Actually, on closer inspection: since `heartbeat()` updates `last_heartbeat` to `now`, the remaining TTL *is* the full session TTL. The name is technically correct. But consider adding a comment making this explicit, because it's non-obvious to a reader.

### H2. Pruning only happens on read paths — no background cleanup

`src/db.rs:357-359`, `src/db.rs:550-551`

`list_sessions()` prunes dead sessions; `list_locks()` prunes expired locks; `broadcast()` prunes old inbox. But if nobody calls these endpoints, stale data accumulates forever. The `prune_all()` is only called once at startup (`main.rs:84`).

**Impact**: Long-running daemons with infrequent queries will accumulate dead rows. For the expected concurrency (one daemon, a few sessions) this is low risk, but it could become a problem if sessions are created and abandoned frequently.

**Recommendation**: Add an optional periodic prune (e.g., every `session_ttl_ms / 2`), or document that this is by-design (lazy pruning) if so intended.

### H3. Mutex gap between prune and query in `list_sessions` / `list_locks`

`src/db.rs:357-381`

```rust
pub fn list_sessions(&self, project: Option<&str>) -> Result<Vec<SessionRow>> {
    self.prune_dead_sessions()?;    // acquires and releases mutex
    let conn = self.conn.lock();     // re-acquires mutex
    ...
}
```

Between `prune_dead_sessions()` releasing the lock and the query re-acquiring it, another thread (or async task) could insert a session that would be immediately eligible for pruning. This is a benign race — the session would survive this call and be pruned on the next — but it's worth noting for correctness.

**Recommendation**: Accept this as a documented benign race (the current approach avoids holding the mutex across the full query, which is good for throughput), or hold the lock across both operations if strict consistency is needed.

### H4. `Identity::bind()` silently ignores mismatched `project` on re-bind

`src/identity.rs:24-48`

```rust
Ok(()) => {
    let _ = self.project.set(project);  // line 28
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
1. First bind: `_ = self.project.set(project)` silently drops the error if `project` was somehow already set (shouldn't happen, but the pattern is unsafe).
2. Re-bind with same `session_id`: the method returns `Ok(())` without verifying that `project` matches. A caller could re-register with a different project and the identity would still carry the old one.

**Recommendation**: Add a project-equality check in the idempotent path. Also use `.expect()` instead of `let _ =` for the project set.

---

## Medium-Priority Findings

### M1. `format_ms()` duplicated in three tool files

`src/tools/presence.rs:185-189`, `src/tools/locks.rs:209-213`, `src/tools/inbox.rs:146-150`

Identical function in all three files. Extract to a shared module (e.g., `src/tools/format.rs` or into `validate.rs` which is already the shared module).

### M2. Duplicated list-query pattern in `db.rs`

`src/db.rs:357-381` vs `src/db.rs:550-573`

`list_sessions` and `list_locks` have the same structure: prune → lock → prepare statement (with/without project filter) → query_map → collect. This pattern could be extracted into a generic helper, though the current explicit approach is readable.

### M3. `#[allow(deprecated)]` on `any_service` without tracking the replacement

`src/main.rs:330-331`

```rust
#[allow(deprecated)]
let app = axum::Router::new().route("/mcp", any_service(mcp_service));
```

This should have a comment explaining *what* the replacement API is and a tracking issue for migration.

### M4. Config silently falls back on bad env values

`src/config.rs:80-94`

`parse_env_or` prints a warning to stderr and falls back to the default. This uses `eprintln!` rather than the configured tracing level. At startup, tracing isn't initialized yet, so this is understandable — but it means config warnings are invisible if stderr is redirected.

**Recommendation**: Consider making invalid env vars a hard error (fail fast) rather than silently degrading.

### M5. PID file not cleaned up on crash

`src/main.rs:310-315`, `src/main.rs:340`

The PID file is written on startup and removed only after graceful shutdown. If the process crashes (OOM, signal 9), the stale PID file remains. The `serve_http` function checks for an already-running daemon via TCP connect, so the stale PID file is harmless — but it's untidy.

### M6. Missing index on `inbox_reads.session_id`

`migrations/0001_initial.sql:44-49`

The `inbox_reads` table has a primary key on `(session_id, message_id)` which implicitly indexes `session_id` as the leading column. The queries in `read_inbox` join on `r.session_id = ?1 AND r.message_id = i.id`, which can use the PK index. This is actually fine — no action needed, but worth noting the index analysis was done.

### M7. `read_inbox` defaults `limit` to `i64::MAX`

`src/db.rs:632`

```rust
let limit = input.limit.unwrap_or(i64::MAX);
```

If a caller omits `limit` and there are millions of inbox messages, this will load them all into memory. The handler layer doesn't cap it either. Consider a configurable default limit (e.g., 100).

---

## Low-Priority / Style Findings

### L1. `EmptyArgs` could be a unit struct

`src/mcp.rs:22-23`

```rust
pub struct EmptyArgs {}
```

Could be `pub struct EmptyArgs;` — marginally more idiomatic for zero-sized types. The braced form is required by rmcp's `Parameters<T>` derive, so this may be an API constraint.

### L2. CLI defaults to `Status` subcommand

`src/main.rs:76-80`

```rust
match cli.command.unwrap_or(Commands::Status { ... })
```

For a daemon, `serve` as the default subcommand would be more conventional. The current default of `Status` makes sense for a CLI-first UX where the daemon is already running.

### L3. Test `Config` construction duplicated

Multiple test files construct `Config` manually with the same field values. The `tests/common/mod.rs` provides `test_config()` but some tests (`session_test.rs:244-254`, `lock_test.rs:361-371`, `inbox_test.rs:310-319`) construct custom configs inline. Consider a `test_config_builder()` or `ConfigBuilder` pattern.

### L4. `inbox` table uses `AUTOINCREMENT`

`migrations/0001_initial.sql:34`

`INTEGER PRIMARY KEY AUTOINCREMENT` is SQLite-specific and prevents rowid reuse. For a message queue that's frequently pruned, plain `INTEGER PRIMARY KEY` (which still auto-increments) would allow reuse and is slightly more efficient. `AUTOINCREMENT` guarantees monotonicity even across vacuum, which may be desirable for message IDs.

### L5. `unwrap_or_default()` on JSON serialization failures

`src/tools/presence.rs:107`, `src/tools/inbox.rs:81`

```rust
serde_json::to_string(v).unwrap_or_default()
```

If JSON serialization fails, the metadata/tags are silently replaced with an empty string `""`. This is unlikely (serializing a `serde_json::Value` should never fail) but the fallback produces a syntactically empty value rather than `None`. Consider logging a warning.

---

## Positive Observations

1. **Error contract pattern** — The `tool / constraint / next_action` triplet on every error variant is excellent. The contract test in `tests/contract.rs` exhaustively checks every variant, which will catch regressions when new variants are added.

2. **Connection-scoped identity** — The `Identity` struct with `OnceCell` is a clean solution for binding session state per-connection without global mutable state.

3. **Prune-on-read** — While it has the gap noted in H2, the lazy pruning approach is pragmatic for the expected workload and avoids the complexity of a background timer.

4. **Scope abstraction** — The `validate::Scope` + `USER_SCOPE_PROJECT` sentinel is a clean way to handle project-scoped vs user-scoped resources without adding a separate table or join.

5. **Test coverage** — 41 unit tests, 7 integration tests (e2e), contract tests for error types. The e2e tests spin up actual daemon subprocesses and exercise the full MCP/JSON-RPC stack. This is unusually thorough for v0.1.0.

6. **Migration hygiene** — The migration file correctly defers PRAGMAs to per-connection setup (as documented in CLAUDE.md), and the comment at the top of the SQL file explicitly states this.

7. **FK cascade** — `ON DELETE CASCADE` on `resource_locks.session_id` ensures locks are cleaned up when sessions are removed, preventing orphaned locks.

---

## Architectural Notes

### Async handlers wrapping sync DB

The tool handlers are `async fn` but call `db.method()` synchronously. The CLAUDE.md documents this as intentional: "DB calls are sync (parking_lot::Mutex), called directly from async handlers." This is fine for the expected concurrency (single daemon, short transactions) but becomes a bottleneck if any query takes longer than a few milliseconds, as it blocks the tokio runtime. If this ever becomes a concern, `spawn_blocking` is the escape hatch.

### Single-process assumption

The design assumes one sangha process per database. The TCP-port check (`main.rs:304`) enforces this. If horizontal scaling is ever needed, the SQLite backend would need to be replaced or fronted with a connection pooler.

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
| God functions | 1 (main, CC=28) |
| Long param lists | 4 |
| Clone groups | 12 (23 duplicate symbols) |
| Security findings | 1 (low, unwrap in export) |
| Test count (estimated) | ~48 |

---

## Priority Summary

| ID | Severity | Summary | File |
|----|----------|---------|------|
| C1 | Critical | God function main() CC=28 | src/main.rs |
| C2 | Critical | unwrap() panic in public API | src/identity.rs:33 |
| C3 | Critical | Duplicate HolderInfo across layers | src/db.rs, src/tools/locks.rs |
| H1 | High | heartbeat TTL naming is misleading | src/db.rs:326 |
| H2 | High | No background pruning | src/db.rs |
| H3 | High | Mutex gap between prune and query | src/db.rs:357 |
| H4 | High | Identity::bind ignores project mismatch | src/identity.rs:28 |
| M1 | Medium | format_ms duplicated 3x | tools/*.rs |
| M2 | Medium | list_sessions/list_locks pattern duplication | src/db.rs |
| M3 | Medium | Deprecated API without tracking comment | src/main.rs:330 |
| M4 | Medium | Silent config fallback on bad env | src/config.rs |
| M5 | Medium | Stale PID file on crash | src/main.rs |
| M7 | Medium | read_inbox limit defaults to i64::MAX | src/db.rs:632 |
| L1 | Low | EmptyArgs braced vs unit | src/mcp.rs |
| L2 | Low | Default subcommand is Status | src/main.rs |
| L3 | Low | Test Config duplication | tests/*.rs |
| L4 | Low | AUTOINCREMENT vs plain PK | migrations |
| L5 | Low | Silent JSON serialization fallback | tools/*.rs |
