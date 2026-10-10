use super::*;
use crate::theme::{DARK, LIGHT};

/// Held by tests that set or rely on the remembered position (global).
pub(crate) static REMEMBERED_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Live camera tests take turns (a second reader preempts the first).
#[cfg(windows)]
pub(crate) static LIVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const LOOK: Look = Look { shape: Shape::Circle, size: CamSize::M, mirror: false };

#[test]
fn config_values_round_trip() {
    for s in Shape::ALL {
        assert_eq!(Shape::from_config(s.key()), s);
    }
    for z in CamSize::ALL {
        assert_eq!(CamSize::from_config(z.key()), z);
    }
    assert_eq!(Shape::from_config("blob"), Shape::Circle);
    assert_eq!(CamSize::from_config(""), CamSize::M);
    assert_eq!(CamSize::ALL.map(|z| z.logical()), [160, 240, 320]);
    assert_eq!(side(CamSize::M, 1.5), 360);
}

/// Every shape at every size (1x and 2x): full inside, empty outside its
/// corners (circle, rounded), one pixel of anti-aliasing along curved
/// edges, crisp straight edges, the right area.
#[test]
fn shape_masks_are_anti_aliased() {
    for scale in [1.0f32, 2.0] {
        for size in CamSize::ALL {
            let d = side(size, scale);
            for shape in Shape::ALL {
                let cov = |x: u32, y: u32| coverage(shape, d, x, y);
                assert_eq!(cov(d / 2, d / 2), 1.0, "{shape:?} {d}: centre");
                // Middle of each edge: inside (the shape touches its box).
                assert!(cov(0, d / 2) > 0.99 && cov(d - 1, d / 2) > 0.99 && cov(d / 2, 0) > 0.99, "{shape:?} {d}: edges");
                let corner = cov(0, 0);
                let area: f32 = (0..d).flat_map(|y| (0..d).map(move |x| (x, y))).map(|(x, y)| cov(x, y)).sum();
                let df = d as f32;
                let partial = (0..d).flat_map(|y| (0..d).map(move |x| (x, y))).filter(|&(x, y)| {
                    let c = cov(x, y);
                    c > 0.02 && c < 0.98
                });
                let partial = partial.count();
                match shape {
                    Shape::Circle => {
                        assert_eq!(corner, 0.0);
                        let want = std::f32::consts::PI * df * df / 4.0;
                        assert!((area - want).abs() / want < 0.005, "circle {d}: area {area} vs {want}");
                        // Soft pixels all round (about the perimeter), none deep inside.
                        assert!(partial as f32 > df * 2.0 && (partial as f32) < df * std::f32::consts::PI * 1.6, "{d}: {partial}");
                    }
                    Shape::Rounded => {
                        assert_eq!(corner, 0.0);
                        let r = df * 0.2;
                        let want = df * df - (4.0 - std::f32::consts::PI) * r * r;
                        assert!((area - want).abs() / want < 0.005, "rounded {d}: area {area} vs {want}");
                        assert!(partial > 0);
                        // Straight runs are crisp.
                        assert_eq!(cov(0, d / 2), 1.0);
                    }
                    Shape::Square => {
                        assert_eq!(corner, 1.0);
                        assert_eq!(partial, 0, "square {d}: pixel-aligned, no soft pixels");
                        assert_eq!(area, df * df);
                    }
                }
                // Coverage never rises going outward along a row from the centre.
                let row: Vec<f32> = (0..d / 2).map(|x| cov(x, d / 5)).collect();
                assert!(row.windows(2).all(|w| w[0] <= w[1] + 1e-6), "{shape:?} {d}: monotonic");
            }
        }
    }
}

