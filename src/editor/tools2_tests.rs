//! Drawing tools, round 2, driven the way the window drives the editor:
//! the straight pencil and marker (Shift), numbered tags (numbering with
//! undo/redo, moving, the tail, size and color remembered), the fill
//! toggle (filled shapes, remembered), Save to Desktop (the export, the
//! notice, the shortcut); plus their preview PNGs.

use super::tests::{annotated, edit_of, preview_app, preview_app_with, settled_frame, synthetic_shot};
use super::*;

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
}

fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const BIG: FRect = FRect { x: 100.0, y: 80.0, w: 1240.0, h: 700.0 };

fn app() -> App {
    let mut a = preview_app(theme::DARK, Some(BIG));
    a.focus_tries = u8::MAX;
    a
}

fn down(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Down { x, y });
}
fn mv(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Move { x, y });
}
fn up(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Up { x, y });
}
fn click(a: &mut App, x: i32, y: i32) {
    down(a, x, y);
    up(a, x, y);
}
fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}
fn key(a: &mut App, vk: u32) {
    key_m(a, vk, NONE);
}
fn key_up(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: true, repeat: false, mods });
}

/// Run `act` as a toolbar press would (without the point).
fn act(a: &mut App, act: Act) {
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { panic!("no editor") };
    let mut e = *e;
    a.apply_act(&mut e, act);
    a.st = State::Edit(Box::new(e));
}

/// Drags see `m` held until the guard drops.
struct HeldMods;
fn hold(m: Mods) -> HeldMods {
    FAKE_MODS.with(|c| c.set(Some(m)));
    HeldMods
}
impl Drop for HeldMods {
    fn drop(&mut self) {
        FAKE_MODS.with(|c| c.set(Some(Mods::default())));
    }
}

/// The toolbar item `f` matches (laid out first).
fn item(a: &mut App, f: impl Fn(&toolbar::Kind) -> bool) -> toolbar::Item {
    let m = a.mouse;
    ed(a).prepare(None, m, Instant::now());
    let tb = ed(a).toolbar.as_ref().expect("toolbar");
    *tb.items.iter().find(|i| f(&i.kind)).expect("no such item")
}

fn mid(r: FRect) -> (i32, i32) {
    ((r.x + r.w / 2.0) as i32, (r.y + r.h / 2.0) as i32)
}

fn counters(a: &mut App) -> Vec<(u32, Pt, Option<Pt>)> {
    ed(a).objects.iter().filter_map(|o| match o {
        Obj::Counter { n, c, tail, .. } => Some((*n, *c, *tail)),
        _ => None,
    }).collect()
}

/// The next capture opens (remembered settings come from `App.cfg`).
fn next_capture(a: &mut App) {
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    a.pending = Some(Pending::editor());
    a.on_event(Ev::Timer);
}

fn close(a: &mut App) {
    key(a, wind::key::ESCAPE);
    key(a, wind::key::ESCAPE);
    assert!(matches!(a.st, State::Hidden), "closed");
}

// ---- Shift: straight pencil and marker ------------------------------------

