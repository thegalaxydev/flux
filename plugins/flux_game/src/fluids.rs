//! Fluids: resource definitions, machine tanks, and the pipe-network
//! simulation.
//!
//! - `*.fluids.json` defines fluid resources (name/colour now; density,
//!   heat capacity, boiling/freezing reserved for later mechanics).
//! - Machines author `tanks: [{id, capacity, accepts}]`; contents live in a
//!   [`Tank`] component persisted through the component save hook. Pipes get
//!   an implicit small tank, so fluid in a line survives save/load.
//! - [`FluidSystem`] advances on a **fixed 0.1 s tick** (accumulator, so the
//!   sim is frame-rate independent): pipe cells flood-fill into networks
//!   (derived state, rebuilt every tick — cheap at this scale and always
//!   correct after load or demolition); machine **output** ports push into
//!   the attached network up to their throughput, the network fluid-locks to
//!   whatever it first carries, and **input** ports pull into accepting
//!   tanks. Facing output/input port pairs also transfer directly, so a pump
//!   can hug a reactor without a pipe between. Every transfer is
//!   `min(available, space, throughput)` — volume is conserved by
//!   construction, nothing flows out of an input-only port, and items can
//!   never enter the fluid path (kind-checked ports).

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use flux_core::{InstanceId, Value, World};

use crate::building::BuildingCatalogCache;
use crate::ports::ResolvedPort;

// ---------------------------------------------------------------------------
// Fluid catalog (`*.fluids.json`)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct FluidCatalogDoc {
    #[serde(default)]
    pub fluids: Vec<FluidDoc>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FluidDoc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_color")]
    pub color: [f32; 4],
    // Reserved for later mechanics.
    #[serde(default = "one")]
    pub density: f32,
    #[serde(default = "default_heat_capacity")]
    pub heat_capacity: f32,
    #[serde(default = "default_boiling")]
    pub boiling_point: f32,
    #[serde(default)]
    pub freezing_point: f32,
}

fn default_color() -> [f32; 4] {
    [0.4, 0.6, 0.9, 1.0]
}
fn one() -> f32 {
    1.0
}
fn default_heat_capacity() -> f32 {
    4.2
}
fn default_boiling() -> f32 {
    100.0
}

pub struct FluidCatalog {
    fluids: Vec<FluidDoc>,
}

impl FluidCatalog {
    pub fn parse(json: &str) -> Result<Self, String> {
        let doc: FluidCatalogDoc = serde_json::from_str(json).map_err(|e| e.to_string())?;
        Ok(FluidCatalog { fluids: doc.fluids })
    }

    pub fn get(&self, id: &str) -> Option<&FluidDoc> {
        self.fluids.iter().find(|f| f.id == id)
    }

    pub fn all(&self) -> &[FluidDoc] {
        &self.fluids
    }
}

#[derive(Default)]
pub struct FluidCatalogCache {
    catalogs: HashMap<String, Option<Rc<FluidCatalog>>>,
}

impl FluidCatalogCache {
    pub fn get(&mut self, rel: &str, root: &Path) -> Option<Rc<FluidCatalog>> {
        if rel.is_empty() {
            return None;
        }
        if let Some(v) = self.catalogs.get(rel) {
            return v.clone();
        }
        let loaded = std::fs::read_to_string(root.join(rel))
            .ok()
            .and_then(|text| FluidCatalog::parse(&text).ok())
            .map(Rc::new);
        self.catalogs.insert(rel.to_string(), loaded.clone());
        loaded
    }
}

// ---------------------------------------------------------------------------
// Tanks (a plugin component, persisted like Inventory)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct TankSlot {
    pub id: String,
    pub fluid: String,
    pub volume: f32,
    pub capacity: f32,
    pub accepts: Vec<String>,
}

impl TankSlot {
    pub fn space(&self) -> f32 {
        (self.capacity - self.volume).max(0.0)
    }

    pub fn accepts_fluid(&self, fluid: &str) -> bool {
        (self.accepts.is_empty() || self.accepts.iter().any(|a| a == fluid))
            && (self.fluid.is_empty() || self.fluid == fluid || self.volume <= f32::EPSILON)
    }

    /// Pour `amount` of `fluid` in; returns how much fit.
    pub fn fill(&mut self, fluid: &str, amount: f32) -> f32 {
        if amount <= 0.0 || !self.accepts_fluid(fluid) {
            return 0.0;
        }
        let added = amount.min(self.space());
        if added > 0.0 {
            self.fluid = fluid.to_string();
            self.volume += added;
        }
        added
    }