#[test]
fn ring_hugs_the_edge_in_the_accent() {
    for shape in Shape::ALL {
        let t = template(Look { shape, ..LOOK }, 1.0, &DARK);
        let d = t.d as usize;
        let rw = ring_width(1.0);
        for y in 0..d {
            for x in 0..d {
                let e = sdf(shape, d as f32, x as f32 + 0.5, y as f32 + 0.5);
                let ring = t.ring[y * d + x];
                if e < -rw - 1.0 || e > 1.0 {
                    assert_eq!(ring, 0, "{shape:?} ({x},{y}) d={e}");
                }
                if e < -0.5 && e > -rw + 0.5 {
                    assert_eq!(ring, 255, "{shape:?} ({x},{y}) d={e}");
                }
                assert!(ring <= t.mask[y * d + x], "the ring is inside the shape");
            }
        }
        // The ring in the window is the theme's accent.
        let mut out = Vec::new();
        compose(&t, None, false, &mut out);
        let (w, pad) = (t.w as usize, t.pad as usize);
        let p = &out[((pad + d / 2) * w + pad + 1) * 4..][..4];
        assert_eq!([p[2], p[1], p[0], p[3]], [DARK.accent.r, DARK.accent.g, DARK.accent.b, 255], "{shape:?}");
    }
}

/// A test picture: left half red, right half blue.
fn halves(d: u32) -> CamFrame {
    let mut bgra = Vec::new();
    for _ in 0..d {
        for x in 0..d {
            bgra.extend_from_slice(if x < d / 2 { &[0, 0, 255, 255] } else { &[255, 0, 0, 255] });
        }
    }
    CamFrame { w: d, h: d, bgra }
}

#[test]
fn compose_masks_mirrors_and_premultiplies() {
    let t = template(LOOK, 1.0, &LIGHT);
    let f = halves(t.d);
    let (w, pad, d) = (t.w as usize, t.pad as usize, t.d as usize);
    let at = |out: &[u8], x: usize, y: usize| <[u8; 4]>::try_from(&out[(y * w + x) * 4..][..4]).unwrap();
    let mut out = Vec::new();
    compose(&t, Some(&f), false, &mut out);
    assert_eq!(out.len(), w * w * 4);
    let (lx, rx, cy) = (pad + d / 4, pad + 3 * d / 4, pad + d / 2);
    assert_eq!(at(&out, lx, cy), [0, 0, 255, 255], "red on the left");
    assert_eq!(at(&out, rx, cy), [255, 0, 0, 255], "blue on the right");
    compose(&t, Some(&f), true, &mut out);
    assert_eq!(at(&out, lx, cy), [255, 0, 0, 255], "mirrored: blue on the left");
    assert_eq!(at(&out, rx, cy), [0, 0, 255, 255]);
    // Outside the circle: only the soft shadow (translucent), never past alpha.
    let corner = at(&out, pad + 2, pad + 2);
    assert!(corner[3] < 128, "{corner:?}");
    assert_eq!(at(&out, 0, 0)[3], 0, "the window's corner is clear");
    assert!(out.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]), "premultiplied");
    // Shadow below more than above (it falls down).
    let above = at(&out, pad + d / 2, pad - 4)[3];
    let below = at(&out, pad + d / 2, pad + d + 4)[3];
    assert!(below > above, "{above} {below}");
    // A picture of the wrong size shows the placeholder (not garbage).
    let small = halves(t.d / 2);
    compose(&t, Some(&small), false, &mut out);
    assert_eq!(at(&out, lx, cy)[3], 255);
    assert_ne!(at(&out, lx, cy), [0, 0, 255, 255]);
}

#[test]
fn placement_is_relative_to_the_area() {
    let area = (100, 50, 800, 600);
    let d = 240;
    // Default: bottom left, 16 px in.
    assert_eq!(place(Rel::default(), area, d, 1.0), (116, 50 + 600 - 16 - 240));
    // At 2x the offsets double.
    assert_eq!(place(Rel::default(), area, 480, 2.0), (132, 650 - 32 - 480));
    // Every corner round-trips through rel_of.
    for (x, y) in [(130, 70), (600, 80), (140, 380), (630, 390)] {
        let r = rel_of((x, y), area, d, 1.0);
        assert_eq!(place(r, area, d, 1.0), (x, y), "{r:?}");
    }
    let r = rel_of((630, 390), area, d, 1.0);
    assert_eq!((r.corner, r.dx, r.dy), (Corner::BottomRight, 900.0 - 630.0 - 240.0, 650.0 - 390.0 - 240.0));
    // The area moves: the bubble keeps its corner and offset.
    assert_eq!(place(r, (0, 0, 1000, 700), d, 1.0), (1000 - 30 - 240, 700 - 20 - 240));
}

