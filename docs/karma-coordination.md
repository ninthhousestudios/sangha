# sangha — enhancements for karma coordination

Status: design-session prep
Date: 2026-05-08
Context: brainstorming session on karma + usage report analysis

---

## the problem

Sangha provides session coordination (registry, locks, broadcast, inbox). Karma needs to use sangha as the agent-to-agent communication channel — dispatching agents, monitoring their progress, and enabling multi-model adversarial reviews. Current sangha has the right primitives but needs targeted enhancements.

## what karma needs from sangha

### 1. targeted messaging

Broadcast today goes to everyone. Karma needs:
- Send to a specific session: `broadcast(to_session=<id>)`
- Send to a group: `broadcast(channel="review-run-42")`
- Topic-based filtering on `read_inbox`

### 2. structured payloads

Current broadcast is human-readable text. Karma agents need machine-readable messages:

```json
{
  "type": "finding",
  "lens": "security",
  "severity": "high",
  "file": "src/handlers/store.rs",
  "line": 42,
  "summary": "unchecked SQL interpolation in dynamic filter"
}
```

For v0: JSON in the text field with a `payload_type` discriminator. Protobuf deferred until polyglot surface demands it (multiple harnesses in different languages).

### 3. richer session metadata

Karma-dispatched agents register with metadata describing their role:

```json
{
  "dispatched_by": "karma",
  "run_id": "review-run-42",
  "lens": "security",
  "model": "claude-sonnet-4-6",
  "task_id": "sutra/19"
}
```

This lets karma filter `session_list` to its own agents, monitor progress, and correlate findings.

### 4. hard registration enforcement

Current state: CLAUDE.md says "register with sangha at session start" — soft contract. Agents sometimes skip it.

Options:
- **SessionStart hook** — calls `sangha session_register` before agent gets control. Works for Claude Code, needs per-harness adaptation.
- **manas serve as gateway** — no registration, no tools. Universal enforcement. Preferred long-term.

### 5. multi-harness support

The killer use case: adversarial design reviews with multiple models debating via sangha.

```
karma dispatches:
  claude (via claude -p) → architectural coherence lens
  gemini (via gemini-cli) → scalability lens
  codex (via codex) → implementation feasibility lens

All register with sangha. All read the design doc. All post findings.
They can read and respond to each other's findings → real-time multi-model debate.
```

Requires: sangha registration works from any MCP client, not just Claude Code. Message ordering preserved. Session metadata includes model identity.

## open questions

1. **Message ordering guarantees.** For multi-model debate, agents need to read the conversation in order. Is the inbox strictly ordered? Is there a cursor mechanism?

2. **Channel/topic lifecycle.** Who creates channels? Auto-created by karma on dispatch? TTL-based cleanup?

3. **Message size limits.** Review findings can include code snippets. What's the practical limit for broadcast payloads?

4. **Presence vs messaging.** Sangha is currently presence-oriented (who's here, what are they holding). Adding messaging makes it more of a bus. Is that scope creep, or natural evolution?

## references

- `sangha/docs/daemon-sketch.md` — current design
- karma design sketch § multi-harness adversarial reviews
- manas architecture § component interaction rules
- Principle 6: hard vs soft contracts
