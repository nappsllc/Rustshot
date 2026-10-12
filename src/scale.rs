//! Image downscaling for thumbnails (History) and the Edit window's view
//! below 100 %: a box filter, each destination pixel the average of the
//! source pixels its area covers (whole pixels: the span from
//! `floor(x * k)` to `floor((x + 1) * k)`, at least one).

use crate::pixbuf::PixBuf;

/// The size `w`×`h` fits in `max_w`×`max_h` with its aspect ratio kept,
/// never enlarged (at least 1×1).
pub fn fit_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let (fw, fh) = (w.max(1) as f64, h.max(1) as f64);
    let k = (max_w as f64 / fw).min(max_h as f64 / fh).min(1.0);
    (((fw * k).round() as u32).clamp(1, max_w.max(1)), ((fh * k).round() as u32).clamp(1, max_h.max(1)))
}

/// `src` scaled to fit inside `max_w`×`max_h` ([`fit_size`]), box-filtered.
pub fn fit(src: &PixBuf, max_w: u32, max_h: u32) -> PixBuf {
    let (dw, dh) = fit_size(src.width(), src.height(), max_w, max_h);
    resize(src, dw, dh)
}

/// `src` box-filtered to exactly `dw`×`dh` (no larger than `src`; a larger
/// size repeats source pixels).
pub fn resize(src: &PixBuf, dw: u32, dh: u32) -> PixBuf {
    let (dw, dh) = (dw.max(1), dh.max(1));
    let mut out = PixBuf::new(dw, dh);
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    if sw == 0 || sh == 0 {
        return out;
    }
    box_rect(src.as_raw(), sw, sh, out.as_raw_mut(), dw as usize, dh as usize, (0, 0, dw as usize, dh as usize));
    out
}

/// The source span of destination index `i` when `n` source pixels map
/// onto `d` destination ones: `[a, b)`, never empty, inside `0..n`.
pub fn span(i: usize, n: usize, d: usize) -> (usize, usize) {
    let a = (i * n / d).min(n - 1);
    let b = ((i + 1) * n / d).clamp(a + 1, n);
    (a, b)
}

/// Destination rect `r` = (x0, y0, x1, y1) of `src` (`sw`×`sh` RGBA)
/// scaled onto `dst` (`dw`×`dh` RGBA), box-filtered: each destination
/// pixel inside `r` is rewritten, the others are left alone.
pub fn box_rect(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], dw: usize, dh: usize, r: (usize, usize, usize, usize)) {
    let (x0, y0) = (r.0.min(dw), r.1.min(dh));
    let (x1, y1) = (r.2.min(dw), r.3.min(dh));
    if x1 <= x0 || y1 <= y0 || sw == 0 || sh == 0 {
        return;
    }
    let n = x1 - x0;
    let spans: Vec<(usize, usize)> = (x0..x1).map(|x| span(x, sw, dw)).collect();
    let mut acc = vec![0u32; n * 4];
    for y in y0..y1 {
        let (sy0, sy1) = span(y, sh, dh);
        acc.fill(0);
        for sy in sy0..sy1 {
            let row = &src[sy * sw * 4..(sy + 1) * sw * 4];
            for (i, &(a, b)) in spans.iter().enumerate() {
                let o = &mut acc[i * 4..i * 4 + 4];
                for px in row[a * 4..b * 4].as_chunks::<4>().0 {
                    o[0] += px[0] as u32;
                    o[1] += px[1] as u32;
                    o[2] += px[2] as u32;
                    o[3] += px[3] as u32;
                }
            }
        }
        let rows = (sy1 - sy0) as u32;
        let out = &mut dst[(y * dw + x0) * 4..(y * dw + x1) * 4];
        for (i, &(a, b)) in spans.iter().enumerate() {
            let cnt = rows * (b - a) as u32;
            let half = cnt / 2;
            for c in 0..4 {
                out[i * 4 + c] = ((acc[i * 4 + c] + half) / cnt) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_aspect_and_never_enlarges() {
        assert_eq!(fit_size(1920, 1080, 200, 140), (200, 113));
        assert_eq!(fit_size(1080, 1920, 200, 140), (79, 140));
        assert_eq!(fit_size(100, 50, 200, 140), (100, 50));
        assert_eq!(fit_size(10000, 1, 200, 140), (200, 1));
        assert_eq!(fit_size(0, 0, 200, 140), (1, 1));
    }

    #[test]
    fn spans_cover_the_source_once() {
        for (n, d) in [(10usize, 3usize), (3840, 1131), (7, 7), (5, 9)] {
            let mut next = 0;
            for i in 0..d {
                let (a, b) = span(i, n, d);
                assert!(a < b && b <= n, "{n}->{d}: {i}");
                if n >= d {
                    assert_eq!(a, next, "{n}->{d}: contiguous");
                    next = b;
                }
            }
            if n >= d {
                assert_eq!(next, n);
            }
        }
    }

    #[test]
    fn box_filter_averages() {
        // 4x2: left half black, right half white, scaled to 2x1.
        let mut src = PixBuf::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                let v = if x >= 2 { 255 } else { 0 };
                let i = (y * 4 + x) * 4;
                src.as_raw_mut()[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let out = resize(&src, 2, 1);
        assert_eq!(out.get_pixel(0, 0), [0, 0, 0, 255]);
        assert_eq!(out.get_pixel(1, 0), [255, 255, 255, 255]);
        let out = resize(&src, 1, 1);
        assert_eq!(out.get_pixel(0, 0), [128, 128, 128, 255]);
        // Only the rect is written.
        let mut dst = vec![7u8; 8];
        box_rect(src.as_raw(), 4, 2, &mut dst, 2, 1, (1, 0, 2, 1));
        assert_eq!(&dst[..4], &[7, 7, 7, 7]);
        assert_eq!(&dst[4..], &[255, 255, 255, 255]);
    }
}
