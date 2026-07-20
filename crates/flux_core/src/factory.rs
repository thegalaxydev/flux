//! Production + logistics simulation for buildings placed on a [`crate::tilemap`].
//!
//! Three data-driven mechanics, advanced by [`step`] each frame:
//!
//! * **Mining** — a building whose [`crate::building::BuildingDef::mines`] is set
//!   pulls ore from the tile beneath it (via the tilemap grid) into its
//!   inventory, at its `rate`.
//! * **Production** — a building with a `Recipe` (looked up in a `*.recipes.json`
//!   catalog) consumes its inputs and, after the recipe's `time`, yields its
//!   outputs — all through the building's [`Inventory`].
//! * **Transport** — each building pushes items an adjacent building wants (a
//!   recipe input, or anything for a `stores` sink) into that neighbour.
//!
//! Inventories live in a transient [`crate::World`] side-table (saved with the
//! game). Recipes and items are just string ids, so nothing here is hardcoded.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::building::{BuildingCatalog, BuildingCatalogCache};
use crate::tilemap::{TileSet, TileSetCache};
use crate::value::Value;
use crate::world::{InstanceId, World};

// ---------------------------------------------------------------------------
// Inventory
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

    /// Free space, or `u32::MAX` when unlimited.
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

    /// Add up to `n`, honoring capacity. Returns how many were added.
    pub fn add(&mut self, item: &str, n: u32) -> u32 {
        let added = n.min(self.free());
        if added > 0 {
            *self.items.entry(item.to_string()).or_insert(0) += added;
        }
        added
    }

    /// Remove up to `n`. Returns how many were removed.
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

    /// Whether at least `n` of `item` are present.
    pub fn has(&self, item: &str, n: u32) -> bool {
        self.count(item) >= n
    }

    /// (item, count) pairs, in insertion order — for saving and UI.
    pub fn iter(&self) -> impl Iterator<Item = (&str, u32)> {
        self.items.iter().map(|(k, v)| (k.as_str(), *v))
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Rebuild from saved `(item, count)` pairs (used by save-load).
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
    pub id: String,
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
                    id: r.id.clone(),
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

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Loads and caches `*.recipes.json` catalogs, mirroring the other asset caches.
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

    pub fn clear(&mut self) {
        self.catalogs.clear();
    }
}

// ---------------------------------------------------------------------------
// Simulation
// ---------------------------------------------------------------------------

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

/// How many whole actions fire this frame given an accumulator, rate and dt;
/// returns `(count, new_accumulator)`.
fn ticks(acc: f32, rate: f32, dt: f32) -> (u32, f32) {
    if rate <= 0.0 {
        return (0, 0.0);
    }
    let a = acc + dt * rate;
    let n = a.floor().max(0.0);
    (n as u32, a - n)
}

/// Advance the factory simulation for every tilemap's buildings by `dt`.
/// Resolves each tilemap's TileSet (for ore ids), building catalog and recipe
/// catalog through the caches (loaded once, reused).
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
        let bc_path = asset(world, tm, "Buildings");
        let rc_path = asset(world, tm, "Recipes");
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

        for &b in &ids {
            mine(world, tm, b, &bcat, tileset.as_deref(), dt);
            produce(world, b, &bcat, &rcat, dt);
        }
        transport(world, tm, &ids, &bcat, &rcat, dt);
    }
}

