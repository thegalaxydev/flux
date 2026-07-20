//! Optional built-in 2D camera controller.
//!
//! A reusable, opt-in pan/zoom controller for the `Camera2D`, configured
//! entirely through instance properties (so it's data-driven and games that
//! want a fully scripted camera just leave `Controls` off — the default).
//!
//! When enabled it provides: keyboard/edge pan, middle-mouse drag, smooth
//! mouse-wheel zoom anchored on the cursor, and position/zoom bounds. The
//! runtime feeds it a [`CameraInput`] each step; all the math lives here,
//! decoupled from the input backend (like [`crate::animation`] /
//! [`crate::tilemap`]).

use glam::Vec2;

use crate::value::Value;
use crate::world::{InstanceId, World};

/// Per-frame input for [`update`], in screen pixels / normalized axes. Built by
/// the runtime from its input backend so this module stays backend-agnostic.
#[derive(Clone, Copy, Debug, Default)]
pub struct CameraInput {
    /// Keyboard pan axis, each component in `-1..1` (e.g. A/D -> x, W/S -> y).
    pub pan: Vec2,
    /// Mouse-wheel delta this frame (+ zooms in).
    pub scroll: f32,
    /// Middle-drag movement this frame, in screen pixels (zero when not dragging).
    pub drag: Vec2,
    /// Cursor position relative to the viewport's top-left, in pixels.
    pub mouse: Vec2,
    /// Viewport size in pixels.
    pub viewport: Vec2,
    /// Whether the cursor is inside the viewport (gates edge-scroll + zoom anchor).
    pub pointer_over: bool,
}

fn num(w: &World, id: InstanceId, name: &str, default: f32) -> f32 {
    match w.get_prop(id, name) {
        Some(Value::Number(n)) => *n as f32,
        _ => default,
    }
}

fn flag(w: &World, id: InstanceId, name: &str) -> bool {
    matches!(w.get_prop(id, name), Some(Value::Bool(true)))
}

fn vec2(w: &World, id: InstanceId, name: &str) -> Vec2 {
    match w.get_prop(id, name) {
        Some(Value::Vec2(v)) => *v,
        _ => Vec2::ZERO,
    }
}

/// The workspace's current `Camera2D`, if one is set and still alive.
fn current_camera(w: &World) -> Option<InstanceId> {
    let ws = w.workspace();
    match w.get_prop(ws, "CurrentCamera") {
        Some(Value::InstanceRef(Some(c))) if w.contains(*c) => Some(*c),
        _ => None,
    }
}

/// Prepare the current camera for a session: seed the smooth-zoom target from
/// its authored `Zoom`, so the first wheel tick eases from the right value.
pub fn init(world: &mut World) {
    if let Some(cam) = current_camera(world) {
        let zoom = num(world, cam, "Zoom", 1.0);
        let _ = world.set_prop(cam, "_ZoomTarget", Value::Number(zoom as f64));
    }
}

