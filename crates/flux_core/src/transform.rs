//! 2D sprite transforms for the scene editor: resolving Position/Size/Scale/
//! Rotation/Pivot into an oriented box, hit-testing it, and the pure math the
//! editor's move/resize tools use.
//!
//! Like [`crate::gui`], this is UI-framework agnostic (plain [`glam::Vec2`]) so
//! the renderer, editor, and tests share one implementation. Rotation is stored
//! in **degrees**; the pivot is a fraction of the sprite's local box where
//! `(0.5, 0.5)` is the centre. `Position` is the world location of the pivot, so
//! rotation and scaling happen about the pivot.

use glam::Vec2;

use crate::gui::Rect2;
use crate::value::Value;
use crate::world::{InstanceId, World};

/// Resolved transform of a sprite-like instance (has both `Position` and `Size`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteXform {
    pub position: Vec2,
    pub size: Vec2,
    pub scale: Vec2,
    pub rotation: f32,
    pub pivot: Vec2,
}

fn vec2_prop(world: &World, id: InstanceId, name: &str, default: Vec2) -> Vec2 {
    match world.get_prop(id, name) {
        Some(Value::Vec2(v)) => *v,
        _ => default,
    }
}

impl SpriteXform {
    /// Read the transform of `id`, or `None` if it isn't a sprite-like object
    /// (needs both `Position` and `Size`).
    pub fn read(world: &World, id: InstanceId) -> Option<Self> {
        let position = match world.get_prop(id, "Position") {
            Some(Value::Vec2(v)) => *v,
            _ => return None,
        };
        let size = match world.get_prop(id, "Size") {
            Some(Value::Vec2(v)) => *v,
            _ => return None,
        };
        Some(Self {
            position,
            size,
            scale: vec2_prop(world, id, "Scale", Vec2::ONE),
            rotation: match world.get_prop(id, "Rotation") {
                Some(Value::Number(n)) => *n as f32,
                _ => 0.0,
            },
            pivot: vec2_prop(world, id, "Pivot", Vec2::splat(0.5)),
        })
    }

    /// True if `id` can be transformed (is sprite-like).
    pub fn exists(world: &World, id: InstanceId) -> bool {
        Self::read(world, id).is_some()
    }

    /// Size after applying `Scale` — the on-screen box before rotation.
    pub fn effective_size(&self) -> Vec2 {
        self.size * self.scale
    }

    fn sin_cos(&self) -> (f32, f32) {
        self.rotation.to_radians().sin_cos()
    }

    /// Local box corner offsets from the pivot, order TL, TR, BR, BL.
    fn local_corners(&self) -> [Vec2; 4] {
        let e = self.effective_size();
        let x0 = -self.pivot.x * e.x;
        let x1 = (1.0 - self.pivot.x) * e.x;
        let y0 = -self.pivot.y * e.y;
        let y1 = (1.0 - self.pivot.y) * e.y;
        [
            Vec2::new(x0, y0),
            Vec2::new(x1, y0),
            Vec2::new(x1, y1),
            Vec2::new(x0, y1),
        ]
    }

    /// World-space corners (rotated about the pivot), order TL, TR, BR, BL.
    pub fn corners(&self) -> [Vec2; 4] {
        let (s, c) = self.sin_cos();
        self.local_corners().map(|p| {
            self.position + Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c)
        })
    }

    /// Axis-aligned bounding box of the rotated corners.
    pub fn aabb(&self) -> Rect2 {
        let cs = self.corners();
        let mut min = cs[0];
        let mut max = cs[0];
        for p in &cs[1..] {
            min = min.min(*p);
            max = max.max(*p);
        }
        Rect2::new(min, max - min)
    }

    /// Oriented hit-test: is world point `p` inside the rotated box?
    pub fn contains(&self, p: Vec2) -> bool {
        let (s, c) = self.sin_cos();
        let d = p - self.position;
        // Rotate the point into the sprite's local (unrotated) frame.
        let local = Vec2::new(d.x * c + d.y * s, -d.x * s + d.y * c);
        let e = self.effective_size();
        local.x >= -self.pivot.x * e.x
            && local.x <= (1.0 - self.pivot.x) * e.x
            && local.y >= -self.pivot.y * e.y
            && local.y <= (1.0 - self.pivot.y) * e.y
    }

    /// Rotate a world vector into the sprite's local frame.
    fn to_local_vec(&self, world: Vec2) -> Vec2 {
        let (s, c) = self.sin_cos();
        Vec2::new(world.x * c + world.y * s, -world.x * s + world.y * c)
    }

    fn rotate_local(&self, local: Vec2) -> Vec2 {
        let (s, c) = self.sin_cos();
        Vec2::new(local.x * c - local.y * s, local.x * s + local.y * c)
    }
}

