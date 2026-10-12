//! The colour picker popover's model: hue ring + saturation/value square,
//! a hex field (`#rgb`, `#rrggbb`, `rgb(r, g, b)`), opacity, the
//! eyedropper and the recent colours. The picker edits a colour live (the
//! tool or the camera ring shows it as it changes); Enter, or a click
//! outside, keeps it, Esc puts the old one back.
//!
//! Also the editor's side of it (`impl Edit` below): opening and closing
//! the picker, its pointer and keyboard input, the recent colours, and the
//! eyedropper's loupe and sampling.

use super::{chrome, compose, config, toolbar, tools, wind, Edit, Interact};
use crate::objects::{FRect, Pt};
use crate::raster::Order;
use crate::uifb::C4;
use crate::wind::Mods;
use compose::PixBufBackdrop;
use std::time::Instant;

/// What the picker colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// The current drawing tool.
    Tool,
    /// The camera bubble's ring.
    Ring,
}

/// A part of the picker that takes the keyboard focus (Tab order).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Hue,
    Sv,
    Hex,
    Alpha,
    Eyedrop,
    Recent(usize),
}

/// HSV (h in degrees 0..360, s and v 0..1) to RGB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 60.0;
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (q(r), q(g), q(b))
}

/// RGB to HSV (h in degrees 0..360; 0 for greys).
pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// What the hex field accepts: `#rgb`, `#rrggbb` (the `#` optional) or
/// `rgb(r, g, b)` with 0-255 components; any case, spaces around.
pub fn parse_input(s: &str) -> Option<(u8, u8, u8)> {
    let t = s.trim().to_ascii_lowercase();
    if let Some(inner) = t.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        let [r, g, b] = parts[..] else { return None };
        let c = |p: &str| p.parse::<u8>().ok();
        return Some((c(r)?, c(g)?, c(b)?));
    }
    let hex = t.strip_prefix('#').unwrap_or(&t);
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let d = |i: usize| byte(&hex[i..i + 1].repeat(2));
            Some((d(0)?, d(1)?, d(2)?))
        }
        6 => Some((byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?)),
        _ => None,
    }
}

/// The field's text for a colour: `#RRGGBB`.
pub fn hex_text(c: C4) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// The longest text the hex field holds.
pub const HEX_MAX: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub target: Target,
    pub h: f32,
    pub s: f32,
    pub v: f32,
    /// Opacity, 0..=255 (the tool target only; the ring is opaque).
    pub a: u8,
    /// The hex field's text, and whether it reads as a colour.
    pub hex: String,
    pub hex_ok: bool,
    /// The field was just focused: the first key typed replaces it.
    pub hex_fresh: bool,
    pub focus: Option<Part>,
    /// The eyedropper is on: the next click on the screenshot picks.
    pub eyedrop: bool,
    /// The colour when the picker opened (Esc puts it back); `None` for a
    /// ring that was the theme's accent.
    pub orig: Option<C4>,
    /// The colour shown when it opened (the accent for such a ring):
    /// closing on it changes nothing.
    pub start: C4,
}

impl Picker {
    /// A picker for `target`, starting at `c` (`orig`: what Esc restores).
    pub fn new(target: Target, c: C4, orig: Option<C4>) -> Picker {
        let mut p = Picker {
            target,
            h: 0.0,
            s: 0.0,
            v: 0.0,
            a: 255,
            hex: String::new(),
            hex_ok: true,
            hex_fresh: false,
            focus: None,
            eyedrop: false,
            orig,
            start: c,
        };
        p.set_color(c);
        p.start = p.color();
        p
    }

    /// The colour picked (the ring's is opaque).
    pub fn color(&self) -> C4 {
        let (r, g, b) = hsv_to_rgb(self.h, self.s, self.v);
        C4::new(r, g, b, if self.target == Target::Ring { 255 } else { self.a })
    }

    /// Opacity applies (the ring has none).
    pub fn has_alpha(&self) -> bool {
        self.target == Target::Tool
    }

    /// Take `c` (alpha too, for the tool); the hue stays where it was for
    /// a grey, which has none.
    pub fn set_color(&mut self, c: C4) {
        let (h, s, v) = rgb_to_hsv(c.r, c.g, c.b);
        if s > 0.0 && v > 0.0 {
            self.h = h;
        }
        self.s = s;
        self.v = v;
        self.a = c.a;
        self.sync_hex();
    }

    /// The field shows the colour again.
    pub fn sync_hex(&mut self) {
        self.hex = hex_text(self.color());
        self.hex_ok = true;
    }

