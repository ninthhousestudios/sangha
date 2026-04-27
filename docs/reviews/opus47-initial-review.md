# Sangha — Initial Code Review (Opus 4.7)

**Date:** 2026-04-27
**Reviewer:** Claude Opus 4.7, via the `ce-code-review` skill, adapted for full-repo initial review (no diff base).
**Scope:** All of `src/` (~2.3 kLOC), `tests/` (~2.4 kLOC), `migrations/0001_initial.sql`, `Cargo.toml`, `README.md`, project `CLAUDE.md` at HEAD `2bc44d5`.
**Mode:** Report-only.
**Reviewer team (6 parallel agents, model `sonnet`):**
- correctness-reliability — logic, races, error propagation, async/sync boundary
- security — input validation, identity, network exposure, DoS surface
- testing — coverage gaps, brittle tests, weak assertions
- maintainability — coupling, complexity, conventions, surface area
- api-contract — MCP tool schemas, naming, versioning, error envelope
- performance-db — schema, query shape, mutex contention, growth paths

Aggregate findings: 11 P1 / 25 P2 / 22 P3 before dedup; 8 P1 / 19 P2 / 17 P3 after merge. Cross-reviewer agreement noted in the Reviewer column.

---

## Headline

The codebase is small, disciplined, and consistent with its stated patterns (CLAUDE.md is well-followed). The biggest concrete defects are **shallow** — a copy-paste in tool-name strings that breaks the error envelope contract, an unindexed orphan-rows pattern in `inbox_reads` that grows forever, and a unit-mismatch in `ttl_sec` ↔ `ttl_ms` that ships wrong field names to MCP clients. None are architectural; all are routine fixes.

The **structural** observations are: (1) the explicit "sync DB inside async handler" decision is sound at the stated envelope but is the single load-bearing assumption — multiple findings (atomicity, busy_timeout-vs-runtime starvation, list TOCTOU) become real defects the moment that assumption is relaxed; (2) test coverage of the new `Drop`-based auto-unregister and of error-envelope wire shape is essentially zero, so the v0.1 contract is not enforced anywhere.

---

## P1 — High

| # | File:Line | Issue | Reviewer(s) | Conf | Route |
|---|-----------|-------|-------------|------|-------|
| 1 | `src/tools/locks.rs:107,109,173,175,191` | `validate::*` calls hardcode `"lock_claim"`/`"lock_release"`/`"lock_list"` but MCP registers these as `resource_claim`/`resource_release`/`resource_list`. Every error envelope from these three tools reports a `tool` name the client never used. Breaks any client that routes/logs by `error.data.tool`. | api-contract | 1.00 | safe_auto → review-fixer |
| 2 | `src/tools/inbox.rs:76,78,107` | Same defect: validate calls use `"inbox_broadcast"`/`"inbox_read"` but MCP names are `broadcast`/`read_inbox`. | api-contract | 1.00 | safe_auto → review-fixer |
| 3 | `src/mcp.rs:58` (Drop impl) | The `SanghaServer::drop` auto-unregister added in `dcc3357` has zero test coverage — neither unit nor E2E. The primary mechanism for cleaning up sessions on MCP disconnect could regress silently. | testing | 0.97 | manual → downstream-resolver (verification) |
| 4 | `tests/contract.rs:25` | `every_error_variant_populates_contract` omits `SanghaError::Db`. All three branches of `db_next_action()` are uncovered; an empty `constraint`/`next_action` here would break the wire contract silently. | testing | 0.95 | safe_auto → review-fixer |
| 5 | `src/tools/{locks,inbox,presence}.rs` (handlers) | Validators are unit-tested in isolation but no integration test asserts that handlers actually call them and propagate `SanghaError::InvalidArgument`. The validation boundary has no end-to-end coverage. | testing | 0.92 | safe_auto → review-fixer |

Findings 1 & 2 are the same defect in two files. Both are mechanical search/replace.
Finding 3 should be fixed by a single E2E that opens an MCP connection, registers, drops the connection, and asserts the session disappears from a fresh `session_list`.

---

## P2 — Moderate

