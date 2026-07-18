mod texture;

use std::path::Path;

use egui::epaint::{Mesh, Vertex};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};
use flux_core::gui::{self, Rect2};
use flux_core::transform::SpriteXform;
use flux_core::{ClassId, Color, InstanceId, Rect as SrcRect, Value, World, registry};

pub use flux_core::animation::AnimationCache;
pub use texture::TextureCache;

/// Screen rect (top-left based) the GUI layer is laid out inside, as a [`Rect2`].
fn gui_screen(rect: Rect) -> Rect2 {
    Rect2::new(
        glam::vec2(rect.min.x, rect.min.y),
        glam::vec2(rect.width(), rect.height()),
    )
}

fn to_egui_rect(r: Rect2) -> Rect {
    Rect::from_min_size(egui::pos2(r.min.x, r.min.y), egui::vec2(r.size.x, r.size.y))
}

/// Absolute screen rect of a GuiObject, laid out inside `screen_rect`.
/// Returns `None` for non-GUI instances.
pub fn gui_absolute_rect(world: &World, id: InstanceId, screen_rect: Rect) -> Option<Rect> {
    gui::absolute_rect(world, id, gui_screen(screen_rect)).map(to_egui_rect)
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub offset: egui::Vec2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            offset: egui::Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

pub fn game_camera(world: &World) -> Option<Camera> {
    let ws = world.workspace();
    let Some(Value::InstanceRef(Some(cam))) = world.get_prop(ws, "CurrentCamera") else {
        return None;
    };
    let Some(Value::Vec2(pos)) = world.get_prop(*cam, "Position") else {
        return None;
    };
    let zoom = match world.get_prop(*cam, "Zoom") {
        Some(Value::Number(z)) => *z as f32,
        _ => 1.0,
    };
    Some(Camera {
        offset: egui::vec2(pos.x, pos.y),
        zoom: zoom.max(0.01),
    })
}

pub fn to_color(c: &Color) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
        (c.a * 255.0) as u8,
    )
}

const SELECT: Color32 = Color32::from_rgb(255, 200, 60);

/// Axis-aligned bounds of four screen-space corners.
fn screen_aabb(corners: &[Pos2; 4]) -> Rect {
    let mut r = Rect::from_min_max(corners[0], corners[0]);
    for p in &corners[1..] {
        r.extend_with(*p);
    }
    r
}

