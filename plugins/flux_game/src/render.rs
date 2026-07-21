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

/// Port marker colour by kind: item amber, liquid blue, gas white, power
/// yellow, heat red.
fn port_color(kind: crate::ports::PortKind) -> Color32 {
    use crate::ports::PortKind::*;
    match kind {
        Item => Color32::from_rgb(255, 190, 70),
        Liquid => Color32::from_rgb(90, 160, 255),
        Gas => Color32::from_rgb(235, 240, 245),
        Power => Color32::from_rgb(255, 230, 60),
        Heat => Color32::from_rgb(255, 90, 60),
    }
}

/// Draw a building's port markers: a dot on the boundary edge, an arrow
/// pointing out for outputs / in for inputs, coloured by kind. `strength`
/// dims markers (compatibility hints draw fainter than selection).
fn draw_ports(
    painter: &Painter,
    ctx: &RenderCtx,
    map_pos: glam::Vec2,
    tw: f32,
    th: f32,
    ports: &[crate::ports::ResolvedPort],
    strength: f32,
) {
    for rp in ports {
        let cell_c = flux_core::tilemap::tile_to_world(rp.cell.0, rp.cell.1, tw, th);
        let face_c = flux_core::tilemap::tile_to_world(rp.facing.0, rp.facing.1, tw, th);
        // Midpoint of the shared edge, nudged outward.
        let edge = (cell_c + face_c) * 0.5;
        let out = (face_c - cell_c) * 0.5;
        let color = port_color(rp.port.kind).gamma_multiply(strength);
        let p0 = ctx.to_screen(map_pos + edge - out * 0.4);
        let p1 = ctx.to_screen(map_pos + edge + out * 0.4);
        let (tail, head) = if rp.port.flow.gives_output() { (p0, p1) } else { (p1, p0) };
        painter.line_segment([tail, head], Stroke::new(2.5, color));
        // Arrowhead.
        let dir = (head - tail).normalized();
        let n = egui::vec2(-dir.y, dir.x);
        let sz = (5.0 * ctx.camera.zoom).clamp(3.0, 8.0);
        painter.add(Shape::convex_polygon(
            vec![head, head - dir * sz + n * sz * 0.6, head - dir * sz - n * sz * 0.6],
            color,
            Stroke::NONE,
        ));
        painter.circle_filled(ctx.to_screen(map_pos + edge), (3.0 * ctx.camera.zoom).clamp(2.0, 5.0), color);
    }
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

            // Selection travels as a TAG on the building (Roblox-style),
            // not a map-level instance reference.
            let selected = world.has_tag(bid, "selected");
            if selected {
                // Selection highlight: a bright footprint outline (drawn over
                // the sprite on purpose — it's a UI adornment).
                painter.add(Shape::closed_line(
                    quad.clone(),
                    Stroke::new(2.0, Color32::from_rgb(255, 210, 80)),
                ));
                if let Some(baked) = crate::ports::of(world, bid) {
                    draw_ports(painter, ctx, pos, tw, th, &baked.0, 1.0);
                }
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

        // Placement preview: the ghost's ports, plus compatible ports on
        // every machine while a belt (item inputs) or pipe (fluid ports) is
        // held — so you can see where connections will land.
        if let Some(gp) = world.component::<crate::building::GhostPorts>(tm) {
            draw_ports(painter, ctx, pos, tw, th, &gp.ports, 1.0);
            if gp.directional || gp.pipe {
                for b in world.children(tm).iter().copied() {
                    let Some(baked) = crate::ports::of(world, b) else { continue };
                    let compatible: Vec<crate::ports::ResolvedPort> = baked
                        .0
                        .iter()
                        .filter(|rp| {
                            if gp.pipe {
                                rp.port.kind.is_fluid()
                            } else {
                                rp.port.kind == crate::ports::PortKind::Item
                            }
                        })
                        .cloned()
                        .collect();
                    if !compatible.is_empty() {
                        draw_ports(painter, ctx, pos, tw, th, &compatible, 0.45);
                    }
                }
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
