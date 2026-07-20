use std::collections::HashMap;
use std::sync::LazyLock;

use glam::Vec2;

use crate::value::{Color, Rect, UDim2, Value, ValueType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassId(usize);

/// The kind of asset an `Asset`-typed property points at, so the editor can
/// present a typed drag-and-drop object field and validate drops by asset type
/// rather than raw file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetType {
    Texture,
    SpriteFrames,
    TileSet,
    WorldGen,
    Script,
    Audio,
    Material,
    Scene,
    Any,
}

#[derive(Clone, Debug)]
pub struct PropDef {
    pub name: &'static str,
    pub ty: ValueType,
    pub default: Value,
    /// Transient runtime state (e.g. playback position): held on the instance so
    /// scripts and the inspector can read it, but never written to scene files.
    pub transient: bool,
    /// For `Asset` properties, the kind of asset expected (drives typed fields).
    pub asset: Option<AssetType>,
}

fn prop(name: &'static str, default: Value) -> PropDef {
    PropDef {
        name,
        ty: default.ty(),
        default,
        transient: false,
        asset: None,
    }
}

/// A transient property — see [`PropDef::transient`].
fn prop_t(name: &'static str, default: Value) -> PropDef {
    PropDef {
        transient: true,
        ..prop(name, default)
    }
}

/// A typed asset-reference property (empty by default).
fn asset_prop(name: &'static str, kind: AssetType) -> PropDef {
    PropDef {
        asset: Some(kind),
        ..prop(name, Value::Asset(String::new()))
    }
}

pub struct ClassInfo {
    pub name: &'static str,
    pub superclass: Option<ClassId>,
    pub creatable: bool,
    pub service: bool,
    pub props: Vec<PropDef>,
}

pub struct ClassRegistry {
    classes: Vec<ClassInfo>,
    by_name: HashMap<&'static str, ClassId>,
}

impl ClassRegistry {
    pub fn find(&self, name: &str) -> Option<ClassId> {
        self.by_name.get(name).copied()
    }

    pub fn info(&self, id: ClassId) -> &ClassInfo {
        &self.classes[id.0]
    }

