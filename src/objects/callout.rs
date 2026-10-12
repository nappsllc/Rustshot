//! The callout: a speech bubble (rounded box, cloud or ellipse) with a
//! tail to a tip anywhere, and wrapped text inside. The bubble keeps the
//! width it was drawn at; the text wraps to it and the bubble grows down
//! to hold it ([`needed_h`]). The tail's base sits on the edge nearest the
//! tip. Text goes through the OS text renderer ([`AnnotFont`]), in white or
//! black (whichever reads on the fill), or in the colour on a white body
//! when the callout is not filled.

use super::{counter_ink, FRect, Pt};
use crate::raster::{Blend, Mask, Surf};
use crate::text::AnnotFont;
use crate::uifb::C4;

/// The bubble's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Rounded,
    Cloud,
    Ellipse,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Rounded, Style::Cloud, Style::Ellipse];

    pub fn id(self) -> &'static str {
        match self {
            Style::Rounded => "rounded",
            Style::Cloud => "cloud",
            Style::Ellipse => "ellipse",
        }
    }

    pub fn from_id(s: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|v| v.id() == s)
    }
}

/// Outline width of an unfilled bubble.
pub const OUTLINE: f32 = 3.0;
/// Smallest bubble a drag makes (a click makes [`DEFAULT_W`]).
pub const MIN_W: f32 = 40.0;
pub const DEFAULT_W: f32 = 240.0;

/// Text padding inside the bubble (x, y) at text size `px`.
pub fn pad(px: f32) -> (f32, f32) {
    (px + 4.0, px * 0.8 + 2.0)
}

/// Corner radius of the rounded bubble.
fn radius(r: FRect, px: f32) -> f32 {
    (px * 0.6).clamp(6.0, 16.0).min(r.w / 2.0).min(r.h / 2.0)
}

/// The cloud's bump radius.
fn bump(r: FRect) -> f32 {
    (r.w.min(r.h) * 0.2).clamp(7.0, 24.0)
}

/// Where the text goes inside bubble `r`.
pub fn text_area(r: FRect, style: Style, px: f32) -> FRect {
    let (px_, py) = pad(px);
    let a = match style {
        Style::Rounded => FRect { x: r.x + px_, y: r.y + py, w: r.w - 2.0 * px_, h: r.h - 2.0 * py },
        // The rectangle inscribed in the (core) ellipse, padded a little.
        Style::Cloud | Style::Ellipse => {
            let core = if style == Style::Cloud { inset(r, bump(r) * 0.6) } else { r };
            let (w, h) = (core.w / std::f32::consts::SQRT_2, core.h / std::f32::consts::SQRT_2);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            FRect { x: cx - w / 2.0 + px_ * 0.3, y: cy - h / 2.0 + py * 0.3, w: w - px_ * 0.6, h: h - py * 0.6 }
        }
    };
    FRect { w: a.w.max(1.0), h: a.h.max(1.0), ..a }
}

/// `text` broken into lines that fit `max_w` at `px`: byte ranges into
/// `text` (a hard line break ends a line and is in neither range; a soft
/// break keeps the spaces at the end of the line before it). A word wider
/// than the line is broken between characters.
pub fn wrap(font: &AnnotFont, text: &str, px: f32, max_w: f32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for para in text.split('\n') {
        let end = start + para.len();
        wrap_para(font, text, start, end, px, max_w, &mut out);
        start = end + 1;
    }
    out
}

fn wrap_para(font: &AnnotFont, text: &str, start: usize, end: usize, px: f32, max_w: f32, out: &mut Vec<(usize, usize)>) {
    let fits = |a: usize, b: usize| font.line_width(text[a..b].trim_end(), px) <= max_w;
    let mut a = start;
    loop {
        if a >= end || fits(a, end) {
            out.push((a, end));
            return;
        }
        // The last word break that fits.
        let mut cut = None;
        for (k, ch) in text[a..end].char_indices() {
            let at = a + k;
            if ch == ' ' && at > a {
                if fits(a, at) {
                    cut = Some(at + 1);
                } else {
                    break;
                }
            }
        }
        let b = match cut {
            Some(b) => b,
            None => {
                // One long word: as many characters as fit (at least one).
                let mut b = a + text[a..end].chars().next().map_or(1, char::len_utf8);
                for (k, ch) in text[a..end].char_indices().skip(1) {
                    if fits(a, a + k + ch.len_utf8()) {
                        b = a + k + ch.len_utf8();
                    } else {
                        break;
                    }
                }
                b
            }
        };
        // Spaces right after a soft break stay on the line before.
        let mut b = b;
        while b < end && text.as_bytes()[b] == b' ' {
            b += 1;
        }
        out.push((a, b));
        a = b;
    }
}

