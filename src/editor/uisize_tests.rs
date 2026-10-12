//! The UI size setting in the overlay: Ctrl+= / Ctrl+- / Ctrl+0 step it
//! live (the toolbar laid out again, the frame repainted in full), it is
//! remembered like the tools' sizes, and the recording's monitors follow
//! it per monitor; plus its preview PNGs.

use super::tests::{annotated, assert_dirty_covers, edit_of, preview_app_with, settled_frame, synthetic_shot};
use super::*;

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
}

fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

const BIG: FRect = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
const CTRL: Mods = Mods { ctrl: true, shift: false, alt: false };

fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

fn bigger(a: &mut App) {
    key_m(a, wind::key::EQUALS, CTRL);
}

fn smaller(a: &mut App) {
    key_m(a, wind::key::MINUS, CTRL);
}

fn reset(a: &mut App) {
    key_m(a, '0' as u32, CTRL);
}

/// The toolbar as laid out now.
fn bar(a: &mut App) -> toolbar::Toolbar {
    let m = a.mouse;
    ed(a).prepare(None, m, Instant::now());
    ed(a).toolbar.clone().expect("toolbar")
}

fn toast(a: &mut App) -> String {
    ed(a).notice.as_ref().map(|t| t.text.clone()).unwrap_or_default()
}

/// Ctrl+= / Ctrl+- step through the list from Auto's 100 %, Ctrl+0 goes
/// back to Auto; each says the size; the scale follows at once.
#[test]
fn ctrl_keys_step_the_ui_size() {
    let mut a = annotated(theme::DARK, BIG);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Auto);
    assert_eq!(ed(&mut a).ui_scale(), 1.0);
    bigger(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(125));
    assert_eq!(ed(&mut a).ui_scale(), 1.25);
    assert_eq!(toast(&mut a), "UI size 125 %");
    bigger(&mut a);
    bigger(&mut a);
    bigger(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(200), "the largest stays");
    assert_eq!(ed(&mut a).ui_scale(), 2.0);
    reset(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Auto);
    assert_eq!(toast(&mut a), "UI size Auto");
    smaller(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(90), "from Auto's 100 %");
    smaller(&mut a);
    smaller(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(75), "the smallest stays");
    assert_eq!(ed(&mut a).ui_scale(), 0.75);
    // The other spellings: Ctrl+Plus (Shift+= on US keyboards), the keypad.
    reset(&mut a);
    key_m(&mut a, wind::key::EQUALS, Mods { shift: true, ..CTRL });
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(125));
    key_m(&mut a, wind::key::ADD, CTRL);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(150));
    key_m(&mut a, wind::key::SUBTRACT, CTRL);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(125));
    key_m(&mut a, wind::key::NUMPAD0, CTRL);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Auto);
    // A bare = or - is not a UI size key.
    a.on_event(Ev::Key { vk: wind::key::EQUALS, up: false, repeat: false, mods: Mods::default() });
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Auto);
}

/// The keys work while typing a text and with the colour picker open
/// (both take every other key), and they can be remapped.
#[test]
fn ui_size_keys_work_in_every_mode_and_remap() {
    let mut a = annotated(theme::DARK, BIG);
    ed(&mut a).text = Some(TextDraft { pos: Pt::new(400.0, 300.0), text: "hi".into(), caret: 2, at: Instant::now() });
    bigger(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(125));
    assert_eq!(ed(&mut a).text.as_ref().map(|t| t.text.as_str()), Some("hi"), "the text is untouched");
    ed(&mut a).text = None;
    ed(&mut a).tool = Some(Tool::Line);
    ed(&mut a).open_picker(pick::Target::Tool);
    smaller(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Auto.smaller().bigger(), "back to 100 %");
    assert!(ed(&mut a).pick.is_some(), "the picker stays open");
    ed(&mut a).close_picker(true);
    // Remapped: Ctrl+= is free, F9 makes it bigger.
    let (km, w) = Keymap::from_config(&[("ui_bigger".to_string(), "F9".to_string())].into_iter().collect());
    assert!(w.is_empty(), "{w:?}");
    ed(&mut a).keys = km;
    bigger(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(100));
    key_m(&mut a, 0x78, Mods::default());
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(125));
}

