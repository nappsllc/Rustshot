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
    const BIG: FRect = FRect { x: 70.0, y: 140.0, w: 1300.0, h: 540.0 };
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

/// The toolbar, its popover and every item as laid out now.
fn layout_of(a: &mut App) -> (FRect, Option<FRect>, Vec<FRect>) {
    let t = bar(a);
    (t.bar, t.pop, t.items.iter().map(|i| i.r).collect())
}

fn same_rect(a: FRect, b: FRect) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.w - b.w).abs() < 0.01 && (a.h - b.h).abs() < 0.01
}

/// A popover open while the size changes is anchored again at the new
/// scale, exactly where a fresh layout at that scale puts it, and what it
/// holds (the picker's colour, focus and hex text) stays.
#[test]
fn an_open_popover_is_anchored_again_at_the_new_size() {
    type Setup = fn(&mut App);
    let cases: [(&str, Setup); 2] = [
        ("palette", |a| ed(a).palette_open = true),
        ("picker", |a| {
            ed(a).tool = Some(Tool::Line);
            ed(a).open_picker(pick::Target::Tool);
            let p = ed(a).pick.as_mut().unwrap();
            p.set_hue(200.0);
            p.set_sv(0.3, 0.8);
            p.focus = Some(pick::Part::Hue);
        }),
    ];
    for (name, setup) in cases {
        for (start, to) in [(UiSize::Auto, UiSize::Pct(125)), (UiSize::Pct(150), UiSize::Pct(75)), (UiSize::Auto, UiSize::Pct(200))] {
            let mut live = annotated(theme::DARK, BIG);
            ed(&mut live).cfg.ui_scale = start;
            setup(&mut live);
            let before = layout_of(&mut live);
            assert!(before.1.is_some(), "{name}: popover open");
            let picked = ed(&mut live).pick.clone();
            ed(&mut live).set_ui_size(to, Instant::now());
            let after = layout_of(&mut live);
            assert_eq!(ed(&mut live).pick, picked, "{name}: the picker keeps its state");
            assert_eq!(ed(&mut live).palette_open, name == "palette", "{name}: still open");
            // A fresh capture at the new size.
            let mut fresh = annotated(theme::DARK, BIG);
            ed(&mut fresh).cfg.ui_scale = to;
            setup(&mut fresh);
            let want = layout_of(&mut fresh);
            let what = format!("{name} {start:?} -> {to:?}");
            assert!(same_rect(after.0, want.0), "{what}: bar {:?} vs {:?}", after.0, want.0);
            assert!(same_rect(after.1.unwrap(), want.1.unwrap()), "{what}: popover {:?} vs {:?}", after.1, want.1);
            assert_eq!(after.2.len(), want.2.len());
            for (x, y) in after.2.iter().zip(&want.2) {
                assert!(same_rect(*x, *y), "{what}: item {x:?} vs {y:?}");
            }
            assert_ne!(after.0, before.0, "{what}: it did move");
        }
    }
}

/// The toast at 75 % and 200 %: the panel is 36 logical px tall and its
/// width follows the scale with the text.
#[test]
fn the_toast_scales() {
    let th = theme::DARK;
    let size = |s: f32, msg: &str| chrome::toast_size(&chrome::Ui { th: &th, s, font: Some(fonts::ui_font()) }, msg);
    let msg = "UI size 125 %";
    let (w1, h1) = size(1.0, msg);
    assert_eq!(h1, 36.0);
    for s in [0.75f32, 2.0] {
        let (w, h) = size(s, msg);
        assert_eq!(h, 36.0 * s, "{s}");
        assert!((w - w1 * s).abs() <= 2.0, "{s}: {w} vs {}", w1 * s);
    }
    // Through the overlay: the scene's toast is drawn at the UI scale.
    for (to, k) in [(UiSize::Pct(75), 0.75), (UiSize::Pct(200), 2.0)] {
        let mut a = annotated(theme::DARK, BIG);
        ed(&mut a).set_ui_size(to, Instant::now());
        let m = a.mouse;
        ed(&mut a).prepare(None, m, Instant::now());
        assert_eq!(ed(&mut a).ui_scale(), k);
        let text = ed(&mut a).scene.as_ref().and_then(|sc| sc.toast.as_ref()).map(|t| t.text.clone());
        assert_eq!(text, Some(to.describe()));
    }
}

