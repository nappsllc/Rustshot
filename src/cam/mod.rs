//! Camera bubble: the camera's picture in a small always-on-top window
//! (circle, rounded square or square) that is recorded with the screen
//! like any other window.
//!
//! This module holds the platform-neutral parts: the bubble's look
//! ([`Look`]: shape, size S/M/L = 160/240/320 logical px, mirror), its
//! pixels ([`template`] + [`compose`]: anti-aliased shape mask, accent ring,
//! soft shadow, premultiplied BGRA for `UpdateLayeredWindow`), where it
//! sits ([`Rel`], [`snap`]: kept relative to the recorded area, snapping
//! to its corners) and the camera's pixels ([`pick_mode`],
//! [`yuy2_to_bgra`], [`nv12_to_bgra`], [`fit_square`]).
//!
//! Windows: Media Foundation capture (`win.rs`) and the layered window
//! (`bubble.rs`). The camera is opened only while the bubble is shown and
//! released when it closes. Background effects are the OS's (Windows
//! Studio Effects apply to the feed by themselves); Rustshot adds none.
//!
//! Position memory is in-memory (process lifetime, not in the config):
//! the corner the bubble was last dropped at and its offset from that
//! corner, so it follows the selection while the Record popover is open
//! and comes back there for the next recording of the same daemon.
#![cfg_attr(not(windows), allow(dead_code))] // the bubble is Windows-only for now

#[cfg(windows)]
mod bubble;
#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use bubble::Bubble;

use crate::capture::IRect;
use crate::theme::Theme;
use crate::uifb::{Fb, C4};
use anyhow::Result;
use std::sync::Mutex;

/// The bubble on platforms without one (never made).
#[cfg(not(windows))]
pub struct Bubble;

/// The capture size asked for first.
pub const PREF: (u32, u32) = (1280, 720);
/// The Camera row's tooltip when there is none.
pub const NO_CAMERA: &str = "No camera found";
pub const MJPG_ONLY: &str = "This camera only offers MJPG, which Rustshot doesn't support yet";
pub const ACCESS_OFF: &str = "Camera access is off in Windows Settings › Privacy › Camera";
pub const NO_FORMAT: &str = "This camera offers no picture format Rustshot supports";

/// A camera: its id (Windows: the symbolic link) and friendly name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CamDevice {
    pub id: String,
    pub name: String,
}

/// One camera picture, BGRA (alpha 255), rows of `w * 4` bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CamFrame {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

/// An open camera.
pub trait CamSource: Send {
    /// The picture size it delivers.
    fn size(&self) -> (u32, u32);
    /// The next picture (blocks for about one frame); `None` when the
    /// camera had nothing this time (a gap in the stream).
    fn read(&mut self) -> Result<Option<CamFrame>>;
}

/// The cameras on this system (none where there is no capture yet).
pub fn list() -> Vec<CamDevice> {
    #[cfg(windows)]
    {
        win::list()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Open camera `id` ("" = the first one) at the mode nearest `pref`.
#[cfg_attr(test, allow(dead_code))] // tests use fake cameras
pub fn open(id: &str, pref: (u32, u32)) -> Result<Box<dyn CamSource>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win::WinCam::open(id, pref)?))
    }
    #[cfg(not(windows))]
    {
        let _ = (id, pref);
        anyhow::bail!("the camera is not supported on this platform yet")
    }
}

// ---------------------------------------------------------------------------
// Look
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Circle,
    Rounded,
    Square,
}

impl Shape {
    pub const ALL: [Shape; 3] = [Shape::Circle, Shape::Rounded, Shape::Square];

