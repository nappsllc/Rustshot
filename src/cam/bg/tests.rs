use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[test]
fn config_values_round_trip() {
    for b in Bg::ALL {
        assert_eq!(Bg::from_config(b.key()), b);
    }
    assert_eq!(Bg::ALL.map(Bg::key), ["off", "light", "strong"]);
    assert_eq!(Bg::from_config("Strong "), Bg::Strong);
    assert_eq!(Bg::from_config("blurry"), Bg::Off);
    assert_eq!(Bg::default(), Bg::Off);
    assert_eq!((Bg::Light.sigma(), Bg::Strong.sigma()), (8.0, 18.0));
    for b in Bg::ALL {
        assert_eq!(Bg::from_u8(b.to_u8()), b);
    }
    assert_eq!(Bg::from_u8(9), Bg::Off);
}

#[test]
fn pacer_halves_the_rate_over_budget_and_never_queues() {
    let mut p = Pacer::default();
    assert_eq!(p.every(), 1);
    assert!((0..5).all(|_| p.want(false)), "every picture while there is no timing yet");
    assert!(!p.want(true), "not while the last one runs");
    p.took(4.0);
    assert!((0..5).all(|_| p.want(false)), "within budget: every picture");
    for _ in 0..10 {
        p.took(12.0);
    }
    assert_eq!(p.every(), 2);
    let picks: Vec<bool> = (0..6).map(|_| p.want(false)).collect();
    assert_eq!(picks.iter().filter(|&&b| b).count(), 3, "every other picture: {picks:?}");
    assert!(picks.windows(2).all(|w| w[0] != w[1]), "alternating: {picks:?}");
    // Faster again: back to every picture.
    for _ in 0..20 {
        p.took(2.0);
    }
    assert_eq!(p.every(), 1);
    assert!(p.avg_ms().unwrap() < BUDGET_MS);
}

#[test]
fn masks_are_smoothed_over_time() {
    let mut s = Smooth::default();
    assert!(!s.has());
    s.push(&SqMask { n: 2, v: vec![1.0; 4] });
    assert_eq!(s.m.v, vec![1.0; 4], "the first mask as it is");
    s.push(&SqMask { n: 2, v: vec![0.0; 4] });
    assert!(s.m.v.iter().all(|v| (v - (1.0 - EMA)).abs() < 1e-6), "{:?}", s.m.v);
    s.push(&SqMask { n: 2, v: vec![0.0; 4] });
    assert!(s.m.v.iter().all(|v| (v - (1.0 - EMA).powi(2)).abs() < 1e-6));
    // A flicker of one frame moves the mask by 60 %, not all the way.
    let mut f = Smooth::default();
    f.push(&SqMask { n: 1, v: vec![1.0] });
    f.push(&SqMask { n: 1, v: vec![0.0] });
    f.push(&SqMask { n: 1, v: vec![1.0] });
    assert!((f.m.v[0] - (EMA + (1.0 - EMA) * (1.0 - EMA))).abs() < 1e-6);
    // Another size starts over.
    s.push(&SqMask { n: 3, v: vec![0.5; 9] });
    assert_eq!((s.m.n, s.m.v.clone()), (3, vec![0.5; 9]));
    s.clear();
    assert!(!s.has());
}

#[test]
fn box_mean_matches_the_window_average() {
    let n = 9;
    let src: Vec<f32> = (0..n * n).map(|i| ((i * 37) % 11) as f32).collect();
    for r in [0usize, 1, 2, 4, 12] {
        let (mut out, mut tmp) = (Vec::new(), Vec::new());
        box_mean(&src, n, r, &mut out, &mut tmp);
        for y in 0..n {
            for x in 0..n {
                let (mut s, mut c) = (0.0, 0);
                for yy in y.saturating_sub(r)..=(y + r).min(n - 1) {
                    for xx in x.saturating_sub(r)..=(x + r).min(n - 1) {
                        s += src[yy * n + xx];
                        c += 1;
                    }
                }
                assert!((out[y * n + x] - s / c as f32).abs() < 1e-4, "r {r} ({x}, {y})");
            }
        }
    }
}

