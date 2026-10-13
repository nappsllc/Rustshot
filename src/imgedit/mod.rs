//! Image edits of the Edit window: an ordered, non-destructive transform
//! stack applied on export after the crop: rotate (90/180/270) and flip
//! (the picture's orientation), resize (W×H, Lanczos-3 or area average
//! down, bilinear up: [`resample`]) and canvas (padding, background,
//! rounded corners, shadow: [`frame`]). `original.png` is never written.
//!
//! **Coordinates.** The objects stay in `original.png`'s pixels (where
//! v0.2 and v0.3 draw them, so an older build that ignores the stack
//! still draws every redaction where it belongs). On export the picture
//! is turned/flipped and the objects with it ([`Orient::map_obj`]), the
//! objects are baked there, then the resize and the canvas apply to the
//! baked picture (objects scale with it).
//!
//! **The upright rule.** Text stays upright after a rotation or a flip:
//! its anchor turns with the picture, the glyphs do not. For a text the
//! anchor is the center of its box (its pos moves so that the box's center
//! lands where the old center went; the box keeps its size); an outlined
//! callout's box keeps its size and orientation too, its center and tail
//! point move; a number's disc moves with its center (its digit stays
//! upright, the disc covers the same pixels). A *filled* callout covers
//! what is under it, so its box is mapped exactly (a quarter turn swaps
//! its sides and the text wraps again inside): it keeps covering the same
//! pixels however often the picture turns. Everything else (strokes,
//! rects, ellipses, pixelate, blur, invert, spotlight, magnifier) is
//! mapped exactly, point by point.

pub mod frame;
pub mod resample;

use crate::objects::{FRect, Obj, Pt};
use crate::pixbuf::PixBuf;
use crate::text::AnnotFont;
pub use frame::{Bg, Pad};
pub use resample::Filter;

/// The picture's orientation: mirrored left-right first (`flip`), then
/// turned `rot` quarter turns clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Orient {
    pub rot: u8,
    pub flip: bool,
}

impl Orient {
    pub const ID: Orient = Orient { rot: 0, flip: false };

    pub fn is_identity(self) -> bool {
        self == Orient::ID
    }

    /// This, then `b` (as seen on the picture this one gives).
    pub fn then(self, b: Orient) -> Orient {
        let rot = if b.flip { (b.rot + 4 - self.rot % 4) % 4 } else { (b.rot + self.rot) % 4 };
        Orient { rot, flip: self.flip != b.flip }
    }

    /// What undoes this.
    pub fn inverse(self) -> Orient {
        if self.flip { Orient { rot: self.rot % 4, flip: true } } else { Orient { rot: (4 - self.rot % 4) % 4, flip: false } }
    }

    /// Turned a quarter clockwise (Rotate right) or counter-clockwise.
    pub fn rotated(self, cw: bool) -> Orient {
        self.then(Orient { rot: if cw { 1 } else { 3 }, flip: false })
    }

    /// Mirrored left-right (Flip horizontal) as the picture shows now.
    pub fn flipped_h(self) -> Orient {
        self.then(Orient { rot: 0, flip: true })
    }

    /// Mirrored top-bottom (Flip vertical): a left-right flip turned 180°.
    pub fn flipped_v(self) -> Orient {
        self.then(Orient { rot: 2, flip: true })
    }

    /// The size of a `w`×`h` picture in this orientation.
    pub fn size(self, (w, h): (u32, u32)) -> (u32, u32) {
        if self.rot % 2 == 1 { (h, w) } else { (w, h) }
    }

    /// Point `p` of a `size` picture, in this orientation's picture.
    pub fn pt(self, p: Pt, (w, h): (u32, u32)) -> Pt {
        let (w, h) = (w as f32, h as f32);
        let x = if self.flip { w - p.x } else { p.x };
        let y = p.y;
        match self.rot % 4 {
            0 => Pt::new(x, y),
            1 => Pt::new(h - y, x),
            2 => Pt::new(w - x, h - y),
            _ => Pt::new(y, w - x),
        }
    }