/// Shift makes the stroke straight from the press point, snapped to 45
/// degree steps; let go and it is freehand again (every point kept).
#[test]
fn shift_draws_the_pencil_and_the_marker_straight() {
    for k in ['P', 'M'] {
        let mut a = app();
        key(&mut a, k as u32);
        down(&mut a, 300, 300);
        mv(&mut a, 340, 330);
        mv(&mut a, 380, 340);
        let free = ed(&mut a).draft.clone().unwrap();
        assert!(matches!(&free, Obj::Path { pts, .. } | Obj::MarkerPath { pts, .. } if pts.len() == 3), "{k}: freehand {free:?}");
        {
            let _m = hold(SHIFT);
            mv(&mut a, 500, 310);
            let flat = |p: Pt| (p.y - 300.0).abs() < 0.01 && (p.x - 300.0 - 200.25).abs() < 0.5;
            match ed(&mut a).draft.clone().unwrap() {
                Obj::Path { pts, .. } => assert!(pts.len() == 2 && pts[0] == Pt::new(300.0, 300.0) && flat(pts[1]), "{k}: {pts:?}"),
                Obj::Marker { a: s, b, color, .. } => {
                    assert!(s == Pt::new(300.0, 300.0) && flat(b) && color.a == 90, "{k}: {s:?} {b:?}");
                }
                o => panic!("{k}: straight? {o:?}"),
            }
            mv(&mut a, 450, 445);
            let end = match ed(&mut a).draft.clone().unwrap() {
                Obj::Path { pts, .. } => pts[1],
                Obj::Marker { b, .. } => b,
                o => panic!("{o:?}"),
            };
            assert!((end.x - end.y).abs() < 0.01, "{k}: 45 degrees {end:?}");
        }
        // Shift let go: freehand through every point, those drawn while
        // it was held included.
        mv(&mut a, 470, 470);
        match ed(&mut a).draft.clone().unwrap() {
            Obj::Path { pts, .. } | Obj::MarkerPath { pts, .. } => assert_eq!(pts.len(), 6, "{k}: {pts:?}"),
            o => panic!("{k}: freehand again? {o:?}"),
        }
        // Released with Shift: the straight stroke is committed.
        {
            let _m = hold(SHIFT);
            mv(&mut a, 600, 302);
            up(&mut a, 600, 302);
        }
        match &ed(&mut a).objects[..] {
            [Obj::Path { pts, .. }] => assert_eq!(pts.len(), 2, "{k}"),
            [Obj::Marker { a: s, b, .. }] => assert!(*s == Pt::new(300.0, 300.0) && (b.y - 300.0).abs() < 0.01),
            o => panic!("{k}: {o:?}"),
        }
    }
}

/// A freehand marker is committed as drawn; a 1 px wiggle is no stroke.
#[test]
fn the_marker_is_freehand_and_drops_tiny_strokes() {
    let mut a = app();
    key(&mut a, 'M' as u32);
    down(&mut a, 200, 200);
    for (x, y) in [(220, 210), (250, 230), (260, 260), (240, 290)] {
        mv(&mut a, x, y);
    }
    up(&mut a, 240, 290);
    assert!(matches!(&ed(&mut a).objects[..], [Obj::MarkerPath { pts, .. }] if pts.len() == 5));
    down(&mut a, 400, 400);
    mv(&mut a, 401, 401);
    up(&mut a, 401, 401);
    assert_eq!(ed(&mut a).objects.len(), 1, "a 1 px stroke is no object");
}

/// Shift / Ctrl pressed or let go without moving (Windows reports the
/// keys) redraw the draft at once.
#[test]
fn modifier_keys_redraw_the_draft_without_a_move() {
    let mut a = app();
    key(&mut a, 'P' as u32);
    down(&mut a, 300, 300);
    mv(&mut a, 350, 330);
    mv(&mut a, 420, 316);
    key_m(&mut a, VK_SHIFT, SHIFT);
    assert!(matches!(&ed(&mut a).draft, Some(Obj::Path { pts, .. }) if pts.len() == 2 && (pts[1].y - 300.0).abs() < 0.01), "straight");
    key_up(&mut a, VK_SHIFT, NONE);
    assert!(matches!(&ed(&mut a).draft, Some(Obj::Path { pts, .. }) if pts.len() == 3), "freehand again");
    up(&mut a, 420, 316);
    // A line: Ctrl snaps too.
    key(&mut a, 'L' as u32);
    down(&mut a, 300, 500);
    mv(&mut a, 420, 512);
    key_m(&mut a, VK_CONTROL, CTRL);
    assert!(matches!(ed(&mut a).draft, Some(Obj::Line { b, .. }) if (b.y - 500.0).abs() < 0.01));
    key_up(&mut a, VK_CONTROL, NONE);
    assert!(matches!(ed(&mut a).draft, Some(Obj::Line { b, .. }) if b == Pt::new(420.0, 512.0)));
    up(&mut a, 420, 512);
    // Not drawing: nothing to redraw (and no object appears).
    let n = ed(&mut a).objects.len();
    key_m(&mut a, VK_SHIFT, SHIFT);
    assert!(ed(&mut a).draft.is_none() && ed(&mut a).objects.len() == n);
}

// ---- numbered tags -----------------------------------------------------------

