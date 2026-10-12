//! Annotation objects: retained data (geometry, color, size; z-order is
//! the list order), rendered from that data. [`json`] writes and reads
//! them (`annotations.json`), [`edit`] hit-tests and transforms them for
//! the select tool, [`callout`] lays out speech bubbles.

pub mod blur;
pub mod callout;
pub mod edit;
#[cfg(test)]
mod v020;
// The History / Edit window reads and writes annotations.json next.
#[allow(dead_code)]
pub mod json;

pub use callout::Style as CalloutStyle;
#[allow(unused_imports)]
pub use json::{from_json, to_json};

use crate::text::AnnotFont;
pub use crate::text::TextFmt;
use crate::pixbuf::PixBuf;
use crate::raster::{Blend, Mask, Surf};
use crate::uifb::C4;

/// The annotation object (the editor's name for it is `Obj`).
#[allow(dead_code)]
pub type Object = Obj;

/// Spotlights dim everything else in the selection by this alpha of black.
pub const SPOT_ALPHA: u8 = 128;

/// One channel dimmed by a spotlight: `(v * (255 - a) + 127) / 255`, the
/// overlay's dimmer formula toward black.
#[inline]
pub fn spot_dim(v: u8) -> u8 {
    ((v as u32 * (255 - SPOT_ALPHA as u32) + 127) / 255) as u8
}

/// A spotlight's pixel rect (`x0, y0, x1, y1`, end-exclusive; edges
/// rounded): the pixels it keeps bright.
pub fn spot_px(r: FRect) -> (i32, i32, i32, i32) {
    (r.x.round() as i32, r.y.round() as i32, r.x1().round() as i32, r.y1().round() as i32)
}

/// The spotlights of `objs` (their rects, in order).
pub fn spots(objs: &[Obj]) -> impl Iterator<Item = FRect> + '_ {
    objs.iter().filter_map(|o| match o {
        Obj::Spotlight { r } => Some(*r),
        _ => None,
    })
}

/// A pixel rect `(x0, y0, x1, y1)`, end-exclusive.
pub type Px = (i32, i32, i32, i32);

/// The spotlight pass of a bake: after the objects below the topmost
/// spotlight (`at`), every pixel inside `area` (the capture's selection)
/// and outside all `spots` is dimmed; the objects above it stay bright.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotPass {
    pub at: usize,
    pub area: Px,
    pub spots: Vec<Px>,
}

impl SpotPass {
    /// The pass of `objs` over `area`; `None` without spotlights.
    pub fn of(objs: &[Obj], area: Px) -> Option<SpotPass> {
        let at = objs.iter().rposition(Obj::is_spot)?;
        Some(SpotPass { at, area, spots: spots(objs).map(spot_px).collect() })
    }

    /// Dim the surface's pixels inside the area and outside every spot.
    pub fn apply(&self, sf: &mut Surf) {
        let (bx0, by0, bx1, by1) = sf.bounds();
        let (x0, y0) = (self.area.0.max(bx0), self.area.1.max(by0));
        let (x1, y1) = (self.area.2.min(bx1), self.area.3.min(by1));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let w = (bx1 - bx0) as usize;
        let ch = sf.order().rgb_idx();
        for y in y0..y1 {
            let row = &mut sf.data[(y - by0) as usize * w * 4..][..w * 4];
            // The spots crossing this row, as x spans.
            let mut x = x0;
            while x < x1 {
                let lit = self.spots.iter().filter(|s| y >= s.1 && y < s.3 && x >= s.0 && x < s.2).map(|s| s.2).max();
                if let Some(e) = lit {
                    x = e;
                    continue;
                }
                // Dim up to the next spot starting on this row.
                let next = self.spots.iter().filter(|s| y >= s.1 && y < s.3 && s.0 > x).map(|s| s.0).min().unwrap_or(x1).min(x1);
                for p in row[(x - bx0) as usize * 4..(next - bx0) as usize * 4].as_chunks_mut::<4>().0 {
                    for c in ch {
                        p[c] = spot_dim(p[c]);
                    }
                }
                x = next;
            }
        }
    }
}

/// Render `objs` in order into `buf`, the spotlight pass at the topmost
/// spotlight (over the whole image): the image as exported.
#[allow(dead_code)]
pub fn render_all(buf: &mut PixBuf, objs: &[Obj], font: Option<&AnnotFont>) {
    let area = (0, 0, buf.width() as i32, buf.height() as i32);
    let pass = SpotPass::of(objs, area);
    let mut sf = Surf::from_buf(buf);
    for (j, o) in objs.iter().enumerate() {
        o.render_into(&mut sf, font);
        if pass.as_ref().is_some_and(|p| p.at == j) {
            pass.as_ref().unwrap().apply(&mut sf);
        }
    }
}

/// A point in image (physical pixel) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A rectangle in image coordinates, normalized (w/h >= 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl FRect {
    pub fn from_pts(a: Pt, b: Pt) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }
    pub fn x1(&self) -> f32 {
        self.x + self.w
    }
    pub fn y1(&self) -> f32 {
        self.y + self.h
    }
    pub fn is_trivial(&self) -> bool {
        self.w < 1.0 || self.h < 1.0
    }
    pub fn clamp_to(&mut self, w: f32, h: f32) {
        self.w = self.w.min(w);
        self.h = self.h.min(h);
        self.x = self.x.clamp(0.0, (w - self.w).max(0.0));
        self.y = self.y.clamp(0.0, (h - self.h).max(0.0));
    }
}

/// A pixelate object's cell grid (see [`Obj::pixel_grid`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellGrid {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub c: i32,
}

impl CellGrid {
    /// `v` moved down to a cell edge where it falls inside `[a0, a1)`.
    fn floor(v: i32, a0: i32, a1: i32, c: i32) -> i32 {
        if v > a0 && v < a1 { a0 + (v - a0) / c * c } else { v }
    }

    /// `v` moved up to a cell edge (or `a1`) where it falls inside.
    fn ceil(v: i32, a0: i32, a1: i32, c: i32) -> i32 {
        if v > a0 && v < a1 { (a0 + (v - a0 + c - 1) / c * c).min(a1) } else { v }
    }

