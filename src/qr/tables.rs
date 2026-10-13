//! QR code (ISO/IEC 18004) symbol tables and the function patterns that
//! the encoder draws and the decoder skips.

/// Error-correction level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ecl {
    L,
    M,
    Q,
    H,
}

impl Ecl {
    pub const ALL: [Ecl; 4] = [Ecl::L, Ecl::M, Ecl::Q, Ecl::H];

    fn ord(self) -> usize {
        self as usize
    }

    /// The two format-information bits.
    pub fn bits(self) -> u16 {
        match self {
            Ecl::L => 1,
            Ecl::M => 0,
            Ecl::Q => 3,
            Ecl::H => 2,
        }
    }
}

#[rustfmt::skip]
const ECC_PER_BLOCK: [[u8; 41]; 4] = [
    [0, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18, 20, 24, 26, 30, 22, 24, 28, 30, 28, 28, 28, 28, 30, 30, 26, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30],
    [0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28],
    [0, 13, 22, 18, 26, 18, 24, 18, 22, 20, 24, 28, 26, 24, 20, 30, 24, 28, 28, 26, 30, 28, 30, 30, 30, 30, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30],
    [0, 17, 28, 22, 16, 22, 28, 26, 26, 24, 28, 24, 28, 22, 24, 24, 30, 28, 28, 26, 28, 30, 24, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30],
];

#[rustfmt::skip]
const NUM_BLOCKS: [[u8; 41]; 4] = [
    [0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4, 4, 4, 4, 4, 6, 6, 6, 6, 7, 8, 8, 9, 9, 10, 12, 12, 12, 13, 14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 24, 25],
    [0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49],
    [0, 1, 1, 2, 2, 4, 4, 6, 6, 8, 8, 8, 10, 12, 16, 12, 17, 16, 18, 21, 20, 23, 23, 25, 27, 29, 34, 34, 35, 38, 40, 43, 45, 48, 51, 53, 56, 59, 62, 65, 68],
    [0, 1, 1, 2, 4, 4, 4, 5, 6, 8, 8, 11, 11, 16, 16, 18, 16, 19, 21, 25, 25, 25, 34, 30, 32, 35, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 70, 74, 77, 81],
];

pub fn size_of(ver: usize) -> usize {
    ver * 4 + 17
}

pub fn ecc_per_block(ver: usize, ecl: Ecl) -> usize {
    ECC_PER_BLOCK[ecl.ord()][ver] as usize
}

pub fn num_blocks(ver: usize, ecl: Ecl) -> usize {
    NUM_BLOCKS[ecl.ord()][ver] as usize
}

/// Modules left for data and check bytes (including remainder bits).
pub fn raw_data_modules(ver: usize) -> usize {
    let mut r = (16 * ver + 128) * ver + 64;
    if ver >= 2 {
        let na = ver / 7 + 2;
        r -= (25 * na - 10) * na - 55;
        if ver >= 7 {
            r -= 36;
        }
    }
    r
}

pub fn num_data_codewords(ver: usize, ecl: Ecl) -> usize {
    raw_data_modules(ver) / 8 - ecc_per_block(ver, ecl) * num_blocks(ver, ecl)
}

/// Centers of the alignment patterns along one axis.
pub fn alignment_positions(ver: usize) -> Vec<usize> {
    if ver == 1 {
        return Vec::new();
    }
    let na = ver / 7 + 2;
    let step = (ver * 8 + na * 3 + 5) / (na * 4 - 4) * 2;
    let mut out = vec![6];
    let mut pos = size_of(ver) - 7;
    while out.len() < na {
        out.insert(1, pos);
        pos = pos.saturating_sub(step);
    }
    out
}

/// The 15 format bits (BCH-coded, masked) for a level and mask.
pub fn format_bits(ecl: Ecl, mask: u8) -> u16 {
    let data = (ecl.bits() << 3) | mask as u16;
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    ((data << 10) | (rem & 0x3FF)) ^ 0x5412
}

/// The 18 version bits (versions 7 and up).
pub fn version_bits(ver: usize) -> u32 {
    let mut rem = ver as u32;
    for _ in 0..12 {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
    }
    ((ver as u32) << 12) | (rem & 0xFFF)
}

/// Format bit `i`'s two module positions `(x, y)`.
pub fn format_positions(size: usize, i: usize) -> [(usize, usize); 2] {
    let a = match i {
        0..=5 => (8, i),
        6 => (8, 7),
        7 => (8, 8),
        8 => (7, 8),
        _ => (14 - i, 8),
    };
    let b = if i < 8 { (size - 1 - i, 8) } else { (8, size - 15 + i) };
    [a, b]
}

/// Version bit `i`'s two module positions `(x, y)`.
pub fn version_positions(size: usize, i: usize) -> [(usize, usize); 2] {
    let a = size - 11 + i % 3;
    let b = i / 3;
    [(a, b), (b, a)]
}

