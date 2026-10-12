//! Editing objects as data: hit tests, selection boxes, moves, resizes
//! (mapping one box onto another), the round handles (end points, tails,
//! the loupe's two centres), and the colour / size / fill a mini-toolbar
//! changes.

use super::{callout, FRect, Obj, Pt};
use crate::text::AnnotFont;
use crate::uifb::C4;

/// A round handle on an object: a line's ends, a tail's tip, the
/// magnifier's source and loupe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    A,
    B,
    Tail,
    Src,
    Dst,
}

/// The range of an object's size (as [`size`] reads it): what the
/// toolbar's steps, a resize and a file read all keep it in.
pub fn size_range(o: &Obj) -> (f32, f32) {
    match o {
        Obj::Text { .. } | Obj::Callout { .. } => (8.0, 96.0),
        Obj::Counter { .. } => (12.0, 120.0),
        Obj::Pixelate { .. } => (4.0, 100.0),
        Obj::Magnify { .. } => (20.0, 300.0),
        _ => (1.0, 50.0),
    }
}

/// How far from the origin a point read from a file may lie.
pub const COORD_MAX: f32 = 1.0e6;

/// `o` made safe to render: sizes in their ranges, the loupe's zoom in
/// 1-8 and its ring in 1-20, rects normalised and at most 2e6 across,
/// points within [`COORD_MAX`] (the file reader's last step).
pub fn sanitize(o: &mut Obj) {
    let c = |v: &mut f32| *v = if v.is_finite() { v.clamp(-COORD_MAX, COORD_MAX) } else { 0.0 };
    let cp = |p: &mut Pt| {
        c(&mut p.x);
        c(&mut p.y);
    };
    let cr = |r: &mut FRect| {
        let ok = |v: f32| v.is_finite() && v.abs() <= COORD_MAX;
        if [r.x, r.y, r.x + r.w, r.y + r.h].into_iter().all(ok) && r.w >= 0.0 && r.h >= 0.0 {
            return; // already sane: kept exactly
        }
        let (mut a, mut b) = (Pt::new(r.x, r.y), Pt::new(r.x + r.w, r.y + r.h));
        cp(&mut a);
        cp(&mut b);
        *r = FRect::from_pts(a, b);
    };
    match o {
        Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } => {
            cp(a);
            cp(b);
        }
        Obj::Path { pts, .. } | Obj::MarkerPath { pts, .. } => pts.iter_mut().for_each(cp),
        Obj::Rect { r, .. }
        | Obj::Ellipse { r, .. }
        | Obj::RectFill { r, .. }
        | Obj::EllipseFill { r, .. }
        | Obj::Pixelate { r, .. }
        | Obj::Invert { r }
        | Obj::Spotlight { r } => cr(r),
        Obj::Text { pos, .. } => cp(pos),
        Obj::Counter { c, tail, .. } => {
            cp(c);
            if let Some(t) = tail {
                cp(t);
            }
        }
        Obj::Callout { r, tail, .. } => {
            cr(r);
            cp(tail);
        }
        Obj::Magnify { src, dst, zoom, width, .. } => {
            cp(src);
            cp(dst);
            *zoom = if zoom.is_finite() { zoom.clamp(1.0, 8.0) } else { 2.0 };
            *width = if width.is_finite() { width.clamp(1.0, 20.0) } else { 3.0 };
        }
    }
    if let Some(v) = size(o) {
        let (lo, hi) = size_range(o);
        set_size(o, if v.is_finite() { v.clamp(lo, hi) } else { lo });
    }
}

/// The box handles an object gets when it is the only one selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boxes {
    /// No box handles (lines, arrows, the magnifier: round ones only).
    None,
    /// The four corners (text, tags: they scale).
    Corners,
    /// Corners and sides.
    All,
}

fn dist_seg(p: Pt, a: Pt, b: Pt) -> f32 {
    let (vx, vy) = (b.x - a.x, b.y - a.y);
    let (wx, wy) = (p.x - a.x, p.y - a.y);
    let len2 = vx * vx + vy * vy;
    let t = if len2 <= 1e-12 { 0.0 } else { ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0) };
    (wx - t * vx).hypot(wy - t * vy)
}