    /// `rec_camera_shape`: "circle" | "rounded" | "square".
    pub fn from_config(s: &str) -> Shape {
        match s {
            "rounded" => Shape::Rounded,
            "square" => Shape::Square,
            _ => Shape::Circle,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // the Settings tab writes it
    pub fn key(self) -> &'static str {
        match self {
            Shape::Circle => "circle",
            Shape::Rounded => "rounded",
            Shape::Square => "square",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Shape::Circle => "Circle",
            Shape::Rounded => "Rounded",
            Shape::Square => "Square",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamSize {
    S,
    M,
    L,
}

impl CamSize {
    pub const ALL: [CamSize; 3] = [CamSize::S, CamSize::M, CamSize::L];

    /// `rec_camera_size`: "s" | "m" | "l".
    pub fn from_config(s: &str) -> CamSize {
        match s {
            "s" => CamSize::S,
            "l" => CamSize::L,
            _ => CamSize::M,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // the Settings tab writes it
    pub fn key(self) -> &'static str {
        match self {
            CamSize::S => "s",
            CamSize::M => "m",
            CamSize::L => "l",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CamSize::S => "S",
            CamSize::M => "M",
            CamSize::L => "L",
        }
    }

    /// The bubble's side in logical px.
    pub fn logical(self) -> u32 {
        match self {
            CamSize::S => 160,
            CamSize::M => 240,
            CamSize::L => 320,
        }
    }
}

/// How the bubble looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub shape: Shape,
    pub size: CamSize,
    /// Flip left-right (a mirror image, what people expect of themselves).
    pub mirror: bool,
}

/// The bubble's side in physical px at UI scale `scale`.
pub fn side(size: CamSize, scale: f32) -> u32 {
    (size.logical() as f32 * scale.clamp(1.0, 4.0)).round() as u32
}

/// Room around the shape for the shadow (physical px).
pub fn pad(scale: f32) -> u32 {
    (16.0 * scale.clamp(1.0, 4.0)).ceil() as u32
}

/// The accent ring's width (physical px).
pub fn ring_width(scale: f32) -> f32 {
    (3.0 * scale.clamp(1.0, 4.0)).round()
}

/// Corner radius of the rounded square: a fifth of its side.
fn radius(shape: Shape, d: f32) -> f32 {
    match shape {
        Shape::Circle => d / 2.0,
        Shape::Rounded => d * 0.2,
        Shape::Square => 0.0,
    }
}

/// Signed distance from (x, y) to the edge of a `d`x`d` `shape` at the
/// origin (negative inside).
pub fn sdf(shape: Shape, d: f32, x: f32, y: f32) -> f32 {
    let h = d / 2.0;
    let r = radius(shape, d);
    let (qx, qy) = ((x - h).abs() - (h - r), (y - h).abs() - (h - r));
    let out = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    out + qx.max(qy).min(0.0) - r
}

/// How much of pixel (`x`, `y`) of a `d`x`d` `shape` is inside it, 0..=1
/// (from the distance at its centre: one pixel of anti-aliasing).
#[cfg_attr(not(test), allow(dead_code))] // tests
pub fn coverage(shape: Shape, d: u32, x: u32, y: u32) -> f32 {
    (0.5 - sdf(shape, d as f32, x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0)
}

/// The fixed parts of the bubble's pixels for one look and scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// The shape's side.
    pub d: u32,
    pub pad: u32,
    /// The window's side (`d + 2 * pad`).
    pub w: u32,
    /// The shadow, premultiplied BGRA `w`x`w`.
    shadow: Vec<u8>,
    /// Shape coverage, `d`x`d`.
    pub mask: Vec<u8>,
    /// Ring coverage, `d`x`d`.
    pub ring: Vec<u8>,
    ring_c: C4,
    /// What shows until the first picture: BGRA `d`x`d`.
    placeholder: Vec<u8>,
}

/// The template for `look` at `scale` in theme `th` (ring in its accent).
pub fn template(look: Look, scale: f32, th: &Theme) -> Template {
    let d = side(look.size, scale);
    let pad = pad(scale);
    let w = d + 2 * pad;
    let s = scale.clamp(1.0, 4.0);
    let (df, rw) = (d as f32, ring_width(scale));
    // A soft shadow, falling a little: from the distance to the shape
    // moved down, faded out over `blur` (smoothstep).
    let (dy, blur) = (3.0 * s, 10.0 * s);
    let peak = if th.dark { 0.5 } else { 0.3 };
    let mut shadow = vec![0u8; (w * w * 4) as usize];
    for y in 0..w {
        for x in 0..w {
            let e = sdf(look.shape, df, x as f32 + 0.5 - pad as f32, y as f32 + 0.5 - pad as f32 - dy);
            let t = ((e + blur * 0.25) / blur).clamp(0.0, 1.0);
            let a = peak * (1.0 - t * t * (3.0 - 2.0 * t));
            shadow[((y * w + x) * 4 + 3) as usize] = (a * 255.0).round() as u8;
        }
    }
    let mut mask = vec![0u8; (d * d) as usize];
    let mut ring = vec![0u8; (d * d) as usize];
    for y in 0..d {
        for x in 0..d {
            let e = sdf(look.shape, df, x as f32 + 0.5, y as f32 + 0.5);
            let inside = (0.5 - e).clamp(0.0, 1.0);
            let inner = (0.5 - (e + rw)).clamp(0.0, 1.0);
            let i = (y * d + x) as usize;
            mask[i] = (inside * 255.0).round() as u8;
            ring[i] = ((inside - inner).max(0.0) * 255.0).round() as u8;
        }
    }
    // Placeholder: a dark field with the camera glyph.
    let mut ph = [28u8, 30, 34, 255].repeat((d * d) as usize);
    {
        let mut f = Fb::new(&mut ph, d as usize);
        let isz = (df * 0.28).round();
        let at = ((df - isz) / 2.0).round();
        crate::icon_path::draw_icon(&mut f, "video", at, at, isz, C4::rgb(150, 156, 168));
    }
    // Drawn RGBA: swap to BGRA.
    for p in ph.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    Template { d, pad, w, shadow, mask, ring, ring_c: th.accent, placeholder: ph }
}

/// The bubble's pixels: `t`'s shadow, the picture `frame` (`t.d` square,
/// see [`fit_square`]; the placeholder when `None`) inside the shape,
/// flipped when `mirror`, and the ring. Premultiplied BGRA `t.w`x`t.w`.
pub fn compose(t: &Template, frame: Option<&CamFrame>, mirror: bool, out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&t.shadow);
    let d = t.d as usize;
    let (pic, flip) = match frame {
        Some(f) if f.w as usize == d && f.h as usize == d && f.bgra.len() >= d * d * 4 => (&f.bgra[..], mirror),
        _ => (&t.placeholder[..], false),
    };
    let rc = [t.ring_c.b as u32, t.ring_c.g as u32, t.ring_c.r as u32, 255u32];
    let (w, pad) = (t.w as usize, t.pad as usize);
    for y in 0..d {
        let row = &pic[y * d * 4..][..d * 4];
        let o = &mut out[((y + pad) * w + pad) * 4..][..d * 4];
        for x in 0..d {
            let m = t.mask[y * d + x] as u32;
            if m == 0 {
                continue;
            }
            let sx = if flip { d - 1 - x } else { x };
            let src = &row[sx * 4..][..4];
            let px = &mut o[x * 4..][..4];
            let rg = t.ring[y * d + x] as u32;
            for c in 0..4 {
                let v = if c == 3 { 255 } else { src[c] as u32 };
                // Picture over the shadow by the mask, then the ring over it.
                let under = (v * m + px[c] as u32 * (255 - m) + 127) / 255;
                px[c] = ((rc[c] * rg + under * (255 - rg) + 127) / 255) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

/// Gap between the bubble and the area's edges at a corner (logical px).
pub const INSET: f32 = 16.0;
/// Within this many logical px of a corner position the bubble snaps to it.
pub const SNAP: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Where the bubble sits relative to the area: the nearest corner and the
/// offset of the bubble's near edges from that corner, inward, in logical
/// px (so it keeps its place when the area or the scale changes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rel {
    pub corner: Corner,
    pub dx: f32,
    pub dy: f32,
}

impl Default for Rel {
    /// Bottom left, inset.
    fn default() -> Rel {
        Rel { corner: Corner::BottomLeft, dx: INSET, dy: INSET }
    }
}

static REMEMBERED: Mutex<Option<Rel>> = Mutex::new(None);

/// Where the bubble was last left (the default before that).
pub fn remembered() -> Rel {
    REMEMBERED.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or_default()
}

pub fn remember(r: Rel) {
    *REMEMBERED.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
}

fn right(a: IRect) -> i32 {
    a.0 + a.2 as i32
}

fn bottom(a: IRect) -> i32 {
    a.1 + a.3 as i32
}

/// Top-left (physical px) of a `d` px bubble placed `rel` in `area`.
pub fn place(rel: Rel, area: IRect, d: i32, scale: f32) -> (i32, i32) {
    let (dx, dy) = ((rel.dx * scale).round() as i32, (rel.dy * scale).round() as i32);
    let x = match rel.corner {
        Corner::TopLeft | Corner::BottomLeft => area.0 + dx,
        Corner::TopRight | Corner::BottomRight => right(area) - dx - d,
    };
    let y = match rel.corner {
        Corner::TopLeft | Corner::TopRight => area.1 + dy,
        Corner::BottomLeft | Corner::BottomRight => bottom(area) - dy - d,
    };
    (x, y)
}

/// The [`Rel`] of a `d` px bubble at `pos`: from the corner of the
/// quadrant its centre is in.
pub fn rel_of(pos: (i32, i32), area: IRect, d: i32, scale: f32) -> Rel {
    let (cx, cy) = (pos.0 + d / 2, pos.1 + d / 2);
    let left = cx < area.0 + area.2 as i32 / 2;
    let top = cy < area.1 + area.3 as i32 / 2;
    let corner = match (top, left) {
        (true, true) => Corner::TopLeft,
        (true, false) => Corner::TopRight,
        (false, true) => Corner::BottomLeft,
        (false, false) => Corner::BottomRight,
    };
    let dx = if left { pos.0 - area.0 } else { right(area) - pos.0 - d };
    let dy = if top { pos.1 - area.1 } else { bottom(area) - pos.1 - d };
    Rel { corner, dx: dx as f32 / scale, dy: dy as f32 / scale }
}

/// `pos` of a `d` px bubble moved onto a corner position of `area`
/// ([`INSET`] in) when it is within [`SNAP`] of it on both axes.
pub fn snap(pos: (i32, i32), area: IRect, d: i32, scale: f32) -> (i32, i32) {
    let reach = (SNAP * scale).round() as i32;
    let corners = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];
    let best = corners
        .into_iter()
        .map(|c| place(Rel { corner: c, dx: INSET, dy: INSET }, area, d, scale))
        .filter(|t| (t.0 - pos.0).abs() <= reach && (t.1 - pos.1).abs() <= reach)
        .min_by_key(|t| (t.0 - pos.0).pow(2) + (t.1 - pos.1).pow(2));
    best.unwrap_or(pos)
}

// ---------------------------------------------------------------------------
// Camera pixels
// ---------------------------------------------------------------------------

/// A camera's pixel format, as far as Rustshot cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Nv12,
    Yuy2,
    Mjpg,
    Other,
}

/// A mode a camera offers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub fmt: Fmt,
    pub w: u32,
    pub h: u32,
    pub fps: f32,
}

/// The mode to use: NV12 or YUY2 (MJPG is not decoded), a smooth rate
/// first (24-30 fps, then faster, then 15+, then slower: a USB 2 camera
/// often has 720p YUY2 only at 10 fps and 480p at 30), then the size
/// nearest `pref` (smaller before larger), NV12 before YUY2.
pub fn pick_mode(modes: &[Mode], pref: (u32, u32)) -> std::result::Result<usize, &'static str> {
    let want = u64::from(pref.0) * u64::from(pref.1);
    let best = modes
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.fmt, Fmt::Nv12 | Fmt::Yuy2) && m.w >= 2 && m.h >= 2)
        .min_by_key(|(_, m)| {
            let rate = if (23.5..=30.5).contains(&m.fps) {
                0
            } else if m.fps > 30.5 {
                1
            } else if m.fps >= 14.5 {
                2
            } else {
                3
            };
            let area = u64::from(m.w) * u64::from(m.h);
            let size = if area <= want { want - area } else { (area - want) * 2 };
            (rate, size, u8::from(m.fmt == Fmt::Yuy2))
        });
    match best {
        Some((i, _)) => Ok(i),
        None if modes.iter().any(|m| m.fmt == Fmt::Mjpg) => Err(MJPG_ONLY),
        None => Err(NO_FORMAT),
    }
}

