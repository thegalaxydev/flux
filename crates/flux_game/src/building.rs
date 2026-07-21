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
    /// Extra cooling coefficient this building lends to edge-adjacent reactors
    /// (cooling towers). 0 = none.
    #[serde(default)]
    pub cooling: f32,
    /// Directional building (conveyors): carries a `Direction` (0=+x, 1=+y,
    /// 2=-x, 3=-y) and only hands items to the building it points at.
    #[serde(default)]
    pub directional: bool,
    /// Money cost to place this building (enforced by the game, not the engine).
    #[serde(default)]
    pub cost: f32,
    /// Animated sprite art: path to a `*.frames.json` with `idle`/`working`/
    /// `starved` clips (reactors also `off`/`running`/`hot`/`meltdown`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<String>,
    /// Frame size in pixels (world units at zoom 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite_size: Option<[f32; 2]>,
    /// Normalized pivot: where the footprint's ground centre sits in the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite_pivot: Option<[f32; 2]>,
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
    pub cooling: f32,
    pub directional: bool,
    pub cost: f32,
    pub sprite: Option<SpriteArt>,
    pub reactor: Option<ReactorParams>,
}

/// Tile-space offset for a `Direction` value (0=+x, 1=+y, 2=-x, 3=-y).
pub fn dir_offset(dir: u8) -> (i32, i32) {
    match dir % 4 {
        0 => (1, 0),
        1 => (0, 1),
        2 => (-1, 0),
        _ => (0, -1),
    }
}