/// The text laid out in a bubble: its lines and where they go.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub lines: Vec<(usize, usize)>,
    pub top: Pt,
    pub line_h: f32,
}

pub fn layout(font: &AnnotFont, r: FRect, text: &str, px: f32, style: Style) -> Layout {
    let a = text_area(r, style, px);
    let m = font.metrics(px);
    let lines = wrap(font, text, px, a.w);
    // The cloud and the ellipse centre their text; the box keeps it at
    // the top (the boards' bubble).
    let th = lines.len().max(1) as f32 * m.line_height;
    let y = match style {
        Style::Rounded => a.y,
        _ => a.y + ((a.h - th) / 2.0).max(0.0),
    };
    Layout { lines, top: Pt::new(a.x.round(), y.round()), line_h: m.line_height }
}

/// The bubble height that holds `text` at width `r.w`.
pub fn needed_h(font: Option<&AnnotFont>, r: FRect, text: &str, px: f32, style: Style) -> f32 {
    let Some(font) = font else { return r.h };
    let lines = wrap(font, text, px, text_area(r, style, px).w).len().max(1);
    let th = lines as f32 * font.metrics(px).line_height;
    let (_, py) = pad(px);
    let h = match style {
        Style::Rounded => th + 2.0 * py,
        Style::Cloud => {
            // The bump depends on the height: settle it in two steps.
            let base = (th + py * 0.6) * std::f32::consts::SQRT_2;
            let h0 = base + 1.2 * bump(FRect { h: base, ..r });
            base + 1.2 * bump(FRect { h: h0, ..r })
        }
        Style::Ellipse => (th + py * 0.6) * std::f32::consts::SQRT_2,
    };
    h.ceil().max(px * 2.0)
}

/// The caret before byte `caret`: its top-left (one line tall).
pub fn caret_at(font: &AnnotFont, lay: &Layout, text: &str, caret: usize, px: f32) -> Pt {
    let caret = caret.min(text.len());
    let mut row = 0;
    for (i, (a, b)) in lay.lines.iter().enumerate() {
        if caret >= *a && caret <= *b {
            row = i;
            // At a soft break the caret goes to the next line's start.
            if caret == *b && lay.lines.get(i + 1).is_some_and(|n| n.0 == *b) {
                continue;
            }
            break;
        }
        if caret > *b {
            row = i;
        }
    }
    let Some(&(a, b)) = lay.lines.get(row) else { return lay.top };
    let x = font.caret_x(&text[a..b], caret.clamp(a, b) - a, px);
    Pt::new(lay.top.x + x, lay.top.y + row as f32 * lay.line_h)
}

/// The byte index nearest to `p` (a click in the text).
pub fn caret_near(font: &AnnotFont, lay: &Layout, text: &str, p: Pt, px: f32) -> usize {
    if lay.lines.is_empty() {
        return text.len();
    }
    let row = (((p.y - lay.top.y) / lay.line_h).floor().max(0.0) as usize).min(lay.lines.len() - 1);
    let (a, b) = lay.lines[row];
    let line = &text[a..b];
    let mut best = (f32::MAX, b);
    let mut idxs: Vec<usize> = line.char_indices().map(|(k, _)| k).collect();
    idxs.push(line.len());
    for k in idxs {
        let d = (lay.top.x + font.caret_x(line, k, px) - p.x).abs();
        if d < best.0 {
            best = (d, a + k);
        }
    }
    best.1
}

