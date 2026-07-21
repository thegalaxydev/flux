use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::class::registry;
use crate::error::CoreError;
use crate::tilemap::{self, Cell};
use crate::value::{Color, Rect, UDim2, Value};
use crate::world::{InstanceId, World};

pub const SCENE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct SceneFile {
    version: u32,
    root: SavedInstance,
}

#[derive(Serialize, Deserialize)]
struct SavedInstance {
    class: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ref_id: Option<u64>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    props: IndexMap<String, SavedValue>,
    /// Roblox-style attributes: free-form named values, serialized in both
    /// scene files and save-games. Never `InstanceRef` (rejected on set).
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    attributes: IndexMap<String, SavedValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    /// A `Tilemap`'s runtime grid — only written by save-games (not scene files),
    /// so runtime edits (mining, terraforming) survive a reload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tiles: Option<SavedTiles>,
    /// Plugin component data (save-games only), keyed by the component name a
    /// plugin registered with [`crate::save::register_component`].
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    components: IndexMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<SavedInstance>,
}

/// A tile grid as run-length-encoded cells, keeping big maps compact.
#[derive(Serialize, Deserialize)]
struct SavedTiles {
    w: u32,
    h: u32,
    /// Runs of `[count, tile, ore, ore_amount]`, row-major.
    runs: Vec<[u32; 4]>,
}

fn rle_encode(cells: &[Cell]) -> Vec<[u32; 4]> {
    let mut runs: Vec<[u32; 4]> = Vec::new();
    for c in cells {
        if let Some(last) = runs.last_mut() {
            if last[1] == c.tile as u32 && last[2] == c.ore as u32 && last[3] == c.ore_amount as u32
            {
                last[0] += 1;
                continue;
            }
        }
        runs.push([1, c.tile as u32, c.ore as u32, c.ore_amount as u32]);
    }
    runs
}

