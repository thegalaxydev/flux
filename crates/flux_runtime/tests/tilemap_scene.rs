//! End-to-end check that a scene containing a `Tilemap` loads through a runtime
//! session and its grid is generated on launch, the way the editor/player rely
//! on. Self-contained: the scene is inline and colour tiles need no assets.

use std::path::Path;

use flux_core::{Value, World};
use flux_runtime::{InputFrame, LogLevel, Session};

fn fixtures() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
}

const SCENE: &str = r#"{
  "version": 1,
  "root": {
    "class": "Game",
    "name": "Game",
    "children": [
      {
        "class": "Workspace",
        "name": "Workspace",
        "children": [
          {
            "class": "Tilemap",
            "name": "World",
            "props": {
              "MapWidth":  { "t": "Number", "v": 32.0 },
              "MapHeight": { "t": "Number", "v": 24.0 },
              "Seed":      { "t": "Number", "v": 1337.0 }
            }
          }
        ]
      },
      { "class": "Storage", "name": "Storage" },
      { "class": "Gui", "name": "Gui" },
      { "class": "Scripts", "name": "Scripts" }
    ]
  }
}"#;

#[test]
fn tilemap_scene_generates_a_grid_on_launch() {
    // Launching a session runs `tilemap::sync`, so the grid exists immediately.
    let session = Session::from_scene_json(SCENE, std::path::Path::new(".")).expect("scene loads");
    let world = session.world();
    let w = world.borrow();

    let tilemap = w
        .descendants(w.workspace())
        .into_iter()
        .find(|&id| w.class_name(id) == Some("Tilemap"))
        .expect("scene has a Tilemap");

    let grid = w.tile_grid(tilemap).expect("grid generated on launch");
    assert_eq!(grid.width(), 32);
    assert_eq!(grid.height(), 24);
    for row in 0..grid.height() as i32 {
        for col in 0..grid.width() as i32 {
            assert!(grid.get(col, row).is_some());
        }
    }
}

/// Build a scene with an 8x8 `Tilemap` named "Map" (colour tileset, no worldgen
/// so the placeholder generator fills it) driven by `script_path`.
fn tilemap_scene(script_path: &str) -> String {
    let mut w = World::new();
    let ws = w.workspace();
    let map = w.create("Tilemap", ws).unwrap();
    w.set_name(map, "Map").unwrap();
    w.set_prop(map, "TileSet", Value::Asset("test.tileset.json".into()))
        .unwrap();
    w.set_prop(map, "MapWidth", Value::Number(8.0)).unwrap();
    w.set_prop(map, "MapHeight", Value::Number(8.0)).unwrap();

    let script = w.create("Script", map).unwrap();
    w.set_prop(script, "SourcePath", Value::Asset(script_path.into()))
        .unwrap();
    w.to_json()
}

#[test]
fn lua_tile_api_reads_writes_and_mines() {
    let json = tilemap_scene("scripts/test_tilemap.luau");
    let session = Session::from_scene_json(&json, fixtures()).expect("scene loads");
    let logs: Vec<String> = session
        .drain_logs()
        .into_iter()
        .filter(|l| l.level == LogLevel::Info)
        .map(|l| l.message)
        .collect();

    let has = |s: &str| logs.iter().any(|m| m == s);
    assert!(has("size 8x8"), "GetMapSize: {logs:?}");
    assert!(has("tile grass"), "SetTile/GetTile: {logs:?}");
    assert!(has("ore coal 100"), "SetOre/GetOre: {logs:?}");
    assert!(has("mined 30"), "MineOre: {logs:?}");
    assert!(has("left 70"), "partial mine: {logs:?}");
    assert!(has("after nil"), "deposit clears when depleted: {logs:?}");
    assert!(has("roundtrip 2,3"), "TileToWorld/WorldToTile: {logs:?}");

    // The mutation is also visible on the world grid directly.
    let world = session.world();
    let w = world.borrow();
    let map = w.find_first_child(w.workspace(), "Map").unwrap();
    let cell = w.tile_grid(map).unwrap().cell(1, 1).unwrap();
    assert!(!cell.has_ore(), "ore should be fully mined out");
}

