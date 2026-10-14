//! The cursor per region (chrome, objects, handles, the tool) and picking
//! objects up with the drawing tools: hover outline, drag moves (snapping,
//! one undo step), click selects, Ctrl draws over, the tools that act on
//! what is under them always draw.

use super::super::tests::{edit_of, preview_app};
use super::super::*;
use super::Under;

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
const ALT: Mods = Mods { shift: false, ctrl: false, alt: true };
const VK_CONTROL: u32 = 0x11;
const SEL: FRect = FRect { x: 100.0, y: 80.0, w: 1200.0, h: 760.0 };
const RED: C4 = C4::rgb(240, 68, 56);

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
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
fn drag(a: &mut App, from: (i32, i32), to: (i32, i32)) {
    down(a, from.0, from.1);
    mv(a, (from.0 + to.0) / 2, (from.1 + to.1) / 2);
    mv(a, to.0, to.1);
    up(a, to.0, to.1);
}
fn click(a: &mut App, x: i32, y: i32) {
    down(a, x, y);
    up(a, x, y);
}

struct Held;
fn hold(m: Mods) -> Held {
    FAKE_MODS.with(|c| c.set(Some(m)));
    Held
}
impl Drop for Held {
    fn drop(&mut self) {
        FAKE_MODS.with(|c| c.set(Some(Mods::default())));
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Obj {
    Obj::Rect { r: FRect { x, y, w, h }, color: RED, width: 4.0 }
}

/// An editor over `SEL` (or nothing selected) with `objs` committed and
/// tool `tool`, a frame prepared (the chrome laid out).
fn with(sel: Option<FRect>, objs: Vec<Obj>, tool: Option<Tool>) -> App {
    let mut a = preview_app(theme::DARK, sel);
    a.focus_tries = u8::MAX;
    for o in objs {
        ed(&mut a).commit_object(o);
    }
    ed(&mut a).tool = tool;
    ed(&mut a).prepare(None, (0, 0), Instant::now());
    a
}

fn at(e: &Edit, x: f32, y: f32) -> Cursor {
    e.cursor_at(Pt::new(x, y), NONE)
}

fn mid(r: FRect) -> Pt {
    Pt::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// The outline of the rect at (300, 300, 200, 100) (its top edge).
const ON: (i32, i32) = (350, 301);

#[test]
fn cursor_per_region_with_a_drawing_tool() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(Tool::Rect));
    let e = ed(&mut a);
    // The object: the move cursor and the pick-up; empty canvas: the tool's cross.
    assert_eq!(at(e, 350.0, 301.0), Cursor::Move);
    assert_eq!(e.under(Pt::new(350.0, 301.0), NONE), Under::Grab(0));
    assert_eq!(e.hover_obj(Pt::new(350.0, 301.0), NONE), Some(0));
    assert_eq!(at(e, 1000.0, 600.0), Cursor::Cross);
    assert_eq!(at(e, 400.0, 350.0), Cursor::Cross, "inside a hollow rect: no hit");
    // Ctrl draws over it.
    assert_eq!(e.cursor_at(Pt::new(350.0, 301.0), CTRL), Cursor::Cross);
    assert_eq!(e.hover_obj(Pt::new(350.0, 301.0), CTRL), None);
    // The chrome: the toolbar (every button) is the arrow.
    let tb = e.toolbar.clone().expect("toolbar");
    assert_eq!(e.cursor_at(mid(tb.bar), NONE), Cursor::Arrow);
    for it in &tb.items {
        assert_eq!(e.cursor_at(mid(it.r), NONE), Cursor::Arrow, "{:?}", it.kind);
    }
    // The size label draws with a drawing tool, is a button (its menu)
    // with none.
    // (Its left end: without a Record button (macOS tests) the label
    // follows the freeform toggle and its right part is under the bar.)
    let label = e.label_at.expect("size label");
    let end = Pt::new(label.x + 4.0, label.y + label.h / 2.0);
    assert!(!tb.contains(end), "{label:?} {:?}", tb.bar);
    assert_eq!(e.cursor_at(end, NONE), Cursor::Cross);
    e.tool = None;
    assert_eq!(e.cursor_at(end, NONE), Cursor::Arrow);
}

#[test]
fn tools_acting_on_what_is_under_them_keep_their_cursor() {
    for t in [Tool::Pixelate, Tool::Blur, Tool::Invert, Tool::Spotlight, Tool::Ruler, Tool::Magnify] {
        let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(t));
        let e = ed(&mut a);
        assert_eq!(at(e, 350.0, 301.0), Cursor::Cross, "{t:?}");
        assert_eq!(e.hover_obj(Pt::new(350.0, 301.0), NONE), None, "{t:?}");
    }
    // The others pick up.
    for t in [Tool::Path, Tool::Line, Tool::Arrow, Tool::Ellipse, Tool::Marker, Tool::Text, Tool::Counter, Tool::Callout] {
        let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(t));
        assert_eq!(at(ed(&mut a), 350.0, 301.0), Cursor::Move, "{t:?}");
    }
}

