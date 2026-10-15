//! Editable objects in the overlay: the select tool (picking up with the
//! drawing tools: `cursor_tests`),
//! moving with snapping and the ghost, handles, the marquee, Del / Ctrl+D /
//! front / back / nudges, color and size on the selection, undo; the
//! callout (typing, wrapping, growing, the tail), the spotlight and the
//! magnifier; in-place re-bakes equal to full ones, repaints that cover
//! every changed pixel; and the preview PNGs.

use super::tests::{assert_dirty_covers, edit_of, preview_app, settled};
use super::*;
use crate::objects::{edit as oe, CalloutStyle};
use crate::wind::key;

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
const ALT: Mods = Mods { shift: false, ctrl: false, alt: true };

const SEL: FRect = FRect { x: 100.0, y: 80.0, w: 1200.0, h: 760.0 };
const RED: C4 = C4::rgb(240, 68, 56);

fn app() -> App {
    let mut a = preview_app(theme::DARK, Some(SEL));
    a.focus_tries = u8::MAX;
    a
}

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
fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}
fn key(a: &mut App, vk: u32) {
    key_m(a, vk, NONE);
}
fn type_str(a: &mut App, s: &str) {
    for u in s.encode_utf16() {
        a.on_event(Ev::Char(u));
    }
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

/// An editor with `objs` committed (one undo step each).
fn with(objs: Vec<Obj>) -> App {
    let mut a = app();
    for o in objs {
        ed(&mut a).commit_object(o);
    }
    a
}

fn r_of(o: &Obj) -> FRect {
    oe::sel_box(o, None).unwrap()
}

#[test]
fn select_tool_picks_moves_and_undoes() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0), rect(700.0, 500.0, 100.0, 100.0)]);
    key(&mut a, 'V' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Select));
    // A click on the outline selects it; empty space inside a hollow
    // rectangle does not.
    click(&mut a, 350, 301);
    assert_eq!(ed(&mut a).obj.sel, vec![0]);
    {
        let _m = hold(ALT); // no snapping
        drag(&mut a, (350, 301), (387, 321));
    }
    assert_eq!(r_of(&ed(&mut a).objects[0]), FRect { x: 337.0, y: 320.0, w: 200.0, h: 100.0 });
    assert_eq!(ed(&mut a).hi, 3, "the move is one undo step");
    assert!(ed(&mut a).obj.guides.is_empty() && matches!(ed(&mut a).interact, Interact::None));
    key_m(&mut a, 'Z' as u32, CTRL);
    assert_eq!(r_of(&ed(&mut a).objects[0]), FRect { x: 300.0, y: 300.0, w: 200.0, h: 100.0 });
    assert!(ed(&mut a).obj.sel.is_empty(), "undo drops the selection");
    // A click on nothing deselects.
    click(&mut a, 350, 301);
    click(&mut a, 1000, 200);
    assert!(ed(&mut a).obj.sel.is_empty());
}

#[test]
fn moves_snap_to_edges_and_the_centre_with_guides_and_a_ghost() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0), rect(700.0, 520.0, 100.0, 100.0)]);
    key(&mut a, 'V' as u32);
    click(&mut a, 720, 521);
    assert_eq!(ed(&mut a).obj.sel, vec![1]);
    // Its top 4 px from the first's: snaps (6 px reach).
    down(&mut a, 720, 521);
    mv(&mut a, 730, 400);
    mv(&mut a, 740, 305);
    let e = ed(&mut a);
    let b = r_of(&e.objects[1]);
    assert_eq!(b.y, 300.0, "snapped to the other's top: {b:?}");
    assert_eq!(b.x, 720.0, "x free");
    assert!(e.obj.guides.iter().any(|(p, q)| p.y == 300.0 && q.y == 300.0), "{:?}", e.obj.guides);
    // The 40 % ghost at the origin, no handles while dragging.
    ed(&mut a).prepare(None, (740, 305), Instant::now());
    let sc = ed(&mut a).scene.clone().unwrap();
    assert_eq!(sc.ghost.len(), 1);
    assert_eq!(oe::color(&sc.ghost[0]).unwrap().a, 102);
    assert!(sc.osel.is_none() && sc.mini.is_none());
    // The image centre (the selection's: x 700).
    mv(&mut a, 672, 400);
    let b = r_of(&ed(&mut a).objects[1]);
    assert_eq!(b.x + b.w / 2.0, 700.0, "centre snapped: {b:?}");
    // Alt: no snapping.
    {
        let _m = hold(ALT);
        mv(&mut a, 725, 305);
    }
    assert_eq!(r_of(&ed(&mut a).objects[1]).y, 304.0);
    up(&mut a, 725, 305);
    assert!(ed(&mut a).obj.guides.is_empty());
}

#[test]
fn handles_resize_and_reaim() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0), Obj::Arrow { a: Pt::new(600.0, 600.0), b: Pt::new(800.0, 500.0), color: RED, width: 4.0 }]);
    key(&mut a, 'V' as u32);
    click(&mut a, 400, 301);
    // The SE handle, on the frame 6 px out (half the stroke and 4).
    drag(&mut a, (506, 406), (566, 456));
    assert_eq!(r_of(&ed(&mut a).objects[0]), FRect { x: 300.0, y: 300.0, w: 260.0, h: 150.0 });
    // An arrow: round handles at both ends only.
    click(&mut a, 700, 550);
    assert_eq!(ed(&mut a).obj.sel, vec![1]);
    let (sq, round) = ed(&mut a).obj_handles();
    assert!(sq.is_empty() && round.len() == 2);
    drag(&mut a, (800, 500), (900, 420));
    let Obj::Arrow { a: p, b: q, .. } = ed(&mut a).objects[1] else { panic!() };
    assert_eq!((p, q), (Pt::new(600.0, 600.0), Pt::new(900.0, 420.0)));
    // Drag the shaft: it moves.
    {
        let _m = hold(ALT);
        drag(&mut a, (750, 510), (760, 530));
    }
    let Obj::Arrow { a: p, .. } = ed(&mut a).objects[1] else { panic!() };
    assert_eq!(p, Pt::new(610.0, 620.0));
    assert_eq!(ed(&mut a).hi, 5, "two commits, three edits");
}

#[test]
fn marquee_shift_click_and_group_resize() {
    let mut a = with(vec![rect(300.0, 300.0, 100.0, 100.0), rect(500.0, 300.0, 100.0, 100.0), rect(900.0, 700.0, 50.0, 50.0)]);
    key(&mut a, 'V' as u32);
    drag(&mut a, (250, 250), (650, 350));
    assert_eq!(ed(&mut a).obj.sel, vec![0, 1], "the marquee touches two");
    {
        let _m = hold(SHIFT);
        click(&mut a, 925, 701);
    }
    assert_eq!(ed(&mut a).obj.sel, vec![0, 1, 2], "Shift adds");
    {
        let _m = hold(SHIFT);
        click(&mut a, 350, 301);
    }
    assert_eq!(ed(&mut a).obj.sel, vec![1, 2], "Shift removes");
    // A group gets its four corners only.
    let (sq, round) = ed(&mut a).obj_handles();
    assert_eq!((sq.len(), round.len()), (4, 0));
    // Shift+marquee keeps the selection.
    {
        let _m = hold(SHIFT);
        drag(&mut a, (250, 250), (420, 420));
    }
    assert_eq!(ed(&mut a).obj.sel, vec![0, 1, 2]);
}

