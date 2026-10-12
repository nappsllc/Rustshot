//! The Edit window without a window: the save flow's state machine, the
//! files it writes (the original never), the window driven with events
//! (tools, drawing at a zoom, zoom and pan, the list), preview PNGs of the
//! board's states dark and light (`RUSTSHOT_PREVIEW_DIR`), and the frame
//! time on a 4K capture.

use super::window::Window;
use super::*;
use crate::history::tests::TempRoot;
use crate::history::Meta;
use crate::objects::{CalloutStyle, FRect, Obj, Pt};
use crate::pixbuf::PixBuf;
use crate::theme::{DARK, LIGHT, Theme};
use crate::uifb::C4;
use crate::wind::{Ev, Mods};

#[test]
fn the_save_flow() {
    let mut f = Flow::default();
    // Save asks; the answer writes (and may be remembered).
    assert!(f.save(None, false).is_empty());
    assert_eq!(f.dialog, Some(Dialog::Choose { choice: Choice::Overwrite, dont_ask: false, close: false }));
    assert!(f.press(Btn::Pick(Choice::New), None).is_empty());
    assert!(f.press(Btn::DontAsk, None).is_empty());
    assert_eq!(f.press(Btn::Save, None), [Do::Write { choice: Choice::New, close: false, remember: true }]);
    assert_eq!(f.dialog, None);
    // Remembered: no question.
    assert_eq!(f.save(Some(Choice::Overwrite), false), [Do::Write { choice: Choice::Overwrite, close: false, remember: false }]);
    // Closing: nothing unsaved closes; unsaved asks.
    assert_eq!(f.close(false), [Do::Close]);
    assert!(f.close(true).is_empty());
    assert_eq!(f.dialog, Some(Dialog::Unsaved));
    assert!(f.close(true).is_empty(), "asked once");
    // Cancel stays, Discard closes, Save as copy writes a new file and closes.
    assert!(f.escape().is_empty() && f.dialog.is_none());
    f.close(true);
    assert_eq!(f.press(Btn::Discard, None), [Do::Close]);
    f.close(true);
    assert_eq!(f.press(Btn::SaveCopy, None), [Do::Write { choice: Choice::New, close: true, remember: false }]);
    // Save from the close question: asks which, then writes and closes.
    f.close(true);
    assert!(f.press(Btn::Save, None).is_empty());
    assert_eq!(f.dialog, Some(Dialog::Choose { choice: Choice::Overwrite, dont_ask: false, close: true }));
    assert_eq!(f.enter(None), [Do::Write { choice: Choice::Overwrite, close: true, remember: false }]);
    f.close(true);
    assert_eq!(f.press(Btn::Save, Some(Choice::New)), [Do::Write { choice: Choice::New, close: true, remember: false }]);
    // Buttons of the other question do nothing.
    f.close(true);
    assert!(f.press(Btn::DontAsk, None).is_empty() && f.dialog == Some(Dialog::Unsaved));
    assert_eq!(Choice::from_config("new"), Some(Choice::New));
    assert_eq!(Choice::from_config(""), None);
}

#[test]
fn edited_names() {
    let r = TempRoot::new("names");
    std::fs::create_dir_all(&r.0).unwrap();
    let p = r.0.join("2026-10-05_14-32.png");
    assert_eq!(edited_path(&p), r.0.join("2026-10-05_14-32-edited.png"));
    std::fs::write(r.0.join("2026-10-05_14-32-edited.png"), b"x").unwrap();
    assert_eq!(edited_path(&p), r.0.join("2026-10-05_14-32-edited_1.png"));
    let cfg = Config::default();
    assert_eq!(target(Choice::Overwrite, Some(&p), &cfg), p);
    // Never saved: a new file either way.
    let auto = target(Choice::Overwrite, None, &cfg);
    assert!(!auto.exists() && auto.extension().is_some());
}