/// A `d`x`d` picture: `left` color left of `edge`, `right` from it.
fn split_pic(d: u32, edge: u32, left: [u8; 3], right: [u8; 3]) -> CamFrame {
    let mut bgra = Vec::with_capacity((d * d * 4) as usize);
    for _ in 0..d {
        for x in 0..d {
            let c = if x < edge { left } else { right };
            bgra.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    CamFrame { w: d, h: d, bgra }
}

#[test]
fn refining_follows_the_pictures_edge() {
    let d = 160u32;
    let n = 40usize;
    // The person (bright) on the left half; the coarse mask's edge is a
    // soft ramp (four low pixels wide) around it.
    let pic = split_pic(d, d / 2, [230, 220, 210], [30, 40, 50]);
    let ramp = |x: f32| (0.5 - (x - 0.5 * n as f32) / 4.0).clamp(0.0, 1.0);
    let m = SqMask { n, v: (0..n * n).map(|i| ramp((i % n) as f32 + 0.5)).collect() };
    let mut out = Vec::new();
    let mut s = Scratch::default();
    refine(&m, &pic, &mut out, &mut s);
    assert_eq!(out.len(), (d * d) as usize);
    let at = |x: u32| out[(d / 2 * d + x) as usize] as f32 / 255.0;
    let soft = |x: u32| ramp((x as f32 + 0.5) * n as f32 / d as f32);
    // Sharper, on the picture's edge: more person just inside it, less
    // just outside than the bilinear mask has.
    for k in [1u32, 2, 4, 8] {
        let (i, o) = (d / 2 - k, d / 2 + k - 1);
        let gain = if k <= 4 { 0.1 } else { 0.0 };
        assert!(at(i) > 0.9 && at(i) >= soft(i) + gain, "{k} px inside: {} (coarse {})", at(i), soft(i));
        assert!(at(o) < 0.1 && at(o) <= soft(o) - gain, "{k} px outside: {} (coarse {})", at(o), soft(o));
    }
    assert!(at(0) > 0.96 && at(d - 1) < 0.04);
    // A flat picture gives no edge to follow: the mask stays where it was
    // (smoothed).
    let flat = split_pic(d, 0, [90, 90, 90], [90, 90, 90]);
    refine(&m, &flat, &mut out, &mut s);
    let at = |x: u32| out[(d / 2 * d + x) as usize] as f32 / 255.0;
    assert!(at(d / 2 - 8) > 0.6 && at(d / 2 + 8) < 0.4 && at(0) > 0.96 && at(d - 1) < 0.04, "{} {}", at(d / 2 - 8), at(d / 2 + 8));
    assert!((at(d / 2) - 0.5).abs() < 0.2, "the edge stays in the middle: {}", at(d / 2));
    // All person: all 255.
    refine(&SqMask { n, v: vec![1.0; n * n] }, &pic, &mut out, &mut s);
    assert!(out.iter().all(|&v| v == 255));
    // Nothing to refine.
    refine(&SqMask::default(), &pic, &mut out, &mut s);
    assert!(out.is_empty());
}

fn noise_pic(d: u32, seed: u32) -> CamFrame {
    let mut s = seed | 1;
    let bgra = (0..d * d)
        .flat_map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            [(s >> 8) as u8, (s >> 16) as u8, (s >> 24) as u8, 255]
        })
        .collect();
    CamFrame { w: d, h: d, bgra }
}

#[test]
fn composite_blurs_only_the_background() {
    let d = 64u32;
    let pic = noise_pic(d, 5);
    let mut s = Scratch::default();
    // All person: untouched.
    let mut p = pic.clone();
    composite(&mut p, &vec![255; (d * d) as usize], 8.0, &mut s);
    assert_eq!(p, pic);
    // All background: exactly the Blur tool's blur.
    let mut p = pic.clone();
    composite(&mut p, &vec![0; (d * d) as usize], 8.0, &mut s);
    let mut want = pic.bgra.clone();
    crate::objects::blur::blur_block(&mut want, d as usize, d as usize, 8.0, &mut Vec::new(), &mut Vec::new());
    assert_eq!(p.bgra, want);
    assert!(p.bgra.as_chunks::<4>().0.iter().all(|q| q[3] == 255), "opaque");
    // Sigma 0 (off): untouched.
    let mut p = pic.clone();
    composite(&mut p, &vec![0; (d * d) as usize], 0.0, &mut s);
    assert_eq!(p, pic);
}

