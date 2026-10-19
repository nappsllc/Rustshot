use super::*;
use crate::pixbuf::PixBuf;
use crate::theme::{DARK, LIGHT};
use crate::ui::preview;

fn rr(x: i32, y: i32, w: i32, h: i32) -> R {
    R::xywh(x, y, w, h)
}

fn total(pieces: &[R]) -> i64 {
    pieces.iter().map(area).sum()
}

fn disjoint(pieces: &[R]) -> bool {
    pieces.iter().enumerate().all(|(i, a)| pieces[i + 1..].iter().all(|b| a.intersect(b).is_empty()))
}

fn mon(x: i32, y: i32, w: u32, h: u32, scale: f32) -> MonInfo {
    MonInfo { x, y, w, h, scale, primary: x == 0 && y == 0 }
}

#[test]
fn subtracting_a_rect_leaves_up_to_four_disjoint_pieces() {
    let r = rr(0, 0, 100, 80);
    // A hole in the middle: above, below, left, right.
    let p = subtract(r, rr(30, 20, 40, 30));
    assert_eq!(p.len(), 4);
    assert!(disjoint(&p));
    assert_eq!(total(&p), 100 * 80 - 40 * 30);
    // Over a corner: two pieces; covering it all: none; apart: itself.
    let c = subtract(r, rr(60, -10, 100, 50));
    assert_eq!((c.len(), total(&c)), (2, 100 * 80 - 40 * 40));
    assert!(subtract(r, rr(-5, -5, 200, 200)).is_empty());
    assert_eq!(subtract(r, rr(200, 0, 10, 10)), vec![r]);
    assert!(subtract(R::default(), rr(0, 0, 1, 1)).is_empty());
}

#[test]
fn the_visible_region_is_the_rect_less_every_window_above() {
    let within = rr(0, 0, 1920, 1080);
    let w = rr(400, 300, 600, 400);
    assert_eq!(visible(w, &[], within), vec![w], "nothing above");
    // Two windows above it, overlapping each other and it.
    let above = [rr(300, 250, 300, 200), rr(500, 350, 200, 500), rr(1500, 0, 100, 100)];
    let v = visible(w, &above, within);
    assert!(disjoint(&v));
    // Area by counting pixels.
    let mut n = 0i64;
    for y in w.y0..w.y1 {
        for x in w.x0..w.x1 {
            if !above.iter().any(|a| a.contains(x, y)) {
                n += 1;
                assert!(v.iter().any(|p| p.contains(x, y)), "({x}, {y}) is visible");
            }
        }
    }
    assert_eq!(total(&v), n);
    // Covered all over, and clipped to the shot.
    assert!(visible(w, &[rr(0, 0, 2000, 2000)], within).is_empty());
    assert_eq!(visible(rr(1800, 1000, 400, 300), &[], within), vec![rr(1800, 1000, 120, 80)]);
    assert!(visible(rr(-500, 0, 400, 300), &[], within).is_empty(), "off the shot");
}

#[test]
fn plan_follows_the_z_order_and_the_shot() {
    // Front to back: a dialog, a hidden picker, an ordinary window under
    // it, a second hidden window that the first covers in part.
    let tops = [
        Top { r: rr(100, 100, 200, 150), hidden: None },
        Top { r: rr(200, 150, 400, 300), hidden: Some(8.0) },
        Top { r: rr(0, 0, 1200, 900), hidden: None },
        Top { r: rr(500, 300, 300, 300), hidden: Some(0.0) },
    ];
    let mons = [mon(0, 0, 1920, 1080, 1.5)];
    let p = plan(&tops, (0, 0), (1920, 1080), &mons);
    assert_eq!(p.len(), 1, "the second hidden one is behind the ordinary window");
    let h = &p[0];
    assert_eq!((h.rect, h.seen, h.radius, h.scale), (rr(200, 150, 400, 300), rr(200, 150, 400, 300), 8.0, 1.5));
    assert_eq!(total(&h.pieces), 400 * 300 - 100 * 100, "less the dialog's corner over it");

    // Hidden windows over each other: the one behind loses what the
    // front one covers (each pixel is painted once).
    let tops = [Top { r: rr(0, 0, 100, 100), hidden: Some(0.0) }, Top { r: rr(50, 50, 100, 100), hidden: Some(0.0) }];
    let p = plan(&tops, (0, 0), (400, 400), &[]);
    assert_eq!(p.len(), 2);
    assert_eq!((total(&p[0].pieces), total(&p[1].pieces)), (100 * 100, 100 * 100 - 50 * 50));
    assert_eq!(p[0].scale, 1.0, "no monitor: 1");

    // A shot of the left monitor of two (virtual origin -1920): image px,
    // the window's part on this monitor, its scale from the monitor that
    // holds most of it.
    let mons = [mon(-1920, 0, 1920, 1080, 1.0), mon(0, 0, 2560, 1440, 2.0)];
    let tops = [Top { r: rr(-200, 100, 800, 400), hidden: Some(16.0) }];
    let p = plan(&tops, (-1920, 0), (1920, 1080), &mons);
    assert_eq!(p[0].rect, rr(1720, 100, 800, 400));
    assert_eq!(p[0].seen, rr(1720, 100, 200, 400));
    assert_eq!(p[0].pieces, vec![rr(1720, 100, 200, 400)]);
    assert_eq!(p[0].scale, 2.0);
    // Outside the shot: nothing.
    assert!(plan(&tops, (5000, 0), (100, 100), &mons).is_empty());
}

