# Daemon sketch

Status: idea, not committed to. Capturing while the thinking is fresh.

## The question

Sangha's coordination features (presence, locks, inbox) work fine over stdio +
shared SQLite. The daemon adds HTTP transport complexity that causes the exact
reliability problems we're trying to solve (stale sessions, dropped connections).

So: stdio becomes the primary MCP transport. What is the daemon *for*?

## What a persistent process enables

A daemon can act when no CC session is asking it to. Stdio processes are
reactive — they exist only while CC is running and only respond to tool calls.
A daemon is proactive.

### Event bridge

The daemon listens for external events and routes them to sessions as inbox
messages.

Sources:
- **GitHub webhooks** — PR merged, review requested, CI failed, issue assigned
- **CI status** — build started/passed/failed (GitHub Actions, local `cargo test`)
- **File system watches** — notify sessions when files they care about change
  (e.g. another session committed to the same file)
- **Git ref changes** — branch pushed, rebase detected, upstream changed

Flow:
```
  GitHub ──webhook──▶ daemon ──inbox──▶ active sessions
  CI     ──status───▶ daemon ──inbox──▶ active sessions
  fs     ──inotify──▶ daemon ──inbox──▶ active sessions
```

Sessions read these via the existing `read_inbox` tool or the UserPromptSubmit
hook surfaces a one-liner ("CI failed on feat/login 2 min ago").

### Instant liveness detection

Instead of heartbeat TTLs (which create ghost windows), the daemon tracks
session PIDs. `kill -0 $pid` is instantaneous. Dead process = immediate prune.
No 2-minute lag, no stale entries.

This is the simplest win and directly fixes the annoyance that prompted this
investigation.

### Reactive coordination

Rules the daemon evaluates continuously:

- Session A is waiting on a lock held by session B. B releases it. Daemon
  immediately notifies A via inbox: "ci lock released, you're next."
- Two sessions are on the same branch. Daemon warns both: "sess-B is also on
  feat/login — coordinate before pushing."
- A session hasn't heartbeated in 90s but its PID is alive. Daemon knows it's
  just idle, not dead. No false prune.

### Session supervision

The daemon could spawn and manage CC sessions directly:

```
sangha spawn --project /home/josh/soft/sangha --branch feat/x --intent "fix the nav bug"
```

This creates a CC session, registers it, and the daemon monitors its lifecycle.
On crash, the daemon can restart or notify the user. This becomes an
orchestration layer — "run 3 sessions on different branches, coordinate their
CI access."

Speculative. Not clear this is better than the user opening terminals manually.

## Architecture

```
┌─────────────────────────────────────┐
│            sangha daemon            │
│                                     │
│  ┌───────────┐  ┌───────────────┐   │
│  │ SQLite DB │  │ event bridge  │   │
│  │ (sessions,│  │ (webhooks,    │   │
│  │  locks,   │  │  fs watch,    │   │
│  │  inbox)   │  │  git poll)    │   │
│  └─────┬─────┘  └──────┬────────┘   │
│        │               │            │
│  ┌─────┴───────────────┴─────┐      │
│  │    coordination engine    │      │
│  │  (prune, notify, rules)   │      │
│  └───────────────────────────┘      │
│                                     │
│  ┌──────────┐  ┌─────────────┐      │
│  │ HTTP API │  │  PID monitor │      │
│  │ (webhooks│  │  (liveness)  │      │
│  │  in)     │  │              │      │
│  └──────────┘  └─────────────┘      │
└─────────────────────────────────────┘
         ▲                    
         │ webhooks           
    GitHub / CI               

┌──────────────┐  ┌──────────────┐
│  CC session  │  │  CC session  │
│  (stdio MCP) │  │  (stdio MCP) │
│  reads/writes│  │  reads/writes│
│  SQLite      │  │  SQLite      │
└──────────────┘  └──────────────┘
```

Key: CC sessions use stdio MCP (direct SQLite access). The daemon is a separate
process that also accesses the same SQLite DB for event routing, PID monitoring,
and reactive rules. Sessions don't connect to the daemon — they share the DB.

This means the daemon can go down without breaking session coordination. Sessions
still work via SQLite. The daemon adds value on top, doesn't gate basic function.

## What's NOT worth building

- **Cross-machine coordination** — interesting in theory but the use case is
  single-developer, single-machine. Network-aware coordination is a different
  product.
- **Session spawning/orchestration** — the user can open terminals. A daemon
  managing CC processes adds complexity for marginal convenience.
- **Web UI / dashboard** — `sangha status` in the terminal is enough.

## Decision

Park this. The stdio transport solves the immediate reliability problem. The
daemon idea has potential (especially the event bridge + PID liveness) but
doesn't solve a problem Josh has today. Revisit when:

- Multiple parallel CC sessions become a regular workflow (not occasional)
- External events (CI, GitHub) cause coordination friction that manual checking
  doesn't solve
- The "check sangha status" loop becomes tedious enough to justify push
  notifications