/// Overwrite replaces the saved file's bytes under its own name (no
/// `_1` copy next to it) and the entry keeps pointing at it; Save as new
/// file writes `-edited` and the entry points there.
#[test]
fn overwrite_replaces_the_saved_file_in_place() {
    let (r, mut w) = window("over", page(400, 300), Vec::new(), DARK, 1.0);
    let saved = r.0.join("2026-10-05_14-32.png");
    let before = std::fs::read(&saved).unwrap();
    w.e.canvas.edit_mut_for_tests().push(Obj::Rect { r: FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 }, color: red(), width: 3.0 });
    w.e.perform(Do::Write { choice: Choice::Overwrite, close: false, remember: false });
    let after = std::fs::read(&saved).unwrap();
    assert_ne!(after, before, "the bytes are replaced");
    let pic = PixBuf::from_png(&after).unwrap();
    assert_eq!(&pic.as_raw()[(10 * 400 + 30) * 4..][..3], &[0xF0, 0x44, 0x38], "the rectangle is in it");
    let names: Vec<String> = std::fs::read_dir(&r.0).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".png") || n.ends_with(".tmp")).collect();
    assert_eq!(names, ["2026-10-05_14-32.png"], "no copy, no temporary file");
    let e = w.e.store.get(&w.e.entry.id).unwrap();
    assert_eq!(e.meta.saved.as_deref(), Some(saved.as_path()), "the entry is the same file");
    assert_eq!(e.meta.bytes, after.len() as u64);
    assert!(e.meta.edited && !w.e.canvas.unsaved());
    // Again: still the same file.
    w.e.canvas.edit_mut_for_tests().push(Obj::Rect { r: FRect { x: 100.0, y: 100.0, w: 50.0, h: 40.0 }, color: red(), width: 3.0 });
    w.e.perform(Do::Write { choice: Choice::Overwrite, close: false, remember: false });
    assert!(!r.0.join("2026-10-05_14-32_1.png").exists());
    assert_eq!(w.e.entry.meta.saved.as_deref(), Some(saved.as_path()));
    // Save as new file: next to it, and the entry follows.
    w.e.canvas.edit_mut_for_tests().pop();
    w.e.perform(Do::Write { choice: Choice::New, close: false, remember: false });
    let edited = r.0.join("2026-10-05_14-32-edited.png");
    assert!(edited.is_file());
    assert_eq!(std::fs::read(&saved).unwrap().len() as u64, std::fs::metadata(&saved).unwrap().len());
    assert_eq!(w.e.store.get(&w.e.entry.id).unwrap().meta.saved.as_deref(), Some(edited.as_path()));
    assert_eq!(w.e.entry.meta.saved.as_deref(), Some(edited.as_path()), "the window's entry is refreshed");
}

/// A Save As that was cancelled (or failed) saves nothing: the edits stay
/// unsaved, so closing still asks. One that wrote a file saves the
/// objects it wrote, not the ones added since.
#[test]
fn a_cancelled_save_as_keeps_the_edits_unsaved() {
    let (r, mut w) = window("saveas", page(400, 300), Vec::new(), DARK, 1.0);
    let rect = Obj::Rect { r: FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 }, color: red(), width: 3.0 };
    w.e.canvas.edit_mut_for_tests().push(rect.clone());
    let objs = w.e.canvas.objects().to_vec();
    let cancelled = crate::export::ExportResult { messages: vec![], error: true, done: vec![], upload: None };
    w.apply_done_for_tests(cancelled, true, objs.clone());
    assert!(w.e.canvas.unsaved(), "cancelled: still unsaved");
    let failed = crate::export::ExportResult { messages: vec![], error: true, done: vec![crate::export::Done::Failed("Save", "disk full".into())], upload: None };
    w.apply_done_for_tests(failed, true, objs.clone());
    assert!(w.e.canvas.unsaved(), "failed: still unsaved");
    at(&mut w, Ev::Close);
    assert_eq!(w.e.flow.dialog, Some(Dialog::Unsaved), "closing asks");
    key(&mut w, crate::wind::key::ESCAPE, Mods::default());
    // Written: saved as of when it started (a later edit stays unsaved).
    let out = r.0.join("elsewhere.png");
    std::fs::write(&out, b"png").unwrap();
    let ok = crate::export::ExportResult { messages: vec![], error: false, done: vec![crate::export::Done::Saved(out.clone())], upload: None };
    w.apply_done_for_tests(ok, true, objs.clone());
    assert!(!w.e.canvas.unsaved());
    assert_eq!(w.e.entry.meta.saved.as_deref(), Some(out.as_path()), "the entry is refreshed");
    w.e.canvas.edit_mut_for_tests().push(rect);
    w.apply_done_for_tests(crate::export::ExportResult { messages: vec![], error: false, done: vec![crate::export::Done::Saved(out)], upload: None }, true, objs);
    assert!(w.e.canvas.unsaved(), "the objects added after it are not saved");
}

