//! Terrain atlas: one 64x32 diamond per tile id, plus ore-vein overlay tiles,
//! emitted together with a regenerated `world.tileset.json` (same ids/colors
//! as before, now with atlas rects).

use std::path::Path;

use crate::canvas::{Canvas, Rgba, Rng, rgb, shade, with_alpha};

pub const TILE_W: u32 = 64;
pub const TILE_H: u32 = 32;

/// `(id, representative color)` — ids and order must stay stable: worldgen
/// JSON references them and the per-cell grid stores indices into this list.
const TILES: &[(&str, Rgba)] = &[
    ("water", rgb(44, 90, 148)),
    ("sand", rgb(202, 184, 122)),
    ("grass", rgb(88, 138, 70)),
    ("forest", rgb(56, 106, 52)),
    ("rock", rgb(120, 116, 110)),
    ("mountain", rgb(164, 164, 172)),
    ("coal", rgb(38, 38, 44)),
    ("iron", rgb(196, 134, 106)),
    ("copper", rgb(204, 122, 58)),
    ("uranium", rgb(110, 218, 90)),
    ("rare", rgb(188, 110, 214)),
];

fn diamond(x: i32, y: i32) -> bool {
    let dx = (x as f32 + 0.5 - TILE_W as f32 / 2.0) / (TILE_W as f32 / 2.0);
    let dy = (y as f32 + 0.5 - TILE_H as f32 / 2.0) / (TILE_H as f32 / 2.0);
    dx.abs() + dy.abs() <= 1.0
}

/// Fill the tile diamond with `base`, speckled and edge-shaded.
fn ground(c: &mut Canvas, ox: i32, base: Rgba, rng: &mut Rng, speckle: f32) {
    for y in 0..TILE_H as i32 {
        for x in 0..TILE_W as i32 {
            if !diamond(x, y) {
                continue;
            }
            let mut col = base;
            let r = rng.next_f32();
            if r < speckle {
                col = shade(base, 0.86);
            } else if r > 1.0 - speckle * 0.6 {
                col = shade(base, 1.12);
            }
            // Southern edges read darker so tiles pop against neighbours.
            let dy = (y as f32 + 0.5 - TILE_H as f32 / 2.0) / (TILE_H as f32 / 2.0);
            let dx = (x as f32 + 0.5 - TILE_W as f32 / 2.0) / (TILE_W as f32 / 2.0);
            let edge = dx.abs() + dy.abs();
            if edge > 0.82 && dy > 0.0 {
                col = shade(col, 0.8);
            }
            c.blend(ox + x, y, col);
        }
    }
}

fn draw_tile(c: &mut Canvas, ox: i32, id: &str, base: Rgba, rng: &mut Rng) {
    match id {
        "water" => {
            ground(c, ox, base, rng, 0.02);
            // Light streak highlights.
            for i in 0..3 {
                let y = 10 + i * 6;
                let x0 = 14 + (i * 11) % 16;
                for x in x0..x0 + 10 {
                    if diamond(x, y) {
                        c.blend(ox + x, y, with_alpha(rgb(130, 178, 224), 170));
                    }
                }
            }
        }
        "forest" => {
            ground(c, ox, shade(base, 1.0), rng, 0.06);
            // A few tiny pines with shadows.
            for (px, py) in [(22, 18), (36, 12), (44, 20)] {
                c.fill_ellipse(ox as f32 + px as f32, py as f32 + 2.0, 4.0, 1.6, with_alpha(rgb(10, 20, 12), 110));
                c.fill_poly(
                    &[
                        (ox as f32 + px as f32 - 3.5, py as f32),
                        (ox as f32 + px as f32 + 3.5, py as f32),
                        (ox as f32 + px as f32, py as f32 - 9.0),
                    ],
                    rgb(40, 92, 50),
                );
                c.blend(ox + px, py + 1, rgb(88, 62, 40));
            }
        }
        "rock" | "mountain" => {
            ground(c, ox, base, rng, 0.1);
            let dark = shade(base, 0.65);
            c.line(ox as f32 + 22.0, 12.0, ox as f32 + 34.0, 18.0, dark);
            c.line(ox as f32 + 34.0, 18.0, ox as f32 + 30.0, 24.0, dark);
            if id == "mountain" {
                for y in 0..8 {
                    for x in 0..TILE_W as i32 {
                        if diamond(x, y + 4) && rng.next_f32() < 0.5 {
                            c.blend(ox + x, y + 4, with_alpha(rgb(228, 232, 240), 140));
                        }
                    }
                }
            }
        }
        // Ore veins: transparent overlays — nugget clusters (drawn by the
        // renderer inset over the ground tile).
        "coal" | "iron" | "copper" | "uranium" | "rare" => {
            if id == "uranium" {
                // Radioactive shimmer behind the nuggets.
                c.fill_ellipse_fn(ox as f32 + 32.0, 16.0, 20.0, 10.0, |nx, ny| {
                    let d = (nx * nx + ny * ny).sqrt();
                    with_alpha(base, ((1.0 - d).max(0.0) * 80.0) as u8)
                });
            }
            for _ in 0..7 {
                let px = 32.0 + rng.range(-16.0, 16.0);
                let py = 16.0 + rng.range(-7.0, 7.0);
                let r = rng.range(1.8, 3.6);
                c.fill_ellipse(ox as f32 + px, py, r, r * 0.7, shade(base, rng.range(0.8, 1.05)));
                c.fill_ellipse(ox as f32 + px - r * 0.3, py - r * 0.3, r * 0.4, r * 0.3, shade(base, 1.45));
            }
        }
        _ => ground(c, ox, base, rng, 0.05),
    }
}

/// Render the atlas + emit the tileset JSON. Returns the tileset JSON string.
pub fn generate(root: &Path) -> Result<(), String> {
    let dir = root.join("art");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Gutter between tiles so linear filtering never bleeds a neighbour in
    // (the renderer also insets UVs half a texel).
    const GUTTER: u32 = 2;
    let stride = TILE_W + GUTTER;
    let mut atlas = Canvas::new(stride * TILES.len() as u32, TILE_H);
    let mut tiles_json = Vec::new();
    for (i, (id, color)) in TILES.iter().enumerate() {
        let mut rng = Rng::new(0xA55E7 + i as u32 * 7919);
        draw_tile(&mut atlas, (i as u32 * stride) as i32, id, *color, &mut rng);
        let c = |v: u8| (v as f32 / 255.0 * 100.0).round() / 100.0;
        tiles_json.push(serde_json::json!({
            "id": id,
            "color": [c(color[0]), c(color[1]), c(color[2]), 1.0],
            "rect": [i as u32 * stride, 0, TILE_W, TILE_H],
        }));
    }
    atlas.save_png(&dir.join("terrain.png"))?;

    let doc = serde_json::json!({
        "tile_width": TILE_W,
        "tile_height": TILE_H,
        "texture": "art/terrain.png",
        "tiles": tiles_json,
    });
    std::fs::write(
        root.join("world.tileset.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .map_err(|e| e.to_string())
}
