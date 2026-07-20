//! Nuclear-reactor simulation and per-map power balance.
//!
//! Each reactor `Building` (a def with [`crate::building::ReactorParams`]) is a
//! small dynamical system over instance properties, so its state is inspectable,
//! scriptable (a control script drives `ControlRods`), and saved for free:
//!
//! * **ControlRods** `0..1` — 1 fully inserted (reaction off, safe), 0 withdrawn
//!   (full reaction). This is the player/AI's control input.
//! * **Fuel** `0..100` — burned while reacting; refilled by consuming a fuel item
//!   (e.g. uranium) from the reactor's inventory, tying it into the factory.
//! * **Temperature** — rises with reaction, sheds heat toward ambient. Generating
//!   efficiency peaks at the reactor's `optimal` temperature.
//! * **Integrity** `0..100` — degrades while over the `meltdown` temperature;
//!   at 0 the reactor has melted down (no power).
//! * **PowerOutput** — MW produced this tick.
//!
//! [`step`] advances every reactor and tallies each tilemap's power balance into
//! transient `_PowerProduced` / `_PowerConsumed` props (read via
//! `Tilemap:GetPower()`).

use std::path::Path;

use crate::building::{BuildingCatalogCache, ReactorParams};
use crate::value::Value;
use crate::world::{InstanceId, World};

const AMBIENT: f32 = 20.0;

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

/// Generating efficiency in `0..1`, peaking at `optimal` and tailing off as the
/// core runs too cold or too hot (a symmetric tent around `optimal`).
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

        let mut produced = 0.0;
        let mut consumed = 0.0;
        for b in ids {
            let Some(def) = cat.get(&text(world, b, "Type")) else {
                continue;
            };
            consumed += def.power_use;
            if let Some(rp) = &def.reactor {
                produced += simulate(world, b, rp, dt);
            }
        }
        set(world, tm, "_PowerProduced", produced);
        set(world, tm, "_PowerConsumed", consumed);
    }
}

