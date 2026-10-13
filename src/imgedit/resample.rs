//! Resampling for Resize and Save at: separable filters with precomputed
//! fixed-point weights, both passes split over threads.
//!
//! Downscale: Lanczos-3 (sharp; the kernel widened by the scale so every
//! source pixel counts) or an exact area average (each destination pixel
//! the mean of the source area it covers, fractional edges included).
//! Upscale: always bilinear (no sharpening, no ringing). Each axis picks
//! its own kernel, so a squash that grows one side and shrinks the other
//! works too. Translucent pictures are resampled premultiplied.

use crate::pixbuf::PixBuf;

/// The downscale filter (an upscale is bilinear whatever is chosen).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// Lanczos-3: sharp, a little ringing at hard edges.
    #[default]
    Lanczos3,
    /// Area average: smooth, no ringing (Save at 1× uses it).
    Area,
}

impl Filter {
    pub fn id(self) -> &'static str {
        match self {
            Filter::Lanczos3 => "lanczos3",
            Filter::Area => "area",
        }
    }

    pub fn from_id(s: &str) -> Option<Filter> {
        match s {
            "lanczos3" => Some(Filter::Lanczos3),
            "area" => Some(Filter::Area),
            _ => None,
        }
    }
}

/// Fixed-point weights: 1.0 is `1 << BITS`.
const BITS: u32 = 14;
const ONE: i32 = 1 << BITS;

/// The weights of one axis: for destination index `i`, taps
/// `start[i]..start[i] + n[i]` with weights `w[i * taps..]`.
struct Axis {
    start: Vec<usize>,
    n: Vec<usize>,
    taps: usize,
    w: Vec<i32>,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let p = std::f64::consts::PI * x;
        p.sin() / p
    }
}

fn lanczos3(x: f64) -> f64 {
    if x.abs() >= 3.0 { 0.0 } else { sinc(x) * sinc(x / 3.0) }
}

fn triangle(x: f64) -> f64 {
    (1.0 - x.abs()).max(0.0)
}

impl Axis {
    /// `src` pixels onto `dst` with `f` (when shrinking) or bilinear.
    fn new(src: usize, dst: usize, f: Filter) -> Axis {
        let scale = src as f64 / dst as f64;
        let mut rows: Vec<(usize, Vec<f64>)> = Vec::with_capacity(dst);
        for i in 0..dst {
            let (lo, ws): (i64, Vec<f64>) = if scale > 1.0 && f == Filter::Area {
                let (x0, x1) = (i as f64 * scale, (i + 1) as f64 * scale);
                let (a, b) = (x0.floor() as i64, x1.ceil() as i64);
                (a, (a..b).map(|j| ((j + 1) as f64).min(x1) - (j as f64).max(x0)).collect())
            } else {
                let (kernel, support): (fn(f64) -> f64, f64) = if scale > 1.0 { (lanczos3, 3.0) } else { (triangle, 1.0) };
                let fs = scale.max(1.0);
                let c = (i as f64 + 0.5) * scale;
                let (a, b) = ((c - support * fs).floor() as i64, (c + support * fs).ceil() as i64);
                (a, (a..b).map(|j| kernel((j as f64 + 0.5 - c) / fs)).collect())
            };
            // Taps past the edges fold onto the edge pixels (a window as
            // long as the taps, not the whole axis).
            let clamp = |k: usize| (lo + k as i64).clamp(0, src as i64 - 1) as usize;
            let (base, top) = (clamp(0), clamp(ws.len().saturating_sub(1)));
            let mut folded = vec![0.0; top - base + 1];
            let (mut first, mut last) = (src, 0);
            for (k, w) in ws.iter().enumerate() {
                let j = clamp(k);
                folded[j - base] += w;
                if *w != 0.0 {
                    first = first.min(j);
                    last = last.max(j);
                }
            }
            if first > last {
                let j = (((i as f64 + 0.5) * scale) as usize).min(src - 1);
                rows.push((j, vec![1.0]));
                continue;
            }
            rows.push((first, folded[first - base..=last - base].to_vec()));
        }
        let taps = rows.iter().map(|r| r.1.len()).max().unwrap_or(1);
        let mut out = Axis { start: Vec::with_capacity(dst), n: Vec::with_capacity(dst), taps, w: vec![0; dst * taps] };
        for (i, (first, ws)) in rows.into_iter().enumerate() {
            let sum: f64 = ws.iter().sum();
            let sum = if sum.abs() < 1e-12 { 1.0 } else { sum };
            let q = &mut out.w[i * taps..i * taps + ws.len()];
            let mut acc = 0;
            for (o, w) in q.iter_mut().zip(&ws) {
                *o = (w / sum * ONE as f64).round() as i32;
                acc += *o;
            }
            // Rounding left over goes to the largest weight: a flat
            // picture stays exactly flat.
            if let Some(m) = (0..ws.len()).max_by_key(|&k| q[k]) {
                q[m] += ONE - acc;
            }
            out.start.push(first);
            out.n.push(ws.len());
        }
        out
    }
}

