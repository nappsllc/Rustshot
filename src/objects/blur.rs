//! The blur object: three box blurs each way, which together approximate
//! a Gaussian of standard deviation `sigma` (the box widths of
//! [`boxes`]). It reads only the pixels of its own rect (on whole pixels,
//! [`px_rect`]), its edges clamped, so it renders the same in any window
//! holding the whole rect, in either channel order (all four channels
//! blur alike), on the software and the GDI path.
//!
//! Each pass keeps a running sum: a row pass walks the row once, a column
//! pass keeps a sum per column and walks the rows (row by row, so it runs
//! over contiguous memory). Averages are exact rounded divisions
//! ([`Div`]).

use super::FRect;
use crate::raster::Surf;

/// The blur's pixel rect `(x0, y0, x1, y1)` (end-exclusive; not clipped
/// to the image): what it reads and writes.
pub fn px_rect(r: FRect) -> (i32, i32, i32, i32) {
    (r.x.max(0.0).floor() as i32, r.y.max(0.0).floor() as i32, r.x1().ceil() as i32, r.y1().ceil() as i32)
}

/// The widths (odd) of three box blurs approximating a Gaussian of
/// standard deviation `sigma` (the variance of the three boxes adds up
/// to sigma²: W3C / Kovesi).
pub fn boxes(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let s2 = sigma.max(0.0).powi(2);
    let ideal = (12.0 * s2 / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let wlf = wl as f32;
    let m = ((12.0 * s2 - n * wlf * wlf - 4.0 * n * wlf - 3.0 * n) / (-4.0 * wlf - 4.0)).round().clamp(0.0, 3.0) as usize;
    let mut out = [0usize; 3];
    for (i, o) in out.iter_mut().enumerate() {
        *o = if i < m { wl as usize } else { wu as usize };
    }
    out
}

/// Exact rounded division by an odd box width `d` (`(s + d / 2) / d` for
/// every sum of up to 255 * d), by a multiply and a shift.
#[derive(Clone, Copy)]
pub struct Div {
    d: u32,
    m: u64,
}

impl Div {
    pub fn new(d: u32) -> Div {
        let d = d.max(1);
        Div { d, m: (1u64 << 32).div_ceil(d as u64) }
    }

    #[inline]
    pub fn avg(self, s: u32) -> u8 {
        (((s + self.d / 2) as u64 * self.m) >> 32) as u8
    }
}

/// One box pass along a row of `n` pixels (`src` -> `dst`, RGBA-like
/// quadruples), half-width `r`, the ends clamped.
fn row_pass(src: &[[u8; 4]], dst: &mut [[u8; 4]], r: usize, dv: Div) {
    let n = src.len();
    if n == 0 {
        return;
    }
    let last = n - 1;
    let mut s = [0u32; 4];
    for c in 0..4 {
        s[c] = src[0][c] as u32 * (r as u32 + 1);
    }
    for j in 1..=r {
        let p = src[j.min(last)];
        for c in 0..4 {
            s[c] += p[c] as u32;
        }
    }
    for i in 0..n {
        let o = &mut dst[i];
        for c in 0..4 {
            o[c] = dv.avg(s[c]);
        }
        let add = src[(i + r + 1).min(last)];
        let sub = src[i.saturating_sub(r)];
        for c in 0..4 {
            s[c] = s[c] + add[c] as u32 - sub[c] as u32;
        }
    }
}

/// One box pass down the columns of a `w` x `h` block (`src` -> `dst`,
/// `w * 4` bytes a row), half-height `r`, the ends clamped. `acc` is
/// scratch of `w * 4` sums.
fn col_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, dv: Div, acc: &mut Vec<u32>) {
    if w == 0 || h == 0 {
        return;
    }
    let n = w * 4;
    let row = |y: usize| &src[y.min(h - 1) * n..][..n];
    acc.clear();
    acc.extend(row(0).iter().map(|&v| v as u32 * (r as u32 + 1)));
    for j in 1..=r {
        for (a, &v) in acc.iter_mut().zip(row(j)) {
            *a += v as u32;
        }
    }
    for y in 0..h {
        let out = &mut dst[y * n..][..n];
        for (o, &a) in out.iter_mut().zip(acc.iter()) {
            *o = dv.avg(a);
        }
        let (add, sub) = (row(y + r + 1), row(y.saturating_sub(r)));
        for ((a, &p), &m) in acc.iter_mut().zip(add).zip(sub) {
            *a = *a + p as u32 - m as u32;
        }
    }
}

