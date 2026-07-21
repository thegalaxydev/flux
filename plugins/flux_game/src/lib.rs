//! The reactor game as a **Flux plugin**.
//!
//! Everything the reactor game needs beyond the generic engine — a `Building`
//! node, a building catalog, the production/logistics + reactor simulations,
//! their Lua API, rendering, and asset types — lives here and registers itself
//! through the engine's extension seams. Apps (editor, player) call [`install`]
//! once at startup, before any world is created.

pub mod building;
pub mod factory;
pub mod fluids;
pub mod pipes;
pub mod ports;
pub mod reactor;
mod lua;
mod render;

use std::sync::Once;

use glam::Vec2;

use flux_core::{ClassRegistry, Color, Value, prop, prop_t};

static INIT: Once = Once::new();

// The runtime-plugin entry point: version-check, adopt the host's registries,
// then run the same install() the static build uses.
flux_plugin::flux_plugin_main!(install);

/// Install the plugin: register its classes, components, systems, Lua API,
/// rendering and asset types with the engine. Idempotent, so it's safe for the
/// app and each test to call. **Must run before any world is created**, so the
/// `Building` class is present.
pub fn install() {
    INIT.call_once(|| {
        install_classes();

        // Inventories and tanks are plugin components; register their save
        // (de)serializers.
        flux_core::save::register_component(
            "inventory",
            factory::save_inventory,
            factory::load_inventory,
        );
        flux_core::save::register_component("tank", fluids::save_tank, fluids::load_tank);

        // Per-session simulation systems.
        flux_runtime::register_system(|| Box::new(factory::FactorySystem::default()));
        flux_runtime::register_system(|| Box::new(reactor::ReactorSystem::default()));
        flux_runtime::register_system(|| Box::new(pipes::PipeSystem::default()));
        flux_runtime::register_system(|| Box::new(fluids::FluidSystem::default()));

        // Lua API + overlay rendering.
        lua::install();
        flux_view::register_overlay(render::overlay);

        // Asset types + drop-to-create targets.
        flux_render::register_asset_kind(".buildings.json", "buildings");
        flux_render::register_asset_kind(".recipes.json", "recipes");
        flux_render::register_asset_kind(".fluids.json", "fluids");
        flux_render::register_drop("buildings", "Tilemap", "Buildings");
        flux_render::register_drop("recipes", "Tilemap", "Recipes");
        flux_render::register_drop("fluids", "Tilemap", "Fluids");
    });
}

fn install_classes() {
    let mut reg = ClassRegistry::builtins();
    // The Building node: base + sim/reactor state (transient sim accumulators
    // never serialize; reactor scalars do, so a running reactor survives a save).
    reg.add(
        "Building",
        Some("Node2D"),
        true,
        false,
        vec![
            prop("Type", Value::String(String::new())),
            prop("Cell", Value::Vec2(Vec2::ZERO)),
            prop("Footprint", Value::Vec2(Vec2::ONE)),
            // Flow direction for directional buildings: 0=+x, 1=+y, 2=-x, 3=-y.
            prop("Direction", Value::Number(0.0)),
            prop("Color", Value::Color(Color::WHITE)),
            prop("Recipe", Value::String(String::new())),
            prop_t("_Timer", Value::Number(0.0)),
            prop_t("_MineT", Value::Number(0.0)),
            prop_t("_Flow", Value::Number(0.0)),
            // Visible sim status (idle/working/starved; reactors also
            // off/running/hot/meltdown) — drives the child sprite's clip.
            prop_t("_State", Value::String(String::new())),
            prop_t("_StateHold", Value::Number(0.0)),
            // Pipe connectivity mask cache (see flux_game::pipes).
            prop_t("_Mask", Value::Number(-1.0)),
            // Human-readable problem/status line ("Missing coolant", …).
            prop_t("_Status", Value::String(String::new())),
            // Spent-fuel accumulator (reactor waste mechanics).
            prop_t("_WasteAcc", Value::Number(0.0)),
            prop("Temperature", Value::Number(20.0)),
            prop("Fuel", Value::Number(0.0)),
            prop("ControlRods", Value::Number(1.0)),
            prop("Integrity", Value::Number(100.0)),
            prop("PowerOutput", Value::Number(0.0)),
        ],
    );
    flux_core::install(reg);
}

// The game's per-map state lives in ATTRIBUTES on its own tilemap, not in
// class extensions — the engine `Tilemap` class stays clean for every other
// game. Names used: `Buildings`/`Recipes`/`Fluids` (catalog asset paths,
// authored in the scene), `Money` (economy, saved), `PowerProduced`/
// `PowerConsumed` (live balance). The selected building carries the
// `selected` TAG rather than a map-level instance reference.

/// Read a string/asset attribute ("" when unset).
pub(crate) fn attr_text(w: &flux_core::World, id: flux_core::InstanceId, name: &str) -> String {
    match w.attribute(id, name) {
        Some(Value::String(s)) | Some(Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Read a numeric attribute (0.0 when unset).
pub(crate) fn attr_num(w: &flux_core::World, id: flux_core::InstanceId, name: &str) -> f64 {
    match w.attribute(id, name) {
        Some(Value::Number(n)) => *n,
        _ => 0.0,
    }
}

/// Write a numeric attribute.
pub(crate) fn set_attr_num(w: &mut flux_core::World, id: flux_core::InstanceId, name: &str, v: f64) {
    let _ = w.set_attribute(id, name, Some(Value::Number(v)));
}