| # | File:Line | Issue | Reviewer(s) | Conf | Route |
|---|-----------|-------|-------------|------|-------|
| 6 | `migrations/0001_initial.sql:44-50` + `src/db.rs:240,694` | `inbox_reads.message_id` has no `FK ... ON DELETE CASCADE` to `inbox(id)`. `prune_old_inbox` deletes from `inbox` only, leaving orphan read-receipts forever. The `LEFT JOIN` in `read_inbox` (db.rs:652) also lacks an index on `message_id` (PK is `(session_id, message_id)`, so the inbox-side join scans). | correctness, maintainability, performance-db | 0.96 | manual → downstream-resolver (needs migration 0002) |
| 7 | `src/tools/validate.rs:62-75` + `src/tools/locks.rs:115` | `ttl_in_range` hardcodes `argument: "ttl_ms"` and `next_action: "...milliseconds"`, but the public field is `ttl_sec` and the caller multiplies by 1000 before passing in. Clients receive an error referencing a field that doesn't exist in the schema. | correctness, maintainability, api-contract | 0.97 | safe_auto → review-fixer |
| 8 | `src/db.rs:415-502` | `claim_resource` does prune (acquires/releases mutex), then re-acquires for the check-then-upsert sequence. Atomicity is currently guaranteed by the single `parking_lot::Mutex<Connection>`, not by the SQL. The moment a connection pool or split mutex is introduced this becomes a real TOCTOU. Also re-reads the lock row at the end (lines 493-501) — pure waste; values are already in scope. | correctness, performance-db | 0.92 | manual → downstream-resolver (verification) |
| 9 | `src/db.rs:594-617` (`broadcast`) | No bound on `inbox` row count. A runaway agent loop calling `broadcast` inserts unboundedly within the 24h retention window. Combined with the synchronous mutex this is also a latency cliff for every other tool call once the table grows large. | security, performance-db | 0.92 | gated_auto → downstream-resolver |
| 10 | `src/tools/{validate,presence,inbox,locks}.rs` (all handlers) | `validate::max_len` is defined and unit-tested but **never called** from any handler. `project`/`branch`/`intent`/`resource`/`message`/`reason`/`metadata` are unbounded TEXT going straight to SQLite. ~10 call-sites; uses an existing helper. | security | 0.95 | safe_auto → review-fixer |
| 11 | `src/db.rs:694-699` (`read_inbox`) | The "mark read" step is an N+1 INSERT loop inside one mutex hold. With `MAX_LIMIT=1000` a worst-case `read_inbox` blocks all other tool calls for 1000 sequential `INSERT OR IGNORE` round-trips. Replace with a single bulk insert wrapped in `BEGIN`/`COMMIT`. | performance-db | 0.90 | safe_auto → review-fixer |
| 12 | `src/db.rs:249-255` (`prune_all`) + hot callers | `prune_all` acquires/releases the mutex three times. `list_sessions` (line 361), `claim_resource` (line 415), and `broadcast` (line 595) each call one pruner before re-acquiring for the main query — every hot-path call pays two extra lock cycles. | performance-db | 0.88 | safe_auto → review-fixer |
| 13 | `src/db.rs:358-383, 551-573` (`list_sessions`/`list_locks`) | Identical "match-prepare-match-query" duplication between the two list functions. Also: prune-then-select across two lock acquisitions creates a TOCTOU window the comment at line 360 admits. Fix is one private helper that takes the connection and runs both inside one lock hold. | maintainability, correctness | 0.87 | safe_auto → review-fixer |
| 14 | `src/db.rs:343` (`unregister_session`) | `SELECT COUNT(*) ... .unwrap_or(0) as usize` silently swallows DB errors as "0 locks released". The COUNT and DELETE also share a mutex but no explicit transaction — atomicity rides on the single-mutex assumption. | correctness | 0.88 | safe_auto → review-fixer |
| 15 | `src/tools/presence.rs:41-47, 67-79, 133, 200` | `RegisterOutput.others[].started_at` is `i64` ms (raw); `session_list[].started_at` is an ISO 8601 `String`. Same semantic field, two types. Pre-1.0 is the time to unify (rename to `started_at_ms` everywhere or format both as ISO). | api-contract | 1.00 | manual → downstream-resolver |
| 16 | `src/tools/presence.rs:58, 150` | `HeartbeatOutput.ttl_remaining_sec` is the only time field in the surface that uses seconds; CLAUDE.md mandates ms throughout. Either rename to `_ms` and stop dividing, or document the exception explicitly. | api-contract, correctness | 0.90 | manual → downstream-resolver |
| 17 | `src/tools/{presence,locks,inbox}.rs` | `read_inbox`, `session_list`, and `resource_list` accept no `limit`/`cursor` (or accept it but with no server-side cap). Default `limit=None` returns everything in the project. Pre-1.0 add a default cap (e.g. 100) and a `has_more` flag before the schema hardens. | api-contract | 0.92 | manual → downstream-resolver |
| 18 | `src/mcp.rs:228-234` (`json_to_rmcp`) | The catch-all serialization-failure path returns `ErrorData::new(..., None)` — `data` is `None`, violating the documented `{tool, constraint, next_action}` triple. | api-contract | 1.00 | safe_auto → review-fixer |
| 19 | `src/mcp.rs:120,131-132,145,169` (tool descriptions) | Several tool descriptions are one-liners that omit critical contract notes the LLM needs to call them correctly: `session_list` doesn't list valid `scope` values; `resource_claim` doesn't say `ok=false` is success-without-grant; `resource_release` doesn't mention `force` or idempotency; `broadcast` doesn't mention scope. | api-contract | 0.90 | manual → downstream-resolver |
| 20 | `src/tools/locks.rs:73-81, 143-148` (`ClaimOutput`) | Top-level `expires_at` duplicates `lock.expires_at` when `ok=true` and is omitted when `ok=false`. Confusing for LLM agents — drop the top-level field. | api-contract | 0.85 | safe_auto → review-fixer |
| 21 | `src/config.rs:35-36` | `SANGHA_HOST` is read with no validation. If a user sets it to `0.0.0.0` (deliberately, by copy-paste, or via a misread doc) sangha becomes reachable from the network with **zero authentication** — anyone can register, claim/release, and read the inbox. At minimum warn loudly when not loopback; consider hard-rejecting. | security | 0.95 | manual → human |
| 22 | `tests/e2e.rs:30-32` | `TestDaemon::start()` binds a `TcpListener` to find a free port, drops it, then spawns the daemon — TOCTOU window. With seven E2E tests running concurrently, spurious port-conflict failures are real. | testing | 0.88 | manual → human |
| 23 | `tests/lock_test.rs:294-330` (`force=true` path) | `Db::release_resource` has two branches on `force`; the `force=true` branch deletes by `(project, resource)` only, skipping the session check. **Zero tests exercise it.** | testing | 0.98 | safe_auto → review-fixer |
| 24 | `tests/session_test.rs:216-235` (double unregister) | No test calls `unregister` twice, nor on an unknown id. Both should return `ok=false` with no panic — the contract is untested. | testing | 0.93 | safe_auto → review-fixer |
| 25 | `tests/inbox_test.rs:374-410` (cursor/ordering) | `ORDER BY created_at DESC` and the `unread_total` decrement after a partial `limit=1` read are both unasserted. The current test broadcasts 3 and asserts counts but never which message was returned, nor that a follow-up read returns the next-newest. | testing | 0.90 | safe_auto → review-fixer |
| 26 | `src/tools/locks.rs:126` + `src/tools/inbox.rs:84` | `handle_claim` and `handle_broadcast` each issue a separate `db.get_session(session_id)?` round-trip solely to copy `branch` onto the row. The branch is already in `Identity` at registration time. Cache it on `Identity` and skip the extra lock acquisition. | maintainability, correctness, performance-db | 0.85 | manual → downstream-resolver |
| 27 | `src/tools/validate.rs:1-225` | `validate.rs` lives under `src/tools/` but isn't a tool — no handler, no Args/Output. Convention is "one file per tool in `src/tools/`". Move to `src/validate.rs`. | maintainability | 0.95 | manual → human |
| 28 | `src/lib.rs:1-8` | All 8 modules re-exported as `pub mod` with no facade. `main.rs` reaches into `db::ReleaseInput` and `tools::validate::USER_SCOPE_PROJECT` directly. Pre-1.0 is the time to lock down the surface. | maintainability | 0.80 | manual → human |

