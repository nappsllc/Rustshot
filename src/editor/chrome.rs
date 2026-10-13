//! Overlay chrome drawing: surfaces, toolbar and palette, selection and
//! handles, size label, toast, tooltip and the empty-state hint.

use super::pick::{self, Part, Picker};
use super::Tool;
use super::toolbar::{label_rect, Act, Dev, Item, Kind, Toolbar};
use crate::icon_path::draw_icon;
use crate::keymap::{Chord, Keymap};
use crate::objects::{FRect, Pt};
use crate::raster::Blend;
use crate::theme::{rgba, Theme, SHADOW};
use crate::uifb::{text_height, text_width, Fb, C4};
use crate::fonts::UiFont;

/// Tokens, DPI scale and UI font: what every chrome call needs.
pub struct Ui<'a> {
    pub th: &'a Theme,
    pub s: f32,
    pub font: Option<&'a UiFont>,
}

impl Ui<'_> {
    pub(super) fn px(&self, v: f32) -> f32 {
        v * self.s
    }

    /// A crisp "1 px" line at this scale.
    fn line(&self) -> f32 {
        self.s.round().max(1.0)
    }

    /// A crisp 1 px line (for sibling chrome modules).
    pub(super) fn line_px(&self) -> f32 {
        self.line()
    }
}

pub const ALL_LAYERS: [usize; 4] = [0, 1, 2, 3];
pub const SMALL_LAYERS: [usize; 2] = [0, 2];

pub(crate) fn round(r: FRect) -> FRect {
    FRect { x: r.x.round(), y: r.y.round(), w: r.w.round(), h: r.h.round() }
}

pub(crate) fn fill(f: &mut Fb, r: FRect, radius: f32, c: C4) {
    f.fill_rounded(r.x as i32, r.y as i32, r.w as i32, r.h as i32, radius, c);
}

/// Inner 1 px ring along the edge of `r`.
pub(crate) fn ring(f: &mut Fb, ui: &Ui, r: FRect, radius: f32, c: C4) {
    let lw = ui.line();
    let inset = FRect { x: r.x + lw / 2.0, y: r.y + lw / 2.0, w: r.w - lw, h: r.h - lw };
    f.stroke_rounded(inset, radius - lw / 2.0, lw, c);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn panel(f: &mut Fb, ui: &Ui, r: FRect, radius: f32, layers: &[usize], bg: C4, border: C4, k: f32) {
    let r = round(r);
    let rad = ui.px(radius);
    for &i in layers.iter().rev() {
        let (dy, sp) = (ui.px(SHADOW[i].0), ui.px(SHADOW[i].1));
        let sr = FRect { x: r.x - sp, y: r.y + dy - sp, w: r.w + 2.0 * sp, h: r.h + 2.0 * sp };
        fill(f, round(sr), rad + sp, ui.th.shadow[i].fade(k));
    }
    fill(f, r, rad, bg.fade(k));
    ring(f, ui, r, rad, border.fade(k));
}

/// Shadow layers (SHADOW indices) + surface fill + 1 px inner border.
pub fn surface(f: &mut Fb, ui: &Ui, r: FRect, radius: f32, layers: &[usize], k: f32) {
    panel(f, ui, r, radius, layers, ui.th.surface, ui.th.border, k);
}

/// Draw `s` vertically centred on `cy`; returns the advance width.
pub(super) fn text(f: &mut Fb, ui: &Ui, size: f32, s: &str, x: f32, cy: f32, c: C4) -> f32 {
    let Some(font) = ui.font else { return 0.0 };
    let _p = crate::perf::span(crate::perf::K::Text);
    let px = ui.px(size);
    f.draw_text(font, px, s, x, cy - text_height(font, px) / 2.0, c)
}

pub(super) fn tw(ui: &Ui, size: f32, s: &str) -> f32 {
    ui.font.map(|f| text_width(f, ui.px(size), s)).unwrap_or(0.0)
}

pub fn act_icon(a: Act) -> &'static str {
    match a {
        Act::Tool(t) => match t {
            Tool::Path => "pencil",
            Tool::Line => "line",
            Tool::Arrow => "arrow",
            Tool::Rect => "rect",
            Tool::Ellipse => "circle",
            Tool::Marker => "marker",
            Tool::Text => "text",
            Tool::Counter => "counter",
            Tool::Pixelate => "pixel",
            Tool::Invert => "invert",
            Tool::Select => "select",
            Tool::Callout => "callout",
            Tool::Spotlight => "spotlight",
            Tool::Magnify => "magnify",
            Tool::Blur => "blur",
        },
        Act::SaveMenu => "chevron",
        Act::Crop => "crop",
        Act::Redact => "redact",
        Act::Freeform => "lasso",
        Act::TextSelect => "ocr",
        Act::CopyAll | Act::TextCopy => "copy",
        Act::RedactKind(_) => "",
        // A menu row (the size label's): no icon.
        Act::Trim => "",
        Act::ObjFront => "front",
        Act::ObjBack => "back",
        Act::ObjDup => "dup",
        Act::ObjDelete => "trash",
        Act::ObjSize(d) if d < 0 => "minus",
        Act::ObjSize(_) => "plus",
        Act::ObjStyle(crate::objects::CalloutStyle::Rounded) => "callout",
        Act::ObjStyle(crate::objects::CalloutStyle::Cloud) => "cloudshape",
        Act::ObjStyle(crate::objects::CalloutStyle::Ellipse) => "ellipse",
        Act::ObjFill => "fill",
        Act::ObjColor => "",
        Act::ObjBold => "bold",
        Act::ObjItalic => "italic",
        Act::ObjUnderline => "underline",
        Act::ObjStrike => "strike",
        Act::ObjAlign(crate::text::Align::Left) => "align-left",
        Act::ObjAlign(crate::text::Align::Center) => "align-center",
        Act::ObjAlign(crate::text::Align::Right) => "align-right",
        Act::ObjFontMenu => "chevron",
        Act::ObjFont(_) => "",
        Act::Undo => "undo",
        Act::Redo => "redo",
        Act::Size(d) if d < 0 => "minus",
        Act::Size(_) => "plus",
        Act::Copy => "copy",
        Act::CopyText => "ocr",
        Act::Save => "save",
        Act::SaveAs => "save-as",
        Act::SaveDesktop => "save-desktop",
        Act::OpenWith => "set-external",
        Act::Share => "share",
        Act::Upload => "cloud",
        Act::Pin => "pin",
        Act::Exit => "x",
        Act::Accept => "check",
        Act::Record => "video",
        Act::Eyedrop => "pipette",
        Act::RingSize(d) if d < 0 => "minus",
        Act::RingSize(_) => "plus",
        Act::Palette
        | Act::Fill
        | Act::Color(_)
        | Act::RecFormat(_)
        | Act::RecFps(_)
        | Act::RecSystem
        | Act::RecMic
        | Act::RecCamera
        | Act::RecDevMenu(_)
        | Act::RecDev(..)
        | Act::RecShape(_)
        | Act::RecBg(_)
        | Act::Mode(_)
        | Act::RecSource(_)
        | Act::RecCountdown(_)
        | Act::RecGifMax(_)
        | Act::RecStart
        | Act::Wheel
        | Act::RingPalette
        | Act::RingColor(_)
        | Act::RingWheel
        | Act::Pick(_)
        | Act::PickColor(_) => "",
    }
}

pub struct BarState<'a> {
    pub hover: Option<usize>,
    /// 0..1 fade of the hovered button's background.
    pub hover_k: f32,
    pub pressed: Option<usize>,
    pub tool: Option<Tool>,
    /// The current tool's color (the swatch, the size preview).
    pub color: C4,
    /// The current tool's size: the number, and the tool and size the
    /// preview next to it draws.
    pub value: &'a str,
    pub ptool: Tool,
    pub size: f32,
    /// The shape tool draws filled (the fill toggle is on).
    pub fill: bool,
    /// 0..1 opacity of the palette popover.
    pub pop_k: f32,
    /// The color picker's state while it is open.
    pub pick: Option<&'a Picker>,
}