    /// Window `(x0, y0, x1, y1)` grown to whole cells where it cuts the
    /// pixelated area, with the grid clipped to `w` x `h`.
    pub fn align(&self, r: (i32, i32, i32, i32), w: i32, h: i32) -> (i32, i32, i32, i32) {
        let (gx1, gy1, c) = (self.x1.min(w), self.y1.min(h), self.c);
        (
            Self::floor(r.0, self.x0, gx1, c),
            Self::floor(r.1, self.y0, gy1, c),
            Self::ceil(r.2, self.x0, gx1, c),
            Self::ceil(r.3, self.y0, gy1, c),
        )
    }

    /// Next row at or after `y` that is a cell edge (or the area's
    /// bottom), for `y` inside the area; `y` itself otherwise.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn row_edge(&self, y: i32, h: i32) -> i32 {
        Self::ceil(y, self.y0, self.y1.min(h), self.c)
    }

    /// The cells two grids with this origin and cell size both render
    /// identically (whole cells inside both areas), as `(x0, y0, x1, y1)`
    /// clipped to `w` x `h`; `None` when they differ in origin or cell.
    pub fn stable_with(&self, o: &CellGrid, w: i32, h: i32) -> Option<(i32, i32, i32, i32)> {
        if (self.x0, self.y0, self.c) != (o.x0, o.y0, o.c) {
            return None;
        }
        let c = self.c;
        let whole = |a0: i32, a1: i32| a0 + (a1 - a0).max(0) / c * c;
        let x1 = whole(self.x0, self.x1.min(o.x1).min(w));
        let y1 = whole(self.y0, self.y1.min(o.y1).min(h));
        Some((self.x0, self.y0, x1, y1))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    Line {
        a: Pt,
        b: Pt,
        color: C4,
        width: f32,
    },
    Arrow {
        a: Pt,
        b: Pt,
        color: C4,
        width: f32,
    },
    Rect {
        r: FRect,
        color: C4,
        width: f32,
    },
    Ellipse {
        r: FRect,
        color: C4,
        width: f32,
    },
    Path {
        pts: Vec<Pt>,
        color: C4,
        width: f32,
    },
    Marker {
        a: Pt,
        b: Pt,
        color: C4,
        width: f32,
    },
    Text {
        pos: Pt,
        text: String,
        color: C4,
        size: f32,
        /// Family, bold / italic / underline / strike, alignment.
        fmt: TextFmt,
    },
    Pixelate {
        r: FRect,
        cell: f32,
    },
    Invert {
        r: FRect,
    },
    /// A freehand highlighter stroke (the straight one is `Marker`).
    MarkerPath {
        pts: Vec<Pt>,
        color: C4,
        width: f32,
    },
    /// A filled rectangle: the outline's silhouette (`r` grown by half
    /// the width, the same rounded corners), in the color with its alpha.
    RectFill {
        r: FRect,
        color: C4,
        width: f32,
    },
    /// A filled ellipse: the outline's silhouette (the inside of `r` and
    /// the ring of the width).
    EllipseFill {
        r: FRect,
        color: C4,
        width: f32,
    },
    /// A numbered tag: a disc of diameter `d` centred on `c` with `n`
    /// inside (black or white, whichever reads), optionally a pointer tail
    /// to `tail`.
    Counter {
        c: Pt,
        n: u32,
        color: C4,
        d: f32,
        tail: Option<Pt>,
    },
    /// A speech bubble `r` with a tail to `tail` and wrapped `text` at
    /// `size` px ([`callout`]).
    Callout {
        r: FRect,
        tail: Pt,
        text: String,
        color: C4,
        size: f32,
        style: CalloutStyle,
        fill: bool,
        /// The text's family and styles (its alignment is unused).
        fmt: TextFmt,
    },
    /// Keeps `r` bright and dims everything else in the selection (all
    /// spotlights together: their union stays bright). Draws no pixels of
    /// its own: the dim is a pass over what is below the topmost one
    /// ([`SpotPass`]).
    Spotlight {
        r: FRect,
    },
    /// A loupe of `radius` at `dst` showing `zoom` x the area around `src`
    /// (radius / zoom), with a ring round both and a connector.
    Magnify {
        src: Pt,
        dst: Pt,
        radius: f32,
        zoom: f32,
        color: C4,
        width: f32,
    },
    /// A Gaussian-like blur of the pixels in `r` (three box passes each
    /// way approximating a Gaussian of standard deviation `sigma`; the
    /// rect's own pixels only, its edges clamped).
    Blur {
        r: FRect,
        sigma: f32,
    },
}

/// `r` grown by `m` on every side.
fn grown(r: FRect, m: f32) -> FRect {
    FRect { x: r.x - m, y: r.y - m, w: r.w + 2.0 * m, h: r.h + 2.0 * m }
}

/// A numbered tag's tail: a triangle from the disc's edge (32 degrees
/// either side of the tip's direction) to the tip; `None` when the tip is
/// inside the disc.
fn counter_tail(c: Pt, d: f32, tail: Option<Pt>) -> Option<[Pt; 3]> {
    let t = tail?;
    let r = d / 2.0;
    let (dx, dy) = (t.x - c.x, t.y - c.y);
    if dx.hypot(dy) <= r + 1.0 {
        return None;
    }
    let th = dy.atan2(dx);
    let a = 32f32.to_radians();
    let at = |ang: f32| Pt::new(c.x + r * ang.cos(), c.y + r * ang.sin());
    Some([at(th - a), t, at(th + a)])
}

/// The tag's number color: white, or black on a light color (white
/// would read below 3:1 there, WCAG's large-text minimum).
pub fn counter_ink(c: C4) -> C4 {
    let lin = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    let l = 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
    let ink = if 1.05 / (l + 0.05) >= 3.0 { C4::rgb(255, 255, 255) } else { C4::rgb(0, 0, 0) };
    ink.with_alpha(c.a)
}

