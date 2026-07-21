//! The game's overlay renderer, registered with `flux_view::register_overlay`.
//!
//! Sprited buildings are drawn by the engine (their child `AnimatedSprite`);
//! here we add only a subtle ground-footprint outline under each building,
//! the flat-diamond fallback for buildings *without* sprite art, and the
//! item-flow pips that make logistics visible.

use egui::{Color32, Painter, Pos2, Shape, Stroke};

use flux_core::{Color, InstanceId, Value, World};
use flux_view::{RenderCtx, to_color};

use crate::factory::FlowLog;

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

/// Stable per-item pip colour, so ore streams are recognizable at a glance.
fn item_color(item: &str) -> Color32 {
    match item {
        "iron" => Color32::from_rgb(196, 134, 106),
        "coal" => Color32::from_rgb(60, 60, 68),
        "copper" => Color32::from_rgb(204, 122, 58),
        "uranium" => Color32::from_rgb(110, 218, 90),
        "iron_plate" => Color32::from_rgb(190, 196, 206),
        _ => {
            let h: u32 = item.bytes().fold(5381u32, |a, b| a.wrapping_mul(33) ^ b as u32);
            Color32::from_rgb(
                120 + (h % 100) as u8,
                120 + ((h >> 8) % 100) as u8,
                120 + ((h >> 16) % 100) as u8,
            )
        }
    }
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
        // Back-to-front by FRONT corner (matches the sprite ZIndex anchor).
        buildings.sort_by_key(|&(_, col, row, w, h)| col + w - 1 + row + h - 1);

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
            let aabb = screen_aabb(&quad);
            if !ctx.rect.intersects(aabb) {
                continue;
            }

            let selected = matches!(
                world.get_prop(tm, "_Selected"),
                Some(Value::InstanceRef(Some(s))) if *s == bid
            );
            if selected {
                // Selection highlight: a bright footprint outline (drawn over
                // the sprite on purpose — it's a UI adornment).
                painter.add(Shape::closed_line(
                    quad.clone(),
                    Stroke::new(2.0, Color32::from_rgb(255, 210, 80)),
                ));
            }
            if crate::building::sprite_of(world, bid).is_some() {
                // The engine draws the sprite; nothing else to add. (No
                // always-on outline: it would cut across neighbouring art.)
            } else {
                // No art: legacy flat diamond in the catalog colour.
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
                painter.add(Shape::convex_polygon(quad, to_color(&color), Stroke::new(1.5, to_color(&outline))));
            }
        }

        // Item-flow pips: each recent hop animates from source to destination.
        if let Some(log) = world.component::<FlowLog>(tm) {
            for e in &log.events {
                let t = (e.age / 0.6).clamp(0.0, 1.0);
                let p = e.from + (e.to - e.from) * t;
                let s = ctx.to_screen(p) + egui::vec2(0.0, -6.0 * ctx.camera.zoom);
                if !ctx.rect.contains(s) {
                    continue;
                }
                let r = (3.5 * ctx.camera.zoom).clamp(1.5, 5.0);
                painter.circle_filled(s, r, item_color(&e.item));
                painter.circle_stroke(s, r, Stroke::new(1.0, Color32::from_black_alpha(120)));
            }
        }
    }
}
