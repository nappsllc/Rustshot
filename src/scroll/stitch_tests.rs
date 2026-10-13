use super::*;

/// A deterministic xorshift.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A page `w` wide and `h` tall like a document: lines of "text" (dark
/// blocks of varying width on white), blank gaps, the odd colored rule.
fn page(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed | 1);
    let mut px = vec![255u8; (w * h * 4) as usize];
    let mut y = 4;
    while y < h {
        let line = 10 + r.below(8) as u32;
        let kind = r.below(10);
        let mut x = 8 + r.below(30) as u32;
        while x + 4 < w {
            let word = 6 + r.below(40) as u32;
            let shade = if kind == 0 { [40, 90, 200] } else { [30, 30, 30] };
            for yy in y..(y + line).min(h) {
                for xx in x..(x + word).min(w - 4) {
                    // Glyph-like texture inside the word.
                    if (xx * 7 + yy * 3 + (x ^ y)) % 5 < 3 {
                        let i = ((yy * w + xx) * 4) as usize;
                        px[i..i + 3].copy_from_slice(&shade);
                    }
                }
            }
            x += word + 5 + r.below(10) as u32;
        }
        y += line + 4 + r.below(14) as u32;
    }
    px
}

/// The frame showing page rows from `o` in a `fh`-tall view with a fixed
/// `head` and `foot` (rows of their own pattern).
fn frame(pg: &[u8], w: u32, fh: u32, o: u32, head: u32, foot: u32) -> Vec<u8> {
    let n = (w * 4) as usize;
    let zone = fh - head - foot;
    let mut out = Vec::with_capacity(n * fh as usize);
    for y in 0..head {
        out.extend((0..w).flat_map(|x| [20, 120 + (y % 8) as u8 * 10, (x % 200) as u8, 255]));
    }
    out.extend_from_slice(&pg[o as usize * n..(o + zone) as usize * n]);
    for y in 0..foot {
        out.extend((0..w).flat_map(|x| [200, (x % 100) as u8 + 50, 60 + (y % 4) as u8 * 30, 255]));
    }
    out
}

fn img(px: &[u8], w: u32) -> Img<'_> {
    Img { w, h: (px.len() / (w as usize * 4)) as u32, px }
}

/// Stitch frames at `offsets` (each a view of the page); returns the
/// steps and the picture.
fn stitch(frames: &[Vec<u8>], w: u32) -> (Vec<Step>, PixBuf) {
    let mut s = Stitcher::new(img(&frames[0], w));
    let steps = frames[1..].iter().map(|f| s.push(img(f, w))).collect();
    (steps, s.finish())
}

#[test]
fn plain_page_is_stitched_whole() {
    let (w, fh) = (200, 150);
    let pg = page(w, 900, 7);
    let offsets = [0, 60, 120, 180, 240, 300];
    let frames: Vec<_> = offsets.iter().map(|&o| frame(&pg, w, fh, o, 0, 0)).collect();
    let (steps, out) = stitch(&frames, w);
    assert_eq!(steps, vec![Step::Moved(60); 5]);
    assert_eq!(out.dimensions(), (w, 300 + fh));
    assert_eq!(out.as_raw()[..], pg[..((300 + fh) * w * 4) as usize], "the page's rows, in order");
}

#[test]
fn sticky_header_and_footer_come_once() {
    let (w, fh, head, foot) = (240, 200, 30, 20);
    let pg = page(w, 1200, 11);
    let offsets = [0, 70, 140, 210, 280];
    let frames: Vec<_> = offsets.iter().map(|&o| frame(&pg, w, fh, o, head, foot)).collect();
    let (steps, out) = stitch(&frames, w);
    assert_eq!(steps, vec![Step::Moved(70); 4]);
    let zone = fh - head - foot;
    let n = (w * 4) as usize;
    assert_eq!(out.height(), head + 280 + zone + foot);
    let raw = out.as_raw();
    // Header, then the page from its top, then the footer, once each.
    assert_eq!(raw[..head as usize * n], frames[0][..head as usize * n]);
    let body = &raw[head as usize * n..(head + 280 + zone) as usize * n];
    assert_eq!(body, &pg[..(280 + zone) as usize * n]);
    assert_eq!(raw[(out.height() - foot) as usize * n..], frames[4][(fh - foot) as usize * n..]);
}

