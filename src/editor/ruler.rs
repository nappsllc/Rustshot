//! The screen ruler (U): drag a line and its length shows live in px (and
//! in logical px when the capture is scaled), with Δx, Δy and the angle;
//! Shift snaps to 15° steps; with Ctrl it measures a rect (width × height
//! and area). On release the measurement stays: Enter keeps it as an
//! annotation (a dimension line with arrowheads at both ends and its
//! label, or the rect with its size), Esc discards it. The magnifier
//! shows while measuring, for pixel accuracy.
//!
//! The kept measurement is made of existing objects (arrows, a rect, a
//! text), so older builds draw it too.

use super::chrome::{self, Ui};
use crate::objects::{FRect, Obj, Pt};
use crate::raster::Blend;
use crate::text::{AnnotFont, TextFmt};
use crate::uifb::{C4, Fb};

/// A measurement (image px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ruler {
    pub a: Pt,
    pub b: Pt,
    /// A rect (Ctrl) rather than a line.
    pub area: bool,
    /// Released: it waits for Enter (keep) or Esc (discard).
    pub done: bool,
}

/// `b` turned about `a` to the nearest 15° step (its length kept).
pub fn snap15(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    if len == 0.0 {
        return b;
    }
    let step = 15f32.to_radians();
    let ang = (dy.atan2(dx) / step).round() * step;
    Pt::new(a.x + len * ang.cos(), a.y + len * ang.sin())
}

/// What a measurement reads (`scale`: the capture's px per logical px).
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub dx: f32,
    pub dy: f32,
    pub len: f32,
    /// Counterclockwise from the right, -180..=180 (screen y is down).
    pub angle: f32,
    pub scale: f32,
}

impl Ruler {
    pub fn reading(&self, scale: f32) -> Reading {
        let (dx, dy) = ((self.b.x - self.a.x).abs(), (self.b.y - self.a.y).abs());
        let angle = (-(self.b.y - self.a.y)).atan2(self.b.x - self.a.x).to_degrees();
        let angle = if angle == -180.0 { 180.0 } else { angle };
        Reading { dx, dy, len: dx.hypot(dy), angle, scale: if scale.is_finite() && scale > 0.0 { scale } else { 1.0 } }
    }

    pub fn rect(&self) -> FRect {
        FRect::from_pts(self.a, self.b)
    }

    /// Too small to keep (a click).
    pub fn trivial(&self) -> bool {
        (self.b.x - self.a.x).hypot(self.b.y - self.a.y) < 2.0
    }
}

fn n(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r:.1}") }
}

