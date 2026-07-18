use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::class::registry;
use crate::error::CoreError;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<SavedInstance>,
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
    pub fn to_json(&self) -> String {
        let mut ref_ids: HashMap<InstanceId, u64> = HashMap::new();
        let mut next = 0u64;
        for id in self.descendants(self.root()) {
            for (_, value) in self.props(id) {
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
            root: save_instance(self, self.root(), &ref_ids),
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
        let root = world.root();
        load_instance(&mut world, &scene.root, root, &mut refs, &mut fixups)?;
        for (id, prop, serial) in fixups {
            let target = refs
                .get(&serial)
                .copied()
                .ok_or_else(|| CoreError::Load(format!("dangling instance ref {serial}")))?;
            world.set_prop(id, prop, Value::InstanceRef(Some(target)))?;
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
    SavedInstance {
        class: info.name.to_string(),
        name: world.name(id).unwrap().to_string(),
        ref_id: ref_ids.get(&id).copied(),
        props,
        children: world
            .children(id)
            .iter()
            .map(|&c| save_instance(world, c, ref_ids))
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

fn load_instance(
    world: &mut World,
    saved: &SavedInstance,
    id: InstanceId,
    refs: &mut HashMap<u64, InstanceId>,
    fixups: &mut Vec<(InstanceId, &'static str, u64)>,
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
    for child in &saved.children {
        let cclass = registry()
            .find(resolve_legacy_class(&child.class))
            .ok_or_else(|| CoreError::UnknownClass(child.class.clone()))?;
        let cid = world.spawn_raw(cclass, id);
        load_instance(world, child, cid, refs, fixups)?;
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
