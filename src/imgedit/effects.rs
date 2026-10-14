//! Effects, the last stage of the stack (after the canvas): a watermark
//! (text or a PNG image; one of nine anchors plus a margin, opacity, size
//! as a share of the width, or tiled), torn edges (one or more sides, a
//! jagged deterministic outline from a seed), a reflection below (height,
//! starting opacity, fading out) and a drop shadow round it all (offset,
//! blur, color, opacity; the presentation shadow's look by default). They add to
//! the picture and hide nothing of what an older build shows, so they
//! need no stand-in (`"effects"` in annotations.json, ignored by older
//! builds).
//!
//! Every value here is `Copy` (the stack is copied around as a value): the
//! watermark's text or image path lives in a fixed buffer ([`Str`]).

use super::frame::{self, blurred_span, over};
use crate::pixbuf::PixBuf;
use crate::uifb::C4;
use std::sync::{Arc, Mutex};

/// A short string in a fixed buffer (UTF-8, cut at a character boundary).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Str<const N: usize> {
    len: u16,
    b: [u8; N],
}

impl<const N: usize> Str<N> {
    pub fn new(s: &str) -> Self {
        let mut end = s.len().min(N);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let mut b = [0u8; N];
        b[..end].copy_from_slice(&s.as_bytes()[..end]);
        Str { len: end as u16, b }
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.b[..self.len as usize]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for Str<N> {
    fn default() -> Self {
        Str { len: 0, b: [0; N] }
    }
}

impl<const N: usize> std::fmt::Debug for Str<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

/// The most bytes of a watermark's text or image path.
pub const MAX_WM: usize = 512;

/// Where a watermark sits: 0..=8, row by row from the top-left.
pub const ANCHORS: [&str; 9] = ["top-left", "top", "top-right", "left", "center", "right", "bottom-left", "bottom", "bottom-right"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Watermark {
    /// An image (`text` is then a PNG file's path) rather than text.
    pub image: bool,
    pub text: Str<MAX_WM>,
    /// One of [`ANCHORS`].
    pub anchor: u8,
    /// From the picture's edges (px).
    pub margin: u32,
    /// 0..=1.
    pub opacity: f32,
    /// The mark's width as a share of the picture's (percent, 1..=100).
    pub size: f32,
    /// Repeated over the whole picture (the anchor is then ignored).
    pub tiled: bool,
    /// The text's color.
    pub color: C4,
}

impl Default for Watermark {
    fn default() -> Self {
        Watermark { image: false, text: Str::default(), anchor: 8, margin: 24, opacity: 0.6, size: 20.0, tiled: false, color: C4::rgb(255, 255, 255) }
    }
}

pub const MAX_MARGIN: u32 = 2000;

impl Watermark {
    pub fn sanitized(mut self) -> Self {
        self.anchor = self.anchor.min(8);
        self.margin = self.margin.min(MAX_MARGIN);
        self.opacity = fin(self.opacity, 0.6).clamp(0.0, 1.0);
        self.size = fin(self.size, 20.0).clamp(1.0, 100.0);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub dx: i32,
    pub dy: i32,
    /// The blur (px; a Gaussian of half this sigma, as CSS `blur()`).
    pub blur: f32,
    pub color: C4,
    pub opacity: f32,
}

impl Default for Shadow {
    /// The presentation shadow.
    fn default() -> Self {
        Shadow { dx: 0, dy: frame::SHADOW_DY as i32, blur: frame::SHADOW_BLUR, color: C4::rgb(0, 0, 0), opacity: frame::SHADOW_ALPHA }
    }
}

pub const MAX_OFFSET: i32 = 500;
pub const MAX_BLUR: f32 = 200.0;

impl Shadow {
    pub fn sanitized(mut self) -> Self {
        self.dx = self.dx.clamp(-MAX_OFFSET, MAX_OFFSET);
        self.dy = self.dy.clamp(-MAX_OFFSET, MAX_OFFSET);
        self.blur = fin(self.blur, 24.0).clamp(0.0, MAX_BLUR);
        self.opacity = fin(self.opacity, 0.25).clamp(0.0, 1.0);
        self
    }

    /// Room round the picture `[top, right, bottom, left]` (px at `k`).
    fn room(&self, k: f32) -> [u32; 4] {
        let ext = (self.blur * k * 1.5).ceil() as i32;
        let (dx, dy) = ((self.dx as f32 * k).round() as i32, (self.dy as f32 * k).round() as i32);
        [(ext - dy).max(0) as u32, (ext + dx).max(0) as u32, (ext + dy).max(0) as u32, (ext - dx).max(0) as u32]
    }
}

/// Sides: bits of [`TOP`], [`RIGHT`], [`BOTTOM`], [`LEFT`].
pub const TOP: u8 = 1;
pub const RIGHT: u8 = 2;
pub const BOTTOM: u8 = 4;
pub const LEFT: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Torn {
    pub sides: u8,
    /// How deep the tears go (px).
    pub depth: u32,
    /// The average tooth width (px).
    pub tooth: u32,
    pub seed: u32,
}

impl Default for Torn {
    fn default() -> Self {
        Torn { sides: BOTTOM, depth: 12, tooth: 18, seed: 1 }
    }
}

pub const MAX_DEPTH: u32 = 200;
pub const MAX_TOOTH: u32 = 400;

impl Torn {
    pub fn sanitized(mut self) -> Self {
        self.sides &= 15;
        self.depth = self.depth.clamp(1, MAX_DEPTH);
        self.tooth = self.tooth.clamp(2, MAX_TOOTH);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    /// Percent of the picture's height (1..=100).
    pub height: f32,
    /// The reflection's opacity at the top (0..=1), fading to none.
    pub opacity: f32,
    /// Between the picture and its reflection (px).
    pub gap: u32,
}

impl Default for Reflection {
    fn default() -> Self {
        Reflection { height: 30.0, opacity: 0.4, gap: 0 }
    }
}

impl Reflection {
    pub fn sanitized(mut self) -> Self {
        self.height = fin(self.height, 30.0).clamp(1.0, 100.0);
        self.opacity = fin(self.opacity, 0.4).clamp(0.0, 1.0);
        self.gap = self.gap.min(MAX_MARGIN);
        self
    }

    fn rows(&self, h: u32) -> u32 {
        ((h as f32 * self.height / 100.0).round() as u32).clamp(1, h.max(1))
    }
}

fn fin(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// The effects (each one off when `None`), applied in this order:
/// watermark, torn edges, reflection, shadow (round both).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effects {
    pub watermark: Option<Watermark>,
    pub torn: Option<Torn>,
    pub shadow: Option<Shadow>,
    pub reflection: Option<Reflection>,
}

impl Effects {
    pub fn is_empty(&self) -> bool {
        self.watermark.is_none_or(|w| w.text.is_empty()) && self.torn.is_none_or(|t| t.sides == 0) && self.shadow.is_none() && self.reflection.is_none()
    }

    pub fn sanitized(self) -> Self {
        Effects {
            watermark: self.watermark.map(Watermark::sanitized),
            torn: self.torn.map(Torn::sanitized),
            shadow: self.shadow.map(Shadow::sanitized),
            reflection: self.reflection.map(Reflection::sanitized),
        }
    }

    /// The output size for a `w`×`h` picture.
    pub fn size(&self, src: (u32, u32)) -> (u32, u32) {
        self.size_at(src, 1.0)
    }

    fn size_at(&self, (mut w, mut h): (u32, u32), k: f32) -> (u32, u32) {
        let e = self.sanitized();
        if let Some(r) = e.reflection {
            h += (r.gap as f32 * k).round() as u32 + r.rows(h);
        }
        if let Some(s) = e.shadow {
            let [t, r, b, l] = s.room(k);
            w += l + r;
            h += t + b;
        }
        (w, h)
    }
}

/// `img` with the effects (at `k`: a preview or thumbnail drawn at `k` of
/// the size, the effects' px scaled with it).
pub fn apply(img: PixBuf, e: &Effects, k: f32) -> PixBuf {
    let e = e.sanitized();
    let mut img = img;
    if img.width() == 0 || img.height() == 0 {
        return img;
    }
    if let Some(w) = e.watermark.filter(|w| !w.text.is_empty()) {
        watermark(&mut img, &w, k);
    }
    if let Some(t) = e.torn.filter(|t| t.sides != 0) {
        torn(&mut img, &t, k);
    }
    if let Some(r) = e.reflection {
        img = reflection(&img, &r, k);
    }
    if let Some(s) = e.shadow {
        img = shadow(&img, &s, k);
    }
    img
}

// ---- watermark ----

/// An alpha mask (0..=255), `w`×`h`.
struct Mask {
    w: usize,
    h: usize,
    a: Vec<u8>,
}

/// The text's coverage at `px` (the annotation font, else the UI font).
fn text_mask(text: &str, px: f32) -> Mask {
    if let Some(f) = crate::text::AnnotFont::load() {
        let c = f.coverage(text, px);
        // trim to what is drawn
        return trim(Mask { w: c.w, h: c.h, a: c.a });
    }
    let ui = crate::fonts::ui_font();
    let line = text.replace('\n', " ");
    let w = ui.width(&line, px).ceil() as usize + 4;
    let h = ui.height(px).ceil() as usize + 4;
    let mut a = vec![0u8; w * h];
    ui.draw(&line, px, 2.0, 2.0, |x, y, c| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            let i = y as usize * w + x as usize;
            a[i] = a[i].max((c.min(1.0) * 255.0) as u8);
        }
    });
    trim(Mask { w, h, a })
}

fn trim(m: Mask) -> Mask {
    let (mut x0, mut y0, mut x1, mut y1) = (m.w, m.h, 0, 0);
    for y in 0..m.h {
        for x in 0..m.w {
            if m.a[y * m.w + x] > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return Mask { w: 0, h: 0, a: Vec::new() };
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut a = Vec::with_capacity(w * h);
    for y in y0..y1 {
        a.extend_from_slice(&m.a[y * m.w + x0..y * m.w + x1]);
    }
    Mask { w, h, a }
}

/// A PNG file decoded once (the last one asked for is kept).
fn load_png(path: &str) -> Option<Arc<PixBuf>> {
    static CACHE: Mutex<Option<(String, std::time::SystemTime, Arc<PixBuf>)>> = Mutex::new(None);
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, t, img)) = c.as_ref()
        && p == path
        && *t == modified
    {
        return Some(img.clone());
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > 256 << 20 {
        return None;
    }
    let img = Arc::new(PixBuf::from_png(&bytes).ok()?);
    if !super::within_cap(img.dimensions()) {
        return None;
    }
    *c = Some((path.to_string(), modified, img.clone()));
    Some(img)
}

/// The mark as straight-alpha RGBA, already at its size and opacity.
fn mark(w: &Watermark, target_w: u32) -> Option<PixBuf> {
    let target_w = target_w.max(1);
    if w.image {
        let src = load_png(w.text.as_str())?;
        let (sw, sh) = src.dimensions();
        let th = ((sh as f64 * target_w as f64 / sw.max(1) as f64).round() as u32).max(1);
        let mut m = if (target_w, th) == (sw, sh) { (*src).clone() } else { super::resample::resize(&src, target_w, th, super::Filter::Area) };
        for p in m.as_raw_mut().as_chunks_mut::<4>().0 {
            p[3] = (p[3] as f32 * w.opacity).round() as u8;
        }
        return Some(m);
    }
    let text = w.text.as_str().trim();
    if text.is_empty() {
        return None;
    }
    // Size the text so it is `target_w` wide.
    let probe = text_mask(text, 100.0);
    if probe.w == 0 {
        return None;
    }
    let px = (100.0 * target_w as f32 / probe.w as f32).clamp(4.0, 2000.0);
    let t = text_mask(text, px);
    if t.w == 0 {
        return None;
    }
    // A soft dark halo under the text keeps it legible on any picture.
    let halo = (px / 24.0).ceil().max(1.0) as usize;
    let (mw, mh) = (t.w + 2 * halo, t.h + 2 * halo);
    let mut out = PixBuf::new(mw as u32, mh as u32);
    let lum = 0.299 * w.color.r as f32 + 0.587 * w.color.g as f32 + 0.114 * w.color.b as f32;
    let hc = if lum > 128.0 { [0u8, 0, 0] } else { [255u8, 255, 255] };
    let d = out.as_raw_mut();
    let px_of = |x: usize, y: usize| -> u8 { if x >= halo && y >= halo && x - halo < t.w && y - halo < t.h { t.a[(y - halo) * t.w + x - halo] } else { 0 } };
    for y in 0..mh {
        for x in 0..mw {
            let mut hv = 0u8;
            for (ox, oy) in [(0, halo), (2 * halo, halo), (halo, 0), (halo, 2 * halo), (halo + halo / 2, halo + halo / 2)] {
                if x + ox >= halo && y + oy >= halo {
                    hv = hv.max(px_of(x + ox - halo, y + oy - halo));
                }
            }
            let i = (y * mw + x) * 4;
            let mut c = [hc[0], hc[1], hc[2], (hv as f32 * 0.35) as u8];
            let a = px_of(x, y);
            over(&mut c, [w.color.r, w.color.g, w.color.b, 255], a as u32 * w.color.a as u32 / 255);
            c[3] = (c[3] as f32 * w.opacity).round() as u8;
            d[i..i + 4].copy_from_slice(&c);
        }
    }
    Some(out)
}

fn blit_over(dst: &mut PixBuf, src: &PixBuf, x0: i64, y0: i64) {
    let (dw, dh) = (dst.width() as i64, dst.height() as i64);
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    let (xa, xb) = (x0.max(0), (x0 + sw).min(dw));
    let (ya, yb) = (y0.max(0), (y0 + sh).min(dh));
    if xa >= xb || ya >= yb {
        return;
    }
    let s = src.as_raw();
    let d = dst.as_raw_mut();
    for y in ya..yb {
        let sr = ((y - y0) * sw + (xa - x0)) as usize * 4;
        let dr = (y * dw + xa) as usize * 4;
        let n = (xb - xa) as usize;
        let srow = s[sr..sr + n * 4].as_chunks::<4>().0;
        let drow = d[dr..dr + n * 4].as_chunks_mut::<4>().0;
        for (o, p) in drow.iter_mut().zip(srow) {
            if p[3] != 0 {
                over(o, *p, 255);
            }
        }
    }
}

/// Where the mark goes for one anchor.
pub fn anchor_pos(anchor: u8, (w, h): (u32, u32), (mw, mh): (u32, u32), margin: u32) -> (i64, i64) {
    let (col, row) = ((anchor.min(8) % 3) as i64, (anchor.min(8) / 3) as i64);
    let m = margin as i64;
    let place = |c: i64, outer: i64, inner: i64| match c {
        0 => m,
        1 => (outer - inner) / 2,
        _ => outer - inner - m,
    };
    (place(col, w as i64, mw as i64), place(row, h as i64, mh as i64))
}

fn watermark(img: &mut PixBuf, w: &Watermark, k: f32) {
    let (iw, ih) = img.dimensions();
    let target = ((iw as f32 * w.size / 100.0).round() as u32).max(1);
    let Some(m) = mark(w, target) else { return };
    let margin = (w.margin as f32 * k).round() as u32;
    let (mw, mh) = m.dimensions();
    if w.tiled {
        let sx = mw as i64 + (mw as i64 / 2).max(margin as i64);
        let sy = mh as i64 * 3 + margin as i64;
        let mut row = 0;
        let mut y = margin as i64 - mh as i64;
        while y < ih as i64 {
            let off = if row % 2 == 1 { sx / 2 } else { 0 };
            let mut x = margin as i64 - sx + off;
            while x < iw as i64 {
                blit_over(img, &m, x, y);
                x += sx;
            }
            y += sy.max(1);
            row += 1;
        }
    } else {
        let (x, y) = anchor_pos(w.anchor, (iw, ih), (mw, mh), margin);
        blit_over(img, &m, x, y);
    }
}

// ---- torn edges ----

/// A deterministic hash to 0..1.
fn rnd(seed: u32, side: u32, i: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ side.wrapping_mul(0x85EB_CA6B) ^ i.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// The tear's depth along a side of `len` px: jagged, alternating deep
/// and shallow points `tooth` px apart (jittered), joined by lines.
pub fn tear_profile(len: u32, t: &Torn, side: u32, k: f32) -> Vec<f32> {
    let seed = t.seed;
    let depth = (t.depth as f32 * k).max(1.0);
    let tooth = (t.tooth as f32 * k).max(2.0);
    let n = (len as f32 / tooth).ceil() as u32 + 2;
    let pts: Vec<(f32, f32)> = (0..=n)
        .map(|i| {
            let jitter = (rnd(t.seed, side, i * 2) - 0.5) * tooth * 0.6;
            let x = i as f32 * tooth + if i == 0 { 0.0 } else { jitter };
            let r = rnd(t.seed, side, i * 2 + 1);
            let d = if i % 2 == 0 { depth * (0.15 + 0.3 * r) } else { depth * (0.6 + 0.4 * r) };
            (x, d)
        })
        .collect();
    let mut out = Vec::with_capacity(len as usize);
    let mut j = 0;
    for x in 0..len {
        let fx = x as f32 + 0.5;
        while j + 2 < pts.len() && pts[j + 1].0 < fx {
            j += 1;
        }
        let (a, b) = (pts[j], pts[j + 1]);
        let t = if b.0 > a.0 { ((fx - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) } else { 0.0 };
        // fine fibers on the coarse teeth: a smaller tooth, a fifth as deep
        let fine = tooth / 4.0;
        let fi = (fx / fine) as u32;
        let ft = fx / fine - fi as f32;
        let n0 = rnd(seed, side + 8, fi);
        let n1 = rnd(seed, side + 8, fi + 1);
        let fib = (n0 + (n1 - n0) * ft) * depth * 0.2;
        out.push((a.1 + (b.1 - a.1) * t + fib).min(depth));
    }
    out
}

fn torn(img: &mut PixBuf, t: &Torn, k: f32) {
    let (w, h) = img.dimensions();
    let depth = (t.depth as f32 * k).ceil() as u32 + 2;
    let d = img.as_raw_mut();
    let cut = |d: &mut Vec<u8>, x: u32, y: u32, dist: f32, prof: f32| {
        // dist: the pixel center's distance from the edge
        let cov = (dist - prof + 0.5).clamp(0.0, 1.0);
        if cov < 1.0 {
            let i = (y as usize * w as usize + x as usize) * 4 + 3;
            d[i] = (d[i] as f32 * cov).round() as u8;
        }
    };
    for (bit, side) in [(TOP, 0u32), (BOTTOM, 2)] {
        if t.sides & bit == 0 {
            continue;
        }
        let prof = tear_profile(w, t, side, k);
        for x in 0..w {
            for r in 0..depth.min(h) {
                let y = if side == 0 { r } else { h - 1 - r };
                cut(d, x, y, r as f32 + 0.5, prof[x as usize]);
            }
        }
    }
    for (bit, side) in [(RIGHT, 1u32), (LEFT, 3)] {
        if t.sides & bit == 0 {
            continue;
        }
        let prof = tear_profile(h, t, side, k);
        for y in 0..h {
            for r in 0..depth.min(w) {
                let x = if side == 3 { r } else { w - 1 - r };
                cut(d, x, y, r as f32 + 0.5, prof[y as usize]);
            }
        }
    }
}

// ---- drop shadow ----

/// One pass of a running box blur of radius `r` over `n` values `stride`
/// apart starting at `src[o]`.
fn box_line(src: &[f32], dst: &mut [f32], o: usize, n: usize, stride: usize, r: usize, tmp: &mut Vec<f32>) {
    tmp.clear();
    tmp.extend((0..n).map(|i| src[o + i * stride]));
    let inv = 1.0 / (2 * r + 1) as f32;
    let mut acc = 0f32;
    let at = |i: isize| -> f32 { if i < 0 || i >= n as isize { 0.0 } else { tmp[i as usize] } };
    for i in -(r as isize)..=(r as isize) {
        acc += at(i);
    }
    for i in 0..n {
        dst[o + i * stride] = acc * inv;
        acc += at(i as isize + r as isize + 1) - at(i as isize - r as isize);
    }
}

/// Three box blurs approximating a Gaussian of sigma `s` over a `w`×`h`
/// field (rows then columns, in parallel).
fn gauss(field: &mut [f32], w: usize, h: usize, s: f32) {
    if s < 0.3 || w == 0 || h == 0 {
        return;
    }
    // box radii for three passes (Kovesi)
    let n = 3.0;
    let wi = ((12.0 * s * s / n + 1.0).sqrt()).floor() as i32;
    let wl = if wi % 2 == 0 { wi - 1 } else { wi };
    let wu = wl + 2;
    let m = ((12.0 * s * s - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0)).round() as i32;
    let radii: Vec<usize> = (0..3).map(|i| (((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize).collect();
    let threads = if w * h < 200_000 { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16) };
    for &r in &radii {
        // rows
        let src = field.to_vec();
        let per = h.div_ceil(threads).max(1);
        std::thread::scope(|sc| {
            for (ci, chunk) in field.chunks_mut(per * w).enumerate() {
                let src = &src;
                sc.spawn(move || {
                    let mut tmp = Vec::new();
                    let rows = chunk.len() / w;
                    let mut buf = vec![0f32; w];
                    for ry in 0..rows {
                        let y = ci * per + ry;
                        box_line(&src[y * w..(y + 1) * w], &mut buf, 0, w, 1, r, &mut tmp);
                        chunk[ry * w..(ry + 1) * w].copy_from_slice(&buf);
                    }
                });
            }
        });
        // columns, on the transposed field
        let mut t = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                t[x * h + y] = field[y * w + x];
            }
        }
        let src = t.clone();
        let per = w.div_ceil(threads).max(1);
        std::thread::scope(|sc| {
            for (ci, chunk) in t.chunks_mut(per * h).enumerate() {
                let src = &src;
                sc.spawn(move || {
                    let mut tmp = Vec::new();
                    let cols = chunk.len() / h;
                    let mut buf = vec![0f32; h];
                    for rx in 0..cols {
                        let x = ci * per + rx;
                        box_line(&src[x * h..(x + 1) * h], &mut buf, 0, h, 1, r, &mut tmp);
                        chunk[rx * h..(rx + 1) * h].copy_from_slice(&buf);
                    }
                });
            }
        });
        for y in 0..h {
            for x in 0..w {
                field[y * w + x] = t[x * h + y];
            }
        }
    }
}

fn shadow(img: &PixBuf, s: &Shadow, k: f32) -> PixBuf {
    let (w, h) = img.dimensions();
    let [t, r, b, l] = s.room(k);
    let (ow, oh) = (w + l + r, h + t + b);
    let mut out = PixBuf::new(ow, oh);
    let sigma = (s.blur * k / 2.0).max(0.0);
    let (dx, dy) = ((s.dx as f32 * k).round() as i64, (s.dy as f32 * k).round() as i64);
    let (sx0, sy0) = (l as i64 + dx, t as i64 + dy);
    let col = [s.color.r, s.color.g, s.color.b];
    let alpha = s.opacity * s.color.a as f32 / 255.0;
    let opaque = !img.as_raw().as_chunks::<4>().0.iter().any(|q| q[3] != 255);
    let d = out.as_raw_mut();
    let row_w = ow as usize * 4;
    if opaque {
        // A rect: the blur is separable and exact (the presentation shadow).
        let sg = sigma.max(0.5);
        let gx: Vec<f32> = (0..ow).map(|x| blurred_span(x as f32 + 0.5, sx0 as f32, (sx0 + w as i64) as f32, sg)).collect();
        let gy: Vec<f32> = (0..oh).map(|y| blurred_span(y as f32 + 0.5, sy0 as f32, (sy0 + h as i64) as f32, sg) * alpha * 255.0).collect();
        let threads = if (ow as usize * oh as usize) < 300_000 { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16) };
        let per = (oh as usize).div_ceil(threads).max(1);
        std::thread::scope(|sc| {
            for (ci, chunk) in d.chunks_mut((per * row_w).max(1)).enumerate() {
                let (gx, gy) = (&gx, &gy);
                sc.spawn(move || {
                    for (ry, row) in chunk.chunks_exact_mut(row_w.max(1)).enumerate() {
                        let fy = gy[ci * per + ry];
                        if fy < 0.5 {
                            continue;
                        }
                        for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                            let a = (gx[x] * fy).round().min(255.0) as u8;
                            if a > 0 {
                                *p = [col[0], col[1], col[2], a];
                            }
                        }
                    }
                });
            }
        });
    } else {
        // Any shape: the picture's alpha, blurred (at 1/f of the size for
        // a wide blur, then sampled back bilinearly).
        let f = ((sigma / 1.5).floor() as usize).clamp(1, 16);
        let (fw, fh) = ((ow as usize).div_ceil(f) + 1, (oh as usize).div_ceil(f) + 1);
        let mut field = vec![0f32; fw * fh];
        let src = img.as_raw();
        let inv = 1.0 / (f * f) as f32;
        let threads = if (ow as usize * oh as usize) < 300_000 { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16) };
        let per_rows = fh.div_ceil(threads).max(1);
        // the source columns that land in the output
        let xa = (-sx0).max(0) as usize;
        let xb = (ow as i64 - sx0).min(w as i64).max(0) as usize;
        std::thread::scope(|sc| {
            for (ci, band) in field.chunks_mut(per_rows * fw).enumerate() {
                sc.spawn(move || {
                    for (ry, row) in band.chunks_exact_mut(fw).enumerate() {
                        let sy = ci * per_rows + ry;
                        for oy in sy * f..sy * f + f {
                            let y = oy as i64 - sy0;
                            if y < 0 || y >= h as i64 {
                                continue;
                            }
                            let srow = &src[y as usize * w as usize * 4..(y as usize + 1) * w as usize * 4];
                            for x in xa..xb.max(xa) {
                                let a = srow[x * 4 + 3];
                                if a != 0 {
                                    row[(x as i64 + sx0) as usize / f] += a as f32 * inv;
                                }
                            }
                        }
                    }
                });
            }
        });
        gauss(&mut field, fw, fh, sigma / f as f32);
        let per = (oh as usize).div_ceil(threads).max(1);
        let ff = f as f32;
        // per column: the two field cells and the weight (times alpha)
        let xs: Vec<(usize, usize, f32)> = (0..ow as usize)
            .map(|x| {
                let gx = ((x as f32 + 0.5) / ff - 0.5).max(0.0);
                let x0 = (gx as usize).min(fw - 1);
                (x0, (x0 + 1).min(fw - 1), gx - x0 as f32)
            })
            .collect();
        std::thread::scope(|sc| {
            for (ci, chunk) in d.chunks_mut((per * row_w).max(1)).enumerate() {
                let (field, xs) = (&field, &xs);
                sc.spawn(move || {
                    let mut line = vec![0f32; fw];
                    for (ry, row) in chunk.chunks_exact_mut(row_w.max(1)).enumerate() {
                        let y = ci * per + ry;
                        let gy = ((y as f32 + 0.5) / ff - 0.5).max(0.0);
                        let y0 = (gy as usize).min(fh - 1);
                        let (y1, ty) = ((y0 + 1).min(fh - 1), gy - y0 as f32);
                        let (r0, r1) = (&field[y0 * fw..(y0 + 1) * fw], &field[y1 * fw..(y1 + 1) * fw]);
                        let mut any = false;
                        for ((l, &a), &b) in line.iter_mut().zip(r0).zip(r1) {
                            *l = (a + (b - a) * ty) * alpha;
                            any |= *l >= 0.5;
                        }
                        if !any {
                            continue;
                        }
                        for (p, &(x0, x1, tx)) in row.as_chunks_mut::<4>().0.iter_mut().zip(xs) {
                            let v = line[x0] + (line[x1] - line[x0]) * tx;
                            if v >= 0.5 {
                                *p = [col[0], col[1], col[2], (v + 0.5).min(255.0) as u8];
                            }
                        }
                    }
                });
            }
        });
    }
    // The picture over its shadow.
    let src = img.as_raw();
    let sw = w as usize * 4;
    let threads = if (w as usize * h as usize) < 300_000 { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16) };
    let per = (h as usize).div_ceil(threads).max(1);
    let body = &mut d[t as usize * row_w..(t + h) as usize * row_w];
    std::thread::scope(|sc| {
        for (ci, band) in body.chunks_mut(per * row_w).enumerate() {
            sc.spawn(move || {
                for (ry, drow) in band.chunks_exact_mut(row_w).enumerate() {
                    let y = ci * per + ry;
                    let srow = &src[y * sw..(y + 1) * sw];
                    let drow = &mut drow[l as usize * 4..l as usize * 4 + sw];
                    if opaque {
                        drow.copy_from_slice(srow);
                        continue;
                    }
                    for (q, p) in drow.as_chunks_mut::<4>().0.iter_mut().zip(srow.as_chunks::<4>().0) {
                        if p[3] == 255 {
                            *q = *p;
                        } else if p[3] != 0 {
                            over(q, *p, 255);
                        }
                    }
                }
            });
        }
    });
    out
}

// ---- reflection ----

fn reflection(img: &PixBuf, r: &Reflection, k: f32) -> PixBuf {
    let (w, h) = img.dimensions();
    let rows = r.rows(h);
    let gap = (r.gap as f32 * k).round() as u32;
    let oh = h + gap + rows;
    let mut out = PixBuf::new(w, oh);
    let sw = w as usize * 4;
    let src = img.as_raw();
    let d = out.as_raw_mut();
    d[..h as usize * sw].copy_from_slice(&src[..h as usize * sw]);
    for i in 0..rows as usize {
        let sy = h as usize - 1 - i;
        let fade = r.opacity * (1.0 - (i as f32 + 0.5) / rows as f32);
        let o = (h as usize + gap as usize + i) * sw;
        let drow = &mut d[o..o + sw];
        drow.copy_from_slice(&src[sy * sw..(sy + 1) * sw]);
        for p in drow.as_chunks_mut::<4>().0 {
            p[3] = (p[3] as f32 * fade).round() as u8;
        }
    }
    out
}

/// The size [`apply`] gives at `k` (thumbnails).
#[cfg(test)]
pub fn size_at(e: &Effects, src: (u32, u32), k: f32) -> (u32, u32) {
    e.size_at(src, k)
}

#[cfg(test)]
mod tests;
