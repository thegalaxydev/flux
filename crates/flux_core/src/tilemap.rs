//! Data-driven isometric tilemaps.
//!
//! Pieces, mirroring the animation subsystem ([`crate::animation`]):
//!
//! * **Iso math** — [`tile_to_world`] / [`world_to_tile`] convert between tile
//!   `(col, row)` coordinates and world pixels for a 2:1 diamond isometric
//!   projection. Pure functions, the single source of truth for the geometry
//!   (like [`crate::gui`] for GUI layout and [`crate::transform`] for sprites).
//!
//! * **`TileSet` asset** — a `*.tileset.json` file defines the tile *palette*:
//!   footprint size plus a list of tile types (id, colour, optional atlas
//!   rect). Includes both terrain tiles and ore tiles. Parsed once and shared
//!   via [`Rc`] through a [`TileSetCache`].
//!
//! * **`WorldGen` asset** — a `*.worldgen.json` file defines *how the world is
//!   generated*: noise feature sizes, an ordered table of elevation/moisture
//!   biome bands, and an ore table (frequency, richness, elevation range). This
//!   is what makes terrain data-driven rather than hardcoded.
//!
//! * **`TileGrid`** — the actual per-cell data ([`Cell`]: base tile + optional
//!   ore + ore amount). A `Tilemap` instance stores only *config* (tileset,
//!   worldgen, tile size, map dimensions, seed); the grid itself is **derived**
//!   deterministically by [`generate`] and held in a transient side-table on the
//!   [`World`], kept in step by [`sync`].

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
/// contains world point `p`. Used for picking/painting and render culling.
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
fn one() -> f32 {
    1.0
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
// WorldGen authoring schema (`*.worldgen.json`)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
pub struct WorldGenDoc {
    /// Feature size (in tiles) of the elevation noise — bigger = broader
    /// landmasses.
    #[serde(default = "default_elev_scale")]
    pub elevation_scale: f32,
    /// Feature size (in tiles) of the moisture noise (drives forest vs plains).
    #[serde(default = "default_moist_scale")]
    pub moisture_scale: f32,
    /// Ordered biome bands: the first whose elevation/moisture window matches a
    /// cell wins, so put more specific (moisture-gated) bands first.
    #[serde(default)]
    pub biomes: Vec<BiomeDoc>,
    /// Ore deposits, tried in order; the first match per cell wins.
    #[serde(default)]
    pub ores: Vec<OreDoc>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BiomeDoc {
    /// Tile id (in the paired `TileSet`) this band paints.
    pub tile: String,
    /// Inclusive upper elevation bound in `[0, 1]` for this band.
    #[serde(default = "one")]
    pub max_elevation: f32,
    #[serde(default)]
    pub min_moisture: f32,
    #[serde(default = "one")]
    pub max_moisture: f32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct OreDoc {
    /// Ore tile id (in the paired `TileSet`) used as the deposit marker.
    pub tile: String,
    /// Fraction (`0..1`) of eligible cells that carry this ore (clustered).
    #[serde(default = "default_freq")]
    pub frequency: f32,
    /// Base deposit amount; scaled up where the deposit noise peaks.
    #[serde(default = "default_richness")]
    pub richness: f32,
    #[serde(default)]
    pub min_elevation: f32,
    #[serde(default = "one")]
    pub max_elevation: f32,
    /// Feature size (in tiles) of the deposit noise — bigger = larger patches.
    #[serde(default = "default_ore_scale")]
    pub scale: f32,
}

fn default_elev_scale() -> f32 {
    14.0
}
fn default_moist_scale() -> f32 {
    9.0
}
fn default_freq() -> f32 {
    0.06
}
fn default_richness() -> f32 {
    2000.0
}
fn default_ore_scale() -> f32 {
    5.0
}

impl WorldGenDoc {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// Parsed world-generation config, shared via [`Rc`].
pub struct WorldGen {
    pub elevation_scale: f32,
    pub moisture_scale: f32,
    pub biomes: Vec<BiomeDoc>,
    pub ores: Vec<OreDoc>,
}

impl WorldGen {
    pub fn parse(json: &str) -> Result<Self, String> {
        Ok(Self::from_doc(&WorldGenDoc::from_json(json)?))
    }

    pub fn from_doc(doc: &WorldGenDoc) -> Self {
        WorldGen {
            elevation_scale: doc.elevation_scale.max(1.0),
            moisture_scale: doc.moisture_scale.max(1.0),
            biomes: doc.biomes.clone(),
            ores: doc.ores.clone(),
        }
    }
}

/// Loads and caches `*.worldgen.json` files, mirroring [`TileSetCache`].
#[derive(Default)]
pub struct WorldGenCache {
    configs: HashMap<String, Option<Rc<WorldGen>>>,
}

impl WorldGenCache {
    pub fn get(&mut self, rel: &str, root: &Path) -> Option<Rc<WorldGen>> {
        if rel.is_empty() {
            return None;
        }
        if let Some(v) = self.configs.get(rel) {
            return v.clone();
        }
        let loaded = std::fs::read_to_string(root.join(rel))
            .ok()
            .and_then(|text| WorldGen::parse(&text).ok())
            .map(Rc::new);
        self.configs.insert(rel.to_string(), loaded.clone());
        loaded
    }

    pub fn clear(&mut self) {
        self.configs.clear();
    }
}

// ---------------------------------------------------------------------------
// TileGrid — the derived per-cell data
// ---------------------------------------------------------------------------

/// Sentinel for [`Cell::ore`] meaning "no ore in this cell".
pub const NO_ORE: u16 = u16::MAX;

/// One map cell: a base terrain tile plus an optional ore deposit. Ore and
/// terrain indices both point into the map's [`TileSet`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub tile: u16,
    pub ore: u16,
    pub ore_amount: u16,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            tile: 0,
            ore: NO_ORE,
            ore_amount: 0,
        }
    }
}

impl Cell {
    pub fn has_ore(&self) -> bool {
        self.ore != NO_ORE
    }
}

/// A rectangular grid of [`Cell`]s, row-major. Bounded and flat for now.
pub struct TileGrid {
    width: u32,
    height: u32,
    cells: Vec<Cell>,
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

    fn index(&self, col: i32, row: i32) -> Option<usize> {
        if col < 0 || row < 0 || col as u32 >= self.width || row as u32 >= self.height {
            return None;
        }
        Some(row as usize * self.width as usize + col as usize)
    }

    /// The full cell at `(col, row)`, or `None` if out of bounds.
    pub fn cell(&self, col: i32, row: i32) -> Option<Cell> {
        self.index(col, row).map(|i| self.cells[i])
    }

    /// The base terrain tile index at `(col, row)`, or `None` if out of bounds.
    pub fn get(&self, col: i32, row: i32) -> Option<u16> {
        self.cell(col, row).map(|c| c.tile)
    }

    /// Overwrite the whole cell at `(col, row)`; returns `false` if out of bounds.
    pub fn set_cell(&mut self, col: i32, row: i32, cell: Cell) -> bool {
        match self.index(col, row) {
            Some(i) => {
                self.cells[i] = cell;
                true
            }
            None => false,
        }
    }

    /// Set the base terrain tile, keeping any ore. Returns `false` if out of bounds.
    pub fn set_tile(&mut self, col: i32, row: i32, tile: u16) -> bool {
        match self.index(col, row) {
            Some(i) => {
                self.cells[i].tile = tile;
                true
            }
            None => false,
        }
    }

    /// Set (or, with `NO_ORE`, clear) the ore in a cell. Returns `false` if out
    /// of bounds.
    pub fn set_ore(&mut self, col: i32, row: i32, ore: u16, amount: u16) -> bool {
        match self.index(col, row) {
            Some(i) => {
                self.cells[i].ore = ore;
                self.cells[i].ore_amount = if ore == NO_ORE { 0 } else { amount };
                true
            }
            None => false,
        }
    }

    /// Remove up to `amount` from a cell's ore deposit, clearing it when
    /// depleted. Returns how much was actually removed (0 if no ore / out of
    /// bounds) — the primitive gameplay mining uses.
    pub fn mine(&mut self, col: i32, row: i32, amount: u16) -> u16 {
        let Some(i) = self.index(col, row) else {
            return 0;
        };
        let cell = &mut self.cells[i];
        if cell.ore == NO_ORE {
            return 0;
        }
        let removed = amount.min(cell.ore_amount);
        cell.ore_amount -= removed;
        if cell.ore_amount == 0 {
            cell.ore = NO_ORE;
        }
        removed
    }
}

/// Config that fully determines a generated grid; hashed into a signature.
fn signature(width: u32, height: u32, seed: u64, tileset: &str, worldgen: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut h);
    height.hash(&mut h);
    seed.hash(&mut h);
    tileset.hash(&mut h);
    worldgen.hash(&mut h);
    h.finish()
}