---

## P3 — Low

| # | File:Line | Issue | Reviewer(s) | Conf | Route |
|---|-----------|-------|-------------|------|-------|
| 29 | `src/main.rs:374-378` (`shutdown_signal`) | `.expect("failed to install CTRL+C handler")` panics inside the signal handler path, killing the daemon non-gracefully and leaving the PID file behind. Use `.ok()` or log-and-return. | correctness | 0.90 | safe_auto → review-fixer |
| 30 | `src/tools/presence.rs:108` + `src/tools/inbox.rs:82` | `serde_json::to_string(...).unwrap_or_default()` silently stores `""` if serialization fails. On read-back `from_str("")` returns `None` and metadata/tags are silently dropped. Propagate the error instead. | correctness | 0.88 | gated_auto → review-fixer |
| 31 | `src/error.rs:121-127` + `db.rs:166-171` (`Db` & `Internal` arms) | `e.to_string()` from rusqlite errors goes verbatim to MCP clients in `data.received.message`, including DB file paths and constraint names. Sanitize the wire payload; log the raw message at debug level. | security | 0.85 | gated_auto → review-fixer |
| 32 | `src/tools/presence.rs:102` + `db.rs` schema | `project` is treated as opaque identity but never normalized. `/a/b`, `/a/b/`, and `/a/b/./` are three separate lock namespaces. Two sessions in the same dir can fail to see each other's locks. Strip trailing slashes / lex-resolve `..` server-side. | security | 0.80 | advisory → human |
| 33 | `src/tools/locks.rs:182` (`force=true`) | Any localhost session can `resource_release force=true` on any other session's lock. Intentional under the cooperative model — needs documentation in README. | security | 0.95 | advisory → human |
| 34 | `src/main.rs:336, 369` (PID file) | Non-atomic write; never cleaned on abnormal exit. Liveness check uses `TcpStream::connect` so PID file is purely informational — say so or write atomically. | security | 0.75 | advisory → human |
| 35 | `src/mcp.rs:37-44` (`Clone` impl for `SanghaServer`) | Clone calls `Identity::new()` rather than cloning the bound identity. Correct **iff** rmcp only clones the handler at connection-establishment time. If it ever clones mid-session, identity is silently lost. Add a doc comment confirming the intended semantics. | correctness | 0.80 | advisory → human |
| 36 | `src/tools/locks.rs:126` | `branch` is fetched in a separate DB call before `claim_resource`. Re-registering the session between the two calls stores a stale branch on the lock. Cosmetic — covered by finding 26 if branch is cached on `Identity`. | correctness | 0.75 | advisory → human |
| 37 | `src/config.rs:97-99` | `const _: fn() = || { let _: Option<SanghaError> = None; };` — a workaround to silence a dead-code lint for an unused import. The lint is correct; just `use crate::error::Result;` and remove the constant. | maintainability | 0.98 | safe_auto → review-fixer |
| 38 | `src/main.rs:118-122, 170-174` | `if user { Some(USER_SCOPE_PROJECT.to_string()) } else { project }` duplicated. Trivial helper. | maintainability | 0.90 | safe_auto → review-fixer |
| 39 | `migrations/0001_initial.sql:44-50` | No index on `inbox_reads(message_id)`. The `LEFT JOIN` in `read_inbox` filters on session via the PK prefix, but the inbox-side join (and a future `ON DELETE CASCADE`) wants `(message_id, session_id)`. | performance-db | 0.85 | safe_auto → review-fixer |
| 40 | `src/db.rs:365,371,556,561` | All `conn.prepare(...)` on hot list paths — never `prepare_cached`. Microseconds per call but constant and free to fix. | performance-db | 0.75 | safe_auto → review-fixer |
| 41 | `src/db.rs:343-352` | `unregister_session` does COUNT then DELETE in two statements with no explicit transaction. Currently safe because of the single mutex; document or wrap in `BEGIN IMMEDIATE`. | correctness, performance-db | 0.85 | safe_auto → review-fixer |
| 42 | `tests/session_test.rs:42-85` (`test_register_upsert`) | Test name says "started_at must be preserved" but the assertion never reads `started_at`. The 5ms sleep is dead weight. Either capture and compare `started_at` across the two registrations or remove the misleading name and sleep. | testing | 0.93 | safe_auto → review-fixer |
| 43 | `tests/e2e.rs:573-596` | No E2E asserts the wire shape of a `SanghaError::InvalidArgument` (`error.data.tool`, `.argument`, `.constraint`, `.next_action`). The serialization path has no end-to-end coverage. | testing | 0.87 | safe_auto → review-fixer |
| 44 | `tests/inbox_test.rs:374` | `read_inbox` clamps `limit` to `1..=1000` — no test for `limit=0` or `limit=99999`. Trivial coverage win. | testing | 0.95 | safe_auto → review-fixer |
| 45 | various `tests/*.rs` | `prune_dead_sessions`/`prune_expired_locks`/`prune_old_inbox` all return `Result<usize>` (count). Tests discard the count. Asserting `==1` after a forced expiry would catch silent-DELETE regressions. | testing | 0.90 | safe_auto → review-fixer |

