//! GDI renderer tests (Windows): painted pixels vs the software frame,
//! export, handle leaks, repaint timing. A child of `editor::tests`.

use super::*;
use compose::PxRect;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};

/// A top-down BGRA DIB section in a memory DC, standing in for the
/// overlay window.
struct Target {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    len: usize,
}

impl Target {
    fn new(w: u32, h: u32) -> Self {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).expect("dib");
            let old = SelectObject(dc, HGDIOBJ(bmp.0));
            let len = w as usize * h as usize * 4;
            std::ptr::write_bytes(bits as *mut u8, 0x5A, len);
            Target { dc, bmp, old, bits: bits as *mut u8, len }
        }
    }

    fn pixels(&self) -> &[u8] {
        unsafe {
            let _ = GdiFlush();
            std::slice::from_raw_parts(self.bits, self.len)
        }
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Hand the edit's capture to GDI bitmaps (as `grab` does).
fn attach(app: &mut App) {
    let e = edit_of(app);
    let dim = e.th.dim.with_alpha(e.th.dim_alpha(e.cfg.contrast_opacity));
    e.gdi = Some(gdi::GdiScreen::from_pixels(&e.base, dim).expect("gdi screen"));
    // Bake the committed objects into the annotation layer.
    e.rebuild();
}

/// Max channel difference between a BGRA paint and an RGBA frame (alpha
/// ignored: the window has none), where, and how many pixels differ.
fn max_diff(got: &[u8], want: &PixBuf) -> (u8, usize, usize) {
    let (mut m, mut at, mut n) = (0u8, 0usize, 0usize);
    for (i, (g, w)) in got.as_chunks::<4>().0.iter().zip(want.as_raw().as_chunks::<4>().0).enumerate() {
        let d = [g[2].abs_diff(w[0]), g[1].abs_diff(w[1]), g[0].abs_diff(w[2])].into_iter().max().unwrap();
        if d > 0 {
            n += 1;
        }
        if d > m {
            (m, at) = (d, i);
        }
    }
    (m, at, n)
}

fn check(got: &[u8], want: &PixBuf, name: &str, tol: u8) {
    let (m, at, n) = max_diff(got, want);
    let w = want.width() as usize;
    assert!(m <= tol, "{name}: max diff {m} at ({}, {}), {n} pixels differ", at % w, at / w);
}

/// Random tiling of the image (the update region of many paints).
fn tiles(size: (u32, u32), seed: u64) -> Vec<PxRect> {
    let (w, h) = (size.0 as i32, size.1 as i32);
    let mut rng = Rng(seed | 1);
    let mut v = Vec::new();
    let mut y = 0;
    while y < h {
        let cap = if rng.below(4) == 0 { 7 } else { 200 };
        let th = 1 + rng.below(cap) as i32;
        let mut x = 0;
        while x < w {
            let cap = if rng.below(4) == 0 { 9 } else { 400 };
            let tw = 1 + rng.below(cap) as i32;
            v.push(PxRect::new(x, y, (x + tw).min(w), (y + th).min(h)));
            x += tw;
        }
        y += th;
    }
    v
}

const BIG: FRect = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };

/// Every tiling state painted through GDI (whole window, then as random
/// tiles) equals the software frame.
#[test]
fn gdi_paint_matches_software_frame() {
    for (i, (name, mut app)) in tiling_states().into_iter().enumerate() {
        let want = settled(&mut app);
        attach(&mut app);
        let e = edit_of(&mut app);
        let t = Target::new(want.width(), want.height());
        e.paint_gdi(t.dc, &[PxRect::image(e.shot.size)]);
        check(t.pixels(), &want, name, 0);
        let t = Target::new(want.width(), want.height());
        e.paint_gdi(t.dc, &tiles(e.shot.size, 0xC0FFEE + i as u64));
        check(t.pixels(), &want, &format!("{name} (tiles)"), 0);
    }
}

/// Mid dim fade: AlphaBlend over the plain capture, within 1.
#[test]
fn gdi_paint_mid_fade_within_one() {
    for (name, mut app) in [("fade sel", annotated(theme::DARK, BIG)), ("fade hint", preview_app(theme::LIGHT, None))] {
        app.frame();
        let e = edit_of(&mut app);
        (e.mo.dim, e.mo.hint) = (mid(1.0), mid(1.0));
        let want = app.frame().expect("frame").clone();
        attach(&mut app);
        let e = edit_of(&mut app);
        let a = e.scene.as_ref().unwrap().dim_alpha;
        assert!(a > 0 && a < e.th.dim_alpha(e.cfg.contrast_opacity), "{name}: alpha {a} mid-fade");
        let t = Target::new(want.width(), want.height());
        e.paint_gdi(t.dc, &[PxRect::image(e.shot.size)]);
        check(t.pixels(), &want, name, 1);
    }
}