#[test]
fn snapping_to_corners() {
    let area = (0, 0, 1000, 800);
    let d = 160;
    let tl = place(Rel { corner: Corner::TopLeft, dx: INSET, dy: INSET }, area, d, 1.0);
    let br = place(Rel { corner: Corner::BottomRight, dx: INSET, dy: INSET }, area, d, 1.0);
    assert_eq!(snap((tl.0 + 24, tl.1 - 20), area, d, 1.0), tl, "within 24 px");
    assert_eq!(snap((tl.0 + 25, tl.1), area, d, 1.0), (tl.0 + 25, tl.1), "just out of reach");
    assert_eq!(snap((br.0 - 10, br.1 + 10), area, d, 1.0), br);
    assert_eq!(snap((500, 400), area, d, 1.0), (500, 400), "the middle stays");
    // At 2x the reach doubles.
    let tl2 = place(Rel { corner: Corner::TopLeft, dx: INSET, dy: INSET }, area, 320, 2.0);
    assert_eq!(snap((tl2.0 + 40, tl2.1 + 40), area, 320, 2.0), tl2);
    // A small area: the nearest corner wins.
    let tiny = (0, 0, 200, 200);
    let p = snap((20, 20), tiny, d, 1.0);
    assert_eq!(p, (16, 16));
}

#[test]
fn remembered_position_survives() {
    let _g = REMEMBERED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let r = Rel { corner: Corner::TopRight, dx: 40.0, dy: 30.0 };
    remember(r);
    assert_eq!(remembered(), r);
    remember(Rel::default());
}

fn m(fmt: Fmt, w: u32, h: u32, fps: f32) -> Mode {
    Mode { fmt, w, h, fps }
}

#[test]
fn mode_choice_prefers_smooth_720p_without_mjpg() {
    let typical = [
        m(Fmt::Mjpg, 1920, 1080, 30.0),
        m(Fmt::Mjpg, 1280, 720, 30.0),
        m(Fmt::Yuy2, 1280, 720, 10.0),
        m(Fmt::Yuy2, 640, 480, 30.0),
        m(Fmt::Yuy2, 320, 240, 30.0),
    ];
    assert_eq!(pick_mode(&typical, PREF), Ok(3), "480p at 30 beats 720p at 10");
    let good = [m(Fmt::Yuy2, 640, 480, 30.0), m(Fmt::Nv12, 1280, 720, 30.0), m(Fmt::Yuy2, 1280, 720, 30.0), m(Fmt::Nv12, 1920, 1080, 30.0)];
    assert_eq!(pick_mode(&good, PREF), Ok(1), "720p NV12");
    let fast = [m(Fmt::Nv12, 1280, 720, 60.0), m(Fmt::Nv12, 1280, 720, 15.0)];
    assert_eq!(pick_mode(&fast, PREF), Ok(0), "60 fps (frames are skipped) before 15");
    let big = [m(Fmt::Nv12, 1920, 1080, 30.0), m(Fmt::Nv12, 960, 540, 30.0)];
    assert_eq!(pick_mode(&big, PREF), Ok(1), "smaller before larger");
    assert_eq!(pick_mode(&[m(Fmt::Mjpg, 1280, 720, 30.0)], PREF), Err(MJPG_ONLY));
    assert_eq!(pick_mode(&[m(Fmt::Other, 1280, 720, 30.0)], PREF), Err(NO_FORMAT));
    assert_eq!(pick_mode(&[], PREF), Err(NO_FORMAT));
}

