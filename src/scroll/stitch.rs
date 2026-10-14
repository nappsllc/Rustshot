//! Stitching a scrolling capture: consecutive frames of the same screen
//! region, taken between scroll steps, are joined into one tall picture.
//!
//! For each new frame against the one before it:
//!
//! 1. **Fixed rows.** Rows at the top and at the bottom that are the same
//!    in both frames at the same place (a sticky header, a footer, a
//!    toolbar; [`FIXED_MATCH`] of a row's pixels within [`TOL`]) are left
//!    out of the search. If nothing outside them moved, the frame is
//!    [`Step::Still`] (the end of the content).
//! 2. **Offset.** Every row of the rest (the scrolling zone) gets a
//!    profile: its average color in [`BINS`] column bins. For each
//!    offset `dy` the cost is the mean (each row's capped at [`ROW_CAP`],
//!    so animated rows cannot dominate) difference between the new
//!    frame's rows and the old frame's rows `dy` further down, over the
//!    rows with something in them (a blank row matches anything). The
//!    lowest costs (ties: the one nearest the expected step) are checked
//!    pixel by pixel: [`VERIFY`] of the rows checked must match, a row
//!    matching when [`ROW_MATCH`] of its pixels are within [`TOL`].
//!    Nothing passing is [`Step::Lost`].
//! 3. **Assembly.** The first frame is kept whole; every later one adds
//!    the `dy` rows it revealed at the bottom of the scrolling zone. The
//!    footer (the fewest fixed bottom rows any pair had) comes once, from
//!    the last frame, so a sticky footer is not repeated down the page.
//!
//! The result never grows past [`MAX_H`]. Pure: tested on synthetic
//! scrolled pages (sticky header and footer, uneven steps, noise, an
//! animation in the zone).

use crate::pixbuf::PixBuf;

/// The tallest stitched picture (px).
pub const MAX_H: u32 = 20_000;
/// Nor past this many bytes (a wide region stops sooner).
pub const MAX_BYTES: usize = 400 << 20;

/// The most rows a `w` px wide picture may have.
pub fn max_h(w: u32) -> u32 {
    (MAX_BYTES / (w.max(1) as usize * 4)).min(MAX_H as usize) as u32
}
/// Column bins of a row's profile (fewer in a narrower frame).
pub const BINS: usize = 64;
/// Per-channel difference two pixels may have and still be the same.
pub const TOL: u8 = 10;
/// Share of a row's pixels that must match for it to be a fixed row.
pub const FIXED_MATCH: f32 = 0.97;
/// Share of a row's pixels that must match for the offset check.
pub const ROW_MATCH: f32 = 0.9;
/// Share of the checked rows that must match for an offset to be taken.
pub const VERIFY: f32 = 0.6;
/// A row's cost is capped here (mean absolute difference, 0-255).
pub const ROW_CAP: u32 = 40;
/// A profile whose bins span less than this (per channel) is blank.
const INFO_SPAN: u8 = 12;
/// The fixed rows may take at most this share of the frame on each side.
const FIXED_MAX: f32 = 0.45;
/// Rows sampled for the cost of an offset (at most).
const SAMPLE_ROWS: usize = 96;
/// Offsets verified pixel by pixel (the best few).
const CANDIDATES: usize = 4;

/// What a new frame did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The content moved up by this many px (it was added).
    Moved(u32),
    /// Nothing scrolled: the end of the content (nothing added).
    Still,
    /// The frames could not be matched (nothing added).
    Lost,
}

/// An RGBA frame (top-down rows of `w * 4` bytes).
#[derive(Clone, Copy)]
pub struct Img<'a> {
    pub w: u32,
    pub h: u32,
    pub px: &'a [u8],
}

impl<'a> Img<'a> {
    pub fn of(p: &'a PixBuf) -> Img<'a> {
        Img { w: p.width(), h: p.height(), px: p.as_raw() }
    }

    fn row(&self, y: u32) -> &'a [u8] {
        let n = self.w as usize * 4;
        &self.px[y as usize * n..(y as usize + 1) * n]
    }
}

/// Row profiles of a frame: `bins` averaged RGB values a row, and which
/// rows have something in them.
pub struct Sig {
    bins: usize,
    prof: Vec<u8>,
    info: Vec<bool>,
}

impl Sig {
    pub fn of(img: Img) -> Sig {
        let (w, h) = (img.w as usize, img.h as usize);
        let bins = BINS.min(w).max(1);
        let bin_of: Vec<usize> = (0..w).map(|x| x * bins / w.max(1)).collect();
        let mut count = vec![0u32; bins];
        for &b in &bin_of {
            count[b] += 1;
        }
        let mut prof = vec![0u8; h * bins * 3];
        let mut info = vec![false; h];
        let mut acc = vec![[0u32; 3]; bins];
        for y in 0..h {
            acc.iter_mut().for_each(|a| *a = [0; 3]);
            let row = img.row(y as u32);
            for (p, &b) in row.as_chunks::<4>().0.iter().zip(&bin_of) {
                let a = &mut acc[b];
                a[0] += p[0] as u32;
                a[1] += p[1] as u32;
                a[2] += p[2] as u32;
            }
            let out = &mut prof[y * bins * 3..(y + 1) * bins * 3];
            let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
            for (b, a) in acc.iter().enumerate() {
                let n = count[b].max(1);
                for c in 0..3 {
                    let v = ((a[c] + n / 2) / n) as u8;
                    out[b * 3 + c] = v;
                    lo[c] = lo[c].min(v);
                    hi[c] = hi[c].max(v);
                }
            }
            // Something in it: the bins differ, or (one bin) the pixels do.
            info[y] = (0..3).any(|c| hi[c] - lo[c] >= INFO_SPAN) || (bins == 1 && row_span(row) >= INFO_SPAN);
        }
        Sig { bins, prof, info }
    }

