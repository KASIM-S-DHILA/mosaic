# Phase 1 — Session Storage Skeleton: Design Spec

Status: approved (2026-09-09). Scope: Personal-only, no UI.

## Goal

The `session` / `message` / `part` data model exists in the Tauri backend
and is exercised by unit tests. No UI, no tool calls, no agent loop.

## Non-goals

- No Tauri commands yet (plain Rust API only; commands arrive with consumers).
- No part state machine (Phase 6 owns `proposed → approved → …`; Phase 1 stores
  `state` as an opaque string).
- No migrations framework (single `CREATE TABLE IF NOT EXISTS` bootstrap).

## Schema

```sql
CREATE TABLE session (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  created_at TEXT NOT NULL,   -- RFC3339 UTC
  archived_at TEXT            -- NULL = active
);
CREATE TABLE message (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES session(id),
  created_at TEXT NOT NULL,   -- RFC3339 UTC
  data TEXT NOT NULL          -- JSON document
);
CREATE TABLE part (
  id TEXT PRIMARY KEY,
  message_id TEXT NOT NULL REFERENCES message(id),
  session_id TEXT NOT NULL REFERENCES session(id),  -- denormalized for sweeps
  state TEXT NOT NULL,
  data TEXT NOT NULL          -- JSON document
);
```

- IDs: UUID v4 strings (chosen over ints for the later Dolt/sync phases).
- `data` columns hold JSON as TEXT; parsed with `serde_json` at the boundary.
- Every connection runs `PRAGMA foreign_keys = ON`.
- DB file: `<app-data-dir>/mosaic.db` in production; tests use throwaway
  tempfiles. The open path is a parameter, never hardcoded.

## Module API (`src-tauri/src/store.rs`, new file; `mod store;` in `main.rs`)

- `Store::open(path) -> Result<Store, StoreError>` — opens/creates the DB,
  runs the bootstrap DDL.
- `create_session(title) -> Session`
- `append_message(session_id, data: JsonValue) -> Message`
- `add_part(message_id, session_id, state, data: JsonValue) -> Part`
- `update_part(id, patch: JsonValue) -> Part` — **merge-only**: reads the row,
  shallow-merges the patch object into `data` in Rust, writes it back.
  Non-object patch → error. No other function in the codebase may write part
  rows, so a full overwrite is impossible by construction, not by convention.

## Dependencies (additive, CI-safe)

`rusqlite` (`bundled` — no system SQLite on any platform), `uuid` (`v4`),
`chrono` (RFC3339 UTC). Dev: `tempfile`. All pure-Rust/bundled.

## Tests (`#[cfg(test)]` in `store.rs`, run via `cargo test`)

1. Create a session; assert it round-trips.
2. Add a message with two parts; assert both stored.
3. `update_part` on one part's `state` (+ one data key); assert the other
   part is byte-identical and untouched keys on the patched part survive.
4. "Rejects full overwrite": assert no API exists that replaces `data`
   wholesale (covered by 3's untouched-keys assertion) and that a non-object
   patch returns an error.

## Evaluation

`cargo test` green. Manual: open the DB in a SQLite browser, run a merge
update, confirm only the intended field changed.
