use super::*;
use crate::uifb::C4;

/// All eight orientations.
fn all() -> Vec<Orient> {
    (0..8).map(|i| Orient { rot: i % 4, flip: i >= 4 }).collect()
}

fn close(a: Pt, b: Pt) -> bool {
    (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
}

/// A picture whose every pixel is different (x, y in the color).
fn coded(w: u32, h: u32) -> PixBuf {
    let mut p = PixBuf::new(w, h);
    for (i, px) in p.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % w as usize, i / w as usize);
        *px = [x as u8, y as u8, (x / 256 + 16 * (y / 256)) as u8, 255];
    }
    p
}

#[test]
fn orientations_compose_and_invert() {
    let size = (40, 25);
    let pts = [Pt::new(0.0, 0.0), Pt::new(3.5, 7.25), Pt::new(40.0, 25.0), Pt::new(12.0, 0.5)];
    for a in all() {
        assert_eq!(a.then(a.inverse()), Orient::ID, "{a:?}");
        assert_eq!(a.inverse().then(a), Orient::ID, "{a:?}");
        for p in pts {
            let q = a.pt(p, size);
            assert!(close(a.inverse().pt(q, a.size(size)), p), "{a:?} {p:?}");
        }
        for b in all() {
            for p in pts {
                let two = b.pt(a.pt(p, size), a.size(size));
                assert!(close(two, a.then(b).pt(p, size)), "{a:?} then {b:?} at {p:?}");
            }
        }
    }
    // The steps the menu offers.
    let mut o = Orient::ID;
    for _ in 0..4 {
        o = o.rotated(true);
    }
    assert_eq!(o, Orient::ID, "four right turns");
    assert_eq!(Orient::ID.rotated(true).rotated(false), Orient::ID);
    assert_eq!(Orient::ID.flipped_h().flipped_h(), Orient::ID);
    assert_eq!(Orient::ID.flipped_v().flipped_v(), Orient::ID);
    assert_eq!(Orient::ID.flipped_h().flipped_v(), Orient { rot: 2, flip: false }, "both flips: a half turn");
    // Rotate right: the top-left corner goes to the top-right.
    assert!(close(Orient::ID.rotated(true).pt(Pt::new(0.0, 0.0), size), Pt::new(25.0, 0.0)));
    // Flip vertical: top-bottom, as seen now (also after a turn).
    let v = Orient::ID.flipped_v();
    assert!(close(v.pt(Pt::new(3.0, 4.0), size), Pt::new(3.0, 21.0)));
    let r = Orient::ID.rotated(true);
    let rv = r.flipped_v();
    let seen = r.pt(Pt::new(3.0, 4.0), size); // on the turned picture (25 x 40)
    assert!(close(rv.pt(Pt::new(3.0, 4.0), size), Pt::new(seen.x, 40.0 - seen.y)));
}

#[test]
fn pixels_follow_the_points() {
    let img = coded(7, 4);
    for o in all() {
        let out = o.image(&img);
        assert_eq!(out.dimensions(), o.size((7, 4)), "{o:?}");
        for y in 0..4 {
            for x in 0..7 {
                let c = o.pt(Pt::new(x as f32 + 0.5, y as f32 + 0.5), (7, 4));
                assert_eq!(out.get_pixel(c.x as u32, c.y as u32), img.get_pixel(x, y), "{o:?} ({x}, {y})");
            }
        }
        assert_eq!(o.inverse().image(&out), img, "{o:?}: back exactly");
    }
    // Larger than a tile.
    let big = coded(300, 130);
    for o in all() {
        assert_eq!(o.inverse().image(&o.image(&big)), big);
    }
}

