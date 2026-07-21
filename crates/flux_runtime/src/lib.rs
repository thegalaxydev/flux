use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::RwLock;

use flux_core::World;
use flux_data::PersistenceProvider;
use flux_script::ScriptHost;

pub use flux_data::{DataBackend, DataError};
pub use flux_script::{InputFrame, LogEntry, LogLevel};

/// A per-session simulation system a plugin adds to the runtime (e.g. a game's
/// factory or reactor sim). Stepped every frame after scripts have run.
pub trait System {
    fn step(&mut self, world: &mut World, root: &Path, dt: f32);
}

/// flux_runtime's registry state — adoptable across a plugin DLL boundary (see
/// `flux_core::registries` for the pattern rationale).
pub struct RuntimeRegistries {
    systems: RwLock<Vec<fn() -> Box<dyn System>>>,
}

static SHARED: std::sync::OnceLock<&'static RuntimeRegistries> = std::sync::OnceLock::new();

fn regs() -> &'static RuntimeRegistries {
    SHARED.get_or_init(|| Box::leak(Box::new(RuntimeRegistries { systems: RwLock::new(Vec::new()) })))
}

/// The process-wide registries — the host passes this to loaded plugins.
pub fn share_registries() -> &'static RuntimeRegistries {
    regs()
}

/// Adopt the host's registries (plugin entry point, before any registration).
pub fn adopt_registries(shared: &'static RuntimeRegistries) {
    let _ = SHARED.set(shared);
}

/// Register a system constructor. Each [`Session`] builds one instance (so a
/// system may own per-session caches) and steps it every frame.
pub fn register_system(ctor: fn() -> Box<dyn System>) {
    regs().systems.write().unwrap().push(ctor);
}

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
    systems: Vec<Box<dyn System>>,
    root: PathBuf,
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
        let systems = regs().systems.read().unwrap().iter().map(|ctor| ctor()).collect();
        Ok(Session {
            host,
            systems,
            root: script_root.to_path_buf(),
        })
    }

    /// A scene switch requested from Luau via `Scene:Load`/`Scene:Reload`, if any.
    /// The caller (editor/player) reloads the new scene into a fresh session.
    pub fn take_scene_request(&self) -> Option<String> {
        self.host.take_scene_request()
    }

    pub fn step(&mut self, dt: f64, input: &InputFrame) {
        self.host.step(dt, input);
        // Plugin systems run after scripts + engine sync for the frame.
        if !self.systems.is_empty() {
            let world = self.host.world();
            let mut w = world.borrow_mut();
            for system in &mut self.systems {
                system.step(&mut w, &self.root, dt as f32);
            }
        }
    }

    pub fn world(&self) -> Rc<RefCell<World>> {
        self.host.world()
    }

    pub fn drain_logs(&self) -> Vec<LogEntry> {
        self.host.drain_logs()
    }
}
