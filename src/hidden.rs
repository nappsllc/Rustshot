//! Windows hidden from screenshots. An app can ask the OS to leave its
//! windows out of every capture: Windows `SetWindowDisplayAffinity`
//! (`WDA_EXCLUDEFROMCAPTURE`: BitBlt, DXGI and Windows.Graphics.Capture
//! all see what is behind the window; `WDA_MONITOR`: they see black),
//! macOS `NSWindow.sharingType = .none`. Password managers do it for their
//! pickers. A capture then shows an "empty" window where the user saw one
//! (window snapping still outlines it, the pixels inside are something
//! else), which reads as a Rustshot bug.
//!
//! So right after the pixel grab (`capture::grab_edit` / `grab_monitor`,
//! and the GDI overlay's `GdiScreen::capture`), [`find`] lists such
//! windows and [`paint`] covers the part of each one that was visible with
//! a neutral placeholder: the theme's panel, the window's rounded corners,
//! a lock and "Hidden from screenshots". It is part of the frozen image,
//! so the overlay, the saved file, the clipboard, History and the Edit
//! window all have it. Nothing of the window is read or drawn: no title,
//! class, app name or content (they may be private too).
//!
//! **Occlusion** is rectangular: a hidden window's visible region is its
//! rect less the rects of the windows above it ([`visible`]), which are
//! taken as opaque boxes. A window above with rounded corners, a drop
//! shadow or see-through parts therefore leaves a sliver of the hidden
//! window unmarked (its corners) or hides a little of the placeholder
//! behind its transparent parts; no window's actual shape is read.
//! Click-through and fully transparent windows above count as not there.
//!
//! Settings › Capture "Mark windows hidden from screenshots"
//! (`mark_hidden_windows`, on by default) turns it off. Recordings and
//! scrolling captures (many frames) are not marked. Linux has no such
//! window flag: [`find`] finds nothing there.

use crate::capture::{MonInfo, Shot};
use crate::fonts::UiFont;
use crate::raster::Order;
use crate::snap::R;
use crate::theme::Theme;
use crate::uifb::{Fb, C4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// A top-level window as the platform lists it, front to back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Top {
    /// Its visible rect (global physical px): the frame as drawn.
    pub r: R,
    /// Hidden from capture: the radius of its rounded corners in px
    /// (0 = square). `None`: an ordinary window (captured, so it covers
    /// what is behind it).
    pub hidden: Option<f32>,
}

/// A hidden window to paint, in image px.
#[derive(Clone, Debug, PartialEq)]
pub struct Hidden {
    /// The whole window (may reach past the image).
    pub rect: R,
    /// `rect` within the image (covered parts included).
    pub seen: R,
    /// Corner radius in px.
    pub radius: f32,
    /// The visible parts: disjoint, inside the image and `rect`.
    pub pieces: Vec<R>,
    /// The UI scale of the window's monitor (text and icon size).
    pub scale: f32,
}

/// More pieces than this and a region stops being cut (a window under a
/// pile of small ones): the rest of the windows above are left out.
const MAX_PIECES: usize = 512;

/// `r` less `cut`: up to four disjoint rects (above, below, left, right
/// of the cut).
pub fn subtract(r: R, cut: R) -> Vec<R> {
    let i = r.intersect(&cut);
    if i.is_empty() {
        return if r.is_empty() { Vec::new() } else { vec![r] };
    }
    let out = [
        R::new(r.x0, r.y0, r.x1, i.y0),
        R::new(r.x0, i.y1, r.x1, r.y1),
        R::new(r.x0, i.y0, i.x0, i.y1),
        R::new(i.x1, i.y0, r.x1, i.y1),
    ];
    out.into_iter().filter(|p| !p.is_empty()).collect()
}

/// The parts of `r` (clipped to `within`) that none of `above` covers,
/// disjoint.
pub fn visible(r: R, above: &[R], within: R) -> Vec<R> {
    let r = r.intersect(&within);
    if r.is_empty() {
        return Vec::new();
    }
    let mut pieces = vec![r];
    for a in above {
        if a.intersect(&r).is_empty() {
            continue;
        }
        if pieces.len() > MAX_PIECES {
            break;
        }
        pieces = pieces.into_iter().flat_map(|p| subtract(p, *a)).collect();
        if pieces.is_empty() {
            break;
        }
    }
    pieces
}