fn bbox(pts: &[Pt]) -> Option<FRect> {
    let f = pts.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (f.x, f.y, f.x, f.y);
    for p in pts {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    Some(FRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

fn inside(r: FRect, p: Pt, tol: f32) -> bool {
    p.x >= r.x - tol && p.x <= r.x1() + tol && p.y >= r.y - tol && p.y <= r.y1() + tol
}

/// The box the selection chrome draws round `o` (its geometry, no stroke
/// or AA margins); `None` for an object with no extent.
pub fn sel_box(o: &Obj, font: Option<&AnnotFont>) -> Option<FRect> {
    match o {
        Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } => bbox(&[*a, *b]),
        Obj::Path { pts, .. } | Obj::MarkerPath { pts, .. } => bbox(pts),
        Obj::Rect { r, .. }
        | Obj::Ellipse { r, .. }
        | Obj::RectFill { r, .. }
        | Obj::EllipseFill { r, .. }
        | Obj::Pixelate { r, .. }
        | Obj::Invert { r }
        | Obj::Spotlight { r }
        | Obj::Callout { r, .. } => Some(*r),
        Obj::Text { pos, text, size, .. } => {
            let (w, h) = font.map_or((size * text.len() as f32 * 0.5, size * 1.2), |f| {
                let (w, h, _) = f.measure(text, *size);
                (w, h)
            });
            Some(FRect { x: pos.x, y: pos.y, w: w.max(1.0), h: h.max(1.0) })
        }
        Obj::Counter { c, d, .. } => Some(FRect { x: c.x - d / 2.0, y: c.y - d / 2.0, w: *d, h: *d }),
        Obj::Magnify { dst, radius, .. } => Some(FRect { x: dst.x - radius, y: dst.y - radius, w: 2.0 * radius, h: 2.0 * radius }),
    }
}

/// `p` is on `o` (within `tol` image px of a stroke; anywhere inside a
/// filled shape, an area effect, a text, a bubble or the loupe).
pub fn hit(o: &Obj, p: Pt, tol: f32, font: Option<&AnnotFont>) -> bool {
    match o {
        Obj::Line { a, b, width, .. } | Obj::Arrow { a, b, width, .. } | Obj::Marker { a, b, width, .. } => {
            dist_seg(p, *a, *b) <= width / 2.0 + tol
        }
        Obj::Path { pts, width, .. } | Obj::MarkerPath { pts, width, .. } => {
            pts.windows(2).any(|s| dist_seg(p, s[0], s[1]) <= width / 2.0 + tol)
                || (pts.len() == 1 && dist_seg(p, pts[0], pts[0]) <= width / 2.0 + tol)
        }
        Obj::Rect { r, width, .. } => {
            let m = width / 2.0 + tol;
            inside(*r, p, m) && !inside(FRect { x: r.x + m, y: r.y + m, w: r.w - 2.0 * m, h: r.h - 2.0 * m }, p, 0.0)
        }
        Obj::Ellipse { r, width, .. } => {
            let (rx, ry) = (r.w / 2.0, r.h / 2.0);
            let (cx, cy) = (r.x + rx, r.y + ry);
            let m = width / 2.0 + tol;
            let k = |dx: f32, dy: f32| ((p.x - cx) / dx.max(0.5)).powi(2) + ((p.y - cy) / dy.max(0.5)).powi(2);
            k(rx + m, ry + m) <= 1.0 && k((rx - m).max(0.5), (ry - m).max(0.5)) >= 1.0
        }
        Obj::EllipseFill { r, width, .. } => {
            let (rx, ry) = (r.w / 2.0 + width / 2.0 + tol, r.h / 2.0 + width / 2.0 + tol);
            ((p.x - r.x - r.w / 2.0) / rx.max(0.5)).powi(2) + ((p.y - r.y - r.h / 2.0) / ry.max(0.5)).powi(2) <= 1.0
        }
        Obj::RectFill { r, width, .. } => inside(*r, p, width / 2.0 + tol),
        Obj::Pixelate { r, .. } | Obj::Invert { r } => inside(*r, p, tol),
        // On its edge only: what it keeps bright stays pickable.
        Obj::Spotlight { r } => {
            let m = tol + 2.0;
            inside(*r, p, m) && !inside(FRect { x: r.x + m, y: r.y + m, w: r.w - 2.0 * m, h: r.h - 2.0 * m }, p, 0.0)
        }
        Obj::Text { .. } => sel_box(o, font).is_some_and(|b| inside(b, p, tol)),
        Obj::Counter { c, d, tail, .. } => {
            (p.x - c.x).hypot(p.y - c.y) <= d / 2.0 + tol || tail.is_some_and(|t| dist_seg(p, *c, t) <= tol + 2.0)
        }
        Obj::Callout { r, tail, size, style, .. } => callout::hit(*r, *tail, *style, *size, p, tol),
        Obj::Magnify { src, dst, radius, zoom, .. } => {
            (p.x - dst.x).hypot(p.y - dst.y) <= radius + tol || (p.x - src.x).hypot(p.y - src.y) <= radius / zoom.max(1.0) + tol
        }
    }
}

/// Which box handles `o` gets alone.
pub fn boxes(o: &Obj) -> Boxes {
    match o {
        Obj::Line { .. } | Obj::Arrow { .. } | Obj::Marker { .. } | Obj::Magnify { .. } => Boxes::None,
        Obj::Text { .. } | Obj::Counter { .. } => Boxes::Corners,
        _ => Boxes::All,
    }
}

/// The round handles of `o`.
pub fn points(o: &Obj) -> Vec<(Point, Pt)> {
    match o {
        Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } => vec![(Point::A, *a), (Point::B, *b)],
        Obj::Counter { tail: Some(t), .. } => vec![(Point::Tail, *t)],
        Obj::Callout { tail, .. } => vec![(Point::Tail, *tail)],
        Obj::Magnify { src, dst, .. } => vec![(Point::Src, *src), (Point::Dst, *dst)],
        _ => Vec::new(),
    }
}

/// Move round handle `which` of `o` to `p`.
pub fn set_point(o: &mut Obj, which: Point, p: Pt) {
    match (o, which) {
        (Obj::Line { a, .. } | Obj::Arrow { a, .. } | Obj::Marker { a, .. }, Point::A) => *a = p,
        (Obj::Line { b, .. } | Obj::Arrow { b, .. } | Obj::Marker { b, .. }, Point::B) => *b = p,
        (Obj::Counter { tail, .. }, Point::Tail) => *tail = Some(p),
        (Obj::Callout { tail, .. }, Point::Tail) => *tail = p,
        (Obj::Magnify { src, .. }, Point::Src) => *src = p,
        (Obj::Magnify { dst, .. }, Point::Dst) => *dst = p,
        _ => {}
    }
}

fn mv(p: &mut Pt, dx: f32, dy: f32) {
    p.x += dx;
    p.y += dy;
}

/// Move `o` by (`dx`, `dy`).
pub fn translate(o: &mut Obj, dx: f32, dy: f32) {
    match o {
        Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } => {
            mv(a, dx, dy);
            mv(b, dx, dy);
        }
        Obj::Path { pts, .. } | Obj::MarkerPath { pts, .. } => pts.iter_mut().for_each(|p| mv(p, dx, dy)),
        Obj::Rect { r, .. }
        | Obj::Ellipse { r, .. }
        | Obj::RectFill { r, .. }
        | Obj::EllipseFill { r, .. }
        | Obj::Pixelate { r, .. }
        | Obj::Invert { r }
        | Obj::Spotlight { r } => {
            r.x += dx;
            r.y += dy;
        }
        Obj::Text { pos, .. } => mv(pos, dx, dy),
        Obj::Counter { c, tail, .. } => {
            mv(c, dx, dy);
            if let Some(t) = tail {
                mv(t, dx, dy);
            }
        }
        Obj::Callout { r, tail, .. } => {
            r.x += dx;
            r.y += dy;
            mv(tail, dx, dy);
        }
        Obj::Magnify { src, dst, .. } => {
            mv(src, dx, dy);
            mv(dst, dx, dy);
        }
    }
}

