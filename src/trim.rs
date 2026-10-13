//! Trim to content: the rect inside an image that is left once the uniform
//! margins around it are cut away (the overlay's "Trim to content", Shift+A
//! by default, and Settings › Capture "Trim margins automatically").
//!
//! Each edge is scanned inward while its row (or column) stays within
//! [`TOL`] per channel of the edge's dominant color (the per-channel median
//! of the outermost line on that side, the corners the top and bottom
//! margins took left out) for all but [`MATCH`]'s 2 % of its pixels (at
//! least [`MIN_MISS`] of them), so JPEG-like noise and a stray pixel or two
//! still count as margin. Top and bottom first, then left and right over
//! the rows left: one pass, so a uniform panel inside the margin (a dialog
//! on a page) is kept whole. [`PAD`] pixels of the margin are kept on
//! every trimmed side. A solid block of one color flush with a side and
//! as tall as what is left is taken for margin there (nothing tells them
//! apart). Pure: tested on synthetic images everywhere.

use crate::pixbuf::PixBuf;

/// Per-channel difference a margin pixel may have from the edge's color.
pub const TOL: u8 = 6;
/// The share of a line's pixels that must match for it to be margin.
pub const MATCH: f32 = 0.98;
/// Off pixels any line of 5 px or more may have and still be margin.
pub const MIN_MISS: usize = 2;
/// Margin kept around the content on each trimmed side (px).
pub const PAD: u32 = 2;

/// A rect in image pixels: x, y, w, h.
pub type Rect = (u32, u32, u32, u32);

struct View<'a> {
    d: &'a [u8],
    w: usize,
}

impl View<'_> {
    fn px(&self, x: u32, y: u32) -> [u8; 3] {
        let i = (y as usize * self.w + x as usize) * 4;
        [self.d[i], self.d[i + 1], self.d[i + 2]]
    }
}

/// The per-channel median of `n` pixels.
fn median(n: usize, px: impl Iterator<Item = [u8; 3]>) -> [u8; 3] {
    let mut hist = [[0u32; 256]; 3];
    for p in px {
        for c in 0..3 {
            hist[c][p[c] as usize] += 1;
        }
    }
    let half = n.div_ceil(2) as u32;
    std::array::from_fn(|c| {
        let mut acc = 0;
        for (v, k) in hist[c].iter().enumerate() {
            acc += k;
            if acc >= half {
                return v as u8;
            }
        }
        255
    })
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    (0..3).all(|c| a[c].abs_diff(b[c]) <= TOL)
}

/// Whether enough of the `n` pixels are near `c`.
fn matches(n: usize, c: [u8; 3], px: impl Iterator<Item = [u8; 3]>) -> bool {
    let need = (n as f32 * MATCH).ceil() as usize;
    let miss_max = (n - need.min(n)).max(if n >= 5 { MIN_MISS } else { 0 });
    let mut miss = 0;
    for p in px {
        if !near(p, c) {
            miss += 1;
            if miss > miss_max {
                return false;
            }
        }
    }
    true
}

