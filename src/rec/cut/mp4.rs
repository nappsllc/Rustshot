//! Trim an MP4 without re-encoding: the samples from the key frame at
//! or before the new start up to the end of the GOP that holds the new
//! end are copied into a new file, and an edit list makes it start and
//! end exactly where asked (players show the presentation the edit list
//! describes; the frames before the start are decoded but not shown).
//! Sound (every sample a sync sample) is cut a little before the start
//! ([`AUDIO_PREROLL`], so the decoder has settled by then) and its own
//! edit list skips that.
//!
//! The file must be a plain (not fragmented) ISO BMFF: `ftyp`, `moov`,
//! `mdat` in any order, as Media Foundation, our muxer, ffmpeg and
//! AVAssetWriter write them. The new file is `ftyp`, `moov` (with `co64`
//! offsets, so its size does not depend on them), then one `mdat` with the
//! samples in their original order. Boxes the trim does not touch (the
//! sample descriptions, handlers, user data) are copied as they are;
//! per-sample tables the trim cannot cut (sample groups, `sdtp`) are
//! dropped.

use anyhow::{anyhow, bail, ensure, Context, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::Duration;

/// Sound kept before the start for the decoder to settle (80 ms, as Opus
/// asks; plenty for AAC's one frame).
pub const AUDIO_PREROLL: Duration = Duration::from_millis(80);

// --- reading ------------------------------------------------------------------

fn be16(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]])).ok_or_else(|| anyhow!("box too short"))
}

fn be32(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| anyhow!("box too short"))
}

fn be64(b: &[u8], at: usize) -> Result<u64> {
    b.get(at..at + 8).map(|s| u64::from_be_bytes(s.try_into().unwrap_or_default())).ok_or_else(|| anyhow!("box too short"))
}

/// A box in memory: its type and its body (after the header).
#[derive(Clone, Debug)]
struct Bx<'a> {
    kind: [u8; 4],
    body: &'a [u8],
    /// The whole box (header included), to copy as is.
    raw: &'a [u8],
}

/// The boxes in `data`.
fn children(data: &[u8]) -> Result<Vec<Bx<'_>>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= data.len() {
        let s32 = be32(data, at)? as usize;
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap_or_default();
        let (size, head) = match s32 {
            1 => (be64(data, at + 8)? as usize, 16),
            0 => (data.len() - at, 8),
            n => (n, 8),
        };
        ensure!(size >= head && at + size <= data.len(), "a broken {} box", String::from_utf8_lossy(&kind));
        out.push(Bx { kind, body: &data[at + head..at + size], raw: &data[at..at + size] });
        at += size;
    }
    Ok(out)
}

fn find<'a>(boxes: &[Bx<'a>], kind: &[u8; 4]) -> Option<Bx<'a>> {
    boxes.iter().find(|b| &b.kind == kind).cloned()
}

fn need<'a>(boxes: &[Bx<'a>], kind: &[u8; 4]) -> Result<Bx<'a>> {
    find(boxes, kind).ok_or_else(|| anyhow!("no {} box", String::from_utf8_lossy(kind)))
}

/// A top-level box of the file: where it is.
#[derive(Clone, Copy, Debug)]
struct Top {
    kind: [u8; 4],
    at: u64,
    size: u64,
    head: u64,
}

fn top_boxes<F: Read + Seek>(f: &mut F) -> Result<Vec<Top>> {
    let len = f.seek(SeekFrom::End(0))?;
    let mut out = Vec::new();
    let mut at = 0u64;
    while at + 8 <= len {
        f.seek(SeekFrom::Start(at))?;
        let mut h = [0u8; 16];
        f.read_exact(&mut h[..8])?;
        let s32 = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        let kind: [u8; 4] = [h[4], h[5], h[6], h[7]];
        let (size, head) = match s32 {
            1 => {
                f.read_exact(&mut h[8..16])?;
                (u64::from_be_bytes(h[8..16].try_into().unwrap_or_default()), 16)
            }
            0 => (len - at, 8),
            n => (n, 8),
        };
        ensure!(size >= head && at + size <= len, "the file is cut short in a {} box", String::from_utf8_lossy(&kind));
        out.push(Top { kind, at, size, head });
        at += size;
    }
    Ok(out)
}