#[test]
fn delete_duplicate_restack_and_nudge() {
    let mut a = with(vec![rect(300.0, 300.0, 100.0, 100.0), rect(500.0, 300.0, 100.0, 100.0)]);
    key(&mut a, 'V' as u32);
    click(&mut a, 350, 301);
    // Ctrl+D duplicates while selected (not Save to Desktop).
    key_m(&mut a, 'D' as u32, CTRL);
    let e = ed(&mut a);
    assert!(!e.done && e.tasks.is_empty(), "no export");
    assert_eq!(e.objects.len(), 3);
    assert_eq!(e.obj.sel, vec![2]);
    assert_eq!(r_of(&e.objects[2]), FRect { x: 312.0, y: 312.0, w: 100.0, h: 100.0 });
    // Back, then front.
    key_m(&mut a, key::LBRACKET, CTRL);
    let e = ed(&mut a);
    assert_eq!(e.obj.sel, vec![0]);
    assert_eq!(r_of(&e.objects[0]).x, 312.0);
    key_m(&mut a, key::RBRACKET, CTRL);
    assert_eq!(r_of(&ed(&mut a).objects[2]).x, 312.0);
    // Nudges: 1 px, Shift 10.
    key(&mut a, key::RIGHT);
    key_m(&mut a, key::DOWN, SHIFT);
    assert_eq!(r_of(&ed(&mut a).objects[2]), FRect { x: 313.0, y: 322.0, w: 100.0, h: 100.0 });
    assert_eq!(ed(&mut a).sel, Some(SEL), "the capture's selection stays");
    // Del removes; undo brings it back.
    key(&mut a, key::DELETE);
    assert_eq!(ed(&mut a).objects.len(), 2);
    assert!(ed(&mut a).obj.sel.is_empty());
    key_m(&mut a, 'Z' as u32, CTRL);
    assert_eq!(ed(&mut a).objects.len(), 3);
    // Esc deselects first, then cancels.
    click(&mut a, 550, 301);
    key(&mut a, key::ESCAPE);
    assert!(ed(&mut a).obj.sel.is_empty() && !ed(&mut a).done);
    // Without a selection Ctrl+D saves to the Desktop again.
    let mut b = with(vec![rect(300.0, 300.0, 100.0, 100.0)]);
    let State::Edit(e) = std::mem::replace(&mut b.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    b.handle_key(&mut e, 'D' as u32, false, CTRL);
    assert!(matches!(e.tasks[..], [Task::SaveDesktop]));
}

#[test]
fn color_and_size_go_to_the_selection() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0)]);
    let tool_color = ed(&mut a).mem.color(Tool::Rect);
    key(&mut a, 'V' as u32);
    click(&mut a, 400, 301);
    let blue = C4::rgb(20, 90, 250);
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.apply_act(&mut e, Act::Color(blue));
    a.apply_act(&mut e, Act::Size(1));
    a.apply_act(&mut e, Act::Fill);
    assert_eq!(oe::color(&e.objects[0]), Some(blue));
    assert_eq!(oe::size(&e.objects[0]), Some(5.0));
    assert!(matches!(e.objects[0], Obj::RectFill { .. }), "filled");
    assert_ne!(blue, tool_color);
    assert_eq!(e.mem.color(Tool::Rect), blue, "the tool remembers it");
    assert!(e.mem.fill(Tool::Rect), "and the fill");
    assert_eq!(e.hi, 4, "one step each");
    // The mini-toolbar: color, fill, size, front / back, duplicate, delete.
    e.prepare(None, (0, 0), Instant::now());
    let m = e.obj.mini.clone().expect("mini-toolbar");
    let acts: Vec<Act> = m.items.iter().filter_map(|i| if let toolbar::Kind::Btn(a) = i.kind { Some(a) } else { None }).collect();
    assert_eq!(acts, [Act::ObjColor, Act::ObjFill, Act::ObjSize(-1), Act::ObjSize(1), Act::ObjFront, Act::ObjBack, Act::ObjDup, Act::ObjDelete]);
    assert_eq!(m.r.h, 32.0);
    let fr = e.osel_frame().unwrap();
    assert_eq!(fr.y, 300.0 - 2.5 - 4.0, "the frame clears the stroke by 4 px");
    assert_eq!(m.r.y1(), fr.y - 8.0, "8 px above the selection");
    // Its buttons act through a press.
    let del = m.items.iter().find(|i| i.kind == toolbar::Kind::Btn(Act::ObjDelete)).unwrap().r;
    a.st = State::Edit(Box::new(e));
    click(&mut a, (del.x + del.w / 2.0) as i32, (del.y + del.h / 2.0) as i32);
    assert!(ed(&mut a).objects.is_empty());
}

#[test]
fn callout_typing_wraps_grows_and_is_one_undo_step() {
    let mut a = app();
    if ed(&mut a).font.is_none() {
        return;
    }
    key(&mut a, 'K' as u32);
    drag(&mut a, (300, 200), (520, 240));
    let e = ed(&mut a);
    assert_eq!(e.objects.len(), 1);
    assert!(e.obj.cedit.is_some(), "typing");
    assert_eq!(e.hi, 0, "nothing committed yet");
    let Obj::Callout { r, .. } = e.objects[0].clone() else { panic!() };
    assert_eq!((r.x, r.w), (300.0, 220.0));
    let h0 = r.h;
    type_str(&mut a, "Move this to the top, above the summary, so people see it first");
    let Obj::Callout { r, text, .. } = ed(&mut a).objects[0].clone() else { panic!() };
    assert_eq!(r.w, 220.0, "keeps its width");
    assert!(r.h > h0 + 20.0, "grows down: {h0} -> {}", r.h);
    assert!(text.starts_with("Move"));
    key(&mut a, key::RETURN);
    type_str(&mut a, "x");
    key(&mut a, key::BACK);
    let Obj::Callout { text, .. } = ed(&mut a).objects[0].clone() else { panic!() };
    assert!(text.ends_with("first\n"), "{text:?}");
    // The caret blinks in the frame.
    ed(&mut a).prepare(None, (0, 0), Instant::now());
    assert!(ed(&mut a).scene.as_ref().unwrap().caret.is_some());
    // Keys go to the text: V types, it does not pick the select tool.
    type_str(&mut a, "V");
    key(&mut a, 'V' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Callout));
    key(&mut a, key::ESCAPE);
    let e = ed(&mut a);
    assert!(e.obj.cedit.is_none() && !e.done);
    assert_eq!(e.hi, 1, "one step");
    key_m(&mut a, 'Z' as u32, CTRL);
    assert!(ed(&mut a).objects.is_empty());
    // An empty new bubble is dropped.
    click(&mut a, 600, 600);
    assert_eq!(ed(&mut a).objects.len(), 1);
    click(&mut a, 100, 850);
    assert!(ed(&mut a).objects.is_empty(), "a click outside finishes; empty goes");
}

#[test]
fn callout_tail_handle_and_re_edit() {
    let mut a = app();
    if ed(&mut a).font.is_none() {
        return;
    }
    key(&mut a, 'K' as u32);
    drag(&mut a, (300, 200), (540, 260));
    type_str(&mut a, "Hello");
    key(&mut a, key::ESCAPE);
    key(&mut a, 'V' as u32);
    assert_eq!(ed(&mut a).obj.sel, vec![0], "still selected after typing");
    let (sq, round) = ed(&mut a).obj_handles();
    assert_eq!((sq.len(), round.len()), (8, 1), "8 box handles and the tail");
    let tip = round[0].1;
    drag(&mut a, (tip.x as i32, tip.y as i32), (700, 400));
    let Obj::Callout { tail, .. } = ed(&mut a).objects[0].clone() else { panic!() };
    assert_eq!(tail, Pt::new(700.0, 400.0));
    // A click on the selected callout types in it again (caret at the click).
    click(&mut a, 320, 220);
    assert!(ed(&mut a).obj.cedit.is_some());
    type_str(&mut a, "!");
    key(&mut a, key::ESCAPE);
    let Obj::Callout { text, .. } = ed(&mut a).objects[0].clone() else { panic!() };
    assert!(text.contains('!') && text.len() == 6, "{text:?}");
    // A side handle re-wraps: narrower, taller.
    assert_eq!(ed(&mut a).obj.sel, vec![0]);
    let Obj::Callout { r, .. } = ed(&mut a).objects[0].clone() else { panic!() };
    let e_handle = Pt::new(r.x1(), r.y + r.h / 2.0);
    // Type more first so the narrow bubble must grow.
    let i = 0;
    if let Obj::Callout { text, .. } = &mut ed(&mut a).objects[i] {
        *text = "Several words that need a few lines when narrow".into();
    }
    drag(&mut a, (e_handle.x as i32 + 4, e_handle.y as i32), (r.x as i32 + 94, e_handle.y as i32));
    let Obj::Callout { r: r2, .. } = ed(&mut a).objects[0].clone() else { panic!() };
    assert_eq!(r2.w, 90.0);
    assert!(r2.h > r.h, "grew to hold the re-wrapped text");
}

#[test]
fn callout_styles_from_the_mini_toolbar() {
    let mut a = with(vec![Obj::Callout {
        fmt: Default::default(),
        r: FRect { x: 300.0, y: 200.0, w: 240.0, h: 80.0 },
        tail: Pt::new(320.0, 330.0),
        text: "Hi".into(),
        color: RED,
        size: 17.0,
        style: CalloutStyle::Rounded,
        fill: true,
    }]);
    key(&mut a, 'V' as u32);
    click(&mut a, 400, 240);
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.apply_act(&mut e, Act::ObjStyle(CalloutStyle::Cloud));
    assert!(matches!(e.objects[0], Obj::Callout { style: CalloutStyle::Cloud, .. }));
    assert_eq!(e.obj.style, CalloutStyle::Cloud, "remembered for the next bubble");
    e.prepare(None, (0, 0), Instant::now());
    let m = e.obj.mini.clone().unwrap();
    for st in CalloutStyle::ALL {
        assert!(m.items.iter().any(|i| i.kind == toolbar::Kind::Btn(Act::ObjStyle(st))));
    }
}