#[test]
fn cursor_with_no_tool_and_the_select_tool() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], None);
    let e = ed(&mut a);
    assert_eq!(at(e, 350.0, 301.0), Cursor::Move);
    assert_eq!(e.hover_obj(Pt::new(350.0, 301.0), NONE), Some(0));
    // The capture selection: its handles resize, inside moves it, outside draws a new one.
    assert_eq!(at(e, SEL.x, SEL.y), Cursor::SizeNWSE);
    assert_eq!(at(e, SEL.x1(), SEL.y + SEL.h / 2.0), Cursor::SizeWE);
    assert_eq!(at(e, 1000.0, 600.0), Cursor::Move);
    assert_eq!(at(e, 1500.0, 900.0), Cursor::Cross);
    e.tool = Some(Tool::Select);
    assert_eq!(at(e, 350.0, 301.0), Cursor::Move);
    assert_eq!(at(e, 1000.0, 600.0), Cursor::Arrow);
    assert_eq!(e.cursor_at(Pt::new(350.0, 301.0), CTRL), Cursor::Move, "Ctrl changes nothing for the select tool");
}

#[test]
fn chrome_with_nothing_selected_and_the_color_ring() {
    // The hint card is up while nothing is selected.
    let mut a = with(None, Vec::new(), None);
    let e = ed(&mut a);
    // Faded in.
    e.prepare(None, (0, 0), Instant::now() + std::time::Duration::from_millis(300));
    let hint = e.scene.as_ref().unwrap().rects(compose::El::Hint).to_vec();
    assert!(!hint.is_empty(), "hint card shown");
    let h = hint[0];
    let c = Pt::new((h.x0 + h.x1) as f32 / 2.0, (h.y0 + h.y1) as f32 / 2.0);
    assert_eq!(e.cursor_at(c, NONE), Cursor::Arrow);
    assert_eq!(at(e, 50.0, 50.0), Cursor::Cross);
    // The color ring takes the pointer.
    let mut a = with(Some(SEL), Vec::new(), Some(Tool::Rect));
    a.on_event(Ev::Right { down: true, x: 700, y: 400 });
    assert!(ed(&mut a).assist.ring.is_some());
    assert_eq!(at(ed(&mut a), 1000.0, 600.0), Cursor::Arrow);
}

#[test]
fn selected_object_handles_and_mini_toolbar() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(Tool::Arrow));
    click(&mut a, ON.0, ON.1);
    assert_eq!(ed(&mut a).obj.sel, vec![0], "a click selects it");
    assert_eq!(ed(&mut a).objects.len(), 1, "nothing drawn");
    assert_eq!(ed(&mut a).tool, Some(Tool::Arrow), "the tool stays");
    ed(&mut a).prepare(None, ON, Instant::now());
    let e = ed(&mut a);
    // The SE handle: on the frame 6 px out.
    assert_eq!(at(e, 506.0, 406.0), Cursor::SizeNWSE);
    assert_eq!(at(e, 506.0, 350.0), Cursor::SizeWE);
    let m = e.obj.mini.clone().expect("mini-toolbar");
    assert_eq!(e.cursor_at(mid(m.r), NONE), Cursor::Arrow);
    // No hover outline for the selected one.
    assert_eq!(e.hover_obj(Pt::new(350.0, 301.0), NONE), None);
    // The handle resizes, with the drawing tool still on.
    drag(&mut a, (506, 406), (566, 456));
    assert_eq!(crate::objects::edit::sel_box(&ed(&mut a).objects[0], None).unwrap(), FRect { x: 300.0, y: 300.0, w: 260.0, h: 150.0 });
    assert_eq!(ed(&mut a).objects.len(), 1);
}