/// The tail: which way it leaves the bubble and its triangle (the base
/// pushed `deep` inside the body so body and tail join without a seam).
/// `None` when the tip is inside the bubble.
fn tail_triangle(r: FRect, tip: Pt, style: Style, px: f32, deep: f32) -> Option<[Pt; 3]> {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    if hw < 1.0 || hh < 1.0 {
        return None;
    }
    let (dx, dy) = ((tip.x - cx) / hw, (tip.y - cy) / hh);
    match style {
        Style::Ellipse | Style::Cloud => {
            if dx * dx + dy * dy <= 1.0 {
                return None;
            }
            let th = dy.atan2(dx);
            let half = 0.22f32;
            let at = |a: f32, k: f32| Pt::new(cx + hw * k * a.cos(), cy + hh * k * a.sin());
            let k = 1.0 - deep / hw.min(hh);
            Some([at(th - half, k), tip, at(th + half, k)])
        }
        Style::Rounded => {
            if dx.abs() <= 1.0 && dy.abs() <= 1.0 {
                return None;
            }
            let rad = radius(r, px);
            let vertical = dy.abs() >= dx.abs();
            let len = if vertical { r.w } else { r.h };
            let bw = (r.w.min(r.h) * 0.4).clamp(14.0, 44.0).min((len - 2.0 * rad).max(4.0));
            // The base starts beside the tip and leans toward the middle
            // (the boards' tail), clamped clear of the corners.
            if vertical {
                let y = if dy > 0.0 { r.y1() - deep } else { r.y + deep };
                let lean = if tip.x < cx { bw / 2.0 } else { -bw / 2.0 };
                let c = (tip.x + lean).clamp(r.x + rad + bw / 2.0, (r.x1() - rad - bw / 2.0).max(r.x + rad + bw / 2.0));
                Some([Pt::new(c - bw / 2.0, y), tip, Pt::new(c + bw / 2.0, y)])
            } else {
                let x = if dx > 0.0 { r.x1() - deep } else { r.x + deep };
                let lean = if tip.y < cy { bw / 2.0 } else { -bw / 2.0 };
                let c = (tip.y + lean).clamp(r.y + rad + bw / 2.0, (r.y1() - rad - bw / 2.0).max(r.y + rad + bw / 2.0));
                Some([Pt::new(x, c - bw / 2.0), tip, Pt::new(x, c + bw / 2.0)])
            }
        }
    }
}

/// `r` inset by `d` on every side.
fn inset(r: FRect, d: f32) -> FRect {
    FRect { x: r.x + d, y: r.y + d, w: (r.w - 2.0 * d).max(0.0), h: (r.h - 2.0 * d).max(0.0) }
}

/// A triangle shrunk by `d` toward its incentre (each edge moves in by `d`).
fn shrink(t: [Pt; 3], d: f32) -> Option<[Pt; 3]> {
    let l = |a: Pt, b: Pt| (a.x - b.x).hypot(a.y - b.y);
    let (a, b, c) = (l(t[1], t[2]), l(t[0], t[2]), l(t[0], t[1]));
    let p = a + b + c;
    if p < 1e-3 {
        return None;
    }
    let ix = (a * t[0].x + b * t[1].x + c * t[2].x) / p;
    let iy = (a * t[0].y + b * t[1].y + c * t[2].y) / p;
    let s = p / 2.0;
    let area = ((s * (s - a) * (s - b) * (s - c)).max(0.0)).sqrt();
    let inr = area / s;
    if inr <= d {
        return None;
    }
    let k = 1.0 - d / inr;
    Some(t.map(|v| Pt::new(ix + (v.x - ix) * k, iy + (v.y - iy) * k)))
}

/// The cloud: a rounded core inside `r` and bumps along its edge (the
/// bumps reach `r`'s edge), plus `grow` px all round (negative: inset).
fn cloud(m: &mut Mask, r: FRect, grow: f32) {
    let b = bump(r);
    // An elliptic core and bumps round its edge, about 1.4 radii apart:
    // the bumps reach `r`'s edge.
    let core = inset(r, b);
    m.ellipse(inset(core, -grow - b * 0.35));
    let (rx, ry) = (core.w / 2.0, core.h / 2.0);
    let (cx, cy) = (core.x + rx, core.y + ry);
    let per = std::f32::consts::PI * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).max(0.0).sqrt());
    let n = ((per / (b * 1.4)).round() as i32).max(6);
    let rad = (b + grow).max(0.5);
    for k in 0..n {
        let a = std::f32::consts::TAU * k as f32 / n as f32;
        m.disc(Pt::new(cx + rx * a.cos(), cy + ry * a.sin()), rad);
    }
}

/// The cloud's tail: three shrinking puffs from the edge to the tip.
fn cloud_puffs(r: FRect, tip: Pt) -> Option<[(Pt, f32); 3]> {
    let t = tail_triangle(r, tip, Style::Cloud, 0.0, 0.0)?;
    let base = Pt::new((t[0].x + t[2].x) / 2.0, (t[0].y + t[2].y) / 2.0);
    let b = bump(r);
    let at = |k: f32| Pt::new(base.x + (tip.x - base.x) * k, base.y + (tip.y - base.y) * k);
    Some([(at(0.3), b * 0.55), (at(0.65), b * 0.4), (at(1.0), b * 0.28)])
}