#[cfg(any(not(target_arch = "x86_64"), test))]
fn clamp8(v: i32) -> u8 {
    ((v + (ONE >> 1)) >> BITS).clamp(0, 255) as u8
}

/// The worker threads for `work` units of output (one for a small job).
fn threads(work: usize) -> usize {
    if work < 200_000 {
        return 1;
    }
    std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16)
}

/// Rows `src` (`sw` px each) resampled across to `dw` px into `dst`.
fn pass_x(src: &[u8], sw: usize, rows: usize, dst: &mut [u8], dw: usize, ax: &Axis) {
    let t = threads(rows * dw * ax.taps);
    let per = rows.div_ceil(t).max(1);
    std::thread::scope(|s| {
        for (ci, chunk) in dst.chunks_mut(per * dw * 4).enumerate() {
            s.spawn(move || {
                for (r, out) in chunk.chunks_exact_mut(dw * 4).enumerate() {
                    let y = ci * per + r;
                    let row = &src[y * sw * 4..(y + 1) * sw * 4];
                    for (i, o) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let (a, n) = (ax.start[i], ax.n[i]);
                        let ws = &ax.w[i * ax.taps..i * ax.taps + n];
                        *o = taps_px(&row[a * 4..(a + n) * 4], ws);
                    }
                }
            });
        }
    });
}

/// Elsewhere than x86-64: the scalar kernels (tests compare them with the
/// SSE2 ones on x86-64 too).
#[cfg(not(target_arch = "x86_64"))]
use {column_scalar as column, taps_px_scalar as taps_px};

/// One output pixel: the pixels `px` weighed by `ws`.
#[cfg(any(not(target_arch = "x86_64"), test))]
fn taps_px_scalar(px: &[u8], ws: &[i32]) -> [u8; 4] {
    let mut acc = [0i32; 4];
    for (p, &w) in px.as_chunks::<4>().0.iter().zip(ws) {
        acc[0] += p[0] as i32 * w;
        acc[1] += p[1] as i32 * w;
        acc[2] += p[2] as i32 * w;
        acc[3] += p[3] as i32 * w;
    }
    [clamp8(acc[0]), clamp8(acc[1]), clamp8(acc[2]), clamp8(acc[3])]
}