#[test]
fn drawing_tool_drag_moves_an_object_in_one_undo_step() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(Tool::Rect));
    let hi = ed(&mut a).hi;
    {
        let _m = hold(ALT); // no snapping
        drag(&mut a, ON, (ON.0 + 37, ON.1 + 20));
    }
    let e = ed(&mut a);
    assert_eq!(e.objects.len(), 1, "moved, nothing drawn");
    assert_eq!(crate::objects::edit::sel_box(&e.objects[0], None).unwrap(), FRect { x: 337.0, y: 320.0, w: 200.0, h: 100.0 });
    assert_eq!(e.hi, hi + 1, "one undo step");
    assert_eq!(e.obj.sel, vec![0]);
    assert_eq!(e.tool, Some(Tool::Rect));
    a.on_event(Ev::Key { vk: 'Z' as u32, up: false, repeat: false, mods: CTRL });
    assert_eq!(crate::objects::edit::sel_box(&ed(&mut a).objects[0], None).unwrap(), FRect { x: 300.0, y: 300.0, w: 200.0, h: 100.0 });
}

#[test]
fn drawing_tool_moves_snap_like_the_select_tool() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0), rect(700.0, 520.0, 100.0, 100.0)], Some(Tool::Line));
    down(&mut a, 720, 521);
    mv(&mut a, 730, 400);
    mv(&mut a, 740, 305);
    let b = crate::objects::edit::sel_box(&ed(&mut a).objects[1], None).unwrap();
    assert_eq!(b.y, 300.0, "snapped to the other's top: {b:?}");
    assert!(!ed(&mut a).obj.guides.is_empty());
    up(&mut a, 740, 305);
    assert_eq!(ed(&mut a).objects.len(), 2);
}

#[test]
fn ctrl_draws_over_and_the_exceptions_always_draw() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(Tool::Rect));
    {
        let _m = hold(CTRL);
        drag(&mut a, ON, (450, 380));
    }
    assert_eq!(ed(&mut a).objects.len(), 2, "Ctrl: a new rect over it");
    assert_eq!(crate::objects::edit::sel_box(&ed(&mut a).objects[0], None).unwrap().x, 300.0, "the first did not move");
    // Ctrl + click with a drawing tool no longer selects.
    {
        let _m = hold(CTRL);
        click(&mut a, ON.0, ON.1);
    }
    assert!(ed(&mut a).obj.sel.is_empty());
    for t in [Tool::Pixelate, Tool::Blur, Tool::Spotlight] {
        let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(t));
        drag(&mut a, ON, (450, 380));
        assert_eq!(ed(&mut a).objects.len(), 2, "{t:?} draws on the object");
        assert!(ed(&mut a).obj.sel.is_empty(), "{t:?}");
    }
}

#[test]
fn no_tool_moves_objects_before_the_selection() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], None);
    {
        let _m = hold(ALT);
        drag(&mut a, ON, (ON.0 + 10, ON.1 + 10));
    }
    let e = ed(&mut a);
    assert_eq!(e.sel, Some(SEL), "the capture selection stays");
    assert_eq!(crate::objects::edit::sel_box(&e.objects[0], None).unwrap().x, 310.0);
    // Elsewhere inside, the selection moves.
    drag(&mut a, (1000, 600), (1010, 600));
    assert_eq!(ed(&mut a).sel.map(|s| s.x), Some(SEL.x + 10.0));
}

