# Handoff — 2026-04-27 (session 5)

subagents will also try to register a session. the server responds with any error that
is already registered, which is good.

## Pick up next

1. **Commit the stdio-default changes** — `src/main.rs`, `CLAUDE.md`, and `~/.claude/settings.json` are modified but uncommitted.

2. **Port slash commands from claude-presence** — biggest UX win, no code changes to sangha. Create `commands/` dir with `/register`, `/presence`, `/claim`, `/release`, `/broadcast`, `/inbox`. See `docs/port-from-claude-presence.md` for details.

3. **Port UserPromptSubmit hook** — passive session awareness. On every prompt, inject a one-liner when other sessions or locks exist. Reference: `~/soft/claude-presence/hooks/user-prompt-submit.sh`.

4. **Verify stdio reliability** — this session actually landed the code change (prior session only changed config). Confirm over the next few sessions.

5. **v0.2 review findings** — 15 items deferred from `docs/reviews/opus47-initial-review.md`.

## Context for next session

- CLI flag flipped: `sangha serve` now defaults to stdio; `--http` opts into HTTP daemon mode
- `~/.local/bin/sangha` is now a symlink to `~/.cargo/bin/sangha` — only `cargo install --path .` needed going forward
- Settings.json updated: sangha args simplified from `["serve", "--stdio"]` to `["serve"]`
- The binary is installed (release build) but the source changes are uncommitted

## Blockers

None.