/// An edit list's first media segment: leading empty time (movie
/// timescale) and the media time it starts at (track timescale).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub empty: u64,
    pub media_time: i64,
}

fn read_edit(elst: &[u8]) -> Result<Edit> {
    let v = elst.first().copied().unwrap_or(0);
    let n = be32(elst, 4)? as usize;
    let mut e = Edit::default();
    let w = if v == 1 { 20 } else { 12 };
    for i in 0..n {
        let at = 8 + i * w;
        let (dur, mt) = if v == 1 { (be64(elst, at)?, be64(elst, at + 8)? as i64) } else { (be32(elst, at)? as u64, be32(elst, at + 4)? as i32 as i64) };
        if mt == -1 {
            e.empty += dur;
        } else {
            e.media_time = mt;
            break;
        }
    }
    Ok(e)
}

/// One track's samples (decode order).
#[derive(Clone, Debug, Default)]
pub struct Samples {
    pub sizes: Vec<u32>,
    pub offsets: Vec<u64>,
    pub durations: Vec<u32>,
    /// Composition offsets (`None`: none), and the `ctts` version.
    pub ctts: Option<(Vec<i32>, u8)>,
    /// Sync samples (`None`: every sample is one).
    pub sync: Option<Vec<bool>>,
    /// Each sample's description index (1-based).
    pub desc: Vec<u32>,
}

impl Samples {
    pub fn len(&self) -> usize {
        self.sizes.len()
    }

    pub fn is_sync(&self, i: usize) -> bool {
        self.sync.as_ref().is_none_or(|s| s[i])
    }

    /// Decode times.
    pub fn dts(&self) -> Vec<u64> {
        let mut t = 0u64;
        self.durations
            .iter()
            .map(|d| {
                let v = t;
                t += *d as u64;
                v
            })
            .collect()
    }

    /// Composition (presentation) time of sample `i`, given its decode time.
    pub fn cts(&self, i: usize, dts: u64) -> i64 {
        dts as i64 + self.ctts.as_ref().map_or(0, |(c, _)| c[i] as i64)
    }
}

/// What the trim needs of a track.
#[derive(Clone, Debug)]
struct Track<'a> {
    tkhd: Bx<'a>,
    mdhd: Bx<'a>,
    hdlr: Bx<'a>,
    /// The `minf` children before `stbl` (`vmhd`/`smhd`, `dinf`).
    minf_rest: Vec<Bx<'a>>,
    stsd: Bx<'a>,
    timescale: u32,
    video: bool,
    edit: Edit,
    s: Samples,
}