/// Scale `o` from box `from` onto box `to` (a resize): points map
/// linearly; a text's size follows the height, a tag's diameter and a
/// loupe's radius the mean scale; strokes keep their width, a callout its
/// text size (it re-wraps) and its tail's tip.
pub fn map_box(o: &mut Obj, from: FRect, to: FRect) {
    let sx = if from.w.abs() > 1e-3 { to.w / from.w } else { 1.0 };
    let sy = if from.h.abs() > 1e-3 { to.h / from.h } else { 1.0 };
    let m = |p: &mut Pt| {
        p.x = to.x + (p.x - from.x) * sx;
        p.y = to.y + (p.y - from.y) * sy;
    };
    let mr = |r: &mut FRect| {
        let (mut a, mut b) = (Pt::new(r.x, r.y), Pt::new(r.x1(), r.y1()));
        m(&mut a);
        m(&mut b);
        *r = FRect::from_pts(a, b);
    };
    let k = ((sx.abs() + sy.abs()) / 2.0).max(0.01);
    match o {
        Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } => {
            m(a);
            m(b);
        }
        Obj::Path { pts, .. } | Obj::MarkerPath { pts, .. } => pts.iter_mut().for_each(m),
        Obj::Rect { r, .. }
        | Obj::Ellipse { r, .. }
        | Obj::RectFill { r, .. }
        | Obj::EllipseFill { r, .. }
        | Obj::Pixelate { r, .. }
        | Obj::Invert { r }
        | Obj::Spotlight { r } => mr(r),
        Obj::Text { pos, size, .. } => {
            m(pos);
            *size = (*size * sy.abs()).clamp(8.0, 96.0);
        }
        Obj::Counter { c, d, tail, .. } => {
            m(c);
            *d = (*d * k).clamp(12.0, 120.0);
            if let Some(t) = tail {
                m(t);
            }
        }
        // The tail's tip stays where it points: resizing the bubble
        // never re-aims it.
        Obj::Callout { r, .. } => mr(r),
        Obj::Magnify { src, dst, radius, .. } => {
            m(src);
            m(dst);
            *radius = (*radius * k).clamp(20.0, 300.0);
        }
    }
}

