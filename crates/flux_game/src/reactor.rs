//! Nuclear-reactor simulation and per-map power balance.
//!
//! Each reactor `Building` is a small dynamical system over instance props
//! (`ControlRods`/`Fuel`/`Temperature`/`Integrity`/`PowerOutput`), so its state
//! is inspectable, scriptable and saved for free. [`ReactorSystem`] advances
//! every reactor each frame and tallies each tilemap's power balance into
//! transient `_PowerProduced`/`_PowerConsumed` props (read via `Tilemap:GetPower`).

use std::path::Path;

use flux_core::{InstanceId, Value, World};

use crate::building::{BuildingCatalogCache, ReactorParams};
use crate::factory::Inventory;

const AMBIENT: f32 = 20.0;

/// The runtime system that advances reactors and the power balance each frame.
#[derive(Default)]
pub struct ReactorSystem {
    buildings: BuildingCatalogCache,
}

impl flux_runtime::System for ReactorSystem {
    fn step(&mut self, world: &mut World, root: &Path, dt: f32) {
        step(world, &mut self.buildings, root, dt);
    }
}

fn num(world: &World, id: InstanceId, name: &str) -> f32 {
    match world.get_prop(id, name) {
        Some(Value::Number(n)) => *n as f32,
        _ => 0.0,
    }
}

fn set(world: &mut World, id: InstanceId, name: &str, v: f32) {
    let _ = world.set_prop(id, name, Value::Number(v as f64));
}

fn text(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(Value::String(s)) | Some(Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Generating efficiency in `0..1`, peaking at `optimal`.
fn efficiency(temp: f32, optimal: f32) -> f32 {
    (1.0 - (temp - optimal).abs() / optimal).clamp(0.0, 1.0)
}

/// Advance all reactors by `dt` and refresh each tilemap's power balance.
pub fn step(world: &mut World, buildings: &mut BuildingCatalogCache, root: &Path, dt: f32) {
    let maps: Vec<InstanceId> = world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("Tilemap"))
        .collect();

    for tm in maps {
        let bc_path = match world.get_prop(tm, "Buildings") {
            Some(Value::Asset(s)) => s.clone(),
            _ => String::new(),
        };
        let Some(cat) = buildings.get(&bc_path, root) else {
            continue;
        };

        let ids: Vec<InstanceId> = world
            .children(tm)
            .iter()
            .copied()
            .filter(|&c| world.class_name(c) == Some("Building"))
            .collect();

        // Footprints for adjacency: cooling towers boost neighbouring reactors.
        let foot = |world: &World, b: InstanceId| -> (i32, i32, i32, i32) {
            let cell = match world.get_prop(b, "Cell") {
                Some(Value::Vec2(v)) => *v,
                _ => glam::Vec2::ZERO,
            };
            let fp = match world.get_prop(b, "Footprint") {
                Some(Value::Vec2(v)) => *v,
                _ => glam::Vec2::ONE,
            };
            (cell.x as i32, cell.y as i32, (fp.x as i32).max(1), (fp.y as i32).max(1))
        };
        let adjacent = |a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)| -> bool {
            // (col, row, w, h): footprints touch when a's 1-tile-expanded rect
            // overlaps b's rect.
            a.0 - 1 < b.0 + b.2 && a.0 + a.2 + 1 > b.0 && a.1 - 1 < b.1 + b.3 && a.1 + a.3 + 1 > b.1
        };
        let towers: Vec<(InstanceId, (i32, i32, i32, i32), f32)> = ids
            .iter()
            .filter_map(|&b| {
                let def = cat.get(&text(world, b, "Type"))?;
                (def.cooling > 0.0).then(|| (b, foot(world, b), def.cooling))
            })
            .collect();

        let mut produced = 0.0;
        let mut consumed = 0.0;
        let mut hot_towers: Vec<InstanceId> = Vec::new();
        for b in ids.clone() {
            let Some(def) = cat.get(&text(world, b, "Type")) else {
                continue;
            };
            consumed += def.power_use;
            if let Some(rp) = &def.reactor {
                let rf = foot(world, b);
                let boost: f32 = towers
                    .iter()
                    .filter(|(t, tf, _)| *t != b && adjacent(*tf, rf))
                    .map(|(_, _, c)| *c)
                    .sum();
                produced += simulate(world, b, rp, boost, dt);
                if num(world, b, "Temperature") > 60.0 {
                    hot_towers.extend(
                        towers.iter().filter(|(t, tf, _)| *t != b && adjacent(*tf, rf)).map(|(t, _, _)| *t),
                    );
                }
            }
        }
        // Cooling towers vent steam while they're actually shedding heat.
        for (t, _, _) in &towers {
            let state = if hot_towers.contains(t) { "working" } else { "idle" };
            crate::factory::apply_state(world, *t, state);
        }
        set(world, tm, "_PowerProduced", produced);
        set(world, tm, "_PowerConsumed", consumed);
    }
}

