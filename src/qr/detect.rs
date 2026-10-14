//! Find QR codes in a picture: binarize (global Otsu, then an adaptive
//! block threshold), find the 1:1:3:1:1 finder patterns, group them in
//! threes, locate the bottom-right alignment pattern, map the modules
//! through a perspective transform and sample the grid.
//!
//! Hostile pictures are bounded: candidates are capped, every lookup is
//! range-checked, searches are bounded in modules, a big picture is read
//! at no more than 2 MP first (full size only when that finds nothing and
//! it is at most 40 MP), and the whole read has a time budget
//! ([`decode_rgba_within`]): past it, what was found so far is returned.

use super::matrix::{self, Content};
use super::tables::{self, Matrix};
use std::time::{Duration, Instant};

thread_local! {
    /// The current read's deadline (none: no limit).
    static DEADLINE: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// The read is past its time budget.
fn late() -> bool {
    DEADLINE.with(|d| d.get()).is_some_and(|t| Instant::now() >= t)
}

/// The most pixels the first pass reads (the picture is box-averaged to it).
pub const FIRST_PASS_PX: usize = 2_000_000;
/// The most pixels a full-size second pass reads.
pub const FULL_PASS_PX: usize = 40_000_000;

/// The luminance of `rgba` box-averaged by `f` (each output pixel the
/// mean of an `f`×`f` block; alpha over white).
pub fn luminance_scaled(w: usize, h: usize, rgba: &[u8], f: usize) -> (usize, usize, Vec<u8>) {
    let f = f.max(1);
    let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
    let mut sum = vec![0u32; sw * sh];
    let mut cnt = vec![0u32; sw * sh];
    let px = rgba.as_chunks::<4>().0;
    for y in 0..h {
        if y % 64 == 0 && late() {
            break;
        }
        let row = (y / f) * sw;
        for x in 0..w {
            let p = px[y * w + x];
            let l = (p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8;
            let a = p[3] as u32;
            sum[row + x / f] += (l * a + 255 * (255 - a)) / 255;
            cnt[row + x / f] += 1;
        }
    }
    (sw, sh, sum.iter().zip(&cnt).map(|(s, c)| if *c == 0 { 255 } else { (s / c) as u8 }).collect())
}

/// A binarized picture (`true` = dark).
pub struct Bin {
    pub w: usize,
    pub h: usize,
    pub px: Vec<bool>,
}

impl Bin {
    fn get(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.px[y as usize * self.w + x as usize]
    }
}

/// Luminance of an RGBA picture (alpha over white).
pub fn luminance(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h);
    for p in rgba.as_chunks::<4>().0.iter().take(w * h) {
        let l = (p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8;
        let a = p[3] as u32;
        out.push(((l * a + 255 * (255 - a)) / 255) as u8);
    }
    out.resize(w * h, 255);
    out
}

/// Otsu's global threshold.
pub fn otsu(w: usize, h: usize, lum: &[u8]) -> Bin {
    let mut hist = [0u64; 256];
    for &l in lum {
        hist[l as usize] += 1;
    }
    let total = lum.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut wb, mut sb, mut best, mut t) = (0f64, 0f64, -1f64, 128usize);
    for (i, &c) in hist.iter().enumerate() {
        wb += c as f64;
        if wb == 0.0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0.0 {
            break;
        }
        sb += i as f64 * c as f64;
        let mb = sb / wb;
        let mf = (sum - sb) / wf;
        let between = wb * wf * (mb - mf) * (mb - mf);
        if between > best {
            best = between;
            t = i;
        }
    }
    Bin { w, h, px: lum.iter().map(|&l| (l as usize) <= t).collect() }
}

/// A block-adaptive threshold (8 px blocks, each pixel against the mean
/// black point of the 5×5 blocks around it; flat blocks lean light).
pub fn adaptive(w: usize, h: usize, lum: &[u8]) -> Bin {
    const B: usize = 8;
    if w < B * 5 || h < B * 5 {
        return otsu(w, h, lum);
    }
    let (bw, bh) = (w.div_ceil(B), h.div_ceil(B));
    let mut bp = vec![0u32; bw * bh];
    let blank = || Bin { w, h, px: vec![false; w * h] };
    for by in 0..bh {
        if by % 32 == 0 && late() {
            return blank();
        }
        let y0 = (by * B).min(h - B);
        for bx in 0..bw {
            let x0 = (bx * B).min(w - B);
            let (mut sum, mut mn, mut mx) = (0u32, 255u8, 0u8);
            for y in y0..y0 + B {
                for &l in &lum[y * w + x0..y * w + x0 + B] {
                    sum += l as u32;
                    mn = mn.min(l);
                    mx = mx.max(l);
                }
            }
            let mut avg = sum / (B * B) as u32;
            if mx - mn <= 24 {
                avg = mn as u32 / 2;
                if by > 0 && bx > 0 {
                    let n = (bp[(by - 1) * bw + bx] + 2 * bp[by * bw + bx - 1] + bp[(by - 1) * bw + bx - 1]) / 4;
                    if (mn as u32) < n {
                        avg = n;
                    }
                }
            }
            bp[by * bw + bx] = avg;
        }
    }
    let mut px = vec![false; w * h];
    for by in 0..bh {
        if by % 32 == 0 && late() {
            return blank();
        }
        let y0 = (by * B).min(h - B);
        let cy = by.clamp(2, bh.saturating_sub(3).max(2));
        for bx in 0..bw {
            let x0 = (bx * B).min(w - B);
            let cx = bx.clamp(2, bw.saturating_sub(3).max(2));
            let mut s = 0u32;
            let mut n = 0u32;
            for yy in cy.saturating_sub(2)..(cy + 3).min(bh) {
                for xx in cx.saturating_sub(2)..(cx + 3).min(bw) {
                    s += bp[yy * bw + xx];
                    n += 1;
                }
            }
            let t = s / n.max(1);
            for y in y0..y0 + B {
                for x in x0..x0 + B {
                    px[y * w + x] = (lum[y * w + x] as u32) <= t;
                }
            }
        }
    }
    Bin { w, h, px }
}

/// A 3×3 majority vote: removes salt-and-pepper noise.
pub fn majority(b: &Bin) -> Bin {
    let mut px = b.px.clone();
    for y in 1..b.h.saturating_sub(1) {
        if y % 64 == 0 && late() {
            return Bin { w: b.w, h: b.h, px: vec![false; b.w * b.h] };
        }
        for x in 1..b.w.saturating_sub(1) {
            let mut n = 0;
            for yy in y - 1..=y + 1 {
                n += b.px[yy * b.w + x - 1..=yy * b.w + x + 1].iter().filter(|&&d| d).count();
            }
            px[y * b.w + x] = n >= 5;
        }
    }
    Bin { w: b.w, h: b.h, px }
}

#[derive(Clone, Copy, Debug)]
pub struct Finder {
    pub x: f64,
    pub y: f64,
    /// Module size (px).
    pub m: f64,
    pub hits: u32,
}

fn ratio_ok(r: &[f64; 5]) -> bool {
    let total: f64 = r.iter().sum();
    if total < 7.0 {
        return false;
    }
    let m = total / 7.0;
    let v = m * 0.7;
    (r[0] - m).abs() < v && (r[1] - m).abs() < v && (r[2] - 3.0 * m).abs() < 3.0 * v && (r[3] - m).abs() < v && (r[4] - m).abs() < v
}

/// Runs along a line through `(cx, cy)` in direction `(dx, dy)`:
/// the center dark run's both halves plus two runs each way. Returns the
/// five runs and the center's offset along the line.
fn cross(b: &Bin, cx: f64, cy: f64, dx: f64, dy: f64, max: f64) -> Option<([f64; 5], f64)> {
    let walk = |sign: f64| -> Option<[f64; 3]> {
        // dark (center half), light, dark
        let mut runs = [0f64; 3];
        let mut state = 0;
        let mut t = 0f64;
        let mut prev = (cx.floor() as i64, cy.floor() as i64);
        loop {
            t += 1.0;
            if t > max {
                return None;
            }
            let x = (cx + sign * dx * t).floor() as i64;
            let y = (cy + sign * dy * t).floor() as i64;
            if (x, y) == prev {
                continue;
            }
            prev = (x, y);
            let dark = b.get(x, y);
            let want_dark = state % 2 == 0;
            if dark == want_dark {
                runs[state] += 1.0;
            } else {
                state += 1;
                if state == 3 {
                    return Some(runs);
                }
                runs[state] += 1.0;
            }
        }
    };
    if !b.get(cx.floor() as i64, cy.floor() as i64) {
        return None;
    }
    let a = walk(-1.0)?;
    let c = walk(1.0)?;
    let center = a[0] + c[0] + 1.0;
    let r = [a[2], a[1], center, c[1], c[2]];
    let off = (c[0] - a[0]) / 2.0;
    Some((r, off))
}

/// Finder candidates, strongest first, capped.
pub fn finders(b: &Bin) -> Vec<Finder> {
    let mut found: Vec<Finder> = Vec::new();
    let step = if b.h > 1600 { 2 } else { 1 };
    let mut runs: Vec<(bool, usize, usize)> = Vec::new();
    for y in (0..b.h).step_by(step) {
        if y % 16 == 0 && late() {
            break;
        }
        runs.clear();
        let row = &b.px[y * b.w..(y + 1) * b.w];
        let mut start = 0;
        for x in 1..=b.w {
            if x == b.w || row[x] != row[start] {
                runs.push((row[start], start, x - start));
                start = x;
            }
        }
        if runs.len() < 5 {
            continue;
        }
        for k in 0..runs.len() - 4 {
            if !runs[k].0 {
                continue;
            }
            let r = [runs[k].2, runs[k + 1].2, runs[k + 2].2, runs[k + 3].2, runs[k + 4].2].map(|v| v as f64);
            if !ratio_ok(&r) {
                continue;
            }
            let total: f64 = r.iter().sum();
            let cx = runs[k + 2].1 as f64 + runs[k + 2].2 as f64 / 2.0;
            let max = total * 2.0;
            let Some((vr, voff)) = cross(b, cx, y as f64 + 0.5, 0.0, 1.0, max) else { continue };
            let vt: f64 = vr.iter().sum();
            if !ratio_ok(&vr) || (vt - total).abs() * 5.0 >= 2.0 * total.max(vt) {
                continue;
            }
            let cy = y as f64 + 0.5 + voff;
            let Some((hr, hoff)) = cross(b, cx, cy, 1.0, 0.0, max) else { continue };
            if !ratio_ok(&hr) {
                continue;
            }
            let cx = cx + hoff;
            // a diagonal check rejects most text and stripes
            let Some((dr, _)) = cross(b, cx, cy, 1.0, 1.0, max) else { continue };
            if !ratio_ok(&dr) {
                continue;
            }
            let Some((dr, _)) = cross(b, cx, cy, 1.0, -1.0, max) else { continue };
            if !ratio_ok(&dr) {
                continue;
            }
            let ht: f64 = hr.iter().sum();
            let m = (ht + vt) / 14.0;
            if let Some(f) = found.iter_mut().find(|f| {
                (f.x - cx).abs() <= m.max(f.m) && (f.y - cy).abs() <= m.max(f.m) && (f.m / m).max(m / f.m) < 1.6
            }) {
                let n = f.hits as f64;
                f.x = (f.x * n + cx) / (n + 1.0);
                f.y = (f.y * n + cy) / (n + 1.0);
                f.m = (f.m * n + m) / (n + 1.0);
                f.hits += 1;
            } else if found.len() < 2000 {
                found.push(Finder { x: cx, y: cy, m, hits: 1 });
            }
        }
    }
    if found.iter().any(|f| f.hits >= 2) {
        found.retain(|f| f.hits >= 2);
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.hits));
    found.truncate(30);
    found
}