    /// Rect `r` of a `size` picture, in this orientation's picture.
    pub fn rect(self, r: FRect, size: (u32, u32)) -> FRect {
        FRect::from_pts(self.pt(Pt::new(r.x, r.y), size), self.pt(Pt::new(r.x1(), r.y1()), size))
    }

    /// A box that stays upright: its center moves, its size stays.
    fn upright(self, r: FRect, size: (u32, u32)) -> FRect {
        let c = self.pt(Pt::new(r.x + r.w / 2.0, r.y + r.h / 2.0), size);
        FRect { x: c.x - r.w / 2.0, y: c.y - r.h / 2.0, ..r }
    }

    /// Object `o` of a `size` picture mapped into this orientation (the
    /// upright rule, see the module; `font` measures texts, without one a
    /// text's top-left is moved as a point).
    pub fn map_obj(self, o: &Obj, size: (u32, u32), font: Option<&AnnotFont>) -> Obj {
        if self.is_identity() {
            return o.clone();
        }
        let p = |q: Pt| self.pt(q, size);
        let r = |q: FRect| self.rect(q, size);
        let mut o = o.clone();
        match &mut o {
            Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } => {
                *a = p(*a);
                *b = p(*b);
            }
            Obj::Path { pts, .. } | Obj::MarkerPath { pts, .. } => {
                for q in pts.iter_mut() {
                    *q = p(*q);
                }
            }
            Obj::Rect { r: q, .. }
            | Obj::Ellipse { r: q, .. }
            | Obj::RectFill { r: q, .. }
            | Obj::EllipseFill { r: q, .. }
            | Obj::Pixelate { r: q, .. }
            | Obj::Invert { r: q }
            | Obj::Spotlight { r: q }
            | Obj::Blur { r: q, .. } => *q = r(*q),
            Obj::Text { pos, text, size: px, fmt, .. } => match font {
                Some(f) => {
                    let (w, h, _) = f.styled(*fmt).measure(text, *px);
                    let b = self.upright(FRect { x: pos.x, y: pos.y, w, h }, size);
                    *pos = Pt::new(b.x, b.y);
                }
                None => *pos = p(*pos),
            },
            Obj::Counter { c, tail, .. } => {
                *c = p(*c);
                if let Some(t) = tail {
                    *t = p(*t);
                }
            }
            Obj::Callout { r: q, tail, fill, .. } => {
                // A filled box covers: mapped exactly (see the module).
                *q = if *fill { r(*q) } else { self.upright(*q, size) };
                *tail = p(*tail);
            }
            Obj::Magnify { src, dst, .. } => {
                *src = p(*src);
                *dst = p(*dst);
            }
        }
        o
    }

    /// The pixels of `img` in this orientation (a permutation: exact).
    pub fn image(self, img: &PixBuf) -> PixBuf {
        if self.is_identity() {
            return img.clone();
        }
        let (w, h) = img.dimensions();
        let (ow, oh) = self.size((w, h));
        let mut out = PixBuf::new(ow, oh);
        let (wu, owu, ohu) = (w as usize, ow as usize, oh as usize);
        let src = img.as_raw().as_chunks::<4>().0;
        // Each output pixel gathers its source pixel (the inverse's map of
        // the output picture), output rows split over threads, in tiles
        // so both buffers are walked within a few cache lines.
        let inv = self.inverse();
        let (rot, flip) = (inv.rot % 4, inv.flip);
        let at = move |x: usize, y: usize| -> usize {
            let fx = if flip { owu - 1 - x } else { x };
            let (sx, sy) = match rot {
                0 => (fx, y),
                1 => (ohu - 1 - y, fx),
                2 => (owu - 1 - fx, ohu - 1 - y),
                _ => (y, owu - 1 - fx),
            };
            sy * wu + sx
        };
        const T: usize = 64;
        let threads = if owu * ohu < 300_000 { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16) };
        let per = ohu.div_ceil(threads).div_ceil(T).max(1) * T;
        let dst = out.as_raw_mut().as_chunks_mut::<4>().0;
        std::thread::scope(|s| {
            for (ci, band) in dst.chunks_mut(per * owu).enumerate() {
                s.spawn(move || {
                    let rows = band.len() / owu;
                    for ty in (0..rows).step_by(T) {
                        for tx in (0..owu).step_by(T) {
                            for y in ty..(ty + T).min(rows) {
                                let gy = ci * per + y;
                                for x in tx..(tx + T).min(owu) {
                                    band[y * owu + x] = src[at(x, gy)];
                                }
                            }
                        }
                    }
                });
            }
        });
        out
    }
}