pub fn toolbar(f: &mut Fb, ui: &Ui, tb: &Toolbar, st: &BarState, k: f32) {
    // Video mode: the bar is the setup panel (14 px corners).
    if !tb.docked {
        surface(f, ui, tb.bar, if tb.video { 14.0 } else { 12.0 }, &ALL_LAYERS, k);
    }
    if let Some(pop) = tb.pop {
        // Opaque: the annotations under a popover never show through it.
        let bg = C4 { a: 255, ..ui.th.surface };
        panel(f, ui, pop, 12.0, &ALL_LAYERS, bg, ui.th.border, k * st.pop_k);
    }
    let kp = k * st.pop_k;
    let focus = st.pick.and_then(|p| p.focus);
    for (i, it) in tb.items.iter().enumerate() {
        let hovered = st.hover == Some(i) && !it.disabled;
        let pressed = st.pressed == Some(i) && !it.disabled;
        // Popover items fade with the popover.
        let in_pop = tb.pop.is_some_and(|p| it.r.x >= p.x && it.r.y >= p.y && it.r.x1() <= p.x1() + 0.5 && it.r.y1() <= p.y1() + 0.5);
        // Video mode's panel is the bar: its items fade with it.
        let kp = if tb.video && !in_pop { k } else { kp };
        match it.kind {
            Kind::Btn(act) => {
                button(f, ui, it, act, (hovered, pressed), st, if in_pop { kp } else { k });
                if act == Act::Eyedrop && focus == Some(Part::Eyedrop) {
                    focus_ring(f, ui, it.r, 8.0, kp);
                }
            }
            Kind::Value => value(f, ui, it, st, k),
            Kind::SepV | Kind::SepH => {
                let r = round(it.r);
                f.fill_rect(r.x as i32, r.y as i32, r.w.max(1.0) as i32, r.h.max(1.0) as i32, ui.th.separator.fade(k));
            }
            Kind::Dot(c) => dot(f, ui, it.r, c, (c == st.color, hovered), kp),
            Kind::MenuItem { label, key, .. } => {
                let r = round(it.r);
                if hovered {
                    fill(f, r, ui.px(6.0), ui.th.accent_bg.fade(kp));
                }
                let cy = r.y + r.h / 2.0;
                let fg = if hovered { ui.th.accent_fg } else { ui.th.text };
                text(f, ui, 12.5, label, r.x + ui.px(10.0), cy, fg.fade(kp));
                let keys = tb.texts.get(key).map_or("", String::as_str);
                let kw = tw(ui, 11.0, keys);
                text(f, ui, 11.0, keys, r.x1() - ui.px(10.0) - kw, cy, if hovered { ui.th.accent_fg } else { ui.th.text_muted }.fade(kp));
            }
            Kind::Check { label, on, .. } => {
                let r = round(it.r);
                if hovered {
                    fill(f, r, ui.px(6.0), ui.th.accent_bg.fade(kp));
                }
                let cy = r.y + r.h / 2.0;
                let bx = round(FRect { x: r.x + ui.px(10.0), y: cy - ui.px(7.0), w: ui.px(14.0), h: ui.px(14.0) });
                if on {
                    fill(f, bx, ui.px(4.0), ui.th.accent.fade(kp));
                    let isz = ui.px(12.0);
                    draw_icon(f, "check", bx.x + (bx.w - isz) / 2.0, bx.y + (bx.h - isz) / 2.0, isz, C4::rgb(255, 255, 255).fade(kp));
                } else {
                    f.stroke_rounded(bx, ui.px(4.0), ui.line(), ui.th.text_muted.fade(kp));
                }
                let fg = if hovered { ui.th.accent_fg } else { ui.th.text };
                text(f, ui, 12.5, label, bx.x1() + ui.px(8.0), cy, fg.fade(kp));
            }
            Kind::Toggle { on, icon, .. } => {
                let r = round(it.r);
                panel(f, ui, r, 8.0, &ALL_LAYERS, ui.th.surface, ui.th.border, k);
                if on {
                    fill(f, r, ui.px(8.0), ui.th.accent_bg.fade(k));
                    ring(f, ui, r, 8.0, ui.th.accent_ring.fade(k));
                } else if hovered {
                    fill(f, r, ui.px(8.0), ui.th.bg_hover.fade(k));
                }
                let isz = ui.px(16.0);
                let fg = if on { ui.th.accent_fg } else if hovered { ui.th.icon_hover } else { ui.th.icon };
                draw_icon(f, icon, r.x + (r.w - isz) / 2.0, r.y + (r.h - isz) / 2.0, isz, fg.fade(k));
            }
            Kind::SegTrack => {
                let r = round(it.r);
                let rad = ui.px(if r.h < ui.px(30.0) { 7.0 } else { 9.0 });
                fill(f, r, rad, ui.th.field_bg.fade(kp));
                ring(f, ui, r, rad, ui.th.border.fade(kp));
            }
            Kind::Seg { label, icon, on, .. } => segment(f, ui, it.r, label, icon, on, hovered, if it.disabled { kp * 0.4 } else { kp }),
            Kind::Label(s) => {
                text(f, ui, 12.0, s, it.r.x, it.r.y + it.r.h / 2.0, ui.th.text_muted.fade(kp));
            }
            Kind::Caption(s) => {
                text(f, ui, 11.0, s, it.r.x, it.r.y + it.r.h / 2.0, ui.th.text_muted.fade(kp));
            }
            Kind::Row { icon, label, value, dim, .. } => {
                // Hovered itself or through one of its parts.
                let hot = st.hover.and_then(|h| tb.items.get(h)).is_some_and(|h| !h.disabled && within(h.r, it.r));
                panel_row(f, ui, it, (icon, label, value), dim, hot && !it.disabled, kp);
            }
            Kind::Switch { on, .. } => toggle(f, ui, it.r, on, hovered, if it.disabled { kp * 0.4 } else { kp }),
            Kind::DevBox { dev, open } => {
                let name = tb.texts.get(dev.text()).map_or("", String::as_str);
                dev_box(f, ui, it.r, name, open || hovered, kp);
            }
            Kind::Meter { lit, clip } => meter(f, ui, it.r, lit, clip, kp),
            Kind::ModeTrack => {
                let r = round(it.r);
                panel(f, ui, r, 9.0, &ALL_LAYERS, ui.th.surface, ui.th.border, k);
            }
            Kind::Mode { video, on } => {
                let (label, icon) = if video { (tr!("tip.video"), "video") } else { (tr!("tip.screenshot"), "camera") };
                mode_btn(f, ui, it.r, label, icon, on, hovered, k);
            }
            Kind::Close => {
                let r = round(it.r);
                panel(f, ui, r, 8.0, &ALL_LAYERS, ui.th.surface, ui.th.border, k);
                if hovered {
                    fill(f, r, ui.px(8.0), ui.th.danger_bg.fade(k));
                }
                let isz = ui.px(16.0);
                let fg = if hovered { ui.th.danger_fg } else { ui.th.icon };
                draw_icon(f, "x", r.x + (r.w - isz) / 2.0, r.y + (r.h - isz) / 2.0, isz, fg.fade(k));
            }
            Kind::Grip => grip(f, ui, it.r, k),
            Kind::DevList => {
                let r = round(it.r);
                panel(f, ui, r, 8.0, &SMALL_LAYERS, ui.th.surface.with_alpha(255), ui.th.border, kp);
            }
            Kind::DevItem { i: n, on, .. } => {
                let name = tb.texts.get(super::toolbar::LIST_TEXT + n).map_or("", String::as_str);
                dev_item(f, ui, it.r, name, on, hovered, kp);
            }
            Kind::Primary { label, .. } => start_button(f, ui, it.r, label, (hovered, pressed), kp),
            Kind::Swatch { c, on, .. } => dot(f, ui, it.r, c.with_alpha(255), (on, hovered), kp),
            Kind::Rainbow(_) => {
                let r = round(it.r);
                if hovered {
                    fill(f, r, ui.px(7.0), ui.th.bg_hover.fade(kp));
                }
                rainbow(f, ui, r.x + r.w / 2.0, r.y + r.h / 2.0, ui.px(9.0), kp);
            }
            Kind::RingValue { w, c } => ring_value(f, ui, it.r, w, c, kp),
            Kind::PickHue => {
                if let Some(p) = st.pick {
                    hue_ring(f, ui, it.r, p, focus == Some(Part::Hue), kp);
                }
            }
            Kind::PickSv => {
                if let Some(p) = st.pick {
                    sv_square(f, ui, it.r, p, focus == Some(Part::Sv), kp);
                }
            }
            Kind::PickSwatch => {
                if let Some(p) = st.pick {
                    let r = round(it.r);
                    let rad = ui.px(8.0);
                    checker(f, ui, r, rad, kp);
                    fill(f, r, rad, p.color().fade(kp));
                    ring(f, ui, r, rad, ui.th.rim.fade(kp));
                }
            }
            Kind::PickHex => {
                if let Some(p) = st.pick {
                    hex_field(f, ui, it.r, p, focus == Some(Part::Hex), hovered, kp);
                }
            }
            Kind::PickAlpha => {
                if let Some(p) = st.pick {
                    alpha_slider(f, ui, it.r, p, focus == Some(Part::Alpha), kp);
                }
            }
            Kind::Recent { i: n, c } => {
                let r = round(it.r);
                if hovered {
                    fill(f, r, ui.px(6.0), ui.th.bg_hover.fade(kp));
                }
                let (cx, cy, rad) = (r.x + r.w / 2.0, r.y + r.h / 2.0, ui.px(8.0));
                swatch_r(f, ui, cx, cy, rad, c, kp);
                if focus == Some(Part::Recent(n)) {
                    focus_ring(f, ui, r, 6.0, kp);
                }
            }
        }
    }
}

/// The keyboard focus: a 2 px accent ring just outside `r`.
fn focus_ring(f: &mut Fb, ui: &Ui, r: FRect, radius: f32, k: f32) {
    let g = ui.px(2.0);
    let r = round(FRect { x: r.x - g, y: r.y - g, w: r.w + 2.0 * g, h: r.h + 2.0 * g });
    f.stroke_rounded(FRect { x: r.x + g / 2.0, y: r.y + g / 2.0, w: r.w - g, h: r.h - g }, ui.px(radius) + g / 2.0, g, ui.th.accent.fade(k));
}

/// A grey checkerboard (what shows through a translucent color) in `r`
/// with corner radius `rad` (anti-aliased edges; a circle when `rad` is
/// half the side).
fn checker(f: &mut Fb, ui: &Ui, r: FRect, rad: f32, k: f32) {
    let cell = ui.px(4.0).round().max(2.0);
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0);
    let (hx, hy) = (r.w / 2.0 - rad, r.h / 2.0 - rad);
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    for y in r.y.floor() as i32..r.y1().ceil() as i32 {
        for x in r.x.floor() as i32..r.x1().ceil() as i32 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (qx, qy) = ((px - cx).abs() - hx, (py - cy).abs() - hy);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad;
            let cov = (0.5 - d).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let odd = (((px - r.x) / cell) as i32 + ((py - r.y) / cell) as i32) % 2 == 1;
            let c = if odd { C4::rgb(204, 204, 210) } else { C4::rgb(255, 255, 255) };
            f.blend_px(x, y, c.fade(cov * k));
        }
    }
}

/// A round swatch of radius `rad`: a checkerboard under a translucent
/// color, then the color and its rim.
pub(super) fn swatch_r(f: &mut Fb, ui: &Ui, cx: f32, cy: f32, rad: f32, c: C4, k: f32) {
    if c.a < 255 {
        checker(f, ui, FRect { x: cx - rad, y: cy - rad, w: 2.0 * rad, h: 2.0 * rad }, rad, k);
    }
    let lw = ui.line();
    f.fill_circle(cx, cy, rad, c.fade(k));
    f.stroke_circle(cx, cy, rad - lw / 2.0, lw, ui.th.rim.fade(k));
}

/// A rainbow swatch (every hue around it): the color picker.
pub(super) fn rainbow(f: &mut Fb, ui: &Ui, cx: f32, cy: f32, rad: f32, k: f32) {
    let (x0, x1) = ((cx - rad - 1.0).floor() as i32, (cx + rad + 1.0).ceil() as i32);
    let (y0, y1) = ((cy - rad - 1.0).floor() as i32, (cy + rad + 1.0).ceil() as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = dx.hypot(dy);
            let cov = (rad + 0.5 - d).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let h = dx.atan2(-dy).to_degrees();
            // Paler towards the centre, like a color wheel.
            let (r, g, b) = pick::hsv_to_rgb(h, (d / rad).clamp(0.0, 1.0) * 0.85 + 0.15, 1.0);
            f.blend_px(x, y, C4::rgb(r, g, b).fade(cov * k));
        }
    }
    let lw = ui.line();
    f.stroke_circle(cx, cy, rad - lw / 2.0, lw, ui.th.rim.fade(k));
}

/// A picker thumb: the color inside a white ring with a dark edge.
fn thumb(f: &mut Fb, ui: &Ui, cx: f32, cy: f32, rad: f32, c: C4, k: f32) {
    f.fill_circle(cx, cy + ui.px(0.5), rad + ui.px(1.5), C4::new(0, 0, 0, 70).fade(k));
    f.fill_circle(cx, cy, rad + ui.px(1.0), C4::rgb(255, 255, 255).fade(k));
    f.fill_circle(cx, cy, rad - ui.px(1.0), c.with_alpha(255).fade(k));
}

