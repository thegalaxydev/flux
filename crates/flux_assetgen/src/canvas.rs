//! A tiny CPU rasterizer: RGBA canvas plus the handful of primitives the iso
//! art kit needs. No anti-aliasing — crisp pixel-art edges — with ordered
//! (Bayer) dithering for gradient fills so large faces don't band.

pub type Rgba = [u8; 4];

pub const CLEAR: Rgba = [0, 0, 0, 0];

pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    [r, g, b, 255]
}

/// Scale the RGB channels by `f` (clamped), preserving alpha.
pub fn shade(c: Rgba, f: f32) -> Rgba {
    let s = |v: u8| ((v as f32 * f).round().clamp(0.0, 255.0)) as u8;
    [s(c[0]), s(c[1]), s(c[2]), c[3]]
}

pub fn with_alpha(c: Rgba, a: u8) -> Rgba {
    [c[0], c[1], c[2], a]
}

pub fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2]), l(a[3], b[3])]
}

/// 4x4 Bayer matrix threshold in `0..1`, for ordered dithering.
pub fn bayer(x: i32, y: i32) -> f32 {
    const M: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (M[(y & 3) as usize][(x & 3) as usize] as f32 + 0.5) / 16.0
}

/// Deterministic xorshift RNG so every generated asset is reproducible.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed.max(1))
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next_f32() * (hi - lo)
    }
}

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    px: Vec<Rgba>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        Canvas {
            w: w as i32,
            h: h as i32,
            px: vec![CLEAR; (w * h) as usize],
        }
    }

    pub fn get(&self, x: i32, y: i32) -> Rgba {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            CLEAR
        } else {
            self.px[(y * self.w + x) as usize]
        }
    }

    /// Alpha-over blend `c` onto the pixel at `(x, y)`.
    pub fn blend(&mut self, x: i32, y: i32, c: Rgba) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || c[3] == 0 {
            return;
        }
        let i = (y * self.w + x) as usize;
        let dst = self.px[i];
        let sa = c[3] as f32 / 255.0;
        let da = dst[3] as f32 / 255.0;
        let oa = sa + da * (1.0 - sa);
        if oa <= 0.0 {
            self.px[i] = CLEAR;
            return;
        }
        let ch = |s: u8, d: u8| {
            ((s as f32 * sa + d as f32 * da * (1.0 - sa)) / oa).round().clamp(0.0, 255.0) as u8
        };
        self.px[i] = [ch(c[0], dst[0]), ch(c[1], dst[1]), ch(c[2], dst[2]), (oa * 255.0).round() as u8];
    }

    /// Fill a convex or concave polygon by scanline (even-odd).
    pub fn fill_poly(&mut self, pts: &[(f32, f32)], c: Rgba) {
        self.fill_poly_fn(pts, |_, _, _| c);
    }

    /// Polygon fill with a vertical top→bottom dithered gradient.
    pub fn fill_poly_shaded(&mut self, pts: &[(f32, f32)], top: Rgba, bottom: Rgba) {
        let (y0, y1) = pts
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        let span = (y1 - y0).max(1.0);
        self.fill_poly_fn(pts, |x, y, _| {
            // Quantize the gradient into a few steps with Bayer jitter: reads
            // as hand-shaded pixel art rather than a smooth (banded) ramp.
            let t = (y as f32 - y0) / span;
            let steps = 5.0;
            let q = ((t * steps + bayer(x, y) - 0.5).floor() / steps).clamp(0.0, 1.0);
            lerp(top, bottom, q)
        });
    }

    /// Polygon fill where each pixel's color comes from `f(x, y, t_scanline)`.
    pub fn fill_poly_fn(&mut self, pts: &[(f32, f32)], f: impl Fn(i32, i32, f32) -> Rgba) {
        if pts.len() < 3 {
            return;
        }
        let (y_min, y_max) = pts
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        let y0 = y_min.floor().max(0.0) as i32;
        let y1 = y_max.ceil().min(self.h as f32) as i32;
        for y in y0..y1 {
            let yc = y as f32 + 0.5;
            let mut xs: Vec<f32> = Vec::with_capacity(8);
            for i in 0..pts.len() {
                let (x0p, y0p) = pts[i];
                let (x1p, y1p) = pts[(i + 1) % pts.len()];
                if (y0p <= yc && y1p > yc) || (y1p <= yc && y0p > yc) {
                    xs.push(x0p + (yc - y0p) / (y1p - y0p) * (x1p - x0p));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let t = (yc - y_min) / (y_max - y_min).max(1.0);
            for pair in xs.chunks(2) {
                if let [a, b] = pair {
                    let xa = a.round().max(0.0) as i32;
                    let xb = b.round().min(self.w as f32) as i32;
                    for x in xa..xb {
                        let c = f(x, y, t);
                        self.blend(x, y, c);
                    }
                }
            }
        }
    }

    pub fn fill_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Rgba) {
        self.fill_ellipse_fn(cx, cy, rx, ry, |_, _| c);
    }

    /// Ellipse fill where each pixel's colour comes from `f(nx, ny)` with
    /// `nx, ny` the normalized (-1..1) position inside the ellipse.
    pub fn fill_ellipse_fn(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, f: impl Fn(f32, f32) -> Rgba) {
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let y0 = (cy - ry).floor() as i32;
        let y1 = (cy + ry).ceil() as i32;
        for y in y0..=y1 {
            let ny = (y as f32 + 0.5 - cy) / ry;
            if ny.abs() > 1.0 {
                continue;
            }
            let half = rx * (1.0 - ny * ny).sqrt();
            let x0 = (cx - half).round() as i32;
            let x1 = (cx + half).round() as i32;
            for x in x0..x1 {
                let nx = (x as f32 + 0.5 - cx) / rx;
                self.blend(x, y, f(nx, ny));
            }
        }
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba) {
        let (mut x, mut y) = (x0.round() as i32, y0.round() as i32);
        let (xe, ye) = (x1.round() as i32, y1.round() as i32);
        let dx = (xe - x).abs();
        let dy = -(ye - y).abs();
        let sx = if x < xe { 1 } else { -1 };
        let sy = if y < ye { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.blend(x, y, c);
            if x == xe && y == ye {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Darken every opaque pixel that borders transparency (or the canvas
    /// edge): a cohesive 1px outline that keeps each shape's own hue.
    pub fn outline(&mut self) {
        let mut edges = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                if self.get(x, y)[3] < 40 {
                    continue;
                }
                let open = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| self.get(x + dx, y + dy)[3] < 40);
                if open {
                    edges.push((x, y));
                }
            }
        }
        for (x, y) in edges {
            let i = (y * self.w + x) as usize;
            self.px[i] = shade(self.px[i], 0.45);
        }
    }

    /// Soft elliptical ground shadow (blend before drawing the structure).
    pub fn shadow(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, max_alpha: u8) {
        self.fill_ellipse_fn(cx, cy, rx, ry, |nx, ny| {
            let d = (nx * nx + ny * ny).sqrt();
            let a = ((1.0 - d) * 1.6).clamp(0.0, 1.0) * max_alpha as f32;
            [10, 10, 14, a as u8]
        });
    }

    /// Alpha-blend `src` onto this canvas at `(dx, dy)`.
    pub fn blit(&mut self, src: &Canvas, dx: i32, dy: i32) {
        for y in 0..src.h {
            for x in 0..src.w {
                let c = src.get(x, y);
                if c[3] > 0 {
                    self.blend(dx + x, dy + y, c);
                }
            }
        }
    }

    pub fn to_image(&self) -> image::RgbaImage {
        let mut img = image::RgbaImage::new(self.w as u32, self.h as u32);
        for y in 0..self.h {
            for x in 0..self.w {
                img.put_pixel(x as u32, y as u32, image::Rgba(self.get(x, y)));
            }
        }
        img
    }

    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        self.to_image().save(path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_over_is_stable() {
        let mut c = Canvas::new(4, 4);
        c.blend(1, 1, [200, 0, 0, 255]);
        c.blend(1, 1, [0, 200, 0, 128]);
        let p = c.get(1, 1);
        assert_eq!(p[3], 255);
        assert!(p[1] > 90 && p[0] > 90, "half-blend of green over red: {p:?}");
    }

    #[test]
    fn fill_poly_covers_triangle() {
        let mut c = Canvas::new(8, 8);
        c.fill_poly(&[(0.0, 0.0), (8.0, 0.0), (0.0, 8.0)], [255, 255, 255, 255]);
        assert_eq!(c.get(1, 1)[3], 255);
        assert_eq!(c.get(7, 7)[3], 0);
    }

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..10 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }
}
