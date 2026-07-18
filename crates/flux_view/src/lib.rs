mod texture;

use std::path::Path;

use egui::epaint::{Mesh, Vertex};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};
use flux_core::gui::{self, Rect2};
use flux_core::transform::SpriteXform;
use flux_core::{ClassId, Color, InstanceId, Rect as SrcRect, Value, World, registry};

pub use texture::TextureCache;

/// Screen rect (top-left based) the GUI layer is laid out inside, as a [`Rect2`].
fn gui_screen(rect: Rect) -> Rect2 {
    Rect2::new(
        glam::vec2(rect.min.x, rect.min.y),
        glam::vec2(rect.width(), rect.height()),
    )
}

fn to_egui_rect(r: Rect2) -> Rect {
    Rect::from_min_size(
        egui::pos2(r.min.x, r.min.y),
        egui::vec2(r.size.x, r.size.y),
    )
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

pub fn draw_scene(
    painter: &Painter,
    ctx: &egui::Context,
    world: &World,
    textures: &mut TextureCache,
    rect: Rect,
    camera: Camera,
    root: Option<&Path>,
    selection: Option<InstanceId>,
    pointer: Option<Pos2>,
    playing: bool,
) -> Vec<(InstanceId, Rect)> {
    let origin = rect.center();
    let to_screen = |x: f32, y: f32| origin + (egui::vec2(x, y) - camera.offset) * camera.zoom;

    let mut sprites: Vec<(InstanceId, f64)> = world
        .descendants(world.workspace())
        .into_iter()
        .filter(|&id| world.class_name(id) == Some("Sprite"))
        .map(|id| (id, zindex(world, id)))
        .collect();
    sprites.sort_by(|a, b| a.1.total_cmp(&b.1));

    let _ = selection; // sprite selection adornments are drawn by the editor.
    let mut drawn = Vec::new();
    for (id, _) in sprites {
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
        // Only visible, unlocked sprites are click-selectable in the editor.
        if visible && !locked {
            drawn.push((id, aabb));
        }

        if !visible || !rect.intersects(aabb) {
            continue;
        }
        let tint_color = to_color(tint);
        let tex = root.and_then(|root| match world.get_prop(id, "Texture") {
            Some(Value::Asset(p)) if !p.is_empty() => textures.get(ctx, root, p),
            _ => None,
        });
        if let Some(handle) = tex {
            let flip_x = matches!(world.get_prop(id, "FlipX"), Some(Value::Bool(true)));
            let flip_y = matches!(world.get_prop(id, "FlipY"), Some(Value::Bool(true)));
            // Map the SourceRect (in texture pixels) to UVs; a whole-texture
            // rect (zero size) uses the full 0..1 range. Flips swap the edges.
            let src = match world.get_prop(id, "SourceRect") {
                Some(Value::Rect(r)) => *r,
                _ => SrcRect::default(),
            };
            let sz = handle.size();
            let (tw, th) = (sz[0] as f32, sz[1] as f32);
            let (mut u0, mut v0, mut u1, mut v1) = if src.is_whole() || tw <= 0.0 || th <= 0.0 {
                (0.0, 0.0, 1.0, 1.0)
            } else {
                (src.x / tw, src.y / th, (src.x + src.w) / tw, (src.y + src.h) / th)
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
                mesh.vertices.push(Vertex { pos: *pos, uv, color: tint_color });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        } else {
            painter.add(Shape::convex_polygon(corners.to_vec(), tint_color, Stroke::NONE));
        }
    }

    let gui_rects = draw_gui(painter, world, rect, playing, pointer, selection);
    drawn.extend(gui_rects);
    drawn
}

/// Draws the GUI layer and returns the absolute rect of each visible GuiObject in
/// ascending render order (so the last entry that contains a point is topmost).
fn draw_gui(
    painter: &Painter,
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
    let is_a = |id: InstanceId, class: Option<ClassId>| {
        matches!((world.class_of(id), class), (Some(c), Some(t)) if registry().is_a(c, t))
    };

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
        draw_gui_object(&clipped, world, id, r, is_a(id, button), playing, pointer);

        if !playing && selection == Some(id) {
            painter.rect_stroke(r.expand(1.0), 3.0, Stroke::new(1.5, SELECT), StrokeKind::Outside);
        }
    }
    hit
}

fn draw_gui_object(
    painter: &Painter,
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
        painter.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(ts), color);
    }
}

fn zindex(world: &World, id: InstanceId) -> f64 {
    match world.get_prop(id, "ZIndex") {
        Some(Value::Number(z)) => *z,
        _ => 0.0,
    }
}