#[test]
fn hover_outline_follows_the_pointer_and_ctrl() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(Tool::Ellipse));
    mv(&mut a, ON.0, ON.1);
    assert_eq!(ed(&mut a).obj.hover, Some(0));
    ed(&mut a).prepare(None, ON, Instant::now());
    let u = ed(&mut a).scene.as_ref().unwrap().osel.clone().expect("hover outline");
    assert_eq!(u.hover.len(), 1);
    assert!(u.boxes.is_empty() && u.square.is_empty(), "no selection chrome");
    let f = u.hover[0];
    assert!(f.x < 300.0 && f.y < 300.0 && f.x1() > 500.0, "{f:?}");
    // Ctrl down: it goes; up: back.
    a.on_event(Ev::Key { vk: VK_CONTROL, up: false, repeat: false, mods: CTRL });
    assert_eq!(ed(&mut a).obj.hover, None);
    a.on_event(Ev::Key { vk: VK_CONTROL, up: true, repeat: false, mods: NONE });
    assert_eq!(ed(&mut a).obj.hover, Some(0));
    // Off it: gone.
    mv(&mut a, 1000, 600);
    assert_eq!(ed(&mut a).obj.hover, None);
    ed(&mut a).prepare(None, (1000, 600), Instant::now());
    assert!(ed(&mut a).scene.as_ref().unwrap().osel.is_none());
}

#[test]
fn callout_tool_click_types_and_drag_moves() {
    let mut a = with(Some(SEL), Vec::new(), Some(Tool::Callout));
    drag(&mut a, (400, 300), (600, 400));
    a.on_event(Ev::Char('h' as u16));
    a.on_event(Ev::Key { vk: crate::wind::key::ESCAPE, up: false, repeat: false, mods: NONE });
    assert!(ed(&mut a).obj.cedit.is_none());
    assert_eq!(ed(&mut a).objects.len(), 1);
    let b = crate::objects::edit::sel_box(&ed(&mut a).objects[0], None).unwrap();
    let c = (b.x as i32 + 20, b.y as i32 + 20);
    {
        let _m = hold(ALT);
        drag(&mut a, c, (c.0 + 30, c.1));
    }
    let b2 = crate::objects::edit::sel_box(&ed(&mut a).objects[0], None).unwrap();
    assert_eq!(b2.x, b.x + 30.0, "moved");
    assert!(ed(&mut a).obj.cedit.is_none());
    click(&mut a, c.0 + 30, c.1);
    assert!(ed(&mut a).obj.cedit.is_some(), "a click types in it");
    assert_eq!(ed(&mut a).objects.len(), 1);
}

/// The Edit window: the same rule through its canvas (window points):
/// outside the canvas (the status bar, the side list) the arrow, an
/// object with a drawing tool the move cursor, Ctrl the cross.
#[test]
fn edit_window_cursor_per_region() {
    let img = PixBuf::from_pixel(400, 300, [200, 210, 220, 255]);
    let mut c = host::Canvas::new(img, vec![rect(100.0, 100.0, 150.0, 80.0)], Config::default(), theme::DARK);
    let area = FRect { x: 0.0, y: 40.0, w: 1200.0, h: 720.0 };
    c.set_rects(area, FRect { x: 0.0, y: 0.0, w: 1200.0, h: 40.0 }, FRect { x: 0.0, y: 0.0, w: 1200.0, h: 800.0 }, 1.0);
    c.key('R' as u32, false, false, NONE);
    let on = c.view.to_win(Pt::new(150.0, 101.0));
    assert_eq!(c.cursor(on), Cursor::Move);
    assert_eq!(c.cursor(c.view.to_win(Pt::new(175.0, 140.0))), Cursor::Cross, "inside the hollow rect");
    assert_eq!(c.cursor(Pt::new(600.0, 790.0)), Cursor::Arrow, "below the canvas");
    {
        let _m = hold(CTRL);
        assert_eq!(c.cursor(on), Cursor::Cross);
    }
    c.key('B' as u32, false, false, NONE);
    assert_eq!(c.cursor(on), Cursor::Cross);
    c.key('V' as u32, false, false, NONE);
    assert_eq!(c.cursor(on), Cursor::Move);
}

// ---- the pointer hidden under the crosshair ---------------------------------

fn ptr(e: &Edit, x: f32, y: f32) -> Cursor {
    e.pointer_at(Pt::new(x, y), NONE, true)
}