/// Module size measured along the line from `a` toward `b` (finder runs
/// both ways from the center: 7 modules).
fn module_toward(bin: &Bin, a: &Finder, b: &Finder) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return a.m;
    }
    let (ux, uy) = (dx / len, dy / len);
    match cross(bin, a.x, a.y, ux, uy, a.m * 12.0) {
        Some((r, _)) if ratio_ok(&r) => {
            let t: f64 = r.iter().sum();
            // steps along a diagonal are longer than one pixel
            t * ux.abs().max(uy.abs()).recip() / 7.0
        }
        _ => a.m,
    }
}

/// A 3×3 projective transform (row-major, `[x y 1]` on the right).
#[derive(Clone, Copy, Debug)]
pub struct Persp([f64; 9]);

impl Persp {
    fn square_to_quad(q: [(f64, f64); 4]) -> Persp {
        let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q;
        let dx3 = x0 - x1 + x2 - x3;
        let dy3 = y0 - y1 + y2 - y3;
        if dx3 == 0.0 && dy3 == 0.0 {
            Persp([x1 - x0, x2 - x1, x0, y1 - y0, y2 - y1, y0, 0.0, 0.0, 1.0])
        } else {
            let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
            let den = dx1 * dy2 - dx2 * dy1;
            let a13 = (dx3 * dy2 - dx2 * dy3) / den;
            let a23 = (dx1 * dy3 - dx3 * dy1) / den;
            Persp([
                x1 - x0 + a13 * x1,
                x3 - x0 + a23 * x3,
                x0,
                y1 - y0 + a13 * y1,
                y3 - y0 + a23 * y3,
                y0,
                a13,
                a23,
                1.0,
            ])
        }
    }

