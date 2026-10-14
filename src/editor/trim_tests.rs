//! Trim to content in the overlay: the key and the size label's menu
//! tighten the selection, undo and redo put it back and again, Settings'
//! "Trim margins automatically" trims every new region or window, a
//! window's client area is tried when its frame has nothing to trim; the
//! export border ("Add a border": around the export, previewed around the
//! selection, never in the history's original); plus the preview PNGs.

use super::tests::{assert_dirty_covers, edit_of, preview_app_with, settled};
use super::*;
use crate::snap::{Node, Snapshot, R};
use crate::wind::key;

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };

fn fr(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

/// A white desktop with a blue card at (300, 200) 400 x 250, and a window
/// at (100, 80) 900 x 600 whose title bar (40 px) has a darker "title" in
/// it and whose client area is light gray around a green box.
fn shot() -> Shot {
    let (w, h) = (1200u32, 800u32);
    let mut img = PixBuf::from_pixel(w, h, [255, 255, 255, 255]);
    let mut put = |x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 3]| {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y as usize * w as usize + x as usize) * 4;
                img.as_raw_mut()[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    };
    put(300, 200, 700, 450, [40, 90, 220]);
    Shot { origin: (0, 0), size: (w, h), scale: 1.0, image: img, monitors: vec![(0, 0, w, h)] }
}

/// [`shot`] with the window drawn on it (and the card under it).
fn window_shot() -> Shot {
    let mut s = shot();
    let w = s.size.0 as usize;
    let mut put = |x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 3]| {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y as usize * w + x as usize) * 4;
                s.image.as_raw_mut()[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    };
    put(100, 80, 1000, 680, [90, 90, 100]); // the frame
    put(100, 80, 1000, 120, [60, 60, 70]); // title bar
    put(110, 81, 400, 119, [230, 230, 230]); // its title
    put(101, 120, 999, 679, [235, 235, 235]); // client area (a 1 px frame)
    put(400, 300, 600, 420, [30, 160, 80]);
    s
}

fn app_with(sel: Option<FRect>, s: Shot) -> App {
    let mut a = preview_app_with(theme::DARK, sel, s);
    a.focus_tries = u8::MAX;
    a
}

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
}

fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

fn click(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Move { x, y });
    a.on_event(Ev::Down { x, y });
    a.on_event(Ev::Up { x, y });
}

/// The card with its 2 px of margin.
const CARD: FRect = FRect { x: 298.0, y: 198.0, w: 404.0, h: 254.0 };

#[test]
fn shift_a_trims_and_undo_redo_restore_the_selection() {
    let around = fr(100.0, 100.0, 800.0, 500.0);
    let mut a = app_with(Some(around), shot());
    key_m(&mut a, 'A' as u32, SHIFT);
    assert_eq!(ed(&mut a).sel, Some(CARD));
    assert_eq!(ed(&mut a).tool, None, "Shift+A is not the arrow");
    key_m(&mut a, 'Z' as u32, CTRL);
    assert_eq!(ed(&mut a).sel, Some(around), "undo puts it back");
    key_m(&mut a, 'Y' as u32, CTRL);
    assert_eq!(ed(&mut a).sel, Some(CARD), "redo trims again");
    // Again on the trimmed selection: nothing more to cut, a toast.
    key_m(&mut a, 'A' as u32, SHIFT);
    assert_eq!(ed(&mut a).sel, Some(CARD));
    assert_eq!(ed(&mut a).notice.as_ref().map(|t| t.text.as_str()), Some(trim_sel::NOTHING.get()));
    // Plain A is still the arrow.
    key_m(&mut a, 'A' as u32, NONE);
    assert_eq!(ed(&mut a).tool, Some(Tool::Arrow));
}