/// Build a scene with a `Tilemap` named "Map" carrying a Buildings catalog,
/// driven by `script_path`.
fn building_scene(script_path: &str) -> String {
    let mut w = World::new();
    let ws = w.workspace();
    let map = w.create("Tilemap", ws).unwrap();
    w.set_name(map, "Map").unwrap();
    w.set_prop(map, "TileSet", Value::Asset("test.tileset.json".into()))
        .unwrap();
    w.set_prop(map, "Buildings", Value::Asset("test.buildings.json".into()))
        .unwrap();
    w.set_prop(map, "Recipes", Value::Asset("test.recipes.json".into()))
        .unwrap();
    w.set_prop(map, "MapWidth", Value::Number(16.0)).unwrap();
    w.set_prop(map, "MapHeight", Value::Number(16.0)).unwrap();

    let script = w.create("Script", map).unwrap();
    w.set_prop(script, "SourcePath", Value::Asset(script_path.into()))
        .unwrap();
    w.to_json()
}

#[test]
fn factory_mines_produces_and_transports_a_chain() {
    let json = building_scene("scripts/test_factory.luau");
    let mut session = Session::from_scene_json(&json, fixtures()).expect("scene loads");
    // Run ~6 seconds of simulation.
    for _ in 0..60 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    let map = w.find_first_child(w.workspace(), "Map").unwrap();

    let by_type = |ty: &str| {
        w.descendants(map)
            .into_iter()
            .find(|&id| w.class_name(id) == Some("Building") && matches!(w.get_prop(id, "Type"), Some(Value::String(s)) if s == ty))
            .unwrap()
    };
    let miner = by_type("miner");
    let storage = by_type("storage");

    // The miner drew coal out of the deposit (started at 1000).
    assert!(
        w.tile_grid(map).unwrap().cell(1, 1).unwrap().ore_amount < 1000,
        "miner did not consume the deposit"
    );
    let _ = miner;
    // Plates crafted from that coal reached the storage sink downstream.
    let plates = w.inventory(storage).map(|i| i.count("plate")).unwrap_or(0);
    assert!(plates > 0, "no plates reached storage: {:?}", w.inventory(storage).map(|i| i.iter().map(|(k,v)|(k.to_string(),v)).collect::<Vec<_>>()));
}

#[test]
fn lua_save_service_persists_and_reloads_world() {
    let root = fixtures();
    let saves = root.join(".flux/saves");
    let _ = std::fs::remove_dir_all(&saves); // deterministic slate

    let json = building_scene("scripts/test_save.luau");
    let session = Session::from_scene_json(&json, root).expect("scene loads");
    let logs: Vec<String> = session
        .drain_logs()
        .into_iter()
        .filter(|l| l.level == LogLevel::Info)
        .map(|l| l.message)
        .collect();
    assert!(logs.iter().any(|m| m == "exists true"), "Save/Exists: {logs:?}");
    assert!(logs.iter().any(|m| m == "listed true"), "List: {logs:?}");

    // SaveService:Load requested a swap to the slot file (like Scene:Load).
    assert_eq!(
        session.take_scene_request().as_deref(),
        Some(".flux/saves/slot1.save.json")
    );

    // Reload the saved world independently and confirm the persisted state.
    let saved = std::fs::read_to_string(saves.join("slot1.save.json")).unwrap();
    let reloaded = Session::from_scene_json(&saved, root).expect("save loads");
    let world = reloaded.world();
    let w = world.borrow();
    let map = w.find_first_child(w.workspace(), "Map").unwrap();
    assert_eq!(w.tile_grid(map).unwrap().get(0, 0), Some(2)); // "stone"
    let building = w
        .descendants(map)
        .into_iter()
        .find(|&id| w.class_name(id) == Some("Building"));
    assert!(building.is_some(), "placed building persisted");

    let _ = std::fs::remove_dir_all(&saves);
}

#[test]
fn lua_building_placement_and_camera_conversion() {
    let json = building_scene("scripts/test_buildings.luau");
    let session = Session::from_scene_json(&json, fixtures()).expect("scene loads");
    let logs: Vec<String> = session
        .drain_logs()
        .into_iter()
        .filter(|l| l.level == LogLevel::Info)
        .map(|l| l.message)
        .collect();

    let has = |s: &str| logs.iter().any(|m| m == s);
    assert!(has("canplace true"), "CanPlace: {logs:?}");
    assert!(has("placed true"), "PlaceBuilding: {logs:?}");
    assert!(has("class Building"), "new node class: {logs:?}");
    assert!(has("type smelter"), "Type prop baked in: {logs:?}");
    assert!(has("at33 true"), "GetBuildingAt covers footprint: {logs:?}");
    assert!(has("blocked true"), "overlap refused: {logs:?}");
    assert!(has("removed true"), "RemoveBuilding: {logs:?}");
    assert!(has("gone true"), "cell freed after removal: {logs:?}");
    assert!(has("cam true"), "ScreenToWorld/WorldToScreen round-trip: {logs:?}");
}