type Step = (&'static str, fn(&mut App, usize));

/// Hover toolbar item `i` (tooltip not shown yet: it waits 400 ms).
fn hover(a: &mut App, i: usize) {
    let e = edit_of(a);
    e.hover = Some(i);
    e.hover_at = Instant::now() + Duration::from_secs(60);
}

/// Incremental painting: paint, change, `damage()` (merged dirty rects),
/// paint only those; the window equals the software frame of the new state.
#[test]
fn gdi_damage_repaints_match() {
    let mut app = annotated(theme::DARK, BIG);
    let want0 = settled(&mut app);
    let n_items = edit_of(&mut app).toolbar.as_ref().unwrap().items.len();
    let mut sw = annotated(theme::DARK, BIG);
    settled(&mut sw);
    attach(&mut app);
    let t = Target::new(want0.width(), want0.height());
    let whole = [0, 0, want0.width() as i32, want0.height() as i32];
    assert_eq!(app.damage(), Some(vec![whole]), "first damage is the whole window");
    edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((want0.width(), want0.height()))]);
    check(t.pixels(), &want0, "initial", 0);
    let steps: [Step; 11] = [
        ("hover", |a, _| hover(a, 2)),
        ("tooltip", |a, _| edit_of(a).hover_at = Instant::now() - Duration::from_secs(2)),
        ("hover move", |a, n| hover(a, n - 1)),
        ("pressed", |a, _| edit_of(a).pressed = Some(3)),
        ("resize", |a, _| edit_of(a).sel = Some(FRect { x: 260.0, y: 150.0, w: 900.0, h: 500.0 })),
        ("draft", |a, _| {
            let e = edit_of(a);
            e.interact = Interact::Drawing { start: Pt::new(0.0, 0.0) };
            e.draft = Some(Obj::Pixelate { r: FRect { x: 500.0, y: 300.0, w: 200.0, h: 120.0 }, cell: 9.0 });
        }),
        ("pixelate grows", |a, _| {
            edit_of(a).draft = Some(Obj::Pixelate { r: FRect { x: 500.0, y: 300.0, w: 731.0, h: 427.5 }, cell: 9.0 });
        }),
        ("pixelate shrinks", |a, _| {
            edit_of(a).draft = Some(Obj::Pixelate { r: FRect { x: 500.0, y: 300.0, w: 412.0, h: 380.0 }, cell: 9.0 });
        }),
        ("pixelate anchor moves", |a, _| {
            edit_of(a).draft = Some(Obj::Pixelate { r: FRect { x: 230.0, y: 120.0, w: 682.0, h: 560.0 }, cell: 9.0 });
        }),
        ("commit", |a, _| {
            let e = edit_of(a);
            let d = e.draft.take().unwrap();
            e.interact = Interact::None;
            e.commit_object(d);
        }),
        ("undo", |a, _| edit_of(a).undo()),
    ];
    for (name, f) in steps {
        f(&mut app, n_items);
        f(&mut sw, n_items);
        let want = settled(&mut sw);
        // Two damages before one paint (the change starts tweens; settle
        // them), as when several events queue invalidations.
        let mut rects = app.damage().unwrap_or_else(|| vec![whole]);
        snap_all(&mut app);
        rects.extend(app.damage().unwrap_or_else(|| vec![whole]));
        assert!(!rects.is_empty(), "{name}: something changed");
        let area: i64 = rects.iter().map(|r| (r[2] - r[0]) as i64 * (r[3] - r[1]) as i64).sum();
        println!("{name}: {} rects, {area} px", rects.len());
        let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
        edit_of(&mut app).paint_gdi(t.dc, &rects);
        check(t.pixels(), &want, name, 0);
    }
}

/// Export from the bitmaps equals the crop of the composed image.
#[test]
fn gdi_export_matches_crop() {
    let mut cases: Vec<(&str, App, FRect)> = vec![
        ("plain", preview_app(theme::DARK, Some(BIG)), BIG),
        ("objects", annotated(theme::DARK, BIG), BIG),
        ("fractional", annotated(theme::DARK, BIG), FRect { x: 240.4, y: 140.6, w: 960.3, h: 540.2 }),
        ("edge", annotated(theme::DARK, BIG), FRect { x: 2300.0, y: 850.0, w: 300.0, h: 200.0 }),
        ("whole", annotated(theme::DARK, BIG), FRect { x: 0.0, y: 0.0, w: 2400.0, h: 900.0 }),
    ];
    let mut app = annotated(theme::LIGHT, BIG);
    let e = edit_of(&mut app);
    e.objects.push(Obj::Pixelate { r: FRect { x: 150.0, y: 400.0, w: 333.0, h: 120.0 }, cell: 11.0 });
    e.objects.push(Obj::Invert { r: FRect { x: 1100.0, y: 100.0, w: 200.0, h: 150.0 } });
    e.objects.push(Obj::Pixelate { r: FRect { x: 300.0, y: 450.0, w: 100.0, h: 300.0 }, cell: 7.0 });
    cases.push(("pixelate invert partly outside", app, BIG));
    // Translucent objects (one alpha per object: the arrow's shaft and
    // head, the text's glyphs share one mask).
    let mut app = annotated(theme::DARK, BIG);
    let e = edit_of(&mut app);
    let half = C4::new(20, 200, 90, 128);
    e.objects.push(Obj::Arrow { a: Pt::new(300.0, 700.0), b: Pt::new(900.0, 300.0), color: half, width: 12.0 });
    e.objects.push(Obj::Rect { r: FRect { x: 500.0, y: 200.0, w: 300.0, h: 200.0 }, color: half, width: 9.0 });
    e.objects.push(Obj::Text { fmt: Default::default(), pos: Pt::new(400.0, 500.0), text: "Half WW".into(), color: half, size: 40.0 });
    cases.push(("translucent", app, BIG));
    // Numbered tags (one cut by the selection edge, one with a tail), the
    // freehand marker, filled shapes (one translucent).
    let mut app = annotated(theme::DARK, BIG);
    let e = edit_of(&mut app);
    e.objects.push(Obj::Counter { c: Pt::new(245.0, 300.0), n: 1, color: C4::rgb(240, 68, 56), d: 40.0, tail: None });
    e.objects.push(Obj::Counter { c: Pt::new(600.5, 400.25), n: 23, color: C4::new(250, 220, 0, 170), d: 52.0, tail: Some(Pt::new(700.0, 520.0)) });
    e.objects.push(Obj::MarkerPath { pts: vec![Pt::new(300.0, 600.0), Pt::new(500.0, 640.0), Pt::new(800.0, 580.0)], color: C4::rgb(250, 220, 0).with_alpha(90), width: 18.0 });
    e.objects.push(Obj::RectFill { r: FRect { x: 820.0, y: 200.0, w: 300.0, h: 120.0 }, color: half, width: 5.0 });
    e.objects.push(Obj::EllipseFill { r: FRect { x: 1000.0, y: 500.0, w: 260.0, h: 140.0 }, color: C4::rgb(59, 130, 246), width: 3.0 });
    cases.push(("tags fills marker", app, BIG));
    for (name, mut app, sel) in cases {
        settled(&mut app);
        let e = edit_of(&mut app);
        let want = crop_to_image(e.composed(), sel);
        attach(&mut app);
        let got = edit_of(&mut app).export(sel);
        assert_eq!(got.dimensions(), want.dimensions(), "{name}");
        assert!(got == want, "{name}: export differs");
    }
}

