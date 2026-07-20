use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use flux_core::World;
use flux_data::PersistenceProvider;
use flux_script::ScriptHost;

pub use flux_data::{DataBackend, DataError};
pub use flux_script::{InputFrame, LogEntry, LogLevel};

/// How a play session is configured: the persistence backend
/// `DataStoreService` writes to, and the current scene's project-relative path
/// (so `Scene`/`Scene.Name`/`Scene:Reload` can report and re-load it).
pub struct SessionOptions {
    pub data: DataBackend,
    pub scene: String,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            data: DataBackend::SqliteMemory,
            scene: String::new(),
        }
    }
}

pub struct Session {
    host: ScriptHost,
}

impl Session {
    /// Launch with the default options (temporary in-memory persistence).
    pub fn from_scene_json(json: &str, script_root: &Path) -> Result<Session, String> {
        Self::launch(json, script_root, SessionOptions::default())
    }

    pub fn launch(
        json: &str,
        script_root: &Path,
        options: SessionOptions,
    ) -> Result<Session, String> {
        let world = World::from_json(json).map_err(|e| e.to_string())?;

        // Persistence must never block playtesting: fall back to in-memory and
        // report the problem to the output console.
        let (provider, warning) = match flux_data::open(&options.data) {
            Ok(p) => (p, None),
            Err(e) => {
                let fallback = flux_data::open(&DataBackend::SqliteMemory)
                    .map_err(|e| e.to_string())?;
                (
                    fallback,
                    Some(format!(
                        "Persistence unavailable ({e}); using temporary in-memory data."
                    )),
                )
            }
        };
        let provider: Rc<dyn PersistenceProvider> = Rc::from(provider);

        let host = ScriptHost::new(world, script_root, provider, options.scene)?;
        if let Some(warning) = warning {
            host.push_error(warning);
        }
        Ok(Session { host })
    }

    /// A scene switch requested from Luau via `Scene:Load`/`Scene:Reload`, if any.
    /// The caller (editor/player) reloads the new scene into a fresh session.
    pub fn take_scene_request(&self) -> Option<String> {
        self.host.take_scene_request()
    }

    pub fn step(&mut self, dt: f64, input: &InputFrame) {
        self.host.step(dt, input);
    }

    pub fn world(&self) -> Rc<RefCell<World>> {
        self.host.world()
    }

    pub fn drain_logs(&self) -> Vec<LogEntry> {
        self.host.drain_logs()
    }
}
