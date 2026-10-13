//! The operators of the segmentation net, f32 NHWC (batch 1).
//!
//! The release build optimizes for size (no automatic vectorization), so
//! the hot loops go four lanes at a time through [`F4`] (SSE2 on x86-64,
//! plain arrays elsewhere), on raw pointers where the shapes were checked
//! once at load. Every convolution ends in [`gemm`], a register-blocked
//! product (four pixels by eight output channels per step): 1x1
//! convolutions directly, larger kernels through one output row of
//! patches at a time (im2col), transposed convolutions as one product per
//! input row whose columns are every kernel tap's outputs, scattered into
//! place. Weights are repacked at load ([`pack_hwio`], [`pack_tconv`],
//! [`pack_dw`]): output channels innermost, padded to a multiple of 4.

use super::format::Act;

/// Height, width, channels.
pub type Dim = (usize, usize, usize);

/// Four f32 lanes.
#[derive(Clone, Copy)]
pub struct F4(#[cfg(target_arch = "x86_64")] std::arch::x86_64::__m128, #[cfg(not(target_arch = "x86_64"))] [f32; 4]);

#[cfg(target_arch = "x86_64")]
impl F4 {
    #[inline(always)]
    pub fn splat(v: f32) -> F4 {
        // SAFETY: SSE2 is part of x86-64.
        F4(unsafe { std::arch::x86_64::_mm_set1_ps(v) })
    }
    /// # Safety
    /// `p..p + 4` is readable.
    #[inline(always)]
    pub unsafe fn ld(p: *const f32) -> F4 {
        // SAFETY: the caller's promise; unaligned load.
        F4(unsafe { std::arch::x86_64::_mm_loadu_ps(p) })
    }
    /// # Safety
    /// `p..p + 4` is writable.
    #[inline(always)]
    pub unsafe fn st(self, p: *mut f32) {
        // SAFETY: the caller's promise.
        unsafe { std::arch::x86_64::_mm_storeu_ps(p, self.0) }
    }
    /// `self + a * b`.
    #[inline(always)]
    pub fn mul_add(self, a: F4, b: F4) -> F4 {
        use std::arch::x86_64::*;
        // SAFETY: SSE2.
        F4(unsafe { _mm_add_ps(self.0, _mm_mul_ps(a.0, b.0)) })
    }
    #[inline(always)]
    pub fn add(self, o: F4) -> F4 {
        // SAFETY: SSE2.
        F4(unsafe { std::arch::x86_64::_mm_add_ps(self.0, o.0) })
    }
    #[inline(always)]
    pub fn mul(self, o: F4) -> F4 {
        // SAFETY: SSE2.
        F4(unsafe { std::arch::x86_64::_mm_mul_ps(self.0, o.0) })
    }
    #[inline(always)]
    pub fn max(self, o: F4) -> F4 {
        // SAFETY: SSE2.
        F4(unsafe { std::arch::x86_64::_mm_max_ps(self.0, o.0) })
    }
    #[inline(always)]
    pub fn min(self, o: F4) -> F4 {
        // SAFETY: SSE2.
        F4(unsafe { std::arch::x86_64::_mm_min_ps(self.0, o.0) })
    }
    /// The four bytes of a pixel as lanes.
    #[inline(always)]
    pub fn from_px(p: [u8; 4]) -> F4 {
        use std::arch::x86_64::*;
        // SAFETY: SSE2.
        unsafe {
            let z = _mm_setzero_si128();
            F4(_mm_cvtepi32_ps(_mm_unpacklo_epi16(_mm_unpacklo_epi8(_mm_cvtsi32_si128(i32::from_ne_bytes(p)), z), z)))
        }
    }
    /// The lanes rounded to the nearest byte (clamped to 0..=255).
    #[inline(always)]
    pub fn to_px(self) -> [u8; 4] {
        use std::arch::x86_64::*;
        // SAFETY: SSE2.
        unsafe {
            let i = _mm_cvtps_epi32(self.0);
            let w = _mm_packs_epi32(i, i);
            _mm_cvtsi128_si32(_mm_packus_epi16(w, w)).to_ne_bytes()
        }
    }
    /// The fourth lane.
    #[inline(always)]
    pub fn lane3(self) -> f32 {
        use std::arch::x86_64::*;
        // SAFETY: SSE2.
        unsafe { _mm_cvtss_f32(_mm_shuffle_ps::<0xff>(self.0, self.0)) }
    }
}

#[cfg(not(target_arch = "x86_64"))]
impl F4 {
    #[inline(always)]
    pub fn splat(v: f32) -> F4 {
        F4([v; 4])
    }
    /// # Safety
    /// `p..p + 4` is readable.
    #[inline(always)]
    pub unsafe fn ld(p: *const f32) -> F4 {
        // SAFETY: the caller's promise.
        F4(unsafe { std::ptr::read_unaligned(p as *const [f32; 4]) })
    }
    /// # Safety
    /// `p..p + 4` is writable.
    #[inline(always)]
    pub unsafe fn st(self, p: *mut f32) {
        // SAFETY: the caller's promise.
        unsafe { std::ptr::write_unaligned(p as *mut [f32; 4], self.0) }
    }
    #[inline(always)]
    pub fn mul_add(self, a: F4, b: F4) -> F4 {
        F4(std::array::from_fn(|i| self.0[i] + a.0[i] * b.0[i]))
    }
    #[inline(always)]
    pub fn add(self, o: F4) -> F4 {
        F4(std::array::from_fn(|i| self.0[i] + o.0[i]))
    }
    #[inline(always)]
    pub fn mul(self, o: F4) -> F4 {
        F4(std::array::from_fn(|i| self.0[i] * o.0[i]))
    }
    #[inline(always)]
    pub fn max(self, o: F4) -> F4 {
        F4(std::array::from_fn(|i| self.0[i].max(o.0[i])))
    }
    #[inline(always)]
    pub fn min(self, o: F4) -> F4 {
        F4(std::array::from_fn(|i| self.0[i].min(o.0[i])))
    }
    #[inline(always)]
    pub fn from_px(p: [u8; 4]) -> F4 {
        F4(p.map(f32::from))
    }
    #[inline(always)]
    pub fn to_px(self) -> [u8; 4] {
        self.0.map(|v| v.round_ties_even().clamp(0.0, 255.0) as u8)
    }
    #[inline(always)]
    pub fn lane3(self) -> f32 {
        self.0[3]
    }
}

/// The activation's clamp range, when it is one (`Tanh` is not).
fn clamp_range(act: Act) -> Option<(f32, f32)> {
    match act {
        Act::None | Act::Tanh => None,
        Act::Relu => Some((0.0, f32::INFINITY)),
        Act::Relu6 => Some((0.0, 6.0)),
        Act::ReluN1To1 => Some((-1.0, 1.0)),
    }
}

#[inline(always)]
pub fn act1(act: Act, v: f32) -> f32 {
    match act {
        Act::None => v,
        Act::Relu => v.max(0.0),
        Act::Relu6 => v.clamp(0.0, 6.0),
        Act::ReluN1To1 => v.clamp(-1.0, 1.0),
        Act::Tanh => v.tanh(),
    }
}

/// `act` over `v` in place.
pub fn activate(act: Act, v: &mut [f32]) {
    match clamp_range(act) {
        Some((lo, hi)) => {
            let (l4, h4) = (F4::splat(lo), F4::splat(hi));
            let (c, rest) = v.as_chunks_mut::<4>();
            for q in c {
                // SAFETY: a chunk is four values.
                unsafe { F4::ld(q.as_ptr()).max(l4).min(h4).st(q.as_mut_ptr()) };
            }
            for x in rest {
                *x = x.clamp(lo, hi);
            }
        }
        None if act == Act::Tanh => v.iter_mut().for_each(|x| *x = x.tanh()),
        None => {}
    }
}

/// `x * relu6(x + 3) / 6`.
pub fn hard_swish(v: &mut [f32]) {
    let (three, six, sixth, zero) = (F4::splat(3.0), F4::splat(6.0), F4::splat(1.0 / 6.0), F4::splat(0.0));
    let (c, rest) = v.as_chunks_mut::<4>();
    for q in c {
        // SAFETY: a chunk is four values.
        unsafe {
            let x = F4::ld(q.as_ptr());
            x.mul(x.add(three).max(zero).min(six)).mul(sixth).st(q.as_mut_ptr());
        }
    }
    for x in rest {
        *x *= (*x + 3.0).clamp(0.0, 6.0) * (1.0 / 6.0);
    }
}

pub fn sigmoid(v: &mut [f32]) {
    for x in v {
        *x = 1.0 / (1.0 + (-*x).exp());
    }
}

pub fn round4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

/// OHWI weights (`o`, `kh`, `kw`, `i`) as HWIO with the outputs padded to
/// a multiple of 4 (zeros): rows in the order of an im2col patch. The
/// bias is padded alike.
pub fn pack_hwio(w: &[f32], (o, kh, kw, i): (usize, usize, usize, usize), bias: Option<&[f32]>) -> (Vec<f32>, Vec<f32>) {
    let op = round4(o);
    let mut out = vec![0.0; kh * kw * i * op];
    for oc in 0..o {
        for y in 0..kh {
            for x in 0..kw {
                for ic in 0..i {
                    out[((y * kw + x) * i + ic) * op + oc] = w[((oc * kh + y) * kw + x) * i + ic];
                }
            }
        }
    }
    (out, pad_bias(bias, o))
}

pub fn pad_bias(bias: Option<&[f32]>, o: usize) -> Vec<f32> {
    let mut b = vec![0.0; round4(o)];
    if let Some(bias) = bias {
        b[..o].copy_from_slice(&bias[..o]);
    }
    b
}

/// Transposed-convolution weights (OHWI) as one `i` x (taps · padded
/// outputs) matrix: row `ic`, column `tap * op + oc`.
pub fn pack_tconv(w: &[f32], (o, kh, kw, i): (usize, usize, usize, usize)) -> Vec<f32> {
    let op = round4(o);
    let cols = kh * kw * op;
    let mut out = vec![0.0; i * cols];
    for oc in 0..o {
        for t in 0..kh * kw {
            for ic in 0..i {
                out[ic * cols + t * op + oc] = w[(oc * kh * kw + t) * i + ic];
            }
        }
    }
    out
}

/// Depthwise weights `1 x kh x kw x c` with the channels padded to a
/// multiple of 4, and the bias alike.
pub fn pack_dw(w: &[f32], (kh, kw, c): Dim, bias: Option<&[f32]>) -> (Vec<f32>, Vec<f32>) {
    let cp = round4(c);
    let mut out = vec![0.0; kh * kw * cp];
    for k in 0..kh * kw {
        out[k * cp..][..c].copy_from_slice(&w[k * c..][..c]);
    }
    (out, pad_bias(bias, c))
}

/// Clamp (when `range`) and store the first `valid` (0 to 4) lanes at `p`.
///
/// # Safety
/// `p..p + valid` is writable.
#[inline(always)]
unsafe fn put(a: F4, p: *mut f32, valid: usize, range: Option<(f32, f32)>) {
    let a = match range {
        Some((lo, hi)) => a.max(F4::splat(lo)).min(F4::splat(hi)),
        None => a,
    };
    // SAFETY: the caller's promise.
    unsafe {
        if valid >= 4 {
            a.st(p);
        } else if valid > 0 {
            let mut t = [0f32; 4];
            a.st(t.as_mut_ptr());
            std::ptr::copy_nonoverlapping(t.as_ptr(), p, valid);
        }
    }
}

/// `out[r][c] = act(b[c] + Σk a[r][k] · w[k][c])` for rows `r < n` and
/// columns `c < oc`: `a`'s rows `lda` apart (`kk` values each), `w`'s
/// `ocp` apart (a multiple of 4, at least `oc`), `out`'s `ldo` apart.
#[allow(clippy::too_many_arguments)]
pub fn gemm(a: &[f32], lda: usize, n: usize, kk: usize, w: &[f32], ocp: usize, b: &[f32], oc: usize, act: Act, out: &mut [f32], ldo: usize) {
    if n == 0 || oc == 0 {
        return;
    }
    assert!(kk <= lda && a.len() >= (n - 1) * lda + kk);
    assert!(oc <= ocp && ocp.is_multiple_of(4) && w.len() >= kk * ocp && b.len() >= ocp);
    assert!(oc <= ldo && out.len() >= (n - 1) * ldo + oc);
    let range = clamp_range(act);
    let (ap, wp, bp, op) = (a.as_ptr(), w.as_ptr(), b.as_ptr(), out.as_mut_ptr());
    let valid = |c: usize| oc.saturating_sub(c).min(4);
    // SAFETY: rows r < n of `a` hold kk values from r * lda; rows k < kk
    // of `w` hold ocp values from k * ocp, and `b` ocp; rows r < n of
    // `out` hold oc values from r * ldo, written only below `valid` (all
    // asserted above); columns c stay below ocp.
    unsafe {
        let mut r = 0;
        while r + 4 <= n {
            let x = [ap.add(r * lda), ap.add((r + 1) * lda), ap.add((r + 2) * lda), ap.add((r + 3) * lda)];
            let o = [op.add(r * ldo), op.add((r + 1) * ldo), op.add((r + 2) * ldo), op.add((r + 3) * ldo)];
            let mut c = 0;
            while c + 8 <= ocp {
                let (b0, b1) = (F4::ld(bp.add(c)), F4::ld(bp.add(c + 4)));
                let (mut s00, mut s01, mut s10, mut s11) = (b0, b1, b0, b1);
                let (mut s20, mut s21, mut s30, mut s31) = (b0, b1, b0, b1);
                let mut wr = wp.add(c);
                for k in 0..kk {
                    let (w0, w1) = (F4::ld(wr), F4::ld(wr.add(4)));
                    let v = F4::splat(*x[0].add(k));
                    s00 = s00.mul_add(v, w0);
                    s01 = s01.mul_add(v, w1);
                    let v = F4::splat(*x[1].add(k));
                    s10 = s10.mul_add(v, w0);
                    s11 = s11.mul_add(v, w1);
                    let v = F4::splat(*x[2].add(k));
                    s20 = s20.mul_add(v, w0);
                    s21 = s21.mul_add(v, w1);
                    let v = F4::splat(*x[3].add(k));
                    s30 = s30.mul_add(v, w0);
                    s31 = s31.mul_add(v, w1);
                    wr = wr.add(ocp);
                }
                for (i, (p, q)) in [(s00, s01), (s10, s11), (s20, s21), (s30, s31)].into_iter().enumerate() {
                    put(p, o[i].add(c), valid(c), range);
                    put(q, o[i].add(c + 4), valid(c + 4), range);
                }
                c += 8;
            }
            if c < ocp {
                let b0 = F4::ld(bp.add(c));
                let (mut s0, mut s1, mut s2, mut s3) = (b0, b0, b0, b0);
                let mut wr = wp.add(c);
                for k in 0..kk {
                    let w0 = F4::ld(wr);
                    s0 = s0.mul_add(F4::splat(*x[0].add(k)), w0);
                    s1 = s1.mul_add(F4::splat(*x[1].add(k)), w0);
                    s2 = s2.mul_add(F4::splat(*x[2].add(k)), w0);
                    s3 = s3.mul_add(F4::splat(*x[3].add(k)), w0);
                    wr = wr.add(ocp);
                }
                for (i, p) in [s0, s1, s2, s3].into_iter().enumerate() {
                    put(p, o[i].add(c), valid(c), range);
                }
            }
            r += 4;
        }
        while r < n {
            let (x, o) = (ap.add(r * lda), op.add(r * ldo));
            let mut c = 0;
            while c < ocp {
                let two = c + 8 <= ocp;
                let (mut s0, mut s1) = (F4::ld(bp.add(c)), if two { F4::ld(bp.add(c + 4)) } else { F4::splat(0.0) });
                let mut wr = wp.add(c);
                for k in 0..kk {
                    let v = F4::splat(*x.add(k));
                    s0 = s0.mul_add(v, F4::ld(wr));
                    if two {
                        s1 = s1.mul_add(v, F4::ld(wr.add(4)));
                    }
                    wr = wr.add(ocp);
                }
                put(s0, o.add(c), valid(c), range);
                if two {
                    put(s1, o.add(c + 4), valid(c + 4), range);
                }
                c += if two { 8 } else { 4 };
            }
            r += 1;
        }
    }
    if act == Act::Tanh {
        for r in 0..n {
            out[r * ldo..][..oc].iter_mut().for_each(|v| *v = v.tanh());
        }
    }
}

/// A convolution's geometry.
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub k: (usize, usize),
    pub stride: (usize, usize),
    pub pad: (usize, usize),
    pub dil: (usize, usize),
}