/// The colour `o` draws in (`None`: pixelate, invert, spotlight).
pub fn color(o: &Obj) -> Option<C4> {
    match o {
        Obj::Line { color, .. }
        | Obj::Arrow { color, .. }
        | Obj::Rect { color, .. }
        | Obj::Ellipse { color, .. }
        | Obj::Path { color, .. }
        | Obj::Marker { color, .. }
        | Obj::Text { color, .. }
        | Obj::MarkerPath { color, .. }
        | Obj::RectFill { color, .. }
        | Obj::EllipseFill { color, .. }
        | Obj::Counter { color, .. }
        | Obj::Callout { color, .. }
        | Obj::Magnify { color, .. } => Some(*color),
        Obj::Pixelate { .. } | Obj::Invert { .. } | Obj::Spotlight { .. } => None,
    }
}

/// Set the colour (the highlighter keeps its own translucency).
pub fn set_color(o: &mut Obj, c: C4) {
    match o {
        Obj::Marker { color, .. } | Obj::MarkerPath { color, .. } => *color = c.with_alpha(((c.a as u32 * 90 + 127) / 255) as u8),
        Obj::Line { color, .. }
        | Obj::Arrow { color, .. }
        | Obj::Rect { color, .. }
        | Obj::Ellipse { color, .. }
        | Obj::Path { color, .. }
        | Obj::Text { color, .. }
        | Obj::RectFill { color, .. }
        | Obj::EllipseFill { color, .. }
        | Obj::Counter { color, .. }
        | Obj::Callout { color, .. }
        | Obj::Magnify { color, .. } => *color = c,
        Obj::Pixelate { .. } | Obj::Invert { .. } | Obj::Spotlight { .. } => {}
    }
}