/// The tag's number in the UI font (Inter, the same everywhere) into
/// `m`, its ink box centred on `c` to the pixel; sized to the disc (a
/// longer number gets smaller to fit).
fn counter_number(m: &mut Mask, c: Pt, n: u32, d: f32) {
    let font = crate::fonts::ui_font();
    let s = n.to_string();
    let mut px = d * 0.8;
    let w = font.width(&s, px);
    if w > d * 0.68 {
        px *= d * 0.68 / w;
    }
    // Where the ink falls drawn at the origin; an integer shift keeps
    // the same coverage, so the box centres exactly.
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    font.draw(&s, px, 0.0, 0.0, |x, y, v| {
        if v > 0.0 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    });
    if x0 > x1 {
        return;
    }
    let dx = (c.x - (x0 + x1) as f32 / 2.0).round();
    let dy = (c.y - (y0 + y1) as f32 / 2.0).round();
    font.draw(&s, px, dx, dy, |x, y, v| m.put(x, y, v));
}

fn counter(sf: &mut Surf, c: Pt, n: u32, color: C4, d: f32, tail: Option<Pt>) {
    let r = d / 2.0;
    let tri = counter_tail(c, d, tail);
    let mut pts = vec![Pt::new(c.x - r, c.y - r), Pt::new(c.x + r, c.y + r)];
    pts.extend(tri.iter().flatten());
    // Disc and tail: one mask, one alpha.
    let mut m = Mask::around(sf, &pts, 1.0);
    m.disc(c, r);
    if let Some(t) = tri {
        m.convex(&t);
    }
    m.blend(sf, color, Blend::Normal);
    let mut m = Mask::around(sf, &pts[..2], 1.0);
    counter_number(&mut m, c, n, d);
    m.blend(sf, counter_ink(color), Blend::Normal);
}

/// Arrow geometry: shaft endpoints plus the filled head triangle.
fn arrow_geometry(a: Pt, b: Pt, width: f32) -> Option<([Pt; 2], [Pt; 3])> {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 2.0 {
        return None;
    }
    let ux = dx / len;
    let uy = dy / len;
    let hl = (width * 3.0).clamp(9.0, 60.0);
    let hw = hl * 0.45;
    let base = Pt::new(b.x - ux * hl, b.y - uy * hl);
    let px = -uy;
    let py = ux;
    let p1 = Pt::new(base.x + px * hw, base.y + py * hw);
    let p2 = Pt::new(base.x - px * hw, base.y - py * hw);
    Some(([a, base], [b, p1, p2]))
}

/// Mosaic `r`. The cell grid is anchored at the rect's corner in image
/// coordinates; cells are clipped to the surface window (exact when the
/// window contains the rect).
fn pixelate(sf: &mut Surf, r: FRect, cell: f32) {
    let (bx0, by0, bx1, by1) = sf.bounds();
    let pw = sf.width() as i32;
    let x0 = r.x.max(0.0).floor() as i32;
    let y0 = r.y.max(0.0).floor() as i32;
    let x1 = (r.x1().min(bx1 as f32).ceil() as i32).min(bx1);
    let y1 = (r.y1().min(by1 as f32).ceil() as i32).min(by1);
    if x1 <= x0.max(bx0) || y1 <= y0.max(by0) {
        return;
    }
    let c = cell.max(2.0).round() as i32;
    let step = (c / 4).max(1);
    let idx = |x: i32, y: i32| (((y - by0) * pw + (x - bx0)) as usize) * 4;
    // First sample at or after `lo` on the grid `start + k * step`.
    let align = |start: i32, lo: i32| {
        if start >= lo {
            start
        } else {
            start + (lo - start + step - 1) / step * step
        }
    };
    let Surf { data, .. } = sf;
    // Average of the in-window samples of the cell at (`cx`, `cy`); `None`
    // when the window clips them all (only when it does not contain the
    // rect): that cell keeps its pixels.
    let avg = |data: &[u8], cx: i32, cy: i32| {
        let ex = (cx + c).min(x1);
        let ey = (cy + c).min(y1);
        let (mut sr, mut sg, mut sb, mut sa, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
        let sx = align(cx, bx0);
        if sx < ex {
            let mut sy = align(cy, by0);
            while sy < ey {
                let row = &data[idx(sx, sy)..idx(ex, sy)];
                for p in row.as_chunks::<4>().0.iter().step_by(step as usize) {
                    sr += p[0] as u32;
                    sg += p[1] as u32;
                    sb += p[2] as u32;
                    sa += p[3] as u32;
                    n += 1;
                }
                sy += step;
            }
        }
        (n > 0).then(|| [(sr / n) as u8, (sg / n) as u8, (sb / n) as u8, (sa / n) as u8])
    };
    // Skip whole cells left of / above the window.
    let cx_first = x0 + ((bx0 - x0).max(0) / c) * c;
    let mut cy = y0 + ((by0 - y0).max(0) / c) * c;
    while cy < y1 {
        let ey = (cy + c).min(y1);
        let top = cy.max(by0);
        // One row of the band of cells, then copied down (all its samples
        // are read by then; cells never sample each other).
        let mut whole = true;
        let mut cx = cx_first;
        while cx < x1 {
            match avg(data, cx, cy) {
                Some(v) => {
                    let ex = (cx + c).min(x1);
                    for p in data[idx(cx.max(bx0), top)..idx(ex, top)].as_chunks_mut::<4>().0 {
                        *p = v;
                    }
                }
                None => whole = false,
            }
            cx += c;
        }
        let (a, b) = (idx(cx_first.max(bx0), top), idx(x1, top));
        if whole {
            for sy in top + 1..ey {
                data.copy_within(a..b, idx(cx_first.max(bx0), sy));
            }
        } else {
            // Rare (clipped window): cell by cell, skipping empty ones.
            let mut cx = cx_first;
            while cx < x1 {
                if avg(data, cx, cy).is_some() {
                    let (xa, ex) = (idx(cx.max(bx0), top), idx((cx + c).min(x1), top));
                    for sy in top + 1..ey {
                        data.copy_within(xa..ex, idx(cx.max(bx0), sy));
                    }
                }
                cx += c;
            }
        }
        cy += c;
    }
}

fn invert(sf: &mut Surf, r: FRect) {
    let (bx0, by0, bx1, by1) = sf.bounds();
    let pw = sf.width() as i32;
    let x0 = (r.x.max(0.0).floor() as i32).max(bx0);
    let y0 = (r.y.max(0.0).floor() as i32).max(by0);
    let x1 = (r.x1().min(bx1 as f32).ceil() as i32).min(bx1);
    let y1 = (r.y1().min(by1 as f32).ceil() as i32).min(by1);
    let Surf { data, .. } = sf;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (((y - by0) * pw + (x - bx0)) as usize) * 4;
            data[i] = 255 - data[i];
            data[i + 1] = 255 - data[i + 1];
            data[i + 2] = 255 - data[i + 2];
        }
    }
}