/// GDI objects are released: 20 capture sessions leave the process' GDI
/// object count where it was. Run alone (other tests allocate GDI objects):
/// `cargo test gdi_no_leaks -- --ignored --test-threads=1`
#[test]
#[ignore = "GDI handle count; run alone"]
fn gdi_no_leaks() {
    use windows::Win32::System::Threading::{
        GetGuiResources, OpenProcess, GR_GDIOBJECTS, PROCESS_QUERY_INFORMATION,
    };
    // A real handle: the pseudo handle from GetCurrentProcess reads 0.
    let me = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, std::process::id()) }.expect("process handle");
    let count = || unsafe { GetGuiResources(me, GR_GDIOBJECTS) };
    let run = || {
        let mut app = annotated(theme::DARK, BIG);
        settled(&mut app);
        attach(&mut app);
        let e = edit_of(&mut app);
        let t = Target::new(e.shot.size.0, e.shot.size.1);
        e.paint_gdi(t.dc, &[PxRect::image(e.shot.size)]);
        let _ = e.export(BIG);
        count() // live: bitmaps, DCs, target
    };
    run(); // first-use allocations
    let before = count();
    let mut live = 0;
    for _ in 0..20 {
        live = live.max(run());
    }
    let after = count();
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(me) };
    println!("GDI objects: {before} -> {after} (up to {live} while a capture is open)");
    assert!(live > before, "the count sees the capture's objects");
    assert_eq!(before, after, "GDI objects leaked");
}

/// GDI repaint timing at 5120x1440 into an offscreen DIB: a full repaint
/// and a toolbar hover change (dirty rects only).
/// `cargo test --release perf_gdi_bench -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_gdi_bench() {
    let (w, h) = (5120u32, 1440u32);
    let src = synthetic_shot();
    let mut img = PixBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (y as usize * w as usize + x as usize) * 4;
            let j = ((y % src.size.1) as usize * src.size.0 as usize + (x % src.size.0) as usize) * 4;
            img.as_raw_mut()[i..i + 4].copy_from_slice(&src.image.as_raw()[j..j + 4]);
        }
    }
    let shot = Shot { origin: (0, 0), size: (w, h), scale: 1.0, image: img, monitors: vec![(0, 0, w, h)] };
    let sel = FRect { x: 1000.0, y: 200.0, w: 2000.0, h: 900.0 };
    let median = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    for (name, s, objs) in [("A no sel, hint", None, false), ("B sel + toolbar", Some(sel), false), ("B + rect", Some(sel), true)] {
        let shot = Shot { image: shot.image.clone(), monitors: shot.monitors.clone(), ..shot };
        let mut app = preview_app_with(theme::DARK, s, shot);
        if objs {
            edit_of(&mut app).commit_object(one_rect(sel));
        }
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let whole = [PxRect::image((w, h))];
        let mut tf = Vec::new();
        for _ in 0..20 {
            let t0 = Instant::now();
            let _ = app.damage();
            edit_of(&mut app).paint_gdi(t.dc, &whole);
            let _ = t.pixels(); // GdiFlush
            tf.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        let (mut ts, mut area) = (Vec::new(), 0i64);
        if s.is_some() {
            let n = edit_of(&mut app).toolbar.as_ref().unwrap().items.len();
            for k in 0..20 {
                edit_of(&mut app).hover = Some(k % n);
                let t0 = Instant::now();
                let rects = app.damage().expect("small damage");
                let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
                edit_of(&mut app).paint_gdi(t.dc, &rects);
                let _ = t.pixels();
                ts.push(t0.elapsed().as_secs_f64() * 1000.0);
                area = rects.iter().map(|r| r.w() as i64 * r.h() as i64).sum();
            }
        }
        let small = if ts.is_empty() { 0.0 } else { median(ts) };
        println!("GDI BENCH {name}: full repaint {:.2} ms, hover repaint {small:.3} ms ({area} px)", median(tf));
    }
}

/// A big image for the benches: the synthetic shot tiled to `w` x `h`.
fn tiled_shot(w: u32, h: u32) -> Shot {
    let src = synthetic_shot();
    let mut img = PixBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (y as usize * w as usize + x as usize) * 4;
            let j = ((y % src.size.1) as usize * src.size.0 as usize + (x % src.size.0) as usize) * 4;
            img.as_raw_mut()[i..i + 4].copy_from_slice(&src.image.as_raw()[j..j + 4]);
        }
    }
    Shot { origin: (0, 0), size: (w, h), scale: 1.0, image: img, monitors: vec![(0, 0, w, h)] }
}

/// Pixelate drag at 5120x1440: each step resizes the draft by a few
/// pixels, then `damage()` + paint of the damaged rects (what one mouse
/// move costs). Full-screen and 2000x900 drafts.
/// `cargo test --release perf_gdi_pixelate_drag -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_gdi_pixelate_drag() {
    let (w, h) = (5120u32, 1440u32);
    let full = FRect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 };
    let mid = FRect { x: 1000.0, y: 200.0, w: 2000.0, h: 900.0 };
    let cases = [("full-screen", full, FRect { x: 2.0, y: 2.0, w: 5110.0, h: 1430.0 }), ("2000x900", mid, mid)];
    for ((name, sel, r), moving) in cases.into_iter().flat_map(|c| [(c, false), (c, true)]) {
        let mut app = preview_app_with(theme::DARK, Some(sel), tiled_shot(w, h));
        edit_of(&mut app).tool = Some(Tool::Pixelate);
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let _ = app.damage();
        edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
        let mut ts = Vec::new();
        for k in 0..24 {
            let e = edit_of(&mut app);
            e.interact = Interact::Drawing { start: Pt::new(r.x, r.y) };
            let d = (k % 6) as f32 * 3.0;
            // Corner drag (anchor fixed), or the anchor moving too (the
            // whole draft repaints: the worst case).
            let a = if moving { (k % 2) as f32 } else { 0.0 };
            e.draft = Some(Obj::Pixelate { r: FRect { x: r.x + a, y: r.y + a, w: r.w - d, h: r.h - d }, cell: 12.0 });
            let t0 = Instant::now();
            let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
            let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
            edit_of(&mut app).paint_gdi(t.dc, &rects);
            let _ = t.pixels();
            ts.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let how = if moving { "anchor moving" } else { "corner drag" };
        println!("GDI BENCH pixelate drag {name} ({how}): median {:.2} ms, max {:.2} ms", ts[ts.len() / 2], ts[ts.len() - 1]);
    }
}