/// A big bubble blurs at half size: what that gives is the full-size
/// blur within a few levels; the person stays exact.
#[test]
fn big_bubbles_blur_at_half_size() {
    let d = 300u32;
    assert!(d as usize >= HALF_FROM);
    // Smooth content (a camera's), not noise: gradients and blobs.
    let mut pic = CamFrame { w: d, h: d, bgra: Vec::new() };
    for y in 0..d {
        for x in 0..d {
            let (u, v) = (x as f32 / d as f32, y as f32 / d as f32);
            let c = [(u * 255.0) as u8, (v * 255.0) as u8, (((u * 9.0).sin() * (v * 7.0).cos() * 0.5 + 0.5) * 255.0) as u8];
            pic.bgra.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut s = Scratch::default();
    let mut half = pic.clone();
    composite(&mut half, &vec![0; (d * d) as usize], 18.0, &mut s);
    let mut full = pic.bgra.clone();
    crate::objects::blur::blur_block(&mut full, d as usize, d as usize, 18.0, &mut Vec::new(), &mut Vec::new());
    let diff: Vec<u32> = half.bgra.iter().zip(&full).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs()).collect();
    let mean = diff.iter().sum::<u32>() as f32 / diff.len() as f32;
    // (The edges differ most: the half picture clamps a pixel further out.)
    assert!(mean < 1.5 && *diff.iter().max().unwrap() <= 16, "mean {mean}, max {}", diff.iter().max().unwrap());
    let mut p = pic.clone();
    composite(&mut p, &vec![255; (d * d) as usize], 18.0, &mut s);
    assert_eq!(p, pic, "all person: untouched");
    // No halo at half size either.
    let pic = split_pic(d, d / 2, [255, 255, 255], [20, 20, 20]);
    let mask: Vec<u8> = (0..d * d).map(|i| if i % d < d / 2 { 255 } else { 0 }).collect();
    let mut p = pic.clone();
    composite(&mut p, &mask, 18.0, &mut s);
    let at = |x: u32| p.bgra[((d / 2 * d + x) * 4) as usize];
    assert!(at(d / 2 + 1) <= 22 && at(d / 2 - 1) == 255, "{} {}", at(d / 2 + 1), at(d / 2 - 1));
}

#[test]
fn no_halo_of_the_person_around_them() {
    // A white person on the left half of a dark background.
    let d = 96u32;
    let pic = split_pic(d, d / 2, [255, 255, 255], [20, 20, 20]);
    let mask: Vec<u8> = (0..d * d).map(|i| if i % d < d / 2 { 255 } else { 0 }).collect();
    let mut p = pic.clone();
    composite(&mut p, &mask, 18.0, &mut Scratch::default());
    let at = |f: &CamFrame, x: u32| f.bgra[((d / 2 * d + x) * 4) as usize];
    // Right next to the person the background stays dark (a plain blur
    // would spread the white into it).
    assert!(at(&p, d / 2 + 1) <= 21, "halo: {}", at(&p, d / 2 + 1));
    let mut plain = pic.bgra.clone();
    crate::objects::blur::blur_block(&mut plain, d as usize, d as usize, 18.0, &mut Vec::new(), &mut Vec::new());
    assert!(plain[((d / 2 * d + d / 2 + 1) * 4) as usize] > 100, "a plain blur glows");
    // The person stays sharp.
    assert_eq!(at(&p, d / 2 - 1), 255);
}

/// A segmenter that finds a disk in the middle of the square, counts its
/// calls and takes `delay` each.
struct Fake {
    calls: Arc<AtomicUsize>,
    delay: Duration,
}

impl Segment for Fake {
    fn input_size(&self) -> (usize, usize) {
        (256, 144)
    }
    fn segment(&mut self, input: &SegInput, out: &mut SqMask) -> bool {
        assert_eq!((input.w, input.h, input.rgba.len()), (256, 144, 256 * 144 * 4));
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        disk(36, out);
        true
    }
}

/// A disk of radius 0.3 (of the side) in an `n`x`n` mask.
fn disk(n: usize, out: &mut SqMask) {
    out.n = n;
    out.v.clear();
    for j in 0..n {
        for i in 0..n {
            let (x, y) = ((i as f32 + 0.5) / n as f32 - 0.5, (j as f32 + 0.5) / n as f32 - 0.5);
            out.v.push(if (x * x + y * y).sqrt() < 0.3 { 1.0 } else { 0.0 });
        }
    }
}

fn fake(delay: Duration) -> (Effect, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let mut once = Some(Fake { calls: c, delay });
    (Effect::with_maker(Box::new(move || once.take().map(|f| Box::new(f) as Box<dyn Segment>))), calls)
}

#[test]
fn effect_blurs_once_a_mask_came() {
    let (mut fx, calls) = fake(Duration::ZERO);
    let raw = noise_pic(320, 3);
    let raw = CamFrame { w: 320, h: 180, bgra: raw.bgra[..320 * 180 * 4].to_vec() };
    let sq = super::super::fit_square(&raw, 120);
    // Off: nothing happens, no thread.
    let mut p = sq.clone();
    fx.apply(Bg::Off, &raw, &mut p);
    assert_eq!((p == sq, fx.running()), (true, None));
    // The segmenter is made on its thread (the camera thread goes on);
    // then a picture goes out; until its mask is back it shows as is.
    let mut p = sq.clone();
    fx.apply(Bg::Strong, &raw, &mut p);
    assert!(p == sq, "no mask yet");
    fx.settle();
    assert_eq!(fx.running(), Some(true));
    let mut p = sq.clone();
    fx.apply(Bg::Strong, &raw, &mut p);
    assert!(p == sq || calls.load(Ordering::SeqCst) >= 1, "a mask shows only after a segmentation (the next one may already be running)");
    fx.settle();
    let mut p = sq.clone();
    fx.apply(Bg::Strong, &raw, &mut p);
    assert_eq!(fx.mask().n, 36);
    let px = |f: &CamFrame, x: u32, y: u32| f.bgra[((y * 120 + x) * 4) as usize..][..4].to_vec();
    assert_eq!(px(&p, 60, 60), px(&sq, 60, 60), "the person (the disk) is sharp");
    assert_ne!(px(&p, 3, 3), px(&sq, 3, 3), "the corner is blurred");
    // Off again: as is, and the old mask is gone.
    let mut p = sq.clone();
    fx.apply(Bg::Off, &raw, &mut p);
    assert!(p == sq && fx.mask().n == 0);
    // A mask made before Off is not used after On.
    let mut p = sq.clone();
    fx.apply(Bg::Strong, &raw, &mut p); // sends a picture
    fx.apply(Bg::Off, &raw, &mut p);
    fx.settle();
    let mut p = sq.clone();
    fx.apply(Bg::Strong, &raw, &mut p);
    assert_eq!(fx.mask().n, 0, "the old mask is dropped");
}

/// A NaN in one mask does not stay in the average.
#[test]
fn bad_mask_values_do_not_stick() {
    let mut s = Smooth::default();
    s.push(&SqMask { n: 2, v: vec![f32::NAN, 2.0, -1.0, 0.5] });
    assert_eq!(s.m.v, vec![0.0, 1.0, 0.0, 0.5]);
    s.push(&SqMask { n: 2, v: vec![1.0, f32::INFINITY, 1.0, 1.0] });
    assert!(s.m.v.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{:?}", s.m.v);
}

#[test]
fn without_a_segmenter_nothing_changes() {
    let mut fx = Effect::with_maker(Box::new(|| None));
    let raw = noise_pic(64, 1);
    let mut p = super::super::fit_square(&raw, 32);
    let was = p.clone();
    for _ in 0..3 {
        fx.apply(Bg::Light, &raw, &mut p);
        fx.settle();
    }
    assert_eq!((p, fx.running()), (was, Some(false)));
}

#[test]
fn slow_segmentation_runs_on_every_other_picture() {
    let (mut fx, calls) = fake(Duration::from_millis(12));
    let raw = CamFrame { w: 320, h: 180, bgra: vec![128; 320 * 180 * 4] };
    let sq = super::super::fit_square(&raw, 64);
    // Pictures at a steady rate, each after the last mask came: the
    // pacer learns it is over budget and skips every other one.
    for _ in 0..12 {
        let mut p = sq.clone();
        fx.apply(Bg::Light, &raw, &mut p);
        fx.settle();
    }
    assert_eq!(fx.pacer.every(), 2, "{:?}", fx.pacer.avg_ms());
    let before = calls.load(Ordering::SeqCst);
    for _ in 0..10 {
        let mut p = sq.clone();
        fx.apply(Bg::Light, &raw, &mut p);
        fx.settle();
    }
    // (One picture's mask is taken on the next: with settling, 5 of 10.)
    let n = calls.load(Ordering::SeqCst) - before;
    assert!((4..=6).contains(&n), "{n} of 10 pictures segmented");
}

/// The camera thread's cost per picture (the bubble at 320 and 480 px,
/// a 720p frame): preparing the segmenter's input, refining the mask,
/// the blurred composite.
/// `cargo test --release bg_frame_bench -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn bg_frame_bench() {
    // The best of 30 (the machine may be busy).
    let best = |f: &mut dyn FnMut()| (0..30).map(|_| {
        let t = Instant::now();
        f();
        t.elapsed().as_secs_f64() * 1000.0
    }).fold(f64::MAX, f64::min);
    let raw = CamFrame { w: 1280, h: 720, bgra: (0..1280 * 720 * 4).map(|i| (i * 31 % 251) as u8).collect() };
    let mut inp = SegInput::default();
    let prep = best(&mut || inp.prepare(&raw, 256, 144));
    for d in [160u32, 240, 320, 480] {
        let sq = super::super::fit_square(&raw, d);
        let mut m = SqMask::default();
        disk(144, &mut m);
        let (mut mask, mut s) = (Vec::new(), Scratch::default());
        let r = best(&mut || refine(&m, &sq, &mut mask, &mut s));
        let mut p = sq.clone();
        let c = best(&mut || {
            p.bgra.copy_from_slice(&sq.bgra);
            composite(&mut p, &mask, 18.0, &mut s);
        });
        let mut b = sq.bgra.clone();
        let (mut tmp, mut acc) = (Vec::new(), Vec::new());
        let bl = best(&mut || crate::objects::blur::blur_block_on(&mut b, d as usize, d as usize, 18.0, &mut tmp, &mut acc, 1));
        println!("BENCH bubble {d}: prepare {prep:.3} ms, refine {r:.3} ms, composite (sigma 18) {c:.3} ms (its blur {bl:.3}), total {:.3} ms", prep + r + c);
    }
}

/// A camera-like picture: a person (head and shoulders) before a busy
/// background (a bookshelf of colored stripes, a window), and the
/// person's coarse mask (as a model gives: low resolution, soft edge).
pub(crate) fn person_scene(w: u32, h: u32) -> (CamFrame, impl Fn(f32, f32) -> f32) {
    // Normalized to the frame's height, around the centre square's middle:
    // (head, shoulders) as distances in units of their own size.
    let parts = move |x: f32, y: f32| {
        let cx = x - w as f32 / (2.0 * h as f32);
        let head = ((cx / 0.15).powi(2) + ((y - 0.38) / 0.19).powi(2)).sqrt();
        let neck = if (0.5..0.7).contains(&y) && cx.abs() < 0.07 { 0.5 } else { 9.0 };
        let body = if y > 0.62 { ((cx / 0.38).powi(2) + ((y - 1.05) / 0.45).powi(2)).sqrt() } else { 9.0 };
        (head.min(neck), body)
    };
    let inside = move |x: f32, y: f32| {
        let (a, b) = parts(x, y);
        a.min(b)
    };
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / h as f32, y as f32 / h as f32);
            let (head, body) = parts(u, v);
            let c: [u8; 3] = if body < 1.0 {
                [150, 80, 40] // a blue shirt (BGR)
            } else if head < 1.0 {
                [120, 150, 205] // skin
            } else if (0.08..0.42).contains(&v) && u > 1.25 {
                [235, 225, 200] // a bright window
            } else {
                // Shelves: bands of colored book spines.
                let band = ((v * 6.0) as u32) % 3;
                let spine = ((u * 40.0) as u32 * 7 + band * 3) % 6;
                let pal = [[40, 60, 160], [60, 140, 60], [150, 50, 50], [30, 160, 200], [120, 40, 140], [90, 90, 90]];
                if (v * 6.0).fract() < 0.08 { [40, 50, 70] } else { pal[spine as usize] }
            };
            bgra.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    // Soft over 8 % of a part's size, centred on its edge.
    let soft = move |x: f32, y: f32| (0.5 - (inside(x, y) - 1.0) / 0.08).clamp(0.0, 1.0);
    (CamFrame { w, h, bgra }, soft)
}

