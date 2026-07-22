//! Runtime plugin loading: games ship as dynamic libraries the engine
//! binaries know nothing about at compile time.
//!
//! # How it works
//!
//! Every engine extension registry lives behind a share/adopt indirection
//! (see `flux_core::registries`). The **host** (editor/player) builds a
//! [`HostApi`] of pointers to its registries and calls the plugin's exported
//! `flux_plugin_entry`. The **plugin** (via [`flux_plugin_main!`]) first
//! checks the version contract, then adopts the host's registries into its
//! own statically-linked copies of the engine crates, then runs its normal
//! `install()` — every registration lands in the host's single instances.
//!
//! # The contract
//!
//! - **Same toolchain**: the boundary is Rust ABI. [`VERSION`] bakes in the
//!   engine version and the exact rustc; a mismatch refuses to load.
//! - **Load once, never unload**: registries hold plugin function pointers,
//!   closures and vtables for the life of the process. The library handle is
//!   deliberately leaked.
//! - **Load before the first World**: the class registry initializes to
//!   engine builtins on first touch and ignores later installs, so plugins
//!   must be loaded before any scene/world exists. The editor relaunches
//!   itself when a project needs plugins that weren't loaded at startup.
//!
//! # Project manifests
//!
//! A `project.json` beside the scene lists the plugins a project needs:
//! `{ "name": "Reactor", "scene": "main.scene.json", "plugins": ["flux_game"] }`.
//! Names resolve to platform library files under the project's `plugins/` folder
//! (e.g. `project/plugins/flux_game.dll`).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

/// Version contract string: engine version + exact compiler.
pub const VERSION: &str = concat!(
    "flux ",
    env!("CARGO_PKG_VERSION"),
    " / ",
    env!("FLUX_RUSTC_VERSION")
);

/// Pointers to the host's registries, handed to a plugin's entry point.
pub struct HostApi {
    pub version: &'static str,
    pub core: &'static flux_core::registries::CoreRegistries,
    pub script: &'static flux_script::ScriptRegistries,
    pub runtime: &'static flux_runtime::RuntimeRegistries,
    pub view: &'static flux_view::ViewRegistries,
    pub render: &'static flux_render::RenderRegistries,
}

/// Build the host's API bundle (host side).
pub fn host_api() -> HostApi {
    HostApi {
        version: VERSION,
        core: flux_core::registries::share(),
        script: flux_script::share_registries(),
        runtime: flux_runtime::share_registries(),
        view: flux_view::share_registries(),
        render: flux_render::share_registries(),
    }
}

/// Adopt every host registry (plugin side; called by [`flux_plugin_main!`]).
pub fn adopt(api: &HostApi) {
    flux_core::registries::adopt(api.core);
    flux_script::adopt_registries(api.script);
    flux_runtime::adopt_registries(api.runtime);
    flux_view::adopt_registries(api.view);
    flux_render::adopt_registries(api.render);
}

/// The symbol a plugin exports.
pub const ENTRY_SYMBOL: &[u8] = b"flux_plugin_entry";

type PluginEntry = fn(&HostApi) -> Result<(), String>;

/// Declare a library as a Flux plugin: emits the exported entry point that
/// version-checks, adopts the host registries, and runs `$install`.
#[macro_export]
macro_rules! flux_plugin_main {
    ($install:path) => {
        #[unsafe(no_mangle)]
        pub fn flux_plugin_entry(api: &$crate::HostApi) -> Result<(), String> {
            if api.version != $crate::VERSION {
                return Err(format!(
                    "engine/plugin version mismatch: host is '{}', plugin built for '{}'",
                    api.version,
                    $crate::VERSION
                ));
            }
            $crate::adopt(api);
            $install();
            Ok(())
        }
    };
}

// ---------------------------------------------------------------------------
// Host-side loading
// ---------------------------------------------------------------------------

fn loaded_names() -> &'static Mutex<Vec<String>> {
    static LOADED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Names of plugins loaded so far in this process.
pub fn loaded_plugins() -> Vec<String> {
    loaded_names().lock().unwrap().clone()
}