/// Every object kind, several partly outside `BIG` (and one past the
/// image edge), committed one at a time.
fn object_set(font: bool) -> Vec<Obj> {
    let red = C4::rgb(240, 68, 56);
    let mut v = vec![
        Obj::Rect { r: FRect { x: 200.0, y: 100.0, w: 300.0, h: 200.0 }, color: red, width: 3.0 },
        Obj::Arrow { a: Pt::new(1150.0, 600.0), b: Pt::new(1300.0, 760.0), color: C4::rgb(20, 200, 90), width: 6.0 },
        Obj::Marker { a: Pt::new(260.0, 650.0), b: Pt::new(700.0, 700.0), color: C4::rgb(250, 220, 0).with_alpha(90), width: 20.0 },
        Obj::Pixelate { r: FRect { x: 150.0, y: 400.0, w: 333.0, h: 120.0 }, cell: 11.0 },
        Obj::Invert { r: FRect { x: 1100.0, y: 100.0, w: 200.0, h: 150.0 } },
        Obj::Pixelate { r: FRect { x: 300.0, y: 450.0, w: 100.0, h: 300.0 }, cell: 7.0 },
        Obj::Ellipse { r: FRect { x: 1350.0, y: 820.0, w: 200.0, h: 150.0 }, color: red, width: 4.0 },
        Obj::Path { pts: vec![Pt::new(600.0, 300.0), Pt::new(640.0, 340.0), Pt::new(700.0, 310.0)], color: red, width: 3.0 },
        // A blur across the selection's edge, one over a pixelate.
        Obj::Blur { r: FRect { x: 180.0, y: 120.0, w: 260.5, h: 140.0 }, sigma: 8.0 },
        Obj::Blur { r: FRect { x: 320.0, y: 420.0, w: 120.0, h: 200.0 }, sigma: 14.0 },
    ];
    if font {
        v.push(Obj::Text { fmt: Default::default(), pos: Pt::new(1180.0, 200.0), text: "Note\nhere".into(), color: red, size: 24.0 });
        let fmt = crate::text::TextFmt { bold: true, italic: true, underline: true, strike: true, align: crate::text::Align::Center, family: crate::text::Family::Serif };
        v.push(Obj::Text { fmt, pos: Pt::new(800.0, 160.0), text: "Styled\ncentre".into(), color: red, size: 28.0 });
    }
    v
}

/// Committed objects of every kind (partly outside the selection), added
/// one by one (the layer grows in place), then undone to zero: each state
/// repainted through `damage()` and painted whole equals the software
/// frame, and the export equals the software crop.
#[test]
fn gdi_layer_matches_software_through_commits_and_undo() {
    let mut app = preview_app(theme::DARK, Some(BIG));
    let mut sw = preview_app(theme::DARK, Some(BIG));
    let objs = object_set(edit_of(&mut app).font.is_some());
    settled(&mut app);
    settled(&mut sw);
    attach(&mut app);
    let (w, h) = edit_of(&mut app).shot.size;
    let t = Target::new(w, h);
    let _ = app.damage();
    edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
    let whole = [0, 0, w as i32, h as i32];
    let step = |app: &mut App, sw: &mut App, name: &str| {
        let want = settled(sw);
        let rects = app.damage().unwrap_or_else(|| vec![whole]);
        let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
        edit_of(app).paint_gdi(t.dc, &rects);
        check(t.pixels(), &want, &format!("{name} (damage)"), 0);
        let t2 = Target::new(w, h);
        edit_of(app).paint_gdi(t2.dc, &tiles((w, h), name.len() as u64));
        check(t2.pixels(), &want, &format!("{name} (tiles)"), 0);
        let es = edit_of(sw);
        let crop = crop_to_image(es.composed(), BIG);
        let wide = FRect { x: 100.0, y: 50.0, w: 1500.0, h: 840.0 };
        let crop_wide = crop_to_image(es.composed(), wide);
        let e = edit_of(app);
        assert!(e.export(BIG) == crop, "{name}: export differs");
        assert!(e.export(wide) == crop_wide, "{name}: wide export differs");
    };
    for (i, o) in objs.iter().enumerate() {
        edit_of(&mut app).commit_object(o.clone());
        edit_of(&mut sw).commit_object(o.clone());
        step(&mut app, &mut sw, &format!("commit {i}"));
    }
    for i in 0..objs.len() {
        edit_of(&mut app).undo();
        edit_of(&mut sw).undo();
        step(&mut app, &mut sw, &format!("undo {i}"));
    }
    let e = edit_of(&mut app);
    assert!(e.objects.is_empty());
    assert_eq!(e.gdi.as_ref().unwrap().layer_rect(), None, "no objects, no layer");
    for i in 0..3 {
        edit_of(&mut app).redo();
        edit_of(&mut sw).redo();
        step(&mut app, &mut sw, &format!("redo {i}"));
    }
}

/// The layer covers the union of the object bounds, clipped to the image.
#[test]
fn gdi_layer_is_the_union_of_object_bounds() {
    let mut app = preview_app(theme::DARK, Some(BIG));
    settled(&mut app);
    attach(&mut app);
    let e = edit_of(&mut app);
    assert_eq!(e.gdi.as_ref().unwrap().layer_rect(), None);
    e.commit_object(Obj::Rect { r: FRect { x: 100.0, y: 100.0, w: 50.0, h: 40.0 }, color: C4::rgb(1, 2, 3), width: 2.0 });
    e.rebuild();
    // Width 2: margin 1 + 2 = 3 px each side.
    assert_eq!(e.gdi.as_ref().unwrap().layer_rect(), Some(PxRect::new(97, 97, 153, 143)));
    e.commit_object(Obj::Invert { r: FRect { x: 1400.0, y: 880.0, w: 100.0, h: 100.0 } });
    e.rebuild();
    let (w, h) = (e.shot.size.0 as i32, e.shot.size.1 as i32);
    assert_eq!(e.gdi.as_ref().unwrap().layer_rect(), Some(PxRect::new(97, 97, w, h)));
}

