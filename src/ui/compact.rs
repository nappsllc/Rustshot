//! The Settings window's compact controls, as on the Settings board
//! (docs/design/rustshot-ui/Settings.dc.html): the section rail, section
//! headings, 32×18 toggles, segmented choices, accent swatches, 28 px
//! buttons, links, and key-binding rows whose key caps record a new
//! chord ("Press shortcut… Esc to cancel"). Colors are the board's
//! tokens ([`Tok`]), derived from the theme so the accent (and its
//! override) follows.

use super::controls::{fade, fill, focus_ring, is_modifier, mix, press_logic, ring, show_focus, text, tw};
use super::{contains, id_of, Ui};
use crate::hotkey;
use crate::icon_path::draw_icon;
use crate::keymap::Chord;
use crate::objects::FRect;
use crate::theme::{rgba, Theme};
use crate::uifb::C4;
use crate::wind::key;

/// The board's color tokens for one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tok {
    /// Window, rail.
    pub win: C4,
    pub side: C4,
    /// Separators, control borders.
    pub sep: C4,
    pub bd: C4,
    /// Icons, hovered icons.
    pub ic: C4,
    pub ich: C4,
    /// Hover fill, field / segment fill.
    pub hv: C4,
    pub sf2: C4,
    pub ac: C4,
    pub acbg: C4,
    pub acrg: C4,
    pub acfg: C4,
    pub tx: C4,
    pub tx2: C4,
    pub tx3: C4,
    pub er: C4,
    pub er_bg: C4,
    /// A toggle's track when off.
    pub tg_off: C4,
    /// Key caps.
    pub key_bg: C4,
    pub key_tx: C4,
}

const fn hex(v: u32) -> C4 {
    rgba(v, 1000)
}

impl Tok {
    pub fn of(th: &Theme) -> Tok {
        let (ac, acbg, acrg, acfg) = (th.accent, th.accent_bg, th.accent_ring, th.accent_fg);
        if th.dark {
            Tok {
                win: hex(0x1B1C20),
                side: hex(0x181A1F),
                sep: rgba(0xFFFFFF, 80),
                bd: rgba(0xFFFFFF, 90),
                ic: hex(0xA3A7B3),
                ich: hex(0xECEEF3),
                hv: rgba(0xFFFFFF, 60),
                sf2: rgba(0xFFFFFF, 50),
                ac,
                acbg,
                acrg,
                acfg,
                tx: hex(0xECEEF3),
                tx2: hex(0x9296A3),
                tx3: hex(0x6C7080),
                er: hex(0xFF7A7A),
                er_bg: rgba(0xFF7A7A, 140),
                tg_off: rgba(0xFFFFFF, 140),
                key_bg: rgba(0xFFFFFF, 120),
                key_tx: hex(0xD4D7DF),
            }
        } else {
            Tok {
                win: hex(0xFAFAFC),
                side: hex(0xF1F2F5),
                sep: rgba(0x000000, 80),
                bd: rgba(0x000000, 90),
                ic: hex(0x5F6473),
                ich: hex(0x15171E),
                hv: rgba(0x000000, 60),
                sf2: rgba(0x000000, 35),
                ac,
                acbg,
                acrg,
                acfg,
                tx: hex(0x15171E),
                tx2: hex(0x737889),
                tx3: hex(0x9A9EAB),
                er: hex(0xD64545),
                er_bg: rgba(0xD64545, 110),
                tg_off: rgba(0x000000, 160),
                key_bg: rgba(0x000000, 70),
                key_tx: hex(0x3B3F4B),
            }
        }
    }
}

/// A key cap as drawn (text, and whether it is in a conflict).
#[derive(Clone, Debug, PartialEq)]
pub struct Cap {
    pub text: String,
    pub err: bool,
}

/// One rebindable chord in a key-binding row.
pub struct Slot<'a> {
    /// Unique per window (focus, recording).
    pub id: String,
    pub chord: &'a mut Option<Chord>,
    /// In a conflict: red caps.
    pub err: bool,
}