/// Blur `block` (`w` x `h` pixels, `w * 4` bytes a row) in place:
/// three row passes, then three column passes. `tmp` is scratch.
pub fn blur_block(block: &mut [u8], w: usize, h: usize, sigma: f32, tmp: &mut Vec<u8>, acc: &mut Vec<u32>) {
    if w == 0 || h == 0 || sigma <= 0.0 {
        return;
    }
    let bs = boxes(sigma);
    if bs.iter().all(|&b| b <= 1) {
        return;
    }
    let len = w * h * 4;
    tmp.clear();
    tmp.resize(len, 0);
    let block = &mut block[..len];
    // Row passes: ping-pong between the block and the scratch.
    let mut in_block = true;
    for &b in &bs {
        let dv = Div::new(b as u32);
        let r = b / 2;
        let (src, dst): (&[u8], &mut [u8]) = if in_block { (&*block, &mut tmp[..]) } else { (&tmp[..], &mut *block) };
        // (Two borrows of different buffers: re-split to satisfy both.)
        let (src, dst) = (src.as_chunks::<4>().0, dst.as_chunks_mut::<4>().0);
        for (s, d) in src.chunks_exact(w).zip(dst.chunks_exact_mut(w)) {
            row_pass(s, d, r, dv);
        }
        in_block = !in_block;
    }
    for &b in &bs {
        let dv = Div::new(b as u32);
        let r = b / 2;
        if in_block {
            col_pass(block, tmp, w, h, r, dv, acc);
        } else {
            col_pass(tmp, block, w, h, r, dv, acc);
        }
        in_block = !in_block;
    }
    if !in_block {
        block.copy_from_slice(&tmp[..len]);
    }
}