/// A pixelate draft straddling many bands and tiles stays exact; its
/// scratch is about one band, and released when the draft ends.
#[test]
fn pixelate_draft_scratch_is_small_and_released() {
    let mut app = annotated(theme::DARK, BIG);
    let e = edit_of(&mut app);
    e.interact = Interact::Drawing { start: Pt::new(0.0, 0.0) };
    e.draft = Some(Obj::Pixelate { r: FRect { x: 180.0, y: 100.0, w: 700.5, h: 500.0 }, cell: 13.0 });
    let want = settled(&mut app);
    attach(&mut app);
    let _ = app.damage();
    let e = edit_of(&mut app);
    let t = Target::new(want.width(), want.height());
    e.paint_gdi(t.dc, &[PxRect::image(e.shot.size)]);
    check(t.pixels(), &want, "pixelate draft whole", 0);
    e.paint_gdi(t.dc, &tiles(e.shot.size, 7));
    check(t.pixels(), &want, "pixelate draft tiles", 0);
    let cap = e.draft_buf.borrow().capacity();
    assert!(cap > 0 && cap < 2400 * 200 * 4, "scratch {cap} bytes: about a band");
    e.draft = None;
    e.interact = Interact::None;
    let _ = app.damage();
    assert_eq!(edit_of(&mut app).draft_buf.borrow().capacity(), 0, "released after the drag");
}

/// Selection drags at 5120x1440 (resize by the SE handle, move, a new
/// selection): per mouse move, `pointer_move` + `damage()` + paint of the
/// damaged rects, as the overlay runs them.
/// `cargo test --release perf_gdi_sel_drag -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_gdi_sel_drag() {
    let (w, h) = (5120u32, 1440u32);
    let sel = FRect { x: 1000.0, y: 200.0, w: 2000.0, h: 900.0 };
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * q) as usize]
    };
    for name in ["resize SE", "move", "new sel"] {
        let mut app = preview_app_with(theme::DARK, Some(sel), tiled_shot(w, h));
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let _ = app.damage();
        edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
        let start = match name {
            "resize SE" => Pt::new(sel.x + sel.w, sel.y + sel.h),
            "move" => Pt::new(sel.x + 50.0, sel.y + 50.0),
            _ => Pt::new(300.0, 100.0),
        };
        edit_of(&mut app).interact = match name {
            "resize SE" => Interact::Resize { handle: Handle::SE, orig: sel, aspect: sel.w / sel.h },
            "move" => Interact::MoveSel { start, orig: sel },
            _ => Interact::NewSel { anchor: start, moved: false },
        };
        let (mut tm, mut td, mut tp, mut tt, mut n, mut area) = (vec![], vec![], vec![], vec![], vec![], vec![]);
        crate::perf::force_on();
        let _ = crate::perf::take();
        for k in 0..300 {
            let (dx, dy) = ((k % 150) as f32 * 3.0, (k % 150) as f32 * 1.0);
            let (x, y) = ((start.x + dx) as i32, (start.y + dy) as i32);
            let t0 = Instant::now();
            app.mouse = (x, y);
            app.pointer_move(x, y);
            let t1 = Instant::now();
            let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
            let t2 = Instant::now();
            let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
            edit_of(&mut app).paint_gdi(t.dc, &rects);
            let _ = t.pixels();
            let t3 = Instant::now();
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
            tm.push(ms(t0, t1));
            td.push(ms(t1, t2));
            tp.push(ms(t2, t3));
            tt.push(ms(t0, t3));
            n.push(rects.len() as f64);
            area.push(rects.iter().map(|r| r.w() as f64 * r.h() as f64).sum::<f64>());
        }
        println!(
            "GDI BENCH sel drag {name}: total median {:.2} p90 {:.2} max {:.2} ms | move {:.3} damage {:.3} paint {:.2} ms | rects {:.0} area {:.0} px (max {:.0})",
            pct(&mut tt.clone(), 0.5), pct(&mut tt.clone(), 0.9), pct(&mut tt, 1.0),
            pct(&mut tm, 0.5), pct(&mut td, 0.5), pct(&mut tp, 0.5),
            pct(&mut n, 0.5), pct(&mut area.clone(), 0.5), pct(&mut area, 1.0)
        );
        let per: Vec<String> = crate::perf::NAMES
            .iter()
            .zip(crate::perf::take())
            .map(|(k, (ms, c))| format!("{k} {:.3} ms ({:.1} calls)", ms / 300.0, c as f64 / 300.0))
            .collect();
        println!("  per move: {}", per.join(", "));
    }
}