/// Known limited-range values: black, white, BT.601 red / blue, grey.
#[test]
fn yuy2_and_nv12_to_bgra() {
    // Two pixels per YUY2 word: (Y0 U Y1 V).
    let src = [16u8, 128, 235, 128, /* row 2 */ 81, 90, 81, 240];
    let mut out = Vec::new();
    yuy2_to_bgra(&src, 4, 2, 1, Matrix::Bt601, &mut out);
    assert_eq!(out, [0, 0, 0, 255, 255, 255, 255, 255], "black, white");
    yuy2_to_bgra(&src[4..], 4, 2, 1, Matrix::Bt601, &mut out);
    assert_eq!(&out[..4], &[0, 0, 255, 255], "BT.601 red");
    // Padded rows (stride 8 for 2 pixels).
    let padded = [126u8, 128, 126, 128, 9, 9, 9, 9, 41, 240, 41, 110];
    yuy2_to_bgra(&padded, 8, 2, 2, Matrix::Bt601, &mut out);
    assert_eq!(&out[..4], &[128, 128, 128, 255], "mid grey");
    assert_eq!(&out[8..12], &[255, 0, 0, 255], "BT.601 blue");
    // NV12, 2x2: one chroma pair for the block.
    let y = [16u8, 235, 126, 126];
    let uv = [128u8, 128];
    nv12_to_bgra(&y, &uv, 2, 2, 2, Matrix::Bt601, &mut out);
    assert_eq!(out.len(), 16);
    assert_eq!(&out[..8], &[0, 0, 0, 255, 255, 255, 255, 255]);
    assert_eq!(&out[8..12], &[128, 128, 128, 255]);
    let red_uv = [90u8, 240];
    nv12_to_bgra(&[81, 81, 81, 81], &red_uv, 2, 2, 2, Matrix::Bt601, &mut out);
    assert!(out.as_chunks::<4>().0.iter().all(|p| *p == [0, 0, 255, 255]));
    // BT.709 red: Y 63, U 102, V 240.
    nv12_to_bgra(&[63, 63, 63, 63], &[102, 240], 2, 2, 2, Matrix::Bt709, &mut out);
    let p = &out[..4];
    assert!(p[2] >= 254 && p[1] <= 2 && p[0] <= 2, "{p:?}");
    // Short input: no panic, a blank picture.
    nv12_to_bgra(&[0; 3], &[], 2, 2, 2, Matrix::Bt601, &mut out);
    assert_eq!(out, [0; 16]);
}

/// Full range (0-255): black is 0 and white 255 without the 16-235
/// stretch; BT.709 full red.
#[test]
fn full_range_yuv() {
    let full601 = Yuv { matrix: Matrix::Bt601, full: true };
    let mut out = Vec::new();
    yuy2_to_bgra(&[0u8, 128, 255, 128], 4, 2, 1, full601, &mut out);
    assert_eq!(out, [0, 0, 0, 255, 255, 255, 255, 255]);
    // Limited range reads 16 as black and stretches the rest.
    yuy2_to_bgra(&[16u8, 128, 128, 128], 4, 2, 1, full601, &mut out);
    assert_eq!((out[0], out[4]), (16, 128));
    yuy2_to_bgra(&[16u8, 128, 128, 128], 4, 2, 1, Matrix::Bt601, &mut out);
    assert_eq!((out[0], out[4]), (0, 130));
    // BT.709 full-range red: Y 54, U 99, V 255.
    nv12_to_bgra(&[54; 4], &[99, 255], 2, 2, 2, Yuv { matrix: Matrix::Bt709, full: true }, &mut out);
    let p = &out[..4];
    assert!(p[2] >= 253 && p[1] <= 2 && p[0] <= 2, "{p:?}");
}

/// A synthetic NV12 buffer: a `w`x`h` picture whose Y plane has `y_rows`
/// rows (the padding below the picture is 0xEE) and whose UV plane has
/// `uv_rows` rows, at `stride`; Y = 100 + row % 50, UV (60, 200) on the
/// picture's rows. With `short_tail` the last UV row has only `w` bytes.
fn nv12_buf(w: usize, h: usize, stride: usize, y_rows: usize, uv_rows: usize, short_tail: bool) -> Vec<u8> {
    let mut b = vec![0xEEu8; stride * (y_rows + uv_rows)];
    for y in 0..h {
        b[y * stride..][..w].fill(100 + (y % 50) as u8);
    }
    for r in 0..h / 2 {
        for x in (0..w).step_by(2) {
            let at = (y_rows + r) * stride + x;
            b[at] = 60;
            b[at + 1] = 200;
        }
    }
    if short_tail {
        b.truncate((y_rows + uv_rows - 1) * stride + w);
    }
    b
}