fn area(r: &R) -> i64 {
    i64::from(r.w()) * i64::from(r.h())
}

/// The UI scale at `r`: the monitor holding most of it (1 without one).
fn scale_at(r: R, mons: &[MonInfo]) -> f32 {
    mons.iter()
        .map(|m| (area(&r.intersect(&R::xywh(m.x, m.y, m.w as i32, m.h as i32))), m.scale))
        .filter(|(a, _)| *a > 0)
        .max_by_key(|(a, _)| *a)
        .map_or(1.0, |(_, s)| s.max(0.25))
}

/// The hidden windows of `tops` (front to back) seen in the shot at
/// `origin`/`size`, with what of each is visible, in image px.
pub fn plan(tops: &[Top], origin: (i32, i32), size: (u32, u32), mons: &[MonInfo]) -> Vec<Hidden> {
    let within = R::xywh(origin.0, origin.1, size.0 as i32, size.1 as i32);
    let mut out = Vec::new();
    for (i, t) in tops.iter().enumerate() {
        let Some(radius) = t.hidden else { continue };
        let above: Vec<R> = tops[..i].iter().map(|a| a.r).collect();
        let pieces = visible(t.r, &above, within);
        if pieces.is_empty() {
            continue;
        }
        out.push(Hidden {
            rect: t.r.offset(-origin.0, -origin.1),
            seen: t.r.intersect(&within).offset(-origin.0, -origin.1),
            radius: radius.max(0.0),
            pieces: pieces.into_iter().map(|p| p.offset(-origin.0, -origin.1)).collect(),
            scale: scale_at(t.r, mons),
        });
    }
    out
}

// ------------------------------------------------------------- settings

static ON: AtomicBool = AtomicBool::new(true);
static THEME: Mutex<Option<Theme>> = Mutex::new(None);

/// Take Settings' choice (`mark_hidden_windows`) and this capture's theme
/// for this process (unset: on, dark).
pub fn configure(cfg: &crate::config::Config, th: &Theme) {
    ON.store(cfg.mark_hidden_windows, Ordering::Relaxed);
    *THEME.lock().unwrap_or_else(|e| e.into_inner()) = Some(*th);
}

/// The theme [`configure`] took (dark when it was not called).
pub fn current_theme() -> Theme {
    THEME.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or(crate::theme::DARK)
}

/// Tests: our own windows count as another app's (the live tests' window
/// is the test process' own).
#[cfg(test)]
pub static OWN_TOO: AtomicBool = AtomicBool::new(false);

/// The hidden windows in the shot at `origin`/`size` (image px), none when
/// Settings turned marking off.
pub fn find(origin: (i32, i32), size: (u32, u32)) -> Vec<Hidden> {
    if !ON.load(Ordering::Relaxed) || size.0 == 0 || size.1 == 0 {
        return Vec::new();
    }
    let within = R::xywh(origin.0, origin.1, size.0 as i32, size.1 as i32);
    #[cfg(test)]
    let own = (!OWN_TOO.load(Ordering::Relaxed)).then(std::process::id);
    #[cfg(not(test))]
    let own = Some(std::process::id());
    let Some(tops) = crate::snap::hidden_tops(own, within) else { return Vec::new() };
    if !tops.iter().any(|t| t.hidden.is_some()) {
        return Vec::new();
    }
    let mons = crate::capture::monitors().unwrap_or_default();
    plan(&tops, origin, size, &mons)
}

/// Paint the placeholders over `shot` (its RGBA pixels).
pub fn mark_shot(shot: &mut Shot) {
    let wins = find(shot.origin, shot.size);
    if wins.is_empty() || shot.image.dimensions() != shot.size {
        return;
    }
    let st = Style::of(&current_theme());
    let w = shot.image.width() as usize;
    for h in &wins {
        paint(shot.image.as_raw_mut(), w, (0, 0), Order::Rgba, h, &st);
    }
    crate::perf::note(&format!("hidden windows marked: {}", wins.len()));
}

/// The placeholder colors (from the theme).
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub panel: C4,
    pub stripe: C4,
    pub border: C4,
    pub badge: C4,
    pub icon: C4,
    pub title: C4,
    pub body: C4,
}