/// The system pointer is hidden exactly where the magnifier's crosshair
/// is drawn through it, and nowhere else.
#[test]
fn pointer_hidden_only_where_the_crosshair_is() {
    // Nothing selected, no tool: the crosshair, no pointer.
    let mut a = with(None, Vec::new(), None);
    let e = ed(&mut a);
    e.assist.dismiss_hint(Instant::now());
    e.prepare(None, (700, 500), Instant::now());
    assert!(e.crosshair_at(Pt::new(700.0, 500.0)));
    assert!(e.scene.as_ref().unwrap().cross.is_some(), "the crosshair is drawn");
    assert_eq!(ptr(e, 700.0, 500.0), Cursor::Hidden);
    // Not focused: the pointer stays.
    assert_eq!(e.pointer_at(Pt::new(700.0, 500.0), NONE, false), Cursor::Cross);
    // Off the capture: no crosshair, the pointer.
    assert!(!e.crosshair_at(Pt::new(-1.0, 500.0)));
    assert_eq!(ptr(e, -1.0, 500.0), Cursor::Cross);
    let (w, h) = (e.shot.size.0 as f32, e.shot.size.1 as f32);
    assert_eq!(ptr(e, w, h - 1.0), Cursor::Cross);
    // The magnifier off: the pointer.
    e.cfg.magnifier = false;
    assert!(!e.crosshair_at(Pt::new(700.0, 500.0)));
    assert_eq!(ptr(e, 700.0, 500.0), Cursor::Cross);
    // The hint card is chrome: the arrow over it, even with the crosshair.
    let mut a = with(None, Vec::new(), None);
    let e = ed(&mut a);
    e.prepare(None, (0, 0), Instant::now() + std::time::Duration::from_millis(300));
    let h = e.scene.as_ref().unwrap().rects(compose::El::Hint)[0];
    let c = Pt::new((h.x0 + h.x1) as f32 / 2.0, (h.y0 + h.y1) as f32 / 2.0);
    assert!(e.crosshair_at(c));
    assert_eq!(e.pointer_at(c, NONE, true), Cursor::Arrow);
    // A selection made, no tool: no crosshair (the toolbar instead).
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], None);
    let e = ed(&mut a);
    for (x, y, want) in [(1000.0, 600.0, Cursor::Move), (1500.0, 900.0, Cursor::Cross), (SEL.x, SEL.y, Cursor::SizeNWSE), (350.0, 301.0, Cursor::Move)] {
        assert_eq!(ptr(e, x, y), want, "({x}, {y})");
    }
    // Drawing tools: no crosshair either.
    e.tool = Some(Tool::Rect);
    assert_eq!(ptr(e, 1000.0, 600.0), Cursor::Cross);
}

/// The Ruler keeps the crosshair over the canvas: hidden there (over an
/// object too, it draws over it), the arrow over the toolbar, a selected
/// object's handles their own cursor.
#[test]
fn pointer_with_the_ruler() {
    let mut a = with(Some(SEL), vec![rect(300.0, 300.0, 200.0, 100.0)], Some(Tool::Ruler));
    let e = ed(&mut a);
    assert_eq!(ptr(e, 1000.0, 600.0), Cursor::Hidden);
    assert_eq!(ptr(e, 350.0, 301.0), Cursor::Hidden, "the ruler draws over objects");
    let tb = e.toolbar.clone().expect("toolbar");
    assert!(e.crosshair_at(mid(tb.bar)));
    assert_eq!(e.pointer_at(mid(tb.bar), NONE, true), Cursor::Arrow);
    for it in &tb.items {
        assert_eq!(e.pointer_at(mid(it.r), NONE, true), Cursor::Arrow, "{:?}", it.kind);
    }
    // A selected object's handle: the resize cursor.
    e.obj.sel = vec![0];
    e.prepare(None, (506, 406), Instant::now());
    assert_eq!(ptr(e, 506.0, 406.0), Cursor::SizeNWSE);
    assert_eq!(ptr(e, 1000.0, 600.0), Cursor::Hidden);
    // A drag with the ruler: hidden while it draws.
    e.obj.sel.clear();
    down(&mut a, 900, 500);
    mv(&mut a, 1000, 560);
    assert!(matches!(ed(&mut a).interact, Interact::Drawing { .. }));
    assert_eq!(ptr(ed(&mut a), 1000.0, 560.0), Cursor::Hidden);
    up(&mut a, 1000, 560);
}