/// The UV plane is found after the Y plane's real rows (padded 1080 to
/// 1088, with or without the trailing padding), never past the buffer's
/// end; a buffer too short for the picture drops it.
#[test]
fn camera_buffer_layouts() {
    let (w, h, stride) = (1920usize, 1080usize, 2048usize);
    let px = |y: u8| {
        let mut p = [0u8; 4];
        yuv_px(y, 60, 200, coef(Matrix::Bt601.into()), &mut p);
        p
    };
    let check = |buf: &[u8], coded_h: u32, want_rows: usize| {
        let l = layout(Fmt::Nv12, w as u32, h as u32, coded_h, stride, buf.len()).expect("fits");
        assert_eq!(l.uv, want_rows * stride, "UV after {want_rows} rows");
        assert!(l.len <= buf.len());
        let (y, uv) = buf[..l.len].split_at(l.uv);
        let mut out = Vec::new();
        nv12_to_bgra(y, uv, l.stride, w as u32, h as u32, Matrix::Bt601, &mut out);
        // The picture's chroma everywhere (not the padding's 0xEE).
        assert_eq!(&out[..4], &px(100));
        let row = h - 1;
        assert_eq!(&out[(row * w + w - 1) * 4..][..4], &px(100 + (row % 50) as u8), "the bottom rows' chroma");
    };
    // No padding.
    check(&nv12_buf(w, h, stride, h, h / 2, false), h as u32, h);
    check(&nv12_buf(w, h, stride, h, h / 2, true), h as u32, h);
    // Padded to 1088 rows, both planes (the frame size says 1080).
    check(&nv12_buf(w, h, stride, 1088, 544, false), h as u32, 1088);
    // Padded Y plane, the UV plane's trailing padding missing.
    check(&nv12_buf(w, h, stride, 1088, 540, false), h as u32, 1088);
    check(&nv12_buf(w, h, stride, 1088, 540, true), h as u32, 1088);
    // A coded height of 1088 with a 1080 aperture says so directly.
    check(&nv12_buf(w, h, stride, 1088, 540, true), 1088, 1088);
    // Too short (a truncated buffer): dropped, never read past the end.
    let mut short = nv12_buf(w, h, stride, h, h / 2, false);
    short.truncate(stride * h + stride * 100);
    assert_eq!(layout(Fmt::Nv12, w as u32, h as u32, h as u32, stride, short.len()), None);
    assert_eq!(layout(Fmt::Nv12, w as u32, h as u32, h as u32, stride, stride * h - 1), None);
    assert_eq!(layout(Fmt::Nv12, w as u32, h as u32, h as u32, 1000, 1 << 24), None, "pitch under the width");
    assert_eq!(layout(Fmt::Nv12, w as u32, h as u32, h as u32, stride, 0), None);
    // Junk past an unpadded picture is not taken for padding.
    let l = layout(Fmt::Nv12, w as u32, h as u32, h as u32, stride, stride * (h + h / 2) + 777).unwrap();
    assert_eq!(l.uv, stride * h);
    // YUY2: the last row needs only its own bytes.
    let (yw, yh, ys) = (640u32, 480u32, 1536usize);
    let need = ys * 479 + 1280;
    assert_eq!(layout(Fmt::Yuy2, yw, yh, yh, ys, need), Some(Layout { stride: ys, uv: 0, len: need }));
    assert_eq!(layout(Fmt::Yuy2, yw, yh, yh, ys, need - 1), None);
    assert_eq!(layout(Fmt::Yuy2, yw, yh, yh, 1000, 1 << 24), None);
    assert_eq!(layout(Fmt::Mjpg, yw, yh, yh, ys, 1 << 24), None);
}

#[test]
fn bottom_up_rows_are_flipped() {
    let mut px: Vec<u8> = (0..3u8).flat_map(|r| [r; 8]).collect(); // 2x3
    flip_rows(&mut px, 2, 3);
    assert_eq!(px, [[2u8; 8], [1; 8], [0; 8]].concat());
    let mut even: Vec<u8> = (0..2u8).flat_map(|r| [r; 4]).collect();
    flip_rows(&mut even, 1, 2);
    assert_eq!(even, [[1u8; 4], [0; 4]].concat());
}

