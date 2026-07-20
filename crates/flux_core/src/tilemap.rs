//! Data-driven isometric tilemaps.
//!
//! Three pieces, mirroring the animation subsystem ([`crate::animation`]):
//!
//! * **Iso math** — [`tile_to_world`] / [`world_to_tile`] convert between tile
//!   `(col, row)` coordinates and world pixels for a 2:1 diamond isometric
//!   projection. Pure functions, the single source of truth for the geometry
//!   (like [`crate::gui`] for GUI layout and [`crate::transform`] for sprites).
//!
//! * **`TileSet` asset** — a `*.tileset.json` file defines the tile *palette*:
//!   footprint size plus a list of tile types (id, colour, optional atlas
//!   rect). Parsed once and shared via [`Rc`] through a [`TileSetCache`], the
//!   same lazy-load pattern as `AnimationCache`.
//!
//! * **`TileGrid`** — the actual per-cell data. A `Tilemap` instance stores only
//!   *config* (tileset, tile size, map dimensions, seed); the grid itself is
//!   **derived** deterministically from `(config, seed)` by [`generate`] and
//!   held in a transient side-table on the [`World`] (so it's shared by editor,
//!   player, and scripts, and never bloats the scene file). [`sync`] keeps that
//!   side-table in step with the instances' config.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::rc::Rc;

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::value::{Color, Rect};
use crate::world::{InstanceId, World};

// ---------------------------------------------------------------------------
// Isometric coordinate math (2:1 diamond)
// ---------------------------------------------------------------------------

/// World-space centre of tile `(col, row)` for a diamond isometric projection
/// with tile footprint `tw` x `th`. Column steps down-right, row steps
/// down-left, so `(0, 0)` sits at the world origin.
pub fn tile_to_world(col: i32, row: i32, tw: f32, th: f32) -> Vec2 {
    Vec2::new(
        (col - row) as f32 * tw * 0.5,
        (col + row) as f32 * th * 0.5,
    )
}

/// Inverse of [`tile_to_world`]: the `(col, row)` of the tile whose diamond
/// contains world point `p`. Used for picking/painting (future milestones).
pub fn world_to_tile(p: Vec2, tw: f32, th: f32) -> (i32, i32) {
    // tile_to_world gives (x, y) = ((col-row)*tw/2, (col+row)*th/2). Invert:
    //   col - row = 2x/tw,  col + row = 2y/th.
    let a = 2.0 * p.x / tw; // col - row
    let b = 2.0 * p.y / th; // col + row
    let col = (a + b) * 0.5;
    let row = (b - a) * 0.5;
    (col.floor() as i32, row.floor() as i32)
}

/// The four diamond corners (top, right, bottom, left) of tile `(col, row)` in
/// world space, ready to map to screen and fill/outline.
pub fn tile_corners(col: i32, row: i32, tw: f32, th: f32) -> [Vec2; 4] {
    let c = tile_to_world(col, row, tw, th);
    let hw = tw * 0.5;
    let hh = th * 0.5;
    [
        Vec2::new(c.x, c.y - hh), // top
        Vec2::new(c.x + hw, c.y), // right
        Vec2::new(c.x, c.y + hh), // bottom
        Vec2::new(c.x - hw, c.y), // left
    ]
}

/// World-space axis-aligned bounds `(min, max)` of a whole `width` x `height`
/// map with footprint `tw` x `th`, for culling and editor selection.
pub fn map_bounds(width: u32, height: u32, tw: f32, th: f32) -> (Vec2, Vec2) {
    if width == 0 || height == 0 {
        return (Vec2::ZERO, Vec2::ZERO);
    }
    let (w, h) = (width as i32, height as i32);
    // Extreme tile centres, then expand by a half-diamond.
    let centres = [
        tile_to_world(0, 0, tw, th),
        tile_to_world(w - 1, 0, tw, th),
        tile_to_world(0, h - 1, tw, th),
        tile_to_world(w - 1, h - 1, tw, th),
    ];
    let mut min = centres[0];
    let mut max = centres[0];
    for c in &centres[1..] {
        min = min.min(*c);
        max = max.max(*c);
    }
    let half = Vec2::new(tw * 0.5, th * 0.5);
    (min - half, max + half)
}

