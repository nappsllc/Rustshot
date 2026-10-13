//! Canvas: padding round the picture over a background (a solid color,
//! or "presentation": a soft diagonal gradient of the picture's dominant
//! colors), the picture's corners rounded and a drop shadow under it.

use crate::pixbuf::PixBuf;
use crate::uifb::C4;

/// The presentation look: corners and shadow.
pub const PRESENT_RADIUS: f32 = 12.0;
/// The drop shadow: 24 px blur (a Gaussian of sigma 12, as CSS
/// `blur(24px)` spreads), 25 % black, 8 px down.
pub const SHADOW_BLUR: f32 = 24.0;
pub const SHADOW_ALPHA: f32 = 0.25;
pub const SHADOW_DY: f32 = 8.0;

/// What is round the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bg {
    /// One color (its alpha kept: a transparent background).
    Solid(C4),
    /// A soft gradient of the picture's two dominant colors.
    Presentation,
}

/// The canvas: padding per side (output px), background, corner radius
/// and the shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
    pub bg: Bg,
    pub radius: f32,
    pub shadow: bool,
}

/// The most padding per side (px).
pub const MAX_PAD: u32 = 2000;
/// The largest corner radius (px).
pub const MAX_RADIUS: f32 = 200.0;

impl Pad {
    /// `px` on every side.
    pub fn uniform(px: u32, bg: Bg, radius: f32, shadow: bool) -> Pad {
        Pad { top: px, right: px, bottom: px, left: px, bg, radius, shadow }
    }

    /// The presentation preset: 64 px, the gradient, 12 px corners, the shadow.
    pub fn presentation() -> Pad {
        Pad::uniform(64, Bg::Presentation, PRESENT_RADIUS, true)
    }

    /// Nothing changes (no padding, square corners, no shadow).
    pub fn is_noop(&self) -> bool {
        self.top == 0 && self.right == 0 && self.bottom == 0 && self.left == 0 && self.radius <= 0.0 && !self.shadow
    }

    /// The same padding on all four sides.
    pub fn is_uniform(&self) -> bool {
        self.top == self.right && self.right == self.bottom && self.bottom == self.left
    }

    /// The output size round a `w`×`h` picture.
    pub fn size(&self, (w, h): (u32, u32)) -> (u32, u32) {
        (w + self.left + self.right, h + self.top + self.bottom)
    }

    /// Values in range (from a file: padding, radius).
    pub fn sanitized(mut self) -> Pad {
        for v in [&mut self.top, &mut self.right, &mut self.bottom, &mut self.left] {
            *v = (*v).min(MAX_PAD);
        }
        self.radius = if self.radius.is_finite() { self.radius.clamp(0.0, MAX_RADIUS) } else { 0.0 };
        self
    }
}

/// `src` over `dst` with coverage `cov` (0-255), straight alpha.
fn over(dst: &mut [u8; 4], src: [u8; 4], cov: u32) {
    let sa = src[3] as u32 * cov / 255;
    if sa == 0 {
        return;
    }
    if sa == 255 {
        *dst = src;
        return;
    }
    let da = dst[3] as u32;
    if da == 255 {
        for c in 0..3 {
            dst[c] = ((src[c] as u32 * sa + dst[c] as u32 * (255 - sa) + 127) / 255) as u8;
        }
        return;
    }
    // General: out_a = sa + da (1 - sa).
    let oa = sa * 255 + da * (255 - sa); // x255
    if oa == 0 {
        *dst = [0, 0, 0, 0];
        return;
    }
    for c in 0..3 {
        let v = src[c] as u32 * sa * 255 + dst[c] as u32 * da * (255 - sa);
        dst[c] = ((v + oa / 2) / oa) as u8;
    }
    dst[3] = ((oa + 127) / 255) as u8;
}