fn parse_samples(stbl: &[Bx]) -> Result<Samples> {
    let mut s = Samples::default();
    // Sizes.
    if let Some(stsz) = find(stbl, b"stsz") {
        let b = stsz.body;
        let (fixed, n) = (be32(b, 4)?, be32(b, 8)? as usize);
        s.sizes = if fixed != 0 { vec![fixed; n] } else { (0..n).map(|i| be32(b, 12 + i * 4)).collect::<Result<_>>()? };
    } else if let Some(stz2) = find(stbl, b"stz2") {
        let b = stz2.body;
        let (bits, n) = (b.get(7).copied().unwrap_or(0), be32(b, 8)? as usize);
        s.sizes = (0..n)
            .map(|i| match bits {
                4 => b.get(12 + i / 2).map(|v| if i % 2 == 0 { (v >> 4) as u32 } else { (v & 15) as u32 }).ok_or_else(|| anyhow!("stz2 too short")),
                8 => b.get(12 + i).map(|v| *v as u32).ok_or_else(|| anyhow!("stz2 too short")),
                16 => be16(b, 12 + i * 2).map(u32::from),
                _ => Err(anyhow!("bad stz2 field size {bits}")),
            })
            .collect::<Result<_>>()?;
    } else {
        bail!("no sample sizes");
    }
    let n = s.sizes.len();
    // Chunk offsets.
    let chunks: Vec<u64> = if let Some(co) = find(stbl, b"stco") {
        (0..be32(co.body, 4)? as usize).map(|i| be32(co.body, 8 + i * 4).map(u64::from)).collect::<Result<_>>()?
    } else {
        let co = need(stbl, b"co64")?;
        (0..be32(co.body, 4)? as usize).map(|i| be64(co.body, 8 + i * 8)).collect::<Result<_>>()?
    };
    // Samples per chunk.
    let stsc = need(stbl, b"stsc")?.body;
    let runs: Vec<(u32, u32, u32)> =
        (0..be32(stsc, 4)? as usize).map(|i| Ok((be32(stsc, 8 + i * 12)?, be32(stsc, 12 + i * 12)?, be32(stsc, 16 + i * 12)?))).collect::<Result<_>>()?;
    s.offsets.reserve(n);
    s.desc.reserve(n);
    let mut k = 0usize;
    for (ci, &chunk_at) in chunks.iter().enumerate() {
        let c = ci as u32 + 1;
        let run = runs.iter().rev().find(|r| r.0 <= c).ok_or_else(|| anyhow!("bad stsc"))?;
        let mut at = chunk_at;
        for _ in 0..run.1 {
            if k >= n {
                break;
            }
            s.offsets.push(at);
            s.desc.push(run.2);
            at += s.sizes[k] as u64;
            k += 1;
        }
    }
    ensure!(k == n, "the chunks hold {k} of {n} samples");
    // Durations.
    let stts = need(stbl, b"stts")?.body;
    for i in 0..be32(stts, 4)? as usize {
        let (cnt, d) = (be32(stts, 8 + i * 8)?, be32(stts, 12 + i * 8)?);
        s.durations.extend(std::iter::repeat_n(d, cnt as usize));
    }
    s.durations.resize(n, s.durations.last().copied().unwrap_or(0));
    // Composition offsets.
    if let Some(ctts) = find(stbl, b"ctts") {
        let b = ctts.body;
        let mut c = Vec::with_capacity(n);
        for i in 0..be32(b, 4)? as usize {
            let (cnt, off) = (be32(b, 8 + i * 8)?, be32(b, 12 + i * 8)? as i32);
            c.extend(std::iter::repeat_n(off, cnt as usize));
        }
        c.resize(n, 0);
        s.ctts = Some((c, b[0]));
    }
    // Sync samples.
    if let Some(stss) = find(stbl, b"stss") {
        let b = stss.body;
        let mut sync = vec![false; n];
        for i in 0..be32(b, 4)? as usize {
            let k = be32(b, 8 + i * 4)? as usize;
            if (1..=n).contains(&k) {
                sync[k - 1] = true;
            }
        }
        s.sync = Some(sync);
    }
    Ok(s)
}

fn parse_track<'a>(trak: Bx<'a>) -> Result<Track<'a>> {
    let kids = children(trak.body)?;
    let tkhd = need(&kids, b"tkhd")?;
    let mdia = children(need(&kids, b"mdia")?.body)?;
    let mdhd = need(&mdia, b"mdhd")?;
    let hdlr = need(&mdia, b"hdlr")?;
    let minf_all = children(need(&mdia, b"minf")?.body)?;
    let stbl = children(need(&minf_all, b"stbl")?.body)?;
    let stsd = need(&stbl, b"stsd")?;
    let minf_rest = minf_all.into_iter().filter(|b| &b.kind != b"stbl").collect();
    let timescale = if mdhd.body.first() == Some(&1) { be32(mdhd.body, 20)? } else { be32(mdhd.body, 12)? };
    ensure!(timescale > 0, "a track without a time scale");
    let video = hdlr.body.get(8..12) == Some(b"vide");
    let edit = match find(&kids, b"edts").map(|e| children(e.body)).transpose()?.and_then(|e| find(&e, b"elst")) {
        Some(elst) => read_edit(elst.body)?,
        None => Edit::default(),
    };
    Ok(Track { tkhd, mdhd, hdlr, minf_rest, stsd, timescale, video, edit, s: parse_samples(&stbl)? })
}