// ---------------------------------------------------------------------------
// TileSet authoring schema (`*.tileset.json`)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
pub struct TileSetDoc {
    /// Optional shared atlas texture. Tiles without a `rect` draw as flat colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    #[serde(default = "default_tw")]
    pub tile_width: f32,
    #[serde(default = "default_th")]
    pub tile_height: f32,
    /// Tile palette, in authored order. The index is the `u16` stored per cell.
    #[serde(default)]
    pub tiles: Vec<TileDoc>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TileDoc {
    pub id: String,
    #[serde(default = "white")]
    pub color: [f32; 4],
    /// `[x, y, w, h]` in atlas pixels; omitted = flat colour (or whole texture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f32; 4]>,
}

fn default_tw() -> f32 {
    64.0
}
fn default_th() -> f32 {
    32.0
}
fn white() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

impl TileSetDoc {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Runtime TileSet (immutable, shared via Rc)
// ---------------------------------------------------------------------------

pub struct TileDef {
    pub id: String,
    pub color: Color,
    /// Atlas sub-region; whole-texture ([`Rect::is_whole`]) when unspecified.
    pub rect: Rect,
}

pub struct TileSet {
    pub texture: Option<String>,
    pub tile_width: f32,
    pub tile_height: f32,
    tiles: Vec<TileDef>,
    by_id: HashMap<String, u16>,
}

impl TileSet {
    pub fn parse(json: &str) -> Result<Self, String> {
        Ok(Self::from_doc(&TileSetDoc::from_json(json)?))
    }

    pub fn from_doc(doc: &TileSetDoc) -> Self {
        let mut by_id = HashMap::new();
        let tiles: Vec<TileDef> = doc
            .tiles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                by_id.insert(t.id.clone(), i as u16);
                TileDef {
                    id: t.id.clone(),
                    color: Color::new(t.color[0], t.color[1], t.color[2], t.color[3]),
                    rect: t
                        .rect
                        .map(|r| Rect::new(r[0], r[1], r[2], r[3]))
                        .unwrap_or_default(),
                }
            })
            .collect();
        TileSet {
            texture: doc.texture.clone(),
            tile_width: doc.tile_width.max(1.0),
            tile_height: doc.tile_height.max(1.0),
            tiles,
            by_id,
        }
    }

    pub fn tile(&self, index: u16) -> Option<&TileDef> {
        self.tiles.get(index as usize)
    }