/// The hue ring in `r` (its bounding square), the thumb at the hue.
fn hue_ring(f: &mut Fb, ui: &Ui, r: FRect, p: &Picker, focused: bool, k: f32) {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let ro = r.w / 2.0;
    let ri = ro - ui.px(super::toolbar::WHEEL_T);
    for y in r.y.floor() as i32..r.y1().ceil() as i32 {
        let dy = y as f32 + 0.5 - cy;
        for x in r.x.floor() as i32..r.x1().ceil() as i32 {
            let dx = x as f32 + 0.5 - cx;
            let d = dx.hypot(dy);
            let cov = (ro + 0.5 - d).clamp(0.0, 1.0).min((d - ri + 0.5).clamp(0.0, 1.0));
            if cov <= 0.0 {
                continue;
            }
            let (rr, g, b) = pick::hsv_to_rgb(dx.atan2(-dy).to_degrees(), 1.0, 1.0);
            f.blend_px(x, y, C4::rgb(rr, g, b).fade(cov * k));
        }
    }
    if focused {
        let g = ui.px(2.0);
        f.stroke_circle(cx, cy, ro + g * 1.5, g, ui.th.accent.fade(k));
    }
    let a = p.h.to_radians();
    let mid = (ro + ri) / 2.0;
    let (tx, ty) = (cx + mid * a.sin(), cy - mid * a.cos());
    let (hr, hg, hb) = pick::hsv_to_rgb(p.h, 1.0, 1.0);
    thumb(f, ui, tx, ty, ui.px(super::toolbar::WHEEL_T / 2.0 - 2.0), C4::rgb(hr, hg, hb), k);
}

/// The saturation (across) / value (down) square at the picker's hue.
fn sv_square(f: &mut Fb, ui: &Ui, r: FRect, p: &Picker, focused: bool, k: f32) {
    let r = round(r);
    for y in r.y as i32..r.y1() as i32 {
        let v = 1.0 - ((y as f32 + 0.5 - r.y) / r.h).clamp(0.0, 1.0);
        for x in r.x as i32..r.x1() as i32 {
            let sat = ((x as f32 + 0.5 - r.x) / r.w).clamp(0.0, 1.0);
            let (rr, g, b) = pick::hsv_to_rgb(p.h, sat, v);
            f.blend_px(x, y, C4::rgb(rr, g, b).fade(k));
        }
    }
    let lw = ui.line();
    f.stroke_rect(r.x + lw / 2.0, r.y + lw / 2.0, r.w - lw, r.h - lw, lw, ui.th.rim.fade(k));
    if focused {
        focus_ring(f, ui, r, 2.0, k);
    }
    let (tx, ty) = (r.x + p.s * r.w, r.y + (1.0 - p.v) * r.h);
    thumb(f, ui, tx, ty, ui.px(5.0), p.color(), k);
}

/// The hex field: recessed, the text (in the error color while it does
/// not read as a color), a caret while focused.
fn hex_field(f: &mut Fb, ui: &Ui, r: FRect, p: &Picker, focused: bool, hovered: bool, k: f32) {
    let th = ui.th;
    let r = round(r);
    let rad = ui.px(8.0);
    fill(f, r, rad, th.field_bg.fade(k));
    if hovered && !focused {
        fill(f, r, rad, th.bg_hover.fade(k));
    }
    ring(f, ui, r, rad, if focused { th.accent } else { th.border }.fade(k));
    let cy = r.y + r.h / 2.0;
    let x = r.x + ui.px(10.0);
    let fg = if p.hex_ok { th.text } else { th.error };
    let shown = fit_text(ui, 13.0, &p.hex, r.w - ui.px(22.0));
    let w = text(f, ui, 13.0, &shown, x, cy, fg.fade(k));
    if focused {
        let h = ui.px(16.0);
        let cw = ui.line().max(ui.px(1.5)).round();
        f.fill_rect((x + w + ui.px(1.0)).round() as i32, (cy - h / 2.0).round() as i32, cw as i32, h.round() as i32, th.accent.fade(k));
    }
}

/// The opacity slider: a checkered track fading in the color, the thumb,
/// the percentage right of it.
fn alpha_slider(f: &mut Fb, ui: &Ui, r: FRect, p: &Picker, focused: bool, k: f32) {
    let th = ui.th;
    let (tx0, tx1) = (r.x + ui.px(8.0), r.x1() - ui.px(8.0));
    let th_h = ui.px(8.0).round();
    let cy = r.y + r.h / 2.0;
    let track = round(FRect { x: tx0, y: cy - th_h / 2.0, w: tx1 - tx0, h: th_h });
    checker(f, ui, track, th_h / 2.0, k);
    let c = p.color();
    for x in track.x as i32..track.x1() as i32 {
        let t = ((x as f32 + 0.5 - track.x) / track.w).clamp(0.0, 1.0);
        let col = c.with_alpha((t * 255.0).round() as u8).fade(k);
        // Rounded ends: inside the capsule only.
        let ex = (x as f32 + 0.5 - track.x).min(track.x1() - x as f32 - 0.5);
        for y in track.y as i32..track.y1() as i32 {
            let ey = (y as f32 + 0.5 - cy).abs();
            let rr = th_h / 2.0;
            let cov = if ex < rr { (rr + 0.5 - (rr - ex).hypot(ey)).clamp(0.0, 1.0) } else { 1.0 };
            if cov > 0.0 {
                f.blend_px(x, y, col.fade(cov));
            }
        }
    }
    ring(f, ui, track, th_h / 2.0, th.border.fade(k));
    if focused {
        focus_ring(f, ui, track, 4.0, k);
    }
    let t = p.a as f32 / 255.0;
    thumb(f, ui, track.x + t * track.w, cy, ui.px(6.0), c.with_alpha(255), k);
    let pct = format!("{}%", p.alpha_pct());
    let w = tw(ui, 12.0, &pct);
    text(f, ui, 12.0, &pct, r.x1() + ui.px(40.0) - w, cy, th.text.fade(k));
}

/// The camera ring's width and a stroke of it in its color ("Off" at 0).
fn ring_value(f: &mut Fb, ui: &Ui, r: FRect, w: u32, c: C4, k: f32) {
    let cy = r.y + r.h / 2.0;
    let n = w.to_string();
    let nz = ui.px(20.0);
    let nw = tw(ui, 12.0, &n);
    text(f, ui, 12.0, &n, r.x + ui.px(2.0) + (nz - nw) / 2.0, cy, ui.th.text.fade(k));
    let bx = r.x + ui.px(24.0);
    let bw = r.x1() - ui.px(4.0) - bx;
    if w == 0 {
        text(f, ui, 11.0, tr!("editor.ring.off"), bx + (bw - tw(ui, 11.0, tr!("editor.ring.off"))) / 2.0, cy, ui.th.text_muted.fade(k));
        return;
    }
    let lw = ui.px(w as f32).min(ui.px(20.0));
    let a = Pt::new(bx + lw / 2.0, cy);
    let b = Pt::new(bx + bw - lw / 2.0, cy);
    f.surf().stroke_polyline(&[a, b], lw, c.with_alpha(255).fade(k), Blend::Normal);
}

fn mix(a: C4, b: C4, t: f32) -> C4 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    C4::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), a.a)
}

/// `s` cut with "…" to fit `max_w` physical px at `size`.
fn fit_text(ui: &Ui, size: f32, s: &str, max_w: f32) -> String {
    if tw(ui, size, s) <= max_w {
        return s.to_string();
    }
    let mut out: String = s.to_string();
    while !out.is_empty() {
        out.pop();
        let t = format!("{}…", out.trim_end());
        if tw(ui, size, &t) <= max_w {
            return t;
        }
    }
    "…".into()
}

/// `a` lies within `b`.
fn within(a: FRect, b: FRect) -> bool {
    a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x1() <= b.x1() + 0.5 && a.y1() <= b.y1() + 0.5
}

/// One segment of a segmented control: the accent tint when chosen; an
/// icon (16 px) and / or a label (12 px), centred, 6 px apart.
#[allow(clippy::too_many_arguments)]
fn segment(f: &mut Fb, ui: &Ui, r: FRect, label: &str, icon: &str, on: bool, hovered: bool, k: f32) {
    let th = ui.th;
    let r = round(r);
    let rad = ui.px(6.0);
    if on {
        fill(f, r, rad, th.accent_bg.fade(k));
        ring(f, ui, r, rad, th.accent_ring.fade(k));
    } else if hovered {
        fill(f, r, rad, th.bg_hover.fade(k));
    }
    let fg = if on { th.accent_fg } else if hovered { th.icon_hover } else { th.icon };
    let size = if r.h < ui.px(24.0) { 11.0 } else { 12.0 };
    let isz = if icon.is_empty() { 0.0 } else { ui.px(16.0) };
    let gap = if icon.is_empty() || label.is_empty() { 0.0 } else { ui.px(6.0) };
    let w = isz + gap + tw(ui, size, label);
    let (x, cy) = (r.x + (r.w - w) / 2.0, r.y + r.h / 2.0);
    if !icon.is_empty() {
        draw_icon(f, icon, x, cy - isz / 2.0, isz, fg.fade(k));
    }
    text(f, ui, size, label, x + isz + gap, cy, fg.fade(k));
}

/// A panel row: hover tint, the icon (20 px), the name (92 px column),
/// a fixed value (when it has no device box), muted while off.
fn panel_row(f: &mut Fb, ui: &Ui, it: &Item, (icon, label, value): (&str, &str, Option<&str>), dim: bool, hot: bool, k: f32) {
    let th = ui.th;
    let r = round(it.r);
    let k = if it.disabled && !icon.is_empty() { k * 0.4 } else { k };
    if hot {
        fill(f, r, ui.px(9.0), th.bg_hover.fade(k));
    }
    let cy = r.y + r.h / 2.0;
    let mut x = r.x + ui.px(8.0);
    if !icon.is_empty() {
        let isz = ui.px(20.0);
        draw_icon(f, icon, x, cy - isz / 2.0, isz, if dim { th.text_muted } else { th.icon }.fade(k));
        x = r.x + ui.px(38.0);
    }
    let name = if dim { th.text_muted } else { th.text };
    let nw = text(f, ui, 12.0, label, x, cy, name.fade(k));
    if let Some(v) = value {
        let vx = if icon.is_empty() { x + nw + ui.px(10.0) } else { r.x + ui.px(38.0 + 92.0 + 10.0) };
        let room = r.x1() - ui.px(6.0 + 32.0 + 10.0) - vx;
        text(f, ui, 12.0, &fit_text(ui, 12.0, v, room), vx, cy, th.text_muted.fade(k));
    }
}