/// The two dominant colors of `img` (the most common one, and the most
/// common one clearly different from it; a lighter or darker shade of
/// the first when there is none).
pub fn dominant(img: &PixBuf) -> (C4, C4) {
    let (w, h) = img.dimensions();
    let px = img.as_raw().as_chunks::<4>().0;
    if px.is_empty() {
        return (C4::rgb(230, 232, 240), C4::rgb(200, 206, 222));
    }
    // About 16 000 samples on a grid.
    let step = ((w as usize * h as usize / 16_000) as f64).sqrt().max(1.0) as usize;
    let mut bins = vec![(0u32, [0u32; 3]); 4096];
    for y in (0..h as usize).step_by(step) {
        for x in (0..w as usize).step_by(step) {
            let p = px[y * w as usize + x];
            if p[3] < 128 {
                continue;
            }
            let k = ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
            let b = &mut bins[k];
            b.0 += 1;
            for (s, v) in b.1.iter_mut().zip(p) {
                *s += v as u32;
            }
        }
    }
    let avg = |b: &(u32, [u32; 3])| C4::rgb((b.1[0] / b.0) as u8, (b.1[1] / b.0) as u8, (b.1[2] / b.0) as u8);
    let mut order: Vec<usize> = (0..4096).filter(|&k| bins[k].0 > 0).collect();
    if order.is_empty() {
        return (C4::rgb(230, 232, 240), C4::rgb(200, 206, 222));
    }
    order.sort_by_key(|&k| std::cmp::Reverse(bins[k].0));
    let a = avg(&bins[order[0]]);
    let dist = |p: C4, q: C4| {
        let d = |u: u8, v: u8| (u as f32 - v as f32).powi(2);
        (d(p.r, q.r) + d(p.g, q.g) + d(p.b, q.b)).sqrt()
    };
    let b = order.iter().skip(1).map(|&k| avg(&bins[k])).find(|c| dist(*c, a) > 64.0).unwrap_or_else(|| {
        let lum = 0.299 * a.r as f32 + 0.587 * a.g as f32 + 0.114 * a.b as f32;
        let t = if lum > 128.0 { 0.0 } else { 255.0 };
        let m = |v: u8| (v as f32 + (t - v as f32) * 0.35).round() as u8;
        C4::rgb(m(a.r), m(a.g), m(a.b))
    });
    (a, b)
}

/// The presentation gradient's two ends: the dominant colors softened
/// (a third of the way to white).
pub fn soft((a, b): (C4, C4)) -> (C4, C4) {
    let m = |c: C4| {
        let f = |v: u8| (v as f32 + (255.0 - v as f32) * 0.33).round() as u8;
        C4::rgb(f(c.r), f(c.g), f(c.b))
    };
    (m(a), m(b))
}

/// erf (Abramowitz and Stegun 7.1.26, error below 1.5e-7).
fn erf(x: f32) -> f32 {
    let s = x.signum();
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0 - (((((1.061_405_4 * t - 1.453_152_1) * t) + 1.421_413_8) * t - 0.284_496_74) * t + 0.254_829_6) * t * (-x * x).exp();
    s * y
}

/// The share of a Gaussian (sigma `s`) blur of the span `[a, b)` that
/// lands on the pixel centred at `x`.
fn blurred_span(x: f32, a: f32, b: f32, s: f32) -> f32 {
    let k = std::f32::consts::SQRT_2 * s;
    0.5 * (erf((x - a) / k) - erf((x - b) / k))
}

/// The coverage (0-255) of a rounded rect corner at pixel (`x`, `y`)
/// inside a `w`×`h` rect with corner radius `r` (255 away from them).
fn corner_cov(x: u32, y: u32, w: u32, h: u32, r: f32) -> u32 {
    if r <= 0.0 {
        return 255;
    }
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cx = if px < r { r } else if px > w as f32 - r { w as f32 - r } else { return 255 };
    let cy = if py < r { r } else if py > h as f32 - r { h as f32 - r } else { return 255 };
    let d = (px - cx).hypot(py - cy);
    ((r - d + 0.5).clamp(0.0, 1.0) * 255.0).round() as u32
}

/// `img` on its canvas `p`.
pub fn apply(img: &PixBuf, p: &Pad) -> PixBuf {
    apply_at(img, p, 1.0)
}