#[test]
fn a_pile_of_windows_above_does_not_blow_up_the_region() {
    let above: Vec<R> = (0..400).map(|i| rr((i % 20) * 50 + 3, (i / 20) * 50 + 3, 20, 20)).collect();
    let v = visible(rr(0, 0, 1000, 1000), &above, rr(0, 0, 4000, 4000));
    assert!(v.len() <= MAX_PIECES * 4 + 4, "{} pieces", v.len());
    assert!(disjoint(&v));
}

const T: &str = "Hidden from screenshots";
const B: &str = "This window protects private information";

#[test]
fn the_label_shrinks_then_drops_lines_to_fit() {
    let f = crate::fonts::ui_font();
    let full = layout(f, T, B, 1.0, rr(0, 0, 600, 400)).unwrap();
    assert_eq!((full.k, full.title, full.body), (1.0, true, true));
    let (cx, cy) = ((full.at.x0 + full.at.x1) / 2, (full.at.y0 + full.at.y1) / 2);
    assert!((cx - 300).abs() <= 1 && (cy - 200).abs() <= 1, "centred: {:?}", full.at);
    let k2 = layout(f, T, B, 2.0, rr(0, 0, 1200, 800)).unwrap();
    assert_eq!(k2.k, 2.0, "the monitor's scale");
    // Narrower than the body line: shrunk a little.
    let w = f.width(B, BODY_PX) as i32;
    let narrow = layout(f, T, B, 1.0, rr(0, 0, w + 2 * MARGIN as i32 - 30, 300)).unwrap();
    assert!(narrow.k < 1.0 && narrow.k >= MIN_K && narrow.body, "{narrow:?}");
    // Far too narrow for the body: the title alone; then the lock alone.
    let t = layout(f, T, B, 1.0, rr(0, 0, 140, 300)).unwrap();
    assert!(t.title && !t.body, "{t:?}");
    let lock = layout(f, T, B, 1.0, rr(0, 0, 70, 70)).unwrap();
    assert!(!lock.title && !lock.body, "{lock:?}");
    assert!(layout(f, T, B, 1.0, rr(0, 0, 30, 30)).is_none(), "not even the lock");
    for l in [full, narrow, t, lock] {
        assert!(l.at.w() > 0 && l.at.h() > 0);
    }
}

#[test]
fn the_label_moves_to_the_largest_piece_when_the_middle_is_covered() {
    let f = crate::fonts::ui_font();
    let rect = rr(100, 100, 600, 400);
    let whole = Hidden { rect, seen: rect, radius: 0.0, pieces: vec![rect], scale: 1.0 };
    let l = place(f, T, B, &whole).unwrap();
    assert_eq!(((l.at.x0 + l.at.x1) / 2, (l.at.y0 + l.at.y1) / 2), (400, 300));
    // A window over its middle and right: the left strip is the biggest.
    let pieces = subtract(rect, rr(330, 120, 600, 260));
    let h = Hidden { pieces: pieces.clone(), ..whole.clone() };
    let l = place(f, T, B, &h).unwrap();
    let big = pieces.iter().copied().max_by_key(area).unwrap();
    assert!(big.contains(l.at.x0, l.at.y0) && big.contains(l.at.x1 - 1, l.at.y1 - 1), "{:?} in {big:?}", l.at);
    // A small window over a corner only: still centred on the window.
    let h = Hidden { pieces: subtract(rect, rr(650, 80, 100, 60)), ..whole };
    let l = place(f, T, B, &h).unwrap();
    assert_eq!(((l.at.x0 + l.at.x1) / 2, (l.at.y0 + l.at.y1) / 2), (400, 300));
}

