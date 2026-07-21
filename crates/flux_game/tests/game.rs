//! End-to-end tests for the reactor-game plugin driven through a runtime
//! `Session`: the tile Lua API, building placement + camera conversion, the
//! factory chain, and save/load — all exercised with `flux_game` installed.

use std::path::Path;
use std::sync::Once;

use flux_core::{Value, World};
use flux_game::factory::Inventory;
use flux_runtime::{InputFrame, LogLevel, Session};

static INSTALLED: Once = Once::new();

/// Install the plugin before the first world is created.
fn setup() -> &'static Path {
    INSTALLED.call_once(flux_game::install);
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
}

fn info_logs(session: &Session) -> Vec<String> {
    session
        .drain_logs()
        .into_iter()
        .filter(|l| l.level == LogLevel::Info)
        .map(|l| l.message)
        .collect()
}

/// A `Tilemap` named "Map" with a TileSet (and optionally Buildings/Recipes)
/// driven by `script_path`.
fn scene(script_path: &str, with_catalogs: bool) -> String {
    let mut w = World::new();
    let map = w.create("Tilemap", w.workspace()).unwrap();
    w.set_name(map, "Map").unwrap();
    w.set_prop(map, "TileSet", Value::Asset("test.tileset.json".into()))
        .unwrap();
    if with_catalogs {
        w.set_prop(map, "Buildings", Value::Asset("test.buildings.json".into()))
            .unwrap();
        w.set_prop(map, "Recipes", Value::Asset("test.recipes.json".into()))
            .unwrap();
    }
    let size = if with_catalogs { 16.0 } else { 8.0 };
    w.set_prop(map, "MapWidth", Value::Number(size)).unwrap();
    w.set_prop(map, "MapHeight", Value::Number(size)).unwrap();

    let script = w.create("Script", map).unwrap();
    w.set_prop(script, "SourcePath", Value::Asset(script_path.into()))
        .unwrap();
    w.to_json()
}

#[test]
fn lua_tile_api_reads_writes_and_mines() {
    let root = setup();
    let session = Session::from_scene_json(&scene("scripts/test_tilemap.luau", false), root).unwrap();
    let logs = info_logs(&session);
    let has = |s: &str| logs.iter().any(|m| m == s);
    assert!(has("size 8x8"), "GetMapSize: {logs:?}");
    assert!(has("tile grass"), "SetTile/GetTile: {logs:?}");
    assert!(has("ore coal 100"), "SetOre/GetOre: {logs:?}");
    assert!(has("mined 30"), "MineOre: {logs:?}");
    assert!(has("after nil"), "deposit clears: {logs:?}");
    assert!(has("roundtrip 2,3"), "TileToWorld/WorldToTile: {logs:?}");
}

#[test]
fn lua_building_placement_and_camera_conversion() {
    let root = setup();
    let session = Session::from_scene_json(&scene("scripts/test_buildings.luau", true), root).unwrap();
    let logs = info_logs(&session);
    let has = |s: &str| logs.iter().any(|m| m == s);
    assert!(has("canplace true"), "CanPlace: {logs:?}");
    assert!(has("placed true"), "PlaceBuilding: {logs:?}");
    assert!(has("class Building"), "new node class: {logs:?}");
    assert!(has("type smelter"), "Type prop: {logs:?}");
    assert!(has("at33 true"), "GetBuildingAt: {logs:?}");
    assert!(has("blocked true"), "overlap refused: {logs:?}");
    assert!(has("removed true"), "RemoveBuilding: {logs:?}");
    assert!(has("gone true"), "cell freed: {logs:?}");
    assert!(has("cam true"), "ScreenToWorld/WorldToScreen: {logs:?}");
    assert!(has("beltdir true"), "PlaceBuilding direction: {logs:?}");
    assert!(has("ghost true"), "SetGhost shows: {logs:?}");
    assert!(has("ghostfree true"), "ghost isn't a building: {logs:?}");
    assert!(has("ghostgone true"), "SetGhost(nil) clears: {logs:?}");
    assert!(has("sel true"), "_Selected accepts an instance: {logs:?}");
    assert!(has("selnil true"), "_Selected accepts nil: {logs:?}");
}

#[test]
fn factory_mines_produces_and_transports_a_chain() {
    let root = setup();
    let mut session =
        Session::from_scene_json(&scene("scripts/test_factory.luau", true), root).unwrap();
    for _ in 0..60 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    let map = w.find_first_child(w.workspace(), "Map").unwrap();

    let storage = w
        .descendants(map)
        .into_iter()
        .find(|&id| {
            w.class_name(id) == Some("Building")
                && matches!(w.get_prop(id, "Type"), Some(Value::String(s)) if s == "storage")
        })
        .unwrap();

    assert!(
        w.tile_grid(map).unwrap().cell(1, 1).unwrap().ore_amount < 1000,
        "miner did not consume the deposit"
    );
    let plates = w.component::<Inventory>(storage).map(|i| i.count("plate")).unwrap_or(0);
    assert!(plates > 0, "no plates reached storage");
}

