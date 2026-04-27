# defer until required — sangha

Features suggested (mostly by gemini-flash, 2026-04-26) that we are intentionally **not** building yet. Each requires a concrete failing scenario before reconsideration.

## 1. Wait queues for locks

**Suggestion:** instead of binary win/fail on `resource_claim`, register interest and be notified when the lock frees.

**Why deferred:** today's realistic concurrency is 1–2 agents. Binary fail + retry handles this. Wait queues require either push notifications (poor fit for current MCP-over-HTTP transport) or long-polling, plus TTL/abandonment semantics for waiters.

**Reconsider when:** real two-agent E2E tests show repeated failed claims, or a workflow appears where the loser of a claim actually needs to do something other than retry.

## 2. Shared blackboard / ephemeral KV

**Suggestion:** TTL'd last-write-wins key/value store for current-session state ("currently refactoring auth"), separate from inbox messages.

**Why deferred:** broadcast + read_inbox + the future `list_sessions` (with intent) covers ~80% of the use cases in the suggestion. Adding a third memory shape (alongside chitta long-term and inbox messages) needs a concrete case where neither existing primitive fits.

**Reconsider when:** a real workflow needs last-write-wins state that's read frequently, written rarely, and shouldn't pollute the inbox stream.

## 3. Sideband event bus / push notifications

**Suggestion:** sangha as the message bus where smriti/chitta publish "scan complete" / "model updated" events and active sessions subscribe.

**Why deferred:** zero publishers and zero subscribers exist today. MCP-over-HTTP is a poor substrate for server-push (would need SSE or notifications protocol work). Pull + freshness envelopes gets ~90% of the value at ~10% of the complexity.

**Reconsider when:** at least one concrete (publisher, subscriber, event) triple exists and a polling implementation has been measured to be too slow or too costly.

## Meta

Gemini's framing — "move from static servers that answer questions to a live system that notifies me when the world changes" — is appealing but premature. Push semantics are expensive in MCP and the world doesn't change fast enough in single-user agentic work to need them. Stay pull-shaped until proven otherwise.
