#![cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))] // no pin window elsewhere
#![cfg_attr(target_os = "macos", allow(dead_code))] // macOS pins: drag and double-click only, for now
//! Pin to screen: the annotated selection as a borderless always-on-top
//! window where it was captured, with a 1 px accent border and the
//! surfaces' soft shadow (Spec "Shadow"). Drag moves it, the wheel zooms
//! it (25–400 %), Ctrl+wheel sets its opacity (20–100 %), a double-click
//! or Esc closes it, a right-click opens its menu (Copy, Save, Save As…,
//! Zoom 100 %, Opacity, Close, Close all pins). Any number of pins; they
//! belong to the daemon (a one-shot `rustshot gui` waits for its last pin),
//! and the tray menu offers "Close all pins" while there are any.
//!
//! This module holds what every platform shares (the clamps and steps,
//! the geometry, the menu, the pixels); the windows are:
//! - Windows: a layered window (`UpdateLayeredWindow`) on the UI thread
//!   (`pin_win.rs`), opacity as the layer's constant alpha.
//! - Linux X11: a managed, undecorated (`_MOTIF_WM_HINTS`), above-others
//!   (`_NET_WM_STATE_ABOVE`) ARGB window on its own thread and Display
//!   (`pin_x11.rs`): an override-redirect window would never get the
//!   keyboard (Esc) and the window manager could not move it.
//! - macOS: a floating, non-activating panel (`wind::Layer`, the floating
//!   level) on the main thread (`pin_macos.rs`): drag and double-click.

use crate::objects::FRect;
use crate::pixbuf::PixBuf;
use crate::theme::Theme;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Zoom and opacity limits, percent.
pub const ZOOM_MIN: u32 = 25;
pub const ZOOM_MAX: u32 = 400;
pub const OPACITY_MIN: u32 = 20;
pub const OPACITY_MAX: u32 = 100;
/// One wheel notch: zoom ×1.1 (or ÷), opacity ±10 points.
const ZOOM_STEP: f32 = 1.1;
const OPACITY_STEP: i32 = 10;
/// The menu's opacity choices.
pub const OPACITIES: [u32; 5] = [100, 80, 60, 40, 20];
/// Two clicks this close (ms, px) are a double-click (X11, macOS; Windows
/// asks the system).
#[cfg_attr(windows, allow(dead_code))] // Windows asks the system
pub const DOUBLE_MS: u128 = 450;
#[cfg_attr(windows, allow(dead_code))]
pub const DOUBLE_PX: i32 = 4;
/// A press that moved this far (physical px) is a drag.
#[cfg_attr(windows, allow(dead_code))]
pub const DRAG_PX: i32 = 3;

/// How a pin is shown: zoom and opacity, percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub zoom: u32,
    pub opacity: u32,
}

impl Default for View {
    fn default() -> Self {
        View { zoom: 100, opacity: 100 }
    }
}

impl View {
    /// The layer's constant alpha (0–255).
    pub fn alpha(self) -> u8 {
        (clamp_opacity(self.opacity) * 255 / 100) as u8
    }
}

pub fn clamp_zoom(z: u32) -> u32 {
    z.clamp(ZOOM_MIN, ZOOM_MAX)
}

pub fn clamp_opacity(o: u32) -> u32 {
    o.clamp(OPACITY_MIN, OPACITY_MAX)
}

/// The zoom `notches` wheel notches from `z` (up: bigger): ×1.1 a notch,
/// at least one percent a notch, clamped; a step across 100 % stops there.
pub fn zoom_step(z: u32, notches: i32) -> u32 {
    let z = clamp_zoom(z);
    if notches == 0 {
        return z;
    }
    let mut next = (z as f32 * ZOOM_STEP.powi(notches)).round() as i64;
    if next == z as i64 {
        next += i64::from(notches.signum());
    }
    let next = (next.clamp(ZOOM_MIN as i64, ZOOM_MAX as i64)) as u32;
    if (z < 100 && next > 100) || (z > 100 && next < 100) { 100 } else { next }
}

/// The opacity `notches` wheel notches from `o` (up: more opaque).
pub fn opacity_step(o: u32, notches: i32) -> u32 {
    clamp_opacity((clamp_opacity(o) as i32 + notches * OPACITY_STEP).max(0) as u32)
}

/// The image's size at `zoom` percent (at least 1×1).
pub fn scaled_size((w, h): (u32, u32), zoom: u32) -> (u32, u32) {
    let k = clamp_zoom(zoom) as f64 / 100.0;
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

/// Where the image's top-left goes when the zoom changes from `z0` to
/// `z1` so the image point under `cursor` stays under it (screen px).
pub fn zoom_about(origin: (i32, i32), cursor: (i32, i32), z0: u32, z1: u32) -> (i32, i32) {
    let k = clamp_zoom(z1) as f64 / clamp_zoom(z0) as f64;
    let f = |o: i32, c: i32| (c as f64 - (c - o) as f64 * k).round() as i32;
    (f(origin.0, cursor.0), f(origin.1, cursor.1))
}

/// Where the image's top-left goes while it is dragged: where it was at
/// the press, moved as far as the pointer.
pub fn dragged(origin: (i32, i32), press: (i32, i32), now: (i32, i32)) -> (i32, i32) {
    (origin.0 + now.0 - press.0, origin.1 + now.1 - press.1)
}

/// The press moved far enough to be a drag.
#[cfg_attr(windows, allow(dead_code))]
pub fn is_drag(press: (i32, i32), now: (i32, i32)) -> bool {
    (now.0 - press.0).abs() > DRAG_PX || (now.1 - press.1).abs() > DRAG_PX
}

/// Two clicks `ms` apart and `d` px apart make a double-click.
#[cfg_attr(windows, allow(dead_code))]
pub fn is_double(ms: u128, d: (i32, i32)) -> bool {
    ms <= DOUBLE_MS && d.0.abs() <= DOUBLE_PX && d.1.abs() <= DOUBLE_PX
}

/// The window's frame around the image (physical px) at UI scale `s`:
/// the border's width and the room for the shadow on every side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub border: i32,
    pub margin: i32,
}