/// Round a world position to the nearest grid multiple (per axis). `grid <= 0`
/// returns the value unchanged.
pub fn snap_to_grid(v: Vec2, grid: f32) -> Vec2 {
    if grid <= 0.0 {
        v
    } else {
        Vec2::new((v.x / grid).round() * grid, (v.y / grid).round() * grid)
    }
}

/// Result of a resize: the new base `Size` (undoing `Scale`) and new `Position`
/// (moved so the anchored edge/corner or centre stays put).
pub struct ResizeResult {
    pub size: Vec2,
    pub position: Vec2,
}

/// Smallest allowed effective extent so a sprite can't collapse or invert.
const MIN_EXTENT: f32 = 1.0;

/// Resize `start` by dragging a handle. `dir` is the handle direction in local
/// axes: each component is -1 (min edge), 0 (unaffected), or +1 (max edge).
/// `world_delta` is the pointer movement in world space. `aspect` preserves the
/// starting ratio; `center` resizes symmetrically about the pivot-centre.
pub fn resize(start: &SpriteXform, dir: Vec2, world_delta: Vec2, aspect: bool, center: bool) -> ResizeResult {
    let e0 = start.effective_size();
    let d = start.to_local_vec(world_delta);
    let factor = if center { 2.0 } else { 1.0 };

    let mut e = e0;
    if dir.x > 0.0 {
        e.x = e0.x + d.x * factor;
    } else if dir.x < 0.0 {
        e.x = e0.x - d.x * factor;
    }
    if dir.y > 0.0 {
        e.y = e0.y + d.y * factor;
    } else if dir.y < 0.0 {
        e.y = e0.y - d.y * factor;
    }

    if aspect && e0.x.abs() > f32::EPSILON && e0.y.abs() > f32::EPSILON {
        let ratio = e0.x / e0.y;
        let corner = dir.x != 0.0 && dir.y != 0.0;
        if corner || dir.x != 0.0 {
            e.y = e.x / ratio;
        } else {
            e.x = e.y * ratio;
        }
    }

    e.x = e.x.max(MIN_EXTENT);
    e.y = e.y.max(MIN_EXTENT);

    // Keep the anchor fixed in world space. Anchor fraction is the opposite edge
    // on active axes; on inactive axes use the pivot so that axis' position holds.
    let anchor = if center {
        Vec2::splat(0.5)
    } else {
        Vec2::new(
            if dir.x > 0.0 { 0.0 } else if dir.x < 0.0 { 1.0 } else { start.pivot.x },
            if dir.y > 0.0 { 0.0 } else if dir.y < 0.0 { 1.0 } else { start.pivot.y },
        )
    };
    let off0 = (anchor - start.pivot) * e0;
    let off1 = (anchor - start.pivot) * e;
    let world_anchor = start.position + start.rotate_local(off0);
    let position = world_anchor - start.rotate_local(off1);

    let scale = Vec2::new(
        if start.scale.x.abs() > f32::EPSILON { start.scale.x } else { 1.0 },
        if start.scale.y.abs() > f32::EPSILON { start.scale.y } else { 1.0 },
    );
    ResizeResult {
        size: e / scale,
        position,
    }
}