#[test]
fn objects_and_the_trim_share_one_undo_history() {
    let around = fr(100.0, 100.0, 800.0, 500.0);
    let mut a = app_with(Some(around), shot());
    let e = ed(&mut a);
    e.commit_object(Obj::Rect { r: fr(320.0, 220.0, 50.0, 50.0), color: C4::rgb(255, 0, 0), width: 3.0 });
    assert!(e.trim_selection(true));
    // Undo: the trim first (the rect stays), then the rect.
    e.undo();
    assert_eq!((e.sel, e.objects.len()), (Some(around), 1));
    e.undo();
    assert_eq!((e.sel, e.objects.len()), (Some(around), 0));
    e.redo();
    e.redo();
    assert_eq!((e.sel, e.objects.len()), (Some(CARD), 1));
    // A new step after an undo drops the trim's redo.
    e.undo();
    e.commit_object(Obj::Rect { r: fr(330.0, 230.0, 50.0, 50.0), color: C4::rgb(0, 255, 0), width: 3.0 });
    e.redo();
    assert_eq!(e.sel, Some(around));
}

#[test]
fn trimming_never_reads_the_objects() {
    // A rect drawn in the margin does not keep the margin.
    let mut a = app_with(Some(fr(100.0, 100.0, 800.0, 500.0)), shot());
    let e = ed(&mut a);
    e.commit_object(Obj::Rect { r: fr(120.0, 120.0, 40.0, 40.0), color: C4::rgb(0, 0, 0), width: 4.0 });
    e.rebuild();
    assert_eq!(e.trimmed_sel(), Some(CARD));
}

#[test]
fn trim_margins_trims_every_new_region_and_window() {
    let mut a = app_with(None, shot());
    ed(&mut a).cfg.trim_margins = true;
    a.on_event(Ev::Move { x: 150, y: 120 });
    a.on_event(Ev::Down { x: 150, y: 120 });
    a.on_event(Ev::Move { x: 400, y: 300 });
    a.on_event(Ev::Move { x: 850, y: 600 });
    a.on_event(Ev::Up { x: 850, y: 600 });
    assert_eq!(ed(&mut a).sel, Some(CARD), "the dragged region");
    key_m(&mut a, 'Z' as u32, CTRL);
    assert_eq!(ed(&mut a).sel, Some(fr(150.0, 120.0, 700.0, 480.0)), "undo: as dragged");
    // A window clicked: trimmed too (its frame is not uniform: the client
    // area is tried).
    let mut a = app_with(None, window_shot());
    ed(&mut a).cfg.trim_margins = true;
    let app_win = Node::new(R::xywh(100, 80, 900, 600), vec![]).with_client(Some(R::xywh(101, 120, 898, 559)));
    ed(&mut a).snap = Some(Snapshot::build(std::slice::from_ref(&app_win), (0, 0), (1200, 800)));
    click(&mut a, 500, 600);
    assert_eq!(ed(&mut a).sel, Some(fr(398.0, 298.0, 204.0, 124.0)), "the green box in the client area");
    // Off (the default): as selected.
    let mut a = app_with(None, shot());
    a.on_event(Ev::Down { x: 150, y: 120 });
    a.on_event(Ev::Move { x: 850, y: 600 });
    a.on_event(Ev::Up { x: 850, y: 600 });
    assert_eq!(ed(&mut a).sel, Some(fr(150.0, 120.0, 700.0, 480.0)));
    // A plain click on the desktop selects the whole screen: not trimmed.
    let mut a = app_with(None, shot());
    ed(&mut a).cfg.trim_margins = true;
    ed(&mut a).cfg.snap_windows = false;
    click(&mut a, 50, 50);
    assert_eq!(ed(&mut a).sel, Some(fr(0.0, 0.0, 1200.0, 800.0)));
}