/// The spotlight dims the selection outside it (50 % black), keeps it
/// bright inside, unions with others, leaves the outside of the capture
/// selection to the overlay dim, and the export carries it.
#[test]
fn spotlight_dims_everything_else() {
    let mut a = app();
    let plain = settled(&mut a);
    key(&mut a, 'S' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Spotlight));
    drag(&mut a, (400, 200), (600, 400));
    drag(&mut a, (800, 500), (900, 600));
    assert_eq!(ed(&mut a).objects.len(), 2);
    let f = settled(&mut a);
    let px = |img: &PixBuf, x: u32, y: u32| img.get_pixel(x, y);
    for (x, y) in [(500, 300), (850, 550)] {
        assert_eq!(px(&f, x, y), px(&plain, x, y), "inside ({x}, {y}) stays");
    }
    for (x, y) in [(700, 300), (150, 400), (1250, 700)] {
        let (p, q) = (px(&plain, x, y), px(&f, x, y));
        for c in 0..3 {
            assert_eq!(q[c], crate::objects::spot_dim(p[c]), "outside ({x}, {y}) is half");
        }
    }
    // Outside the capture selection: the overlay dim only.
    assert_eq!(px(&f, 50, 40), px(&plain, 50, 40));
    let out = ed(&mut a).export(SEL);
    let base = &ed(&mut a).base;
    let (bx, by) = (700u32, 300u32);
    let o = out.get_pixel(bx - SEL.x as u32, by - SEL.y as u32);
    let b = base.get_pixel(bx, by);
    assert_eq!(o[0], crate::objects::spot_dim(b[0]), "export dimmed");
    let o = out.get_pixel(500 - SEL.x as u32, 300 - SEL.y as u32);
    assert_eq!(o, base.get_pixel(500, 300), "export bright inside");
}

#[test]
fn magnify_shows_twice_the_source() {
    let mut a = app();
    key(&mut a, 'Z' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Magnify));
    // A drag: the source at the press, the loupe at the release.
    drag(&mut a, (380, 170), (700, 300));
    let o = ed(&mut a).objects[0].clone();
    let Obj::Magnify { src, dst, radius, zoom, .. } = o else { panic!("{o:?}") };
    assert_eq!((src, dst, radius, zoom), (Pt::new(380.0, 170.0), Pt::new(700.0, 300.0), 60.0, 2.0));
    let f = settled(&mut a);
    let base = ed(&mut a).base.clone();
    // A loupe pixel 20 px right of its centre shows the source 10 px right.
    assert_eq!(f.get_pixel(720, 300), base.get_pixel(390, 170));
    assert_eq!(f.get_pixel(700, 310), base.get_pixel(380, 175));
    // A click: the loupe up and right of the source.
    click(&mut a, 600, 600);
    let Obj::Magnify { src, dst, .. } = ed(&mut a).objects[1] else { panic!() };
    assert!(dst.x > src.x && dst.y < src.y);
    // Selected: round handles on the source and the loupe.
    key(&mut a, 'V' as u32);
    click(&mut a, 700, 300);
    let (sq, round) = ed(&mut a).obj_handles();
    assert_eq!((sq.len(), round.len()), (0, 2));
    drag(&mut a, (380, 170), (400, 180));
    let Obj::Magnify { src, .. } = ed(&mut a).objects[0] else { panic!() };
    assert_eq!(src, Pt::new(400.0, 180.0), "the source handle moves the source");
}

/// Re-baking in place matches baking from scratch through moves,
/// restyles, restacks, deletes and undos over pixelates and loupes.
#[test]
fn in_place_rebake_matches_a_full_bake() {
    let mut a = with(vec![
        rect(300.0, 300.0, 200.0, 100.0),
        Obj::Pixelate { r: FRect { x: 350.0, y: 250.0, w: 333.0, h: 200.0 }, cell: 11.0 },
        Obj::Arrow { a: Pt::new(200.0, 200.0), b: Pt::new(600.0, 420.0), color: RED, width: 6.0 },
        Obj::Magnify { src: Pt::new(420.0, 330.0), dst: Pt::new(900.0, 300.0), radius: 70.0, zoom: 2.0, color: C4::rgb(255, 255, 255), width: 3.0 },
        Obj::Invert { r: FRect { x: 850.0, y: 200.0, w: 120.0, h: 300.0 } },
        Obj::Spotlight { r: FRect { x: 280.0, y: 280.0, w: 500.0, h: 300.0 } },
    ]);
    let full = |a: &mut App| {
        let e = ed(a);
        // Every object in order, the spotlights' dim (over the
        // selection) after the topmost one.
        let mut img = e.base.clone();
        let pass = crate::objects::SpotPass::of(&e.objects, compose::spot_area(e.sel, e.shot.size));
        let mut sf = crate::raster::Surf::from_buf(&mut img);
        for (j, o) in e.objects.iter().enumerate() {
            o.render_into(&mut sf, e.font.as_ref());
            if let Some(p) = pass.as_ref().filter(|p| p.at == j) {
                p.apply(&mut sf);
            }
        }
        img
    };
    settled(&mut a);
    type Step = Box<dyn Fn(&mut Vec<Obj>)>;
    let steps: Vec<Step> = vec![
        Box::new(|o| oe::translate(&mut o[0], 17.0, -9.0)),
        Box::new(|o| oe::translate(&mut o[2], 5.5, 40.25)),
        Box::new(|o| oe::translate(&mut o[3], -300.0, 20.0)),
        Box::new(|o| oe::set_point(&mut o[3], oe::Point::Src, Pt::new(520.0, 300.0))),
        // Wheel notches on the loupe: its radius alone (the ring band).
        Box::new(|o| oe::set_size(&mut o[3], 140.0)),
        Box::new(|o| oe::set_size(&mut o[3], 61.0)),
        Box::new(|o| oe::set_size(&mut o[3], 68.0)),
        Box::new(|o| oe::translate(&mut o[1], 3.0, 7.0)),
        Box::new(|o| oe::set_color(&mut o[0], C4::rgb(1, 200, 3))),
        Box::new(|o| {
            let x = o.remove(0);
            o.push(x);
        }),
        Box::new(|o| {
            o.remove(2);
        }),
        Box::new(|o| oe::translate(&mut o[4], 50.0, 50.0)),
    ];
    for (i, f) in steps.iter().enumerate() {
        f(&mut ed(&mut a).objects);
        ed(&mut a).dirty = true;
        settled(&mut a);
        let want = full(&mut a);
        let e = ed(&mut a);
        assert!(*e.composed() == want, "step {i}: in-place bake differs");
        assert!(e.baked == e.objects);
    }
}

