use std::cell::RefCell;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::{DataError, PersistenceProvider};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS datastore_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope TEXT NOT NULL,
    store_name TEXT NOT NULL,
    key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (scope, store_name, key)
);
";

const UPSERT: &str = "
INSERT INTO datastore_entries (scope, store_name, key, value_json)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(scope, store_name, key)
DO UPDATE SET value_json = excluded.value_json,
              version = version + 1,
              updated_at = datetime('now')
";

pub struct SqliteProvider {
    conn: RefCell<Connection>,
}

impl SqliteProvider {
    pub fn open_in_memory() -> Result<Self, DataError> {
        Self::init(Connection::open_in_memory()?)
    }

    pub fn open_file(path: &Path) -> Result<Self, DataError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DataError::Db(e.to_string()))?;
        }
        Self::init(Connection::open(path)?)
    }

    fn init(conn: Connection) -> Result<Self, DataError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: RefCell::new(conn),
        })
    }
}

fn to_number(value: f64) -> Result<Value, DataError> {
    if value.fract() == 0.0 && value.is_finite() && value.abs() < 9.007e15 {
        Ok(Value::Number((value as i64).into()))
    } else {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| DataError::Serialization("number is not finite".to_string()))
    }
}

impl PersistenceProvider for SqliteProvider {
    fn get(&self, scope: &str, store: &str, key: &str) -> Result<Option<Value>, DataError> {
        let conn = self.conn.borrow();
        let raw: Option<String> = conn
            .query_row(
                "SELECT value_json FROM datastore_entries WHERE scope=?1 AND store_name=?2 AND key=?3",
                params![scope, store, key],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    fn set(&self, scope: &str, store: &str, key: &str, value: &Value) -> Result<(), DataError> {
        let json = serde_json::to_string(value)?;
        let conn = self.conn.borrow();
        conn.execute(UPSERT, params![scope, store, key, json])?;
        Ok(())
    }

    fn remove(&self, scope: &str, store: &str, key: &str) -> Result<Option<Value>, DataError> {
        let old = self.get(scope, store, key)?;
        let conn = self.conn.borrow();
        conn.execute(
            "DELETE FROM datastore_entries WHERE scope=?1 AND store_name=?2 AND key=?3",
            params![scope, store, key],
        )?;
        Ok(old)
    }

    fn increment(&self, scope: &str, store: &str, key: &str, delta: f64) -> Result<f64, DataError> {
        let mut conn = self.conn.borrow_mut();
        let tx = conn.transaction()?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value_json FROM datastore_entries WHERE scope=?1 AND store_name=?2 AND key=?3",
                params![scope, store, key],
                |row| row.get(0),
            )
            .optional()?;
        let base = match raw {
            Some(json) => {
                let current: Value = serde_json::from_str(&json)?;
                current
                    .as_f64()
                    .ok_or_else(|| DataError::NotANumber(key.to_string()))?
            }
            None => 0.0,
        };
        let next = base + delta;
        let value_json = serde_json::to_string(&to_number(next)?)?;
        tx.execute(UPSERT, params![scope, store, key, value_json])?;
        tx.commit()?;
        Ok(next)
    }

    fn update(
        &self,
        scope: &str,
        store: &str,
        key: &str,
        f: &mut dyn FnMut(Option<Value>) -> Result<Option<Value>, DataError>,
    ) -> Result<Option<Value>, DataError> {
        loop {
            let (current, version): (Option<Value>, i64) = {
                let conn = self.conn.borrow();
                let row: Option<(String, i64)> = conn
                    .query_row(
                        "SELECT value_json, version FROM datastore_entries WHERE scope=?1 AND store_name=?2 AND key=?3",
                        params![scope, store, key],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                match row {
                    Some((json, v)) => (Some(serde_json::from_str(&json)?), v),
                    None => (None, 0),
                }
            };

            let next = f(current)?;

            let mut conn = self.conn.borrow_mut();
            let tx = conn.transaction()?;
            let live: i64 = tx
                .query_row(
                    "SELECT version FROM datastore_entries WHERE scope=?1 AND store_name=?2 AND key=?3",
                    params![scope, store, key],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(0);
            if live != version {
                // Another writer changed the row while the callback ran; retry.
                continue;
            }
            match &next {
                Some(value) => {
                    let json = serde_json::to_string(value)?;
                    tx.execute(UPSERT, params![scope, store, key, json])?;
                }
                None => {
                    if version > 0 {
                        tx.execute(
                            "DELETE FROM datastore_entries WHERE scope=?1 AND store_name=?2 AND key=?3",
                            params![scope, store, key],
                        )?;
                    }
                }
            }
            tx.commit()?;
            return Ok(next);
        }
    }

    fn list_keys(&self, scope: &str, store: &str) -> Result<Vec<String>, DataError> {
        let conn = self.conn.borrow();
        let mut stmt = conn.prepare(
            "SELECT key FROM datastore_entries WHERE scope=?1 AND store_name=?2 ORDER BY key",
        )?;
        let rows = stmt.query_map(params![scope, store], |row| row.get::<_, String>(0))?;
        let mut keys = Vec::new();
        for key in rows {
            keys.push(key?);
        }
        Ok(keys)
    }
}