    fn row(&self, y: u32) -> &[u8] {
        let n = self.bins * 3;
        &self.prof[y as usize * n..(y as usize + 1) * n]
    }
}

/// The largest per-channel spread of a row's pixels.
fn row_span(row: &[u8]) -> u8 {
    let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
    for p in row.as_chunks::<4>().0 {
        for c in 0..3 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    (0..3).map(|c| hi[c].saturating_sub(lo[c])).max().unwrap_or(0)
}

/// Whether `share` of the pixels of two rows are within [`TOL`].
fn rows_match(a: &[u8], b: &[u8], share: f32) -> bool {
    let n = a.len() / 4;
    let allowed = n - ((n as f32 * share).ceil() as usize).min(n);
    let mut off = 0;
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        if p[0].abs_diff(q[0]) > TOL || p[1].abs_diff(q[1]) > TOL || p[2].abs_diff(q[2]) > TOL {
            off += 1;
            if off > allowed {
                return false;
            }
        }
    }
    true
}

/// Mean absolute difference of two profiles (0-255).
fn prof_diff(a: &[u8], b: &[u8]) -> u32 {
    let s: u32 = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u32).sum();
    s / a.len().max(1) as u32
}

/// How two frames of the same size relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub step: Step,
    /// Fixed rows at the top and the bottom (both frames alike there).
    pub head: u32,
    pub foot: u32,
}

/// Compare `cur` with `prev` (same size, with their [`Sig`]s); `expect`
/// is the offset expected (the last one), which wins ties.
pub fn find(prev: Img, ps: &Sig, cur: Img, cs: &Sig, expect: Option<u32>) -> Match {
    let h = cur.h;
    let lost = Match { step: Step::Lost, head: 0, foot: 0 };
    if (prev.w, prev.h) != (cur.w, cur.h) || h < 8 || cur.w == 0 {
        return lost;
    }
    let cap = ((h as f32 * FIXED_MAX) as u32).max(1);
    let same = |y: u32| rows_match(prev.row(y), cur.row(y), FIXED_MATCH);
    let mut head = 0;
    while head < h && same(head) {
        head += 1;
    }
    if head == h {
        return Match { step: Step::Still, head: 0, foot: 0 };
    }
    let mut foot = 0;
    while foot < h - head && same(h - 1 - foot) {
        foot += 1;
    }
    let (head, foot) = (head.min(cap), foot.min(cap));
    let (top, bot) = (head, h - foot);
    let zh = bot - top;
    if zh < 8 {
        return Match { step: Step::Lost, head, foot };
    }
    // Rows of the zone with something in them: the ones that count.
    let informative: Vec<u32> = (top..bot).filter(|&y| cs.info[y as usize]).collect();
    if informative.len() < 4 {
        return Match { step: Step::Lost, head, foot };
    }
    // Nothing moved but an animation: still.
    let at_rest = informative.iter().filter(|&&y| rows_match(prev.row(y), cur.row(y), ROW_MATCH)).count();
    if at_rest as f32 >= informative.len() as f32 * 0.8 {
        return Match { step: Step::Still, head, foot };
    }
    let min_overlap = (zh / 8).max(8);
    let max_dy = zh.saturating_sub(min_overlap);
    // The rows the cost looks at: an even sample of the informative ones.
    let sample: Vec<u32> = informative.iter().copied().step_by(informative.len().div_ceil(SAMPLE_ROWS)).collect();
    let mut costs: Vec<(u32, u32)> = Vec::new(); // (cost x 16, dy)
    for dy in 1..=max_dy {
        // The new frame's rows y map to the old one's y + dy.
        let (mut sum, mut k) = (0u32, 0u32);
        for &y in sample.iter().take_while(|&&y| y + dy < bot) {
            sum += prof_diff(cs.row(y), ps.row(y + dy)).min(ROW_CAP);
            k += 1;
        }
        if k >= 4 {
            costs.push((sum * 16 / k, dy));
        }
    }
    // No match, and what changed is a sliver (an animation, a caret):
    // nothing scrolled.
    let lost = Match { step: if zh < h / 5 { Step::Still } else { Step::Lost }, head, foot };
    if costs.is_empty() {
        return lost;
    }
    // The local minima, best first (ties: nearest the expected step).
    let near = |dy: u32| expect.map_or(0, |e| e.abs_diff(dy));
    let mut minima: Vec<(u32, u32)> = costs
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            let l = i.checked_sub(1).map_or(u32::MAX, |j| costs[j].0);
            let r = costs.get(i + 1).map_or(u32::MAX, |x| x.0);
            c.0 <= l && c.0 <= r
        })
        .map(|(_, c)| *c)
        .collect();
    let best = minima.iter().map(|c| c.0).min().unwrap_or(0);
    // Costs within 8 % (and half a level) of the best tie.
    let tie = best + (best / 12).max(8);
    minima.sort_by_key(|&(c, dy)| (if c <= tie { 0 } else { c }, near(dy), dy));
    for &(_, dy) in minima.iter().take(CANDIDATES) {
        let rows: Vec<u32> = informative.iter().copied().filter(|&y| y + dy < bot).collect();
        let step = rows.len().div_ceil(SAMPLE_ROWS).max(1);
        let checked: Vec<u32> = rows.into_iter().step_by(step).collect();
        let ok = checked.iter().filter(|&&y| rows_match(cur.row(y), prev.row(y + dy), ROW_MATCH)).count();
        if !checked.is_empty() && ok as f32 >= checked.len() as f32 * VERIFY {
            return Match { step: Step::Moved(dy), head, foot };
        }
    }
    lost
}

