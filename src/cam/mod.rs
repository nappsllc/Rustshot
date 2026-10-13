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
//! (`bubble.rs`). Linux: V4L2 (`rec::linux::v4l2`) on X11, the camera
//! portal and PipeWire (`rec::linux::pipewire`) on Wayland and in Flatpak,
//! and an X11 ARGB window (`x11.rs`). macOS: an `AVCaptureSession` and a
//! transparent panel (`macos.rs`). The camera is opened only while the
//! bubble is shown and released when it closes. The OS's own effects
//! (Windows Studio Effects, macOS Center Stage and Portrait) apply to the
//! feed by themselves; Rustshot can blur the background behind the person
//! itself ([`bg`]: `rec_camera_bg`, segmented by the embedded model
//! ([`seg`]) or, on macOS 12+, by Vision).
//!
//! Position memory is in-memory (process lifetime, not in the config):
//! the corner the bubble was last dropped at and its offset from that
//! corner, so it follows the selection while the Record popover is open
//! and comes back there for the next recording of the same daemon.
#![cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))] // no bubble elsewhere

#[cfg(windows)]
mod bubble;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "macos")]
mod macos;

pub mod bg;
#[cfg(target_os = "macos")]
mod seg_macos;
pub mod seg;

#[cfg(windows)]
pub use bubble::Bubble;
#[cfg(all(windows, test))]
pub(crate) use bubble::Opener;
#[cfg(target_os = "linux")]
pub use x11::Bubble;
#[cfg(target_os = "macos")]
pub use macos::Bubble;

use crate::capture::IRect;
use crate::theme::Theme;
use crate::uifb::{Fb, C4};
use anyhow::Result;
use std::sync::Mutex;

/// The bubble on platforms without one (never made).
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub struct Bubble;

/// Linux: the id of the camera reached through the camera portal (the
/// portal picks the device; PipeWire's default camera is used).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const PORTAL_CAM: &str = "portal";

/// The capture size asked for first.
pub const PREF: (u32, u32) = (1280, 720);
/// The Camera row's tooltip when there is none.
pub const NO_CAMERA: &str = "No camera found";
pub const MJPG_ONLY: &str = "This camera only offers MJPG, which Rustshot doesn't support yet";
#[cfg_attr(not(windows), allow(dead_code))]
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
    #[cfg_attr(not(windows), allow(dead_code))]
    fn size(&self) -> (u32, u32);
    /// The next picture into `into` (its buffer is reused; blocks for
    /// about one frame); false when the camera had nothing this time (a
    /// gap in the stream, or a picture that was dropped).
    fn read(&mut self, into: &mut CamFrame) -> Result<bool>;
}

/// The cameras on this system (none where there is no capture yet).
#[cfg_attr(test, allow(dead_code))] // tests list fake cameras
pub fn list() -> Vec<CamDevice> {
    #[cfg(windows)]
    {
        win::list()
    }
    #[cfg(target_os = "linux")]
    {
        linux_list()
    }
    #[cfg(target_os = "macos")]
    {
        macos::list()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// Whether the camera portal says there is a camera (`None`: no portal).
#[cfg(target_os = "linux")]
fn portal_camera() -> Option<bool> {
    let mut c = crate::dbus::Conn::session().ok()?;
    crate::rec::linux::portal::camera_present(&mut c).ok()
}

/// Wayland and Flatpak: the camera portal's camera (one entry; the portal
/// chooses the device). X11, or Wayland without a camera portal: V4L2.
#[cfg(target_os = "linux")]
#[cfg_attr(test, allow(dead_code))]
fn linux_list() -> Vec<CamDevice> {
    use crate::rec::linux::portal;
    if portal::needs_portal_now() {
        match portal_camera() {
            Some(true) => return vec![CamDevice { id: PORTAL_CAM.into(), name: "Camera".into() }],
            Some(false) => return Vec::new(),
            // No /dev access in Flatpak without the portal.
            None if portal::in_flatpak() => return Vec::new(),
            None => {}
        }
    }
    crate::rec::linux::v4l2::list()
}

/// Open camera `id` ("" = the first one) at the mode nearest `pref`.
#[cfg_attr(test, allow(dead_code))] // tests use fake cameras
#[cfg_attr(all(target_os = "linux", not(test)), allow(dead_code))] // the bubble opens with `open_or_cancel` there
pub fn open(id: &str, pref: (u32, u32)) -> Result<Box<dyn CamSource>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win::WinCam::open(id, pref)?))
    }
    #[cfg(target_os = "linux")]
    {
        open_or_cancel(id, pref, &std::sync::atomic::AtomicBool::new(false))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacCam::open(id, pref)?))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = (id, pref);
        anyhow::bail!("the camera is not supported on this platform yet")
    }
}