/// A click places the next number; undo/redo keep the order (the next one
/// is one more than the tags there are), other objects do not count.
#[test]
fn tags_number_in_order_through_undo_and_redo() {
    let mut a = app();
    key(&mut a, 'N' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Counter));
    for (x, y) in [(300, 300), (400, 300), (500, 300)] {
        click(&mut a, x, y);
    }
    let ns = |a: &mut App| counters(a).iter().map(|c| c.0).collect::<Vec<_>>();
    assert_eq!(ns(&mut a), [1, 2, 3]);
    assert_eq!(counters(&mut a)[1].1, Pt::new(400.0, 300.0), "where clicked");
    act(&mut a, Act::Undo);
    assert_eq!(ns(&mut a), [1, 2]);
    click(&mut a, 600, 300);
    assert_eq!(ns(&mut a), [1, 2, 3], "undoing tag 3 makes the next one 3 again");
    act(&mut a, Act::Undo);
    act(&mut a, Act::Undo);
    act(&mut a, Act::Redo);
    assert_eq!(ns(&mut a), [1, 2]);
    // Another object in between: not a tag.
    key(&mut a, 'R' as u32);
    down(&mut a, 200, 500);
    mv(&mut a, 300, 600);
    up(&mut a, 300, 600);
    key(&mut a, 'N' as u32);
    click(&mut a, 700, 300);
    assert_eq!(ns(&mut a), [1, 2, 3]);
    act(&mut a, Act::Undo); // the tag
    act(&mut a, Act::Undo); // the rect
    act(&mut a, Act::Redo);
    act(&mut a, Act::Redo);
    assert_eq!(ns(&mut a), [1, 2, 3], "redo brings tag 3 back");
    click(&mut a, 800, 300);
    assert_eq!(ns(&mut a), [1, 2, 3, 4]);
    // The tag has the tool's size and color.
    let e = ed(&mut a);
    assert!(matches!(e.objects.last(), Some(Obj::Counter { d, color, .. }) if *d == e.mem.size(Tool::Counter) && *color == e.mem.color(Tool::Counter)));
    assert_eq!(e.mem.size(Tool::Counter), tools::COUNTER_SIZE);
}

/// A drag moves the tag until the release; Ctrl+drag leaves it at the
/// press point with a tail to the pointer (switchable mid-drag).
#[test]
fn tags_move_with_a_drag_and_ctrl_draws_a_tail() {
    let mut a = app();
    key(&mut a, 'N' as u32);
    down(&mut a, 300, 300);
    mv(&mut a, 330, 320);
    assert_eq!(counters(&mut a), [], "nothing committed while dragging");
    assert!(matches!(ed(&mut a).draft, Some(Obj::Counter { c, tail: None, n: 1, .. }) if c == Pt::new(330.0, 320.0)));
    mv(&mut a, 360, 340);
    up(&mut a, 360, 340);
    assert_eq!(counters(&mut a), [(1, Pt::new(360.0, 340.0), None)]);
    {
        let _m = hold(CTRL);
        down(&mut a, 500, 300);
        mv(&mut a, 560, 380);
        mv(&mut a, 600, 420);
        up(&mut a, 600, 420);
    }
    assert_eq!(counters(&mut a)[1], (2, Pt::new(500.0, 300.0), Some(Pt::new(600.0, 420.0))));
    // Ctrl let go mid-drag (the key itself): the tag follows again.
    down(&mut a, 700, 300);
    {
        let _m = hold(CTRL);
        mv(&mut a, 760, 360);
    }
    assert!(matches!(ed(&mut a).draft, Some(Obj::Counter { c, tail: Some(t), .. }) if c == Pt::new(700.0, 300.0) && t == Pt::new(760.0, 360.0)));
    key_up(&mut a, VK_CONTROL, NONE);
    assert!(matches!(ed(&mut a).draft, Some(Obj::Counter { c, tail: None, .. }) if c == Pt::new(760.0, 360.0)));
    up(&mut a, 760, 360);
    assert_eq!(counters(&mut a).len(), 3);
}