/// Builds the tall picture frame by frame.
pub struct Stitcher {
    w: u32,
    h: u32,
    /// The first frame, whole.
    first: Vec<u8>,
    /// Each later frame's rows from `h - foot - dy` to its bottom, with its
    /// `dy` and `foot`.
    strips: Vec<(Vec<u8>, u32, u32)>,
    /// The previous frame and its profiles.
    prev: Option<(Vec<u8>, Sig)>,
    /// The fewest fixed bottom rows any pair had.
    foot: Option<u32>,
    /// The latest offset (the next one is expected to be alike).
    last_dy: Option<u32>,
    height: u32,
    /// [`max_h`] of its width.
    max_h: u32,
}

impl Stitcher {
    /// Start with the first frame (RGBA, `w`×`h`).
    pub fn new(first: Img) -> Stitcher {
        let sig = Sig::of(first);
        Stitcher {
            w: first.w,
            h: first.h,
            first: first.px.to_vec(),
            strips: Vec::new(),
            prev: Some((first.px.to_vec(), sig)),
            foot: None,
            last_dy: None,
            height: first.h,
            max_h: max_h(first.w),
        }
    }

    /// The stitched height so far.
    pub fn height(&self) -> u32 {
        self.height.min(self.max_h)
    }

    /// The picture reached [`MAX_H`].
    pub fn full(&self) -> bool {
        self.height >= self.max_h
    }

    /// The last offset found.
    pub fn last_dy(&self) -> Option<u32> {
        self.last_dy
    }

    /// Add the next frame (same size as the first; another size is lost).
    pub fn push(&mut self, cur: Img) -> Step {
        if (cur.w, cur.h) != (self.w, self.h) || self.full() {
            return if self.full() { Step::Still } else { Step::Lost };
        }
        let cs = Sig::of(cur);
        let Some((pp, ps)) = self.prev.as_ref() else { return Step::Lost };
        let prev = Img { w: self.w, h: self.h, px: pp };
        let m = find(prev, ps, cur, &cs, self.last_dy);
        let Step::Moved(dy) = m.step else { return m.step };
        // Past the cap the picture is cut at the end ([`Stitcher::finish`]).
        let n = self.w as usize * 4;
        let from = (self.h - m.foot - dy) as usize;
        self.strips.push((cur.px[from * n..].to_vec(), dy, m.foot));
        self.foot = Some(self.foot.map_or(m.foot, |f| f.min(m.foot)));
        self.last_dy = Some(dy);
        self.height += dy;
        self.prev = Some((cur.px.to_vec(), cs));
        Step::Moved(dy)
    }

    /// The tall picture: the first frame down to the footer, what each
    /// later frame added, then the last frame's footer.
    pub fn finish(self) -> PixBuf {
        let (w, h) = (self.w, self.h);
        let n = w as usize * 4;
        let foot = self.foot.unwrap_or(0);
        let mut out = Vec::with_capacity(self.height.min(self.max_h) as usize * n);
        out.extend_from_slice(&self.first[..(h - foot) as usize * n]);
        for (strip, dy, f) in &self.strips {
            // The strip starts `f - foot` rows above the rows wanted.
            let skip = (f - foot) as usize * n;
            out.extend_from_slice(&strip[skip..skip + *dy as usize * n]);
        }
        let tail = match self.strips.last() {
            Some((strip, _, _)) => &strip[strip.len() - foot as usize * n..],
            None => &self.first[(h - foot) as usize * n..],
        };
        out.extend_from_slice(tail);
        out.truncate(self.max_h as usize * n);
        let rows = (out.len() / n.max(1)) as u32;
        PixBuf::from_raw(w, rows, out).unwrap_or_else(|| PixBuf::new(w.max(1), 1))
    }
}

#[cfg(test)]
#[path = "stitch_tests.rs"]
mod tests;