/// Linux: [`open`], with the camera portal's dialog (Wayland, Flatpak)
/// called off once `cancel` is set (the bubble closed meanwhile).
#[cfg(target_os = "linux")]
pub fn open_or_cancel(id: &str, pref: (u32, u32), cancel: &std::sync::atomic::AtomicBool) -> Result<Box<dyn CamSource>> {
    let _ = pref; // 1280x720 asked for; V4L2 falls back to 640x480
    let portal = id == PORTAL_CAM || (id.is_empty() && crate::rec::linux::portal::needs_portal_now() && portal_camera() == Some(true));
    if portal {
        Ok(Box::new(crate::rec::linux::pipewire::PwCam::open(Some(cancel))?))
    } else {
        Ok(Box::new(crate::rec::linux::v4l2::V4l2Cam::open(id)?))
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
    #[cfg_attr(not(test), allow(dead_code))]
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

    /// The Record panel's two choices: round, or the 16 px-radius square
    /// (a sharp `square` from an older config counts as square).
    pub fn is_round(self) -> bool {
        self == Shape::Circle
    }
}

/// The bubble's size: 120 / 160 / 200 / 240 logical px, stepped with the
/// scroll wheel over the bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamSize {
    S,
    M,
    L,
    Xl,
}

impl CamSize {
    #[cfg_attr(target_os = "macos", allow(dead_code))] // the wheel resizes (no scroll events on macOS yet)
    pub const ALL: [CamSize; 4] = [CamSize::S, CamSize::M, CamSize::L, CamSize::Xl];

    /// `rec_camera_size`: "s" | "m" | "l" | "xl".
    pub fn from_config(s: &str) -> CamSize {
        match s {
            "s" => CamSize::S,
            "l" => CamSize::L,
            "xl" => CamSize::Xl,
            _ => CamSize::M,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // the Settings tab writes the strings
    pub fn key(self) -> &'static str {
        match self {
            CamSize::S => "s",
            CamSize::M => "m",
            CamSize::L => "l",
            CamSize::Xl => "xl",
        }
    }

    /// The bubble's side in logical px.
    pub fn logical(self) -> u32 {
        match self {
            CamSize::S => 120,
            CamSize::M => 160,
            CamSize::L => 200,
            CamSize::Xl => 240,
        }
    }

    /// One size up (`dir` > 0: the wheel turned away from the user) or
    /// down, stopping at the ends.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn step(self, dir: i32) -> CamSize {
        let i = CamSize::ALL.iter().position(|z| *z == self).unwrap_or(1) as i32;
        CamSize::ALL[(i + dir.signum()).clamp(0, 3) as usize]
    }
}

/// How the bubble looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub shape: Shape,
    pub size: CamSize,
    /// Flip left-right (a mirror image, what people expect of themselves).
    pub mirror: bool,
    /// The ring's color (opaque); `None` = the theme's accent.
    pub ring: Option<C4>,
    /// The ring's width in logical px; 0 = no ring.
    pub ring_w: u8,
    /// The background behind the person: off, or blurred (light / strong).
    pub bg: bg::Bg,
}

impl Look {
    /// `shape` and `size`, mirrored, with the default accent ring (tests).
    #[cfg(test)]
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn new(shape: Shape, size: CamSize) -> Look {
        Look { shape, size, mirror: true, ring: None, ring_w: crate::config::RING_WIDTH_DEFAULT as u8, bg: bg::Bg::Off }
    }
}

/// The bubble's side in physical px at UI scale `scale`.
pub fn side(size: CamSize, scale: f32) -> u32 {
    (size.logical() as f32 * crate::uisize::clamp(scale)).round() as u32
}

/// Room around the shape for the shadow (physical px).
pub fn pad(scale: f32) -> u32 {
    (16.0 * crate::uisize::clamp(scale)).ceil() as u32
}

/// The ring's width (physical px) for `logical` px at UI scale `scale`.
pub fn ring_width(logical: u8, scale: f32) -> f32 {
    (logical as f32 * crate::uisize::clamp(scale)).round()
}

