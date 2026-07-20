//! The game's overlay renderer: draws `Building` footprints on the tilemap.
//! Registered with `flux_view::register_overlay`, so the engine stays ignorant
//! of the `Building` node type.

use egui::{Painter, Pos2, Shape, Stroke};

use flux_core::{Color, InstanceId, Value, World};
use flux_view::{RenderCtx, to_color};

fn numf(world: &World, id: InstanceId, name: &str, default: f32) -> f32 {
    match world.get_prop(id, name) {
        Some(Value::Number(n)) => *n as f32,
        _ => default,
    }
}

fn screen_aabb(pts: &[Pos2]) -> egui::Rect {
    let mut r = egui::Rect::from_min_max(pts[0], pts[0]);
    for p in &pts[1..] {
        r.extend_with(*p);
    }
    r
}

pub(crate) fn overlay(painter: &Painter, world: &World, ctx: &RenderCtx) {
    for tm in world.descendants(world.workspace()) {
        if world.class_name(tm) != Some("Tilemap") {
            continue;
        }
        let pos = match world.get_prop(tm, "Position") {
            Some(Value::Vec2(p)) => *p,
            _ => glam::Vec2::ZERO,
        };
        let tw = numf(world, tm, "TileWidth", 64.0).max(1.0);
        let th = numf(world, tm, "TileHeight", 32.0).max(1.0);

        let mut buildings: Vec<(InstanceId, i32, i32, i32, i32)> = world
            .children(tm)
            .iter()
            .copied()
            .filter(|&c| world.class_name(c) == Some("Building"))
            .filter_map(|c| {
                let cell = match world.get_prop(c, "Cell") {
                    Some(Value::Vec2(v)) => *v,
                    _ => return None,
                };
                let fp = match world.get_prop(c, "Footprint") {
                    Some(Value::Vec2(v)) => *v,
                    _ => glam::Vec2::ONE,
                };
                Some((c, cell.x as i32, cell.y as i32, (fp.x as i32).max(1), (fp.y as i32).max(1)))
            })
            .collect();
        // Back-to-front: smaller (col + row) is farther from the camera.
        buildings.sort_by_key(|&(_, col, row, _, _)| col + row);

        for (bid, col, row, w, h) in buildings {
            if !matches!(world.get_prop(bid, "Visible"), Some(Value::Bool(true))) {
                continue;
            }
            let tc = |c: i32, r: i32, i: usize| {
                let p = flux_core::tilemap::tile_corners(c, r, tw, th)[i];
                ctx.to_screen(glam::vec2(pos.x + p.x, pos.y + p.y))
            };
            let quad = vec![
                tc(col, row, 0),                 // top
                tc(col + w - 1, row, 1),         // right
                tc(col + w - 1, row + h - 1, 2), // bottom
                tc(col, row + h - 1, 3),         // left
            ];
            if !ctx.rect.intersects(screen_aabb(&quad)) {
                continue;
            }
            let color = match world.get_prop(bid, "Color") {
                Some(Value::Color(c)) => *c,
                _ => Color::WHITE,
            };
            let outline = Color {
                r: color.r * 0.55,
                g: color.g * 0.55,
                b: color.b * 0.55,
                a: color.a,
            };
            painter.add(Shape::convex_polygon(
                quad,
                to_color(&color),
                Stroke::new(1.5, to_color(&outline)),
            ));
        }
    }
}