    fn adjoint(&self) -> Persp {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        Persp([e * i - f * h, c * h - b * i, b * f - c * e, f * g - d * i, a * i - c * g, c * d - a * f, d * h - e * g, b * g - a * h, a * e - b * d])
    }

    fn times(&self, o: &Persp) -> Persp {
        let a = &self.0;
        let b = &o.0;
        let mut r = [0f64; 9];
        for i in 0..3 {
            for j in 0..3 {
                r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
            }
        }
        Persp(r)
    }

    /// The transform taking quad `from` onto quad `to` (corners in order
    /// top-left, top-right, bottom-right, bottom-left).
    pub fn quad_to_quad(from: [(f64, f64); 4], to: [(f64, f64); 4]) -> Persp {
        let s2q = Persp::square_to_quad(to);
        let q2s = Persp::square_to_quad(from).adjoint();
        s2q.times(&q2s)
    }

    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.0;
        let w = m[6] * x + m[7] * y + m[8];
        ((m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w)
    }
}

/// Alignment pattern candidates within `r` px of `(ex, ey)`, nearest first.
fn alignment_near(b: &Bin, ex: f64, ey: f64, m: f64, r: f64) -> Vec<(f64, f64)> {
    if !ex.is_finite() || !ey.is_finite() || m < 0.5 {
        return Vec::new();
    }
    let ok = |r: &[f64; 3]| {
        let mm = (r[0] + r[1] + r[2]) / 3.0;
        (mm - m).abs() < m * 0.6 && r.iter().all(|&v| (v - mm).abs() < mm * 0.7)
    };
    // light, dark (center), light runs along an axis through (x, y); each
    // is about a module, so a run past 1.5 modules is no alignment pattern
    let runs = |x: i64, y: i64, dx: i64, dy: i64| -> Option<([f64; 3], f64)> {
        let lim = (m * 1.5) as i64 + 2;
        let mut c0 = 0;
        while c0 < lim && b.get(x - dx * (c0 + 1), y - dy * (c0 + 1)) {
            c0 += 1;
        }
        let mut c1 = 0;
        while c1 < lim && b.get(x + dx * (c1 + 1), y + dy * (c1 + 1)) {
            c1 += 1;
        }
        let mut l0 = 0;
        while l0 < lim && !b.get(x - dx * (c0 + 1 + l0), y - dy * (c0 + 1 + l0)) {
            l0 += 1;
        }
        let mut l1 = 0;
        while l1 < lim && !b.get(x + dx * (c1 + 1 + l1), y + dy * (c1 + 1 + l1)) {
            l1 += 1;
        }
        if c0 >= lim || c1 >= lim || l0 >= lim || l1 >= lim {
            return None;
        }
        // the outer dark ring
        let ring = |sign: i64, from: i64| {
            let mut n = 0;
            while n < lim && b.get(x + sign * dx * (from + n), y + sign * dy * (from + n)) {
                n += 1;
            }
            n as f64
        };
        if ring(-1, c0 + 1 + l0) < m * 0.4 || ring(1, c1 + 1 + l1) < m * 0.4 {
            return None;
        }
        Some(([l0 as f64, (c0 + c1 + 1) as f64, l1 as f64], (c1 - c0) as f64 / 2.0))
    };
    let mut found: Vec<(f64, (f64, f64))> = Vec::new();
    let r = r.min(2000.0);
    let (x0, x1) = ((ex - r).floor() as i64, (ex + r).ceil() as i64);
    let (y0, y1) = ((ey - r).floor() as i64, (ey + r).ceil() as i64);
    // The center is a module wide: a third of a module apart still hits it.
    let stride = ((m / 3.0) as usize).max(1);
    for y in (y0..=y1).step_by(stride) {
        if late() {
            break;
        }
        for x in (x0..=x1).step_by(stride) {
            if !b.get(x, y) {
                continue;
            }
            let Some((h, hoff)) = runs(x, y, 1, 0) else { continue };
            if !ok(&h) {
                continue;
            }
            let Some((v, voff)) = runs(x, y, 0, 1) else { continue };
            if !ok(&v) {
                continue;
            }
            let (cx, cy) = (x as f64 + 0.5 + hoff, y as f64 + 0.5 + voff);
            let d = (cx - ex).powi(2) + (cy - ey).powi(2);
            if !found.iter().any(|(_, (fx, fy))| (fx - cx).abs() < m && (fy - cy).abs() < m) {
                found.push((d, (cx, cy)));
            } else if let Some(f) = found.iter_mut().find(|(fd, (fx, fy))| (fx - cx).abs() < m && (fy - cy).abs() < m && d < *fd) {
                *f = (d, (cx, cy));
            }
        }
    }
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    found.into_iter().take(4).map(|f| f.1).collect()
}

