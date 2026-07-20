//! Data-driven buildings placed on an isometric Tilemap.
//!
//! A **building catalog** (`*.buildings.json`) defines the placeable building
//! *types*. A placed building is a `Building` instance (registered by this
//! plugin) parented to the tilemap, with its visuals baked into props at
//! placement so rendering/serialization need no catalog. These functions back
//! the `Tilemap:PlaceBuilding` / `GetBuildingAt` Lua methods.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use glam::Vec2;
use serde::{Deserialize, Serialize};

use flux_core::{Color, InstanceId, Value, World};

use crate::factory::Inventory;

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
    #[serde(default = "one_by_one")]
    pub size: [u32; 2],
    #[serde(default = "white")]
    pub color: [f32; 4],
    #[serde(default)]
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    #[serde(default)]
    pub mines: bool,
    #[serde(default = "one_f")]
    pub rate: f32,
    #[serde(default = "default_capacity")]
    pub capacity: u32,
    #[serde(default)]
    pub stores: bool,
    #[serde(default)]
    pub power_use: f32,
    /// Money cost to place this building (enforced by the game, not the engine).
    #[serde(default)]
    pub cost: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactor: Option<ReactorDoc>,
}

/// Nuclear-reactor tuning (see [`crate::reactor`]).
#[derive(Serialize, Deserialize, Clone)]
pub struct ReactorDoc {
    #[serde(default = "hundred")]
    pub power: f32,
    #[serde(default = "default_heat")]
    pub heat: f32,
    #[serde(default = "default_cooling")]
    pub cooling: f32,
    #[serde(default = "default_burn")]
    pub burn: f32,
    #[serde(default = "default_meltdown")]
    pub meltdown: f32,
    #[serde(default = "default_optimal")]
    pub optimal: f32,
    #[serde(default)]
    pub fuel: String,
    #[serde(default = "default_refuel")]
    pub refuel: f32,
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
fn hundred() -> f32 {
    100.0
}
fn default_heat() -> f32 {
    400.0
}
fn default_cooling() -> f32 {
    0.3
}
fn default_burn() -> f32 {
    0.5
}
fn default_meltdown() -> f32 {
    800.0
}
fn default_optimal() -> f32 {
    350.0
}
fn default_refuel() -> f32 {
    20.0
}

impl BuildingCatalogDoc {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Runtime catalog
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
    pub power_use: f32,
    pub cost: f32,
    pub reactor: Option<ReactorParams>,
}

#[derive(Clone)]
pub struct ReactorParams {
    pub power: f32,
    pub heat: f32,
    pub cooling: f32,
    pub burn: f32,
    pub meltdown: f32,
    pub optimal: f32,
    pub fuel_item: String,
    pub refuel: f32,
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
                    power_use: b.power_use.max(0.0),
                    cost: b.cost.max(0.0),
                    reactor: b.reactor.as_ref().map(|r| ReactorParams {
                        power: r.power.max(0.0),
                        heat: r.heat.max(0.0),
                        cooling: r.cooling.max(0.0),
                        burn: r.burn.max(0.0),
                        meltdown: r.meltdown.max(1.0),
                        optimal: r.optimal.max(1.0),
                        fuel_item: r.fuel.clone(),
                        refuel: r.refuel.max(0.0),
                    }),
                }
            })
            .collect();
        BuildingCatalog { defs, by_id }
    }

    pub fn get(&self, id: &str) -> Option<&BuildingDef> {
        self.by_id.get(id).map(|&i| &self.defs[i])
    }

    /// All building defs, in catalog order (for build menus).
    pub fn defs(&self) -> impl Iterator<Item = &BuildingDef> {
        self.defs.iter()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// Loads and caches `*.buildings.json` catalogs by relative path.
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

fn buildings_of(world: &World, tilemap: InstanceId) -> impl Iterator<Item = InstanceId> + '_ {
    world
        .children(tilemap)
        .iter()
        .copied()
        .filter(move |&c| world.class_name(c) == Some("Building"))
}

/// The building whose footprint covers `(col, row)`, if any.
pub fn building_at(world: &World, tilemap: InstanceId, col: i32, row: i32) -> Option<InstanceId> {
    buildings_of(world, tilemap).find(|&b| footprint_of(world, b).is_some_and(|f| f.contains(col, row)))
}

/// Whether a `def`-sized building fits at `(col, row)`.
pub fn can_place(world: &World, tilemap: InstanceId, def: &BuildingDef, col: i32, row: i32) -> bool {
    let (mw, mh) = map_dims(world, tilemap);
    let (w, h) = (def.width as i32, def.height as i32);
    if col < 0 || row < 0 || col + w > mw || row + h > mh {
        return false;
    }
    let want = Footprint { col, row, w, h };
    !buildings_of(world, tilemap).any(|b| footprint_of(world, b).is_some_and(|f| f.overlaps(&want)))
}

/// Place a `def` building at `(col, row)`, or `None` if it can't be placed.
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
    let _ = world.set_prop(id, "Recipe", Value::String(def.recipe.clone().unwrap_or_default()));
    world.set_component::<Inventory>(id, Inventory::new(def.capacity));
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
        assert_eq!(s.category, "production");
    }

    #[test]
    fn place_respects_bounds_and_overlap() {
        crate::install();
        let cat = BuildingCatalog::parse(CATALOG).unwrap();
        let mut w = World::new();
        let tm = map(&mut w);

        let smelter = cat.get("smelter").unwrap();
        let b = place(&mut w, tm, smelter, 2, 2).expect("placed");
        assert_eq!(w.class_name(b), Some("Building"));
        assert_eq!(building_at(&w, tm, 3, 3), Some(b));
        assert_eq!(building_at(&w, tm, 4, 4), None);

        assert!(!can_place(&w, tm, smelter, 3, 3));
        assert!(place(&mut w, tm, smelter, 3, 3).is_none());
        assert!(!can_place(&w, tm, smelter, 15, 2));

        let belt = cat.get("belt").unwrap();
        assert!(place(&mut w, tm, belt, 0, 0).is_some());

        assert!(remove_at(&mut w, tm, 3, 3));
        assert_eq!(building_at(&w, tm, 2, 2), None);
    }
}
