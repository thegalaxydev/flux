//! The reactor game as a **Flux plugin**.
//!
//! Everything the reactor game needs beyond the generic engine — a `Building`
//! node, a building catalog, the production/logistics + reactor simulations,
//! their Lua API, rendering, and asset types — lives here and registers itself
//! through the engine's extension seams. Apps (editor, player) call [`install`]
//! once at startup, before any world is created.

pub mod building;
pub mod factory;
pub mod reactor;
mod lua;
mod render;

use std::sync::Once;

use glam::Vec2;

use flux_core::{AssetType, ClassRegistry, Color, Value, asset_prop, prop, prop_t};

static INIT: Once = Once::new();

/// Install the plugin: register its classes, components, systems, Lua API,
/// rendering and asset types with the engine. Idempotent, so it's safe for the
/// app and each test to call. **Must run before any world is created**, so the
/// `Building` class is present.
pub fn install() {
    INIT.call_once(|| {
        install_classes();

        // Inventories are a plugin component; register their save (de)serializer.
        flux_core::save::register_component(
            "inventory",
            factory::save_inventory,
            factory::load_inventory,
        );

        // Per-session simulation systems.
        flux_runtime::register_system(|| Box::new(factory::FactorySystem::default()));
        flux_runtime::register_system(|| Box::new(reactor::ReactorSystem::default()));

        // Lua API + overlay rendering.
        lua::install();
        flux_view::register_overlay(render::overlay);

        // Asset types + drop-to-create targets.
        flux_render::register_asset_kind(".buildings.json", "buildings");
        flux_render::register_asset_kind(".recipes.json", "recipes");
        flux_render::register_drop("buildings", "Tilemap", "Buildings");
        flux_render::register_drop("recipes", "Tilemap", "Recipes");
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
            prop("Color", Value::Color(Color::WHITE)),
            prop("Recipe", Value::String(String::new())),
            prop_t("_Timer", Value::Number(0.0)),
            prop_t("_MineT", Value::Number(0.0)),
            prop_t("_Flow", Value::Number(0.0)),
            prop("Temperature", Value::Number(20.0)),
            prop("Fuel", Value::Number(0.0)),
            prop("ControlRods", Value::Number(1.0)),
            prop("Integrity", Value::Number(100.0)),
            prop("PowerOutput", Value::Number(0.0)),
        ],
    );
    // Extend the engine's Tilemap with the game's catalog refs + power balance.
    reg.extend(
        "Tilemap",
        vec![
            asset_prop("Buildings", AssetType::Custom("buildings")),
            asset_prop("Recipes", AssetType::Custom("recipes")),
            prop_t("_PowerProduced", Value::Number(0.0)),
            prop_t("_PowerConsumed", Value::Number(0.0)),
        ],
    );
    flux_core::install(reg);
}