impl Style {
    pub fn of(th: &Theme) -> Style {
        let (panel, stripe, border) = if th.dark {
            (C4::rgb(0x1F, 0x21, 0x26), C4::rgb(0x23, 0x25, 0x2B), C4::new(0xFF, 0xFF, 0xFF, 26))
        } else {
            (C4::rgb(0xEC, 0xED, 0xF1), C4::rgb(0xE6, 0xE7, 0xEC), C4::new(0x00, 0x00, 0x00, 24))
        };
        Style { panel, stripe, border, badge: th.accent_bg, icon: th.accent_fg, title: th.text, body: th.text_muted }
    }
}

// ------------------------------------------------------------- painting

/// Logical px of the label: the badge (a circle round the lock), the gap
/// under it, the two lines' sizes and the gap between them, the margin
/// it keeps from the region's edges.
const BADGE: f32 = 44.0;
const ICON: f32 = 22.0;
const GAP: f32 = 12.0;
const TITLE_PX: f32 = 16.0;
const LINE_GAP: f32 = 4.0;
const BODY_PX: f32 = 13.0;
const MARGIN: f32 = 12.0;
/// Smallest scale the label is shrunk to before a line is dropped.
const MIN_K: f32 = 0.6;
/// Stripe period in logical px (diagonal, half of it the stripe).
const STRIPE: f32 = 14.0;

/// What of the label is drawn, at which scale (`k` = UI scale × fit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Label {
    pub k: f32,
    pub title: bool,
    pub body: bool,
    /// Its box (image px), centred where it goes.
    pub at: R,
}

fn label_size(font: &UiFont, title: Option<&str>, body: Option<&str>, k: f32) -> (f32, f32) {
    let mut w = BADGE * k;
    let mut h = BADGE * k;
    if let Some(t) = title {
        w = w.max(font.width(t, TITLE_PX * k));
        h += (GAP + TITLE_PX) * k;
    }
    if let Some(b) = body {
        w = w.max(font.width(b, BODY_PX * k));
        h += (LINE_GAP + BODY_PX) * k;
    }
    (w, h)
}

/// The largest label (all of it, else without the second line, else the
/// lock alone) that fits `room` at the window's UI `scale`, shrunk to at
/// most [`MIN_K`] of it; `None` when not even the lock fits.
pub fn layout(font: &UiFont, title: &str, body: &str, scale: f32, room: R) -> Option<Label> {
    let (rw, rh) = (room.w() as f32 - 2.0 * MARGIN * scale, room.h() as f32 - 2.0 * MARGIN * scale);
    if rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    for (t, b) in [(true, true), (true, false), (false, false)] {
        let (w, h) = label_size(font, t.then_some(title), b.then_some(body), scale);
        let fit = (rw / w).min(rh / h).min(1.0);
        if fit >= MIN_K {
            let k = scale * fit;
            let (w, h) = label_size(font, t.then_some(title), b.then_some(body), k);
            let (cx, cy) = ((room.x0 + room.x1) as f32 / 2.0, (room.y0 + room.y1) as f32 / 2.0);
            let at = R::new((cx - w / 2.0).floor() as i32, (cy - h / 2.0).floor() as i32, (cx + w / 2.0).ceil() as i32, (cy + h / 2.0).ceil() as i32);
            return Some(Label { k, title: t, body: b, at });
        }
    }
    None
}

/// Whether the disjoint `pieces` cover all of `r`.
fn covered(r: R, pieces: &[R]) -> bool {
    pieces.iter().map(|p| area(&p.intersect(&r))).sum::<i64>() >= area(&r)
}

/// Where the label goes: centred on the window as seen in the image when
/// it fits there uncovered, else centred on the largest visible piece.
pub fn place(font: &UiFont, title: &str, body: &str, h: &Hidden) -> Option<Label> {
    let whole = h.seen;
    if let Some(l) = layout(font, title, body, h.scale, whole) {
        let margin = (MARGIN * h.scale).round() as i32;
        if covered(l.at.grow(margin, margin, margin, margin).intersect(&whole), &h.pieces) {
            return Some(l);
        }
    }
    let big = h.pieces.iter().copied().max_by_key(area)?;
    layout(font, title, body, h.scale, big)
}

