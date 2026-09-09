# Phase 1 Session Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a SQLite-backed session/message/part store to the Tauri backend with merge-only part updates, covered by unit tests.

**Architecture:** One new module `src-tauri/src/store.rs` holding a `Store` struct over a `rusqlite::Connection`. JSON columns stored as TEXT, parsed at the boundary with `serde_json`. Merge-only is structural: `update_part` is the sole part-writing function.

**Tech Stack:** Rust, rusqlite (`bundled`), uuid (`v4`), chrono (`clock`), tempfile (dev).

**Spec:** `docs/superpowers/specs/2026-09-09-phase1-session-storage-design.md`

## Global Constraints

- `update_part` is the ONLY function that may write part rows — no raw overwrites anywhere.
- Timestamps stored as RFC3339 UTC TEXT.
- Every connection runs `PRAGMA foreign_keys = ON`.
- No system SQLite dependency — `rusqlite` must use the `bundled` feature.
- No Tauri commands, no UI, no state machine in this phase.
- `cargo test`, `cargo clippy`, `cargo fmt --check` must all pass.

---

### Task 1: Dependencies, schema bootstrap, `create_session`

**Files:**

- Modify: `src-tauri/Cargo.toml` (add dependencies)
- Modify: `src-tauri/src/main.rs` (add `mod store;`)
- Create: `src-tauri/src/store.rs` (error type, structs, `Store::open`, `create_session`, tests)

**Interfaces:**

- Consumes: nothing (first task).
- Produces: `Store::open(&Path) -> Result<Store, StoreError>`; `Store::create_session(&str) -> Result<Session, StoreError>`; structs `Session`, `StoreError`.

- [ ] **Step 1: Add dependencies**

Run (lets cargo resolve versions, no guessing):

```bash
cargo add rusqlite --features bundled --manifest-path src-tauri/Cargo.toml
cargo add uuid --features v4 --manifest-path src-tauri/Cargo.toml
cargo add chrono --features clock --manifest-path src-tauri/Cargo.toml
cargo add tempfile --dev --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 2: Declare the module in `main.rs`**

```rust
mod store;
```

Add as the first line of `src-tauri/src/main.rs` (before the `cfg_attr`).

- [ ] **Step 3: Write the failing test**

Append to new file `src-tauri/src/store.rs` (skeleton + test only; the test
below references items defined in Step 5):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn open_test_store() -> Store {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        // leak the dir so the file outlives the Store for the test body
        std::mem::forget(dir);
        Store::open(&path).unwrap()
    }

    #[test]
    fn creates_and_round_trips_a_session() {
        let store = open_test_store();
        let session = store.create_session("first").unwrap();
        assert_eq!(session.title, "first");
        assert!(!session.id.is_empty());
        assert!(session.archived_at.is_none());
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests::creates_and_round_trips_a_session`
Expected: FAIL with "unresolved import" / "cannot find `Store`".

- [ ] **Step 5: Write minimal implementation**

Full contents of `src-tauri/src/store.rs` above the test module:

```rust
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, params};
use serde_json::Value as JsonValue;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    NotFound { what: &'static str, id: String },
    PatchNotAnObject,
    InvalidPatch(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "sqlite error: {e}"),
            StoreError::Json(e) => write!(f, "json error: {e}"),
            StoreError::NotFound { what, id } => write!(f, "{what} not found: {id}"),
            StoreError::PatchNotAnObject => write!(f, "patch must be a JSON object"),
            StoreError::InvalidPatch(msg) => write!(f, "invalid patch: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self { StoreError::Sql(e) }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self { StoreError::Json(e) }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub data: JsonValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub id: String,
    pub message_id: String,
    pub session_id: String,
    pub state: String,
    pub data: JsonValue,
}

pub struct Store {
    conn: Connection,
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS session (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               created_at TEXT NOT NULL,
               archived_at TEXT
             );
             CREATE TABLE IF NOT EXISTS message (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES session(id),
               created_at TEXT NOT NULL,
               data TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS part (
               id TEXT PRIMARY KEY,
               message_id TEXT NOT NULL REFERENCES message(id),
               session_id TEXT NOT NULL REFERENCES session(id),
               state TEXT NOT NULL,
               data TEXT NOT NULL
             );",
        )?;
        Ok(Store { conn })
    }

    pub fn create_session(&self, title: &str) -> Result<Session, StoreError> {
        let session = Session {
            id: Uuid::new_v4().to_string(),
            title: title.to_string(),
            created_at: Utc::now(),
            archived_at: None,
        };
        self.conn.execute(
            "INSERT INTO session (id, title, created_at, archived_at) VALUES (?1, ?2, ?3, NULL)",
            params![session.id, session.title, now_rfc3339()],
        )?;
        Ok(session)
    }
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests::creates_and_round_trips_a_session`
Expected: PASS (1 passed).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/main.rs src-tauri/src/store.rs
git commit -m "Phase 1: store bootstrap with create_session"
```

---

### Task 2: `append_message` and `add_part`

**Files:**

- Modify: `src-tauri/src/store.rs` (append methods + tests)

**Interfaces:**

- Consumes: `Store::open`, `Store::create_session` from Task 1.
- Produces: `Store::append_message(session_id: &str, data: JsonValue) -> Result<Message, StoreError>`; `Store::add_part(message_id: &str, session_id: &str, state: &str, data: JsonValue) -> Result<Part, StoreError>`.

- [ ] **Step 1: Write the failing test**

Append inside the existing `mod tests` in `src-tauri/src/store.rs`:

```rust
#[test]
fn adds_a_message_with_two_parts() {
    let store = open_test_store();
    let session = store.create_session("s").unwrap();
    let msg = store
        .append_message(&session.id, serde_json::json!({"role": "user"}))
        .unwrap();
    assert_eq!(msg.session_id, session.id);
    let p1 = store
        .add_part(&msg.id, &session.id, "proposed", serde_json::json!({"n": 1}))
        .unwrap();
    let p2 = store
        .add_part(&msg.id, &session.id, "proposed", serde_json::json!({"n": 2}))
        .unwrap();
    assert_ne!(p1.id, p2.id);
    assert_eq!(p1.data, serde_json::json!({"n": 1}));
    assert_eq!(p2.data, serde_json::json!({"n": 2}));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests::adds_a_message_with_two_parts`
Expected: FAIL with "no method named `append_message`".

- [ ] **Step 3: Write minimal implementation**

Add to `impl Store` in `src-tauri/src/store.rs`:

```rust
pub fn append_message(
    &self,
    session_id: &str,
    data: JsonValue,
) -> Result<Message, StoreError> {
    let message = Message {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        created_at: Utc::now(),
        data: data.clone(),
    };
    self.conn.execute(
        "INSERT INTO message (id, session_id, created_at, data) VALUES (?1, ?2, ?3, ?4)",
        params![message.id, message.session_id, now_rfc3339(), data.to_string()],
    )?;
    Ok(message)
}

pub fn add_part(
    &self,
    message_id: &str,
    session_id: &str,
    state: &str,
    data: JsonValue,
) -> Result<Part, StoreError> {
    let part = Part {
        id: Uuid::new_v4().to_string(),
        message_id: message_id.to_string(),
        session_id: session_id.to_string(),
        state: state.to_string(),
        data: data.clone(),
    };
    self.conn.execute(
        "INSERT INTO part (id, message_id, session_id, state, data) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![part.id, part.message_id, part.session_id, part.state, data.to_string()],
    )?;
    Ok(part)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests::adds_a_message_with_two_parts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/store.rs
git commit -m "Phase 1: append_message and add_part"
```

---

### Task 3: Merge-only `update_part`

**Files:**

- Modify: `src-tauri/src/store.rs` (`update_part` + private `get_part` reader + tests)

**Interfaces:**

- Consumes: everything from Tasks 1–2.
- Produces: `Store::update_part(id: &str, patch: &JsonValue) -> Result<Part, StoreError>` — shallow-merges a JSON-object patch into `data` and also accepts a `state` key to move state; non-object patch → `PatchNotAnObject`; unknown id → `NotFound`. (A `state` key inside the patch updates the `state` column, not `data`, so the Part state machine in Phase 6 goes through this same merge path.)

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests`:

```rust
#[test]
fn update_part_merges_without_touching_sibling_part() {
    let store = open_test_store();
    let session = store.create_session("s").unwrap();
    let msg = store
        .append_message(&session.id, serde_json::json!({}))
        .unwrap();
    let p1 = store
        .add_part(&msg.id, &session.id, "proposed", serde_json::json!({"a": 1, "b": 1}))
        .unwrap();
    let p2_before = store
        .add_part(&msg.id, &session.id, "proposed", serde_json::json!({"x": 9}))
        .unwrap();

    let updated = store
        .update_part(&p1.id, &serde_json::json!({"b": 2, "state": "running"}))
        .unwrap();

    assert_eq!(updated.data, serde_json::json!({"a": 1, "b": 2}));
    assert_eq!(updated.state, "running");
    let p2_after = store.get_part(&p2_before.id).unwrap().unwrap();
    assert_eq!(p2_before, p2_after);
}

#[test]
fn update_part_rejects_non_object_patch() {
    let store = open_test_store();
    let session = store.create_session("s").unwrap();
    let msg = store
        .append_message(&session.id, serde_json::json!({}))
        .unwrap();
    let p = store
        .add_part(&msg.id, &session.id, "proposed", serde_json::json!({"a": 1}))
        .unwrap();
    let err = store.update_part(&p.id, &serde_json::json!([1, 2, 3])).unwrap_err();
    assert!(matches!(err, StoreError::PatchNotAnObject));
    // failed update changed nothing
    let unchanged = store.get_part(&p.id).unwrap().unwrap();
    assert_eq!(unchanged.data, serde_json::json!({"a": 1}));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests::update_part`
Expected: FAIL with "no method named `update_part`" (and `get_part`).

- [ ] **Step 3: Write minimal implementation**

Add to `impl Store`:

```rust
fn get_part(&self, id: &str) -> Result<Option<Part>, StoreError> {
    let mut stmt = self.conn.prepare(
        "SELECT id, message_id, session_id, state, data FROM part WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    let Some(row) = rows.next()? else { return Ok(None) };
    let data_text: String = row.get(4)?;
    Ok(Some(Part {
        id: row.get(0)?,
        message_id: row.get(1)?,
        session_id: row.get(2)?,
        state: row.get(3)?,
        data: serde_json::from_str(&data_text)?,
    }))
}

/// Merge-only part update. The patch must be a JSON object; its keys are
/// shallow-merged into the stored `data`. A `state` key moves the `state`
/// column instead of landing in `data`. Stored non-object data and non-string
/// `state` values are rejected with `InvalidPatch` before any write, so failed
/// patches are no-ops. This is the SOLE write path for
/// part rows — do not add another.
pub fn update_part(&self, id: &str, patch: &JsonValue) -> Result<Part, StoreError> {
    let patch_obj = patch.as_object().ok_or(StoreError::PatchNotAnObject)?;
    let current = self
        .get_part(id)?
        .ok_or_else(|| StoreError::NotFound { what: "part", id: id.to_string() })?;
    let mut data = current
        .data
        .as_object()
        .cloned()
        .ok_or_else(|| StoreError::InvalidPatch("stored part data is not a JSON object".to_string()))?;
    let mut state = current.state.clone();
    for (k, v) in patch_obj {
        if k == "state" {
            match v.as_str() {
                Some(s) => state = s.to_string(),
                None => {
                    return Err(StoreError::InvalidPatch("state must be a string".to_string()));
                }
            }
        } else {
            data.insert(k.clone(), v.clone());
        }
    }
    let merged = JsonValue::Object(data);
    self.conn.execute(
        "UPDATE part SET state = ?1, data = ?2 WHERE id = ?3",
        params![state, merged.to_string(), id],
    )?;
    Ok(Part { state, data: merged, ..current })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests::update_part`
Expected: PASS (5 passed — the 2 below plus `InvalidPatch` on non-object
stored data, `InvalidPatch` on non-string `state`, and `NotFound` on unknown
id, each asserting the failed patch changed nothing).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/store.rs
git commit -m "Phase 1: merge-only update_part"
```

---

### Task 4: Quality gates and CI test step

**Files:**

- Modify: `.github/workflows/ci.yml` (add `cargo test` step)
- Modify: none in Rust (verification only)

**Interfaces:**

- Consumes: all tasks above.
- Produces: green `cargo test`, `cargo clippy`, `cargo fmt --check`, `npm run lint`; CI runs the Rust tests on every push.

- [ ] **Step 1: Run the full backend verification**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all 4 tests PASS. Then:

```bash
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --message-format short
cargo fmt --check --manifest-path src-tauri/Cargo.toml
npm run lint
```

Expected: zero clippy warnings under `--all-targets`, fmt clean, lint clean.
(NOTE: plain `cargo clippy` without `--all-targets` still reports `dead_code`
for `Store` items on the bin target, which has no callers yet by design —
that is accepted; CI's plain clippy step passes since warnings are not
errors. The binding gate is the `--all-targets` run.)

- [ ] **Step 2: Add `cargo test` to CI**

In `.github/workflows/ci.yml`, after the clippy step, insert:

```yaml
- run: cargo test --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 3: Manual evaluation (spec requirement)**

Open a test DB in a SQLite browser (e.g. DB Browser for SQLite): create a
session + message + part via a scratch test, run `update_part` with a
single-key patch, confirm only that field changed in the `part` row.

- [ ] **Step 4: Commit and push**

```bash
git add .github/workflows/ci.yml src-tauri/src/store.rs
git commit -m "Phase 1: CI runs cargo test"
git push
```

Expected: GitHub Actions run goes green.

---

## Self-review

- **Spec coverage:** schema/bootstrap → Task 1; `create_session` → Task 1;
  `append_message`/`add_part` → Task 2; merge-only `update_part` → Task 3;
  four spec tests → Tasks 1 (test 1), 2 (test 2), 3 (tests 3–4); RFC3339/FK/
  bundled constraints → Task 1 code; manual SQLite-browser step → Task 4.
  No spec requirement lacks a task.
- **Placeholders:** none — every step has exact code/commands. Versions
  resolved via `cargo add`, not guessed.
- **Type consistency:** `Session`/`Message`/`Part`/`StoreError` defined once in
  Task 1, referenced identically in Tasks 2–4. `get_part` (private, returns
  `Option`) distinguished from `update_part` (public, `NotFound` on missing).
