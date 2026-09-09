use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection};
use serde_json::Value as JsonValue;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    NotFound { what: &'static str, id: String },
    PatchNotAnObject,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "sqlite error: {e}"),
            StoreError::Json(e) => write!(f, "json error: {e}"),
            StoreError::NotFound { what, id } => write!(f, "{what} not found: {id}"),
            StoreError::PatchNotAnObject => write!(f, "patch must be a JSON object"),
        }
    }
}

impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
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

    pub fn append_message(&self, session_id: &str, data: JsonValue) -> Result<Message, StoreError> {
        let message = Message {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            created_at: Utc::now(),
            data: data.clone(),
        };
        self.conn.execute(
            "INSERT INTO message (id, session_id, created_at, data) VALUES (?1, ?2, ?3, ?4)",
            params![
                message.id,
                message.session_id,
                now_rfc3339(),
                data.to_string()
            ],
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

    fn get_part(&self, id: &str) -> Result<Option<Part>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, message_id, session_id, state, data FROM part WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
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
    /// column instead of landing in `data`. This is the SOLE write path for
    /// part rows — do not add another.
    pub fn update_part(&self, id: &str, patch: &JsonValue) -> Result<Part, StoreError> {
        let patch_obj = patch.as_object().ok_or(StoreError::PatchNotAnObject)?;
        let current = self.get_part(id)?.ok_or_else(|| StoreError::NotFound {
            what: "part",
            id: id.to_string(),
        })?;
        let mut data = current.data.as_object().cloned().unwrap_or_default();
        let mut state = current.state.clone();
        for (k, v) in patch_obj {
            if k == "state" {
                state = v.as_str().unwrap_or(&state).to_string();
            } else {
                data.insert(k.clone(), v.clone());
            }
        }
        let merged = JsonValue::Object(data);
        self.conn.execute(
            "UPDATE part SET state = ?1, data = ?2 WHERE id = ?3",
            params![state, merged.to_string(), id],
        )?;
        Ok(Part {
            state,
            data: merged,
            ..current
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn adds_a_message_with_two_parts() {
        let store = open_test_store();
        let session = store.create_session("s").unwrap();
        let msg = store
            .append_message(&session.id, serde_json::json!({"role": "user"}))
            .unwrap();
        assert_eq!(msg.session_id, session.id);
        let p1 = store
            .add_part(
                &msg.id,
                &session.id,
                "proposed",
                serde_json::json!({"n": 1}),
            )
            .unwrap();
        let p2 = store
            .add_part(
                &msg.id,
                &session.id,
                "proposed",
                serde_json::json!({"n": 2}),
            )
            .unwrap();
        assert_ne!(p1.id, p2.id);
        assert_eq!(p1.data, serde_json::json!({"n": 1}));
        assert_eq!(p2.data, serde_json::json!({"n": 2}));
    }

    #[test]
    fn update_part_merges_without_touching_sibling_part() {
        let store = open_test_store();
        let session = store.create_session("s").unwrap();
        let msg = store
            .append_message(&session.id, serde_json::json!({}))
            .unwrap();
        let p1 = store
            .add_part(
                &msg.id,
                &session.id,
                "proposed",
                serde_json::json!({"a": 1, "b": 1}),
            )
            .unwrap();
        let p2_before = store
            .add_part(
                &msg.id,
                &session.id,
                "proposed",
                serde_json::json!({"x": 9}),
            )
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
            .add_part(
                &msg.id,
                &session.id,
                "proposed",
                serde_json::json!({"a": 1}),
            )
            .unwrap();
        let err = store
            .update_part(&p.id, &serde_json::json!([1, 2, 3]))
            .unwrap_err();
        assert!(matches!(err, StoreError::PatchNotAnObject));
        // failed update changed nothing
        let unchanged = store.get_part(&p.id).unwrap().unwrap();
        assert_eq!(unchanged.data, serde_json::json!({"a": 1}));
    }
}