/// Object edits through the GDI layer (moves, a resize, a loupe, a
/// restack, a delete, spotlights) paint exactly the software frame,
/// through the damage and as random tiles, and export the software crop.
#[test]
fn gdi_layer_matches_software_through_object_edits() {
    use crate::objects::edit as oe;
    let mut app = preview_app(theme::DARK, Some(BIG));
    let mut sw = preview_app(theme::DARK, Some(BIG));
    let mut objs = object_set(edit_of(&mut app).font.is_some());
    objs.push(Obj::Magnify { src: Pt::new(400.0, 450.0), dst: Pt::new(800.0, 300.0), radius: 80.0, zoom: 2.0, color: C4::rgb(255, 255, 255), width: 3.0 });
    objs.push(Obj::Spotlight { r: FRect { x: 300.0, y: 200.0, w: 500.0, h: 400.0 } });
    for o in &objs {
        edit_of(&mut app).commit_object(o.clone());
        edit_of(&mut sw).commit_object(o.clone());
    }
    settled(&mut app);
    settled(&mut sw);
    attach(&mut app);
    let (w, h) = edit_of(&mut app).shot.size;
    let t = Target::new(w, h);
    let _ = app.damage();
    edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
    let whole = [0, 0, w as i32, h as i32];
    type Step = Box<dyn Fn(&mut Vec<Obj>)>;
    let edits: Vec<(&str, Step)> = vec![
        ("move rect", Box::new(|o| oe::translate(&mut o[0], 40.0, 25.0))),
        ("move past the layer", Box::new(|o| oe::translate(&mut o[1], -900.0, -500.0))),
        ("move under the pixelate", Box::new(|o| oe::translate(&mut o[2], 30.0, -230.0))),
        ("loupe source", Box::new(|o| {
            let n = o.len() - 2;
            oe::set_point(&mut o[n], oe::Point::Src, Pt::new(330.0, 120.0))
        })),
        ("move spotlight", Box::new(|o| {
            let n = o.len() - 1;
            oe::translate(&mut o[n], 120.0, 60.0)
        })),
        ("resize", Box::new(|o| oe::map_box(&mut o[4], FRect { x: 1100.0, y: 100.0, w: 200.0, h: 150.0 }, FRect { x: 1000.0, y: 90.0, w: 300.0, h: 260.0 }))),
        ("move blur", Box::new(|o| oe::translate(&mut o[8], 30.0, -20.0))),
        ("blur strength", Box::new(|o| oe::set_size(&mut o[9], 20.0))),
        ("blur resize", Box::new(|o| oe::map_box(&mut o[9], FRect { x: 320.0, y: 420.0, w: 120.0, h: 200.0 }, FRect { x: 300.0, y: 380.0, w: 210.0, h: 150.0 }))),
        ("text style", Box::new(|o| {
            if let Some(t) = o.iter_mut().rfind(|t| matches!(t, Obj::Text { .. })) {
                let f = oe::fmt(t).unwrap_or_default();
                oe::set_fmt(t, crate::text::TextFmt { bold: !f.bold, align: crate::text::Align::Right, family: crate::text::Family::Mono, ..f });
            }
        })),
        ("to the back", Box::new(|o| {
            let x = o.remove(3);
            o.insert(0, x);
        })),
        ("delete", Box::new(|o| {
            o.remove(1);
        })),
        ("no spotlight", Box::new(|o| o.retain(|o| !o.is_spot()))),
    ];
    for (name, f) in &edits {
        for a in [&mut app, &mut sw] {
            f(&mut edit_of(a).objects);
            edit_of(a).dirty = true;
        }
        let want = settled(&mut sw);
        let rects = app.damage().unwrap_or_else(|| vec![whole]);
        let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
        edit_of(&mut app).paint_gdi(t.dc, &rects);
        check(t.pixels(), &want, &format!("{name} (damage)"), 0);
        let t2 = Target::new(w, h);
        edit_of(&mut app).paint_gdi(t2.dc, &tiles((w, h), name.len() as u64));
        check(t2.pixels(), &want, &format!("{name} (tiles)"), 0);
        let crop = edit_of(&mut sw).export(BIG);
        assert!(edit_of(&mut app).export(BIG) == crop, "{name}: export differs");
    }
}

