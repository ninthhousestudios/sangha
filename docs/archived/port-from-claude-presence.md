# Port from claude-presence

Reference: `~/soft/claude-presence` (v0.1.1, MIT, ~800 LOC TypeScript).

Sangha and claude-presence solve the same problem with the same 9-tool MCP
surface. claude-presence has a better UX layer. This doc captures what to port.

## 1. Slash commands

claude-presence ships 6 markdown slash commands in `commands/`. Copy the
pattern, adapt tool names to sangha's.

| Command | What it does | Source |
|---|---|---|
| `/register [intent]` | Register session. Auto-detects branch, takes intent from `$ARGUMENTS`. | `commands/register.md` |
| `/presence` | List other sessions + active locks, excluding self. One-screen summary. | `commands/presence.md` |
| `/claim <resource> [reason]` | Claim a named resource lock. Shows holder info on conflict. | `commands/claim.md` |
| `/release <resource>` | Release a lock. | `commands/release.md` |
| `/broadcast <message>` | Post message to project inbox. | `commands/broadcast.md` |
| `/inbox [all\|unread]` | Read inbox messages. Default: unread only. | `commands/inbox.md` |

Destination: `commands/` in the sangha repo, installed to `~/.claude/commands/`.

Key detail from `/register`: it uses the CC session_id from hook payload if
available, otherwise generates a short random one. Sangha generates server-side
UUIDv7s — the slash command should accept the server-generated ID and stash it
for subsequent commands. Consider a `.claude-session` dotfile or env var to
persist the session ID across tool calls within one CC session.

## 2. Hooks

### SessionStart

`hooks/session-start.sh` — emits a `systemMessage` reminding the agent to
register and check locks. Does NOT auto-register (registration is explicit via
`/register` or `session_register` tool call).

For sangha: we already have a SessionStart hook that tells the agent to call
`session_register`. The claude-presence approach is lighter — just a reminder
message, registration is a separate explicit step. Consider whether auto-register
(current sangha) or remind-to-register (claude-presence) is better UX. Auto is
less friction; explicit gives the user a chance to set intent.

### UserPromptSubmit

`hooks/user-prompt-submit.sh` — this is the high-value one. On every user
prompt, shells out to the CLI to check session count + lock count, and injects
a one-liner into `additionalContext`:

```
claude-presence: 2 other session(s) active, 1 active resource lock(s).
Call session_list and resource_list for details before shared operations.
```

Only fires when there's something to report (other sessions or locks exist).
Silent otherwise. Must be fast (< 100ms).

For sangha: implement the equivalent. `sangha status --project $CWD --json`
already exists. Parse it, count others, count locks, emit the one-liner. The
CLI path is `sangha` not `claude-presence`, but the logic is identical.

## 3. Client-provided session IDs

claude-presence takes `session_id` as a client argument (extracted from CC hook
payload). Sangha generates server-side UUIDv7s.

Trade-off:
- Client-provided: session can reconnect after MCP restart, simpler mental model
- Server-generated: no spoofing, guaranteed uniqueness

For local single-machine use, client-provided is more practical. Consider
accepting an optional `client_id` in `session_register` that maps to the
server-generated UUID, enabling reconnection.

## 4. Resilient heartbeat

claude-presence's `session_heartbeat` accepts an optional `recreateWith` payload.
If the session was pruned (TTL expired), it auto-recreates from the payload
instead of failing. This prevents "session not found" errors after long idle
periods.

Sangha should consider the same pattern — heartbeat that auto-recreates is more
robust than requiring the agent to handle the error and re-register.

## 5. Prune-on-read pattern

claude-presence runs `pruneDeadSessions()` inside `listSessions()`, and
`pruneExpiredLocks()` inside `listLocks()` / `claimResource()`. Every read
operation returns clean data — no stale entries.

Sangha already does some of this but should verify it's consistent across all
read paths.

## 6. Output formatting

claude-presence wraps tool responses with user-friendly messages:

- Lock acquired: `"Lock acquired on 'ci'. Remember to resource_release when done."`
- Lock denied: `"Resource 'ci' is already claimed by another session. Consider waiting, coordinating via broadcast, or asking the user."`
- Register with others: `"⚠️ 1 other session(s) active on this project. Check session_list / resource_list before making shared changes."`

Sangha's responses are more terse/structured. Adding these advisory messages
alongside the structured data improves agent behavior without requiring prompt
engineering in CLAUDE.md.

## Implementation order

1. Slash commands (biggest UX win, no code changes to sangha itself)
2. UserPromptSubmit hook (passive awareness)
3. Output message improvements (small code changes)
4. Resilient heartbeat with auto-recreate
5. Client ID mapping (if session reliability remains an issue after stdio is primary)