/// A resize: to `w`×`h` for a picture of `from` (the crop and the
/// orientation it was set for; a different picture is scaled by the same
/// factors), shrinking with `filter`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resize {
    pub w: u32,
    pub h: u32,
    pub from: (u32, u32),
    pub filter: Filter,
}

/// The largest side a resize may ask for (px).
pub const MAX_SIDE: u32 = 30_000;

/// The most pixels an export may have, the canvas included (200 MP: a
/// 4-byte buffer of 800 MB): the dialogs refuse more, the reader fails
/// closed on a stack that asks for more, and an export clamps the resize.
pub const MAX_PIXELS: u64 = 200_000_000;

/// Whether a `w`×`h` picture is within [`MAX_PIXELS`].
pub fn within_cap((w, h): (u32, u32)) -> bool {
    w as u64 * h as u64 <= MAX_PIXELS
}

impl Resize {
    /// The output size for a `src` picture.
    pub fn target(&self, src: (u32, u32)) -> (u32, u32) {
        if src == self.from || self.from.0 == 0 || self.from.1 == 0 {
            return (self.w.clamp(1, MAX_SIDE), self.h.clamp(1, MAX_SIDE));
        }
        let k = |v: u32, t: u32, f: u32| ((v as f64 * t as f64 / f as f64).round() as u32).clamp(1, MAX_SIDE);
        (k(src.0, self.w, self.from.0), k(src.1, self.h, self.from.1))
    }
}

/// The Edit window's image edits (none: the picture as captured).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edits {
    pub orient: Orient,
    pub resize: Option<Resize>,
    pub canvas: Option<Pad>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.orient.is_identity() && self.resize.is_none() && self.canvas.is_none()
    }

    /// The output size for a picture whose crop is `src` (in the
    /// original's orientation).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn out_size(&self, src: (u32, u32)) -> (u32, u32) {
        self.finish_size(self.orient.size(src))
    }

    /// The output size for a picture already turned (`src`: the crop as
    /// it shows): the resize, then the canvas.
    pub fn finish_size(&self, src: (u32, u32)) -> (u32, u32) {
        let r = self.resize_to(src);
        self.canvas.map_or(r, |c| c.size(r))
    }

    /// The size the resize gives a `src` picture (turned), clamped so the
    /// output with its canvas stays within [`MAX_PIXELS`].
    pub fn resize_to(&self, src: (u32, u32)) -> (u32, u32) {
        let Some(r) = self.resize else { return src };
        let (mut w, mut h) = r.target(src);
        let pad = |(w, h): (u32, u32)| self.canvas.map_or((w, h), |c| c.size((w, h)));
        while !within_cap(pad((w, h))) && (w > 1 || h > 1) {
            (w, h) = (((w as f64 * 0.97) as u32).max(1), ((h as f64 * 0.97) as u32).max(1));
        }
        (w, h)
    }

    /// The edits for the picture turned or flipped to `to`: a resize keeps
    /// its factors with the sides following the turn.
    pub fn reoriented(&self, to: Orient) -> Edits {
        let swap = (self.orient.rot + to.rot) % 2 == 1;
        let resize = self.resize.map(|r| if swap { Resize { w: r.h, h: r.w, from: (r.from.1, r.from.0), ..r } } else { r });
        Edits { orient: to, resize, ..*self }
    }
}