/// The caps of `c`: "Ctrl", "Shift", "S" (`split`), or one "Ctrl S".
pub fn caps_of(c: Option<&Chord>, split: bool) -> Vec<String> {
    let Some(c) = c else { return vec!["\u{2014}".into()] };
    let caps = crate::editor::style::chord_caps(c);
    if split { caps } else { vec![caps.join(" ")] }
}

/// CSS font sizes (the boards) → the baked font's `px` (its ascent to
/// descent height): Inter's ascent + descent is 1.21 em.
pub const CSS: f32 = 1.21;

/// The text "Press shortcut…" cap.
pub const REC_TEXT: crate::i18n::Key = key!("ui.key.press");
pub const REC_HINT: crate::i18n::Key = key!("ui.key.esc_cancel");

/// A group label's height in the rail (logical px).
pub const RAIL_LABEL_H: f32 = 24.0;

/// How [`Ui::rail`] lays out its items: an item's height, the distance
/// from one item's top to the next, and the group labels (each shown
/// before item `.0`).
#[derive(Clone, Copy, Debug)]
pub struct RailStyle<'a> {
    pub item: f32,
    pub pitch: f32,
    pub groups: &'a [(usize, crate::i18n::Key)],
}

/// The rail's item tops and its group labels' tops (with their index in
/// `style.groups`), logical px: from the top of `area`, items from
/// `bottom_from` on at its bottom.
pub fn rail_layout(n: usize, bottom_from: usize, area: FRect, style: &RailStyle) -> (Vec<f32>, Vec<(f32, usize)>) {
    let mut labels = Vec::new();
    let mut tops = Vec::with_capacity(n);
    let mut y = area.y;
    for i in 0..n {
        if i < bottom_from {
            if let Some(g) = style.groups.iter().position(|(at, _)| *at == i) {
                labels.push((y, g));
                y += RAIL_LABEL_H;
            }
            tops.push(y);
            y += style.pitch;
        } else {
            tops.push(area.y1() - (n - i) as f32 * style.pitch + (style.pitch - style.item));
        }
    }
    (tops, labels)
}