impl Frame {
    pub fn at(s: f32) -> Frame {
        let (dy, sp) = crate::theme::SHADOW[3];
        Frame { border: s.round().max(1.0) as i32, margin: ((dy + sp) * s).ceil() as i32 + 1 }
    }

    /// The image's offset in the window.
    pub fn inset(self) -> i32 {
        self.border + self.margin
    }

    /// The window's size for an image `w`×`h`.
    #[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
    pub fn window(self, (w, h): (u32, u32)) -> (u32, u32) {
        let e = 2 * self.inset() as u32;
        (w + e, h + e)
    }

    /// The window's top-left for the image's top-left `origin`.
    pub fn window_pos(self, origin: (i32, i32)) -> (i32, i32) {
        (origin.0 - self.inset(), origin.1 - self.inset())
    }
}

// --- the menu ----------------------------------------------------------------

/// A pin's menu command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Copy,
    Save,
    SaveAs,
    Zoom100,
    Opacity(u32),
    Close,
    CloseAll,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Cmd {
    /// A menu id (Windows `TrackPopupMenu`), never 0.
    pub fn id(self) -> u32 {
        match self {
            Cmd::Copy => 1,
            Cmd::Save => 2,
            Cmd::SaveAs => 3,
            Cmd::Zoom100 => 4,
            Cmd::Close => 5,
            Cmd::CloseAll => 6,
            Cmd::Opacity(o) => 100 + clamp_opacity(o),
        }
    }

    pub fn from_id(id: u32) -> Option<Cmd> {
        Some(match id {
            1 => Cmd::Copy,
            2 => Cmd::Save,
            3 => Cmd::SaveAs,
            4 => Cmd::Zoom100,
            5 => Cmd::Close,
            6 => Cmd::CloseAll,
            i if (100 + OPACITY_MIN..=100 + OPACITY_MAX).contains(&i) => Cmd::Opacity(i - 100),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item { cmd: Cmd, label: String, checked: bool, enabled: bool },
    /// "Opacity" and its choices.
    Sub { label: &'static str, items: Vec<Entry> },
    Sep,
}

fn item(cmd: Cmd, label: impl Into<String>, checked: bool, enabled: bool) -> Entry {
    Entry::Item { cmd, label: label.into(), checked, enabled }
}

/// The right-click menu of a pin shown at `v`, with `pins` pins up:
/// Copy, Save, Save As…, Zoom 100 % (off at 100 %), Opacity (the current
/// one checked), Close, Close all pins (off with one pin).
pub fn menu(v: View, pins: usize) -> Vec<Entry> {
    let ops = OPACITIES.iter().map(|&o| item(Cmd::Opacity(o), format!("{o} %"), v.opacity == o, true)).collect();
    vec![
        item(Cmd::Copy, "Copy", false, true),
        item(Cmd::Save, "Save", false, true),
        item(Cmd::SaveAs, "Save As\u{2026}", false, true),
        Entry::Sep,
        item(Cmd::Zoom100, "Zoom 100 %", false, v.zoom != 100),
        Entry::Sub { label: "Opacity", items: ops },
        Entry::Sep,
        item(Cmd::Close, "Close", false, true),
        item(Cmd::CloseAll, "Close all pins", false, pins > 1),
    ]
}

/// What `c` makes of the view (Zoom 100 %, an opacity); None for the
/// commands that act on the pin instead.
pub fn apply(v: View, c: Cmd) -> Option<View> {
    match c {
        Cmd::Zoom100 => Some(View { zoom: 100, ..v }),
        Cmd::Opacity(o) => Some(View { opacity: clamp_opacity(o), ..v }),
        _ => None,
    }
}

/// Copy, Save or Save As… of a pin's image (its top-left at `at`): what
/// the export did, said in a toast near `centre`.
#[cfg_attr(test, allow(dead_code))]
pub fn export(img: &PixBuf, at: (i32, i32), centre: (i32, i32), c: Cmd, th: Theme) {
    use crate::export::Task;
    let task = match c {
        Cmd::Copy => Task::Copy,
        Cmd::Save => Task::Save { path: None, ask: false },
        Cmd::SaveAs => Task::Save { path: None, ask: true },
        _ => return,
    };
    let cfg = crate::config::load();
    let res = crate::export::run_export(img, at, &[task], &cfg);
    for m in &res.messages {
        println!("{m}");
    }
    if let Some(n) = crate::notice::compose(&res.done, false)
        && !crate::notice::show(n.clone(), centre, th)
    {
        crate::tray::notify(&n.text);
    }
}

// --- the drawn menu (X11) ----------------------------------------------------

/// A row of the drawn menu: an item (indented under "Opacity" for its
/// choices), the "Opacity" caption, or a separator.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum Row {
    Item { cmd: Cmd, label: String, checked: bool, enabled: bool, indent: bool },
    Caption(&'static str),
    Sep,
}

/// Board "menu" metrics (logical px): 6 padding, 30 high rows, 9 high
/// separators, 200 wide.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MENU_W: f32 = 200.0;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MENU_PAD: f32 = 6.0;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const ROW_H: f32 = 30.0;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const SEP_H: f32 = 9.0;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MENU_ROOM: f32 = 16.0;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct MenuLayout {
    /// The window (panel and shadow room), physical px.
    pub size: (u32, u32),
    pub panel: FRect,
    pub rows: Vec<(Row, FRect)>,
}

/// The drawn menu's rows: the submenu flattened under its caption.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn rows(entries: &[Entry]) -> Vec<Row> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            Entry::Item { cmd, label, checked, enabled } => {
                out.push(Row::Item { cmd: *cmd, label: label.clone(), checked: *checked, enabled: *enabled, indent: false })
            }
            Entry::Sub { label, items } => {
                out.push(Row::Caption(label));
                for i in items {
                    if let Entry::Item { cmd, label, checked, enabled } = i {
                        out.push(Row::Item { cmd: *cmd, label: label.clone(), checked: *checked, enabled: *enabled, indent: true });
                    }
                }
            }
            Entry::Sep => out.push(Row::Sep),
        }
    }
    out
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn menu_layout(entries: &[Entry], s: f32) -> MenuLayout {
    let px = |v: f32| (v * s).round();
    let (x0, y0) = (px(MENU_ROOM), px(MENU_ROOM / 2.0));
    let mut y = y0 + px(MENU_PAD);
    let mut rows_out = Vec::new();
    for r in rows(entries) {
        let h = if r == Row::Sep { px(SEP_H) } else { px(ROW_H) };
        rows_out.push((r, FRect { x: x0 + px(MENU_PAD), y, w: px(MENU_W - 2.0 * MENU_PAD), h }));
        y += h;
    }
    let panel = FRect { x: x0, y: y0, w: px(MENU_W), h: y + px(MENU_PAD) - y0 };
    let size = ((panel.x1() + px(MENU_ROOM)) as u32, (panel.y1() + px(MENU_ROOM * 1.5)) as u32);
    MenuLayout { size, panel, rows: rows_out }
}