/// The export-only edits on the baked picture `img` (already turned):
/// the resize, then the canvas.
pub fn finish(img: PixBuf, e: &Edits) -> PixBuf {
    let img = match e.resize {
        Some(r) => {
            let (w, h) = e.resize_to(img.dimensions());
            if (w, h) == img.dimensions() { img } else { resample::resize(&img, w, h, r.filter) }
        }
        None => img,
    };
    match &e.canvas {
        Some(p) if !p.is_noop() => frame::apply(&img, p),
        _ => img,
    }
}

/// What the picture exports as: `original` with `objects` (its pixels),
/// cut by the lasso `mask` and `crop` (its pixels too), then the edits:
/// turned, baked, resized, on its canvas.
pub fn render(original: &PixBuf, objects: &[Obj], crop: Option<FRect>, mask: Option<&[Pt]>, e: &Edits) -> PixBuf {
    finish(render_picture(original, objects, crop, mask, e), e)
}

/// [`render`] before the resize and the canvas: cropped, turned, baked.
pub fn render_picture(original: &PixBuf, objects: &[Obj], crop: Option<FRect>, mask: Option<&[Pt]>, e: &Edits) -> PixBuf {
    if e.orient.is_identity() {
        return crate::history::render(original, objects, crop, mask);
    }
    let size = original.dimensions();
    let o = e.orient;
    let font = objects.iter().any(|x| matches!(x, Obj::Text { .. })).then(AnnotFont::load).flatten();
    let pic = o.image(original);
    let objs: Vec<Obj> = objects.iter().map(|x| o.map_obj(x, size, font.as_ref())).collect();
    // On whole pixels before it turns (a half pixel rounds the same way
    // in every orientation); one that misses the picture is kept as it is
    // (the cut then fails closed).
    let crop = crop.map(|c| crate::objects::mask::crop_rect(c, size).unwrap_or(c)).map(|c| o.rect(c, size));
    let mask: Option<Vec<Pt>> = mask.map(|m| m.iter().map(|p| o.pt(*p, size)).collect());
    crate::history::render(&pic, &objs, crop, mask.as_deref())
}

/// The export of [`render`] fitted into `max_w`×`max_h` (a thumbnail),
/// without ever making the full-size resize or canvas: the picture is
/// shrunk to the thumbnail's scale first, then resized and put on its
/// canvas at that scale.
pub fn render_thumb(original: &PixBuf, objects: &[Obj], crop: Option<FRect>, mask: Option<&[Pt]>, e: &Edits, max_w: u32, max_h: u32) -> PixBuf {
    let pic = render_picture(original, objects, crop, mask, e);
    let out = e.finish_size(pic.dimensions());
    let fit = crate::scale::fit_size(out.0, out.1, max_w, max_h);
    let k = (fit.0 as f32 / out.0.max(1) as f32).min(1.0);
    let (rw, rh) = e.resize_to(pic.dimensions());
    let (sw, sh) = (((rw as f32 * k).round() as u32).max(1), ((rh as f32 * k).round() as u32).max(1));
    let small = if (sw, sh) == pic.dimensions() { pic } else { resample::resize(&pic, sw, sh, Filter::Area) };
    let img = match &e.canvas {
        Some(p) if !p.is_noop() => {
            let s = |v: u32| (v as f32 * k).round() as u32;
            let q = Pad { top: s(p.top), right: s(p.right), bottom: s(p.bottom), left: s(p.left), radius: p.radius * k, ..*p };
            frame::apply_at(&small, &q, k)
        }
        _ => small,
    };
    crate::scale::fit(&img, max_w, max_h)
}

#[cfg(test)]
mod tests;
