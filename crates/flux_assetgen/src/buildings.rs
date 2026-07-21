//! Per-building sprite composition: each building renders its animation clips
//! into fixed-size frames, which get packed into one sheet + a `*.frames.json`
//! the engine's `AnimatedSprite` consumes directly.

use std::path::Path;

use crate::canvas::{Canvas, Rgba, Rng, lerp, shade, with_alpha};
use crate::iso::{self, HALF_H, HALF_W, IsoBox, cylinder, dome, ground_pad, hazard_stripes, lamp, puffs};
use crate::palette::*;

const PAD: f32 = 6.0; // margin around each frame
const PAD_BOTTOM: f32 = 4.0;

/// A rendered clip: frames plus playback config.
pub struct Clip {
    pub name: &'static str,
    pub frames: Vec<Canvas>,
    pub duration: f32,
    pub looped: bool,
}

/// Clips that reuse another clip's frames (no extra sheet pixels).
pub struct Alias {
    pub name: &'static str,
    pub target: &'static str,
}

pub struct BuildingArt {
    pub id: &'static str,
    /// Footprint in tiles (w, d) — must match the building catalog.
    pub foot: (f32, f32),
    pub frame: (u32, u32),
    /// Normalized pivot: where the footprint's ground-diamond centre sits.
    pub pivot: (f32, f32),
    pub clips: Vec<Clip>,
    pub aliases: Vec<Alias>,
}

/// Geometry shared by every composer: frame size + the screen position of the
/// footprint's back corner inside the frame.
struct Fr {
    w: u32,
    h: u32,
    ox: f32,
    oy: f32,
}

fn frame_for(foot_w: f32, foot_d: f32, structure_h: f32) -> Fr {
    let w = ((foot_w + foot_d) * HALF_W + PAD * 2.0) as u32;
    let ground_h = (foot_w + foot_d) * HALF_H;
    let h = (structure_h + ground_h + PAD_BOTTOM + PAD) as u32;
    Fr {
        w,
        h,
        ox: PAD + foot_d * HALF_W,
        oy: h as f32 - ground_h - PAD_BOTTOM,
    }
}

fn pivot_of(fr: &Fr, foot_w: f32, foot_d: f32) -> (f32, f32) {
    let cy = fr.oy + (foot_w + foot_d) * HALF_H * 0.5;
    (0.5, cy / fr.h as f32)
}

fn base(fr: &Fr, foot_w: f32, foot_d: f32) -> Canvas {
    let mut c = Canvas::new(fr.w, fr.h);
    let (cx, cy) = (fr.w as f32 * 0.5, fr.oy + (foot_w + foot_d) * HALF_H * 0.5);
    c.shadow(cx, cy + 2.0, (foot_w + foot_d) * HALF_W * 0.52, (foot_w + foot_d) * HALF_H * 0.55, 90);
    ground_pad(&mut c, fr.ox, fr.oy, foot_w, foot_d, PAD_CONCRETE);
    c
}

/// Soft additive-looking glow halo.
fn glow(c: &mut Canvas, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgba, strength: f32) {
    c.fill_ellipse_fn(cx, cy, rx, ry, |nx, ny| {
        let d = (nx * nx + ny * ny).sqrt();
        let a = ((1.0 - d).max(0.0) * 170.0 * strength) as u8;
        with_alpha(color, a)
    });
}

// ---------------------------------------------------------------------------
// Composers (one per building type)
// ---------------------------------------------------------------------------