/// A desktop-ish backdrop: a diagonal gradient, so untouched pixels are
/// easy to tell from the placeholder.
fn backdrop(w: u32, h: u32) -> PixBuf {
    let mut img = PixBuf::new(w, h);
    for (i, p) in img.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
        *p = [(40.0 + 120.0 * x) as u8, (90.0 + 80.0 * y) as u8, (160.0 + 60.0 * (1.0 - x)) as u8, 255];
    }
    img
}

fn px(img: &PixBuf, x: i32, y: i32) -> [u8; 4] {
    img.get_pixel(x as u32, y as u32)
}

fn is_panel(p: [u8; 4], st: &Style) -> bool {
    [st.panel, st.stripe].iter().any(|c| p == [c.r, c.g, c.b, 255])
}

fn paint_all(img: &mut PixBuf, wins: &[Hidden], st: &Style) {
    let w = img.width() as usize;
    for h in wins {
        paint(img.as_raw_mut(), w, (0, 0), Order::Rgba, h, st);
    }
}

#[test]
fn paint_covers_the_visible_pieces_and_nothing_else() {
    let st = Style::of(&DARK);
    let rect = rr(40, 30, 320, 200);
    let cut = rr(250, 0, 200, 120);
    let h = Hidden { rect, seen: rect, radius: 12.0, pieces: subtract(rect, cut), scale: 1.0 };
    let before = backdrop(400, 260);
    let mut img = before.clone();
    paint_all(&mut img, std::slice::from_ref(&h), &st);
    for y in 0..260 {
        for x in 0..400 {
            let inside = h.pieces.iter().any(|p| p.contains(x, y));
            if !inside {
                assert_eq!(px(&img, x, y), px(&before, x, y), "({x}, {y}) is not the window's");
            }
        }
    }
    // The flat panel inside, away from the label and the edges.
    for (x, y) in [(60, 60), (60, 200), (300, 200), (200, 50)] {
        assert!(is_panel(px(&img, x, y), &st), "({x}, {y}): {:?}", px(&img, x, y));
    }
    // A rounded corner: its very corner pixel is the backdrop, the border
    // runs along the edge.
    assert_eq!(px(&img, 40, 30), px(&before, 40, 30));
    assert_eq!(px(&img, 40, 229), px(&before, 40, 229));
    assert!(!is_panel(px(&img, 40, 120), &st), "the border");
    assert!(is_panel(px(&img, 44, 120), &st));
    // The label: the badge, the lock and text in the middle of the
    // largest piece (the left part, under the cut's band).
    let l = place(crate::fonts::ui_font(), crate::tr!("capture.hidden.title"), crate::tr!("capture.hidden.body"), &h).unwrap();
    let mut ink = 0;
    for y in l.at.y0..l.at.y1 {
        for x in l.at.x0..l.at.x1 {
            ink += usize::from(!is_panel(px(&img, x, y), &st));
        }
    }
    assert!(ink > 400, "only {ink} label pixels");
}