/// The size the mini-toolbar steps: stroke width, text size, tag
/// diameter, pixel cell, loupe radius (`None`: invert, spotlight).
pub fn size(o: &Obj) -> Option<f32> {
    match o {
        Obj::Line { width, .. }
        | Obj::Arrow { width, .. }
        | Obj::Rect { width, .. }
        | Obj::Ellipse { width, .. }
        | Obj::Path { width, .. }
        | Obj::Marker { width, .. }
        | Obj::MarkerPath { width, .. }
        | Obj::RectFill { width, .. }
        | Obj::EllipseFill { width, .. } => Some(*width),
        Obj::Text { size, .. } | Obj::Callout { size, .. } => Some(*size),
        Obj::Counter { d, .. } => Some(*d),
        Obj::Pixelate { cell, .. } => Some(*cell),
        Obj::Magnify { radius, .. } => Some(*radius),
        Obj::Invert { .. } | Obj::Spotlight { .. } => None,
    }
}

pub fn set_size(o: &mut Obj, v: f32) {
    match o {
        Obj::Line { width, .. }
        | Obj::Arrow { width, .. }
        | Obj::Rect { width, .. }
        | Obj::Ellipse { width, .. }
        | Obj::Path { width, .. }
        | Obj::Marker { width, .. }
        | Obj::MarkerPath { width, .. }
        | Obj::RectFill { width, .. }
        | Obj::EllipseFill { width, .. } => *width = v,
        Obj::Text { size, .. } | Obj::Callout { size, .. } => *size = v,
        Obj::Counter { d, .. } => *d = v,
        Obj::Pixelate { cell, .. } => *cell = v,
        Obj::Magnify { radius, .. } => *radius = v,
        Obj::Invert { .. } | Obj::Spotlight { .. } => {}
    }
}

/// Filled or outlined (`None`: no fill toggle).
pub fn fill(o: &Obj) -> Option<bool> {
    match o {
        Obj::Rect { .. } | Obj::Ellipse { .. } => Some(false),
        Obj::RectFill { .. } | Obj::EllipseFill { .. } => Some(true),
        Obj::Callout { fill, .. } => Some(*fill),
        _ => None,
    }
}

/// Fill or outline the shape (a rectangle / ellipse becomes the other
/// variant, keeping its geometry, colour and width).
pub fn set_fill(o: &mut Obj, on: bool) {
    let next = match &*o {
        Obj::Rect { r, color, width } if on => Obj::RectFill { r: *r, color: *color, width: *width },
        Obj::RectFill { r, color, width } if !on => Obj::Rect { r: *r, color: *color, width: *width },
        Obj::Ellipse { r, color, width } if on => Obj::EllipseFill { r: *r, color: *color, width: *width },
        Obj::EllipseFill { r, color, width } if !on => Obj::Ellipse { r: *r, color: *color, width: *width },
        Obj::Callout { .. } => {
            if let Obj::Callout { fill, .. } = o {
                *fill = on;
            }
            return;
        }
        _ => return,
    };
    *o = next;
}

/// `o` at `k` of its opacity (the ghost left at a drag's origin); `None`
/// for effects that have no colour of their own.
pub fn faded(o: &Obj, k: f32) -> Option<Obj> {
    let c = color(o)?;
    let mut g = o.clone();
    // Directly: `set_color` would re-derive the highlighter's alpha.
    match &mut g {
        Obj::Line { color, .. }
        | Obj::Arrow { color, .. }
        | Obj::Rect { color, .. }
        | Obj::Ellipse { color, .. }
        | Obj::Path { color, .. }
        | Obj::Marker { color, .. }
        | Obj::Text { color, .. }
        | Obj::MarkerPath { color, .. }
        | Obj::RectFill { color, .. }
        | Obj::EllipseFill { color, .. }
        | Obj::Counter { color, .. }
        | Obj::Callout { color, .. } => *color = c.fade(k),
        // A loupe's content is the image: its rings only.
        Obj::Magnify { .. } => return None,
        _ => return None,
    }
    Some(g)
}