/// The content rect of `img` (its uniform margins cut, [`PAD`] kept);
/// `None` when there is nothing to trim or nothing but margin.
pub fn content(img: &PixBuf) -> Option<Rect> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let v = View { d: img.as_raw(), w: w as usize };
    // Half-open bounds of what is left.
    let (mut l, mut t, mut r, mut b) = (0u32, 0u32, w, h);
    let row = |y: u32, l: u32, r: u32| (l..r).map(move |x| (x, y));
    let col = |x: u32, t: u32, b: u32| (t..b).map(move |y| (x, y));
    {
        // Top.
        let n = (r - l) as usize;
        let c = median(n, row(t, l, r).map(|(x, y)| v.px(x, y)));
        while t < b && matches(n, c, row(t, l, r).map(|(x, y)| v.px(x, y))) {
            t += 1;
        }
        if t == b {
            return None; // all margin
        }
        // Bottom.
        let c = median(n, row(b - 1, l, r).map(|(x, y)| v.px(x, y)));
        while b > t && matches(n, c, row(b - 1, l, r).map(|(x, y)| v.px(x, y))) {
            b -= 1;
        }
        // Left: its color from the rows left (the corners, the top and
        // bottom margins' own, left out).
        let n = (b - t) as usize;
        let c = median(n, col(l, t, b).map(|(x, y)| v.px(x, y)));
        while l < r && matches(n, c, col(l, t, b).map(|(x, y)| v.px(x, y))) {
            l += 1;
        }
        if l == r {
            return None;
        }
        // Right.
        let c = median(n, col(r - 1, t, b).map(|(x, y)| v.px(x, y)));
        while r > l && matches(n, c, col(r - 1, t, b).map(|(x, y)| v.px(x, y))) {
            r -= 1;
        }
    }
    let (l, t) = (l.saturating_sub(PAD), t.saturating_sub(PAD));
    let (r, b) = ((r + PAD).min(w), (b + PAD).min(h));
    ((l, t, r, b) != (0, 0, w, h)).then_some((l, t, r - l, b - t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(img: &mut PixBuf, (x, y, w, h): Rect, c: [u8; 4]) {
        let iw = img.width() as usize;
        let d = img.as_raw_mut();
        for yy in y..y + h {
            for xx in x..x + w {
                let i = (yy as usize * iw + xx as usize) * 4;
                d[i..i + 4].copy_from_slice(&c);
            }
        }
    }

    /// A deterministic pseudo-random sequence (xorshift).
    fn rng(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    #[test]
    fn solid_margins_are_cut_keeping_the_padding() {
        let mut img = PixBuf::from_pixel(200, 120, [255, 255, 255, 255]);
        fill(&mut img, (40, 30, 100, 50), [20, 90, 200, 255]);
        assert_eq!(content(&img), Some((38, 28, 104, 54)));
        // Each edge has its own color: black above, white below, gray at
        // the sides; the gray between the bands and the red is the sides'.
        let mut img = PixBuf::from_pixel(100, 100, [128, 128, 128, 255]);
        fill(&mut img, (0, 0, 100, 10), [0, 0, 0, 255]);
        fill(&mut img, (0, 90, 100, 10), [250, 250, 250, 255]);
        fill(&mut img, (30, 40, 20, 20), [255, 0, 0, 255]);
        assert_eq!(content(&img), Some((28, 8, 24, 84)));
    }

    #[test]
    fn content_at_an_edge_keeps_that_edge() {
        // Content touching the left side (text-like: not one color down
        // its edge column).
        let mut img = PixBuf::from_pixel(60, 40, [0, 0, 0, 255]);
        fill(&mut img, (0, 10, 30, 20), [255, 255, 0, 255]);
        for y in (10..30).step_by(3) {
            fill(&mut img, (0, y, 2, 1), [0, 0, 255, 255]);
        }
        assert_eq!(content(&img), Some((0, 8, 32, 24)));
        // Tall white bands above and below, gray at the sides: the sides'
        // color comes from between the bands, so they are cut too.
        let mut img = PixBuf::from_pixel(100, 200, [128, 128, 128, 255]);
        fill(&mut img, (0, 0, 100, 80), [255, 255, 255, 255]);
        fill(&mut img, (0, 120, 100, 80), [255, 255, 255, 255]);
        fill(&mut img, (40, 80, 20, 40), [255, 0, 0, 255]);
        assert_eq!(content(&img), Some((38, 78, 24, 44)));
        // Less margin than the padding on a side: that side stays.
        let mut img = PixBuf::from_pixel(60, 40, [0, 0, 0, 255]);
        fill(&mut img, (1, 1, 30, 20), [255, 255, 0, 255]);
        assert_eq!(content(&img), Some((0, 0, 33, 23)));
    }

    #[test]
    fn a_gradient_is_content() {
        // Top to bottom, one level a row: only the first rows within the
        // tolerance of each edge's color are margin.
        let (w, h) = (64u32, 256u32);
        let mut img = PixBuf::new(w, h);
        for y in 0..h {
            fill(&mut img, (0, y, w, 1), [y as u8, y as u8, y as u8, 255]);
        }
        let (x, y, cw, ch) = content(&img).expect("the flat ends trim");
        assert_eq!((x, cw), (0, w), "columns are not uniform");
        let cut = TOL as u32 + 1 - PAD;
        assert_eq!((y, ch), (cut, h - 2 * cut));
        // Left to right: the same on the columns.
        let mut img = PixBuf::new(h, w);
        for x in 0..h {
            fill(&mut img, (x, 0, 1, w), [0, x as u8, 255 - x as u8, 255]);
        }
        assert_eq!(content(&img), Some((cut, 0, h - 2 * cut, w)));
    }

    #[test]
    fn noisy_jpeg_like_borders_still_trim() {
        let (w, h) = (300u32, 200u32);
        let mut img = PixBuf::new(w, h);
        let mut seed = 0x1234_5678u32;
        let d = img.as_raw_mut();
        for (i, px) in d.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % w as usize, i / w as usize);
            let n = rng(&mut seed);
            // ±3 noise around (200, 210, 220), and far-off speckles spread
            // so that no line has more than 2 % of them.
            let speck = (x * 7 + y * 13) % 211 == 0;
            let v = if speck { [0, 0, 0] } else { [200, 210, 220].map(|c: u8| (c as i32 + (n >> 8) as i32 % 7 - 3) as u8) };
            px.copy_from_slice(&[v[0], v[1], v[2], 255]);
        }
        fill(&mut img, (50, 60, 120, 70), [10, 10, 10, 255]);
        fill(&mut img, (60, 70, 100, 50), [240, 30, 30, 255]);
        assert_eq!(content(&img), Some((48, 58, 124, 74)));
    }

    /// A short side tolerates two stray pixels (2 % of it would be none).
    #[test]
    fn short_lines_allow_two_stray_pixels() {
        let mut img = PixBuf::from_pixel(200, 30, [255, 255, 255, 255]);
        fill(&mut img, (20, 5, 160, 20), [0, 128, 0, 255]);
        fill(&mut img, (3, 7, 1, 1), [0, 0, 0, 255]);
        fill(&mut img, (5, 20, 1, 1), [0, 0, 0, 255]);
        assert_eq!(content(&img), Some((18, 3, 164, 24)));
        // Three: that column is content.
        fill(&mut img, (5, 12, 1, 1), [0, 0, 0, 255]);
        fill(&mut img, (5, 15, 1, 1), [0, 0, 0, 255]);
        assert_eq!(content(&img).map(|r| r.0), Some(3));
    }

    #[test]
    fn nothing_to_trim_or_all_uniform_is_none() {
        // Content touching every edge.
        let mut img = PixBuf::new(50, 50);
        let mut seed = 7u32;
        for px in img.as_raw_mut().as_chunks_mut::<4>().0 {
            let n = rng(&mut seed);
            px.copy_from_slice(&[n as u8, (n >> 8) as u8, (n >> 16) as u8, 255]);
        }
        assert_eq!(content(&img), None);
        // One flat color: no content to keep.
        assert_eq!(content(&PixBuf::from_pixel(80, 30, [9, 9, 9, 255])), None);
        // Uniform but for a few speckles (under 2 % of every line): margin.
        let mut img = PixBuf::from_pixel(100, 100, [50, 50, 50, 255]);
        fill(&mut img, (10, 10, 1, 1), [255, 255, 255, 255]);
        assert_eq!(content(&img), None);
        // A margin of 1 px (under the padding) is nothing to trim.
        let mut img = PixBuf::from_pixel(20, 20, [0, 0, 0, 255]);
        fill(&mut img, (1, 1, 18, 18), [255, 255, 255, 255]);
        let mut seed = 3u32;
        let d = img.as_raw_mut();
        for y in 1..19usize {
            for x in 1..19usize {
                let n = rng(&mut seed) as u8;
                d[(y * 20 + x) * 4] = n;
            }
        }
        assert_eq!(content(&img), None);
        assert_eq!(content(&PixBuf::from_pixel(1, 1, [0, 0, 0, 255])), None);
    }

    #[test]
    fn a_uniform_panel_inside_the_margin_is_kept() {
        // A gray dialog on a white page: the page goes, the dialog's own
        // background stays (one pass).
        let mut img = PixBuf::from_pixel(100, 100, [255, 255, 255, 255]);
        fill(&mut img, (20, 20, 60, 60), [128, 128, 128, 255]);
        fill(&mut img, (40, 40, 20, 20), [0, 0, 255, 255]);
        assert_eq!(content(&img), Some((18, 18, 64, 64)));
    }
}
