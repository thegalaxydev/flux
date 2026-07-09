//! Roblox-style GUI layout: resolving `UDim2` position/size against a parent
//! rectangle, walking the GUI hierarchy, applying `AnchorPoint`, and clipping.
//!
//! The math is deliberately UI-framework agnostic (plain [`glam::Vec2`]) so the
//! renderer, the editor, and the scripting layer all share one source of truth.

use glam::Vec2;

use crate::class::registry;
use crate::value::{UDim2, Value};
use crate::world::{InstanceId, World};

/// An axis-aligned rectangle stored as top-left corner + size (both in pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect2 {
    pub min: Vec2,
    pub size: Vec2,
}

impl Rect2 {
    pub fn new(min: Vec2, size: Vec2) -> Self {
        Self { min, size }
    }

    pub fn from_screen(size: Vec2) -> Self {
        Self { min: Vec2::ZERO, size }
    }

    pub fn max(&self) -> Vec2 {
        self.min + self.size
    }

    pub fn center(&self) -> Vec2 {
        self.min + self.size * 0.5
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x
            && p.y >= self.min.y
            && p.x <= self.min.x + self.size.x
            && p.y <= self.min.y + self.size.y
    }

    /// Intersection of two rectangles, or `None` if they do not overlap.
    pub fn intersect(&self, other: &Rect2) -> Option<Rect2> {
        let min = self.min.max(other.min);
        let max = self.max().min(other.max());
        if max.x >= min.x && max.y >= min.y {
            Some(Rect2::new(min, max - min))
        } else {
            None
        }
    }
}

/// True if `id` is a `GuiObject` (Frame/Label/Button/...), i.e. laid out with UDim2.
pub fn is_gui_object(world: &World, id: InstanceId) -> bool {
    let reg = registry();
    matches!(
        (world.class_of(id), reg.find("GuiObject")),
        (Some(c), Some(base)) if reg.is_a(c, base)
    )
}

fn udim2_prop(world: &World, id: InstanceId, name: &str) -> UDim2 {
    match world.get_prop(id, name) {
        Some(Value::UDim2(u)) => *u,
        _ => UDim2::default(),
    }
}

/// The `AnchorPoint` of a GuiObject (defaults to top-left `(0, 0)`).
pub fn anchor_point(world: &World, id: InstanceId) -> Vec2 {
    match world.get_prop(id, "AnchorPoint") {
        Some(Value::Vec2(v)) => *v,
        _ => Vec2::ZERO,
    }
}

/// Absolute rect of the container `id` renders inside: its parent's absolute rect
/// when the parent is a GuiObject, otherwise the screen rect (for top-level GUIs).
pub fn parent_rect(world: &World, id: InstanceId, screen: Rect2) -> Rect2 {
    match world.parent(id) {
        Some(parent) if is_gui_object(world, parent) => {
            absolute_rect(world, parent, screen).unwrap_or(screen)
        }
        _ => screen,
    }
}

/// Compute the absolute pixel rectangle of a GuiObject, resolving `Position`,
/// `Size` and `AnchorPoint` against its ancestors up to `screen`.
pub fn absolute_rect(world: &World, id: InstanceId, screen: Rect2) -> Option<Rect2> {
    if !is_gui_object(world, id) {
        return None;
    }
    let parent = parent_rect(world, id, screen);
    Some(resolve_rect(
        udim2_prop(world, id, "Position"),
        udim2_prop(world, id, "Size"),
        anchor_point(world, id),
        parent,
    ))
}

/// Pure UDim2 → pixel resolution against a parent rect, including anchor offset.
pub fn resolve_rect(position: UDim2, size: UDim2, anchor: Vec2, parent: Rect2) -> Rect2 {
    let s = Vec2::new(size.x.resolve(parent.size.x), size.y.resolve(parent.size.y));
    let corner = Vec2::new(
        parent.min.x + position.x.resolve(parent.size.x),
        parent.min.y + position.y.resolve(parent.size.y),
    );
    let min = corner - s * anchor;
    Rect2::new(min, s)
}

/// Effective clip rectangle for `id`: the intersection of every ancestor that has
/// `ClipsDescendants = true`. `None` means the object is fully clipped away; a
/// return of `screen`-sized rect means unclipped.
pub fn clip_rect(world: &World, id: InstanceId, screen: Rect2) -> Option<Rect2> {
    let mut clip = screen;
    let mut cur = world.parent(id);
    while let Some(p) = cur {
        if !is_gui_object(world, p) {
            break;
        }
        if matches!(world.get_prop(p, "ClipsDescendants"), Some(Value::Bool(true))) {
            let pr = absolute_rect(world, p, screen)?;
            clip = clip.intersect(&pr)?;
        }
        cur = world.parent(p);
    }
    Some(clip)
}