// --- the plan -----------------------------------------------------------------

/// The samples of a track to keep, and its new edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    /// Samples `first..last` (decode order).
    pub first: usize,
    pub last: usize,
    /// Leading empty time (movie timescale), the media time the shown part
    /// starts at in the new track (track timescale), and how long it
    /// shows (movie timescale).
    pub empty: u64,
    pub media_time: u64,
    pub shown: u64,
}

fn ticks(d: Duration, scale: u32) -> u64 {
    (d.as_nanos() * scale as u128 / 1_000_000_000) as u64
}

/// Which samples of a track to keep for `start..end` (presentation
/// time), the track's `edit` and timescale given, the movie's timescale
/// `movie`. Video starts at the last key frame shown at or before the
/// start and ends with the GOP that shows the end; sound starts
/// `preroll` before the start.
pub fn plan(s: &Samples, edit: Edit, timescale: u32, movie: u32, video: bool, start: Duration, end: Duration, preroll: Duration) -> Option<Keep> {
    let n = s.len();
    if n == 0 || end <= start {
        return None;
    }
    let dts = s.dts();
    // Presentation p → media time: edit.media_time + (p - empty).
    let empty_d = Duration::from_nanos((edit.empty as u128 * 1_000_000_000 / movie.max(1) as u128) as u64);
    let from = start.max(empty_d) - empty_d; // into the media part
    let to = end.saturating_sub(empty_d);
    if to.is_zero() {
        return None;
    }
    let m0 = edit.media_time.max(0) as u64;
    let (ms, me) = (m0 + ticks(from, timescale), m0 + ticks(to, timescale));
    let (first, last) = if video {
        let first = (0..n).rev().find(|&i| s.is_sync(i) && s.cts(i, dts[i]) <= ms as i64).or_else(|| (0..n).find(|&i| s.is_sync(i)))?;
        let last = (first + 1..n).find(|&i| s.is_sync(i) && s.cts(i, dts[i]) > me as i64).unwrap_or(n);
        (first, last)
    } else {
        let back = ms.saturating_sub(ticks(preroll, timescale));
        let first = (0..n).rev().find(|&i| dts[i] <= back).unwrap_or(0);
        let last = (first..n).find(|&i| dts[i] >= me).unwrap_or(n).max(first + 1);
        (first, last)
    };
    let base = dts[first];
    let empty = ticks(empty_d.saturating_sub(start), movie);
    let media_time = ms.saturating_sub(base);
    let shown = ticks(end - start.max(empty_d), movie);
    Some(Keep { first, last, empty, media_time, shown })
}

// --- writing ------------------------------------------------------------------

fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn put64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 8);
    put32(&mut b, (body.len() + 8) as u32);
    b.extend_from_slice(kind);
    b.extend_from_slice(body);
    b
}