fn control_room() -> BuildingArt {
    let (fw, fd) = (3.0, 3.0);
    let fr = frame_for(fw, fd, 64.0);
    let render = |beacon: bool| {
        let mut c = base(&fr, fw, fd);
        IsoBox { tx: 0.25, ty: 0.25, w: 2.5, d: 2.5, h: 34.0, color: CONCRETE }.draw(&mut c, fr.ox, fr.oy);
        IsoBox { tx: 0.55, ty: 0.55, w: 1.9, d: 1.9, h: 46.0, color: STEEL }.draw(&mut c, fr.ox, fr.oy);
        // Lit window strips on both visible faces of the tower.
        let (bx, by) = iso::proj(0.55 + 1.9, 0.55 + 1.9);
        for i in 0..3 {
            let y = fr.oy + by - 40.0 + i as f32 * 9.0;
            c.fill_poly(
                &[(fr.ox + bx - 22.0, y - 11.0), (fr.ox + bx - 4.0, y - 2.0), (fr.ox + bx - 4.0, y + 2.0), (fr.ox + bx - 22.0, y - 7.0)],
                WINDOW_LIT,
            );
            c.fill_poly(
                &[(fr.ox + bx + 4.0, y - 2.0), (fr.ox + bx + 22.0, y - 11.0), (fr.ox + bx + 22.0, y - 7.0), (fr.ox + bx + 4.0, y + 2.0)],
                shade(WINDOW_LIT, 0.8),
            );
        }
        // Antenna mast + beacon.
        let (ax, ay) = iso::proj(1.5, 1.5);
        let (mx, my) = (fr.ox + ax, fr.oy + ay - 46.0);
        c.line(mx, my, mx, my - 16.0, STEEL_DARK);
        c.outline();
        lamp(&mut c, mx, my - 17.0, LAMP_RED, beacon);
        c
    };
    BuildingArt {
        id: "control",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![Clip { name: "idle", frames: vec![render(false), render(true)], duration: 0.6, looped: true }],
        aliases: vec![Alias { name: "working", target: "idle" }, Alias { name: "starved", target: "idle" }],
    }
}

fn reactor() -> BuildingArt {
    let (fw, fd) = (4.0, 4.0);
    let fr = frame_for(fw, fd, 96.0);
    let (gx, gy) = iso::proj(2.0, 2.0); // footprint centre
    let (cx, cy) = (fr.ox + gx, fr.oy + gy);

    let render = |glow_color: Option<Rgba>, pulse: f32, beacon: Option<Rgba>, cracked: bool| {
        let mut c = base(&fr, fw, fd);
        hazard_stripes(&mut c, fr.ox, fr.oy, fw, fd, 5.0);
        // Annex block behind-left of the containment vessel.
        IsoBox { tx: 0.1, ty: 2.3, w: 1.3, d: 1.5, h: 22.0, color: CONCRETE_DARK }.draw(&mut c, fr.ox, fr.oy);
        // Containment vessel + dome.
        let body = if cracked { shade(CONCRETE, 0.7) } else { CONCRETE };
        cylinder(&mut c, cx, cy + 6.0, 68.0, |_| 50.0, body);
        dome(&mut c, cx, cy - 62.0, 50.0, body);
        // Vent slits (solid — they pick up the glow colour when lit).
        for i in 0..5 {
            let vx = cx - 30.0 + i as f32 * 15.0;
            let vent = match glow_color {
                Some(g) => lerp(WINDOW_DARK, g, 0.35 + 0.55 * pulse),
                None => WINDOW_DARK,
            };
            c.fill_poly(
                &[(vx, cy - 46.0), (vx + 6.0, cy - 43.0), (vx + 6.0, cy - 37.0), (vx, cy - 40.0)],
                vent,
            );
        }
        if cracked {
            let dark = shade(CONCRETE, 0.3);
            c.line(cx - 12.0, cy - 60.0, cx - 22.0, cy - 30.0, dark);
            c.line(cx - 22.0, cy - 30.0, cx - 14.0, cy - 8.0, dark);
            c.line(cx + 18.0, cy - 48.0, cx + 26.0, cy - 20.0, dark);
        }
        // Beacon mast on the dome crown.
        c.line(cx, cy - 92.0, cx, cy - 101.0, STEEL_DARK);
        // Outline the solid structure only — soft effects (glow, halos) come
        // after, or their alpha edges grow ugly dark rims.
        c.outline();
        if let Some(g) = glow_color {
            glow(&mut c, cx, cy - 60.0, 56.0, 17.0, g, 0.55 + 0.45 * pulse);
        }
        if let Some(b) = beacon {
            lamp(&mut c, cx, cy - 102.0, b, true);
        } else {
            lamp(&mut c, cx, cy - 102.0, LAMP_RED, false);
        }
        c
    };

    let pulse4 = |g: Rgba, beacon: Option<Rgba>| -> Vec<Canvas> {
        (0..4)
            .map(|i| {
                let p = (i as f32 / 4.0 * std::f32::consts::TAU).sin() * 0.5 + 0.5;
                render(Some(g), p, if i % 2 == 0 { beacon } else { None }, false)
            })
            .collect()
    };

    BuildingArt {
        id: "reactor",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![
            Clip { name: "off", frames: vec![render(None, 0.0, None, false)], duration: 0.5, looped: true },
            Clip { name: "running", frames: pulse4(GLOW_CYAN, Some(LAMP_GREEN)), duration: 0.18, looped: true },
            Clip { name: "hot", frames: pulse4(GLOW_ORANGE, Some(LAMP_AMBER)), duration: 0.12, looped: true },
            Clip {
                name: "meltdown",
                frames: vec![render(Some(GLOW_RED), 1.0, Some(LAMP_RED), true), render(None, 0.0, None, true)],
                duration: 0.16,
                looped: true,
            },
        ],
        aliases: vec![Alias { name: "idle", target: "off" }, Alias { name: "working", target: "running" }, Alias { name: "starved", target: "off" }],
    }
}