/// Every pixel an object edit changes is in the dirty rects, and a move
/// repaints about the object, not the screen.
#[test]
fn dirty_rects_cover_object_edits() {
    let mut a = with(vec![
        rect(300.0, 300.0, 200.0, 100.0),
        Obj::Pixelate { r: FRect { x: 350.0, y: 250.0, w: 333.0, h: 200.0 }, cell: 11.0 },
        Obj::Magnify { src: Pt::new(420.0, 330.0), dst: Pt::new(900.0, 300.0), radius: 70.0, zoom: 2.0, color: C4::rgb(255, 255, 255), width: 3.0 },
    ]);
    settled(&mut a);
    let edit = |f: fn(&mut Vec<Obj>)| move |a: &mut App| {
        f(&mut ed(a).objects);
        ed(a).dirty = true;
    };
    let k = assert_dirty_covers(&mut a, "move under pixelate and loupe", edit(|o| oe::translate(&mut o[0], 20.0, 10.0)));
    assert!(k < 0.12, "a move repaints locally: {k}");
    assert_dirty_covers(&mut a, "move loupe", edit(|o| oe::translate(&mut o[2], -40.0, 30.0)));
    assert_dirty_covers(&mut a, "move pixelate", edit(|o| oe::translate(&mut o[1], 4.0, 4.0)));
    // A wheel notch on the loupe (over the pixelate and the rect): the
    // band between its rings, not the box round source and loupe.
    let up = assert_dirty_covers(&mut a, "loupe radius up", edit(|o| oe::set_size(&mut o[2], 160.0)));
    let down = assert_dirty_covers(&mut a, "loupe radius down", edit(|o| oe::set_size(&mut o[2], 143.0)));
    let b = crate::objects::Obj::bounds(&ed(&mut a).objects[2], None).unwrap();
    let (w, h) = ed(&mut a).shot.size;
    let boxk = (b.w * b.h) as f64 / (w as f64 * h as f64);
    assert!(up < boxk * 0.75 && down < boxk * 0.5, "a radius step repaints the ring band: {up} {down} (box {boxk})");
    assert_dirty_covers(&mut a, "first spotlight", edit(|o| o.push(Obj::Spotlight { r: FRect { x: 500.0, y: 500.0, w: 100.0, h: 100.0 } })));
    let k = assert_dirty_covers(&mut a, "move spotlight", edit(|o| oe::translate(&mut o[3], 30.0, 0.0)));
    assert!(k < 0.05, "a spotlight move repaints its rects: {k}");
    assert_dirty_covers(&mut a, "second spotlight", edit(|o| o.push(Obj::Spotlight { r: FRect { x: 900.0, y: 600.0, w: 100.0, h: 100.0 } })));
    assert_dirty_covers(&mut a, "no spotlight", edit(|o| o.retain(|o| !o.is_spot())));
    assert_dirty_covers(&mut a, "delete", edit(|o| {
        o.remove(0);
    }));
    // The chrome: selecting, the mini-toolbar, typing in a callout.
    assert_dirty_covers(&mut a, "select", |a| ed(a).obj.sel = vec![0]);
    if ed(&mut a).font.is_some() {
        assert_dirty_covers(&mut a, "callout", |a| {
            let e = ed(a);
            e.obj.sel.clear();
            e.tool = Some(Tool::Callout);
            let d = e.make_draft(Tool::Callout, Pt::new(300.0, 600.0), Pt::new(500.0, 650.0)).unwrap();
            e.place_callout(d);
        });
        assert_dirty_covers(&mut a, "typing", |a| ed(a).callout_char('W'));
    }
}

/// The overlay's preview PNGs for the object boards: selection chrome
/// (single, arrow, group, drag with guides and ghost), the callout's
/// shapes and its text editing, the spotlight and the magnifier.
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_object_previews() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sel = FRect { x: 240.0, y: 140.0, w: 960.0, h: 600.0 };
    let base = |th: Theme| {
        let mut a = preview_app(th, Some(sel));
        a.focus_tries = u8::MAX;
        a
    };
    let save = |a: &mut App, name: &str| {
        let png = settled(a).to_png().unwrap();
        std::fs::write(dir.join(name), png).unwrap();
    };
    let callout = |x: f32, y: f32, style: CalloutStyle, fill: bool, text: &str| Obj::Callout {
        fmt: Default::default(),
        r: FRect { x, y, w: 260.0, h: 60.0 },
        tail: Pt::new(x + 50.0, y + 150.0),
        text: text.into(),
        color: RED,
        size: 17.0,
        style,
        fill,
    };
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        // Selection chrome: a rectangle with its mini-toolbar.
        let mut a = base(th);
        for o in [rect(420.0, 300.0, 300.0, 160.0), Obj::Arrow { a: Pt::new(500.0, 650.0), b: Pt::new(760.0, 520.0), color: RED, width: 4.0 }] {
            ed(&mut a).commit_object(o);
        }
        ed(&mut a).tool = Some(Tool::Select);
        ed(&mut a).obj.sel = vec![0];
        save(&mut a, &format!("{tn}-object-selected.png"));
        ed(&mut a).obj.sel = vec![1];
        save(&mut a, &format!("{tn}-object-arrow-selected.png"));
        ed(&mut a).obj.sel = vec![0, 1];
        save(&mut a, &format!("{tn}-object-group.png"));
        // A marquee.
        ed(&mut a).obj.sel = vec![];
        down(&mut a, 380, 260);
        mv(&mut a, 600, 500);
        mv(&mut a, 800, 700);
        save(&mut a, &format!("{tn}-object-marquee.png"));
        up(&mut a, 800, 700);
        // Dragging: ghost and guides (snapped to the image centre).
        ed(&mut a).obj.sel = vec![0];
        down(&mut a, 480, 301);
        mv(&mut a, 510, 330);
        mv(&mut a, 482, 361);
        save(&mut a, &format!("{tn}-object-drag.png"));
        up(&mut a, 482, 361);
        // A drawing tool over an object: its pick-up outline.
        ed(&mut a).obj.sel = vec![];
        ed(&mut a).tool = Some(Tool::Rect);
        mv(&mut a, 600, 362);
        assert_eq!(ed(&mut a).obj.hover, Some(0));
        save(&mut a, &format!("{tn}-object-hover.png"));
        mv(&mut a, 630, 586);
        save(&mut a, &format!("{tn}-object-arrow-hover.png"));

        // Callout shapes, filled and outlined, and one being typed in.
        let mut a = base(th);
        for o in [
            callout(300.0, 180.0, CalloutStyle::Rounded, true, "Move this to the top, above the summary"),
            callout(640.0, 180.0, CalloutStyle::Cloud, true, "A cloud: thinking about it"),
            callout(300.0, 470.0, CalloutStyle::Ellipse, true, "An ellipse"),
            callout(640.0, 470.0, CalloutStyle::Rounded, false, "Outlined bubble"),
        ] {
            ed(&mut a).commit_object(o);
            let i = ed(&mut a).objects.len() - 1;
            ed(&mut a).refit_callout(i);
        }
        save(&mut a, &format!("{tn}-callout-styles.png"));
        ed(&mut a).tool = Some(Tool::Select);
        ed(&mut a).obj.sel = vec![0];
        save(&mut a, &format!("{tn}-callout-selected.png"));
        ed(&mut a).begin_callout_edit(0, None, false);
        type_str(&mut a, ", so people see it fir");
        save(&mut a, &format!("{tn}-callout-editing.png"));

        // Spotlight and magnifier.
        let mut a = base(th);
        for o in [
            Obj::Spotlight { r: FRect { x: 360.0, y: 520.0, w: 360.0, h: 260.0 } },
            Obj::Spotlight { r: FRect { x: 1080.0, y: 520.0, w: 230.0, h: 260.0 } },
            Obj::Magnify { src: Pt::new(520.0, 180.0), dst: Pt::new(860.0, 300.0), radius: 90.0, zoom: 2.0, color: C4::rgb(255, 255, 255), width: 3.0 },
        ] {
            ed(&mut a).commit_object(o);
        }
        save(&mut a, &format!("{tn}-spotlight-magnify.png"));
        ed(&mut a).tool = Some(Tool::Select);
        ed(&mut a).obj.sel = vec![2];
        save(&mut a, &format!("{tn}-magnify-selected.png"));

        // Blurs (a weak and a strong one, one selected), the toolbar's
        // Blur slot.
        let mut a = base(th);
        for o in [
            Obj::Blur { r: FRect { x: 300.0, y: 200.0, w: 380.0, h: 200.0 }, sigma: 4.0 },
            Obj::Blur { r: FRect { x: 720.0, y: 200.0, w: 380.0, h: 200.0 }, sigma: 18.0 },
        ] {
            ed(&mut a).commit_object(o);
        }
        ed(&mut a).tool = Some(Tool::Blur);
        save(&mut a, &format!("{tn}-blur.png"));
        ed(&mut a).tool = Some(Tool::Select);
        ed(&mut a).obj.sel = vec![1];
        save(&mut a, &format!("{tn}-blur-selected.png"));
        ed(&mut a).tool = Some(Tool::Invert);
        ed(&mut a).obj.sel = vec![];
        save(&mut a, &format!("{tn}-blur-slot-invert.png"));

        // Text styles: each family, B / I / U / S, aligned; one selected
        // with its mini-toolbar and the font list open; the text draft's
        // bar; a styled callout.
        use crate::text::{Align, Family, TextFmt};
        let mut a = base(th);
        let styles = [
            ("Sans plain", TextFmt::default()),
            ("Serif bold", TextFmt { family: Family::Serif, bold: true, ..Default::default() }),
            ("Mono italic", TextFmt { family: Family::Mono, italic: true, ..Default::default() }),
            ("Underlined", TextFmt { underline: true, ..Default::default() }),
            ("Struck out", TextFmt { strike: true, ..Default::default() }),
            ("Centred\ntwo lines", TextFmt { align: Align::Center, bold: true, ..Default::default() }),
        ];
        for (i, (t, f)) in styles.iter().enumerate() {
            let (x, y) = (300.0 + (i % 3) as f32 * 300.0, 200.0 + (i / 3) as f32 * 150.0);
            ed(&mut a).commit_object(Obj::Text { pos: Pt::new(x, y), text: (*t).into(), color: RED, size: 28.0, fmt: *f });
        }
        let mut c = callout(300.0, 520.0, CalloutStyle::Rounded, true, "A bold italic serif bubble");
        oe::set_fmt(&mut c, TextFmt { family: Family::Serif, bold: true, italic: true, ..Default::default() });
        ed(&mut a).commit_object(c);
        let n = ed(&mut a).objects.len() - 1;
        ed(&mut a).refit_callout(n);
        save(&mut a, &format!("{tn}-text-styles.png"));
        ed(&mut a).tool = Some(Tool::Select);
        ed(&mut a).obj.sel = vec![5];
        save(&mut a, &format!("{tn}-text-selected.png"));
        ed(&mut a).obj.font_menu = true;
        ed(&mut a).obj.font_menu_sel = vec![5];
        save(&mut a, &format!("{tn}-text-font-list.png"));
        ed(&mut a).obj.font_menu = false;
        ed(&mut a).obj.sel = vec![n];
        save(&mut a, &format!("{tn}-callout-styled-selected.png"));
        ed(&mut a).obj.sel = vec![];
        ed(&mut a).tool = Some(Tool::Text);
        ed(&mut a).mem.set_fmt(Tool::Text, TextFmt { underline: true, align: Align::Right, ..Default::default() });
        click(&mut a, 700, 660);
        type_str(&mut a, "Typing");
        key_m(&mut a, key::RETURN, SHIFT);
        type_str(&mut a, "here");
        save(&mut a, &format!("{tn}-text-draft-bar.png"));
    }
}