/// Advance one reactor; returns the power it generated (0 after meltdown).
/// `cooling_boost` is the summed coefficient from adjacent cooling towers.
fn simulate(world: &mut World, b: InstanceId, rp: &ReactorParams, cooling_boost: f32, dt: f32) -> f32 {
    let mut temp = num(world, b, "Temperature");
    let mut fuel = num(world, b, "Fuel");
    let mut integrity = num(world, b, "Integrity");
    let rods = num(world, b, "ControlRods").clamp(0.0, 1.0);

    // Refuel from inventory when low.
    if !rp.fuel_item.is_empty() && fuel <= 100.0 - rp.refuel {
        let took = world
            .component_mut::<Inventory>(b)
            .map(|inv| inv.take(&rp.fuel_item, 1))
            .unwrap_or(0);
        if took > 0 {
            fuel = (fuel + rp.refuel).min(100.0);
        }
    }

    let reactivity = if fuel > 0.0 { 1.0 - rods } else { 0.0 };
    fuel = (fuel - reactivity * rp.burn * dt).max(0.0);

    let heat_in = reactivity * rp.heat * dt;
    let cooling = (rp.cooling + cooling_boost) * (temp - AMBIENT) * dt;
    temp = (temp + heat_in - cooling).max(AMBIENT);

    if temp > rp.meltdown {
        integrity = (integrity - (temp - rp.meltdown) * 0.02 * dt).max(0.0);
    }

    let power = if integrity <= 0.0 {
        0.0
    } else {
        reactivity * rp.power * efficiency(temp, rp.optimal)
    };

    set(world, b, "Temperature", temp);
    set(world, b, "Fuel", fuel);
    set(world, b, "Integrity", integrity);
    set(world, b, "PowerOutput", power);

    // Visible reactor state -> sprite clip (off/running/hot/meltdown).
    let state = if integrity <= 0.0 {
        "meltdown"
    } else if temp > rp.optimal * 1.2 {
        "hot"
    } else if power > 0.5 {
        "running"
    } else {
        "off"
    };
    crate::factory::apply_state(world, b, state);
    power
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::BuildingCatalog;
    use std::rc::Rc;

    const CATALOG: &str = r#"{ "buildings": [
        { "id": "reactor", "size": [2,2],
          "reactor": { "power": 100, "heat": 400, "cooling": 1.3, "burn": 0.2,
                       "meltdown": 800, "optimal": 350 } },
        { "id": "lamp", "size": [1,1], "power_use": 5 }
    ]}"#;

    fn setup() -> (World, InstanceId, InstanceId, Rc<BuildingCatalog>) {
        crate::install();
        let mut w = World::new();
        let tm = w.create("Tilemap", w.workspace()).unwrap();
        w.set_prop(tm, "MapWidth", Value::Number(16.0)).unwrap();
        w.set_prop(tm, "MapHeight", Value::Number(16.0)).unwrap();
        let cat = Rc::new(BuildingCatalog::parse(CATALOG).unwrap());
        let reactor = crate::building::place(&mut w, tm, cat.get("reactor").unwrap(), 2, 2).unwrap();
        (w, tm, reactor, cat)
    }

    fn run(world: &mut World, cat: &Rc<BuildingCatalog>, r: InstanceId, dt: f32) {
        let rp = cat.get("reactor").unwrap().reactor.as_ref().unwrap();
        simulate(world, r, rp, 0.0, dt);
    }

    #[test]
    fn withdrawn_rods_with_fuel_heat_and_generate() {
        let (mut w, _tm, r, cat) = setup();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(0.0)).unwrap();
        for _ in 0..100 {
            run(&mut w, &cat, r, 0.1);
        }
        assert!(num(&w, r, "Temperature") > 100.0);
        assert!(num(&w, r, "PowerOutput") > 0.0);
        assert!(num(&w, r, "Fuel") < 100.0);
    }

    #[test]
    fn inserted_rods_stay_cold_and_idle() {
        let (mut w, _tm, r, cat) = setup();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(1.0)).unwrap();
        for _ in 0..50 {
            run(&mut w, &cat, r, 0.1);
        }
        assert!(num(&w, r, "Temperature") <= 20.5);
        assert_eq!(num(&w, r, "PowerOutput"), 0.0);
    }

    #[test]
    fn runaway_reactor_melts_down() {
        crate::install();
        let cat = Rc::new(
            BuildingCatalog::parse(
                r#"{ "buildings": [{ "id": "reactor", "size":[2,2],
                "reactor": { "power":100, "heat":2000, "cooling":0.0, "burn":0.0,
                             "meltdown":500, "optimal":350 } }]}"#,
            )
            .unwrap(),
        );
        let mut w = World::new();
        let tm = w.create("Tilemap", w.workspace()).unwrap();
        w.set_prop(tm, "MapWidth", Value::Number(8.0)).unwrap();
        w.set_prop(tm, "MapHeight", Value::Number(8.0)).unwrap();
        let r = crate::building::place(&mut w, tm, cat.get("reactor").unwrap(), 0, 0).unwrap();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(0.0)).unwrap();
        for _ in 0..2000 {
            run(&mut w, &cat, r, 0.1);
        }
        assert_eq!(num(&w, r, "Integrity"), 0.0);
        assert_eq!(num(&w, r, "PowerOutput"), 0.0);
    }

    #[test]
    fn cooling_tower_boosts_reactor_and_steams() {
        crate::install();
        let catalog = r#"{ "buildings": [
            { "id": "reactor", "size": [2,2], "sprite": "art/r.frames.json",
              "reactor": { "power": 100, "heat": 400, "cooling": 0.1, "burn": 0.0,
                           "meltdown": 2000, "optimal": 350 } },
            { "id": "cooling", "size": [2,2], "cooling": 1.2, "sprite": "art/c.frames.json" }
        ]}"#;
        let dir = std::env::temp_dir().join("flux_game_cooling_test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.buildings.json"), catalog).unwrap();

        let mut w = World::new();
        let tm = w.create("Tilemap", w.workspace()).unwrap();
        w.set_prop(tm, "MapWidth", Value::Number(16.0)).unwrap();
        w.set_prop(tm, "MapHeight", Value::Number(16.0)).unwrap();
        w.set_prop(tm, "Buildings", Value::Asset("b.buildings.json".into())).unwrap();
        let cat = Rc::new(BuildingCatalog::parse(catalog).unwrap());
        let r = crate::building::place(&mut w, tm, cat.get("reactor").unwrap(), 2, 2).unwrap();
        let t = crate::building::place(&mut w, tm, cat.get("cooling").unwrap(), 4, 2).unwrap();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(0.0)).unwrap();

        let mut cache = BuildingCatalogCache::default();
        for _ in 0..300 {
            step(&mut w, &mut cache, &dir, 0.1);
        }
        // The tower sheds heat: equilibrium far below the uncooled ~3000 C.
        let temp = num(&w, r, "Temperature");
        assert!(temp < 400.0, "boosted cooling should tame the reactor: {temp}");
        // Both publish visible states, and their sprites follow.
        assert_eq!(text(&w, r, "_State"), "running");
        assert_eq!(text(&w, t, "_State"), "working");
        let sprite = crate::building::sprite_of(&w, t).expect("tower sprite");
        assert_eq!(text(&w, sprite, "Animation"), "working");
    }

    #[test]
    fn step_tallies_power_balance() {
        let (mut w, tm, r, cat) = setup();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(0.0)).unwrap();
        let dir = std::env::temp_dir().join("flux_game_reactor_test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.buildings.json"), CATALOG).unwrap();
        w.set_prop(tm, "Buildings", Value::Asset("b.buildings.json".into()))
            .unwrap();
        crate::building::place(&mut w, tm, cat.get("lamp").unwrap(), 8, 8).unwrap();
        crate::building::place(&mut w, tm, cat.get("lamp").unwrap(), 10, 10).unwrap();

        let mut cache = BuildingCatalogCache::default();
        for _ in 0..50 {
            step(&mut w, &mut cache, &dir, 0.1);
        }
        assert!(num(&w, tm, "_PowerProduced") > 0.0);
        assert_eq!(num(&w, tm, "_PowerConsumed"), 10.0);
    }
}