/// The entry was pruned or deleted while the window had it open: Save
/// still writes the user's file, says so, and the edits are saved.
#[test]
fn saving_after_the_entry_is_gone() {
    let (r, mut w) = window("gone", page(200, 100), Vec::new(), DARK, 1.0);
    w.e.canvas.edit_mut_for_tests().push(Obj::Rect { r: FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 }, color: red(), width: 3.0 });
    w.e.store.delete(&w.e.entry.id, false).unwrap();
    w.e.perform(Do::Write { choice: Choice::New, close: false, remember: false });
    assert!(r.0.join("2026-10-05_14-32-edited.png").is_file());
    assert!(!w.e.canvas.unsaved());
    let (text, kind, _) = w.e.toast.clone().unwrap();
    assert_eq!(text, "Saved 2026-10-05_14-32-edited.png (History entry gone)");
    assert_eq!(kind, crate::editor::style::ToastKind::Info);
}

/// The daemon quitting: unsaved edits ask "Save changes?" (Cancel calls
/// the quit off); none close at once.
#[test]
fn quitting_asks_about_unsaved_edits() {
    let (_r, mut w) = window("quit", page(200, 100), Vec::new(), DARK, 1.0);
    w.e.quit();
    assert!(w.e.closing, "nothing unsaved: closes");
    let (_r2, mut w) = window("quit2", page(200, 100), Vec::new(), DARK, 1.0);
    w.e.canvas.edit_mut_for_tests().push(Obj::Rect { r: FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 }, color: red(), width: 3.0 });
    w.e.quit();
    assert_eq!(w.e.flow.dialog, Some(Dialog::Unsaved));
    let _ = OPEN.take_cancelled();
    key(&mut w, crate::wind::key::ESCAPE, Mods::default());
    assert!(!w.e.closing && !w.e.quitting);
    assert!(OPEN.take_cancelled(), "Cancel calls the quit off");
}

/// Text being typed counts as unsaved, and Save writes it.
#[test]
fn typed_text_is_unsaved_and_saved() {
    let (_r, mut w) = window("text", page(400, 300), Vec::new(), DARK, 1.0);
    let c = w.e.canvas.button("text").expect("the Text tool");
    click(&mut w, Pt::new(c.x + c.w / 2.0, c.y + c.h / 2.0));
    let p = w.e.canvas.view.to_win(Pt::new(100.0, 100.0));
    click(&mut w, p);
    for ch in "hi".encode_utf16() {
        at(&mut w, Ev::Char(ch));
    }
    assert!(w.e.canvas.unsaved(), "typing");
    w.e.perform(Do::Write { choice: Choice::New, close: false, remember: false });
    assert!(!w.e.canvas.unsaved());
    assert!(w.e.canvas.objects().iter().any(|o| matches!(o, Obj::Text { text, .. } if text == "hi")));
}

/// A middle-button release does not end a left-button drag.
#[test]
fn a_middle_release_leaves_a_left_drag_alone() {
    let (_r, mut w) = window("middle", page(400, 300), Vec::new(), DARK, 1.0);
    let c = w.e.canvas.button("rect").expect("the Rectangle tool");
    click(&mut w, Pt::new(c.x + c.w / 2.0, c.y + c.h / 2.0));
    let v = w.e.canvas.view;
    let [a, m, b] = [(50.0, 50.0), (80.0, 70.0), (150.0, 120.0)].map(|(x, y)| v.to_win(Pt::new(x, y)));
    at(&mut w, Ev::Move { x: a.x as i32, y: a.y as i32 });
    at(&mut w, Ev::Down { x: a.x as i32, y: a.y as i32 });
    at(&mut w, Ev::Move { x: m.x as i32, y: m.y as i32 });
    at(&mut w, Ev::Middle { down: false, x: m.x as i32, y: m.y as i32 });
    assert!(w.e.canvas.objects().is_empty(), "the drag goes on");
    at(&mut w, Ev::Move { x: b.x as i32, y: b.y as i32 });
    at(&mut w, Ev::Up { x: b.x as i32, y: b.y as i32 });
    assert_eq!(w.e.canvas.objects().len(), 1);
    let Obj::Rect { r, .. } = &w.e.canvas.objects()[0] else { panic!("a rectangle") };
    assert!((r.w - 100.0).abs() < 3.0, "drawn to the last point: {r:?}");
}

