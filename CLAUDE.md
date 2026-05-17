# sangha

Session coordination daemon for manas. Rust, MPL 2.0.

## Build & Test

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

## Patterns

- Mirror chitta-rs crate patterns (rmcp, thiserror, clap, axum, lib+bin split)
- One file per tool in `src/tools/`
- TDD: tests written before implementation
- All timestamps are Unix milliseconds (i64)
- SQLite WAL mode, foreign_keys=ON, busy_timeout=5000
- PRAGMAs set per-connection in Db::open(), NOT in migrations
- Error types always populate `{tool, constraint, next_action}` triple
- Session IDs are server-generated UUIDv7
- `parking_lot::Mutex` for DB (no poisoning)
- DB calls are sync (parking_lot::Mutex), called directly from async handlers

## Architecture

- `sangha serve` (default) uses stdio transport
- `sangha serve --http` binds TCP localhost:3200 (multi-session)
- Each HTTP connection gets its own MCP session + Identity
- Db is shared via Arc

## Naming

- Filenames: lowercase, hyphens for .md files
- Rust: standard snake_case conventions