/// The kernel taps that land inside an input of `n` for output position
/// `o` along one axis: `[lo, hi)`.
#[inline(always)]
fn taps(o: usize, stride: usize, pad: usize, dil: usize, k: usize, n: usize) -> (usize, usize) {
    let base = (o * stride) as isize - pad as isize;
    let d = dil as isize;
    let lo = if base >= 0 { 0 } else { (-base + d - 1) / d };
    let hi = if base >= n as isize { 0 } else { ((n as isize - base + d - 1) / d).min(k as isize) };
    (lo.max(0) as usize, hi.max(lo) as usize)
}

/// Whether a convolution is a plain 1x1 (a matrix product over pixels).
pub fn is_pointwise(g: Geo, xd: Dim, od: Dim) -> bool {
    g.k == (1, 1) && g.stride == (1, 1) && g.pad == (0, 0) && (xd.0, xd.1) == (od.0, od.1)
}

/// Output rows `rows` of a convolution: `x` (`xd`) with packed weights
/// `w` ([`pack_hwio`]) and bias `b`, then `act`, into `out` (those rows
/// only). `scratch` holds one output row of patches.
#[allow(clippy::too_many_arguments)]
pub fn conv(x: &[f32], xd: Dim, w: &[f32], b: &[f32], g: Geo, act: Act, out: &mut [f32], od: Dim, rows: (usize, usize), scratch: &mut Vec<f32>) {
    let (ih, iw, ic) = xd;
    let (_, ow, oc) = od;
    let ocp = round4(oc);
    if is_pointwise(g, xd, od) {
        let (p0, p1) = (rows.0 * ow, rows.1 * ow);
        return gemm(&x[p0 * ic..p1 * ic], ic, p1 - p0, ic, w, ocp, b, oc, act, out, oc);
    }
    let kk = g.k.0 * g.k.1 * ic;
    scratch.clear();
    scratch.resize(ow * kk, 0.0);
    for oy in rows.0..rows.1 {
        let (ky0, ky1) = taps(oy, g.stride.0, g.pad.0, g.dil.0, g.k.0, ih);
        for ox in 0..ow {
            let (kx0, kx1) = taps(ox, g.stride.1, g.pad.1, g.dil.1, g.k.1, iw);
            let p = &mut scratch[ox * kk..][..kk];
            if (ky1 - ky0) * (kx1 - kx0) != g.k.0 * g.k.1 {
                p.fill(0.0);
            }
            for ky in ky0..ky1 {
                let iy = oy * g.stride.0 + ky * g.dil.0 - g.pad.0;
                for kx in kx0..kx1 {
                    let ix = ox * g.stride.1 + kx * g.dil.1 - g.pad.1;
                    p[(ky * g.k.1 + kx) * ic..][..ic].copy_from_slice(&x[(iy * iw + ix) * ic..][..ic]);
                }
            }
        }
        gemm(scratch, kk, ow, kk, w, ocp, b, oc, act, &mut out[(oy - rows.0) * ow * oc..][..ow * oc], oc);
    }
}

