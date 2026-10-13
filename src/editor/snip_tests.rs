//! Text mode, Quick Redact, the freeform lasso and the crop frame through
//! the editor (a fake recognizer and clipboard), in the overlay and the
//! Edit window; their frames tile exactly and their changes repaint.

use super::tests::{assert_dirty_covers, assert_tiles_match, edit_of, preview_app, settled, settled_frame};
use super::*;
use crate::ocr::{Line, Word};

fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };

/// The selection the fake recognizer reads.
const SEL: FRect = FRect { x: 100.0, y: 80.0, w: 600.0, h: 300.0 };

fn app() -> App {
    let mut a = preview_app(theme::DARK, Some(SEL));
    a.focus_tries = u8::MAX;
    a
}

fn key(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

fn down(a: &mut App, p: Pt) {
    a.on_event(Ev::Down { x: p.x as i32, y: p.y as i32 });
}

fn drag(a: &mut App, from: Pt, to: Pt) {
    down(a, from);
    let mid = Pt::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    a.on_event(Ev::Move { x: mid.x as i32, y: mid.y as i32 });
    a.on_event(Ev::Move { x: to.x as i32, y: to.y as i32 });
    a.on_event(Ev::Up { x: to.x as i32, y: to.y as i32 });
}

fn click(a: &mut App, p: Pt) {
    down(a, p);
    a.on_event(Ev::Up { x: p.x as i32, y: p.y as i32 });
}

/// "Hello Rustshot world" and "mail jane@example.com 4111 1111 1111 1111"
/// in the crop's pixels (20 px from its top-left, words 70 px apart).
fn doc(_: &PixBuf) -> Result<Vec<Line>, crate::ocr::OcrError> {
    let l = |y: f32, ws: &[&str]| Line { words: ws.iter().enumerate().map(|(i, t)| Word { text: (*t).into(), x: 20.0 + i as f32 * 70.0, y, w: 60.0, h: 18.0 }).collect() };
    Ok(vec![l(60.0, &["mail", "jane@example.com", "4111", "1111", "1111", "1111"]), l(20.0, &["Hello", "Rustshot", "world"])])
}

/// The middle of word `w` of reading-order line `l` (capture px).
fn word(l: usize, w: usize) -> Pt {
    let y = if l == 0 { 20.0 } else { 60.0 };
    Pt::new(SEL.x + 20.0 + w as f32 * 70.0 + 30.0, SEL.y + y + 9.0)
}

/// Tick until the recognition delivered.
fn settle_ocr(a: &mut App) {
    let t0 = Instant::now();
    while edit_of(a).snip.ocr.is_some() {
        assert!(t0.elapsed() < Duration::from_secs(10), "recognition never finished");
        std::thread::sleep(Duration::from_millis(2));
        a.on_event(Ev::Timer);
    }
}

fn copied() -> Vec<String> {
    crate::ocr::COPIED.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

fn with_doc(a: &mut App) {
    crate::ocr::FAKE_LINES.with(|f| f.set(Some(doc)));
    let _ = copied();
    // Text mode up, its words in the capture's pixels.
    key(a, 'O' as u32, NONE);
    assert!(matches!(a.st, State::Edit(_)), "Let me select: the overlay stays");
    settle_ocr(a);
    assert!(edit_of(a).snip.text.is_some(), "Text mode");
}

fn notice(a: &mut App) -> String {
    edit_of(a).notice.as_ref().map(|t| t.text.clone()).unwrap_or_default()
}

#[test]
fn text_mode_selects_words_in_reading_order_and_copies_them() {
    let mut a = app();
    with_doc(&mut a);
    assert_eq!(notice(&mut a), "Select text to copy \u{b7} 2 lines");
    // The words are where the crop has them, the I-beam over them.
    let tm = edit_of(&mut a).snip.text.as_ref().unwrap();
    assert_eq!(tm.lines[0].words[0], Word { text: "Hello".into(), x: 120.0, y: 100.0, w: 60.0, h: 18.0 });
    a.mouse = (word(0, 1).x as i32, word(0, 1).y as i32);
    assert_eq!(a.cursor(), Cursor::IBeam);
    // A drag from "Rustshot" to "jane@…": across the line break.
    drag(&mut a, word(0, 1), word(1, 1));
    key(&mut a, 'C' as u32, CTRL);
    assert!(matches!(a.st, State::Edit(_)), "Ctrl+C copies the words, not the picture");
    assert_eq!(copied(), ["Rustshot world\nmail jane@example.com"]);
    assert_eq!(notice(&mut a), "Copied text \u{b7} 2 lines");
    // Shift+click extends; a double-click takes a word, a triple its line.
    click(&mut a, word(0, 0));
    FAKE_MODS.with(|m| m.set(Some(SHIFT)));
    click(&mut a, word(0, 2));
    FAKE_MODS.with(|m| m.set(Some(NONE)));
    key(&mut a, 'C' as u32, CTRL);
    assert_eq!(copied(), ["Hello Rustshot world"]);
    std::thread::sleep(Duration::from_millis(500)); // not a third click
    let w = word(1, 1);
    click(&mut a, w);
    click(&mut a, w);
    key(&mut a, 'C' as u32, CTRL);
    assert_eq!(copied(), ["jane@example.com"]);
    click(&mut a, w);
    key(&mut a, 'C' as u32, CTRL);
    assert_eq!(copied(), ["mail jane@example.com 4111 1111 1111 1111"]);
    // Ctrl+A: everything; Enter copies it too.
    key(&mut a, 'A' as u32, CTRL);
    key(&mut a, wind::key::RETURN, NONE);
    assert_eq!(copied(), ["Hello Rustshot world\nmail jane@example.com 4111 1111 1111 1111"]);
    // A press in the gap between words, inside the area, keeps the words
    // but clears the selection; Esc leaves Text mode (not the overlay).
    click(&mut a, Pt::new(SEL.x + 300.0, SEL.y + 250.0));
    assert!(edit_of(&mut a).snip.text.as_ref().unwrap().sel.is_none());
    // Ctrl+C with nothing selected copies every word.
    key(&mut a, 'C' as u32, CTRL);
    assert_eq!(copied(), ["Hello Rustshot world\nmail jane@example.com 4111 1111 1111 1111"]);
    assert_eq!(notice(&mut a), "Copied all text");
    key(&mut a, wind::key::ESCAPE, NONE);
    assert!(edit_of(&mut a).snip.text.is_none() && matches!(a.st, State::Edit(_)));
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}

#[test]
fn text_mode_bar_copies_all_and_the_pill_the_selection() {
    let mut a = app();
    with_doc(&mut a);
    settled_frame(&mut a);
    let bar = edit_of(&mut a).scene.as_ref().unwrap().snip.as_ref().unwrap().bar.clone().expect("the bar");
    assert!(bar.pill.is_none(), "no pill without a selection");
    let all = bar.btns.iter().find(|b| b.act == Act::CopyAll).unwrap().r;
    assert!(all.y1() <= SEL.y1() && all.y >= SEL.y, "inside the area, along its bottom: {all:?}");
    click(&mut a, Pt::new(all.x + 5.0, all.y + 5.0));
    assert_eq!(copied(), ["Hello Rustshot world\nmail jane@example.com 4111 1111 1111 1111"]);
    drag(&mut a, word(0, 0), word(0, 1));
    settled_frame(&mut a);
    let bar = edit_of(&mut a).scene.as_ref().unwrap().snip.as_ref().unwrap().bar.clone().unwrap();
    let pill = bar.pill.expect("the Copy pill by the selection").r;
    click(&mut a, Pt::new(pill.x + 5.0, pill.y + 5.0));
    assert_eq!(copied(), ["Hello Rustshot"]);
    // The Copy text button again leaves Text mode.
    key(&mut a, 'O' as u32, NONE);
    assert!(edit_of(&mut a).snip.text.is_none());
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}

#[test]
fn copy_text_all_at_once_still_closes_the_overlay() {
    let mut a = app();
    a.kind = RunKind::Daemon;
    edit_of(&mut a).cfg.copy_text_mode = "all".into();
    key(&mut a, 'O' as u32, NONE);
    assert!(matches!(a.st, State::Hidden), "copied all at once (the old Copy text)");
}

#[test]
fn quick_redact_blurs_the_sensitive_words_in_one_step() {
    let mut a = app();
    crate::ocr::FAKE_LINES.with(|f| f.set(Some(doc)));
    let hist = edit_of(&mut a).hist.len();
    key(&mut a, 'R' as u32, SHIFT);
    // (A fast recognizer may have delivered already.)
    if edit_of(&mut a).snip.ocr.is_some() {
        assert_eq!(notice(&mut a), "Looking for sensitive text\u{2026}");
    }
    settle_ocr(&mut a);
    let e = edit_of(&mut a);
    // Opaque fills in the color around them (old builds draw them too).
    let fills: Vec<FRect> = e
        .objects
        .iter()
        .map(|o| match o {
            Obj::RectFill { r, color, .. } => {
                assert_eq!(color.a, 255, "opaque");
                assert_eq!(Some(*color), snip::border_median(&e.export(*r)), "the surroundings' median");
                *r
            }
            o => panic!("not a fill: {o:?}"),
        })
        .collect();
    // The e-mail and the card (its four words), padded max(3 px, 15 %).
    assert_eq!(fills, vec![r(187.0, 137.0, 66.0, 24.0), r(257.0, 137.0, 276.0, 24.0)]);
    assert_eq!(e.hist.len(), hist + 1, "one undo step");
    assert_eq!(notice(&mut a), "Redacted 2 items");
    key(&mut a, 'Z' as u32, CTRL);
    assert!(edit_of(&mut a).objects.is_empty(), "one undo takes them all");
    // Without e-mails (the Copy text menu's checkbox, remembered).
    let e = edit_of(&mut a);
    e.apply_act(Act::RedactKind(crate::ocr::redact::Kind::Email), Pt::new(0.0, 0.0));
    assert_eq!(e.cfg.redact_kinds, "phones,cards,ids,urls,ips");
    assert_eq!(e.pending.get("redact_kinds"), Some(&Some("\"phones,cards,ids,urls,ips\"".to_string())));
    assert!(e.snip.copy_menu, "a checkbox keeps the menu open");
    key(&mut a, 'R' as u32, SHIFT);
    settle_ocr(&mut a);
    assert_eq!(edit_of(&mut a).objects.len(), 1);
    assert_eq!(notice(&mut a), "Redacted 1 item");
    // The Blur style: a mosaic of cells twice the text height, then a
    // blur of one cell, over each find.
    key(&mut a, 'Z' as u32, CTRL);
    edit_of(&mut a).cfg.redact_style = "blur".into();
    key(&mut a, 'R' as u32, SHIFT);
    settle_ocr(&mut a);
    let q = r(257.0, 137.0, 276.0, 24.0);
    assert_eq!(edit_of(&mut a).objects, vec![Obj::Pixelate { r: q, cell: 48.0 }, Obj::Blur { r: q, sigma: 48.0 }]);
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}

#[test]
fn redact_selected_words_in_text_mode() {
    let mut a = app();
    with_doc(&mut a);
    drag(&mut a, word(0, 1), word(0, 2));
    key(&mut a, 'R' as u32, SHIFT);
    let e = edit_of(&mut a);
    assert!(matches!(e.objects[..], [Obj::RectFill { r: q, .. }] if q == r(187.0, 97.0, 136.0, 24.0)), "{:?}", e.objects);
    assert!(e.snip.text.as_ref().is_some_and(|t| t.sel.is_none()), "Text mode stays, the selection is used up");
    // Nothing sensitive: said so, nothing added.
    crate::ocr::FAKE_LINES.with(|f| f.set(Some(|_| Ok(vec![Line { words: vec![Word { text: "hello".into(), x: 0.0, y: 0.0, w: 9.0, h: 9.0 }] }]))));
    key(&mut a, wind::key::ESCAPE, NONE);
    key(&mut a, 'R' as u32, SHIFT);
    settle_ocr(&mut a);
    assert_eq!(edit_of(&mut a).objects.len(), 1);
    assert_eq!(notice(&mut a), "Nothing sensitive found");
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}

/// An export asked for while Redact runs waits for it: the capture goes
/// out with its fills, never without.
#[test]
fn an_export_waits_for_redact() {
    use crate::history::{Store, tests::TempRoot};
    let _ = (crate::pin::take_opened(), crate::notice::take_shown());
    let root = TempRoot::new("redact-wait");
    let mut a = app();
    a.kind = RunKind::Daemon;
    a.history_root = Some(root.0.clone());
    crate::ocr::FAKE_LINES.with(|f| f.set(Some(doc)));
    key(&mut a, 'R' as u32, SHIFT);
    key(&mut a, 'P' as u32, CTRL);
    if matches!(a.st, State::Edit(_)) {
        assert_eq!(notice(&mut a), "Redacting\u{2026} it goes on when done");
        assert!(edit_of(&mut a).snip.queued.is_some());
    }
    let t0 = Instant::now();
    while matches!(a.st, State::Edit(_)) {
        assert!(t0.elapsed() < Duration::from_secs(10), "the pin never went out");
        std::thread::sleep(Duration::from_millis(2));
        a.on_event(Ev::Timer);
    }
    let id = a.last_entry.clone().expect("kept");
    crate::history::flush(Duration::from_secs(20));
    let st = Store::new(&root.0);
    let objs = st.objects(&st.get(&id).unwrap()).unwrap();
    assert_eq!(objs.iter().filter(|o| matches!(o, Obj::RectFill { .. })).count(), 2, "pinned with the fills: {objs:?}");
    // Enter waits too (a run that never delivers here).
    let mut a = app();
    let (_tx, rx) = crate::chan::channel();
    let e = edit_of(&mut a);
    e.snip.ocr = Some(snip::OcrRun { rx, want: snip::Want::Redact, origin: Pt::new(0.0, 0.0), area: SEL, sel: e.sel, mask: None });
    key(&mut a, wind::key::RETURN, NONE);
    assert!(matches!(a.st, State::Edit(_)), "not before the fills");
    assert_eq!(edit_of(&mut a).snip.queued, Some(snip::Queued::Enter));
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
    let _ = crate::pin::take_opened();
}

/// The selection moved while Redact read it: the old words are dropped
/// and the new area is read.
#[test]
fn redact_reads_again_when_the_selection_moved() {
    let mut a = app();
    // Slow enough that the nudge comes first.
    crate::ocr::FAKE_LINES.with(|f| {
        f.set(Some(|img| {
            std::thread::sleep(Duration::from_millis(300));
            doc(img)
        }))
    });
    key(&mut a, 'R' as u32, SHIFT);
    // Nudged right by one pixel before the words came.
    key(&mut a, wind::key::RIGHT, NONE);
    settle_ocr(&mut a);
    let e = edit_of(&mut a);
    let first = e.objects.iter().find_map(|o| if let Obj::RectFill { r, .. } = o { Some(*r) } else { None }).expect("redacted");
    assert_eq!(first.x, 188.0, "at the new place (one pixel right)");
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}

/// Text mode and Redact see only what the lasso keeps: the picture read
/// is cut by it, and words outside it are dropped.
#[test]
fn ocr_reads_only_inside_the_lasso() {
    let mut a = app();
    key(&mut a, 'F' as u32, SHIFT);
    // A lasso over the first line's first two words only (crop px 20..160
    // across, 20..38 down; the selection starts at its box).
    draw_lasso(&mut a, &[Pt::new(110.0, 90.0), Pt::new(270.0, 90.0), Pt::new(270.0, 125.0), Pt::new(110.0, 125.0)]);
    crate::ocr::FAKE_LINES.with(|f| {
        f.set(Some(|img| {
            // The corner outside the lasso is cut (transparent white).
            let w = img.width() as usize;
            assert_eq!(img.as_raw()[(17 * w + 80) * 4 + 3], 255, "inside");
            let l = |y: f32, ws: &[&str]| Line { words: ws.iter().enumerate().map(|(i, t)| Word { text: (*t).into(), x: 10.0 + i as f32 * 70.0, y, w: 60.0, h: 18.0 }).collect() };
            Ok(vec![l(5.0, &["Hello", "Rustshot", "world"])])
        }))
    });
    key(&mut a, 'O' as u32, NONE);
    settle_ocr(&mut a);
    let tm = edit_of(&mut a).snip.text.as_ref().expect("Text mode");
    let words: Vec<&str> = tm.lines.iter().flat_map(|l| l.words.iter().map(|w| w.text.as_str())).collect();
    assert_eq!(words, ["Hello", "Rustshot"], "the word outside the lasso is dropped");
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}

/// A triangle lasso (capture px).
fn triangle() -> [Pt; 3] {
    [Pt::new(300.0, 200.0), Pt::new(700.0, 200.0), Pt::new(500.0, 600.0)]
}

fn draw_lasso(a: &mut App, pts: &[Pt]) {
    down(a, pts[0]);
    for w in pts.windows(2).chain(std::iter::once(&[pts[pts.len() - 1], pts[0]][..])) {
        for i in 1..=20 {
            let t = i as f32 / 20.0;
            let p = Pt::new(w[0].x + (w[1].x - w[0].x) * t, w[0].y + (w[1].y - w[0].y) * t);
            a.on_event(Ev::Move { x: p.x as i32, y: p.y as i32 });
        }
    }
    a.on_event(Ev::Up { x: pts[0].x as i32, y: pts[0].y as i32 });
}

#[test]
fn freeform_draws_a_lasso_whose_outside_is_transparent() {
    let mut a = app();
    key(&mut a, 'F' as u32, SHIFT);
    let e = edit_of(&mut a);
    assert!(e.snip.freeform && e.sel.is_none(), "the selection goes: the next is a lasso");
    assert_eq!(e.pending.get("selection_mode"), Some(&Some("\"freeform\"".to_string())));
    draw_lasso(&mut a, &triangle());
    let e = edit_of(&mut a);
    let sel = e.sel.expect("its box is the selection");
    assert_eq!(sel, r(300.0, 200.0, 400.0, 400.0));
    assert!(e.live_mask().is_some());
    let img = e.masked(e.export(sel), sel);
    assert_eq!(img.dimensions(), (400, 400));
    assert_eq!(img.get_pixel(200, 100)[3], 255, "inside");
    assert_eq!(img.get_pixel(5, 395), [255, 255, 255, 0], "outside: transparent");
    assert_eq!(img.get_pixel(395, 395)[3], 0);
    // Moving the selection takes the lasso along.
    drag(&mut a, Pt::new(500.0, 300.0), Pt::new(520.0, 310.0));
    let e = edit_of(&mut a);
    assert_eq!(e.sel, Some(r(320.0, 210.0, 400.0, 400.0)));
    assert_eq!(e.live_mask().unwrap()[0], Pt::new(320.0, 210.0));
    // Trim (and its undo) keeps the lasso.
    let e = edit_of(&mut a);
    let was = e.sel;
    e.sel = Some(r(330.0, 220.0, 300.0, 300.0));
    e.rebind_mask(was);
    assert!(e.live_mask().is_some(), "still cut by the lasso");
    // A new rectangle (freeform off) drops the lasso.
    key(&mut a, 'F' as u32, SHIFT);
    drag(&mut a, Pt::new(50.0, 50.0), Pt::new(90.0, 90.0));
    let e = edit_of(&mut a);
    assert!(!e.snip.freeform && e.live_mask().is_none());
}

/// A magnifier inside the lasso whose source lies outside it magnifies
/// nothing of the outside: the export blanks it before the objects bake
/// (the overlay, Text mode's picture, the Edit window).
#[test]
fn a_magnifier_never_shows_what_the_lasso_leaves_out() {
    let mut a = app();
    key(&mut a, 'F' as u32, SHIFT);
    draw_lasso(&mut a, &triangle());
    let e = edit_of(&mut a);
    let sel = e.sel.unwrap();
    // Its source in the box's bottom-left corner (outside the triangle),
    // its loupe in the middle (inside).
    e.commit_object(Obj::Magnify { src: Pt::new(330.0, 570.0), dst: Pt::new(500.0, 300.0), radius: 40.0, zoom: 2.0, color: C4::rgb(0, 0, 255), width: 2.0 });
    let blank = |p: [u8; 4]| p[3] == 0 || p[..3] == [255, 255, 255];
    let img = e.cut_export(sel);
    let c = img.get_pixel(200, 100);
    assert!(blank(c), "the loupe's middle shows the outside: {c:?}");
    // Without the lasso the same loupe shows the capture there.
    let plain = e.export(sel);
    assert!(!blank(plain.get_pixel(200, 100)));
    // The Edit window: the same picture opened with its lasso.
    let tri: Vec<Pt> = triangle().iter().map(|p| Pt::new(p.x - sel.x, p.y - sel.y)).collect();
    let (orig, objs) = e.history_shot(sel);
    let mut c = host::Canvas::new(orig, objs, Config::default(), theme::DARK);
    c.set_doc(None, Some(tri));
    assert!(blank(c.export().get_pixel(200, 100)));
}

#[test]
fn a_freeform_capture_keeps_its_lasso_in_the_history() {
    use crate::history::{Store, tests::TempRoot};
    let _ = (crate::pin::take_opened(), crate::notice::take_shown());
    let root = TempRoot::new("lasso");
    let mut a = app();
    a.kind = RunKind::Daemon;
    a.history_root = Some(root.0.clone());
    key(&mut a, 'F' as u32, SHIFT);
    draw_lasso(&mut a, &triangle());
    key(&mut a, 'P' as u32, CTRL);
    let id = a.last_entry.clone().expect("kept");
    crate::history::flush(Duration::from_secs(20));
    let st = Store::new(&root.0);
    let e = st.get(&id).unwrap();
    let mask = e.meta.mask.clone().expect("the lasso");
    assert_eq!(mask[0], Pt::new(0.0, 0.0), "in original.png's pixels");
    // original.png itself is cut: an older build (v0.2, v0.3 ignore the
    // mask) draws it with its objects, and nothing outside shows there.
    let orig = st.original(&e).unwrap();
    assert_eq!(orig.get_pixel(5, 395), [255, 255, 255, 0]);
    assert_eq!(orig.get_pixel(200, 100)[3], 255);
    let old_doc = crate::objects::v030::json::from_json(&std::fs::read_to_string(e.annotations()).unwrap()).unwrap();
    let old_pic = crate::editor::bake_objects(&orig, &old_doc, None);
    assert_eq!(old_pic.get_pixel(5, 395)[3], 0, "v0.3's picture: transparent outside");
    assert!(crate::objects::v020::json::from_json(&std::fs::read_to_string(e.annotations()).unwrap()).is_ok());
    // The picture as it opens again: transparent outside.
    let pic = crate::history_ui::picture(&st, &e).unwrap();
    assert_eq!(pic.get_pixel(5, 395)[3], 0);
    assert_eq!(pic.get_pixel(200, 100)[3], 255);
    // A BMP (or JPG) of it is white there: the bottom-left pixel.
    let bmp = crate::export::encode(&pic, crate::export::Format::Bmp, 90).unwrap();
    let i = 66 + (399 * 400) * 4;
    assert_eq!(&bmp[i..i + 4], &[255, 255, 255, 255]);
    let _ = crate::pin::take_opened();
}

#[test]
fn the_crop_frame_changes_the_overlay_selection() {
    let mut a = app();
    key(&mut a, 'C' as u32, NONE);
    let e = edit_of(&mut a);
    assert!(e.snip.crop_mode.as_ref().is_some_and(|c| c.r == SEL && c.bounds == SEL));
    // Its bottom-right handle in, then Enter.
    drag(&mut a, Pt::new(SEL.x1(), SEL.y1()), Pt::new(500.0, 300.0));
    a.mouse = (300, 200);
    assert_eq!(a.cursor(), Cursor::Move, "inside the frame");
    key(&mut a, wind::key::RETURN, NONE);
    let e = edit_of(&mut a);
    assert!(e.snip.crop_mode.is_none());
    assert_eq!(e.sel, Some(r(100.0, 80.0, 400.0, 220.0)), "the selection is the crop");
    assert!(matches!(a.st, State::Edit(_)), "Enter crops (no export)");
    // Esc cancels a frame.
    key(&mut a, 'C' as u32, NONE);
    drag(&mut a, Pt::new(100.0, 80.0), Pt::new(200.0, 150.0));
    key(&mut a, wind::key::ESCAPE, NONE);
    assert_eq!(edit_of(&mut a).sel, Some(r(100.0, 80.0, 400.0, 220.0)));
    assert!(matches!(a.st, State::Edit(_)));
}

#[test]
fn snip_frames_tile_exactly_and_repaint_what_changed() {
    // Text mode with a selection.
    let mut a = app();
    with_doc(&mut a);
    drag(&mut a, word(0, 1), word(1, 1));
    let want = settled(&mut a);
    assert_tiles_match(&mut a, &want, "text mode", 11);
    assert_dirty_covers(&mut a, "text selection", |a| {
        let tm = edit_of(a).snip.text.as_mut().unwrap();
        tm.sel = Some(((0, 0), (0, 0)));
    });
    assert_dirty_covers(&mut a, "leave text mode", |a| edit_of(a).leave_text_mode());
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
    // A lasso.
    let mut a = app();
    key(&mut a, 'F' as u32, SHIFT);
    draw_lasso(&mut a, &triangle());
    let want = settled(&mut a);
    assert_tiles_match(&mut a, &want, "lasso", 12);
    // The crop frame.
    let mut a = app();
    key(&mut a, 'C' as u32, NONE);
    drag(&mut a, Pt::new(SEL.x1(), SEL.y1()), Pt::new(500.0, 300.0));
    let want = settled(&mut a);
    assert_tiles_match(&mut a, &want, "crop", 13);
    assert_dirty_covers(&mut a, "crop drag", |a| {
        let c = edit_of(a).snip.crop_mode.as_mut().unwrap();
        c.r.w -= 37.0;
    });
}

// ---- the Edit window ----

fn canvas() -> host::Canvas {
    let img = PixBuf::from_pixel(400, 300, [200, 210, 220, 255]);
    let mut c = host::Canvas::new(img, Vec::new(), Config::default(), theme::DARK);
    let area = r(0.0, 40.0, 1200.0, 720.0);
    c.set_rects(area, r(0.0, 0.0, 1200.0, 40.0), r(0.0, 0.0, 1200.0, 800.0), 1.0);
    c
}

fn win(c: &host::Canvas, p: Pt) -> Pt {
    c.view.to_win(p)
}

fn ckey(c: &mut host::Canvas, vk: u32, mods: Mods) {
    c.key(vk, false, false, mods);
}

fn cdrag(c: &mut host::Canvas, from: Pt, to: Pt) {
    let (a, b) = (win(c, from), win(c, to));
    c.press(a);
    c.moved(Pt::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0), true);
    c.moved(b, true);
    c.release(b);
}

#[test]
fn the_edit_window_crops_without_losing_anything() {
    let mut c = canvas();
    ckey(&mut c, 'C' as u32, NONE);
    cdrag(&mut c, Pt::new(0.0, 0.0), Pt::new(100.0, 50.0));
    ckey(&mut c, wind::key::RETURN, NONE);
    assert_eq!(c.crop(), Some(r(100.0, 50.0, 300.0, 250.0)));
    assert!(c.unsaved(), "a crop is a change");
    assert_eq!(c.export().dimensions(), (300, 250), "the export is cut");
    assert_eq!(c.size(), (400, 300), "the picture is whole");
    // Undo puts it back, redo crops again.
    ckey(&mut c, 'Z' as u32, CTRL);
    assert_eq!(c.crop(), None);
    assert_eq!(c.export().dimensions(), (400, 300));
    ckey(&mut c, 'Y' as u32, CTRL);
    assert_eq!(c.crop(), Some(r(100.0, 50.0, 300.0, 250.0)));
    // Opened again at the crop: the frame starts there, and can grow.
    ckey(&mut c, 'C' as u32, NONE);
    cdrag(&mut c, Pt::new(100.0, 50.0), Pt::new(0.0, 0.0));
    ckey(&mut c, wind::key::RETURN, NONE);
    assert_eq!(c.crop(), None, "the whole picture again: no crop");
    // Saved as a document with the crop; opened again it is there.
    c.set_doc(Some(r(10.0, 10.0, 50.0, 40.0)), None);
    assert!(!c.unsaved());
    assert_eq!(c.export().dimensions(), (50, 40));
}

#[test]
fn the_edit_window_has_text_mode_and_redact() {
    crate::ocr::FAKE_LINES.with(|f| f.set(Some(doc)));
    let _ = copied();
    let mut c = canvas();
    ckey(&mut c, 'O' as u32, NONE);
    let t0 = Instant::now();
    while !c.poll() {
        assert!(t0.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(c.take_notice().map(|n| n.0).as_deref(), Some("Select text to copy \u{b7} 2 lines"));
    // Words at the picture's own pixels (the area is the whole picture).
    let w = Pt::new(20.0 + 70.0 + 30.0, 20.0 + 9.0);
    c.press(win(&c, w));
    c.release(win(&c, w));
    c.press(win(&c, w));
    c.release(win(&c, w));
    ckey(&mut c, 'C' as u32, CTRL);
    assert_eq!(copied(), ["Rustshot"]);
    ckey(&mut c, 'R' as u32, SHIFT);
    assert_eq!(c.objects().len(), 1, "Redact selected words");
    ckey(&mut c, wind::key::ESCAPE, NONE);
    ckey(&mut c, 'R' as u32, SHIFT);
    let t0 = Instant::now();
    while !c.poll() {
        assert!(t0.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(c.objects().len(), 3, "and the e-mail and the card");
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}

#[test]
fn the_edit_window_hides_what_a_lasso_leaves_out() {
    let mut c = canvas();
    let tri = vec![Pt::new(0.0, 0.0), Pt::new(400.0, 0.0), Pt::new(0.0, 300.0)];
    c.set_doc(None, Some(tri));
    let img = c.export();
    assert_eq!(img.get_pixel(10, 10)[3], 255);
    assert_eq!(img.get_pixel(390, 290)[3], 0);
}

// ---- previews ----

#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_snip_previews() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        let save = |a: &mut App, name: &str| {
            let img = settled_frame(a);
            std::fs::write(dir.join(format!("{tn}-{name}.png")), img.to_png().unwrap()).unwrap();
        };
        let mk = || {
            let mut a = preview_app(th, Some(SEL));
            a.focus_tries = u8::MAX;
            a
        };
        // Text mode: the words, a selection across a line break, its pill.
        let mut a = mk();
        with_doc(&mut a);
        save(&mut a, "text-mode");
        drag(&mut a, word(0, 1), word(1, 1));
        save(&mut a, "text-mode-selected");
        crate::ocr::FAKE_LINES.with(|f| f.set(Some(doc)));
        // The Copy text menu (a long press).
        let mut a = mk();
        settled_frame(&mut a);
        edit_of(&mut a).snip.copy_menu = true;
        save(&mut a, "copy-text-menu");
        // Redacted.
        let mut a = mk();
        key(&mut a, 'R' as u32, SHIFT);
        settle_ocr(&mut a);
        save(&mut a, "redacted");
        // A lasso, and its toggle by the mode switch.
        let mut a = mk();
        key(&mut a, 'F' as u32, SHIFT);
        draw_lasso(&mut a, &triangle());
        save(&mut a, "freeform");
        // The crop frame.
        let mut a = mk();
        key(&mut a, 'C' as u32, NONE);
        drag(&mut a, Pt::new(SEL.x1(), SEL.y1()), Pt::new(560.0, 320.0));
        save(&mut a, "crop-frame");
    }
    crate::ocr::FAKE_LINES.with(|f| f.set(None));
}