#[test]
fn uneven_steps_and_noise_are_followed() {
    let (w, fh) = (220, 160);
    let pg = page(w, 1000, 23);
    let offsets = [0u32, 37, 89, 130, 193, 222, 270];
    let mut r = Rng(99);
    let frames: Vec<_> = offsets
        .iter()
        .map(|&o| {
            let mut f = frame(&pg, w, fh, o, 0, 0);
            // Capture noise: every channel off by up to 3.
            for v in f.iter_mut() {
                let d = r.below(7) as i32 - 3;
                *v = (*v as i32 + d).clamp(0, 255) as u8;
            }
            f
        })
        .collect();
    let (steps, out) = stitch(&frames, w);
    let want: Vec<Step> = offsets.windows(2).map(|p| Step::Moved(p[1] - p[0])).collect();
    assert_eq!(steps, want);
    assert_eq!(out.height(), 270 + fh);
}

#[test]
fn an_animation_in_the_zone_does_not_throw_it_off() {
    let (w, fh) = (240, 180);
    let pg = page(w, 1000, 5);
    let offsets = [0u32, 50, 100, 150, 200];
    let (bx, by, bs) = (180u32, 60u32, 24u32);
    let frames: Vec<_> = offsets
        .iter()
        .enumerate()
        .map(|(k, &o)| {
            let mut f = frame(&pg, w, fh, o, 0, 0);
            // A spinner at a fixed place on screen: another color each frame.
            for y in by..by + bs {
                for x in bx..bx + bs {
                    let i = ((y * w + x) * 4) as usize;
                    f[i..i + 3].copy_from_slice(&[(k * 60) as u8, 200 - (k * 40) as u8, 90]);
                }
            }
            f
        })
        .collect();
    let (steps, out) = stitch(&frames, w);
    assert_eq!(steps, vec![Step::Moved(50); 4]);
    assert_eq!(out.height(), 200 + fh);
    // Outside the spinner's columns the page is exact.
    let n = (w * 4) as usize;
    for y in 0..out.height() {
        let row = &out.as_raw()[y as usize * n..(y as usize + 1) * n];
        let want = &pg[y as usize * n..(y as usize + 1) * n];
        assert_eq!(row[..bx as usize * 4], want[..bx as usize * 4], "row {y}");
    }
}

#[test]
fn the_end_of_the_content_is_still() {
    let (w, fh) = (200, 150);
    let pg = page(w, 600, 3);
    let a = frame(&pg, w, fh, 100, 0, 0);
    let mut s = Stitcher::new(img(&a, w));
    assert_eq!(s.push(img(&a, w)), Step::Still);
    // Still with a caret blinking somewhere.
    let mut b = a.clone();
    for x in 50..52 {
        for y in 40..56 {
            let i = ((y * w + x) * 4) as usize;
            b[i..i + 3].copy_from_slice(&[0, 0, 0]);
        }
    }
    assert_eq!(s.push(img(&b, w)), Step::Still);
    assert_eq!(s.height(), fh, "nothing added");
}

#[test]
fn unrelated_frames_are_lost() {
    let (w, fh) = (200, 150);
    let a = frame(&page(w, 400, 1), w, fh, 0, 0, 0);
    let b = frame(&page(w, 400, 77), w, fh, 0, 0, 0);
    let mut s = Stitcher::new(img(&a, w));
    assert_eq!(s.push(img(&b, w)), Step::Lost);
    // Another size is lost too.
    let c = frame(&page(w + 10, 400, 1), w + 10, fh, 0, 0, 0);
    assert_eq!(s.push(img(&c, w + 10)), Step::Lost);
    assert_eq!(s.finish().dimensions(), (w, fh), "the first frame stays");
}