#[test]
fn a_window_frame_without_margins_falls_back_to_its_client_area() {
    let frame = fr(100.0, 80.0, 900.0, 600.0);
    let mut a = app_with(Some(frame), window_shot());
    // No snapshot: the frame's title bar keeps every side.
    assert_eq!(ed(&mut a).trimmed_sel(), None);
    let app_win = Node::new(R::xywh(100, 80, 900, 600), vec![]).with_client(Some(R::xywh(101, 120, 898, 559)));
    ed(&mut a).snap = Some(Snapshot::build(std::slice::from_ref(&app_win), (0, 0), (1200, 800)));
    assert_eq!(ed(&mut a).trimmed_sel(), Some(fr(398.0, 298.0, 204.0, 124.0)));
    // A client area with nothing to trim: the window stays whole.
    let mut s = window_shot();
    let w = s.size.0 as usize;
    for y in 120..679usize {
        for x in 101..999usize {
            let v = ((x * 31 + y * 17) % 251) as u8;
            s.image.as_raw_mut()[(y * w + x) * 4..][..3].copy_from_slice(&[v, v, v]);
        }
    }
    let mut a = app_with(Some(frame), s);
    ed(&mut a).snap = Some(Snapshot::build(std::slice::from_ref(&app_win), (0, 0), (1200, 800)));
    assert_eq!(ed(&mut a).trimmed_sel(), None);
    // A region around the window (no snap) never takes its client area.
    let mut a = app_with(Some(fr(90.0, 70.0, 920.0, 620.0)), window_shot());
    ed(&mut a).snap = Some(Snapshot::build(std::slice::from_ref(&app_win), (0, 0), (1200, 800)));
    assert_eq!(ed(&mut a).trimmed_sel(), Some(fr(98.0, 78.0, 904.0, 604.0)), "only the white around the window");
}

/// The size label's menu: a click on the label opens it under the label,
/// Esc or a click elsewhere closes it, its row trims.
#[test]
fn the_size_label_menu_trims() {
    let around = fr(100.0, 100.0, 800.0, 500.0);
    let mut a = app_with(Some(around), shot());
    settled(&mut a);
    let label = ed(&mut a).label_at.expect("the label is drawn");
    let (lx, ly) = ((label.x + label.w / 2.0) as i32, (label.y + label.h / 2.0) as i32);
    click(&mut a, lx, ly);
    assert!(ed(&mut a).size_menu);
    settled(&mut a);
    let tb = ed(&mut a).toolbar.clone().expect("toolbar");
    let pop = tb.pop.expect("the menu");
    assert!(pop.y >= label.y1() && (pop.x - label.x).abs() < 1.0, "under the label: {pop:?} {label:?}");
    let row = tb.items.iter().find(|i| matches!(i.kind, toolbar::Kind::MenuItem { act: Act::Trim, .. })).expect("Trim row").r;
    // (Shift is "⇧" on macOS.)
    assert_eq!(tb.texts.last().cloned(), Some(format!("{}+A", chrome::SHIFT)), "its key");
    // Esc closes it (the capture stays).
    key_m(&mut a, key::ESCAPE, NONE);
    assert!(!ed(&mut a).size_menu && !ed(&mut a).done);
    // A click elsewhere closes it and does nothing else.
    click(&mut a, lx, ly);
    click(&mut a, 1100, 750);
    assert!(!ed(&mut a).size_menu);
    assert_eq!(ed(&mut a).sel, Some(around));
    // The row trims.
    click(&mut a, lx, ly);
    settled(&mut a);
    click(&mut a, (row.x + row.w / 2.0) as i32, (row.y + row.h / 2.0) as i32);
    assert_eq!(ed(&mut a).sel, Some(CARD));
    assert!(!ed(&mut a).size_menu);
}

/// With a drawing tool a press on the label draws as anywhere else; the
/// select tool opens the menu; video mode never opens it (nor swallows
/// the next press).
#[test]
fn the_label_menu_waits_for_no_drawing_tool() {
    // A whole-screen selection: the label sits inside it.
    let mut a = app_with(Some(fr(0.0, 0.0, 1200.0, 800.0)), shot());
    settled(&mut a);
    let label = ed(&mut a).label_at.expect("label");
    let (lx, ly) = ((label.x + label.w / 2.0) as i32, (label.y + label.h / 2.0) as i32);
    key_m(&mut a, 'R' as u32, NONE);
    a.on_event(Ev::Move { x: lx, y: ly });
    a.on_event(Ev::Down { x: lx, y: ly });
    a.on_event(Ev::Move { x: lx + 60, y: ly + 50 });
    a.on_event(Ev::Up { x: lx + 60, y: ly + 50 });
    assert!(!ed(&mut a).size_menu, "the rectangle tool draws");
    assert_eq!(ed(&mut a).objects.len(), 1);
    key_m(&mut a, 'V' as u32, NONE);
    settled(&mut a);
    click(&mut a, lx, ly);
    assert!(ed(&mut a).size_menu, "the select tool opens the menu");
    // Video mode: no menu, and the next press is not swallowed.
    let mut a = app_with(Some(fr(100.0, 100.0, 800.0, 500.0)), shot());
    ed(&mut a).rec.open = true;
    ed(&mut a).size_menu = true;
    settled(&mut a);
    assert!(!ed(&mut a).size_menu, "a menu that cannot open is not left open");
    let label = ed(&mut a).label_at.expect("label");
    assert!(!ed(&mut a).on_label(Pt::new(label.x + 2.0, label.y + 2.0)));
    // No border preview there either (recordings get none).
    ed(&mut a).cfg.border_width = 3;
    settled(&mut a);
    assert!(ed(&mut a).scene.as_ref().and_then(|s| s.sel_ui.as_ref()).is_some_and(|u| u.border.is_none()));
}

