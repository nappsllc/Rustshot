//! A test-only MP4 reader: the box tree of a file, so the muxer's output
//! can be checked field by field.

/// One box: its type, its body (after the header) and, for containers,
/// the boxes in it.
#[derive(Clone, Debug)]
pub struct Bx {
    pub kind: [u8; 4],
    /// Where the box starts in the file, and its whole size.
    pub at: usize,
    pub size: usize,
    pub body: Vec<u8>,
    pub kids: Vec<Bx>,
}

/// Bytes into the body where a container's children start.
fn kids_at(kind: &[u8; 4]) -> Option<usize> {
    match kind {
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"dinf" | b"edts" => Some(0),
        b"stsd" | b"dref" => Some(8),
        // Sample entries: avc1 (visual, 78 bytes), Opus (audio, 28 bytes).
        b"avc1" => Some(78),
        b"Opus" => Some(28),
        _ => None,
    }
}

/// The boxes in `data[from..to]` (offsets are from the file start).
/// Panics on a box that does not fit.
pub fn boxes(data: &[u8], from: usize, to: usize) -> Vec<Bx> {
    let mut out = Vec::new();
    let mut at = from;
    while at < to {
        assert!(at + 8 <= to, "box header past the end at {at}");
        let s32 = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        let (size, head) = match s32 {
            1 => (u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap()) as usize, 16),
            0 => (to - at, 8),
            n => (n, 8),
        };
        assert!(size >= head && at + size <= to, "box {} at {at} size {size} runs past {to}", String::from_utf8_lossy(&kind));
        let body = data[at + head..at + size].to_vec();
        let kids = kids_at(&kind).map_or_else(Vec::new, |k| boxes(data, at + head + k, at + size));
        out.push(Bx { kind, at, size, body, kids });
        at += size;
    }
    out
}

impl Bx {
    /// The child at `path` (types, one level each).
    pub fn get(&self, path: &[&[u8; 4]]) -> &Bx {
        let mut b = self;
        for k in path {
            b = b.kids.iter().find(|c| &&c.kind == k).unwrap_or_else(|| panic!("no {} in {}", String::from_utf8_lossy(*k), String::from_utf8_lossy(&b.kind)));
        }
        b
    }

    pub fn has(&self, kind: &[u8; 4]) -> bool {
        self.kids.iter().any(|c| &c.kind == kind)
    }

    pub fn u8(&self, off: usize) -> u8 {
        self.body[off]
    }

    pub fn u16(&self, off: usize) -> u16 {
        u16::from_be_bytes(self.body[off..off + 2].try_into().unwrap())
    }

    pub fn u32(&self, off: usize) -> u32 {
        u32::from_be_bytes(self.body[off..off + 4].try_into().unwrap())
    }

    pub fn u64(&self, off: usize) -> u64 {
        u64::from_be_bytes(self.body[off..off + 8].try_into().unwrap())
    }

    /// A full box's table of `count` entries of `width` 32-bit words after
    /// the version, flags and count (stts, stss, stsz's sizes, stco...).
    pub fn table(&self, skip: usize, width: usize) -> Vec<Vec<u64>> {
        let n = self.u32(skip) as usize;
        let wide = &self.kind == b"co64";
        (0..n)
            .map(|i| {
                (0..width)
                    .map(|j| {
                        if wide {
                            self.u64(skip + 4 + (i * width + j) * 8)
                        } else {
                            u64::from(self.u32(skip + 4 + (i * width + j) * 4))
                        }
                    })
                    .collect()
            })
            .collect()
    }
}