/// The Blur / Invert button: a click picks Blur; a long press opens a
/// menu with both, and Invert from it picks Invert (the button then shows
/// Invert, its tooltip too); a press outside closes the menu.
#[test]
fn the_blur_button_holds_invert_too() {
    let mut a = app();
    ed(&mut a).prepare(None, (0, 0), Instant::now());
    let btn = |a: &mut App| ed(a).toolbar.as_ref().unwrap().items.iter().find(|i| i.kind == toolbar::Kind::Btn(Act::Tool(Tool::Blur))).unwrap().r;
    let b = btn(&mut a);
    let (bx, by) = ((b.x + b.w / 2.0) as i32, (b.y + b.h / 2.0) as i32);
    click(&mut a, bx, by);
    assert_eq!(ed(&mut a).tool, Some(Tool::Blur), "a click");
    // A long press: the menu.
    down(&mut a, bx, by);
    ed(&mut a).slot_hold = Some(Instant::now() - Duration::from_millis(500));
    ed(&mut a).prepare(None, (0, 0), Instant::now());
    up(&mut a, bx, by);
    assert!(ed(&mut a).slot_menu && ed(&mut a).tool == Some(Tool::Blur), "the release picks nothing");
    ed(&mut a).prepare(None, (0, 0), Instant::now());
    let tb = ed(&mut a).toolbar.clone().unwrap();
    let row = tb.items.iter().find(|i| matches!(i.kind, toolbar::Kind::MenuItem { act: Act::Tool(Tool::Invert), .. })).expect("an Invert row").r;
    assert!(tb.items.iter().any(|i| matches!(i.kind, toolbar::Kind::MenuItem { act: Act::Tool(Tool::Blur), .. })));
    click(&mut a, (row.x + row.w / 2.0) as i32, (row.y + row.h / 2.0) as i32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Invert));
    assert!(!ed(&mut a).slot_menu);
    assert_eq!(toolbar::slot_act(Act::Tool(Tool::Blur), ed(&mut a).tool), Act::Tool(Tool::Invert), "it shows Invert");
    assert_eq!(chrome::act_tip(toolbar::slot_act(Act::Tool(Tool::Blur), Some(Tool::Invert)), &ed(&mut a).keys).0, "Invert");
    // Open again, then a press on the capture only closes it.
    ed(&mut a).slot_menu = true;
    click(&mut a, 700, 500);
    assert!(!ed(&mut a).slot_menu && ed(&mut a).objects.is_empty());
}

/// The text style: the mini-toolbar of a text has the font box, B / I /
/// U / S and the alignment (a callout no alignment); its acts style the
/// selection (one undo step) and the tool remembers them; the font list
/// opens, picks, closes; Ctrl+B / I / U style the draft while typing
/// (Ctrl+U does not upload), and the committed text keeps the style.
#[test]
fn text_styles_from_the_mini_toolbar_and_the_keys() {
    use crate::text::{Align, Family, TextFmt};
    let text = Obj::Text { pos: Pt::new(400.0, 300.0), text: "Hello".into(), color: RED, size: 24.0, fmt: TextFmt::default() };
    let mut a = with(vec![text, Obj::Callout {
        fmt: TextFmt::default(),
        r: FRect { x: 300.0, y: 500.0, w: 240.0, h: 80.0 },
        tail: Pt::new(320.0, 630.0),
        text: "Bubble".into(),
        color: RED,
        size: 17.0,
        style: CalloutStyle::Rounded,
        fill: true,
    }]);
    key(&mut a, 'V' as u32);
    click(&mut a, 410, 310);
    assert_eq!(ed(&mut a).obj.sel, vec![0]);
    ed(&mut a).prepare(None, (0, 0), Instant::now());
    let m = ed(&mut a).obj.mini.clone().unwrap();
    let has = |m: &mini::MiniUi, act: Act| m.items.iter().any(|i| i.kind == toolbar::Kind::Btn(act));
    for act in [Act::ObjFontMenu, Act::ObjBold, Act::ObjItalic, Act::ObjUnderline, Act::ObjStrike, Act::ObjAlign(Align::Center)] {
        assert!(has(&m, act), "{act:?}");
    }
    // A press on B, through the mini-toolbar.
    let b = m.items.iter().find(|i| i.kind == toolbar::Kind::Btn(Act::ObjBold)).unwrap().r;
    let hist = ed(&mut a).hist.len();
    click(&mut a, (b.x + b.w / 2.0) as i32, (b.y + b.h / 2.0) as i32);
    assert!(matches!(&ed(&mut a).objects[0], Obj::Text { fmt, .. } if fmt.bold));
    assert_eq!(ed(&mut a).hist.len(), hist + 1, "one undo step");
    assert!(ed(&mut a).mem.fmt(Tool::Text).bold, "the tool remembers it");
    // The font list: open, pick Mono (it closes).
    let e = ed(&mut a);
    e.fmt_act(Act::ObjFontMenu);
    e.prepare(None, (0, 0), Instant::now());
    let m = e.obj.mini.clone().unwrap();
    assert!(m.pop.is_some() && has(&m, Act::ObjFont(Family::Mono)));
    let row = m.items.iter().find(|i| i.kind == toolbar::Kind::Btn(Act::ObjFont(Family::Mono))).unwrap().r;
    click(&mut a, (row.x + row.w / 2.0) as i32, (row.y + row.h / 2.0) as i32);
    assert!(matches!(&ed(&mut a).objects[0], Obj::Text { fmt, .. } if fmt.family == Family::Mono && fmt.bold));
    assert!(!ed(&mut a).obj.font_menu);
    // Open over the text, then another selection: it closes.
    let e = ed(&mut a);
    e.fmt_act(Act::ObjFontMenu);
    e.obj.sel = vec![1];
    e.prepare(None, (0, 0), Instant::now());
    assert!(!e.obj.font_menu && e.obj.mini.as_ref().is_some_and(|m| m.pop.is_none()), "closed on the next object");
    e.obj.sel = vec![0];
    ed(&mut a).fmt_act(Act::ObjAlign(Align::Right));
    assert!(matches!(&ed(&mut a).objects[0], Obj::Text { fmt, .. } if fmt.align == Align::Right));
    ed(&mut a).undo();
    assert!(matches!(&ed(&mut a).objects[0], Obj::Text { fmt, .. } if fmt.align == Align::Left && fmt.family == Family::Mono));
    // A callout: the style, no alignment buttons.
    let e = ed(&mut a);
    e.obj.sel = vec![1];
    e.prepare(None, (0, 0), Instant::now());
    let m = e.obj.mini.clone().unwrap();
    assert!(has(&m, Act::ObjItalic) && !has(&m, Act::ObjAlign(Align::Left)));
    e.fmt_act(Act::ObjItalic);
    assert!(matches!(&e.objects[1], Obj::Callout { fmt, .. } if fmt.italic));
    assert!(e.mem.fmt(Tool::Callout).italic && !e.mem.fmt(Tool::Text).italic, "each tool its own");
    // Typing: Ctrl+B / I / U style the draft; Ctrl+U is not Upload.
    let mut a = app();
    key(&mut a, 'T' as u32);
    click(&mut a, 500, 400);
    type_str(&mut a, "Hi");
    key_m(&mut a, 'B' as u32, CTRL);
    key_m(&mut a, 'I' as u32, CTRL);
    key_m(&mut a, 'U' as u32, CTRL);
    key_m(&mut a, 'U' as u32, CTRL);
    assert!(ed(&mut a).tasks.is_empty() && !ed(&mut a).done, "no upload");
    assert_eq!(ed(&mut a).text.as_ref().unwrap().text, "Hi", "the keys type nothing");
    ed(&mut a).prepare(None, (0, 0), Instant::now());
    let m = ed(&mut a).obj.mini.clone().expect("the draft's bar");
    assert!(m.inp.draft && has(&m, Act::ObjStrike) && !has(&m, Act::ObjDelete));
    key(&mut a, key::RETURN);
    let want = TextFmt { bold: true, italic: true, ..Default::default() };
    assert!(matches!(&ed(&mut a).objects[..], [Obj::Text { fmt, .. }] if *fmt == want), "{:?}", ed(&mut a).objects);
    // Typing in a callout: Ctrl+B bolds it.
    let mut a = app();
    key(&mut a, 'K' as u32);
    drag(&mut a, (300, 300), (560, 360));
    type_str(&mut a, "Note");
    key_m(&mut a, 'B' as u32, CTRL);
    assert!(matches!(&ed(&mut a).objects[..], [Obj::Callout { fmt, text, .. }] if fmt.bold && text == "Note"));
}