impl Obj {
    /// Conservative bounds of every pixel `render` may touch (half the
    /// stroke width, arrowhead, AA and glyph overhang included); `None`
    /// when it draws nothing.
    pub fn bounds(&self, font: Option<&AnnotFont>) -> Option<FRect> {
        fn bbox(pts: &[Pt], m: f32) -> Option<FRect> {
            let first = pts.first()?;
            let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
            for p in pts {
                (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
            }
            Some(FRect { x: x0 - m, y: y0 - m, w: x1 - x0 + 2.0 * m, h: y1 - y0 + 2.0 * m })
        }
        let m = |w: f32| w.max(0.5) / 2.0 + 2.0;
        let grow = |r: &FRect, m: f32| FRect { x: r.x - m, y: r.y - m, w: r.w + 2.0 * m, h: r.h + 2.0 * m };
        match self {
            Obj::Line { a, b, width, .. } | Obj::Marker { a, b, width, .. } => bbox(&[*a, *b], m(*width)),
            Obj::Arrow { a, b, width, .. } => match arrow_geometry(*a, *b, *width) {
                Some((_, head)) => bbox(&[*a, *b, head[1], head[2]], m(*width)),
                None => bbox(&[*a, *b], m(*width)),
            },
            Obj::Path { pts, width, .. } => (pts.len() >= 2).then(|| bbox(pts, m(*width))).flatten(),
            Obj::Rect { r, width, .. } | Obj::Ellipse { r, width, .. } => Some(grow(r, m(*width))),
            Obj::Pixelate { r, .. } | Obj::Invert { r } | Obj::Blur { r, .. } => Some(grow(r, 1.0)),
            Obj::MarkerPath { pts, width, .. } => (pts.len() >= 2).then(|| bbox(pts, m(*width))).flatten(),
            Obj::RectFill { r, width, .. } | Obj::EllipseFill { r, width, .. } => Some(grow(r, m(*width))),
            Obj::Counter { c, d, tail, .. } => {
                let r = d / 2.0;
                let mut pts = vec![Pt::new(c.x - r, c.y - r), Pt::new(c.x + r, c.y + r)];
                pts.extend(tail.filter(|_| counter_tail(*c, *d, *tail).is_some()));
                bbox(&pts, 2.0)
            }
            Obj::Callout { r, tail, .. } => Some(callout::bounds(*r, *tail)),
            // (Its outline while drawn reaches a pixel or two out.)
            Obj::Spotlight { r } => Some(grown(*r, 3.0)),
            Obj::Magnify { src, dst, radius, zoom, width, .. } => {
                let (rs, m) = (radius / zoom.max(1.0), width.max(0.5) + 2.0);
                let a = FRect { x: src.x - rs - m, y: src.y - rs - m, w: 2.0 * (rs + m), h: 2.0 * (rs + m) };
                let b = FRect { x: dst.x - radius - m, y: dst.y - radius - m, w: 2.0 * (radius + m), h: 2.0 * (radius + m) };
                let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
                Some(FRect { x: x0, y: y0, w: a.x1().max(b.x1()) - x0, h: a.y1().max(b.y1()) - y0 })
            }
            Obj::Text { pos, text, size, fmt, .. } => {
                let (w, h, _) = font?.styled(*fmt).measure(text, *size);
                // Overhang, rounding to whole pixels and AA bleed.
                let m = size + 2.0;
                Some(FRect { x: pos.x - m, y: pos.y - m, w: w + 2.0 * m, h: h + 2.0 * m })
            }
        }
    }

    /// Pixelate only: the mosaic grid as rendered into a window covering
    /// the whole image: the pixelated area `[x0, x1) x [y0, y1)` (`x1`/`y1`
    /// not yet clipped to the image) and the cell size, cells anchored at
    /// (`x0`, `y0`). A window whose edges inside that area fall on cell
    /// edges renders those pixels exactly as the whole image does.
    ///
    /// A blur reads its whole rect for any of its pixels: its grid is one
    /// cell as big as the rect, so every window that cuts it grows to all
    /// of it ([`blur::px_rect`]).
    pub fn pixel_grid(&self) -> Option<CellGrid> {
        match self {
            Obj::Pixelate { r, cell } => Some(CellGrid {
                x0: r.x.max(0.0).floor() as i32,
                y0: r.y.max(0.0).floor() as i32,
                x1: r.x1().ceil() as i32,
                y1: r.y1().ceil() as i32,
                c: cell.max(2.0).round() as i32,
            }),
            Obj::Blur { r, .. } => {
                let (x0, y0, x1, y1) = blur::px_rect(*r);
                Some(CellGrid { x0, y0, x1, y1, c: (x1 - x0).max(y1 - y0).max(1) })
            }
            _ => None,
        }
    }

    /// Bake this object into the image.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn render(&self, buf: &mut PixBuf, font: Option<&AnnotFont>) {
        self.render_into(&mut Surf::from_buf(buf), font);
    }