/// Whether the timing patterns between the finders alternate (most
/// wrong groupings fail here, before the whole grid is sampled).
fn timing_ok(b: &Bin, t: &Persp, dim: usize) -> bool {
    let mut bad = 0;
    let n = dim.saturating_sub(16);
    for i in 8..dim.saturating_sub(8) {
        for (x, y) in [(i, 6), (6, i)] {
            let (px, py) = t.map(x as f64 + 0.5, y as f64 + 0.5);
            if !px.is_finite() || !py.is_finite() || b.get(px.floor() as i64, py.floor() as i64) != (i % 2 == 0) {
                bad += 1;
            }
        }
    }
    bad * 4 <= n * 2
}

fn sample(b: &Bin, t: &Persp, dim: usize) -> Option<Matrix> {
    let mut m = Matrix::new(dim);
    for y in 0..dim {
        for x in 0..dim {
            let (px, py) = t.map(x as f64 + 0.5, y as f64 + 0.5);
            if !px.is_finite() || !py.is_finite() {
                return None;
            }
            let (ix, iy) = (px.floor() as i64, py.floor() as i64);
            let slack = 2;
            if ix < -slack || iy < -slack || ix >= b.w as i64 + slack || iy >= b.h as i64 + slack {
                return None;
            }
            m.set(x, y, b.get(ix, iy));
        }
    }
    Some(m)
}

