//! Nuclear-reactor simulation and per-map power balance.
//!
//! Each reactor `Building` is a small dynamical system over instance props
//! (`ControlRods`/`Fuel`/`Temperature`/`Integrity`/`PowerOutput`), so its state
//! is inspectable, scriptable and saved for free. [`ReactorSystem`] advances
//! every reactor each frame and tallies each tilemap's power balance into the
//! `PowerProduced`/`PowerConsumed` attributes (read via `Tilemap:GetPower`).

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
        let bc_path = crate::attr_text(world, tm, "Buildings");
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
            if let Some(tp) = &def.turbine {
                produced += turbine(world, b, tp, dt);
            }
        }
        // Cooling towers vent steam while they're actually shedding heat.
        for (t, _, _) in &towers {
            let state = if hot_towers.contains(t) { "working" } else { "idle" };
            crate::factory::apply_state(world, *t, state);
        }
        crate::set_attr_num(world, tm, "PowerProduced", produced as f64);
        crate::set_attr_num(world, tm, "PowerConsumed", consumed as f64);
    }
}

fn set_status(world: &mut World, b: InstanceId, status: &str) {
    if text(world, b, "_Status") != status {
        let _ = world.set_prop(b, "_Status", Value::String(status.to_string()));
    }
}

/// Advance one reactor; returns the ELECTRIC power it generated this frame.
///
/// Two modes, chosen by the def's tanks:
/// - **Steam reactor** (declares `water` + `steam` tanks): consumes coolant
///   water ∝ reactivity, converts it to steam (its real output — turbines
///   make the electricity), emits waste items per fuel burned, and loses
///   most of its cooling when coolant is missing or the steam tank is full.
///   Generates no electricity directly. A blocked waste output SCRAMs it.
/// - **Legacy air-cooled** (no tanks): the original direct-power dynamics.
///
/// `cooling_boost` is the summed coefficient from adjacent cooling towers.
fn simulate(world: &mut World, b: InstanceId, rp: &ReactorParams, cooling_boost: f32, dt: f32) -> f32 {
    let mut temp = num(world, b, "Temperature");
    let mut fuel = num(world, b, "Fuel");
    let mut integrity = num(world, b, "Integrity");
    let rods = num(world, b, "ControlRods").clamp(0.0, 1.0);

    let steam_mode = world
        .component::<crate::fluids::Tank>(b)
        .is_some_and(|t| t.slot("water").is_some() && t.slot("steam").is_some());

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

    let mut reactivity = if fuel > 0.0 { 1.0 - rods } else { 0.0 };
    let mut status = if fuel <= 0.0 { "No fuel" } else { "" };

    // Waste handling first: a blocked waste output SCRAMs the reactor.
    let mut waste_blocked = false;
    if steam_mode && !rp.waste_item.is_empty() {
        let acc = num(world, b, "_WasteAcc");
        if acc >= rp.waste_every {
            let added = world
                .component_mut::<Inventory>(b)
                .map(|inv| inv.add(&rp.waste_item, 1))
                .unwrap_or(0);
            if added > 0 {
                set(world, b, "_WasteAcc", acc - rp.waste_every);
            } else {
                waste_blocked = true;
                reactivity = 0.0;
                status = "Output blocked";
            }
        }
    }

    // Coolant conversion: water in -> steam out; missing coolant or a full
    // steam tank collapses the water-loop cooling.
    let mut coolant_factor = 1.0;
    if steam_mode && reactivity > 0.0 {
        let want = reactivity * rp.water_use * dt;
        let (drained, steamed) = match world.component_mut::<crate::fluids::Tank>(b) {
            Some(tank) => {
                // Convert what both sides allow: available water AND steam space.
                let avail = tank.slot("water").map(|s| s.volume).unwrap_or(0.0);
                let space = tank.slot("steam").map(|s| s.space()).unwrap_or(0.0);
                let convert = want.min(avail).min(space);
                if convert > 0.0 {
                    tank.slot_mut("water").unwrap().drain(convert);
                    tank.slot_mut("steam").unwrap().fill("steam", convert);
                }
                (avail, convert)
            }
            None => (0.0, 0.0),
        };
        if want > 0.0 {
            coolant_factor = 0.15 + 0.85 * (steamed / want).clamp(0.0, 1.0);
        }
        if !waste_blocked {
            if drained < want * 0.5 {
                status = "Missing coolant";
            } else if steamed < want * 0.5 {
                status = "Steam tank full";
            }
        }
    }

    let burned = reactivity * rp.burn * dt;
    fuel = (fuel - burned).max(0.0);
    if steam_mode && !rp.waste_item.is_empty() {
        set(world, b, "_WasteAcc", num(world, b, "_WasteAcc") + burned);
    }

    let heat_in = reactivity * rp.heat * dt;
    let water_cooling = if steam_mode { (rp.cooling + cooling_boost) * coolant_factor } else { rp.cooling + cooling_boost };
    let cooling = water_cooling * (temp - AMBIENT) * dt;
    temp = (temp + heat_in - cooling).max(AMBIENT);

    if temp > rp.meltdown {
        integrity = (integrity - (temp - rp.meltdown) * 0.02 * dt).max(0.0);
    }

    // Steam reactors report their steam rate on the gauge and generate no
    // electricity directly; legacy reactors keep direct generation.
    let (power, output) = if integrity <= 0.0 {
        (0.0, 0.0)
    } else if steam_mode {
        (0.0, reactivity * rp.water_use * coolant_factor)
    } else {
        let p = reactivity * rp.power * efficiency(temp, rp.optimal);
        (p, p)
    };

    set(world, b, "Temperature", temp);
    set(world, b, "Fuel", fuel);
    set(world, b, "Integrity", integrity);
    set(world, b, "PowerOutput", output);

    // Visible reactor state -> sprite clip (off/running/hot/meltdown).
    let state = if integrity <= 0.0 {
        set_status(world, b, "MELTDOWN");
        "meltdown"
    } else if temp > rp.optimal * 1.2 {
        if status.is_empty() {
            status = "Overheating";
        }
        set_status(world, b, status);
        "hot"
    } else if reactivity > 0.0 && (output > 0.05 || !steam_mode) {
        set_status(world, b, status);
        "running"
    } else {
        if status.is_empty() && rods >= 1.0 {
            status = "Control rods inserted";
        }
        set_status(world, b, status);
        "off"
    };
    crate::factory::apply_state(world, b, state);
    power
}