fn cooling_tower() -> BuildingArt {
    let (fw, fd) = (3.0, 3.0);
    let fr = frame_for(fw, fd, 160.0);
    let (gx, gy) = iso::proj(1.5, 1.5);
    let (cx, cy) = (fr.ox + gx, fr.oy + gy);

    let render = |steam_phase: Option<f32>| {
        let mut c = base(&fr, fw, fd);
        // Hyperboloid shell: wide base, pinched waist, flared rim.
        cylinder(&mut c, cx, cy + 6.0, 92.0, |t| {
            let waist = (t - 0.42).abs() / 0.6;
            30.0 + 16.0 * waist * waist + 4.0 * (1.0 - t)
        }, CONCRETE);
        // Rim shadow ring.
        c.fill_ellipse_fn(cx, cy - 86.0, 33.0, 15.0, |nx, ny| {
            let d = (nx * nx + ny * ny).sqrt();
            if d > 0.62 { with_alpha(CONCRETE_DARK, 0) } else { shade(WINDOW_DARK, 1.1) }
        });
        c.outline();
        if let Some(p) = steam_phase {
            iso::plume(&mut c, cx, cy - 96.0, p, STEAM);
        }
        c
    };

    BuildingArt {
        id: "cooling",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![
            Clip { name: "idle", frames: vec![render(None)], duration: 0.5, looped: true },
            Clip { name: "working", frames: (0..4).map(|i| render(Some(i as f32 / 4.0))).collect(), duration: 0.16, looped: true },
        ],
        aliases: vec![Alias { name: "starved", target: "idle" }],
    }
}