/// Advance one reactor; returns the power it generated this tick (0 after
/// meltdown).
fn simulate(world: &mut World, b: InstanceId, rp: &ReactorParams, dt: f32) -> f32 {
    let mut temp = num(world, b, "Temperature");
    let mut fuel = num(world, b, "Fuel");
    let mut integrity = num(world, b, "Integrity");
    let rods = num(world, b, "ControlRods").clamp(0.0, 1.0);

    // Refuel from inventory when low.
    if !rp.fuel_item.is_empty() && fuel <= 100.0 - rp.refuel {
        let took = world
            .inventory_mut(b)
            .map(|inv| inv.take(&rp.fuel_item, 1))
            .unwrap_or(0);
        if took > 0 {
            fuel = (fuel + rp.refuel).min(100.0);
        }
    }

    // Reaction proceeds only with fuel and withdrawn rods.
    let reactivity = if fuel > 0.0 { 1.0 - rods } else { 0.0 };
    fuel = (fuel - reactivity * rp.burn * dt).max(0.0);

    // Heat in from the reaction, passive cooling toward ambient.
    let heat_in = reactivity * rp.heat * dt;
    let cooling = rp.cooling * (temp - AMBIENT) * dt;
    temp = (temp + heat_in - cooling).max(AMBIENT); // cooling never pushes below ambient

    // Overheating erodes integrity (faster the further past meltdown temp).
    if temp > rp.meltdown {
        integrity = (integrity - (temp - rp.meltdown) * 0.02 * dt).max(0.0);
    }

    let melted = integrity <= 0.0;
    let power = if melted {
        0.0
    } else {
        reactivity * rp.power * efficiency(temp, rp.optimal)
    };

    set(world, b, "Temperature", temp);
    set(world, b, "Fuel", fuel);
    set(world, b, "Integrity", integrity);
    set(world, b, "PowerOutput", power);
    power
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::BuildingCatalog;
    use std::rc::Rc;

    // Cooling strong enough to stabilize below meltdown with rods fully out, so
    // these tests exercise sustained generation (the runaway case is separate).
    const CATALOG: &str = r#"{ "buildings": [
        { "id": "reactor", "size": [2,2],
          "reactor": { "power": 100, "heat": 400, "cooling": 1.3, "burn": 0.2,
                       "meltdown": 800, "optimal": 350 } },
        { "id": "lamp", "size": [1,1], "power_use": 5 }
    ]}"#;

    /// A world with a tilemap wired to an in-memory building catalog, plus a
    /// reactor placed via the core building API.
    fn setup() -> (World, InstanceId, InstanceId, Rc<BuildingCatalog>) {
        let mut w = World::new();
        let tm = w.create("Tilemap", w.workspace()).unwrap();
        w.set_prop(tm, "MapWidth", Value::Number(16.0)).unwrap();
        w.set_prop(tm, "MapHeight", Value::Number(16.0)).unwrap();
        let cat = Rc::new(BuildingCatalog::parse(CATALOG).unwrap());
        let reactor = crate::building::place(&mut w, tm, cat.get("reactor").unwrap(), 2, 2).unwrap();
        (w, tm, reactor, cat)
    }

    fn run(world: &mut World, cat: &Rc<BuildingCatalog>, tm: InstanceId, r: InstanceId, dt: f32) {
        let def = cat.get("reactor").unwrap();
        let rp = def.reactor.as_ref().unwrap();
        let p = simulate(world, r, rp, dt);
        set(world, tm, "_PowerProduced", p);
    }

    #[test]
    fn withdrawn_rods_with_fuel_heat_and_generate() {
        let (mut w, tm, r, cat) = setup();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(0.0)).unwrap();
        for _ in 0..100 {
            run(&mut w, &cat, tm, r, 0.1);
        }
        assert!(num(&w, r, "Temperature") > 100.0, "reactor should heat up");
        assert!(num(&w, r, "PowerOutput") > 0.0, "should generate power");
        assert!(num(&w, r, "Fuel") < 100.0, "should burn fuel");
    }

    #[test]
    fn inserted_rods_stay_cold_and_idle() {
        let (mut w, tm, r, cat) = setup();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(1.0)).unwrap();
        for _ in 0..50 {
            run(&mut w, &cat, tm, r, 0.1);
        }
        assert!(num(&w, r, "Temperature") <= 20.5);
        assert_eq!(num(&w, r, "PowerOutput"), 0.0);
    }

    #[test]
    fn runaway_reactor_melts_down() {
        // A reactor with strong heat, no cooling and rods out overheats past
        // meltdown and loses all integrity.
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
            run(&mut w, &cat, tm, r, 0.1);
        }
        assert_eq!(num(&w, r, "Integrity"), 0.0, "should have melted down");
        assert_eq!(num(&w, r, "PowerOutput"), 0.0, "no power after meltdown");
    }

    #[test]
    fn step_tallies_power_balance() {
        let (mut w, tm, r, _cat) = setup();
        w.set_prop(r, "Fuel", Value::Number(100.0)).unwrap();
        w.set_prop(r, "ControlRods", Value::Number(0.0)).unwrap();
        // Place two lamps (5 MW each) via the same catalog path resolution.
        // Use the cache-driven step with an on-disk-free catalog: write a temp file.
        let dir = std::env::temp_dir().join("flux_reactor_test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.buildings.json"), CATALOG).unwrap();
        w.set_prop(tm, "Buildings", Value::Asset("b.buildings.json".into()))
            .unwrap();
        crate::building::place(&mut w, tm, _cat.get("lamp").unwrap(), 8, 8).unwrap();
        crate::building::place(&mut w, tm, _cat.get("lamp").unwrap(), 10, 10).unwrap();

        let mut cache = BuildingCatalogCache::default();
        // Warm the reactor up a bit first.
        for _ in 0..50 {
            step(&mut w, &mut cache, &dir, 0.1);
        }
        assert!(num(&w, tm, "_PowerProduced") > 0.0);
        assert_eq!(num(&w, tm, "_PowerConsumed"), 10.0); // two 5 MW lamps
    }
}
