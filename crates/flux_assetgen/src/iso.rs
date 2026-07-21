//! Isometric drawing kit: 2:1 projection helpers plus the shaded primitives
//! (boxes, cylinders, domes) every building is composed from. Light comes from
//! the upper-left: tops brightest, right (south-east) faces medium, left
//! (south-west) faces darkest — the classic iso read.

use crate::canvas::{Canvas, Rgba, lerp, shade};

/// One tile step along +x in screen pixels (2:1 diamonds, 64x32 tiles).
pub const HALF_W: f32 = 32.0;
pub const HALF_H: f32 = 16.0;

/// Project a tile-space offset to a screen offset.
pub fn proj(tx: f32, ty: f32) -> (f32, f32) {
    ((tx - ty) * HALF_W, (tx + ty) * HALF_H)
}

/// Face brightness factors.
pub const TOP: f32 = 1.0;
pub const RIGHT: f32 = 0.78;
pub const LEFT: f32 = 0.56;

/// A shaded iso box: back corner at tile `(tx, ty)` relative to the origin,
/// `w x d` tiles in footprint, `h` pixels tall.
pub struct IsoBox {
    pub tx: f32,
    pub ty: f32,
    pub w: f32,
    pub d: f32,
    pub h: f32,
    pub color: Rgba,
}

impl IsoBox {
    /// Screen position of the box's ground corners (a=back, b=right, c=front,
    /// d=left) given the footprint origin at screen `(ox, oy)`.
    fn corners(&self, ox: f32, oy: f32) -> [(f32, f32); 4] {
        let p = |tx: f32, ty: f32| {
            let (x, y) = proj(tx, ty);
            (ox + x, oy + y)
        };
        [
            p(self.tx, self.ty),
            p(self.tx + self.w, self.ty),
            p(self.tx + self.w, self.ty + self.d),
            p(self.tx, self.ty + self.d),
        ]
    }

    pub fn draw(&self, c: &mut Canvas, ox: f32, oy: f32) {
        let [a, b, f, d] = self.corners(ox, oy);
        let up = |p: (f32, f32)| (p.0, p.1 - self.h);
        let (at, bt, ft, dt) = (up(a), up(b), up(f), up(d));

        // Left (south-west) face: darkest, slight vertical falloff.
        c.fill_poly_shaded(
            &[dt, ft, f, d],
            shade(self.color, LEFT * 1.08),
            shade(self.color, LEFT * 0.86),
        );
        // Right (south-east) face.
        c.fill_poly_shaded(
            &[ft, bt, b, f],
            shade(self.color, RIGHT * 1.06),
            shade(self.color, RIGHT * 0.88),
        );
        // Top face, with a subtle highlight toward the back-left light.
        c.fill_poly_shaded(&[at, bt, ft, dt], shade(self.color, TOP * 1.1), shade(self.color, TOP * 0.94));
        // Crisp top edges.
        let hi = shade(self.color, 1.3);
        c.line(at.0, at.1, bt.0, bt.1, hi);
        c.line(at.0, at.1, dt.0, dt.1, hi);
    }
}

/// A vertical cylinder standing on the ground at screen `(cx, base_y)`.
/// `profile(t)` gives the radius at `t` (0 = top, 1 = base) — a constant
/// profile is a plain cylinder, a pinched one a cooling-tower hyperboloid.
pub fn cylinder(
    c: &mut Canvas,
    cx: f32,
    base_y: f32,
    h: f32,
    profile: impl Fn(f32) -> f32,
    color: Rgba,
) {
    let top_y = base_y - h;
    for y in top_y.round() as i32..base_y.round() as i32 {
        let t = ((y as f32 - top_y) / h).clamp(0.0, 1.0);
        let r = profile(t).max(1.0);
        for x in (cx - r).round() as i32..(cx + r).round() as i32 {
            let nx = (x as f32 + 0.5 - cx) / r; // -1 .. 1 across the width
            // Horizontal lighting: brightest band left of centre, dark limbs.
            let lit = 1.12 - 0.52 * (nx + 0.35).abs() - 0.18 * nx.max(0.0);
            // Quantize into pixel-art shading bands with Bayer jitter.
            let q = ((lit * 6.0 + crate::canvas::bayer(x, y) - 0.5).floor() / 6.0).clamp(0.3, 1.25);
            c.blend(x, y, shade(color, q));
        }
    }
    // Elliptical top cap.
    let rt = profile(0.0).max(1.0);
    c.fill_ellipse_fn(cx, top_y, rt, rt * 0.5, |nx, ny| {
        let l = 1.08 - 0.2 * (nx * 0.7 + ny * 0.7);
        shade(color, l)
    });
}