fn turbine() -> BuildingArt {
    let (fw, fd) = (3.0, 2.0);
    let fr = frame_for(fw, fd, 78.0);
    let render = |rotor: Option<f32>| {
        let mut c = base(&fr, fw, fd);
        // Machine hall: light body with a thin dark roof cap.
        IsoBox { tx: 0.2, ty: 0.2, w: 2.6, d: 1.6, h: 30.0, color: STEEL }.draw(&mut c, fr.ox, fr.oy);
        IsoBox { tx: 0.2, ty: 0.2, w: 2.6, d: 1.6, h: 6.0, color: ROOF }.draw(&mut c, fr.ox, fr.oy - 30.0);
        // Three round rotor windows along the right (south-east) face.
        for i in 0..3 {
            let t = 0.6 + i as f32 * 0.8;
            let (px, py) = iso::proj(0.2 + t, 1.8);
            let (wx, wy) = (fr.ox + px, fr.oy + py - 16.0);
            c.fill_ellipse(wx, wy, 7.0, 7.5, WINDOW_DARK);
            if let Some(phase) = rotor {
                let ang = phase * std::f32::consts::TAU / 4.0 + i as f32 * 0.5;
                for k in 0..3 {
                    let a = ang + k as f32 * std::f32::consts::TAU / 3.0;
                    c.line(wx, wy, wx + a.cos() * 5.5, wy + a.sin() * 5.5, shade(STEEL, 1.3));
                }
                c.fill_ellipse(wx, wy, 1.6, 1.6, shade(STEEL, 1.4));
            } else {
                c.fill_ellipse(wx, wy, 1.6, 1.6, STEEL_DARK);
            }
        }
        // Exhaust stack rising through the roof at the far corner.
        let (sx, sy) = iso::proj(0.55, 0.55);
        cylinder(&mut c, fr.ox + sx, fr.oy + sy, 52.0, |_| 4.5, STEEL_DARK);
        c.outline();
        if let Some(phase) = rotor {
            puffs(&mut c, fr.ox + sx, fr.oy + sy - 54.0, phase, STEAM, 3, 20.0, 0.9);
        }
        c
    };
    BuildingArt {
        id: "turbine",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![
            Clip { name: "idle", frames: vec![render(None)], duration: 0.5, looped: true },
            Clip { name: "working", frames: (0..4).map(|i| render(Some(i as f32))).collect(), duration: 0.12, looped: true },
        ],
        aliases: vec![Alias { name: "starved", target: "idle" }],
    }
}

fn smelter() -> BuildingArt {
    let (fw, fd) = (2.0, 2.0);
    let fr = frame_for(fw, fd, 66.0);
    let render = |fire: Option<f32>, warn: bool| {
        let mut c = base(&fr, fw, fd);
        IsoBox { tx: 0.15, ty: 0.15, w: 1.7, d: 1.7, h: 26.0, color: RUST }.draw(&mut c, fr.ox, fr.oy);
        // Ore hopper feeding the top.
        IsoBox { tx: 0.95, ty: 0.3, w: 0.6, d: 0.6, h: 12.0, color: STEEL_DARK }.draw(&mut c, fr.ox, fr.oy - 26.0);
        // Furnace mouth on the left face.
        let (mx, my) = iso::proj(0.6, 0.15 + 1.7);
        let (mx, my) = (fr.ox + mx, fr.oy + my - 9.0);
        let mouth = match fire {
            Some(p) => lerp(FIRE, GLOW_RED, 0.3 + 0.4 * ((p * std::f32::consts::TAU).sin() * 0.5 + 0.5)),
            None => WINDOW_DARK,
        };
        c.fill_poly(&[(mx - 7.0, my - 4.0), (mx + 7.0, my + 3.0), (mx + 7.0, my + 9.0), (mx - 7.0, my + 2.0)], mouth);
        // Chimney at the back corner.
        let (chx, chy) = iso::proj(1.55, 0.4);
        cylinder(&mut c, fr.ox + chx, fr.oy + chy - 18.0, 40.0, |t| 5.0 + t * 1.5, CONCRETE_DARK);
        c.outline();
        if fire.is_some() {
            glow(&mut c, mx, my + 2.0, 12.0, 8.0, FIRE, 0.6);
        }
        if let Some(p) = fire {
            puffs(&mut c, fr.ox + chx, fr.oy + chy - 58.0, p, SMOKE, 3, 24.0, 1.3);
        }
        lamp(&mut c, fr.ox + iso::proj(1.85, 1.85).0, fr.oy + iso::proj(1.85, 1.85).1 - 28.0, LAMP_AMBER, warn);
        c
    };
    BuildingArt {
        id: "smelter",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![
            Clip { name: "idle", frames: vec![render(None, false)], duration: 0.5, looped: true },
            Clip { name: "working", frames: (0..4).map(|i| render(Some(i as f32 / 4.0), false)).collect(), duration: 0.13, looped: true },
            Clip { name: "starved", frames: vec![render(None, true), render(None, false)], duration: 0.4, looped: true },
        ],
        aliases: vec![],
    }
}