/// A row's on/off toggle (32 × 18, the 14 px knob), at the right of `r`.
fn toggle(f: &mut Fb, ui: &Ui, r: FRect, on: bool, hovered: bool, k: f32) {
    let th = ui.th;
    let (sw, sh) = (ui.px(32.0).round(), ui.px(18.0).round());
    let t = FRect { x: (r.x1() - ui.px(6.0) - sw).round(), y: (r.y + (r.h - sh) / 2.0).round(), w: sw, h: sh };
    let track = if on {
        if hovered { mix(th.accent, C4::rgb(255, 255, 255), 0.12) } else { th.accent }
    } else if hovered {
        th.text_muted.fade(0.55)
    } else {
        th.text_muted.fade(0.35)
    };
    fill(f, t, sh / 2.0, track.fade(k));
    let kr = ui.px(7.0);
    let cx = if on { t.x1() - ui.px(9.0) } else { t.x + ui.px(9.0) };
    let ky = t.y + t.h / 2.0;
    f.fill_circle(cx, ky + ui.px(0.5), kr + ui.px(0.5), C4::new(0, 0, 0, 70).fade(k));
    f.fill_circle(cx, ky, kr, C4::rgb(255, 255, 255).fade(k));
}

/// The level meter: 7 bars of fixed heights, lit left to right; the top
/// two amber while clipping.
fn meter(f: &mut Fb, ui: &Ui, r: FRect, lit: u8, clip: bool, k: f32) {
    let th = ui.th;
    let (bw, gap) = (ui.px(3.0), ui.px(2.0));
    let cy = r.y + r.h / 2.0;
    for (i, h) in crate::rec_ui::meter::BARS.iter().enumerate() {
        let h = ui.px(*h);
        let b = round(FRect { x: r.x + i as f32 * (bw + gap), y: cy - h / 2.0, w: bw, h });
        let on = (i as u8) < lit;
        let c = if !on {
            th.text_muted.fade(0.35)
        } else if clip && i >= crate::rec_ui::meter::BARS.len() - 2 {
            crate::rec_ui::AMBER
        } else {
            th.accent
        };
        fill(f, b, bw / 2.0, c.fade(k));
    }
}

/// A mode of the mode switch: icon (14 px) and label (11.5 px).
#[allow(clippy::too_many_arguments)]
fn mode_btn(f: &mut Fb, ui: &Ui, r: FRect, label: &str, icon: &str, on: bool, hovered: bool, k: f32) {
    let th = ui.th;
    let r = round(r);
    let rad = ui.px(6.0);
    if on {
        fill(f, r, rad, th.accent_bg.fade(k));
        ring(f, ui, r, rad, th.accent_ring.fade(k));
    } else if hovered {
        fill(f, r, rad, th.bg_hover.fade(k));
    }
    let fg = if on { th.accent_fg } else if hovered { th.icon_hover } else { th.icon };
    let (isz, gap) = (ui.px(14.0), ui.px(6.0));
    let x = r.x + ui.px(10.0);
    let cy = r.y + r.h / 2.0;
    draw_icon(f, icon, x, cy - isz / 2.0, isz, fg.fade(k));
    text(f, ui, 11.5, label, x + isz + gap, cy, fg.fade(k));
}

/// "Drag · scroll to resize" under the camera bubble: a pill with the grip.
fn grip(f: &mut Fb, ui: &Ui, r: FRect, k: f32) {
    let th = ui.th;
    let r = round(r);
    panel(f, ui, r, r.h / 2.0 / ui.s, &SMALL_LAYERS, th.surface, th.border, k);
    let cy = r.y + r.h / 2.0;
    let x = r.x + ui.px(8.0);
    // Six dots (the board's grip glyph, 14 px).
    let u = ui.px(14.0) / 24.0;
    for (dx, dy) in [(9.0, 7.0), (15.0, 7.0), (9.0, 12.0), (15.0, 12.0), (9.0, 17.0), (15.0, 17.0)] {
        f.fill_circle(x + dx * u, cy + (dy - 12.0) * u, (1.4 * u).max(0.8), th.icon.fade(k));
    }
    text(f, ui, 11.0, crate::cam::GRIP_HINT.get(), x + ui.px(14.0 + 6.0), cy, th.text.fade(k));
}

/// A row's device: the chosen name (muted, cut to fit) and a chevron
/// right after it; brighter while hovered or open.
pub(super) fn dev_box(f: &mut Fb, ui: &Ui, r: FRect, name: &str, hot: bool, k: f32) {
    let th = ui.th;
    let r = round(r);
    let isz = ui.px(14.0);
    let cy = r.y + r.h / 2.0;
    let room = r.w - ui.px(4.0) - isz;
    let c = if hot { th.text } else { th.text_muted };
    let w = text(f, ui, 12.0, &fit_text(ui, 12.0, name, room), r.x, cy, c.fade(k));
    draw_icon(f, "chevron", r.x + w + ui.px(4.0), cy - isz / 2.0, isz, c.fade(k));
}

/// A microphone in the open list: tinted when hovered, checked when chosen.
pub(super) fn dev_item(f: &mut Fb, ui: &Ui, r: FRect, name: &str, on: bool, hovered: bool, k: f32) {
    let th = ui.th;
    let r = round(r);
    if hovered {
        fill(f, r, ui.px(6.0), th.bg_hover.fade(k));
    }
    let isz = ui.px(14.0);
    let cy = r.y + r.h / 2.0;
    let room = r.w - ui.px(8.0 + 8.0 + 6.0) - isz;
    let c = if on { th.accent_fg } else { th.text };
    text(f, ui, 12.0, &fit_text(ui, 12.0, name, room), r.x + ui.px(8.0), cy, c.fade(k));
    if on {
        draw_icon(f, "check", r.x1() - ui.px(6.0) - isz, cy - isz / 2.0, isz, th.accent_fg.fade(k));
    }
}

/// "Start recording ⏎": the board's red button (40 px, 10 px corners),
/// a dark dot, the label and the Enter key cap.
fn start_button(f: &mut Fb, ui: &Ui, r: FRect, label: &str, (hovered, pressed): (bool, bool), k: f32) {
    use crate::rec_ui::{ON_REC, REC};
    let r = round(r);
    let bg = if pressed {
        mix(REC, C4::rgb(0, 0, 0), 0.15)
    } else if hovered {
        mix(REC, C4::rgb(255, 255, 255), 0.12)
    } else {
        REC
    };
    let rad = ui.px(10.0);
    fill(f, r, rad, bg.fade(k));
    ring(f, ui, r, rad, C4::new(255, 255, 255, 46).fade(k));
    let (dot_r, gap) = (ui.px(5.0), ui.px(10.0));
    let cap_w = ui.px(22.0);
    let w = 2.0 * dot_r + gap + tw(ui, 13.0, label) + gap + cap_w;
    let (x, cy) = (r.x + (r.w - w) / 2.0, r.y + r.h / 2.0);
    f.fill_circle(x + dot_r, cy, dot_r, ON_REC.fade(k));
    let tx = x + 2.0 * dot_r + gap;
    let lw = text(f, ui, 13.0, label, tx, cy, ON_REC.fade(k));
    let ch = ui.px(18.0);
    let cap = round(FRect { x: tx + lw + gap, y: cy - ch / 2.0, w: cap_w, h: ch });
    fill(f, cap, ui.px(4.0), C4::new(0, 0, 0, 46).fade(k));
    let isz = ui.px(12.0);
    draw_icon(f, "enter", cap.x + (cap.w - isz) / 2.0, cy - isz / 2.0, isz, ON_REC.fade(k));
}

fn button(f: &mut Fb, ui: &Ui, it: &Item, act: Act, (hovered, pressed): (bool, bool), st: &BarState, k: f32) {
    let th = ui.th;
    // The Blur slot shows Invert while it is the tool (key I).
    let act = super::toolbar::slot_act(act, st.tool);
    let r = round(it.r);
    let active = matches!(act, Act::Tool(t) if st.tool == Some(t))
        || (act == Act::Eyedrop && st.pick.is_some_and(|p| p.eyedrop))
        || (act == Act::Fill && st.fill && !it.disabled);
    let danger = act == Act::Exit;
    let k = if it.disabled { k * 0.34 } else { k };
    let rad = ui.px(8.0);
    let bg = if active {
        Some(if hovered { th.accent_bg_hover } else { th.accent_bg })
    } else if pressed {
        Some(th.bg_pressed)
    } else if hovered {
        Some(if danger { th.danger_bg } else { th.bg_hover }.fade(st.hover_k))
    } else {
        None
    };
    if let Some(bg) = bg {
        fill(f, r, rad, bg.fade(k));
    }
    if active {
        ring(f, ui, r, rad, th.accent_ring.fade(k));
    }
    let fg = if active {
        th.accent_fg
    } else if danger && hovered {
        th.danger_fg
    } else if hovered || pressed {
        th.icon_hover
    } else {
        th.icon
    }
    .fade(k);
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    match act {
        Act::Palette => swatch(f, ui, cx, cy, st.color, k),
        Act::Fill => fill_icon(f, ui, cx, cy, st.ptool == Tool::Ellipse, st.fill && !it.disabled, fg),
        // The split button's chevron: a small one.
        Act::SaveMenu => {
            let icon = ui.px(14.0);
            draw_icon(f, "chevron", cx - icon / 2.0, cy - icon / 2.0, icon, fg);
        }
        Act::Accept => {
            let (icon, gap) = (ui.px(20.0), ui.px(6.0));
            let x0 = cx - (icon + gap + tw(ui, 12.0, "OK")) / 2.0;
            draw_icon(f, "check", x0, cy - icon / 2.0, icon, fg);
            text(f, ui, 12.0, "OK", x0 + icon + gap, cy, fg);
        }
        _ => {
            let icon = ui.px(if pressed { 19.2 } else { 20.0 });
            draw_icon(f, act_icon(act), cx - icon / 2.0, cy - icon / 2.0, icon, fg);
        }
    }
}

/// The fill toggle's glyph: the shape (a square, or a circle for the
/// ellipse), solid when filling, outlined otherwise.
pub(super) fn fill_icon(f: &mut Fb, ui: &Ui, cx: f32, cy: f32, round: bool, on: bool, c: C4) {
    let side = ui.px(15.0).round();
    let lw = ui.px(2.0);
    let r = FRect { x: (cx - side / 2.0).round(), y: (cy - side / 2.0).round(), w: side, h: side };
    if round {
        let (ccx, ccy, rad) = (r.x + side / 2.0, r.y + side / 2.0, side / 2.0);
        if on {
            f.fill_circle(ccx, ccy, rad, c);
        } else {
            f.stroke_circle(ccx, ccy, rad - lw / 2.0, lw, c);
        }
    } else if on {
        fill(f, r, ui.px(3.0), c);
    } else {
        f.stroke_rounded(FRect { x: r.x + lw / 2.0, y: r.y + lw / 2.0, w: r.w - lw, h: r.h - lw }, ui.px(3.0) - lw / 2.0, lw, c);
    }
}

