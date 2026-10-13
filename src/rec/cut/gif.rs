//! GIF frames: an index of a file's frames ([`Index`]), a decoder that
//! composes them ([`Player`], for the trim window's thumbnails and
//! preview) and trimming by frames ([`trim`]).
//!
//! A trim keeps the frames shown during `start..end`. The first of them
//! is written again as a whole picture (in a GIF a frame may hold only
//! what changed since the one before), with its delay shortened to what
//! is left of it after the start; the rest are copied byte for byte, the
//! last with its delay cut at the end. When the first kept frame clears
//! itself afterwards (disposal "background" or "previous", which our
//! encoder never writes) every kept frame is composed and encoded again.

use super::super::gif::GifEncoder;
use super::super::{Frame as RecFrame, VideoEncoder};
use anyhow::{anyhow, bail, ensure, Result};
use std::io::{Cursor, Read, Seek, Write};
use std::sync::Arc;
use std::time::Duration;

/// One frame of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    /// Its bytes: the graphic control extension (when it has one) through
    /// the end of the image data.
    pub start: usize,
    pub end: usize,
    /// Where its graphic control extension starts.
    pub gce: Option<usize>,
    pub delay_cs: u16,
    pub disposal: u8,
    pub transparent: Option<u8>,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub interlaced: bool,
    /// The local color table (offset, entries).
    pub palette: Option<(usize, usize)>,
    /// The LZW minimum code size's offset (the data follows it).
    pub data: usize,
}

/// A GIF's frames, with the file in memory.
pub struct Index {
    pub bytes: Vec<u8>,
    pub w: u16,
    pub h: u16,
    /// The global color table (offset, entries).
    gct: Option<(usize, usize)>,
    /// Where the first frame's bytes start (header, global table and the
    /// extensions before it come first: the loop count).
    pub head_end: usize,
    pub frames: Vec<FrameInfo>,
}

fn u16le(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| anyhow!("the GIF is cut short"))
}

fn byte(b: &[u8], at: usize) -> Result<u8> {
    b.get(at).copied().ok_or_else(|| anyhow!("the GIF is cut short"))
}

/// Skip data sub-blocks starting at `at`; the offset after the terminator.
fn skip_blocks(b: &[u8], mut at: usize) -> Result<usize> {
    loop {
        let n = byte(b, at)? as usize;
        at += 1;
        if n == 0 {
            return Ok(at);
        }
        at += n;
        ensure!(at <= b.len(), "the GIF is cut short");
    }
}

impl Index {
    pub fn read<R: Read>(r: &mut R) -> Result<Index> {
        let mut bytes = Vec::new();
        r.read_to_end(&mut bytes)?;
        Index::parse(bytes)
    }

    pub fn parse(bytes: Vec<u8>) -> Result<Index> {
        let b = &bytes;
        ensure!(b.len() >= 13 && (b.starts_with(b"GIF89a") || b.starts_with(b"GIF87a")), "not a GIF");
        let (w, h) = (u16le(b, 6)?, u16le(b, 8)?);
        let flags = b[10];
        let mut at = 13;
        let gct = if flags & 0x80 != 0 {
            let n = 2usize << (flags & 7);
            let g = Some((at, n));
            at += 3 * n;
            g
        } else {
            None
        };
        let mut frames = Vec::new();
        let mut head_end = None;
        let mut gce: Option<(usize, u16, u8, Option<u8>)> = None;
        loop {
            match byte(b, at)? {
                0x21 => {
                    let label = byte(b, at + 1)?;
                    if label == 0xF9 && byte(b, at + 2)? == 4 {
                        let packed = byte(b, at + 3)?;
                        let delay = u16le(b, at + 4)?;
                        let t = (packed & 1 != 0).then_some(byte(b, at + 6)?);
                        gce = Some((at, delay, (packed >> 2) & 7, t));
                    }
                    at = skip_blocks(b, at + 2)?;
                }
                0x2C => {
                    let start = gce.map_or(at, |g| g.0);
                    head_end.get_or_insert(start);
                    let (x, y, fw, fh) = (u16le(b, at + 1)?, u16le(b, at + 3)?, u16le(b, at + 5)?, u16le(b, at + 7)?);
                    let pf = byte(b, at + 9)?;
                    at += 10;
                    let palette = if pf & 0x80 != 0 {
                        let n = 2usize << (pf & 7);
                        let p = Some((at, n));
                        at += 3 * n;
                        p
                    } else {
                        None
                    };
                    let data = at;
                    at = skip_blocks(b, at + 1)?;
                    let (g_at, delay, disposal, transparent) = gce.take().map_or((None, 0, 0, None), |g| (Some(g.0), g.1, g.2, g.3));
                    frames.push(FrameInfo {
                        start,
                        end: at,
                        gce: g_at,
                        delay_cs: delay,
                        disposal,
                        transparent,
                        x,
                        y,
                        w: fw,
                        h: fh,
                        interlaced: pf & 0x40 != 0,
                        palette,
                        data,
                    });
                }
                0x3B => break,
                other => bail!("a broken GIF (block 0x{other:02X} at {at})"),
            }
        }
        ensure!(!frames.is_empty(), "the GIF has no frames");
        let head_end = head_end.unwrap_or(at);
        Ok(Index { bytes, w, h, gct, head_end, frames })
    }