    /// Bake this object into any surface (offset window, either channel
    /// order). Pixelate/invert read the surface's own pixels, so the caller
    /// pre-fills it with the background; pixelate is exact when the surface
    /// window contains the whole rect.
    pub fn render_into(&self, sf: &mut Surf, font: Option<&AnnotFont>) {
        match self {
            Obj::Line { a, b, color, width } => {
                sf.stroke_polyline(&[*a, *b], *width, *color, Blend::Normal);
            }
            Obj::Marker { a, b, color, width } => {
                sf.stroke_polyline(&[*a, *b], *width, *color, Blend::Multiply);
            }
            Obj::Arrow { a, b, color, width } => {
                if let Some((shaft, head)) = arrow_geometry(*a, *b, *width) {
                    // One mask for shaft and head: a translucent arrow is
                    // blended once where they overlap (no darker join).
                    let mut m = Mask::around(sf, &[shaft[0], shaft[1], head[0], head[1], head[2]], width.max(0.5) / 2.0 + 1.0);
                    m.polyline(&shaft, *width);
                    m.convex(&head);
                    m.blend(sf, *color, Blend::Normal);
                } else {
                    sf.stroke_polyline(&[*a, *b], *width, *color, Blend::Normal);
                }
            }
            Obj::Rect { r, color, width } => {
                sf.stroke_round_rect(*r, *width, *width, *color, Blend::Normal);
            }
            Obj::Ellipse { r, color, width } => {
                sf.stroke_ellipse(*r, *width, *color, Blend::Normal);
            }
            Obj::Path { pts, color, width } => {
                if pts.len() < 2 {
                    return;
                }
                sf.stroke_polyline(pts, *width, *color, Blend::Normal);
            }
            Obj::Text { pos, text, color, size, fmt } => {
                if let Some(font) = font {
                    font.styled(*fmt).render(sf, text, *size, *pos, *color);
                }
            }
            Obj::Pixelate { r, cell } => pixelate(sf, *r, *cell),
            Obj::Blur { r, sigma } => blur::render(sf, *r, *sigma),
            Obj::Invert { r } => invert(sf, *r),
            Obj::MarkerPath { pts, color, width } => {
                if pts.len() >= 2 {
                    sf.stroke_polyline(pts, *width, *color, Blend::Multiply);
                }
            }
            Obj::RectFill { r, color, width } => {
                let w = width.max(0.5);
                let mut m = Mask::new(sf, (r.x - w).floor() as i32, (r.y - w).floor() as i32, (r.x1() + w).ceil() as i32, (r.y1() + w).ceil() as i32);
                m.round_box(grown(*r, w / 2.0), 1.5 * w);
                m.blend(sf, *color, Blend::Normal);
            }
            Obj::EllipseFill { r, color, width } => {
                let w = width.max(0.5);
                let mut m = Mask::new(sf, (r.x - w).floor() as i32, (r.y - w).floor() as i32, (r.x1() + w).ceil() as i32, (r.y1() + w).ceil() as i32);
                // The inside plus the outline's own ring: exactly the
                // outlined ellipse's silhouette.
                m.ellipse(*r);
                m.ellipse_ring(*r, w);
                m.blend(sf, *color, Blend::Normal);
            }
            Obj::Counter { c, n, color, d, tail } => counter(sf, *c, *n, *color, *d, *tail),
            Obj::Callout { r, tail, text, color, size, style, fill, fmt } => {
                let font = font.map(|f| f.styled(*fmt));
                callout::render(sf, *r, *tail, text, *color, *size, *style, *fill, font.as_ref());
            }
            // The dim is the spotlight pass over the finished image.
            Obj::Spotlight { .. } => {}
            Obj::Magnify { src, dst, radius, zoom, color, width } => magnify(sf, None, *src, *dst, *radius, *zoom, *color, *width),
        }
    }

    /// A spotlight (draws nothing itself; see [`SpotPass`]).
    pub fn is_spot(&self) -> bool {
        matches!(self, Obj::Spotlight { .. })
    }

    /// Reads pixels anywhere in its bounds to draw any of them: a window
    /// must hold its whole bounds to render it (pixelate cells are
    /// handled by [`Obj::pixel_grid`]).
    pub fn reads_far(&self) -> bool {
        matches!(self, Obj::Magnify { .. })
    }

    /// The magnifier: the area it reads (the source, a pixel round) and
    /// the loupe it draws them into.
    pub fn loupe_parts(&self) -> Option<(FRect, FRect)> {
        let Obj::Magnify { src, dst, radius, zoom, width, .. } = self else { return None };
        let rs = radius / zoom.max(1.0) + 2.0;
        let rl = radius + width.max(1.0) + 2.0;
        Some((
            FRect { x: src.x - rs, y: src.y - rs, w: 2.0 * rs, h: 2.0 * rs },
            FRect { x: dst.x - rl, y: dst.y - rl, w: 2.0 * rl, h: 2.0 * rl },
        ))
    }

    /// Render a magnifier reading its source from `source` (a surface
    /// holding the image as drawn below it, around the source) instead
    /// of from `sf`; anything else renders as usual.
    pub fn render_with_source(&self, sf: &mut Surf, source: &Surf, font: Option<&AnnotFont>) {
        match self {
            Obj::Magnify { src, dst, radius, zoom, color, width } => magnify(sf, Some(source), *src, *dst, *radius, *zoom, *color, *width),
            o => o.render_into(sf, font),
        }
    }
}

