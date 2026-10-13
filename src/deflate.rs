//! DEFLATE (RFC 1951) and its zlib wrapper (RFC 1950), hand-rolled: an
//! inflater (for PNG files and the compressed data embedded in the binary)
//! and a compressor (hash-chain LZ77 with lazy matching, then a fixed,
//! dynamic or stored block, whichever is smallest).
//!
//! build.rs includes this file too (`#[path]`), to compress the embedded
//! data at build time, so it uses nothing from the crate.
#![allow(dead_code)] // each of the crate and build.rs uses a part

/// Why a stream did not inflate.
pub type Error = &'static str;

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097,
    6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order of the code length code lengths in a dynamic block header.
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// The fixed literal/length code lengths (and 30 fixed distance codes of 5 bits).
fn fixed_lengths() -> [u8; 288] {
    let mut l = [8u8; 288];
    l[144..256].fill(9);
    l[256..280].fill(7);
    l
}

// ------------------------------------------------------------------ inflate

/// LSB-first bit reader. Past the end it reads zeros, counting them in
/// `over`; a stream that needs them is truncated.
struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    buf: u64,
    n: u32,
    over: usize,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8]) -> Self {
        Bits { src, pos: 0, buf: 0, n: 0, over: 0 }
    }

    #[inline(always)]
    fn refill(&mut self) {
        if self.pos + 8 <= self.src.len() {
            let w = u64::from_le_bytes(self.src[self.pos..self.pos + 8].try_into().unwrap());
            self.buf |= w << self.n;
            self.pos += ((63 - self.n) / 8) as usize;
            self.n |= 56;
        } else {
            while self.n <= 56 {
                let b = match self.src.get(self.pos) {
                    Some(&b) => b,
                    None => {
                        self.over += 1;
                        0
                    }
                };
                self.pos += 1;
                self.buf |= (b as u64) << self.n;
                self.n += 8;
            }
        }
    }

    /// `k` <= 32 bits (the buffer must hold them: call `refill` first).
    #[inline(always)]
    fn take(&mut self, k: u32) -> u32 {
        debug_assert!(k <= self.n && k <= 32);
        let v = (self.buf & ((1u64 << k) - 1)) as u32;
        self.buf >>= k;
        self.n -= k;
        v
    }

    fn bits(&mut self, k: u32) -> u32 {
        if self.n < k {
            self.refill();
        }
        self.take(k)
    }

    /// Bytes consumed so far (rounded up), and whether that is past the end.
    fn consumed(&self) -> usize {
        self.pos - (self.n / 8) as usize
    }

    fn truncated(&self) -> bool {
        self.consumed() > self.src.len() || self.over > 16
    }
}

/// Fast lookup bits of a Huffman table; longer codes take the slow path.
const FAST: u32 = 10;

/// A canonical Huffman decoding table.
struct Huff {
    /// By the next `FAST` input bits: `symbol << 4 | length`, 0 for a
    /// longer code.
    fast: Vec<u16>,
    counts: [u16; 16],
    /// Symbols in canonical code order.
    syms: Vec<u16>,
}