/// The tag's size (diameter) and color are its own, remembered into the
/// next capture; the readout previews a mini tag.
#[test]
fn tag_size_and_color_are_remembered() {
    let mut a = app();
    a.kind = RunKind::Daemon;
    key(&mut a, 'N' as u32);
    for _ in 0..4 {
        act(&mut a, Act::Size(1));
    }
    let blue = C4::rgb(59, 130, 246);
    act(&mut a, Act::Color(blue));
    assert_eq!(ed(&mut a).size_label(), "36");
    assert_eq!(ed(&mut a).pending.get("tool_counter_size"), Some(&Some("36".to_string())));
    assert_eq!(ed(&mut a).pending.get("tool_counter_color"), Some(&Some("\"#3b82f6\"".to_string())));
    // The preview: the tag's color in the readout's box.
    let img = settled_frame(&mut a);
    let v = item(&mut a, |k| *k == toolbar::Kind::Value);
    let th = ed(&mut a).th;
    let ui = chrome::Ui { th: &th, s: 1.0, font: None };
    let b = chrome::preview_box(&ui, v.r);
    let (cx, cy) = ((b.x + b.w / 2.0) as u32, (b.y + 3.0) as u32);
    assert_eq!(img.get_pixel(cx, cy), [59, 130, 246, 255], "the disc's top in the box");
    close(&mut a);
    assert_eq!(a.cfg.tool_sizes.get("counter"), Some(&36.0));
    assert_eq!(a.cfg.tool_colors.get("counter").map(String::as_str), Some("#3b82f6"));
    next_capture(&mut a);
    let e = ed(&mut a);
    assert_eq!((e.mem.size(Tool::Counter), e.mem.color(Tool::Counter)), (36.0, blue));
}

// ---- fill ---------------------------------------------------------------------

/// The fill toggle by the size: on for the rectangle and the ellipse (each
/// its own), disabled for the other tools; filled shapes keep Shift's
/// square; remembered into the next capture.
#[test]
fn the_fill_toggle_draws_filled_shapes() {
    let mut a = app();
    a.kind = RunKind::Daemon;
    key(&mut a, 'L' as u32);
    assert!(item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Fill)).disabled, "a line has no fill");
    act(&mut a, Act::Fill);
    assert!(!ed(&mut a).mem.fill(Tool::Line) && ed(&mut a).pending.is_empty());
    key(&mut a, 'R' as u32);
    let b = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::Fill));
    assert!(!b.disabled);
    click(&mut a, mid(b.r).0, mid(b.r).1);
    assert!(ed(&mut a).mem.fill(Tool::Rect) && !ed(&mut a).mem.fill(Tool::Ellipse));
    assert!(ed(&mut a).objects.is_empty(), "the press was the button's");
    let half = C4::new(250, 220, 0, 128);
    act(&mut a, Act::Color(half));
    down(&mut a, 300, 300);
    mv(&mut a, 400, 350);
    up(&mut a, 400, 350);
    assert!(matches!(ed(&mut a).objects[..], [Obj::RectFill { r: rr, color, .. }] if rr == r(300.0, 300.0, 100.0, 50.0) && color == half));
    {
        let _m = hold(SHIFT);
        down(&mut a, 500, 300);
        mv(&mut a, 560, 500);
        up(&mut a, 560, 500);
    }
    assert!(matches!(ed(&mut a).objects[1], Obj::RectFill { r: rr, .. } if rr == r(500.0, 300.0, 200.0, 200.0)), "a square");
    key(&mut a, 'E' as u32);
    down(&mut a, 300, 500);
    mv(&mut a, 400, 560);
    up(&mut a, 400, 560);
    assert!(matches!(ed(&mut a).objects[2], Obj::Ellipse { .. }), "the ellipse keeps its own");
    act(&mut a, Act::Fill);
    down(&mut a, 300, 600);
    mv(&mut a, 400, 660);
    up(&mut a, 400, 660);
    assert!(matches!(ed(&mut a).objects[3], Obj::EllipseFill { .. }));
    act(&mut a, Act::Fill);
    assert!(!ed(&mut a).mem.fill(Tool::Ellipse), "toggles back");
    assert_eq!(ed(&mut a).pending.get("tool_rect_fill"), Some(&Some("true".to_string())));
    assert_eq!(ed(&mut a).pending.get("tool_ellipse_fill"), Some(&Some("false".to_string())));
    // The translucent fill bakes in: half the capture under it.
    ed(&mut a).rebuild();
    let base = ed(&mut a).base.get_pixel(350, 325);
    let got = ed(&mut a).composed().get_pixel(350, 325);
    for c in 0..3 {
        let want = (half_of(half, c) + base[c] as f32 * (1.0 - 128.0 / 255.0)).round() as i32;
        assert!((got[c] as i32 - want).abs() <= 1, "channel {c}: {got:?} vs {base:?}");
    }
    close(&mut a);
    assert_eq!(a.cfg.tool_fills.get("rect"), Some(&true));
    assert_eq!(a.cfg.tool_fills.get("ellipse"), Some(&false));
    next_capture(&mut a);
    assert!(ed(&mut a).mem.fill(Tool::Rect) && !ed(&mut a).mem.fill(Tool::Ellipse), "restored");
}