fn full(kind: &[u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
    let mut b = vec![version, 0, 0, 0];
    b.extend_from_slice(body);
    boxed(kind, &b)
}

/// A copy of a header box's body with its duration set (versions 0 and 1:
/// `mvhd`, `mdhd` at 16 / 24, `tkhd` at 20 / 28).
fn with_duration(body: &[u8], at0: usize, at1: usize, d: u64) -> Vec<u8> {
    let mut b = body.to_vec();
    if b.first() == Some(&1) {
        if b.len() >= at1 + 8 {
            b[at1..at1 + 8].copy_from_slice(&d.to_be_bytes());
        }
    } else if b.len() >= at0 + 4 {
        b[at0..at0 + 4].copy_from_slice(&(d.min(u32::MAX as u64) as u32).to_be_bytes());
    }
    b
}

/// Run-length (count, value) pairs.
fn runs<T: PartialEq + Copy>(v: impl Iterator<Item = T>) -> Vec<(u32, T)> {
    let mut out: Vec<(u32, T)> = Vec::new();
    for x in v {
        match out.last_mut() {
            Some((n, y)) if *y == x => *n += 1,
            _ => out.push((1, x)),
        }
    }
    out
}

fn elst(k: &Keep) -> Vec<u8> {
    let wide = k.shown > u32::MAX as u64 || k.media_time > i32::MAX as u64 || k.empty > u32::MAX as u64;
    let entries: Vec<(u64, i64)> = if k.empty > 0 { vec![(k.empty, -1), (k.shown, k.media_time as i64)] } else { vec![(k.shown, k.media_time as i64)] };
    let mut b = Vec::new();
    put32(&mut b, entries.len() as u32);
    for (d, mt) in entries {
        if wide {
            put64(&mut b, d);
            put64(&mut b, mt as u64);
        } else {
            put32(&mut b, d as u32);
            put32(&mut b, mt as i32 as u32);
        }
        put32(&mut b, 0x0001_0000); // rate 1.0
    }
    boxed(b"edts", &full(b"elst", u8::from(wide), &b))
}

/// The new `trak` for `t` keeping `k`, its samples at `offsets`.
fn write_trak(t: &Track, k: &Keep, offsets: &[u64]) -> Vec<u8> {
    let s = &t.s;
    let r = k.first..k.last;
    let media_dur: u64 = s.durations[r.clone()].iter().map(|d| *d as u64).sum();
    let mut stbl = t.stsd.raw.to_vec();
    // stts
    let rl = runs(s.durations[r.clone()].iter().copied());
    let mut b = Vec::new();
    put32(&mut b, rl.len() as u32);
    for (n, d) in rl {
        put32(&mut b, n);
        put32(&mut b, d);
    }
    stbl.extend(full(b"stts", 0, &b));
    // ctts
    if let Some((c, v)) = &s.ctts {
        let rl = runs(c[r.clone()].iter().copied());
        let mut b = Vec::new();
        put32(&mut b, rl.len() as u32);
        for (n, o) in rl {
            put32(&mut b, n);
            put32(&mut b, o as u32);
        }
        stbl.extend(full(b"ctts", *v, &b));
    }
    // stss
    if s.sync.is_some() {
        let idx: Vec<u32> = r.clone().filter(|&i| s.is_sync(i)).map(|i| (i - k.first + 1) as u32).collect();
        let mut b = Vec::new();
        put32(&mut b, idx.len() as u32);
        for i in idx {
            put32(&mut b, i);
        }
        stbl.extend(full(b"stss", 0, &b));
    }
    // stsz
    let mut b = Vec::new();
    put32(&mut b, 0);
    put32(&mut b, r.len() as u32);
    for i in r.clone() {
        put32(&mut b, s.sizes[i]);
    }
    stbl.extend(full(b"stsz", 0, &b));
    // stsc: one sample a chunk, a run per description.
    let mut b = Vec::new();
    let mut entries: Vec<(u32, u32)> = Vec::new();
    for (j, i) in r.clone().enumerate() {
        if entries.last().is_none_or(|e| e.1 != s.desc[i]) {
            entries.push((j as u32 + 1, s.desc[i]));
        }
    }
    put32(&mut b, entries.len() as u32);
    for (first, d) in entries {
        put32(&mut b, first);
        put32(&mut b, 1);
        put32(&mut b, d);
    }
    stbl.extend(full(b"stsc", 0, &b));
    // co64
    let mut b = Vec::new();
    put32(&mut b, offsets.len() as u32);
    for o in offsets {
        put64(&mut b, *o);
    }
    stbl.extend(full(b"co64", 0, &b));
    let mut minf = Vec::new();
    for x in &t.minf_rest {
        minf.extend_from_slice(x.raw);
    }
    minf.extend(boxed(b"stbl", &stbl));
    let mut mdia = boxed(b"mdhd", &with_duration(t.mdhd.body, 16, 24, media_dur));
    mdia.extend_from_slice(t.hdlr.raw);
    mdia.extend(boxed(b"minf", &minf));
    let mut trak = boxed(b"tkhd", &with_duration(t.tkhd.body, 20, 28, k.empty + k.shown));
    trak.extend(elst(k));
    trak.extend(boxed(b"mdia", &mdia));
    boxed(b"trak", &trak)
}

/// How a trim went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trimmed {
    /// The video's kept samples (decode order) and how many come before
    /// the start (decoded, not shown).
    pub video_samples: usize,
    pub lead_in: usize,
    pub bytes: u64,
}

