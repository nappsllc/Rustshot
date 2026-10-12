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
//! ([`Div`]) in 16 bits. The release build optimizes for size, so the
//! inner loops are SSE2 by hand on x86-64 (scalar elsewhere), and a big
//! block is split over a few threads; an 800x400 blur takes about 1.6 ms.

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

/// The widest box [`Div`] divides exactly by.
pub const MAX_BOX: u32 = 127;

/// Exact rounded division by an odd box width `d` (3 to [`MAX_BOX`]):
/// `(s + d / 2) / d` for every sum `s` of up to 255 * d, in 16 bits (the
/// high half of a 16 x 16 multiply, then a shift: SSE2's `pmulhuw`, so
/// the column passes run eight lanes wide). Width 1 is the identity.
#[derive(Clone, Copy)]
pub struct Div {
    half: u16,
    m: u16,
    k: u32,
}

impl Div {
    pub fn new(d: u32) -> Div {
        let d = d.clamp(1, MAX_BOX);
        if d == 1 {
            return Div { half: 0, m: 0, k: 0 };
        }
        // The smallest shift exact for every x <= 255.5 d (x * d < 2^(16+k));
        // then 2^k < d keeps the multiplier in 16 bits.
        let mut k = 0;
        while 255.5 * (d * d) as f64 >= (1u64 << (16 + k)) as f64 {
            k += 1;
        }
        debug_assert!(1u32 << k < d);
        Div { half: (d / 2) as u16, m: (1u32 << (16 + k)).div_ceil(d) as u16, k }
    }

    #[inline(always)]
    pub fn avg(self, s: u16) -> u8 {
        if self.m == 0 {
            return s as u8;
        }
        ((((s + self.half) as u32 * self.m as u32) >> 16) as u16 >> self.k) as u8
    }
}

/// One box pass down the columns of a `w` x `h` block (`src` -> `dst`,
/// `w * 4` bytes a row), half-height `r` (at least 1), the ends clamped.
/// `acc` is scratch of `w * 4` sums (16 bits: at most 255 * 127).
#[cfg(test)]
fn col_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, dv: Div, acc: &mut Vec<u16>) {
    let n = w * 4;
    assert!(src.len() >= n * h && dst.len() >= n * h);
    // SAFETY: both buffers hold h rows of n bytes (asserted).
    unsafe { col_range(src.as_ptr(), dst.as_mut_ptr(), n, h, (0, n), r, dv, acc) }
}

/// [`col_pass`] over the byte columns `cols` of rows `n` bytes apart, by
/// pointers: the slab threads each run their own columns of one block.
///
/// # Safety
/// `src` and `dst` hold `h` rows of `n` bytes, `cols` lies within `n`,
/// and nothing else touches those columns of `dst` meanwhile.
#[allow(clippy::too_many_arguments)]
unsafe fn col_range(src: *const u8, dst: *mut u8, n: usize, h: usize, cols: (usize, usize), r: usize, dv: Div, acc: &mut Vec<u16>) {
    let (c0, m) = (cols.0, cols.1 - cols.0);
    if m == 0 || h == 0 {
        return;
    }
    // SAFETY: rows y < h at columns c0..c0 + m (the caller's promise).
    let row = |y: usize| unsafe { std::slice::from_raw_parts(src.add(y.min(h - 1) * n + c0), m) };
    acc.clear();
    acc.extend(row(0).iter().map(|&v| v as u16 * (r as u16 + 1)));
    for j in 1..=r {
        for (a, &v) in acc.iter_mut().zip(row(j)) {
            *a += v as u16;
        }
    }
    let acc = &mut acc[..m];
    for y in 0..h {
        // SAFETY: as above; these columns of `dst` are this caller's.
        let out = unsafe { std::slice::from_raw_parts_mut(dst.add(y * n + c0), m) };
        row_step(out, acc, row(y + r + 1), row(y.saturating_sub(r)), dv);
    }
}

/// Two blocks' pointers, shared by the slab threads (each writes only its
/// own columns).
#[derive(Clone, Copy)]
struct Bufs(*mut u8, *mut u8);
// SAFETY: the threads touch disjoint columns ([`col_range`]).
unsafe impl Send for Bufs {}
unsafe impl Sync for Bufs {}