fn half_of(c: C4, i: usize) -> f32 {
    [c.r, c.g, c.b][i] as f32 * 128.0 / 255.0
}

/// The fill choice is written to the config file like the sizes.
#[test]
fn the_fill_choice_is_written_to_the_config() {
    let d = std::env::temp_dir().join(format!("rustshot-tools2-fill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("config.toml");
    std::fs::write(&p, "# mine\ntheme = \"dark\"\n").unwrap();
    let mut a = app();
    ed(&mut a).cfg_path = Some(p.clone());
    key(&mut a, 'E' as u32);
    act(&mut a, Act::Fill);
    close(&mut a);
    let text = std::fs::read_to_string(&p).unwrap();
    assert_eq!(text, "# mine\ntheme = \"dark\"\ntool_ellipse_fill = true\n");
    let _ = std::fs::remove_dir_all(&d);
}

// ---- Save to Desktop ---------------------------------------------------------

/// The button and Ctrl+D save the selection to the Desktop with the
/// filename pattern and format (no daily folder); a second save gets a
/// suffix; the notice says where it went.
#[test]
fn save_to_desktop_writes_the_capture_there() {
    let d = std::env::temp_dir().join(format!("rustshot-tools2-desk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    crate::desktop::TEST_DIR.with(|t| *t.borrow_mut() = Some(Some(d.clone())));
    let _ = crate::notice::take_shown();
    let mut a = app();
    a.kind = RunKind::Daemon;
    assert!(!item(&mut a, |k| *k == toolbar::Kind::Btn(Act::SaveMenu)).disabled);
    ed(&mut a).cfg.filename_pattern = "shot".into();
    ed(&mut a).cfg.save_subfolder = true;
    ed(&mut a).cfg.save_format = "jpg".into();
    key_m(&mut a, 'D' as u32, CTRL);
    assert!(matches!(a.st, State::Hidden), "exported and hidden");
    assert!(d.join("shot.jpg").is_file(), "on the Desktop, no daily folder");
    let shown = crate::notice::take_shown();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].0.text, "Saved to Desktop \u{b7} shot.jpg");
    // The button, again: a suffix.
    next_capture(&mut a);
    ed(&mut a).sel = Some(r(10.0, 10.0, 40.0, 30.0));
    ed(&mut a).cfg.filename_pattern = "shot".into();
    ed(&mut a).cfg.save_format = "jpg".into();
    // Through the Save button's menu.
    let b = item(&mut a, |k| *k == toolbar::Kind::Btn(Act::SaveMenu));
    click(&mut a, mid(b.r).0, mid(b.r).1);
    assert!(ed(&mut a).save_menu, "the menu opens");
    let b = item(&mut a, |k| matches!(k, toolbar::Kind::MenuItem { act: Act::SaveDesktop, .. }));
    click(&mut a, mid(b.r).0, mid(b.r).1);
    assert!(d.join("shot_1.jpg").is_file());
    let shown = crate::notice::take_shown();
    assert_eq!(shown[0].0.text, "Saved to Desktop \u{b7} shot_1.jpg");
    // No Desktop folder: an error notice.
    crate::desktop::TEST_DIR.with(|t| *t.borrow_mut() = Some(None));
    next_capture(&mut a);
    key_m(&mut a, 'D' as u32, CTRL);
    let shown = crate::notice::take_shown();
    assert_eq!(shown[0].0.text, "Save failed: no Desktop folder");
    assert_eq!(a.exit_code.load(Ordering::SeqCst), 1);
    crate::desktop::TEST_DIR.with(|t| *t.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&d);
}

/// The toolbar's new buttons and their tooltips.
#[test]
fn new_buttons_have_tooltips_and_shortcuts() {
    let km = Keymap::defaults();
    assert_eq!(chrome::act_tip(Act::SaveDesktop, &km).1.last().map(String::as_str), Some("D"));
    assert_eq!(chrome::act_tip(Act::Tool(Tool::Counter), &km).1, ["N"]);
    assert_eq!(chrome::act_tip(Act::Fill, &km), ("Fill shape", Vec::new()));
    assert_eq!(action_of(Act::SaveDesktop), Some(Action::SaveDesktop));
    let (label, keys) = chrome::act_tip(Act::SaveAs, &km);
    assert_eq!(label, "Save as\u{2026}");
    assert_eq!(keys.last().map(String::as_str), Some("S"), "{keys:?}");
    assert_eq!(keys.len(), 3, "Ctrl+Shift+S: {keys:?}");
    assert_eq!(action_of(Act::SaveAs), Some(Action::SaveAs));
    assert_eq!(chrome::act_icon(Act::SaveAs), "save-as");
    assert_ne!(chrome::act_icon(Act::SaveAs), chrome::act_icon(Act::Save));
    let mut a = app();
    let tb = {
        let m = a.mouse;
        ed(&mut a).prepare(None, m, Instant::now());
        ed(&mut a).toolbar.clone().unwrap()
    };
    let at = |x: Act| tb.items.iter().position(|i| i.kind == toolbar::Kind::Btn(x)).unwrap();
    assert_eq!(at(Act::SaveMenu), at(Act::Save) + 1, "Save's split chevron");
    assert!(tb.items.iter().all(|i| i.kind != toolbar::Kind::Btn(Act::SaveAs) && i.kind != toolbar::Kind::Btn(Act::SaveDesktop)), "in its menu");
    // The chevron opens the menu: Save As, then Save to Desktop, with keys.
    ed(&mut a).save_menu = true;
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let tb = ed(&mut a).toolbar.clone().unwrap();
    let rows: Vec<Act> = tb.items.iter().filter_map(|i| match i.kind { toolbar::Kind::MenuItem { act, .. } => Some(act), _ => None }).collect();
    assert_eq!(rows, [Act::SaveAs, Act::SaveDesktop, Act::OpenWith]);
    assert_eq!(tb.texts.len(), 3);
    assert!(tb.texts[0].ends_with('S') && tb.texts[1].ends_with('D') && tb.texts[2].ends_with('O'), "{:?}", tb.texts);
    assert!(tb.pop.is_some());
    assert_eq!(at(Act::Fill), at(Act::Size(1)) + 1, "next to the size");
    assert_eq!(at(Act::Tool(Tool::Callout)), at(Act::Tool(Tool::Text)) + 1);
    assert_eq!(at(Act::Tool(Tool::Counter)), at(Act::Tool(Tool::Callout)) + 1);
}

// ---- previews ---------------------------------------------------------------

fn shot2x() -> Shot {
    let s = synthetic_shot();
    let (w, h) = (s.size.0 * 2, s.size.1 * 2);
    let mut img = PixBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = s.image.get_pixel(x / 2, y / 2);
            let i = (y as usize * w as usize + x as usize) * 4;
            img.as_raw_mut()[i..i + 4].copy_from_slice(&p);
        }
    }
    Shot { origin: (0, 0), size: (w, h), scale: 2.0, image: img, monitors: vec![(0, 0, w, h)] }
}