/// The union of boxes.
pub fn union(boxes: impl IntoIterator<Item = FRect>) -> Option<FRect> {
    boxes.into_iter().reduce(|a, b| {
        let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
        FRect { x: x0, y: y0, w: a.x1().max(b.x1()) - x0, h: a.y1().max(b.y1()) - y0 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> C4 {
        C4::rgb(240, 68, 56)
    }

    #[test]
    fn hits_strokes_near_and_shapes_inside() {
        let line = Obj::Arrow { a: Pt::new(0.0, 0.0), b: Pt::new(100.0, 0.0), color: red(), width: 4.0 };
        assert!(hit(&line, Pt::new(50.0, 5.0), 4.0, None));
        assert!(!hit(&line, Pt::new(50.0, 12.0), 4.0, None));
        let r = FRect { x: 10.0, y: 10.0, w: 100.0, h: 60.0 };
        let outline = Obj::Rect { r, color: red(), width: 3.0 };
        assert!(hit(&outline, Pt::new(11.0, 40.0), 3.0, None), "on the edge");
        assert!(!hit(&outline, Pt::new(60.0, 40.0), 3.0, None), "an outline is hollow");
        let mut filled = outline.clone();
        set_fill(&mut filled, true);
        assert!(matches!(filled, Obj::RectFill { .. }));
        assert!(hit(&filled, Pt::new(60.0, 40.0), 3.0, None));
        let el = Obj::Ellipse { r, color: red(), width: 3.0 };
        assert!(hit(&el, Pt::new(60.0, 10.5), 3.0, None) && !hit(&el, Pt::new(60.0, 40.0), 3.0, None));
        let pix = Obj::Pixelate { r, cell: 10.0 };
        assert!(hit(&pix, Pt::new(60.0, 40.0), 0.0, None));
        let mag = Obj::Magnify { src: Pt::new(0.0, 0.0), dst: Pt::new(200.0, 0.0), radius: 50.0, zoom: 2.0, color: red(), width: 3.0 };
        assert!(hit(&mag, Pt::new(200.0, 40.0), 0.0, None) && hit(&mag, Pt::new(20.0, 0.0), 0.0, None));
        assert!(!hit(&mag, Pt::new(100.0, 0.0), 0.0, None));
    }

    #[test]
    fn move_and_resize_map_geometry() {
        let mut o = Obj::Callout {
            r: FRect { x: 10.0, y: 10.0, w: 100.0, h: 50.0 },
            tail: Pt::new(30.0, 90.0),
            text: "hi".into(),
            color: red(),
            size: 16.0,
            style: super::super::CalloutStyle::Cloud,
            fill: true,
        };
        translate(&mut o, 5.0, -5.0);
        assert_eq!(sel_box(&o, None), Some(FRect { x: 15.0, y: 5.0, w: 100.0, h: 50.0 }));
        let from = sel_box(&o, None).unwrap();
        map_box(&mut o, from, FRect { x: 15.0, y: 5.0, w: 200.0, h: 50.0 });
        let Obj::Callout { r, tail, size: px, .. } = &o else { unreachable!() };
        assert_eq!((r.w, *px), (200.0, 16.0), "wider, same text size");
        assert_eq!(*tail, Pt::new(15.0 + 20.0, 85.0), "the tip stays");
        let mut t = Obj::Text { pos: Pt::new(0.0, 0.0), text: "x".into(), color: red(), size: 20.0 };
        map_box(&mut t, FRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, FRect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 });
        assert_eq!(size(&t), Some(40.0));
    }

    #[test]
    fn colour_size_points() {
        let mut m = Obj::Marker { a: Pt::new(0.0, 0.0), b: Pt::new(9.0, 9.0), color: C4::rgb(250, 220, 0).with_alpha(90), width: 12.0 };
        set_color(&mut m, C4::rgb(0, 0, 255));
        assert_eq!(color(&m), Some(C4::new(0, 0, 255, 90)), "the highlighter stays translucent");
        set_size(&mut m, 20.0);
        assert_eq!(size(&m), Some(20.0));
        set_point(&mut m, Point::B, Pt::new(50.0, 0.0));
        assert_eq!(points(&m)[1], (Point::B, Pt::new(50.0, 0.0)));
        assert_eq!(boxes(&m), Boxes::None);
        assert_eq!(color(&Obj::Spotlight { r: FRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 } }), None);
        let g = faded(&Obj::Line { a: Pt::new(0.0, 0.0), b: Pt::new(1.0, 1.0), color: red(), width: 1.0 }, 0.4).unwrap();
        assert_eq!(color(&g).unwrap().a, 102);
    }
}
