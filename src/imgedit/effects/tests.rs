use super::*;
use crate::imgedit::{Edits, Pad, finish};

fn pic(w: u32, h: u32) -> PixBuf {
    let mut p = PixBuf::new(w, h);
    for (i, px) in p.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        *px = [(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 160, 255];
    }
    p
}

fn alpha(p: &PixBuf, x: u32, y: u32) -> u8 {
    p.get_pixel(x, y)[3]
}

#[test]
fn fixed_strings_cut_at_a_character() {
    let s = Str::<5>::new("aé日本");
    assert_eq!(s.as_str(), "aé"); // 1 + 2 bytes; 日 would need 3 more
    assert!(Str::<4>::default().is_empty());
    assert_eq!(Str::<64>::new("hello").as_str(), "hello");
}

#[test]
fn nothing_is_a_no_op() {
    let e = Effects::default();
    assert!(e.is_empty());
    assert_eq!(e.size((10, 20)), (10, 20));
    let img = pic(30, 20);
    assert_eq!(apply(img.clone(), &e, 1.0), img);
    // an empty watermark and a tear of no side are nothing too
    let e = Effects { watermark: Some(Watermark::default()), torn: Some(Torn { sides: 0, ..Default::default() }), ..Default::default() };
    assert!(e.is_empty());
}

#[test]
fn shadow_grows_the_picture_and_keeps_it() {
    let img = pic(40, 30);
    let s = Shadow { dx: 4, dy: 8, blur: 10.0, color: C4::rgb(0, 0, 0), opacity: 0.5 };
    let e = Effects { shadow: Some(s), ..Default::default() };
    let out = apply(img.clone(), &e, 1.0);
    let [t, r, b, l] = s.room(1.0);
    assert_eq!(out.dimensions(), (40 + l + r, 30 + t + b));
    assert_eq!(out.dimensions(), e.size((40, 30)));
    // the picture is unchanged in the middle
    assert_eq!(out.get_pixel(l + 5, t + 5), img.get_pixel(5, 5));
    // shadow below-right, none far top-left
    assert!(alpha(&out, l + 20, t + 30 + 6) > 40);
    assert_eq!(alpha(&out, 0, 0), 0);
    // the shape path (non-opaque) agrees with the rect path roughly
    let mut holey = img.clone();
    holey.as_raw_mut()[3] = 254;
    let out2 = apply(holey, &e, 1.0);
    let (a1, a2) = (alpha(&out, l + 20, t + 30 + 6) as i32, alpha(&out2, l + 20, t + 30 + 6) as i32);
    assert!((a1 - a2).abs() < 30, "{a1} vs {a2}");
}

#[test]
fn torn_edges_cut_only_their_sides_and_are_deterministic() {
    let img = pic(200, 100);
    let t = Torn { sides: BOTTOM | RIGHT, depth: 10, tooth: 12, seed: 7 };
    let e = Effects { torn: Some(t), ..Default::default() };
    let a = apply(img.clone(), &e, 1.0);
    let b = apply(img.clone(), &e, 1.0);
    assert_eq!(a, b);
    assert_eq!(a.dimensions(), (200, 100));
    // top and left untouched
    for x in 0..185 {
        assert_eq!(alpha(&a, x, 0), 255);
    }
    for y in 0..85 {
        assert_eq!(alpha(&a, 0, y), 255);
    }
    // the bottom row is cut somewhere, and nothing deeper than the depth
    assert!((0..190).any(|x| alpha(&a, x, 99) < 255));
    for x in 0..180 {
        assert_eq!(alpha(&a, x, 100 - 12), 255);
    }
    // another seed tears differently
    let c = apply(img, &Effects { torn: Some(Torn { seed: 8, ..t }), ..Default::default() }, 1.0);
    assert_ne!(a, c);
}

#[test]
fn reflection_mirrors_and_fades() {
    let img = pic(20, 40);
    let r = Reflection { height: 50.0, opacity: 0.5, gap: 2 };
    let e = Effects { reflection: Some(r), ..Default::default() };
    let out = apply(img.clone(), &e, 1.0);
    assert_eq!(out.dimensions(), (20, 40 + 2 + 20));
    assert_eq!(out.dimensions(), e.size((20, 40)));
    assert_eq!(alpha(&out, 3, 41), 0); // the gap
    let first = out.get_pixel(3, 42);
    assert_eq!(&first[..3], &img.get_pixel(3, 39)[..3]);
    assert!(first[3] > 100 && first[3] <= 128);
    assert!(alpha(&out, 3, 61) < 10);
}