/// Opening and closing the menu, and the trim, repaint every pixel
/// that changed.
#[test]
fn the_menu_and_the_trim_repaint_what_changed() {
    for th in [theme::DARK, theme::LIGHT] {
        let mut a = app_with(Some(fr(100.0, 100.0, 800.0, 500.0)), shot());
        ed(&mut a).th = th;
        ed(&mut a).cfg.magnifier = false;
        ed(&mut a).assist.dismiss_hint(Instant::now());
        settled(&mut a);
        let label = ed(&mut a).label_at.expect("label");
        let (lx, ly) = ((label.x + 4.0) as i32, (label.y + 4.0) as i32);
        assert_dirty_covers(&mut a, "menu opens", |a| click(a, lx, ly));
        assert_dirty_covers(&mut a, "menu closes", |a| key_m(a, key::ESCAPE, NONE));
        assert_dirty_covers(&mut a, "trimmed", |a| key_m(a, 'A' as u32, SHIFT));
        assert_dirty_covers(&mut a, "undone", |a| key_m(a, 'Z' as u32, CTRL));
    }
}

/// The export carries the border (at the capture's scale), its top-left
/// moved out by it; the history keeps the capture without it.
#[test]
fn the_export_has_the_border_and_the_history_original_not() {
    let mut s = shot();
    s.scale = 1.5;
    let mut a = app_with(Some(fr(300.0, 200.0, 400.0, 250.0)), s);
    a.history_root = Some(std::env::temp_dir().join("rustshot-border-hist-unused"));
    let e = ed(&mut a);
    e.cfg.border_width = 4;
    e.cfg.border_color = "#ff0000".into();
    e.tasks = vec![Task::Copy];
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.finish(&mut e, false);
    let State::Finish(job) = &a.st else { panic!("finish job") };
    // 4 logical px at 150 % are 6 px on each side.
    assert_eq!(job.img.dimensions(), (412, 262));
    assert_eq!(job.sel_global, (294, 194));
    assert_eq!(job.img.get_pixel(0, 0), [255, 0, 0, 255]);
    assert_eq!(job.img.get_pixel(5, 130), [255, 0, 0, 255]);
    assert_eq!(job.img.get_pixel(6, 130), [40, 90, 220, 255], "the capture inside");
    let h = job.hist.as_ref().expect("history shot");
    assert_eq!(h.original.dimensions(), (400, 250), "no border in the history");
    assert_eq!(a.last_region, Some((300, 200, 400, 250)), "Repeat takes the region, not the border");
    // Copy text reads the capture alone.
    let mut a = app_with(Some(fr(300.0, 200.0, 400.0, 250.0)), shot());
    let e = ed(&mut a);
    e.cfg.border_width = 4;
    e.tasks = vec![Task::Text];
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.finish(&mut e, false);
    let State::Finish(job) = &a.st else { panic!("finish job") };
    assert_eq!((job.img.dimensions(), job.sel_global), ((400, 250), (300, 200)), "no border for OCR");
}

