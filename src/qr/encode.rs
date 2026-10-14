//! QR code encoder: segments → codewords → blocks with Reed–Solomon check
//! bytes → module matrix, with the mask of lowest penalty.

use super::gf;
use super::tables::{self, Ecl, Matrix};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // numeric and alphanumeric: the decoder's tests
pub enum Mode {
    Numeric,
    Alphanumeric,
    Byte,
}

pub const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

impl Mode {
    fn indicator(self) -> u32 {
        match self {
            Mode::Numeric => 1,
            Mode::Alphanumeric => 2,
            Mode::Byte => 4,
        }
    }

    pub fn count_bits(self, ver: usize) -> usize {
        let i = if ver <= 9 { 0 } else if ver <= 26 { 1 } else { 2 };
        match self {
            Mode::Numeric => [10, 12, 14][i],
            Mode::Alphanumeric => [9, 11, 13][i],
            Mode::Byte => [8, 16, 16][i],
        }
    }
}

/// A run of input in one mode, its payload already packed into bits.
#[derive(Clone, Debug)]
pub struct Segment {
    pub mode: Mode,
    pub count: usize,
    pub bits: Vec<bool>,
}

fn push(bits: &mut Vec<bool>, v: u32, n: usize) {
    for i in (0..n).rev() {
        bits.push((v >> i) & 1 != 0);
    }
}

impl Segment {
    pub fn bytes(d: &[u8]) -> Segment {
        let mut bits = Vec::with_capacity(d.len() * 8);
        for &b in d {
            push(&mut bits, b as u32, 8);
        }
        Segment { mode: Mode::Byte, count: d.len(), bits }
    }

    /// Digits only (`None` otherwise).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn numeric(s: &str) -> Option<Segment> {
        let d = s.as_bytes();
        if !d.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let mut bits = Vec::new();
        for c in d.chunks(3) {
            let v = c.iter().fold(0u32, |a, &x| a * 10 + (x - b'0') as u32);
            push(&mut bits, v, c.len() * 3 + 1);
        }
        Some(Segment { mode: Mode::Numeric, count: d.len(), bits })
    }

    /// The 45-character alphanumeric set only (`None` otherwise).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn alphanumeric(s: &str) -> Option<Segment> {
        let idx: Option<Vec<u32>> =
            s.bytes().map(|b| ALNUM.iter().position(|&a| a == b).map(|p| p as u32)).collect();
        let idx = idx?;
        let mut bits = Vec::new();
        for c in idx.chunks(2) {
            if c.len() == 2 {
                push(&mut bits, c[0] * 45 + c[1], 11);
            } else {
                push(&mut bits, c[0], 6);
            }
        }
        Some(Segment { mode: Mode::Alphanumeric, count: idx.len(), bits })
    }

    /// The most compact single segment for `s`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn auto(s: &str) -> Segment {
        Segment::numeric(s)
            .or_else(|| Segment::alphanumeric(s))
            .unwrap_or_else(|| Segment::bytes(s.as_bytes()))
    }
}

/// An encoded symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCode {
    pub version: usize,
    pub ecl: Ecl,
    pub mask: u8,
    pub matrix: Matrix,
}

impl QrCode {
    pub fn size(&self) -> usize {
        self.matrix.size
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        self.matrix.get(x, y)
    }
}

/// Bits the segments need at a version (`None` if a count overflows).
fn total_bits(segs: &[Segment], ver: usize) -> Option<usize> {
    let mut n = 0usize;
    for s in segs {
        let cb = s.mode.count_bits(ver);
        if s.count >= 1 << cb {
            return None;
        }
        n += 4 + cb + s.bits.len();
    }
    Some(n)
}

/// The data codewords (with terminator and padding) at a fixed version.
pub fn data_codewords(segs: &[Segment], ver: usize, ecl: Ecl) -> Option<Vec<u8>> {
    let cap = tables::num_data_codewords(ver, ecl) * 8;
    if total_bits(segs, ver)? > cap {
        return None;
    }
    let mut bits = Vec::with_capacity(cap);
    for s in segs {
        push(&mut bits, s.mode.indicator(), 4);
        push(&mut bits, s.count as u32, s.mode.count_bits(ver));
        bits.extend_from_slice(&s.bits);
    }
    let term = (cap - bits.len()).min(4);
    push(&mut bits, 0, term);
    let pad = (8 - bits.len() % 8) % 8;
    push(&mut bits, 0, pad);
    let mut out: Vec<u8> = bits
        .chunks(8)
        .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | b as u8))
        .collect();
    let mut p = 0xECu8;
    while out.len() < cap / 8 {
        out.push(p);
        p ^= 0xEC ^ 0x11;
    }
    Some(out)
}