/// The picture shows only inside the ring: at the anti-aliased edge the
/// ring and the shadow share the pixel and the picture has no part (no
/// fringe of picture colour outside the ring).
#[test]
fn no_picture_fringe_at_the_edge() {
    for shape in Shape::ALL {
        for s in [1.0f32, 2.0] {
            let t = template(Look { shape, ..LOOK }, s, &DARK);
            let d = t.d as usize;
            for i in 0..d * d {
                assert_eq!(t.pic[i] as u32 + t.ring[i] as u32, t.mask[i] as u32, "{shape:?} {s}x pixel {i}");
            }
            // Pictures that differ only in colour give the same pixels
            // wherever the picture has no coverage.
            let solid = |c: [u8; 4]| CamFrame { w: t.d, h: t.d, bgra: c.repeat(d * d) };
            let (mut a, mut b) = (Vec::new(), Vec::new());
            compose(&t, Some(&solid([255, 255, 255, 255])), false, &mut a);
            compose(&t, Some(&solid([0, 0, 0, 255])), false, &mut b);
            let (w, pad) = (t.w as usize, t.pad as usize);
            let mut edge = 0;
            for y in 0..d {
                for x in 0..d {
                    let o = ((y + pad) * w + x + pad) * 4;
                    let i = y * d + x;
                    if t.pic[i] == 0 {
                        assert_eq!(a[o..o + 4], b[o..o + 4], "{shape:?} ({x},{y}): the picture leaks");
                        if t.mask[i] > 0 && t.mask[i] < 255 {
                            edge += 1;
                        }
                    }
                }
            }
            if shape != Shape::Square {
                assert!(edge > 0, "{shape:?}: the soft edge was checked");
            }
            assert!(a.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]), "premultiplied");
        }
    }
}

/// Off the toolbar and popover: the nearest free corner of the area;
/// outside a small area (on its screen); kept when nothing is in the way.
#[test]
fn bubble_keeps_off_the_toolbar() {
    let (area, screen, d) = ((100, 100, 1000, 700), (0, 0, 1920, 1080), 240);
    let bl = place(Rel::default(), area, d, 1.0);
    assert_eq!(clear_of(bl, area, d, 1.0, &[], screen), bl);
    assert_eq!(clear_of(bl, area, d, 1.0, &[(800, 100, 200, 40)], screen), bl, "not in the way");
    // A popover over the bottom left: the top left is free.
    let br = place(Rel { corner: Corner::BottomRight, dx: INSET, dy: INSET }, area, d, 1.0);
    assert_eq!(clear_of(bl, area, d, 1.0, &[(110, 400, 300, 380)], screen), place(Rel { corner: Corner::TopLeft, dx: INSET, dy: INSET }, area, d, 1.0), "the nearest free corner");
    // The left side covered: the bottom right.
    assert_eq!(clear_of(bl, area, d, 1.0, &[(110, 100, 300, 700)], screen), br);
    // The whole bottom covered: the top left (nearer than the top right).
    let tl = place(Rel { corner: Corner::TopLeft, dx: INSET, dy: INSET }, area, d, 1.0);
    assert_eq!(clear_of(bl, area, d, 1.0, &[(100, 400, 1000, 400)], screen), tl);
    // An area too small for a free corner: just outside it, on screen.
    let small = (600, 300, 300, 260);
    let p = clear_of(place(Rel::default(), small, d, 1.0), small, d, 1.0, &[small], screen);
    let r = (p.0, p.1, d as u32, d as u32);
    assert!(!overlaps(r, small) && inside(r, screen), "{p:?}");
    assert_eq!(p, (600, 300 - 16 - d), "above the area (the nearest), left aligned");
    // At the top of the screen: never off it.
    for top in [(600, 0, 300, 260), (0, 0, 300, 260), (1620, 0, 300, 260)] {
        let p = clear_of(place(Rel::default(), top, d, 1.0), top, d, 1.0, &[top], screen);
        let r = (p.0, p.1, d as u32, d as u32);
        assert!(!overlaps(r, top) && inside(r, screen), "{top:?}: {p:?}");
    }
    // Nowhere free: where it was.
    assert_eq!(clear_of(bl, area, d, 1.0, &[screen], screen), bl);
}

