//! Process-shared registry storage, adoptable across a plugin DLL boundary.
//!
//! Registries used to be plain `static`s. A runtime-loaded plugin statically
//! links its own copy of this crate, so those statics would be **duplicated**:
//! the plugin's `install()` would fill its copy while the host reads its own,
//! empty one. Instead, all registry state lives in one leaked
//! [`CoreRegistries`] reached through an indirection cell:
//!
//! - the **host** calls [`share`] and hands the pointer to plugins,
//! - a **plugin** calls [`adopt`] (before anything touches a registry in its
//!   copy of the crate) so both sides operate on the host's single instance,
//! - static builds/tests never call either and lazily get a private default —
//!   exactly the old behaviour.

use std::sync::{OnceLock, RwLock};

use crate::class::ClassRegistry;
use crate::save::ComponentSerde;

/// All of flux_core's registry state: the class registry plus the plugin
/// component (de)serializers.
pub struct CoreRegistries {
    pub(crate) class: OnceLock<ClassRegistry>,
    pub(crate) components: RwLock<Vec<ComponentSerde>>,
}

impl CoreRegistries {
    fn new() -> Self {
        CoreRegistries {
            class: OnceLock::new(),
            components: RwLock::new(Vec::new()),
        }
    }
}

static SHARED: OnceLock<&'static CoreRegistries> = OnceLock::new();

pub(crate) fn regs() -> &'static CoreRegistries {
    SHARED.get_or_init(|| Box::leak(Box::new(CoreRegistries::new())))
}

/// The process-wide registries — the host passes this to loaded plugins.
pub fn share() -> &'static CoreRegistries {
    regs()
}

/// Adopt the host's registries (called from a plugin's entry point, before
/// any world/class access inside the plugin). No-op if already initialized.
pub fn adopt(shared: &'static CoreRegistries) {
    let _ = SHARED.set(shared);
}