/// The size toast says a change once: not when nothing changed, and
/// never over an error that is still showing.
#[test]
fn the_size_toast_is_for_changes_and_spares_errors() {
    let mut a = annotated(theme::DARK, BIG);
    let t = Instant::now();
    ed(&mut a).set_ui_size(UiSize::Auto, t);
    assert!(ed(&mut a).notice.is_none(), "unchanged: no toast");
    ed(&mut a).set_ui_size(UiSize::Pct(200), t);
    assert_eq!(toast(&mut a), "UI size 200 %");
    ed(&mut a).notice = None;
    bigger(&mut a);
    assert!(ed(&mut a).notice.is_none(), "already the largest: nothing to say");
    ed(&mut a).notice = Some(Toast::new("Could not save", ToastKind::Error));
    smaller(&mut a);
    assert_eq!(ed(&mut a).cfg.ui_scale, UiSize::Pct(150), "the size still changed");
    assert_eq!(toast(&mut a), "Could not save", "the error stays");
    assert_eq!(ed(&mut a).notice.as_ref().map(|t| t.kind), Some(ToastKind::Error));
    // An older info toast is replaced.
    ed(&mut a).notice = Some(Toast::new("Saved", ToastKind::Success));
    smaller(&mut a);
    assert_eq!(toast(&mut a), "UI size 125 %");
}

/// Two monitors of different OS scale at 125 %: each gets its own scale
/// times 1.25 (clamped like the overlay's), a monitor the system does not
/// know falls back to the shot's, and the camera follows the area's.
#[test]
fn mixed_dpi_monitors_at_125_percent() {
    let mut a = annotated(theme::DARK, BIG);
    let mons = [
        capture::MonInfo { x: 0, y: 0, w: 720, h: 900, scale: 1.0, primary: true },
        capture::MonInfo { x: 720, y: 0, w: 720, h: 900, scale: 1.75, primary: false },
    ];
    ed(&mut a).shot.monitors = vec![(0, 0, 720, 900), (720, 0, 720, 900)];
    ed(&mut a).set_ui_size(UiSize::Pct(125), Instant::now());
    let g = ed(&mut a).rec_geom(&mons);
    assert_eq!(g.monitors.iter().map(|m| m.1).collect::<Vec<_>>(), [1.25, 2.1875]);
    assert_eq!(g.mult, 1.25);
    assert_eq!(g.scale, 1.25, "the shot's scale (100 %) times it");
    // The border keeps the OS scale: 2 px, and 1.75 * 2 = 3.5 rounds to 4.
    assert_eq!(g.border_monitors().iter().map(|m| m.1).collect::<Vec<_>>(), [2, 4]);
    // An OS scale past the clamp counts as 4 (as the overlay always did).
    let wild = [
        capture::MonInfo { x: 0, y: 0, w: 720, h: 900, scale: 1.0, primary: true },
        capture::MonInfo { x: 720, y: 0, w: 720, h: 900, scale: 6.0, primary: false },
    ];
    let g = ed(&mut a).rec_geom(&wild);
    assert_eq!(g.monitors[1].1, 5.0);
    // No match: the shot's scale.
    let g = ed(&mut a).rec_geom(&[]);
    assert!(g.monitors.iter().all(|m| m.1 == 1.25));
    // The area on the second monitor: the bubble at that monitor's scale.
    ed(&mut a).sel = Some(r(800.0, 100.0, 400.0, 300.0));
    let g = ed(&mut a).rec_geom(&mons);
    assert_eq!(ed(&mut a).cam_want(&g).scale, 2.1875);
}

/// The border does not follow the UI size: 75 % and 200 % give the same
/// thickness as Auto on each monitor.
#[test]
fn the_recording_border_ignores_the_ui_size() {
    let mons = [
        capture::MonInfo { x: 0, y: 0, w: 720, h: 900, scale: 1.0, primary: true },
        capture::MonInfo { x: 720, y: 0, w: 720, h: 900, scale: 2.0, primary: false },
    ];
    let thick = |size: UiSize| {
        let mut a = annotated(theme::DARK, BIG);
        ed(&mut a).shot.monitors = vec![(0, 0, 720, 900), (720, 0, 720, 900)];
        ed(&mut a).cfg.ui_scale = size;
        ed(&mut a).rec_geom(&mons).border_monitors().iter().map(|m| m.1).collect::<Vec<_>>()
    };
    assert_eq!(thick(UiSize::Auto), [2, 4]);
    assert_eq!(thick(UiSize::Pct(75)), [2, 4]);
    assert_eq!(thick(UiSize::Pct(200)), [2, 4]);
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
