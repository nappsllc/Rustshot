//! Freeform (lasso) masks and non-destructive crops of a picture.
//!
//! A lasso is a closed polygon in the picture's pixels: what is outside
//! it is transparent in a PNG (an anti-aliased edge: exact area coverage,
//! [`crate::raster::Coverage`]) and white in a JPG or BMP
//! ([`flatten_white`]); the picture keeps its bounding box. A crop is a
//! rect of the picture: the export is that part, the rest is kept (the
//! Edit window's crop can be undone or widened again).

use super::{FRect, Pt};
use crate::pixbuf::PixBuf;

/// Points closer than this (px) to the last one kept are dropped: a
/// drag's lasso stays a few hundred points.
pub const MIN_STEP: f32 = 1.5;

/// The lasso `pts` without points closer than [`MIN_STEP`] to the one
/// before (the first and last are kept).
pub fn simplify(pts: &[Pt]) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for (i, p) in pts.iter().enumerate() {
        let keep = match out.last() {
            None => true,
            Some(q) => (p.x - q.x).hypot(p.y - q.y) >= MIN_STEP || i + 1 == pts.len(),
        };
        if keep && p.x.is_finite() && p.y.is_finite() {
            out.push(*p);
        }
    }
    out
}

/// The lasso is a shape (three points that enclose some area).
pub fn is_shape(pts: &[Pt]) -> bool {
    pts.len() >= 3 && area(pts).abs() >= 4.0
}

/// The polygon's signed area (shoelace).
pub fn area(pts: &[Pt]) -> f32 {
    let n = pts.len();
    (0..n).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        a.x * b.y - b.x * a.y
    }).sum::<f32>()
        / 2.0
}

/// `p` lies inside the closed polygon `pts` (nonzero winding, as the
/// coverage fills it).
pub fn contains(pts: &[Pt], p: Pt) -> bool {
    let n = pts.len();
    let mut wind = 0i32;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let cross = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
        if a.y <= p.y {
            if b.y > p.y && cross > 0.0 {
                wind += 1;
            }
        } else if b.y <= p.y && cross < 0.0 {
            wind -= 1;
        }
    }
    n >= 3 && wind != 0
}

/// The bounding box of `pts` (zero-sized for none).
pub fn bbox(pts: &[Pt]) -> FRect {
    let Some(first) = pts.first() else { return FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 } };
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    FRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// `pts` moved by (`dx`, `dy`).
pub fn moved(pts: &[Pt], dx: f32, dy: f32) -> Vec<Pt> {
    pts.iter().map(|p| Pt::new(p.x + dx, p.y + dy)).collect()
}

/// The coverage (0-255) of the closed polygon `pts` over a `w`×`h`
/// grid whose top-left is at `origin` in the polygon's coordinates.
pub fn coverage(pts: &[Pt], w: u32, h: u32, origin: Pt) -> Vec<u8> {
    let (wu, hu) = (w as usize, h as usize);
    if wu == 0 || hu == 0 {
        return Vec::new();
    }
    if !is_shape(pts) {
        return vec![0; wu * hu];
    }
    let mut c = crate::raster::Coverage::new(wu, hu);
    let n = pts.len();
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        // Clamped left and right of the grid: an edge past the side still
        // counts for the rows it crosses (its area lands in the edge
        // column), past the top or bottom it is clipped by rows.
        let cl = |p: Pt| Pt::new((p.x - origin.x).clamp(0.0, w as f32), p.y - origin.y);
        c.line(cl(a), cl(b));
    }
    let mut out = vec![0u8; wu * hu];
    c.for_each(|x, y, v| out[y as usize * wu + x as usize] = (v.min(1.0) * 255.0).round() as u8);
    out
}

/// Make `img` (whose top-left is `origin` in the lasso's coordinates)
/// transparent outside the lasso `pts`: alpha times the coverage; the
/// pixels wholly outside become transparent white.
pub fn apply(img: &mut PixBuf, pts: &[Pt], origin: Pt) {
    let (w, h) = img.dimensions();
    let cov = coverage(pts, w, h, origin);
    for (px, &c) in img.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().zip(&cov) {
        if c == 0 {
            *px = [255, 255, 255, 0];
        } else if c < 255 {
            px[3] = ((px[3] as u16 * c as u16 + 127) / 255) as u8;
        }
    }
}