/// Animated-sprite art for a building type.
#[derive(Clone)]
pub struct SpriteArt {
    pub frames: String,
    pub size: Vec2,
    pub pivot: Vec2,
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
                    cooling: b.cooling.max(0.0),
                    directional: b.directional,
                    cost: b.cost.max(0.0),
                    sprite: b.sprite.as_ref().map(|frames| SpriteArt {
                        frames: frames.clone(),
                        size: b.sprite_size.map(|s| Vec2::new(s[0], s[1])).unwrap_or(Vec2::new(64.0, 64.0)),
                        pivot: b.sprite_pivot.map(|p| Vec2::new(p[0], p[1])).unwrap_or(Vec2::new(0.5, 0.5)),
                    }),
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
/// `dir` only matters for directional buildings (conveyors).
pub fn place(
    world: &mut World,
    tilemap: InstanceId,
    def: &BuildingDef,
    col: i32,
    row: i32,
    dir: u8,
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
    if def.directional {
        let _ = world.set_prop(id, "Direction", Value::Number((dir % 4) as f64));
    }
    world.set_component::<Inventory>(id, Inventory::new(def.capacity));
    if def.sprite.is_some() {
        let sprite = attach_sprite(world, tilemap, id, def, col, row, dir);
        debug_assert!(sprite.is_some());
    }
    Some(id)
}

/// World-space centre of a `w x h` footprint at `(col, row)` on `tilemap`
/// (absolute — includes the tilemap's own position).
pub fn footprint_centre(world: &World, tilemap: InstanceId, col: i32, row: i32, w: u32, h: u32) -> Vec2 {
    let map_pos = match world.get_prop(tilemap, "Position") {
        Some(Value::Vec2(p)) => *p,
        _ => Vec2::ZERO,
    };
    let numf = |name: &str, d: f32| match world.get_prop(tilemap, name) {
        Some(Value::Number(n)) => *n as f32,
        _ => d,
    };
    let (tw, th) = (numf("TileWidth", 64.0), numf("TileHeight", 32.0));
    // tile_to_world is linear, so the formula holds for fractional cells.
    let cf = col as f32 + (w as f32 - 1.0) * 0.5;
    let rf = row as f32 + (h as f32 - 1.0) * 0.5;
    map_pos + Vec2::new((cf - rf) * tw * 0.5, (cf + rf) * th * 0.5)
}

/// Mirror flags that orient east-authored directional art: E, S, W, N.
fn dir_flips(dir: u8) -> (bool, bool) {
    match dir % 4 {
        0 => (false, false),
        1 => (true, false),
        2 => (true, true),
        _ => (false, true),
    }
}

fn style_sprite(world: &mut World, sprite: InstanceId, def: &BuildingDef, tilemap: InstanceId, col: i32, row: i32, dir: u8) {
    let Some(art) = &def.sprite else { return };
    let _ = world.set_prop(sprite, "Frames", Value::Asset(art.frames.clone()));
    let _ = world.set_prop(sprite, "AutoPlay", Value::Bool(true));
    let _ = world.set_prop(sprite, "Size", Value::Vec2(art.size));
    let _ = world.set_prop(sprite, "Pivot", Value::Vec2(art.pivot));
    let centre = footprint_centre(world, tilemap, col, row, def.width, def.height);
    let _ = world.set_prop(sprite, "Position", Value::Vec2(centre));
    if def.directional {
        let (fx, fy) = dir_flips(dir);
        let _ = world.set_prop(sprite, "FlipX", Value::Bool(fx));
        let _ = world.set_prop(sprite, "FlipY", Value::Bool(fy));
    }
    // Iso depth anchored on the FRONT corner (nearest to camera): a multi-
    // tile building must sort by its closest tile, or smaller neighbours
    // placed in front would overdraw its body. Tilemaps sit at ZIndex 0.
    let front = (col + def.width as i32 - 1) + (row + def.height as i32 - 1);
    let _ = world.set_prop(sprite, "ZIndex", Value::Number(10.0 + front as f64));
}

/// Create the building's child `AnimatedSprite`, positioned so its authored
/// pivot lands on the footprint's ground-diamond centre. Sprites don't inherit
/// parent transforms, so the position is absolute (buildings never move).
fn attach_sprite(
    world: &mut World,
    tilemap: InstanceId,
    b: InstanceId,
    def: &BuildingDef,
    col: i32,
    row: i32,
    dir: u8,
) -> Option<InstanceId> {
    def.sprite.as_ref()?;
    let sprite = world.create("AnimatedSprite", b).ok()?;
    let _ = world.set_name(sprite, "Sprite");
    let _ = world.set_prop(sprite, "Animation", Value::String("idle".into()));
    style_sprite(world, sprite, def, tilemap, col, row, dir);
    Some(sprite)
}

/// Show, move or clear the placement ghost: a translucent preview sprite that
/// follows the cursor, tinted by placement validity. `None` clears it.
pub fn set_ghost(world: &mut World, tilemap: InstanceId, ghost: Option<(&BuildingDef, i32, i32, u8)>) {
    let existing = world
        .children(tilemap)
        .iter()
        .copied()
        .find(|&c| world.name(c) == Some("_Ghost"));
    let Some((def, col, row, dir)) = ghost else {
        if let Some(g) = existing {
            let _ = world.destroy(g);
        }
        return;
    };
    if def.sprite.is_none() {
        return;
    }
    let sprite = match existing {
        Some(g) => g,
        None => {
            let Ok(g) = world.create("AnimatedSprite", tilemap) else { return };
            let _ = world.set_name(g, "_Ghost");
            g
        }
    };
    style_sprite(world, sprite, def, tilemap, col, row, dir);
    let _ = world.set_prop(sprite, "Animation", Value::String("idle".into()));
    // Always on top of real buildings; tint signals validity.
    let _ = world.set_prop(sprite, "ZIndex", Value::Number(900.0));
    let tint = if can_place(world, tilemap, def, col, row) {
        Color::new(0.7, 1.0, 0.75, 0.55)
    } else {
        Color::new(1.0, 0.45, 0.45, 0.55)
    };
    let _ = world.set_prop(sprite, "Tint", Value::Color(tint));
}

/// The building's child `AnimatedSprite`, if it has one.
pub fn sprite_of(world: &World, b: InstanceId) -> Option<InstanceId> {
    world
        .children(b)
        .iter()
        .copied()
        .find(|&c| world.class_name(c) == Some("AnimatedSprite"))
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
        let b = place(&mut w, tm, smelter, 2, 2, 0).expect("placed");
        assert_eq!(w.class_name(b), Some("Building"));
        assert_eq!(building_at(&w, tm, 3, 3), Some(b));
        assert_eq!(building_at(&w, tm, 4, 4), None);

        assert!(!can_place(&w, tm, smelter, 3, 3));
        assert!(place(&mut w, tm, smelter, 3, 3, 0).is_none());
        assert!(!can_place(&w, tm, smelter, 15, 2));

        let belt = cat.get("belt").unwrap();
        assert!(place(&mut w, tm, belt, 0, 0, 0).is_some());

        assert!(remove_at(&mut w, tm, 3, 3));
        assert_eq!(building_at(&w, tm, 2, 2), None);
    }
}
