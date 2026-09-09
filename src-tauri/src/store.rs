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
}
