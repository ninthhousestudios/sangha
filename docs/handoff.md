# Handoff — 2026-04-27 (session 3)

## Pick up next

1. **v0.2 contract & lockdown pass** — 15 review findings deferred from the Opus 4.7 review (docs/reviews/opus47-initial-review.md). Key items:
   - **#15/#16**: Unify `started_at` type (i64 ms in RegisterOutput vs ISO string in ListOutput) and rename `ttl_remaining_sec` to `_ms` or document the exception
   - **#17**: Add default limit cap + `has_more` flag to `session_list`, `resource_list`, `read_inbox`
   - **#19**: Expand tool descriptions with contract notes (scope values, ok=false semantics, force, idempotency)
   - **#21**: Warn or reject non-loopback `SANGHA_HOST` — zero-auth network exposure risk
   - **#27**: Move `validate.rs` out of `src/tools/` (it's not a tool)
   - **#28**: Lock down `pub mod` surface in `lib.rs`
   - **#3**: E2E test for Drop-based auto-unregister (open MCP connection, register, drop, assert session disappears)

2. **Gemini session_register visibility** — still open from prior session. Gemini connects but doesn't call session_register.

3. **`sangha locks` text output** — doesn't print branch (JSON mode does). One-liner fix in `cmd_locks`.

## Context for next session

- HEAD is at 4 new commits past `2bc44d5` — review implementation work
- Test suite is 98 tests, all passing, clippy clean
- Migration 0002 drops and recreates `inbox_reads` — safe pre-1.0 but would need data-preserving approach post-1.0
- The single-mutex DB design is explicitly load-bearing and documented as such in the review; several consolidation changes in this session depend on it

## Blockers

None.