/// Preview PNGs: the M and XL bubbles (circle and rounded) with the
/// background off, light and strong, from a synthetic person and mask,
/// dark and light, at 2x, under `RUSTSHOT_PREVIEW_DIR`.
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_blur_previews() {
    use super::super::{compose, fit_square, template, CamSize, Look, Shape};
    use crate::pixbuf::PixBuf;
    use crate::theme::{DARK, LIGHT};
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (raw, soft) = person_scene(1280, 720);
    // The coarse mask of the square, as the model's 144 px would have it.
    let n = 144usize;
    let side = 720.0f32;
    let x0 = (1280.0 - side) / 2.0;
    let m = SqMask {
        n,
        v: (0..n * n)
            .map(|k| {
                let (i, j) = ((k % n) as f32 + 0.5, (k / n) as f32 + 0.5);
                soft((x0 + i * side / n as f32) / 720.0, j * side / n as f32 / 720.0)
            })
            .collect(),
    };
    // And the real model's own mask of the scene (models/selfie.bin).
    let mut masks = vec![("", m)];
    if let Ok(bytes) = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("models/selfie.bin")) {
        let mut net = seg::Net::selfie(&bytes).unwrap();
        let mut inp = SegInput::default();
        inp.prepare(&raw, 256, 144);
        let mut mm = SqMask::default();
        assert!(net.segment(&inp, &mut mm));
        masks.push(("-model", mm));
    }
    let cases = [(Shape::Circle, CamSize::M), (Shape::Rounded, CamSize::Xl)];
    for (tn, th, bg) in [("dark", &DARK, [38u8, 44, 58]), ("light", &LIGHT, [214, 222, 232])] {
        for ((suffix, m), (shape, size)) in masks.iter().flat_map(|m| cases.iter().map(move |c| (m, *c))) {
            let look = Look { shape, size, mirror: true, ring: None, ring_w: 2, bg: Bg::Off };
            let t = template(look, 2.0, th);
            let mut cells = Vec::new();
            for b in Bg::ALL {
                let mut pic = fit_square(&raw, t.d);
                let (mut mask, mut s) = (Vec::new(), Scratch::default());
                refine(m, &pic, &mut mask, &mut s);
                composite(&mut pic, &mask, b.sigma(), &mut s);
                let mut out = Vec::new();
                compose(&t, Some(&pic), look.mirror, &mut out);
                cells.push(crate::rec_ui::layer_over(&out, bg));
            }
            let (cw, n) = (t.w, cells.len() as u32);
            let mut sheet = vec![0u8; (cw * n * cw * 4) as usize];
            for (k, c) in cells.iter().enumerate() {
                for y in 0..cw {
                    let at = ((y * cw * n + k as u32 * cw) * 4) as usize;
                    sheet[at..at + (cw * 4) as usize].copy_from_slice(&c[(y * cw * 4) as usize..][..(cw * 4) as usize]);
                }
            }
            let name = format!("{tn}-bubble-blur-{}-{}{suffix}@2x.png", shape.key(), size.key());
            std::fs::write(dir.join(name), PixBuf::from_raw(cw * n, cw, sheet).unwrap().to_png().unwrap()).unwrap();
        }
    }
}