/// [`taps_px`] with SSE2 (x86-64's baseline): two taps per multiply-add.
#[cfg(target_arch = "x86_64")]
fn taps_px(px: &[u8], ws: &[i32]) -> [u8; 4] {
    use std::arch::x86_64::*;
    let n = ws.len();
    debug_assert!(px.len() >= n * 4);
    // SAFETY: SSE2 is always there on x86-64; every load reads inside
    // `px` (two pixels while two are left, else one).
    unsafe {
        let zero = _mm_setzero_si128();
        let mut acc = _mm_set1_epi32(ONE >> 1);
        let mut t = 0;
        while t + 2 <= n {
            let v = _mm_loadl_epi64(px.as_ptr().add(t * 4) as *const __m128i);
            let x = _mm_unpacklo_epi8(v, zero);
            let y = _mm_unpacklo_epi16(x, _mm_srli_si128::<8>(x));
            let w = _mm_set1_epi32(((ws[t + 1] as u32) << 16 | (ws[t] as u32 & 0xffff)) as i32);
            acc = _mm_add_epi32(acc, _mm_madd_epi16(y, w));
            t += 2;
        }
        if t < n {
            let v = _mm_cvtsi32_si128(i32::from_le_bytes([px[t * 4], px[t * 4 + 1], px[t * 4 + 2], px[t * 4 + 3]]));
            let x = _mm_unpacklo_epi8(v, zero);
            let y = _mm_unpacklo_epi16(x, zero);
            let w = _mm_set1_epi32(ws[t] & 0xffff);
            acc = _mm_add_epi32(acc, _mm_madd_epi16(y, w));
        }
        let s = _mm_srai_epi32::<{ BITS as i32 }>(acc);
        let p = _mm_packus_epi16(_mm_packs_epi32(s, zero), zero);
        _mm_cvtsi128_si32(p).to_le_bytes()
    }
}

/// One output row: `out[i]` the bytes `rows[t][i]` weighed by `ws[t]`
/// (`acc`: scratch as long as the row).
#[cfg(any(not(target_arch = "x86_64"), test))]
fn column_scalar(rows: &[&[u8]], ws: &[i32], acc: &mut [i32], out: &mut [u8]) {
    acc.fill(ONE >> 1);
    for (row, &wt) in rows.iter().zip(ws) {
        for (s, &v) in acc.iter_mut().zip(row.iter()) {
            *s += v as i32 * wt;
        }
    }
    for (o, s) in out.iter_mut().zip(acc.iter()) {
        *o = (*s >> BITS).clamp(0, 255) as u8;
    }
}

/// [`column`] with SSE2: 16 bytes and two taps per step.
#[cfg(target_arch = "x86_64")]
fn column(rows: &[&[u8]], ws: &[i32], _acc: &mut [i32], out: &mut [u8]) {
    use std::arch::x86_64::*;
    let n = out.len();
    let k = rows.len();
    let mut i = 0;
    // SAFETY: SSE2 is always there on x86-64; loads and stores stay
    // within the rows and `out` (16 bytes while 16 are left).
    unsafe {
        let zero = _mm_setzero_si128();
        let half = _mm_set1_epi32(ONE >> 1);
        while i + 16 <= n {
            let (mut a0, mut a1, mut a2, mut a3) = (half, half, half, half);
            let mut t = 0;
            while t < k {
                let r0 = _mm_loadu_si128(rows[t].as_ptr().add(i) as *const __m128i);
                let (r1, w1) = if t + 1 < k { (_mm_loadu_si128(rows[t + 1].as_ptr().add(i) as *const __m128i), ws[t + 1]) } else { (zero, 0) };
                let w = _mm_set1_epi32(((w1 as u32) << 16 | (ws[t] as u32 & 0xffff)) as i32);
                let (l0, l1) = (_mm_unpacklo_epi8(r0, zero), _mm_unpacklo_epi8(r1, zero));
                let (h0, h1) = (_mm_unpackhi_epi8(r0, zero), _mm_unpackhi_epi8(r1, zero));
                a0 = _mm_add_epi32(a0, _mm_madd_epi16(_mm_unpacklo_epi16(l0, l1), w));
                a1 = _mm_add_epi32(a1, _mm_madd_epi16(_mm_unpackhi_epi16(l0, l1), w));
                a2 = _mm_add_epi32(a2, _mm_madd_epi16(_mm_unpacklo_epi16(h0, h1), w));
                a3 = _mm_add_epi32(a3, _mm_madd_epi16(_mm_unpackhi_epi16(h0, h1), w));
                t += 2;
            }
            let sh = |a| _mm_srai_epi32::<{ BITS as i32 }>(a);
            let lo = _mm_packs_epi32(sh(a0), sh(a1));
            let hi = _mm_packs_epi32(sh(a2), sh(a3));
            _mm_storeu_si128(out.as_mut_ptr().add(i) as *mut __m128i, _mm_packus_epi16(lo, hi));
            i += 16;
        }
    }
    for (j, o) in out.iter_mut().enumerate().skip(i) {
        let s: i32 = rows.iter().zip(ws).map(|(r, &w)| r[j] as i32 * w).sum::<i32>() + (ONE >> 1);
        *o = (s >> BITS).clamp(0, 255) as u8;
    }
}