/// A dome (half-sphere) sitting on top of a cylinder or box.
pub fn dome(c: &mut Canvas, cx: f32, base_y: f32, r: f32, color: Rgba) {
    for y in (base_y - r * 0.62).round() as i32..base_y.round() as i32 {
        let ny = (base_y - y as f32) / (r * 0.62); // 0 at base .. 1 at crown
        let half = r * (1.0 - ny * ny).sqrt().max(0.05);
        for x in (cx - half).round() as i32..(cx + half).round() as i32 {
            let nx = (x as f32 + 0.5 - cx) / r;
            let lit = 1.15 - 0.5 * ((nx + 0.4).abs() + (1.0 - ny) * 0.35);
            let q = ((lit * 6.0 + crate::canvas::bayer(x, y) - 0.5).floor() / 6.0).clamp(0.35, 1.3);
            c.blend(x, y, shade(color, q));
        }
    }
}

/// The filled ground diamond of a `w x d` footprint (a concrete pad).
pub fn ground_pad(c: &mut Canvas, ox: f32, oy: f32, w: f32, d: f32, color: Rgba) {
    let p = |tx: f32, ty: f32| {
        let (x, y) = proj(tx, ty);
        (ox + x, oy + y)
    };
    let pts = [p(0.0, 0.0), p(w, 0.0), p(w, d), p(0.0, d)];
    c.fill_poly_shaded(&pts, shade(color, 1.04), shade(color, 0.9));
    // Edge kerb.
    let kerb = shade(color, 0.7);
    for i in 0..4 {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        c.line(a.0, a.1, b.0, b.1, kerb);
    }
}

/// Diagonal hazard stripes along the bottom of the two visible faces.
pub fn hazard_stripes(c: &mut Canvas, ox: f32, oy: f32, w: f32, d: f32, band: f32) {
    let yellow: Rgba = [226, 190, 52, 255];
    let black: Rgba = [30, 30, 34, 255];
    let p = |tx: f32, ty: f32| {
        let (x, y) = proj(tx, ty);
        (ox + x, oy + y)
    };
    // Front-left edge (d..) and front-right edge (w..): short vertical band.
    for (from, to) in [(p(0.0, d), p(w, d)), (p(w, d), p(w, 0.0))] {
        let steps = 14;
        for i in 0..steps {
            let t0 = i as f32 / steps as f32;
            let t1 = (i as f32 + 1.0) / steps as f32;
            let (x0, y0) = (from.0 + (to.0 - from.0) * t0, from.1 + (to.1 - from.1) * t0);
            let (x1, y1) = (from.0 + (to.0 - from.0) * t1, from.1 + (to.1 - from.1) * t1);
            let col = if i % 2 == 0 { yellow } else { black };
            c.fill_poly(&[(x0, y0), (x1, y1), (x1, y1 - band), (x0, y0 - band)], col);
        }
    }
}

/// A small status lamp with an optional glow halo.
pub fn lamp(c: &mut Canvas, x: f32, y: f32, color: Rgba, on: bool) {
    if on {
        c.fill_ellipse_fn(x, y, 4.0, 3.0, |nx, ny| {
            let d = (nx * nx + ny * ny).sqrt();
            let a = ((1.0 - d) * 150.0).clamp(0.0, 150.0) as u8;
            crate::canvas::with_alpha(color, a)
        });
        c.fill_ellipse(x, y, 1.6, 1.4, lerp(color, [255, 255, 255, 255], 0.55));
    } else {
        c.fill_ellipse(x, y, 1.6, 1.4, shade(color, 0.35));
    }
}

/// Rising smoke/steam puffs; `phase` in 0..1 scrolls them upward and fades.
/// `scale` sizes the puffs to the sprite (1.0 suits a chimney, ~2+ a tower).
pub fn puffs(c: &mut Canvas, x: f32, y: f32, phase: f32, color: Rgba, count: u32, rise: f32, scale: f32) {
    for i in 0..count {
        let t = ((phase + i as f32 / count as f32) % 1.0).max(0.001);
        let py = y - t * rise;
        let px = x + (t * 9.0 + i as f32 * 2.3).sin() * (2.0 + t * 5.0) * scale;
        let r = (2.5 + t * 6.5) * scale;
        let a = ((1.0 - t * 0.85) * 210.0) as u8;
        c.fill_ellipse_fn(px, py, r, r * 0.8, |nx, ny| {
            let d = (nx * nx + ny * ny).sqrt();
            crate::canvas::with_alpha(color, ((1.0 - d * d) as f32 * a as f32) as u8)
        });
    }
}