fn kinds() -> Vec<Obj> {
    let c = C4::rgb(240, 68, 56);
    let r = FRect { x: 10.0, y: 20.0, w: 60.0, h: 30.0 };
    vec![
        Obj::Line { a: Pt::new(1.0, 2.0), b: Pt::new(50.0, 70.0), color: c, width: 3.0 },
        Obj::Arrow { a: Pt::new(5.0, 6.0), b: Pt::new(70.0, 80.0), color: c, width: 4.0 },
        Obj::Marker { a: Pt::new(1.0, 1.0), b: Pt::new(20.0, 2.0), color: c, width: 15.0 },
        Obj::Path { pts: vec![Pt::new(1.0, 2.0), Pt::new(3.5, 40.0), Pt::new(90.0, 7.125)], color: c, width: 3.0 },
        Obj::MarkerPath { pts: vec![Pt::new(1.0, 1.0), Pt::new(9.0, 3.0)], color: c, width: 15.0 },
        Obj::Rect { r, color: c, width: 3.0 },
        Obj::RectFill { r, color: c, width: 3.0 },
        Obj::Ellipse { r, color: c, width: 2.0 },
        Obj::EllipseFill { r, color: c, width: 2.0 },
        Obj::Pixelate { r, cell: 12.0 },
        Obj::Blur { r, sigma: 8.0 },
        Obj::Invert { r },
        Obj::Spotlight { r },
        Obj::Counter { c: Pt::new(30.0, 40.0), n: 3, color: c, d: 32.0, tail: Some(Pt::new(80.0, 90.0)) },
        Obj::Callout {
            r: FRect { x: 20.0, y: 10.0, w: 80.0, h: 40.0 },
            tail: Pt::new(30.0, 90.0),
            text: "Hi".into(),
            color: c,
            size: 17.0,
            style: crate::objects::CalloutStyle::Rounded,
            fill: true,
            fmt: Default::default(),
        },
        Obj::Magnify { src: Pt::new(40.0, 40.0), dst: Pt::new(90.0, 30.0), radius: 20.0, zoom: 2.0, color: c, width: 3.0 },
        Obj::Text { pos: Pt::new(12.0, 30.0), text: "Upright".into(), color: c, size: 16.0, fmt: Default::default() },
    ]
}

fn obj_close(a: &Obj, b: &Obj) -> bool {
    // Through the JSON text form: every number rounded the same way.
    let round = |o: &Obj| {
        let mut o = o.clone();
        crate::objects::edit::translate(&mut o, 0.0, 0.0);
        format!("{o:?}")
    };
    if round(a) == round(b) {
        return true;
    }
    // Numbers within 1e-3.
    let nums = |s: String| s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter_map(|t| t.parse::<f32>().ok()).collect::<Vec<_>>();
    let (x, y) = (nums(round(a)), nums(round(b)));
    x.len() == y.len() && x.iter().zip(&y).all(|(p, q)| (p - q).abs() < 1e-3)
}

/// Every kind maps into each orientation and back to where it was.
#[test]
fn objects_round_trip_under_every_orientation() {
    let font = AnnotFont::load();
    let size = (120, 100);
    for o in all() {
        for k in kinds() {
            let m = o.map_obj(&k, size, font.as_ref());
            let back = o.inverse().map_obj(&m, o.size(size), font.as_ref());
            assert!(obj_close(&back, &k), "{o:?}: {k:?} came back as {back:?}");
        }
    }
    assert_eq!(Orient::ID.map_obj(&kinds()[0], size, None), kinds()[0], "identity: the object itself");
}