/// Columns of `src` (`w` px wide, any height) resampled down to `dh` rows.
fn pass_y(src: &[u8], w: usize, dst: &mut [u8], dh: usize, ax: &Axis) {
    let n = w * 4;
    let t = threads(dh * w * ax.taps);
    let per = dh.div_ceil(t).max(1);
    std::thread::scope(|s| {
        for (ci, chunk) in dst.chunks_mut(per * n).enumerate() {
            s.spawn(move || {
                let mut acc = vec![0i32; n];
                let mut rows: Vec<&[u8]> = Vec::with_capacity(ax.taps);
                for (r, out) in chunk.chunks_exact_mut(n).enumerate() {
                    let y = ci * per + r;
                    let (a, k) = (ax.start[y], ax.n[y]);
                    let ws = &ax.w[y * ax.taps..y * ax.taps + k];
                    rows.clear();
                    rows.extend((0..k).map(|t| &src[(a + t) * n..(a + t + 1) * n]));
                    column(&rows, ws, &mut acc, out);
                }
            });
        }
    });
}

/// Straight alpha to premultiplied (in place).
fn premultiply(d: &mut [u8]) {
    for p in d.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

/// Premultiplied back to straight alpha (in place; a ringing color over
/// its alpha is clipped to it first).
fn unpremultiply(d: &mut [u8]) {
    for p in d.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a == 0 {
            *p = [255, 255, 255, 0];
        } else if a < 255 {
            for c in &mut p[..3] {
                *c = (((*c as u32).min(a) * 255 + a / 2) / a) as u8;
            }
        }
    }
}

