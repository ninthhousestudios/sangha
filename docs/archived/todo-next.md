# todo — next on sangha

Things worth doing before adding new primitives. Cheap, grounded in real use, no new transport semantics.

## 1. `list_sessions` tool

Pull-form intent brokering. Returns active sessions with `(session_id, identity, intent, last_heartbeat, registered_at)`. No new state — purely a query over what `session_register` already stores.

Why: lets a waking agent see "is anyone else active in this repo, and what are they doing?" without pushing new infrastructure. This is the cheapest of gemini's four sangha suggestions and the only one with an obvious caller today.

Filter args worth considering: `identity_prefix`, `active_within_secs`, `intent_contains`. Keep the tool small.

## 2. Freshness signals on existing responses

Add `as_of` (server time, ms) and `last_heartbeat` (where applicable) to every read response. Caller can decide if data is too stale to trust.

This is the sangha-side of the manas-wide freshness-envelope principle. See `manas/docs/freshness-envelopes.md` (TBD) for the cross-subsystem rule.

Touches: `read_inbox`, any future `list_sessions`, lock status reads.

## 3. Real two-agent E2E tests

Current E2E tests are single-client. Run two concurrent CC sessions against the same daemon and exercise:

- Both register, both heartbeat, one expires.
- Both contend for the same lock — verify TTL semantics under realistic timing.
- One broadcasts, the other reads inbox after a delay.
- One claims a lock and crashes (no release) — verify TTL takeover.

Goal: surface which of the deferred features (queues, blackboard, push) are actually needed vs. theoretical. Don't add features without a failing real-world scenario.

## Out of scope here

See `defer-until-required.md` for the things deliberately not on this list.
