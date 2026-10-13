//! Decode a sampled module matrix: format and version information, unmask,
//! de-interleave, Reed–Solomon correction, and the data segments.
//! Every step checks its input; nothing here may panic on a hostile grid.

use super::encode::ALNUM;
use super::gf;
use super::tables::{self, Ecl, Matrix};

/// The nearest valid format `(level, mask)` within 3 bit errors.
pub fn read_format(m: &Matrix) -> Option<(Ecl, u8)> {
    let size = m.size;
    let mut best: Option<(u32, Ecl, u8)> = None;
    for copy in 0..2 {
        let mut bits = 0u16;
        for i in 0..15 {
            let (x, y) = tables::format_positions(size, i)[copy];
            if m.get(x, y) {
                bits |= 1 << i;
            }
        }
        for ecl in Ecl::ALL {
            for mask in 0..8 {
                let d = (tables::format_bits(ecl, mask) ^ bits).count_ones();
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, ecl, mask));
                }
            }
        }
    }
    best.filter(|b| b.0 <= 3).map(|(_, e, k)| (e, k))
}

/// The version read from a version 7+ symbol's version blocks.
pub fn read_version(m: &Matrix) -> Option<usize> {
    let size = m.size;
    if size < 45 {
        return None;
    }
    let mut best: Option<(u32, usize)> = None;
    for copy in 0..2 {
        let mut bits = 0u32;
        for i in 0..18 {
            let (x, y) = tables::version_positions(size, i)[copy];
            if m.get(x, y) {
                bits |= 1 << i;
            }
        }
        for v in 7..=40 {
            let d = (tables::version_bits(v) ^ bits).count_ones();
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, v));
            }
        }
    }
    best.filter(|b| b.0 <= 3).map(|b| b.1)
}

/// What a symbol held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub text: String,
    pub version: usize,
    pub ecl: Ecl,
    /// Bytes the error correction fixed.
    pub corrected: usize,
}

/// Decode a matrix, also trying it mirrored.
pub fn decode(m: &Matrix) -> Option<Content> {
    decode_one(m).or_else(|| decode_one(&m.transposed()))
}

fn decode_one(m: &Matrix) -> Option<Content> {
    let size = m.size;
    if !(21..=177).contains(&size) || size % 4 != 1 || m.cells.len() != size * size {
        return None;
    }
    let ver = (size - 17) / 4;
    let (ecl, mask) = read_format(m)?;
    let raw = tables::raw_data_modules(ver) / 8;
    let mut cw = vec![0u8; raw];
    for (i, &(x, y)) in tables::cached_data_positions(ver).iter().enumerate() {
        if i / 8 >= raw {
            break;
        }
        if m.get(x, y) ^ tables::mask_bit(mask, x, y) {
            cw[i / 8] |= 0x80 >> (i % 8);
        }
    }
    let (lens, ecc) = tables::block_layout(ver, ecl);
    let mut blocks: Vec<Vec<u8>> = lens.iter().map(|&l| vec![0u8; l + ecc]).collect();
    for (k, (j, i)) in tables::interleave_order(&lens, ecc).into_iter().enumerate() {
        blocks[j][i] = *cw.get(k)?;
    }
    let mut data = Vec::new();
    let mut corrected = 0;
    for (b, &len) in blocks.iter_mut().zip(&lens) {
        corrected += gf::correct(b, ecc)?;
        data.extend_from_slice(&b[..len]);
    }
    let text = parse(&data, ver)?;
    Some(Content { text, version: ver, ecl, corrected })
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn left(&self) -> usize {
        self.d.len() * 8 - self.pos
    }

    fn read(&mut self, n: usize) -> Option<u32> {
        if n > 32 || n > self.left() {
            return None;
        }
        let mut v = 0u32;
        for _ in 0..n {
            let b = (self.d[self.pos / 8] >> (7 - self.pos % 8)) & 1;
            v = (v << 1) | b as u32;
            self.pos += 1;
        }
        Some(v)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Charset {
    Guess,
    Utf8,
    Latin1,
}

fn flush(bytes: &mut Vec<u8>, cs: Charset, out: &mut String) {
    if bytes.is_empty() {
        return;
    }
    let s = match cs {
        Charset::Latin1 => None,
        Charset::Utf8 => Some(String::from_utf8_lossy(bytes).into_owned()),
        Charset::Guess => String::from_utf8(bytes.clone()).ok(),
    };
    match s {
        Some(s) => out.push_str(&s),
        None => out.extend(bytes.iter().map(|&b| b as char)),
    }
    bytes.clear();
}

/// The data segments as text: numeric, alphanumeric, byte (UTF-8 or
/// ISO-8859-1, by ECI or by guess), ECI; Kanji and structured append/FNC1
/// headers are skipped.
pub fn parse(data: &[u8], ver: usize) -> Option<String> {
    let mut r = Bits { d: data, pos: 0 };
    let mut out = String::new();
    let mut bytes = Vec::new();
    let mut cs = Charset::Guess;
    let tier = if ver <= 9 { 0 } else if ver <= 26 { 1 } else { 2 };
    while r.left() >= 4 {
        let mode = r.read(4)?;
        match mode {
            0 => break,
            1 => {
                flush(&mut bytes, cs, &mut out);
                let mut n = r.read([10, 12, 14][tier])? as usize;
                while n > 0 {
                    let (k, bits, lim) = match n {
                        1 => (1, 4, 10),
                        2 => (2, 7, 100),
                        _ => (3, 10, 1000),
                    };
                    let v = r.read(bits)?;
                    if v >= lim {
                        return None;
                    }
                    let s = format!("{v:0k$}");
                    out.push_str(&s);
                    n -= k;
                }
            }
            2 => {
                flush(&mut bytes, cs, &mut out);
                let mut n = r.read([9, 11, 13][tier])? as usize;
                while n >= 2 {
                    let v = r.read(11)? as usize;
                    if v >= 45 * 45 {
                        return None;
                    }
                    out.push(ALNUM[v / 45] as char);
                    out.push(ALNUM[v % 45] as char);
                    n -= 2;
                }
                if n == 1 {
                    let v = r.read(6)? as usize;
                    out.push(*ALNUM.get(v)? as char);
                }
            }
            4 => {
                let n = r.read([8, 16, 16][tier])? as usize;
                if n * 8 > r.left() {
                    return None;
                }
                for _ in 0..n {
                    bytes.push(r.read(8)? as u8);
                }
            }
            8 => {
                flush(&mut bytes, cs, &mut out);
                let n = r.read([8, 10, 12][tier])? as usize;
                if n * 13 > r.left() {
                    return None;
                }
                r.pos += n * 13;
            }
            7 => {
                flush(&mut bytes, cs, &mut out);
                let a = r.read(8)?;
                let id = if a & 0x80 == 0 {
                    a
                } else if a & 0xC0 == 0x80 {
                    ((a & 0x3F) << 8) | r.read(8)?
                } else if a & 0xE0 == 0xC0 {
                    ((a & 0x1F) << 16) | r.read(16)?
                } else {
                    return None;
                };
                cs = match id {
                    26 => Charset::Utf8,
                    1 | 3 => Charset::Latin1,
                    _ => Charset::Guess,
                };
            }
            3 => {
                r.read(16)?;
            }
            5 => {}
            9 => {
                r.read(8)?;
            }
            _ => return None,
        }
    }
    flush(&mut bytes, cs, &mut out);
    Some(out)
}