/// Advance one steam turbine; returns the electric power generated.
fn turbine(world: &mut World, b: InstanceId, tp: &crate::building::TurbineDoc, dt: f32) -> f32 {
    let want = tp.steam_use * dt;
    let got = {
        let Some(tank) = world.component_mut::<crate::fluids::Tank>(b) else {
            return 0.0;
        };
        match tank.slot_mut(&tp.tank) {
            Some(s) if s.fluid == "steam" || s.fluid.is_empty() => s.drain(want),
            _ => 0.0,
        }
    };
    let load = if want > 0.0 { (got / want).clamp(0.0, 1.0) } else { 0.0 };
    let power = tp.power * load;
    set(world, b, "PowerOutput", power);
    let hold = if load > 0.05 {
        set_status(world, b, "");
        0.25
    } else {
        (num(world, b, "_StateHold") - dt).max(0.0)
    };
    set(world, b, "_StateHold", hold);
    if hold > 0.0 {
        crate::factory::apply_state(world, b, "working");
    } else {
        set_status(world, b, "No steam");
        crate::factory::apply_state(world, b, "idle");
    }
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
        let reactor = crate::building::place(&mut w, tm, cat.get("reactor").unwrap(), 2, 2, 0).unwrap();
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
        let r = crate::building::place(&mut w, tm, cat.get("reactor").unwrap(), 0, 0, 0).unwrap();
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
        w.set_attribute(tm, "Buildings", Value::Asset("b.buildings.json".into())).unwrap();
        let cat = Rc::new(BuildingCatalog::parse(catalog).unwrap());
        let r = crate::building::place(&mut w, tm, cat.get("reactor").unwrap(), 2, 2, 0).unwrap();
        let t = crate::building::place(&mut w, tm, cat.get("cooling").unwrap(), 4, 2, 0).unwrap();
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
        w.set_attribute(tm, "Buildings", Value::Asset("b.buildings.json".into())).unwrap();
        crate::building::place(&mut w, tm, cat.get("lamp").unwrap(), 8, 8, 0).unwrap();
        crate::building::place(&mut w, tm, cat.get("lamp").unwrap(), 10, 10, 0).unwrap();

        let mut cache = BuildingCatalogCache::default();
        for _ in 0..50 {
            step(&mut w, &mut cache, &dir, 0.1);
        }
        assert!(crate::attr_num(&w, tm, "PowerProduced") > 0.0);
        assert_eq!(crate::attr_num(&w, tm, "PowerConsumed"), 10.0);
    }
}