// ---- review fixes ------------------------------------------------------

/// Keys during an object drag: undo, redo, Del, Ctrl+D, restack and the
/// arrows wait for the release (nothing changes under the drag, no panic
/// on the next move); Esc puts the drag back and ends it.
#[test]
fn keys_during_a_drag_wait_or_cancel_it() {
    let arrow = || Obj::Arrow { a: Pt::new(600.0, 600.0), b: Pt::new(800.0, 500.0), color: RED, width: 4.0 };
    let keys: [(&str, u32, Mods); 8] = [
        ("undo", 'Z' as u32, CTRL),
        ("redo", 'Y' as u32, CTRL),
        ("delete", key::DELETE, NONE),
        ("backspace", key::BACK, NONE),
        ("duplicate", 'D' as u32, CTRL),
        ("front", key::RBRACKET, CTRL),
        ("back", key::LBRACKET, CTRL),
        ("nudge", key::LEFT, NONE),
    ];
    for (name, vk, mods) in keys {
        for point in [true, false] {
            let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0), arrow()]);
            key(&mut a, 'V' as u32);
            click(&mut a, 700, 550);
            assert_eq!(ed(&mut a).obj.sel, vec![1]);
            // On the end handle (a point drag) or the shaft (a move).
            let from = if point { (800, 500) } else { (700, 550) };
            down(&mut a, from.0, from.1);
            mv(&mut a, from.0 + 20, from.1 + 10);
            let (n, hi) = (ed(&mut a).objects.len(), ed(&mut a).hi);
            key_m(&mut a, vk, mods);
            assert_eq!((ed(&mut a).objects.len(), ed(&mut a).hi), (n, hi), "{name}: nothing happened");
            mv(&mut a, from.0 + 40, from.1 + 20);
            up(&mut a, from.0 + 40, from.1 + 20);
            let e = ed(&mut a);
            assert_eq!(e.objects.len(), 2, "{name}");
            assert_eq!(e.hi, 3, "{name}: the drag is the one step");
            assert!(matches!(e.objects[1], Obj::Arrow { .. }), "{name}: the arrow is still the arrow");
            // After the release the key works again.
            key_m(&mut a, 'Z' as u32, CTRL);
            assert_eq!(ed(&mut a).objects[1], arrow(), "{name}: undo after the drag");
        }
    }
    // Esc: the drag is undone on the spot and ends; nothing is committed.
    let mut a = with(vec![arrow()]);
    key(&mut a, 'V' as u32);
    click(&mut a, 700, 550);
    down(&mut a, 700, 550);
    mv(&mut a, 760, 590);
    key(&mut a, key::ESCAPE);
    let e = ed(&mut a);
    assert!(matches!(e.interact, Interact::None) && e.objects[0] == arrow() && e.hi == 1 && !e.done);
    mv(&mut a, 800, 600);
    up(&mut a, 800, 600);
    assert_eq!(ed(&mut a).objects[0], arrow(), "later moves do nothing");
    // The wheel waits too.
    click(&mut a, 700, 550);
    down(&mut a, 800, 500);
    mv(&mut a, 820, 510);
    a.on_event(Ev::Wheel { delta: 120, x: 820, y: 510 });
    up(&mut a, 820, 510);
    assert_eq!(oe::size(&ed(&mut a).objects[0]), Some(4.0));
}

/// A drag whose list changes under it (it cannot from the keys now, but
/// whatever else might) ends instead of writing a stale index.
#[test]
fn a_drag_over_a_changed_list_ends() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0), Obj::Arrow { a: Pt::new(600.0, 600.0), b: Pt::new(800.0, 500.0), color: RED, width: 4.0 }]);
    key(&mut a, 'V' as u32);
    click(&mut a, 700, 550);
    down(&mut a, 800, 500);
    mv(&mut a, 810, 505);
    ed(&mut a).objects.truncate(1);
    mv(&mut a, 820, 510);
    up(&mut a, 820, 510);
    assert!(matches!(ed(&mut a).interact, Interact::None));
    assert_eq!(ed(&mut a).objects.len(), 1);
}

/// A bubble just placed takes shape, size, fill and color from the
/// mini-toolbar and the toolbar and stays (typing stays open); the shape
/// is remembered; one undo step when typing ends.
#[test]
fn a_new_callout_survives_restyling() {
    let mut a = app();
    key(&mut a, 'K' as u32);
    click(&mut a, 400, 300);
    assert!(ed(&mut a).obj.cedit.is_some());
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    for act in [Act::ObjStyle(CalloutStyle::Ellipse), Act::ObjSize(1), Act::ObjFill, Act::Size(1), Act::Fill, Act::Color(C4::rgb(1, 2, 3))] {
        a.apply_act(&mut e, act);
        assert_eq!(e.objects.len(), 1, "{act:?}: the bubble stays");
        assert!(e.obj.cedit.is_some(), "{act:?}: still typing");
    }
    assert_eq!(e.obj.style, CalloutStyle::Ellipse);
    let Obj::Callout { style, size, fill, color, .. } = e.objects[0].clone() else { panic!() };
    assert_eq!((style, size, fill, color), (CalloutStyle::Ellipse, 21.0, true, C4::rgb(1, 2, 3)), "17 two steps up");
    assert_eq!(e.hi, 0, "no step while typing");
    a.st = State::Edit(Box::new(e));
    type_str(&mut a, "Hi");
    key(&mut a, key::ESCAPE);
    assert_eq!(ed(&mut a).hi, 1);
    // The next bubble gets the shape.
    click(&mut a, 600, 600);
    assert!(matches!(ed(&mut a).objects[1], Obj::Callout { style: CalloutStyle::Ellipse, .. }));
    // An empty new bubble given only a color goes when typing ends, and
    // leaves no phantom step behind.
    ed(&mut a).color_selection(C4::rgb(9, 9, 9));
    key(&mut a, key::ESCAPE);
    assert_eq!(ed(&mut a).objects.len(), 1);
    assert_eq!(ed(&mut a).hi, 1);
}

