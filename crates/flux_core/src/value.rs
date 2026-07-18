use glam::Vec2;

use crate::world::InstanceId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

/// One axis of a [`UDim2`]: a fraction of the parent's extent plus a pixel offset.
/// Mirrors Roblox's `UDim`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UDim {
    pub scale: f32,
    pub offset: f32,
}

impl UDim {
    pub fn new(scale: f32, offset: f32) -> Self {
        Self { scale, offset }
    }

    /// Resolve to pixels given the parent's extent along this axis.
    pub fn resolve(self, parent: f32) -> f32 {
        parent * self.scale + self.offset
    }
}

/// A 2D position or size expressed in scale (fraction of parent) + offset (pixels).
/// Mirrors Roblox's `UDim2`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UDim2 {
    pub x: UDim,
    pub y: UDim,
}

impl UDim2 {
    pub fn new(x_scale: f32, x_offset: f32, y_scale: f32, y_offset: f32) -> Self {
        Self {
            x: UDim::new(x_scale, x_offset),
            y: UDim::new(y_scale, y_offset),
        }
    }

    pub fn from_offset(x: f32, y: f32) -> Self {
        Self::new(0.0, x, 0.0, y)
    }

    pub fn from_scale(x: f32, y: f32) -> Self {
        Self::new(x, 0.0, y, 0.0)
    }
}

/// A rectangle in texture pixels: top-left `(x, y)` plus `(w, h)`. Used for a
/// `SpriteRenderer`'s source region (and, later, 9-slice margins). A zero or
/// negative extent means "the whole texture" — see [`Rect::is_whole`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Whole-texture sentinel: a non-positive width or height means the renderer
    /// should draw the entire image rather than a sub-region.
    pub fn is_whole(self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Bool,
    Number,
    String,
    Vec2,
    UDim2,
    Color,
    Rect,
    Asset,
    InstanceRef,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    String(String),
    Vec2(Vec2),
    UDim2(UDim2),
    Color(Color),
    Rect(Rect),
    Asset(String),
    InstanceRef(Option<InstanceId>),
}

impl Value {
    pub fn ty(&self) -> ValueType {
        match self {
            Value::Bool(_) => ValueType::Bool,
            Value::Number(_) => ValueType::Number,
            Value::String(_) => ValueType::String,
            Value::Vec2(_) => ValueType::Vec2,
            Value::UDim2(_) => ValueType::UDim2,
            Value::Color(_) => ValueType::Color,
            Value::Rect(_) => ValueType::Rect,
            Value::Asset(_) => ValueType::Asset,
            Value::InstanceRef(_) => ValueType::InstanceRef,
        }
    }
}