/// One decoded code and where it is (corners clockwise from the
/// top-left, picture px).
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub content: Content,
    pub corners: [(f64, f64); 4],
}

fn try_triple(b: &Bin, tl: &Finder, tr: &Finder, bl: &Finder) -> Option<Found> {
    let d_tr = ((tr.x - tl.x).powi(2) + (tr.y - tl.y).powi(2)).sqrt();
    let d_bl = ((bl.x - tl.x).powi(2) + (bl.y - tl.y).powi(2)).sqrt();
    let m_tr = (module_toward(b, tl, tr) + module_toward(b, tr, tl)) / 2.0;
    let m_bl = (module_toward(b, tl, bl) + module_toward(b, bl, tl)) / 2.0;
    if m_tr < 0.5 || m_bl < 0.5 {
        return None;
    }
    let est = (d_tr / m_tr + d_bl / m_bl) / 2.0 + 7.0;
    if !(17.0..=190.0).contains(&est) {
        return None;
    }
    let mut dim = est.round() as i64;
    match dim & 3 {
        0 => dim += 1,
        2 => dim -= 1,
        3 => dim += 2,
        _ => {}
    }
    let mut dims = vec![dim, dim - 4, dim + 4];
    let mut tried = Vec::new();
    while let Some(d) = dims.first().copied() {
        dims.remove(0);
        if late() {
            break;
        }
        if !(21..=177).contains(&d) || tried.contains(&d) {
            continue;
        }
        tried.push(d);
        let dimf = d as f64;
        let m = (m_tr + m_bl) / 2.0;
        let (brx, bry) = (tr.x - tl.x + bl.x, tr.y - tl.y + bl.y);
        let mut brs = Vec::new();
        if d > 21 {
            let k = 1.0 - 3.0 / (dimf - 7.0);
            let ex = tl.x + k * (brx - tl.x);
            let ey = tl.y + k * (bry - tl.y);
            // Within a few modules of where it should be (a strong skew
            // falls back to the affine guess below).
            brs = alignment_near(b, ex, ey, m, m * 3.0);
            if brs.is_empty() {
                brs = alignment_near(b, ex, ey, m, m * 7.0);
            }
            brs.truncate(3);
        }
        let mut quads: Vec<((f64, f64), f64)> = brs.into_iter().map(|p| (p, dimf - 6.5)).collect();
        quads.push(((brx, bry), dimf - 3.5));
        for (p, at) in quads {
            let from = [(3.5, 3.5), (dimf - 3.5, 3.5), (at, at), (3.5, dimf - 3.5)];
            let to = [(tl.x, tl.y), (tr.x, tr.y), p, (bl.x, bl.y)];
            let t = Persp::quad_to_quad(from, to);
            if !timing_ok(b, &t, d as usize) {
                continue;
            }
            let Some(grid) = sample(b, &t, d as usize) else { continue };
            if let Some(v) = matrix::read_version(&grid) {
                let vd = tables::size_of(v) as i64;
                if vd != d && !tried.contains(&vd) && !dims.contains(&vd) {
                    dims.insert(0, vd);
                }
            }
            if let Some(content) = matrix::decode(&grid) {
                let corners = [t.map(0.0, 0.0), t.map(dimf, 0.0), t.map(dimf, dimf), t.map(0.0, dimf)];
                return Some(Found { content, corners });
            }
        }
    }
    None
}