/// The loupe: the pixels around `src` (in `ext`, else already in the
/// surface, which must hold them) scaled `zoom` x into a disc at `dst`,
/// then one mask for the rings and the connector.
#[allow(clippy::too_many_arguments)]
fn magnify(sf: &mut Surf, ext: Option<&Surf>, src: Pt, dst: Pt, radius: f32, zoom: f32, color: C4, width: f32) {
    let zoom = zoom.max(1.0);
    let rs = radius / zoom;
    if radius < 2.0 {
        return;
    }
    if ![src.x, src.y, dst.x, dst.y, radius, zoom].iter().all(|v| v.is_finite()) || radius > 1.0e5 {
        return;
    }
    // Copy the source first (the loupe may cover it): only the part the
    // read surface holds.
    let (bx0, by0, bx1, by1) = ext.map_or(sf.bounds(), |e| e.bounds());
    let clip = |v: f32, lo: i32, hi: i32| (v as f64).clamp(lo as f64, hi as f64) as i32;
    let (sx0, sy0) = (clip((src.x - rs).floor() - 1.0, bx0, bx1), clip((src.y - rs).floor() - 1.0, by0, by1));
    let (sx1, sy1) = (clip((src.x + rs).ceil() + 1.0, bx0, bx1), clip((src.y + rs).ceil() + 1.0, by0, by1));
    let sw = (sx1 - sx0).max(0) as usize;
    let mut tmp = vec![None; sw * (sy1 - sy0).max(0) as usize];
    for y in sy0..sy1 {
        for x in sx0..sx1 {
            tmp[(y - sy0) as usize * sw + (x - sx0) as usize] = match ext {
                Some(e) => e.get_raw(x, y),
                None => sf.get_raw(x, y),
            };
        }
    }
    let mut m = Mask::new(sf, (dst.x - radius).floor() as i32 - 1, (dst.y - radius).floor() as i32 - 1, (dst.x + radius).ceil() as i32 + 1, (dst.y + radius).ceil() as i32 + 1);
    m.disc(dst, radius);
    m.each(|x, y, cov| {
        let qx = (src.x + (x as f32 + 0.5 - dst.x) / zoom).floor() as i32;
        let qy = (src.y + (y as f32 + 0.5 - dst.y) / zoom).floor() as i32;
        if qx < sx0 || qy < sy0 || qx >= sx1 || qy >= sy1 {
            return;
        }
        if let Some(px) = tmp[(qy - sy0) as usize * sw + (qx - sx0) as usize] {
            sf.mix_raw(x, y, px, cov);
        }
    });
    // Rings round the loupe and the source, the connector between them.
    let w = width.max(1.0);
    let ws = (w / 2.0).max(1.0);
    let pts = [
        Pt::new(src.x - rs - w, src.y - rs - w),
        Pt::new(src.x + rs + w, src.y + rs + w),
        Pt::new(dst.x - radius - w, dst.y - radius - w),
        Pt::new(dst.x + radius + w, dst.y + radius + w),
    ];
    let mut ring = Mask::around(sf, &pts, 2.0);
    ring.ellipse_ring(FRect { x: dst.x - radius, y: dst.y - radius, w: 2.0 * radius, h: 2.0 * radius }, w);
    ring.ellipse_ring(FRect { x: src.x - rs, y: src.y - rs, w: 2.0 * rs, h: 2.0 * rs }, ws);
    let (dx, dy) = (dst.x - src.x, dst.y - src.y);
    let d = dx.hypot(dy);
    if d > radius + rs + 2.0 {
        let (ux, uy) = (dx / d, dy / d);
        let a = Pt::new(src.x + ux * rs, src.y + uy * rs);
        let b = Pt::new(dst.x - ux * radius, dst.y - uy * radius);
        ring.polyline(&[a, b], ws);
    }
    ring.blend(sf, color, Blend::Normal);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frect_from_pts_normalizes() {
        let r = FRect::from_pts(Pt::new(10.0, 20.0), Pt::new(4.0, 8.0));
        assert_eq!(r.x, 4.0);
        assert_eq!(r.y, 8.0);
        assert_eq!(r.w, 6.0);
        assert_eq!(r.h, 12.0);
    }

    #[test]
    fn renders_line_into_buffer() {
        let mut img = PixBuf::new(20, 20);
        Obj::Line {
            a: Pt::new(2.0, 2.0),
            b: Pt::new(18.0, 18.0),
            color: C4::rgb(255, 0, 0),
            width: 3.0,
        }
        .render(&mut img, None);
        let i = ((10 * 20 + 10) * 4) as usize;
        let d = img.as_raw();
        assert_ne!(d[i], 0, "line should cover the middle pixel");
        assert_eq!(d[i + 3], 255);
    }

    #[test]
    fn pixelate_fills_region() {
        let mut img = PixBuf::new(32, 32);
        // gradient-ish content: set a few distinct pixels
        for i in 0..(32 * 32) {
            let v = (i % 255) as u8;
            let d = &mut img.as_raw_mut()[i * 4..i * 4 + 4];
            d[0] = v;
            d[1] = 255 - v;
            d[2] = 100;
            d[3] = 255;
        }
        Obj::Pixelate {
            r: FRect {
                x: 0.0,
                y: 0.0,
                w: 16.0,
                h: 16.0,
            },
            cell: 8.0,
        }
        .render(&mut img, None);
        // Top-left 8x8 cell must now be a solid color.
        let d = img.as_raw();
        let first = &d[0..4];
        for y in 0..8 {
            for x in 0..8 {
                let i = (y * 32 + x) as usize * 4;
                assert_eq!(&d[i..i + 4], first, "at {x},{y}");
            }
        }
    }

    #[test]
    fn text_render_does_not_panic_without_font() {
        let mut img = PixBuf::new(10, 10);
        Obj::Text {
            fmt: Default::default(),
            pos: Pt::new(1.0, 1.0),
            text: "hi".into(),
            color: C4::rgb(255, 255, 255),
            size: 8.0,
        }
        .render(&mut img, None);
    }

    #[test]
    fn every_object_matches_full_rgba_in_offset_bgra() {
        use crate::uifb::tests::equiv::check;
        let c = C4::new(250, 60, 20, 210);
        let (a, b) = (Pt::new(18.0, 62.0), Pt::new(82.0, 22.0));
        let rect = FRect { x: 24.0, y: 14.0, w: 56.0, h: 40.0 };
        let objs = vec![
            Obj::Line { a, b, color: c, width: 4.0 },
            Obj::Arrow { a, b, color: c, width: 5.0 },
            Obj::Arrow { a, b: Pt::new(18.5, 62.5), color: c, width: 5.0 },
            Obj::Rect { r: rect, color: c, width: 3.0 },
            Obj::Ellipse { r: rect, color: c, width: 3.0 },
            Obj::Path {
                pts: vec![a, Pt::new(40.0, 20.0), Pt::new(60.0, 60.0), b],
                color: c,
                width: 3.0,
            },
            Obj::Marker { a, b, color: C4::new(255, 230, 0, 160), width: 14.0 },
            Obj::Invert { r: rect },
            Obj::Pixelate { r: FRect { x: 33.0, y: 23.0, w: 30.0, h: 22.0 }, cell: 6.0 },
            Obj::Pixelate { r: FRect { x: 31.0, y: 21.0, w: 38.0, h: 28.0 }, cell: 8.0 },
            Obj::MarkerPath { pts: vec![a, Pt::new(40.0, 20.0), Pt::new(60.0, 60.0), b], color: C4::new(255, 230, 0, 160), width: 12.0 },
            Obj::RectFill { r: rect, color: c, width: 3.0 },
            Obj::EllipseFill { r: rect, color: c, width: 3.0 },
            Obj::Counter { c: Pt::new(50.5, 40.25), n: 8, color: c, d: 34.0, tail: Some(Pt::new(90.0, 70.0)) },
            Obj::Counter { c: Pt::new(30.0, 30.0), n: 120, color: C4::rgb(250, 220, 0), d: 26.0, tail: None },
        ];
        for (n, o) in objs.iter().enumerate() {
            check(&format!("obj {n}"), |s| o.render_into(s, None));
        }
        if let Some(font) = AnnotFont::load() {
            let t = Obj::Text {
                fmt: Default::default(),
                pos: Pt::new(22.0, 16.0),
                text: "Hello\nWorld wy".into(),
                color: c,
                size: 16.0,
            };
            check("text", |s| t.render_into(s, Some(&font)));
        }
    }

    /// A window that clips the rect mid-cell (the overlay's band edges):
    /// no panic, and cells left without samples keep their pixels instead
    /// of turning transparent black.
    #[test]
    fn pixelate_straddling_window_skips_empty_cells() {
        let (w, h) = (64u32, 64u32);
        let mut full = PixBuf::new(w, h);
        for (i, p) in full.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *p = [(i * 7) as u8, (i * 3) as u8, 200, 255];
        }
        let o = Obj::Pixelate { r: FRect { x: 3.0, y: 5.0, w: 50.0, h: 47.0 }, cell: 12.0 };
        // Windows starting just before a cell boundary leave the clipped
        // cell's samples outside.
        for (wx, wy) in [(14, 16), (13, 15), (0, 0), (26, 28), (40, 2)] {
            let (ww, wh) = (20u32, 21u32);
            let mut win = vec![0u8; (ww * wh * 4) as usize];
            for y in 0..wh {
                for x in 0..ww {
                    let s = full.get_pixel((wx + x).min(w - 1), (wy + y).min(h - 1));
                    win[((y * ww + x) * 4) as usize..][..4].copy_from_slice(&s);
                }
            }
            o.render_into(&mut Surf::with_origin(&mut win, ww, wh, wx as i32, wy as i32, crate::raster::Order::Rgba), None);
            for (i, p) in win.as_chunks::<4>().0.iter().enumerate() {
                assert_ne!(*p, [0, 0, 0, 0], "window ({wx}, {wy}): pixel {i} cleared");
            }
        }
    }

    /// A 50 % arrow: shaft and head are one mask, so every fully covered
    /// pixel around the head's base has the same value (no darker
    /// semicircle where the shaft's round cap sits under the head).
    #[test]
    fn translucent_arrow_blends_once_at_the_join() {
        for (a, b) in [
            (Pt::new(10.0, 50.0), Pt::new(110.0, 50.0)),
            (Pt::new(110.0, 50.0), Pt::new(10.0, 50.0)),
        ] {
            let mut img = PixBuf::new(120, 100);
            for p in img.as_raw_mut().as_chunks_mut::<4>().0 {
                *p = [255, 255, 255, 255];
            }
            Obj::Arrow { a, b, color: C4::new(0, 0, 0, 128), width: 8.0 }.render(&mut img, None);
            // Head length 24: the base is 24 px before the tip; the shaft's
            // cap would reach 4 px into the head.
            let base = if b.x > a.x { b.x - 24.0 } else { b.x + 24.0 };
            let mut vals = std::collections::BTreeSet::new();
            for y in 47..53 {
                for x in (base as i32 - 8)..(base as i32 + 8) {
                    vals.insert(img.get_pixel(x as u32, y as u32)[0]);
                }
            }
            assert_eq!(vals.len(), 1, "{a:?}->{b:?}: values around the base {vals:?}");
            let v = *vals.first().unwrap();
            assert!((126..=128).contains(&v), "half of white: {v}");
        }
    }

    /// The other objects that draw several primitives (text: glyphs and
    /// lines) also blend once: overlapping glyphs of a translucent color
    /// are no darker than one glyph.
    #[test]
    fn translucent_text_is_one_alpha() {
        let Some(font) = AnnotFont::load() else { return };
        let mut img = PixBuf::new(200, 80);
        for p in img.as_raw_mut().as_chunks_mut::<4>().0 {
            *p = [255, 255, 255, 255];
        }
        // "ll" squeezed: at 40 px the stems are wide; "WW" overlap in most
        // fonts' AA edges. Every pixel is at least half of white.
        Obj::Text { fmt: Default::default(), pos: Pt::new(6.0, 6.0), text: "WWAVffll\nWW".into(), color: C4::new(0, 0, 0, 128), size: 30.0 }
            .render(&mut img, Some(&font));
        let darkest = img.as_raw().as_chunks::<4>().0.iter().map(|p| p[0]).min().unwrap();
        assert!(darkest >= 126, "a pixel blended twice: {darkest}");
        assert!(darkest <= 128, "the text was drawn: {darkest}");
    }

    fn white(w: u32, h: u32) -> PixBuf {
        let mut img = PixBuf::new(w, h);
        for p in img.as_raw_mut().as_chunks_mut::<4>().0 {
            *p = [255, 255, 255, 255];
        }
        img
    }

    /// The tag: a disc of the diameter in the color, the number in the
    /// ink that reads on it, centred; the bounds hold every pixel.
    #[test]
    fn counter_draws_a_numbered_disc() {
        assert_eq!(counter_ink(C4::rgb(240, 68, 56)), C4::rgb(255, 255, 255), "white on the default red");
        assert_eq!(counter_ink(C4::rgb(59, 130, 246)), C4::rgb(255, 255, 255));
        assert_eq!(counter_ink(C4::rgb(250, 220, 0)), C4::rgb(0, 0, 0), "black on yellow");
        assert_eq!(counter_ink(C4::rgb(255, 255, 255)), C4::rgb(0, 0, 0));
        assert_eq!(counter_ink(C4::new(0, 0, 0, 77)).a, 77, "the color's opacity");
        for (d, n) in [(20.0f32, 1u32), (32.0, 7), (48.0, 12), (96.0, 100)] {
            let mut img = white(140, 140);
            let o = Obj::Counter { c: Pt::new(70.0, 70.0), n, color: C4::rgb(20, 40, 160), d, tail: None };
            o.render(&mut img, None);
            let r = d / 2.0;
            let b = o.bounds(None).unwrap();
            let (mut ink, mut sx, mut sy) = (0usize, 0.0f32, 0.0f32);
            for y in 0..140u32 {
                for x in 0..140u32 {
                    let p = img.get_pixel(x, y);
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let dist = (fx - 70.0).hypot(fy - 70.0);
                    if p != [255, 255, 255, 255] {
                        assert!(fx >= b.x && fx <= b.x1() && fy >= b.y && fy <= b.y1(), "d {d}: ({x}, {y}) outside the bounds");
                        assert!(dist <= r + 1.0, "d {d}: ({x}, {y}) off the disc");
                    }
                    if dist > r + 1.0 {
                        continue;
                    }
                    if p[0] > 200 && p[2] > 200 && dist < r - 1.0 {
                        // The white number inside the blue disc.
                        ink += 1;
                        sx += fx;
                        sy += fy;
                    }
                }
            }
            assert!(ink > (d * d / 40.0) as usize, "d {d} n {n}: the number shows ({ink} px)");
            let (cx, cy) = (sx / ink as f32, sy / ink as f32);
            // The ink's centroid sits near the centre (digit shapes are not
            // symmetric: a few px for "7").
            assert!((cx - 70.0).abs() < d * 0.12 + 1.0 && (cy - 70.0).abs() < d * 0.12 + 1.0, "d {d} n {n}: centred? ({cx}, {cy})");
            assert_eq!(img.get_pixel(70, (70.0 - r + 2.0) as u32), [20, 40, 160, 255], "d {d}: the disc's top");
        }
    }

    /// A translucent tag with its tail is one alpha: no darker join where
    /// the tail meets the disc.
    #[test]
    fn counter_tail_blends_once() {
        let mut img = white(160, 120);
        let o = Obj::Counter { c: Pt::new(40.0, 60.0), n: 1, color: C4::new(0, 0, 0, 128), d: 40.0, tail: Some(Pt::new(150.0, 60.0)) };
        o.render(&mut img, None);
        let mut vals = std::collections::BTreeSet::new();
        // Along the axis from inside the disc's edge into the tail.
        for x in 52..75 {
            for y in 58..62 {
                vals.insert(img.get_pixel(x, y)[0]);
            }
        }
        assert_eq!(vals.len(), 1, "{vals:?}");
        assert!((126..=128).contains(vals.first().unwrap()));
        assert!(img.get_pixel(140, 60)[0] < 200, "the tail reaches the tip");
        assert!(o.bounds(None).unwrap().x1() >= 150.0);
        // A tip inside the disc draws no tail.
        let inside = Obj::Counter { c: Pt::new(40.0, 60.0), n: 1, color: C4::rgb(0, 0, 0), d: 40.0, tail: Some(Pt::new(50.0, 60.0)) };
        let plain = Obj::Counter { c: Pt::new(40.0, 60.0), n: 1, color: C4::rgb(0, 0, 0), d: 40.0, tail: None };
        let (mut a, mut b) = (white(160, 120), white(160, 120));
        inside.render(&mut a, None);
        plain.render(&mut b, None);
        assert!(a == b);
    }

    /// Filled shapes cover the outline's silhouette; a translucent fill
    /// is one even tint.
    #[test]
    fn filled_shapes_cover_the_outline_silhouette() {
        let rect = FRect { x: 20.0, y: 20.0, w: 80.0, h: 50.0 };
        let red = C4::rgb(200, 0, 0);
        for (outline, fill) in [
            (Obj::Rect { r: rect, color: red, width: 6.0 }, Obj::RectFill { r: rect, color: red, width: 6.0 }),
            (Obj::Ellipse { r: rect, color: red, width: 6.0 }, Obj::EllipseFill { r: rect, color: red, width: 6.0 }),
        ] {
            let (mut a, mut b) = (white(120, 90), white(120, 90));
            outline.render(&mut a, None);
            fill.render(&mut b, None);
            for y in 0..90u32 {
                for x in 0..120u32 {
                    let (o, f) = (a.get_pixel(x, y)[1], b.get_pixel(x, y)[1]);
                    // Wherever the outline paints (half covered or more), so
                    // does the fill (AA edges may differ by a fraction).
                    assert!(o > 128 || f < 160, "{fill:?} ({x}, {y}): outline {o} fill {f}");
                }
            }
            assert_eq!(b.get_pixel(60, 45), [200, 0, 0, 255], "solid inside");
            assert_eq!(b.get_pixel(2, 2), [255, 255, 255, 255], "nothing outside");
            assert!(fill.bounds(None).unwrap().x <= 16.0);
        }
        let mut img = white(120, 90);
        Obj::RectFill { r: rect, color: C4::new(0, 0, 0, 128), width: 3.0 }.render(&mut img, None);
        let mut vals = std::collections::BTreeSet::new();
        for y in 25..65 {
            for x in 25..95 {
                vals.insert(img.get_pixel(x, y)[0]);
            }
        }
        assert_eq!(vals.len(), 1, "{vals:?}");
    }

    #[test]
    fn marker_path_is_one_multiply_stroke() {
        let mut img = white(100, 60);
        let pts = vec![Pt::new(10.0, 30.0), Pt::new(50.0, 30.0), Pt::new(10.0, 31.0)];
        Obj::MarkerPath { pts, color: C4::new(250, 220, 0, 90), width: 12.0 }.render(&mut img, None);
        // Doubled back over itself: still one blend.
        let v = img.get_pixel(30, 30);
        let w = img.get_pixel(48, 30);
        assert_eq!(v, w, "overlapping parts blend once");
        assert!(v[2] + 50 < v[0], "a yellow tint: {v:?}");
    }

    #[test]
    fn render_wrapper_matches_render_into() {
        let mut a = PixBuf::new(30, 30);
        let mut b = PixBuf::new(30, 30);
        let o = Obj::Ellipse {
            r: FRect { x: 3.0, y: 4.0, w: 20.0, h: 18.0 },
            color: C4::rgb(1, 2, 3),
            width: 2.0,
        };
        o.render(&mut a, None);
        o.render_into(&mut Surf::from_buf(&mut b), None);
        assert_eq!(a.as_raw(), b.as_raw());
    }
}