fn miner() -> BuildingArt {
    let (fw, fd) = (2.0, 2.0);
    let fr = frame_for(fw, fd, 66.0);
    let (gx, gy) = iso::proj(1.0, 1.0);
    let (cx, cy) = (fr.ox + gx, fr.oy + gy);
    let render = |spin: Option<f32>, warn: bool| {
        let mut c = base(&fr, fw, fd);
        // Machine deck the rig stands on.
        IsoBox { tx: 0.15, ty: 0.15, w: 1.7, d: 1.7, h: 7.0, color: STEEL_DARK }.draw(&mut c, fr.ox, fr.oy);
        // Portal frame: left + right legs (screen-facing) carrying the head.
        IsoBox { tx: 0.2, ty: 1.45, w: 0.35, d: 0.35, h: 42.0, color: RUST }.draw(&mut c, fr.ox, fr.oy);
        IsoBox { tx: 1.45, ty: 0.2, w: 0.35, d: 0.35, h: 42.0, color: RUST }.draw(&mut c, fr.ox, fr.oy);
        // Crossbeam girder between the legs (screen-space, reads horizontal).
        c.fill_poly(
            &[(cx - 36.0, cy - 30.0), (cx + 36.0, cy - 30.0), (cx + 36.0, cy - 36.0), (cx - 36.0, cy - 36.0)],
            shade(RUST, 0.9),
        );
        // Drive housing hanging from the beam over the bite point.
        IsoBox { tx: 0.62, ty: 0.62, w: 0.76, d: 0.76, h: 14.0, color: LAMP_AMBER }.draw(&mut c, fr.ox, fr.oy - 34.0);
        // Drill shaft: diagonal stripes scroll to read as rotation.
        let phase = spin.unwrap_or(0.0) * 8.0;
        let (r, top, bot) = (8.0_f32, cy - 22.0, cy + 6.0);
        for y in top as i32..bot as i32 {
            for x in (cx - r) as i32..(cx + r) as i32 {
                let band = ((x as f32 - cx) + (y as f32) * 1.4 + phase).rem_euclid(8.0);
                let col = if band < 4.0 { shade(STEEL, 1.2) } else { STEEL_DARK };
                let nx = (x as f32 + 0.5 - cx) / r;
                c.blend(x, y, shade(col, 1.05 - 0.4 * (nx + 0.3).abs()));
            }
        }
        // Drill tip biting into the ground.
        c.fill_poly(&[(cx - r, bot), (cx + r, bot), (cx, bot + 8.0)], STEEL_DARK);
        c.outline();
        if spin.is_some() {
            let mut rng = Rng::new(9 + phase as u32);
            for _ in 0..8 {
                let dx = rng.range(-12.0, 12.0);
                let dy = rng.range(-2.0, 5.0);
                c.fill_ellipse(cx + dx, bot + 5.0 + dy, 2.0, 1.2, with_alpha(CONCRETE, 170));
            }
        }
        lamp(&mut c, cx + 39.0, cy - 33.0, LAMP_AMBER, warn);
        c
    };
    BuildingArt {
        id: "miner",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![
            Clip { name: "idle", frames: vec![render(None, false)], duration: 0.5, looped: true },
            Clip { name: "working", frames: (0..4).map(|i| render(Some(i as f32 / 4.0), false)).collect(), duration: 0.1, looped: true },
            Clip { name: "starved", frames: vec![render(None, true), render(None, false)], duration: 0.4, looped: true },
        ],
        aliases: vec![],
    }
}