/// The bubble's silhouette grown by `grow` (negative: inset), tail
/// included, into `m`.
fn silhouette(m: &mut Mask, r: FRect, tip: Pt, style: Style, px: f32, grow: f32) {
    match style {
        Style::Rounded => {
            m.round_box(inset(r, -grow), (radius(r, px) + grow).max(0.0));
            let deep = OUTLINE * 2.0 + 4.0;
            if let Some(t) = tail_triangle(r, tip, style, px, deep) {
                let t = if grow < 0.0 { shrink(t, -grow) } else { Some(t) };
                if let Some(t) = t {
                    m.convex(&t);
                }
            }
        }
        Style::Ellipse => {
            m.ellipse(inset(r, -grow));
            let deep = OUTLINE * 2.0 + 4.0;
            if let Some(t) = tail_triangle(r, tip, style, px, deep) {
                let t = if grow < 0.0 { shrink(t, -grow) } else { Some(t) };
                if let Some(t) = t {
                    m.convex(&t);
                }
            }
        }
        Style::Cloud => {
            cloud(m, r, grow);
            for (c, rad) in cloud_puffs(r, tip).into_iter().flatten() {
                if rad + grow > 0.5 {
                    m.disc(c, rad + grow);
                }
            }
        }
    }
}

/// Conservative bounds of the bubble and its tail.
pub fn bounds(r: FRect, tip: Pt) -> FRect {
    let (x0, y0) = (r.x.min(tip.x), r.y.min(tip.y));
    let (x1, y1) = (r.x1().max(tip.x), r.y1().max(tip.y));
    let m = OUTLINE + 4.0;
    FRect { x: x0 - m, y: y0 - m, w: x1 - x0 + 2.0 * m, h: y1 - y0 + 2.0 * m }
}

/// Whether `p` is on the bubble or its tail (`tol` px of slack).
pub fn hit(r: FRect, tip: Pt, style: Style, px: f32, p: Pt, tol: f32) -> bool {
    let g = FRect { x: r.x - tol, y: r.y - tol, w: r.w + 2.0 * tol, h: r.h + 2.0 * tol };
    if p.x >= g.x && p.x <= g.x1() && p.y >= g.y && p.y <= g.y1() {
        return true;
    }
    // Rasterise the silhouette around `p` (a few pixels).
    let mut px_buf = [0u8; 4 * 4];
    let (x0, y0) = (p.x.floor() as i32 - 1, p.y.floor() as i32 - 1);
    let sf = Surf::with_origin(&mut px_buf, 2, 2, x0, y0, crate::raster::Order::Rgba);
    let mut m = Mask::new(&sf, x0 - tol.ceil() as i32, y0 - tol.ceil() as i32, x0 + 2 + tol.ceil() as i32, y0 + 2 + tol.ceil() as i32);
    silhouette(&mut m, r, tip, style, px, tol);
    let mut any = false;
    m.each(|_, _, _| any = true);
    any
}