/// One box pass along a row of pixels (`src` -> `dst`), half-width `r`,
/// the ends clamped: a running sum of the four channels (SSE2: one
/// register of four 16-bit lanes on x86-64).
#[cfg(target_arch = "x86_64")]
fn row_pass(src: &[[u8; 4]], dst: &mut [[u8; 4]], r: usize, dv: Div) {
    use std::arch::x86_64::*;
    let n = src.len().min(dst.len());
    if n == 0 {
        return;
    }
    let last = n - 1;
    #[inline(always)]
    fn px(p: [u8; 4]) -> __m128i {
        // SAFETY: SSE2 is part of x86-64.
        unsafe { _mm_unpacklo_epi8(_mm_cvtsi32_si128(i32::from_ne_bytes(p)), _mm_setzero_si128()) }
    }
    // SAFETY: SSE2 is part of x86-64; indices are clamped into the row.
    unsafe {
        let (half, m, k) = (_mm_set1_epi16(dv.half as i16), _mm_set1_epi16(dv.m as i16), _mm_cvtsi32_si128(dv.k as i32));
        let mut s = _mm_mullo_epi16(px(src[0]), _mm_set1_epi16(r as i16 + 1));
        for j in 1..=r {
            s = _mm_add_epi16(s, px(src[j.min(last)]));
        }
        for i in 0..n {
            let a = _mm_srl_epi16(_mm_mulhi_epu16(_mm_add_epi16(s, half), m), k);
            dst[i] = _mm_cvtsi128_si32(_mm_packus_epi16(a, a)).to_ne_bytes();
            s = _mm_sub_epi16(_mm_add_epi16(s, px(src[(i + r + 1).min(last)])), px(src[i.saturating_sub(r)]));
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn row_pass(src: &[[u8; 4]], dst: &mut [[u8; 4]], r: usize, dv: Div) {
    let n = src.len().min(dst.len());
    if n == 0 {
        return;
    }
    let last = n - 1;
    let mut s = [0u16; 4];
    for c in 0..4 {
        s[c] = src[0][c] as u16 * (r as u16 + 1);
    }
    for j in 1..=r {
        for c in 0..4 {
            s[c] += src[j.min(last)][c] as u16;
        }
    }
    for i in 0..n {
        let (p, q) = (src[(i + r + 1).min(last)], src[i.saturating_sub(r)]);
        for c in 0..4 {
            dst[i][c] = dv.avg(s[c]);
            s[c] = s[c].wrapping_add(p[c] as u16).wrapping_sub(q[c] as u16);
        }
    }
}

/// One row of a column pass: `o = avg(a)`, then `a += add - sub`, all
/// slices `n` long. The release build optimizes for size (no automatic
/// vectorization): SSE2 by hand on x86-64, 16 bytes a step.
#[inline]
fn row_step(o: &mut [u8], a: &mut [u16], add: &[u8], sub: &[u8], dv: Div) {
    let n = o.len().min(a.len()).min(add.len()).min(sub.len());
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is part of x86-64; every access is in bounds
        // (`i + 16 <= n`, unaligned loads and stores).
        unsafe {
            let (half, m, k) = (_mm_set1_epi16(dv.half as i16), _mm_set1_epi16(dv.m as i16), _mm_cvtsi32_si128(dv.k as i32));
            let zero = _mm_setzero_si128();
            while i + 16 <= n {
                let pa = a.as_mut_ptr().add(i) as *mut __m128i;
                let (lo, hi) = (_mm_loadu_si128(pa), _mm_loadu_si128(pa.add(1)));
                let alo = _mm_srl_epi16(_mm_mulhi_epu16(_mm_add_epi16(lo, half), m), k);
                let ahi = _mm_srl_epi16(_mm_mulhi_epu16(_mm_add_epi16(hi, half), m), k);
                _mm_storeu_si128(o.as_mut_ptr().add(i) as *mut __m128i, _mm_packus_epi16(alo, ahi));
                let p = _mm_loadu_si128(add.as_ptr().add(i) as *const __m128i);
                let q = _mm_loadu_si128(sub.as_ptr().add(i) as *const __m128i);
                let lo = _mm_sub_epi16(_mm_add_epi16(lo, _mm_unpacklo_epi8(p, zero)), _mm_unpacklo_epi8(q, zero));
                let hi = _mm_sub_epi16(_mm_add_epi16(hi, _mm_unpackhi_epi8(p, zero)), _mm_unpackhi_epi8(q, zero));
                _mm_storeu_si128(pa, lo);
                _mm_storeu_si128(pa.add(1), hi);
                i += 16;
            }
        }
    }
    while i < n {
        o[i] = dv.avg(a[i]);
        a[i] = a[i].wrapping_add(add[i] as u16).wrapping_sub(sub[i] as u16);
        i += 1;
    }
}

/// Blur `block` (`w` x `h` pixels, `w * 4` bytes a row) in place: three
/// row passes, then three column passes. A big block is shared by up to
/// eight threads: rows in bands, then (after a barrier) columns in slabs;
/// rows and columns never read each other within a phase, so the result is
/// exactly the one-thread one. `tmp` is scratch.
pub fn blur_block(block: &mut [u8], w: usize, h: usize, sigma: f32, tmp: &mut Vec<u8>, acc: &mut Vec<u16>) {
    if w == 0 || h == 0 || sigma <= 0.0 {
        return;
    }
    // (The boxes stay within what `Div` divides by; width 1 is no pass.)
    let bs = boxes(sigma.min(60.0));
    let ws: Vec<usize> = bs.iter().copied().filter(|&b| b > 1).collect();
    if ws.is_empty() {
        return;
    }
    let (n, len) = (w * 4, w * h * 4);
    // Scratch is written before it is read: grown, never cleared.
    if tmp.len() < len {
        tmp.resize(len, 0);
    }
    assert!(block.len() >= len);
    let bufs = Bufs(block.as_mut_ptr(), tmp.as_mut_ptr());
    let threads = if w * h >= 60_000 { std::thread::available_parallelism().map_or(1, |p| p.get()).min(8) } else { 1 };
    // Column slabs on 16-byte bounds, at least 256 bytes wide; row bands
    // at least 4 rows.
    let k = threads.min(n / 256).min(h / 4).max(1);
    let edge = move |i: usize| if i == k { n } else { (n * i / k) & !15 };
    let ws = &ws;
    let work = move |i: usize, barrier: Option<&std::sync::Barrier>, acc: &mut Vec<u16>| {
        let bufs = bufs;
        let (mut ra, mut rb) = (vec![[0u8; 4]; w], vec![[0u8; 4]; w]);
        for y in h * i / k..h * (i + 1) / k {
            // SAFETY: row y < h of the block; the rows of a band are this
            // thread's alone in this phase.
            let row = unsafe { std::slice::from_raw_parts_mut(bufs.0.add(y * n) as *mut [u8; 4], w) };
            ra.copy_from_slice(row);
            for &b in ws {
                row_pass(&ra, &mut rb, b / 2, Div::new(b as u32));
                std::mem::swap(&mut ra, &mut rb);
            }
            row.copy_from_slice(&ra);
        }
        if let Some(b) = barrier {
            b.wait();
        }
        let cols = (edge(i), edge(i + 1));
        let mut in_block = true;
        for &b in ws {
            let (src, dst) = if in_block { (bufs.0, bufs.1) } else { (bufs.1, bufs.0) };
            // SAFETY: both buffers hold h rows of n bytes (asserted and
            // grown); `cols` is this slab's alone in this phase.
            unsafe { col_range(src, dst, n, h, cols, b / 2, Div::new(b as u32), acc) };
            in_block = !in_block;
        }
        if !in_block {
            for y in 0..h {
                // SAFETY: as above, this slab's columns of row y.
                unsafe { std::ptr::copy_nonoverlapping(bufs.1.add(y * n + cols.0), bufs.0.add(y * n + cols.0), cols.1 - cols.0) };
            }
        }
    };
    if k == 1 {
        work(0, None, acc);
        return;
    }
    let barrier = std::sync::Barrier::new(k);
    let barrier = &barrier;
    std::thread::scope(|s| {
        for i in 1..k {
            s.spawn(move || work(i, Some(barrier), &mut Vec::new()));
        }
        work(0, Some(barrier), acc);
    });
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
        static SCRATCH: std::cell::RefCell<(Vec<u8>, Vec<u8>, Vec<u16>)> = const { std::cell::RefCell::new((Vec::new(), Vec::new(), Vec::new())) };
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

    /// The parts' cost at 800x400.
    /// `cargo test --release blur_parts_bench -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark; run in release"]
    fn blur_parts_bench() {
        let (w, h) = (800usize, 400usize);
        let src: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 251) as u8).collect();
        let mut dst = vec![0u8; w * h * 4];
        let mut acc = Vec::new();
        let t = std::time::Instant::now();
        for _ in 0..20 {
            col_pass(&src, &mut dst, w, h, 5, Div::new(11), &mut acc);
        }
        println!("BENCH col_pass {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
        let t = std::time::Instant::now();
        for _ in 0..20 {
            for (s, d) in src.as_chunks::<4>().0.chunks_exact(w).zip(dst.as_chunks_mut::<4>().0.chunks_exact_mut(w)) {
                row_pass(s, d, 5, Div::new(11));
            }
        }
        println!("BENCH row passes {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
        let (mut blk, mut tmp) = (src.clone(), Vec::new());
        let t = std::time::Instant::now();
        for _ in 0..20 {
            blur_block(&mut blk, w, h, 10.0, &mut tmp, &mut acc);
        }
        println!("BENCH blur_block {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    }

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
                            next[(y as usize * w + x as usize) * 4 + c] = dv.avg(s as u16);
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
        for d in (1..=MAX_BOX).step_by(2) {
            let dv = Div::new(d);
            for s in 0..=255 * d {
                assert_eq!(dv.avg(s as u16) as u32, (s + d / 2) / d, "{s} / {d}");
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
        assert!(boxes(60.0).iter().all(|&b| b as u32 <= MAX_BOX), "the widest blur divides exactly");
    }

    /// The running sums give what summing every clamped window gives,
    /// rects narrower than the box included.
    #[test]
    fn passes_match_the_reference() {
        // (301 x 220: big enough for the threads, odd so slabs and bands split unevenly.)
        for (w, h, sigma) in [(37usize, 23usize, 3.0f32), (5, 40, 6.0), (64, 3, 2.5), (1, 1, 4.0), (9, 9, 12.0), (301, 220, 5.0)] {
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