/// A mock page like the board's (a side bar, cards).
fn page(w: u32, h: u32) -> PixBuf {
    let mut img = PixBuf::from_pixel(w, h, [0xF6, 0xF7, 0xF9, 255]);
    let mut rect = |x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 3]| {
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                let i = ((y * w + x) * 4) as usize;
                img.as_raw_mut()[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    };
    let k = w as f32 / 1180.0;
    let s = |v: f32| (v * k) as u32;
    rect(0, 0, s(208.0), h, [0xEE, 0xF0, 0xF4]);
    for (i, top) in [60.0f32, 250.0, 400.0].iter().enumerate() {
        rect(s(236.0), s(*top), s(1150.0), s(top + if i == 2 { 320.0 } else { 160.0 }), [0xFF, 0xFF, 0xFF]);
        rect(s(258.0), s(top + 22.0), s(296.0), s(top + 60.0), [[0x8E, 0x96, 0xE8], [0xF0, 0x84, 0x6B], [0x7D, 0xD3, 0xA8]][i]);
        rect(s(312.0), s(top + 26.0), s(1090.0), s(top + 38.0), [0xE6, 0xE8, 0xEE]);
    }
    rect(s(312.0), s(480.0), s(840.0), s(690.0), [0xDC, 0xE6, 0xF5]);
    img
}

fn red() -> C4 {
    C4::rgb(0xF0, 0x44, 0x38)
}

/// The board's objects: an arrow, numbers 1 and 2, the callout (front).
fn objects() -> Vec<Obj> {
    vec![
        Obj::Arrow { a: Pt::new(417.0, 653.0), b: Pt::new(615.0, 530.0), color: red(), width: 4.0 },
        Obj::Counter { c: Pt::new(275.0, 103.0), n: 1, color: red(), d: 32.0, tail: None },
        Obj::Counter { c: Pt::new(275.0, 292.0), n: 2, color: red(), d: 32.0, tail: None },
        Obj::Callout { fmt: Default::default(),
            r: FRect { x: 724.0, y: 120.0, w: 320.0, h: 92.0 },
            tail: Pt::new(780.0, 250.0),
            text: "Move this to the top, above the summary".into(),
            color: red(),
            size: 18.0,
            style: CalloutStyle::Rounded,
            fill: true,
        },
    ]
}

/// An Edit window over a History entry of `img` with `objs`, saved to a
/// file in the temp folder.
fn window(tag: &str, img: PixBuf, objs: Vec<Obj>, th: Theme, k: f32) -> (TempRoot, Window) {
    let root = TempRoot::new(tag);
    let st = Store::new(&root.0);
    let t = (2026, 10, 5, 14, 32, 0);
    let (id, dir) = st.reserve(t).unwrap();
    let saved = root.0.join("2026-10-05_14-32.png");
    std::fs::create_dir_all(&root.0).unwrap();
    std::fs::write(&saved, img.to_png().unwrap()).unwrap();
    let meta = Meta { captured: t, title: "#design-review".into(), app: "Slack".into(), saved: Some(saved), ..Meta::default() };
    st.write_shot(&dir, &img, &objs, None, &meta).unwrap();
    let e = st.get(&id).unwrap();
    let theme = if th == DARK { "dark" } else { "light" };
    let cfg = Config { theme: theme.into(), ..Config::default() };
    let canvas = Canvas::new(img, objs, cfg.clone(), th);
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut w = Window::new(Edit::new(st, e, canvas, cfg), rx);
    w.scale = Some(k);
    w.event(Ev::Resize((W as f32 * k) as u32, (H as f32 * k) as u32));
    w.render();
    (root, w)
}

fn at(w: &mut Window, ev: Ev) {
    if w.event(ev) {
        w.render();
    }
}

fn click(w: &mut Window, p: Pt) {
    let (x, y) = (p.x as i32, p.y as i32);
    for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
        at(w, e);
    }
}

fn drag(w: &mut Window, a: Pt, b: Pt) {
    at(w, Ev::Move { x: a.x as i32, y: a.y as i32 });
    at(w, Ev::Down { x: a.x as i32, y: a.y as i32 });
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        at(w, Ev::Move { x: (a.x + (b.x - a.x) * t) as i32, y: (a.y + (b.y - a.y) * t) as i32 });
    }
    at(w, Ev::Up { x: b.x as i32, y: b.y as i32 });
}

fn key(w: &mut Window, vk: u32, mods: Mods) {
    at(w, Ev::Key { vk, up: false, repeat: false, mods });
    at(w, Ev::Key { vk, up: true, repeat: false, mods });
}