/// The enabled command of the row under `p` (window px).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn menu_at(l: &MenuLayout, p: crate::objects::Pt) -> Option<Cmd> {
    l.rows.iter().find_map(|(r, b)| match r {
        Row::Item { cmd, enabled: true, .. } if p.x >= b.x && p.x < b.x1() && p.y >= b.y && p.y < b.y1() => Some(*cmd),
        _ => None,
    })
}

/// Draw the menu (premultiplied BGRA of [`MenuLayout::size`]); `hover`:
/// the command under the pointer.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn render_menu(l: &MenuLayout, th: &Theme, s: f32, hover: Option<Cmd>) -> Vec<u8> {
    use crate::editor::style::{fill, surface, Ui, ALL_LAYERS};
    let ui = Ui { th, s, font: Some(crate::fonts::ui_font()) };
    let font = crate::fonts::ui_font();
    let (w, h) = l.size;
    crate::rec_ui::render_layer(w as usize, h as usize, |f| {
        surface(f, &ui, l.panel, 10.0, &ALL_LAYERS, 1.0);
        for (r, b) in &l.rows {
            match r {
                Row::Sep => {
                    let y = (b.y + b.h / 2.0).round() as i32;
                    f.fill_rect((b.x + 6.0 * s) as i32, y, (b.w - 12.0 * s) as i32, s.round().max(1.0) as i32, th.separator);
                }
                Row::Caption(t) => {
                    let tpx = 11.0 * s;
                    let ty = b.y + b.h / 2.0 - crate::uifb::text_height(font, tpx) / 2.0;
                    f.draw_text(font, tpx, t, b.x + 10.0 * s, ty, th.text_muted);
                }
                Row::Item { cmd, label, checked, enabled, indent } => {
                    let hot = *enabled && hover == Some(*cmd);
                    if hot {
                        fill(f, *b, 6.0 * s, th.accent_bg);
                    }
                    let c = if !enabled {
                        th.text_muted.fade(0.6)
                    } else if hot {
                        th.accent_fg
                    } else {
                        th.text
                    };
                    let x = b.x + if *indent { 36.0 } else { 10.0 } * s;
                    if *checked {
                        let isz = 14.0 * s;
                        crate::icon_path::draw_icon(f, "check", x - 20.0 * s, b.y + (b.h - isz) / 2.0, isz, c);
                    }
                    let tpx = 12.5 * s;
                    let ty = b.y + b.h / 2.0 - crate::uifb::text_height(font, tpx) / 2.0;
                    f.draw_text(font, tpx, label, x, ty, c);
                }
            }
        }
    })
}

// --- pixels --------------------------------------------------------------------