/// Preview PNGs under `RUSTSHOT_PREVIEW_DIR`: the toolbar with the new
/// buttons (the number tool, the fill toggle off and on, its tooltip, Save
/// to Desktop's and Save As's tooltips), tags at several sizes and colors, filled and
/// translucent shapes, the straight marker, the size preview sheet.
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_tools2_previews() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let save = |img: &PixBuf, name: &str| std::fs::write(dir.join(name), img.to_png().unwrap()).unwrap();
    let colors = [C4::rgb(240, 68, 56), C4::rgb(59, 130, 246), C4::rgb(45, 192, 111), C4::rgb(250, 220, 0), C4::rgb(17, 19, 24), C4::rgb(255, 255, 255), C4::rgb(236, 72, 153), C4::new(139, 92, 246, 150)];
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        for (sn, k) in [("1x", 1.0f32), ("2x", 2.0)] {
            let shot = || if k == 1.0 { synthetic_shot() } else { shot2x() };
            let s = |v: f32| v * k;
            let big = r(s(100.0), s(80.0), s(1240.0), s(700.0));
            let name = |what: &str| format!("{tn}-{sn}-{what}.png");
            let fresh = || {
                let mut a = preview_app_with(th, Some(big), shot());
                a.focus_tries = u8::MAX;
                a
            };
            let hover = |a: &mut App, x: Act| {
                settled_frame(a);
                let i = ed(a).toolbar.as_ref().unwrap().items.iter().position(|it| it.kind == toolbar::Kind::Btn(x));
                ed(a).hover = i;
                ed(a).hover_at = Instant::now() - Duration::from_secs(2);
            };
            // The bar: the number tool chosen; the rectangle with the fill
            // on (its tooltip); the ellipse filled; Save to Desktop's tip.
            let mut a = fresh();
            ed(&mut a).tool = Some(Tool::Counter);
            save(&settled_frame(&mut a), &name("toolbar-number"));
            let mut a = fresh();
            ed(&mut a).tool = Some(Tool::Rect);
            ed(&mut a).mem.toggle_fill(Tool::Rect);
            hover(&mut a, Act::Fill);
            save(&settled_frame(&mut a), &name("toolbar-fill-on"));
            let mut a = fresh();
            ed(&mut a).tool = Some(Tool::Ellipse);
            ed(&mut a).mem.toggle_fill(Tool::Ellipse);
            save(&settled_frame(&mut a), &name("toolbar-ellipse-filled"));
            let mut a = fresh();
            ed(&mut a).tool = Some(Tool::Rect);
            save(&settled_frame(&mut a), &name("toolbar-fill-off"));
            let mut a = fresh();
            ed(&mut a).save_menu = true;
            hover(&mut a, Act::SaveMenu);
            save(&settled_frame(&mut a), &name("toolbar-save-menu"));
            let mut a = fresh();
            hover(&mut a, Act::SaveMenu);
            save(&settled_frame(&mut a), &name("toolbar-save-as-tip"));
            let mut a = fresh();
            ed(&mut a).tool = Some(Tool::Counter);
            hover(&mut a, Act::Tool(Tool::Counter));
            save(&settled_frame(&mut a), &name("toolbar-number-tip"));
            // Narrow selection: the bar wraps.
            let mut a = preview_app_with(th, Some(r(s(560.0), s(300.0), s(300.0), s(160.0))), shot());
            ed(&mut a).tool = Some(Tool::Rect);
            save(&settled_frame(&mut a), &name("toolbar-wrapped"));
            // Tags: sizes across, colors down; with a tail; two-digit.
            let mut a = fresh();
            let e = ed(&mut a);
            let mut n = 0;
            for (row, c) in colors.iter().enumerate() {
                for (col, d) in [16.0f32, 24.0, 32.0, 44.0, 60.0, 84.0].into_iter().enumerate() {
                    n += 1;
                    let x = 180.0 + col as f32 * 120.0;
                    let y = 130.0 + row as f32 * 80.0;
                    e.objects.push(Obj::Counter { c: Pt::new(s(x), s(y)), n: if col == 5 { n + 90 } else { n }, color: *c, d: s(d), tail: None });
                }
            }
            for (i, c) in colors.iter().take(4).enumerate() {
                let (x, y) = (1000.0 + (i % 2) as f32 * 160.0, 160.0 + (i / 2) as f32 * 220.0);
                let tip = Pt::new(s(x + 70.0), s(y + 110.0 - (i % 2) as f32 * 160.0));
                e.objects.push(Obj::Counter { c: Pt::new(s(x), s(y)), n: i as u32 + 1, color: *c, d: s(36.0), tail: Some(tip) });
            }
            e.tool = Some(Tool::Counter);
            e.dirty = true;
            save(&settled_frame(&mut a), &name("tags"));
            // Shapes: outlined, filled, translucent highlight boxes over
            // the capture, a filled square; the straight and freehand marker.
            let mut a = fresh();
            let e = ed(&mut a);
            let red = C4::rgb(240, 68, 56);
            e.objects.push(Obj::Rect { r: r(s(160.0), s(120.0), s(220.0), s(120.0)), color: red, width: s(3.0) });
            e.objects.push(Obj::RectFill { r: r(s(420.0), s(120.0), s(220.0), s(120.0)), color: red, width: s(3.0) });
            e.objects.push(Obj::RectFill { r: r(s(680.0), s(120.0), s(260.0), s(120.0)), color: C4::new(250, 220, 0, 110), width: s(2.0) });
            e.objects.push(Obj::RectFill { r: r(s(980.0), s(120.0), s(200.0), s(200.0)), color: C4::new(59, 130, 246, 90), width: s(8.0) });
            e.objects.push(Obj::Ellipse { r: r(s(160.0), s(300.0), s(220.0), s(140.0)), color: red, width: s(3.0) });
            e.objects.push(Obj::EllipseFill { r: r(s(420.0), s(300.0), s(220.0), s(140.0)), color: C4::rgb(45, 192, 111), width: s(3.0) });
            e.objects.push(Obj::EllipseFill { r: r(s(680.0), s(300.0), s(260.0), s(140.0)), color: C4::new(236, 72, 153, 100), width: s(3.0) });
            let my = |c: C4| marker_color(c);
            e.objects.push(Obj::Marker { a: Pt::new(s(170.0), s(520.0)), b: Pt::new(s(620.0), s(520.0)), color: my(C4::rgb(250, 220, 0)), width: s(18.0) });
            e.objects.push(Obj::Marker { a: Pt::new(s(170.0), s(570.0)), b: Pt::new(s(420.0), s(720.0)), color: my(C4::rgb(45, 192, 111)), width: s(14.0) });
            let wave: Vec<Pt> = (0..60).map(|i| Pt::new(s(680.0 + i as f32 * 8.0), s(560.0 + (i as f32 * 0.25).sin() * 30.0))).collect();
            e.objects.push(Obj::MarkerPath { pts: wave, color: my(C4::rgb(250, 220, 0)), width: s(18.0) });
            e.objects.push(Obj::Path { pts: vec![Pt::new(s(680.0), s(680.0)), Pt::new(s(1160.0), s(680.0))], color: red, width: s(3.0) });
            e.tool = Some(Tool::Rect);
            e.mem.toggle_fill(Tool::Rect);
            e.dirty = true;
            save(&settled_frame(&mut a), &name("shapes-marker"));
            // A Shift-straight marker being drawn (the draft).
            let mut a = fresh();
            ed(&mut a).tool = Some(Tool::Marker);
            let p0 = (s(300.0) as i32, s(400.0) as i32);
            down(&mut a, p0.0, p0.1);
            for i in 1..12 {
                mv(&mut a, p0.0 + s(i as f32 * 30.0) as i32, p0.1 + s((i as f32).sin() * 25.0) as i32);
            }
            {
                let _m = hold(SHIFT);
                mv(&mut a, p0.0 + s(500.0) as i32, p0.1 + s(30.0) as i32);
            }
            assert!(matches!(ed(&mut a).draft, Some(Obj::Marker { .. })), "straight while Shift is held");
            save(&settled_frame(&mut a), &name("marker-shift-draft"));
            // The size preview of the number tool, sizes across.
            let sizes = [12.0f32, 16.0, 24.0, 32.0, 48.0, 96.0];
            let mut cells = Vec::new();
            for c in [C4::rgb(240, 68, 56), C4::rgb(250, 220, 0)] {
                for &v in &sizes {
                    let mut a = fresh();
                    let e = ed(&mut a);
                    e.tool = Some(Tool::Counter);
                    e.mem.set_size(Tool::Counter, v);
                    e.mem.set_color(Tool::Counter, c);
                    let img = settled_frame(&mut a);
                    let tb = ed(&mut a).toolbar.clone().unwrap();
                    let at = |a: Act| tb.items.iter().find(|i| i.kind == toolbar::Kind::Btn(a)).unwrap().r;
                    let (m, p) = (at(Act::Size(-1)), at(Act::Palette));
                    let pad = 4.0 * k;
                    cells.push(img.crop((m.x - pad) as u32, (m.y - pad) as u32, (p.x1() - m.x + 2.0 * pad) as u32, (m.h + 2.0 * pad) as u32));
                }
            }
            let (cw, ch) = (cells[0].width(), cells[0].height());
            let cols = sizes.len() as u32;
            let mut sheet = PixBuf::from_pixel(cols * cw, 2 * ch, [0, 0, 0, 255]);
            for (i, c) in cells.iter().enumerate() {
                let (ox, oy) = ((i as u32 % cols) * cw, (i as u32 / cols) * ch);
                for y in 0..ch {
                    for x in 0..cw {
                        let p = c.get_pixel(x, y);
                        let o = ((oy + y) as usize * (cols * cw) as usize + (ox + x) as usize) * 4;
                        sheet.as_raw_mut()[o..o + 4].copy_from_slice(&p);
                    }
                }
            }
            save(&sheet, &name("number-size-preview"));
        }
    }
    // The annotated preview at 1x for a quick look at the whole bar.
    let mut a = annotated(theme::DARK, BIG);
    save(&settled_frame(&mut a), "dark-1x-toolbar-annotated.png");
}