    /// Each frame's start time (centiseconds).
    pub fn starts_cs(&self) -> Vec<u64> {
        let mut t = 0;
        self.frames
            .iter()
            .map(|f| {
                let s = t;
                t += f.delay_cs as u64;
                s
            })
            .collect()
    }

    pub fn length(&self) -> Duration {
        Duration::from_millis(self.frames.iter().map(|f| f.delay_cs as u64).sum::<u64>() * 10)
    }

    /// The frame shown at `t`.
    pub fn frame_at(&self, t: Duration) -> usize {
        let cs = (t.as_millis() / 10) as u64;
        let starts = self.starts_cs();
        starts.iter().rposition(|&s| s <= cs).unwrap_or(0)
    }

    fn palette(&self, f: &FrameInfo) -> &[u8] {
        let (at, n) = f.palette.or(self.gct).unwrap_or((0, 0));
        self.bytes.get(at..at + 3 * n).unwrap_or(&[])
    }
}

/// GIF LZW: `data` is the sub-blocks after the minimum code size; at most
/// `max` indices come out.
pub fn lzw_decode(data: &[u8], min: u8, max: usize) -> Vec<u8> {
    let min = u32::from(min.clamp(2, 11));
    let clear = 1u32 << min;
    let eoi = clear + 1;
    let mut prefix = vec![0u16; 4096];
    let mut suffix = vec![0u8; 4096];
    for (i, s) in suffix.iter_mut().take(clear as usize).enumerate() {
        *s = i as u8;
    }
    let mut out = Vec::with_capacity(max);
    let mut width = min + 1;
    let mut next = eoi + 1;
    let mut prev: Option<u32> = None;
    let (mut acc, mut nbits) = (0u32, 0u32);
    let mut stack = Vec::with_capacity(4096);
    for &byte in data {
        acc |= u32::from(byte) << nbits;
        nbits += 8;
        while nbits >= width {
            let code = acc & ((1 << width) - 1);
            acc >>= width;
            nbits -= width;
            if code == clear {
                width = min + 1;
                next = eoi + 1;
                prev = None;
                continue;
            }
            if code == eoi || out.len() >= max {
                return out;
            }
            let Some(p) = prev else {
                if code < clear {
                    out.push(code as u8);
                    prev = Some(code);
                }
                continue;
            };
            // The string of `code` (or, not yet in the table, of the
            // previous code plus its first byte).
            let known = code < next;
            let mut c = if known { code } else { p };
            stack.clear();
            while c >= clear {
                stack.push(suffix[c as usize]);
                c = prefix[c as usize] as u32;
            }
            stack.push(c as u8);
            let first = c as u8;
            out.extend(stack.iter().rev());
            if !known {
                out.push(first);
            }
            if next < 4096 {
                prefix[next as usize] = p as u16;
                suffix[next as usize] = first;
                next += 1;
                if next == 1 << width && width < 12 {
                    width += 1;
                }
            }
            prev = Some(code);
        }
    }
    out.truncate(max);
    out
}