// ---- noise primitives -------------------------------------------------------

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

/// Two-octave value noise (broad shape + finer detail), in `[0, 1]`.
fn fbm(col: u32, row: u32, scale: f32, seed: u64) -> f32 {
    let (c, r) = (col as f32, row as f32);
    let big = value_noise(c / scale, r / scale, seed);
    let small = value_noise(c / (scale / 3.0).max(1.0), r / (scale / 3.0).max(1.0), seed ^ 0xABCD);
    (big * 0.65 + small * 0.35).clamp(0.0, 1.0)
}

// ---- generation -------------------------------------------------------------

struct ResolvedBiome {
    tile: u16,
    max_elev: f32,
    min_m: f32,
    max_m: f32,
}

struct ResolvedOre {
    tile: u16,
    freq: f32,
    richness: f32,
    min_e: f32,
    max_e: f32,
    scale: f32,
    seed: u64,
}

/// Generate a deterministic `width` x `height` grid from `seed`.
///
/// With a [`WorldGen`] config and its paired [`TileSet`], terrain and ore are
/// fully data-driven. Without them (or if the config resolves to no biomes), a
/// built-in placeholder generator runs so a `Tilemap` still shows *something*.
pub fn generate(
    width: u32,
    height: u32,
    seed: u64,
    config: Option<&WorldGen>,
    tileset: Option<&TileSet>,
) -> TileGrid {
    let resolved = config.zip(tileset).and_then(|(cfg, ts)| {
        let biomes: Vec<ResolvedBiome> = cfg
            .biomes
            .iter()
            .filter_map(|b| {
                Some(ResolvedBiome {
                    tile: ts.index_of(&b.tile)?,
                    max_elev: b.max_elevation,
                    min_m: b.min_moisture,
                    max_m: b.max_moisture,
                })
            })
            .collect();
        if biomes.is_empty() {
            return None;
        }
        let ores: Vec<ResolvedOre> = cfg
            .ores
            .iter()
            .enumerate()
            .filter_map(|(i, o)| {
                Some(ResolvedOre {
                    tile: ts.index_of(&o.tile)?,
                    freq: o.frequency.clamp(0.0, 1.0),
                    richness: o.richness.max(0.0),
                    min_e: o.min_elevation,
                    max_e: o.max_elevation,
                    scale: o.scale.max(1.0),
                    seed: seed ^ 0x9e37_79b1u64.wrapping_mul(i as u64 + 1),
                })
            })
            .collect();
        Some((cfg, biomes, ores))
    });

    let mut cells = Vec::with_capacity((width as usize) * (height as usize));
    match resolved {
        Some((cfg, biomes, ores)) => {
            for row in 0..height {
                for col in 0..width {
                    let e = fbm(col, row, cfg.elevation_scale, seed);
                    let m = value_noise(
                        col as f32 / cfg.moisture_scale,
                        row as f32 / cfg.moisture_scale,
                        seed ^ 0x5EED,
                    );
                    let tile = pick_biome(&biomes, e, m);
                    let (ore, ore_amount) = pick_ore(&ores, col, row, e);
                    cells.push(Cell {
                        tile,
                        ore,
                        ore_amount,
                    });
                }
            }
        }
        None => {
            // Placeholder: rolling terrain over four palette indices.
            for row in 0..height {
                for col in 0..width {
                    let e = fbm(col, row, 12.0, seed);
                    let tile: u16 = if e < 0.34 {
                        0
                    } else if e < 0.40 {
                        1
                    } else if e < 0.72 {
                        2
                    } else {
                        3
                    };
                    cells.push(Cell {
                        tile,
                        ..Cell::default()
                    });
                }
            }
        }
    }

    TileGrid {
        width,
        height,
        cells,
        signature: 0,
    }
}