#[test]
fn fit_matches_the_board_and_zoom_keeps_the_point() {
    let (_r, mut w) = window("fit", page(1180, 760), objects(), DARK, 1.0);
    // 1180 x 760 in the board's canvas: 72 %.
    let z = (w.e.canvas.zoom() * 100.0).round();
    assert!((72.0..=73.0).contains(&z), "{z}");
    assert!(w.e.canvas.is_fit());
    // Ctrl+= zooms in a step; Ctrl+0 back to Fit; Ctrl+1 is 100 %.
    let ctrl = Mods { ctrl: true, ..Mods::default() };
    key(&mut w, 0xBB, ctrl);
    assert!((w.e.canvas.zoom() - 0.75).abs() < 1e-3 && !w.e.canvas.is_fit());
    key(&mut w, '1' as u32, ctrl);
    assert_eq!(w.e.canvas.zoom(), 1.0);
    key(&mut w, '0' as u32, ctrl);
    assert!(w.e.canvas.is_fit());
    // Zooming about a point keeps the image point under it.
    let p = Pt::new(600.0, 400.0);
    let before = w.e.canvas.view.to_img(p);
    w.e.canvas.zoom_by(2.0, p);
    let after = w.e.canvas.view.to_img(p);
    assert!((before.x - after.x).abs() < 1.0 && (before.y - after.y).abs() < 1.0, "{before:?} {after:?}");
    // The zoom control's Fit.
    let lay = draw::layout(&w.e, W as f32, H as f32, w.e.canvas.bar_width());
    assert!(!lay.zoom_in_status, "room for the zoom control in the strip at 1200 px");
    click(&mut w, Pt::new(lay.zoom.fit.x + 5.0, lay.zoom.fit.y + 5.0));
    assert!(w.e.canvas.is_fit());
}

#[test]
fn drawing_at_a_zoom_lands_in_image_coordinates() {
    let (_r, mut w) = window("draw", page(1180, 760), Vec::new(), DARK, 1.0);
    // The arrow tool from the docked toolbar.
    let arrow = w.e.canvas.button("arrow").expect("arrow button");
    click(&mut w, Pt::new(arrow.x + arrow.w / 2.0, arrow.y + arrow.h / 2.0));
    let v = w.e.canvas.view;
    let (a, b) = (v.to_win(Pt::new(200.0, 200.0)), v.to_win(Pt::new(500.0, 400.0)));
    drag(&mut w, a, b);
    let o = w.e.canvas.objects().to_vec();
    assert_eq!(o.len(), 1);
    let Obj::Arrow { a: oa, b: ob, .. } = &o[0] else { panic!("{o:?}") };
    assert!((oa.x - 200.0).abs() < 2.0 && (oa.y - 200.0).abs() < 2.0, "{oa:?}");
    assert!((ob.x - 500.0).abs() < 2.0 && (ob.y - 400.0).abs() < 2.0, "{ob:?}");
    assert!(w.e.canvas.unsaved());
    // Esc goes back to Select, never closes the window.
    key(&mut w, crate::wind::key::ESCAPE, Mods::default());
    key(&mut w, crate::wind::key::ESCAPE, Mods::default());
    assert!(!w.e.closing && w.e.flow.dialog.is_none());
    // Space-drag pans (at 200 %).
    w.e.canvas.zoom_by(2.8, Pt::new(600.0, 400.0));
    let v0 = w.e.canvas.view;
    at(&mut w, Ev::Key { vk: crate::wind::key::SPACE, up: false, repeat: false, mods: Mods::default() });
    drag(&mut w, Pt::new(600.0, 400.0), Pt::new(560.0, 380.0));
    at(&mut w, Ev::Key { vk: crate::wind::key::SPACE, up: true, repeat: false, mods: Mods::default() });
    assert_eq!(w.e.canvas.objects().len(), 1, "a pan draws nothing");
    assert!((w.e.canvas.view.ox - (v0.ox - 40.0)).abs() < 1.0, "{:?} {:?}", w.e.canvas.view, v0);
    // A middle drag pans too.
    let v1 = w.e.canvas.view;
    at(&mut w, Ev::Middle { down: true, x: 600, y: 400 });
    at(&mut w, Ev::Move { x: 620, y: 410 });
    at(&mut w, Ev::Middle { down: false, x: 620, y: 410 });
    assert!((w.e.canvas.view.oy - (v1.oy + 10.0)).abs() < 1.0);
}