/// The color picker and its eyedropper over a selection: the objects
/// show the color as it changes, keeping it is one undo step, Esc puts
/// them back; a kept color is also the tool's next one.
#[test]
fn the_picker_colors_the_selection() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0)]);
    let tool = ed(&mut a).mem.color(Tool::Rect);
    key(&mut a, 'V' as u32);
    click(&mut a, 350, 301);
    let e = ed(&mut a);
    e.open_picker(pick::Target::Tool);
    assert_eq!(e.pick.as_ref().unwrap().color(), RED, "starts at the selection's color");
    e.pick.as_mut().unwrap().set_color(C4::rgb(10, 200, 30));
    e.pick_changed();
    assert_eq!(oe::color(&e.objects[0]), Some(C4::rgb(10, 200, 30)), "live on the object");
    assert_eq!(e.hi, 1, "no step yet");
    e.close_picker(true);
    assert_eq!(oe::color(&e.objects[0]), Some(C4::rgb(10, 200, 30)));
    assert_eq!(e.hi, 2, "one step");
    assert_ne!(tool, C4::rgb(10, 200, 30));
    assert_eq!(e.mem.color(Tool::Rect), C4::rgb(10, 200, 30), "kept: the tool remembers it");
    // Esc puts it back.
    e.open_picker(pick::Target::Tool);
    e.pick.as_mut().unwrap().set_color(C4::rgb(1, 1, 1));
    e.pick_changed();
    e.close_picker(false);
    assert_eq!(oe::color(&e.objects[0]), Some(C4::rgb(10, 200, 30)));
    assert_eq!(e.hi, 2);
    // The eyedropper: a pixel of the capture.
    e.open_picker(pick::Target::Tool);
    let c = e.sample(Pt::new(150.0, 400.0));
    e.pick.as_mut().unwrap().set_color(c);
    e.pick_changed();
    e.close_picker(true);
    assert_eq!(oe::color(&e.objects[0]).map(|c| c.with_alpha(255)), Some(c.with_alpha(255)));
}

/// A spotlight dims what is below it in z-order; objects drawn after it
/// stay bright. The dim follows the selection (moving it repaints and
/// re-bakes the edges), and the export carries it.
#[test]
fn spotlights_dim_only_what_is_below() {
    let below = rect(700.0, 250.0, 100.0, 100.0);
    let above = Obj::RectFill { r: FRect { x: 900.0, y: 250.0, w: 100.0, h: 100.0 }, color: RED, width: 2.0 };
    let mut a = with(vec![below.clone(), Obj::Spotlight { r: FRect { x: 300.0, y: 300.0, w: 200.0, h: 200.0 } }, above.clone()]);
    let f = settled(&mut a);
    let base = ed(&mut a).base.clone();
    let red_dim = crate::objects::spot_dim(240);
    assert_eq!(f.get_pixel(700, 300)[0], red_dim, "the outline below: dimmed");
    assert_eq!(f.get_pixel(950, 300), [240, 68, 56, 255], "the fill above: bright");
    assert_eq!(f.get_pixel(400, 400), base.get_pixel(400, 400), "inside: bright");
    // Restacked under the spotlight: dimmed too.
    ed(&mut a).obj.sel = vec![2];
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.apply_act(&mut e, Act::ObjBack);
    a.st = State::Edit(Box::new(e));
    let f = settled(&mut a);
    assert_eq!(f.get_pixel(950, 300)[0], red_dim);
    // The selection shrinks: the dim follows it (the strip now outside is
    // the overlay dim only), with repaints that cover it.
    let k = assert_dirty_covers(&mut a, "selection with a spotlight", |a| ed(a).sel = Some(FRect { x: 150.0, y: 120.0, w: 1100.0, h: 700.0 }));
    assert!(k < 0.5, "only the edges: {k}");
    let e = ed(&mut a);
    let out = e.export(e.sel.unwrap());
    let p = out.get_pixel(1200 - 150, 300 - 120);
    assert_eq!(p[0], crate::objects::spot_dim(base.get_pixel(1200, 300)[0]), "export dimmed outside the spot");
}

/// Hostile documents: non-finite numbers are refused, huge and negative
/// sizes clamped, rects normalised, counts and lengths bounded; what is
/// read renders quickly.
#[test]
fn hostile_annotations_are_refused_or_tamed() {
    use crate::objects::from_json;
    let doc = |o: &str| format!("{{\"version\":1,\"objects\":[{o}]}}");
    for bad in [
        "{\"type\":\"line\",\"a\":[1e400,0],\"b\":[1,2],\"color\":\"#000\",\"width\":1}",
        "{\"type\":\"magnify\",\"src\":[0,0],\"dst\":[1,1],\"radius\":1e39,\"zoom\":2,\"color\":\"#fff\",\"width\":3}",
        "{\"type\":\"number\",\"c\":[0,0],\"n\":1e400,\"color\":\"#000\",\"d\":20}",
        "{\"type\":\"text\",\"pos\":[0,-1e300],\"text\":\"x\",\"color\":\"#000\",\"size\":10}",
    ] {
        assert!(from_json(&doc(bad)).is_err(), "{bad}");
    }
    let tame = from_json(&doc(
        "{\"type\":\"magnify\",\"src\":[1e9,-1e9],\"dst\":[10,10],\"radius\":1e6,\"zoom\":1000,\"color\":\"#fff\",\"width\":-5},\
         {\"type\":\"callout\",\"r\":[0,0,1e9,100],\"tail\":[5,500],\"text\":\"hi\",\"color\":\"#f00\",\"size\":-3,\"style\":\"cloud\",\"fill\":true},\
         {\"type\":\"rect\",\"r\":[100,100,-50,-20],\"color\":\"#000\",\"width\":1e9},\
         {\"type\":\"number\",\"c\":[1,1],\"n\":-5,\"color\":\"#000\",\"d\":0}",
    ))
    .unwrap();
    let Obj::Magnify { src, radius, zoom, width, .. } = tame[0] else { panic!() };
    assert_eq!((src, radius, zoom, width), (Pt::new(1e6, -1e6), 300.0, 8.0, 1.0));
    let Obj::Callout { r, size, .. } = tame[1] else { panic!() };
    assert_eq!((r.w, size), (1e6, 8.0));
    let Obj::Rect { r, width, .. } = tame[2] else { panic!() };
    assert_eq!((r, width), (FRect { x: 50.0, y: 80.0, w: 50.0, h: 20.0 }, 50.0));
    assert!(matches!(tame[3], Obj::Counter { n: 0, d: 12.0, .. }));
    // They render (and fast) into a small surface.
    let t0 = Instant::now();
    let mut img = PixBuf::new(200, 200);
    let font = AnnotFont::load();
    for o in &tame {
        o.render(&mut img, font.as_ref());
        let _ = oe::hit(o, Pt::new(10.0, 10.0), 4.0, font.as_ref());
    }
    assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    // Too many objects or points, too long a text; deep nesting; cut short.
    let many = vec!["{\"type\":\"invert\",\"r\":[0,0,1,1]}"; 10_001].join(",");
    assert!(from_json(&doc(&many)).is_err());
    let pts = vec!["[1,1]"; 100_001].join(",");
    assert!(from_json(&doc(&format!("{{\"type\":\"pencil\",\"pts\":[{pts}],\"color\":\"#000\",\"width\":1}}"))).is_err());
    let long = "x".repeat(100_001);
    assert!(from_json(&doc(&format!("{{\"type\":\"text\",\"pos\":[0,0],\"text\":\"{long}\",\"color\":\"#000\",\"size\":10}}"))).is_err());
    assert!(from_json(&format!("{}{}", "[".repeat(10_000), "]".repeat(10_000))).is_err());
    let full = doc("{\"type\":\"invert\",\"r\":[0,0,1,1]}");
    for cut in 1..full.len() {
        assert!(from_json(&full[..cut]).is_err(), "cut at {cut}");
    }
    assert!(from_json("{\"version\":\"1\",\"objects\":[]}").unwrap_err().to_string().contains("not a number"));
    // A high surrogate without its low half: U+FFFD, then what follows.
    let v = from_json(&doc("{\"type\":\"text\",\"pos\":[0,0],\"text\":\"\\ud83dA\\ud83d\\u0041\",\"color\":\"#000\",\"size\":10}")).unwrap();
    assert!(matches!(&v[0], Obj::Text { text, .. } if text == "\u{FFFD}A\u{FFFD}A"), "{v:?}");
}

/// Emoji typed into a callout arrive as UTF-16 pairs and are kept.
#[test]
fn emoji_reach_the_callout() {
    let mut a = app();
    key(&mut a, 'K' as u32);
    click(&mut a, 400, 300);
    type_str(&mut a, "ok \u{1F600}!");
    let Obj::Callout { text, .. } = &ed(&mut a).objects[0] else { panic!() };
    assert_eq!(text, "ok \u{1F600}!");
}

/// Nudges in a quick run, and wheel ticks, are one undo step.
#[test]
fn nudges_and_wheel_ticks_coalesce() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0)]);
    key(&mut a, 'V' as u32);
    click(&mut a, 350, 301);
    for _ in 0..5 {
        key(&mut a, key::RIGHT);
    }
    assert_eq!(r_of(&ed(&mut a).objects[0]).x, 305.0);
    assert_eq!(ed(&mut a).hi, 2, "one step for the run");
    for _ in 0..3 {
        a.on_event(Ev::Wheel { delta: 120, x: 350, y: 301 });
    }
    assert_eq!(oe::size(&ed(&mut a).objects[0]), Some(7.0));
    assert_eq!(ed(&mut a).hi, 3);
    key_m(&mut a, 'Z' as u32, CTRL);
    assert_eq!(oe::size(&ed(&mut a).objects[0]), Some(4.0), "the ticks undo together");
    assert_eq!(r_of(&ed(&mut a).objects[0]).x, 305.0);
}