fn pick_biome(biomes: &[ResolvedBiome], e: f32, m: f32) -> u16 {
    for b in biomes {
        if e <= b.max_elev && m >= b.min_m && m <= b.max_m {
            return b.tile;
        }
    }
    biomes.last().map(|b| b.tile).unwrap_or(0)
}

fn pick_ore(ores: &[ResolvedOre], col: u32, row: u32, e: f32) -> (u16, u16) {
    for o in ores {
        if e < o.min_e || e > o.max_e || o.freq <= 0.0 {
            continue;
        }
        let n = value_noise(col as f32 / o.scale, row as f32 / o.scale, o.seed);
        let thr = 1.0 - o.freq;
        if n > thr {
            // Amount ramps from 40% of richness at the deposit edge to 100% at
            // its peak, so bigger deposits are richer in the middle.
            let t = ((n - thr) / (1.0 - thr)).clamp(0.0, 1.0);
            let amount = (o.richness * (0.4 + 0.6 * t)).min(u16::MAX as f32) as u16;
            return (o.tile, amount);
        }
    }
    (NO_ORE, 0)
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

fn asset(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(crate::value::Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

/// For every `Tilemap` in the workspace, (re)generate its [`TileGrid`] into the
/// world's side-table when it's missing or its config changed. Cheap when
/// nothing changed (a hash compare per tilemap), so it's safe to call each
/// frame from the editor and the runtime step. `root` and the caches resolve
/// the `TileSet`/`WorldGen` assets (loaded once, reused).
pub fn sync(
    world: &mut World,
    tilesets: &mut TileSetCache,
    worldgens: &mut WorldGenCache,
    root: &Path,
) {
    let maps: Vec<(InstanceId, u32, u32, u64, String, String)> = world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("Tilemap"))
        .map(|id| {
            let w = num(world, id, "MapWidth").clamp(0.0, 4096.0) as u32;
            let h = num(world, id, "MapHeight").clamp(0.0, 4096.0) as u32;
            let seed = num(world, id, "Seed") as i64 as u64;
            let ts = asset(world, id, "TileSet");
            let wg = asset(world, id, "WorldGen");
            (id, w, h, seed, ts, wg)
        })
        .collect();

    for (id, w, h, seed, ts_path, wg_path) in maps {
        let sig = signature(w, h, seed, &ts_path, &wg_path);
        let stale = world.tile_grid(id).map(|g| g.signature != sig).unwrap_or(true);
        if stale {
            let tileset = tilesets.get(&ts_path, root);
            let worldgen = worldgens.get(&wg_path, root);
            let mut grid = generate(w, h, seed, worldgen.as_deref(), tileset.as_deref());
            grid.signature = sig;
            world.set_tile_grid(id, grid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caches() -> (TileSetCache, WorldGenCache) {
        (TileSetCache::default(), WorldGenCache::default())
    }

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
        assert!(ts.tile(0).unwrap().rect.is_whole());
        assert_eq!(ts.tile(1).unwrap().rect, Rect::new(0.0, 0.0, 48.0, 24.0));
    }

    #[test]
    fn placeholder_generation_is_deterministic_and_bounded() {
        let a = generate(20, 16, 42, None, None);
        let b = generate(20, 16, 42, None, None);
        for row in 0..16 {
            for col in 0..20 {
                assert_eq!(a.cell(col, row), b.cell(col, row));
                assert!(a.get(col, row).unwrap() <= 3);
                assert!(!a.cell(col, row).unwrap().has_ore());
            }
        }
        assert_eq!(a.get(-1, 0), None);
        let c = generate(20, 16, 7, None, None);
        let differs = (0..16).any(|r| (0..20).any(|col| a.get(col, r) != c.get(col, r)));
        assert!(differs);
    }

    const TILESET: &str = r#"{
        "tiles": [
            { "id": "water" }, { "id": "grass" }, { "id": "mountain" },
            { "id": "coal" }, { "id": "uranium" }
        ]
    }"#;

    const WORLDGEN: &str = r#"{
        "elevation_scale": 8, "moisture_scale": 6,
        "biomes": [
            { "tile": "water",    "max_elevation": 0.35 },
            { "tile": "grass",    "max_elevation": 0.75 },
            { "tile": "mountain", "max_elevation": 1.01 }
        ],
        "ores": [
            { "tile": "coal",    "frequency": 0.5, "richness": 5000,
              "min_elevation": 0.35, "max_elevation": 0.75, "scale": 4 },
            { "tile": "uranium", "frequency": 0.5, "richness": 1000,
              "min_elevation": 0.75, "max_elevation": 1.01, "scale": 4 }
        ]
    }"#;

    #[test]
    fn data_driven_generation_respects_biomes_and_ores() {
        let ts = TileSet::parse(TILESET).unwrap();
        let wg = WorldGen::parse(WORLDGEN).unwrap();
        let grid = generate(64, 64, 2024, Some(&wg), Some(&ts));

        let (water, grass, mountain) = (0u16, 1, 2);
        let (coal, uranium) = (3u16, 4);
        let mut biome_seen = [false; 3];
        let mut ore_seen = [false; 2];

        for row in 0..64 {
            for col in 0..64 {
                let c = grid.cell(col, row).unwrap();
                // Every base tile is a real biome tile, never an ore tile.
                assert!(c.tile == water || c.tile == grass || c.tile == mountain);
                if c.tile == water {
                    biome_seen[0] = true;
                    // Water is below every ore's elevation window -> never ore.
                    assert!(!c.has_ore(), "ore on water at ({col},{row})");
                }
                if c.tile == grass {
                    biome_seen[1] = true;
                    // Grass sits in coal's band; uranium is highland-only.
                    assert_ne!(c.ore, uranium, "uranium in grass at ({col},{row})");
                }
                if c.tile == mountain {
                    biome_seen[2] = true;
                    assert_ne!(c.ore, coal, "coal in mountain at ({col},{row})");
                }
                if c.ore == coal {
                    ore_seen[0] = true;
                    assert!(c.ore_amount > 0);
                }
                if c.ore == uranium {
                    ore_seen[1] = true;
                }
            }
        }
        assert!(biome_seen.iter().all(|&b| b), "all biomes present");
        assert!(ore_seen.iter().all(|&o| o), "both ores present");
    }

    #[test]
    fn sync_generates_and_regenerates_on_config_change() {
        let (mut ts, mut wg) = caches();
        let root = Path::new(".");
        let mut world = World::new();
        let tm = world.create("Tilemap", world.workspace()).unwrap();
        world
            .set_prop(tm, "MapWidth", crate::value::Value::Number(8.0))
            .unwrap();
        world
            .set_prop(tm, "MapHeight", crate::value::Value::Number(8.0))
            .unwrap();
        assert!(world.tile_grid(tm).is_none());
        sync(&mut world, &mut ts, &mut wg, root);
        assert_eq!(world.tile_grid(tm).unwrap().width(), 8);

        let before = collect(world.tile_grid(tm).unwrap());
        world
            .set_prop(tm, "Seed", crate::value::Value::Number(99.0))
            .unwrap();
        sync(&mut world, &mut ts, &mut wg, root);
        let after = collect(world.tile_grid(tm).unwrap());
        assert_ne!(before, after);
    }

    #[test]
    fn cell_mutators_and_mining() {
        let ts = TileSet::parse(TILESET).unwrap();
        let wg = WorldGen::parse(WORLDGEN).unwrap();
        let mut grid = generate(16, 16, 5, Some(&wg), Some(&ts));

        assert!(grid.set_tile(2, 3, 1));
        assert_eq!(grid.get(2, 3), Some(1));
        assert!(!grid.set_tile(-1, 0, 1)); // out of bounds

        // Place an ore, mine it in chunks, watch it deplete and clear.
        assert!(grid.set_ore(2, 3, 3, 100));
        let c = grid.cell(2, 3).unwrap();
        assert_eq!((c.ore, c.ore_amount), (3, 100));
        assert_eq!(grid.mine(2, 3, 30), 30);
        assert_eq!(grid.cell(2, 3).unwrap().ore_amount, 70);
        assert_eq!(grid.mine(2, 3, 999), 70); // only what's left
        let c = grid.cell(2, 3).unwrap();
        assert!(!c.has_ore());
        assert_eq!(grid.mine(2, 3, 10), 0); // nothing left
        // set_ore with NO_ORE clears.
        grid.set_ore(4, 4, 3, 50);
        assert!(grid.cell(4, 4).unwrap().has_ore());
        grid.set_ore(4, 4, NO_ORE, 0);
        assert!(!grid.cell(4, 4).unwrap().has_ore());
    }

    fn collect(g: &TileGrid) -> Vec<u16> {
        (0..g.height() as i32)
            .flat_map(|r| (0..g.width() as i32).map(move |c| (c, r)))
            .map(|(c, r)| g.get(c, r).unwrap())
            .collect()
    }
}