#[test]
fn the_list_selects_and_reorders() {
    let (_r, mut w) = window("list", page(1180, 760), objects(), DARK, 1.0);
    let lay = draw::layout(&w.e, W as f32, H as f32, w.e.canvas.bar_width());
    // Top of the list is front-most: the callout.
    assert_eq!(lay.rows.iter().map(|r| r.1).collect::<Vec<_>>(), [3, 2, 1, 0]);
    assert_eq!(draw::obj_name(&w.e.canvas.objects()[3]), "Callout \"Move this to\u{2026}\"");
    assert_eq!(draw::obj_name(&w.e.canvas.objects()[1]), "Number 1");
    let r = lay.rows[3].0;
    click(&mut w, Pt::new(r.x + 50.0, r.y + 10.0));
    assert_eq!(w.e.canvas.selected(), [0]);
    // Drag the arrow (bottom row) to the top: it becomes front-most.
    let top = lay.rows[0].0;
    drag(&mut w, Pt::new(r.x + 50.0, r.y + 10.0), Pt::new(top.x + 50.0, top.y + 4.0));
    assert!(matches!(w.e.canvas.objects().last(), Some(Obj::Arrow { .. })));
    assert_eq!(w.e.canvas.selected(), [3]);
    // Del removes it.
    key(&mut w, crate::wind::key::DELETE, Mods::default());
    assert_eq!(w.e.canvas.objects().len(), 3);
    // Collapse to the rail.
    click(&mut w, Pt::new(lay.collapse.x + 5.0, lay.collapse.y + 5.0));
    assert!(w.e.collapsed);
}

#[test]
fn saving_writes_the_file_and_the_entry_never_the_original() {
    let (r, mut w) = window("save", page(400, 300), objects()[..1].to_vec(), DARK, 1.0);
    let orig = std::fs::read(w.e.entry.original()).unwrap();
    w.e.canvas.edit_mut_for_tests().push(Obj::Rect { r: FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 }, color: red(), width: 3.0 });
    assert!(w.e.canvas.unsaved());
    // Ctrl+S asks; Save as new file.
    key(&mut w, 'S' as u32, Mods { ctrl: true, ..Mods::default() });
    assert!(matches!(w.e.flow.dialog, Some(Dialog::Choose { .. })));
    let (over, new) = w.e.choice_files();
    assert_eq!((over.as_str(), new.as_str()), ("2026-10-05_14-32.png", "2026-10-05_14-32-edited.png"));
    let rem = w.e.remembered();
    for d in w.e.flow.press(Btn::Pick(Choice::New), rem).into_iter().chain(w.e.flow.press(Btn::Save, rem)) {
        w.e.perform(d);
    }
    let out = r.0.join("2026-10-05_14-32-edited.png");
    let saved = PixBuf::from_png(&std::fs::read(&out).expect("written")).unwrap();
    assert_eq!(saved.dimensions(), (400, 300));
    assert!(!w.e.canvas.unsaved());
    let e = w.e.store.get(&w.e.entry.id).unwrap();
    assert!(e.meta.edited);
    assert_eq!(e.meta.saved.as_deref(), Some(out.as_path()));
    assert_eq!(w.e.store.objects(&e).unwrap().len(), 2);
    assert_eq!(std::fs::read(e.original()).unwrap(), orig, "original.png is never written");
    // Closing with nothing unsaved closes at once.
    at(&mut w, Ev::Close);
    assert!(w.e.closing);
}

#[test]
fn closing_with_unsaved_changes_asks() {
    let (_r, mut w) = window("close", page(400, 300), Vec::new(), LIGHT, 1.0);
    w.e.canvas.edit_mut_for_tests().push(Obj::Rect { r: FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 }, color: red(), width: 3.0 });
    at(&mut w, Ev::Close);
    assert_eq!(w.e.flow.dialog, Some(Dialog::Unsaved));
    assert!(!w.e.closing);
    key(&mut w, crate::wind::key::ESCAPE, Mods::default());
    assert!(w.e.flow.dialog.is_none() && !w.e.closing, "Cancel");
    at(&mut w, Ev::Close);
    // Discard (the button).
    let (_, bs) = {
        let lay = draw::layout(&w.e, W as f32, H as f32, w.e.canvas.bar_width());
        let th = DARK;
        let inp = crate::ui::Input::default();
        let mut focus = crate::ui::FocusState::default();
        let mut clip = crate::ui::preview::FakeClip::default();
        let mut px = [0u8; 4];
        let ui = crate::ui::Ui::new(crate::uifb::Fb::new(&mut px, 1), &th, &inp, &mut focus, &mut clip, 1.0, FRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
        draw::dialog_lay(&ui, &w.e, &lay)
    };
    let d = bs.iter().find(|(_, b)| *b == Btn::Discard).unwrap().0;
    click(&mut w, Pt::new(d.x + 5.0, d.y + 5.0));
    assert!(w.e.closing);
}