/// Solve for the `Position`/`Size` UDim2 that place a GuiObject at `target`
/// (absolute pixels) while preserving the existing scale terms and anchor.
/// This is what editor move/resize use: they manipulate the absolute rect and
/// write the result back as pure-offset deltas on top of the scale terms.
pub fn solve_offsets(
    position: UDim2,
    size: UDim2,
    anchor: Vec2,
    parent: Rect2,
    target: Rect2,
) -> (UDim2, UDim2) {
    let new_size = UDim2::new(
        size.x.scale,
        target.size.x - parent.size.x * size.x.scale,
        size.y.scale,
        target.size.y - parent.size.y * size.y.scale,
    );
    // corner = min + size * anchor  (invert the anchor shift applied in resolve_rect)
    let corner = target.min + target.size * anchor;
    let new_pos = UDim2::new(
        position.x.scale,
        corner.x - parent.min.x - parent.size.x * position.x.scale,
        position.y.scale,
        corner.y - parent.min.y - parent.size.y * position.y.scale,
    );
    (new_pos, new_size)
}

/// Rewrite a UDim2 to express the same resolved pixels purely as offset (scale 0).
pub fn to_offset(value: UDim2, parent: Vec2) -> UDim2 {
    UDim2::new(
        0.0,
        value.x.resolve(parent.x),
        0.0,
        value.y.resolve(parent.y),
    )
}