/// Rects (redactions included) cover the same pixels on the turned
/// picture as on the original.
#[test]
fn redactions_cover_the_same_pixels_when_turned() {
    let size = (64u32, 48u32);
    // Noise: no blur or mosaic leaves a pixel as it was by chance.
    let mut img = coded(size.0, size.1);
    let mut seed = 0x2545_f491u32;
    for px in img.as_raw_mut().as_chunks_mut::<4>().0 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *px = [seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255];
    }
    let black = C4::rgb(0, 0, 0);
    let objs = vec![
        Obj::RectFill { r: FRect { x: 8.0, y: 5.0, w: 20.0, h: 12.0 }, color: black, width: 1.0 },
        Obj::Pixelate { r: FRect { x: 30.0, y: 20.0, w: 24.0, h: 24.0 }, cell: 8.0 },
        Obj::Blur { r: FRect { x: 2.0, y: 30.0, w: 20.0, h: 14.0 }, sigma: 6.0 },
    ];
    let baked = crate::editor::bake_objects(&img, &objs, None);
    for o in all() {
        let turned = o.image(&img);
        let mapped: Vec<Obj> = objs.iter().map(|x| o.map_obj(x, size, None)).collect();
        let a = crate::editor::bake_objects(&turned, &mapped, None);
        let b = o.image(&baked);
        // The black fill is exact; a pixel inside a pixelate/blur rect is
        // changed in both (they hide alike, their cells may differ).
        for y in 0..b.height() {
            for x in 0..b.width() {
                let (pa, pb) = (a.get_pixel(x, y), b.get_pixel(x, y));
                let orig = turned.get_pixel(x, y);
                assert_eq!(pa == orig, pb == orig, "{o:?} ({x}, {y}): hidden in one and not the other");
                if pb == [0, 0, 0, 255] {
                    assert_eq!(pa, pb, "{o:?} ({x}, {y}): the fill");
                }
            }
        }
    }
}