/// Where the size preview goes in the readout `r`: right of the number.
pub fn preview_box(ui: &Ui, r: FRect) -> FRect {
    let (w, h) = (ui.px(40.0), ui.px(24.0));
    FRect { x: r.x + ui.px(32.0), y: r.y + (r.h - h) / 2.0, w, h }
}

/// The size readout: the number, then the size drawn as the tool draws
/// it (nothing for Invert, which has no size).
fn value(f: &mut Fb, ui: &Ui, it: &Item, st: &BarState, k: f32) {
    if it.disabled {
        return;
    }
    let r = it.r;
    let cy = r.y + r.h / 2.0;
    let (nx, nz) = (r.x + ui.px(4.0), ui.px(26.0));
    let vw = tw(ui, 12.0, st.value);
    text(f, ui, 12.0, st.value, nx + (nz - vw) / 2.0, cy, ui.th.text.fade(k));
    size_preview(f, ui, preview_box(ui, r), st.ptool, st.size, st.color, k);
}

/// `tool` at `size` (image px, as drawn), capped to `b`'s height:
/// strokes a short line (a translucent one for the marker), pixelate a few
/// blocks, text "Aa".
pub fn size_preview(f: &mut Fb, ui: &Ui, b: FRect, tool: Tool, size: f32, c: C4, k: f32) {
    let cap = b.h;
    let cy = b.y + b.h / 2.0;
    match tool {
        Tool::Path | Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse | Tool::Marker => {
            let w = size.clamp(0.5, cap);
            let c = if tool == Tool::Marker { super::marker_color(c) } else { c };
            // Whole rows: an odd width centred on a pixel, an even one
            // on a pixel edge.
            let odd = (w.round() as i32) % 2 == 1;
            let cy = cy.floor() + if odd { 0.5 } else { 0.0 };
            let a = Pt::new(b.x + w / 2.0, cy);
            let e = Pt::new(b.x1() - w / 2.0, cy);
            f.surf().stroke_polyline(&[a, e], w, c.fade(k), Blend::Normal);
        }
        Tool::Pixelate => {
            // Blocks of the size (at most two rows), in a few greys.
            let cell = size.clamp(2.0, cap / 2.0).round().max(2.0);
            let shades = [0.55f32, 0.25, 0.4, 0.18];
            let (cols, rows) = ((b.w / cell).ceil() as i32, (b.h / cell).ceil() as i32);
            let (x0, y0) = (b.x.round() as i32, b.y.round() as i32);
            let (x1, y1) = ((b.x1()).round() as i32, (b.y1()).round() as i32);
            let ci = cell as i32;
            for j in 0..rows {
                for i in 0..cols {
                    let (x, y) = (x0 + i * ci, y0 + j * ci);
                    let sh = shades[((i * 3 + j * 5) % 4) as usize];
                    f.fill_rect(x, y, ci.min(x1 - x), ci.min(y1 - y), ui.th.text.fade(sh * k));
                }
            }
        }
        Tool::Text | Tool::Callout => {
            let Some(font) = ui.font else { return };
            let mut px = size;
            while px > 4.0 && text_height(font, px) > cap {
                px -= 1.0;
            }
            let w = text_width(font, px, "Aa");
            f.draw_text(font, px, "Aa", b.x + (b.w - w) / 2.0, cy - text_height(font, px) / 2.0, c.fade(k));
        }
        Tool::Counter => {
            // A mini tag: the diameter as drawn (capped to the box), "1".
            let d = size.clamp(4.0, cap).round();
            let c0 = Pt::new((b.x + b.w / 2.0).round(), cy.round());
            let tag = crate::objects::Obj::Counter { c: c0, n: 1, color: c.fade(k), d, tail: None };
            tag.render_into(&mut f.surf(), None);
        }
        Tool::Magnify => {
            // The loupe's ring, its diameter capped to the box.
            let r = (size / 4.0).clamp(3.0, cap / 2.0 - 1.0);
            f.stroke_circle(b.x + b.w / 2.0, cy, r, ui.px(2.0), c.fade(k));
        }
        Tool::Blur => {
            // A soft dot: rings fading out to the blur's reach (capped).
            let r = (size / 2.0 + 2.0).clamp(3.0, cap / 2.0);
            let (x, y) = (b.x + b.w / 2.0, cy);
            for i in 0..4 {
                let t = i as f32 / 4.0;
                f.fill_circle(x, y, r * (1.0 - t * 0.7), ui.th.text.fade((0.12 + 0.12 * i as f32) * k));
            }
        }
        Tool::Invert | Tool::Select | Tool::Spotlight => {}
    }
}

fn swatch(f: &mut Fb, ui: &Ui, cx: f32, cy: f32, c: C4, k: f32) {
    swatch_r(f, ui, cx, cy, ui.px(9.0), c, k);
}

fn dot(f: &mut Fb, ui: &Ui, r: FRect, c: C4, (selected, hovered): (bool, bool), k: f32) {
    let r = round(r);
    if selected || hovered {
        fill(f, r, ui.px(7.0), ui.th.bg_hover.fade(k));
    }
    if selected {
        ring(f, ui, r, ui.px(7.0), ui.th.accent_ring.fade(k));
    }
    swatch(f, ui, r.x + r.w / 2.0, r.y + r.h / 2.0, c, k);
}

/// Accent border with dark (or light) outer/inner lines (the selection's,
/// and the window snapping's outline).
pub fn outline(f: &mut Fb, ui: &Ui, sr: FRect, k: f32) {
    let sr = round(sr);
    let th = ui.th;
    let lw = ui.line();
    let grow = |d: f32| FRect { x: sr.x - d, y: sr.y - d, w: sr.w + 2.0 * d, h: sr.h + 2.0 * d };
    for (d, c) in [(1.5, th.sel_outer), (0.5, th.accent), (-0.5, th.sel_inner)] {
        let r = grow(lw * d);
        f.stroke_rect(r.x, r.y, r.w, r.h, lw, c.fade(k));
    }
}

/// Where the export border's preview goes: a band `b` px wide just
/// outside the selection's outline (the border itself is added on export,
/// right at the image's edge), 1 px clear of the outline.
fn border_band(ui: &Ui, sr: FRect, b: f32) -> FRect {
    let sr = round(sr);
    let g = (2.0 * ui.line()).ceil() + 1.0;
    FRect { x: sr.x - g - b, y: sr.y - g - b, w: sr.w + 2.0 * (g + b), h: sr.h + 2.0 * (g + b) }
}

/// Settings › Capture "Add a border": its preview, a band of the border's
/// color and width around the selection's outline.
pub fn border_preview(f: &mut Fb, ui: &Ui, sr: FRect, b: f32, c: C4) {
    let o = border_band(ui, sr, b);
    let (x, y, w, h, b) = (o.x as i32, o.y as i32, o.w as i32, o.h as i32, b.round().max(1.0) as i32);
    f.fill_rect(x, y, w, b, c);
    f.fill_rect(x, y + h - b, w, b, c);
    f.fill_rect(x, y + b, b, h - 2 * b, c);
    f.fill_rect(x + w - b, y + b, b, h - 2 * b, c);
}

/// What [`border_preview`] covers: its four strips.
pub fn border_preview_bounds(ui: &Ui, sr: FRect, b: f32) -> Vec<FRect> {
    let o = border_band(ui, sr, b);
    let b = b.round().max(1.0) + AA;
    vec![
        FRect { x: o.x - AA, y: o.y - AA, w: o.w + 2.0 * AA, h: b + AA },
        FRect { x: o.x - AA, y: o.y1() - b, w: o.w + 2.0 * AA, h: b + AA },
        FRect { x: o.x - AA, y: o.y - AA, w: b + AA, h: o.h + 2.0 * AA },
        FRect { x: o.x1() - b, y: o.y - AA, w: b + AA, h: o.h + 2.0 * AA },
    ]
}

/// The [`outline`] plus 8 handles: white 6 ⌀ dot, 2 px accent ring (3
/// when hot), 1 px halo.
pub fn selection(f: &mut Fb, ui: &Ui, sr: FRect, hot: Option<usize>, k: f32) {
    outline(f, ui, sr, k);
    let sr = round(sr);
    let th = ui.th;
    for (n, (_, p)) in super::handle_points(sr).iter().enumerate() {
        let ring_w = if hot == Some(n) { 3.0 } else { 2.0 };
        f.fill_circle(p.x, p.y, ui.px(3.0 + ring_w + 1.0), th.handle_halo.fade(k));
        f.fill_circle(p.x, p.y, ui.px(3.0 + ring_w), th.accent.fade(k));
        f.fill_circle(p.x, p.y, ui.px(3.0), C4::rgb(255, 255, 255).fade(k));
    }
}

fn label_texts(sr: FRect) -> (String, String) {
    let size = format!("{} × {}", sr.w.round() as i32, sr.h.round() as i32);
    let pos = format!("{}, {}", sr.x.round() as i32, sr.y.round() as i32);
    (size, pos)
}

/// Where `size_label` puts its panel.
/// `at`: what to keep off (the toolbar above), and the mode switch it
/// follows (10 px right of it, centred on it) when there is one.
pub(super) fn size_label_rect(ui: &Ui, sr: FRect, show_pos: bool, area: FRect, at: (Option<FRect>, Option<FRect>)) -> FRect {
    let (size, pos) = label_texts(sr);
    let pad = ui.px(8.0);
    let mut w = 2.0 * pad + tw(ui, 12.0, &size);
    if show_pos {
        w += pad + tw(ui, 11.0, &pos);
    }
    if let Some(m) = at.1 {
        // Right of the switch; when that runs into what to keep off (the
        // Close button) or off the monitor, under the switch instead.
        let h = ui.px(22.0);
        let x = m.x1() + ui.px(10.0);
        let r = FRect { x, y: m.y + (m.h - h) / 2.0, w, h };
        let clash = at.0.is_some_and(|a| r.x < a.x1() && a.x < r.x1() && r.y < a.y1() && a.y < r.y1()) || r.x1() > area.x1();
        return if clash { FRect { x: m.x, y: m.y1() + ui.px(6.0), w, h } } else { r };
    }
    label_rect(sr, w, ui.s, area, at.0)
}

/// "W × H" (and "x, y" when not dragging) above the selection.
pub fn size_label(f: &mut Fb, ui: &Ui, sr: FRect, show_pos: bool, area: FRect, at: (Option<FRect>, Option<FRect>), k: f32) {
    let (size, pos) = label_texts(sr);
    let pad = ui.px(8.0);
    let r = size_label_rect(ui, sr, show_pos, area, at);
    surface(f, ui, r, 6.0, &SMALL_LAYERS, k);
    let cy = r.y + r.h / 2.0;
    let x = r.x + pad + text(f, ui, 12.0, &size, r.x + pad, cy, ui.th.text.fade(k));
    if show_pos {
        text(f, ui, 11.0, &pos, x + pad, cy, ui.th.text_muted.fade(k));
    }
}

