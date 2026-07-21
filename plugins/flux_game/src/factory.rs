//! Production + logistics simulation for buildings (mining, crafting, transport).
//!
//! Building inventories live in the engine's generic per-instance component
//! store (`World::component::<Inventory>`); this plugin registers their save
//! (de)serialization. The [`FactorySystem`] is registered with the runtime and
//! stepped every frame.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use flux_core::tilemap::{TileSet, TileSetCache};
use flux_core::{InstanceId, Value, World};

use crate::building::{BuildingCatalog, BuildingCatalogCache};

// ---------------------------------------------------------------------------
// Inventory (a plugin component)
// ---------------------------------------------------------------------------

/// A building's item buffer: item id -> count, with an optional capacity
/// (0 = unlimited). Kept ordered so saves are stable.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Inventory {
    items: IndexMap<String, u32>,
    capacity: u32,
}

impl Inventory {
    pub fn new(capacity: u32) -> Self {
        Self {
            items: IndexMap::new(),
            capacity,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn total(&self) -> u32 {
        self.items.values().sum()
    }

    pub fn free(&self) -> u32 {
        if self.capacity == 0 {
            u32::MAX
        } else {
            self.capacity.saturating_sub(self.total())
        }
    }

    pub fn count(&self, item: &str) -> u32 {
        self.items.get(item).copied().unwrap_or(0)
    }

    pub fn add(&mut self, item: &str, n: u32) -> u32 {
        let added = n.min(self.free());
        if added > 0 {
            *self.items.entry(item.to_string()).or_insert(0) += added;
        }
        added
    }

    pub fn take(&mut self, item: &str, n: u32) -> u32 {
        let have = self.count(item);
        let taken = n.min(have);
        if taken > 0 {
            let remaining = have - taken;
            if remaining == 0 {
                self.items.shift_remove(item);
            } else {
                self.items[item] = remaining;
            }
        }
        taken
    }

    pub fn has(&self, item: &str, n: u32) -> bool {
        self.count(item) >= n
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, u32)> {
        self.items.iter().map(|(k, v)| (k.as_str(), *v))
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn from_pairs(capacity: u32, pairs: impl IntoIterator<Item = (String, u32)>) -> Self {
        let mut inv = Inventory::new(capacity);
        for (k, v) in pairs {
            if v > 0 {
                inv.items.insert(k, v);
            }
        }
        inv
    }
}

// ---- save/load (registered with flux_core::save) ----------------------------

#[derive(Serialize, Deserialize)]
struct SavedInventory {
    cap: u32,
    #[serde(default)]
    items: Vec<(String, u32)>,
}

pub(crate) fn save_inventory(world: &World, id: InstanceId) -> Option<serde_json::Value> {
    let inv = world.component::<Inventory>(id)?;
    let saved = SavedInventory {
        cap: inv.capacity(),
        items: inv.iter().map(|(k, v)| (k.to_string(), v)).collect(),
    };
    serde_json::to_value(saved).ok()
}

pub(crate) fn load_inventory(world: &mut World, id: InstanceId, value: &serde_json::Value) {
    if let Ok(s) = serde_json::from_value::<SavedInventory>(value.clone()) {
        world.set_component::<Inventory>(id, Inventory::from_pairs(s.cap, s.items));
    }
}

// ---------------------------------------------------------------------------
// Recipe catalog (`*.recipes.json`)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct RecipeCatalogDoc {
    #[serde(default)]
    pub recipes: Vec<RecipeDoc>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RecipeDoc {
    pub id: String,
    #[serde(default = "one_f")]
    pub time: f32,
    #[serde(default)]
    pub inputs: Vec<Stack>,
    #[serde(default)]
    pub outputs: Vec<Stack>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Stack {
    pub item: String,
    #[serde(default = "one_u")]
    pub count: u32,
}

fn one_f() -> f32 {
    1.0
}
fn one_u() -> u32 {
    1
}

pub struct Recipe {
    pub time: f32,
    pub inputs: Vec<(String, u32)>,
    pub outputs: Vec<(String, u32)>,
}

pub struct RecipeCatalog {
    by_id: HashMap<String, Recipe>,
}

impl RecipeCatalog {
    pub fn parse(json: &str) -> Result<Self, String> {
        let doc: RecipeCatalogDoc = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let by_id = doc
            .recipes
            .into_iter()
            .map(|r| {
                let recipe = Recipe {
                    time: r.time.max(0.01),
                    inputs: r.inputs.into_iter().map(|s| (s.item, s.count.max(1))).collect(),
                    outputs: r.outputs.into_iter().map(|s| (s.item, s.count.max(1))).collect(),
                };
                (r.id, recipe)
            })
            .collect();
        Ok(RecipeCatalog { by_id })
    }

    pub fn get(&self, id: &str) -> Option<&Recipe> {
        self.by_id.get(id)
    }
}

#[derive(Default)]
pub struct RecipeCatalogCache {
    catalogs: HashMap<String, Option<Rc<RecipeCatalog>>>,
}

impl RecipeCatalogCache {
    pub fn get(&mut self, rel: &str, root: &Path) -> Option<Rc<RecipeCatalog>> {
        if rel.is_empty() {
            return None;
        }
        if let Some(v) = self.catalogs.get(rel) {
            return v.clone();
        }
        let loaded = std::fs::read_to_string(root.join(rel))
            .ok()
            .and_then(|text| RecipeCatalog::parse(&text).ok())
            .map(Rc::new);
        self.catalogs.insert(rel.to_string(), loaded.clone());
        loaded
    }
}

// ---------------------------------------------------------------------------
// System + simulation
// ---------------------------------------------------------------------------

/// The runtime system that advances mining, production and transport each frame.
#[derive(Default)]
pub struct FactorySystem {
    tilesets: TileSetCache,
    buildings: BuildingCatalogCache,
    recipes: RecipeCatalogCache,
}

impl flux_runtime::System for FactorySystem {
    fn step(&mut self, world: &mut World, root: &Path, dt: f32) {
        step(world, &mut self.tilesets, &mut self.buildings, &mut self.recipes, root, dt);
    }
}

fn num(world: &World, id: InstanceId, name: &str) -> f32 {
    match world.get_prop(id, name) {
        Some(Value::Number(n)) => *n as f32,
        _ => 0.0,
    }
}

fn text(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn set_num(world: &mut World, id: InstanceId, name: &str, v: f32) {
    let _ = world.set_prop(id, name, Value::Number(v as f64));
}

fn cell(world: &World, id: InstanceId) -> (i32, i32) {
    match world.get_prop(id, "Cell") {
        Some(Value::Vec2(v)) => (v.x as i32, v.y as i32),
        _ => (0, 0),
    }
}

fn footprint(world: &World, id: InstanceId) -> (i32, i32) {
    match world.get_prop(id, "Footprint") {
        Some(Value::Vec2(v)) => ((v.x as i32).max(1), (v.y as i32).max(1)),
        _ => (1, 1),
    }
}

fn ticks(acc: f32, rate: f32, dt: f32) -> (u32, f32) {
    if rate <= 0.0 {
        return (0, 0.0);
    }
    let a = acc + dt * rate;
    let n = a.floor().max(0.0);
    (n as u32, a - n)
}

/// Advance the factory simulation for every tilemap's buildings by `dt`.
pub fn step(
    world: &mut World,
    tilesets: &mut TileSetCache,
    buildings: &mut BuildingCatalogCache,
    recipes: &mut RecipeCatalogCache,
    root: &Path,
    dt: f32,
) {
    let maps: Vec<InstanceId> = world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("Tilemap"))
        .collect();

    for tm in maps {
        let ts_path = asset(world, tm, "TileSet");
        let bc_path = crate::attr_text(world, tm, "Buildings");
        let rc_path = crate::attr_text(world, tm, "Recipes");
        let (Some(bcat), Some(rcat)) = (buildings.get(&bc_path, root), recipes.get(&rc_path, root))
        else {
            continue;
        };
        let tileset = tilesets.get(&ts_path, root);

        let ids: Vec<InstanceId> = world
            .children(tm)
            .iter()
            .copied()
            .filter(|&c| world.class_name(c) == Some("Building"))
            .collect();

        // Age out old flow-visualization events.
        if let Some(log) = world.component_mut::<FlowLog>(tm) {
            for e in &mut log.events {
                e.age += dt;
            }
            log.events.retain(|e| e.age < 0.6);
        }

        for &b in &ids {
            // Ports are derived data: loaded worlds arrive without the baked
            // component, so (re)bake lazily from the catalog.
            if world.component::<crate::ports::BakedPorts>(b).is_none() {
                if let Some(def) = bcat.get(&text(world, b, "Type")) {
                    if !def.ports.is_empty() {
                        crate::ports::bake(world, b, def);
                    }
                }
            }
            let mined = mine(world, tm, b, &bcat, tileset.as_deref(), dt);
            let produced = produce(world, b, &rcat, dt);
            resolve_state(world, b, &bcat, mined, produced, dt);
        }
        let moved = transport(world, tm, &ids, &bcat, &rcat, dt);
        for b in moved {
            // Flow keeps passive buildings (belts, storage) visibly working
            // for a moment; `resolve_state` decays the hold.
            set_num(world, b, "_StateHold", 0.9);
            apply_state(world, b, "working");
        }
    }
}

/// Combine this frame's mine/produce verdicts (`None` = no decision, keep the
/// current state) into the building's `_State`, and keep the sprite's clip in
/// sync. Reactors are skipped — `ReactorSystem` owns their state machine.
fn resolve_state(
    world: &mut World,
    b: InstanceId,
    cat: &BuildingCatalog,
    mined: Option<&'static str>,
    produced: Option<&'static str>,
    dt: f32,
) {
    let Some(def) = cat.get(&text(world, b, "Type")) else {
        return;
    };
    // ReactorSystem owns reactor/cooling-tower/turbine states, FluidSystem
    // owns pumps; pipe sprites are shape-driven (connectivity masks).
    if def.reactor.is_some()
        || def.cooling > 0.0
        || def.pipe
        || def.turbine.is_some()
        || def.pump.is_some()
    {
        return;
    }
    let hold = (num(world, b, "_StateHold") - dt).max(0.0);
    set_num(world, b, "_StateHold", hold);

    if let Some(state) = mined.or(produced) {
        apply_state(world, b, state);
    } else if !def.mines && def.recipe.is_none() && hold <= 0.0 {
        apply_state(world, b, "idle");
    }
}

/// Publish `_State` and switch the child sprite's animation clip on change.
pub(crate) fn apply_state(world: &mut World, b: InstanceId, state: &str) {
    if text(world, b, "_State") == state {
        return;
    }
    let _ = world.set_prop(b, "_State", flux_core::Value::String(state.to_string()));
    if let Some(sprite) = crate::building::sprite_of(world, b) {
        flux_core::animation::play(world, sprite, state, false);
    }
}

fn asset(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Returns this tick's verdict (`working`/`starved`), or `None` between ticks
/// (or for non-miners) so the previous state persists.
fn mine(
    world: &mut World,
    tilemap: InstanceId,
    b: InstanceId,
    cat: &BuildingCatalog,
    tileset: Option<&TileSet>,
    dt: f32,
) -> Option<&'static str> {
    let def = cat.get(&text(world, b, "Type"))?;
    if !def.mines {
        return None;
    }
    let (n, acc) = ticks(num(world, b, "_MineT"), def.rate, dt);
    set_num(world, b, "_MineT", acc);
    if n == 0 {
        return None;
    }
    let (col, row) = cell(world, b);
    // No deposit under the drill: starved.
    let Some(ore_idx) = world.tile_grid(tilemap).and_then(|g| {
        let c = g.cell(col, row)?;
        c.has_ore().then_some(c.ore)
    }) else {
        return Some("starved");
    };
    let Some(ore_id) = tileset.and_then(|ts| ts.tile(ore_idx).map(|t| t.id.clone())) else {
        return Some("starved");
    };
    let free = world.component::<Inventory>(b).map(|i| i.free()).unwrap_or(0);
    let want = n.min(free);
    if want == 0 {
        return Some("starved"); // output buffer full
    }
    let mined = world
        .tile_grid_mut(tilemap)
        .map(|g| g.mine(col, row, want.min(u16::MAX as u32) as u16))
        .unwrap_or(0) as u32;
    if mined > 0 {
        if let Some(inv) = world.component_mut::<Inventory>(b) {
            inv.add(&ore_id, mined);
        }
        Some("working")
    } else {
        Some("starved") // deposit exhausted
    }
}

/// Returns the producer's verdict, or `None` for buildings without a recipe.
fn produce(world: &mut World, b: InstanceId, recipes: &RecipeCatalog, dt: f32) -> Option<&'static str> {
    let recipe_id = text(world, b, "Recipe");
    if recipe_id.is_empty() {
        return None;
    }
    let recipe = recipes.get(&recipe_id)?;

    let timer = num(world, b, "_Timer");
    if timer > 0.0 {
        let t = timer - dt;
        if t > 0.0 {
            set_num(world, b, "_Timer", t);
            return Some("working");
        }
        set_num(world, b, "_Timer", 0.0);
        if let Some(inv) = world.component_mut::<Inventory>(b) {
            for (item, count) in &recipe.outputs {
                inv.add(item, *count);
            }
        }
        return Some("working");
    }

    let inv = world.component::<Inventory>(b)?;
    let inputs_ready = recipe.inputs.iter().all(|(i, c)| inv.has(i, *c));
    let out_total: u32 = recipe.outputs.iter().map(|(_, c)| c).sum();
    let in_total: u32 = recipe.inputs.iter().map(|(_, c)| c).sum();
    if !inputs_ready || inv.free() + in_total < out_total {
        return Some("starved"); // missing inputs, or output blocked
    }
    if let Some(inv) = world.component_mut::<Inventory>(b) {
        for (item, count) in &recipe.inputs {
            inv.take(item, *count);
        }
    }
    set_num(world, b, "_Timer", recipe.time);
    Some("working")
}

/// A recent item hop between two buildings, for the flow-visualization
/// overlay. Positions are absolute world-space footprint centres.
pub struct FlowEvent {
    pub from: glam::Vec2,
    pub to: glam::Vec2,
    pub item: String,
    pub age: f32,
}

/// Transient per-tilemap log of recent hops (never saved: not registered with
/// the component save hook).
#[derive(Default)]
pub struct FlowLog {
    pub events: Vec<FlowEvent>,
}

/// The cells a directional building's front edge points at.
fn front_cells(world: &World, b: InstanceId) -> Vec<(i32, i32)> {
    let (c, r) = cell(world, b);
    let (w, h) = footprint(world, b);
    let dir = num(world, b, "Direction") as u8;
    crate::building::front_cells_of(c, r, w, h, dir)
}

/// Absolute world-space centre of a building's footprint on its tilemap.
fn world_centre(world: &World, tilemap: InstanceId, b: InstanceId) -> glam::Vec2 {
    let map_pos = match world.get_prop(tilemap, "Position") {
        Some(Value::Vec2(p)) => *p,
        _ => glam::Vec2::ZERO,
    };
    let dim = |name, d| match world.get_prop(tilemap, name) {
        Some(Value::Number(n)) => *n as f32,
        _ => d,
    };
    let (tw, th) = (dim("TileWidth", 64.0), dim("TileHeight", 32.0));
    let (c, r) = cell(world, b);
    let (w, h) = footprint(world, b);
    let cf = c as f32 + (w as f32 - 1.0) * 0.5;
    let rf = r as f32 + (h as f32 - 1.0) * 0.5;
    map_pos + glam::Vec2::new((cf - rf) * tw * 0.5, (cf + rf) * th * 0.5)
}

/// Returns every building that sent or received an item this frame.
///
/// Logistics require conveyors: producers only hand items to adjacent belts,
/// belts push along their `Direction` into whatever they point at (another
/// belt, or a consumer that accepts the item), and stores/consumers never give.
fn transport(
    world: &mut World,
    tilemap: InstanceId,
    ids: &[InstanceId],
    cat: &BuildingCatalog,
    recipes: &RecipeCatalog,
    dt: f32,
) -> Vec<InstanceId> {
    let mut occupancy: HashMap<(i32, i32), InstanceId> = HashMap::new();
    for &b in ids {
        let (c, r) = cell(world, b);
        let (w, h) = footprint(world, b);
        for dc in 0..w {
            for dr in 0..h {
                occupancy.insert((c + dc, r + dr), b);
            }
        }
    }

    // Connection limits: rank the belts feeding each limited port cell so a
    // single-connection port only ever accepts from its first (deterministic)
    // feeder. Keyed by (machine, port cell); feeders sorted by belt cell.
    let mut feeders: HashMap<(InstanceId, (i32, i32)), Vec<InstanceId>> = HashMap::new();
    for &b in ids {
        let Some(def) = cat.get(&text(world, b, "Type")) else {
            continue;
        };
        if !def.directional {
            continue;
        }
        for fc in front_cells(world, b) {
            if let Some(&t) = occupancy.get(&fc) {
                if t != b && crate::ports::of(world, t).is_some() {
                    feeders.entry((t, fc)).or_default().push(b);
                }
            }
        }
    }
    for list in feeders.values_mut() {
        list.sort_by_key(|&b| cell(world, b));
    }
    // Whether `belt` may deliver `item` into machine `t` at `fc` — port rules:
    // an item INPUT port on that cell, resource accepted, connection slot free.
    // Machines without ports keep legacy open-sided behaviour (`None`).
    let port_gate = |world: &World, t: InstanceId, fc: (i32, i32), belt: InstanceId, item: &str| -> Option<bool> {
        let baked = crate::ports::of(world, t)?;
        let Some(rp) = baked.0.iter().find(|rp| {
            rp.cell == fc && rp.port.kind == crate::ports::PortKind::Item && rp.port.flow.takes_input()
        }) else {
            return Some(false); // has ports, but no item input on that cell
        };
        if !rp.port.accepts_resource(item) {
            return Some(false);
        }
        if rp.port.limit > 0 {
            if let Some(list) = feeders.get(&(t, fc)) {
                let rank = list.iter().position(|&x| x == belt).unwrap_or(usize::MAX);
                if rank >= rp.port.limit as usize {
                    return Some(false);
                }
            }
        }
        Some(true)
    };

    let mut moves: Vec<(InstanceId, InstanceId, String, u32)> = Vec::new();
    for &b in ids {
        let Some(def) = cat.get(&text(world, b, "Type")) else {
            continue;
        };
        // Belts, producers, and machines with item OUTPUT ports emit items
        // (e.g. a reactor pushing spent fuel); stores/consumers just hold.
        let is_producer = def.mines || def.recipe.is_some();
        let out_port_accepts: Option<Vec<String>> = crate::ports::of(world, b).map(|baked| {
            baked
                .0
                .iter()
                .filter(|rp| rp.port.kind == crate::ports::PortKind::Item && rp.port.flow.gives_output())
                .flat_map(|rp| rp.port.accepts.clone())
                .collect()
        });
        let has_item_out = out_port_accepts.is_some()
            && crate::ports::of(world, b).is_some_and(|baked| {
                baked.0.iter().any(|rp| {
                    rp.port.kind == crate::ports::PortKind::Item && rp.port.flow.gives_output()
                })
            });
        if !def.directional && !is_producer && !has_item_out {
            continue;
        }
        let (n, acc) = ticks(num(world, b, "_Flow"), def.rate.max(1.0), dt);
        set_num(world, b, "_Flow", acc);
        if n == 0 {
            continue;
        }
        let Some(inv) = world.component::<Inventory>(b) else {
            continue;
        };
        let my_recipe = text(world, b, "Recipe");
        let my_inputs: Vec<&str> = recipes
            .get(&my_recipe)
            .map(|rc| rc.inputs.iter().map(|(i, _)| i.as_str()).collect())
            .unwrap_or_default();
        let givable: Vec<(String, u32)> = inv
            .iter()
            .filter(|(id, _)| !my_inputs.contains(id))
            .filter(|(id, _)| match &out_port_accepts {
                // A ports-machine only ships what its output ports declare
                // (a reactor gives waste, never its uranium fuel). An empty
                // accepts list on an out port means "anything non-input".
                Some(accepts) if !def.directional && !def.mines && def.recipe.is_none() => {
                    accepts.is_empty() || accepts.iter().any(|a| a == id)
                }
                _ => true,
            })
            .map(|(id, c)| (id.to_string(), c))
            .collect();
        if givable.is_empty() {
            continue;
        }
        let (c, r) = cell(world, b);
        let (w, h) = footprint(world, b);
        // Targets carry the cell they're reached through, so port rules can
        // check the exact boundary cell (None = belt-to-belt / legacy path).
        let targets: Vec<(InstanceId, Option<(i32, i32)>)> = if def.directional {
            // A belt feeds exactly the building its front edge points at.
            let mut out = Vec::new();
            for fc in front_cells(world, b) {
                if let Some(&t) = occupancy.get(&fc) {
                    if t != b && !out.iter().any(|(o, _)| *o == t) {
                        out.push((t, Some(fc)));
                    }
                }
            }
            out
        } else {
            // Producers dump onto adjacent conveyors — restricted to their
            // item OUTPUT ports when they declare ports, and never onto a
            // conveyor that points back INTO them (an inescapable trap).
            let out_cells: Option<Vec<(i32, i32)>> = crate::ports::of(world, b).map(|baked| {
                baked
                    .0
                    .iter()
                    .filter(|rp| rp.port.kind == crate::ports::PortKind::Item && rp.port.flow.gives_output())
                    .map(|rp| rp.facing)
                    .collect()
            });
            edge_neighbours(&occupancy, b, c, r, w, h)
                .into_iter()
                .filter(|&nb| {
                    cat.get(&text(world, nb, "Type")).is_some_and(|d| d.directional)
                        && !front_cells(world, nb)
                            .iter()
                            .any(|cell| occupancy.get(cell) == Some(&b))
                        && match &out_cells {
                            // With ports: the belt must sit on an output port's
                            // facing cell.
                            Some(cells) => {
                                let (nc, nr) = cell(world, nb);
                                let (nw, nh) = footprint(world, nb);
                                cells.iter().any(|&(cx, cy)| {
                                    cx >= nc && cx < nc + nw && cy >= nr && cy < nr + nh
                                })
                            }
                            None => true, // legacy: any adjacent belt
                        }
                })
                .map(|nb| (nb, None))
                .collect()
        };

        let mut budget = n;
        for (item, _have) in givable {
            if budget == 0 {
                break;
            }
            for &(nb, via_cell) in &targets {
                if budget == 0 {
                    break;
                }
                // Port rules first (belt -> ports machine); legacy accepts()
                // covers belts, stores and port-less machines.
                let allowed = match via_cell.and_then(|fc| port_gate(world, nb, fc, b, &item)) {
                    Some(v) => v,
                    None => accepts(world, nb, &item, cat, recipes),
                };
                if !allowed {
                    continue;
                }
                let room = world.component::<Inventory>(nb).map(|i| i.free()).unwrap_or(0);
                let have = world.component::<Inventory>(b).map(|i| i.count(&item)).unwrap_or(0);
                let send = budget.min(room).min(have);
                if send > 0 {
                    moves.push((b, nb, item.clone(), send));
                    budget -= send;
                }
            }
        }
    }

    let mut movers = Vec::new();
    for (from, to, item, n) in moves {
        let taken = world.component_mut::<Inventory>(from).map(|i| i.take(&item, n)).unwrap_or(0);
        if taken > 0 {
            let added = world.component_mut::<Inventory>(to).map(|i| i.add(&item, taken)).unwrap_or(0);
            if added < taken {
                if let Some(inv) = world.component_mut::<Inventory>(from) {
                    inv.add(&item, taken - added);
                }
            }
            if added > 0 {
                movers.push(from);
                movers.push(to);
                let (fp, tp) = (world_centre(world, tilemap, from), world_centre(world, tilemap, to));
                if world.component::<FlowLog>(tilemap).is_none() {
                    world.set_component::<FlowLog>(tilemap, FlowLog::default());
                }
                if let Some(log) = world.component_mut::<FlowLog>(tilemap) {
                    log.events.push(FlowEvent { from: fp, to: tp, item, age: 0.0 });
                }
            }
        }
    }
    movers
}

fn edge_neighbours(
    occupancy: &HashMap<(i32, i32), InstanceId>,
    b: InstanceId,
    c: i32,
    r: i32,
    w: i32,
    h: i32,
) -> Vec<InstanceId> {
    let mut out: Vec<InstanceId> = Vec::new();
    let mut push = |cell: (i32, i32)| {
        if let Some(&other) = occupancy.get(&cell) {
            if other != b && !out.contains(&other) {
                out.push(other);
            }
        }
    };
    for dc in 0..w {
        push((c + dc, r - 1));
        push((c + dc, r + h));
    }
    for dr in 0..h {
        push((c - 1, r + dr));
        push((c + w, r + dr));
    }
    out
}

fn accepts(
    world: &World,
    nb: InstanceId,
    item: &str,
    cat: &BuildingCatalog,
    recipes: &RecipeCatalog,
) -> bool {
    let Some(def) = cat.get(&text(world, nb, "Type")) else {
        return false;
    };
    // Conveyors relay anything (space permitting).
    if def.directional {
        return true;
    }
    if def.stores {
        return true;
    }
    if let Some(rp) = &def.reactor {
        if !rp.fuel_item.is_empty() && rp.fuel_item == item {
            return true;
        }
    }
    let rid = text(world, nb, "Recipe");
    recipes.get(&rid).is_some_and(|r| r.inputs.iter().any(|(i, _)| i == item))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_add_take_capacity() {
        let mut inv = Inventory::new(10);
        assert_eq!(inv.add("ore", 6), 6);
        assert_eq!(inv.add("ore", 8), 4);
        assert_eq!(inv.total(), 10);
        assert_eq!(inv.take("ore", 3), 3);
        assert_eq!(inv.count("ore"), 7);
        assert_eq!(inv.take("ore", 100), 7);
        assert!(inv.is_empty());
        assert_eq!(Inventory::new(0).add("x", 1_000_000), 1_000_000);
    }

    #[test]
    fn recipe_catalog_parses() {
        let json = r#"{ "recipes": [
            { "id": "plate", "time": 2.0, "inputs": [{"item":"ore","count":1}],
              "outputs": [{"item":"plate"}] }
        ]}"#;
        let cat = RecipeCatalog::parse(json).unwrap();
        let r = cat.get("plate").unwrap();
        assert_eq!(r.time, 2.0);
        assert_eq!(r.inputs, vec![("ore".to_string(), 1)]);
        assert_eq!(r.outputs, vec![("plate".to_string(), 1)]);
    }

    #[test]
    fn ticks_accumulate_by_rate() {
        let (n1, acc1) = ticks(0.0, 2.0, 0.4);
        assert_eq!(n1, 0);
        let (n2, _) = ticks(acc1, 2.0, 0.4);
        assert_eq!(n2, 1);
    }
}