/// Load a plugin library file and run its entry. The library handle is
/// intentionally leaked — plugins are never unloaded (registries hold their
/// code for the life of the process).
pub fn load_library(name: &str, path: &Path) -> Result<(), String> {
    if loaded_plugins().iter().any(|n| n == name) {
        return Ok(());
    }
    unsafe {
        let lib = open_plugin(path)?;
        let entry: libloading::Symbol<PluginEntry> = lib
            .get(ENTRY_SYMBOL)
            .map_err(|e| format!("{}: no flux_plugin_entry export: {e}", path.display()))?;
        entry(&host_api())?;
        std::mem::forget(lib); // never unload
    }
    loaded_names().lock().unwrap().push(name.to_string());
    Ok(())
}

fn open_plugin(path: &Path) -> Result<libloading::Library, String> {
    #[cfg(windows)]
    {
        use libloading::os::windows::{
            Library as WinLib, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        };
        let abs = std::fs::canonicalize(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let win = unsafe {
            WinLib::load_with_flags(
                &abs,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }
        .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(win.into())
    }
    #[cfg(not(windows))]
    {
        unsafe { libloading::Library::new(path) }
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Resolve a manifest plugin name to a library file.
///
/// A project's own `<project>/plugins/<lib>` wins (the dev `build.rs` syncs the
/// freshly built plugin there). Otherwise we fall back to a stock `plugins/`
/// folder beside the running executable — how a **distributed** build ships its
/// plugins, so projects created/opened by end users load them without the
/// dev-only sync. Returns the first existing path, or `None`.
pub fn resolve_plugin(root: &Path, name: &str) -> Option<PathBuf> {
    let lib = libloading::library_filename(name);
    let in_project = root.join("plugins").join(&lib);
    if in_project.is_file() {
        return Some(in_project);
    }
    let bundled = bundled_plugins_dir()?.join(&lib);
    bundled.is_file().then_some(bundled)
}

/// The `plugins/` folder next to the current executable (where a distribution
/// stages its stock plugins). `None` if the executable path can't be resolved.
fn bundled_plugins_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("plugins"))
}

/// A project's `project.json`.
#[derive(Deserialize, Default, Clone)]
pub struct ProjectManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub scene: String,
    #[serde(default)]
    pub plugins: Vec<String>,
}

/// Read `<root>/project.json`, if present.
pub fn manifest(root: &Path) -> Option<ProjectManifest> {
    let text = std::fs::read_to_string(root.join("project.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Outcome of preparing a project's plugins.
pub enum Ensure {
    /// Every required plugin is loaded (or none are required).
    Ready(Vec<String>),
    /// The class registry is already initialized without the missing plugins:
    /// the process must restart to load them (contains the missing names).
    NeedsRestart(Vec<String>),
    Error(String),
}

/// Make sure the project at `root` has its plugins loaded. Call BEFORE
/// creating/loading any world for that project.
pub fn ensure_project(root: &Path) -> Ensure {
    let wanted = manifest(root).map(|m| m.plugins).unwrap_or_default();
    let loaded = loaded_plugins();
    let missing: Vec<String> = wanted.into_iter().filter(|w| !loaded.contains(w)).collect();
    if missing.is_empty() {
        return Ensure::Ready(loaded);
    }
    if flux_core::registries::class_installed() {
        // Too late: worlds already exist against a registry without these
        // classes. Only a fresh process can load them first.
        return Ensure::NeedsRestart(missing);
    }
    for name in &missing {
        let Some(path) = resolve_plugin(root, name) else {
            return Ensure::Error(format!(
                "plugin '{name}' not found (expected in {}/plugins)",
                root.display()
            ));
        };
        if let Err(e) = load_library(name, &path) {
            return Ensure::Error(e);
        }
    }
    Ensure::Ready(loaded_plugins())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_plugin_prefers_project_then_bundled() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("flux_plugin_res_{nanos}"));
        let plugins = root.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();

        // Nothing staged yet: unresolved (bundled dir beside the test exe has no
        // such lib either).
        assert!(resolve_plugin(&root, "definitely_absent_plugin").is_none());

        // A file in the project's own plugins/ resolves to that path.
        let lib = libloading::library_filename("demo");
        let in_project = plugins.join(&lib);
        std::fs::write(&in_project, b"not a real dll").unwrap();
        assert_eq!(resolve_plugin(&root, "demo"), Some(in_project));

        std::fs::remove_dir_all(&root).ok();
    }
}