/// All codes found in a binarized picture.
pub fn scan(b: &Bin) -> Vec<Found> {
    let fs = finders(b);
    let mut used = vec![false; fs.len()];
    let mut out: Vec<Found> = Vec::new();
    let n = fs.len();
    for i in 0..n {
        for j in i + 1..n {
            for k in j + 1..n {
                if late() {
                    return out;
                }
                if used[i] || used[j] || used[k] {
                    continue;
                }
                let tri = [fs[i], fs[j], fs[k]];
                let (mn, mx) = tri.iter().fold((f64::MAX, 0f64), |(a, c), f| (a.min(f.m), c.max(f.m)));
                if mx / mn > 2.5 {
                    continue;
                }
                let d2 = |a: &Finder, c: &Finder| (a.x - c.x).powi(2) + (a.y - c.y).powi(2);
                let (dij, djk, dik) = (d2(&tri[0], &tri[1]), d2(&tri[1], &tri[2]), d2(&tri[0], &tri[2]));
                // the corner is opposite the longest side
                let (c, p, q) = if djk >= dij && djk >= dik {
                    (0, 1, 2)
                } else if dik >= dij && dik >= djk {
                    (1, 0, 2)
                } else {
                    (2, 0, 1)
                };
                let (a, mut b1, mut c1) = (tri[c], tri[p], tri[q]);
                let (l1, l2) = (d2(&a, &b1).sqrt(), d2(&a, &c1).sqrt());
                if l1.min(l2) < mn * 9.0 || l1.max(l2) / l1.min(l2) > 2.0 {
                    continue;
                }
                let (ux, uy, vx, vy) = (b1.x - a.x, b1.y - a.y, c1.x - a.x, c1.y - a.y);
                let cos = (ux * vx + uy * vy) / (l1 * l2);
                if cos.abs() > 0.6 {
                    continue;
                }
                if ux * vy - uy * vx < 0.0 {
                    std::mem::swap(&mut b1, &mut c1);
                }
                let hit = try_triple(b, &a, &b1, &c1).or_else(|| try_triple(b, &a, &c1, &b1));
                if let Some(f) = hit {
                    used[i] = true;
                    used[j] = true;
                    used[k] = true;
                    out.push(f);
                }
            }
        }
    }
    out
}