/// Blur the pixels of `r` in the surface (the part of its pixel rect the
/// surface holds: exact when the surface holds all of it).
pub fn render(sf: &mut Surf, r: FRect, sigma: f32) {
    let (bx0, by0, bx1, by1) = sf.bounds();
    let (x0, y0, x1, y1) = px_rect(r);
    let (x0, y0, x1, y1) = (x0.max(bx0), y0.max(by0), x1.min(bx1), y1.min(by1));
    if x1 <= x0 || y1 <= y0 || !sigma.is_finite() {
        return;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let pw = sf.width() as usize;
    thread_local! {
        static SCRATCH: std::cell::RefCell<(Vec<u8>, Vec<u8>, Vec<u32>)> = const { std::cell::RefCell::new((Vec::new(), Vec::new(), Vec::new())) };
    }
    SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        let (block, tmp, acc) = &mut *s;
        block.clear();
        block.reserve(w * h * 4);
        let data = &mut sf.data;
        for y in y0..y1 {
            let i = ((y - by0) as usize * pw + (x0 - bx0) as usize) * 4;
            block.extend_from_slice(&data[i..i + w * 4]);
        }
        blur_block(block, w, h, sigma, tmp, acc);
        for (k, y) in (y0..y1).enumerate() {
            let i = ((y - by0) as usize * pw + (x0 - bx0) as usize) * 4;
            data[i..i + w * 4].copy_from_slice(&block[k * w * 4..][..w * 4]);
        }
        // A big drag's scratch is not kept for ever.
        if block.capacity() > 64 << 20 {
            *block = Vec::new();
            *tmp = Vec::new();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The passes the slow way: every output the clamped window's sum.
    fn reference(src: &[u8], w: usize, h: usize, sigma: f32) -> Vec<u8> {
        let mut cur = src.to_vec();
        let bs = boxes(sigma);
        let at = |v: &Vec<u8>, x: isize, y: isize, c: usize| v[((y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize) * 4 + c] as u32;
        for vertical in [false, true] {
            for &b in &bs {
                let (dv, r) = (Div::new(b as u32), (b / 2) as isize);
                let mut next = cur.clone();
                for y in 0..h as isize {
                    for x in 0..w as isize {
                        for c in 0..4 {
                            let s: u32 = (-r..=r).map(|k| if vertical { at(&cur, x, y + k, c) } else { at(&cur, x + k, y, c) }).sum();
                            next[(y as usize * w + x as usize) * 4 + c] = dv.avg(s);
                        }
                    }
                }
                cur = next;
            }
        }
        cur
    }

    fn noise(w: usize, h: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..w * h * 4)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s >> 8) as u8
            })
            .collect()
    }

    #[test]
    fn division_is_exact_rounding() {
        for d in (1..=255u32).step_by(2) {
            let dv = Div::new(d);
            for s in 0..=255 * d {
                assert_eq!(dv.avg(s) as u32, (s + d / 2) / d, "{s} / {d}");
            }
        }
    }

    #[test]
    fn boxes_approximate_the_variance() {
        for sigma in [1.0f32, 2.0, 3.5, 8.0, 12.0, 25.0, 50.0] {
            let b = boxes(sigma);
            assert!(b.iter().all(|w| w % 2 == 1), "{b:?}");
            // A box of width w has variance (w² - 1) / 12.
            let var: f32 = b.iter().map(|&w| ((w * w) as f32 - 1.0) / 12.0).sum();
            assert!((var.sqrt() - sigma).abs() < 0.6 + sigma * 0.05, "sigma {sigma}: {b:?} gives {}", var.sqrt());
        }
        assert_eq!(boxes(0.0), [1, 1, 1]);
    }

    /// The running sums give what summing every clamped window gives,
    /// rects narrower than the box included.
    #[test]
    fn passes_match_the_reference() {
        for (w, h, sigma) in [(37usize, 23usize, 3.0f32), (5, 40, 6.0), (64, 3, 2.5), (1, 1, 4.0), (9, 9, 12.0)] {
            let src = noise(w, h, (w * 31 + h) as u32);
            let want = reference(&src, w, h, sigma);
            let mut got = src.clone();
            blur_block(&mut got, w, h, sigma, &mut Vec::new(), &mut Vec::new());
            assert!(got == want, "{w}x{h} sigma {sigma}");
        }
    }

    /// Close to a true Gaussian: an impulse spreads as one (within a few
    /// levels), and a flat area stays flat.
    #[test]
    fn approximates_a_gaussian() {
        let (w, h, sigma) = (81usize, 81usize, 6.0f32);
        let mut src = vec![0u8; w * h * 4];
        // A bright 9x9 square (an impulse would round away).
        for y in 36..45 {
            for x in 36..45 {
                src[(y * w + x) * 4..][..4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let mut got = src.clone();
        blur_block(&mut got, w, h, sigma, &mut Vec::new(), &mut Vec::new());
        let g = |d: f32| (-(d * d) / (2.0 * sigma * sigma)).exp();
        let norm: f32 = (-40..=40).map(|k| g(k as f32)).sum();
        // The 1-D Gaussian of a 9 px box, squared (separable).
        let line = |x: f32| (36..45).map(|k| g(x - k as f32)).sum::<f32>() / norm;
        let mut worst = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                let want = 255.0 * line(x as f32) * line(y as f32);
                worst = worst.max((got[(y * w + x) * 4] as f32 - want).abs());
            }
        }
        assert!(worst <= 6.0, "max error {worst}");
        let mut flat = vec![77u8; 30 * 20 * 4];
        blur_block(&mut flat, 30, 20, 9.0, &mut Vec::new(), &mut Vec::new());
        assert!(flat.iter().all(|&v| v == 77));
    }

    /// In a window holding the whole rect, offset and in BGRA, the blur
    /// is what the whole image gets; it touches only its rect.
    #[test]
    fn renders_the_same_in_any_window() {
        use crate::raster::Order;
        let (w, h) = (90u32, 70u32);
        let img = noise(w as usize, h as usize, 9);
        let r = FRect { x: 10.4, y: 7.0, w: 50.2, h: 40.6 };
        let mut full = img.clone();
        render(&mut Surf::with_origin(&mut full, w, h, 0, 0, Order::Rgba), r, 5.0);
        let (x0, y0, x1, y1) = px_rect(r);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = ((y * w as i32 + x) * 4) as usize;
                let inside = x >= x0 && x < x1 && y >= y0 && y < y1;
                if !inside {
                    assert_eq!(full[i..i + 4], img[i..i + 4], "({x}, {y}) outside changed");
                }
            }
        }
        assert!(full != img);
        // A window from (4, 3) to (75, 60), BGRA.
        let (wx, wy, ww, wh) = (4i32, 3i32, 71u32, 57u32);
        let mut win = vec![0u8; (ww * wh * 4) as usize];
        for y in 0..wh as i32 {
            for x in 0..ww as i32 {
                let s = (((y + wy) * w as i32 + x + wx) * 4) as usize;
                let d = ((y * ww as i32 + x) * 4) as usize;
                win[d..d + 4].copy_from_slice(&[img[s + 2], img[s + 1], img[s], img[s + 3]]);
            }
        }
        render(&mut Surf::with_origin(&mut win, ww, wh, wx, wy, Order::Bgra), r, 5.0);
        for y in 0..wh as i32 {
            for x in 0..ww as i32 {
                let s = (((y + wy) * w as i32 + x + wx) * 4) as usize;
                let d = ((y * ww as i32 + x) * 4) as usize;
                assert_eq!([win[d + 2], win[d + 1], win[d], win[d + 3]], full[s..s + 4], "({x}, {y})");
            }
        }
    }
}
