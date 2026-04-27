# Handoff — 2026-04-27

### joshs notes

(==+++==>>>>) sangha
1 active session(s):

  019dcd00-6d01-7822-82aa-89eda4e218ae (/home/josh/nhs/soft/pages)

[josh@amma] [main] [~/soft/smriti] 
(==+++==>>>>) sangha
1 active session(s):

  019dcd18-7673-7823-84e3-2e2e008eba99 (/home/josh/nhs/soft/claudius_vimius)
    branch: caesar
    intent: Add UI for responding to tool permission prompts from Claude

[josh@amma] [main] [~/soft/smriti] 

but the first session was still going. so something happened


## In progress

A pile of uncommitted WIP from before this session that wasn't reviewed:

- `src/main.rs` — refactor splitting `Commands::*` arms into `cmd_serve`/`cmd_status`/`cmd_locks`/`cmd_clear`/`cmd_health` helpers
- `src/identity.rs` — added project-rebind guard alongside the session-rebind guard
- `src/util.rs` (new) + `src/lib.rs` (`pub mod util`) — `format_ms` extracted out of `src/tools/presence.rs`
- `src/tools/presence.rs` — uses `crate::util::format_ms` (paired with the util.rs extraction)
- `src/db.rs`, `migrations/0001_initial.sql` — modified, contents not reviewed
- `src/tools/inbox.rs`, `src/tools/locks.rs` — modified, contents not reviewed
- `tests/identity_test.rs` — modified, presumably for the rebind guard
- `Cargo.toml`, `Cargo.lock`, `README.md` — modified
- Untracked: `docs/defer-until-required.md`, `docs/reviews/`, `docs/todo-next.md`

The deployed `~/.local/bin/sangha` was built **from this WIP**, so what's running is ahead of `main` (commit `b8b1ca3`). Functionally equivalent for the MCP-discovery fix, but you'll want to either (a) review and commit the WIP, or (b) rebuild from HEAD if you want the binary to match.

## Pick up next

1. Triage the WIP. Likely two or three logical commits:
   - identity rebind guard + test
   - `main.rs` cmd_* refactor
   - `format_ms` extraction (presence.rs + util.rs + lib.rs)
   - whatever the db.rs / migrations / inbox.rs / locks.rs changes are
2. Read the new `docs/defer-until-required.md`, `docs/todo-next.md`, `docs/reviews/` and decide whether they belong in the manifest (`docs/index.md`).
3. Sanity-check that Claude Code in a fresh session now sees `mcp__sangha__*` tools (this session's CC won't pick them up until restarted — its discovery already failed and was cached).

## Context for next session

- Today's bug: `RegisterArgs.metadata: Option<serde_json::Value>` → schemars rendered `metadata: true` (boolean any-schema), which Claude Code and Gemini CLI both reject as "Invalid input" at `tools[7].inputSchema.properties.metadata` in their Zod validators, dropping the entire tools/list response. Fixed by switching to `Option<HashMap<String, Value>>` (commit `b8b1ca3`). **Don't reintroduce raw `serde_json::Value` in any future MCP input struct.**
- Diagnostic gold: `~/.cache/claude-cli-nodejs/-home-josh-soft-sangha/mcp-logs-sangha/*.jsonl` — CC's per-server logs show validator errors that the daemon side (rmcp INFO) does not log.

## Blockers

None.