impl Ui<'_> {
    /// Text at `size` (logical px) left-aligned at `x`, centred on `cy`
    /// (physical px); `bold` thickens Inter Medium towards SemiBold.
    pub fn text_at(&mut self, size: f32, s: &str, x: f32, cy: f32, c: C4, bold: bool) -> f32 {
        let w = text(self, size * CSS, s, x, cy, c);
        if bold {
            text(self, size * CSS, s, x + self.px(0.3), cy, c);
        }
        w
    }

    /// Width of `s` at `size` (CSS px, see [`CSS`]) in logical px.
    pub fn text_w(&self, size: f32, s: &str) -> f32 {
        tw(self, size * CSS, s)
    }

    /// The width of `full[..end]` at `size` as part of `full` (logical
    /// px): see [`crate::fonts::UiFont::prefix_width`].
    pub fn prefix_w(&self, size: f32, full: &str, end: usize) -> f32 {
        crate::fonts::UI.prefix_width(full, end, self.px(size * CSS)) / self.k
    }

    /// `s` cut with an ellipsis to `w` logical px at `size`: the longest
    /// prefix (a binary search over its character boundaries, each
    /// candidate measured as it will be drawn) that fits with the "…".
    pub fn fit_text(&self, size: f32, s: &str, w: f32) -> String {
        if self.text_w(size, s) <= w {
            return s.to_string();
        }
        let ends: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
        let cut = |n: usize| format!("{}\u{2026}", s[..ends.get(n).copied().unwrap_or(s.len())].trim_end());
        // ends[n] is the end of the first n characters; 0 always "fits".
        let (mut lo, mut hi) = (0usize, ends.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.text_w(size, &cut(mid)) <= w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        cut(lo)
    }

    /// A section heading ("APPEARANCE"): 10.5/600, uppercase, tracked
    /// .07em, 14 px above and 6 below (33 px in all).
    pub fn sec(&mut self, s: &str, tok: &Tok) {
        let r = self.alloc(None, 33.0);
        let size = 10.5;
        let up = crate::i18n::upper(s);
        let track = self.px(size * 0.07);
        let cy = r.y + self.px(14.0 + 6.5);
        let mut x = r.x;
        let c = tok.tx3.fade(fade(self));
        if !crate::i18n::tracks(&up) {
            self.text_at(size, &up, x, cy, c, true);
            return;
        }
        for ch in up.chars() {
            let t = ch.to_string();
            self.text_at(size, &t, x, cy, c, true);
            x += self.px(self.text_w(size, &t)) + track;
        }
    }

    /// The board's 32×18 switch, centred in a control-height box.
    pub fn switch(&mut self, id: &str, on: &mut bool, tok: &Tok) -> bool {
        let r = self.alloc(Some(32.0), self.ctl_h);
        let id = id_of(id);
        let focused = self.focusable(id);
        let (hov, _, mut clicked) = press_logic(self, id, r);
        clicked |= self.take_key(focused, &[key::RETURN, key::SPACE]);
        if clicked && self.enabled {
            *on = !*on;
        }
        let k = fade(self);
        let (w, h) = (self.px(32.0).round(), self.px(18.0).round());
        let t = FRect { x: r.x, y: (r.y + (r.h - h) / 2.0).round(), w, h };
        let track = match (*on, hov) {
            (true, true) => mix(tok.ac, C4::rgb(255, 255, 255), 0.12),
            (true, false) => tok.ac,
            (false, true) => tok.tg_off.with_alpha(tok.tg_off.a.saturating_add(20)),
            (false, false) => tok.tg_off,
        };
        fill(self, t, 9.0, track.fade(k));
        let kr = self.px(7.0);
        let cx = if *on { t.x1() - self.px(9.0) } else { t.x + self.px(9.0) };
        let cy = t.y + t.h / 2.0;
        self.fb.fill_circle(cx, cy + self.px(1.0), kr + self.px(0.5), C4::new(0, 0, 0, 70).fade(k));
        self.fb.fill_circle(cx, cy, kr, C4::rgb(255, 255, 255).fade(k));
        if show_focus(self, id) {
            focus_ring(self, t, 9.0);
        }
        clicked && self.enabled
    }

    /// Width a [`Ui::seg`] takes for `labels` (logical px).
    pub fn seg_width(&self, labels: &[&str]) -> f32 {
        labels.iter().map(|l| (self.text_w(11.5, l) + 20.0).ceil()).sum::<f32>() + 2.0 * (labels.len() as f32 - 1.0) + 6.0
    }

    /// Segmented choice (Theme, format, ...): 3 px padding, 24 px items,
    /// the current one with the accent tint. `sel` past the end shows
    /// none chosen. Left/Right move while focused.
    pub fn seg(&mut self, id: &str, labels: &[&str], sel: &mut usize, tok: &Tok) -> bool {
        let total = self.seg_width(labels);
        let r = self.alloc(Some(total), self.ctl_h);
        let id = id_of(id);
        let focused = self.focusable(id);
        let old = *sel;
        let n = labels.len();
        let h = self.px(30.0).round();
        let box_ = FRect { x: r.x, y: (r.y + (r.h - h) / 2.0).round(), w: r.w, h };
        let pad = self.px(3.0).round();
        let mut segs = Vec::with_capacity(n);
        let mut x = box_.x + pad;
        for l in labels {
            let w = self.px((self.text_w(11.5, l) + 20.0).ceil()).round();
            segs.push(FRect { x, y: box_.y + pad, w, h: box_.h - 2.0 * pad });
            x += w + self.px(2.0);
        }
        if let Some(p) = self.input.pressed
            && self.pressed_in(box_)
        {
            self.focus.set(Some(id), false);
            if let Some(i) = segs.iter().position(|s| contains(*s, p)) {
                *sel = i;
            }
        }
        if focused && self.enabled && n > 0 {
            for (i, vk, m) in self.keys() {
                if m.ctrl || m.alt {
                    continue;
                }
                match super::visual_key(&self.fb, vk) {
                    key::LEFT => *sel = (*sel).min(n).saturating_sub(1),
                    key::RIGHT => *sel = if *sel >= n { 0 } else { (*sel + 1).min(n - 1) },
                    _ => continue,
                }
                self.consume(i);
            }
        }
        let k = fade(self);
        fill(self, box_, 8.0, tok.sf2.fade(k));
        ring(self, box_, 8.0, tok.bd.fade(k));
        for (i, s) in segs.iter().enumerate() {
            let on = i == *sel;
            let hov = self.hovered(*s);
            if on {
                fill(self, *s, 5.0, tok.acbg.fade(k));
                ring(self, *s, 5.0, tok.acrg.fade(k));
            } else if hov {
                fill(self, *s, 5.0, tok.hv.fade(k));
            }
            let c = if on { tok.acfg } else if hov { tok.ich } else { tok.ic };
            let x = s.x + (s.w - self.px(self.text_w(11.5, labels[i]))) / 2.0;
            self.text_at(11.5, labels[i], x, s.y + s.h / 2.0, c.fade(k), false);
        }
        if show_focus(self, id) {
            focus_ring(self, box_, 8.0);
        }
        *sel != old
    }

    /// A 20 px color swatch; `on` draws the 2 px ring outside a 2 px gap.
    /// `c = None` draws the custom swatch (a plus). Returns a click.
    pub fn swatch(&mut self, id: &str, c: Option<C4>, on: bool, tok: &Tok) -> bool {
        let r = self.alloc(Some(20.0), self.ctl_h);
        let id = id_of(id);
        let focused = self.focusable(id);
        let (hov, _, mut clicked) = press_logic(self, id, r);
        clicked |= self.take_key(focused, &[key::RETURN, key::SPACE]);
        let k = fade(self);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let rad = self.px(10.0);
        if on {
            self.fb.fill_circle(cx, cy, rad + self.px(4.0), tok.ac.fade(k));
            self.fb.fill_circle(cx, cy, rad + self.px(2.0), tok.win.fade(k));
        }
        match c {
            Some(c) => self.fb.fill_circle(cx, cy, rad, c.fade(k)),
            None => {
                self.fb.fill_circle(cx, cy, rad, tok.sf2.fade(k));
                let s = self.px(14.0);
                draw_icon(&mut self.fb, "set-plus", cx - s / 2.0, cy - s / 2.0, s, if hov { tok.ich } else { tok.ic }.fade(k));
            }
        }
        let inner = if tok.win.r > 128 { C4::new(0, 0, 0, 40) } else { C4::new(255, 255, 255, 56) };
        self.fb.stroke_circle(cx, cy, rad - self.px(0.5), self.px(1.0), inner.fade(k));
        if hov && !on {
            self.fb.stroke_circle(cx, cy, rad + self.px(2.5), self.px(1.0), tok.bd.fade(k));
        }
        if show_focus(self, id) {
            let lw = self.px(2.0).round();
            self.fb.stroke_circle(cx, cy, rad + self.px(6.0), lw, tok.ac.fade(0.7));
        }
        clicked && self.enabled
    }

    /// Width a [`Ui::cbutton`] takes (logical px).
    pub fn cbutton_width(&self, label: &str, icon: bool) -> f32 {
        (self.text_w(12.0, label) + 24.0 + if icon { 22.0 } else { 0.0 }).ceil()
    }

    /// The board's 28 px button: `hv` fill with a `bd` ring, or (`pri`)
    /// the accent tint; an optional 16 px icon before the label.
    pub fn cbutton(&mut self, id: &str, label: &str, icon: Option<&str>, pri: bool, tok: &Tok) -> bool {
        self.cbutton_ex(id, label, icon, pri, 12.0, tok)
    }

    /// [`Ui::cbutton`] with `size` text, centred when the box is wider
    /// than the label (a placed or widened button), cut to fit.
    pub fn cbutton_ex(&mut self, id: &str, label: &str, icon: Option<&str>, pri: bool, size: f32, tok: &Tok) -> bool {
        let w = self.cbutton_width(label, icon.is_some());
        let r = self.alloc(Some(w), self.ctl_h);
        let id = id_of(id);
        let focused = self.focusable(id);
        let (hov, pressed, mut clicked) = press_logic(self, id, r);
        clicked |= self.take_key(focused, &[key::RETURN, key::SPACE]);
        let k = fade(self);
        let h = self.px(28.0).round();
        let b = FRect { x: r.x, y: (r.y + (r.h - h) / 2.0).round(), w: r.w, h };
        let (bg, fg, rg) = if pri {
            (if hov { self.theme.accent_bg_hover } else { tok.acbg }, tok.acfg, tok.acrg)
        } else {
            (tok.hv, tok.tx, tok.bd)
        };
        fill(self, b, 7.0, bg.fade(k));
        if (hov && !pri) || pressed {
            fill(self, b, 7.0, tok.hv.fade(k));
        }
        ring(self, b, 7.0, rg.fade(k));
        if show_focus(self, id) {
            focus_ring(self, b, 7.0);
        }
        let cy = b.y + b.h / 2.0;
        let room = b.w / self.k - 16.0 - if icon.is_some() { 22.0 } else { 0.0 };
        let shown = self.fit_text(size, label, room);
        #[cfg(test)]
        if shown != label {
            super::preview::note_cut(label);
        }
        let content = self.px(self.text_w(size, &shown) + if icon.is_some() { 22.0 } else { 0.0 });
        let mut x = (b.x + (b.w - content) / 2.0).max(b.x + self.px(8.0)).round();
        if let Some(name) = icon {
            let s = self.px(16.0);
            draw_icon(&mut self.fb, name, x, cy - s / 2.0, s, fg.fade(k));
            x += self.px(22.0);
        }
        self.text_at(size, &shown, x, cy, fg.fade(k), false);
        clicked && self.enabled
    }

    /// A link (12.5/500, `acfg`), with the external-link glyph when `ext`.
    pub fn clink(&mut self, id: &str, label: &str, size: f32, ext: bool, tok: &Tok) -> bool {
        self.clink_in(id, label, size, ext, tok.acfg)
    }

    /// A 16 px checkbox and its 12 px label (as in the Edit window's
    /// "Don't ask again"). Returns whether it flipped.
    pub fn checkbox(&mut self, id: &str, label: &str, on: &mut bool, tok: &Tok) -> bool {
        let w = (16.0 + 8.0 + self.text_w(12.0, label)).ceil();
        let r = self.alloc(Some(w), self.ctl_h);
        let id = id_of(id);
        let focused = self.focusable(id);
        let (hov, _, mut clicked) = press_logic(self, id, r);
        clicked |= self.take_key(focused, &[key::SPACE]);
        if clicked && self.enabled {
            *on = !*on;
        }
        let k = fade(self);
        let s = self.px(16.0).round();
        let b = FRect { x: r.x, y: (r.y + (r.h - s) / 2.0).round(), w: s, h: s };
        if *on {
            fill(self, b, 4.0, tok.ac.fade(k));
            let is = self.px(12.0);
            let o = self.px(2.0);
            draw_icon(&mut self.fb, "check", b.x + o, b.y + o, is, C4::rgb(0x0E, 0x10, 0x20).fade(k));
        } else {
            let lw = (1.5 * self.k).max(1.0);
            let c = if hov { tok.tx2 } else { tok.tg_off };
            let inset = FRect { x: b.x + lw / 2.0, y: b.y + lw / 2.0, w: b.w - lw, h: b.h - lw };
            self.fb.stroke_rounded(inset, 3.25 * self.k, lw, c.fade(k));
        }
        if show_focus(self, id) {
            focus_ring(self, b, 4.0);
        }
        self.text_at(12.0, label, b.x + self.px(24.0), r.y + r.h / 2.0, tok.tx2.fade(k), false);
        clicked && self.enabled
    }

    /// [`Ui::clink`] in color `c` (a status line that is also a link).
    pub fn clink_in(&mut self, id: &str, label: &str, size: f32, ext: bool, c: C4) -> bool {
        let w = (self.text_w(size, label) + if ext { 19.0 } else { 0.0 }).ceil();
        let r = self.alloc(Some(w), self.ctl_h);
        let id = id_of(id);
        let focused = self.focusable(id);
        let (hov, _, mut clicked) = press_logic(self, id, r);
        clicked |= self.take_key(focused, &[key::RETURN, key::SPACE]);
        let k = fade(self);
        let c = c.fade(k);
        let cy = r.y + r.h / 2.0;
        let tw_ = self.text_at(size, label, r.x, cy, c, false);
        if ext {
            let s = self.px(13.0);
            let ix = r.x + self.px(self.text_w(size, label) + 6.0);
            draw_icon(&mut self.fb, "set-external", ix, cy - s / 2.0, s, c);
        }
        if hov {
            let lw = self.k.round().max(1.0);
            let y = (cy + self.px(size * 0.5 * CSS)).round();
            self.fb.fill_rect(r.x as i32, y as i32, tw_.max(0.0) as i32, lw as i32, c);
        }
        if show_focus(self, id) {
            let fr = FRect { x: r.x - self.px(4.0), y: cy - self.px(11.0), w: r.w + self.px(8.0), h: self.px(22.0) };
            focus_ring(self, fr, 4.0);
        }
        clicked && self.enabled
    }

    /// The section rail: one focusable list (Up/Down/Home/End), items
    /// `style.item` px tall, `style.pitch` apart from the top of `area`; a
    /// group label (`style.groups`: shown before item i) takes
    /// [`RAIL_LABEL_H`]; items from `bottom_from` on sit at the bottom of
    /// `area` (About). Returns whether `sel` changed.
    #[allow(clippy::too_many_arguments)]
    pub fn rail(&mut self, id: &str, items: &[(&str, &str)], bottom_from: usize, area: FRect, sel: &mut usize, style: &RailStyle, tok: &Tok) -> bool {
        let id = id_of(id);
        let k = self.k;
        let px = |v: f32| (v * k).round();
        let n = items.len();
        let (tops, labels) = rail_layout(n, bottom_from, area, style);
        let rects: Vec<FRect> = tops.iter().map(|y| FRect { x: px(area.x), y: px(*y), w: px(area.w), h: px(style.item) }).collect();
        let whole = FRect { x: px(area.x), y: px(area.y), w: px(area.w), h: px(area.h) };
        self.last_alloc = rects.get(*sel).copied().unwrap_or(whole);
        let focused = self.focusable(id);
        let old = *sel;
        if let Some(p) = self.input.pressed
            && self.pressed_in(whole)
            && let Some(i) = rects.iter().position(|r| contains(*r, p))
        {
            self.focus.set(Some(id), false);
            *sel = i;
        }
        if focused && self.enabled {
            for (i, vk, m) in self.keys() {
                if m.ctrl || m.alt {
                    continue;
                }
                match vk {
                    key::UP => *sel = sel.saturating_sub(1),
                    key::DOWN => *sel = (*sel + 1).min(n - 1),
                    key::HOME => *sel = 0,
                    key::END => *sel = n - 1,
                    _ => continue,
                }
                self.consume(i);
            }
        }
        let f = fade(self);
        for (y, gi) in labels {
            // Like the board's `.sec` (uppercase, tracked), a size smaller.
            let up = crate::i18n::upper(&style.groups[gi].1);
            let size = 10.0;
            let track = self.px(size * 0.07);
            let cy = px(y + RAIL_LABEL_H - 9.0);
            let mut x = px(area.x + 10.0);
            let c = tok.tx3.fade(f);
            if !crate::i18n::tracks(&up) {
                self.text_at(size, &up, x, cy, c, true);
                continue;
            }
            for ch in up.chars() {
                let t = ch.to_string();
                self.text_at(size, &t, x, cy, c, true);
                x += self.px(self.text_w(size, &t)) + track;
            }
        }
        for (i, ((icon, label), r)) in items.iter().zip(&rects).enumerate() {
            let on = i == *sel;
            let hov = self.hovered(*r);
            if on {
                fill(self, *r, 8.0, tok.acbg.fade(f));
                ring(self, *r, 8.0, tok.acrg.fade(f));
            } else if hov {
                fill(self, *r, 8.0, tok.hv.fade(f));
            }
            let c = if on { tok.acfg } else if hov { tok.ich } else { tok.ic };
            let s = self.px(16.0);
            let cy = r.y + r.h / 2.0;
            let ix = r.x + self.px(10.0);
            draw_icon(&mut self.fb, icon, ix, (cy - s / 2.0).round(), s, c.fade(f));
            self.text_at(12.5, label, r.x + self.px(36.0), cy, c.fade(f), false);
        }
        if show_focus(self, id)
            && let Some(r) = rects.get(*sel)
        {
            focus_ring(self, *r, 8.0);
        }
        *sel != old
    }

    /// Key caps right-aligned in `r` (physical px): 22 px tall, 4 apart;
    /// returns each cap's rect. `rec` replaces them with the recording cap.
    fn draw_caps(&mut self, r: FRect, caps: &[Cap], rec: bool, tok: &Tok) -> Vec<FRect> {
        let k = fade(self);
        let h = self.px(22.0).round();
        let cy = r.y + r.h / 2.0;
        let y = (cy - h / 2.0).round();
        if rec {
            let w = self.px((self.text_w(11.0, &REC_TEXT) + self.text_w(11.0, &REC_HINT) + 18.0).max(110.0)).round();
            let cr = FRect { x: r.x1() - w, y, w, h };
            fill(self, cr, 5.0, tok.acbg.fade(k));
            ring(self, cr, 5.0, tok.acrg.fade(k));
            let x = cr.x + self.px(7.0);
            let w = self.text_at(11.0, &REC_TEXT, x, cy, tok.acfg.fade(k), false);
            self.text_at(11.0, &REC_HINT, x + w + self.px(4.0), cy, tok.acfg.fade(0.6 * k), false);
            return vec![cr];
        }
        let ws: Vec<f32> = caps.iter().map(|c| self.px((self.text_w(11.0, &c.text) + 14.0).max(22.0)).round()).collect();
        let gap = self.px(4.0).round();
        let mut x = r.x1() - ws.iter().sum::<f32>() - gap * (ws.len().saturating_sub(1)) as f32;
        let mut out = Vec::with_capacity(caps.len());
        for (c, w) in caps.iter().zip(ws) {
            let cr = FRect { x, y, w, h };
            let (bg, fg) = if c.err { (tok.er_bg, tok.er) } else { (tok.key_bg, tok.key_tx) };
            fill(self, cr, 5.0, bg.fade(k));
            // The board's inset bottom edge.
            let lw = self.k.round().max(1.0);
            let edge = FRect { x: cr.x + self.px(2.0), y: cr.y1() - lw, w: cr.w - self.px(4.0), h: lw };
            fill(self, edge, 0.0, C4::new(0, 0, 0, if tok.win.r > 128 { 30 } else { 90 }).fade(k));
            let tx = cr.x + (cr.w - self.px(self.text_w(11.0, &c.text))) / 2.0;
            self.text_at(11.0, &c.text, tx, cy, fg.fade(k), true);
            out.push(cr);
            x += w + gap;
        }
        out
    }

    /// Recording keys for slot `id`: Esc cancels, Backspace unbinds, a
    /// modifier alone waits; returns whether `chord` changed.
    fn record_keys(&mut self, id: u64, chord: &mut Option<Chord>, focused: bool) -> bool {
        let old = *chord;
        let mut recording = self.focus.recording == Some(id);
        if !(focused && self.enabled) {
            return false;
        }
        for (i, vk, m) in self.keys() {
            if !recording {
                if matches!(vk, key::RETURN | key::SPACE) && !m.ctrl && !m.alt {
                    recording = true;
                    self.consume(i);
                }
                continue;
            }
            self.consume(i);
            if is_modifier(vk) {
                continue;
            }
            if vk == key::ESCAPE && !m.ctrl && !m.alt && !m.shift {
                recording = false;
            } else if vk == key::BACK && !m.ctrl && !m.alt && !m.shift {
                *chord = None;
                recording = false;
            } else if hotkey::key_name(vk).is_some() {
                let meta = self.allow_meta && crate::wind::meta_down();
                *chord = Some(Chord { vk, ctrl: m.ctrl, shift: m.shift, alt: m.alt, meta });
                recording = false;
            }
        }
        self.focus.recording = recording.then_some(id);
        *chord != old
    }

    /// A key-binding row (`h` tall, the board's `.krow`): `label` left,
    /// the slots' caps right. One slot: a click anywhere on the row
    /// records; several: a click on a cap records that one (the label
    /// then names it: `names[i]`). `split` draws one cap per key. Returns
    /// the slot whose chord changed.
    pub fn key_row(&mut self, label: &str, names: &[&str], slots: &mut [Slot], split: bool, h: f32, tok: &Tok) -> Option<usize> {
        let r = self.alloc(None, h);
        let ids: Vec<u64> = slots.iter().map(|s| id_of(&s.id)).collect();
        let k = fade(self);
        let rec = ids.iter().position(|i| self.focus.recording == Some(*i));
        let row_hov = self.hovered(r);
        if rec.is_some() {
            fill(self, r, 7.0, tok.acbg.fade(k));
        } else if row_hov {
            fill(self, r, 7.0, tok.hv.fade(k));
        }
        // Caps: laid out right to left from the row's right padding.
        let inner = FRect { x: r.x + self.px(8.0), y: r.y, w: r.w - self.px(16.0), h: r.h };
        let caps: Vec<Vec<Cap>> = slots
            .iter()
            .map(|s| caps_of(s.chord.as_ref(), split).into_iter().map(|t| Cap { text: t, err: s.err }).collect())
            .collect();
        // Each slot's caps as one group; groups 4 apart.
        let gap = self.px(4.0).round();
        let mut right = inner.x1();
        let mut areas = vec![FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; slots.len()];
        for i in (0..slots.len()).rev() {
            // While one slot records, it alone shows (its name is in the label).
            if rec.is_some_and(|r| r != i) {
                areas[i] = FRect { x: inner.x1(), y: r.y, w: 0.0, h: r.h };
                continue;
            }
            let width = if rec == Some(i) {
                self.px((self.text_w(11.0, &REC_TEXT) + self.text_w(11.0, &REC_HINT) + 18.0).max(110.0)).round()
            } else {
                let ws: f32 = caps[i].iter().map(|c| self.px((self.text_w(11.0, &c.text) + 14.0).max(22.0)).round()).sum();
                ws + gap * (caps[i].len().saturating_sub(1)) as f32
            };
            areas[i] = FRect { x: right - width, y: r.y, w: width, h: r.h };
            right -= width + gap;
        }
        let mut changed = None;
        for i in 0..slots.len() {
            self.last_alloc = if slots.len() == 1 { r } else { areas[i] };
            let focused = self.focusable(ids[i]);
            let hit = if slots.len() == 1 { r } else { areas[i] };
            if self.pressed_in(hit) {
                self.focus.set(Some(ids[i]), false);
                self.focus.recording = Some(ids[i]);
            } else if self.input.pressed.is_some() && self.focus.recording == Some(ids[i]) {
                self.focus.recording = None;
            }
            if self.record_keys(ids[i], slots[i].chord, focused) {
                changed = Some(i);
            }
        }
        let rec = ids.iter().position(|i| self.focus.recording == Some(*i));
        let cy = r.y + r.h / 2.0;
        let name = match (rec, slots.len()) {
            (Some(i), n) if n > 1 => tr!("ui.key.row_slot", label, name = names.get(i).copied().unwrap_or("")),
            _ => label.to_string(),
        };
        let size = if h < 32.0 { 12.0 } else { 12.5 };
        self.text_at(size, &name, inner.x, cy, tok.tx.fade(k), false);
        for i in 0..slots.len() {
            let a = areas[i];
            if a.w <= 0.0 {
                continue;
            }
            let hov = slots.len() > 1 && rec != Some(i) && self.hovered(a);
            self.draw_caps(a, &caps[i], rec == Some(i), tok);
            if hov {
                let hr = FRect { x: a.x - self.px(2.0), y: cy - self.px(13.0), w: a.w + self.px(4.0), h: self.px(26.0) };
                ring(self, hr, 6.0, tok.acrg.fade(k));
            }
            if show_focus(self, ids[i]) && rec != Some(i) {
                let fr = if slots.len() == 1 { r } else { FRect { x: a.x - self.px(2.0), y: cy - self.px(13.0), w: a.w + self.px(4.0), h: self.px(26.0) } };
                focus_ring(self, fr, 6.0);
            }
        }
        changed
    }
}