/// The board's states, dark and light, at 100 % and 150 %.
#[test]
fn preview_edit_pngs() {
    for k in [1.0f32, 1.5] {
        for (tn, th) in [("dark", DARK), ("light", LIGHT)] {
            let name = |what: &str| if k == 1.0 { format!("edit-{what}-{tn}.png") } else { format!("edit-{what}-{tn}-{}.png", (k * 100.0) as u32) };
            // State 2: the callout selected, its mini-toolbar, Delete's tip.
            let (_r, mut w) = window(&format!("pv-{tn}-{k}"), page(1180, 760), objects(), th, k);
            w.e.canvas.select(3, false);
            let unsaved = Obj::Rect { r: FRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, color: C4::new(0, 0, 0, 0), width: 1.0 };
            let _ = unsaved;
            w.render();
            crate::ui::preview::save(&name("callout"), w.pixels());
            // State 1: nothing selected, the list collapsed.
            let (_r2, mut w) = window(&format!("pv1-{tn}-{k}"), page(1180, 760), objects(), th, k);
            w.e.collapsed = true;
            w.event(Ev::Resize((W as f32 * k) as u32, (H as f32 * k) as u32));
            w.render();
            crate::ui::preview::save(&name("collapsed"), w.pixels());
            // The arrow selected at 100 %.
            w.e.collapsed = false;
            w.e.canvas.zoom_actual();
            w.e.canvas.select(0, false);
            w.render();
            crate::ui::preview::save(&name("arrow-100"), w.pixels());
            // Several selected (Ctrl+A).
            w.e.canvas.zoom_fit();
            key(&mut w, 'A' as u32, Mods { ctrl: true, ..Mods::default() });
            crate::ui::preview::save(&name("multi"), w.pixels());
            // The save questions.
            w.e.flow.dialog = Some(Dialog::Unsaved);
            w.render();
            crate::ui::preview::save(&name("unsaved"), w.pixels());
            w.e.flow.dialog = Some(Dialog::Choose { choice: Choice::Overwrite, dont_ask: false, close: false });
            w.render();
            crate::ui::preview::save(&name("choose"), w.pixels());
            w.e.flow.dialog = None;
            // Typing in the callout.
            let v = w.e.canvas.view;
            key(&mut w, crate::wind::key::ESCAPE, Mods::default());
            key(&mut w, 'K' as u32, Mods::default());
            let p = v.to_win(Pt::new(800.0, 150.0));
            click(&mut w, p);
            for c in " so people see it".encode_utf16() {
                at(&mut w, Ev::Char(c));
            }
            crate::ui::preview::save(&name("typing"), w.pixels());
        }
    }
}