/// Rewrite a UDim2 to express the same resolved pixels purely as scale (offset 0).
/// A zero parent extent leaves that axis as offset (scale is undefined there).
pub fn to_scale(value: UDim2, parent: Vec2) -> UDim2 {
    let axis = |u: crate::value::UDim, p: f32| {
        if p.abs() < f32::EPSILON {
            crate::value::UDim::new(0.0, u.resolve(p))
        } else {
            crate::value::UDim::new(u.resolve(p) / p, 0.0)
        }
    };
    UDim2 {
        x: axis(value.x, parent.x),
        y: axis(value.y, parent.y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use crate::world::{InstanceId, World};

    const SCREEN: Rect2 = Rect2 {
        min: Vec2::ZERO,
        size: Vec2::new(800.0, 600.0),
    };

    fn approx(a: Vec2, b: Vec2) {
        assert!((a - b).length() < 1e-3, "{a:?} != {b:?}");
    }

    fn frame(w: &mut World, parent: InstanceId) -> InstanceId {
        w.create("Frame", parent).unwrap()
    }

    fn set(w: &mut World, id: InstanceId, prop: &str, v: Value) {
        w.set_prop(id, prop, v).unwrap();
    }

    #[test]
    fn child_follows_parent_movement() {
        let mut w = World::new();
        let gui = w.gui().unwrap();
        let parent = frame(&mut w, gui);
        set(&mut w, parent, "Position", Value::UDim2(UDim2::from_offset(100.0, 100.0)));
        set(&mut w, parent, "Size", Value::UDim2(UDim2::from_offset(200.0, 200.0)));
        let child = frame(&mut w, parent);
        set(&mut w, child, "Position", Value::UDim2(UDim2::from_offset(10.0, 10.0)));
        set(&mut w, child, "Size", Value::UDim2(UDim2::from_offset(50.0, 50.0)));

        let before = absolute_rect(&w, child, SCREEN).unwrap();
        approx(before.min, Vec2::new(110.0, 110.0));

        // Move the parent: the child's absolute position shifts by the same delta.
        set(&mut w, parent, "Position", Value::UDim2(UDim2::from_offset(140.0, 130.0)));
        let after = absolute_rect(&w, child, SCREEN).unwrap();
        approx(after.min, Vec2::new(150.0, 140.0));
        approx(after.size, before.size);
    }

    #[test]
    fn child_scale_size_tracks_parent_resize() {
        let mut w = World::new();
        let gui = w.gui().unwrap();
        let parent = frame(&mut w, gui);
        set(&mut w, parent, "Position", Value::UDim2(UDim2::from_offset(0.0, 0.0)));
        set(&mut w, parent, "Size", Value::UDim2(UDim2::from_offset(200.0, 100.0)));
        let child = frame(&mut w, parent);
        // Half the parent on each axis.
        set(&mut w, child, "Size", Value::UDim2(UDim2::from_scale(0.5, 0.5)));
        set(&mut w, child, "Position", Value::UDim2(UDim2::default()));

        approx(absolute_rect(&w, child, SCREEN).unwrap().size, Vec2::new(100.0, 50.0));
        set(&mut w, parent, "Size", Value::UDim2(UDim2::from_offset(400.0, 300.0)));
        approx(absolute_rect(&w, child, SCREEN).unwrap().size, Vec2::new(200.0, 150.0));
    }

    #[test]
    fn offset_only_size_stays_fixed_when_parent_resizes() {
        let mut w = World::new();
        let gui = w.gui().unwrap();
        let parent = frame(&mut w, gui);
        set(&mut w, parent, "Size", Value::UDim2(UDim2::from_offset(200.0, 200.0)));
        let child = frame(&mut w, parent);
        set(&mut w, child, "Size", Value::UDim2(UDim2::from_offset(64.0, 64.0)));

        approx(absolute_rect(&w, child, SCREEN).unwrap().size, Vec2::new(64.0, 64.0));
        set(&mut w, parent, "Size", Value::UDim2(UDim2::from_offset(500.0, 40.0)));
        approx(absolute_rect(&w, child, SCREEN).unwrap().size, Vec2::new(64.0, 64.0));
    }

    #[test]
    fn anchor_center_centers_on_position() {
        let mut w = World::new();
        let gui = w.gui().unwrap();
        let f = frame(&mut w, gui);
        set(&mut w, f, "Size", Value::UDim2(UDim2::from_offset(100.0, 80.0)));
        set(&mut w, f, "Position", Value::UDim2(UDim2::from_offset(400.0, 300.0)));
        set(&mut w, f, "AnchorPoint", Value::Vec2(Vec2::new(0.5, 0.5)));

        let r = absolute_rect(&w, f, SCREEN).unwrap();
        // The anchored point (0.5,0.5) sits exactly on the position.
        approx(r.center(), Vec2::new(400.0, 300.0));
        approx(r.min, Vec2::new(350.0, 260.0));
    }

    #[test]
    fn nested_gui_objects_compose() {
        let mut w = World::new();
        let gui = w.gui().unwrap();
        let a = frame(&mut w, gui);
        set(&mut w, a, "Position", Value::UDim2(UDim2::from_offset(50.0, 50.0)));
        set(&mut w, a, "Size", Value::UDim2(UDim2::from_offset(400.0, 400.0)));
        let b = frame(&mut w, a);
        set(&mut w, b, "Position", Value::UDim2(UDim2::from_scale(0.5, 0.5)));
        set(&mut w, b, "Size", Value::UDim2(UDim2::from_offset(100.0, 100.0)));
        let c = frame(&mut w, b);
        set(&mut w, c, "Position", Value::UDim2(UDim2::from_offset(10.0, 20.0)));
        set(&mut w, c, "Size", Value::UDim2(UDim2::from_offset(30.0, 30.0)));

        // b at a.min + a.size*0.5 = (250, 250); c at b.min + (10,20) = (260, 270).
        approx(absolute_rect(&w, b, SCREEN).unwrap().min, Vec2::new(250.0, 250.0));
        approx(absolute_rect(&w, c, SCREEN).unwrap().min, Vec2::new(260.0, 270.0));
    }

    #[test]
    fn drag_updates_offset_preserving_scale() {
        // Simulates an editor drag: move the absolute rect by (20, 0) and confirm
        // only the offset changes while the scale terms are preserved.
        let parent = Rect2::new(Vec2::ZERO, Vec2::new(400.0, 400.0));
        let pos = UDim2::new(0.5, 10.0, 0.0, 30.0);
        let size = UDim2::from_offset(80.0, 40.0);
        let anchor = Vec2::ZERO;
        let start = resolve_rect(pos, size, anchor, parent);

        let target = Rect2::new(start.min + Vec2::new(20.0, 0.0), start.size);
        let (new_pos, new_size) = solve_offsets(pos, size, anchor, parent, target);

        assert_eq!(new_pos.x.scale, 0.5);
        assert!((new_pos.x.offset - 30.0).abs() < 1e-3); // +20 on offset
        assert!((new_pos.y.offset - 30.0).abs() < 1e-3);
        assert_eq!(new_size, size);
        approx(resolve_rect(new_pos, new_size, anchor, parent).min, target.min);
    }

    #[test]
    fn resize_right_edge_updates_width_offset() {
        let parent = Rect2::new(Vec2::ZERO, Vec2::new(400.0, 400.0));
        let pos = UDim2::from_offset(50.0, 50.0);
        let size = UDim2::new(0.25, 20.0, 0.0, 60.0); // width = 120
        let anchor = Vec2::ZERO;
        let start = resolve_rect(pos, size, anchor, parent);
        assert!((start.size.x - 120.0).abs() < 1e-3);

        // Drag the right edge +30px: width grows, offset absorbs it, scale unchanged.
        let target = Rect2::new(start.min, start.size + Vec2::new(30.0, 0.0));
        let (new_pos, new_size) = solve_offsets(pos, size, anchor, parent, target);
        assert_eq!(new_pos, pos);
        assert_eq!(new_size.x.scale, 0.25);
        assert!((new_size.x.offset - 50.0).abs() < 1e-3); // 20 + 30
    }

    #[test]
    fn clip_rect_bounds_descendants() {
        let mut w = World::new();
        let gui = w.gui().unwrap();
        let clip = frame(&mut w, gui);
        set(&mut w, clip, "Position", Value::UDim2(UDim2::from_offset(0.0, 0.0)));
        set(&mut w, clip, "Size", Value::UDim2(UDim2::from_offset(100.0, 100.0)));
        set(&mut w, clip, "ClipsDescendants", Value::Bool(true));
        let child = frame(&mut w, clip);

        let r = clip_rect(&w, child, SCREEN).unwrap();
        approx(r.min, Vec2::ZERO);
        approx(r.size, Vec2::new(100.0, 100.0));
    }
}