    /// Take up to `amount` out; returns how much came.
    pub fn drain(&mut self, amount: f32) -> f32 {
        let taken = amount.clamp(0.0, self.volume);
        self.volume -= taken;
        if self.volume <= f32::EPSILON {
            self.volume = 0.0;
            self.fluid.clear();
        }
        taken
    }
}

/// Per-building fluid storage: one slot per authored tank.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tank {
    pub slots: Vec<TankSlot>,
}

impl Tank {
    pub fn slot(&self, id: &str) -> Option<&TankSlot> {
        self.slots.iter().find(|s| s.id == id)
    }

    pub fn slot_mut(&mut self, id: &str) -> Option<&mut TankSlot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }
}

/// Authoring schema for a machine tank (in `*.buildings.json`).
#[derive(Serialize, Deserialize, Clone)]
pub struct TankDoc {
    pub id: String,
    #[serde(default = "default_tank_capacity")]
    pub capacity: f32,
    #[serde(default)]
    pub accepts: Vec<String>,
}

fn default_tank_capacity() -> f32 {
    100.0
}

/// Fluid a single pipe segment holds (its implicit tank capacity).
pub const PIPE_CAPACITY: f32 = 10.0;

// ---- save/load (registered with flux_core::save) ----------------------------

#[derive(Serialize, Deserialize)]
struct SavedTank {
    slots: Vec<(String, String, f32, f32, Vec<String>)>,
}

pub(crate) fn save_tank(world: &World, id: InstanceId) -> Option<serde_json::Value> {
    let tank = world.component::<Tank>(id)?;
    let saved = SavedTank {
        slots: tank
            .slots
            .iter()
            .map(|s| (s.id.clone(), s.fluid.clone(), s.volume, s.capacity, s.accepts.clone()))
            .collect(),
    };
    serde_json::to_value(saved).ok()
}

pub(crate) fn load_tank(world: &mut World, id: InstanceId, value: &serde_json::Value) {
    if let Ok(s) = serde_json::from_value::<SavedTank>(value.clone()) {
        let slots = s
            .slots
            .into_iter()
            .map(|(id, fluid, volume, capacity, accepts)| TankSlot { id, fluid, volume, capacity, accepts })
            .collect();
        world.set_component::<Tank>(id, Tank { slots });
    }
}

// ---------------------------------------------------------------------------
// The pipe-network simulation
// ---------------------------------------------------------------------------

/// Fixed simulation tick (seconds) — deterministic regardless of frame rate.
pub const TICK: f32 = 0.1;

#[derive(Default)]
pub struct FluidSystem {
    buildings: BuildingCatalogCache,
    tilesets: flux_core::tilemap::TileSetCache,
    acc: f32,
}

impl flux_runtime::System for FluidSystem {
    fn step(&mut self, world: &mut World, root: &Path, dt: f32) {
        self.acc += dt;
        while self.acc >= TICK {
            self.acc -= TICK;
            tick(world, &mut self.buildings, &mut self.tilesets, root);
        }
    }
}