/// Draw the callout.
#[allow(clippy::too_many_arguments)]
pub fn render(sf: &mut Surf, r: FRect, tip: Pt, text: &str, color: C4, px: f32, style: Style, fill: bool, font: Option<&AnnotFont>) {
    if r.w < 1.0 || r.h < 1.0 {
        return;
    }
    let b = bounds(r, tip);
    let win = |sf: &Surf| Mask::new(sf, b.x.floor() as i32, b.y.floor() as i32, b.x1().ceil() as i32, b.y1().ceil() as i32);
    let mut body = win(sf);
    silhouette(&mut body, r, tip, style, px, 0.0);
    let ink = if fill {
        body.blend(sf, color, Blend::Normal);
        counter_ink(color)
    } else {
        // A white body inside a ring of the colour.
        let mut inner = win(sf);
        silhouette(&mut inner, r, tip, style, px, -OUTLINE);
        inner.blend(sf, C4::rgb(255, 255, 255).with_alpha(color.a), Blend::Normal);
        body.cut(&inner);
        body.blend(sf, color, Blend::Normal);
        // The colour on white, unless it is too light to read.
        if counter_ink(color.with_alpha(255)) == C4::rgb(255, 255, 255) { color } else { C4::rgb(0, 0, 0).with_alpha(color.a) }
    };
    let Some(font) = font else { return };
    if text.trim().is_empty() {
        return;
    }
    let lay = layout(font, r, text, px, style);
    let joined: Vec<&str> = lay.lines.iter().map(|&(a, b)| text[a..b].trim_end()).collect();
    font.render(sf, &joined.join("\n"), px, lay.top, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_and_breaks_long_ones() {
        let Some(font) = AnnotFont::load() else { return };
        let t = "Move this to the top, above the summary";
        let w = font.line_width("Move this to the", 17.0) + 2.0;
        let lines = wrap(&font, t, 17.0, w);
        assert!(lines.len() >= 2, "{lines:?}");
        for &(a, b) in &lines {
            assert!(font.line_width(t[a..b].trim_end(), 17.0) <= w, "{:?}", &t[a..b]);
        }
        // Every byte is on a line, in order.
        assert_eq!(lines.first().unwrap().0, 0);
        assert_eq!(lines.last().unwrap().1, t.len());
        for p in lines.windows(2) {
            assert_eq!(p[0].1, p[1].0);
        }
        // Hard breaks, and a word wider than the line.
        let t = "a\nWWWWWWWWWWWWWWWWWWWW";
        let lines = wrap(&font, t, 17.0, font.line_width("WWWW", 17.0) + 1.0);
        assert_eq!(lines[0], (0, 1));
        assert!(lines.len() >= 5, "{lines:?}");
        assert_eq!(lines[1].0, 2);
    }

    #[test]
    fn bubble_grows_down_for_more_text() {
        let Some(font) = AnnotFont::load() else { return };
        let r = FRect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 };
        let one = needed_h(Some(&font), r, "Hi", 17.0, Style::Rounded);
        let many = needed_h(Some(&font), r, "Move this to the top, above the summary, so people see it first", 17.0, Style::Rounded);
        assert!(many > one * 1.5, "{one} {many}");
        for st in Style::ALL {
            assert!(needed_h(Some(&font), r, "Hi\nthere", 17.0, st) > needed_h(Some(&font), r, "Hi", 17.0, st));
        }
    }

    #[test]
    fn caret_follows_lines() {
        let Some(font) = AnnotFont::load() else { return };
        let r = FRect { x: 10.0, y: 10.0, w: 160.0, h: 100.0 };
        let t = "one two three four five six seven";
        let lay = layout(&font, r, t, 16.0, Style::Rounded);
        assert!(lay.lines.len() >= 2);
        let first = caret_at(&font, &lay, t, 0, 16.0);
        assert_eq!(first, lay.top);
        let end = caret_at(&font, &lay, t, t.len(), 16.0);
        assert!(end.y > first.y, "the last line is below");
        // A click near the start of the second line finds its first byte.
        let second = lay.lines[1].0;
        let p = caret_at(&font, &lay, t, second, 16.0);
        assert_eq!(p.x, lay.top.x, "a soft break puts the caret at the next line's start");
        assert_eq!(caret_near(&font, &lay, t, Pt::new(p.x - 3.0, p.y + 4.0), 16.0), second);
    }

    #[test]
    fn tail_base_follows_the_nearest_edge() {
        let r = FRect { x: 100.0, y: 100.0, w: 200.0, h: 80.0 };
        let below = tail_triangle(r, Pt::new(150.0, 260.0), Style::Rounded, 16.0, 0.0).unwrap();
        assert!(below[0].y == r.y1() && below[2].y == r.y1(), "{below:?}");
        let right = tail_triangle(r, Pt::new(420.0, 140.0), Style::Rounded, 16.0, 0.0).unwrap();
        assert!(right[0].x == r.x1(), "{right:?}");
        let above = tail_triangle(r, Pt::new(290.0, 20.0), Style::Rounded, 16.0, 0.0).unwrap();
        assert!(above[0].y == r.y, "{above:?}");
        assert!(tail_triangle(r, Pt::new(150.0, 150.0), Style::Rounded, 16.0, 0.0).is_none(), "tip inside");
        assert!(hit(r, Pt::new(150.0, 260.0), Style::Rounded, 16.0, Pt::new(150.0, 255.0), 1.0), "on the tail");
        assert!(!hit(r, Pt::new(150.0, 260.0), Style::Rounded, 16.0, Pt::new(380.0, 255.0), 1.0));
    }
}