/// A new size lays the toolbar out again at once: scaled by the ratio,
/// still centred on the selection and just below it.
#[test]
fn live_relayout_keeps_the_toolbar_placed() {
    // Wide enough for the bar on one row at 125 %.
    const BIG: FRect = FRect { x: 100.0, y: 140.0, w: 1240.0, h: 540.0 };
    let mut a = annotated(theme::DARK, BIG);
    let before = bar(&mut a);
    bigger(&mut a);
    let after = bar(&mut a);
    let k = 1.25;
    let near = |x: f32, y: f32| (x - y).abs() <= 1.0;
    assert!(near(after.bar.h, before.bar.h * k), "{:?} -> {:?}", before.bar, after.bar);
    assert!(near(after.bar.w, before.bar.w * k), "{:?} -> {:?}", before.bar, after.bar);
    let mid = |b: FRect| b.x + b.w / 2.0;
    assert!(near(mid(after.bar), BIG.x + BIG.w / 2.0) && near(mid(before.bar), mid(after.bar)), "centred");
    assert!(after.bar.y > BIG.y1() && after.bar.y - BIG.y1() > before.bar.y - BIG.y1() - 1.0, "below, the gap scaled");
    assert!(after.bar.y1() <= 900.0, "on screen");
    // Every item scaled with it.
    for (b, n) in before.items.iter().zip(&after.items) {
        assert!(near(n.r.w, b.r.w * k) && near(n.r.h, b.r.h * k), "{:?} -> {:?}", b.r, n.r);
    }
    // Smaller: the same, the other way.
    reset(&mut a);
    smaller(&mut a);
    let small = bar(&mut a);
    assert!(near(small.bar.h, before.bar.h * 0.9) && near(mid(small.bar), mid(before.bar)));
    // At 200 % near an edge the toolbar still fits the monitor.
    let mut a = annotated(theme::DARK, r(1180.0, 200.0, 250.0, 300.0));
    for _ in 0..4 {
        bigger(&mut a);
    }
    let b = bar(&mut a);
    assert!(b.bar.x >= 0.0 && b.bar.x1() <= 1440.0 && b.bar.y >= 0.0 && b.bar.y1() <= 900.0, "{:?}", b.bar);
    assert!(b.wrapped, "two rows at 200 % on a narrow selection");
}

/// A scale change repaints the whole frame (the scene's scale differs);
/// every changed pixel is covered. At 100 % a move still repaints little.
#[test]
fn a_new_size_repaints_in_full() {
    let mut a = annotated(theme::DARK, BIG);
    settled_frame(&mut a);
    let f = assert_dirty_covers(&mut a, "bigger", bigger);
    assert_eq!(f, 1.0, "full repaint");
    let f = assert_dirty_covers(&mut a, "reset", reset);
    assert_eq!(f, 1.0, "full repaint");
    // Unchanged (already Auto): no full repaint for the toast alone.
    settled_frame(&mut a);
    let f = assert_dirty_covers(&mut a, "same", reset);
    assert!(f < 0.5, "only the toast: {f}");
    // At 100 % an ordinary change stays partial (perf unchanged).
    settled_frame(&mut a);
    let f = assert_dirty_covers(&mut a, "hover", |a| {
        let p = a.mouse;
        a.on_event(Ev::Move { x: p.0 + 3, y: p.1 + 2 });
    });
    assert!(f < 0.5, "a move repaints little: {f}");
}

/// The new size is saved like the tools' sizes: a line edit of the
/// config after a pause, or when the overlay closes; the next capture
/// starts from it.
#[test]
fn the_size_is_remembered() {
    let dir = std::env::temp_dir().join(format!("rustshot-uisize-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "# mine\ntheme = \"dark\"\n").unwrap();
    let mut a = annotated(theme::DARK, BIG);
    a.focus_tries = u8::MAX;
    ed(&mut a).cfg_path = Some(path.clone());
    bigger(&mut a);
    bigger(&mut a);
    assert_eq!(ed(&mut a).pending.get("ui_scale"), Some(&Some("150".to_string())), "coalesced");
    a.pump();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("ui_scale"), "not before the pause");
    // Closed before the pause: written on close; the daemon's config follows.
    a.on_event(Ev::Key { vk: wind::key::ESCAPE, up: false, repeat: false, mods: Mods::default() });
    a.on_event(Ev::Key { vk: wind::key::ESCAPE, up: false, repeat: false, mods: Mods::default() });
    assert!(matches!(a.st, State::Hidden));
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "# mine\ntheme = \"dark\"\nui_scale = 150\n");
    assert_eq!(a.cfg.ui_scale, UiSize::Pct(150), "the next capture starts at 150 %");
    // Back to Auto is written as "auto".
    let _ = std::fs::remove_dir_all(&dir);
}