/// The fitted picture and the scratch are reused (no new allocation for
/// the same size).
#[test]
fn fit_square_into_reuses_its_buffers() {
    let pic = sample_picture(320, 180);
    let (mut tmp, mut out) = (Vec::new(), CamFrame::default());
    fit_square_into(&pic, 96, &mut tmp, &mut out);
    let (p0, t0) = (out.bgra.as_ptr(), tmp.as_ptr());
    fit_square_into(&pic, 96, &mut tmp, &mut out);
    assert_eq!((out.bgra.as_ptr(), tmp.as_ptr()), (p0, t0));
    assert_eq!(out, fit_square(&pic, 96));
}

#[test]
fn fit_square_crops_the_centre_and_averages() {
    // 4x2: columns 1..3 are the centre square; averaged down to 1x1.
    let px = |v: u8| [v, v, v, 255];
    let row: Vec<u8> = [10u8, 20, 40, 90].iter().flat_map(|&v| px(v)).collect();
    let f = CamFrame { w: 4, h: 2, bgra: [row.clone(), row].concat() };
    let s = fit_square(&f, 1);
    assert_eq!((s.w, s.h), (1, 1));
    assert_eq!(s.bgra, px(30));
    let s = fit_square(&f, 2);
    assert_eq!(s.bgra, [px(20), px(40), px(20), px(40)].concat());
    // Upscaling repeats pixels.
    let s = fit_square(&f, 4);
    assert_eq!(&s.bgra[..8], &[px(20), px(20)].concat()[..]);
    assert_eq!(fit_square(&CamFrame::default(), 4), CamFrame::default());
}

/// A synthetic camera picture: a warm gradient with a head-and-shoulders
/// shape (for previews).
pub(crate) fn sample_picture(w: u32, h: u32) -> CamFrame {
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let (hx, hy) = ((fx - 0.5) * w as f32 / h as f32, fy - 0.42);
            let head = hx * hx / 0.018 + hy * hy / 0.035 < 1.0;
            let body = fy > 0.68 && ((fx - 0.5) * w as f32 / h as f32).abs() < 0.32 + (fy - 0.68) * 0.9;
            let p = if head {
                [120, 160, 220]
            } else if body {
                [150, 90, 60]
            } else {
                [(200.0 - 80.0 * fy) as u8, (170.0 - 40.0 * fx) as u8, (90.0 + 60.0 * fx) as u8]
            };
            bgra.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
    }
    CamFrame { w, h, bgra }
}