/// Output rows `rows` of a depthwise convolution (multiplier 1): each
/// channel with its own kernel; weights from [`pack_dw`]; `out` holds
/// those rows only.
#[allow(clippy::too_many_arguments)]
pub fn depthwise(x: &[f32], xd: Dim, w: &[f32], b: &[f32], g: Geo, act: Act, out: &mut [f32], od: Dim, rows: (usize, usize)) {
    let (ih, iw, c) = xd;
    let (_, ow, _) = od;
    let cp = round4(c);
    let c4 = c / 4 * 4;
    let (kh, kw) = g.k;
    let nrows = rows.1 - rows.0;
    assert!(x.len() >= ih * iw * c && out.len() >= nrows * ow * c && w.len() >= kh * kw * cp && b.len() >= cp);
    let range = clamp_range(act);
    let mut offs = vec![(0usize, 0usize); kh * kw];
    let (xp, wp, bp, op) = (x.as_ptr(), w.as_ptr(), b.as_ptr(), out.as_mut_ptr());
    for oy in rows.0..rows.1 {
        let (ky0, ky1) = taps(oy, g.stride.0, g.pad.0, g.dil.0, kh, ih);
        for ox in 0..ow {
            let (kx0, kx1) = taps(ox, g.stride.1, g.pad.1, g.dil.1, kw, iw);
            let mut t = 0;
            for ky in ky0..ky1 {
                let iy = oy * g.stride.0 + ky * g.dil.0 - g.pad.0;
                for kx in kx0..kx1 {
                    let ix = ox * g.stride.1 + kx * g.dil.1 - g.pad.1;
                    offs[t] = ((iy * iw + ix) * c, (ky * kw + kx) * cp);
                    t += 1;
                }
            }
            let taps = &offs[..t];
            // SAFETY: each tap's pixel (iy < ih, ix < iw) holds c values
            // and its weights cp; the output pixel holds c (asserted);
            // vector lanes stay below c4 <= c.
            unsafe {
                let o = op.add(((oy - rows.0) * ow + ox) * c);
                let mut j = 0;
                while j < c4 {
                    let mut a = F4::ld(bp.add(j));
                    for &(xo, wo) in taps {
                        a = a.mul_add(F4::ld(xp.add(xo + j)), F4::ld(wp.add(wo + j)));
                    }
                    put(a, o.add(j), 4, range);
                    j += 4;
                }
                for j in c4..c {
                    let mut a = *bp.add(j);
                    for &(xo, wo) in taps {
                        a += *xp.add(xo + j) * *wp.add(wo + j);
                    }
                    *o.add(j) = match range {
                        Some((lo, hi)) => a.clamp(lo, hi),
                        None => a,
                    };
                }
            }
        }
    }
    if act == Act::Tanh {
        out[..nrows * ow * c].iter_mut().for_each(|v| *v = v.tanh());
    }
}