/// `img` over white, opaque: what a JPG or BMP shows of a picture with
/// transparent parts (none: unchanged).
pub fn flatten_white(img: &PixBuf) -> std::borrow::Cow<'_, PixBuf> {
    if img.as_raw().as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
        return std::borrow::Cow::Borrowed(img);
    }
    let mut out = img.clone();
    for p in out.as_raw_mut().as_chunks_mut::<4>().0 {
        let a = p[3] as u16;
        for c in &mut p[..3] {
            *c = ((*c as u16 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
    std::borrow::Cow::Owned(out)
}

/// Whether `img` has any transparent (or translucent) pixel.
#[cfg_attr(all(not(windows), not(test)), allow(dead_code))] // the Windows clipboard's
pub fn has_alpha(img: &PixBuf) -> bool {
    img.as_raw().as_chunks::<4>().0.iter().any(|p| p[3] != 255)
}

/// A crop rect on whole pixels inside a `size` picture: (x, y, w, h);
/// `None` when nothing of it is inside.
pub fn crop_px(r: FRect, size: (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    let x0 = r.x.min(r.x + r.w).round().clamp(0.0, size.0 as f32) as u32;
    let y0 = r.y.min(r.y + r.h).round().clamp(0.0, size.1 as f32) as u32;
    let x1 = r.x.max(r.x + r.w).round().clamp(0.0, size.0 as f32) as u32;
    let y1 = r.y.max(r.y + r.h).round().clamp(0.0, size.1 as f32) as u32;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// `r` on whole pixels inside the picture (the crop as it applies).
pub fn crop_rect(r: FRect, size: (u32, u32)) -> Option<FRect> {
    crop_px(r, size).map(|(x, y, w, h)| FRect { x: x as f32, y: y as f32, w: w as f32, h: h as f32 })
}

/// The part `crop` of `img`; a crop that covers none of it gives one
/// transparent pixel (fail closed: never the whole picture).
pub fn crop(img: &PixBuf, r: FRect) -> PixBuf {
    match crop_px(r, img.dimensions()) {
        Some((0, 0, w, h)) if (w, h) == img.dimensions() => img.clone(),
        Some((x, y, w, h)) => img.crop(x, y, w, h),
        None => PixBuf::new(1, 1),
    }
}

/// The picture's area outside `crop` as up to four bands (top, bottom,
/// left, right) on whole pixels: what an older build must not show.
pub fn outside(crop: FRect, size: (u32, u32)) -> Vec<FRect> {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let Some(c) = crop_rect(crop, size) else { return vec![FRect { x: 0.0, y: 0.0, w, h }] };
    let mut v = vec![
        FRect { x: 0.0, y: 0.0, w, h: c.y },
        FRect { x: 0.0, y: c.y + c.h, w, h: h - c.y - c.h },
        FRect { x: 0.0, y: c.y, w: c.x, h: c.h },
        FRect { x: c.x + c.w, y: c.y, w: w - c.x - c.w, h: c.h },
    ];
    v.retain(|r| r.w > 0.0 && r.h > 0.0);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, s: f32) -> Vec<Pt> {
        vec![Pt::new(x, y), Pt::new(x + s, y), Pt::new(x + s, y + s), Pt::new(x, y + s)]
    }

    #[test]
    fn coverage_is_exact_on_pixel_edges_and_soft_between() {
        let c = coverage(&square(2.0, 2.0, 4.0), 8, 8, Pt::new(0.0, 0.0));
        let at = |x: usize, y: usize| c[y * 8 + x];
        assert_eq!((at(2, 2), at(5, 5), at(1, 1), at(6, 3)), (255, 255, 0, 0));
        assert_eq!(c.iter().filter(|v| **v == 255).count(), 16);
        let half = coverage(&square(2.5, 2.0, 4.0), 8, 8, Pt::new(0.0, 0.0));
        assert!((120..=135).contains(&half[2 * 8 + 2]), "a half-covered pixel: {}", half[2 * 8 + 2]);
        // The origin moves the grid; a polygon past the sides still fills.
        let o = coverage(&square(-10.0, 0.0, 14.0), 8, 8, Pt::new(0.0, 0.0));
        assert_eq!((o[0], o[3], o[4]), (255, 255, 0));
        assert_eq!(coverage(&square(10.0, 10.0, 4.0), 4, 4, Pt::new(10.0, 10.0)).iter().filter(|v| **v == 255).count(), 16);
        assert!(coverage(&[Pt::new(0.0, 0.0), Pt::new(5.0, 5.0)], 8, 8, Pt::new(0.0, 0.0)).iter().all(|v| *v == 0), "no shape: nothing");
    }

    #[test]
    fn outside_the_lasso_is_transparent_then_white() {
        let mut img = PixBuf::from_pixel(8, 8, [10, 20, 30, 255]);
        // A triangle: the corners outside it go.
        let tri = vec![Pt::new(0.0, 0.0), Pt::new(8.0, 0.0), Pt::new(0.0, 8.0)];
        apply(&mut img, &tri, Pt::new(0.0, 0.0));
        assert_eq!(img.get_pixel(0, 0), [10, 20, 30, 255]);
        assert_eq!(img.get_pixel(7, 7), [255, 255, 255, 0]);
        let edge = img.get_pixel(3, 4)[3];
        assert!(edge > 0 && edge < 255, "anti-aliased edge: {edge}");
        assert!(has_alpha(&img));
        let flat = flatten_white(&img);
        assert_eq!(flat.get_pixel(7, 7), [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(0, 0), [10, 20, 30, 255]);
        let e = flat.get_pixel(3, 4);
        assert!(e[0] > 10 && e[3] == 255, "{e:?}");
        let opaque = PixBuf::from_pixel(2, 2, [1, 2, 3, 255]);
        assert!(matches!(flatten_white(&opaque), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn crops_stay_inside_and_the_rest_is_four_bands() {
        let r = FRect { x: 10.2, y: -5.0, w: 30.0, h: 40.0 };
        assert_eq!(crop_px(r, (100, 50)), Some((10, 0, 30, 35)));
        assert_eq!(crop_px(FRect { x: 200.0, y: 0.0, w: 5.0, h: 5.0 }, (100, 50)), None);
        let img = PixBuf::from_pixel(100, 50, [0, 0, 0, 255]);
        assert_eq!(crop(&img, r).dimensions(), (30, 35));
        assert_eq!(crop(&img, FRect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }).dimensions(), (100, 50));
        let off = crop(&img, FRect { x: 500.0, y: 0.0, w: 5.0, h: 5.0 });
        assert_eq!((off.dimensions(), off.as_raw()[3]), ((1, 1), 0), "outside the picture: nothing shown");
        let o = outside(FRect { x: 10.0, y: 5.0, w: 30.0, h: 20.0 }, (100, 50));
        assert_eq!(o.len(), 4);
        let sum: f32 = o.iter().map(|b| b.w * b.h).sum();
        assert_eq!(sum, 100.0 * 50.0 - 30.0 * 20.0, "everything but the crop, once");
        assert_eq!(outside(FRect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 }, (100, 50)), Vec::<FRect>::new());
        assert_eq!(outside(FRect { x: 0.0, y: 0.0, w: 100.0, h: 20.0 }, (100, 50)).len(), 1);
    }

    #[test]
    fn a_drag_is_thinned_and_measured() {
        let pts: Vec<Pt> = (0..100).map(|i| Pt::new(i as f32 * 0.5, 0.0)).collect();
        let s = simplify(&pts);
        assert!(s.len() < 40 && s.first() == pts.first() && s.last() == pts.last(), "{}", s.len());
        assert_eq!(bbox(&square(3.0, 4.0, 5.0)), FRect { x: 3.0, y: 4.0, w: 5.0, h: 5.0 });
        assert_eq!(area(&square(0.0, 0.0, 4.0)).abs(), 16.0);
        assert!(is_shape(&square(0.0, 0.0, 4.0)) && !is_shape(&square(0.0, 0.0, 1.0)));
        assert_eq!(moved(&[Pt::new(1.0, 2.0)], 3.0, -1.0), vec![Pt::new(4.0, 1.0)]);
    }
}
