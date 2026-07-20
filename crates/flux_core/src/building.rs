//! Data-driven buildings placed on an isometric [`crate::tilemap::Tilemap`].
//!
//! A **building catalog** (`*.buildings.json`) defines the placeable building
//! *types* (id, footprint, colour, category) — the "never hardcode gameplay"
//! backbone for the reactor game. It's parsed + cached exactly like a
//! [`crate::tilemap::TileSet`].
//!
//! A placed building is a `Building` instance parented to the tilemap. Placement
//! resolves the catalog def, checks the footprint fits and doesn't overlap an
//! existing building, then creates the node with its visuals baked into props
//! (`Type`/`Cell`/`Footprint`/`Color`) — so rendering and serialization need no
//! catalog at all; only placement does. The functions here are the reusable core
//! the Lua `Tilemap:PlaceBuilding` / `GetBuildingAt` methods call.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::value::{Color, Value};
use crate::world::{InstanceId, World};

// ---------------------------------------------------------------------------
// Catalog authoring schema (`*.buildings.json`)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct BuildingCatalogDoc {
    #[serde(default)]
    pub buildings: Vec<BuildingDoc>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BuildingDoc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Footprint `[width, height]` in tiles (defaults to 1x1).
    #[serde(default = "one_by_one")]
    pub size: [u32; 2],
    #[serde(default = "white")]
    pub color: [f32; 4],
    /// Free-form grouping for build menus (e.g. "production", "logistics").
    #[serde(default)]
    pub category: String,
    /// Recipe id (in the paired recipe catalog) assigned on placement, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// Extracts ore from the tile beneath it into its inventory.
    #[serde(default)]
    pub mines: bool,
    /// Ore/sec a miner extracts (also the production/transfer cadence baseline).
    #[serde(default = "one_f")]
    pub rate: f32,
    /// Inventory capacity in items; 0 = unlimited (for storage/sinks).
    #[serde(default = "default_capacity")]
    pub capacity: u32,
    /// Accepts any item pushed to it (a storage sink), not just recipe inputs.
    #[serde(default)]
    pub stores: bool,
}

fn one_by_one() -> [u32; 2] {
    [1, 1]
}
fn white() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}
fn one_f() -> f32 {
    1.0
}
fn default_capacity() -> u32 {
    50
}

impl BuildingCatalogDoc {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Runtime catalog (immutable, shared via Rc)
// ---------------------------------------------------------------------------

pub struct BuildingDef {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub color: Color,
    pub category: String,
    pub recipe: Option<String>,
    pub mines: bool,
    pub rate: f32,
    pub capacity: u32,
    pub stores: bool,
}

pub struct BuildingCatalog {
    defs: Vec<BuildingDef>,
    by_id: HashMap<String, usize>,
}

impl BuildingCatalog {
    pub fn parse(json: &str) -> Result<Self, String> {
        Ok(Self::from_doc(&BuildingCatalogDoc::from_json(json)?))
    }

    pub fn from_doc(doc: &BuildingCatalogDoc) -> Self {
        let mut by_id = HashMap::new();
        let defs = doc
            .buildings
            .iter()
            .enumerate()
            .map(|(i, b)| {
                by_id.insert(b.id.clone(), i);
                BuildingDef {
                    id: b.id.clone(),
                    name: if b.name.is_empty() { b.id.clone() } else { b.name.clone() },
                    width: b.size[0].max(1),
                    height: b.size[1].max(1),
                    color: Color::new(b.color[0], b.color[1], b.color[2], b.color[3]),
                    category: b.category.clone(),
                    recipe: b.recipe.clone(),
                    mines: b.mines,
                    rate: b.rate.max(0.0),
                    capacity: b.capacity,
                    stores: b.stores,
                }
            })
            .collect();
        BuildingCatalog { defs, by_id }
    }

    pub fn get(&self, id: &str) -> Option<&BuildingDef> {
        self.by_id.get(id).map(|&i| &self.defs[i])
    }