/// Drawing a new region and resizing it keep the crosshair (and no
/// pointer) through the drag; the region drawn: the pointer is back.
#[test]
fn pointer_while_a_region_is_drawn_and_resized() {
    let mut a = with(None, Vec::new(), None);
    down(&mut a, 200, 150);
    mv(&mut a, 600, 400);
    assert!(matches!(ed(&mut a).interact, Interact::NewSel { .. }));
    assert_eq!(ptr(ed(&mut a), 600.0, 400.0), Cursor::Hidden);
    up(&mut a, 600, 400);
    ed(&mut a).prepare(None, (600, 400), Instant::now());
    assert!(!ed(&mut a).crosshair_at(Pt::new(600.0, 400.0)), "a region: the toolbar, no crosshair");
    assert_eq!(ptr(ed(&mut a), 600.0, 400.0), Cursor::SizeNWSE, "its corner handle, at once");
    // Hovering a handle: its cursor; dragging it: the crosshair stands in.
    assert_eq!(ptr(ed(&mut a), 200.0, 150.0), Cursor::SizeNWSE);
    down(&mut a, 200, 150);
    mv(&mut a, 180, 130);
    assert!(matches!(ed(&mut a).interact, Interact::Resize { .. }));
    assert_eq!(ptr(ed(&mut a), 180.0, 130.0), Cursor::Hidden);
    up(&mut a, 180, 130);
    assert_eq!(ptr(ed(&mut a), 180.0, 130.0), Cursor::SizeNWSE);
}

/// The color ring and text suppress the crosshair: the pointer is theirs.
#[test]
fn pointer_where_the_crosshair_is_suppressed() {
    let mut a = with(Some(SEL), Vec::new(), Some(Tool::Ruler));
    a.on_event(Ev::Right { down: true, x: 700, y: 400 });
    assert!(ed(&mut a).assist.ring.is_some());
    assert!(!ed(&mut a).crosshair_at(Pt::new(1000.0, 600.0)));
    assert_eq!(ptr(ed(&mut a), 1000.0, 600.0), Cursor::Arrow);
    let mut a = with(Some(SEL), Vec::new(), Some(Tool::Text));
    click(&mut a, 600, 400);
    assert!(ed(&mut a).text.is_some());
    ed(&mut a).tool = Some(Tool::Ruler);
    assert!(!ed(&mut a).crosshair_at(Pt::new(600.0, 400.0)));
    assert_eq!(ptr(ed(&mut a), 600.0, 400.0), Cursor::IBeam);
}

/// The overlay's one resolver through the driver: hidden over the
/// crosshair while focused; focus lost, then Esc, bring the pointer back.
#[test]
fn pointer_restored_on_focus_loss_and_close() {
    use crate::wind::Driver;
    let mut a = with(None, Vec::new(), None);
    ed(&mut a).assist.dismiss_hint(Instant::now());
    mv(&mut a, 700, 500);
    assert_eq!(a.cursor(), Cursor::Hidden);
    a.on_event(Ev::Focus(false));
    assert_eq!(a.cursor(), Cursor::Cross, "another window has the focus");
    a.on_event(Ev::Focus(true));
    assert_eq!(a.cursor(), Cursor::Hidden);
    // The magnifier turned off: the pointer.
    ed(&mut a).cfg.magnifier = false;
    assert_eq!(a.cursor(), Cursor::Cross);
    ed(&mut a).cfg.magnifier = true;
    a.on_event(Ev::Key { vk: crate::wind::key::ESCAPE, up: false, repeat: false, mods: NONE });
    assert!(!matches!(a.st, State::Edit(_)), "Esc closes the overlay");
    assert_eq!(a.cursor(), Cursor::Arrow, "closed: the pointer");
}