/// Preview PNGs: every shape at S/M/L, mirrored and not, the placeholder,
/// dark and light, at 1x and 2x, under `RUSTSHOT_PREVIEW_DIR`.
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_bubble_previews() {
    use crate::pixbuf::PixBuf;
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pic = sample_picture(1280, 720);
    for (tn, th, bg) in [("dark", &DARK, [38u8, 44, 58]), ("light", &LIGHT, [214, 222, 232])] {
        for s in [1.0f32, 2.0] {
            // One sheet per scale: rows = shapes, columns = S M L.
            let cells: Vec<Vec<(Template, CamFrame)>> = Shape::ALL
                .iter()
                .map(|&shape| {
                    CamSize::ALL
                        .iter()
                        .map(|&size| {
                            let t = template(Look { shape, size, mirror: false }, s, th);
                            let f = fit_square(&pic, t.d);
                            (t, f)
                        })
                        .collect()
                })
                .collect();
            let col_w: Vec<u32> = (0..3).map(|c| cells[0][c].0.w).collect();
            let (sw, sh) = (col_w.iter().sum::<u32>(), cells.iter().map(|r| r[2].0.w).sum::<u32>());
            let mut sheet = [bg[0], bg[1], bg[2], 255].repeat((sw * sh) as usize);
            let mut y0 = 0;
            for row in &cells {
                let mut x0 = 0;
                for (c, (t, f)) in row.iter().enumerate() {
                    let mut out = Vec::new();
                    compose(t, Some(f), false, &mut out);
                    let over = crate::rec_ui::layer_over(&out, bg);
                    for y in 0..t.w {
                        let src = &over[(y * t.w * 4) as usize..][..(t.w * 4) as usize];
                        let at = (((y0 + y) * sw + x0) * 4) as usize;
                        sheet[at..at + src.len()].copy_from_slice(src);
                    }
                    x0 += col_w[c];
                }
                y0 += row[2].0.w;
            }
            let img = PixBuf::from_raw(sw, sh, sheet).unwrap();
            std::fs::write(dir.join(format!("{tn}-bubble-shapes@{s}x.png")), img.to_png().unwrap()).unwrap();
        }
        // Mirror and placeholder, M circle and rounded at 2x.
        for (name, look, frame) in [
            ("mirror-off", Look { shape: Shape::Rounded, size: CamSize::M, mirror: false }, true),
            ("mirror-on", Look { shape: Shape::Rounded, size: CamSize::M, mirror: true }, true),
            ("placeholder", LOOK, false),
        ] {
            let t = template(look, 2.0, th);
            let f = fit_square(&pic, t.d);
            let mut out = Vec::new();
            compose(&t, frame.then_some(&f), look.mirror, &mut out);
            let img = PixBuf::from_raw(t.w, t.w, crate::rec_ui::layer_over(&out, bg)).unwrap();
            std::fs::write(dir.join(format!("{tn}-bubble-{name}@2x.png")), img.to_png().unwrap()).unwrap();
        }
    }
}

/// A live camera, when there is one: listed, opened at a supported mode,
/// a few pictures read. Skips (passes) on a machine without a camera.
#[cfg(windows)]
#[test]
#[ignore = "needs a camera; run with --ignored"]
fn live_cam_opens_and_reads() {
    let _g = LIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cams = list();
    let Some(first) = cams.first() else {
        eprintln!("SKIP: no camera on this machine");
        return;
    };
    eprintln!("cameras: {cams:?}");
    let mut cam = match win::WinCam::open(&first.id, PREF) {
        Ok(c) => c,
        Err(e) => {
            let e = format!("{e:#}");
            if e.contains(ACCESS_OFF) || e.contains(MJPG_ONLY) {
                eprintln!("SKIP: {e}");
                return;
            }
            panic!("open {}: {e}", first.name);
        }
    };
    eprintln!("mode: {:?}", cam.mode());
    let (w, h) = cam.size();
    let mut got = 0;
    let t0 = std::time::Instant::now();
    let want: usize = std::env::var("RUSTSHOT_CAM_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
    let mut f = CamFrame::default();
    while got < want && t0.elapsed() < std::time::Duration::from_secs(10) {
        if cam.read(&mut f).unwrap() {
            assert_eq!((f.w, f.h), (w, h));
            assert_eq!(f.bgra.len(), (w * h * 4) as usize);
            // Mean colour only (no picture is kept): a wrong conversion
            // shows as all-black or strongly green.
            let n = (w * h) as u64;
            let mean: Vec<u64> = (0..3).map(|c| f.bgra.as_chunks::<4>().0.iter().map(|p| p[c] as u64).sum::<u64>() / n).collect();
            let px = f.bgra.as_chunks::<4>().0;
            let chroma = px.iter().map(|p| (p[2] as i32 - p[1] as i32).unsigned_abs() as u64 + (p[0] as i32 - p[1] as i32).unsigned_abs() as u64).sum::<u64>() / n;
            let var = px.iter().map(|p| (p[1] as i64 - mean[1] as i64).pow(2) as u64).sum::<u64>() / n;
            eprintln!("picture {got}: mean BGR {mean:?}, mean |R-G|+|B-G| {chroma}, G variance {var}");
            got += 1;
        }
    }
    assert_eq!(got, want, "pictures within 10 s");
    let (drops, yuv, coded_h, stride) = cam.stats();
    eprintln!("dropped {drops}; {yuv:?}, coded height {coded_h}, stride {stride}");
    assert_eq!(drops, 0, "no picture dropped");
}

#[cfg(windows)]
mod window;