/// Session-driven world for direct (non-Lua) transport tests.
fn session_world(root: &Path) -> Session {
    Session::from_scene_json(&scene("scripts/noop.luau", true), root).unwrap()
}

#[test]
fn conveyor_respects_ports() {
    let root = setup();
    let mut session = session_world(root);
    {
        let world = session.world();
        let mut w = world.borrow_mut();
        let map = w.find_first_child(w.workspace(), "Map").unwrap();
        let cat = flux_game::building::BuildingCatalog::parse(
            &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
        )
        .unwrap();

        // Boiler's only port is a LIQUID input on its north cell (4,4): a belt
        // pointing there must never deliver items.
        flux_game::building::place(&mut w, map, cat.get("boiler").unwrap(), 4, 4, 0).unwrap();
        let feed = flux_game::building::place(&mut w, map, cat.get("belt").unwrap(), 4, 3, 1).unwrap();
        if let Some(inv) = w.component_mut::<Inventory>(feed) {
            inv.add("coal", 3);
        }

        // Hopper's single item input has limit 1: with two belts pointing at
        // it, only the first (deterministic by cell order) may deliver.
        let hopper = flux_game::building::place(&mut w, map, cat.get("hopper").unwrap(), 9, 9, 0).unwrap();
        let north = flux_game::building::place(&mut w, map, cat.get("belt").unwrap(), 9, 8, 1).unwrap();
        let west = flux_game::building::place(&mut w, map, cat.get("belt").unwrap(), 8, 9, 0).unwrap();
        if let Some(inv) = w.component_mut::<Inventory>(north) {
            inv.add("from_north", 2);
        }
        if let Some(inv) = w.component_mut::<Inventory>(west) {
            inv.add("from_west", 2);
        }
        let _ = (hopper, north, west);
    }
    for _ in 0..40 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    let map = w.find_first_child(w.workspace(), "Map").unwrap();
    let by_type = |ty: &str| {
        w.descendants(map)
            .into_iter()
            .find(|&id| {
                w.class_name(id) == Some("Building")
                    && matches!(w.get_prop(id, "Type"), Some(Value::String(s)) if s == ty)
            })
            .unwrap()
    };
    // Liquid port rejected the conveyor entirely.
    let boiler = by_type("boiler");
    assert!(
        w.component::<Inventory>(boiler).is_none_or(|i| i.total() == 0),
        "items crossed a liquid port"
    );
    // Limit 1: exactly one belt's cargo arrived — the lower-cell one (west).
    let hopper = by_type("hopper");
    let inv = w.component::<Inventory>(hopper).unwrap();
    assert_eq!(inv.count("from_west"), 2, "first feeder should deliver");
    assert_eq!(inv.count("from_north"), 0, "second feeder must be blocked by limit");
}

#[test]
fn map_held_inventory_round_trips_through_save() {
    // The shop/inventory game stores the player's building stock in an Inventory
    // on the Tilemap itself (not a Building). Prove that generic inventory
    // survives a save-string round trip via the registered component hook.
    setup();
    let mut w = World::new();
    let map = w.create("Tilemap", w.workspace()).unwrap();
    w.set_component::<Inventory>(map, Inventory::from_pairs(0, [("miner".into(), 3), ("smelter".into(), 1)]));

    let saved = w.to_save_string();
    let reloaded = World::from_json(&saved).unwrap();
    let map2 = reloaded
        .descendants(reloaded.workspace())
        .into_iter()
        .find(|&id| reloaded.class_name(id) == Some("Tilemap"))
        .expect("tilemap restored");
    let inv = reloaded.component::<Inventory>(map2).expect("map inventory restored");
    assert_eq!(inv.count("miner"), 3);
    assert_eq!(inv.count("smelter"), 1);
}

#[test]
fn lua_save_service_persists_and_reloads_world() {
    let root = setup();
    let saves = root.join(".flux/saves");
    let _ = std::fs::remove_dir_all(&saves);

    let session = Session::from_scene_json(&scene("scripts/test_save.luau", true), root).unwrap();
    let logs = info_logs(&session);
    assert!(logs.iter().any(|m| m == "exists true"), "Save/Exists: {logs:?}");
    assert!(logs.iter().any(|m| m == "listed true"), "List: {logs:?}");
    assert_eq!(
        session.take_scene_request().as_deref(),
        Some(".flux/saves/slot1.save.json")
    );

    // Reload the saved world and confirm the persisted state.
    let saved = std::fs::read_to_string(saves.join("slot1.save.json")).unwrap();
    let reloaded = Session::from_scene_json(&saved, root).unwrap();
    let world = reloaded.world();
    let w = world.borrow();
    let map = w.find_first_child(w.workspace(), "Map").unwrap();
    assert_eq!(w.tile_grid(map).unwrap().get(0, 0), Some(2)); // "stone"
    let building = w
        .descendants(map)
        .into_iter()
        .find(|&id| w.class_name(id) == Some("Building"))
        .expect("placed building persisted");
    // Its inventory component came back too.
    assert!(w.component::<Inventory>(building).is_some(), "inventory restored");

    let _ = std::fs::remove_dir_all(&saves);
}
