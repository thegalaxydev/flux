//! Small UI icons (one PNG each — the engine's `ImageFrame` takes a whole
//! texture): building glyphs as mini iso cubes with an accent, item glyphs as
//! nugget piles / plate stacks.

use std::path::Path;

use crate::canvas::{Canvas, Rgba, Rng, rgb, shade};
use crate::iso::IsoBox;

const SIZE: u32 = 28;

fn cube_icon(color: Rgba, accent: Option<Rgba>) -> Canvas {
    let mut c = Canvas::new(SIZE, SIZE);
    IsoBox { tx: 0.0, ty: 0.0, w: 0.55, d: 0.55, h: 11.0, color }.draw(&mut c, 14.0, 16.0);
    if let Some(a) = accent {
        c.fill_ellipse(14.0, 8.0, 3.0, 2.0, a);
    }
    c.outline();
    c
}

fn nuggets_icon(color: Rgba, seed: u32) -> Canvas {
    let mut c = Canvas::new(SIZE, SIZE);
    let mut rng = Rng::new(seed);
    for _ in 0..5 {
        let x = 14.0 + rng.range(-7.0, 7.0);
        let y = 16.0 + rng.range(-5.0, 5.0);
        let r = rng.range(2.5, 4.5);
        c.fill_ellipse(x, y, r, r * 0.75, color);
        c.fill_ellipse(x - r * 0.3, y - r * 0.3, r * 0.4, r * 0.3, shade(color, 1.5));
    }
    c.outline();
    c
}

fn plates_icon(color: Rgba) -> Canvas {
    let mut c = Canvas::new(SIZE, SIZE);
    for i in 0..3 {
        let y = 19.0 - i as f32 * 4.0;
        c.fill_poly(
            &[(5.0, y), (14.0, y - 4.5), (23.0, y), (14.0, y + 4.5)],
            shade(color, 1.0 + i as f32 * 0.12),
        );
    }
    c.outline();
    c
}

pub fn generate(root: &Path) -> Result<(), String> {
    let dir = root.join("art").join("icons");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let save = |name: &str, c: Canvas| c.save_png(&dir.join(format!("{name}.png")));

    // Buildings (colors echo their sprite palettes).
    save("control", cube_icon(rgb(138, 148, 160), Some(rgb(255, 80, 70))))?;
    save("reactor", cube_icon(rgb(164, 162, 156), Some(rgb(110, 230, 255))))?;
    save("cooling", cube_icon(rgb(164, 162, 156), Some(rgb(235, 240, 245))))?;
    save("turbine", cube_icon(rgb(84, 100, 122), None))?;
    save("smelter", cube_icon(rgb(168, 104, 60), Some(rgb(255, 168, 64))))?;
    save("miner", cube_icon(rgb(92, 100, 112), Some(rgb(255, 192, 64))))?;
    save("belt", cube_icon(rgb(92, 100, 112), None))?;
    save("pipe", {
        // A short horizontal tube.
        let mut c = Canvas::new(SIZE, SIZE);
        let steel = rgb(112, 126, 146);
        for i in 0..=10 {
            let x = 5.0 + i as f32 * 1.8;
            c.fill_ellipse(x, 14.0, 3.6, 4.2, shade(steel, 1.0 - i as f32 * 0.015));
        }
        c.fill_ellipse(23.0, 14.0, 2.2, 4.2, shade(steel, 0.7));
        c.fill_ellipse(5.0, 14.0, 2.2, 4.2, shade(steel, 1.25));
        c.outline();
        c
    })?;
    save("storage", cube_icon(rgb(146, 110, 72), None))?;

    // Items.
    save("coal", nuggets_icon(rgb(38, 38, 44), 11))?;
    save("iron", nuggets_icon(rgb(196, 134, 106), 22))?;
    save("copper", nuggets_icon(rgb(204, 122, 58), 33))?;
    save("uranium", nuggets_icon(rgb(110, 218, 90), 44))?;
    save("rare", nuggets_icon(rgb(188, 110, 214), 55))?;
    save("iron_plate", plates_icon(rgb(180, 186, 196)))?;
    Ok(())
}