/// Codewords split into blocks, each followed by its check bytes, then
/// interleaved for placement.
pub fn add_ecc_and_interleave(data: &[u8], ver: usize, ecl: Ecl) -> Vec<u8> {
    let (lens, ecc) = tables::block_layout(ver, ecl);
    let mut blocks = Vec::with_capacity(lens.len());
    let mut k = 0;
    for &len in &lens {
        let mut b = data[k..k + len].to_vec();
        k += len;
        let r = gf::remainder(&b, ecc);
        b.extend_from_slice(&r);
        blocks.push(b);
    }
    tables::interleave_order(&lens, ecc).into_iter().map(|(j, i)| blocks[j][i]).collect()
}

/// Encode segments at the smallest version in `min_ver..=40` that fits;
/// `mask: None` picks the mask of lowest penalty.
pub fn encode_segments(segs: &[Segment], ecl: Ecl, min_ver: usize, mask: Option<u8>) -> Option<QrCode> {
    let ver = (min_ver.max(1)..=40).find(|&v| data_codewords(segs, v, ecl).is_some())?;
    let data = data_codewords(segs, ver, ecl)?;
    let cw = add_ecc_and_interleave(&data, ver, ecl);
    let (base, func) = tables::function_patterns(ver);
    let mut m = base;
    for (i, (x, y)) in tables::data_positions(&func).into_iter().enumerate() {
        let byte = cw.get(i / 8).copied().unwrap_or(0);
        m.set(x, y, (byte >> (7 - i % 8)) & 1 != 0);
    }
    let apply = |mask: u8| {
        let mut out = m.clone();
        for y in 0..out.size {
            for x in 0..out.size {
                if !func.get(x, y) && tables::mask_bit(mask, x, y) {
                    out.set(x, y, !out.get(x, y));
                }
            }
        }
        draw_format(&mut out, ecl, mask, ver);
        out
    };
    let (mask, matrix) = match mask {
        Some(k) => (k & 7, apply(k & 7)),
        None => (0..8u8)
            .map(|k| (k, apply(k)))
            .min_by_key(|(_, mm)| penalty(mm))
            .expect("eight masks"),
    };
    Some(QrCode { version: ver, ecl, mask, matrix })
}

/// Text as UTF-8 bytes in byte mode (the Make QR code window's encoding).
pub fn encode_text(text: &str, ecl: Ecl) -> Option<QrCode> {
    encode_segments(&[Segment::bytes(text.as_bytes())], ecl, 1, None)
}

fn draw_format(m: &mut Matrix, ecl: Ecl, mask: u8, ver: usize) {
    let size = m.size;
    let f = tables::format_bits(ecl, mask);
    for i in 0..15 {
        for (x, y) in tables::format_positions(size, i) {
            m.set(x, y, (f >> i) & 1 != 0);
        }
    }
    m.set(8, size - 8, true);
    if ver >= 7 {
        let v = tables::version_bits(ver);
        for i in 0..18 {
            for (x, y) in tables::version_positions(size, i) {
                m.set(x, y, (v >> i) & 1 != 0);
            }
        }
    }
}

/// The standard's mask penalty (runs, 2×2 blocks, finder-like patterns,
/// dark balance).
pub fn penalty(m: &Matrix) -> u32 {
    let n = m.size;
    let mut p = 0u32;
    let line = |get: &dyn Fn(usize) -> bool, p: &mut u32| {
        let mut run = 1;
        for i in 1..n {
            if get(i) == get(i - 1) {
                run += 1;
                if run == 5 {
                    *p += 3;
                } else if run > 5 {
                    *p += 1;
                }
            } else {
                run = 1;
            }
        }
        const PAT: [bool; 11] = [true, false, true, true, true, false, true, false, false, false, false];
        for i in 0..n.saturating_sub(10) {
            if (0..11).all(|k| get(i + k) == PAT[k]) || (0..11).all(|k| get(i + k) == PAT[10 - k]) {
                *p += 40;
            }
        }
    };
    for y in 0..n {
        line(&|x| m.get(x, y), &mut p);
    }
    for x in 0..n {
        line(&|y| m.get(x, y), &mut p);
    }
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let c = m.get(x, y);
            if c == m.get(x + 1, y) && c == m.get(x, y + 1) && c == m.get(x + 1, y + 1) {
                p += 3;
            }
        }
    }
    let dark = m.cells.iter().filter(|&&c| c).count();
    let total = n * n;
    let pct = dark * 100 / total;
    p += (pct.abs_diff(50) / 5) as u32 * 10;
    p
}
