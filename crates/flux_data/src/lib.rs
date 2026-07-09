mod sqlite;

use std::path::PathBuf;

use serde_json::Value;

pub use sqlite::SqliteProvider;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("database error: {0}")]
    Db(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("value for key '{0}' is not a number")]
    NotANumber(String),
    #[error("{0}")]
    Callback(String),
}

impl From<rusqlite::Error> for DataError {
    fn from(e: rusqlite::Error) -> Self {
        DataError::Db(e.to_string())
    }
}

impl From<serde_json::Error> for DataError {
    fn from(e: serde_json::Error) -> Self {
        DataError::Serialization(e.to_string())
    }
}

/// A persistence backend. Game scripts never see this — they only touch
/// `DataStoreService`, which routes here through the runtime.
pub trait PersistenceProvider {
    fn get(&self, scope: &str, store: &str, key: &str) -> Result<Option<Value>, DataError>;
    fn set(&self, scope: &str, store: &str, key: &str, value: &Value) -> Result<(), DataError>;
    fn remove(&self, scope: &str, store: &str, key: &str) -> Result<Option<Value>, DataError>;
    fn increment(&self, scope: &str, store: &str, key: &str, delta: f64) -> Result<f64, DataError>;

    /// Read the current value, hand it to `f`, and atomically write whatever `f`
    /// returns (removing the key if `f` returns `None`), bumping the version.
    fn update(
        &self,
        scope: &str,
        store: &str,
        key: &str,
        f: &mut dyn FnMut(Option<Value>) -> Result<Option<Value>, DataError>,
    ) -> Result<Option<Value>, DataError>;

    fn list_keys(&self, scope: &str, store: &str) -> Result<Vec<String>, DataError>;
}

/// Which database backs persistence. SQLite now; Postgres slots in here later
/// without touching any script-facing code.
#[derive(Clone, Debug)]
pub enum DataBackend {
    /// Throwaway in-memory database — the default for editor playtesting.
    SqliteMemory,
    /// A project-local SQLite file — standalone builds and persistent playtests.
    SqliteFile(PathBuf),
    // Postgres { url: String }, // future — see docs/datastore_service.md
}

pub fn open(backend: &DataBackend) -> Result<Box<dyn PersistenceProvider>, DataError> {
    match backend {
        DataBackend::SqliteMemory => Ok(Box::new(SqliteProvider::open_in_memory()?)),
        DataBackend::SqliteFile(path) => Ok(Box::new(SqliteProvider::open_file(path)?)),
    }
}