/// The recording's monitors carry the UI size per monitor (mixed DPI:
/// each monitor's own scale times it).
#[test]
fn recording_geometry_scales_per_monitor() {
    let mut a = annotated(theme::DARK, BIG);
    let mons = [
        capture::MonInfo { x: 0, y: 0, w: 720, h: 900, scale: 1.0, primary: true },
        capture::MonInfo { x: 720, y: 0, w: 720, h: 900, scale: 2.0, primary: false },
    ];
    ed(&mut a).shot.monitors = vec![(0, 0, 720, 900), (720, 0, 720, 900)];
    let g = ed(&mut a).rec_geom(&mons);
    assert_eq!(g.monitors.iter().map(|m| m.1).collect::<Vec<_>>(), [1.0, 2.0]);
    bigger(&mut a);
    bigger(&mut a);
    let g = ed(&mut a).rec_geom(&mons);
    assert_eq!(g.monitors.iter().map(|m| m.1).collect::<Vec<_>>(), [1.5, 3.0]);
    assert_eq!(g.scale, 1.5);
    // The camera bubble follows (its side at the area's scale).
    let w = ed(&mut a).cam_want(&g);
    assert_eq!(w.scale, g.area_scale());
}

// ---- previews ----------------------------------------------------------------

/// Toolbar, palette, picker and Record popover at 75, 100, 150 and 200 %,
/// dark and light (on a 2560×1440 shot, 100 % OS scale).
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_uisize_previews() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let save = |img: &PixBuf, name: &str| std::fs::write(dir.join(name), img.to_png().unwrap()).unwrap();
    let shot = || {
        let s = synthetic_shot();
        let (w, h) = (2560u32, 1440u32);
        let mut img = PixBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let p = s.image.get_pixel(x * s.size.0 / w, y * s.size.1 / h);
                let i = (y as usize * w as usize + x as usize) * 4;
                img.as_raw_mut()[i..i + 4].copy_from_slice(&p);
            }
        }
        Shot { origin: (0, 0), size: (w, h), scale: 1.0, image: img, monitors: vec![(0, 0, w, h)] }
    };
    let sel = r(480.0, 260.0, 1600.0, 760.0);
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        for pct in [75u16, 100, 150, 200] {
            let name = |what: &str| format!("uisize-{pct}-{tn}-{what}.png");
            let app = || {
                let mut a = preview_app_with(th, Some(sel), shot());
                let e = ed(&mut a);
                e.cfg.ui_scale = UiSize::Pct(pct);
                e.tool = Some(Tool::Arrow);
                e.last_tool = Tool::Arrow;
                e.objects.push(Obj::Rect { r: r(560.0, 330.0, 500.0, 260.0), color: C4::rgb(240, 68, 56), width: 3.0 });
                e.dirty = true;
                a
            };
            let mut a = app();
            ed(&mut a).notice = Some(Toast::new(UiSize::Pct(pct).describe(), ToastKind::Info));
            save(&settled_frame(&mut a), &name("toolbar"));
            let mut a = app();
            ed(&mut a).palette_open = true;
            save(&settled_frame(&mut a), &name("palette"));
            let mut a = app();
            ed(&mut a).tool = Some(Tool::Line);
            ed(&mut a).open_picker(pick::Target::Tool);
            save(&settled_frame(&mut a), &name("picker"));
            let mut a = app();
            let e = ed(&mut a);
            e.rec.open = true;
            e.rec.load_cams();
            e.rec.opts.camera = true;
            save(&settled_frame(&mut a), &name("record"));
        }
    }
}