/// The YUV → RGB matrix of a camera's pictures (limited range).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
}

/// Fixed-point (×256) coefficients: (R from V, G from U, G from V, B from U).
fn coef(m: Matrix) -> (i32, i32, i32, i32) {
    match m {
        Matrix::Bt601 => (409, -100, -208, 516),
        Matrix::Bt709 => (459, -55, -136, 541),
    }
}

#[inline(always)]
fn yuv_px(y: u8, u: u8, v: u8, k: (i32, i32, i32, i32), out: &mut [u8]) {
    let c = 298 * (i32::from(y) - 16) + 128;
    let (d, e) = (i32::from(u) - 128, i32::from(v) - 128);
    out[0] = ((c + k.3 * d) >> 8).clamp(0, 255) as u8;
    out[1] = ((c + k.1 * d + k.2 * e) >> 8).clamp(0, 255) as u8;
    out[2] = ((c + k.0 * e) >> 8).clamp(0, 255) as u8;
    out[3] = 255;
}

/// YUY2 (Y0 U Y1 V per pixel pair, rows `stride` bytes apart) to BGRA.
pub fn yuy2_to_bgra(src: &[u8], stride: usize, w: u32, h: u32, m: Matrix, out: &mut Vec<u8>) {
    let (w, h) = (w as usize & !1, h as usize);
    out.clear();
    out.resize(w * h * 4, 0);
    if stride < w * 2 || src.len() < stride * h.saturating_sub(1) + w * 2 {
        return;
    }
    let k = coef(m);
    for y in 0..h {
        let row = &src[y * stride..][..w * 2];
        let o = &mut out[y * w * 4..][..w * 4];
        for (q, p) in row.as_chunks::<4>().0.iter().zip(o.as_chunks_mut::<8>().0) {
            let (a, b) = p.split_at_mut(4);
            yuv_px(q[0], q[1], q[3], k, a);
            yuv_px(q[2], q[1], q[3], k, b);
        }
    }
}

