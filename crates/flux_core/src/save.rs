//! Save/load extension point for plugin components.
//!
//! The engine serializes the scene tree and (for save games) tilemap grids, but
//! it can't serialize the opaque plugin data in [`World`]'s component store (see
//! [`World::component`]). A plugin registers a name plus a
//! `serde_json::Value` (de)serializer per component type here; the save system
//! calls them generically, keeping `flux_core` ignorant of game types.

use indexmap::IndexMap;
use serde_json::Value as Json;

use crate::world::{InstanceId, World};

pub(crate) struct ComponentSerde {
    name: &'static str,
    serialize: fn(&World, InstanceId) -> Option<Json>,
    deserialize: fn(&mut World, InstanceId, &Json),
}

/// Register save/load for a plugin component. `serialize` returns `None` for
/// instances without the component; `deserialize` reattaches it. Called once per
/// component type at plugin install. (Storage lives in [`crate::registries`] so
/// runtime-loaded plugins register into the host's list.)
pub fn register_component(
    name: &'static str,
    serialize: fn(&World, InstanceId) -> Option<Json>,
    deserialize: fn(&mut World, InstanceId, &Json),
) {
    crate::registries::regs().components.write().unwrap().push(ComponentSerde {
        name,
        serialize,
        deserialize,
    });
}

/// Serialize every registered component present on `id` (save games only).
pub(crate) fn save_components(world: &World, id: InstanceId) -> IndexMap<String, Json> {
    let mut out = IndexMap::new();
    for c in crate::registries::regs().components.read().unwrap().iter() {
        if let Some(v) = (c.serialize)(world, id) {
            out.insert(c.name.to_string(), v);
        }
    }
    out
}

/// Reattach saved components to `id` using the registered deserializers.
pub(crate) fn load_components(world: &mut World, id: InstanceId, comps: &IndexMap<String, Json>) {
    let regs = crate::registries::regs().components.read().unwrap();
    for (name, value) in comps {
        if let Some(c) = regs.iter().find(|c| c.name == *name) {
            (c.deserialize)(world, id, value);
        }
    }
}