fn belt() -> BuildingArt {
    let (fw, fd) = (1.0, 1.0);
    let fr = frame_for(fw, fd, 10.0);
    // Authored pointing +x (screen lower-right); the game derives the other
    // three directions with FlipX/FlipY, so the art must stay mirror-clean.
    let render = |phase: f32| {
        let mut c = base(&fr, fw, fd);
        IsoBox { tx: 0.06, ty: 0.14, w: 0.88, d: 0.72, h: 4.0, color: STEEL_DARK }.draw(&mut c, fr.ox, fr.oy);
        // Chevrons marching along +x on the top plate.
        let p = |tx: f32, ty: f32| {
            let (x, y) = iso::proj(tx, ty);
            (fr.ox + x, fr.oy + y - 4.0)
        };
        let arrow = shade(LAMP_AMBER, 1.0);
        for i in 0..3 {
            let t = 0.12 + ((i as f32 + phase * 0.25) % 3.0) * 0.27;
            if t > 0.82 {
                continue;
            }
            let tip = p(t + 0.16, 0.5);
            let (a, b) = (p(t, 0.26), p(t, 0.74));
            c.line(a.0, a.1, tip.0, tip.1, arrow);
            c.line(b.0, b.1, tip.0, tip.1, arrow);
        }
        c.outline();
        c
    };
    BuildingArt {
        id: "belt",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![
            Clip { name: "idle", frames: vec![render(0.0)], duration: 0.5, looped: true },
            Clip { name: "working", frames: (0..4).map(|i| render(i as f32)).collect(), duration: 0.1, looped: true },
        ],
        aliases: vec![Alias { name: "starved", target: "idle" }],
    }
}

/// Clip names for the 16 pipe connectivity masks (N=1, E=2, S=4, W=8 in tile
/// space). The runtime picks the clip from its neighbour mask.
pub const PIPE_MASKS: [&str; 16] = [
    "m0", "m1", "m2", "m3", "m4", "m5", "m6", "m7", "m8", "m9", "m10", "m11", "m12", "m13", "m14",
    "m15",
];

fn pipe() -> BuildingArt {
    let (fw, fd) = (1.0, 1.0);
    let fr = frame_for(fw, fd, 16.0);
    let steel: Rgba = crate::canvas::rgb(112, 126, 146);
    let (cx, cy) = {
        let (x, y) = iso::proj(0.5, 0.5);
        (fr.ox + x, fr.oy + y)
    };
    // Tile-space edge midpoints for each mask bit: N(-y), E(+x), S(+y), W(-x).
    let edges = [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)];

    let render = |mask: u8| {
        let mut c = Canvas::new(fr.w, fr.h);
        c.shadow(cx, cy + 2.0, 26.0, 12.0, 70);
        // Arms toward each connected edge, drawn as capsule runs of ellipses.
        for (bit, (tx, ty)) in edges.iter().enumerate() {
            if mask & (1 << bit) == 0 {
                continue;
            }
            let (ex, ey) = iso::proj(*tx, *ty);
            let (ex, ey) = (fr.ox + ex, fr.oy + ey);
            for i in 0..=8 {
                let t = i as f32 / 8.0;
                let (px, py) = (cx + (ex - cx) * t, cy + (ey - cy) * t - 5.0);
                c.fill_ellipse(px, py, 5.5, 3.6, shade(steel, 1.02 - t * 0.12));
                c.fill_ellipse(px - 1.0, py - 1.5, 2.4, 1.2, shade(steel, 1.35));
            }
        }
        // Central hub (also the whole art for a lone segment).
        c.fill_ellipse(cx, cy - 5.0, 7.0, 4.6, shade(steel, 1.12));
        c.fill_ellipse(cx - 1.5, cy - 6.5, 2.8, 1.5, shade(steel, 1.45));
        c.outline();
        c
    };

    let mut clips: Vec<Clip> = (0u8..16)
        .map(|m| Clip {
            name: PIPE_MASKS[m as usize],
            frames: vec![render(m)],
            duration: 0.5,
            looped: true,
        })
        .collect();
    // State clips are aliases: pipe visuals are shape-driven, not state-driven.
    clips.shrink_to_fit();
    BuildingArt {
        id: "pipe",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips,
        aliases: vec![
            Alias { name: "idle", target: "m0" },
            Alias { name: "working", target: "m0" },
            Alias { name: "starved", target: "m0" },
        ],
    }
}