impl Huff {
    fn new(lens: &[u8]) -> Result<Huff, Error> {
        let mut counts = [0u16; 16];
        for &l in lens {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left = (left << 1) - c as i32;
            if left < 0 {
                return Err("deflate: over-subscribed code");
            }
        }
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + counts[l];
        }
        let mut syms = vec![0u16; lens.len()];
        for (s, &l) in lens.iter().enumerate() {
            if l != 0 {
                syms[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        let mut fast = vec![0u16; 1 << FAST];
        let mut code = 0u32;
        let mut k = 0usize;
        for l in 1..=FAST {
            for _ in 0..counts[l as usize] {
                let rev = code.reverse_bits() >> (32 - l);
                let e = (syms[k] << 4) | l as u16;
                let mut i = rev as usize;
                while i < 1 << FAST {
                    fast[i] = e;
                    i += 1 << l;
                }
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        Ok(Huff { fast, counts, syms })
    }

    #[inline(always)]
    fn decode(&self, b: &mut Bits) -> Result<u16, Error> {
        if b.n < 15 {
            b.refill();
        }
        let e = self.fast[(b.buf & ((1 << FAST) - 1)) as usize];
        if e != 0 {
            b.take((e & 15) as u32);
            return Ok(e >> 4);
        }
        // Canonical decode, one bit at a time (puff's way).
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for l in 1..16 {
            code |= b.take(1) as i32;
            let count = self.counts[l] as i32;
            if code - first < count {
                return Ok(self.syms[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("deflate: bad code")
    }
}

/// Inflate a raw DEFLATE stream, at most `limit` bytes of output. Returns
/// the data and the input bytes the stream took.
pub fn inflate(src: &[u8], limit: usize) -> Result<(Vec<u8>, usize), Error> {
    let mut out = Vec::new();
    let used = inflate_into(src, limit, &mut out)?;
    Ok((out, used))
}

fn inflate_into(src: &[u8], limit: usize, out: &mut Vec<u8>) -> Result<usize, Error> {
    // Room for what the input could give (DEFLATE expands at most 1032:1),
    // up to the limit: no regrowing on the way.
    out.reserve(limit.min(src.len().saturating_mul(1032)));
    let mut b = Bits::new(src);
    loop {
        let last = b.bits(1);
        match b.bits(2) {
            0 => {
                let drop = b.n % 8;
                b.take(drop);
                let len = b.bits(16);
                let nlen = b.bits(16);
                if len != !nlen & 0xffff {
                    return Err("deflate: bad stored block");
                }
                let len = len as usize;
                if out.len() + len > limit {
                    return Err("deflate: too much data");
                }
                // Whole bytes still in the bit buffer, then straight from the input.
                let mut k = 0;
                while k < len && b.n >= 8 {
                    out.push(b.take(8) as u8);
                    k += 1;
                }
                if b.n == 0 {
                    b.buf = 0; // it may hold look-ahead bits of the bytes copied below
                }
                let rest = len - k;
                if b.pos + rest > src.len() {
                    return Err("deflate: truncated");
                }
                out.extend_from_slice(&src[b.pos..b.pos + rest]);
                b.pos += rest;
            }
            1 => {
                let l = fixed_lengths();
                let lit = Huff::new(&l)?;
                let dist = Huff::new(&[5u8; 30])?;
                codes(&mut b, &lit, &dist, out, limit)?;
            }
            2 => {
                let (lit, dist) = dynamic(&mut b)?;
                codes(&mut b, &lit, &dist, out, limit)?;
            }
            _ => return Err("deflate: bad block type"),
        }
        if b.truncated() {
            return Err("deflate: truncated");
        }
        if last == 1 {
            return Ok(b.consumed());
        }
    }
}

fn dynamic(b: &mut Bits) -> Result<(Huff, Huff), Error> {
    let nlen = b.bits(5) as usize + 257;
    let ndist = b.bits(5) as usize + 1;
    let ncode = b.bits(4) as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err("deflate: bad counts");
    }
    let mut cl = [0u8; 19];
    for &i in &CL_ORDER[..ncode] {
        cl[i] = b.bits(3) as u8;
    }
    let clh = Huff::new(&cl)?;
    let mut lens = [0u8; 286 + 30];
    let mut i = 0;
    while i < nlen + ndist {
        let s = clh.decode(b)?;
        let (v, n) = match s {
            0..=15 => {
                lens[i] = s as u8;
                i += 1;
                continue;
            }
            16 => {
                if i == 0 {
                    return Err("deflate: repeat with no length");
                }
                (lens[i - 1], 3 + b.bits(2) as usize)
            }
            17 => (0, 3 + b.bits(3) as usize),
            _ => (0, 11 + b.bits(7) as usize),
        };
        if i + n > nlen + ndist {
            return Err("deflate: too many lengths");
        }
        lens[i..i + n].fill(v);
        i += n;
        if b.truncated() {
            return Err("deflate: truncated");
        }
    }
    if lens[256] == 0 {
        return Err("deflate: no end-of-block code");
    }
    Ok((Huff::new(&lens[..nlen])?, Huff::new(&lens[nlen..nlen + ndist])?))
}

fn codes(b: &mut Bits, lit: &Huff, dist: &Huff, out: &mut Vec<u8>, limit: usize) -> Result<(), Error> {
    loop {
        if b.over > 16 {
            return Err("deflate: truncated");
        }
        let s = lit.decode(b)? as usize;
        if s < 256 {
            if out.len() >= limit {
                return Err("deflate: too much data");
            }
            out.push(s as u8);
            continue;
        }
        if s == 256 {
            return Ok(());
        }
        let s = s - 257;
        if s >= 29 {
            return Err("deflate: bad length code");
        }
        let len = LEN_BASE[s] as usize + b.bits(LEN_EXTRA[s] as u32) as usize;
        let d = dist.decode(b)? as usize;
        if d >= 30 {
            return Err("deflate: bad distance code");
        }
        let d = DIST_BASE[d] as usize + b.bits(DIST_EXTRA[d] as u32) as usize;
        if d > out.len() {
            return Err("deflate: distance too far back");
        }
        if out.len() + len > limit {
            return Err("deflate: too much data");
        }
        let start = out.len() - d;
        if d >= len {
            out.extend_from_within(start..start + len);
        } else if d == 1 {
            let v = out[start];
            out.resize(out.len() + len, v);
        } else {
            for k in 0..len {
                let v = out[start + k];
                out.push(v);
            }
        }
    }
}

/// Adler-32 of `data`, continuing from `a` (1 to start).
pub fn adler32(a: u32, data: &[u8]) -> u32 {
    const M: u32 = 65521;
    let (mut s1, mut s2) = (a & 0xffff, a >> 16);
    // 5552 bytes keep the sums in u32. 8 bytes at a time: s2 gains 8 * s1
    // plus the bytes weighted 8..1, sums done as 16-bit lanes of a u64
    // (multiplying by a constant adds the lanes, weighted, into the top one).
    const E: u64 = 0x00ff_00ff_00ff_00ff;
    for chunk in data.chunks(5552) {
        let (words, tail) = chunk.as_chunks::<8>();
        for w in words {
            let w = u64::from_le_bytes(*w);
            let (even, odd) = (w & E, (w >> 8) & E);
            let sum = ((even + odd).wrapping_mul(0x0001_0001_0001_0001) >> 48) as u32;
            let weighted = (even.wrapping_mul(0x0008_0006_0004_0002) >> 48)
                + (odd.wrapping_mul(0x0007_0005_0003_0001) >> 48);
            s2 += 8 * s1 + weighted as u32;
            s1 += sum;
        }
        for &x in tail {
            s1 += x as u32;
            s2 += s1;
        }
        s1 %= M;
        s2 %= M;
    }
    (s2 << 16) | s1
}

/// The Adler-32 of `a`'s input followed by `b`'s (`b_len` bytes), from
/// theirs (zlib's `adler32_combine`).
pub fn adler32_combine(a: u32, b: u32, b_len: usize) -> u32 {
    const M: u64 = 65521;
    let rem = b_len as u64 % M;
    let (a1, a2, b1, b2) = ((a & 0xffff) as u64, (a >> 16) as u64, (b & 0xffff) as u64, (b >> 16) as u64);
    let s1 = (a1 + b1 + M - 1) % M;
    let s2 = (rem * a1 % M + a2 + b2 + M - rem) % M;
    (s2 << 16 | s1) as u32
}

/// Inflate a zlib stream (header, DEFLATE data, Adler-32), at most `limit`
/// bytes of output.
pub fn zlib_decompress(src: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    if src.len() < 6 {
        return Err("zlib: truncated");
    }
    let (cmf, flg) = (src[0], src[1]);
    if cmf & 15 != 8 || cmf >> 4 > 7 || !((cmf as u16) << 8 | flg as u16).is_multiple_of(31) {
        return Err("zlib: bad header");
    }
    if flg & 0x20 != 0 {
        return Err("zlib: preset dictionary");
    }
    let mut out = Vec::new();
    let used = inflate_into(&src[2..], limit, &mut out)?;
    let t = 2 + used;
    let want = src.get(t..t + 4).ok_or("zlib: truncated")?;
    if u32::from_be_bytes(want.try_into().unwrap()) != adler32(1, &out) {
        return Err("zlib: bad checksum");
    }
    Ok(out)
}

// ----------------------------------------------------------------- compress

/// How hard the compressor looks for matches.
#[derive(Clone, Copy)]
pub struct Effort {
    /// Hash chain candidates tried per position.
    pub chain: u32,
    /// A match this long ends the search.
    pub nice: usize,
    /// Matches shorter than this try the next position too (lazy matching).
    pub lazy: usize,
    /// Matches longer than this put only their last positions in the hash
    /// chains (runs, flat areas: fast, and they rarely matter).
    pub insert_max: usize,
}

impl Effort {
    /// Fast, tuned on screenshots (filtered PNG rows): a 4K one takes about
    /// as long as the png crate's default did, and comes out less than
    /// half the size (mostly from the better filters).
    pub const DEFAULT: Effort = Effort { chain: 8, nice: 64, lazy: 8, insert_max: 16 };
    /// For data compressed once at build time.
    pub const BEST: Effort = Effort { chain: 4096, nice: 258, lazy: 258, insert_max: 258 };
}

const WSIZE: usize = 1 << 15;
const WMASK: usize = WSIZE - 1;
const HBITS: u32 = 15;
const MIN_MATCH: usize = 4;
const MAX_MATCH: usize = 258;
/// Tokens per block.
const BLOCK: usize = 1 << 15;

/// LSB-first bit writer.
struct Out {
    v: Vec<u8>,
    buf: u64,
    n: u32,
}

impl Out {
    #[inline(always)]
    fn put(&mut self, bits: u32, k: u32) {
        debug_assert!(k <= 32 && (k == 32 || bits >> k == 0));
        self.buf |= (bits as u64) << self.n;
        self.n += k;
        if self.n >= 32 {
            self.v.extend_from_slice(&(self.buf as u32).to_le_bytes());
            self.buf >>= 32;
            self.n -= 32;
        }
    }

    fn align(&mut self) {
        while self.n > 0 {
            self.v.push(self.buf as u8);
            self.buf >>= 8;
            self.n = self.n.saturating_sub(8);
        }
        self.buf = 0;
    }
}

/// Length (3..=258) to its code index (0..29).
const LEN_CODE: [u8; 259] = {
    let mut t = [0u8; 259];
    let mut c = 0;
    let mut l = 3;
    while l <= 258 {
        if c < 28 && LEN_BASE[c + 1] as usize <= l {
            c += 1;
        }
        t[l] = c as u8;
        l += 1;
    }
    t
};

#[inline(always)]
fn len_code(len: usize) -> usize {
    LEN_CODE[len] as usize
}

/// Distance (1..=32768) to its code (0..30).
#[inline(always)]
fn dist_code(d: usize) -> usize {
    if d <= 4 {
        d - 1
    } else {
        let k = (usize::BITS - 1 - (d - 1).leading_zeros()) as usize; // floor(log2(d - 1))
        2 * k + (((d - 1) >> (k - 1)) & 1)
    }
}

/// Code lengths (at most `limit` bits) for `freq`: a Huffman code, its
/// overlong lengths folded back (JPEG's Annex K.3 adjustment), lengths
/// given out by frequency. Always a complete code of two or more symbols
/// when any symbol is used.
fn code_lengths(freq: &[u32], limit: usize) -> Vec<u8> {
    let mut lens = vec![0u8; freq.len()];
    let mut used: Vec<usize> = (0..freq.len()).filter(|&s| freq[s] > 0).collect();
    if used.is_empty() {
        return lens;
    }
    if used.len() == 1 {
        let other = if used[0] == 0 { 1 } else { 0 };
        lens[used[0]] = 1;
        lens[other] = 1;
        return lens;
    }
    used.sort_by_key(|&s| (freq[s], s));
    let n = used.len();
    // Two-queue Huffman: leaves 0..n, internal nodes n..2n-1.
    let mut weight: Vec<u64> = used.iter().map(|&s| freq[s] as u64).collect();
    weight.resize(2 * n - 1, 0);
    let mut parent = vec![0usize; 2 * n - 1];
    let (mut leaf, mut node) = (0usize, n);
    for next in n..2 * n - 1 {
        let mut pick = || {
            if leaf < n && (node >= next || weight[leaf] <= weight[node]) {
                leaf += 1;
                leaf - 1
            } else {
                node += 1;
                node - 1
            }
        };
        let (a, b) = (pick(), pick());
        weight[next] = weight[a] + weight[b];
        parent[a] = next;
        parent[b] = next;
    }
    let mut depth = vec![0usize; 2 * n - 1];
    for i in (0..2 * n - 2).rev() {
        depth[i] = depth[parent[i]] + 1;
    }
    let maxd = depth[..n].iter().copied().max().unwrap();
    let mut count = vec![0usize; maxd.max(limit) + 1];
    for &d in &depth[..n] {
        count[d] += 1;
    }
    for i in (limit + 1..=maxd).rev() {
        while count[i] > 0 {
            let mut j = i - 2;
            while count[j] == 0 {
                j -= 1;
            }
            count[i] -= 2;
            count[i - 1] += 1;
            count[j + 1] += 2;
            count[j] -= 1;
        }
    }
    // Most frequent first, shortest lengths first.
    let mut k = n;
    for (l, &c) in count.iter().enumerate().take(limit + 1).skip(1) {
        for _ in 0..c {
            k -= 1;
            lens[used[k]] = l as u8;
        }
    }
    lens
}

/// Canonical codes for `lens`, bit-reversed for the LSB-first writer.
fn codes_for(lens: &[u8]) -> Vec<u16> {
    let mut count = [0u16; 16];
    for &l in lens {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut next = [0u16; 16];
    let mut code = 0u16;
    for l in 1..16 {
        code = (code + count[l - 1]) << 1;
        next[l] = code;
    }
    lens.iter()
        .map(|&l| {
            if l == 0 {
                return 0;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            ((c as u32).reverse_bits() >> (32 - l as u32)) as u16
        })
        .collect()
}

/// A literal (< 256) or a match: `1 << 31 | len << 16 | (dist - 1)`.
type Tok = u32;

struct Block {
    toks: Vec<Tok>,
    lit: [u32; 286],
    dist: [u32; 30],
}

/// Run-length coded code lengths: (symbol, extra bits value).
fn rle_lengths(lens: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lens.len() {
        let v = lens[i];
        let mut run = 1;
        while i + run < lens.len() && lens[i + run] == v {
            run += 1;
        }
        let mut r = run;
        if v == 0 {
            while r >= 11 {
                let k = r.min(138);
                out.push((18, (k - 11) as u8));
                r -= k;
            }
            if r >= 3 {
                out.push((17, (r - 3) as u8));
                r = 0;
            }
        } else {
            out.push((v, 0));
            r -= 1;
            while r >= 3 {
                let k = r.min(6);
                out.push((16, (k - 3) as u8));
                r -= k;
            }
        }
        for _ in 0..r {
            out.push((v, 0));
        }
        i += run;
    }
    out
}

fn extra_bits(b: &Block) -> u64 {
    let l: u64 = (0..29).map(|c| b.lit[257 + c] as u64 * LEN_EXTRA[c] as u64).sum();
    let d: u64 = (0..30).map(|c| b.dist[c] as u64 * DIST_EXTRA[c] as u64).sum();
    l + d
}

fn cost(freq: &[u32], lens: &[u8]) -> u64 {
    freq.iter().zip(lens).map(|(&f, &l)| f as u64 * l as u64).sum()
}

fn write_block(o: &mut Out, b: &Block, raw: &[u8], last: bool) {
    let mut lit = b.lit;
    lit[256] = 1;
    let fixed_l = fixed_lengths();
    let fixed_cost = 3 + cost(&lit, &fixed_l[..286]) + 5 * b.dist.iter().map(|&f| f as u64).sum::<u64>() + extra_bits(b);

    let llens = code_lengths(&lit, 15);
    let mut dlens = code_lengths(&b.dist, 15);
    if b.dist.iter().all(|&f| f == 0) {
        dlens[..2].fill(1); // a complete (unused) distance code
    }
    let nlit = 257 + llens[257..].iter().rposition(|&l| l != 0).map_or(0, |p| p + 1);
    let ndist = 1 + dlens.iter().rposition(|&l| l != 0).unwrap_or(0);
    let mut all = llens[..nlit].to_vec();
    all.extend_from_slice(&dlens[..ndist]);
    let rle = rle_lengths(&all);
    let mut clf = [0u32; 19];
    for &(s, _) in &rle {
        clf[s as usize] += 1;
    }
    let cll = code_lengths(&clf, 7);
    let ncl = 4.max(1 + CL_ORDER.iter().rposition(|&i| cll[i] != 0).unwrap_or(0));
    let rle_extra: u64 = rle.iter().map(|&(s, _)| [2u64, 3, 7].get((s as usize).wrapping_sub(16)).copied().unwrap_or(0)).sum();
    let dyn_cost = 3
        + 14
        + 3 * ncl as u64
        + cost(&clf, &cll)
        + rle_extra
        + cost(&lit, &llens)
        + cost(&b.dist, &dlens)
        + extra_bits(b);
    let stored_cost = {
        let chunks = raw.len().div_ceil(65535).max(1) as u64;
        8 * raw.len() as u64 + chunks * (3 + 7 + 32)
    };

    if stored_cost < fixed_cost.min(dyn_cost) {
        let mut chunks = raw.chunks(65535).peekable();
        if raw.is_empty() {
            o.put(last as u32, 3);
            o.align();
            o.v.extend_from_slice(&[0, 0, 0xff, 0xff]);
        }
        while let Some(c) = chunks.next() {
            let fin = last && chunks.peek().is_none();
            o.put(fin as u32, 3);
            o.align();
            o.v.extend_from_slice(&(c.len() as u16).to_le_bytes());
            o.v.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
            o.v.extend_from_slice(c);
        }
        return;
    }
    let (ll, dl) = if fixed_cost <= dyn_cost {
        o.put(last as u32 | 1 << 1, 3);
        (fixed_l.to_vec(), vec![5u8; 30]) // all 288: the 9-bit codes start after every 8-bit one
    } else {
        o.put(last as u32 | 2 << 1, 3);
        o.put((nlit - 257) as u32, 5);
        o.put((ndist - 1) as u32, 5);
        o.put((ncl - 4) as u32, 4);
        for &i in &CL_ORDER[..ncl] {
            o.put(cll[i] as u32, 3);
        }
        let clc = codes_for(&cll);
        for &(s, x) in &rle {
            o.put(clc[s as usize] as u32, cll[s as usize] as u32);
            match s {
                16 => o.put(x as u32, 2),
                17 => o.put(x as u32, 3),
                18 => o.put(x as u32, 7),
                _ => {}
            }
        }
        (llens, dlens)
    };
    let lc = codes_for(&ll);
    let dc = codes_for(&dl);
    for &t in &b.toks {
        if t >> 31 == 0 {
            o.put(lc[t as usize] as u32, ll[t as usize] as u32);
        } else {
            let len = ((t >> 16) & 0x1ff) as usize;
            let d = (t & 0xffff) as usize + 1;
            let c = len_code(len);
            o.put(lc[257 + c] as u32, ll[257 + c] as u32);
            o.put((len - LEN_BASE[c] as usize) as u32, LEN_EXTRA[c] as u32);
            let c = dist_code(d);
            o.put(dc[c] as u32, dl[c] as u32);
            o.put((d - DIST_BASE[c] as usize) as u32, DIST_EXTRA[c] as u32);
        }
    }
    o.put(lc[256] as u32, ll[256] as u32);
}

#[inline(always)]
fn hash4(d: &[u8], p: usize) -> usize {
    let v = u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]);
    (v.wrapping_mul(2654435761) >> (32 - HBITS)) as usize
}

/// Common prefix length of `d[a..]` and `d[b..]`, at most `max`.
#[inline(always)]
fn match_len(d: &[u8], a: usize, b: usize, max: usize) -> usize {
    let mut n = 0;
    while n + 8 <= max {
        let x = u64::from_le_bytes(d[a + n..a + n + 8].try_into().unwrap());
        let y = u64::from_le_bytes(d[b + n..b + n + 8].try_into().unwrap());
        let z = x ^ y;
        if z != 0 {
            return n + (z.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    while n < max && d[a + n] == d[b + n] {
        n += 1;
    }
    n
}

struct Matcher {
    head: Vec<u32>,
    prev: Vec<u32>,
}

impl Matcher {
    /// Insert position `p` (`p + 4 <= len`); positions are stored plus one.
    #[inline(always)]
    fn insert(&mut self, d: &[u8], p: usize) {
        let h = hash4(d, p);
        self.prev[p & WMASK] = self.head[h];
        self.head[h] = p as u32 + 1;
    }

    /// The longest match for `p` (already inserted), longer than `have`.
    #[inline(always)]
    fn find(&self, d: &[u8], p: usize, mut chain: u32, nice: usize, have: usize) -> (usize, usize) {
        let max = MAX_MATCH.min(d.len() - p);
        let (mut best, mut dist) = (have, 0);
        if max < MIN_MATCH {
            return (0, 0);
        }
        let mut c = self.prev[p & WMASK] as usize;
        while c != 0 && chain > 0 {
            let q = c - 1;
            if q >= p || p - q > WSIZE {
                break;
            }
            if d[q + best.min(max - 1)] == d[p + best.min(max - 1)] {
                let n = match_len(d, q, p, max);
                if n > best {
                    best = n;
                    dist = p - q;
                    if n >= nice || n == max {
                        break;
                    }
                }
            }
            let nc = self.prev[q & WMASK] as usize;
            if nc >= c {
                break;
            }
            c = nc;
            chain -= 1;
        }
        if dist == 0 { (0, 0) } else { (best, dist) }
    }
}

/// Input kept for matching and the lookahead: once the buffer holds this
/// much past the window, it is compressed up to the lookahead and slid.
const SEGMENT: usize = 1 << 20;
const LOOKAHEAD: usize = MAX_MATCH + MIN_MATCH;

/// A streaming compressor: [`Deflater::write`] any amount, then
/// [`Deflater::finish`]. The input is compressed a segment at a time
/// (a 32 KiB window plus up to [`SEGMENT`] bytes buffered).
pub struct Deflater {
    e: Effort,
    zlib: bool,
    adler: u32,
    o: Out,
    m: Matcher,
    blk: Block,
    /// Buffered input; positions below are indices into it.
    buf: Vec<u8>,
    /// The next position to code, the current block's first one, and the
    /// first one not yet in the hash chains.
    p: usize,
    start: usize,
    ins: usize,
}

impl Deflater {
    /// A raw DEFLATE stream, or a zlib one (header and Adler-32 around it).
    pub fn new(e: Effort, zlib: bool) -> Deflater {
        Deflater {
            e,
            zlib,
            adler: 1,
            o: Out { v: if zlib { vec![0x78, 0x9c] } else { Vec::new() }, buf: 0, n: 0 },
            m: Matcher { head: vec![0; 1 << HBITS], prev: vec![0; WSIZE] },
            blk: Block { toks: Vec::with_capacity(BLOCK), lit: [0; 286], dist: [0; 30] },
            buf: Vec::with_capacity(WSIZE + SEGMENT + LOOKAHEAD),
            p: 0,
            start: 0,
            ins: 0,
        }
    }

    pub fn write(&mut self, mut data: &[u8]) {
        self.adler = adler32(self.adler, data);
        while !data.is_empty() {
            let room = (WSIZE + SEGMENT + LOOKAHEAD).saturating_sub(self.buf.len()).max(1);
            let (now, rest) = data.split_at(room.min(data.len()));
            self.buf.extend_from_slice(now);
            data = rest;
            if self.buf.len() >= WSIZE + SEGMENT + LOOKAHEAD {
                self.code(self.buf.len() - LOOKAHEAD);
                self.flush(false);
                self.slide();
            }
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        let n = self.buf.len();
        self.code(n);
        self.flush(true);
        self.o.align();
        if self.zlib {
            self.o.v.extend_from_slice(&self.adler.to_be_bytes());
        }
        self.o.v
    }

    /// End a raw part of a stream that other parts continue (`last`:
    /// none do): byte-aligned with an empty stored block unless last, so
    /// parts compressed apart can be concatenated. Returns the part and
    /// the Adler-32 of its input (see [`adler32_combine`]).
    pub fn finish_part(mut self, last: bool) -> (Vec<u8>, u32) {
        let n = self.buf.len();
        self.code(n);
        self.flush(last);
        if !last {
            self.o.put(0, 3);
            self.o.align();
            self.o.v.extend_from_slice(&[0, 0, 0xff, 0xff]);
        }
        self.o.align();
        (self.o.v, self.adler)
    }

    /// Write the tokens so far as a block.
    fn flush(&mut self, last: bool) {
        if self.blk.toks.is_empty() && !last {
            return;
        }
        write_block(&mut self.o, &self.blk, &self.buf[self.start..self.p], last);
        self.start = self.p;
        self.blk.toks.clear();
        self.blk.lit = [0; 286];
        self.blk.dist = [0; 30];
    }

    /// Drop all but the window before the next position (a whole number of
    /// windows, so the chains' `& WMASK` slots stay put).
    fn slide(&mut self) {
        let delta = (self.p - WSIZE) / WSIZE * WSIZE;
        if delta == 0 {
            return;
        }
        self.buf.drain(..delta);
        let d = delta as u32;
        for v in self.m.head.iter_mut().chain(self.m.prev.iter_mut()) {
            *v = v.saturating_sub(d);
        }
        self.p -= delta;
        self.start -= delta;
        self.ins = self.ins.saturating_sub(delta);
    }

    /// Code the positions up to `limit` (the input may run past it: the
    /// lookahead for matches).
    fn code(&mut self, limit: usize) {
        let e = self.e;
        let data = &self.buf[..];
        let n = data.len();
        let (m, blk) = (&mut self.m, &mut self.blk);
        let (mut p, mut ins) = (self.p, self.ins);
        let can_hash = |q: usize| q + MIN_MATCH <= n;
        while p < limit {
            let mut tok_len = 1;
            if can_hash(p) {
                while ins <= p {
                    m.insert(data, ins);
                    ins += 1;
                }
                let (mut len, mut dist) = m.find(data, p, e.chain, e.nice, MIN_MATCH - 1);
                if len >= MIN_MATCH {
                    while len < e.lazy && can_hash(p + 1) {
                        if ins <= p + 1 {
                            m.insert(data, p + 1);
                            ins = p + 2;
                        }
                        let chain = if len >= 16 { e.chain / 4 } else { e.chain };
                        let (l2, d2) = m.find(data, p + 1, chain.max(1), e.nice, len);
                        if l2 > len {
                            blk.toks.push(data[p] as Tok);
                            blk.lit[data[p] as usize] += 1;
                            p += 1;
                            len = l2;
                            dist = d2;
                        } else {
                            break;
                        }
                    }
                    blk.toks.push(1 << 31 | (len as u32) << 16 | (dist - 1) as u32);
                    blk.lit[257 + len_code(len)] += 1;
                    blk.dist[dist_code(dist)] += 1;
                    tok_len = len;
                }
            }
            if tok_len == 1 {
                blk.toks.push(data[p] as Tok);
                blk.lit[data[p] as usize] += 1;
            } else {
                // Positions inside the match go into the chains.
                let end = (p + tok_len).min(n.saturating_sub(MIN_MATCH - 1));
                if tok_len > e.insert_max {
                    ins = ins.max(end.saturating_sub(MIN_MATCH));
                }
                while ins < end {
                    m.insert(data, ins);
                    ins += 1;
                }
            }
            p += tok_len;
            if blk.toks.len() >= BLOCK {
                write_block(&mut self.o, blk, &data[self.start..p], false);
                self.start = p;
                blk.toks.clear();
                blk.lit = [0; 286];
                blk.dist = [0; 30];
            }
        }
        (self.p, self.ins) = (p, ins);
    }
}

/// Raw DEFLATE of `data`.
pub fn deflate(data: &[u8], e: Effort) -> Vec<u8> {
    let mut d = Deflater::new(e, false);
    d.write(data);
    d.finish()
}

/// A zlib stream of `data`.
pub fn zlib_compress(data: &[u8], e: Effort) -> Vec<u8> {
    let mut d = Deflater::new(e, true);
    d.write(data);
    d.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    fn samples() -> Vec<Vec<u8>> {
        let mut s = 0x9e3779b97f4a7c15u64;
        let mut v: Vec<Vec<u8>> = vec![vec![], vec![7], vec![0; 100_000], b"abcabcabcabcabcabcabd".to_vec()];
        v.push((0..70_000).map(|_| rng(&mut s) as u8).collect()); // incompressible: stored
        v.push((0..200_000).map(|_| (rng(&mut s) % 4) as u8).collect());
        v.push((0..300_000u32).map(|i| ((i / 7) % 13) as u8 ^ (i % 3) as u8).collect());
        let text = "the quick brown fox jumps over the lazy dog; ".repeat(2000);
        v.push(text.into_bytes());
        // Skewed frequencies: Huffman lengths past 15 bits before the fold.
        let mut skew = Vec::new();
        for k in 0..25u32 {
            for _ in 0..(1u32 << k.min(17)) {
                skew.push(k as u8);
            }
        }
        for i in (1..skew.len()).rev() {
            let j = (rng(&mut s) as usize) % (i + 1);
            skew.swap(i, j);
        }
        v.push(skew);
        v
    }

    #[test]
    fn round_trips() {
        for d in samples() {
            for e in [Effort::DEFAULT, Effort { chain: 1, nice: 8, lazy: 0, insert_max: 8 }, Effort::BEST] {
                let z = zlib_compress(&d, e);
                assert_eq!(zlib_decompress(&z, d.len()).unwrap(), d, "len {}", d.len());
                let (raw, used) = inflate(&z[2..], d.len()).unwrap();
                assert_eq!(raw, d);
                assert_eq!(used, z.len() - 6);
            }
        }
    }

    #[test]
    fn compresses() {
        let d = vec![0u8; 100_000];
        assert!(zlib_compress(&d, Effort::DEFAULT).len() < 200);
        let t = "the quick brown fox jumps over the lazy dog; ".repeat(2000);
        assert!(zlib_compress(t.as_bytes(), Effort::DEFAULT).len() < 600);
    }

    #[test]
    fn limit_and_corruption_fail_cleanly() {
        let d = vec![1u8; 10_000];
        let z = zlib_compress(&d, Effort::DEFAULT);
        assert!(zlib_decompress(&z, 9_999).is_err(), "over the limit");
        let mut bad = z.clone();
        let k = bad.len() - 1;
        bad[k] ^= 1;
        assert!(zlib_decompress(&bad, 10_000).is_err(), "checksum");
        assert!(zlib_decompress(&z[..z.len() - 5], 10_000).is_err(), "truncated");
        // Random corruption and truncation: errors or wrong data, never a panic.
        let mut s = 12345u64;
        for d in samples() {
            let z = zlib_compress(&d, Effort::DEFAULT);
            for _ in 0..60 {
                let mut c = z.clone();
                let flips = 1 + rng(&mut s) % 4;
                for _ in 0..flips {
                    let i = (rng(&mut s) as usize) % c.len();
                    c[i] ^= 1 << (rng(&mut s) % 8);
                }
                let cut = (rng(&mut s) as usize) % (c.len() + 1);
                let _ = zlib_decompress(&c, d.len() + 100);
                let _ = zlib_decompress(&c[..cut], d.len() + 100);
                let _ = inflate(&c[..cut], 1 << 20);
            }
        }
        for _ in 0..2000 {
            let n = (rng(&mut s) % 300) as usize;
            let junk: Vec<u8> = (0..n).map(|_| rng(&mut s) as u8).collect();
            let _ = inflate(&junk, 1 << 16);
        }
    }

    #[test]
    fn parts_concatenate_into_one_stream() {
        let mut s = 21u64;
        let d: Vec<u8> = (0..300_000).map(|_| (rng(&mut s) % 6) as u8).collect();
        let cuts = [0, 1, 70_000, 70_001, 250_000, 300_000];
        let mut z = vec![0x78, 0x9c];
        let mut a = 1;
        for w in cuts.windows(2) {
            let mut p = Deflater::new(Effort::DEFAULT, false);
            p.write(&d[w[0]..w[1]]);
            let (part, pa) = p.finish_part(w[1] == d.len());
            assert_eq!(pa, adler32(1, &d[w[0]..w[1]]));
            z.extend_from_slice(&part);
            a = adler32_combine(a, pa, w[1] - w[0]);
        }
        z.extend_from_slice(&a.to_be_bytes());
        assert_eq!(a, adler32(1, &d));
        assert_eq!(zlib_decompress(&z, d.len()).unwrap(), d);
        // Written in pieces: the same stream as at once.
        let mut p = Deflater::new(Effort::DEFAULT, true);
        for c in d.chunks(777) {
            p.write(c);
        }
        assert_eq!(p.finish(), zlib_compress(&d, Effort::DEFAULT));
    }

    #[test]
    fn adler32_matches_the_byte_loop() {
        assert_eq!(adler32(1, b"Wikipedia"), 0x11e6_0398);
        let mut s = 5u64;
        let d: Vec<u8> = (0..40_000).map(|_| rng(&mut s) as u8).collect();
        let mut want = 1u32;
        for n in [0, 1, 7, 8, 9, 5551, 5552, 5553, 20_000, 40_000] {
            let (mut a, mut b) = (1u32, 0u32);
            for &x in &d[..n] {
                a = (a + x as u32) % 65521;
                b = (b + a) % 65521;
            }
            want = b << 16 | a;
            assert_eq!(adler32(1, &d[..n]), want, "{n}");
        }
        assert_eq!(adler32(adler32(1, &d[..12_345]), &d[12_345..]), want, "continued");
        assert_eq!(adler32(1, &[255; 100_000]), {
            let (mut a, mut b) = (1u32, 0u32);
            for _ in 0..100_000 {
                a = (a + 255) % 65521;
                b = (b + a) % 65521;
            }
            b << 16 | a
        });
    }

    #[test]
    fn decodes_known_streams() {
        // zlib.compress(b"hello hello hello", 9) and a stored block.
        let z = [0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x90, 0x00, 0x3a, 0x2e, 0x06, 0x7d];
        assert_eq!(zlib_decompress(&z, 100).unwrap(), b"hello hello hello");
        let stored = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(inflate(&stored, 10).unwrap(), (b"abc".to_vec(), 8));
    }

    #[test]
    fn length_limited_codes_are_complete() {
        let mut s = 99u64;
        for _ in 0..200 {
            let n = 2 + (rng(&mut s) % 300) as usize;
            let f: Vec<u32> = (0..n).map(|i| if rng(&mut s).is_multiple_of(3) { 0 } else { 1 + (rng(&mut s) % (1 << (i % 24))) as u32 }).collect();
            for limit in [7, 15] {
                // The code length alphabet (7 bits) has 19 symbols.
                let f = &f[..if limit == 7 { n.min(19) } else { n }];
                let n = f.len();
                let l = code_lengths(f, limit);
                let k: u64 = l.iter().filter(|&&x| x > 0).map(|&x| 1u64 << (limit - x as usize)).sum();
                if f.iter().any(|&x| x > 0) {
                    assert_eq!(k, 1 << limit, "complete");
                }
                assert!(l.iter().all(|&x| x as usize <= limit));
                for i in 0..n {
                    assert_eq!(l[i] > 0, f[i] > 0 || f.iter().filter(|&&x| x > 0).count() == 1 && l[i] == 1);
                }
            }
        }
        for d in 1..=32768 {
            let c = dist_code(d);
            assert!(DIST_BASE[c] as usize <= d && d < DIST_BASE[c] as usize + (1 << DIST_EXTRA[c]), "{d}");
        }
    }

    /// Small inputs take fixed blocks: literals of all three fixed code
    /// lengths (8 bits below 144, 9 bits from 144), lengths and distances.
    #[test]
    fn fixed_blocks_round_trip() {
        for d in [
            (0..1001u32).map(|i| (i * 7) as u8).collect::<Vec<u8>>(),
            [b"0123456789".as_slice(), &(140..=160u8).collect::<Vec<u8>>()].concat().repeat(3),
            [200u8, 201, 202, 200, 201, 202, 200, 201, 202, 1, 2, 3, 1, 2, 3, 1, 2, 3].repeat(3),
        ] {
            for e in [Effort::DEFAULT, Effort::BEST] {
                let z = deflate(&d, e);
                assert_eq!(z[0] >> 1 & 3, 1, "a fixed block");
                assert_eq!(inflate(&z, d.len()).unwrap().0, d);
            }
        }
    }
}