/// The time a read gets by default (debug builds are several times slower).
pub fn default_budget() -> Duration {
    if cfg!(debug_assertions) { Duration::from_secs(4) } else { Duration::from_millis(300) }
}

/// Every code in an RGBA picture (each found once, in reading order),
/// within [`default_budget`].
pub fn decode_rgba(w: usize, h: usize, rgba: &[u8]) -> Vec<Found> {
    decode_rgba_within(w, h, rgba, default_budget())
}

/// [`decode_rgba`] within `budget`: a picture past [`FIRST_PASS_PX`] is
/// read box-averaged to it first (half the budget); at full size only
/// when that finds nothing and it is at most [`FULL_PASS_PX`].
pub fn decode_rgba_within(w: usize, h: usize, rgba: &[u8], budget: Duration) -> Vec<Found> {
    if w == 0 || h == 0 || rgba.len() < w.saturating_mul(h).saturating_mul(4) || w.saturating_mul(h) > 200_000_000 {
        return Vec::new();
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            DEADLINE.with(|d| d.set(None));
        }
    }
    let _reset = Reset;
    let start = Instant::now();
    let px = w * h;
    let mut out = Vec::new();
    if px > FIRST_PASS_PX {
        let f = (px as f64 / FIRST_PASS_PX as f64).sqrt().ceil() as usize;
        DEADLINE.with(|d| d.set(Some(start + budget / 2)));
        let (sw, sh, lum) = luminance_scaled(w, h, rgba, f);
        out = decode_lum(sw, sh, &lum);
        let k = f as f64;
        for o in &mut out {
            o.corners = o.corners.map(|(x, y)| (x * k, y * k));
        }
    }
    DEADLINE.with(|d| d.set(Some(start + budget)));
    if out.is_empty() && px <= FULL_PASS_PX && !late() {
        out = decode_lum(w, h, &luminance(w, h, rgba));
    }
    out
}

fn decode_lum(w: usize, h: usize, lum: &[u8]) -> Vec<Found> {
    let global = otsu(w, h, lum);
    let mut out = scan(&global);
    let mut more = if late() { Vec::new() } else { scan(&adaptive(w, h, lum)) };
    if out.is_empty() && more.is_empty() && !late() {
        more = scan(&majority(&global));
    }
    for f in more {
        let (cx, cy) = center(&f);
        let dup = out.iter().any(|o| {
            let (ox, oy) = center(o);
            o.content.text == f.content.text && (ox - cx).abs() < 20.0 && (oy - cy).abs() < 20.0
        });
        if !dup {
            out.push(f);
        }
    }
    out.sort_by(|a, b| {
        let (ax, ay) = center(a);
        let (bx, by) = center(b);
        (ay, ax).partial_cmp(&(by, bx)).unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

pub fn center(f: &Found) -> (f64, f64) {
    let (sx, sy) = f.corners.iter().fold((0.0, 0.0), |(a, b), c| (a + c.0, b + c.1));
    (sx / 4.0, sy / 4.0)
}