/// A depthwise convolution with a channel multiplier: output channel `o`
/// reads input channel `o / mult`; raw `1 x kh x kw x (c·mult)` weights.
#[allow(clippy::too_many_arguments)]
pub fn depthwise_mult(x: &[f32], xd: Dim, w: &[f32], b: Option<&[f32]>, g: Geo, mult: usize, act: Act, out: &mut [f32], od: Dim) {
    let (ih, iw, c) = xd;
    let (oh, ow, oc) = od;
    for oy in 0..oh {
        let (ky0, ky1) = taps(oy, g.stride.0, g.pad.0, g.dil.0, g.k.0, ih);
        for ox in 0..ow {
            let (kx0, kx1) = taps(ox, g.stride.1, g.pad.1, g.dil.1, g.k.1, iw);
            for o in 0..oc {
                let mut a = b.map_or(0.0, |b| b[o]);
                for ky in ky0..ky1 {
                    let iy = oy * g.stride.0 + ky * g.dil.0 - g.pad.0;
                    for kx in kx0..kx1 {
                        let ix = ox * g.stride.1 + kx * g.dil.1 - g.pad.1;
                        a += x[(iy * iw + ix) * c + o / mult] * w[(ky * g.k.1 + kx) * oc + o];
                    }
                }
                out[(oy * ow + ox) * oc + o] = act1(act, a);
            }
        }
    }
}