/// Copy `src`'s `start..end` (presentation time) into `dst`.
pub fn trim<R: Read + Seek, W: Write>(src: &mut R, dst: &mut W, start: Duration, end: Duration) -> Result<Trimmed> {
    ensure!(end > start, "the trim is empty");
    let tops = top_boxes(src)?;
    let ftyp = tops.iter().find(|t| &t.kind == b"ftyp").copied();
    let moov_at = tops.iter().find(|t| &t.kind == b"moov").copied().context("not an MP4 (no moov)")?;
    let read = |src: &mut R, t: Top| -> Result<Vec<u8>> {
        src.seek(SeekFrom::Start(t.at))?;
        let mut v = vec![0u8; t.size as usize];
        src.read_exact(&mut v)?;
        Ok(v)
    };
    let ftyp_raw = ftyp.map(|t| read(src, t)).transpose()?;
    let moov_raw = read(src, moov_at)?;
    let moov_kids = children(&moov_raw[moov_at.head as usize..])?;
    ensure!(find(&moov_kids, b"mvex").is_none(), "a fragmented MP4 cannot be trimmed here");
    let mvhd = need(&moov_kids, b"mvhd")?;
    let movie = if mvhd.body.first() == Some(&1) { be32(mvhd.body, 20)? } else { be32(mvhd.body, 12)? }.max(1);
    let tracks: Vec<Track> = moov_kids.iter().filter(|b| &b.kind == b"trak").cloned().map(parse_track).collect::<Result<_>>()?;
    ensure!(!tracks.is_empty(), "no tracks");
    let keeps: Vec<Option<Keep>> =
        tracks.iter().map(|t| plan(&t.s, t.edit, t.timescale, movie, t.video, start, end, if t.video { Duration::ZERO } else { AUDIO_PREROLL })).collect();
    ensure!(keeps.iter().any(Option::is_some), "nothing to keep in that range");
    // The samples in file order, and where each goes in the new mdat.
    let mut order: Vec<(u64, usize, usize)> = Vec::new(); // (old offset, track, sample)
    for (ti, (t, k)) in tracks.iter().zip(&keeps).enumerate() {
        if let Some(k) = k {
            order.extend((k.first..k.last).map(|i| (t.s.offsets[i], ti, i)));
        }
    }
    order.sort_unstable();
    let mut rel: Vec<Vec<u64>> = tracks.iter().map(|_| Vec::new()).collect();
    let mut placed: Vec<(usize, usize, u64)> = Vec::with_capacity(order.len());
    let mut pos = 0u64;
    for &(_, ti, i) in &order {
        placed.push((ti, i, pos));
        pos += tracks[ti].s.sizes[i] as u64;
    }
    let mdat_len = pos;
    // Each track's offsets in its sample order.
    for (ti, k) in keeps.iter().enumerate() {
        if let Some(k) = k {
            rel[ti] = vec![0; k.last - k.first];
        }
    }
    for &(ti, i, p) in &placed {
        if let Some(k) = &keeps[ti] {
            rel[ti][i - k.first] = p;
        }
    }
    let moov = |shift: u64| -> Vec<u8> {
        let shown = keeps.iter().flatten().map(|k| k.empty + k.shown).max().unwrap_or(0);
        let mut body = boxed(b"mvhd", &with_duration(mvhd.body, 16, 24, shown));
        for (ti, (t, k)) in tracks.iter().zip(&keeps).enumerate() {
            if let Some(k) = k {
                let offs: Vec<u64> = rel[ti].iter().map(|o| o + shift).collect();
                body.extend(write_trak(t, k, &offs));
            }
        }
        for b in moov_kids.iter().filter(|b| !matches!(&b.kind, b"mvhd" | b"trak")) {
            body.extend_from_slice(b.raw);
        }
        boxed(b"moov", &body)
    };
    let ftyp_bytes = ftyp_raw.unwrap_or_else(|| boxed(b"ftyp", b"isom\0\0\x02\0isomiso6mp41"));
    let moov_len = moov(0).len() as u64;
    let data_at = ftyp_bytes.len() as u64 + moov_len + 16;
    dst.write_all(&ftyp_bytes)?;
    dst.write_all(&moov(data_at))?;
    let mut head = Vec::new();
    put32(&mut head, 1);
    head.extend_from_slice(b"mdat");
    put64(&mut head, mdat_len + 16);
    dst.write_all(&head)?;
    // Copy the samples, reading runs of adjacent ones at once.
    let mut buf = Vec::new();
    let mut k = 0;
    while k < order.len() {
        let start_at = order[k].0;
        let mut end_at = start_at + tracks[order[k].1].s.sizes[order[k].2] as u64;
        let mut j = k + 1;
        while j < order.len() && order[j].0 == end_at && end_at - start_at < (4 << 20) {
            end_at += tracks[order[j].1].s.sizes[order[j].2] as u64;
            j += 1;
        }
        buf.resize((end_at - start_at) as usize, 0);
        src.seek(SeekFrom::Start(start_at))?;
        src.read_exact(&mut buf).context("read the samples")?;
        dst.write_all(&buf)?;
        k = j;
    }
    dst.flush()?;
    let (video_samples, lead_in) = tracks
        .iter()
        .zip(&keeps)
        .find(|(t, _)| t.video)
        .and_then(|(t, k)| {
            let k = (*k)?;
            let dts = t.s.dts();
            let ms = dts[k.first] + k.media_time;
            Some((k.last - k.first, (k.first..k.last).filter(|&i| t.s.cts(i, dts[i]) < ms as i64).count()))
        })
        .unwrap_or((0, 0));
    Ok(Trimmed { video_samples, lead_in, bytes: data_at + mdat_len })
}