/// `src` resized to `w`×`h`: an area average when it shrinks (each output
/// pixel the mean of the source pixels it covers), the nearest pixel when
/// it grows (screenshots stay crisp when zoomed in).
pub fn resample(src: &PixBuf, w: u32, h: u32) -> PixBuf {
    let (sw, sh) = src.dimensions();
    if (w, h) == (sw, sh) || sw == 0 || sh == 0 || w == 0 || h == 0 {
        return if (w, h) == (sw, sh) { src.clone() } else { PixBuf::new(w, h) };
    }
    // Per axis: the source spans (start, end) of each output pixel.
    let spans = |n: u32, sn: u32| -> Vec<(f64, f64)> {
        let k = sn as f64 / n as f64;
        (0..n).map(|i| (i as f64 * k, ((i + 1) as f64 * k).min(sn as f64))).collect()
    };
    let (xs, ys) = (spans(w, sw), spans(h, sh));
    let s = src.as_raw();
    // Horizontal pass into f32 rows (sh × w), then vertical.
    let mut tmp = vec![0f32; sh as usize * w as usize * 4];
    for y in 0..sh as usize {
        let row = &s[y * sw as usize * 4..(y + 1) * sw as usize * 4];
        for (x, &(a, b)) in xs.iter().enumerate() {
            let o = &mut tmp[(y * w as usize + x) * 4..][..4];
            sample(row, a, b, 4, o);
        }
    }
    let mut out = PixBuf::new(w, h);
    let d = out.as_raw_mut();
    let col_stride = w as usize * 4;
    for (y, &(a, b)) in ys.iter().enumerate() {
        for x in 0..w as usize {
            let mut acc = [0f32; 4];
            sample_f(&tmp[x * 4..], a, b, col_stride, &mut acc);
            for c in 0..4 {
                d[(y * w as usize + x) * 4 + c] = acc[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The mean over source span [a, b) of a u8 line (`stride` bytes between
/// samples) into `out`; a span under one pixel (growing) takes the
/// nearest one.
fn sample(line: &[u8], a: f64, b: f64, stride: usize, out: &mut [f32]) {
    if b - a <= 1.0 {
        let i = (((a + b) / 2.0).floor() as usize).min(line.len() / stride - 1);
        for c in 0..4 {
            out[c] = line[i * stride + c] as f32;
        }
        return;
    }
    let mut acc = [0f64; 4];
    let mut i = a.floor() as usize;
    while (i as f64) < b && i * stride < line.len() {
        let wgt = ((i + 1) as f64).min(b) - (i as f64).max(a);
        for c in 0..4 {
            acc[c] += line[i * stride + c] as f64 * wgt;
        }
        i += 1;
    }
    for c in 0..4 {
        out[c] = (acc[c] / (b - a)) as f32;
    }
}

fn sample_f(line: &[f32], a: f64, b: f64, stride: usize, out: &mut [f32; 4]) {
    let n = (line.len() + stride - 4) / stride;
    if b - a <= 1.0 {
        let i = (((a + b) / 2.0).floor() as usize).min(n - 1);
        out.copy_from_slice(&line[i * stride..i * stride + 4]);
        return;
    }
    let mut acc = [0f64; 4];
    let mut i = a.floor() as usize;
    while (i as f64) < b && i < n {
        let wgt = ((i + 1) as f64).min(b) - (i as f64).max(a);
        for c in 0..4 {
            acc[c] += line[i * stride + c] as f64 * wgt;
        }
        i += 1;
    }
    for c in 0..4 {
        out[c] = (acc[c] / (b - a)) as f32;
    }
}

/// Coverage of pixel centre (x, y) by the rounded rect `r` (radius `rad`).
pub(crate) fn cover(x: f32, y: f32, r: FRect, rad: f32) -> f32 {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (hx, hy) = (r.w / 2.0 - rad, r.h / 2.0 - rad);
    let (qx, qy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad;
    (0.5 - d).clamp(0.0, 1.0)
}

/// The pin's window pixels (premultiplied BGRA): `img` (already at its
/// zoom) inside a `frame.border` accent border, the four shadow layers
/// around it. `img` is RGBA, opaque.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub fn render(img: &PixBuf, th: &Theme, s: f32) -> (u32, u32, Vec<u8>) {
    render_zoomed(img, 100, th, s).unwrap_or_default()
}

/// [`render`] of `img` at `zoom` percent: shrunk by area average first,
/// grown by sampling the nearest source pixel straight into the window
/// (no zoomed copy). None when the window would be too big
/// ([`MAX_WINDOW_PIXELS`]) or its memory cannot be had.
pub fn render_zoomed(img: &PixBuf, zoom: u32, th: &Theme, s: f32) -> Option<(u32, u32, Vec<u8>)> {
    let (sw, sh) = img.dimensions();
    if sw == 0 || sh == 0 {
        return None;
    }
    let (zw, zh) = scaled_size((sw, sh), zoom);
    if (zw, zh) == (sw, sh) || zw > sw || zh > sh {
        let src = img.as_raw();
        let (xs, ys): (Vec<usize>, Vec<usize>) = (
            (0..zw as u64).map(|x| (x * sw as u64 / zw as u64) as usize).collect(),
            (0..zh as u64).map(|y| (y * sh as u64 / zh as u64) as usize).collect(),
        );
        render_by((zw, zh), th, s, |x, y| {
            let i = (ys[y] * sw as usize + xs[x]) * 4;
            [src[i], src[i + 1], src[i + 2]]
        })
    } else {
        let small = resample(img, zw, zh);
        let src = small.as_raw();
        render_by((zw, zh), th, s, |x, y| {
            let i = (y * zw as usize + x) * 4;
            [src[i], src[i + 1], src[i + 2]]
        })
    }
}

/// The largest window a pin may have (pixels): about 32 Mpx, 128 MB of
/// pixels; [`zoom_cap`] keeps the zoom under it.
pub const MAX_PIXELS: u64 = 32_000_000;
/// Hard limit for [`render_zoomed`] (the frame on top of [`MAX_PIXELS`],
/// or a capture already bigger than it at 100 %).
const MAX_WINDOW_PIXELS: u64 = 600_000_000;

/// The highest zoom for an image `img` (px) on a desktop `screen` (the
/// virtual screen, px): the zoomed image no bigger than the screen and
/// [`MAX_PIXELS`], never below 100 % (the capture fit there already).
pub fn zoom_cap((w, h): (u32, u32), (vw, vh): (u32, u32)) -> u32 {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let k = (vw.max(1) as f64 / w).min(vh.max(1) as f64 / h).min((MAX_PIXELS as f64 / (w * h)).sqrt());
    ((k * 100.0).floor() as u32).clamp(100, ZOOM_MAX)
}

/// The size of the virtual screen (all monitors' bounding box), px.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn screen_size() -> (u32, u32) {
    let mons = crate::capture::monitors().unwrap_or_default();
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &mons {
        x0 = x0.min(m.x);
        y0 = y0.min(m.y);
        x1 = x1.max(m.x + m.w as i32);
        y1 = y1.max(m.y + m.h as i32);
    }
    if mons.is_empty() { (1920, 1080) } else { ((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32) }
}

/// [`zoom_step`], at most `cap`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn zoom_step_capped(z: u32, notches: i32, cap: u32) -> u32 {
    zoom_step(z, notches).min(cap.max(100))
}

/// The window pixels around an image `iw`×`ih` whose RGB at (x, y) is
/// `px(x, y)` (see [`render`]).
fn render_by((iw, ih): (u32, u32), th: &Theme, s: f32, px_at: impl Fn(usize, usize) -> [u8; 3]) -> Option<(u32, u32, Vec<u8>)> {
    let fr = Frame::at(s);
    let e = 2 * fr.inset() as u64;
    let (w64, h64) = (iw as u64 + e, ih as u64 + e);
    let n = w64.checked_mul(h64)?;
    if n > MAX_WINDOW_PIXELS || w64 > u32::MAX as u64 || h64 > u32::MAX as u64 {
        return None;
    }
    let (w, h) = (w64 as u32, h64 as u32);
    let len = usize::try_from(n.checked_mul(4)?).ok()?;
    let mut out = Vec::new();
    out.try_reserve_exact(len).ok()?;
    out.resize(len, 0u8);
    let m = fr.margin;
    let b = fr.border;
    // The bordered rect, window px.
    let rect = FRect { x: m as f32, y: m as f32, w: (iw as i32 + 2 * b) as f32, h: (ih as i32 + 2 * b) as f32 };
    let layers: Vec<(FRect, f32, f32)> = (0..4)
        .map(|i| {
            let (dy, sp) = crate::theme::SHADOW[i];
            let (dy, sp) = (dy * s, sp * s);
            (FRect { x: rect.x - sp, y: rect.y + dy - sp, w: rect.w + 2.0 * sp, h: rect.h + 2.0 * sp }, sp, th.shadow[i].a as f32 / 255.0)
        })
        .collect();
    let acc = th.accent;
    let (x0, y0, x1, y1) = (m, m, m + iw as i32 + 2 * b, m + ih as i32 + 2 * b);
    for y in 0..h as i32 {
        let row = &mut out[y as usize * w as usize * 4..(y as usize + 1) * w as usize * 4];
        for x in 0..w as i32 {
            let px = &mut row[x as usize * 4..x as usize * 4 + 4];
            if x >= x0 && x < x1 && y >= y0 && y < y1 {
                let (ix, iy) = (x - x0 - b, y - y0 - b);
                if ix >= 0 && iy >= 0 && (ix as u32) < iw && (iy as u32) < ih {
                    let [r, g, bl] = px_at(ix as usize, iy as usize);
                    px.copy_from_slice(&[bl, g, r, 255]);
                } else {
                    px.copy_from_slice(&[acc.b, acc.g, acc.r, 255]);
                }
                continue;
            }
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut clear = 1.0f32;
            for (r, rad, a) in &layers {
                clear *= 1.0 - a * cover(fx, fy, *r, *rad);
            }
            let a = ((1.0 - clear) * 255.0).round() as u8;
            px.copy_from_slice(&[0, 0, 0, a]);
        }
    }
    Some((w, h, out))
}

// --- the windows ---------------------------------------------------------------

#[cfg(windows)]
#[path = "pin_win.rs"]
mod win;
#[cfg(target_os = "linux")]
#[path = "pin_x11.rs"]
mod x11;
#[cfg(target_os = "macos")]
#[path = "pin_macos.rs"]
mod macos;

/// The pins up (every platform's windows report here; the tray reads it).
static COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn count() -> usize {
    #[cfg(test)]
    {
        TEST.with(|t| t.borrow().len())
    }
    #[cfg(not(test))]
    COUNT.load(Ordering::SeqCst)
}

/// A backend's pins came or went: the tray menu follows.
#[cfg_attr(test, allow(dead_code))]
fn set_count(n: usize) {
    if COUNT.swap(n, Ordering::SeqCst) != n {
        crate::tray::pins_changed();
    }
}

/// Pin `img` (physical px, RGBA) with its top-left at `at` (global
/// physical px); false when there are no pin windows here (Wayland
/// without XWayland, or the window could not be made).
pub fn open(img: Arc<PixBuf>, at: (i32, i32), th: Theme) -> bool {
    if img.width() == 0 || img.height() == 0 {
        return false;
    }
    #[cfg(test)]
    {
        let _ = th;
        TEST.with(|t| t.borrow_mut().push((img.dimensions(), at)));
        true
    }
    #[cfg(all(windows, not(test)))]
    {
        win::open(img, at, th)
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        x11::open(img, at, th)
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos::open(img, at, th)
    }
    #[cfg(all(not(any(windows, target_os = "linux", target_os = "macos")), not(test)))]
    {
        let _ = (img, at, th);
        false
    }
}

/// Close every pin (the tray's "Close all pins").
pub fn close_all() {
    #[cfg(test)]
    TEST.with(|t| t.borrow_mut().clear());
    #[cfg(all(windows, not(test)))]
    win::close_all();
    #[cfg(all(target_os = "linux", not(test)))]
    x11::close_all();
    #[cfg(all(target_os = "macos", not(test)))]
    macos::close_all();
}

/// Advance the pins where the driver runs them (macOS: AppKit's main
/// thread, every tick); elsewhere they have their own message loops.
pub fn tick() {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::tick();
}

/// Tests: a pin opened (its size, top-left).
#[cfg(test)]
pub type Opened = ((u32, u32), (i32, i32));

#[cfg(test)]
thread_local! {
    /// Tests: the pins opened on this thread.
    static TEST: std::cell::RefCell<Vec<Opened>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Tests: the pins opened on this thread since the last call.
#[cfg(test)]
pub fn take_opened() -> Vec<Opened> {
    TEST.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn zoom_steps_and_clamps() {
        assert_eq!(zoom_step(100, 1), 110);
        assert_eq!(zoom_step(100, -1), 91);
        assert_eq!(zoom_step(110, 1), 121);
        assert_eq!(zoom_step(380, 1), 400, "clamped at 400");
        assert_eq!(zoom_step(400, 3), 400);
        assert_eq!(zoom_step(26, -1), 25, "clamped at 25");
        assert_eq!(zoom_step(25, -5), 25);
        assert_eq!(zoom_step(95, 1), 100, "a step across 100 % stops there");
        assert_eq!(zoom_step(105, -1), 100);
        assert_eq!(zoom_step(100, 0), 100);
        assert_eq!(zoom_step(1000, 0), 400, "out of range comes back in");
        // Every notch moves (small zooms round to the same value otherwise).
        assert!(zoom_step(25, 1) > 25);
        // Up and down again come back near the start.
        let mut z = 100;
        for _ in 0..30 {
            z = zoom_step(z, 1);
        }
        assert_eq!(z, ZOOM_MAX);
        for _ in 0..60 {
            z = zoom_step(z, -1);
        }
        assert_eq!(z, ZOOM_MIN);
    }

    /// The zoom cap: the zoomed image no bigger than the virtual screen
    /// nor 32 Mpx, never under 100 %; the wheel stops there.
    #[test]
    fn the_zoom_is_capped_by_screen_and_pixels() {
        assert_eq!(zoom_cap((200, 100), (3840, 2160)), 400, "small: the full range");
        assert_eq!(zoom_cap((1920, 1080), (3840, 2160)), 200, "twice the size fits the screen");
        assert_eq!(zoom_cap((3840, 2160), (3840, 2160)), 100, "a full screen: 100 %");
        assert_eq!(zoom_cap((7680, 4320), (7680, 4320)), 100, "never under 100 % (it was captured so)");
        // 32 Mpx: a 2000x2000 image on a huge desktop stops at 282 %.
        let c = zoom_cap((2000, 2000), (20000, 20000));
        assert_eq!(c, 282);
        let (w, h) = scaled_size((2000, 2000), c);
        assert!((w as u64) * (h as u64) <= MAX_PIXELS);
        assert_eq!(zoom_cap((0, 0), (0, 0)), 100);
        assert_eq!(zoom_step_capped(190, 1, 200), 200);
        assert_eq!(zoom_step_capped(200, 3, 200), 200);
        assert_eq!(zoom_step_capped(200, -1, 200), 182);
        assert_eq!(zoom_step_capped(100, 1, 50), 100, "the cap is at least 100 %");
    }

    /// Zoomed in, the window is sampled straight from the capture (nearest
    /// pixel); zoomed out, averaged; a window past the hard limit is None.
    #[test]
    fn rendering_zoomed() {
        let img = sample_shot(40, 30);
        let th = &crate::theme::DARK;
        let (w, h, px) = render_zoomed(&img, 200, th, 1.0).unwrap();
        let fr = Frame::at(1.0);
        assert_eq!((w, h), fr.window((80, 60)));
        let i = fr.inset() as u32;
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..4];
        let src = img.get_pixel(39, 29);
        assert_eq!(at(i + 79, i + 59), [src[2], src[1], src[0], 255]);
        let (w2, h2, _) = render_zoomed(&img, 50, th, 1.0).unwrap();
        assert_eq!((w2, h2), fr.window((20, 15)));
        assert_eq!(render_zoomed(&PixBuf::default(), 100, th, 1.0), None);
        // Past the hard limit: refused before anything is allocated.
        assert_eq!(render_by((30000, 30000), th, 1.0, |_, _| [0, 0, 0]), None);
    }

    #[test]
    fn opacity_steps_and_clamps() {
        assert_eq!(opacity_step(100, -1), 90);
        assert_eq!(opacity_step(100, 1), 100);
        assert_eq!(opacity_step(30, -1), 20);
        assert_eq!(opacity_step(20, -3), 20, "never below 20 %");
        assert_eq!(opacity_step(0, 0), 20);
        assert_eq!(opacity_step(50, 2), 70);
        assert_eq!(View { zoom: 100, opacity: 100 }.alpha(), 255);
        assert_eq!(View { zoom: 100, opacity: 20 }.alpha(), 51);
        assert_eq!(View { zoom: 100, opacity: 5 }.alpha(), 51, "clamped");
        assert_eq!(View::default(), View { zoom: 100, opacity: 100 });
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        // A 200×100 image at (1000, 500), the pointer on its centre.
        let o = zoom_about((1000, 500), (1100, 550), 100, 200);
        assert_eq!(o, (900, 450));
        assert_eq!(scaled_size((200, 100), 200), (400, 200));
        // The pointer's image point: (100, 50) before and after.
        assert_eq!(((1100 - o.0) / 2, (550 - o.1) / 2), (100, 50));
        // At the top-left corner nothing moves.
        assert_eq!(zoom_about((1000, 500), (1000, 500), 100, 25), (1000, 500));
        // Negative coordinates (a monitor left of the primary).
        assert_eq!(zoom_about((-1920, 0), (-1820, 100), 100, 50), (-1870, 50));
        assert_eq!(scaled_size((3, 3), 25), (1, 1), "never empty");
        assert_eq!(scaled_size((640, 480), 1000), (2560, 1920), "clamped to 400 %");
    }

    #[test]
    fn drags_and_double_clicks() {
        assert_eq!(dragged((100, 100), (150, 120), (170, 90)), (120, 70));
        assert!(!is_drag((10, 10), (13, 7)));
        assert!(is_drag((10, 10), (14, 10)));
        assert!(is_double(300, (2, -3)));
        assert!(!is_double(600, (0, 0)));
        assert!(!is_double(100, (5, 0)));
    }

    #[test]
    fn the_frame_holds_the_border_and_the_shadow() {
        let f = Frame::at(1.0);
        assert_eq!(f, Frame { border: 1, margin: 22 });
        assert_eq!(f.window((200, 100)), (246, 146));
        assert_eq!(f.window_pos((1000, 500)), (977, 477));
        let f2 = Frame::at(2.0);
        assert_eq!((f2.border, f2.margin), (2, 43));
    }

    #[test]
    fn menu_model() {
        let m = menu(View::default(), 1);
        let labels: Vec<String> = m
            .iter()
            .map(|e| match e {
                Entry::Item { label, .. } => label.clone(),
                Entry::Sub { label, .. } => format!("{label} >"),
                Entry::Sep => "-".into(),
            })
            .collect();
        assert_eq!(labels, ["Copy", "Save", "Save As\u{2026}", "-", "Zoom 100 %", "Opacity >", "-", "Close", "Close all pins"]);
        let find = |m: &[Entry], c: Cmd| {
            m.iter().find_map(|e| match e {
                Entry::Item { cmd, checked, enabled, .. } if *cmd == c => Some((*checked, *enabled)),
                _ => None,
            })
        };
        assert_eq!(find(&m, Cmd::Zoom100), Some((false, false)), "already at 100 %");
        assert_eq!(find(&m, Cmd::CloseAll), Some((false, false)), "one pin: Close does it");
        let m = menu(View { zoom: 150, opacity: 60 }, 3);
        assert_eq!(find(&m, Cmd::Zoom100), Some((false, true)));
        assert_eq!(find(&m, Cmd::CloseAll), Some((false, true)));
        let Some(Entry::Sub { items, .. }) = m.iter().find(|e| matches!(e, Entry::Sub { .. })) else { panic!("no Opacity") };
        let checked: Vec<Cmd> = items
            .iter()
            .filter_map(|e| match e {
                Entry::Item { cmd, checked: true, .. } => Some(*cmd),
                _ => None,
            })
            .collect();
        assert_eq!(checked, [Cmd::Opacity(60)]);
        assert_eq!(items.len(), OPACITIES.len());
        // Ids round-trip and never clash.
        let mut ids = Vec::new();
        for c in [Cmd::Copy, Cmd::Save, Cmd::SaveAs, Cmd::Zoom100, Cmd::Close, Cmd::CloseAll]
            .into_iter()
            .chain(OPACITIES.iter().map(|o| Cmd::Opacity(*o)))
        {
            assert_eq!(Cmd::from_id(c.id()), Some(c));
            assert_ne!(c.id(), 0);
            ids.push(c.id());
        }
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 6 + OPACITIES.len());
        assert_eq!(Cmd::from_id(0), None);
        assert_eq!(Cmd::from_id(99), None);
        // What the view commands do.
        let v = View { zoom: 250, opacity: 40 };
        assert_eq!(apply(v, Cmd::Zoom100), Some(View { zoom: 100, opacity: 40 }));
        assert_eq!(apply(v, Cmd::Opacity(80)), Some(View { zoom: 250, opacity: 80 }));
        assert_eq!(apply(v, Cmd::Copy), None);
    }

    #[test]
    fn the_drawn_menu_flattens_opacity_and_hits_rows() {
        let m = menu(View { zoom: 120, opacity: 80 }, 2);
        let r = rows(&m);
        assert_eq!(r.len(), 9 - 1 + 1 + OPACITIES.len(), "the submenu's caption and its choices");
        assert!(r.contains(&Row::Caption("Opacity")));
        for s in [1.0f32, 2.0] {
            let l = menu_layout(&m, s);
            assert!(l.panel.x1() < l.size.0 as f32 && l.panel.y1() < l.size.1 as f32);
            let at = |c: Cmd| {
                let b = l.rows.iter().find(|(r, _)| matches!(r, Row::Item { cmd, .. } if *cmd == c)).unwrap().1;
                crate::objects::Pt::new(b.x + 4.0, b.y + b.h / 2.0)
            };
            assert_eq!(menu_at(&l, at(Cmd::Copy)), Some(Cmd::Copy));
            assert_eq!(menu_at(&l, at(Cmd::Opacity(40))), Some(Cmd::Opacity(40)));
            assert_eq!(menu_at(&l, at(Cmd::CloseAll)), Some(Cmd::CloseAll));
            assert_eq!(menu_at(&l, crate::objects::Pt::new(1.0, 1.0)), None);
            let px = render_menu(&l, &crate::theme::DARK, s, Some(Cmd::Save));
            assert_eq!(px.len(), (l.size.0 * l.size.1 * 4) as usize);
        }
        // A disabled row does not act.
        let l = menu_layout(&menu(View::default(), 1), 1.0);
        let b = l.rows.iter().find(|(r, _)| matches!(r, Row::Item { cmd: Cmd::Zoom100, .. })).unwrap().1;
        assert_eq!(menu_at(&l, crate::objects::Pt::new(b.x + 4.0, b.y + 4.0)), None);
    }

    #[test]
    fn resample_averages_down_and_keeps_pixels_up() {
        // 4×2: left half black, right half white.
        let mut src = PixBuf::new(4, 2);
        for (i, p) in src.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = if i % 4 >= 2 { 255 } else { 0 };
            *p = [v, v, v, 255];
        }
        let d = resample(&src, 2, 1);
        assert_eq!(d.get_pixel(0, 0), [0, 0, 0, 255]);
        assert_eq!(d.get_pixel(1, 0), [255, 255, 255, 255]);
        let d = resample(&src, 1, 1);
        assert_eq!(d.get_pixel(0, 0)[0], 128, "the mean");
        let u = resample(&src, 8, 4);
        assert_eq!(u.get_pixel(3, 3), [0, 0, 0, 255]);
        assert_eq!(u.get_pixel(4, 0), [255, 255, 255, 255], "nearest: crisp edges");
        assert_eq!(resample(&src, 4, 2), src);
        // A fraction: 3 → 2 averages 1.5 pixels each.
        let mut g = PixBuf::new(3, 1);
        for (i, p) in g.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *p = [i as u8 * 100, 0, 0, 255];
        }
        let d = resample(&g, 2, 1);
        assert_eq!(d.get_pixel(0, 0)[0], 33); // (0 + 0.5 × 100) / 1.5
        assert_eq!(d.get_pixel(1, 0)[0], 167);
    }

    #[test]
    fn pixels_border_and_shadow() {
        let img = PixBuf::from_pixel(40, 30, [10, 200, 30, 255]);
        for (th, s) in [(&crate::theme::DARK, 1.0f32), (&crate::theme::LIGHT, 2.0)] {
            let fr = Frame::at(s);
            let (w, h, px) = render(&img, th, s);
            assert_eq!((w, h), fr.window((40, 30)));
            let at = |x: i32, y: i32| &px[((y as u32 * w + x as u32) * 4) as usize..][..4];
            let i = fr.inset();
            assert_eq!(at(i, i), [30, 200, 10, 255], "the image, BGRA");
            assert_eq!(at(i + 39, i + 29), [30, 200, 10, 255]);
            let a = th.accent;
            assert_eq!(at(fr.margin, fr.margin), [a.b, a.g, a.r, 255], "the accent border");
            assert_eq!(at(i - 1, i + 10), [a.b, a.g, a.r, 255]);
            // The shadow: under the pin, none above it, clear corners.
            let below = at(w as i32 / 2, (i + 30 + fr.border) + (4.0 * s) as i32);
            assert!(below[3] > 20 && below[0] == 0, "{below:?}");
            assert_eq!(at(w as i32 / 2, 1)[3], 0, "no shadow above");
            assert_eq!(at(0, 0)[3], 0);
            assert_eq!(at(w as i32 - 1, h as i32 - 1)[3], 0);
        }
    }

    #[test]
    fn test_pins_are_recorded() {
        let _ = take_opened();
        assert!(!open(Arc::new(PixBuf::default()), (0, 0), crate::theme::DARK), "nothing to pin");
        assert!(open(Arc::new(PixBuf::new(5, 4)), (-10, 20), crate::theme::DARK));
        assert_eq!(count(), 1);
        close_all();
        assert_eq!(count(), 0);
        assert!(take_opened().is_empty());
    }

    /// Preview PNGs of a pin (dark and light, 1x and 2x, at 100 % and
    /// zoomed out), and its drawn menu, under `RUSTSHOT_PREVIEW_DIR`.
    #[test]
    #[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
    fn render_pin_previews() {
        let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let img = sample_shot(320, 200);
        for (tn, th, bg) in [("dark", &crate::theme::DARK, [38u8, 44, 58]), ("light", &crate::theme::LIGHT, [214, 222, 232])] {
            for s in [1.0f32, 2.0] {
                for zoom in [100u32, 50] {
                    let (zw, zh) = scaled_size(img.dimensions(), zoom);
                    let (w, h, px) = render(&resample(&img, zw, zh), th, s);
                    let png = PixBuf::from_raw(w, h, crate::rec_ui::layer_over(&px, bg)).unwrap().to_png().unwrap();
                    std::fs::write(dir.join(format!("{tn}-pin-{zoom}@{s}x.png")), png).unwrap();
                }
                let l = menu_layout(&menu(View { zoom: 150, opacity: 80 }, 2), s);
                let px = render_menu(&l, th, s, Some(Cmd::Opacity(60)));
                let png = PixBuf::from_raw(l.size.0, l.size.1, crate::rec_ui::layer_over(&px, bg)).unwrap().to_png().unwrap();
                std::fs::write(dir.join(format!("{tn}-pin-menu@{s}x.png")), png).unwrap();
            }
        }
    }

    /// A stand-in screenshot: a light page with a few blocks.
    pub(crate) fn sample_shot(w: u32, h: u32) -> PixBuf {
        let mut p = PixBuf::from_pixel(w, h, [246, 247, 249, 255]);
        let d = p.as_raw_mut();
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let c = if y < h / 6 {
                    [27, 28, 32, 255]
                } else if x > w / 10 && x < w * 6 / 10 && (y % (h / 8).max(1)) < h / 20 {
                    [218, 222, 230, 255]
                } else if x > w * 2 / 3 && y > h / 3 && y < h * 3 / 4 {
                    [139, 147, 255, 255]
                } else {
                    continue;
                };
                d[i..i + 4].copy_from_slice(&c);
            }
        }
        p
    }
}