/// The upright rule: a text keeps its size, its glyphs are not turned,
/// its box's centre goes where the picture takes it.
#[test]
fn text_stays_upright_and_its_centre_follows() {
    let Some(font) = AnnotFont::load() else { return };
    let size = (300, 200);
    let t = Obj::Text { pos: Pt::new(40.0, 30.0), text: "Upright".into(), color: C4::rgb(0, 0, 0), size: 20.0, fmt: Default::default() };
    let (w, h, _) = font.measure("Upright", 20.0);
    let centre = Pt::new(40.0 + w / 2.0, 30.0 + h / 2.0);
    for o in all() {
        let Obj::Text { pos, size: px, text, .. } = o.map_obj(&t, size, Some(&font)) else { panic!() };
        assert_eq!((px, text.as_str()), (20.0, "Upright"), "same size and text");
        let c = Pt::new(pos.x + w / 2.0, pos.y + h / 2.0);
        assert!(close(c, o.pt(centre, size)), "{o:?}: centre {c:?}");
    }
    // Drawn on the turned picture it reads left to right: its ink is wider than tall.
    let r = Orient::ID.rotated(true);
    let m = r.map_obj(&t, size, Some(&font));
    let mut pic = PixBuf::from_pixel(200, 300, [255, 255, 255, 255]);
    m.render(&mut pic, Some(&font));
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..300 {
        for x in 0..200 {
            if pic.get_pixel(x, y)[0] < 128 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    assert!(x1 > x0 && x1 - x0 > 2 * (y1 - y0), "upright ink: {}x{}", x1 - x0, y1 - y0);
    // A callout's box keeps its size.
    let co = &kinds()[14];
    let Obj::Callout { r: q, .. } = r.map_obj(co, size, None) else { panic!() };
    assert_eq!((q.w, q.h), (80.0, 40.0));
}

#[test]
fn resize_targets_follow_the_picture() {
    let r = Resize { w: 960, h: 540, from: (1920, 1080), filter: Filter::Lanczos3 };
    assert_eq!(r.target((1920, 1080)), (960, 540));
    assert_eq!(r.target((1000, 500)), (500, 250), "a new crop: the same factors");
    assert!(!r.enlarges((1920, 1080)));
    assert!(Resize { w: 2000, ..r }.enlarges((1920, 1080)));
    let e = Edits { resize: Some(r), ..Default::default() };
    assert_eq!(e.out_size((1920, 1080)), (960, 540));
    let turned = e.reoriented(Orient::ID.rotated(true));
    assert_eq!(turned.out_size((1920, 1080)), (540, 960), "the sides turn with the picture");
    assert_eq!(turned.reoriented(Orient::ID).resize, Some(r));
    let c = Edits { canvas: Some(Pad::uniform(10, Bg::Solid(C4::rgb(255, 255, 255)), 0.0, false)), ..turned };
    assert_eq!(c.out_size((1920, 1080)), (560, 980));
    assert!(Edits::default().is_empty() && !c.is_empty());
}

/// The export pipeline: the crop, the turn, the bake, the resize and the
/// canvas, in that order.
#[test]
fn render_applies_the_stack_in_order() {
    let size = (80u32, 60u32);
    let img = coded(size.0, size.1);
    let crop = FRect { x: 10.0, y: 5.0, w: 40.0, h: 30.0 };
    let objs = vec![Obj::RectFill { r: FRect { x: 12.0, y: 8.0, w: 10.0, h: 6.0 }, color: C4::rgb(0, 0, 0), width: 1.0 }];
    let plain = crate::history::render(&img, &objs, Some(crop), None);
    for o in all() {
        let e = Edits { orient: o, ..Default::default() };
        let out = render(&img, &objs, Some(crop), None, &e);
        assert_eq!(out, o.image(&plain), "{o:?}: cropping then turning");
    }
    let e = Edits {
        orient: Orient::ID.rotated(false),
        resize: Some(Resize { w: 15, h: 20, from: (30, 40), filter: Filter::Area }),
        canvas: Some(Pad::uniform(5, Bg::Solid(C4::rgb(0, 0, 255)), 0.0, false)),
    };
    let out = render(&img, &objs, Some(crop), None, &e);
    assert_eq!(out.dimensions(), (25, 30));
    assert_eq!(e.out_size((40, 30)), (25, 30));
    assert_eq!(out.get_pixel(0, 0), [0, 0, 255, 255]);
    let mid = resample::resize(&e.orient.image(&plain), 15, 20, Filter::Area);
    assert_eq!(out.get_pixel(12, 12), mid.get_pixel(7, 7));
}

/// Release timings (`cargo test --release -- --ignored perf_`): a 4K
/// picture's resize and canvas, and an export with every transform.
#[test]
#[ignore]
fn perf_4k() {
    let img = coded(3840, 2160);
    let time = |what: &str, f: &mut dyn FnMut()| {
        f();
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            f();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("perf {what}: {best:.1} ms");
        best
    };
    let a = time("resize 4K -> 1920x1080 lanczos3", &mut || {
        std::hint::black_box(resample::resize(&img, 1920, 1080, Filter::Lanczos3));
    });
    let b = time("resize 4K -> 1920x1080 area", &mut || {
        std::hint::black_box(resample::resize(&img, 1920, 1080, Filter::Area));
    });
    let c = time("resize 4K -> 2880x1620 lanczos3", &mut || {
        std::hint::black_box(resample::resize(&img, 2880, 1620, Filter::Lanczos3));
    });
    let d = time("canvas 4K presentation", &mut || {
        std::hint::black_box(frame::apply(&img, &Pad::presentation()));
    });
    let objs = vec![
        Obj::RectFill { r: FRect { x: 100.0, y: 100.0, w: 600.0, h: 80.0 }, color: C4::rgb(0, 0, 0), width: 1.0 },
        Obj::Blur { r: FRect { x: 900.0, y: 300.0, w: 500.0, h: 120.0 }, sigma: 12.0 },
        Obj::Arrow { a: Pt::new(2000.0, 1500.0), b: Pt::new(2600.0, 900.0), color: C4::rgb(240, 68, 56), width: 6.0 },
    ];
    let e = Edits {
        orient: Orient::ID.rotated(true).flipped_h(),
        resize: Some(Resize { w: 1440, h: 2560, from: (2160, 3840), filter: Filter::Lanczos3 }),
        canvas: Some(Pad::presentation()),
    };
    let f = time("export 4K rotate+flip+resize+canvas+objects", &mut || {
        std::hint::black_box(render(&img, &objs, Some(FRect { x: 0.0, y: 0.0, w: 3840.0, h: 2160.0 }), None, &e));
    });
    if !cfg!(debug_assertions) {
        assert!(a < 50.0 && b < 50.0 && c < 50.0 && d < 50.0, "resize/canvas under 50 ms");
        assert!(f < 150.0, "export under 150 ms");
    }
}
