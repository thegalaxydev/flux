//! Velopack auto-update integration.
//!
//! Velopack ships Flux as a per-user, self-updating install (see
//! `packaging/velopack.md`). Two touch points:
//!
//! * [`run_hooks`] — the very first thing `main` does. When the exe is invoked
//!   as the installer/updater/uninstaller, Velopack handles it here and may
//!   exit or restart the process. In a normal (or `cargo run`) launch it's a
//!   cheap no-op.
//! * [`Updater`] — a background "is there a newer release?" check against the
//!   GitHub releases feed. On success it downloads the update and flips to
//!   [`UpdateState::Downloaded`]; the UI then offers a restart that applies it.
//!
//! The release feed is the package's `repository` URL (`CARGO_PKG_REPOSITORY`).
//! Until that's set in `crates/flux_editor/Cargo.toml`, the updater is inert, so
//! dev builds never phone home.

use std::sync::{Arc, Mutex};

use velopack::VelopackApp;
use velopack::sources::GithubSource;

/// The GitHub repo hosting the release feed, from the crate's `repository`
/// field. `None` (unset) disables the updater entirely.
const REPO_URL: Option<&str> = option_env!("CARGO_PKG_REPOSITORY");

/// Run Velopack's install/update/uninstall hooks. Must be the first line of
/// `main` — it can terminate or restart the process.
pub fn run_hooks() {
    VelopackApp::build().run();
}

#[derive(Clone, PartialEq, Eq)]
pub enum UpdateState {
    /// No check running and nothing to apply.
    Idle,
    /// A background check/download is in flight.
    Checking,
    /// Already on the latest release.
    UpToDate,
    /// A newer release was downloaded and is ready to apply on restart.
    Downloaded,
    /// The check failed (offline, not a Velopack install, …). Kept quiet in UI.
    Failed(String),
}

/// Shared, cheaply-clonable handle to the background update check's state.
#[derive(Clone)]
pub struct Updater {
    state: Arc<Mutex<UpdateState>>,
}

impl Default for Updater {
    fn default() -> Self {
        Self { state: Arc::new(Mutex::new(UpdateState::Idle)) }
    }
}

impl Updater {
    pub fn state(&self) -> UpdateState {
        self.state.lock().unwrap().clone()
    }

    fn set(&self, s: UpdateState) {
        *self.state.lock().unwrap() = s;
    }

    /// Whether auto-update is wired up (a `repository` URL is configured).
    pub fn enabled() -> bool {
        matches!(REPO_URL, Some(u) if u.contains("github.com"))
    }

    /// Kick off a one-shot background check + download. No-op if disabled or a
    /// check is already running. Safe to call once at startup.
    pub fn spawn_check(&self) {
        if !Self::enabled() || self.state() == UpdateState::Checking {
            return;
        }
        self.set(UpdateState::Checking);
        let state = self.state.clone();
        std::thread::spawn(move || {
            let next = match check_and_download() {
                Ok(true) => UpdateState::Downloaded,
                Ok(false) => UpdateState::UpToDate,
                Err(e) => UpdateState::Failed(e),
            };
            *state.lock().unwrap() = next;
        });
    }

    /// Apply the downloaded update and restart into it. On success the process
    /// does not return; on failure the error is recorded and we carry on.
    pub fn apply_and_restart(&self) {
        if let Err(e) = apply() {
            self.set(UpdateState::Failed(e));
        }
    }
}

fn manager() -> Result<velopack::UpdateManager, String> {
    let url = REPO_URL.ok_or("no release feed configured")?;
    // `prerelease = false`: only stable tagged releases update users.
    let source = GithubSource::new(url, None, false);
    velopack::UpdateManager::new(source, None, None).map_err(|e| e.to_string())
}

/// Check the feed and, if a newer release exists, download it. `Ok(true)` when
/// an update was downloaded, `Ok(false)` when already current.
fn check_and_download() -> Result<bool, String> {
    let um = manager()?;
    match um.check_for_updates().map_err(|e| e.to_string())? {
        velopack::UpdateCheck::UpdateAvailable(updates) => {
            um.download_updates(&updates, None).map_err(|e| e.to_string())?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Re-resolve the already-downloaded update and apply it (restarts the process).
fn apply() -> Result<(), String> {
    let um = manager()?;
    if let velopack::UpdateCheck::UpdateAvailable(updates) =
        um.check_for_updates().map_err(|e| e.to_string())?
    {
        um.apply_updates_and_restart(&*updates).map_err(|e| e.to_string())?;
    }
    Ok(())
}