/// NV12 (a Y plane, then interleaved U V at half resolution, both with
/// rows `stride` bytes apart) to BGRA.
pub fn nv12_to_bgra(y_plane: &[u8], uv_plane: &[u8], stride: usize, w: u32, h: u32, m: Matrix, out: &mut Vec<u8>) {
    let (w, h) = (w as usize & !1, h as usize & !1);
    out.clear();
    out.resize(w * h * 4, 0);
    if stride < w || y_plane.len() < stride * (h - 1) + w || uv_plane.len() < stride * (h / 2 - 1) + w {
        return;
    }
    let k = coef(m);
    for y in 0..h {
        let row = &y_plane[y * stride..][..w];
        let uv = &uv_plane[(y / 2) * stride..][..w];
        let o = &mut out[y * w * 4..][..w * 4];
        for x in 0..w {
            let c = &uv[(x & !1)..][..2];
            yuv_px(row[x], c[0], c[1], k, &mut o[x * 4..][..4]);
        }
    }
}

/// The centre square of `f`, resampled (box filter) to `d`x`d`.
pub fn fit_square(f: &CamFrame, d: u32) -> CamFrame {
    let (w, h) = (f.w as usize, f.h as usize);
    let d = d as usize;
    if w == 0 || h == 0 || d == 0 || f.bgra.len() < w * h * 4 {
        return CamFrame::default();
    }
    let s = w.min(h);
    let (x0, y0) = ((w - s) / 2, (h - s) / 2);
    // Source span [a, b) of destination pixel i.
    let span = |i: usize| {
        let a = i * s / d;
        let b = ((i + 1) * s / d).max(a + 1).min(s);
        (a, b)
    };
    // Horizontal pass: `d` columns for each of the `s` source rows.
    let mut tmp = vec![0u32; s * d * 4];
    for y in 0..s {
        let row = &f.bgra[((y0 + y) * w + x0) * 4..][..s * 4];
        for i in 0..d {
            let (a, b) = span(i);
            let mut acc = [0u32; 4];
            for p in row[a * 4..b * 4].as_chunks::<4>().0 {
                for c in 0..4 {
                    acc[c] += u32::from(p[c]);
                }
            }
            let n = (b - a) as u32;
            for c in 0..4 {
                tmp[(y * d + i) * 4 + c] = (acc[c] + n / 2) / n;
            }
        }
    }
    let mut out = vec![0u8; d * d * 4];
    for j in 0..d {
        let (a, b) = span(j);
        let n = (b - a) as u32;
        for i in 0..d {
            let mut acc = [0u32; 4];
            for y in a..b {
                for c in 0..4 {
                    acc[c] += tmp[(y * d + i) * 4 + c];
                }
            }
            for c in 0..4 {
                out[(j * d + i) * 4 + c] = ((acc[c] + n / 2) / n) as u8;
            }
        }
    }
    CamFrame { w: d as u32, h: d as u32, bgra: out }
}

#[cfg(test)]
mod tests;
