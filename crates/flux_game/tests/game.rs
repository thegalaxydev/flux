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
fn pipe_visuals_follow_connectivity() {
    let root = setup();
    let mut session = session_world(root);
    let (mid, end_w, arm_s, lone) = {
        let world = session.world();
        let mut w = world.borrow_mut();
        let map = w.find_first_child(w.workspace(), "Map").unwrap();
        let cat = flux_game::building::BuildingCatalog::parse(
            &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
        )
        .unwrap();
        let pipe = cat.get("pipe").unwrap();
        // A T shape: west-east run through (5,5) plus a south arm.
        let end_w = flux_game::building::place(&mut w, map, pipe, 4, 5, 0).unwrap();
        let mid = flux_game::building::place(&mut w, map, pipe, 5, 5, 0).unwrap();
        let _e = flux_game::building::place(&mut w, map, pipe, 6, 5, 0).unwrap();
        let arm_s = flux_game::building::place(&mut w, map, pipe, 5, 6, 0).unwrap();
        // Plus a lone pipe next to the boiler's liquid port: boiler at (10,10),
        // port cell (10,10) facing (10,9) -> pipe there connects south.
        flux_game::building::place(&mut w, map, cat.get("boiler").unwrap(), 10, 10, 0).unwrap();
        let lone = flux_game::building::place(&mut w, map, pipe, 10, 9, 0).unwrap();
        (mid, end_w, arm_s, lone)
    };
    for _ in 0..3 {
        session.step(0.05, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    let anim = |b| {
        let sprite = flux_game::building::sprite_of(&w, b).unwrap();
        match w.get_prop(sprite, "Animation") {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        }
    };
    assert_eq!(anim(mid), "m14", "T junction: E+S+W");
    assert_eq!(anim(end_w), "m2", "west end connects east only");
    assert_eq!(anim(arm_s), "m1", "south arm connects north only");
    assert_eq!(anim(lone), "m4", "pipe connects to the boiler port to its south");
}

fn tank_volume(w: &flux_core::World, b: flux_core::InstanceId, slot: &str) -> f32 {
    w.component::<flux_game::fluids::Tank>(b)
        .and_then(|t| t.slot(slot))
        .map(|s| s.volume)
        .unwrap_or(0.0)
}

#[test]
fn liquid_flows_through_pipes_and_is_conserved() {
    let root = setup();
    let mut session = session_world(root);
    let (map, well, intake, pipes) = {
        let world = session.world();
        let mut w = world.borrow_mut();
        let map = w.find_first_child(w.workspace(), "Map").unwrap();
        let cat = flux_game::building::BuildingCatalog::parse(
            &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
        )
        .unwrap();
        // well(2,2) -e-> pipes (3..5,2) -> intake(6,2) with a west input.
        let well = flux_game::building::place(&mut w, map, cat.get("well").unwrap(), 2, 2, 0).unwrap();
        let p: Vec<_> = (3..=5)
            .map(|x| flux_game::building::place(&mut w, map, cat.get("pipe").unwrap(), x, 2, 0).unwrap())
            .collect();
        let intake = flux_game::building::place(&mut w, map, cat.get("intake").unwrap(), 6, 2, 0).unwrap();
        if let Some(t) = w.component_mut::<flux_game::fluids::Tank>(well) {
            t.slot_mut("out").unwrap().fill("water", 100.0);
        }
        (map, well, intake, p)
    };
    for _ in 0..40 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    let received = tank_volume(&w, intake, "in");
    assert!(received > 20.0, "water should reach the intake: {received}");
    // Conservation: everything that left the well is in pipes or the intake.
    let mut total = tank_volume(&w, well, "out") + received;
    for &p in &pipes {
        total += tank_volume(&w, p, "pipe");
    }
    assert!((total - 100.0).abs() < 0.01, "volume must be conserved: {total}");
    let fluid = w
        .component::<flux_game::fluids::Tank>(intake)
        .and_then(|t| t.slot("in"))
        .map(|s| s.fluid.clone())
        .unwrap();
    assert_eq!(fluid, "water");
    let _ = map;
}

#[test]
fn severed_pipe_network_stops_flow() {
    let root = setup();
    let mut session = session_world(root);
    let (map, well, intake) = {
        let world = session.world();
        let mut w = world.borrow_mut();
        let map = w.find_first_child(w.workspace(), "Map").unwrap();
        let cat = flux_game::building::BuildingCatalog::parse(
            &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
        )
        .unwrap();
        let well = flux_game::building::place(&mut w, map, cat.get("well").unwrap(), 2, 2, 0).unwrap();
        for x in 3..=5 {
            flux_game::building::place(&mut w, map, cat.get("pipe").unwrap(), x, 2, 0).unwrap();
        }
        let intake = flux_game::building::place(&mut w, map, cat.get("intake").unwrap(), 6, 2, 0).unwrap();
        if let Some(t) = w.component_mut::<flux_game::fluids::Tank>(well) {
            t.slot_mut("out").unwrap().fill("water", 200.0);
        }
        (map, well, intake)
    };
    for _ in 0..10 {
        session.step(0.1, &InputFrame::default());
    }
    // Sever the line, let the cut-off remnant drain into the intake...
    {
        let world = session.world();
        let mut w = world.borrow_mut();
        assert!(flux_game::building::remove_at(&mut w, map, 4, 2));
    }
    for _ in 0..20 {
        session.step(0.1, &InputFrame::default());
    }
    let after_drain = {
        let world = session.world();
        let w = world.borrow();
        (tank_volume(&w, intake, "in"), tank_volume(&w, well, "out"))
    };
    // ...then nothing more crosses the gap, in either direction.
    for _ in 0..20 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    assert_eq!(tank_volume(&w, intake, "in"), after_drain.0, "no flow across a severed network");
    assert_eq!(tank_volume(&w, well, "out"), after_drain.1, "source stops once its side is full");
}

#[test]
fn input_only_ports_never_emit() {
    let root = setup();
    let mut session = session_world(root);
    let (pipe, intake) = {
        let world = session.world();
        let mut w = world.borrow_mut();
        let map = w.find_first_child(w.workspace(), "Map").unwrap();
        let cat = flux_game::building::BuildingCatalog::parse(
            &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
        )
        .unwrap();
        // The intake's only port is an INPUT facing west — a full tank must
        // never leak back out into the pipe.
        let pipe = flux_game::building::place(&mut w, map, cat.get("pipe").unwrap(), 1, 2, 0).unwrap();
        let intake = flux_game::building::place(&mut w, map, cat.get("intake").unwrap(), 2, 2, 0).unwrap();
        if let Some(t) = w.component_mut::<flux_game::fluids::Tank>(intake) {
            t.slot_mut("in").unwrap().fill("water", 150.0);
        }
        (pipe, intake)
    };
    for _ in 0..30 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    assert_eq!(tank_volume(&w, pipe, "pipe"), 0.0, "input-only port emitted fluid");
    assert_eq!(tank_volume(&w, intake, "in"), 150.0);
}

#[test]
fn reactor_steam_loop_end_to_end() {
    let root = setup();
    let mut session = session_world(root);
    let (reactor, turbine, storage) = {
        let world = session.world();
        let mut w = world.borrow_mut();
        let map = w.find_first_child(w.workspace(), "Map").unwrap();
        let cat = flux_game::building::BuildingCatalog::parse(
            &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
        )
        .unwrap();
        // A water tile next to the pump; pump -> pipes -> reactor west port;
        // turbine hugs the reactor's east steam port (direct pair); waste
        // exits south onto a belt into storage.
        let water_idx = 3; // "water" in test.tileset.json
        w.tile_grid_mut(map).unwrap().set_tile(1, 5, water_idx);
        let pump = flux_game::building::place(&mut w, map, cat.get("pump").unwrap(), 2, 5, 0).unwrap();
        flux_game::building::place(&mut w, map, cat.get("pipe").unwrap(), 3, 5, 0).unwrap();
        flux_game::building::place(&mut w, map, cat.get("pipe").unwrap(), 4, 5, 0).unwrap();
        let reactor =
            flux_game::building::place(&mut w, map, cat.get("nreactor").unwrap(), 5, 5, 0).unwrap();
        let turbine =
            flux_game::building::place(&mut w, map, cat.get("turbine2").unwrap(), 7, 5, 0).unwrap();
        let belt = flux_game::building::place(&mut w, map, cat.get("belt").unwrap(), 5, 7, 1).unwrap();
        let storage =
            flux_game::building::place(&mut w, map, cat.get("storage").unwrap(), 5, 8, 0).unwrap();
        // Fuel straight into the reactor (port-fed delivery covered elsewhere)
        // and pull the rods.
        if let Some(inv) = w.component_mut::<Inventory>(reactor) {
            inv.add("uranium", 10);
        }
        w.set_prop(reactor, "ControlRods", Value::Number(0.0)).unwrap();
        let _ = (pump, belt);
        (reactor, turbine, storage)
    };
    for _ in 0..400 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    let num = |b, p: &str| match w.get_prop(b, p) {
        Some(Value::Number(n)) => *n as f32,
        _ => 0.0,
    };
    let status = |b| match w.get_prop(b, "_Status") {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    // Water reached the reactor, steam reached the turbine, power flows.
    assert!(tank_volume(&w, reactor, "water") > 0.0, "coolant arrived");
    assert!(num(turbine, "PowerOutput") > 50.0, "turbine generating: {}", num(turbine, "PowerOutput"));
    assert_eq!(status(reactor), "", "reactor healthy: {:?}", status(reactor));
    // Spent fuel left through the waste port onto the belt into storage.
    let waste = w.component::<Inventory>(storage).map(|i| i.count("waste")).unwrap_or(0);
    assert!(waste >= 1, "waste should reach storage: {waste}");
    // And the reactor never gave away its uranium.
    let fuel_left = w.component::<Inventory>(reactor).map(|i| i.count("uranium")).unwrap_or(99);
    let storage_uranium = w.component::<Inventory>(storage).map(|i| i.count("uranium")).unwrap_or(0);
    assert_eq!(storage_uranium, 0, "fuel must not leak out of the waste port");
    let _ = fuel_left;
}

#[test]
fn reactor_without_coolant_reports_and_stalls() {
    let root = setup();
    let mut session = session_world(root);
    let reactor = {
        let world = session.world();
        let mut w = world.borrow_mut();
        let map = w.find_first_child(w.workspace(), "Map").unwrap();
        let cat = flux_game::building::BuildingCatalog::parse(
            &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
        )
        .unwrap();
        let reactor =
            flux_game::building::place(&mut w, map, cat.get("nreactor").unwrap(), 5, 5, 0).unwrap();
        if let Some(inv) = w.component_mut::<Inventory>(reactor) {
            inv.add("uranium", 5);
        }
        w.set_prop(reactor, "ControlRods", Value::Number(0.0)).unwrap();
        reactor
    };
    for _ in 0..50 {
        session.step(0.1, &InputFrame::default());
    }
    let world = session.world();
    let w = world.borrow();
    let status = match w.get_prop(reactor, "_Status") {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    assert_eq!(status, "Missing coolant");
    assert_eq!(tank_volume(&w, reactor, "steam"), 0.0, "no steam without water");
}

#[test]
fn tanks_round_trip_through_save() {
    let root = setup();
    let mut w = World::new();
    let map = w.create("Tilemap", w.workspace()).unwrap();
    w.set_prop(map, "MapWidth", Value::Number(16.0)).unwrap();
    w.set_prop(map, "MapHeight", Value::Number(16.0)).unwrap();
    let cat = flux_game::building::BuildingCatalog::parse(
        &std::fs::read_to_string(root.join("test.buildings.json")).unwrap(),
    )
    .unwrap();
    let boiler = flux_game::building::place(&mut w, map, cat.get("boiler").unwrap(), 2, 2, 0).unwrap();
    if let Some(t) = w.component_mut::<flux_game::fluids::Tank>(boiler) {
        t.slot_mut("water").unwrap().fill("water", 42.5);
    }

    let saved = w.to_save_string();
    let reloaded = World::from_json(&saved).unwrap();
    let boiler2 = reloaded
        .descendants(reloaded.workspace())
        .into_iter()
        .find(|&id| {
            reloaded.class_name(id) == Some("Building")
                && matches!(reloaded.get_prop(id, "Type"), Some(Value::String(s)) if s == "boiler")
        })
        .expect("boiler restored");
    let tank = reloaded.component::<flux_game::fluids::Tank>(boiler2).expect("tank restored");
    let slot = tank.slot("water").unwrap();
    assert_eq!(slot.fluid, "water");
    assert!((slot.volume - 42.5).abs() < 1e-4);
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
