# sangha

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)

Session coordination daemon for [manas](https://github.com/ninthhousestudios/manas). Provides a session registry with heartbeat TTL, advisory resource locks, and a broadcast inbox — all backed by SQLite.

Design inspired by [claude-presence](https://github.com/AshDevFr/claude-presence) by Georges Garnier (MIT).

## Quick start

```bash
cargo build --release

# Start the daemon (default: 127.0.0.1:3200)
sangha serve

# In another terminal, check status
sangha status
sangha health
```

## Architecture

One long-lived `sangha` process, multiple Claude Code sessions connecting via streamable HTTP:

```
CC Session 1  ──HTTP──→  ┌─────────────────────┐
CC Session 2  ──HTTP──→  │  sangha daemon       │  ──→  ~/.sangha/state.db
CC Session 3  ──HTTP──→  │  (axum + rmcp)       │
                          │  127.0.0.1:3200      │
                          └─────────────────────┘
```

Each HTTP connection gets its own MCP session with independent identity. The first `session_register` call binds the connection. All subsequent tool calls derive session context from that binding.

SQLite in WAL mode. Advisory trust model — locks are cooperative, not enforced.

## MCP tools

| Tool | Description | Identity required |
|------|-------------|:-----------------:|
| `session_register` | Register or re-register a session | No (creates it) |
| `session_heartbeat` | Bump session TTL, auto-extend locks | Yes |
| `session_unregister` | Remove session, cascade-release locks | Yes |
| `session_list` | List active sessions | No |
| `resource_claim` | Claim or renew an advisory lock | Yes |
| `resource_release` | Release a lock | Yes |
| `resource_list` | List active locks | No |
| `broadcast` | Post a message to the project inbox | Yes |
| `read_inbox` | Read inbox messages, mark as read | Yes |

### session_register

```json
{
  "project": "/home/user/my-project",
  "branch": "main",
  "intent": "implementing feature X",
  "metadata": {"editor": "vscode"}
}
```

Returns `session_id` (server-generated UUIDv7) and `others` (list of other active sessions on the same project).

### resource_claim

```json
{
  "resource": "handoff",
  "scope": "project",
  "reason": "writing docs/handoff.md",
  "long_op": false,
  "ttl_sec": 600
}
```

- `scope`: `"project"` (default) or `"user"` (cross-project via `__user__` sentinel)
- `long_op`: uses extended TTL (default 30 min vs 10 min)
- Returns `held_by` info if another session holds the resource

### broadcast / read_inbox

```json
{ "message": "Finished refactoring auth module", "tags": ["done", "auth"] }
```

```json
{ "unread_only": true, "limit": 10 }
```

## CLI commands

| Command | Description |
|---------|-------------|
| `sangha serve` | Start the daemon (default: HTTP on 127.0.0.1:3200) |
| `sangha serve --stdio` | Stdio transport (testing / mcpjungle) |
| `sangha status` | Show active sessions (default when no command given) |
| `sangha locks` | Show active resource locks |
| `sangha clear --all` | Clear all sessions and locks |
| `sangha clear --force-release <resource>` | Force-release a specific lock |
| `sangha path` | Print the database path |
| `sangha health` | Check daemon and database health |

All inspection commands (`status`, `locks`, `clear`, `health`) open the DB directly — no running daemon required. Add `--json` for machine-readable output.

## Configuration

All settings via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `SANGHA_DB_PATH` | `~/.sangha/state.db` | SQLite database path |
| `SANGHA_HOST` | `127.0.0.1` | Bind address |
| `SANGHA_PORT` | `3200` | Bind port |
| `SANGHA_SESSION_TTL_SEC` | `600` | Session heartbeat TTL |
| `SANGHA_LOCK_TTL_SEC` | `600` | Default lock TTL |
| `SANGHA_LOCK_LONG_OP_TTL_SEC` | `1800` | Long-operation lock TTL |
| `SANGHA_LOCK_MAX_TTL_SEC` | `86400` | Maximum lock TTL |
| `SANGHA_INBOX_RETENTION_SEC` | `86400` | Inbox message retention |
| `SANGHA_LOG_LEVEL` | `info` | Tracing filter level |

## Claude Code MCP config

```json
{
  "sangha": {
    "type": "streamable-http",
    "url": "http://127.0.0.1:3200/mcp"
  }
}
```

## Lock vocabulary

| Resource | Scope | Used by |
|----------|-------|---------|
| `handoff` | project | Writing `docs/handoff.md` |
| `reflect:user` | user | Running `/reflect` skill |
| `smriti-scan` | user | Running smriti background scan |