/// What the window snapping badge says after the size, until Tab is used.
const SNAP_TAB: crate::i18n::Key = key!("editor.snap.parts");

fn tab_cap_w(ui: &Ui) -> f32 {
    tw(ui, 10.5, "Tab") + ui.px(10.0)
}

/// Where `snap_label` puts its panel: like the selection's size label.
fn snap_label_rect(ui: &Ui, sr: FRect, hint: bool, area: FRect) -> FRect {
    let (size, _) = label_texts(sr);
    let pad = ui.px(8.0);
    let mut w = 2.0 * pad + tw(ui, 12.0, &size);
    if hint {
        w += pad + tab_cap_w(ui) + ui.px(6.0) + tw(ui, 11.0, &SNAP_TAB);
    }
    label_rect(sr, w, ui.s, area, None)
}

/// Window snapping's badge: "W × H" of the window (or part) outlined and,
/// while it has parts and Tab was never used, "[Tab] parts of the window".
pub fn snap_label(f: &mut Fb, ui: &Ui, sr: FRect, hint: bool, area: FRect, k: f32) {
    let (size, _) = label_texts(sr);
    let pad = ui.px(8.0);
    let r = snap_label_rect(ui, sr, hint, area);
    surface(f, ui, r, 6.0, &SMALL_LAYERS, k);
    let cy = r.y + r.h / 2.0;
    let x = r.x + pad + text(f, ui, 12.0, &size, r.x + pad, cy, ui.th.text.fade(k));
    if hint {
        let (w, h) = (tab_cap_w(ui), ui.px(16.0));
        let cap = round(FRect { x: x + pad, y: cy - h / 2.0, w, h });
        fill(f, cap, ui.px(4.0), ui.th.bg_pressed.fade(k));
        text(f, ui, 10.5, "Tab", cap.x + (w - tw(ui, 10.5, "Tab")) / 2.0, cy, ui.th.text.fade(k));
        text(f, ui, 11.0, &SNAP_TAB, cap.x + w + ui.px(6.0), cy, ui.th.text_muted.fade(k));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

/// The toast panel at opacity `k` (it rises 4 px while fading in).
fn toast_rect(ui: &Ui, msg: &str, area: FRect, k: f32) -> FRect {
    let (h, isz, gap, lpad) = (ui.px(36.0), ui.px(16.0), ui.px(8.0), ui.px(10.0));
    let w = (lpad + isz + gap + tw(ui, 12.0, msg) + ui.px(14.0)).min(area.w - ui.px(24.0));
    let rise = ui.px(4.0) * (1.0 - k);
    FRect { x: area.x + (area.w - w) / 2.0, y: area.y1() - ui.px(28.0) - h + rise, w, h }
}

/// Where the toast's panel is for `msg` in `area` at opacity `k`.
pub fn toast_panel(ui: &Ui, msg: &str, area: FRect, k: f32) -> FRect {
    toast_rect(ui, msg, area, k)
}

/// The toast panel's (w, h) for `msg`, uncapped (the post-capture notice
/// window is sized to it).
pub fn toast_size(ui: &Ui, msg: &str) -> (f32, f32) {
    let r = toast_rect(ui, msg, FRect { x: 0.0, y: 0.0, w: f32::MAX, h: 0.0 }, 1.0);
    (r.w, r.h)
}

/// Bottom-centre notice; `k` drives both fade and the 4 px rise.
pub fn toast(f: &mut Fb, ui: &Ui, msg: &str, kind: ToastKind, area: FRect, k: f32) {
    let th = ui.th;
    let (icon, color) = match kind {
        ToastKind::Info => ("info", th.accent),
        ToastKind::Success => ("okc", th.success),
        ToastKind::Error => ("alert", th.error),
    };
    let (h, isz, gap, lpad) = (ui.px(36.0), ui.px(16.0), ui.px(8.0), ui.px(10.0));
    let r = toast_rect(ui, msg, area, k);
    surface(f, ui, r, 12.0, &ALL_LAYERS, k);
    let cy = r.y + h / 2.0;
    draw_icon(f, icon, r.x + lpad, cy - isz / 2.0, isz, color.fade(k));
    text(f, ui, 12.0, msg, r.x + lpad + isz + gap, cy, th.text.fade(k));
}

fn key_cap(f: &mut Fb, ui: &Ui, x: f32, cy: f32, w: f32, s: &str, k: f32) {
    let h = ui.px(18.0);
    let r = round(FRect { x, y: cy - h / 2.0, w, h });
    fill(f, r, ui.px(4.0), ui.th.key_bg.fade(k));
    text(f, ui, 11.0, s, x + (w - tw(ui, 11.0, s)) / 2.0, cy, ui.th.key_text.fade(k));
}

fn key_w(ui: &Ui, s: &str) -> f32 {
    (tw(ui, 11.0, s) + ui.px(10.0)).max(ui.px(18.0))
}

// Key caps: words everywhere but macOS, where the glyphs are the norm.
#[cfg(target_os = "macos")]
const MOD: &str = "⌘";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl";
#[cfg(target_os = "macos")]
const ALT: &str = "⌥";
#[cfg(not(target_os = "macos"))]
const ALT: &str = "Alt";
#[cfg(target_os = "macos")]
pub(crate) const SHIFT: &str = "⇧";
#[cfg(not(target_os = "macos"))]
pub(crate) const SHIFT: &str = "Shift";
#[cfg(target_os = "macos")]
const META: &str = "⌘";
#[cfg(not(target_os = "macos"))]
const META: &str = crate::keymap::META_NAME;

/// Key caps for a chord: modifiers, then the key.
pub(crate) fn chord_caps(c: &Chord) -> Vec<String> {
    let mut out: Vec<String> = [(c.ctrl, MOD), (c.alt, ALT), (c.shift, SHIFT), (c.meta, META)]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, s)| s.to_string())
        .collect();
    out.push(c.key());
    out
}

/// Tooltip label and key caps (the first chord bound in `km`).
pub fn act_tip(a: Act, km: &Keymap) -> (&'static str, Vec<String>) {
    let label = match a {
        Act::Tool(Tool::Path) => tr!("tip.pencil"),
        Act::Tool(Tool::Line) => tr!("tip.line"),
        Act::Tool(Tool::Arrow) => tr!("tip.arrow"),
        Act::Tool(Tool::Rect) => tr!("tip.rectangle"),
        Act::Tool(Tool::Ellipse) => tr!("tip.ellipse"),
        Act::Tool(Tool::Marker) => tr!("tip.marker"),
        Act::Tool(Tool::Text) => tr!("tip.text"),
        Act::Tool(Tool::Counter) => tr!("tip.number_ctrl_drag_pointer"),
        Act::Tool(Tool::Pixelate) => tr!("tip.pixelate"),
        Act::Tool(Tool::Invert) => tr!("tip.invert"),
        Act::Tool(Tool::Select) => tr!("tip.select_shift_add_drag"),
        Act::Tool(Tool::Callout) => tr!("tip.callout"),
        Act::Tool(Tool::Spotlight) => tr!("tip.spotlight"),
        Act::Tool(Tool::Magnify) => tr!("tip.magnify"),
        Act::Tool(Tool::Blur) => tr!("tip.blur"),
        Act::ObjColor => tr!("tip.color"),
        Act::ObjFill => tr!("tip.fill"),
        Act::ObjSize(d) if d < 0 => tr!("tip.smaller"),
        Act::ObjSize(_) => tr!("tip.larger"),
        Act::ObjStyle(crate::objects::CalloutStyle::Rounded) => tr!("tip.rounded"),
        Act::ObjStyle(crate::objects::CalloutStyle::Cloud) => tr!("tip.cloud"),
        Act::ObjStyle(crate::objects::CalloutStyle::Ellipse) => tr!("tip.ellipse"),
        Act::SaveMenu => tr!("tip.more_ways_to_save"),
        Act::Crop => tr!("tip.crop"),
        Act::Redact => tr!("tip.quick_redact"),
        Act::Freeform => tr!("tip.freeform_selection"),
        Act::TextSelect => tr!("tip.select_text"),
        Act::CopyAll => tr!("tip.copy_all_text"),
        Act::TextCopy => tr!("tip.copy"),
        Act::RedactKind(k) => k.label(),
        Act::ObjFront => tr!("tip.bring_to_front"),
        Act::ObjBack => tr!("tip.send_to_back"),
        Act::ObjDup => tr!("tip.duplicate"),
        Act::ObjDelete => tr!("tip.delete"),
        Act::ObjBold => tr!("tip.bold"),
        Act::ObjItalic => tr!("tip.italic"),
        Act::ObjUnderline => tr!("tip.underline"),
        Act::ObjStrike => tr!("tip.strikethrough"),
        Act::ObjAlign(crate::text::Align::Left) => tr!("tip.align_left"),
        Act::ObjAlign(crate::text::Align::Center) => tr!("tip.align_center"),
        Act::ObjAlign(crate::text::Align::Right) => tr!("tip.align_right"),
        Act::ObjFontMenu | Act::ObjFont(_) => tr!("tip.font"),
        Act::Undo => tr!("tip.undo"),
        Act::Redo => tr!("tip.redo"),
        Act::Size(d) if d < 0 => tr!("tip.smaller"),
        Act::Size(_) => tr!("tip.larger"),
        Act::Palette => tr!("tip.color"),
        Act::Fill => tr!("tip.fill_shape"),
        Act::Color(_) => tr!("tip.use_color"),
        Act::Copy => tr!("tip.copy"),
        Act::CopyText => tr!("tip.copy_text"),
        Act::Save => tr!("tip.save"),
        Act::SaveAs => tr!("tip.save_as"),
        Act::SaveDesktop => tr!("tip.save_to_desktop"),
        Act::OpenWith => tr!("tip.open_with"),
        Act::Share => tr!("tip.share"),
        Act::Upload => tr!("tip.upload"),
        Act::Pin => tr!("tip.pin_to_screen"),
        Act::Trim => tr!("tip.trim_to_content"),
        Act::Exit => tr!("tip.close"),
        Act::Accept => tr!("tip.accept"),
        Act::Record => tr!("tip.record_video"),
        Act::Mode(false) => tr!("tip.screenshot"),
        Act::Mode(true) => tr!("tip.video"),
        Act::RecSource(crate::rec_ui::RecSource::Screen) => tr!("tip.full_screen"),
        Act::RecSource(crate::rec_ui::RecSource::Window) => tr!("tip.window"),
        Act::RecSource(crate::rec_ui::RecSource::Region) => tr!("tip.region"),
        Act::RecCountdown(_) => tr!("tip.countdown"),
        Act::RecGifMax(_) => tr!("tip.max_length"),
        Act::RecFormat(_) => tr!("tip.format"),
        Act::RecFps(_) => tr!("tip.frame_rate"),
        Act::RecSystem => tr!("tip.system_sound"),
        Act::RecMic | Act::RecDevMenu(Dev::Mic) | Act::RecDev(Dev::Mic, _) => tr!("tip.microphone"),
        Act::RecCamera | Act::RecDevMenu(Dev::Cam) | Act::RecDev(Dev::Cam, _) => tr!("tip.camera"),
        Act::RecShape(_) => tr!("tip.camera_shape"),
        Act::RecBg(_) => tr!("tip.camera_background"),
        Act::RecStart => tr!("tip.start_recording"),
        Act::Wheel | Act::RingWheel => tr!("tip.more_colors"),
        Act::RingSize(d) if d < 0 => tr!("tip.thinner_ring"),
        Act::RingSize(_) => tr!("tip.thicker_ring"),
        Act::RingPalette | Act::RingColor(_) => tr!("tip.ring_color"),
        Act::Pick(_) | Act::PickColor(_) => tr!("tip.color"),
        Act::Eyedrop => tr!("tip.pick_from_screen"),
    };
    let keys = match a {
        Act::Size(_) => vec![tr!("tip.key.wheel").to_string()],
        // Built in (not remappable): the mode keys, the source keys.
        Act::Mode(v) => vec![if v { "2" } else { "1" }.to_string()],
        Act::RecSource(src) => vec![crate::rec_ui::source_key(src).to_string()],
        Act::RecStart => vec!["Enter".to_string()],
        _ => super::action_of(a).and_then(|x| km.chords(x).first()).map(chord_caps).unwrap_or_default(),
    };
    (label, keys)
}