/// Distance from (x, y) to the edge of the rounded rect `r` (radius
/// `rad`): negative inside.
fn sdf(r: &R, rad: f32, x: f32, y: f32) -> f32 {
    let (cx, cy) = ((r.x0 + r.x1) as f32 / 2.0, (r.y0 + r.y1) as f32 / 2.0);
    let (hx, hy) = (r.w() as f32 / 2.0, r.h() as f32 / 2.0);
    let rad = rad.min(hx).min(hy).max(0.0);
    let (qx, qy) = ((x - cx).abs() - (hx - rad), (y - cy).abs() - (hy - rad));
    let out = qx.max(0.0).hypot(qy.max(0.0));
    out + qx.max(qy).min(0.0) - rad
}

fn mix(a: C4, b: C4, t: f32) -> C4 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    C4::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), 255)
}

/// Paint the placeholder for `h` into `buf` (4 bytes a pixel in `order`,
/// `stride` px a row, its top-left pixel at image px `origin`): only
/// inside `h`'s visible pieces (and the buffer).
pub fn paint(buf: &mut [u8], stride: usize, origin: (i32, i32), order: Order, h: &Hidden, st: &Style) {
    let mut fb = Fb::with_origin(buf, stride, origin.0, origin.1, order);
    let (bx0, by0, bx1, by1) = fb.bounds();
    let bounds = R::new(bx0, by0, bx1, by1);
    let s = h.scale.max(0.25);
    let rad = h.radius;
    let bw = s.round().max(1.0);
    let period = (STRIPE * s).round().max(4.0) as i32;
    // How near an edge a pixel needs the rounded-rect distance (corners,
    // the border); every other pixel is the flat panel.
    let edge = rad.max(0.0) + bw + 1.0;
    let r = h.rect;
    for p in &h.pieces {
        let p = p.intersect(&bounds);
        for y in p.y0..p.y1 {
            let fy = y as f32 + 0.5;
            let near_y = fy < r.y0 as f32 + edge || fy > r.y1 as f32 - edge;
            for x in p.x0..p.x1 {
                let ph = (x - r.x0 + y - r.y0).rem_euclid(period);
                let base = if ph < period / 2 { st.stripe } else { st.panel };
                let fx = x as f32 + 0.5;
                let near = near_y || fx < r.x0 as f32 + edge || fx > r.x1 as f32 - edge;
                if !near {
                    fb.blend_px(x, y, base);
                    continue;
                }
                let d = sdf(&r, rad, fx, fy);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                // The border: `bw` px inside the edge.
                let on = (bw + 0.5 + d).clamp(0.0, 1.0) * st.border.a as f32 / 255.0;
                let c = mix(base, st.border.with_alpha(255), on);
                fb.blend_px(x, y, c.fade(cov));
            }
        }
    }
    let font = crate::fonts::ui_font();
    let (title, body) = (crate::tr!("capture.hidden.title"), crate::tr!("capture.hidden.body"));
    let Some(l) = place(font, title, body, h) else { return };
    draw_label(&mut fb, font, title, body, &l, st);
}

fn draw_label(fb: &mut Fb, font: &UiFont, title: &str, body: &str, l: &Label, st: &Style) {
    let k = l.k;
    let cx = (l.at.x0 + l.at.x1) as f32 / 2.0;
    let mut y = l.at.y0 as f32;
    let d = (BADGE * k).round();
    // The badge on whole px, so the lock is crisp at 100 %.
    let (bx, by) = ((cx - d / 2.0).round(), y.round());
    fb.fill_circle(bx + d / 2.0, by + d / 2.0, d / 2.0, st.badge);
    let ic = ICON * k;
    crate::icon_path::draw_icon(fb, "lock", bx + (d - ic) / 2.0, by + (d - ic) / 2.0, ic, st.icon);
    y += BADGE * k;
    if l.title {
        y += GAP * k;
        let px = TITLE_PX * k;
        let w = font.width(title, px);
        fb.draw_text(font, px, title, (cx - w / 2.0).round(), y.round(), st.title);
        y += px;
    }
    if l.body {
        y += LINE_GAP * k;
        let px = BODY_PX * k;
        let w = font.width(body, px);
        fb.draw_text(font, px, body, (cx - w / 2.0).round(), y.round(), st.body);
    }
}

#[cfg(test)]
#[path = "hidden_tests.rs"]
mod tests;