---

## Cross-cutting observations

- **The single-mutex DB design is load-bearing.** It is correct, deliberate (per CLAUDE.md), and adequate for the operating envelope. But it is the *one* invariant that prevents findings 8, 13, 14, and 41 from being real defects today. If you ever introduce a connection pool, a separate-mutex-per-table, or move to async SQLite, that work needs to land **with** explicit `BEGIN IMMEDIATE` / `COMMIT` wrappers around the check-then-act sequences. Worth a comment on `Db` saying so.
- **`busy_timeout=5000` ms is dead-letter inside the daemon.** Inside one process the mutex serializes all writers before SQLite sees them, so the busy timeout only fires for *out-of-process* contention — i.e. CLI subcommands run while the daemon is up. Worth a one-line comment in `Db::open` documenting the intent, since the value otherwise reads like a stall hazard.
- **The validate-as-tool naming defect (P1 #1, #2) suggests the file-per-tool/string-name coupling needs a single source of truth.** A `pub const TOOL_NAME: &str = "..."` per tool module, used both at MCP registration and inside validators, would have made this impossible. Cheap once `validate.rs` is moved out (finding 27).
- **Test coverage of the wire contract is weak.** Findings 3, 4, 5, and 43 all surface the same gap: handlers and validators have unit tests, but the JSON-RPC envelope they produce is barely asserted at all. A small fixture that calls each tool over HTTP and snapshots the success and error payloads would catch most of the API-contract findings in this report.
- **Pre-1.0 is the cheap time to fix all the schema/contract drift.** Findings 15, 16, 17, 19, 20 are essentially free to land now and breaking changes after v1.0.

---

## Coverage

- **Suppressed:** zero findings dropped under the 0.60 confidence gate.
- **Reviewers:** all 6 returned. One flagged a partial scope (api-contract reviewer didn't read `db.rs` / `config.rs` end-to-end — its DB-leaning findings were corroborated by the dedicated performance-db reviewer).
- **Out of scope by design:** dependency-CVE audit (`Cargo.lock` not analyzed), rmcp's HTTP-layer request-size limits, multi-machine / multi-user deployments, encryption at rest.
- **Threat-model assumption used by the security reviewer:** single trusted user, localhost-only, all callers are this user's own Claude Code sessions. Severities would shift up by one or two grades for a multi-user or network-exposed deployment.

## Residual risks

- `inbox_reads` orphan growth (finding 6) is silent and unmetered. With current message volume it is years from biting; with a chatty agent loop it can grow much faster.
- The TTL sweeper invocation cadence is not stated in this review — if it's in `src/ttl.rs` and runs every few seconds, finding 12 (three-mutex-acquire `prune_all`) becomes more relevant under load.
- `cmd_clear --all` iterates sessions and calls `unregister_session` in a loop — N separate DELETE statements under N separate mutex acquisitions. Fine at small N; pathological if the DB ever accumulates thousands of zombie rows.
- `parse_env_or` silently falls back to defaults on unparseable values with only an `eprintln` warning. There's no way to distinguish "default" from "misconfigured fell-back" at runtime.

## Verdict

**Ready with fixes.** No P0. The five P1 findings are all routine: two are mechanical search/replace (#1, #2), three are missing tests for already-correct code (#3, #4, #5). The P2 set is dominated by API-contract polish that's nearly free pre-1.0 and one real schema gap (`inbox_reads` orphan growth, #6) that warrants migration `0002`.

**Suggested fix order:**
1. P1 #1 + #2 — fix tool-name strings in `validate::*` calls. ~10 lines.
2. P1 #3 + #4 + #5 — add the missing tests. Each is small and self-contained.
3. P2 #7 — rename `ttl_in_range` argument or thread the field name through. Tied to #15/#16 unit-consistency cleanup.
4. P2 #6 — migration `0002` adding `FOREIGN KEY ... ON DELETE CASCADE` and `idx_inbox_reads_message`.
5. P2 #10 — wire `validate::max_len` into all handlers.
6. P2 #18 — give `json_to_rmcp` a real `data` payload.
7. Sweep the remaining safe_auto fixes (#11, #12, #13, #14, #20, #29, #37, #38, #40, #41, plus the test additions in P3).
8. Schedule the manual/human items (#15, #16, #17, #19, #21, #22, #27, #28) for a small "v0.2 contract & lockdown" pass.

No findings recommended deletion of `docs/brainstorms/`, `docs/plans/`, or `docs/solutions/` paths (per the protected-artifacts rule).