/// Every output pixel of a transposed convolution starts at the bias.
pub fn tconv_bias(out: &mut [f32], (oh, ow, oc): Dim, b: &[f32]) {
    for p in out[..oh * ow * oc].chunks_exact_mut(oc) {
        p.copy_from_slice(&b[..oc]);
    }
}

/// What input rows `rows` of a transposed convolution add to the output
/// (`out`, all `od` of it, started at the bias by [`tconv_bias`]; the
/// activation is the caller's).
///
/// # Safety
/// `out` holds `oh * ow * oc` values, and nothing else touches the output
/// rows these input rows reach meanwhile (`iy * stride - pad + 0..kh`).
#[allow(clippy::too_many_arguments)]
pub unsafe fn transpose_conv_rows(
    x: &[f32],
    xd: Dim,
    w: &[f32],
    k: (usize, usize),
    zero: &[f32],
    stride: (usize, usize),
    pad: (usize, usize),
    out: *mut f32,
    od: Dim,
    rows: (usize, usize),
    scratch: &mut Vec<f32>,
) {
    let (_, iw, ic) = xd;
    let (oh, ow, oc) = od;
    let ocp = round4(oc);
    let cols = k.0 * k.1 * ocp;
    scratch.clear();
    scratch.resize(iw * cols, 0.0);
    for iy in rows.0..rows.1 {
        gemm(&x[iy * iw * ic..][..iw * ic], ic, iw, ic, w, cols, zero, cols, Act::None, scratch, cols);
        for ky in 0..k.0 {
            let oy = (iy * stride.0 + ky) as isize - pad.0 as isize;
            if oy < 0 || oy >= oh as isize {
                continue;
            }
            for ix in 0..iw {
                for kx in 0..k.1 {
                    let ox = (ix * stride.1 + kx) as isize - pad.1 as isize;
                    if ox < 0 || ox >= ow as isize {
                        continue;
                    }
                    let src = &scratch[ix * cols + (ky * k.1 + kx) * ocp..][..oc];
                    // SAFETY: (oy, ox) is an output pixel (checked), its row
                    // this caller's alone (the caller's promise).
                    let dst = unsafe { std::slice::from_raw_parts_mut(out.add((oy as usize * ow + ox as usize) * oc), oc) };
                    for (d, s) in dst.iter_mut().zip(src) {
                        *d += s;
                    }
                }
            }
        }
    }
}