/// Corner radius of the rounded square: 8 % of its side (16 px at 200).
fn radius(shape: Shape, d: f32) -> f32 {
    match shape {
        Shape::Circle => d / 2.0,
        Shape::Rounded => d * 0.08,
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
    /// Ring coverage, `d`x`d` (`mask - pic`).
    pub ring: Vec<u8>,
    /// The picture's coverage: the shape inside the ring, `d`x`d`.
    pub pic: Vec<u8>,
    ring_c: C4,
    /// What shows until the first picture: BGRA `d`x`d`.
    placeholder: Vec<u8>,
}

/// The template for `look` at `scale` in theme `th` (the ring in the
/// look's color, else the theme's accent).
pub fn template(look: Look, scale: f32, th: &Theme) -> Template {
    let d = side(look.size, scale);
    let pad = pad(scale);
    let w = d + 2 * pad;
    let s = crate::uisize::clamp(scale);
    let (df, rw) = (d as f32, ring_width(look.ring_w, scale));
    // A soft shadow, falling a little: from the distance to the shape
    // moved down, faded out over `blur` (smoothstep).
    let (dy, blur) = (3.0 * s, 10.0 * s);
    let peak = if th.dark { 0.5 } else { 0.3 };
    // With a ring, a 1 px dark halo just outside it (45 %), so the ring
    // reads on light pixels too.
    let halo_w = if rw > 0.0 { s.round().max(1.0) } else { 0.0 };
    let mut shadow = vec![0u8; (w * w * 4) as usize];
    for y in 0..w {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5 - pad as f32, y as f32 + 0.5 - pad as f32);
            let e = sdf(look.shape, df, px, py - dy);
            let t = ((e + blur * 0.25) / blur).clamp(0.0, 1.0);
            let sh = peak * (1.0 - t * t * (3.0 - 2.0 * t));
            let eh = sdf(look.shape, df, px, py);
            let halo = HALO * (0.5 + eh).clamp(0.0, 1.0).min((0.5 - (eh - halo_w)).clamp(0.0, 1.0));
            let a = halo + sh * (1.0 - halo);
            shadow[((y * w + x) * 4 + 3) as usize] = (a * 255.0).round() as u8;
        }
    }
    let mut mask = vec![0u8; (d * d) as usize];
    let mut ring = vec![0u8; (d * d) as usize];
    let mut pic = vec![0u8; (d * d) as usize];
    for y in 0..d {
        for x in 0..d {
            let e = sdf(look.shape, df, x as f32 + 0.5, y as f32 + 0.5);
            let inside = (0.5 - e).clamp(0.0, 1.0);
            // No ring: the picture fills the shape.
            let inner = if rw <= 0.0 { inside } else { (0.5 - (e + rw)).clamp(0.0, 1.0).min(inside) };
            let i = (y * d + x) as usize;
            mask[i] = (inside * 255.0).round() as u8;
            // The picture only inside the ring; the ring takes the rest of
            // the shape, so the edge has no picture fringe.
            pic[i] = ((inner * 255.0).round() as u8).min(mask[i]);
            ring[i] = mask[i] - pic[i];
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
    Template { d, pad, w, shadow, mask, ring, pic, ring_c: look.ring.unwrap_or(th.accent).with_alpha(255), placeholder: ph }
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
            let (rg, pm) = (t.ring[y * d + x] as u32, t.pic[y * d + x] as u32);
            for c in 0..4 {
                let v = if c == 3 { 255 } else { src[c] as u32 };
                // The picture by its coverage, the ring by its own, the
                // shadow where the shape leaves room (rg + pm == m, so a
                // color never exceeds its alpha).
                px[c] = ((v * pm + rc[c] * rg + px[c] as u32 * (255 - m) + 127) / 255) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

/// The halo's opacity (the board's `rgba(0,0,0,.45)`).
const HALO: f32 = 0.45;

/// Gap between the bubble and the area's edges at a corner (logical px).
pub const INSET: f32 = 24.0;
/// The bubble's opacity while it is dragged (what is under it shows).
pub const DRAG_OPACITY: f32 = 0.4;
/// The grip hint under the bubble while the Record panel is open.
pub const GRIP_HINT: &str = "Drag \u{b7} scroll to resize";
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
/// A size the wheel gave the bubble, not yet taken by its owner (X11: set
/// on the bubble's window thread). Tests keep one per thread.
#[cfg(not(test))]
static RESIZED: Mutex<Option<CamSize>> = Mutex::new(None);
#[cfg(test)]
thread_local! {
    static RESIZED: std::cell::Cell<Option<CamSize>> = const { std::cell::Cell::new(None) };
}

/// The bubble was resized with the wheel (the overlay's Record panel
/// follows it: [`take_resized`]).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn note_resized(z: CamSize) {
    #[cfg(not(test))]
    {
        *RESIZED.lock().unwrap_or_else(|e| e.into_inner()) = Some(z);
    }
    #[cfg(test)]
    RESIZED.with(|r| r.set(Some(z)));
}

/// The size the wheel last gave the bubble, once.
pub fn take_resized() -> Option<CamSize> {
    #[cfg(not(test))]
    {
        RESIZED.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
    #[cfg(test)]
    {
        RESIZED.with(|r| r.take())
    }
}

/// The look after one wheel notch over the bubble (`delta` > 0: away from
/// the user, bigger); `None` at either end.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn wheel_look(look: Look, delta: i32) -> Option<Look> {
    let size = look.size.step(delta);
    (delta != 0 && size != look.size).then_some(Look { size, ..look })
}

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

/// Where a `d` px bubble dropped at `pos` comes to rest: the corner
/// position ([`INSET`] in) of the area's quadrant its centre is in.
pub fn settle(pos: (i32, i32), area: IRect, d: i32, scale: f32) -> (i32, i32) {
    let corner = rel_of(pos, area, d, scale).corner;
    place(Rel { corner, dx: INSET, dy: INSET }, area, d, scale)
}

/// A premultiplied BGRA layer at opacity `k` (the bubble while dragged).
pub fn fade_layer(px: &mut [u8], k: f32) {
    let k = (k.clamp(0.0, 1.0) * 255.0).round() as u32;
    for b in px.iter_mut() {
        *b = ((*b as u32 * k + 127) / 255) as u8;
    }
}

fn overlaps(a: IRect, b: IRect) -> bool {
    a.0 < right(b) && b.0 < right(a) && a.1 < bottom(b) && b.1 < bottom(a)
}

fn inside(a: IRect, b: IRect) -> bool {
    a.0 >= b.0 && a.1 >= b.1 && right(a) <= right(b) && bottom(a) <= bottom(b)
}

/// `pos` of a `d` px bubble moved off the rects in `avoid` (the overlay's
/// toolbar and popover): kept when it covers none of them, else the
/// nearest corner position of `area` that is free, else the nearest spot
/// just outside the area on `screen` that is free (a small area), else
/// `pos` as it was.
pub fn clear_of(pos: (i32, i32), area: IRect, d: i32, scale: f32, avoid: &[IRect], screen: IRect) -> (i32, i32) {
    let du = d.max(0) as u32;
    let gap = (8.0 * scale).round() as i32;
    let grown: Vec<IRect> = avoid.iter().map(|r| (r.0 - gap, r.1 - gap, r.2 + 2 * gap as u32, r.3 + 2 * gap as u32)).collect();
    let free = |p: (i32, i32)| !grown.iter().any(|r| overlaps((p.0, p.1, du, du), *r));
    if free(pos) {
        return pos;
    }
    let dist = |p: &(i32, i32)| (p.0 - pos.0).pow(2) + (p.1 - pos.1).pow(2);
    let mut corners: Vec<(i32, i32)> = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight]
        .into_iter()
        .map(|c| place(Rel { corner: c, dx: INSET, dy: INSET }, area, d, scale))
        .filter(|&p| inside((p.0, p.1, du, du), area))
        .collect();
    corners.sort_by_key(dist);
    if let Some(p) = corners.into_iter().find(|&p| free(p)) {
        return p;
    }
    let inset = (INSET * scale).round() as i32;
    let (l, t, r, b) = (area.0, area.1, right(area), bottom(area));
    let mut outside = vec![
        (l - inset - d, b - d),
        (r + inset, b - d),
        (l - inset - d, t),
        (r + inset, t),
        (l, b + inset),
        (r - d, b + inset),
        (l, t - inset - d),
        (r - d, t - inset - d),
    ];
    outside.retain(|&p| inside((p.0, p.1, du, du), screen));
    outside.sort_by_key(dist);
    outside.into_iter().find(|&p| free(p)).unwrap_or(pos)
}

// ---------------------------------------------------------------------------
// Camera pixels
// ---------------------------------------------------------------------------

/// A camera's pixel format, as far as Rustshot cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))] // MJPG and others are only listed on Windows
pub enum Fmt {
    Nv12,
    Yuy2,
    Mjpg,
    Other,
}

/// A mode a camera offers.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
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
#[cfg_attr(not(windows), allow(dead_code))]
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

/// The YUV → RGB matrix of a camera's pictures.
#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
}