fn asset(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Extract ore from under a miner into its inventory.
fn mine(
    world: &mut World,
    tilemap: InstanceId,
    b: InstanceId,
    cat: &BuildingCatalog,
    tileset: Option<&TileSet>,
    dt: f32,
) {
    let Some(def) = cat.get(&text(world, b, "Type")) else {
        return;
    };
    if !def.mines {
        return;
    }
    let (n, acc) = ticks(num(world, b, "_MineT"), def.rate, dt);
    set_num(world, b, "_MineT", acc);
    if n == 0 {
        return;
    }
    let (col, row) = cell(world, b);
    // Which ore sits under the miner's origin cell?
    let Some(ore_idx) = world.tile_grid(tilemap).and_then(|g| {
        let c = g.cell(col, row)?;
        c.has_ore().then_some(c.ore)
    }) else {
        return;
    };
    let Some(ore_id) = tileset.and_then(|ts| ts.tile(ore_idx).map(|t| t.id.clone())) else {
        return;
    };
    let free = world.inventory(b).map(|i| i.free()).unwrap_or(0);
    let want = n.min(free);
    if want == 0 {
        return;
    }
    let mined = world
        .tile_grid_mut(tilemap)
        .map(|g| g.mine(col, row, want.min(u16::MAX as u32) as u16))
        .unwrap_or(0) as u32;
    if mined > 0 {
        if let Some(inv) = world.inventory_mut(b) {
            inv.add(&ore_id, mined);
        }
    }
}

/// Run a producer's recipe: consume inputs, then yield outputs after `time`.
fn produce(
    world: &mut World,
    b: InstanceId,
    cat: &BuildingCatalog,
    recipes: &RecipeCatalog,
    dt: f32,
) {
    let recipe_id = text(world, b, "Recipe");
    if recipe_id.is_empty() {
        return;
    }
    let Some(recipe) = recipes.get(&recipe_id) else {
        return;
    };
    let _ = cat; // def not needed here; kept for symmetry/future speed mults.

    let timer = num(world, b, "_Timer");
    if timer > 0.0 {
        // Crafting in progress (inputs already consumed).
        let t = timer - dt;
        if t > 0.0 {
            set_num(world, b, "_Timer", t);
            return;
        }
        set_num(world, b, "_Timer", 0.0);
        if let Some(inv) = world.inventory_mut(b) {
            for (item, count) in &recipe.outputs {
                inv.add(item, *count);
            }
        }
        return;
    }

    // Idle: start a craft if inputs are present and there's room for outputs.
    let Some(inv) = world.inventory(b) else {
        return;
    };
    let inputs_ready = recipe.inputs.iter().all(|(i, c)| inv.has(i, *c));
    let out_total: u32 = recipe.outputs.iter().map(|(_, c)| c).sum();
    let in_total: u32 = recipe.inputs.iter().map(|(_, c)| c).sum();
    // Enough room once inputs are consumed.
    if !inputs_ready || inv.free() + in_total < out_total {
        return;
    }
    if let Some(inv) = world.inventory_mut(b) {
        for (item, count) in &recipe.inputs {
            inv.take(item, *count);
        }
    }
    set_num(world, b, "_Timer", recipe.time);
}

/// Push each building's spare items to an adjacent building that wants them.
fn transport(
    world: &mut World,
    _tilemap: InstanceId,
    ids: &[InstanceId],
    cat: &BuildingCatalog,
    recipes: &RecipeCatalog,
    dt: f32,
) {
    // Index buildings by every cell they occupy, for O(1) neighbour lookup.
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

    // Plan moves first (can't hold two inventory borrows at once), then apply.
    let mut moves: Vec<(InstanceId, InstanceId, String, u32)> = Vec::new();
    for &b in ids {
        let def_rate = cat.get(&text(world, b, "Type")).map(|d| d.rate).unwrap_or(1.0);
        let (n, acc) = ticks(num(world, b, "_Flow"), def_rate.max(1.0), dt);
        set_num(world, b, "_Flow", acc);
        if n == 0 {
            continue;
        }
        let Some(inv) = world.inventory(b) else {
            continue;
        };
        // Items this building is willing to give away: everything it isn't
        // itself consuming as a recipe input.
        let my_recipe = text(world, b, "Recipe");
        let my_inputs: Vec<&str> = recipes
            .get(&my_recipe)
            .map(|rc| rc.inputs.iter().map(|(i, _)| i.as_str()).collect())
            .unwrap_or_default();
        let givable: Vec<(String, u32)> = inv
            .iter()
            .filter(|(id, _)| !my_inputs.contains(id))
            .map(|(id, c)| (id.to_string(), c))
            .collect();
        if givable.is_empty() {
            continue;
        }
        let (c, r) = cell(world, b);
        let (w, h) = footprint(world, b);
        let neighbours = edge_neighbours(&occupancy, b, c, r, w, h);
        let mut budget = n;
        for (item, _have) in givable {
            if budget == 0 {
                break;
            }
            for &nb in &neighbours {
                if budget == 0 {
                    break;
                }
                if !accepts(world, nb, &item, cat, recipes) {
                    continue;
                }
                let room = world.inventory(nb).map(|i| i.free()).unwrap_or(0);
                let send = budget.min(room).min(world.inventory(b).map(|i| i.count(&item)).unwrap_or(0));
                if send > 0 {
                    moves.push((b, nb, item.clone(), send));
                    budget -= send;
                }
            }
        }
    }

    for (from, to, item, n) in moves {
        let taken = world.inventory_mut(from).map(|i| i.take(&item, n)).unwrap_or(0);
        if taken > 0 {
            let added = world.inventory_mut(to).map(|i| i.add(&item, taken)).unwrap_or(0);
            // Return anything the target couldn't hold (raced with another move).
            if added < taken {
                if let Some(inv) = world.inventory_mut(from) {
                    inv.add(&item, taken - added);
                }
            }
        }
    }
}

/// The distinct buildings orthogonally adjacent to `b`'s footprint.
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

/// Whether building `nb` will accept `item`: a storage sink takes anything, a
/// producer takes its recipe inputs.
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
    if def.stores {
        return true;
    }
    let rid = text(world, nb, "Recipe");
    recipes
        .get(&rid)
        .is_some_and(|r| r.inputs.iter().any(|(i, _)| i == item))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_add_take_capacity() {
        let mut inv = Inventory::new(10);
        assert_eq!(inv.add("ore", 6), 6);
        assert_eq!(inv.add("ore", 8), 4); // capped at capacity 10
        assert_eq!(inv.total(), 10);
        assert_eq!(inv.take("ore", 3), 3);
        assert_eq!(inv.count("ore"), 7);
        assert_eq!(inv.take("ore", 100), 7); // only what's there
        assert!(inv.is_empty());

        let mut unlimited = Inventory::new(0);
        assert_eq!(unlimited.add("x", 1_000_000), 1_000_000);
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
        assert_eq!(r.outputs, vec![("plate".to_string(), 1)]); // count defaulted to 1
    }

    #[test]
    fn ticks_accumulate_by_rate() {
        // 2/sec over 0.4s = 0.8 -> 0 whole, then +0.4s -> 1.6 -> 1 whole.
        let (n1, acc1) = ticks(0.0, 2.0, 0.4);
        assert_eq!(n1, 0);
        let (n2, _) = ticks(acc1, 2.0, 0.4);
        assert_eq!(n2, 1);
    }
}