fn text(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(Value::String(s)) | Some(Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

fn cell_of(world: &World, id: InstanceId) -> (i32, i32) {
    match world.get_prop(id, "Cell") {
        Some(Value::Vec2(v)) => (v.x as i32, v.y as i32),
        _ => (0, 0),
    }
}

/// One fluid port attached to a network or direct link.
struct Attached {
    building: InstanceId,
    port: ResolvedPort,
}

/// Advance every tilemap's fluid networks by one fixed tick.
pub fn tick(
    world: &mut World,
    buildings: &mut BuildingCatalogCache,
    tilesets: &mut flux_core::tilemap::TileSetCache,
    root: &Path,
) {
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

        // Pumps: source their fluid while on/next to their source terrain.
        let ts_path = match world.get_prop(tm, "TileSet") {
            Some(Value::Asset(s)) => s.clone(),
            _ => String::new(),
        };
        let tileset = tilesets.get(&ts_path, root);
        for &b in &ids {
            let Some(def) = cat.get(&text(world, b, "Type")) else { continue };
            let Some(pd) = &def.pump else { continue };
            let (c, r) = cell_of(world, b);
            let (w, h) = (def.width as i32, def.height as i32);
            let near_source = tileset
                .as_deref()
                .and_then(|ts| ts.index_of(&pd.source_tile))
                .is_some_and(|want| {
                    world.tile_grid(tm).is_some_and(|g| {
                        (c - 1..=c + w).any(|x| {
                            (r - 1..=r + h).any(|y| g.cell(x, y).is_some_and(|cl| cl.tile == want))
                        })
                    })
                });
            if near_source {
                let filled = world
                    .component_mut::<Tank>(b)
                    .and_then(|t| t.slot_mut(&pd.tank).map(|s| s.fill(&pd.fluid, pd.rate * TICK)))
                    .unwrap_or(0.0);
                let status = if filled > 0.0 { "" } else { "Tank full" };
                if text(world, b, "_Status") != status {
                    let _ = world.set_prop(b, "_Status", Value::String(status.into()));
                }
                crate::factory::apply_state(world, b, if filled > 0.0 { "working" } else { "idle" });
            } else {
                if text(world, b, "_Status") != "No water source" {
                    let _ = world.set_prop(b, "_Status", Value::String("No water source".into()));
                }
                crate::factory::apply_state(world, b, "starved");
            }
        }

        // Pipes sorted by cell for deterministic iteration; ensure their
        // implicit tank exists (loaded worlds may lack the component only if
        // never saved with fluid — place() attaches it).
        let mut pipes: Vec<(InstanceId, (i32, i32))> = ids
            .iter()
            .copied()
            .filter(|&b| cat.get(&text(world, b, "Type")).is_some_and(|d| d.pipe))
            .map(|b| (b, cell_of(world, b)))
            .collect();
        pipes.sort_by_key(|&(_, c)| c);
        for &(p, _) in &pipes {
            if world.component::<Tank>(p).is_none() {
                world.set_component::<Tank>(
                    p,
                    Tank {
                        slots: vec![TankSlot {
                            id: "pipe".into(),
                            fluid: String::new(),
                            volume: 0.0,
                            capacity: PIPE_CAPACITY,
                            accepts: Vec::new(),
                        }],
                    },
                );
            }
        }
        let pipe_index: HashMap<(i32, i32), usize> =
            pipes.iter().enumerate().map(|(i, &(_, c))| (c, i)).collect();

        // Flood-fill pipes into networks (indices into `pipes`).
        let mut group = vec![usize::MAX; pipes.len()];
        let mut n_groups = 0usize;
        for start in 0..pipes.len() {
            if group[start] != usize::MAX {
                continue;
            }
            let g = n_groups;
            n_groups += 1;
            let mut stack = vec![start];
            group[start] = g;
            while let Some(i) = stack.pop() {
                let (c, r) = pipes[i].1;
                for (dc, dr) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                    if let Some(&j) = pipe_index.get(&(c + dc, r + dr)) {
                        if group[j] == usize::MAX {
                            group[j] = g;
                            stack.push(j);
                        }
                    }
                }
            }
        }

        // Fluid ports, attached to the network whose pipe they face (if any).
        let mut attached: Vec<Vec<Attached>> = (0..n_groups).map(|_| Vec::new()).collect();
        let mut loose: Vec<Attached> = Vec::new(); // for direct port-to-port links
        for &b in &ids {
            let Some(baked) = crate::ports::of(world, b) else { continue };
            for rp in &baked.0 {
                if !rp.port.kind.is_fluid() {
                    continue;
                }
                let a = Attached { building: b, port: rp.clone() };
                if let Some(&pi) = pipe_index.get(&rp.facing) {
                    attached[group[pi]].push(a);
                } else {
                    loose.push(a);
                }
            }
        }
        for list in &mut attached {
            list.sort_by_key(|a| (a.port.cell, a.port.port.id.clone()));
        }

        // Direct machine-to-machine links: an output port and an input port
        // facing each other transfer without pipes.
        loose.sort_by_key(|a| (a.port.cell, a.port.port.id.clone()));
        let mut direct: Vec<(usize, usize)> = Vec::new();
        for i in 0..loose.len() {
            for j in 0..loose.len() {
                if i == j {
                    continue;
                }
                let (o, inp) = (&loose[i], &loose[j]);
                if o.port.port.flow.gives_output()
                    && inp.port.port.flow.takes_input()
                    && o.port.facing == inp.port.cell
                    && inp.port.facing == o.port.cell
                    && o.port.port.kind == inp.port.port.kind
                {
                    direct.push((i, j));
                }
            }
        }
        for (oi, ii) in direct {
            let (src_b, src_tank, throughput_o) = {
                let a = &loose[oi];
                (a.building, a.port.port.tank.clone(), a.port.port.throughput)
            };
            let (dst_b, dst_tank, throughput_i) = {
                let a = &loose[ii];
                (a.building, a.port.port.tank.clone(), a.port.port.throughput)
            };
            let budget = throughput_o.min(throughput_i) * TICK;
            let (fluid, avail) = match world.component::<Tank>(src_b).and_then(|t| t.slot(&src_tank)) {
                Some(s) if s.volume > 0.0 => (s.fluid.clone(), s.volume),
                _ => continue,
            };
            let moved = {
                let Some(dst) = world.component_mut::<Tank>(dst_b) else { continue };
                let Some(slot) = dst.slot_mut(&dst_tank) else { continue };
                slot.fill(&fluid, budget.min(avail))
            };
            if moved > 0.0 {
                if let Some(src) = world.component_mut::<Tank>(src_b) {
                    if let Some(slot) = src.slot_mut(&src_tank) {
                        slot.drain(moved);
                    }
                }
            }
        }

        // Per-network: sources push, then consumers pull, then rebalance.
        for g in 0..n_groups {
            let members: Vec<usize> = (0..pipes.len()).filter(|&i| group[i] == g).collect();
            let capacity = members.len() as f32 * PIPE_CAPACITY;
            let (mut volume, mut fluid) = {
                let mut v = 0.0;
                let mut f = String::new();
                for &i in &members {
                    if let Some(t) = world.component::<Tank>(pipes[i].0) {
                        if let Some(s) = t.slot("pipe") {
                            v += s.volume;
                            if f.is_empty() && !s.fluid.is_empty() {
                                f = s.fluid.clone();
                            }
                        }
                    }
                }
                (v, f)
            };

            // Sources: machine output ports feed the network.
            for a in attached[g].iter().filter(|a| a.port.port.flow.gives_output()) {
                let budget = a.port.port.throughput * TICK;
                let Some(tank) = world.component_mut::<Tank>(a.building) else { continue };
                let Some(slot) = tank.slot_mut(&a.port.port.tank) else { continue };
                if slot.volume <= 0.0 {
                    continue;
                }
                // The network carries exactly one fluid at a time.
                if !fluid.is_empty() && slot.fluid != fluid {
                    continue; // incompatible source: blocked
                }
                let space = capacity - volume;
                let take = budget.min(slot.volume).min(space);
                if take > 0.0 {
                    fluid = slot.fluid.clone();
                    slot.drain(take);
                    volume += take;
                }
            }

            // Consumers: machine input ports drain the network.
            if !fluid.is_empty() {
                for a in attached[g].iter().filter(|a| a.port.port.flow.takes_input()) {
                    if volume <= 0.0 {
                        break;
                    }
                    let budget = a.port.port.throughput * TICK;
                    let Some(tank) = world.component_mut::<Tank>(a.building) else { continue };
                    let Some(slot) = tank.slot_mut(&a.port.port.tank) else { continue };
                    let moved = slot.fill(&fluid, budget.min(volume));
                    volume -= moved;
                }
            }

            // Rebalance the remaining volume across member pipes (visual fill
            // + persistence); exact conservation via last-gets-remainder.
            if !members.is_empty() {
                let share = volume / members.len() as f32;
                let mut assigned = 0.0;
                let last = members.len() - 1;
                for (k, &i) in members.iter().enumerate() {
                    let v = if k == last { volume - assigned } else { share };
                    assigned += v;
                    if let Some(t) = world.component_mut::<Tank>(pipes[i].0) {
                        if let Some(s) = t.slot_mut("pipe") {
                            s.volume = v.max(0.0);
                            s.fluid = if s.volume > 0.0 { fluid.clone() } else { String::new() };
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tank_slot_fill_drain_and_locking() {
        let mut s = TankSlot {
            id: "t".into(),
            fluid: String::new(),
            volume: 0.0,
            capacity: 10.0,
            accepts: vec!["water".into()],
        };
        assert_eq!(s.fill("steam", 5.0), 0.0, "accepts list enforced");
        assert_eq!(s.fill("water", 6.0), 6.0);
        assert_eq!(s.fill("water", 6.0), 4.0, "capacity clamps");
        assert_eq!(s.drain(100.0), 10.0);
        assert!(s.fluid.is_empty(), "empty slot forgets its fluid");
    }

    #[test]
    fn fluid_catalog_parses() {
        let cat = FluidCatalog::parse(
            r#"{ "fluids": [ { "id": "water", "name": "Water", "color": [0.2,0.4,0.8,1] } ] }"#,
        )
        .unwrap();
        assert!(cat.get("water").is_some());
        assert_eq!(cat.all().len(), 1);
    }
}