    pub fn iter(&self) -> impl Iterator<Item = &BuildingDef> {
        self.defs.iter()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// Loads and caches `*.buildings.json` catalogs by relative path, mirroring
/// [`crate::tilemap::TileSetCache`].
#[derive(Default)]
pub struct BuildingCatalogCache {
    catalogs: HashMap<String, Option<Rc<BuildingCatalog>>>,
}

impl BuildingCatalogCache {
    pub fn get(&mut self, rel: &str, root: &Path) -> Option<Rc<BuildingCatalog>> {
        if rel.is_empty() {
            return None;
        }
        if let Some(v) = self.catalogs.get(rel) {
            return v.clone();
        }
        let loaded = std::fs::read_to_string(root.join(rel))
            .ok()
            .and_then(|text| BuildingCatalog::parse(&text).ok())
            .map(Rc::new);
        self.catalogs.insert(rel.to_string(), loaded.clone());
        loaded
    }

    pub fn clear(&mut self) {
        self.catalogs.clear();
    }
}

// ---------------------------------------------------------------------------
// Placement on a Tilemap
// ---------------------------------------------------------------------------

/// A placed building's grid cell and footprint, read from its instance props.
struct Footprint {
    col: i32,
    row: i32,
    w: i32,
    h: i32,
}

impl Footprint {
    fn contains(&self, col: i32, row: i32) -> bool {
        col >= self.col && col < self.col + self.w && row >= self.row && row < self.row + self.h
    }

    fn overlaps(&self, other: &Footprint) -> bool {
        self.col < other.col + other.w
            && self.col + self.w > other.col
            && self.row < other.row + other.h
            && self.row + self.h > other.row
    }
}

fn footprint_of(world: &World, id: InstanceId) -> Option<Footprint> {
    let cell = match world.get_prop(id, "Cell") {
        Some(Value::Vec2(v)) => *v,
        _ => return None,
    };
    let size = match world.get_prop(id, "Footprint") {
        Some(Value::Vec2(v)) => *v,
        _ => Vec2::ONE,
    };
    Some(Footprint {
        col: cell.x as i32,
        row: cell.y as i32,
        w: (size.x as i32).max(1),
        h: (size.y as i32).max(1),
    })
}

/// The map's grid size in tiles — from the derived grid if generated, else the
/// `MapWidth`/`MapHeight` config props.
fn map_dims(world: &World, tilemap: InstanceId) -> (i32, i32) {
    if let Some(g) = world.tile_grid(tilemap) {
        return (g.width() as i32, g.height() as i32);
    }
    let num = |name| match world.get_prop(tilemap, name) {
        Some(Value::Number(n)) => *n as i32,
        _ => 0,
    };
    (num("MapWidth"), num("MapHeight"))
}

/// Direct `Building` children of a tilemap.
fn buildings_of(world: &World, tilemap: InstanceId) -> impl Iterator<Item = InstanceId> + '_ {
    world
        .children(tilemap)
        .iter()
        .copied()
        .filter(move |&c| world.class_name(c) == Some("Building"))
}

/// The building whose footprint covers `(col, row)`, if any.
pub fn building_at(world: &World, tilemap: InstanceId, col: i32, row: i32) -> Option<InstanceId> {
    buildings_of(world, tilemap).find(|&b| {
        footprint_of(world, b).is_some_and(|f| f.contains(col, row))
    })
}

/// Whether a `def`-sized building fits at `(col, row)`: fully in bounds and not
/// overlapping any existing building.
pub fn can_place(
    world: &World,
    tilemap: InstanceId,
    def: &BuildingDef,
    col: i32,
    row: i32,
) -> bool {
    let (mw, mh) = map_dims(world, tilemap);
    let (w, h) = (def.width as i32, def.height as i32);
    if col < 0 || row < 0 || col + w > mw || row + h > mh {
        return false;
    }
    let want = Footprint { col, row, w, h };
    !buildings_of(world, tilemap).any(|b| footprint_of(world, b).is_some_and(|f| f.overlaps(&want)))
}

/// Place a `def` building at `(col, row)`, creating the `Building` node with its
/// visuals baked into props. Returns the new instance, or `None` if it can't be
/// placed there.
pub fn place(
    world: &mut World,
    tilemap: InstanceId,
    def: &BuildingDef,
    col: i32,
    row: i32,
) -> Option<InstanceId> {
    if !can_place(world, tilemap, def, col, row) {
        return None;
    }
    let id = world.create("Building", tilemap).ok()?;
    let _ = world.set_name(id, &def.name);
    let _ = world.set_prop(id, "Type", Value::String(def.id.clone()));
    let _ = world.set_prop(id, "Cell", Value::Vec2(Vec2::new(col as f32, row as f32)));
    let _ = world.set_prop(
        id,
        "Footprint",
        Value::Vec2(Vec2::new(def.width as f32, def.height as f32)),
    );
    let _ = world.set_prop(id, "Color", Value::Color(def.color));
    let _ = world.set_prop(
        id,
        "Recipe",
        Value::String(def.recipe.clone().unwrap_or_default()),
    );
    // Give the building its inventory buffer up front.
    world.set_inventory(id, crate::factory::Inventory::new(def.capacity));
    Some(id)
}

/// Remove the building covering `(col, row)`. Returns whether one was removed.
pub fn remove_at(world: &mut World, tilemap: InstanceId, col: i32, row: i32) -> bool {
    if let Some(b) = building_at(world, tilemap, col, row) {
        world.destroy(b).is_ok()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"{
        "buildings": [
            { "id": "belt",    "name": "Conveyor", "size": [1, 1], "color": [0.5, 0.5, 0.5, 1] },
            { "id": "smelter", "name": "Smelter",  "size": [2, 2], "color": [0.8, 0.3, 0.2, 1],
              "category": "production" }
        ]
    }"#;

    fn map(world: &mut World) -> InstanceId {
        let tm = world.create("Tilemap", world.workspace()).unwrap();
        world.set_prop(tm, "MapWidth", Value::Number(16.0)).unwrap();
        world.set_prop(tm, "MapHeight", Value::Number(16.0)).unwrap();
        tm
    }

    #[test]
    fn catalog_parses() {
        let cat = BuildingCatalog::parse(CATALOG).unwrap();
        assert_eq!(cat.len(), 2);
        let s = cat.get("smelter").unwrap();
        assert_eq!((s.width, s.height), (2, 2));
        assert_eq!(s.name, "Smelter");
        assert_eq!(s.category, "production");
        assert!(cat.get("nope").is_none());
    }

    #[test]
    fn place_respects_bounds_and_overlap() {
        let cat = BuildingCatalog::parse(CATALOG).unwrap();
        let mut w = World::new();
        let tm = map(&mut w);

        let smelter = cat.get("smelter").unwrap();
        // Fits at (2,2).
        let b = place(&mut w, tm, smelter, 2, 2).expect("placed");
        assert_eq!(w.class_name(b), Some("Building"));
        assert_eq!(building_at(&w, tm, 3, 3), Some(b)); // covers its 2x2 footprint
        assert_eq!(building_at(&w, tm, 4, 4), None);

        // Overlapping placement is refused.
        assert!(!can_place(&w, tm, smelter, 3, 3));
        assert!(place(&mut w, tm, smelter, 3, 3).is_none());

        // Out of bounds (2x2 at col 15 spills past width 16).
        assert!(!can_place(&w, tm, smelter, 15, 2));

        // A non-overlapping spot works.
        let belt = cat.get("belt").unwrap();
        assert!(place(&mut w, tm, belt, 0, 0).is_some());

        // Removal frees the cell.
        assert!(remove_at(&mut w, tm, 3, 3));
        assert_eq!(building_at(&w, tm, 2, 2), None);
        assert!(can_place(&w, tm, smelter, 2, 2));
    }
}