/// Blur at 3840x2160 (target: an 800x400 blur under 4 ms a move): moving a
/// committed blur with the select tool (pointer_move + damage + paint),
/// and a draft's corner drag (damage + paint), sigma 10 and 25.
/// `cargo test --release perf_gdi_blur_drag -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_gdi_blur_drag() {
    let (w, h) = (3840u32, 2160u32);
    let sel = FRect { x: 200.0, y: 150.0, w: 3400.0, h: 1850.0 };
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * q) as usize]
    };
    for sigma in [10.0f32, 25.0] {
        let r = FRect { x: 1000.0, y: 600.0, w: 800.0, h: 400.0 };
        // A committed blur moved.
        let mut app = preview_app_with(theme::DARK, Some(sel), tiled_shot(w, h));
        edit_of(&mut app).commit_object(Obj::Blur { r, sigma });
        edit_of(&mut app).tool = Some(Tool::Select);
        edit_of(&mut app).obj.sel = vec![0];
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let _ = app.damage();
        edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
        let from = Pt::new(1400.0, 800.0);
        app.mouse = (from.x as i32, from.y as i32);
        app.pointer_down(from.x as i32, from.y as i32);
        let mut tt = Vec::new();
        for k in 0..100 {
            let d = (k % 50) as f32 * 2.0;
            let (x, y) = ((from.x + d) as i32, (from.y + d * 0.5) as i32);
            let t0 = Instant::now();
            app.mouse = (x, y);
            app.pointer_move(x, y);
            let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
            let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
            edit_of(&mut app).paint_gdi(t.dc, &rects);
            let _ = t.pixels();
            tt.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        app.pointer_up();
        println!("GDI BENCH 4K blur 800x400 sigma {sigma} move: median {:.2} p90 {:.2} max {:.2} ms", pct(&mut tt.clone(), 0.5), pct(&mut tt.clone(), 0.9), pct(&mut tt, 1.0));
        // A draft's corner dragged.
        let mut app = preview_app_with(theme::DARK, Some(sel), tiled_shot(w, h));
        edit_of(&mut app).tool = Some(Tool::Blur);
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let _ = app.damage();
        edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
        let mut tt = Vec::new();
        for k in 0..60 {
            let e = edit_of(&mut app);
            e.interact = Interact::Drawing { start: Pt::new(r.x, r.y) };
            let d = (k % 6) as f32 * 3.0;
            e.draft = Some(Obj::Blur { r: FRect { w: r.w - d, h: r.h - d, ..r }, sigma });
            let t0 = Instant::now();
            let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
            let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
            edit_of(&mut app).paint_gdi(t.dc, &rects);
            let _ = t.pixels();
            tt.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        println!("GDI BENCH 4K blur 800x400 sigma {sigma} draft drag: median {:.2} p90 {:.2} max {:.2} ms", pct(&mut tt.clone(), 0.5), pct(&mut tt.clone(), 0.9), pct(&mut tt, 1.0));
    }
    // The blur alone (no compose): one 800x400 block.
    let mut block = vec![128u8; 800 * 400 * 4];
    for (i, v) in block.iter_mut().enumerate() {
        *v = (i * 7 % 251) as u8;
    }
    let (mut tmp, mut acc) = (Vec::new(), Vec::new());
    let mut tt = Vec::new();
    for _ in 0..30 {
        let t0 = Instant::now();
        crate::objects::blur::blur_block(&mut block, 800, 400, 10.0, &mut tmp, &mut acc);
        tt.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    println!("BENCH blur_block 800x400 sigma 10: median {:.2} ms", pct(&mut tt, 0.5));
}

/// Object drags at 3840x2160 (the 4K perf target: under 4 ms a move):
/// per mouse move `pointer_move` + `damage()` + paint of the damaged
/// rects, with 27 objects on screen, dragging a rectangle, a pixelate, an
/// arrow end, a callout side handle and a spotlight.
/// `cargo test --release perf_gdi_object_drag -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_gdi_object_drag() {
    let (w, h) = (3840u32, 2160u32);
    let sel = FRect { x: 200.0, y: 150.0, w: 3400.0, h: 1850.0 };
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * q) as usize]
    };
    let red = C4::rgb(240, 68, 56);
    let mut objs = Vec::new();
    for i in 0..24u32 {
        let (x, y) = (300.0 + (i % 6) as f32 * 520.0, 300.0 + (i / 6) as f32 * 400.0);
        objs.push(match i % 4 {
            0 => Obj::Rect { r: FRect { x, y, w: 300.0, h: 180.0 }, color: red, width: 4.0 },
            1 => Obj::Arrow { a: Pt::new(x, y), b: Pt::new(x + 300.0, y + 150.0), color: red, width: 6.0 },
            2 => Obj::Pixelate { r: FRect { x, y, w: 260.0, h: 200.0 }, cell: 12.0 },
            _ => Obj::Counter { c: Pt::new(x + 40.0, y + 40.0), n: i, color: red, d: 40.0, tail: None },
        });
    }
    objs.push(Obj::Callout {
        fmt: Default::default(),
        r: FRect { x: 1500.0, y: 1700.0, w: 420.0, h: 120.0 },
        tail: Pt::new(1520.0, 1900.0),
        text: "Move this to the top, above the summary, so people see it first".into(),
        color: red,
        size: 24.0,
        style: crate::objects::CalloutStyle::Rounded,
        fill: true,
    });
    objs.push(Obj::Magnify { src: Pt::new(900.0, 900.0), dst: Pt::new(2600.0, 1750.0), radius: 140.0, zoom: 2.0, color: C4::rgb(255, 255, 255), width: 4.0 });
    objs.push(Obj::Spotlight { r: FRect { x: 250.0, y: 250.0, w: 1600.0, h: 900.0 } });
    let n = objs.len();
    for (name, idx, from) in [
        ("move rect", 0usize, Pt::new(450.0, 300.0)),
        ("move pixelate", 2, Pt::new(1500.0, 400.0)),
        ("arrow end", 1, Pt::new(1120.0, 450.0)),
        ("callout side", n - 3, Pt::new(1500.0 + 420.0 + 4.0, 1760.0)),
        ("move spotlight", n - 1, Pt::new(1000.0, 250.0)),
        // The capture's selection moved with a spotlight on it: the dim
        // follows (its edges re-baked).
        ("capture selection (spotlight)", usize::MAX, Pt::new(3500.0, 1900.0)),
    ] {
        let mut app = preview_app_with(theme::DARK, Some(sel), tiled_shot(w, h));
        for o in &objs {
            edit_of(&mut app).commit_object(o.clone());
        }
        if idx != usize::MAX {
            edit_of(&mut app).tool = Some(Tool::Select);
            edit_of(&mut app).obj.sel = vec![idx];
        }
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let _ = app.damage();
        edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
        app.mouse = (from.x as i32, from.y as i32);
        app.pointer_down(from.x as i32, from.y as i32);
        assert!(!matches!(edit_of(&mut app).interact, Interact::None), "{name}: the press took the object");
        let (mut tt, mut area) = (vec![], vec![]);
        crate::perf::force_on();
        let _ = crate::perf::take();
        for k in 0..200 {
            let d = (k % 100) as f32 * 2.0;
            let (x, y) = ((from.x + d) as i32, (from.y + d * 0.5) as i32);
            let t0 = Instant::now();
            app.mouse = (x, y);
            app.pointer_move(x, y);
            let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
            // Through a GDI region, as WM_PAINT gets them.
            let rects = crate::wind::paint_rects_for(&rects);
            let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
            edit_of(&mut app).paint_gdi(t.dc, &rects);
            let _ = t.pixels();
            tt.push(t0.elapsed().as_secs_f64() * 1000.0);
            area.push(rects.iter().map(|r| r.w() as f64 * r.h() as f64).sum::<f64>());
            if k == 50 && std::env::var_os("RUSTSHOT_BENCH_RECTS").is_some() {
                println!("  rects at move 50: {rects:?}");
            }
        }
        app.pointer_up();
        println!(
            "GDI BENCH 4K object drag {name}: median {:.2} p90 {:.2} max {:.2} ms | area median {:.0} px",
            pct(&mut tt.clone(), 0.5),
            pct(&mut tt.clone(), 0.9),
            pct(&mut tt, 1.0),
            pct(&mut area, 0.5)
        );
        let per: Vec<String> = crate::perf::NAMES
            .iter()
            .zip(crate::perf::take())
            .map(|(k, (ms, c))| format!("{k} {:.3} ms ({:.1} calls)", ms / 200.0, c as f64 / 200.0))
            .collect();
        println!("  per move: {}", per.join(", "));
    }
    // The first spotlight comes and goes (undo): the selection dims at
    // once, re-baked into the layer and painted from it.
    let mut app = preview_app_with(theme::DARK, Some(sel), tiled_shot(w, h));
    for o in objs.iter().filter(|o| !o.is_spot()) {
        edit_of(&mut app).commit_object(o.clone());
    }
    settled(&mut app);
    attach(&mut app);
    let t = Target::new(w, h);
    let _ = app.damage();
    edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
    let mut times = Vec::new();
    for k in 0..10 {
        if k % 2 == 0 {
            edit_of(&mut app).commit_object(objs[n - 1].clone());
        } else {
            edit_of(&mut app).undo();
        }
        let t0 = Instant::now();
        let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
        let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
        edit_of(&mut app).paint_gdi(t.dc, &rects);
        let _ = t.pixels();
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    println!("GDI BENCH 4K first spotlight add/undo: median {:.2} max {:.2} ms", pct(&mut times.clone(), 0.5), pct(&mut times, 1.0));
}