    pub fn is_a(&self, id: ClassId, ancestor: ClassId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.classes[c.0].superclass;
        }
        false
    }

    pub fn creatable_classes(&self) -> impl Iterator<Item = &ClassInfo> {
        self.classes.iter().filter(|c| c.creatable)
    }

    fn add(
        &mut self,
        name: &'static str,
        superclass: Option<&str>,
        creatable: bool,
        service: bool,
        own_props: Vec<PropDef>,
    ) {
        let superclass = superclass.map(|s| self.by_name[s]);
        let mut props = superclass
            .map(|s| self.classes[s.0].props.clone())
            .unwrap_or_default();
        props.extend(own_props);
        let id = ClassId(self.classes.len());
        self.classes.push(ClassInfo {
            name,
            superclass,
            creatable,
            service,
            props,
        });
        self.by_name.insert(name, id);
    }

    fn build() -> Self {
        let mut reg = Self {
            classes: Vec::new(),
            by_name: HashMap::new(),
        };
        reg.add("Instance", None, false, false, vec![]);
        reg.add("Game", Some("Instance"), false, true, vec![]);
        reg.add(
            "Workspace",
            Some("Instance"),
            false,
            true,
            vec![prop("CurrentCamera", Value::InstanceRef(None))],
        );
        reg.add("Storage", Some("Instance"), false, true, vec![]);
        // Top-level home for Scripts that aren't attached to a specific object.
        reg.add("Scripts", Some("Instance"), false, true, vec![]);
        reg.add("Folder", Some("Instance"), true, false, vec![]);
        reg.add(
            "Node2D",
            Some("Instance"),
            true,
            false,
            vec![
                prop("Position", Value::Vec2(Vec2::ZERO)),
                prop("Rotation", Value::Number(0.0)),
                prop("Scale", Value::Vec2(Vec2::ONE)),
                prop("ZIndex", Value::Number(0.0)),
                prop("Visible", Value::Bool(true)),
                prop("Locked", Value::Bool(false)),
            ],
        );
        // The 2D render node. It only draws: a texture region (`SourceRect`,
        // whole-texture when zero-sized) stretched to `Size`, tinted and
        // flipped. It knows nothing about animation — for sprite-frame playback
        // use `AnimatedSprite`.
        reg.add(
            "Sprite",
            Some("Node2D"),
            true,
            false,
            vec![
                asset_prop("Texture", AssetType::Texture),
                prop("Size", Value::Vec2(Vec2::new(100.0, 100.0))),
                prop("Pivot", Value::Vec2(Vec2::new(0.5, 0.5))),
                // Sub-region of the texture, in pixels. Zero size = whole image.
                prop("SourceRect", Value::Rect(Rect::default())),
                prop("Tint", Value::Color(Color::WHITE)),
                prop("FlipX", Value::Bool(false)),
                prop("FlipY", Value::Bool(false)),
                // Reserved for a future shader/material system.
                asset_prop("Material", AssetType::Material),
            ],
        );
        // Self-contained sprite-frame animation node. It owns playback AND
        // rendering: it resolves the current frame of the selected `Animation`
        // from its `Frames` library (the single source of truth for the
        // texture) and draws it directly — it never mutates another node.
        // Authored config serializes; runtime state (Playing/CurrentFrame/
        // TimePosition) is transient and read-only.
        reg.add(
            "AnimatedSprite",
            Some("Node2D"),
            true,
            false,
            vec![
                asset_prop("Frames", AssetType::SpriteFrames),
                prop("Animation", Value::String(String::new())),
                prop("AutoPlay", Value::Bool(false)),
                prop("SpeedScale", Value::Number(1.0)),
                prop("Size", Value::Vec2(Vec2::new(100.0, 100.0))),
                prop("Pivot", Value::Vec2(Vec2::new(0.5, 0.5))),
                prop("Tint", Value::Color(Color::WHITE)),
                prop("FlipX", Value::Bool(false)),
                prop("FlipY", Value::Bool(false)),
                asset_prop("Material", AssetType::Material),
                // Transient runtime state — never serialized.
                prop_t("Playing", Value::Bool(false)),
                prop_t("CurrentFrame", Value::Number(0.0)),
                prop_t("TimePosition", Value::Number(0.0)),
            ],
        );
        // Legacy sprite-only animation node, kept only so older scenes load and
        // migrate to `AnimatedSprite`. Not creatable; the `AnimationPlayer` name
        // is reserved for a future general-purpose property animator.
        reg.add(
            "LegacyAnimationPlayer",
            Some("Instance"),
            false,
            false,
            vec![
                prop("Frames", Value::Asset(String::new())),
                prop("AutoPlay", Value::String(String::new())),
                prop("Speed", Value::Number(1.0)),
                prop("CurrentClip", Value::String(String::new())),
                prop_t("TimePosition", Value::Number(0.0)),
                prop_t("CurrentFrame", Value::Number(0.0)),
                prop_t("Playing", Value::Bool(false)),
            ],
        );
        // An isometric tilemap. Stores only config; the per-cell grid is derived
        // from `Seed` + dimensions and held in a transient world side-table (see
        // `crate::tilemap`), so it never bloats the scene file.
        reg.add(
            "Tilemap",
            Some("Node2D"),
            true,
            false,
            vec![
                asset_prop("TileSet", AssetType::TileSet),
                asset_prop("WorldGen", AssetType::WorldGen),
                prop("TileWidth", Value::Number(64.0)),
                prop("TileHeight", Value::Number(32.0)),
                prop("MapWidth", Value::Number(64.0)),
                prop("MapHeight", Value::Number(64.0)),
                prop("Seed", Value::Number(0.0)),
            ],
        );
        // The 2D camera. `Position`/`Zoom` are the live view; the rest configure
        // the optional built-in controller (see `crate::camera`) — all off by
        // default, so a scripted or static camera is unaffected.
        reg.add(
            "Camera2D",
            Some("Instance"),
            true,
            false,
            vec![
                prop("Position", Value::Vec2(Vec2::ZERO)),
                prop("Zoom", Value::Number(1.0)),
                // Built-in pan/zoom controller config.
                prop("Controls", Value::Bool(false)),
                prop("PanSpeed", Value::Number(800.0)),
                prop("ZoomSpeed", Value::Number(0.15)),
                prop("MinZoom", Value::Number(0.1)),
                prop("MaxZoom", Value::Number(8.0)),
                prop("EdgeScroll", Value::Bool(false)),
                // Position clamp; an empty/zero range on an axis means unbounded.
                prop("BoundsMin", Value::Vec2(Vec2::ZERO)),
                prop("BoundsMax", Value::Vec2(Vec2::ZERO)),
                // Transient smooth-zoom target — never serialized.
                prop_t("_ZoomTarget", Value::Number(0.0)),
            ],
        );
        reg.add(
            "Script",
            Some("Instance"),
            true,
            false,
            vec![
                asset_prop("SourcePath", AssetType::Script),
                prop("Enabled", Value::Bool(true)),
            ],
        );
        // A Module is loaded on demand via `require`, not run automatically.
        reg.add(
            "Module",
            Some("Instance"),
            true,
            false,
            vec![asset_prop("SourcePath", AssetType::Script)],
        );
        reg.add("Gui", Some("Instance"), false, true, vec![]);
        reg.add(
            "GuiObject",
            Some("Instance"),
            false,
            false,
            vec![
                prop("Position", Value::UDim2(UDim2::new(0.0, 20.0, 0.0, 20.0))),
                prop("Size", Value::UDim2(UDim2::new(0.0, 160.0, 0.0, 32.0))),
                prop("AnchorPoint", Value::Vec2(Vec2::ZERO)),
                prop(
                    "BackgroundColor",
                    Value::Color(Color::new(0.14, 0.15, 0.19, 1.0)),
                ),
                prop("BackgroundTransparency", Value::Number(0.0)),
                prop("Visible", Value::Bool(true)),
                prop("ClipsDescendants", Value::Bool(false)),
                prop("ZIndex", Value::Number(0.0)),
            ],
        );
        reg.add("Frame", Some("GuiObject"), true, false, vec![]);
        // An image panel with optional 9-slice. `SliceMargins` reuses Rect as
        // four source-pixel border insets `(left, top, right, bottom)`: zero
        // means the image is simply stretched to fill; non-zero keeps the
        // corners crisp while the edges/centre stretch.
        reg.add(
            "ImageFrame",
            Some("GuiObject"),
            true,
            false,
            vec![
                asset_prop("Image", AssetType::Texture),
                prop("ImageColor", Value::Color(Color::WHITE)),
                prop("SliceMargins", Value::Rect(Rect::default())),
            ],
        );
        reg.add(
            "Label",
            Some("GuiObject"),
            true,
            false,
            vec![
                prop("Text", Value::String("Label".to_string())),
                prop("TextColor", Value::Color(Color::WHITE)),
                prop("TextSize", Value::Number(16.0)),
            ],
        );
        reg.add(
            "Button",
            Some("GuiObject"),
            true,
            false,
            vec![
                prop("Text", Value::String("Button".to_string())),
                prop("TextColor", Value::Color(Color::WHITE)),
                prop("TextSize", Value::Number(16.0)),
            ],
        );
        reg
    }
}

static REGISTRY: LazyLock<ClassRegistry> = LazyLock::new(ClassRegistry::build);

pub fn registry() -> &'static ClassRegistry {
    &REGISTRY
}