/// A small object (a 12 px tag, a 16 px line) still moves by its middle;
/// its ends stay reachable.
#[test]
fn small_objects_move_by_their_middle() {
    let mut a = with(vec![
        Obj::Counter { c: Pt::new(400.0, 400.0), n: 1, color: RED, d: 12.0, tail: None },
        Obj::Line { a: Pt::new(600.0, 600.0), b: Pt::new(616.0, 600.0), color: RED, width: 3.0 },
    ]);
    key(&mut a, 'V' as u32);
    click(&mut a, 400, 400);
    {
        let _m = hold(ALT);
        drag(&mut a, (400, 400), (430, 410));
    }
    assert!(matches!(ed(&mut a).objects[0], Obj::Counter { c, .. } if c == Pt::new(430.0, 410.0)), "moved");
    click(&mut a, 608, 600);
    {
        let _m = hold(ALT);
        drag(&mut a, (608, 600), (618, 620));
    }
    assert!(matches!(ed(&mut a).objects[1], Obj::Line { a: p, .. } if p == Pt::new(610.0, 620.0)), "moved");
    drag(&mut a, (626, 620), (700, 640));
    assert!(matches!(ed(&mut a).objects[1], Obj::Line { b, .. } if b == Pt::new(700.0, 640.0)), "the end re-aims");
}

/// Undo and redo through every kind of object edit, interleaved.
#[test]
fn every_edit_undoes_and_redoes() {
    let start = vec![
        rect(300.0, 300.0, 200.0, 100.0),
        Obj::Arrow { a: Pt::new(600.0, 600.0), b: Pt::new(800.0, 500.0), color: RED, width: 4.0 },
        Obj::Callout { fmt: Default::default(), r: FRect { x: 900.0, y: 200.0, w: 200.0, h: 80.0 }, tail: Pt::new(920.0, 330.0), text: "x".into(), color: RED, size: 17.0, style: CalloutStyle::Rounded, fill: true },
    ];
    let mut a = with(start.clone());
    key(&mut a, 'V' as u32);
    let mut states = vec![ed(&mut a).objects.clone()];
    let mut step = |a: &mut App, f: &dyn Fn(&mut App)| {
        f(a);
        states.push(ed(a).objects.clone());
    };
    let act = |x: Act| move |a: &mut App| {
        let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
        let mut e = *e;
        a.apply_act(&mut e, x);
        a.st = State::Edit(Box::new(e));
    };
    step(&mut a, &|a| {
        click(a, 350, 301);
        drag(a, (506, 406), (566, 456));
    });
    step(&mut a, &|a| {
        click(a, 700, 550);
        drag(a, (800, 500), (850, 450));
    });
    step(&mut a, &|a| {
        click(a, 350, 301);
        act(Act::ObjFront)(a)
    });
    step(&mut a, &act(Act::ObjDup));
    step(&mut a, &act(Act::ObjFill));
    step(&mut a, &|a| {
        click(a, 1000, 220);
        act(Act::ObjStyle(CalloutStyle::Cloud))(a)
    });
    let n = states.len();
    let hist = ed(&mut a).hi;
    for i in (0..n - 1).rev() {
        key_m(&mut a, 'Z' as u32, CTRL);
        assert_eq!(ed(&mut a).objects, states[i], "undo to {i}");
    }
    for (i, s) in states.iter().enumerate().skip(1) {
        key_m(&mut a, 'Y' as u32, CTRL);
        assert_eq!(&ed(&mut a).objects, s, "redo to {i}");
    }
    assert_eq!(ed(&mut a).hi, hist);
}

/// The snap guides fade out over 100 ms after the drag (frames are due
/// meanwhile), then go.
#[test]
fn guides_fade_out() {
    let mut a = with(vec![rect(300.0, 300.0, 200.0, 100.0), rect(700.0, 520.0, 100.0, 100.0)]);
    key(&mut a, 'V' as u32);
    click(&mut a, 720, 521);
    down(&mut a, 720, 521);
    mv(&mut a, 730, 400);
    mv(&mut a, 740, 305);
    up(&mut a, 740, 305);
    let now = Instant::now();
    let e = ed(&mut a);
    let (g, k) = e.guides_now(now);
    assert!(!g.is_empty() && k > 0.5 && super::animating(e, None, now));
    let (g, _) = e.guides_now(now + Duration::from_millis(150));
    assert!(g.is_empty() && !e.guides_fading(now + Duration::from_millis(150)));
}

/// What a callout is given while selected or typed in (color, size,
/// fill, shape) is what the next callout gets, here and in the config.
#[test]
fn a_callouts_style_is_remembered() {
    let mut a = app();
    key(&mut a, 'K' as u32);
    click(&mut a, 400, 300);
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    let fill0 = e.mem.fill(Tool::Callout);
    for act in [Act::ObjStyle(CalloutStyle::Cloud), Act::ObjSize(1), Act::ObjFill] {
        a.apply_act(&mut e, act);
    }
    e.color_selection(C4::rgb(10, 20, 30));
    assert_eq!(e.mem.color(Tool::Callout), C4::rgb(10, 20, 30));
    assert_eq!(e.mem.size(Tool::Callout), 19.0, "17 a step up");
    assert_eq!(e.mem.fill(Tool::Callout), !fill0);
    assert_eq!(e.cfg.callout_style, "cloud");
    assert_eq!(e.cfg.tool_colors.get("callout").map(String::as_str), Some("#0a141e"));
    let keys: Vec<String> = e.pending.take().into_iter().map(|(k, _)| k).collect();
    for k in ["tool_callout_style", "tool_callout_size", "tool_callout_color", "tool_callout_fill"] {
        assert!(keys.iter().any(|x| x == k), "{k} queued: {keys:?}");
    }
    a.st = State::Edit(Box::new(e));
    type_str(&mut a, "Hi");
    key(&mut a, key::ESCAPE);
    click(&mut a, 700, 600);
    let Obj::Callout { style, size, fill, color, .. } = ed(&mut a).objects[1].clone() else { panic!() };
    assert_eq!((style, size, fill, color), (CalloutStyle::Cloud, 19.0, !fill0, C4::rgb(10, 20, 30)));
    // A new overlay starts from the config.
    let mut cfg = ed(&mut a).cfg.clone();
    cfg.callout_style = "ellipse".into();
    let mut b = app();
    ed(&mut b).obj.style = crate::objects::CalloutStyle::from_id(&cfg.callout_style).unwrap();
    assert_eq!(ed(&mut b).obj.style, CalloutStyle::Ellipse);
}

/// A text size change refits the bubble away from its tail, and resizing
/// the bubble by a handle leaves the tail's tip where it points.
#[test]
fn callout_size_and_resize_keep_the_tail() {
    let tip = Pt::new(420.0, 520.0);
    let r0 = FRect { x: 300.0, y: 300.0, w: 260.0, h: 60.0 };
    let c = Obj::Callout { fmt: Default::default(), r: r0, tail: tip, text: "Two words and then some more so it wraps".into(), color: RED, size: 17.0, style: CalloutStyle::Rounded, fill: true };
    let mut a = with(vec![c.clone()]);
    ed(&mut a).tool = Some(Tool::Select);
    ed(&mut a).obj.sel = vec![0];
    for d in [4, 4, -6] {
        let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
        let mut e = *e;
        a.apply_act(&mut e, Act::ObjSize(d));
        let Obj::Callout { r, tail, .. } = e.objects[0].clone() else { panic!() };
        assert_eq!(tail, tip, "size {d}: the tip stays");
        assert!((r.y1() - r0.y1()).abs() < 0.01, "size {d}: the edge by the tail stays ({r:?})");
        a.st = State::Edit(Box::new(e));
    }
    // Resize: the bubble scales, the tip does not.
    let mut o = c;
    oe::map_box(&mut o, r0, FRect { x: 250.0, y: 250.0, w: 400.0, h: 120.0 });
    let Obj::Callout { r, tail, .. } = o else { panic!() };
    assert_eq!(tail, tip);
    assert_eq!(r, FRect { x: 250.0, y: 250.0, w: 400.0, h: 120.0 });
}