    pub fn set_hue(&mut self, h: f32) {
        self.h = h.rem_euclid(360.0);
        self.sync_hex();
    }

    pub fn set_sv(&mut self, s: f32, v: f32) {
        self.s = s.clamp(0.0, 1.0);
        self.v = v.clamp(0.0, 1.0);
        self.sync_hex();
    }

    pub fn set_alpha(&mut self, a: f32) {
        self.a = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
    }

    /// Opacity in percent.
    pub fn alpha_pct(&self) -> u32 {
        (self.a as u32 * 100 + 127) / 255
    }

    /// Opacity from a percentage (0..=100).
    pub fn set_alpha_pct(&mut self, pct: i32) {
        self.a = ((pct.clamp(0, 100) as u32 * 255 + 50) / 100) as u8;
    }

    /// Typed into the hex field; the colour follows while the text reads
    /// as one.
    pub fn type_char(&mut self, ch: char) {
        if self.hex_fresh {
            self.hex.clear();
            self.hex_fresh = false;
        }
        if self.hex.chars().count() < HEX_MAX && !ch.is_control() {
            self.hex.push(ch);
        }
        self.hex_changed();
    }

    pub fn backspace(&mut self) {
        if self.hex_fresh {
            self.hex.clear();
            self.hex_fresh = false;
        } else {
            self.hex.pop();
        }
        self.hex_changed();
    }

    fn hex_changed(&mut self) {
        match parse_input(&self.hex) {
            Some((r, g, b)) => {
                let (h, s, v) = rgb_to_hsv(r, g, b);
                if s > 0.0 && v > 0.0 {
                    self.h = h;
                }
                (self.s, self.v) = (s, v);
                self.hex_ok = true;
            }
            None => self.hex_ok = false,
        }
    }

    /// The parts Tab visits, in order (`recents` colours listed).
    pub fn parts(&self, recents: usize) -> Vec<Part> {
        let mut v = vec![Part::Hue, Part::Sv, Part::Hex];
        if self.has_alpha() {
            v.push(Part::Alpha);
        }
        v.push(Part::Eyedrop);
        v.extend((0..recents).map(Part::Recent));
        v
    }

    /// Tab (`back`: Shift+Tab) to the next part.
    pub fn tab(&mut self, back: bool, recents: usize) {
        let parts = self.parts(recents);
        let n = parts.len();
        let i = self.focus.and_then(|f| parts.iter().position(|p| *p == f));
        let next = match (i, back) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
        };
        self.focus = Some(parts[next]);
        self.hex_fresh = self.focus == Some(Part::Hex);
    }

    /// An arrow key on the focused part: `dx`/`dy` -1, 0 or 1 (screen
    /// directions), `big` (Shift) for ten steps. True when the colour
    /// changed.
    pub fn nudge(&mut self, dx: i32, dy: i32, big: bool, recents: usize) -> bool {
        let k = if big { 10.0 } else { 1.0 };
        match self.focus {
            Some(Part::Hue) => {
                // Right / up turn the hue forward.
                self.set_hue(self.h + k * (dx - dy) as f32);
                true
            }
            Some(Part::Sv) => {
                self.set_sv(self.s + 0.01 * k * dx as f32, self.v - 0.01 * k * dy as f32);
                true
            }
            Some(Part::Alpha) if self.has_alpha() => {
                self.set_alpha_pct(self.alpha_pct() as i32 + k as i32 * (dx - dy));
                true
            }
            Some(Part::Recent(i)) if recents > 0 => {
                let n = recents as i32;
                self.focus = Some(Part::Recent((i as i32 + dx).rem_euclid(n) as usize));
                false
            }
            _ => false,
        }
    }
}

// ---- the editor's side ------------------------------------------------------

impl Edit {
    /// The picker's recent colours, newest first.
    pub(super) fn recents(&self) -> Vec<C4> {
        self.cfg.recent_colors.iter().filter_map(|s| tools::parse(s)).take(config::RECENT_MAX).collect()
    }

    /// `c` heads the recent colours (once; at most [`config::RECENT_MAX`]).
    pub(super) fn add_recent(&mut self, c: C4, now: Instant) {
        let mut v = self.recents();
        v.retain(|x| *x != c);
        v.insert(0, c);
        v.truncate(config::RECENT_MAX);
        let list: Vec<String> = v.iter().map(|c| tools::hex(*c)).collect();
        self.pending.set("recent_colors".into(), Some(config::toml_strings(&list)), now);
        self.cfg.recent_colors = list;
    }