#[allow(clippy::too_many_arguments)]
pub fn draw_scene(
    painter: &Painter,
    ctx: &egui::Context,
    world: &World,
    textures: &mut TextureCache,
    anim: &mut flux_core::animation::AnimationCache,
    rect: Rect,
    camera: Camera,
    root: Option<&Path>,
    selection: Option<InstanceId>,
    pointer: Option<Pos2>,
    playing: bool,
) -> Vec<(InstanceId, Rect)> {
    let origin = rect.center();
    let to_screen = |x: f32, y: f32| origin + (egui::vec2(x, y) - camera.offset) * camera.zoom;

    // Both Sprites and AnimatedSprites render as textured quads, ordered by ZIndex.
    let mut nodes: Vec<(InstanceId, f64)> = world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| {
            matches!(
                world.class_name(id),
                Some("Sprite") | Some("AnimatedSprite")
            )
        })
        .map(|id| (id, zindex(world, id)))
        .collect();
    nodes.sort_by(|a, b| a.1.total_cmp(&b.1));

    let _ = selection; // selection adornments are drawn by the editor.
    let mut drawn = Vec::new();
    for (id, _) in nodes {
        let (Some(xf), Some(Value::Color(tint))) =
            (SpriteXform::read(world, id), world.get_prop(id, "Tint"))
        else {
            continue;
        };
        // Screen-space oriented corners (TL, TR, BR, BL), rotation + scale + pivot.
        let corners = xf.corners().map(|p| to_screen(p.x, p.y));
        let aabb = screen_aabb(&corners);
        let visible = matches!(world.get_prop(id, "Visible"), Some(Value::Bool(true)));
        let locked = matches!(world.get_prop(id, "Locked"), Some(Value::Bool(true)));
        // Only visible, unlocked nodes are click-selectable in the editor.
        if visible && !locked {
            drawn.push((id, aabb));
        }
        if !visible || !rect.intersects(aabb) {
            continue;
        }
        let tint_color = to_color(tint);

        // Resolve the texture + source rect. An AnimatedSprite gets both from its
        // current frame (single source of truth); a Sprite from its own props.
        let (handle, src) = if world.class_name(id) == Some("AnimatedSprite") {
            match root.and_then(|r| flux_core::animation::current_frame(world, anim, r, id)) {
                Some((tex, rect)) => {
                    let h = match (tex, root) {
                        (Some(p), Some(r)) if !p.is_empty() => textures.get(ctx, r, &p),
                        _ => None,
                    };
                    (h, rect)
                }
                None => (None, SrcRect::default()),
            }
        } else {
            let h = root.and_then(|r| match world.get_prop(id, "Texture") {
                Some(Value::Asset(p)) if !p.is_empty() => textures.get(ctx, r, p),
                _ => None,
            });
            let src = match world.get_prop(id, "SourceRect") {
                Some(Value::Rect(r)) => *r,
                _ => SrcRect::default(),
            };
            (h, src)
        };

        if let Some(handle) = handle {
            let flip_x = matches!(world.get_prop(id, "FlipX"), Some(Value::Bool(true)));
            let flip_y = matches!(world.get_prop(id, "FlipY"), Some(Value::Bool(true)));
            // Map the SourceRect (texture pixels) to UVs; a whole-texture rect
            // (zero size) uses the full 0..1 range. Flips swap the edges.
            let sz = handle.size();
            let (tw, th) = (sz[0] as f32, sz[1] as f32);
            let (mut u0, mut v0, mut u1, mut v1) = if src.is_whole() || tw <= 0.0 || th <= 0.0 {
                (0.0, 0.0, 1.0, 1.0)
            } else {
                (
                    src.x / tw,
                    src.y / th,
                    (src.x + src.w) / tw,
                    (src.y + src.h) / th,
                )
            };
            if flip_x {
                std::mem::swap(&mut u0, &mut u1);
            }
            if flip_y {
                std::mem::swap(&mut v0, &mut v1);
            }
            // UVs match corner order TL, TR, BR, BL.
            let uvs = [
                egui::pos2(u0, v0),
                egui::pos2(u1, v0),
                egui::pos2(u1, v1),
                egui::pos2(u0, v1),
            ];
            let mut mesh = Mesh::with_texture(handle.id());
            for (pos, uv) in corners.iter().zip(uvs) {
                mesh.vertices.push(Vertex {
                    pos: *pos,
                    uv,
                    color: tint_color,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        } else {
            painter.add(Shape::convex_polygon(
                corners.to_vec(),
                tint_color,
                Stroke::NONE,
            ));
        }
    }

    let gui_rects = draw_gui(
        painter, ctx, textures, root, world, rect, playing, pointer, selection,
    );
    drawn.extend(gui_rects);
    drawn
}

/// Draws the GUI layer and returns the absolute rect of each visible GuiObject in
/// ascending render order (so the last entry that contains a point is topmost).
#[allow(clippy::too_many_arguments)]
fn draw_gui(
    painter: &Painter,
    ctx: &egui::Context,
    textures: &mut TextureCache,
    root: Option<&Path>,
    world: &World,
    rect: Rect,
    playing: bool,
    pointer: Option<Pos2>,
    selection: Option<InstanceId>,
) -> Vec<(InstanceId, Rect)> {
    let mut hit = Vec::new();
    let Some(gui) = world.gui() else { return hit };
    let screen = gui_screen(rect);
    let button = registry().find("Button");
    let is_a = |id: InstanceId, class: Option<ClassId>| matches!((world.class_of(id), class), (Some(c), Some(t)) if registry().is_a(c, t));

    let mut items: Vec<(InstanceId, f64)> = world
        .descendants(gui)
        .into_iter()
        .filter(|&id| gui::is_gui_object(world, id))
        .map(|id| (id, zindex(world, id)))
        .collect();
    // Stable sort keeps sibling document order as the tiebreak within a ZIndex.
    items.sort_by(|a, b| a.1.total_cmp(&b.1));

    for (id, _) in items {
        if !matches!(world.get_prop(id, "Visible"), Some(Value::Bool(true))) {
            continue;
        }
        let Some(abs) = gui::absolute_rect(world, id, screen) else {
            continue;
        };
        // Descendants of a clipping ancestor are culled/clipped to that region.
        let Some(clip) = gui::clip_rect(world, id, screen) else {
            continue;
        };
        let r = to_egui_rect(abs);
        hit.push((id, r));

        let clipped = painter.with_clip_rect(to_egui_rect(clip));
        draw_gui_object(
            &clipped,
            ctx,
            textures,
            root,
            world,
            id,
            r,
            is_a(id, button),
            playing,
            pointer,
        );

        if !playing && selection == Some(id) {
            painter.rect_stroke(
                r.expand(1.0),
                3.0,
                Stroke::new(1.5, SELECT),
                StrokeKind::Outside,
            );
        }
    }
    hit
}

#[allow(clippy::too_many_arguments)]
fn draw_gui_object(
    painter: &Painter,
    ctx: &egui::Context,
    textures: &mut TextureCache,
    root: Option<&Path>,
    world: &World,
    id: InstanceId,
    r: Rect,
    is_button: bool,
    playing: bool,
    pointer: Option<Pos2>,
) {
    let transparency = match world.get_prop(id, "BackgroundTransparency") {
        Some(Value::Number(n)) => (*n as f32).clamp(0.0, 1.0),
        _ => 0.0,
    };
    if let Some(Value::Color(bg)) = world.get_prop(id, "BackgroundColor") {
        let alpha = bg.a * (1.0 - transparency);
        if alpha > 0.0 {
            painter.rect_filled(r, 3.0, to_color(&Color { a: alpha, ..*bg }));
        }
    }
    // ImageFrame: draw its image, 9-sliced when SliceMargins are set.
    if let (Some(root), Some(Value::Asset(path))) = (root, world.get_prop(id, "Image")) {
        if !path.is_empty() {
            if let Some(handle) = textures.get(ctx, root, path) {
                let sz = handle.size();
                let m = match world.get_prop(id, "SliceMargins") {
                    Some(Value::Rect(m)) => (m.x, m.y, m.w, m.h),
                    _ => (0.0, 0.0, 0.0, 0.0),
                };
                let tint = match world.get_prop(id, "ImageColor") {
                    Some(Value::Color(c)) => to_color(c),
                    _ => Color32::WHITE,
                };
                let mut mesh = Mesh::with_texture(handle.id());
                for (dest, uv) in nine_slice_quads(r, sz[0] as f32, sz[1] as f32, m) {
                    mesh.add_rect_with_uv(dest, uv, tint);
                }
                painter.add(Shape::mesh(mesh));
            }
        }
    }
    if is_button && playing && pointer.is_some_and(|p| r.contains(p)) {
        painter.rect_stroke(r, 3.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
    }
    if let Some(Value::String(text)) = world.get_prop(id, "Text") {
        let color = match world.get_prop(id, "TextColor") {
            Some(Value::Color(c)) => to_color(c),
            _ => Color32::WHITE,
        };
        let ts = match world.get_prop(id, "TextSize") {
            Some(Value::Number(n)) => *n as f32,
            _ => 16.0,
        };
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(ts),
            color,
        );
    }
}

/// Up to 9 `(dest, uv)` quads for a 9-slice of a `tw x th` texture into `dest`,
/// with border insets `(left, top, right, bottom)` in source pixels. Zero-area
/// quads are omitted, so all-zero margins collapse to a single stretched quad.
/// Opposite borders are scaled down if they would overlap within `dest`.
fn nine_slice_quads(dest: Rect, tw: f32, th: f32, m: (f32, f32, f32, f32)) -> Vec<(Rect, Rect)> {
    if tw <= 0.0 || th <= 0.0 {
        let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
        return vec![(dest, uv)];
    }
    // Borders are (left, top, right, bottom); none can exceed the image.
    let l = m.0.clamp(0.0, tw);
    let t = m.1.clamp(0.0, th);
    let r = m.2.clamp(0.0, tw);
    let b = m.3.clamp(0.0, th);
    let (dl, dr) = fit_pair(l, r, dest.width());
    let (dt, db) = fit_pair(t, b, dest.height());
    let sx = [0.0, l, (tw - r).max(l), tw];
    let sy = [0.0, t, (th - b).max(t), th];
    let dx = [
        dest.left(),
        dest.left() + dl,
        dest.right() - dr,
        dest.right(),
    ];
    let dy = [
        dest.top(),
        dest.top() + dt,
        dest.bottom() - db,
        dest.bottom(),
    ];
    let mut out = Vec::new();
    for row in 0..3 {
        for col in 0..3 {
            let d = Rect::from_min_max(
                Pos2::new(dx[col], dy[row]),
                Pos2::new(dx[col + 1], dy[row + 1]),
            );
            if d.width() <= 0.01 || d.height() <= 0.01 {
                continue;
            }
            let uv = Rect::from_min_max(
                Pos2::new(sx[col] / tw, sy[row] / th),
                Pos2::new(sx[col + 1] / tw, sy[row + 1] / th),
            );
            out.push((d, uv));
        }
    }
    out
}

/// Clamp a pair of opposite borders so their sum doesn't exceed `avail`.
fn fit_pair(a: f32, b: f32, avail: f32) -> (f32, f32) {
    let (a, b) = (a.max(0.0), b.max(0.0));
    if a + b > avail && a + b > 0.0 {
        let s = avail / (a + b);
        (a * s, b * s)
    } else {
        (a, b)
    }
}

fn zindex(world: &World, id: InstanceId) -> f64 {
    match world.get_prop(id, "ZIndex") {
        Some(Value::Number(z)) => *z,
        _ => 0.0,
    }
}

#[cfg(test)]
mod nine_slice_tests {
    use super::nine_slice_quads;
    use egui::{Pos2, Rect, vec2};

    fn dest(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(w, h))
    }

    #[test]
    fn margins_produce_nine_quads_with_fixed_corners() {
        let quads = nine_slice_quads(dest(100.0, 100.0), 32.0, 32.0, (8.0, 8.0, 8.0, 8.0));
        assert_eq!(quads.len(), 9);
        // First quad is the top-left corner: 8x8 in dest, uv 0..0.25.
        let (d, uv) = quads[0];
        assert!((d.width() - 8.0).abs() < 1e-3 && (d.height() - 8.0).abs() < 1e-3);
        assert!((uv.min.x - 0.0).abs() < 1e-3 && (uv.max.x - 0.25).abs() < 1e-3);
        // Corners keep source size while the panel is much larger, so the
        // centre stretches: exactly one dest quad is 84x84 (100 - 8 - 8).
        let center = quads
            .iter()
            .find(|(d, _)| (d.width() - 84.0).abs() < 1e-3 && (d.height() - 84.0).abs() < 1e-3);
        assert!(center.is_some(), "missing stretched centre quad");
    }

    #[test]
    fn zero_margins_collapse_to_a_single_stretched_quad() {
        let quads = nine_slice_quads(dest(50.0, 40.0), 16.0, 16.0, (0.0, 0.0, 0.0, 0.0));
        assert_eq!(quads.len(), 1);
        let (d, uv) = quads[0];
        assert_eq!(d, dest(50.0, 40.0));
        assert!((uv.max.x - 1.0).abs() < 1e-3 && (uv.max.y - 1.0).abs() < 1e-3);
    }

    #[test]
    fn oversized_borders_scale_down_to_fit_dest() {
        // Borders of 40+40 can't fit a 50px-wide panel; they scale to 25+25.
        let quads = nine_slice_quads(dest(50.0, 200.0), 64.0, 64.0, (40.0, 10.0, 40.0, 10.0));
        let left_col_max = quads
            .iter()
            .map(|(d, _)| d.min.x + (d.width().min(25.01)))
            .fold(0.0_f32, f32::max);
        // No dest quad should extend a left border past the midpoint (25).
        let widest_left = quads
            .iter()
            .filter(|(d, _)| d.min.x < 1.0)
            .map(|(d, _)| d.width())
            .fold(0.0_f32, f32::max);
        assert!(
            widest_left <= 25.0 + 1e-3,
            "left border not clamped: {widest_left}"
        );
        let _ = left_col_max;
    }
}