/// Settings › Saving "Save at 1×": a 200 % capture goes out at half its
/// pixels (Copy, Save, Upload, Pin), the history keeps all of them.
#[test]
fn save_at_one_x_halves_the_export_and_the_history_keeps_it_all() {
    let mut s = shot();
    s.scale = 2.0;
    let mut a = app_with(Some(fr(300.0, 200.0, 400.0, 250.0)), s);
    a.history_root = Some(std::env::temp_dir().join("rustshot-saveat-hist-unused"));
    let e = ed(&mut a);
    e.cfg.save_scale = "logical".into();
    e.cfg.border_width = 2;
    // (The thumbnail card has a Pin too: with it on, its screen-size
    // picture is kept as well.)
    e.cfg.show_thumbnail = false;
    e.tasks = vec![Task::Copy];
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.finish(&mut e, false);
    let State::Finish(job) = &a.st else { panic!("finish job") };
    // 200 x 125 at 1x, and a 2 px border (2 logical px at 1x).
    assert_eq!(job.img.dimensions(), (204, 129));
    assert_eq!(job.sel_global, (298, 198));
    assert_eq!(job.hist.as_ref().expect("history shot").original.dimensions(), (400, 250), "the history keeps the screen's pixels");
    assert!(job.pin.is_none(), "no pin asked for");
    // A pin shows the capture at screen size (its border at 200 %: 4 px).
    let mut s = shot();
    s.scale = 2.0;
    let mut a = app_with(Some(fr(300.0, 200.0, 400.0, 250.0)), s);
    let e = ed(&mut a);
    e.cfg.save_scale = "logical".into();
    e.cfg.border_width = 2;
    e.tasks = vec![Task::Copy, Task::Pin];
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.finish(&mut e, false);
    let State::Finish(job) = &a.st else { panic!("finish job") };
    assert_eq!(job.img.dimensions(), (204, 129), "Copy: at 1x");
    let (pin, at) = job.pin.as_ref().expect("the pin's own picture");
    assert_eq!((pin.dimensions(), *at), ((408, 258), (296, 196)));
}

/// The preview: a band of the border color outside the outline, nothing
/// inside the selection; none when off.
#[test]
fn the_border_is_previewed_around_the_selection() {
    let sel = fr(300.0, 200.0, 400.0, 250.0);
    let mut a = app_with(Some(sel), shot());
    let plain = settled(&mut a);
    let e = ed(&mut a);
    e.cfg.border_width = 3;
    e.cfg.border_color = "#00ff00".into();
    let framed = settled(&mut a);
    // 1 px outside the outline (2 px at 100 %): green, 3 px wide (at the
    // sides: clear of the label's and the toolbar's shadows, the handles).
    for d in 3..6 {
        assert_eq!(framed.get_pixel(300 - 1 - d, 330), [0, 255, 0, 255], "left, {d}");
        assert_eq!(framed.get_pixel(700 + d, 290), [0, 255, 0, 255], "right, {d}");
    }
    assert_ne!(framed.get_pixel(300 - 7, 330), [0, 255, 0, 255], "3 px wide");
    assert_ne!(framed.get_pixel(300 - 3, 330), [0, 255, 0, 255], "clear of the outline");
    assert_eq!(framed.get_pixel(500, 300), plain.get_pixel(500, 300), "the selection untouched");
    assert_dirty_covers(&mut a, "border off", |a| ed(a).cfg.border_width = 0);
}

#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_trim_preview_pngs() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        let mut a = app_with(Some(fr(100.0, 100.0, 800.0, 500.0)), shot());
        ed(&mut a).th = th;
        ed(&mut a).size_menu = true;
        std::fs::write(dir.join(format!("trim-{tn}-menu.png")), settled(&mut a).to_png().unwrap()).unwrap();
        ed(&mut a).trim_selection(true);
        std::fs::write(dir.join(format!("trim-{tn}-trimmed.png")), settled(&mut a).to_png().unwrap()).unwrap();
        ed(&mut a).cfg.border_width = 4;
        ed(&mut a).cfg.border_color = "#f04438".into();
        std::fs::write(dir.join(format!("border-{tn}-preview.png")), settled(&mut a).to_png().unwrap()).unwrap();
    }
}