#[test]
fn paint_is_the_same_in_bgra_and_in_a_window_of_the_image() {
    let st = Style::of(&LIGHT);
    let rect = rr(20, 10, 300, 180);
    let h = Hidden { rect, seen: rect, radius: 8.0, pieces: subtract(rect, rr(0, 0, 120, 60)), scale: 1.25 };
    let mut full = backdrop(360, 220);
    paint_all(&mut full, std::slice::from_ref(&h), &st);
    // The GDI overlay paints a BGRA box of the visible pieces only.
    let (x0, y0, w, hh) = (20usize, 10usize, 300usize, 180usize);
    let src = backdrop(360, 220);
    let mut sub = vec![0u8; w * hh * 4];
    for y in 0..hh {
        for x in 0..w {
            let p = src.get_pixel((x0 + x) as u32, (y0 + y) as u32);
            sub[(y * w + x) * 4..][..4].copy_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    paint(&mut sub, w, (x0 as i32, y0 as i32), Order::Bgra, &h, &st);
    for y in 0..hh {
        for x in 0..w {
            let p = full.get_pixel((x0 + x) as u32, (y0 + y) as u32);
            let q = &sub[(y * w + x) * 4..][..4];
            assert_eq!([q[2], q[1], q[0], q[3]], p, "({x}, {y})");
        }
    }
}

/// Every language's label fits a mid-sized window (a password manager's
/// picker, 420 × 260) at full size, both lines on one line each.
#[test]
fn the_label_fits_a_picker_in_every_language() {
    let f = crate::fonts::ui_font();
    for lang in preview::langs() {
        crate::i18n::with_lang(lang, || {
            let (t, b) = (crate::tr!("capture.hidden.title"), crate::tr!("capture.hidden.body"));
            for k in [1.0f32, 1.5, 2.0] {
                let room = rr(0, 0, (420.0 * k) as i32, (260.0 * k) as i32);
                let l = layout(f, t, b, k, room).unwrap_or_else(|| panic!("{lang}: no label"));
                assert!(l.title && l.body && (l.k - k).abs() < 1e-4, "{lang} at {k}: shrunk or cut: {l:?} ({t:?} / {b:?})");
            }
        });
    }
}

/// An app window drawn over the backdrop (a captured window above the
/// hidden one).
fn draw_window(img: &mut PixBuf, r: R, dark: bool) {
    let w = img.width() as usize;
    let mut fb = Fb::new(img.as_raw_mut(), w);
    let (body, bar) = if dark { (C4::rgb(0x2B, 0x2D, 0x33), C4::rgb(0x20, 0x22, 0x27)) } else { (C4::rgb(0xFF, 0xFF, 0xFF), C4::rgb(0xF0, 0xF1, 0xF4)) };
    fb.fill_rounded(r.x0, r.y0, r.w(), r.h(), 8.0, body);
    fb.fill_rounded(r.x0, r.y0, r.w(), 32, 8.0, bar);
    fb.fill_rect(r.x0, r.y0 + 16, r.w(), 16, bar);
    for i in 0..6 {
        let c = if dark { C4::rgb(0x45, 0x48, 0x50) } else { C4::rgb(0xDD, 0xDF, 0xE5) };
        fb.fill_rounded(r.x0 + 20, r.y0 + 56 + i * 28, r.w() * (9 - i) / 12, 12, 6.0, c);
    }
}

/// Placeholders as a capture shows them (RUSTSHOT_PREVIEW_DIR): light
/// and dark; a picker-sized window, a large one at 150 %, a small one,
/// one partly under another window.
#[test]
fn preview_hidden_window_pngs() {
    for (name, th) in [("dark", &DARK), ("light", &LIGHT)] {
        let st = Style::of(th);
        let mons = [mon(0, 0, 1600, 1000, 1.0)];
        // A picker (no rounding: a popup), and a small one beside it.
        let tops = [Top { r: rr(80, 80, 420, 260), hidden: Some(0.0) }, Top { r: rr(600, 120, 180, 120), hidden: Some(8.0) }, Top { r: rr(860, 100, 96, 72), hidden: Some(8.0) }];
        let mut img = backdrop(1000, 420);
        paint_all(&mut img, &plan(&tops, (0, 0), (1000, 420), &mons), &st);
        preview::save(&format!("hidden-picker-{name}.png"), &img);
        // A large framed window at 150 %, rounded.
        let mons = [mon(0, 0, 1800, 1200, 1.5)];
        let tops = [Top { r: rr(120, 90, 1400, 860), hidden: Some(12.0) }];
        let mut img = backdrop(1640, 1040);
        paint_all(&mut img, &plan(&tops, (0, 0), (1640, 1040), &mons), &st);
        preview::save(&format!("hidden-large-{name}.png"), &img);
        // Partly covered: an app window over its middle and right.
        let mons = [mon(0, 0, 1600, 1000, 1.0)];
        let over = rr(400, 140, 520, 330);
        let tops = [Top { r: over, hidden: None }, Top { r: rr(120, 80, 560, 360), hidden: Some(8.0) }];
        let mut img = backdrop(1000, 560);
        paint_all(&mut img, &plan(&tops, (0, 0), (1000, 560), &mons), &st);
        draw_window(&mut img, over, th.dark);
        preview::save(&format!("hidden-covered-{name}.png"), &img);
    }
    // The label in the long pseudo-languages and the fit catalogs.
    for lang in preview::langs() {
        crate::i18n::with_lang(lang, || {
            let tops = [Top { r: rr(20, 20, 420, 260), hidden: Some(8.0) }];
            let mut img = backdrop(460, 300);
            paint_all(&mut img, &plan(&tops, (0, 0), (460, 300), &[]), &Style::of(&DARK));
            preview::save(&format!("hidden-lang/{lang}.png"), &img);
        });
    }
}

#[cfg(windows)]
mod live {
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, SetWindowDisplayAffinity, ShowWindow, TranslateMessage, MSG, PM_REMOVE,
        SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    struct OwnToo;
    impl Drop for OwnToo {
        fn drop(&mut self) {
            OWN_TOO.store(false, Ordering::Relaxed);
            ON.store(true, Ordering::Relaxed);
        }
    }

    fn pump(ms: u64) {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            unsafe {
                let mut m = MSG::default();
                while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }

    /// The test window's pixels in a capture are the placeholder (where a
    /// plain grab has what is behind it), its middle the label.
    fn check(img: &PixBuf, at: R, st: &Style, what: &str) {
        let (x0, y0) = (at.x0, at.y0);
        let pts = [(10, 10), (at.w() - 12, 12), (12, at.h() - 12), (at.w() - 12, at.h() - 12), (20, at.h() / 2)];
        for (x, y) in pts {
            let p = px(img, x0 + x, y0 + y);
            assert!(is_panel(p, st), "{what}: ({x}, {y}) of the window is {p:?}, not the placeholder");
        }
        let (cx, cy) = (x0 + at.w() / 2, y0 + at.h() / 2);
        let ink = (cy - 40..cy + 40).flat_map(|y| (cx - 60..cx + 60).map(move |x| (x, y))).filter(|&(x, y)| !is_panel(px(img, x, y), st)).count();
        assert!(ink > 200, "{what}: {ink} label pixels");
    }

    /// A window of this test process hidden with WDA_EXCLUDEFROMCAPTURE:
    /// the editor's grab (memory) and the GDI overlay's bitmaps get the
    /// placeholder over it; with marking off they get what is behind.
    #[test]
    fn live_a_window_excluded_from_capture_gets_the_placeholder() {
        let _guard = crate::wind::test_window_lock();
        crate::capture::enable_dpi_awareness();
        let mons = crate::capture::monitors().expect("monitors");
        let (mi, m) = mons.iter().enumerate().find(|(_, m)| m.primary).unwrap_or((0, &mons[0]));
        let (w, h) = ((420.0 * m.scale) as i32, (260.0 * m.scale) as i32);
        let (x, y) = (m.x + (m.w as i32 - w) / 2, m.y + (m.h as i32 - h) / 3);
        let hwnd = unsafe {
            CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("STATIC"), w!(""), WS_POPUP, x, y, w, h, None, None, None, None)
                .expect("test window")
        };
        unsafe {
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).expect("exclude from capture");
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        pump(400);
        let _back = OwnToo;
        OWN_TOO.store(true, Ordering::Relaxed);
        let cfg = crate::config::Config::default();
        assert!(cfg.mark_hidden_windows, "on by default");
        configure(&cfg, &DARK);
        let st = Style::of(&DARK);
        let found = find((m.x, m.y), (m.w, m.h));
        let at = rr(x - m.x, y - m.y, w, h);
        let mine = found.iter().find(|f| f.rect == at);
        assert!(mine.is_some(), "the test window at {at:?} among {found:?}");
        assert_eq!(mine.unwrap().radius, 0.0, "a popup is square");

        let shot = crate::capture::grab_edit(Some(mi as u32), false).expect("grab");
        check(&shot.image, at, &st, "grab_edit");
        let gdi = crate::editor::gdi::GdiScreen::capture(shot.origin, shot.size, DARK.dim).expect("gdi");
        check(&gdi.plain_pixels(), at, &st, "GdiScreen::capture");
        drop(gdi);

        // Off: what is behind the window (not the placeholder's flat colors).
        let off = crate::config::Config { mark_hidden_windows: false, ..cfg };
        configure(&off, &DARK);
        assert!(find((m.x, m.y), (m.w, m.h)).is_empty(), "off");
        let plain = crate::capture::grab_edit(Some(mi as u32), false).expect("grab");
        let flat = [(10, 10), (20, h / 2), (w - 12, h - 12)].iter().filter(|&&(dx, dy)| is_panel(px(&plain.image, at.x0 + dx, at.y0 + dy), &st)).count();
        assert!(flat < 3, "with marking off the window's pixels are what is behind it");
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