/// The 4-D NHWC shape of `dims` (right-aligned, missing leading dims 1).
pub fn dims4(dims: &[u32]) -> [usize; 4] {
    let mut d = [1usize; 4];
    for (k, &v) in dims.iter().rev().take(4).enumerate() {
        d[3 - k] = v as usize;
    }
    d
}

/// `o = a op b` over equal-length rows.
fn row_op<const MUL: bool>(a: &[f32], b: &[f32], o: &mut [f32]) {
    let n = a.len().min(b.len()).min(o.len());
    let n4 = n / 4 * 4;
    // SAFETY: lanes below n4 <= every length.
    unsafe {
        let (ap, bp, op) = (a.as_ptr(), b.as_ptr(), o.as_mut_ptr());
        let mut i = 0;
        while i < n4 {
            let (x, y) = (F4::ld(ap.add(i)), F4::ld(bp.add(i)));
            (if MUL { x.mul(y) } else { x.add(y) }).st(op.add(i));
            i += 4;
        }
    }
    for i in n4..n {
        o[i] = if MUL { a[i] * b[i] } else { a[i] + b[i] };
    }
}

/// `acc += p` over equal-length rows.
fn add_into(acc: &mut [f32], p: &[f32]) {
    let n = acc.len().min(p.len());
    let n4 = n / 4 * 4;
    // SAFETY: lanes below n4 <= both lengths.
    unsafe {
        let (ap, pp) = (acc.as_mut_ptr(), p.as_ptr());
        let mut i = 0;
        while i < n4 {
            F4::ld(ap.add(i)).add(F4::ld(pp.add(i))).st(ap.add(i));
            i += 4;
        }
    }
    for i in n4..n {
        acc[i] += p[i];
    }
}