/// Where a tooltip goes: by `anchor`, inside `area`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipAt {
    pub anchor: FRect,
    pub area: FRect,
    /// Below the anchor (else above it, below only when there is no room).
    pub below: bool,
}

/// Dark label + key caps, 8 above the anchor (below if no room, or when
/// asked).
pub fn tooltip(f: &mut Fb, ui: &Ui, at: TipAt, label: &str, keys: &[String], k: f32) {
    let th = ui.th;
    let (h, gap, kgap) = (ui.px(26.0), ui.px(8.0), ui.px(4.0));
    let r = tooltip_rect(ui, at, label, keys);
    panel(f, ui, r, 7.0, &SMALL_LAYERS, th.tooltip_bg, rgba(0xFFFFFF, 80), k);
    let cy = r.y + h / 2.0;
    let mut cx = r.x + ui.px(9.0);
    cx += text(f, ui, 12.0, label, cx, cy, th.tooltip_text.fade(k));
    if !keys.is_empty() {
        cx += gap;
    }
    for s in keys {
        let kw = key_w(ui, s);
        key_cap(f, ui, cx, cy, kw, s, k);
        cx += kw + kgap;
    }
}

/// Where `tooltip` puts its panel.
fn tooltip_rect(ui: &Ui, at: TipAt, label: &str, keys: &[String]) -> FRect {
    let TipAt { anchor, area, below } = at;
    let (h, gap, kgap) = (ui.px(26.0), ui.px(8.0), ui.px(4.0));
    let keys_w: f32 = keys.iter().map(|s| key_w(ui, s)).sum::<f32>() + kgap * keys.len().saturating_sub(1) as f32;
    let mut w = ui.px(9.0) + tw(ui, 12.0, label) + ui.px(if keys.is_empty() { 9.0 } else { 6.0 });
    if !keys.is_empty() {
        w += gap + keys_w;
    }
    let x = (anchor.x + anchor.w / 2.0 - w / 2.0).clamp(area.x, (area.x1() - w).max(area.x));
    let mut y = anchor.y - ui.px(8.0) - h;
    if below || y < area.y {
        y = anchor.y1() + ui.px(8.0);
    }
    FRect { x, y, w, h }
}

/// One piece of the overlay's hint line: words, or a key cap.
#[derive(Clone, PartialEq, Debug)]
pub enum HintPart {
    T(String),
    K(String),
}

fn hint_part_w(ui: &Ui, p: &HintPart) -> f32 {
    match p {
        HintPart::T(s) => tw(ui, 13.0, s),
        HintPart::K(s) => key_w(ui, s),
    }
}

/// Total width of the hint line (texts, key caps and 6 px gaps).
fn hint_width(ui: &Ui, parts: &[HintPart]) -> f32 {
    parts.iter().map(|p| hint_part_w(ui, p)).sum::<f32>() + ui.px(6.0) * parts.len().saturating_sub(1) as f32
}

/// The first-open hint card ("Drag to select · Ctrl+C copy · …", the
/// keys as the keymap has them), centred in the active monitor on a
/// faint panel: over a window outlined while snapping (shown plain, not
/// dimmed) the words still read.
pub fn hint(f: &mut Fb, ui: &Ui, area: FRect, k: f32, parts: &[HintPart]) {
    let sp = ui.px(6.0);
    let total = hint_width(ui, parts);
    let (mut x, cy) = (area.x + (area.w - total) / 2.0, area.y + area.h / 2.0);
    let k = k * HINT_ALPHA;
    surface(f, ui, hint_panel(ui, x, cy, total), 16.0, &SMALL_LAYERS, k);
    for p in parts {
        let w = hint_part_w(ui, p);
        match p {
            HintPart::T(s) => {
                let c = if s == HINT_DOT { ui.th.text_muted } else { ui.th.text };
                text(f, ui, 13.0, s, x, cy, c.fade(k));
            }
            HintPart::K(s) => {
                let h = ui.px(18.0);
                let r = round(FRect { x, y: cy - h / 2.0, w, h });
                fill(f, r, ui.px(4.0), ui.th.bg_pressed.fade(k));
                text(f, ui, 11.0, s, x + (w - tw(ui, 11.0, s)) / 2.0, cy, ui.th.text.fade(k));
            }
        }
        x += w + sp;
    }
}

/// The separator between the hint's phrases.
pub const HINT_DOT: &str = "\u{b7}";

/// The hint card's opacity at its fullest ("faint").
const HINT_ALPHA: f32 = 0.92;

/// The hint's panel around a line `total` wide at `x`, `cy`.
fn hint_panel(ui: &Ui, x: f32, cy: f32, total: f32) -> FRect {
    let (m, h) = (ui.px(12.0), ui.px(32.0));
    FRect { x: x - m, y: cy - h / 2.0, w: total + 2.0 * m, h }
}

/// The eyedropper's loupe: [`LOUPE_N`]² pixels around the pointer, each
/// [`LOUPE_CELL`] logical px, and the color under it.
pub const LOUPE_N: usize = 9;
const LOUPE_CELL: f32 = 8.0;
const LOUPE_PAD: f32 = 6.0;
const LOUPE_LABEL: f32 = 24.0;

/// Where the loupe goes for pointer `p` on monitor `area` (the one under
/// the pointer): below right of it, flipped to stay on the monitor, and
/// flipped to the other side when it would cover one of `avoid` (the
/// toolbar and its popover) while that side has room.
pub fn loupe_rect(ui: &Ui, p: Pt, area: FRect, avoid: &[FRect]) -> FRect {
    let g = LOUPE_N as f32 * LOUPE_CELL;
    let (w, h) = (ui.px(g + 2.0 * LOUPE_PAD), ui.px(g + 2.0 * LOUPE_PAD + LOUPE_LABEL));
    let off = ui.px(20.0);
    let (right, left) = (p.x + off, p.x - off - w);
    let (below, above) = (p.y + off, p.y - off - h);
    let fits_x = |x: f32| x >= area.x && x + w <= area.x1();
    let fits_y = |y: f32| y >= area.y && y + h <= area.y1();
    let clear = |r: FRect| {
        let gap = ui.px(6.0);
        avoid.iter().all(|a| {
            r.x1() + gap <= a.x || a.x1() + gap <= r.x || r.y1() + gap <= a.y || a.y1() + gap <= r.y
        })
    };
    let at = |x: f32, y: f32| FRect { x, y, w, h };
    let cands = [(right, below), (left, below), (right, above), (left, above)];
    // On the monitor and clear of the toolbar; else on the monitor (the
    // old rule: each axis flips only when it does not fit).
    let (x, y) = cands
        .into_iter()
        .find(|&(x, y)| fits_x(x) && fits_y(y) && clear(at(x, y)))
        .unwrap_or((if right + w > area.x1() { left } else { right }, if below + h > area.y1() { above } else { below }));
    round(FRect { x: x.max(area.x), y: y.max(area.y), w, h })
}

/// The loupe at `r`: the magnified pixels `px` (row by row), the centre
/// one outlined, its color as hex below.
pub fn loupe(f: &mut Fb, ui: &Ui, r: FRect, px: &[C4]) {
    surface(f, ui, r, 10.0, &SMALL_LAYERS, 1.0);
    let cell = ui.px(LOUPE_CELL);
    let (gx, gy) = (r.x + ui.px(LOUPE_PAD), r.y + ui.px(LOUPE_PAD));
    for (i, c) in px.iter().enumerate().take(LOUPE_N * LOUPE_N) {
        let (cx, cy) = ((i % LOUPE_N) as f32, (i / LOUPE_N) as f32);
        let x0 = (gx + cx * cell).round() as i32;
        let y0 = (gy + cy * cell).round() as i32;
        let x1 = (gx + (cx + 1.0) * cell).round() as i32;
        let y1 = (gy + (cy + 1.0) * cell).round() as i32;
        f.fill_rect(x0, y0, x1 - x0, y1 - y0, c.with_alpha(255));
    }
    let m = (LOUPE_N / 2) as f32;
    let lw = ui.line();
    let (x, y) = ((gx + m * cell).round(), (gy + m * cell).round());
    f.stroke_rect(x - lw / 2.0, y - lw / 2.0, cell.round() + lw, cell.round() + lw, lw, C4::rgb(0, 0, 0));
    f.stroke_rect(x - lw * 1.5, y - lw * 1.5, cell.round() + 3.0 * lw, cell.round() + 3.0 * lw, lw, C4::rgb(255, 255, 255));
    let c = px.get(LOUPE_N * LOUPE_N / 2).copied().unwrap_or(C4::rgb(0, 0, 0));
    let cy = gy + LOUPE_N as f32 * cell + ui.px(LOUPE_LABEL) / 2.0;
    let hex = pick::hex_text(c);
    let (dr, gap) = (ui.px(6.0), ui.px(6.0));
    let w = 2.0 * dr + gap + tw(ui, 12.0, &hex);
    let x0 = r.x + (r.w - w) / 2.0;
    swatch_r(f, ui, x0 + dr, cy, dr, c.with_alpha(255), 1.0);
    text(f, ui, 12.0, &hex, x0 + 2.0 * dr + gap, cy, ui.th.text);
}

pub fn loupe_bounds(ui: &Ui, r: FRect) -> FRect {
    surface_bounds(ui, r)
}

// ---- conservative bounds (shadows, AA, glyph overhang) for rect composition ----

/// Margin around anything anti-aliased.
const AA: f32 = 2.0;