#[test]
fn offset_edges() {
    let (w, fh) = (200, 160);
    let pg = page(w, 800, 13);
    let base = frame(&pg, w, fh, 100, 0, 0);
    let bs = Sig::of(img(&base, w));
    let at = |o: u32| {
        let f = frame(&pg, w, fh, o, 0, 0);
        let s = Sig::of(img(&f, w));
        find(img(&base, w), &bs, img(&f, w), &s, None).step
    };
    assert_eq!(at(101), Step::Moved(1), "one px");
    assert_eq!(at(100 + fh / 2), Step::Moved(fh / 2), "half the frame");
    assert_eq!(at(100 + fh * 3 / 4), Step::Moved(fh * 3 / 4), "three quarters");
    assert!(!matches!(at(100 + fh - 4), Step::Moved(_)), "a step past the overlap needed");
    // A blank page has nothing to match.
    let blank = vec![255u8; (w * fh * 4) as usize];
    let mut moved = blank.clone();
    moved[(10 * w * 4) as usize] = 0; // one dark pixel: still blank rows
    let (s1, s2) = (Sig::of(img(&blank, w)), Sig::of(img(&moved, w)));
    assert_ne!(find(img(&blank, w), &s1, img(&moved, w), &s2, None).step, Step::Moved(0));
}

#[test]
fn a_repeating_pattern_takes_the_expected_step() {
    // Stripes every 40 px: offsets 40 apart look the same.
    let (w, fh) = (160, 200);
    let mut pg = vec![255u8; (w * 1200 * 4) as usize];
    for y in 0..1200u32 {
        if y % 40 < 12 {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                pg[i..i + 3].copy_from_slice(&[(x % 50) as u8 * 4, 40, 40]);
            }
        }
    }
    // Half a period on: 20, 60, 100 px all match; the expected step wins.
    let a = frame(&pg, w, fh, 0, 0, 0);
    let b = frame(&pg, w, fh, 20, 0, 0);
    let (sa, sb) = (Sig::of(img(&a, w)), Sig::of(img(&b, w)));
    for e in [20, 60, 100] {
        assert_eq!(find(img(&a, w), &sa, img(&b, w), &sb, Some(e)).step, Step::Moved(e), "expected {e}");
    }
}

#[test]
fn the_picture_stops_at_the_cap() {
    let (w, fh) = (64, 400);
    let pg = page(w, MAX_H + 2000, 17);
    let mut s = Stitcher::new(img(&frame(&pg, w, fh, 0, 0, 0), w));
    let mut o = 0;
    while !s.full() {
        o += 300;
        assert!(matches!(s.push(img(&frame(&pg, w, fh, o, 0, 0), w)), Step::Moved(_)), "at {o}");
    }
    assert_eq!(s.height(), MAX_H);
    assert_eq!(s.push(img(&frame(&pg, w, fh, o + 300, 0, 0), w)), Step::Still, "full: nothing more");
    let out = s.finish();
    assert_eq!(out.height(), MAX_H);
    assert_eq!(out.as_raw()[..], pg[..(MAX_H * w * 4) as usize]);
}

/// Perf: 20 frames of 1080p stitched in under a second (release build:
/// `cargo test --release stitch_perf -- --ignored --nocapture`).
#[test]
#[ignore = "benchmark"]
fn stitch_perf_1080p() {
    let (w, fh) = (1920, 1080);
    let pg = page(w, 1080 + 19 * 400, 31);
    let frames: Vec<_> = (0..20).map(|k| frame(&pg, w, fh, k * 400, 60, 40)).collect();
    let t = std::time::Instant::now();
    let (steps, out) = stitch(&frames, w);
    let took = t.elapsed();
    println!("stitched 20 frames of 1080p in {took:?} -> {}x{}", out.width(), out.height());
    assert!(steps.iter().all(|s| *s == Step::Moved(400)), "{steps:?}");
    if !cfg!(debug_assertions) {
        assert!(took < std::time::Duration::from_secs(1), "{took:?}");
    }
}