    /// Open the colour picker for `target` at its colour.
    pub(super) fn open_picker(&mut self, target: Target) {
        let p = match target {
            // Over a selection: its colour.
            Target::Tool => {
                let c = self.sel_color().unwrap_or(self.color());
                Picker::new(target, c, Some(c))
            }
            Target::Ring => {
                let ring = self.rec.opts.cam_ring;
                Picker::new(target, ring.unwrap_or(self.th.accent), ring)
            }
        };
        self.pick = Some(p);
        self.palette_open = false;
        self.loupe = None;
    }

    /// The picker's colour shows on its target as it changes.
    pub(super) fn pick_changed(&mut self) {
        let Some(p) = &self.pick else { return };
        let c = p.color();
        match p.target {
            Target::Tool if !self.obj.sel.is_empty() => self.preview_color(c),
            Target::Tool => {
                let t = self.cur_tool();
                self.mem.set_color(t, c);
            }
            Target::Ring => self.rec.opts.cam_ring = Some(c),
        }
    }

    /// Close the picker: `keep` its colour (remembered, and a recent one
    /// when it changed), or put the old one back.
    pub(super) fn close_picker(&mut self, keep: bool) {
        let Some(p) = self.pick.take() else { return };
        self.loupe = None;
        if matches!(self.interact, Interact::Pick(_)) {
            self.interact = Interact::None;
        }
        let now = Instant::now();
        let c = p.color();
        // Kept but not changed (dismissed, or moved and put back): as
        // Esc. Nothing is remembered and no recent is added; a ring that
        // followed the accent keeps following it.
        let keep = keep && c != p.start;
        if p.target == Target::Tool && (!self.obj.sel.is_empty() || self.obj.pick_backup.is_some()) {
            // A selection's colour: one undo step, or the objects back.
            self.end_preview_color(keep, c);
            if keep {
                self.add_recent(c, now);
            }
            return;
        }
        match (p.target, keep) {
            (Target::Tool, true) => {
                self.set_tool_color(c, now);
                self.add_recent(c, now);
            }
            (Target::Tool, false) => {
                let t = self.cur_tool();
                if let Some(o) = p.orig {
                    self.mem.set_color(t, o);
                }
            }
            (Target::Ring, true) => {
                self.rec.opts.cam_ring = Some(c);
                self.remember_ring(now);
                self.add_recent(c, now);
            }
            (Target::Ring, false) => self.rec.opts.cam_ring = p.orig,
        }
    }

    /// The rect of the picker item `kind` (as laid out last).
    pub(super) fn pick_rect(&self, kind: toolbar::Kind) -> Option<FRect> {
        self.toolbar.as_ref()?.items.iter().find(|it| it.kind == kind).map(|it| it.r)
    }

    /// A press on picker part `part` at `p`: it takes the focus; the ring,
    /// the square and the slider follow the pointer until it is released.
    pub(super) fn pick_press(&mut self, part: Part, p: Pt) {
        let Some(pk) = self.pick.as_mut() else { return };
        pk.focus = Some(part);
        pk.eyedrop = false;
        pk.hex_fresh = part == Part::Hex;
        self.loupe = None;
        if matches!(part, Part::Hue | Part::Sv | Part::Alpha) {
            self.interact = Interact::Pick(part);
            self.pick_drag(part, p);
        }
    }

    /// The pointer at `p` while dragging `part`.
    pub(super) fn pick_drag(&mut self, part: Part, p: Pt) {
        let kind = match part {
            Part::Hue => toolbar::Kind::PickHue,
            Part::Sv => toolbar::Kind::PickSv,
            Part::Alpha => toolbar::Kind::PickAlpha,
            _ => return,
        };
        let Some(r) = self.pick_rect(kind) else { return };
        let s = self.ui_scale();
        let Some(pk) = self.pick.as_mut() else { return };
        match part {
            Part::Hue => {
                let (dx, dy) = (p.x - (r.x + r.w / 2.0), p.y - (r.y + r.h / 2.0));
                if dx != 0.0 || dy != 0.0 {
                    pk.set_hue(dx.atan2(-dy).to_degrees());
                }
            }
            Part::Sv => pk.set_sv((p.x - r.x) / r.w, 1.0 - (p.y - r.y) / r.h),
            _ => {
                let (x0, x1) = (r.x + 8.0 * s, r.x1() - 8.0 * s);
                pk.set_alpha((p.x - x0) / (x1 - x0).max(1.0));
            }
        }
        self.pick_changed();
    }

