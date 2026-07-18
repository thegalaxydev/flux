//! Asset-driven spritesheet animation.
//!
//! A clip library is a `*.frames.json` asset: a set of named [`Clip`]s, each an
//! explicit list of texture rectangles with per-frame durations (so irregular
//! atlases and variable timing are first-class). Libraries are parsed once and
//! shared across every entity via [`Rc`] — a hundred goblins playing `"Run"`
//! share one [`Clip`] allocation.
//!
//! An `AnimationPlayer` instance references a library (its `Frames` asset) and
//! holds playback state in properties (`CurrentClip`, `TimePosition`,
//! `CurrentFrame`, `Playing`, `Speed`). Each frame [`advance`] resolves the
//! active [`Clip`] from the [`AnimationCache`] and writes the current frame's
//! `Texture`/`SourceRect` into the player's parent [`Sprite`]. The player never
//! draws; the Sprite never times.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::value::{Rect, Value};
use crate::world::{InstanceId, World};

// ---------------------------------------------------------------------------
// Authoring schema (`*.frames.json`) — (de)serialized directly and edited by
// the animation editor. The immutable runtime [`SpriteFrames`] is built from it
// via [`SpriteFrames::from_doc`].
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct FramesDoc {
    /// Default texture for clips that don't specify their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    /// Named clips, in authored order.
    #[serde(default)]
    pub clips: IndexMap<String, ClipDoc>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ClipDoc {
    #[serde(rename = "loop", default = "yes")]
    pub looped: bool,
    #[serde(default = "one")]
    pub speed: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    #[serde(default)]
    pub frames: Vec<FrameDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<EventDoc>,
}

impl Default for ClipDoc {
    fn default() -> Self {
        Self { looped: true, speed: 1.0, texture: None, frames: Vec::new(), events: Vec::new() }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FrameDoc {
    /// `[x, y, w, h]` in texture pixels.
    pub rect: [f32; 4],
    #[serde(default = "default_duration")]
    pub duration: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct EventDoc {
    pub time: f32,
    pub name: String,
}

impl FramesDoc {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn default_duration() -> f32 {
    1.0 / 12.0
}

// ---------------------------------------------------------------------------
// Runtime asset data (immutable, shared via Rc)
// ---------------------------------------------------------------------------

/// A parsed clip library, shared by every player that references the same file.
pub struct SpriteFrames {
    default_texture: Option<String>,
    clips: HashMap<String, Rc<Clip>>,
}

pub struct Clip {
    pub looped: bool,
    pub speed: f32,
    pub texture: Option<String>,
    pub frames: Vec<Frame>,
    /// Prefix sum of frame start times, length `frames.len() + 1`; the last
    /// entry is the clip's total duration. Enables O(log n) frame lookup.
    cum: Vec<f32>,
}

pub struct Frame {
    pub rect: Rect,
    pub duration: f32,
    pub texture: Option<String>,
}

#[allow(dead_code)] // consumed once the frame-events feature lands
pub struct FrameEvent {
    pub time: f32,
    pub name: String,
}

impl Clip {
    pub fn total(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Index of the frame shown at clip-time `t` (clamped to the clip).
    pub fn frame_at(&self, t: f32) -> usize {
        if self.frames.is_empty() {
            return 0;
        }
        // Largest i with cum[i] <= t. `cum[0]` is 0, so this is >= 1, and we
        // step back to the frame index.
        let pp = self.cum.partition_point(|&c| c <= t);
        pp.saturating_sub(1).min(self.frames.len() - 1)
    }
}

impl SpriteFrames {
    pub fn clip(&self, name: &str) -> Option<Rc<Clip>> {
        self.clips.get(name).cloned()
    }

    /// Clip names, sorted, for the editor's clip list and AutoPlay dropdown.
    pub fn clip_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.clips.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn default_texture(&self) -> Option<&str> {
        self.default_texture.as_deref()
    }

    /// Parse a library from JSON text.
    pub fn parse(json: &str) -> Result<Self, String> {
        Ok(Self::from_doc(&FramesDoc::from_json(json)?))
    }

    /// Build immutable runtime data from an authoring [`FramesDoc`]. Frame
    /// durations are floored to a small positive value so a zero-duration frame
    /// can't stall playback.
    pub fn from_doc(doc: &FramesDoc) -> Self {
        let clips = doc
            .clips
            .iter()
            .map(|(name, c)| {
                let frames: Vec<Frame> = c
                    .frames
                    .iter()
                    .map(|f| Frame {
                        rect: Rect::new(f.rect[0], f.rect[1], f.rect[2], f.rect[3]),
                        duration: f.duration.max(1e-4),
                        texture: f.texture.clone(),
                    })
                    .collect();
                let mut cum = Vec::with_capacity(frames.len() + 1);
                let mut t = 0.0;
                cum.push(0.0);
                for f in &frames {
                    t += f.duration;
                    cum.push(t);
                }
                let clip = Clip {
                    looped: c.looped,
                    speed: c.speed.max(0.0),
                    texture: c.texture.clone(),
                    frames,
                    cum,
                };
                (name.clone(), Rc::new(clip))
            })
            .collect();
        SpriteFrames { default_texture: doc.texture.clone(), clips }
    }
}

// ---------------------------------------------------------------------------
// Runtime cache (one per session; shared across all players)
// ---------------------------------------------------------------------------

/// Loads and caches `*.frames.json` libraries by relative asset path. A failed
/// load is remembered as `None` so it isn't retried every frame.
#[derive(Default)]
pub struct AnimationCache {
    libs: HashMap<String, Option<Rc<SpriteFrames>>>,
}

impl AnimationCache {
    pub fn get(&mut self, rel: &str, root: &Path) -> Option<Rc<SpriteFrames>> {
        if rel.is_empty() {
            return None;
        }
        if let Some(v) = self.libs.get(rel) {
            return v.clone();
        }
        let loaded = std::fs::read_to_string(root.join(rel))
            .ok()
            .and_then(|text| SpriteFrames::parse(&text).ok())
            .map(Rc::new);
        self.libs.insert(rel.to_string(), loaded.clone());
        loaded
    }

    /// Drop cached libraries (e.g. on hot-reload or project switch).
    pub fn clear(&mut self) {
        self.libs.clear();
    }
}

// ---------------------------------------------------------------------------
// Instance helpers
// ---------------------------------------------------------------------------

fn num(world: &World, id: InstanceId, name: &str) -> f64 {
    match world.get_prop(id, name) {
        Some(Value::Number(n)) => *n,
        _ => 0.0,
    }
}

fn text(world: &World, id: InstanceId, name: &str) -> String {
    match world.get_prop(id, name) {
        Some(Value::String(s)) | Some(Value::Asset(s)) => s.clone(),
        _ => String::new(),
    }
}

fn flag(world: &World, id: InstanceId, name: &str) -> bool {
    matches!(world.get_prop(id, name), Some(Value::Bool(true)))
}

/// The Sprite an `AnimationPlayer` drives: its parent, if that parent is a Sprite.
pub fn target(world: &World, player: InstanceId) -> Option<InstanceId> {
    let parent = world.parent(player)?;
    (world.class_name(parent) == Some("Sprite")).then_some(parent)
}

fn players(world: &World) -> Vec<InstanceId> {
    world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("AnimationPlayer"))
        .collect()
}

/// Write the target Sprite's `Texture`/`SourceRect` for the player's current
/// frame of `clip`. Texture falls back frame -> clip -> library default; if none
/// is specified the Sprite keeps whatever texture it already has.
fn write_frame(world: &mut World, player: InstanceId, frames: &SpriteFrames, clip: &Clip) {
    let Some(target) = target(world, player) else { return };
    let idx = (num(world, player, "CurrentFrame") as usize).min(clip.frames.len().saturating_sub(1));
    let Some(frame) = clip.frames.get(idx) else { return };
    if let Some(tex) = frame
        .texture
        .as_deref()
        .or(clip.texture.as_deref())
        .or_else(|| frames.default_texture())
    {
        let _ = world.set_prop(target, "Texture", Value::Asset(tex.to_string()));
    }
    let _ = world.set_prop(target, "SourceRect", Value::Rect(frame.rect));
}

/// Resolve the library + active clip for a player, if both are available.
fn resolve(
    world: &World,
    cache: &mut AnimationCache,
    root: &Path,
    player: InstanceId,
) -> Option<(Rc<SpriteFrames>, Rc<Clip>)> {
    let frames = cache.get(&text(world, player, "Frames"), root)?;
    let clip = frames.clip(&text(world, player, "CurrentClip"))?;
    Some((frames, clip))
}

// ---------------------------------------------------------------------------
// Playback control (called from the Lua bridge and the editor)
// ---------------------------------------------------------------------------

/// `:Play(name)` — switch to clip `name` and play from the start. Honors the
/// no-restart rule: if `name` is already the current clip and it is playing,
/// this is a no-op unless `restart` is set.
pub fn play(
    world: &mut World,
    cache: &mut AnimationCache,
    root: &Path,
    player: InstanceId,
    clip: &str,
    restart: bool,
) {
    if !restart && flag(world, player, "Playing") && text(world, player, "CurrentClip") == clip {
        return;
    }
    let _ = world.set_prop(player, "CurrentClip", Value::String(clip.to_string()));
    let _ = world.set_prop(player, "TimePosition", Value::Number(0.0));
    let _ = world.set_prop(player, "CurrentFrame", Value::Number(0.0));
    let _ = world.set_prop(player, "Playing", Value::Bool(true));
    if let Some((frames, clip)) = resolve(world, cache, root, player) {
        write_frame(world, player, &frames, &clip);
    }
}

/// `:Pause()` — freeze on the current frame.
pub fn pause(world: &mut World, player: InstanceId) {
    let _ = world.set_prop(player, "Playing", Value::Bool(false));
}

/// `:Resume()` — continue from the current `TimePosition`.
pub fn resume(world: &mut World, player: InstanceId) {
    let _ = world.set_prop(player, "Playing", Value::Bool(true));
}

/// `:Stop()` — stop and reset to the first frame.
pub fn stop(world: &mut World, cache: &mut AnimationCache, root: &Path, player: InstanceId) {
    let _ = world.set_prop(player, "Playing", Value::Bool(false));
    let _ = world.set_prop(player, "TimePosition", Value::Number(0.0));
    let _ = world.set_prop(player, "CurrentFrame", Value::Number(0.0));
    if let Some((frames, clip)) = resolve(world, cache, root, player) {
        write_frame(world, player, &frames, &clip);
    }
}

// ---------------------------------------------------------------------------
// Per-session lifecycle
// ---------------------------------------------------------------------------

/// Prepare players for a fresh session: reset each, then start any whose
/// `AutoPlay` names a clip.
pub fn init(world: &mut World, cache: &mut AnimationCache, root: &Path) {
    for id in players(world) {
        let auto = text(world, id, "AutoPlay");
        let _ = world.set_prop(id, "TimePosition", Value::Number(0.0));
        let _ = world.set_prop(id, "CurrentFrame", Value::Number(0.0));
        if auto.is_empty() {
            let _ = world.set_prop(id, "Playing", Value::Bool(false));
        } else {
            play(world, cache, root, id, &auto, true);
        }
    }
}

/// Advance every playing `AnimationPlayer` by `dt` seconds.
pub fn advance(world: &mut World, cache: &mut AnimationCache, root: &Path, dt: f64) {
    for id in players(world) {
        if flag(world, id, "Playing") {
            step_one(world, cache, root, id, dt);
        }
    }
}

fn step_one(world: &mut World, cache: &mut AnimationCache, root: &Path, id: InstanceId, dt: f64) {
    let Some((frames, clip)) = resolve(world, cache, root, id) else { return };
    let total = clip.total();
    if clip.frames.is_empty() || total <= 0.0 {
        return;
    }
    let speed = num(world, id, "Speed") as f32 * clip.speed;
    let mut t = num(world, id, "TimePosition") as f32 + dt as f32 * speed;

    if t >= total {
        if clip.looped {
            t = t.rem_euclid(total);
        } else {
            let last = clip.frames.len() - 1;
            let _ = world.set_prop(id, "TimePosition", Value::Number(total as f64));
            let _ = world.set_prop(id, "CurrentFrame", Value::Number(last as f64));
            let _ = world.set_prop(id, "Playing", Value::Bool(false));
            write_frame(world, id, &frames, &clip);
            return;
        }
    } else if t < 0.0 {
        // Negative speed before the start: clamp (reverse looping can come later).
        t = if clip.looped { t.rem_euclid(total) } else { 0.0 };
    }

    let idx = clip.frame_at(t);
    let _ = world.set_prop(id, "TimePosition", Value::Number(t as f64));
    let _ = world.set_prop(id, "CurrentFrame", Value::Number(idx as f64));
    write_frame(world, id, &frames, &clip);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const LIB: &str = r#"{
        "texture": "hero.png",
        "clips": {
            "Idle": { "loop": true, "frames": [
                { "rect": [0,0,16,16], "duration": 0.1 },
                { "rect": [16,0,16,16], "duration": 0.1 }
            ]},
            "Run": { "loop": true, "frames": [
                { "rect": [0,16,16,16], "duration": 0.1 },
                { "rect": [16,16,18,16], "duration": 0.2 },
                { "rect": [34,16,16,16], "duration": 0.1 }
            ]},
            "Attack": { "loop": false, "texture": "hero_attack.png", "frames": [
                { "rect": [0,0,32,32], "duration": 0.05 },
                { "rect": [32,0,32,32], "duration": 0.05 }
            ]}
        }
    }"#;

    /// A world with a Sprite + AnimationPlayer, a cache pre-seeded with LIB, and
    /// the player's `Frames` pointed at it (loaded from an in-memory entry).
    fn setup() -> (World, AnimationCache, PathBuf, InstanceId, InstanceId) {
        let mut w = World::new();
        let ws = w.workspace();
        let sprite = w.create("Sprite", ws).unwrap();
        let player = w.create("AnimationPlayer", sprite).unwrap();
        w.set_prop(player, "Frames", Value::Asset("hero.frames.json".into())).unwrap();

        // Seed the cache directly so tests don't touch the filesystem.
        let mut cache = AnimationCache::default();
        cache
            .libs
            .insert("hero.frames.json".into(), Some(Rc::new(SpriteFrames::parse(LIB).unwrap())));
        (w, cache, PathBuf::from("."), sprite, player)
    }

    fn rect(w: &World, sprite: InstanceId) -> Rect {
        match w.get_prop(sprite, "SourceRect") {
            Some(Value::Rect(r)) => *r,
            _ => Rect::default(),
        }
    }

    fn texture(w: &World, sprite: InstanceId) -> String {
        text(w, sprite, "Texture")
    }

    #[test]
    fn parse_builds_clips_and_cumulative_times() {
        let f = SpriteFrames::parse(LIB).unwrap();
        assert_eq!(f.clip_names(), ["Attack", "Idle", "Run"]);
        let run = f.clip("Run").unwrap();
        assert_eq!(run.frames.len(), 3);
        assert!((run.total() - 0.4).abs() < 1e-6); // 0.1 + 0.2 + 0.1
        // Variable timing: frame lookup respects the long middle frame.
        assert_eq!(run.frame_at(0.0), 0);
        assert_eq!(run.frame_at(0.15), 1);
        assert_eq!(run.frame_at(0.25), 1);
        assert_eq!(run.frame_at(0.35), 2);
    }

    #[test]
    fn play_sets_first_frame_and_texture() {
        let (mut w, mut c, root, sprite, player) = setup();
        play(&mut w, &mut c, &root, player, "Run", false);
        assert_eq!(rect(&w, sprite), Rect::new(0.0, 16.0, 16.0, 16.0));
        assert_eq!(texture(&w, sprite), "hero.png"); // library default
    }

    #[test]
    fn advance_walks_frames_with_variable_timing() {
        let (mut w, mut c, root, sprite, player) = setup();
        play(&mut w, &mut c, &root, player, "Run", false);
        advance(&mut w, &mut c, &root, 0.1); // -> frame 1 (the wide one)
        assert_eq!(rect(&w, sprite), Rect::new(16.0, 16.0, 18.0, 16.0));
        advance(&mut w, &mut c, &root, 0.2); // 0.3 total -> frame 2
        assert_eq!(rect(&w, sprite), Rect::new(34.0, 16.0, 16.0, 16.0));
    }

    #[test]
    fn per_clip_texture_override_applies() {
        let (mut w, mut c, root, sprite, player) = setup();
        play(&mut w, &mut c, &root, player, "Attack", false);
        assert_eq!(texture(&w, sprite), "hero_attack.png");
    }

    #[test]
    fn no_restart_when_replaying_same_clip() {
        let (mut w, mut c, root, sprite, player) = setup();
        play(&mut w, &mut c, &root, player, "Run", false);
        advance(&mut w, &mut c, &root, 0.1); // frame 1
        play(&mut w, &mut c, &root, player, "Run", false); // same clip, playing
        assert_eq!(num(&w, player, "CurrentFrame") as i64, 1); // NOT reset
        // But a different clip switches immediately.
        play(&mut w, &mut c, &root, player, "Idle", false);
        assert_eq!(rect(&w, sprite), Rect::new(0.0, 0.0, 16.0, 16.0));
        // And an explicit restart resets even the same clip.
        advance(&mut w, &mut c, &root, 0.1);
        play(&mut w, &mut c, &root, player, "Idle", true);
        assert_eq!(num(&w, player, "CurrentFrame") as i64, 0);
    }

    #[test]
    fn looped_wraps_non_looped_stops_on_last() {
        let (mut w, mut c, root, _sprite, player) = setup();
        play(&mut w, &mut c, &root, player, "Idle", false); // looped, total 0.2
        advance(&mut w, &mut c, &root, 0.25); // wraps to 0.05 -> frame 0
        assert_eq!(num(&w, player, "CurrentFrame") as i64, 0);
        assert!(flag(&w, player, "Playing"));

        play(&mut w, &mut c, &root, player, "Attack", false); // not looped, total 0.1
        advance(&mut w, &mut c, &root, 1.0);
        assert_eq!(num(&w, player, "CurrentFrame") as i64, 1); // last frame
        assert!(!flag(&w, player, "Playing"));
    }

    #[test]
    fn stop_resets_to_first_frame() {
        let (mut w, mut c, root, sprite, player) = setup();
        play(&mut w, &mut c, &root, player, "Run", false);
        advance(&mut w, &mut c, &root, 0.15);
        stop(&mut w, &mut c, &root, player);
        assert!(!flag(&w, player, "Playing"));
        assert_eq!(num(&w, player, "TimePosition"), 0.0);
        assert_eq!(rect(&w, sprite), Rect::new(0.0, 16.0, 16.0, 16.0)); // frame 0 of Run
    }

    #[test]
    fn autoplay_starts_named_clip_on_init() {
        let (mut w, mut c, root, sprite, player) = setup();
        w.set_prop(player, "AutoPlay", Value::String("Idle".into())).unwrap();
        init(&mut w, &mut c, &root);
        assert!(flag(&w, player, "Playing"));
        assert_eq!(w.get_prop(player, "CurrentClip"), Some(&Value::String("Idle".into())));
        assert_eq!(rect(&w, sprite), Rect::new(0.0, 0.0, 16.0, 16.0));
    }

    #[test]
    fn speed_scales_playback() {
        let (mut w, mut c, root, _sprite, player) = setup();
        w.set_prop(player, "Speed", Value::Number(2.0)).unwrap();
        play(&mut w, &mut c, &root, player, "Run", false);
        advance(&mut w, &mut c, &root, 0.05); // 0.05 * 2 = 0.1 -> frame 1
        assert_eq!(num(&w, player, "CurrentFrame") as i64, 1);
    }
}