/// Advance the current camera by `dt` given this frame's [`CameraInput`].
/// No-op unless the camera's `Controls` property is enabled, so a scripted or
/// static camera is never disturbed.
pub fn update(world: &mut World, input: &CameraInput, dt: f64) {
    let Some(cam) = current_camera(world) else {
        return;
    };
    if !flag(world, cam, "Controls") {
        return;
    }

    let pos = vec2(world, cam, "Position");
    let zoom = num(world, cam, "Zoom", 1.0).max(1e-3);
    let pan_speed = num(world, cam, "PanSpeed", 800.0);
    let zoom_speed = num(world, cam, "ZoomSpeed", 0.15).max(0.0);
    let min_zoom = num(world, cam, "MinZoom", 0.1).max(1e-3);
    let max_zoom = num(world, cam, "MaxZoom", 8.0).max(min_zoom);
    let edge_scroll = flag(world, cam, "EdgeScroll");
    let bmin = vec2(world, cam, "BoundsMin");
    let bmax = vec2(world, cam, "BoundsMax");

    let mut z_target = num(world, cam, "_ZoomTarget", 0.0);
    if z_target <= 0.0 {
        z_target = zoom; // lazily initialize if init() didn't run
    }
    if input.scroll != 0.0 {
        // `scroll` is a raw pixel delta (~50/notch), not a notch count — convert
        // to notches and clamp, so one wheel tick is one gentle zoom step rather
        // than a jump straight to the limit.
        let notches = (input.scroll / 50.0).clamp(-3.0, 3.0);
        z_target *= (1.0 + zoom_speed).powf(notches);
    }
    z_target = z_target.clamp(min_zoom, max_zoom);

    // Ease the actual zoom toward the target (frame-rate independent).
    let t = 1.0 - (-12.0 * dt as f32).exp();
    let new_zoom = (zoom + (z_target - zoom) * t).clamp(min_zoom, max_zoom);

    let center = input.viewport * 0.5;
    let mut offset = pos;

    // Keep the world point under the cursor fixed as the zoom changes.
    if input.pointer_over && (new_zoom - zoom).abs() > 1e-6 {
        let world_under = pos + (input.mouse - center) / zoom;
        offset = world_under - (input.mouse - center) / new_zoom;
    }

    // Keyboard pan, plus edge-scroll when the cursor hugs a viewport edge.
    let mut pan = input.pan;
    if edge_scroll && input.pointer_over {
        const M: f32 = 24.0;
        if input.mouse.x < M {
            pan.x -= 1.0;
        } else if input.mouse.x > input.viewport.x - M {
            pan.x += 1.0;
        }
        if input.mouse.y < M {
            pan.y -= 1.0;
        } else if input.mouse.y > input.viewport.y - M {
            pan.y += 1.0;
        }
    }
    if pan != Vec2::ZERO {
        // Speed is in screen pixels/sec, so divide by zoom for a consistent feel.
        offset += pan.normalize_or_zero() * (pan_speed * dt as f32 / new_zoom);
    }

    // Middle-drag moves the world opposite the cursor, 1:1 in screen space.
    if input.drag != Vec2::ZERO {
        offset -= input.drag / new_zoom;
    }

    // Clamp to bounds on each axis where a positive range is configured.
    if bmax.x > bmin.x {
        offset.x = offset.x.clamp(bmin.x, bmax.x);
    }
    if bmax.y > bmin.y {
        offset.y = offset.y.clamp(bmin.y, bmax.y);
    }

    let _ = world.set_prop(cam, "Position", Value::Vec2(offset));
    let _ = world.set_prop(cam, "Zoom", Value::Number(new_zoom as f64));
    let _ = world.set_prop(cam, "_ZoomTarget", Value::Number(z_target as f64));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(controls: bool) -> (World, InstanceId) {
        let mut w = World::new();
        let cam = current_camera(&w).unwrap();
        w.set_prop(cam, "Controls", Value::Bool(controls)).unwrap();
        (w, cam)
    }

    fn input() -> CameraInput {
        CameraInput {
            viewport: Vec2::new(800.0, 600.0),
            mouse: Vec2::new(400.0, 300.0), // centre -> no zoom-anchor shift
            pointer_over: true,
            ..Default::default()
        }
    }

    #[test]
    fn disabled_camera_is_untouched() {
        let (mut w, cam) = setup(false);
        let inp = CameraInput { pan: Vec2::new(1.0, 0.0), ..input() };
        update(&mut w, &inp, 1.0);
        assert_eq!(vec2(&w, cam, "Position"), Vec2::ZERO);
    }

    #[test]
    fn keyboard_pan_moves_by_speed_over_time() {
        let (mut w, cam) = setup(true);
        w.set_prop(cam, "PanSpeed", Value::Number(100.0)).unwrap();
        let inp = CameraInput { pan: Vec2::new(1.0, 0.0), ..input() };
        update(&mut w, &inp, 1.0); // 100 px/s * 1s / zoom(1) = 100
        assert!((vec2(&w, cam, "Position").x - 100.0).abs() < 1e-3);
    }

    #[test]
    fn wheel_zoom_eases_toward_clamped_target() {
        let (mut w, cam) = setup(true);
        w.set_prop(cam, "MaxZoom", Value::Number(4.0)).unwrap();
        init(&mut w);
        // One wheel notch (~50px) is one gentle step, not a jump to the limit.
        let inp = CameraInput { scroll: 150.0, ..input() }; // 3 notches (clamped)
        update(&mut w, &inp, 0.1);
        let after_one = num(&w, cam, "Zoom", 1.0);
        assert!(after_one > 1.0 && after_one <= 4.0);
        // Keep scrolling in; zoom eases toward and clamps at MaxZoom, not past.
        for _ in 0..60 {
            update(&mut w, &CameraInput { scroll: 150.0, ..input() }, 0.1);
        }
        assert!((num(&w, cam, "Zoom", 1.0) - 4.0).abs() < 1e-2);
    }

    #[test]
    fn drag_moves_opposite_the_cursor() {
        let (mut w, cam) = setup(true);
        let inp = CameraInput { drag: Vec2::new(10.0, -6.0), ..input() };
        update(&mut w, &inp, 0.016);
        let p = vec2(&w, cam, "Position");
        assert!((p.x + 10.0).abs() < 1e-3 && (p.y - 6.0).abs() < 1e-3);
    }

    #[test]
    fn bounds_clamp_position() {
        let (mut w, cam) = setup(true);
        w.set_prop(cam, "PanSpeed", Value::Number(10000.0)).unwrap();
        w.set_prop(cam, "BoundsMin", Value::Vec2(Vec2::new(-50.0, -50.0)))
            .unwrap();
        w.set_prop(cam, "BoundsMax", Value::Vec2(Vec2::new(50.0, 50.0)))
            .unwrap();
        let inp = CameraInput { pan: Vec2::new(1.0, 0.0), ..input() };
        update(&mut w, &inp, 1.0);
        assert!((vec2(&w, cam, "Position").x - 50.0).abs() < 1e-3);
    }

    #[test]
    fn wheel_zoom_anchors_on_cursor() {
        let (mut w, cam) = setup(true);
        init(&mut w);
        // Cursor off-centre: zooming should shift the offset to keep the world
        // point under the cursor fixed.
        let inp = CameraInput {
            scroll: 100.0,
            mouse: Vec2::new(600.0, 300.0), // 200px right of centre
            ..input()
        };
        update(&mut w, &inp, 0.1);
        assert_ne!(vec2(&w, cam, "Position").x, 0.0);
    }
}
