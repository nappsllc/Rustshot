//! PNG files, hand-rolled on [`crate::deflate`] (replacing the png crate).
//!
//! [`decode`] reads every PNG a screenshot tool meets: gray, RGB, palette,
//! gray + alpha and RGBA, 1 to 16 bits a sample, interlaced (Adam7) or
//! not, `tRNS` transparency, and gives RGBA8 (16-bit samples keep their
//! high byte, low-bit gray is scaled to 0..=255). Chunk CRCs and the zlib
//! Adler-32 are checked; sizes are capped ([`MAX_PIXELS`], the editor's
//! own cap) before anything is allocated, and allocations are fallible: bad
//! or huge input is an error, never a panic or an abort.
//!
//! [`encode`] writes RGBA8 or RGB8: each row with the filter whose output
//! has the smallest sum of absolute values (the usual heuristic), then one
//! zlib stream.

use crate::deflate;

/// Why a PNG did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngError(pub &'static str);

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "png: {}", self.0)
    }
}

impl std::error::Error for PngError {}

/// Largest width or height.
pub const MAX_SIDE: u32 = 1 << 16;
/// Largest picture: what the image editor takes ([`crate::imgedit::MAX_PIXELS`],
/// 200 MP, 800 MB of RGBA).
pub const MAX_PIXELS: u64 = crate::imgedit::MAX_PIXELS;