fn binary_impl<const MUL: bool>(a: &[f32], ad: [usize; 4], b: &[f32], bd: [usize; 4], od: [usize; 4], out: &mut [f32]) {
    let n: usize = od.iter().product();
    let c = od[3];
    let per_channel = |d: [usize; 4]| d[..3] == [1, 1, 1] && d[3] == c;
    let scalar = |d: [usize; 4]| d == [1, 1, 1, 1];
    // (The operations commute: a broadcast `a` swaps places with `b`.)
    let (full, other, od_other) = if ad == od { (a, b, bd) } else { (b, a, ad) };
    let full_ok = ad == od || bd == od;
    if full_ok && od_other == od {
        row_op::<MUL>(full, other, &mut out[..n]);
    } else if full_ok && scalar(od_other) {
        let s = vec![other[0]; c.max(4)];
        for (o, x) in out[..n].chunks_exact_mut(c).zip(full.chunks_exact(c)) {
            row_op::<MUL>(x, &s[..c], o);
        }
    } else if full_ok && per_channel(od_other) {
        for (o, x) in out[..n].chunks_exact_mut(c).zip(full.chunks_exact(c)) {
            row_op::<MUL>(x, &other[..c], o);
        }
    } else {
        let idx = |d: [usize; 4], p: [usize; 4]| {
            let q: [usize; 4] = std::array::from_fn(|k| if d[k] == 1 { 0 } else { p[k] });
            ((q[0] * d[1] + q[1]) * d[2] + q[2]) * d[3] + q[3]
        };
        let mut i = 0;
        for n0 in 0..od[0] {
            for y in 0..od[1] {
                for x in 0..od[2] {
                    for ch in 0..od[3] {
                        let p = [n0, y, x, ch];
                        let (u, v) = (a[idx(ad, p)], b[idx(bd, p)]);
                        out[i] = if MUL { u * v } else { u + v };
                        i += 1;
                    }
                }
            }
        }
    }
}

/// `a op b` with broadcasting (each dim equal or 1), into `out` of shape
/// `od`, then `act`.
#[allow(clippy::too_many_arguments)]
pub fn binary(a: &[f32], ad: [usize; 4], b: &[f32], bd: [usize; 4], od: [usize; 4], mul: bool, act: Act, out: &mut [f32]) {
    if mul {
        binary_impl::<true>(a, ad, b, bd, od, out);
    } else {
        binary_impl::<false>(a, ad, b, bd, od, out);
    }
    let n: usize = od.iter().product();
    activate(act, &mut out[..n]);
}