/// `src` resampled to exactly `dw`×`dh` (see the module).
pub fn resize(src: &PixBuf, dw: u32, dh: u32, f: Filter) -> PixBuf {
    let (dw, dh) = (dw.max(1), dh.max(1));
    let (sw, sh) = src.dimensions();
    if (sw, sh) == (dw, dh) || sw == 0 || sh == 0 {
        return if sw == 0 || sh == 0 { PixBuf::new(dw, dh) } else { src.clone() };
    }
    let alpha = src.as_raw().as_chunks::<4>().0.iter().any(|p| p[3] != 255);
    let pre;
    let mut cur: &[u8] = if alpha {
        let mut d = src.as_raw().clone();
        premultiply(&mut d);
        pre = d;
        &pre
    } else {
        src.as_raw()
    };
    let (sw, sh, dwu, dhu) = (sw as usize, sh as usize, dw as usize, dh as usize);
    // Down first (whole rows at a time, which vectorizes), then across
    // the fewer rows left.
    let mid;
    let mut h = sh;
    if dhu != sh {
        let ax = Axis::new(sh, dhu, f);
        let mut m = vec![0u8; sw * dhu * 4];
        pass_y(cur, sw, &mut m, dhu, &ax);
        mid = m;
        cur = &mid;
        h = dhu;
    }
    let mut out = if dwu != sw {
        let ax = Axis::new(sw, dwu, f);
        let mut o = vec![0u8; dwu * h * 4];
        pass_x(cur, sw, h, &mut o, dwu, &ax);
        o
    } else {
        cur.to_vec()
    };
    if alpha {
        unpremultiply(&mut out);
    }
    PixBuf::from_raw(dw, dh, out).unwrap_or_else(|| PixBuf::new(dw, dh))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth test signal (low frequencies only) at `(x, y)` of a
    /// picture `size` px across, 0..255 per channel.
    fn signal(x: f64, y: f64, size: f64) -> [f64; 3] {
        let (u, v) = (x / size, y / size);
        let tau = std::f64::consts::TAU;
        [
            127.5 + 100.0 * (tau * 1.5 * u).sin() * (tau * 1.0 * v).cos(),
            127.5 + 110.0 * (tau * (2.0 * u + 0.5 * v)).cos(),
            127.5 + 90.0 * (tau * 2.5 * v).sin(),
        ]
    }

    /// The signal sampled at pixel centers of a `w`×`h` picture whose
    /// side `w` stands for `size` units.
    fn sampled(w: u32, h: u32, size: f64) -> PixBuf {
        let k = size / w as f64;
        let mut p = PixBuf::new(w, h);
        for (i, px) in p.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = ((i % w as usize) as f64 + 0.5, (i / w as usize) as f64 + 0.5);
            let s = signal(x * k, y * k, size);
            *px = [s[0].round() as u8, s[1].round() as u8, s[2].round() as u8, 255];
        }
        p
    }

    fn psnr(a: &PixBuf, b: &PixBuf) -> f64 {
        assert_eq!(a.dimensions(), b.dimensions());
        let mut se = 0.0;
        let mut n = 0.0;
        for (p, q) in a.as_raw().as_chunks::<4>().0.iter().zip(b.as_raw().as_chunks::<4>().0) {
            for c in 0..3 {
                let d = p[c] as f64 - q[c] as f64;
                se += d * d;
                n += 1.0;
            }
        }
        let mse = se / n;
        if mse == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
    }

    /// Resampling quality on synthetic pictures: each result against the
    /// same signal sampled directly at the new size.
    #[test]
    fn quality_against_the_signal() {
        let size = 512.0;
        let big = sampled(512, 384, size);
        for f in [Filter::Lanczos3, Filter::Area] {
            for (w, h) in [(256, 192), (171, 128), (100, 75)] {
                let out = resize(&big, w, h, f);
                let p = psnr(&out, &sampled(w, h, size));
                assert!(p > 38.0, "{f:?} 512x384 -> {w}x{h}: PSNR {p:.1} dB");
            }
        }
        // Upscale (bilinear): against the signal at the large size.
        let small = sampled(128, 96, size);
        let up = resize(&small, 512, 384, Filter::Lanczos3);
        let p = psnr(&up, &big);
        assert!(p > 32.0, "bilinear 128x96 -> 512x384: PSNR {p:.1} dB");
        // Point-sampling (the naive way) is clearly worse on a busy picture.
        let busy = {
            let mut b = PixBuf::new(400, 400);
            for (i, px) in b.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let v = if (i % 400 + i / 400) % 2 == 0 { 0 } else { 255 };
                *px = [v, v, v, 255];
            }
            b
        };
        let flat = PixBuf::from_pixel(100, 100, [128, 128, 128, 255]);
        for f in [Filter::Lanczos3, Filter::Area] {
            let p = psnr(&resize(&busy, 100, 100, f), &flat);
            assert!(p > 40.0, "{f:?}: a checkerboard averages to gray: {p:.1}");
        }
    }

    #[test]
    fn flat_stays_flat_and_sizes_are_exact() {
        let flat = PixBuf::from_pixel(37, 23, [10, 200, 30, 255]);
        for f in [Filter::Lanczos3, Filter::Area] {
            for (w, h) in [(5, 3), (36, 22), (74, 46), (37, 50), (1, 1), (200, 1)] {
                let out = resize(&flat, w, h, f);
                assert_eq!(out.dimensions(), (w, h));
                assert!(out.as_raw().as_chunks::<4>().0.iter().all(|p| *p == [10, 200, 30, 255]), "{f:?} {w}x{h}");
            }
        }
        assert_eq!(resize(&flat, 37, 23, Filter::Area), flat, "same size: the picture itself");
    }

    #[test]
    fn area_average_is_exact() {
        // 3 px (0, 90, 180) onto 2: each half covers 1.5 px.
        let mut src = PixBuf::new(3, 1);
        for (i, v) in [0u8, 90, 180].into_iter().enumerate() {
            src.as_raw_mut()[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, 255]);
        }
        let out = resize(&src, 2, 1, Filter::Area);
        assert_eq!(out.get_pixel(0, 0)[0], 30, "(0 + 90/2) / 1.5");
        assert_eq!(out.get_pixel(1, 0)[0], 150, "(90/2 + 180) / 1.5");
    }

    #[test]
    fn transparency_does_not_bleed_its_color() {
        // Opaque red next to transparent (black) pixels: the edge stays red.
        let mut src = PixBuf::new(8, 8);
        for (i, px) in src.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *px = if i % 8 < 4 { [255, 0, 0, 255] } else { [0, 0, 0, 0] };
        }
        for f in [Filter::Lanczos3, Filter::Area] {
            let out = resize(&src, 3, 3, f);
            for y in 0..3 {
                let p = out.get_pixel(1, y);
                assert!(p[3] > 60 && p[3] < 200, "{f:?} half covered: {p:?}");
                assert!(p[0] > 240 && p[1] < 10, "{f:?} no dark fringe: {p:?}");
            }
        }
    }

    /// The SSE2 kernels (x86-64) give exactly what the scalar ones (every
    /// other target) do, on random rows and real weights, odd tap counts
    /// and row tails included.
    #[test]
    fn simd_and_scalar_kernels_agree() {
        let mut s = 0x9e37_79b9u32;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        for (src, dst, f) in [(1000usize, 333usize, Filter::Lanczos3), (517, 200, Filter::Area), (90, 400, Filter::Lanczos3), (37, 5, Filter::Area)] {
            let ax = Axis::new(src, dst, f);
            let row: Vec<u8> = (0..src * 4).map(|_| rnd() as u8).collect();
            for i in 0..dst {
                let (a, n) = (ax.start[i], ax.n[i]);
                let ws = &ax.w[i * ax.taps..i * ax.taps + n];
                assert_eq!(taps_px(&row[a * 4..(a + n) * 4], ws), taps_px_scalar(&row[a * 4..(a + n) * 4], ws), "{src}->{dst} px {i}");
            }
            // Columns: rows of an odd length (a tail past the 16-byte steps).
            let w = 37;
            let rows: Vec<Vec<u8>> = (0..src).map(|_| (0..w * 4).map(|_| rnd() as u8).collect()).collect();
            for i in 0..dst {
                let (a, n) = (ax.start[i], ax.n[i]);
                let ws = &ax.w[i * ax.taps..i * ax.taps + n];
                let rs: Vec<&[u8]> = (a..a + n).map(|j| rows[j].as_slice()).collect();
                let (mut o1, mut o2, mut acc) = (vec![0u8; w * 4], vec![0u8; w * 4], vec![0i32; w * 4]);
                column(&rs, ws, &mut acc, &mut o1);
                column_scalar(&rs, ws, &mut acc, &mut o2);
                assert_eq!(o1, o2, "{src}->{dst} row {i}");
            }
        }
    }

    /// A large picture split over threads gives what one thread would.
    #[test]
    fn threads_agree() {
        let big = sampled(1200, 700, 600.0);
        let out = resize(&big, 517, 333, Filter::Lanczos3);
        // Down, then the rows across one at a time.
        let ay = Axis::new(700, 333, Filter::Lanczos3);
        let mut mid = vec![0u8; 1200 * 333 * 4];
        pass_y(big.as_raw(), 1200, &mut mid, 333, &ay);
        let ax = Axis::new(1200, 517, Filter::Lanczos3);
        let mut one = vec![0u8; 517 * 333 * 4];
        for y in 0..333 {
            pass_x(&mid[y * 1200 * 4..(y + 1) * 1200 * 4], 1200, 1, &mut one[y * 517 * 4..(y + 1) * 517 * 4], 517, &ax);
        }
        assert_eq!(out.as_raw(), &one);
    }
}