/// [`apply`] for a preview drawn at `k` of the size (`p` already scaled):
/// the shadow's blur and offset scaled too.
pub fn apply_at(img: &PixBuf, p: &Pad, k: f32) -> PixBuf {
    let p = p.sanitized();
    let (w, h) = img.dimensions();
    let (ow, oh) = p.size((w, h));
    let mut out = PixBuf::new(ow, oh);
    let opaque = !img.as_raw().as_chunks::<4>().0.iter().any(|q| q[3] != 255);
    let r = p.radius.min(w as f32 / 2.0).min(h as f32 / 2.0).max(0.0);
    let m = r.ceil() as u32 + 1;
    // What the picture hides for sure on row `y` (opaque, away from the
    // corners): nothing of the background or the shadow is drawn there.
    let hidden = |y: u32| -> (usize, usize) {
        let inside = opaque && y >= p.top && y < p.top + h;
        if !inside {
            return (0, 0);
        }
        let iy = y - p.top;
        if iy >= m && iy + m < h { (p.left as usize, (p.left + w) as usize) } else if w > 2 * m { ((p.left + m) as usize, (p.left + w - m) as usize) } else { (0, 0) }
    };
    let row_w = ow as usize * 4;
    // The background: a color per x + y (the gradient runs diagonally).
    let (c0, c1) = match p.bg {
        Bg::Solid(c) => (c, c),
        Bg::Presentation => soft(dominant(img)),
    };
    let span = (ow as f32 + oh as f32).max(1.0);
    let lut: Vec<[u8; 4]> = if matches!(p.bg, Bg::Presentation) {
        (0..(ow + oh) as usize)
            .map(|i| {
                let t = i as f32 / span;
                let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
                [l(c0.r, c1.r), l(c0.g, c1.g), l(c0.b, c1.b), 255]
            })
            .collect()
    } else {
        vec![[c0.r, c0.g, c0.b, c0.a]]
    };
    let grad = lut.len() > 1;
    // The shadow: a blurred rect (its corners' rounding is lost in the blur).
    let s = (SHADOW_BLUR * k / 2.0).max(0.5);
    let (x0, x1) = (p.left as f32, p.left as f32 + w as f32);
    let dy = SHADOW_DY * k;
    let (y0, y1) = (p.top as f32 + dy, p.top as f32 + h as f32 + dy);
    let gx: Vec<f32> = if p.shadow { (0..ow).map(|x| blurred_span(x as f32 + 0.5, x0, x1, s) * SHADOW_ALPHA * 255.0).collect() } else { Vec::new() };
    let d = out.as_raw_mut();
    let threads = if (ow as usize * oh as usize) < 300_000 { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16) };
    let per = (oh as usize).div_ceil(threads).max(1);
    std::thread::scope(|sc| {
        for (ci, chunk) in d.chunks_mut((per * row_w).max(1)).enumerate() {
            let (lut, gx, hidden) = (&lut, &gx, &hidden);
            sc.spawn(move || {
                for (ry, row) in chunk.chunks_exact_mut(row_w.max(1)).enumerate() {
                    let y = (ci * per + ry) as u32;
                    let (hx0, hx1) = hidden(y);
                    let fy = if p.shadow { blurred_span(y as f32 + 0.5, y0, y1, s) } else { 0.0 };
                    let px = row.as_chunks_mut::<4>().0;
                    for range in [0..hx0, hx1..ow as usize] {
                        for x in range {
                            let mut c = if grad { lut[x + y as usize] } else { lut[0] };
                            if fy > 0.0 {
                                let a = (gx[x] * fy).round() as u32;
                                if a > 0 {
                                    over(&mut c, [0, 0, 0, 255], a);
                                }
                            }
                            px[x] = c;
                        }
                    }
                }
            });
        }
    });
    // The picture, its corners rounded.
    let src = img.as_raw();
    let (sw, l, t) = (w as usize * 4, p.left as usize, p.top as usize);
    let m = r.ceil() as u32 + 1;
    for y in 0..h {
        let srow = &src[y as usize * sw..(y as usize + 1) * sw];
        let drow = &mut d[(y as usize + t) * row_w + l * 4..(y as usize + t) * row_w + l * 4 + sw];
        let corner_row = y < m || y + m >= h;
        if opaque && !corner_row {
            drow.copy_from_slice(srow);
            continue;
        }
        for (x, (o, s)) in drow.as_chunks_mut::<4>().0.iter_mut().zip(srow.as_chunks::<4>().0).enumerate() {
            let cov = if corner_row { corner_cov(x as u32, y, w, h, r) } else { 255 };
            over(o, *s, cov);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_background_and_size() {
        let img = PixBuf::from_pixel(10, 6, [255, 0, 0, 255]);
        let p = Pad { top: 1, right: 2, bottom: 3, left: 4, bg: Bg::Solid(C4::rgb(0, 0, 255)), radius: 0.0, shadow: false };
        let out = apply(&img, &p);
        assert_eq!(out.dimensions(), (16, 10));
        assert_eq!(out.get_pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(4, 1), [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(13, 6), [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(14, 6), [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(13, 7), [0, 0, 255, 255]);
        // A transparent background stays transparent.
        let clear = apply(&img, &Pad { bg: Bg::Solid(C4::new(0, 0, 0, 0)), ..p });
        assert_eq!(clear.get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn corners_round_and_the_shadow_falls_below() {
        let img = PixBuf::from_pixel(100, 60, [255, 255, 255, 255]);
        let p = Pad::uniform(40, Bg::Solid(C4::rgb(128, 128, 128)), 12.0, true);
        let out = apply(&img, &p);
        assert_eq!(out.dimensions(), (180, 140));
        // The very corner of the picture shows the background (and a bit of shadow).
        let c = out.get_pixel(40, 40);
        assert!(c[0] < 140, "rounded off: {c:?}");
        // Inside the picture: white.
        assert_eq!(out.get_pixel(90, 70), [255, 255, 255, 255]);
        // Below the picture is darker than above (the shadow is 8 px down).
        let above = out.get_pixel(90, 30)[0];
        let below = out.get_pixel(90, 109)[0];
        assert!(below < above, "below {below}, above {above}");
        assert!((128 - 33..128).contains(&below), "25 % at most: {below}");
        // Far away: the plain background.
        assert_eq!(out.get_pixel(1, 1), [128, 128, 128, 255]);
        // No shadow, no rounding: the corner pixel is the picture's.
        let flat = apply(&img, &Pad::uniform(40, Bg::Solid(C4::rgb(128, 128, 128)), 0.0, false));
        assert_eq!(flat.get_pixel(40, 40), [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(90, 109), [128, 128, 128, 255]);
    }

    #[test]
    fn presentation_uses_the_dominant_colors() {
        // Mostly blue with an orange band.
        let mut img = PixBuf::from_pixel(200, 100, [30, 60, 200, 255]);
        for y in 70..100 {
            for x in 0..200 {
                let i = (y * 200 + x) * 4;
                img.as_raw_mut()[i..i + 4].copy_from_slice(&[240, 140, 20, 255]);
            }
        }
        let (a, b) = dominant(&img);
        assert!(a.b > 150 && a.r < 60, "blue first: {a:?}");
        assert!(b.r > 200 && b.b < 60, "orange second: {b:?}");
        let out = apply(&img, &Pad::presentation());
        let tl = out.get_pixel(0, 0);
        let br = out.get_pixel(out.width() - 1, out.height() - 1);
        assert!(tl[2] > tl[0], "top-left leans blue: {tl:?}");
        assert!(br[0] > br[2], "bottom-right leans orange: {br:?}");
        assert!(tl[0] > 30 && tl[1] > 60, "softened toward white: {tl:?}");
        // One color only: a shade of it.
        let (a, b) = dominant(&PixBuf::from_pixel(10, 10, [20, 20, 20, 255]));
        assert_eq!(a, C4::rgb(20, 20, 20));
        assert!(b.r > 80, "{b:?}");
    }

    #[test]
    fn a_translucent_picture_is_composited() {
        let img = PixBuf::from_pixel(4, 4, [255, 0, 0, 128]);
        let out = apply(&img, &Pad::uniform(2, Bg::Solid(C4::rgb(0, 0, 255)), 0.0, false));
        let p = out.get_pixel(3, 3);
        assert_eq!(p[3], 255);
        assert!((120..=135).contains(&p[0]) && (120..=135).contains(&p[2]), "{p:?}");
    }

    #[test]
    fn no_padding_keeps_the_size() {
        let img = PixBuf::from_pixel(30, 20, [1, 2, 3, 255]);
        let out = apply(&img, &Pad::uniform(0, Bg::Solid(C4::rgb(255, 255, 255)), 0.0, false));
        assert_eq!(out, img);
        assert!(Pad::uniform(0, Bg::Presentation, 0.0, false).is_noop());
        assert!(!Pad::presentation().is_noop());
    }
}
