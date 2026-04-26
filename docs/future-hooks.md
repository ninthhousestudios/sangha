# Future: hook integration

Deferred to manas-cli phase 4. This document captures the planned hook
integration for when the manas CLI orchestrator lands.

## Goal

Automate session registration so agents don't need a CLAUDE.md instruction
to call `session_register` manually.

## Planned hooks

### SessionStart hook

On every Claude Code session start, automatically register with sangha:

```bash
#!/usr/bin/env bash
# hooks/session-start.sh
curl -s -X POST http://127.0.0.1:3200/mcp \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 1,
    \"method\": \"tools/call\",
    \"params\": {
      \"name\": \"session_register\",
      \"arguments\": {
        \"project\": \"$PWD\",
        \"branch\": \"$(git branch --show-current 2>/dev/null)\"
      }
    }
  }"
```

### UserPromptSubmit hook

On each prompt submission, send a heartbeat:

```bash
#!/usr/bin/env bash
# hooks/user-prompt-submit.sh
curl -s -X POST http://127.0.0.1:3200/mcp \
  -H "Content-Type: application/json" \
  -H "Mcp-Session-Id: $SANGHA_SESSION_ID" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 1,
    \"method\": \"tools/call\",
    \"params\": {
      \"name\": \"session_heartbeat\",
      \"arguments\": {}
    }
  }"
```

## Current approach (v1)

For v1, session registration is handled by a CLAUDE.md instruction that tells
the agent to call `session_register` at session start:

```markdown
## Sangha — Session Coordination

At session start, call `session_register` with:
- project: current working directory path
- branch: current git branch (if in a git repo)
- intent: brief description of what this session will do (if known)

Before writing docs/handoff.md, call `resource_claim` with resource="handoff".
Release after writing. If claim fails, surface the conflict to the user.
```

## When to implement

After manas-cli lands (phase 4 of the manas roadmap) and provides a hook
execution framework that can inject environment variables and run scripts
at session lifecycle events.