/// How a camera's YUV is read: the matrix, and full range (0-255) or
/// limited (16-235, the usual one; a bare [`Matrix`] means limited).
#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Yuv {
    pub matrix: Matrix,
    pub full: bool,
}

impl From<Matrix> for Yuv {
    fn from(matrix: Matrix) -> Yuv {
        Yuv { matrix, full: false }
    }
}

/// Fixed-point (×256) conversion coefficients.
#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
#[derive(Clone, Copy)]
struct Coef {
    /// Y gain and offset.
    yk: i32,
    yo: i32,
    /// R from V, G from U, G from V, B from U.
    rv: i32,
    gu: i32,
    gv: i32,
    bu: i32,
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
fn coef(cs: Yuv) -> Coef {
    let (yk, yo) = if cs.full { (256, 0) } else { (298, 16) };
    let (rv, gu, gv, bu) = match (cs.matrix, cs.full) {
        (Matrix::Bt601, false) => (409, -100, -208, 516),
        (Matrix::Bt709, false) => (459, -55, -136, 541),
        (Matrix::Bt601, true) => (359, -88, -183, 454),
        (Matrix::Bt709, true) => (403, -48, -120, 475),
    };
    Coef { yk, yo, rv, gu, gv, bu }
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
#[inline(always)]
fn yuv_px(y: u8, u: u8, v: u8, k: Coef, out: &mut [u8]) {
    let c = k.yk * (i32::from(y) - k.yo) + 128;
    let (d, e) = (i32::from(u) - 128, i32::from(v) - 128);
    out[0] = ((c + k.bu * d) >> 8).clamp(0, 255) as u8;
    out[1] = ((c + k.gu * d + k.gv * e) >> 8).clamp(0, 255) as u8;
    out[2] = ((c + k.rv * e) >> 8).clamp(0, 255) as u8;
    out[3] = 255;
}

/// YUY2 (Y0 U Y1 V per pixel pair, rows `stride` bytes apart) to BGRA;
/// `None` (and a blank `out`) when `src` is too short for the picture: the
/// caller skips it rather than show black.
#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
pub fn yuy2_to_bgra(src: &[u8], stride: usize, w: u32, h: u32, cs: impl Into<Yuv>, out: &mut Vec<u8>) -> Option<()> {
    let (w, h) = (w as usize & !1, h as usize);
    out.clear();
    out.resize(w * h * 4, 0);
    if h == 0 || stride < w * 2 || src.len() < stride * (h - 1) + w * 2 {
        return None;
    }
    let k = coef(cs.into());
    for y in 0..h {
        let row = &src[y * stride..][..w * 2];
        let o = &mut out[y * w * 4..][..w * 4];
        for (q, p) in row.as_chunks::<4>().0.iter().zip(o.as_chunks_mut::<8>().0) {
            let (a, b) = p.split_at_mut(4);
            yuv_px(q[0], q[1], q[3], k, a);
            yuv_px(q[2], q[1], q[3], k, b);
        }
    }
    Some(())
}

/// NV12 (a Y plane, then interleaved U V at half resolution, both with
/// rows `stride` bytes apart) to BGRA; `None` when a plane is too short
/// (as [`yuy2_to_bgra`]).
#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
pub fn nv12_to_bgra(y_plane: &[u8], uv_plane: &[u8], stride: usize, w: u32, h: u32, cs: impl Into<Yuv>, out: &mut Vec<u8>) -> Option<()> {
    let (w, h) = (w as usize & !1, h as usize & !1);
    out.clear();
    out.resize(w * h * 4, 0);
    if h == 0 || stride < w || y_plane.len() < stride * (h - 1) + w || uv_plane.len() < stride * (h / 2 - 1) + w {
        return None;
    }
    let k = coef(cs.into());
    for y in 0..h {
        let row = &y_plane[y * stride..][..w];
        let uv = &uv_plane[(y / 2) * stride..][..w];
        let o = &mut out[y * w * 4..][..w * 4];
        for x in 0..w {
            let c = &uv[(x & !1)..][..2];
            yuv_px(row[x], c[0], c[1], k, &mut o[x * 4..][..4]);
        }
    }
    Some(())
}

/// Where one picture sits in a locked camera buffer.
#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Bytes from one row to the next.
    pub stride: usize,
    /// NV12: where the UV plane starts (bytes from the start; 0 for YUY2).
    pub uv: usize,
    /// The bytes the picture needs (never more than the buffer has).
    pub len: usize,
}

/// The [`Layout`] of a visible `w`x`h` `fmt` picture in a buffer of `len`
/// bytes with rows `stride` apart; `None` when the buffer is too short for
/// it (the picture is dropped). `coded_h` is the media type's frame height
/// (more than `h` when a display aperture crops it).
///
/// NV12's UV plane starts after the Y plane's rows, which may be more than
/// `h` (padded to a multiple of 16: 1080 → 1088). The first of these that
/// fits wins: `coded_h`; the rows of a padded Y plane plus a padded UV
/// plane (`len / stride = 1.5 * rows`); the rows of a padded Y plane when
/// the UV plane's trailing padding is missing (`len / stride = rows +
/// h / 2`); `h`. Padded heights count only when they are multiples of 8
/// and less than 64 rows over `h`.
#[cfg_attr(target_os = "macos", allow(dead_code))] // Windows and Linux cameras
pub fn layout(fmt: Fmt, w: u32, h: u32, coded_h: u32, stride: usize, len: usize) -> Option<Layout> {
    match fmt {
        Fmt::Yuy2 => {
            let (w, h) = (w as usize & !1, h as usize);
            let need = (stride * h.checked_sub(1)?).checked_add(w * 2)?;
            (w > 0 && stride >= w * 2 && need <= len).then_some(Layout { stride, uv: 0, len: need })
        }
        Fmt::Nv12 => {
            let (w, h) = (w as usize & !1, h as usize & !1);
            if w == 0 || h == 0 || stride < w {
                return None;
            }
            let uv_end = |rows: usize| rows * stride + stride * (h / 2 - 1) + w;
            let rows = len.div_ceil(stride);
            let padded = |r: usize| r >= h && r < h + 64 && r.is_multiple_of(8);
            let mut cands = Vec::with_capacity(4);
            if coded_h as usize > h {
                cands.push(coded_h as usize);
            }
            if rows.is_multiple_of(3) && padded(rows / 3 * 2) {
                cands.push(rows / 3 * 2);
            }
            if let Some(r) = rows.checked_sub(h / 2)
                && padded(r)
            {
                cands.push(r);
            }
            cands.push(h);
            let y_rows = cands.into_iter().find(|&r| uv_end(r) <= len)?;
            Some(Layout { stride, uv: y_rows * stride, len: uv_end(y_rows) })
        }
        Fmt::Mjpg | Fmt::Other => None,
    }
}

/// Turn a BGRA `w`x`h` picture upside down (from a bottom-up buffer).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn flip_rows(bgra: &mut [u8], w: u32, h: u32) {
    let row = w as usize * 4;
    let h = h as usize;
    if bgra.len() < row * h {
        return;
    }
    for y in 0..h / 2 {
        let (top, rest) = bgra.split_at_mut((h - 1 - y) * row);
        top[y * row..][..row].swap_with_slice(&mut rest[..row]);
    }
}

/// The centre square of `f`, resampled (box filter) to `d`x`d`.
#[cfg_attr(not(test), allow(dead_code))] // tests and previews
pub fn fit_square(f: &CamFrame, d: u32) -> CamFrame {
    let mut out = CamFrame::default();
    fit_square_into(f, d, &mut Vec::new(), &mut out);
    out
}

/// [`fit_square`] into `out` with `tmp` as scratch (both reused from one
/// picture to the next: no allocation once they have grown).
pub fn fit_square_into(f: &CamFrame, d: u32, tmp: &mut Vec<u32>, out: &mut CamFrame) {
    let (w, h) = (f.w as usize, f.h as usize);
    let d = d as usize;
    if w == 0 || h == 0 || d == 0 || f.bgra.len() < w * h * 4 {
        out.w = 0;
        out.h = 0;
        out.bgra.clear();
        return;
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
    tmp.clear();
    tmp.resize(s * d * 4, 0);
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
    out.w = d as u32;
    out.h = d as u32;
    out.bgra.clear();
    out.bgra.resize(d * d * 4, 0);
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
            for (o, a) in out.bgra[(j * d + i) * 4..][..4].iter_mut().zip(acc) {
                *o = ((a + n / 2) / n) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests;