/// The frame time on a 4K capture with objects: dragging an object at
/// Fit and at 100 % (`cargo test --release edit_frame_time -- --ignored
/// --nocapture` for the numbers; the debug build is far slower).
#[test]
#[ignore = "timing; run in release with --nocapture"]
fn edit_frame_time() {
    for (label, k) in [("100% UI", 1.0f32), ("150% UI", 1.5)] {
        let img = page(3840, 2160);
        let objs: Vec<Obj> = objects()
            .into_iter()
            .map(|mut o| {
                crate::objects::edit::map_box(&mut o, FRect { x: 0.0, y: 0.0, w: 1180.0, h: 760.0 }, FRect { x: 0.0, y: 0.0, w: 3840.0, h: 2473.0 });
                o
            })
            .collect();
        let (_r, mut w) = window(&format!("perf-{k}"), img, objs, DARK, k);
        for (zoom, fit) in [("Fit", true), ("1:1", false)] {
            if fit {
                w.e.canvas.zoom_fit();
            } else {
                w.e.canvas.zoom_actual();
            }
            w.render();
            // Drag the callout about (on screen at 100 % too).
            let c = w.e.canvas.objects()[3].clone();
            let Obj::Callout { r, .. } = c else { unreachable!() };
            let at_c = w.e.canvas.view.to_win(Pt::new(r.x + r.w / 2.0, r.y + r.h / 2.0));
            w.e.canvas.pan_by(W as f32 * k * 0.6 - at_c.x, H as f32 * k * 0.5 - at_c.y);
            w.render();
            let v = w.e.canvas.view;
            let start = v.to_win(Pt::new(r.x + 30.0, r.y + 30.0));
            w.e.canvas.select(3, false);
            at(&mut w, Ev::Move { x: start.x as i32, y: start.y as i32 });
            at(&mut w, Ev::Down { x: start.x as i32, y: start.y as i32 });
            let mut times = Vec::new();
            #[cfg(windows)]
            {
                crate::perf::force_on();
                crate::perf::take();
            }
            for i in 0..60 {
                let x = start.x + (i as f32 * 3.0) % 120.0;
                let y = start.y + (i as f32 * 2.0) % 80.0;
                let t0 = std::time::Instant::now();
                w.event(Ev::Move { x: x as i32, y: y as i32 });
                w.render();
                times.push(t0.elapsed().as_secs_f64() * 1000.0);
            }
            #[cfg(windows)]
            {
                let t = crate::perf::take();
                let parts: Vec<String> = crate::perf::NAMES.iter().zip(t).filter(|(_, (_, n))| *n > 0).map(|(k, (ms, n))| format!("{k} {:.2}/{n}", ms / 60.0)).collect();
                println!("  per frame: {}", parts.join(", "));
            }
            at(&mut w, Ev::Up { x: start.x as i32, y: start.y as i32 });
            // Drawing a new arrow (the draft).
            key(&mut w, 'A' as u32, Mods::default());
            let a = Pt::new(w.e.canvas.view.ox + 100.0, w.e.canvas.view.oy + 100.0);
            at(&mut w, Ev::Move { x: a.x as i32, y: a.y as i32 });
            at(&mut w, Ev::Down { x: a.x as i32, y: a.y as i32 });
            let mut draft = Vec::new();
            for i in 0..60 {
                let t0 = std::time::Instant::now();
                w.event(Ev::Move { x: (a.x + 5.0 * i as f32) as i32, y: (a.y + 3.0 * i as f32) as i32 });
                w.render();
                draft.push(t0.elapsed().as_secs_f64() * 1000.0);
            }
            at(&mut w, Ev::Up { x: a.x as i32 + 300, y: a.y as i32 + 180 });
            key(&mut w, crate::wind::key::ESCAPE, Mods::default());
            let stat = |v: &mut Vec<f64>| {
                v.sort_by(|a, b| a.total_cmp(b));
                (v.iter().sum::<f64>() / v.len() as f64, v[v.len() / 2], v[v.len() * 95 / 100])
            };
            let (m1, p50, p95) = stat(&mut times);
            let (m2, d50, d95) = stat(&mut draft);
            println!("4K {label} {zoom}: move object mean {m1:.2} ms p50 {p50:.2} p95 {p95:.2}; draw arrow mean {m2:.2} ms p50 {d50:.2} p95 {d95:.2}");
        }
    }
}

/// Smoke (needs a display): the real Edit window opens over an entry,
/// draws and closes after about a second.
/// `cargo test edit_window_smoke -- --ignored --test-threads=1`
#[cfg(not(target_os = "macos"))]
#[test]
#[ignore = "opens a window"]
fn edit_window_smoke() {
    if !crate::wind::has_display() {
        eprintln!("no display; skipped");
        return;
    }
    let _guard = crate::wind::test_window_lock();
    // RUSTSHOT_EDIT_SMOKE_4K=1: a 4K capture (with RUSTSHOT_PERFLOG, the
    // paint lines say what presenting a frame costs).
    let big = std::env::var_os("RUSTSHOT_EDIT_SMOKE_4K").is_some();
    let img = if big { page(3840, 2160) } else { page(1180, 760) };
    let (_r, mut w) = window("smoke", img, objects(), DARK, 1.0);
    w.scale = None;
    let mut d = crate::wind::AutoClose::new(&mut w, if big { 3000 } else { 1000 });
    let spec = crate::wind::WindowSpec { title: "Rustshot Edit".into(), w: W, h: H, resizable: true, min: MIN };
    crate::wind::run_window(spec, &mut d).expect("edit window");
    assert!(d.frames > 0, "drew a frame");
}