/// The magnifier at 3840x2160 (scale 1.5) through the GDI renderer: per
/// pointer move, the driver's move handling + `damage()` + paint of the
/// damaged rects, before a selection (hover) and while drawing the
/// region, with the magnifier off (before) and on.
/// `cargo test --release perf_gdi_magnifier -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_gdi_magnifier() {
    let (w, h) = (3840u32, 2160u32);
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * q) as usize]
    };
    for (name, on, drag) in [("hover", false, false), ("hover", true, false), ("new sel", false, true), ("new sel", true, true)] {
        let shot = Shot { scale: 1.5, ..tiled_shot(w, h) };
        let mut app = preview_app_with(theme::DARK, None, shot);
        edit_of(&mut app).cfg.magnifier = on;
        edit_of(&mut app).assist.dismiss_hint(Instant::now());
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let _ = app.damage();
        edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
        if drag {
            app.mouse = (400, 300);
            app.pointer_down(400, 300);
        }
        let (mut tt, mut area, mut nr) = (vec![], vec![], vec![]);
        crate::perf::force_on();
        let _ = crate::perf::take();
        for k in 0..300 {
            let (x, y) = (500 + (k % 150) * 7, 400 + (k % 150) * 4);
            let t0 = Instant::now();
            app.mouse = (x, y);
            let p = Pt::new(x as f32, y as f32);
            if drag {
                app.pointer_move(x, y);
            } else {
                let e = edit_of(&mut app);
                let _ = e.track_hover(p) | e.track_assist(p);
            }
            let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
            let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
            edit_of(&mut app).paint_gdi(t.dc, &rects);
            let _ = t.pixels();
            tt.push((Instant::now() - t0).as_secs_f64() * 1000.0);
            area.push(rects.iter().map(|r| r.w() as f64 * r.h() as f64).sum::<f64>());
            nr.push(rects.len() as f64);
        }
        println!(
            "GDI BENCH 4K {name}, magnifier {}: median {:.2} p90 {:.2} max {:.2} ms | rects {:.0} (max {:.0}) damage {:.0} px",
            if on { "on" } else { "off" },
            pct(&mut tt.clone(), 0.5),
            pct(&mut tt.clone(), 0.9),
            pct(&mut tt, 1.0),
            pct(&mut nr.clone(), 0.5),
            pct(&mut nr, 1.0),
            pct(&mut area, 0.5)
        );
        let per: Vec<String> = crate::perf::NAMES
            .iter()
            .zip(crate::perf::take())
            .filter(|(_, (ms, _))| *ms > 0.0)
            .map(|(k, (ms, c))| format!("{k} {:.3} ms ({:.1})", ms / 300.0, c as f64 / 300.0))
            .collect();
        println!("  per move: {}", per.join(", "));
    }
}

/// The wheel resizing the tool (Line, Magnify) and a selected loupe at
/// 3840x2160 (scale 1.5): per notch, the driver's wheel handling +
/// `damage()` + paint of the damaged rects; and how far a quick scroll
/// (notches back to back, as a flick sends them) moves the size.
/// `cargo test --release perf_gdi_wheel_size -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_gdi_wheel_size() {
    let (w, h) = (3840u32, 2160u32);
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * q) as usize]
    };
    let sel = FRect { x: 400.0, y: 300.0, w: 2400.0, h: 1400.0 };
    for (name, tool, obj) in [("line", Tool::Line, false), ("magnify", Tool::Magnify, false), ("selected loupe", Tool::Magnify, true)] {
        let shot = Shot { scale: 1.5, ..tiled_shot(w, h) };
        let mut app = preview_app_with(theme::DARK, Some(sel), shot);
        {
            let e = edit_of(&mut app);
            e.assist.dismiss_hint(Instant::now());
            e.tool = Some(tool);
            if obj {
                let c = C4::rgb(255, 0, 0);
                e.commit_object(Obj::Magnify { src: Pt::new(900.0, 700.0), dst: Pt::new(1600.0, 1000.0), radius: 60.0, zoom: 2.0, color: c, width: 3.0 });
                e.obj.sel = vec![0];
            }
        }
        settled(&mut app);
        attach(&mut app);
        let t = Target::new(w, h);
        let _ = app.damage();
        edit_of(&mut app).paint_gdi(t.dc, &[PxRect::image((w, h))]);
        let size = |app: &mut App| {
            let e = edit_of(app);
            if obj { crate::objects::edit::size(&e.objects[0]).unwrap() } else { e.mem.size(tool) }
        };
        let (mut tt, mut th, mut area) = (vec![], vec![], vec![]);
        crate::perf::force_on();
        let _ = crate::perf::take();
        for k in 0..120 {
            let delta = if (k / 30) % 2 == 0 { 120 } else { -120 };
            let t0 = Instant::now();
            app.on_event(Ev::Wheel { delta, x: 1600, y: 1000 });
            th.push(t0.elapsed().as_secs_f64() * 1000.0);
            let rects = app.damage().unwrap_or_else(|| vec![[0, 0, w as i32, h as i32]]);
            let rects: Vec<PxRect> = rects.iter().map(|r| PxRect::new(r[0], r[1], r[2], r[3])).collect();
            edit_of(&mut app).paint_gdi(t.dc, &rects);
            let _ = t.pixels();
            tt.push(t0.elapsed().as_secs_f64() * 1000.0);
            area.push(rects.iter().map(|r| r.w() as f64 * r.h() as f64).sum::<f64>());
        }
        let per: Vec<String> = crate::perf::NAMES
            .iter()
            .zip(crate::perf::take())
            .filter(|(_, (ms, _))| *ms > 0.0)
            .map(|(k, (ms, c))| format!("{k} {:.3} ms ({:.1})", ms / 120.0, c as f64 / 120.0))
            .collect();
        // A flick: ten notches up as fast as they come.
        let s0 = size(&mut app);
        for _ in 0..10 {
            app.on_event(Ev::Wheel { delta: 120, x: 1600, y: 1000 });
        }
        let s1 = size(&mut app);
        println!(
            "GDI BENCH 4K wheel {name}: handler {:.3} ms, with paint median {:.3} p90 {:.3} max {:.3} ms | damage {:.0} px | 10 quick notches: {s0} -> {s1}",
            pct(&mut th, 0.5),
            pct(&mut tt.clone(), 0.5),
            pct(&mut tt.clone(), 0.9),
            pct(&mut tt, 1.0),
            pct(&mut area, 0.5)
        );
        println!("  per notch: {}", per.join(", "));
    }
}