fn storage() -> BuildingArt {
    let (fw, fd) = (1.0, 1.0);
    let fr = frame_for(fw, fd, 26.0);
    let render = |rx: bool| {
        let mut c = base(&fr, fw, fd);
        IsoBox { tx: 0.1, ty: 0.1, w: 0.8, d: 0.8, h: 12.0, color: WOOD }.draw(&mut c, fr.ox, fr.oy);
        IsoBox { tx: 0.25, ty: 0.2, w: 0.5, d: 0.5, h: 10.0, color: shade(WOOD, 1.15) }.draw(&mut c, fr.ox, fr.oy - 12.0);
        c.outline();
        let (lx, ly) = iso::proj(0.9, 0.9);
        lamp(&mut c, fr.ox + lx, fr.oy + ly - 16.0, LAMP_GREEN, rx);
        c
    };
    BuildingArt {
        id: "storage",
        foot: (fw, fd),
        frame: (fr.w, fr.h),
        pivot: pivot_of(&fr, fw, fd),
        clips: vec![
            Clip { name: "idle", frames: vec![render(false)], duration: 0.5, looped: true },
            Clip { name: "working", frames: vec![render(true), render(false)], duration: 0.35, looped: true },
        ],
        aliases: vec![Alias { name: "starved", target: "idle" }],
    }
}

pub fn all() -> Vec<BuildingArt> {
    vec![
        control_room(),
        reactor(),
        cooling_tower(),
        turbine(),
        smelter(),
        miner(),
        belt(),
        pipe(),
        storage(),
    ]
}

// ---------------------------------------------------------------------------
// Sheet packing + frames.json
// ---------------------------------------------------------------------------

/// Gutter between packed frames (pixels). With linear filtering, adjacent
/// frames bleed into each other at shared edges; the renderer also insets UVs
/// half a texel, and this gap makes any residual taps hit transparent gutter
/// rather than a neighbouring frame.
const GUTTER: u32 = 2;

/// Pack a building's clips into one sheet (one row per clip) and emit the
/// engine `*.frames.json` next to it. Returns `(sheet, json)`.
pub fn pack(art: &BuildingArt) -> (Canvas, String) {
    let (fw, fh) = art.frame;
    let cols = art.clips.iter().map(|c| c.frames.len()).max().unwrap_or(1) as u32;
    let (cw, ch) = (fw + GUTTER, fh + GUTTER);
    let mut sheet = Canvas::new(cw * cols, ch * art.clips.len() as u32);
    let mut clips_json = serde_json::Map::new();

    for (row, clip) in art.clips.iter().enumerate() {
        let mut frames = Vec::new();
        for (col, frame) in clip.frames.iter().enumerate() {
            sheet.blit(frame, col as i32 * cw as i32, row as i32 * ch as i32);
            frames.push(serde_json::json!({
                "rect": [col as u32 * cw, row as u32 * ch, fw, fh],
                "duration": clip.duration,
            }));
        }
        clips_json.insert(
            clip.name.to_string(),
            serde_json::json!({ "loop": clip.looped, "frames": frames }),
        );
    }
    for alias in &art.aliases {
        if let Some(v) = clips_json.get(alias.target).cloned() {
            clips_json.insert(alias.name.to_string(), v);
        }
    }

    let doc = serde_json::json!({
        "texture": format!("art/{}.png", art.id),
        "clips": clips_json,
    });
    (sheet, serde_json::to_string_pretty(&doc).unwrap())
}

/// Render every building's sheet + frames.json into `<root>/art/`.
/// Returns per-building metadata for catalog authoring.
pub fn generate(root: &Path) -> Result<Vec<Meta>, String> {
    let dir = root.join("art");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut metas = Vec::new();
    for art in all() {
        let (sheet, json) = pack(&art);
        sheet.save_png(&dir.join(format!("{}.png", art.id)))?;
        std::fs::write(dir.join(format!("{}.frames.json", art.id)), &json).map_err(|e| e.to_string())?;
        metas.push(Meta {
            id: art.id,
            frames_asset: format!("art/{}.frames.json", art.id),
            frame: art.frame,
            pivot: art.pivot,
            foot: (art.foot.0 as u32, art.foot.1 as u32),
        });
    }
    Ok(metas)
}

pub struct Meta {
    pub id: &'static str,
    pub frames_asset: String,
    pub frame: (u32, u32),
    pub pivot: (f32, f32),
    pub foot: (u32, u32),
}