/// The image data's sub-blocks joined.
fn data_of(b: &[u8], at: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = at;
    while let Some(&n) = b.get(i) {
        i += 1;
        if n == 0 {
            break;
        }
        let end = (i + n as usize).min(b.len());
        out.extend_from_slice(&b[i..end]);
        i = end;
    }
    out
}

/// Composes frames onto the canvas (RGBA), with seek checkpoints.
pub struct Player {
    pub idx: Arc<Index>,
    canvas: Vec<u8>,
    /// The next frame to draw.
    next: usize,
    /// The canvas before the last drawn frame (its disposal "previous").
    saved: Option<Vec<u8>>,
    /// Canvases after every `every`-th frame (and the state to go on from).
    checkpoints: Vec<(usize, Vec<u8>)>,
    every: usize,
}

/// Memory the checkpoints may take.
const CHECKPOINT_BUDGET: usize = 64 << 20;

impl Player {
    pub fn new(idx: Arc<Index>) -> Player {
        let size = idx.w as usize * idx.h as usize * 4;
        let every = (idx.frames.len() * size / CHECKPOINT_BUDGET.max(1)).max(16);
        Player { canvas: vec![0; size], idx, next: 0, saved: None, checkpoints: Vec::new(), every }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.idx.w as u32, self.idx.h as u32)
    }

    /// The canvas as shown during frame `i` (RGBA).
    pub fn frame(&mut self, i: usize) -> &[u8] {
        let i = i.min(self.idx.frames.len() - 1);
        if i + 1 < self.next {
            // Back: from the last checkpoint at or before it.
            match self.checkpoints.iter().rev().find(|(k, _)| *k <= i) {
                Some((k, c)) => {
                    self.canvas.copy_from_slice(c);
                    self.next = k + 1;
                }
                None => {
                    self.canvas.fill(0);
                    self.next = 0;
                }
            }
            self.saved = None;
        }
        while self.next <= i {
            self.draw(self.next);
            self.next += 1;
        }
        &self.canvas
    }

    fn draw(&mut self, i: usize) {
        let idx = self.idx.clone();
        let (cw, ch) = (idx.w as usize, idx.h as usize);
        // The previous frame's disposal first.
        if i > 0 {
            let p = &idx.frames[i - 1];
            match p.disposal {
                2 => {
                    let (x0, x1) = ((p.x as usize).min(cw), (p.x as usize + p.w as usize).min(cw));
                    for y in p.y as usize..(p.y as usize + p.h as usize).min(ch) {
                        self.canvas[(y * cw + x0) * 4..(y * cw + x1) * 4].fill(0);
                    }
                }
                3 => {
                    if let Some(s) = self.saved.take() {
                        self.canvas = s;
                    }
                }
                _ => {}
            }
        }
        let f = &idx.frames[i];
        if f.disposal == 3 {
            self.saved = Some(self.canvas.clone());
        }
        let pal = idx.palette(f);
        let min = idx.bytes.get(f.data).copied().unwrap_or(2);
        let px = lzw_decode(&data_of(&idx.bytes, f.data + 1), min, f.w as usize * f.h as usize);
        let (fw, fh) = (f.w as usize, f.h as usize);
        let rows: Vec<usize> = if f.interlaced {
            (0..fh).step_by(8).chain((4..fh).step_by(8)).chain((2..fh).step_by(4)).chain((1..fh).step_by(2)).collect()
        } else {
            (0..fh).collect()
        };
        for (k, &ry) in rows.iter().enumerate() {
            let y = f.y as usize + ry;
            if y >= ch {
                continue;
            }
            for rx in 0..fw {
                let x = f.x as usize + rx;
                let Some(&v) = px.get(k * fw + rx) else { break };
                if x >= cw || Some(v) == f.transparent {
                    continue;
                }
                let Some(c) = pal.get(v as usize * 3..v as usize * 3 + 3) else { continue };
                let o = (y * cw + x) * 4;
                self.canvas[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        if i % self.every == 0 && self.checkpoints.last().is_none_or(|(k, _)| *k < i) && f.disposal != 3 {
            self.checkpoints.push((i, self.canvas.clone()));
        }
    }
}

/// RGBA → the BGRA our encoder takes (opaque: a hole shows black).
fn bgra_of(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for p in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    out
}

/// The frames to keep for `start..end`: first, last (inclusive), the
/// first's delay after the start and the last's up to the end (cs).
pub fn plan(idx: &Index, start: Duration, end: Duration) -> Option<(usize, usize, u16, u16)> {
    let (s, e) = ((start.as_millis() / 10) as u64, (end.as_millis() / 10) as u64);
    if e <= s {
        return None;
    }
    let starts = idx.starts_cs();
    let shown = |i: usize| (starts[i], starts[i] + idx.frames[i].delay_cs as u64);
    let first = (0..idx.frames.len()).find(|&i| shown(i).1 > s)?;
    let last = (first..idx.frames.len()).rev().find(|&i| shown(i).0 < e).unwrap_or(first);
    let cut = |i: usize| {
        let (a, b) = shown(i);
        (b.min(e).saturating_sub(a.max(s))).min(0xFFFF) as u16
    };
    if first == last {
        return Some((first, last, cut(first), cut(first)));
    }
    let first_d = (shown(first).1 - s.max(shown(first).0)).min(0xFFFF) as u16;
    let last_d = (e.min(shown(last).1) - shown(last).0).min(0xFFFF) as u16;
    Some((first, last, first_d, last_d))
}

/// `canvas` (RGBA, w×h) as one GIF frame block (graphic control
/// extension + image) with `delay` centiseconds.
fn whole_frame(canvas: &[u8], w: u32, h: u32, delay: u16) -> Result<Vec<u8>> {
    let mut enc = GifEncoder::new(Cursor::new(Vec::new()), w, h, 100)?;
    enc.push_video(&RecFrame::bgra(w, h, bgra_of(canvas), Duration::ZERO))?;
    let out = enc.into_inner()?.into_inner();
    // Header (13) and the loop extension (19) before it, the trailer after.
    ensure!(out.len() > 33, "an empty frame");
    let mut block = out[32..out.len() - 1].to_vec();
    ensure!(block.starts_with(&[0x21, 0xF9, 0x04]), "no control extension");
    block[4..6].copy_from_slice(&delay.to_le_bytes());
    Ok(block)
}

/// Write `src`'s `start..end` to `dst`.
pub fn trim<R: Read, W: Write + Seek + Send>(src: &mut R, dst: &mut W, start: Duration, end: Duration) -> Result<()> {
    let idx = Arc::new(Index::read(src)?);
    let (first, last, first_d, last_d) = plan(&idx, start, end).ok_or_else(|| anyhow!("nothing to keep in that range"))?;
    let (w, h) = (idx.w as u32, idx.h as u32);
    let mut player = Player::new(idx.clone());
    if matches!(idx.frames[first].disposal, 2 | 3) {
        // Composed and encoded again, frame by frame.
        let mut enc = GifEncoder::new(&mut *dst, w, h, 100)?;
        let starts = idx.starts_cs();
        let s_cs = (start.as_millis() / 10) as u64;
        for i in first..=last {
            let ts = Duration::from_millis(starts[i].max(s_cs).saturating_sub(s_cs) * 10);
            let px = bgra_of(player.frame(i));
            enc.push_video(&RecFrame::bgra(w, h, px, ts))?;
        }
        // The last frame lasts until the end (a repeat only extends it).
        let px = bgra_of(player.frame(last));
        enc.push_video(&RecFrame::bgra(w, h, px, end.saturating_sub(start)))?;
        return Box::new(enc).finish();
    }
    let b = &idx.bytes;
    dst.write_all(&b[..idx.head_end])?;
    dst.write_all(&whole_frame(player.frame(first), w, h, first_d)?)?;
    for i in first + 1..=last {
        let f = &idx.frames[i];
        let mut bytes = b[f.start..f.end].to_vec();
        if i == last
            && let Some(g) = f.gce
        {
            let at = g - f.start + 4;
            bytes[at..at + 2].copy_from_slice(&last_d.to_le_bytes());
        }
        dst.write_all(&bytes)?;
    }
    dst.write_all(&[0x3B])?;
    dst.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "gif_tests.rs"]
mod tests;