/// The presentation length of an MP4 (`mvhd`'s duration).
pub fn duration<R: Read + Seek>(src: &mut R) -> Result<Duration> {
    let tops = top_boxes(src)?;
    let t = tops.iter().find(|t| &t.kind == b"moov").copied().context("not an MP4 (no moov)")?;
    src.seek(SeekFrom::Start(t.at))?;
    let mut v = vec![0u8; t.size as usize];
    src.read_exact(&mut v)?;
    let kids = children(&v[t.head as usize..])?;
    let mvhd = need(&kids, b"mvhd")?;
    let (scale, d) = if mvhd.body.first() == Some(&1) { (be32(mvhd.body, 20)?, be64(mvhd.body, 24)?) } else { (be32(mvhd.body, 12)?, be32(mvhd.body, 16)? as u64) };
    ensure!(scale > 0, "no time scale");
    Ok(Duration::from_nanos((d as u128 * 1_000_000_000 / scale as u128) as u64))
}

/// The key frames' presentation times of the first video track (the trim
/// window snaps nothing to them, but says where the cut really starts).
pub fn key_times<R: Read + Seek>(src: &mut R) -> Result<Vec<Duration>> {
    let tops = top_boxes(src)?;
    let t = tops.iter().find(|t| &t.kind == b"moov").copied().context("not an MP4 (no moov)")?;
    src.seek(SeekFrom::Start(t.at))?;
    let mut v = vec![0u8; t.size as usize];
    src.read_exact(&mut v)?;
    let kids = children(&v[t.head as usize..])?;
    let Some(tr) = kids.iter().filter(|b| &b.kind == b"trak").cloned().map(parse_track).find(|t| t.as_ref().is_ok_and(|t| t.video)) else {
        return Ok(Vec::new());
    };
    let tr = tr?;
    let dts = tr.s.dts();
    let m0 = tr.edit.media_time.max(0);
    Ok((0..tr.s.len())
        .filter(|&i| tr.s.is_sync(i))
        .map(|i| {
            let p = (tr.s.cts(i, dts[i]) - m0).max(0) as u128;
            Duration::from_nanos((p * 1_000_000_000 / tr.timescale as u128) as u64)
        })
        .collect())
}

#[cfg(test)]
#[path = "mp4_tests.rs"]
mod tests;