    /// A key while the picker is open (it takes them all).
    pub(super) fn pick_key(&mut self, vk: u32, mods: Mods) {
        let recents = self.recents();
        let n = recents.len();
        let Some(pk) = self.pick.as_mut() else { return };
        let k = wind::key::RETURN;
        match vk {
            wind::key::ESCAPE if pk.eyedrop => {
                pk.eyedrop = false;
                self.loupe = None;
            }
            wind::key::ESCAPE => self.close_picker(false),
            wind::key::TAB => pk.tab(mods.shift, n),
            wind::key::BACK if pk.focus == Some(Part::Hex) => {
                pk.backspace();
                self.pick_changed();
            }
            v if v == k || v == wind::key::SPACE => match pk.focus {
                Some(Part::Eyedrop) => {
                    pk.eyedrop = !pk.eyedrop;
                    self.loupe = None;
                }
                Some(Part::Recent(i)) if i < n => {
                    pk.set_color(recents[i]);
                    self.pick_changed();
                    if v == k {
                        self.close_picker(true);
                    }
                }
                // Enter keeps the colour (not while the field reads as none).
                _ if v == k && (pk.hex_ok || pk.focus != Some(Part::Hex)) => self.close_picker(true),
                _ => {}
            },
            wind::key::LEFT | wind::key::RIGHT | wind::key::UP | wind::key::DOWN => {
                let (dx, dy) = match vk {
                    wind::key::LEFT => (-1, 0),
                    wind::key::RIGHT => (1, 0),
                    wind::key::UP => (0, -1),
                    _ => (0, 1),
                };
                if pk.nudge(dx, dy, mods.shift, n) {
                    self.pick_changed();
                }
            }
            _ => {}
        }
    }

    /// A character typed while the picker is open: into the hex field
    /// (which takes the focus).
    pub(super) fn pick_char(&mut self, ch: char) {
        let Some(pk) = self.pick.as_mut() else { return };
        if ch.is_control() || (ch == ' ' && pk.focus != Some(Part::Hex)) {
            return;
        }
        if pk.focus != Some(Part::Hex) {
            pk.focus = Some(Part::Hex);
            pk.hex_fresh = true;
        }
        pk.type_char(ch);
        self.pick_changed();
    }

    /// The pointer moved: the eyedropper's loupe follows it; true when it
    /// must be drawn again.
    pub(super) fn track_loupe(&mut self, p: Pt) -> bool {
        if self.pick.as_ref().is_some_and(|k| k.eyedrop) {
            self.loupe = Some(p);
            true
        } else {
            self.loupe.take().is_some()
        }
    }

    /// The capture's pixels (with the objects) in `r` (inside the image),
    /// RGBA row by row.
    pub(super) fn backdrop_pixels(&self, r: compose::PxRect) -> Vec<u8> {
        let mut buf = vec![0u8; r.w() as usize * r.h() as usize * 4];
        if r.is_empty() {
            return buf;
        }
        use compose::Backdrop;
        #[cfg(windows)]
        if let Some(scr) = self.gdi.as_ref() {
            super::gdi::GdiBackdrop { scr, opaque: true }.fill(r, compose::Source::Plain, &mut buf, r.w() as usize, Order::Rgba);
            return buf;
        }
        PixBufBackdrop { img: self.composed(), dim: self.th.dim }.fill(r, compose::Source::Plain, &mut buf, r.w() as usize, Order::Rgba);
        buf
    }

    /// The loupe's pixels around `p` (black off the image), row by row.
    pub(super) fn loupe_pixels(&self, p: Pt) -> Vec<C4> {
        let n = chrome::LOUPE_N as i32;
        let (x0, y0) = (p.x.floor() as i32 - n / 2, p.y.floor() as i32 - n / 2);
        let r = compose::PxRect::new(x0, y0, x0 + n, y0 + n);
        let q = r.intersect(&compose::PxRect::image(self.shot.size));
        let mut out = vec![C4::rgb(0, 0, 0); (n * n) as usize];
        let buf = self.backdrop_pixels(q);
        for y in q.y0..q.y1 {
            for x in q.x0..q.x1 {
                let i = (((y - q.y0) * q.w() + (x - q.x0)) * 4) as usize;
                out[((y - y0) * n + (x - x0)) as usize] = C4::rgb(buf[i], buf[i + 1], buf[i + 2]);
            }
        }
        out
    }