/// Thousands with a thin space ("28 200").
fn grouped(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

/// The label's lines: the length (or the size and area), then the
/// deltas and the angle (`rich`: the font has Δ, ° and ²).
pub fn lines(r: &Ruler, scale: f32, rich: bool) -> Vec<String> {
    let m = r.reading(scale);
    let scaled = (m.scale - 1.0).abs() > 0.01;
    if r.area {
        let (w, h) = (m.dx.round(), m.dy.round());
        let first = if scaled {
            tr!("ruler.size_logical", w = n(w), h = n(h), lw = n(w / m.scale), lh = n(h / m.scale))
        } else {
            tr!("ruler.size", w = n(w), h = n(h))
        };
        let unit = if rich { "px\u{b2}" } else { "sq px" };
        vec![first, tr!("ruler.area", area = grouped((w * h) as u64), unit)]
    } else {
        let first = if scaled { tr!("ruler.length_logical", len = n(m.len), logical = n(m.len / m.scale)) } else { tr!("ruler.length", len = n(m.len)) };
        let (d, deg) = if rich { ("\u{394}", "\u{b0}") } else { ("d", " deg") };
        vec![first, tr!("ruler.deltas", d, dx = n(m.dx), dy = n(m.dy), angle = n(m.angle), deg)]
    }
}

/// The kept annotation's label: the length, or the size.
pub fn short_label(r: &Ruler, scale: f32) -> String {
    let m = r.reading(scale);
    if r.area { tr!("ruler.size", w = n(m.dx.round()), h = n(m.dy.round())) } else { tr!("ruler.length", len = n(m.len)) }
}

/// The measurement as objects: a dimension line (two arrows from the
/// middle out to the ends) or the rect, and its label beside it.
pub fn objects(r: &Ruler, color: C4, width: f32, scale: f32, font: Option<&AnnotFont>) -> Vec<Obj> {
    let width = width.clamp(1.0, 12.0);
    let label = short_label(r, scale);
    let size = (13.0 * scale.max(1.0)).max(width * 4.0).round();
    let (tw, th) = font.map_or((label.chars().count() as f32 * size * 0.55, size * 1.2), |f| {
        let (w, h, _) = f.styled(TextFmt::default()).measure(&label, size);
        (w, h)
    });
    let gap = width * 2.0 + size * 0.4;
    let mut out = Vec::new();
    if r.area {
        let rc = r.rect();
        out.push(Obj::Rect { r: rc, color, width });
        // Above the rect's top edge, centered (below it when there is no room).
        let y = if rc.y - gap - th >= 0.0 { rc.y - gap - th } else { rc.y1() + gap };
        out.push(Obj::Text { pos: Pt::new(rc.x + (rc.w - tw) / 2.0, y), text: label, color, size, fmt: TextFmt::default() });
    } else {
        let mid = Pt::new((r.a.x + r.b.x) / 2.0, (r.a.y + r.b.y) / 2.0);
        out.push(Obj::Arrow { a: mid, b: r.a, color, width });
        out.push(Obj::Arrow { a: mid, b: r.b, color, width });
        // The label's center a little off the line's middle, on the side
        // above it (screen up).
        let (dx, dy) = (r.b.x - r.a.x, r.b.y - r.a.y);
        let len = dx.hypot(dy).max(1e-3);
        let (mut nx, mut ny) = (dy / len, -dx / len);
        if ny > 0.0 || (ny == 0.0 && nx < 0.0) {
            (nx, ny) = (-nx, -ny);
        }
        // Far enough that the box clears the line whatever its slope.
        let off = gap + (tw * nx.abs() + th * ny.abs()) / 2.0;
        let c = Pt::new(mid.x + nx * off, mid.y + ny * off);
        out.push(Obj::Text { pos: Pt::new(c.x - tw / 2.0, c.y - th / 2.0), text: label, color, size, fmt: TextFmt::default() });
    }
    out
}

/// What the overlay paints (capture px).
#[derive(Clone, Debug, PartialEq)]
pub struct RulerUi {
    pub a: Pt,
    pub b: Pt,
    pub area: bool,
    pub lines: Vec<String>,
    /// The keep / discard hint (once released).
    pub hint: bool,
    /// The label's panel.
    pub label: FRect,
}

const LABEL_SIZE: f32 = 12.0;
const SUB_SIZE: f32 = 11.0;
const ROW: f32 = 18.0;

/// The scene's ruler for `r` in `area` (the visible part, capture px).
pub fn scene(ui: &Ui, r: &Ruler, scale: f32, area: FRect) -> RulerUi {
    let rich = ui.font.is_some_and(|f| f.has_glyph('\u{394}') && f.has_glyph('\u{b0}') && f.has_glyph('\u{b2}'));
    let mut lines = lines(r, scale, rich);
    if r.done {
        lines.push(tr!("ruler.hint").to_string());
    }
    let pad = ui.px(8.0);
    let w = lines
        .iter()
        .enumerate()
        .map(|(i, l)| chrome::tw(ui, if i == 0 { LABEL_SIZE } else { SUB_SIZE }, l))
        .fold(0.0, f32::max)
        + 2.0 * pad;
    let h = ui.px(ROW) * lines.len() as f32 + pad;
    // Off the line's middle on its upper side (above a rect), away from
    // the pointer and the magnifier by it; kept inside the area.
    let gap = ui.px(12.0);
    let (x, y) = if r.area {
        let rc = r.rect();
        let y = if rc.y - gap - h >= area.y { rc.y - gap - h } else { rc.y1() + gap };
        (rc.x + (rc.w - w) / 2.0, y)
    } else {
        let mid = Pt::new((r.a.x + r.b.x) / 2.0, (r.a.y + r.b.y) / 2.0);
        let (dx, dy) = (r.b.x - r.a.x, r.b.y - r.a.y);
        let len = dx.hypot(dy).max(1e-3);
        let (mut nx, mut ny) = (dy / len, -dx / len);
        if len < 1.0 {
            (nx, ny) = (0.0, -1.0);
        }
        if ny > 0.0 || (ny == 0.0 && nx < 0.0) {
            (nx, ny) = (-nx, -ny);
        }
        let off = gap + (w * nx.abs() + h * ny.abs()) / 2.0;
        (mid.x + nx * off - w / 2.0, mid.y + ny * off - h / 2.0)
    };
    let x = x.clamp(area.x, (area.x1() - w).max(area.x));
    let y = y.clamp(area.y, (area.y1() - h).max(area.y));
    RulerUi { a: r.a, b: r.b, area: r.area, lines, hint: r.done, label: FRect { x: x.round(), y: y.round(), w: w.ceil(), h: h.ceil() } }
}

fn grow(r: FRect, d: f32) -> FRect {
    FRect { x: r.x - d, y: r.y - d, w: r.w + 2.0 * d, h: r.h + 2.0 * d }
}

/// Bounds of what [`draw`] paints.
pub fn bounds(ui: &Ui, u: &RulerUi) -> Vec<FRect> {
    let tick = ui.px(7.0) + ui.px(3.0);
    vec![grow(FRect::from_pts(u.a, u.b), tick + 2.0), chrome::surface_bounds(ui, u.label)]
}

/// Paint `u` (capture px) into `f`.
pub fn draw(f: &mut Fb, ui: &Ui, u: &RulerUi) {
    let th = ui.th;
    let lw = ui.px(1.5);
    let halo = lw + ui.px(2.0);
    {
        let mut sf = f.surf();
        let stroke = |sf: &mut crate::raster::Surf, pts: &[Pt]| {
            sf.stroke_polyline(pts, halo, th.sel_outer, Blend::Normal);
            sf.stroke_polyline(pts, lw, th.accent, Blend::Normal);
        };
        if u.area {
            let r = FRect::from_pts(u.a, u.b);
            stroke(&mut sf, &[Pt::new(r.x, r.y), Pt::new(r.x1(), r.y), Pt::new(r.x1(), r.y1()), Pt::new(r.x, r.y1()), Pt::new(r.x, r.y)]);
        } else {
            stroke(&mut sf, &[u.a, u.b]);
            // Ticks across both ends.
            let (dx, dy) = (u.b.x - u.a.x, u.b.y - u.a.y);
            let len = dx.hypot(dy);
            if len > 0.5 {
                let t = ui.px(7.0);
                let (nx, ny) = (-dy / len * t, dx / len * t);
                for p in [u.a, u.b] {
                    stroke(&mut sf, &[Pt::new(p.x - nx, p.y - ny), Pt::new(p.x + nx, p.y + ny)]);
                }
            }
        }
    }
    chrome::surface(f, ui, u.label, 6.0, &chrome::SMALL_LAYERS, 1.0);
    let pad = ui.px(8.0);
    for (i, l) in u.lines.iter().enumerate() {
        let cy = u.label.y + pad / 2.0 + ui.px(ROW) * (i as f32 + 0.5);
        let (size, c) = match i {
            0 => (LABEL_SIZE, th.text),
            _ if u.hint && i + 1 == u.lines.len() => (SUB_SIZE, th.text_muted),
            _ => (SUB_SIZE, th.text_muted),
        };
        chrome::text(f, ui, size, l, u.label.x + pad, cy, c);
    }
}

impl super::Edit {
    /// Enter with a measurement up: it becomes objects (one undo step).
    pub(super) fn keep_ruler(&mut self) -> bool {
        let Some(r) = self.ruler.filter(|r| r.done) else { return false };
        self.ruler = None;
        let t = super::Tool::Ruler;
        let objs = objects(&r, self.mem.color(t), self.mem.size(t), self.shot.scale, self.font.as_ref());
        self.objects.extend(objs);
        self.push_history();
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(a: (f32, f32), b: (f32, f32), area: bool) -> Ruler {
        Ruler { a: Pt::new(a.0, a.1), b: Pt::new(b.0, b.1), area, done: false }
    }

    #[test]
    fn readings() {
        let m = r((0.0, 0.0), (30.0, -40.0), false).reading(1.0);
        assert_eq!((m.dx, m.dy, m.len), (30.0, 40.0, 50.0));
        assert!((m.angle - 53.13).abs() < 0.01, "{}", m.angle);
        assert_eq!(r((10.0, 0.0), (0.0, 0.0), false).reading(1.0).angle, 180.0);
        assert_eq!(lines(&r((0.0, 0.0), (30.0, -40.0), false), 1.0, true), vec!["50 px".to_string(), "\u{394}x 30  \u{394}y 40  53.1\u{b0}".to_string()]);
        assert_eq!(lines(&r((0.0, 0.0), (30.0, -40.0), false), 1.0, false)[1], "dx 30  dy 40  53.1 deg");
        // a 200 % capture: logical px too
        assert_eq!(lines(&r((0.0, 0.0), (100.0, 0.0), false), 2.0, true)[0], "100 px  (50 logical)");
        let a = lines(&r((10.0, 10.0), (210.0, 151.0), true), 1.0, true);
        assert_eq!(a, vec!["200 \u{d7} 141 px".to_string(), "Area 28\u{2009}200 px\u{b2}".to_string()]);
        assert!(lines(&r((0.0, 0.0), (20.0, 10.0), true), 1.5, false)[0].contains("logical"));
        assert!(r((1.0, 1.0), (2.0, 2.0), false).trivial());
    }

    #[test]
    fn shift_snaps_to_15_degrees() {
        let a = Pt::new(0.0, 0.0);
        let b = snap15(a, Pt::new(100.0, 8.0));
        assert!((b.y).abs() < 1e-3 && (b.x - 100.32).abs() < 0.01, "{b:?}");
        let b = snap15(a, Pt::new(100.0, 30.0)); // 16.7° -> 15°
        let ang = b.y.atan2(b.x).to_degrees();
        assert!((ang - 15.0).abs() < 1e-3);
        assert!(((b.x).hypot(b.y) - 100f32.hypot(30.0)).abs() < 1e-3, "length kept");
        assert_eq!(snap15(a, a), a);
    }

    #[test]
    fn kept_as_a_dimension_line_or_a_rect() {
        let c = C4::rgb(255, 0, 0);
        let line = objects(&r((0.0, 100.0), (200.0, 100.0), false), c, 3.0, 1.0, None);
        assert_eq!(line.len(), 3);
        assert!(matches!(&line[0], Obj::Arrow { a, b, .. } if *a == Pt::new(100.0, 100.0) && *b == Pt::new(0.0, 100.0)));
        assert!(matches!(&line[1], Obj::Arrow { b, .. } if *b == Pt::new(200.0, 100.0)));
        let Obj::Text { pos, text, .. } = &line[2] else { panic!() };
        assert_eq!(text, "200 px");
        assert!(pos.y < 100.0, "the label is above a level line");
        let rect = objects(&r((10.0, 50.0), (110.0, 100.0), true), c, 2.0, 1.0, None);
        assert!(matches!(&rect[0], Obj::Rect { r, .. } if *r == FRect { x: 10.0, y: 50.0, w: 100.0, h: 50.0 }));
        assert!(matches!(&rect[1], Obj::Text { text, .. } if text == "100 \u{d7} 50 px"));
    }
}