fn rle_decode(t: &SavedTiles) -> Vec<Cell> {
    let mut cells = Vec::with_capacity((t.w as usize) * (t.h as usize));
    for r in &t.runs {
        let cell = Cell {
            tile: r[1] as u16,
            ore: r[2] as u16,
            ore_amount: r[3] as u16,
        };
        for _ in 0..r[0] {
            cells.push(cell);
        }
    }
    cells
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
enum SavedValue {
    Bool(bool),
    Number(f64),
    String(String),
    Vec2([f32; 2]),
    /// `[x_scale, x_offset, y_scale, y_offset]`.
    UDim2([f32; 4]),
    Color([f32; 4]),
    /// `[x, y, w, h]` in texture pixels.
    Rect([f32; 4]),
    Asset(String),
    Ref(Option<u64>),
}

impl World {
    /// Serialize the scene tree (no derived tilemap grids — those regenerate).
    pub fn to_json(&self) -> String {
        self.to_json_impl(false)
    }

    /// Serialize for a save game: the tree plus each `Tilemap`'s runtime grid, so
    /// runtime edits (mined ore, terraforming) and placed buildings all persist.
    /// Loads back through [`World::from_json`] like any scene.
    pub fn to_save_string(&self) -> String {
        self.to_json_impl(true)
    }

    fn to_json_impl(&self, include_tiles: bool) -> String {
        let mut ref_ids: HashMap<InstanceId, u64> = HashMap::new();
        let mut next = 0u64;
        for id in self.descendants(self.root()) {
            let prop_refs = self.props(id).map(|(_, v)| v);
            let attr_refs = self.attributes(id).map(|(_, v)| v);
            for value in prop_refs.chain(attr_refs) {
                if let Value::InstanceRef(Some(target)) = value {
                    if self.contains(*target) && !ref_ids.contains_key(target) {
                        ref_ids.insert(*target, next);
                        next += 1;
                    }
                }
            }
        }
        let scene = SceneFile {
            version: SCENE_VERSION,
            root: save_instance(self, self.root(), &ref_ids, include_tiles),
        };
        serde_json::to_string_pretty(&scene).unwrap()
    }

    pub fn from_json(json: &str) -> Result<World, CoreError> {
        let scene: SceneFile =
            serde_json::from_str(json).map_err(|e| CoreError::Load(e.to_string()))?;
        if scene.version != SCENE_VERSION {
            return Err(CoreError::Load(format!(
                "unsupported scene version {}",
                scene.version
            )));
        }
        if scene.root.class != "Game" {
            return Err(CoreError::Load("root instance must be a Game".to_string()));
        }
        let mut world = World::empty_game();
        let mut refs: HashMap<u64, InstanceId> = HashMap::new();
        let mut fixups: Vec<(InstanceId, &'static str, u64)> = Vec::new();
        let mut attr_fixups: Vec<(InstanceId, String, u64)> = Vec::new();
        let mut tiles: Vec<(InstanceId, SavedTiles)> = Vec::new();
        let mut comps: Vec<(InstanceId, IndexMap<String, serde_json::Value>)> = Vec::new();
        let root = world.root();
        load_instance(
            &mut world,
            &scene.root,
            root,
            &mut refs,
            &mut fixups,
            &mut attr_fixups,
            &mut tiles,
            &mut comps,
        )?;
        for (id, prop, serial) in fixups {
            let target = refs
                .get(&serial)
                .copied()
                .ok_or_else(|| CoreError::Load(format!("dangling instance ref {serial}")))?;
            world.set_prop(id, prop, Value::InstanceRef(Some(target)))?;
        }
        for (id, name, serial) in attr_fixups {
            let target = refs
                .get(&serial)
                .copied()
                .ok_or_else(|| CoreError::Load(format!("dangling instance ref {serial}")))?;
            world.set_attribute(id, &name, Some(Value::InstanceRef(Some(target))))?;
        }
        // Restore saved tilemap grids (save games) now that props are set, so the
        // signature matches and `sync` won't regenerate over them.
        for (id, t) in tiles {
            let cells = rle_decode(&t);
            tilemap::restore(&mut world, id, t.w, t.h, cells);
        }
        for (id, c) in &comps {
            crate::save::load_components(&mut world, *id, c);
        }
        // Convert legacy Sprite + AnimationPlayer pairs to AnimatedSprite.
        migrate_legacy_animation(&mut world);
        // Older scenes may predate a service (e.g. `Scripts`); add any missing.
        world.ensure_services();
        Ok(world)
    }
}

fn save_instance(
    world: &World,
    id: InstanceId,
    ref_ids: &HashMap<InstanceId, u64>,
    include_tiles: bool,
) -> SavedInstance {
    let info = registry().info(world.class_of(id).unwrap());
    let mut props = IndexMap::new();
    for pd in &info.props {
        // Transient runtime state (playback position, etc.) is never serialized.
        if pd.transient {
            continue;
        }
        let value = world.get_prop(id, pd.name).unwrap();
        if *value != pd.default {
            props.insert(pd.name.to_string(), save_value(value, ref_ids));
        }
    }
    let tiles = if include_tiles && info.name == "Tilemap" {
        world.tile_grid(id).map(|g| SavedTiles {
            w: g.width(),
            h: g.height(),
            runs: rle_encode(g.cells()),
        })
    } else {
        None
    };
    let components = if include_tiles {
        crate::save::save_components(world, id)
    } else {
        IndexMap::new()
    };
    SavedInstance {
        class: info.name.to_string(),
        name: world.name(id).unwrap().to_string(),
        ref_id: ref_ids.get(&id).copied(),
        props,
        // Attributes/tags are authored data: serialized in BOTH modes.
        attributes: world
            .attributes(id)
            .map(|(k, v)| (k.to_string(), save_value(v, ref_ids)))
            .collect(),
        tags: world.tags(id).map(str::to_string).collect(),
        tiles,
        components,
        children: world
            .children(id)
            .iter()
            .map(|&c| save_instance(world, c, ref_ids, include_tiles))
            .collect(),
    }
}

fn save_value(value: &Value, ref_ids: &HashMap<InstanceId, u64>) -> SavedValue {
    match value {
        Value::Bool(b) => SavedValue::Bool(*b),
        Value::Number(n) => SavedValue::Number(*n),
        Value::String(s) => SavedValue::String(s.clone()),
        Value::Vec2(v) => SavedValue::Vec2([v.x, v.y]),
        Value::UDim2(u) => SavedValue::UDim2([u.x.scale, u.x.offset, u.y.scale, u.y.offset]),
        Value::Color(c) => SavedValue::Color([c.r, c.g, c.b, c.a]),
        Value::Rect(r) => SavedValue::Rect([r.x, r.y, r.w, r.h]),
        Value::Asset(s) => SavedValue::Asset(s.clone()),
        Value::InstanceRef(t) => SavedValue::Ref(t.and_then(|t| ref_ids.get(&t).copied())),
    }
}

#[allow(clippy::too_many_arguments)]
fn load_instance(
    world: &mut World,
    saved: &SavedInstance,
    id: InstanceId,
    refs: &mut HashMap<u64, InstanceId>,
    fixups: &mut Vec<(InstanceId, &'static str, u64)>,
    attr_fixups: &mut Vec<(InstanceId, String, u64)>,
    tiles: &mut Vec<(InstanceId, SavedTiles)>,
    comps: &mut Vec<(InstanceId, IndexMap<String, serde_json::Value>)>,
) -> Result<(), CoreError> {
    world.set_name_raw(id, saved.name.clone());
    if let Some(r) = saved.ref_id {
        refs.insert(r, id);
    }
    let class = world.class_of(id).unwrap();
    let info = registry().info(class);
    for (pname, sv) in &saved.props {
        let Some(pd) = info.props.iter().find(|p| p.name == pname.as_str()) else {
            continue;
        };
        match sv {
            SavedValue::Ref(Some(serial)) => fixups.push((id, pd.name, *serial)),
            _ => world.set_prop(id, pd.name, load_value(sv))?,
        }
    }
    for (aname, sv) in &saved.attributes {
        match sv {
            // Object attributes resolve after the whole tree exists.
            SavedValue::Ref(Some(serial)) => attr_fixups.push((id, aname.clone(), *serial)),
            _ => world.set_attribute(id, aname, Some(load_value(sv)))?,
        }
    }
    for tag in &saved.tags {
        world.add_tag(id, tag);
    }
    if let Some(t) = &saved.tiles {
        tiles.push((id, SavedTiles { w: t.w, h: t.h, runs: t.runs.clone() }));
    }
    if !saved.components.is_empty() {
        comps.push((id, saved.components.clone()));
    }
    for child in &saved.children {
        let cclass = registry()
            .find(resolve_legacy_class(&child.class))
            .ok_or_else(|| CoreError::UnknownClass(child.class.clone()))?;
        let cid = world.spawn_raw(cclass, id);
        load_instance(world, child, cid, refs, fixups, attr_fixups, tiles, comps)?;
    }
    Ok(())
}

/// Map removed class names to their compatibility stand-ins so older scenes
/// still load (they are then converted by [`migrate_legacy_animation`]).
fn resolve_legacy_class(name: &str) -> &str {
    match name {
        "AnimationPlayer" | "SpriteAnimator" => "LegacyAnimationPlayer",
        other => other,
    }
}

/// Convert legacy `Sprite` + animation-player pairs into self-contained
/// `AnimatedSprite` nodes. A player not parented to a Sprite can't be migrated
/// meaningfully and is dropped.
fn migrate_legacy_animation(world: &mut World) {
    let root = world.root();
    let legacy: Vec<InstanceId> = world
        .descendants(root)
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("LegacyAnimationPlayer"))
        .collect();

    for player in legacy {
        let Some(sprite) = world.parent(player) else {
            let _ = world.destroy(player);
            continue;
        };
        if world.class_name(sprite) != Some("Sprite") {
            let _ = world.destroy(player);
            continue;
        }
        let Some(parent) = world.parent(sprite) else {
            let _ = world.destroy(player);
            continue;
        };
        let index = world.child_index(sprite).unwrap_or(0);

        let anim = world.spawn("AnimatedSprite", parent);
        let _ = world.reparent_at(anim, parent, index);
        if let Some(name) = world.name(sprite).map(str::to_string) {
            world.set_name_raw(anim, name);
        }
        // Transfer transform + visual configuration from the Sprite.
        for p in [
            "Position", "Rotation", "Scale", "ZIndex", "Visible", "Locked", "Size", "Pivot",
            "Tint", "FlipX", "FlipY", "Material",
        ] {
            if let Some(v) = world.get_prop(sprite, p).cloned() {
                let _ = world.set_prop(anim, p, v);
            }
        }
        // Transfer animation configuration from the player.
        if let Some(v) = world.get_prop(player, "Frames").cloned() {
            let _ = world.set_prop(anim, "Frames", v);
        }
        if let Some(Value::Number(s)) = world.get_prop(player, "Speed").cloned() {
            let _ = world.set_prop(anim, "SpeedScale", Value::Number(s));
        }
        // The legacy AutoPlay was a clip-name string; a non-empty value maps to
        // AutoPlay=true plus that animation.
        let mut animation = String::new();
        if let Some(Value::String(clip)) = world.get_prop(player, "AutoPlay").cloned() {
            if !clip.is_empty() {
                let _ = world.set_prop(anim, "AutoPlay", Value::Bool(true));
                animation = clip;
            }
        }
        if animation.is_empty() {
            if let Some(Value::String(clip)) = world.get_prop(player, "CurrentClip").cloned() {
                animation = clip;
            }
        }
        if !animation.is_empty() {
            let _ = world.set_prop(anim, "Animation", Value::String(animation));
        }
        // Re-home the Sprite's other children under the new node.
        let kids: Vec<InstanceId> = world
            .children(sprite)
            .iter()
            .copied()
            .filter(|&c| c != player)
            .collect();
        for (i, kid) in kids.into_iter().enumerate() {
            let _ = world.reparent_at(kid, anim, i);
        }
        let _ = world.destroy(sprite);
    }
}

#[cfg(test)]
mod save_tests {
    use crate::tilemap::{self, TileSetCache, WorldGenCache};
    use crate::value::Value;
    use crate::world::World;
    use std::path::Path;

    fn tilemap_of(w: &World) -> crate::world::InstanceId {
        w.descendants(w.workspace())
            .into_iter()
            .find(|&id| w.class_name(id) == Some("Tilemap"))
            .unwrap()
    }

    fn re_camera(w: &World) -> crate::world::InstanceId {
        w.descendants(w.workspace())
            .into_iter()
            .find(|&id| w.class_name(id) == Some("Camera2D"))
            .unwrap()
    }

    #[test]
    fn attributes_and_tags_round_trip_in_scene_and_save() {
        let mut w = World::new();
        let tm = w.create("Tilemap", w.workspace()).unwrap();
        w.set_attribute(tm, "Money", Some(Value::Number(150.0))).unwrap();
        w.set_attribute(tm, "Buildings", Some(Value::Asset("cat.buildings.json".into()))).unwrap();
        w.set_attribute(tm, "Spawn", Some(Value::Vec2(glam::Vec2::new(3.0, 4.0)))).unwrap();
        w.add_tag(tm, "main-map");
        w.add_tag(tm, "hazardous");

        // Object attribute: points at another instance, survives the round
        // trip through the ref-id fixup machinery.
        let cam = re_camera(&w);
        w.set_attribute(tm, "Cam", Some(Value::InstanceRef(Some(cam)))).unwrap();

        for json in [w.to_json(), w.to_save_string()] {
            let re = World::from_json(&json).unwrap();
            let tm2 = tilemap_of(&re);
            assert_eq!(re.attribute(tm2, "Money"), Some(&Value::Number(150.0)));
            assert_eq!(
                re.attribute(tm2, "Buildings"),
                Some(&Value::Asset("cat.buildings.json".into()))
            );
            assert_eq!(re.attribute(tm2, "Spawn"), Some(&Value::Vec2(glam::Vec2::new(3.0, 4.0))));
            assert!(re.has_tag(tm2, "main-map") && re.has_tag(tm2, "hazardous"));
            assert_eq!(re.tagged("main-map"), vec![tm2]);
            let cam2 = re_camera(&re);
            assert_eq!(
                re.attribute(tm2, "Cam"),
                Some(&Value::InstanceRef(Some(cam2))),
                "object attribute must re-link to the reloaded target"
            );
        }

        // Removal really removes (and serializes as absent).
        w.set_attribute(tm, "Money", None).unwrap();
        w.remove_tag(tm, "hazardous");
        let re = World::from_json(&w.to_json()).unwrap();
        let tm2 = tilemap_of(&re);
        assert_eq!(re.attribute(tm2, "Money"), None);
        assert!(!re.has_tag(tm2, "hazardous"));
        assert!(re.has_tag(tm2, "main-map"));
    }

    #[test]
    fn clone_and_destroy_carry_attributes_and_tags() {
        let mut w = World::new();
        let s = w.create("Sprite", w.workspace()).unwrap();
        w.set_attribute(s, "Team", Some(Value::String("red".into()))).unwrap();
        w.add_tag(s, "unit");

        let sub = w.snapshot_subtree(s).unwrap();
        let map = w.restore_subtree(w.workspace(), 0, &sub).unwrap();
        let copy = map[&s];
        assert_ne!(copy, s);
        assert_eq!(w.attribute(copy, "Team"), Some(&Value::String("red".into())));
        assert!(w.has_tag(copy, "unit"));
        assert_eq!(w.tagged("unit").len(), 2);

        w.destroy(s).unwrap();
        assert_eq!(w.tagged("unit"), vec![copy]);
        assert_eq!(w.attribute(s, "Team"), None);
    }

    #[test]
    fn save_round_trips_tilemap_grid_but_scene_json_omits_it() {
        let mut w = World::new();
        let tm = w.create("Tilemap", w.workspace()).unwrap();
        w.set_prop(tm, "MapWidth", Value::Number(8.0)).unwrap();
        w.set_prop(tm, "MapHeight", Value::Number(8.0)).unwrap();
        let (mut ts, mut wg) = (TileSetCache::default(), WorldGenCache::default());
        tilemap::sync(&mut w, &mut ts, &mut wg, Path::new("."));

        // Runtime edits to the grid.
        w.tile_grid_mut(tm).unwrap().set_tile(0, 0, 5);
        w.tile_grid_mut(tm).unwrap().set_ore(1, 1, 3, 250);

        // Scene JSON stays lean; only the save carries the grid.
        assert!(!w.to_json().contains("\"tiles\""));
        let save = w.to_save_string();
        assert!(save.contains("\"tiles\""));

        let mut w2 = World::from_json(&save).unwrap();
        let tm2 = tilemap_of(&w2);
        {
            let g = w2.tile_grid(tm2).expect("grid restored from save");
            assert_eq!(g.get(0, 0), Some(5));
            assert_eq!(g.cell(1, 1).unwrap().ore_amount, 250);
        }

        // A fresh sync must NOT wipe the restored grid (signature matches config).
        tilemap::sync(&mut w2, &mut ts, &mut wg, Path::new("."));
        assert_eq!(
            w2.tile_grid(tm2).unwrap().get(0, 0),
            Some(5),
            "sync clobbered a restored save"
        );
    }
}

fn load_value(sv: &SavedValue) -> Value {
    match sv {
        SavedValue::Bool(b) => Value::Bool(*b),
        SavedValue::Number(n) => Value::Number(*n),
        SavedValue::String(s) => Value::String(s.clone()),
        SavedValue::Vec2([x, y]) => Value::Vec2(glam::Vec2::new(*x, *y)),
        SavedValue::UDim2([xs, xo, ys, yo]) => Value::UDim2(UDim2::new(*xs, *xo, *ys, *yo)),
        SavedValue::Color([r, g, b, a]) => Value::Color(Color::new(*r, *g, *b, *a)),
        SavedValue::Rect([x, y, w, h]) => Value::Rect(Rect::new(*x, *y, *w, *h)),
        SavedValue::Asset(s) => Value::Asset(s.clone()),
        SavedValue::Ref(None) => Value::InstanceRef(None),
        SavedValue::Ref(Some(_)) => unreachable!(),
    }
}