/// Average pooling over the valid part of each window (TensorFlow Lite).
#[allow(clippy::too_many_arguments)]
pub fn avg_pool(x: &[f32], xd: Dim, size: (usize, usize), stride: (usize, usize), pad: (usize, usize), act: Act, out: &mut [f32], od: Dim) {
    let (ih, iw, c) = xd;
    let (oh, ow, _) = od;
    let mut sum = vec![0.0f32; c];
    for oy in 0..oh {
        let (ky0, ky1) = taps(oy, stride.0, pad.0, 1, size.0, ih);
        for ox in 0..ow {
            let (kx0, kx1) = taps(ox, stride.1, pad.1, 1, size.1, iw);
            sum.iter_mut().for_each(|s| *s = 0.0);
            for ky in ky0..ky1 {
                let iy = oy * stride.0 + ky - pad.0;
                for kx in kx0..kx1 {
                    let ix = ox * stride.1 + kx - pad.1;
                    for (s, v) in sum.iter_mut().zip(&x[(iy * iw + ix) * c..][..c]) {
                        *s += v;
                    }
                }
            }
            let n = ((ky1 - ky0) * (kx1 - kx0)).max(1) as f32;
            for (o, s) in out[(oy * ow + ox) * c..][..c].iter_mut().zip(&sum) {
                *o = act1(act, s / n);
            }
        }
    }
}

/// The mean of each channel over the picture: f32 sums along each row
/// (vectors), the rows' sums added in f64.
pub fn mean(x: &[f32], (h, w, c): Dim, out: &mut [f32]) {
    let mut total = vec![0.0f64; c];
    let mut row = vec![0.0f32; c];
    for y in 0..h {
        row.iter_mut().for_each(|v| *v = 0.0);
        for p in x[y * w * c..][..w * c].chunks_exact(c) {
            add_into(&mut row, p);
        }
        for (t, r) in total.iter_mut().zip(&row) {
            *t += *r as f64;
        }
    }
    for (o, s) in out[..c].iter_mut().zip(total) {
        *o = (s / (h * w).max(1) as f64) as f32;
    }
}

/// Where output index `i` reads along an axis of `n_in` -> `n_out`:
/// (lower, upper, weight of the upper), TensorFlow Lite's rules.
pub fn lerp_index(i: usize, n_in: usize, n_out: usize, align: bool, half: bool) -> (usize, usize, f32) {
    let scale = if align && n_out > 1 { (n_in as f32 - 1.0) / (n_out as f32 - 1.0) } else { n_in as f32 / n_out as f32 };
    let v = if half { (i as f32 + 0.5) * scale - 0.5 } else { i as f32 * scale };
    let lo = (v.floor() as i64).max(0) as usize;
    let hi = (v.ceil() as i64).clamp(0, n_in as i64 - 1) as usize;
    (lo.min(n_in - 1), hi, v - lo as f32)
}

/// Bilinear resize (TensorFlow Lite's `RESIZE_BILINEAR`).
pub fn resize_bilinear(x: &[f32], (ih, iw, c): Dim, (oh, ow): (usize, usize), align: bool, half: bool, out: &mut [f32]) {
    let xs: Vec<_> = (0..ow).map(|i| lerp_index(i, iw, ow, align, half)).collect();
    let c4 = c / 4 * 4;
    for oy in 0..oh {
        let (y0, y1, fy) = lerp_index(oy, ih, oh, align, half);
        for (ox, &(x0, x1, fx)) in xs.iter().enumerate() {
            let wt = [(1.0 - fy) * (1.0 - fx), fy * (1.0 - fx), (1.0 - fy) * fx, fy * fx];
            let at = [(y0 * iw + x0) * c, (y1 * iw + x0) * c, (y0 * iw + x1) * c, (y1 * iw + x1) * c];
            let o = &mut out[(oy * ow + ox) * c..][..c];
            let w4 = wt.map(F4::splat);
            // SAFETY: the four corners are pixels of the input (indices
            // clamped by `lerp_index`), c values each; lanes below c4.
            unsafe {
                let (xp, op) = (x.as_ptr(), o.as_mut_ptr());
                let mut j = 0;
                while j < c4 {
                    let s = F4::ld(xp.add(at[0] + j)).mul(w4[0]).mul_add(F4::ld(xp.add(at[1] + j)), w4[1]);
                    s.mul_add(F4::ld(xp.add(at[2] + j)), w4[2]).mul_add(F4::ld(xp.add(at[3] + j)), w4[3]).st(op.add(j));
                    j += 4;
                }
            }
            for k in c4..c {
                o[k] = x[at[0] + k] * wt[0] + x[at[1] + k] * wt[1] + x[at[2] + k] * wt[2] + x[at[3] + k] * wt[3];
            }
        }
    }
}

/// Inputs concatenated along the channels.
pub fn concat(parts: &[(&[f32], usize)], pixels: usize, out: &mut [f32]) {
    let oc: usize = parts.iter().map(|p| p.1).sum();
    for p in 0..pixels {
        let mut at = p * oc;
        for &(x, c) in parts {
            out[at..at + c].copy_from_slice(&x[p * c..][..c]);
            at += c;
        }
    }
}