    /// The colour of the pixel at `p` (the eyedropper).
    pub(super) fn sample(&self, p: Pt) -> C4 {
        let n = chrome::LOUPE_N;
        self.loupe_pixels(p)[n * n / 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips_every_8th_rgb() {
        for r in (0..=255u32).step_by(8) {
            for g in (0..=255u32).step_by(8) {
                for b in (0..=255u32).step_by(8) {
                    let (h, s, v) = rgb_to_hsv(r as u8, g as u8, b as u8);
                    assert!((0.0..360.0).contains(&h) && (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&v));
                    assert_eq!(hsv_to_rgb(h, s, v), (r as u8, g as u8, b as u8), "{r} {g} {b}");
                }
            }
        }
        // And HSV -> RGB -> HSV for saturated colours.
        for h in (0..360).step_by(15) {
            let (r, g, b) = hsv_to_rgb(h as f32, 1.0, 1.0);
            let (h2, s2, v2) = rgb_to_hsv(r, g, b);
            assert!((h2 - h as f32).abs() < 1.0 && s2 == 1.0 && v2 == 1.0, "{h}: {h2}");
        }
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 0.5), (0, 0, 128));
        assert_eq!(hsv_to_rgb(360.0, 0.0, 1.0), (255, 255, 255));
    }

    #[test]
    fn hex_field_input() {
        assert_eq!(parse_input("#f04438"), Some((0xf0, 0x44, 0x38)));
        assert_eq!(parse_input("#F04438"), Some((0xf0, 0x44, 0x38)));
        assert_eq!(parse_input("  f04438 "), Some((0xf0, 0x44, 0x38)));
        assert_eq!(parse_input("#FA0"), Some((0xff, 0xaa, 0x00)));
        assert_eq!(parse_input("rgb(1, 2, 255)"), Some((1, 2, 255)));
        assert_eq!(parse_input("RGB(10,20,30)"), Some((10, 20, 30)));
        for bad in ["", "#", "#ff", "#ff00", "#gg0000", "#ff000000", "rgb(1,2)", "rgb(1,2,256)", "rgb(a,b,c)", "red", "rgb 1 2 3"] {
            assert_eq!(parse_input(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn typing_follows_while_it_reads() {
        let mut p = Picker::new(Target::Tool, C4::rgb(255, 0, 0), Some(C4::rgb(255, 0, 0)));
        assert_eq!(p.hex, "#FF0000");
        p.focus = Some(Part::Hex);
        p.hex_fresh = true;
        for ch in "#00f".chars() {
            p.type_char(ch);
        }
        assert!(p.hex_ok);
        assert_eq!(p.color(), C4::rgb(0, 0, 255));
        p.type_char('x');
        assert!(!p.hex_ok, "not a color");
        assert_eq!(p.color(), C4::rgb(0, 0, 255), "keeps the last good one");
        p.backspace();
        assert!(p.hex_ok);
        // Greys keep the hue.
        p.set_color(C4::rgb(128, 128, 128));
        assert_eq!(p.h, 240.0);
        p.set_sv(1.0, 1.0);
        assert_eq!(p.color(), C4::rgb(0, 0, 255));
    }

    #[test]
    fn keyboard_tabs_and_nudges() {
        let mut p = Picker::new(Target::Tool, C4::rgb(255, 0, 0), None);
        p.tab(false, 2);
        assert_eq!(p.focus, Some(Part::Hue));
        assert!(p.nudge(1, 0, false, 2));
        assert_eq!(p.h, 1.0);
        assert!(p.nudge(0, 1, true, 2));
        assert_eq!(p.h, 351.0, "down turns back, by ten with Shift");
        p.tab(false, 2);
        assert_eq!(p.focus, Some(Part::Sv));
        p.nudge(-1, 1, false, 2);
        assert!((p.s - 0.99).abs() < 1e-4 && (p.v - 0.99).abs() < 1e-4);
        p.tab(false, 2);
        assert_eq!(p.focus, Some(Part::Hex));
        assert!(p.hex_fresh);
        p.tab(false, 2);
        assert_eq!(p.focus, Some(Part::Alpha));
        p.nudge(-1, 0, true, 2);
        assert_eq!(p.alpha_pct(), 90);
        for want in [Part::Eyedrop, Part::Recent(0), Part::Recent(1), Part::Hue] {
            p.tab(false, 2);
            assert_eq!(p.focus, Some(want));
        }
        p.tab(true, 2);
        assert_eq!(p.focus, Some(Part::Recent(1)));
        p.nudge(1, 0, false, 2);
        assert_eq!(p.focus, Some(Part::Recent(0)), "wraps");
        // The ring has no opacity.
        let r = Picker::new(Target::Ring, C4::new(1, 2, 3, 40), None);
        assert!(!r.parts(0).contains(&Part::Alpha));
        assert_eq!(r.color().a, 255);
    }
}