const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// A decoded picture, RGBA8 rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// CRC-32 (the PNG / zlib one) of the pieces, one after the other.
pub fn crc32(parts: &[&[u8]]) -> u32 {
    let mut c = !0u32;
    for p in parts {
        for &b in *p {
            c = CRC_TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
    }
    !c
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// The header fields that matter.
#[derive(Clone, Copy)]
struct Header {
    w: u32,
    h: u32,
    depth: u8,
    color: u8,
    interlaced: bool,
}

impl Header {
    fn channels(&self) -> usize {
        match self.color {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        }
    }

    /// Bytes of one row of `w` pixels (without the filter byte).
    fn row_bytes(&self, w: u32) -> usize {
        (w as usize * self.channels() * self.depth as usize).div_ceil(8)
    }

    /// The filter's byte distance: one pixel, at least one byte.
    fn bpp(&self) -> usize {
        (self.channels() * self.depth as usize).div_ceil(8)
    }
}

/// Adam7: x0, y0, dx, dy of each pass.
const ADAM7: [(u32, u32, u32, u32); 7] =
    [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

/// The (x0, y0, dx, dy, width, height) of each pass: one pass of the whole
/// picture when not interlaced.
fn passes(h: &Header) -> Vec<(u32, u32, u32, u32, u32, u32)> {
    if !h.interlaced {
        return vec![(0, 0, 1, 1, h.w, h.h)];
    }
    ADAM7
        .iter()
        .map(|&(x0, y0, dx, dy)| {
            let pw = if h.w > x0 { (h.w - x0).div_ceil(dx) } else { 0 };
            let ph = if h.h > y0 { (h.h - y0).div_ceil(dy) } else { 0 };
            (x0, y0, dx, dy, pw, ph)
        })
        .collect()
}

fn err<T>(m: &'static str) -> Result<T, PngError> {
    Err(PngError(m))
}

/// Decode a PNG file into RGBA8.
pub fn decode(b: &[u8]) -> Result<Image, PngError> {
    if b.len() < 8 || b[..8] != SIG {
        return err("not a PNG file");
    }
    let mut pos = 8;
    let mut hdr: Option<Header> = None;
    let mut palette: Vec<[u8; 4]> = Vec::new();
    // tRNS: palette alphas, or the transparent 16-bit gray / RGB sample(s).
    let mut trns_key: Option<[u16; 3]> = None;
    let mut trns_alpha: Option<&[u8]> = None;
    let mut idat: Vec<u8> = Vec::new();
    let (mut seen_idat, mut idat_done) = (false, false);
    loop {
        if pos == b.len() && seen_idat {
            break; // no IEND: what came is enough
        }
        if b.len() - pos < 12 {
            return err("truncated chunk");
        }
        let len = be32(&b[pos..]) as usize;
        let kind: [u8; 4] = b[pos + 4..pos + 8].try_into().unwrap();
        if len > 0x7fff_ffff || b.len() - pos - 12 < len {
            return err("truncated chunk");
        }
        let data = &b[pos + 8..pos + 8 + len];
        if crc32(&[&kind, data]) != be32(&b[pos + 8 + len..]) {
            return err("bad chunk CRC");
        }
        pos += 12 + len;
        if hdr.is_none() && &kind != b"IHDR" {
            return err("IHDR is not first");
        }
        match &kind {
            b"IHDR" => {
                if hdr.is_some() || len != 13 {
                    return err("bad IHDR");
                }
                let h = Header {
                    w: be32(data),
                    h: be32(&data[4..]),
                    depth: data[8],
                    color: data[9],
                    interlaced: data[12] == 1,
                };
                let ok_depth = match h.color {
                    0 => matches!(h.depth, 1 | 2 | 4 | 8 | 16),
                    3 => matches!(h.depth, 1 | 2 | 4 | 8),
                    2 | 4 | 6 => matches!(h.depth, 8 | 16),
                    _ => false,
                };
                if !ok_depth || data[10] != 0 || data[11] != 0 || data[12] > 1 {
                    return err("unsupported IHDR");
                }
                if h.w == 0 || h.h == 0 || h.w > MAX_SIDE || h.h > MAX_SIDE {
                    return err("bad size");
                }
                if h.w as u64 * h.h as u64 > MAX_PIXELS {
                    return err("picture too big");
                }
                hdr = Some(h);
            }
            b"PLTE" => {
                if !len.is_multiple_of(3) || len > 768 || seen_idat {
                    return err("bad PLTE");
                }
                palette = data.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2], 255]).collect();
            }
            b"tRNS" => {
                let h = hdr.unwrap();
                let s = |i: usize| u16::from_be_bytes([data[i], data[i + 1]]);
                match h.color {
                    0 if len >= 2 => trns_key = Some([s(0), 0, 0]),
                    2 if len >= 6 => trns_key = Some([s(0), s(2), s(4)]),
                    3 => trns_alpha = Some(data),
                    _ => {} // ignored, as decoders do
                }
            }
            b"IDAT" => {
                if idat_done {
                    return err("IDAT chunks apart");
                }
                seen_idat = true;
                idat.try_reserve(len).map_err(|_| PngError("out of memory"))?;
                idat.extend_from_slice(data);
            }
            b"IEND" => break,
            _ if kind[0] & 0x20 == 0 => return err("unknown critical chunk"),
            _ => {}
        }
        // Another chunk after the image data: no more IDAT may follow.
        idat_done |= seen_idat && &kind != b"IDAT";
    }
    let h = hdr.ok_or(PngError("no IHDR"))?;
    if !seen_idat {
        return err("no image data");
    }
    if h.color == 3 {
        if palette.is_empty() {
            return err("no palette");
        }
        if let Some(a) = trns_alpha {
            for (p, &v) in palette.iter_mut().zip(a) {
                p[3] = v;
            }
        }
    }
    let ps = passes(&h);
    let raw_len: usize = ps.iter().filter(|p| p.4 > 0).map(|p| p.5 as usize * (1 + h.row_bytes(p.4))).sum();
    let mut raw = deflate::zlib_decompress(&idat, raw_len).map_err(PngError)?;
    if raw.len() != raw_len {
        return err("not enough image data");
    }
    drop(idat);
    let mut out = Vec::new();
    out.try_reserve_exact(h.w as usize * h.h as usize * 4).map_err(|_| PngError("out of memory"))?;
    out.resize(h.w as usize * h.h as usize * 4, 0);
    let bpp = h.bpp();
    let mut at = 0;
    let mut px = Vec::new();
    for &(x0, y0, dx, dy, pw, ph) in &ps {
        if pw == 0 {
            continue;
        }
        let rb = h.row_bytes(pw);
        let zero = vec![0u8; rb];
        px.resize(pw as usize * 4, 0);
        for y in 0..ph {
            // Unfiltered in place: the row above is the pass's previous row.
            let (done, rest) = raw.split_at_mut(at + 1);
            let f = done[at];
            let prev = if y == 0 { &zero[..] } else { &done[at - rb..at] };
            let cur = &mut rest[..rb];
            unfilter(f, cur, prev, bpp)?;
            at += 1 + rb;
            let oy = (y0 + y * dy) as usize;
            if dx == 1 {
                let o = oy * h.w as usize * 4;
                to_rgba(&h, cur, &palette, trns_key, &mut out[o..o + pw as usize * 4])?;
            } else {
                to_rgba(&h, cur, &palette, trns_key, &mut px)?;
                for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
                    let o = (oy * h.w as usize + (x0 + i as u32 * dx) as usize) * 4;
                    out[o..o + 4].copy_from_slice(p);
                }
            }
        }
    }
    Ok(Image { w: h.w, h: h.h, rgba: out })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i16, b as i16, c as i16);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn unfilter(f: u8, cur: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), PngError> {
    let n = cur.len();
    match f {
        0 => {}
        1 if bpp == 4 => {
            // A pixel at a time, as four lanes of a u32.
            let mut last = 0u32;
            for p in cur.as_chunks_mut::<4>().0 {
                let x = u32::from_le_bytes(*p);
                last = ((x & 0x7f7f_7f7f) + (last & 0x7f7f_7f7f)) ^ ((x ^ last) & 0x8080_8080);
                *p = last.to_le_bytes();
            }
        }
        1 => {
            for i in bpp..n {
                cur[i] = cur[i].wrapping_add(cur[i - bpp]);
            }
        }
        2 => {
            let (cs, _) = cur.as_chunks_mut::<8>();
            let words = cs.len();
            for (c, p) in cs.iter_mut().zip(prev.as_chunks::<8>().0) {
                let (x, y) = (u64::from_le_bytes(*c), u64::from_le_bytes(*p));
                *c = (((x & !HI) + (y & !HI)) ^ ((x ^ y) & HI)).to_le_bytes();
            }
            for i in words * 8..n {
                cur[i] = cur[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..n {
                let a = if i >= bpp { cur[i - bpp] } else { 0 };
                cur[i] = cur[i].wrapping_add(((a as u16 + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..n {
                let (a, c) = if i >= bpp { (cur[i - bpp], prev[i - bpp]) } else { (0, 0) };
                cur[i] = cur[i].wrapping_add(paeth(a, prev[i], c));
            }
        }
        _ => return err("bad filter type"),
    }
    Ok(())
}

/// One unfiltered row as RGBA8 into `out` (4 bytes a pixel).
fn to_rgba(h: &Header, row: &[u8], palette: &[[u8; 4]], key: Option<[u16; 3]>, out: &mut [u8]) -> Result<(), PngError> {
    let n = out.len() / 4;
    let d = h.depth as usize;
    // Sample `i` of the row (any depth), as stored.
    let sample = |i: usize| -> u16 {
        match d {
            16 => u16::from_be_bytes([row[2 * i], row[2 * i + 1]]),
            8 => row[i] as u16,
            _ => {
                let bit = i * d;
                ((row[bit / 8] >> (8 - d - bit % 8)) & ((1 << d) - 1) as u8) as u16
            }
        }
    };
    // To 8 bits: the high byte, or low-bit gray scaled up.
    let to8 = |v: u16| -> u8 {
        match d {
            16 => (v >> 8) as u8,
            8 => v as u8,
            _ => (v * (255 / ((1 << d) - 1))) as u8,
        }
    };
    let px = out.as_chunks_mut::<4>().0;
    match h.color {
        0 => {
            for (i, o) in px.iter_mut().enumerate() {
                let v = sample(i);
                let a = if key.is_some_and(|k| k[0] == v) { 0 } else { 255 };
                let g = to8(v);
                *o = [g, g, g, a];
            }
        }
        2 if d == 8 && key.is_none() => {
            for (o, p) in px.iter_mut().zip(row.as_chunks::<3>().0) {
                *o = [p[0], p[1], p[2], 255];
            }
        }
        2 => {
            for (i, o) in px.iter_mut().enumerate() {
                let (r, g, b) = (sample(3 * i), sample(3 * i + 1), sample(3 * i + 2));
                let a = if key == Some([r, g, b]) { 0 } else { 255 };
                *o = [to8(r), to8(g), to8(b), a];
            }
        }
        3 => {
            for (i, o) in px.iter_mut().enumerate() {
                *o = *palette.get(sample(i) as usize).ok_or(PngError("palette index out of range"))?;
            }
        }
        4 => {
            for (i, o) in px.iter_mut().enumerate() {
                let (g, a) = (to8(sample(2 * i)), to8(sample(2 * i + 1)));
                *o = [g, g, g, a];
            }
        }
        _ if d == 8 => out.copy_from_slice(&row[..n * 4]),
        _ => {
            for (i, o) in out.iter_mut().enumerate() {
                *o = row[2 * i];
            }
        }
    }
    Ok(())
}

/// What [`encode`] is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    #[cfg_attr(not(test), allow(dead_code))] // tests' previews
    Rgb,
    Rgba,
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

const HI: u64 = 0x8080_8080_8080_8080;
const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;

/// Byte-wise wrapping `x - y` of 8 bytes at once.
#[inline(always)]
fn sub8(x: u64, y: u64) -> u64 {
    ((x | HI) - (y & !HI)) ^ ((x ^ !y) & HI)
}

/// 1 in each byte of `v` that is not zero (0 elsewhere).
#[inline(always)]
fn nonzero8(v: u64) -> u64 {
    ((((v & LO7) + LO7) | v) & HI) >> 7
}

/// Sum of the byte lanes of `v` (each at most 255).
#[inline(always)]
fn lanes(v: u64) -> u64 {
    let s = (v & 0x00ff_00ff_00ff_00ff) + ((v >> 8) & 0x00ff_00ff_00ff_00ff);
    s.wrapping_mul(0x0001_0001_0001_0001) >> 48
}

#[inline(always)]
fn word(b: &[u8; 8]) -> u64 {
    u64::from_le_bytes(*b)
}

/// The bytes of `cur` that would not be zero filtered with None, Sub and
/// Up (`prev` is the row above, `bpp` the Sub distance): fewer tends to
/// compress better. (Avg and Paeth are not tried: on screenshots they
/// rarely win, and they cost most of the encoding time.)
fn row_scores(cur: &[u8], prev: &[u8], bpp: usize) -> [u64; 3] {
    let n = cur.len();
    let mut tot = [0u64; 3];
    for i in 0..bpp.min(n) {
        let nz = (cur[i] != 0) as u64;
        tot[0] += nz;
        tot[1] += nz;
        tot[2] += (cur[i] != prev[i]) as u64;
    }
    if n <= bpp {
        return tot;
    }
    let (xs, xt) = cur[bpp..].as_chunks::<8>();
    let (as_, _) = cur[..n - bpp].as_chunks::<8>();
    let (bs, _) = prev[bpp..].as_chunks::<8>();
    let words = xs.len();
    assert!(as_.len() == words && bs.len() == words);
    // Index loops (iterator adapters may stay calls at opt-level "z").
    let mut k = 0;
    while k < words {
        let end = words.min(k + 255); // a lane counts to 255 at most
        let (mut a0, mut a1, mut a2) = (0u64, 0u64, 0u64);
        while k < end {
            let x = word(&xs[k]);
            a0 += nonzero8(x);
            a1 += nonzero8(sub8(x, word(&as_[k])));
            a2 += nonzero8(sub8(x, word(&bs[k])));
            k += 1;
        }
        tot[0] += lanes(a0);
        tot[1] += lanes(a1);
        tot[2] += lanes(a2);
    }
    let t0 = n - xt.len();
    for i in t0..n {
        tot[0] += (cur[i] != 0) as u64;
        tot[1] += (cur[i] != cur[i - bpp]) as u64;
        tot[2] += (cur[i] != prev[i]) as u64;
    }
    tot
}

/// Append `cur` filtered with None (0), Sub (1) or Up (2) to `out`.
fn push_filtered(f: u8, cur: &[u8], prev: &[u8], bpp: usize, out: &mut Vec<u8>) {
    let n = cur.len();
    let start = out.len();
    out.extend_from_slice(cur);
    if f == 0 {
        return;
    }
    let o = &mut out[start..];
    let (from, back): (usize, &[u8]) = if f == 1 { (bpp.min(n), &cur[..n - bpp.min(n)]) } else { (0, prev) };
    let (os, _) = o[from..].as_chunks_mut::<8>();
    let (xs, _) = cur[from..].as_chunks::<8>();
    let (ys, _) = back.as_chunks::<8>();
    let words = os.len();
    assert!(xs.len() == words && ys.len() >= words);
    for k in 0..words {
        os[k] = sub8(word(&xs[k]), word(&ys[k])).to_le_bytes();
    }
    for i in from + words * 8..n {
        let y = if f == 1 { cur[i - bpp] } else { prev[i] };
        o[i] = cur[i].wrapping_sub(y);
    }
}

/// Filter `cur` (the row above is `prev`) with filter `f` into `out`.
#[cfg(test)]
fn filter_row(f: u8, cur: &[u8], prev: &[u8], bpp: usize, out: &mut [u8]) {
    let n = cur.len();
    match f {
        0 => out.copy_from_slice(cur),
        1 => {
            out[..bpp].copy_from_slice(&cur[..bpp]);
            for i in bpp..n {
                out[i] = cur[i].wrapping_sub(cur[i - bpp]);
            }
        }
        2 => {
            for i in 0..n {
                out[i] = cur[i].wrapping_sub(prev[i]);
            }
        }
        3 => {
            for i in 0..n {
                let a = if i >= bpp { cur[i - bpp] } else { 0 };
                out[i] = cur[i].wrapping_sub(((a as u16 + prev[i] as u16) / 2) as u8);
            }
        }
        _ => {
            for i in 0..n {
                let (a, c) = if i >= bpp { (cur[i - bpp], prev[i - bpp]) } else { (0, 0) };
                out[i] = cur[i].wrapping_sub(paeth(a, prev[i], c));
            }
        }
    }
}

/// Rows `r.0..r.1` of `data` filtered (the row above the first is in
/// `data` too) and compressed: a DEFLATE part, its input's Adler-32 and
/// length.
fn compress_rows(data: &[u8], rb: usize, ch: usize, r: (usize, usize), last: bool) -> (Vec<u8>, u32, usize) {
    let mut z = deflate::Deflater::new(deflate::Effort::DEFAULT, false);
    let zero = vec![0u8; rb];
    let mut row = Vec::with_capacity(rb + 1);
    for y in r.0..r.1 {
        let cur = &data[y * rb..(y + 1) * rb];
        let prev = if y == 0 { &zero[..] } else { &data[(y - 1) * rb..y * rb] };
        let s = row_scores(cur, prev, ch);
        // Ties go to the simpler filter (None, then Sub); Up equals None on the first row.
        let f = if s[0] <= s[1] && s[0] <= s[2] { 0 } else if s[1] <= s[2] { 1 } else { 2 };
        row.clear();
        row.push(f);
        push_filtered(f, cur, prev, ch, &mut row);
        z.write(&row);
    }
    let (part, adler) = z.finish_part(last);
    (part, adler, (r.1 - r.0) * (rb + 1))
}

/// A PNG of `w` x `h` pixels of 8-bit RGB or RGBA `data` (rows top to
/// bottom, no padding). `data` must be exactly that long.
pub fn encode(w: u32, h: u32, color: Color, data: &[u8]) -> Vec<u8> {
    let ch = if color == Color::Rgba { 4 } else { 3 };
    let rb = w as usize * ch;
    assert_eq!(data.len(), rb * h as usize, "png: data size");
    // Big pictures in bands compressed on threads of their own, each a
    // part of one DEFLATE stream (a band's matches do not reach into the
    // band above: a few hundred bytes).
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    let bands = threads.min(data.len() / (1 << 20)).clamp(1, h as usize);
    let per = (h as usize).div_ceil(bands);
    let rows: Vec<(usize, usize)> = (0..bands).map(|b| (b * per, ((b + 1) * per).min(h as usize))).filter(|r| r.0 < r.1).collect();
    let last = rows.len() - 1;
    let parts: Vec<(Vec<u8>, u32, usize)> = if rows.len() == 1 {
        vec![compress_rows(data, rb, ch, rows[0], true)]
    } else {
        std::thread::scope(|s| {
            let hs: Vec<_> = rows
                .iter()
                .enumerate()
                .map(|(i, &r)| s.spawn(move || compress_rows(data, rb, ch, r, i == last)))
                .collect();
            hs.into_iter().map(|h| h.join().expect("png band")).collect()
        })
    };
    let mut z = vec![0x78, 0x9c];
    let mut adler = 1u32;
    for (part, a, len) in parts {
        z.extend_from_slice(&part);
        adler = deflate::adler32_combine(adler, a, len);
    }
    z.extend_from_slice(&adler.to_be_bytes());
    let mut out = Vec::with_capacity(z.len() + 64);
    out.extend_from_slice(&SIG);
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&w.to_be_bytes());
    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = if color == Color::Rgba { 6 } else { 2 };
    chunk(&mut out, b"IHDR", &ihdr);
    for part in z.chunks(1 << 20) {
        chunk(&mut out, b"IDAT", part);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
#[path = "pngfile_tests.rs"]
mod tests;