/// Whether mask `m` flips module `(x, y)`.
pub fn mask_bit(m: u8, x: usize, y: usize) -> bool {
    match m {
        0 => (x + y) % 2 == 0,
        1 => y % 2 == 0,
        2 => x % 3 == 0,
        3 => (x + y) % 3 == 0,
        4 => (x / 3 + y / 2) % 2 == 0,
        5 => x * y % 2 + x * y % 3 == 0,
        6 => (x * y % 2 + x * y % 3) % 2 == 0,
        _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
    }
}

/// A square module matrix (`true` = dark), row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matrix {
    pub size: usize,
    pub cells: Vec<bool>,
}

impl Matrix {
    pub fn new(size: usize) -> Self {
        Matrix { size, cells: vec![false; size * size] }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        self.cells[y * self.size + x]
    }

    pub fn set(&mut self, x: usize, y: usize, v: bool) {
        self.cells[y * self.size + x] = v;
    }

    /// The matrix mirrored over its main diagonal.
    pub fn transposed(&self) -> Matrix {
        let mut t = Matrix::new(self.size);
        for y in 0..self.size {
            for x in 0..self.size {
                t.set(y, x, self.get(x, y));
            }
        }
        t
    }
}

/// A version's function patterns: the modules as drawn (format and version
/// areas blank) and which modules are function modules.
pub fn function_patterns(ver: usize) -> (Matrix, Matrix) {
    let size = size_of(ver);
    let mut m = Matrix::new(size);
    let mut f = Matrix::new(size);
    let mut set = |x: usize, y: usize, v: bool| {
        m.set(x, y, v);
        f.set(x, y, true);
    };
    for i in 0..size {
        set(6, i, i % 2 == 0);
        set(i, 6, i % 2 == 0);
    }
    for (cx, cy) in [(3, 3), (size - 4, 3), (3, size - 4)] {
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if x >= 0 && y >= 0 && (x as usize) < size && (y as usize) < size {
                    let d = dx.abs().max(dy.abs());
                    set(x as usize, y as usize, d != 2 && d != 4);
                }
            }
        }
    }
    let al = alignment_positions(ver);
    let n = al.len();
    for i in 0..n {
        for j in 0..n {
            if (i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0) {
                continue;
            }
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let d = dx.abs().max(dy.abs());
                    set((al[i] as i32 + dx) as usize, (al[j] as i32 + dy) as usize, d != 1);
                }
            }
        }
    }
    for i in 0..15 {
        for (x, y) in format_positions(size, i) {
            set(x, y, false);
        }
    }
    set(8, size - 8, true);
    if ver >= 7 {
        for i in 0..18 {
            for (x, y) in version_positions(size, i) {
                set(x, y, false);
            }
        }
    }
    (m, f)
}

/// The data modules' order: two-column zigzag from the bottom-right,
/// skipping the vertical timing column.
pub fn data_positions(func: &Matrix) -> Vec<(usize, usize)> {
    let size = func.size;
    let mut out = Vec::with_capacity(size * size);
    let mut right = size as i32 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2 {
                let x = (right - j) as usize;
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if !func.get(x, y) {
                    out.push((x, y));
                }
            }
        }
        right -= 2;
    }
    out
}

/// [`data_positions`] of a version, computed once.
pub fn cached_data_positions(ver: usize) -> &'static [(usize, usize)] {
    static CACHE: [std::sync::OnceLock<Vec<(usize, usize)>>; 40] = [const { std::sync::OnceLock::new() }; 40];
    CACHE[ver.clamp(1, 40) - 1].get_or_init(|| data_positions(&function_patterns(ver.clamp(1, 40)).1))
}

/// Block layout: `(data lengths per block, check bytes per block)`; short
/// blocks come first.
pub fn block_layout(ver: usize, ecl: Ecl) -> (Vec<usize>, usize) {
    let nb = num_blocks(ver, ecl);
    let ecc = ecc_per_block(ver, ecl);
    let raw = raw_data_modules(ver) / 8;
    let short = nb - raw % nb;
    let short_len = raw / nb;
    let lens = (0..nb).map(|i| short_len - ecc + usize::from(i >= short)).collect();
    (lens, ecc)
}

/// The codeword order of interleaved blocks: `(block, index in block)` for
/// each transmitted codeword.
pub fn interleave_order(data_lens: &[usize], ecc: usize) -> Vec<(usize, usize)> {
    let max = data_lens.iter().copied().max().unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..max {
        for (j, &len) in data_lens.iter().enumerate() {
            if i < len {
                out.push((j, i));
            }
        }
    }
    for i in 0..ecc {
        for (j, &len) in data_lens.iter().enumerate() {
            out.push((j, len + i));
        }
    }
    out
}