#[test]
fn text_watermark_lands_at_its_anchor() {
    let img = PixBuf::from_pixel(400, 200, [40, 40, 40, 255]);
    let wm = Watermark { text: Str::new("Rustshot"), anchor: 8, margin: 10, opacity: 1.0, size: 25.0, ..Default::default() };
    let out = apply(img.clone(), &Effects { watermark: Some(wm), ..Default::default() }, 1.0);
    assert_eq!(out.dimensions(), img.dimensions());
    let changed = |x0: u32, y0: u32, x1: u32, y1: u32| (y0..y1).any(|y| (x0..x1).any(|x| out.get_pixel(x, y) != img.get_pixel(x, y)));
    assert!(changed(250, 120, 400, 200), "bottom-right has the mark");
    assert!(!changed(0, 0, 200, 100), "top-left is untouched");
    // about 25 % of the width
    let cols: Vec<u32> = (0..400).filter(|&x| (0..200).any(|y| out.get_pixel(x, y) != img.get_pixel(x, y))).collect();
    let span = cols.last().unwrap() - cols.first().unwrap();
    assert!((80..=125).contains(&span), "{span}");
    assert!(*cols.last().unwrap() <= 400 - 10 + 3);
    // tiled covers both halves
    let tiled = apply(img.clone(), &Effects { watermark: Some(Watermark { tiled: true, ..wm }), ..Default::default() }, 1.0);
    let ch = |x0: u32, x1: u32| (0..200).any(|y| (x0..x1).any(|x| tiled.get_pixel(x, y) != img.get_pixel(x, y)));
    assert!(ch(0, 150) && ch(250, 400));
}