fn grow4(r: FRect, l: f32, t: f32, rt: f32, b: f32) -> FRect {
    FRect { x: r.x - l, y: r.y - t, w: r.w + l + rt, h: r.h + t + b }
}

fn union(a: FRect, b: FRect) -> FRect {
    super::toolbar::union(a, b)
}

/// A panel at `r` with up to all four shadow layers: the widest spreads
/// 9 px sideways and reaches 12 + 9 px below.
pub(super) fn surface_bounds(ui: &Ui, r: FRect) -> FRect {
    let (sp, dy) = (ui.px(SHADOW[3].1), ui.px(SHADOW[3].0));
    grow4(r, sp + 1.0 + AA, 1.0 + AA, sp + 1.0 + AA, dy + sp + 1.0 + AA)
}

/// The [`outline`]'s border strips (three 1 px lines): top, bottom,
/// left, right.
pub fn outline_bounds(ui: &Ui, sr: FRect) -> Vec<FRect> {
    let sr = round(sr);
    let b = 2.0 * ui.line() + AA;
    vec![
        FRect { x: sr.x - b, y: sr.y - b, w: sr.w + 2.0 * b, h: 2.0 * b },
        FRect { x: sr.x - b, y: sr.y1() - b, w: sr.w + 2.0 * b, h: 2.0 * b },
        FRect { x: sr.x - b, y: sr.y - b, w: 2.0 * b, h: sr.h + 2.0 * b },
        FRect { x: sr.x1() - b, y: sr.y - b, w: 2.0 * b, h: sr.h + 2.0 * b },
    ]
}

/// Border strips and the eight handles.
pub fn selection_bounds(ui: &Ui, sr: FRect) -> Vec<FRect> {
    let mut v = outline_bounds(ui, sr);
    let sr = round(sr);
    let hr = ui.px(3.0 + 3.0 + 1.0) + 1.0 + AA;
    for (_, p) in super::handle_points(sr) {
        v.push(FRect { x: p.x - hr, y: p.y - hr, w: 2.0 * hr, h: 2.0 * hr });
    }
    v
}

pub fn size_label_bounds(ui: &Ui, sr: FRect, show_pos: bool, area: FRect, at: (Option<FRect>, Option<FRect>)) -> FRect {
    surface_bounds(ui, size_label_rect(ui, sr, show_pos, area, at))
}

/// The panel plus its texts, which may run past a panel clamped to the area.
pub fn snap_label_bounds(ui: &Ui, sr: FRect, hint: bool, area: FRect) -> FRect {
    let r = snap_label_rect(ui, sr, hint, area);
    let (size, _) = label_texts(sr);
    let mut w = ui.px(8.0) * 2.0 + tw(ui, 12.0, &size);
    if hint {
        w += ui.px(8.0) + tab_cap_w(ui) + ui.px(6.0) + tw(ui, 11.0, &SNAP_TAB);
    }
    union(surface_bounds(ui, r), grow4(FRect { w, ..r }, AA, AA, AA, AA))
}

/// The panel plus the message, which may run past a panel clamped to the area.
pub fn toast_bounds(ui: &Ui, msg: &str, area: FRect, k: f32) -> FRect {
    let r = toast_rect(ui, msg, area, k);
    let x = r.x + ui.px(10.0 + 16.0 + 8.0);
    let t = FRect { x, y: r.y, w: tw(ui, 12.0, msg) + ui.px(12.0), h: r.h };
    union(surface_bounds(ui, r), grow4(t, AA, AA, AA, AA))
}

/// The bar and its popover; then each surface outside them (the mode
/// switch, Close, the grip hint), apart: they are far from the bar.
pub fn toolbar_bounds(ui: &Ui, tb: &Toolbar) -> Vec<FRect> {
    let b = surface_bounds(ui, tb.bar);
    let mut v = vec![tb.pop.map_or(b, |p| union(b, surface_bounds(ui, p)))];
    v.extend(tb.extra.iter().map(|r| surface_bounds(ui, *r)));
    v
}

pub fn tooltip_bounds(ui: &Ui, at: TipAt, label: &str, keys: &[String]) -> FRect {
    surface_bounds(ui, tooltip_rect(ui, at, label, keys))
}

pub fn hint_bounds(ui: &Ui, area: FRect, parts: &[HintPart]) -> FRect {
    let total = hint_width(ui, parts);
    let (x, cy) = (area.x + (area.w - total) / 2.0, area.y + area.h / 2.0);
    let m = ui.px(13.0);
    let line = FRect { x: x - m - AA, y: cy - ui.px(14.0) - AA, w: total + 2.0 * (m + AA), h: ui.px(28.0) + 2.0 * AA };
    union(line, surface_bounds(ui, hint_panel(ui, x, cy, total)))
}

#[cfg(test)]
mod tests {
    use super::super::toolbar::{layout, Input};
    use super::*;
    use crate::theme::DARK;

    fn buf(w: usize, h: usize, v: u8) -> Vec<u8> {
        let mut d = vec![v; w * h * 4];
        for p in d.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        d
    }

    fn px(d: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [d[i], d[i + 1], d[i + 2], d[i + 3]]
    }

    fn ui() -> Ui<'static> {
        Ui { th: &DARK, s: 1.0, font: None }
    }

    fn max_r(d: &[u8], w: usize, r: FRect) -> u8 {
        let mut m = 0;
        for y in r.y as usize..r.y1() as usize {
            for x in r.x as usize..r.x1() as usize {
                m = m.max(px(d, w, x, y)[0]);
            }
        }
        m
    }

    fn bar(tool: Option<Tool>, can_undo: bool) -> (Vec<u8>, Toolbar) {
        let tb = layout(&Input {
            sel: FRect { x: 20.0, y: 20.0, w: 760.0, h: 100.0 },
            area: FRect { x: 0.0, y: 0.0, w: 800.0, h: 200.0 },
            s: 1.0,
            busy: false,
            can_undo,
            can_redo: true,
            palette: None,
            record: true,
            rec: None,
            grip: None,
            top: true,
            sizes: true,
            colors: true,
            fills: true,
            pick: None,
            save_menu: None,
            slot_menu: None,
            copy_menu: None,
            freeform: None,
        });
        let mut d = buf(800, 200, 0);
        let st = BarState {
            hover: None,
            hover_k: 1.0,
            pressed: None,
            tool,
            color: C4::rgb(240, 68, 56),
            value: "3",
            ptool: Tool::Line,
            size: 3.0,
            fill: false,
            pop_k: 1.0,
            pick: None,
        };
        toolbar(&mut Fb::new(&mut d, 800), &ui(), &tb, &st, 1.0);
        (d, tb)
    }

    fn btn_rect(tb: &Toolbar, a: Act) -> FRect {
        tb.items.iter().find(|i| i.kind == Kind::Btn(a)).unwrap().r
    }

    #[test]
    fn surface_fill_border_and_shadow() {
        let mut d = buf(60, 50, 0);
        let r = FRect { x: 10.0, y: 10.0, w: 40.0, h: 20.0 };
        surface(&mut Fb::new(&mut d, 60), &ui(), r, 6.0, &ALL_LAYERS, 1.0);
        let c = px(&d, 60, 30, 20);
        let exp = |v: u32| (v * 245 / 255) as i32;
        assert!((c[0] as i32 - exp(0x1B)).abs() <= 1 && (c[2] as i32 - exp(0x20)).abs() <= 1, "{c:?}");
        assert!(px(&d, 60, 10, 20)[0] > c[0] + 10, "1px inner border is lighter");

        let mut w = buf(60, 50, 255);
        surface(&mut Fb::new(&mut w, 60), &ui(), r, 6.0, &ALL_LAYERS, 1.0);
        let sh = px(&w, 60, 30, 35)[0];
        assert!(sh < 250 && sh > 180, "soft shadow below: {sh}");
    }

    #[test]
    fn active_tool_gets_accent_tint() {
        let (d, tb) = bar(Some(Tool::Arrow), true);
        let a = btn_rect(&tb, Act::Tool(Tool::Arrow));
        let p = btn_rect(&tb, Act::Tool(Tool::Path));
        let active = px(&d, 800, a.x as usize + 3, a.y as usize + 16);
        let idle = px(&d, 800, p.x as usize + 3, p.y as usize + 16);
        assert!(active[2] > idle[2] + 20 && active[2] > active[0], "{active:?} vs {idle:?}");
    }

    #[test]
    fn disabled_icon_is_dimmer() {
        let (d, tb) = bar(None, false);
        let undo = max_r(&d, 800, btn_rect(&tb, Act::Undo));
        let redo = max_r(&d, 800, btn_rect(&tb, Act::Redo));
        assert!(undo + 40 < redo, "undo {undo} vs redo {redo}");
    }

    #[test]
    fn every_button_icon_exists() {
        let (_, tb) = bar(None, true);
        for it in &tb.items {
            if let Kind::Btn(a) = it.kind {
                let n = act_icon(a);
                if !n.is_empty() {
                    assert!(crate::icon_path::polylines(n).is_some(), "{n}");
                }
            }
        }
        assert!(crate::icon_path::polylines(act_icon(Act::Accept)).is_some());
    }

    #[test]
    fn selection_draws_accent_line_and_white_handles() {
        let mut d = buf(100, 80, 0);
        let sr = FRect { x: 20.0, y: 20.0, w: 60.0, h: 40.0 };
        selection(&mut Fb::new(&mut d, 100), &ui(), sr, None, 1.0);
        let line = px(&d, 100, 19, 30);
        assert!((line[0] as i32 - 0x8B).abs() <= 2 && line[2] > 240, "accent line: {line:?}");
        assert_eq!(px(&d, 100, 20, 20)[0], 255, "handle centre is white");
    }

    #[test]
    fn tool_tooltips_match_real_shortcuts() {
        let km = Keymap::defaults();
        for t in super::super::toolbar::TOOL_ORDER {
            let (_, keys) = act_tip(Act::Tool(t), &km);
            let vk = keys[0].as_bytes()[0] as u32;
            let a = km.resolve(vk, Default::default()).and_then(super::super::act_of);
            assert_eq!(a, Some(Act::Tool(t)), "{t:?}");
        }
        assert_eq!(act_tip(Act::Redo, &km).1, [MOD, SHIFT, "Z"]);
        assert_eq!(act_tip(Act::Save, &km).1, [MOD, "S"]);
        assert_eq!(act_tip(Act::Exit, &km).1, ["Esc"]);
        let (km, _) = Keymap::from_config(&[("save".to_string(), "Alt+F2".to_string())].into_iter().collect());
        assert_eq!(act_tip(Act::Save, &km).1, [ALT, "F2"]);
        let (km, _) = Keymap::from_config(&[("upload".to_string(), String::new())].into_iter().collect());
        assert!(act_tip(Act::Upload, &km).1.is_empty());
    }
}