    pub fn index_of(&self, id: &str) -> Option<u16> {
        self.by_id.get(id).copied()
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

/// Loads and caches `*.tileset.json` files by relative asset path. A failed load
/// is remembered as `None` so it isn't retried every frame. Mirrors
/// [`crate::animation::AnimationCache`].
#[derive(Default)]
pub struct TileSetCache {
    sets: HashMap<String, Option<Rc<TileSet>>>,
}

impl TileSetCache {
    pub fn get(&mut self, rel: &str, root: &Path) -> Option<Rc<TileSet>> {
        if rel.is_empty() {
            return None;
        }
        if let Some(v) = self.sets.get(rel) {
            return v.clone();
        }
        let loaded = std::fs::read_to_string(root.join(rel))
            .ok()
            .and_then(|text| TileSet::parse(&text).ok())
            .map(Rc::new);
        self.sets.insert(rel.to_string(), loaded.clone());
        loaded
    }

    /// Drop cached sets (e.g. on hot-reload or project switch).
    pub fn clear(&mut self) {
        self.sets.clear();
    }
}

// ---------------------------------------------------------------------------
// TileGrid — the derived per-cell data
// ---------------------------------------------------------------------------

/// A rectangular grid of tile indices (into a [`TileSet`]'s palette), row-major.
/// Bounded and flat for now; chunked storage for very large maps is a later
/// milestone.
pub struct TileGrid {
    width: u32,
    height: u32,
    cells: Vec<u16>,
    /// Hash of the `(config, seed)` this grid was generated from, so [`sync`]
    /// can tell when it must be regenerated.
    signature: u64,
}

impl TileGrid {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Tile index at `(col, row)`, or `None` if out of bounds.
    pub fn get(&self, col: i32, row: i32) -> Option<u16> {
        if col < 0 || row < 0 || col as u32 >= self.width || row as u32 >= self.height {
            return None;
        }
        let idx = row as usize * self.width as usize + col as usize;
        self.cells.get(idx).copied()
    }
}

/// Config that fully determines a generated grid; hashed into a signature.
fn signature(width: u32, height: u32, seed: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut h);
    height.hash(&mut h);
    seed.hash(&mut h);
    h.finish()
}

// ---- deterministic minimal generator (value noise) --------------------------
//
// A placeholder terrain generator so tilemaps are visible and non-trivial. It
// emits four palette indices (0 water, 1 sand, 2 grass, 3 rock) so a demo
// tileset authored in that order reads as terrain. Milestone 2b replaces this
// with a data-driven biome/ore generator.

fn hash01(x: i32, y: i32, seed: u64) -> f32 {
    let mut h = (x as i64 as u64).wrapping_mul(0x27d4_eb2f_1656_67c5);
    h ^= (y as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^= seed.wrapping_mul(0x1656_67b1_9e37_79f9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    // Top 24 bits -> [0, 1).
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Bilinearly interpolated value noise in `[0, 1]` at continuous `(fx, fy)`.
fn value_noise(fx: f32, fy: f32, seed: u64) -> f32 {
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;
    let tx = smoothstep(fx - x0 as f32);
    let ty = smoothstep(fy - y0 as f32);
    let a = hash01(x0, y0, seed);
    let b = hash01(x0 + 1, y0, seed);
    let c = hash01(x0, y0 + 1, seed);
    let d = hash01(x0 + 1, y0 + 1, seed);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

/// Generate a deterministic `width` x `height` grid from `seed`.
pub fn generate(width: u32, height: u32, seed: u64) -> TileGrid {
    let mut cells = Vec::with_capacity((width as usize) * (height as usize));
    for row in 0..height {
        for col in 0..width {
            // Two octaves of value noise -> a rolling terrain height field.
            let n = value_noise(col as f32 / 12.0, row as f32 / 12.0, seed) * 0.65
                + value_noise(col as f32 / 4.0, row as f32 / 4.0, seed ^ 0xABCD) * 0.35;
            let idx: u16 = if n < 0.34 {
                0 // water
            } else if n < 0.40 {
                1 // sand
            } else if n < 0.72 {
                2 // grass
            } else {
                3 // rock
            };
            cells.push(idx);
        }
    }
    TileGrid {
        width,
        height,
        cells,
        signature: signature(width, height, seed),
    }
}

// ---------------------------------------------------------------------------
// Syncing instance config -> world side-table
// ---------------------------------------------------------------------------

fn num(world: &World, id: InstanceId, name: &str) -> f64 {
    match world.get_prop(id, name) {
        Some(crate::value::Value::Number(n)) => *n,
        _ => 0.0,
    }
}

/// For every `Tilemap` in the workspace, (re)generate its [`TileGrid`] into the
/// world's side-table when it's missing or its config changed. Cheap when
/// nothing changed (a hash compare per tilemap), so it's safe to call each
/// frame from the editor and the runtime step.
pub fn sync(world: &mut World) {
    let maps: Vec<(InstanceId, u32, u32, u64)> = world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("Tilemap"))
        .map(|id| {
            let w = num(world, id, "MapWidth").clamp(0.0, 4096.0) as u32;
            let h = num(world, id, "MapHeight").clamp(0.0, 4096.0) as u32;
            let seed = num(world, id, "Seed") as i64 as u64;
            (id, w, h, seed)
        })
        .collect();
    for (id, w, h, seed) in maps {
        let sig = signature(w, h, seed);
        let stale = world.tile_grid(id).map(|g| g.signature != sig).unwrap_or(true);
        if stale {
            world.set_tile_grid(id, generate(w, h, seed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_world_round_trips_at_tile_centres() {
        let (tw, th) = (64.0, 32.0);
        for &(c, r) in &[(0, 0), (1, 0), (0, 1), (3, 2), (-2, 5), (7, -4)] {
            let w = tile_to_world(c, r, tw, th);
            assert_eq!(world_to_tile(w, tw, th), (c, r), "round trip ({c},{r})");
        }
    }

    #[test]
    fn tile_to_world_matches_the_diamond_formula() {
        assert_eq!(tile_to_world(0, 0, 64.0, 32.0), Vec2::ZERO);
        assert_eq!(tile_to_world(1, 0, 64.0, 32.0), Vec2::new(32.0, 16.0));
        assert_eq!(tile_to_world(0, 1, 64.0, 32.0), Vec2::new(-32.0, 16.0));
    }

    #[test]
    fn map_bounds_covers_the_footprint() {
        let (min, max) = map_bounds(4, 4, 64.0, 32.0);
        // Widest tiles are the left corner (0,3) and right corner (3,0).
        assert_eq!(min.x, tile_to_world(0, 3, 64.0, 32.0).x - 32.0);
        assert_eq!(max.x, tile_to_world(3, 0, 64.0, 32.0).x + 32.0);
        assert!(min.y < max.y);
    }

    #[test]
    fn tileset_parses_ids_colours_and_rects() {
        let json = r#"{
            "texture": "tiles.png",
            "tile_width": 48, "tile_height": 24,
            "tiles": [
                { "id": "water", "color": [0.1, 0.3, 0.7, 1.0] },
                { "id": "grass", "color": [0.3, 0.6, 0.25, 1.0], "rect": [0, 0, 48, 24] }
            ]
        }"#;
        let ts = TileSet::parse(json).unwrap();
        assert_eq!(ts.len(), 2);
        assert_eq!(ts.tile_width, 48.0);
        assert_eq!(ts.index_of("grass"), Some(1));
        assert_eq!(ts.index_of("nope"), None);
        let water = ts.tile(0).unwrap();
        assert!(water.rect.is_whole()); // no rect -> whole texture
        assert_eq!(ts.tile(1).unwrap().rect, Rect::new(0.0, 0.0, 48.0, 24.0));
    }

    #[test]
    fn generation_is_deterministic_and_bounded() {
        let a = generate(20, 16, 42);
        let b = generate(20, 16, 42);
        assert_eq!(a.width(), 20);
        assert_eq!(a.height(), 16);
        for row in 0..16 {
            for col in 0..20 {
                assert_eq!(a.get(col, row), b.get(col, row));
                assert!(a.get(col, row).unwrap() <= 3);
            }
        }
        assert_eq!(a.get(-1, 0), None);
        assert_eq!(a.get(20, 0), None);
        // A different seed yields a different map (overwhelmingly likely).
        let c = generate(20, 16, 7);
        let differs = (0..16).any(|r| (0..20).any(|col| a.get(col, r) != c.get(col, r)));
        assert!(differs);
    }

    #[test]
    fn sync_generates_and_regenerates_on_config_change() {
        let mut world = World::new();
        let ws = world.workspace();
        let tm = world.create("Tilemap", ws).unwrap();
        world
            .set_prop(tm, "MapWidth", crate::value::Value::Number(8.0))
            .unwrap();
        world
            .set_prop(tm, "MapHeight", crate::value::Value::Number(8.0))
            .unwrap();
        assert!(world.tile_grid(tm).is_none());
        sync(&mut world);
        assert_eq!(world.tile_grid(tm).unwrap().width(), 8);

        // Changing the seed regenerates.
        let before: Vec<u16> = collect(world.tile_grid(tm).unwrap());
        world
            .set_prop(tm, "Seed", crate::value::Value::Number(99.0))
            .unwrap();
        sync(&mut world);
        let after: Vec<u16> = collect(world.tile_grid(tm).unwrap());
        assert_ne!(before, after);
    }

    fn collect(g: &TileGrid) -> Vec<u16> {
        (0..g.height() as i32)
            .flat_map(|r| (0..g.width() as i32).map(move |c| (c, r)))
            .map(|(c, r)| g.get(c, r).unwrap())
            .collect()
    }
}