#[test]
fn image_watermark_reads_a_png_and_a_missing_file_is_nothing() {
    let dir = std::env::temp_dir().join(format!("rustshot-wm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mark.png");
    std::fs::write(&path, PixBuf::from_pixel(10, 5, [255, 0, 0, 255]).to_png().unwrap()).unwrap();
    let img = PixBuf::from_pixel(100, 100, [0, 0, 255, 255]);
    let wm = Watermark { image: true, text: Str::new(path.to_str().unwrap()), anchor: 0, margin: 0, opacity: 1.0, size: 50.0, ..Default::default() };
    let out = apply(img.clone(), &Effects { watermark: Some(wm), ..Default::default() }, 1.0);
    assert_eq!(out.get_pixel(5, 5), [255, 0, 0, 255]);
    assert_eq!(out.get_pixel(60, 5), [0, 0, 255, 255]);
    assert_eq!(out.get_pixel(5, 30), [0, 0, 255, 255]); // 50 × 25
    let missing = Watermark { text: Str::new(dir.join("nope.png").to_str().unwrap()), ..wm };
    assert_eq!(apply(img.clone(), &Effects { watermark: Some(missing), ..Default::default() }, 1.0), img);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn anchors_place_the_mark() {
    let s = (100, 50);
    let m = (20, 10);
    assert_eq!(anchor_pos(0, s, m, 5), (5, 5));
    assert_eq!(anchor_pos(4, s, m, 5), (40, 20));
    assert_eq!(anchor_pos(8, s, m, 5), (75, 35));
    assert_eq!(anchor_pos(7, s, m, 5), (40, 35));
}

#[test]
fn effects_follow_the_canvas_in_the_stack() {
    let img = pic(50, 40);
    let e = Edits { canvas: Some(Pad::presentation()), effects: Effects { reflection: Some(Reflection::default()), ..Default::default() }, ..Default::default() };
    assert!(!e.is_empty());
    let out = finish(img.clone(), &e);
    assert_eq!(out.dimensions(), e.finish_size((50, 40)));
    let canvas = Pad::presentation().size((50, 40));
    assert_eq!(out.width(), canvas.0);
    assert!(out.height() > canvas.1);
}

#[test]
fn hostile_values_are_clamped() {
    let img = pic(30, 30);
    let e = Effects {
        watermark: Some(Watermark { text: Str::new("x"), size: f32::NAN, opacity: f32::INFINITY, margin: u32::MAX, anchor: 200, ..Default::default() }),
        torn: Some(Torn { sides: 255, depth: u32::MAX, tooth: 0, seed: 0 }),
        shadow: Some(Shadow { dx: i32::MIN, dy: i32::MAX, blur: f32::NAN, opacity: -3.0, ..Default::default() }),
        reflection: Some(Reflection { height: 1e9, opacity: f32::NAN, gap: u32::MAX }),
    };
    let out = apply(img, &e, 1.0);
    assert_eq!(out.dimensions(), e.size((30, 30)));
    assert!(out.width() < 2000 && out.height() < 6000);
    // a one-pixel picture survives every effect
    let _ = apply(PixBuf::from_pixel(1, 1, [1, 2, 3, 255]), &e, 1.0);
}

#[test]
fn thumbnails_scale_the_effects() {
    let e = Effects { shadow: Some(Shadow::default()), reflection: Some(Reflection::default()), ..Default::default() };
    let full = e.size((400, 300));
    let half = size_at(&e, (200, 150), 0.5);
    assert!((full.0 as i32 / 2 - half.0 as i32).abs() <= 2 && (full.1 as i32 / 2 - half.1 as i32).abs() <= 2, "{full:?} {half:?}");
}

#[test]
fn each_effect_is_fast_on_4k() {
    let img = pic(3840, 2160);
    let cases: [(&str, Effects); 5] = [
        ("watermark", Effects { watermark: Some(Watermark { text: Str::new("Rustshot"), ..Default::default() }), ..Default::default() }),
        ("torn", Effects { torn: Some(Torn { sides: 15, ..Default::default() }), ..Default::default() }),
        ("shadow", Effects { shadow: Some(Shadow::default()), ..Default::default() }),
        ("shadow (shape)", Effects { shadow: Some(Shadow::default()), ..Default::default() }),
        ("reflection", Effects { reflection: Some(Reflection::default()), ..Default::default() }),
    ];
    let mut torn = img.clone();
    torn.as_raw_mut()[3] = 0;
    for (name, e) in cases {
        let src = if name == "shadow (shape)" { torn.clone() } else { img.clone() };
        let t = std::time::Instant::now();
        let out = apply(src, &e, 1.0);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("{name}: {ms:.1} ms ({}x{})", out.width(), out.height());
        if !cfg!(debug_assertions) {
            assert!(ms < 30.0, "{name}: {ms:.1} ms");
        }
    }
}

#[test]
fn preview_pngs() {
    use crate::theme::{DARK, LIGHT};
    let sample = {
        let mut p = PixBuf::new(360, 220);
        for (i, px) in p.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i as u32 % 360, i as u32 / 360);
            let band = if (x / 30 + y / 30) % 2 == 0 { 235 } else { 205 };
            *px = [band, band, 255, 255];
        }
        p
    };
    let all = [
        ("watermark", Effects { watermark: Some(Watermark { text: Str::new("Rustshot"), color: C4::rgb(30, 30, 30), opacity: 0.8, ..Default::default() }), ..Default::default() }),
        ("watermark_tiled", Effects { watermark: Some(Watermark { text: Str::new("DRAFT"), tiled: true, size: 15.0, opacity: 0.4, color: C4::rgb(200, 40, 40), ..Default::default() }), ..Default::default() }),
        ("torn", Effects { torn: Some(Torn { sides: BOTTOM | RIGHT, ..Default::default() }), shadow: Some(Shadow::default()), ..Default::default() }),
        ("shadow", Effects { shadow: Some(Shadow { opacity: 0.45, ..Default::default() }), ..Default::default() }),
        ("reflection", Effects { reflection: Some(Reflection::default()), ..Default::default() }),
    ];
    for (theme, th) in [("dark", DARK), ("light", LIGHT)] {
        for (name, e) in &all {
            let out = apply(sample.clone(), e, 1.0);
            // on the window's surface, as the Edit window shows it
            let bg = [th.surface.r, th.surface.g, th.surface.b, 255];
            let mut shown = PixBuf::from_pixel(out.width() + 40, out.height() + 40, bg);
            blit_over(&mut shown, &out, 20, 20);
            crate::ui::preview::save(&format!("effect_{name}_{theme}.png"), &shown);
        }
    }
}