/// Snap an angle (degrees) to the nearest `step` degrees.
pub fn snap_angle(deg: f32, step: f32) -> f32 {
    if step <= 0.0 {
        deg
    } else {
        (deg / step).round() * step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Vec2, b: Vec2) {
        assert!((a - b).length() < 1e-3, "{a:?} != {b:?}");
    }

    fn xform(position: Vec2, size: Vec2) -> SpriteXform {
        SpriteXform {
            position,
            size,
            scale: Vec2::ONE,
            rotation: 0.0,
            pivot: Vec2::splat(0.5),
        }
    }

    #[test]
    fn corners_and_aabb_centered() {
        let xf = xform(Vec2::new(100.0, 100.0), Vec2::new(40.0, 20.0));
        let c = xf.corners();
        approx(c[0], Vec2::new(80.0, 90.0)); // TL
        approx(c[2], Vec2::new(120.0, 110.0)); // BR
        let bb = xf.aabb();
        approx(bb.min, Vec2::new(80.0, 90.0));
        approx(bb.size, Vec2::new(40.0, 20.0));
        assert!(xf.contains(Vec2::new(100.0, 100.0)));
        assert!(!xf.contains(Vec2::new(79.0, 100.0)));
    }

    #[test]
    fn scale_grows_effective_box() {
        let mut xf = xform(Vec2::ZERO, Vec2::new(50.0, 50.0));
        xf.scale = Vec2::new(2.0, 1.0);
        approx(xf.effective_size(), Vec2::new(100.0, 50.0));
        approx(xf.aabb().size, Vec2::new(100.0, 50.0));
    }

    #[test]
    fn rotation_90_swaps_aabb_extents() {
        let mut xf = xform(Vec2::ZERO, Vec2::new(40.0, 20.0));
        xf.rotation = 90.0;
        approx(xf.aabb().size, Vec2::new(20.0, 40.0));
        // Centre-pivot rotation keeps the centre on the position.
        approx(xf.aabb().center(), Vec2::ZERO);
    }

    #[test]
    fn pivot_changes_rotation_center() {
        let mut center = xform(Vec2::ZERO, Vec2::new(40.0, 40.0));
        center.rotation = 90.0;
        let mut corner = center;
        corner.pivot = Vec2::ZERO;
        // Centre pivot keeps the box centred on the position; corner pivot does not.
        approx(center.aabb().center(), Vec2::ZERO);
        assert!(corner.aabb().center().length() > 1.0);
        // Rotating about the top-left corner keeps that corner at the position.
        assert!(corner.corners().iter().any(|c| c.length() < 1e-3));
    }

    #[test]
    fn oriented_hit_test_respects_rotation() {
        let mut xf = xform(Vec2::ZERO, Vec2::new(80.0, 20.0));
        xf.rotation = 90.0;
        // After a 90° turn the long axis is vertical.
        assert!(xf.contains(Vec2::new(0.0, 35.0)));
        assert!(!xf.contains(Vec2::new(35.0, 0.0)));
    }

    #[test]
    fn grid_snapping() {
        approx(snap_to_grid(Vec2::new(17.0, -3.0), 10.0), Vec2::new(20.0, 0.0));
        approx(snap_to_grid(Vec2::new(17.0, -3.0), 0.0), Vec2::new(17.0, -3.0));
    }

    #[test]
    fn resize_right_edge_keeps_left_fixed() {
        let start = xform(Vec2::ZERO, Vec2::new(100.0, 50.0));
        let r = resize(&start, Vec2::new(1.0, 0.0), Vec2::new(30.0, 0.0), false, false);
        approx(r.size, Vec2::new(130.0, 50.0));
        // Left edge was at x = -50 and must stay there; new box is [-50, 80].
        approx(r.position, Vec2::new(15.0, 0.0));
    }

    #[test]
    fn resize_from_center_is_symmetric() {
        let start = xform(Vec2::ZERO, Vec2::new(100.0, 50.0));
        let r = resize(&start, Vec2::new(1.0, 0.0), Vec2::new(30.0, 0.0), false, true);
        approx(r.size, Vec2::new(160.0, 50.0));
        approx(r.position, Vec2::ZERO); // centre stays put
    }

    #[test]
    fn resize_corner_with_aspect_preserves_ratio() {
        let start = xform(Vec2::ZERO, Vec2::new(100.0, 50.0)); // ratio 2:1
        let r = resize(&start, Vec2::new(1.0, 1.0), Vec2::new(40.0, 0.0), true, false);
        approx(r.size, Vec2::new(140.0, 70.0));
    }

    #[test]
    fn resize_undoes_scale_into_base_size() {
        let mut start = xform(Vec2::ZERO, Vec2::new(50.0, 50.0));
        start.scale = Vec2::new(2.0, 2.0); // effective 100x100
        let r = resize(&start, Vec2::new(1.0, 0.0), Vec2::new(20.0, 0.0), false, false);
        // Effective width 120 with scale 2 -> base size 60.
        approx(r.size, Vec2::new(60.0, 50.0));
    }

    #[test]
    fn angle_snapping() {
        assert_eq!(snap_angle(7.0, 15.0), 0.0);
        assert_eq!(snap_angle(8.0, 15.0), 15.0);
        assert_eq!(snap_angle(63.0, 15.0), 60.0);
    }
}
