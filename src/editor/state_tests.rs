//! The editor state machine driven the way the window drives it: synthetic
//! `wind::Ev` input through `Driver::on_event` (selection, every tool,
//! text editing, history, palette and sizes, toasts, accept/cancel, the
//! upload result and `begin_capture` with a queued test shot). Nothing
//! touches the screen, the clipboard or the network: `grab` returns the
//! queued shot, exports go to `Task::Geometry` or a temp folder.

use super::tests::{edit_of, preview_app, preview_app_with, synthetic_shot, two_monitor_shot, upload_in_flight};
use super::*;
use crate::wind::key;

fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
const CTRL_SHIFT: Mods = Mods { shift: true, ctrl: true, alt: false };

/// The preview editor (1440x900 shot, one monitor), never stealing focus
/// and never copying an uploaded URL to the clipboard.
fn app(sel: Option<FRect>) -> App {
    quiet(preview_app(theme::DARK, sel))
}

fn quiet(mut a: App) -> App {
    a.focus_tries = u8::MAX;
    a.cfg.copy_url_after_upload = false;
    a
}

/// One second before now, for wheel-throttle stamps (no underflow panic
/// on a clock that started less than a second ago).
fn a_second_ago() -> Instant {
    let now = Instant::now();
    now.checked_sub(Duration::from_secs(1)).unwrap_or(now)
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
/// Press, move halfway, move to `to`, release.
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
fn repeat(a: &mut App, vk: u32) {
    a.on_event(Ev::Key { vk, up: false, repeat: true, mods: NONE });
}
fn type_str(a: &mut App, s: &str) {
    for u in s.encode_utf16() {
        a.on_event(Ev::Char(u));
    }
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

fn hidden(a: &App) -> bool {
    matches!(a.st, State::Hidden)
}

fn code(a: &App) -> i32 {
    a.exit_code.load(Ordering::SeqCst)
}

/// A fresh temp folder for exports (removed on drop).
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("rustshot-state-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- selection --------------------------------------------------------------

#[test]
fn drag_creates_a_selection_and_a_click_selects_everything() {
    let mut a = app(None);
    drag(&mut a, (100, 100), (300, 250));
    assert_eq!(ed(&mut a).sel, Some(r(100.0, 100.0, 200.0, 150.0)));
    assert!(matches!(ed(&mut a).interact, Interact::None));

    // Dragging up-left from the anchor normalises the rect.
    let mut a = app(None);
    drag(&mut a, (300, 250), (100, 100));
    assert_eq!(ed(&mut a).sel, Some(r(100.0, 100.0, 200.0, 150.0)));

    // A click (or a move under CLICK_PX) selects the whole shot.
    for wiggle in [0, 1] {
        let mut a = app(None);
        down(&mut a, 500, 500);
        mv(&mut a, 500 + wiggle, 500);
        up(&mut a, 500 + wiggle, 500);
        assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 1440.0, 900.0)), "wiggle {wiggle}");
    }
}

#[test]
fn shift_drag_makes_a_square_selection() {
    let mut a = app(None);
    let _m = hold(SHIFT);
    drag(&mut a, (100, 100), (300, 160));
    assert_eq!(ed(&mut a).sel, Some(r(100.0, 100.0, 200.0, 200.0)));
}

#[test]
fn new_selection_is_clamped_to_the_shot() {
    let mut a = app(None);
    drag(&mut a, (1000, 600), (5000, 4000));
    let s = ed(&mut a).sel.unwrap();
    assert!(s.x >= 0.0 && s.y >= 0.0 && s.x1() <= 1440.0 && s.y1() <= 900.0, "{s:?}");
    let mut a = app(None);
    drag(&mut a, (100, 100), (-300, -300));
    assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 100.0, 100.0)), "stops at the edge, never past the anchor");
    let mut a = app(None);
    drag(&mut a, (1000, 600), (5000, 700));
    assert_eq!(ed(&mut a).sel, Some(r(1000.0, 600.0, 440.0, 100.0)));
}

#[test]
fn shift_drag_past_the_shot_keeps_the_anchor_and_stays_square_inside() {
    // (anchor, pointer far past the shot, expected square). 1440x900 shot.
    for (from, to, want) in [
        ((1000, 600), (5000, 4000), r(1000.0, 600.0, 300.0, 300.0)), // bottom-right corner
        ((100, 200), (-500, -300), r(0.0, 100.0, 100.0, 100.0)),      // top-left corner
        ((1300, 100), (3000, -50), r(1300.0, 0.0, 100.0, 100.0)),     // top-right corner
        ((200, 800), (-400, 1200), r(100.0, 800.0, 100.0, 100.0)),    // bottom-left corner
        ((1000, 600), (5000, 700), r(1000.0, 600.0, 300.0, 300.0)),   // right edge
        ((400, 500), (300, -2000), r(0.0, 100.0, 400.0, 400.0)),       // top edge
        ((400, 300), (-900, 350), r(0.0, 300.0, 400.0, 400.0)),       // left edge
        ((700, 400), (750, 3000), r(700.0, 400.0, 500.0, 500.0)),     // bottom edge
    ] {
        let mut a = app(None);
        let _m = hold(SHIFT);
        drag(&mut a, from, to);
        let s = ed(&mut a).sel.unwrap();
        assert_eq!(s, want, "{from:?} -> {to:?}");
        assert_eq!(s.w, s.h, "square {from:?} -> {to:?}");
        assert!(s.x >= 0.0 && s.y >= 0.0 && s.x1() <= 1440.0 && s.y1() <= 900.0, "inside {s:?}");
        let corners = [(s.x, s.y), (s.x1(), s.y), (s.x, s.y1()), (s.x1(), s.y1())];
        assert!(corners.contains(&(from.0 as f32, from.1 as f32)), "anchor fixed {from:?} {s:?}");
    }
}

#[test]
fn dragging_inside_moves_the_selection_and_clamps_it() {
    let mut a = app(Some(r(100.0, 100.0, 200.0, 100.0)));
    drag(&mut a, (150, 150), (250, 200));
    assert_eq!(ed(&mut a).sel, Some(r(200.0, 150.0, 200.0, 100.0)));
    drag(&mut a, (250, 200), (-1000, -1000));
    assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 200.0, 100.0)), "clamped top-left");
    drag(&mut a, (100, 50), (5000, 5000));
    assert_eq!(ed(&mut a).sel, Some(r(1240.0, 800.0, 200.0, 100.0)), "clamped bottom-right");
}

#[test]
fn every_handle_resizes_its_edges() {
    let sel = r(400.0, 300.0, 200.0, 100.0);
    // (handle point, drag target, expected rect): each pulled 20 px outward.
    let cases = [
        ("NW", (400, 300), (380, 280), r(380.0, 280.0, 220.0, 120.0)),
        ("N", (500, 300), (500, 280), r(400.0, 280.0, 200.0, 120.0)),
        ("NE", (600, 300), (620, 280), r(400.0, 280.0, 220.0, 120.0)),
        ("E", (600, 350), (620, 350), r(400.0, 300.0, 220.0, 100.0)),
        ("SE", (600, 400), (620, 420), r(400.0, 300.0, 220.0, 120.0)),
        ("S", (500, 400), (500, 420), r(400.0, 300.0, 200.0, 120.0)),
        ("SW", (400, 400), (380, 420), r(380.0, 300.0, 220.0, 120.0)),
        ("W", (400, 350), (380, 350), r(380.0, 300.0, 220.0, 100.0)),
    ];
    for (name, at, to, want) in cases {
        let mut a = app(Some(sel));
        down(&mut a, at.0, at.1);
        assert!(matches!(ed(&mut a).interact, Interact::Resize { .. }), "{name}: grabbed");
        mv(&mut a, to.0, to.1);
        up(&mut a, to.0, to.1);
        assert_eq!(ed(&mut a).sel, Some(want), "{name}");
    }
}

#[test]
fn resize_modifiers_flip_and_clamp() {
    let sel = r(400.0, 300.0, 200.0, 100.0);
    // Shift: symmetric about the centre.
    let mut a = app(Some(sel));
    {
        let _m = hold(SHIFT);
        drag(&mut a, (600, 400), (620, 420));
    }
    assert_eq!(ed(&mut a).sel, Some(r(380.0, 280.0, 240.0, 140.0)));
    // Ctrl: keeps the 2:1 aspect, centred on the dragged box.
    let mut a = app(Some(sel));
    {
        let _m = hold(CTRL);
        drag(&mut a, (600, 400), (620, 500));
    }
    assert_eq!(ed(&mut a).sel, Some(r(400.0, 345.0, 220.0, 110.0)));
    // Past the opposite edge: the rect flips.
    let mut a = app(Some(sel));
    drag(&mut a, (600, 350), (300, 350));
    assert_eq!(ed(&mut a).sel, Some(r(300.0, 300.0, 100.0, 100.0)));
    // Off the shot: moved back inside, size kept.
    let mut a = app(Some(sel));
    drag(&mut a, (400, 300), (-50, -50));
    assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 650.0, 450.0)));
}

#[test]
fn selection_spans_and_clamps_across_monitors() {
    // 2400x900: left monitor (0,200) 1200x700, right (1200,0) 1200x900.
    let mut a = quiet(preview_app_with(theme::DARK, None, two_monitor_shot()));
    drag(&mut a, (1000, 300), (1300, 400));
    assert_eq!(ed(&mut a).sel, Some(r(1000.0, 300.0, 300.0, 100.0)), "spans both monitors");
    drag(&mut a, (1100, 350), (5000, 350));
    assert_eq!(ed(&mut a).sel, Some(r(2100.0, 300.0, 300.0, 100.0)), "clamped to the whole shot");
    // Chrome follows the monitor with the larger overlap.
    let now = Instant::now();
    ed(&mut a).prepare(None, (0, 0), now);
    assert_eq!(ed(&mut a).area_drawn, Some(r(1200.0, 0.0, 1200.0, 900.0)));

    // No selection: the monitor under the pointer.
    let mut a = quiet(preview_app_with(theme::DARK, None, two_monitor_shot()));
    ed(&mut a).prepare(None, (100, 500), now);
    assert_eq!(ed(&mut a).area_drawn, Some(r(0.0, 200.0, 1200.0, 700.0)));
    mv(&mut a, 1500, 100); // area moved: a repaint is asked, nothing else
    let m = a.mouse;
    ed(&mut a).prepare(None, m, now);
    assert_eq!(ed(&mut a).area_drawn, Some(r(1200.0, 0.0, 1200.0, 900.0)));
}

#[test]
fn arrow_keys_nudge_and_grow_the_selection() {
    let mut a = app(Some(r(100.0, 100.0, 50.0, 40.0)));
    for (vk, want) in [
        (key::LEFT, r(99.0, 100.0, 50.0, 40.0)),
        (key::RIGHT, r(100.0, 100.0, 50.0, 40.0)),
        (key::UP, r(100.0, 99.0, 50.0, 40.0)),
        (key::DOWN, r(100.0, 100.0, 50.0, 40.0)),
    ] {
        key(&mut a, vk);
        assert_eq!(ed(&mut a).sel, Some(want), "nudge {vk:#x}");
    }
    for (vk, want) in [
        (key::LEFT, r(99.0, 100.0, 51.0, 40.0)),
        (key::RIGHT, r(99.0, 100.0, 52.0, 40.0)),
        (key::UP, r(99.0, 99.0, 52.0, 41.0)),
        (key::DOWN, r(99.0, 99.0, 52.0, 42.0)),
    ] {
        key_m(&mut a, vk, SHIFT);
        assert_eq!(ed(&mut a).sel, Some(want), "grow {vk:#x}");
    }
    // Arrows auto-repeat; Ctrl+arrow is not a nudge.
    repeat(&mut a, key::RIGHT);
    assert_eq!(ed(&mut a).sel.unwrap().x, 100.0);
    key_m(&mut a, key::RIGHT, CTRL);
    assert_eq!(ed(&mut a).sel.unwrap().x, 100.0);
    // Clamped at the shot edge.
    let mut a = app(Some(r(0.0, 0.0, 50.0, 40.0)));
    key(&mut a, key::LEFT);
    key(&mut a, key::UP);
    assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 50.0, 40.0)));
    // HiDPI: one step is one logical pixel.
    let mut a = app(Some(r(100.0, 100.0, 50.0, 40.0)));
    ed(&mut a).shot.scale = 2.0;
    key(&mut a, key::RIGHT);
    assert_eq!(ed(&mut a).sel.unwrap().x, 102.0);
    // No selection: arrows do nothing.
    let mut a = app(None);
    key(&mut a, key::LEFT);
    assert_eq!(ed(&mut a).sel, None);
}

#[test]
fn select_all_and_a_second_press_while_dragging() {
    let mut a = app(Some(r(10.0, 10.0, 20.0, 20.0)));
    down(&mut a, 15, 15); // grabs the NW handle
    key_m(&mut a, 'A' as u32, CTRL);
    assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 1440.0, 900.0)));
    assert!(matches!(ed(&mut a).interact, Interact::None), "select-all ends the drag");
    // A press while a drag runs is ignored.
    let mut a = app(None);
    down(&mut a, 100, 100);
    down(&mut a, 400, 400);
    let Interact::NewSel { anchor, .. } = ed(&mut a).interact else { panic!("still the first drag") };
    assert_eq!(anchor, Pt::new(100.0, 100.0));
}

#[test]
fn cursor_follows_handles_selection_and_tool() {
    let a = app(None);
    assert_eq!(a.cursor(), Cursor::Cross, "no selection");
    let mut a = app(Some(r(400.0, 300.0, 200.0, 100.0)));
    for ((x, y), want) in [
        ((400, 300), Cursor::SizeNWSE),
        ((600, 400), Cursor::SizeNWSE),
        ((600, 300), Cursor::SizeNESW),
        ((400, 400), Cursor::SizeNESW),
        ((500, 300), Cursor::SizeNS),
        ((500, 400), Cursor::SizeNS),
        ((400, 350), Cursor::SizeWE),
        ((600, 350), Cursor::SizeWE),
        ((500, 350), Cursor::Move),
        ((50, 50), Cursor::Cross),
    ] {
        mv(&mut a, x, y);
        assert_eq!(a.cursor(), want, "at {x},{y}");
    }
    key(&mut a, 'R' as u32);
    assert_eq!(a.cursor(), Cursor::Cross, "a tool");
    key(&mut a, 'T' as u32);
    click(&mut a, 450, 320);
    assert_eq!(a.cursor(), Cursor::IBeam, "text draft");
    a.st = State::Hidden;
    assert_eq!(a.cursor(), Cursor::Arrow, "nothing open");
}

// ---- drawing ---------------------------------------------------------------

#[test]
fn every_tool_drafts_then_commits_on_release() {
    let cases: [(char, Tool); 8] = [
        ('P', Tool::Path),
        ('L', Tool::Line),
        ('A', Tool::Arrow),
        ('R', Tool::Rect),
        ('E', Tool::Ellipse),
        ('M', Tool::Marker),
        ('B', Tool::Pixelate),
        ('I', Tool::Invert),
    ];
    for (k, tool) in cases {
        let mut a = app(Some(r(100.0, 100.0, 800.0, 600.0)));
        key(&mut a, k as u32);
        assert_eq!(ed(&mut a).tool, Some(tool), "{k}");
        down(&mut a, 200, 200);
        mv(&mut a, 250, 230);
        mv(&mut a, 300, 260);
        let draft = ed(&mut a).draft.clone().unwrap_or_else(|| panic!("{tool:?}: draft while dragging"));
        assert!(ed(&mut a).objects.is_empty(), "{tool:?}: nothing committed yet");
        up(&mut a, 300, 260);
        let e = ed(&mut a);
        assert_eq!(e.objects, vec![draft.clone()], "{tool:?}: the draft is committed");
        assert!(e.draft.is_none() && e.stroke_pts.is_empty());
        let ok = match (tool, &draft) {
            (Tool::Path, Obj::Path { pts, .. }) => pts.len() == 3,
            (Tool::Line, Obj::Line { a, b, .. }) | (Tool::Arrow, Obj::Arrow { a, b, .. }) => {
                *a == Pt::new(200.0, 200.0) && *b == Pt::new(300.0, 260.0)
            }
            // Freehand: every point (Shift draws it straight).
            (Tool::Marker, Obj::MarkerPath { pts, color, width }) => {
                pts.len() == 3 && pts[0] == Pt::new(200.0, 200.0) && pts[2] == Pt::new(300.0, 260.0) && color.a == 90 && *width == e.mem.size(Tool::Marker)
            }
            (Tool::Rect, Obj::Rect { r: rr, .. })
            | (Tool::Ellipse, Obj::Ellipse { r: rr, .. })
            | (Tool::Pixelate, Obj::Pixelate { r: rr, .. })
            | (Tool::Invert, Obj::Invert { r: rr }) => *rr == r(200.0, 200.0, 100.0, 60.0),
            _ => false,
        };
        assert!(ok, "{tool:?}: {draft:?}");
        // The export bakes it in.
        e.rebuild();
        let img = e.export(r(100.0, 100.0, 800.0, 600.0));
        assert_eq!(img.dimensions(), (800, 600));
        assert!(e.composed.is_some(), "{tool:?}: composed after commit");
    }
}

#[test]
fn tiny_strokes_are_dropped() {
    for k in ['D', 'A', 'M', 'R', 'C', 'B', 'I'] {
        let mut a = app(Some(r(100.0, 100.0, 800.0, 600.0)));
        key(&mut a, k as u32);
        down(&mut a, 200, 200);
        mv(&mut a, 200, 201);
        up(&mut a, 200, 201);
        assert!(ed(&mut a).objects.is_empty(), "{k}: a 1 px stroke is no object");
    }
    // A pencil click still draws a dot (two points).
    let mut a = app(Some(r(100.0, 100.0, 800.0, 600.0)));
    key(&mut a, 'P' as u32);
    click(&mut a, 200, 200);
    assert_eq!(ed(&mut a).objects.len(), 1);
}

/// Shift-drawn shapes keep their press point as a corner in every drag
/// direction and are square (the editor side of "draws with Shift jump and
/// get cut off": that was the overlay window being moved, see
/// `wind::imp::tests::overlay_ignores_outside_moves_and_never_enters_move_size`).
#[test]
fn shift_shapes_keep_the_press_point_in_every_direction() {
    for k in ['R', 'E', 'B', 'I'] {
        for (dx, dy) in [(60, 200), (-60, 200), (60, -200), (-60, -200), (200, 60), (-200, -60)] {
            let mut a = app(Some(r(100.0, 100.0, 800.0, 600.0)));
            key(&mut a, k as u32);
            {
                let _m = hold(SHIFT);
                drag(&mut a, (500, 400), (500 + dx, 400 + dy));
            }
            let rr = match ed(&mut a).objects[..] {
                [Obj::Rect { r, .. } | Obj::Ellipse { r, .. } | Obj::Pixelate { r, .. } | Obj::Invert { r }] => r,
                ref o => panic!("{k}: {o:?}"),
            };
            let side = dx.abs().max(dy.abs()) as f32;
            let x = if dx < 0 { 500.0 - side } else { 500.0 };
            let y = if dy < 0 { 400.0 - side } else { 400.0 };
            let want = r(x, y, side, side);
            assert_eq!(rr, want, "{k} ({dx}, {dy})");
        }
    }
}

#[test]
fn shift_constrains_lines_and_shapes() {
    let mut a = app(Some(r(0.0, 0.0, 1440.0, 900.0)));
    key(&mut a, 'L' as u32);
    {
        let _m = hold(SHIFT);
        drag(&mut a, (200, 200), (300, 210));
    }
    let Obj::Line { b, .. } = ed(&mut a).objects[0] else { panic!("line") };
    assert!((b.y - 200.0).abs() < 0.01 && (b.x - 200.0 - 100.5f32).abs() < 0.5, "snapped flat: {b:?}");
    {
        let _m = hold(CTRL);
        drag(&mut a, (200, 200), (300, 290));
    }
    let Obj::Line { b, .. } = ed(&mut a).objects[1] else { panic!("line") };
    assert!((b.x - 200.0 - (b.y - 200.0)).abs() < 0.01, "snapped to 45 degrees: {b:?}");
    key(&mut a, 'R' as u32);
    {
        let _m = hold(SHIFT);
        drag(&mut a, (500, 500), (560, 700));
    }
    let Obj::Rect { r: rr, .. } = ed(&mut a).objects[2] else { panic!("rect") };
    assert_eq!(rr, r(500.0, 500.0, 200.0, 200.0), "a square");
    // Pencil points closer than 2 px are merged.
    key(&mut a, 'P' as u32);
    down(&mut a, 10, 10);
    for x in 11..20 {
        mv(&mut a, x, 10);
    }
    up(&mut a, 19, 10);
    let Obj::Path { pts, .. } = &ed(&mut a).objects[3] else { panic!("path") };
    assert_eq!(pts.len(), 5, "{pts:?}");
}

#[test]
fn tools_toggle_and_escape_steps_back() {
    let mut a = app(Some(r(100.0, 100.0, 300.0, 200.0)));
    key(&mut a, 'R' as u32);
    key(&mut a, 'R' as u32);
    assert_eq!(ed(&mut a).tool, None, "same key twice: off");
    key(&mut a, 'R' as u32);
    key(&mut a, 'E' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Ellipse), "another tool replaces it");
    // Esc mid-drag drops the draft and the tool, keeps the capture.
    down(&mut a, 150, 150);
    mv(&mut a, 200, 200);
    key(&mut a, key::ESCAPE);
    let e = ed(&mut a);
    assert!(e.tool.is_none() && e.draft.is_none() && matches!(e.interact, Interact::None) && !e.done);
    key(&mut a, key::SPACE);
    assert!(ed(&mut a).palette_open);
    key(&mut a, key::ESCAPE);
    assert!(!ed(&mut a).palette_open, "Esc closes the palette first");
    // Keys other than arrows and editing keys are edge-triggered.
    repeat(&mut a, 'R' as u32);
    assert_eq!(ed(&mut a).tool, None);
    // Key-up events are ignored.
    a.on_event(Ev::Key { vk: 'R' as u32, up: true, repeat: false, mods: NONE });
    assert_eq!(ed(&mut a).tool, None);
}

// ---- text --------------------------------------------------------------------

/// Text tool, a click at (300, 300) and `s` typed.
fn typing(s: &str) -> App {
    let mut a = app(Some(r(100.0, 100.0, 800.0, 600.0)));
    key(&mut a, 'T' as u32);
    click(&mut a, 300, 300);
    assert!(ed(&mut a).text.is_some(), "a click opens the draft");
    type_str(&mut a, s);
    a
}

fn draft(a: &mut App) -> (String, usize) {
    let td = ed(a).text.as_ref().expect("text draft");
    (td.text.clone(), td.caret)
}

#[test]
fn text_insert_and_caret_moves() {
    let mut a = typing("Héllo");
    assert_eq!(draft(&mut a), ("Héllo".into(), 6));
    key(&mut a, key::LEFT);
    key(&mut a, key::LEFT);
    assert_eq!(draft(&mut a).1, 4);
    type_str(&mut a, "日");
    assert_eq!(draft(&mut a), ("Hél日lo".into(), 7));
    key(&mut a, key::HOME);
    assert_eq!(draft(&mut a).1, 0);
    key(&mut a, key::LEFT);
    assert_eq!(draft(&mut a).1, 0, "stays at the start");
    key(&mut a, key::RIGHT);
    key(&mut a, key::RIGHT);
    assert_eq!(draft(&mut a).1, 3, "over the two-byte é");
    key(&mut a, key::END);
    assert_eq!(draft(&mut a).1, "Hél日lo".len());
    key(&mut a, key::RIGHT);
    assert_eq!(draft(&mut a).1, "Hél日lo".len(), "stays at the end");
    // Control characters (Enter's '\r', Tab) are not inserted.
    type_str(&mut a, "\r\t\u{8}");
    assert_eq!(draft(&mut a).0, "Hél日lo");
    // Tools keys type nothing and switch nothing while editing.
    key(&mut a, 'R' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Text));
}

#[test]
fn text_backspace_and_delete_repeat() {
    let mut a = typing("ab日c");
    key(&mut a, key::BACK);
    assert_eq!(draft(&mut a), ("ab日".into(), 5));
    repeat(&mut a, key::BACK);
    assert_eq!(draft(&mut a), ("ab".into(), 2), "Backspace auto-repeats over a 3-byte char");
    key(&mut a, key::HOME);
    key(&mut a, key::DELETE);
    assert_eq!(draft(&mut a), ("b".into(), 0));
    repeat(&mut a, key::DELETE);
    assert_eq!(draft(&mut a), ("".into(), 0));
    key(&mut a, key::DELETE);
    key(&mut a, key::BACK);
    assert_eq!(draft(&mut a), ("".into(), 0), "nothing left to delete");
}

#[test]
fn text_commit_cancel_and_click_outside() {
    // Enter commits at the click point with the current colour and size.
    let mut a = typing("Hello  ");
    key(&mut a, key::RETURN);
    let e = ed(&mut a);
    assert!(e.text.is_none() && !e.done, "Enter commits the draft, not the capture");
    assert_eq!(
        e.objects,
        vec![Obj::Text { pos: Pt::new(300.0, 300.0), text: "Hello  ".into(), color: e.mem.color(Tool::Text), size: e.mem.size(Tool::Text) }]
    );
    // Shift+Enter neither commits nor inserts (single-line drafts).
    let mut a = typing("x");
    key_m(&mut a, key::RETURN, SHIFT);
    assert_eq!(draft(&mut a).0, "x");
    // Esc drops the draft, keeps the tool.
    key(&mut a, key::ESCAPE);
    let e = ed(&mut a);
    assert!(e.text.is_none() && e.objects.is_empty() && e.tool == Some(Tool::Text) && !e.done);
    // Whitespace only: nothing to commit.
    let mut a = typing("   ");
    key(&mut a, key::RETURN);
    assert!(ed(&mut a).objects.is_empty());
    // A click inside the box keeps editing; one outside commits.
    let mut a = typing("abc");
    click(&mut a, 302, 305);
    assert!(ed(&mut a).text.is_some(), "inside the draft box");
    click(&mut a, 700, 600);
    let e = ed(&mut a);
    assert!(e.text.is_none() && e.objects.len() == 1, "outside commits");
    // The next click starts a new draft there.
    click(&mut a, 700, 600);
    assert_eq!(ed(&mut a).text.as_ref().map(|t| t.pos), Some(Pt::new(700.0, 600.0)));
    // The wheel does not change sizes while typing.
    let before = ed(&mut a).mem.size(Tool::Text);
    ed(&mut a).last_wheel = a_second_ago();
    a.on_event(Ev::Wheel { delta: 120, x: 0, y: 0 });
    assert_eq!(ed(&mut a).mem.size(Tool::Text), before);
}

// ---- history -----------------------------------------------------------------

/// Draw a rect from (x, 100) to (x + 50, 150) with the current tool.
fn stroke(a: &mut App, x: i32) {
    drag(a, (x, 100), (x + 50, 150));
}

#[test]
fn undo_redo_across_tools() {
    let mut a = app(Some(r(0.0, 0.0, 1440.0, 900.0)));
    key(&mut a, 'R' as u32);
    stroke(&mut a, 100);
    key(&mut a, 'E' as u32);
    stroke(&mut a, 200);
    key(&mut a, 'B' as u32);
    stroke(&mut a, 300);
    let kinds = |a: &mut App| ed(a).objects.len();
    assert_eq!(kinds(&mut a), 3);
    key_m(&mut a, 'Z' as u32, CTRL);
    key_m(&mut a, 'Z' as u32, CTRL);
    assert!(matches!(ed(&mut a).objects[..], [Obj::Rect { .. }]));
    key_m(&mut a, 'Y' as u32, CTRL);
    assert!(matches!(ed(&mut a).objects[..], [Obj::Rect { .. }, Obj::Ellipse { .. }]));
    key_m(&mut a, 'Z' as u32, CTRL_SHIFT);
    assert_eq!(kinds(&mut a), 3, "Ctrl+Shift+Z redoes too");
    key_m(&mut a, 'Y' as u32, CTRL);
    assert_eq!(kinds(&mut a), 3, "nothing more to redo");
    for _ in 0..5 {
        key_m(&mut a, 'Z' as u32, CTRL);
    }
    assert!(ed(&mut a).objects.is_empty(), "undo stops at the empty start");
    assert!(ed(&mut a).dirty);
    // Rebuilt with no objects: the composed copy is dropped.
    ed(&mut a).rebuild();
    assert!(ed(&mut a).composed.is_none());
    // A new object after an undo discards the redo branch.
    key_m(&mut a, 'Y' as u32, CTRL);
    key(&mut a, 'I' as u32);
    stroke(&mut a, 400);
    assert!(matches!(ed(&mut a).objects[..], [Obj::Rect { .. }, Obj::Invert { .. }]));
    key_m(&mut a, 'Y' as u32, CTRL);
    assert_eq!(ed(&mut a).objects.len(), 2, "the old redo branch is gone");
}

#[test]
fn undo_limit_drops_the_oldest_states() {
    let mut a = app(Some(r(0.0, 0.0, 1440.0, 900.0)));
    ed(&mut a).cfg.undo_limit = 3;
    key(&mut a, 'R' as u32);
    for i in 0..5 {
        stroke(&mut a, 100 + i * 100);
    }
    assert_eq!(ed(&mut a).hist.len(), 3, "limit");
    for _ in 0..10 {
        key_m(&mut a, 'Z' as u32, CTRL);
    }
    assert_eq!(ed(&mut a).objects.len(), 3, "only two steps back");
    // undo_limit = 0 behaves as 1: no undo at all.
    let mut a = app(Some(r(0.0, 0.0, 1440.0, 900.0)));
    ed(&mut a).cfg.undo_limit = 0;
    key(&mut a, 'R' as u32);
    stroke(&mut a, 100);
    stroke(&mut a, 200);
    key_m(&mut a, 'Z' as u32, CTRL);
    assert_eq!(ed(&mut a).objects.len(), 2);
}

// ---- palette, sizes, toolbar ------------------------------------------------

#[test]
fn wheel_and_size_acts_change_the_active_tool_size() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    let wheel = |a: &mut App, delta: i32| {
        ed(a).last_wheel = a_second_ago();
        a.on_event(Ev::Wheel { delta, x: 10, y: 10 });
    };
    // No tool chosen yet: the pencil's size (the first tool).
    let line = ed(&mut a).mem.size(Tool::Path);
    wheel(&mut a, 120);
    assert_eq!(ed(&mut a).mem.size(Tool::Path), line + 1.0);
    assert_eq!(ed(&mut a).mem.size(Tool::Line), line, "every tool has its own");
    assert_eq!(ed(&mut a).notice.as_ref().map(|t| t.text.clone()), Some(format!("Size {}", line as i32 + 1)));
    // A second notch within 160 ms is ignored, as is a zero delta.
    a.on_event(Ev::Wheel { delta: 120, x: 10, y: 10 });
    wheel(&mut a, 0);
    assert_eq!(ed(&mut a).mem.size(Tool::Path), line + 1.0);
    // Invert has no size: the wheel does nothing, the readout is empty.
    key(&mut a, 'I' as u32);
    wheel(&mut a, 120);
    assert_eq!(ed(&mut a).size_label(), "");
    assert_eq!(ed(&mut a).mem.size(Tool::Pixelate), 12.0, "not the pixelate size");
    key(&mut a, 'I' as u32);
    for (k, lo, hi) in [('R', 1.0, 50.0), ('M', 1.0, 50.0), ('B', 4.0, 100.0), ('T', 8.0, 96.0), ('L', 1.0, 50.0)] {
        key(&mut a, k as u32);
        for _ in 0..120 {
            wheel(&mut a, -120);
        }
        let v = |a: &mut App| ed(a).size_label().parse::<f32>().unwrap();
        assert_eq!(v(&mut a), lo, "{k}: floor");
        for _ in 0..120 {
            let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
            let mut e = *e;
            a.apply_act(&mut e, Act::Size(1));
            a.st = State::Edit(Box::new(e));
        }
        assert_eq!(v(&mut a), hi, "{k}: ceiling");
        key(&mut a, k as u32); // tool off again
    }
}

#[test]
fn palette_opens_only_with_a_selection_and_picks_a_colour() {
    let mut a = app(None);
    key(&mut a, key::SPACE);
    assert!(!ed(&mut a).palette_open, "no toolbar without a selection");
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    key(&mut a, key::SPACE);
    assert!(ed(&mut a).palette_open);
    let now = Instant::now();
    ed(&mut a).prepare(None, (0, 0), now);
    let tb = ed(&mut a).toolbar.clone().expect("toolbar");
    let dots: Vec<_> = tb.items.iter().filter_map(|it| if let toolbar::Kind::Dot(c) = it.kind { Some((c, it.r)) } else { None }).collect();
    assert!(!dots.is_empty() && dots.len() <= palette_colors(&ed(&mut a).cfg).len());
    let (c, dr) = *dots.last().unwrap();
    click(&mut a, (dr.x + dr.w / 2.0) as i32, (dr.y + dr.h / 2.0) as i32);
    assert_eq!(ed(&mut a).color(), c);
    assert!(!ed(&mut a).palette_open, "picking a colour closes it");
}

/// Centre of the toolbar button for `act` (lays the toolbar out first).
fn button(a: &mut App, act: Act) -> (i32, i32) {
    let m = a.mouse;
    ed(a).prepare(None, m, Instant::now());
    let tb = ed(a).toolbar.as_ref().expect("toolbar");
    let it = tb.items.iter().find(|i| i.kind == toolbar::Kind::Btn(act)).unwrap_or_else(|| panic!("{act:?} button"));
    ((it.r.x + it.r.w / 2.0) as i32, (it.r.y + it.r.h / 2.0) as i32)
}

#[test]
fn toolbar_buttons_act_and_consume_the_press() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    let (x, y) = button(&mut a, Act::Tool(Tool::Rect));
    down(&mut a, x, y);
    assert_eq!(ed(&mut a).tool, Some(Tool::Rect));
    assert!(ed(&mut a).pressed.is_some() && matches!(ed(&mut a).interact, Interact::None), "no drag starts");
    up(&mut a, x, y);
    assert!(ed(&mut a).pressed.is_none());
    // Hovering a button is tracked (tooltip).
    mv(&mut a, x, y);
    assert!(ed(&mut a).hover.is_some());
    mv(&mut a, 5, 5);
    assert!(ed(&mut a).hover.is_none());
    // Disabled Undo consumes the press but does nothing.
    let (ux, uy) = button(&mut a, Act::Undo);
    click(&mut a, ux, uy);
    assert!(ed(&mut a).objects.is_empty() && matches!(ed(&mut a).interact, Interact::None));
    // Size buttons and the palette button.
    let line = ed(&mut a).mem.size(Tool::Rect);
    let (sx, sy) = button(&mut a, Act::Size(1));
    click(&mut a, sx, sy);
    assert_eq!(ed(&mut a).mem.size(Tool::Rect), line + 1.0);
    let (px, py) = button(&mut a, Act::Palette);
    click(&mut a, px, py);
    assert!(ed(&mut a).palette_open);
    // A press on the bar commits an open text draft first.
    let mut a = typing("note");
    let (tx, ty) = button(&mut a, Act::Tool(Tool::Text));
    down(&mut a, tx, ty);
    let e = ed(&mut a);
    assert!(e.text.is_none() && e.objects.len() == 1 && e.tool.is_none(), "committed, and the Text tool toggled off");
}

#[test]
fn output_buttons_and_keys_set_the_tasks() {
    // (act, tasks) without running the export.
    let run = |act: Act, save_dialog: bool| {
        let mut a = app(Some(r(10.0, 10.0, 50.0, 50.0)));
        let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
        let mut e = *e;
        e.cfg.save_dialog = save_dialog;
        a.apply_act(&mut e, act);
        e
    };
    assert!(matches!(run(Act::Copy, false).tasks[..], [Task::Copy]));
    assert!(matches!(run(Act::Upload, false).tasks[..], [Task::Upload]));
    assert!(matches!(run(Act::Save, false).tasks[..], [Task::Save { path: None, ask: false }]));
    assert!(matches!(run(Act::Save, true).tasks[..], [Task::Save { path: None, ask: true }]), "save_dialog asks");
    assert!(matches!(run(Act::SaveAs, false).tasks[..], [Task::Save { path: None, ask: true }]));
    let e = run(Act::Exit, false);
    assert!(e.done && e.cancelled);
    let e = run(Act::Accept, false);
    assert!(e.done && !e.cancelled && e.tasks.is_empty());
    for (vk, mods, act) in [('C', CTRL, Act::Copy), ('U', CTRL, Act::Upload), ('S', CTRL, Act::Save)] {
        let mut a = app(Some(r(10.0, 10.0, 50.0, 50.0)));
        let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
        let mut e = *e;
        a.handle_key(&mut e, vk as u32, false, mods);
        assert!(e.done, "{act:?}");
        assert_eq!(action_of(act).and_then(act_of), Some(act), "{act:?} round-trips through its keymap action");
    }
    assert_eq!(action_of(Act::Exit), Some(Action::Cancel));
    assert_eq!(action_of(Act::Accept), Some(Action::Accept));
    assert_eq!(action_of(Act::Color(C4::rgb(1, 2, 3))), None);
    assert_eq!(action_of(Act::Size(1)), Some(Action::SizeUp));
    assert_eq!(action_of(Act::Size(-1)), Some(Action::SizeDown));
    for a in [Action::SelectAll, Action::Accept, Action::Cancel] {
        assert_eq!(act_of(a), None);
    }
}

// ---- accept / cancel -------------------------------------------------------

#[test]
fn escape_cancels_one_shot_with_code_2_and_hides_a_daemon() {
    let mut a = app(Some(r(10.0, 10.0, 50.0, 50.0)));
    key(&mut a, key::ESCAPE);
    assert!(hidden(&a));
    assert_eq!(code(&a), 2);
    let mut a = app(None);
    a.kind = RunKind::Daemon;
    key(&mut a, key::ESCAPE);
    assert!(hidden(&a), "daemon: hidden, waiting for the next hotkey");
    assert_eq!(code(&a), 0);
    // The toolbar's exit button does the same.
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    let (x, y) = button(&mut a, Act::Exit);
    down(&mut a, x, y);
    assert!(hidden(&a));
    assert_eq!(code(&a), 2);
}

#[test]
fn enter_exports_the_selection_and_closes_a_one_shot() {
    let dir = TempDir::new("enter");
    let mut a = app(Some(r(100.0, 80.0, 300.0, 200.0)));
    ed(&mut a).tasks = vec![Task::Save { path: Some(dir.0.clone()), ask: false }, Task::Geometry];
    ed(&mut a).cfg.filename_pattern = "shot".into();
    key(&mut a, 'R' as u32);
    stroke(&mut a, 150);
    key(&mut a, key::RETURN);
    assert!(hidden(&a), "export ran and the one-shot closed");
    assert_eq!(code(&a), 0);
    let saved = dir.0.join("shot.png");
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&saved).expect("saved PNG")));
    let reader = dec.read_info().expect("png");
    assert_eq!((reader.info().width, reader.info().height), (300, 200), "the selection, cropped");
    assert!(a.upload_slot.lock().unwrap().is_none());
}

#[test]
fn enter_without_a_selection_takes_the_whole_shot_and_errors_set_code_1() {
    let dir = TempDir::new("all");
    // The parent "folder" is a file: the save fails.
    let blocker = dir.0.join("file");
    std::fs::write(&blocker, b"x").unwrap();
    let mut a = app(None);
    a.kind = RunKind::Daemon;
    ed(&mut a).tasks = vec![Task::Save { path: Some(blocker.join("out.png")), ask: false }];
    key(&mut a, key::RETURN);
    assert!(hidden(&a), "daemon: back to hidden after the export");
    assert_eq!(code(&a), 1, "a failed task sets the exit code");
    // accept_key alone: no selection selects all; no tasks means Save.
    let mut a = app(None);
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    accept_key(&mut e);
    assert_eq!(e.sel, Some(r(0.0, 0.0, 1440.0, 900.0)));
    assert!(e.done && matches!(e.tasks[..], [Task::Save { path: None, ask: false }]));
    // With accept_on_select it accepts right away, keeping the given tasks.
    let mut e = {
        let mut a = app(None);
        let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
        *e
    };
    e.accept_on_select = true;
    accept_key(&mut e);
    assert!(e.done && e.tasks.is_empty());
}

#[test]
fn accept_on_select_finishes_when_the_selection_ends() {
    let mut a = app(None);
    ed(&mut a).accept_on_select = true;
    ed(&mut a).tasks = vec![Task::Geometry];
    drag(&mut a, (10, 10), (110, 60));
    assert!(hidden(&a));
    assert_eq!(code(&a), 0);
}

#[test]
fn dirty_objects_are_baked_before_the_export() {
    let mut a = app(Some(r(0.0, 0.0, 100.0, 100.0)));
    let e = ed(&mut a);
    e.objects.push(Obj::Invert { r: r(0.0, 0.0, 10.0, 10.0) });
    e.dirty = true;
    let before = e.base.as_raw()[..4].to_vec();
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    a.finish(&mut e, false);
    let State::Finish(job) = &a.st else { panic!("finish job") };
    assert_eq!(job.img.dimensions(), (100, 100));
    assert_ne!(job.img.as_raw()[..3], before[..3], "the inverted corner is in the export");
    assert_eq!(job.sel_global, (0, 0));
}

// ---- toasts and timers ------------------------------------------------------

fn aged(t: &mut Toast, ms: u64) {
    t.at = Instant::now().checked_sub(Duration::from_millis(ms)).expect("uptime");
}

#[test]
fn toast_lifecycle() {
    let mut t = Toast::new("Size 4", ToastKind::Info);
    assert_eq!(t.ttl_ms(), 1600.0);
    assert_eq!(Toast::new("x", ToastKind::Error).ttl_ms(), 4000.0);
    assert!(t.animating(Instant::now()), "fading in");
    aged(&mut t, 800);
    let now = Instant::now();
    assert!(!t.animating(now) && (t.opacity(now) - 1.0).abs() < 1e-6 && !t.expired(now), "fully shown");
    aged(&mut t, 1650);
    let now = Instant::now();
    assert!(t.animating(now) && t.opacity(now) < 1.0 && t.opacity(now) > 0.0 && !t.expired(now), "fading out");
    aged(&mut t, 1800);
    assert!(t.expired(Instant::now()));

    // The pump drops expired toasts (the editor's and the app's).
    let mut a = app(Some(r(10.0, 10.0, 50.0, 50.0)));
    let mut old = Toast::new("Size 2", ToastKind::Info);
    aged(&mut old, 5000);
    ed(&mut a).notice = Some(old);
    let mut gone = Toast::new("Uploaded", ToastKind::Success);
    aged(&mut gone, 5000);
    a.notice = Some(gone);
    assert!(a.on_event(Ev::Timer), "the toast went away: a new frame");
    assert!(ed(&mut a).notice.is_none() && a.notice.is_none());
    // A live toast stays.
    ed(&mut a).notice = Some(Toast::new("Size 3", ToastKind::Info));
    a.on_event(Ev::Timer);
    assert!(ed(&mut a).notice.is_some());
}

#[test]
fn timer_repaints_only_while_something_changes() {
    let mut a = app(Some(r(10.0, 10.0, 200.0, 100.0)));
    assert!(a.on_event(Ev::Timer), "fade-in running");
    let e = ed(&mut a);
    e.prepare(None, (0, 0), Instant::now());
    e.toast_drawn = None;
    e.caret_drawn = None;
    let e = ed(&mut a);
    for t in [&mut e.mo.dim, &mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint, &mut e.mo.hover] {
        t.snap(t.target());
    }
    assert!(!a.on_event(Ev::Timer), "settled: no frame");
    // Hidden: only a state change asks for a frame.
    a.st = State::Hidden;
    assert!(!a.on_event(Ev::Timer));
}

#[test]
fn window_events_the_overlay_ignores() {
    let mut a = app(Some(r(10.0, 10.0, 50.0, 50.0)));
    assert!(!a.on_event(Ev::Close));
    assert!(!a.on_event(Ev::Resize(10, 10)));
    assert!(!a.on_event(Ev::Focus(true)));
    assert!(matches!(a.st, State::Edit(_)));
    // Input with nothing open is harmless.
    a.st = State::Hidden;
    for ev in [Ev::Down { x: 1, y: 1 }, Ev::Move { x: 2, y: 2 }, Ev::Up { x: 2, y: 2 }, Ev::Char('a' as u16)] {
        assert!(a.on_event(ev));
    }
    key(&mut a, key::RETURN);
    assert!(hidden(&a) && a.mouse == (2, 2));
    assert!(a.frame().is_none() && a.damage().is_none());
}

#[test]
fn software_frame_and_no_gdi_damage() {
    let mut a = app(Some(r(100.0, 100.0, 300.0, 200.0)));
    let f = a.frame().expect("software frame");
    assert_eq!(f.dimensions(), (1440, 900));
    assert!(a.damage().is_none(), "software: the whole window");
    #[cfg(windows)]
    assert!(!a.paint(windows::Win32::Graphics::Gdi::HDC::default(), &[]), "no GDI screen");
    // The frame buffer is reused.
    let p = a.frame().unwrap().as_raw().as_ptr();
    assert_eq!(a.frame().unwrap().as_raw().as_ptr(), p);
}

// ---- upload results --------------------------------------------------------

#[test]
fn upload_results_become_toasts() {
    let mut a = app(None);
    a.st = State::Hidden;
    let tx = upload_in_flight(&a);
    a.on_event(Ev::Timer);
    assert!(a.upload_slot.lock().unwrap().is_some(), "still in flight");
    assert!(a.notice.is_none());
    tx.send(Ok("https://i.imgur.com/x.png".into())).unwrap();
    a.on_event(Ev::Timer);
    assert!(a.upload_slot.lock().unwrap().is_none());
    let t = a.notice.as_ref().expect("toast");
    assert!(t.text == "Uploaded https://i.imgur.com/x.png" && t.kind == ToastKind::Success, "{}", t.text);

    let tx = upload_in_flight(&a);
    tx.send(Err("HTTP 429".into())).unwrap();
    a.poll_upload();
    let t = a.notice.as_ref().expect("toast");
    assert!(t.text == "Upload failed: HTTP 429" && t.kind == ToastKind::Error, "{}", t.text);

    // The uploader went away without a word: just forget it.
    a.notice = None;
    drop(upload_in_flight(&a));
    a.poll_upload();
    assert!(a.upload_slot.lock().unwrap().is_none() && a.notice.is_none());
}

/// Ctrl+U in the daemon: the overlay hides, the upload runs, and its
/// result reaches the user outside the hidden overlay: in the notice
/// (shown as "Uploading…"), or a tray balloon once `gone` took it down.
fn daemon_upload(fake: crate::export::FakeUpload, gone: bool) -> (App, Vec<crate::notice::Note>, Vec<String>) {
    crate::export::FAKE_UPLOAD.with(|f| f.set(Some(fake)));
    let _ = (crate::tray::take_notes(), crate::notice::take_shown());
    let mut a = app(Some(r(100.0, 80.0, 300.0, 200.0)));
    a.kind = RunKind::Daemon;
    key_m(&mut a, 'U' as u32, CTRL);
    assert!(hidden(&a), "the overlay closed");
    if gone {
        crate::notice::hide();
    }
    let t0 = Instant::now();
    while a.upload_slot.lock().unwrap().is_some() {
        assert!(t0.elapsed() < Duration::from_secs(10), "upload never finished");
        std::thread::sleep(Duration::from_millis(5));
        a.on_event(Ev::Timer);
    }
    crate::export::FAKE_UPLOAD.with(|f| f.set(None));
    let shown = crate::notice::take_shown();
    // Bottom-centre of the monitor under the capture: its centre is the anchor.
    assert!(shown.iter().all(|(_, at)| *at == (250, 180)), "{shown:?}");
    (a, shown.into_iter().map(|(n, _)| n).collect(), crate::tray::take_notes())
}

fn texts(notes: &[crate::notice::Note]) -> Vec<&str> {
    notes.iter().map(|n| n.text.as_str()).collect()
}

#[test]
fn daemon_upload_result_is_shown_after_the_overlay_hides() {
    let (a, shown, balloons) = daemon_upload(
        |png| {
            assert!(png.starts_with(b"\x89PNG"), "a PNG is uploaded");
            Ok("https://i.imgur.com/x.png".into())
        },
        false,
    );
    assert_eq!(code(&a), 0);
    assert_eq!(texts(&shown), ["Uploading\u{2026}", "Uploaded \u{b7} i.imgur.com/x.png"]);
    assert!(balloons.is_empty(), "{balloons:?}");

    let (_, shown, balloons) = daemon_upload(|_| Err("HTTP 429".into()), false);
    assert_eq!(texts(&shown), ["Uploading\u{2026}", "Upload failed: HTTP 429"]);
    assert_eq!(shown[1].kind, ToastKind::Error);
    assert!(balloons.is_empty(), "{balloons:?}");
}

#[test]
fn a_late_upload_result_is_a_tray_balloon() {
    let (_, shown, balloons) = daemon_upload(|_| Ok("https://i.imgur.com/x.png".into()), true);
    assert!(shown.is_empty(), "the notice was gone: {shown:?}");
    assert_eq!(balloons, ["Uploaded\nhttps://i.imgur.com/x.png"]);
    let (_, _, balloons) = daemon_upload(|_| Err("HTTP 429".into()), true);
    assert_eq!(balloons, ["Upload failed: HTTP 429"]);
}

#[test]
fn a_daemon_save_says_so_after_the_overlay_hides() {
    let dir = TempDir::new("notice");
    let _ = crate::notice::take_shown();
    let mut a = app(Some(r(100.0, 80.0, 300.0, 200.0)));
    a.kind = RunKind::Daemon;
    ed(&mut a).tasks = vec![Task::Save { path: Some(dir.0.clone()), ask: false }];
    ed(&mut a).cfg.filename_pattern = "shot".into();
    key(&mut a, key::RETURN);
    assert!(hidden(&a));
    let shown = crate::notice::take_shown();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!((shown[0].0.text.as_str(), shown[0].1), ("Saved \u{b7} shot.png", (250, 180)));

    // A one-shot run says it on stdout: no notice.
    let mut a = app(Some(r(100.0, 80.0, 300.0, 200.0)));
    ed(&mut a).tasks = vec![Task::Save { path: Some(dir.0.clone()), ask: false }];
    key(&mut a, key::RETURN);
    assert!(crate::notice::take_shown().is_empty());

    // The next capture takes the notice down.
    crate::notice::show(crate::notice::compose(&[crate::export::Done::Copied], false).unwrap(), (0, 0), crate::theme::DARK);
    queue(Err(anyhow::anyhow!("no screen")));
    a.begin_capture(Pending::editor());
    assert!(crate::notice::take_shown().is_empty(), "hidden when a capture begins");
}

// ---- begin_capture ---------------------------------------------------------

fn queue(shot: anyhow::Result<Shot>) {
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(shot));
}

/// A hidden app waiting to start `pending`.
fn waiting(kind: RunKind, pending: Pending) -> App {
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = kind;
    a.pending = Some(pending);
    a
}

#[test]
fn begin_capture_maps_the_region_and_takes_the_overrides() {
    let mut shot = synthetic_shot();
    shot.origin = (-1440, 100);
    queue(Ok(shot));
    let mut p = Pending::editor();
    p.region = Some((-1400, 150, 300, 200));
    p.filename = Some("%F_x".into());
    p.tasks = vec![Task::Geometry];
    let mut a = waiting(RunKind::OneShot, p);
    a.cfg.draw_color = "#00ff00".into();
    assert!(a.on_event(Ev::Timer), "the editor opened");
    let e = ed(&mut a);
    assert_eq!(e.sel, Some(r(40.0, 50.0, 300.0, 200.0)), "global → image coordinates");
    assert_eq!(e.cfg.filename_pattern, "%F_x");
    assert_eq!(e.color(), C4::rgb(0, 255, 0));
    assert!(e.shot.image.as_raw().is_empty() && e.base.dimensions() == (1440, 900), "pixels held once");
    assert!(matches!(e.tasks[..], [Task::Geometry]));
    assert!(a.pending.is_none());

    // A region past the shot is clamped; a degenerate one is no selection.
    queue(Ok(synthetic_shot()));
    let mut p = Pending::editor();
    p.region = Some((1400, 880, 300, 200));
    let mut a = waiting(RunKind::OneShot, p);
    a.on_event(Ev::Timer);
    assert_eq!(ed(&mut a).sel, Some(r(1140.0, 700.0, 300.0, 200.0)));
    queue(Ok(synthetic_shot()));
    let mut p = Pending::editor();
    p.region = Some((10, 10, 0, 50));
    let mut a = waiting(RunKind::OneShot, p);
    a.on_event(Ev::Timer);
    assert_eq!(ed(&mut a).sel, None);
}

#[test]
fn begin_capture_accepts_a_given_region_right_away() {
    queue(Ok(synthetic_shot()));
    let mut p = Pending::editor();
    p.region = Some((0, 0, 64, 32));
    p.accept_on_select = true;
    p.tasks = vec![Task::Geometry];
    let mut a = waiting(RunKind::OneShot, p);
    a.on_event(Ev::Timer);
    assert!(hidden(&a), "accepted, exported, closed");
    assert_eq!(code(&a), 0);
    // Without a region the editor waits for the selection.
    queue(Ok(synthetic_shot()));
    let mut p = Pending::editor();
    p.accept_on_select = true;
    let mut a = waiting(RunKind::OneShot, p);
    a.on_event(Ev::Timer);
    assert!(matches!(a.st, State::Edit(_)));
}

#[test]
fn capture_failure_exits_a_one_shot_and_keeps_a_daemon() {
    queue(Err(anyhow::anyhow!("no display")));
    let mut a = waiting(RunKind::OneShot, Pending::editor());
    a.on_event(Ev::Timer);
    assert!(hidden(&a));
    assert_eq!(code(&a), 1);
    queue(Err(anyhow::anyhow!("no display")));
    let mut a = waiting(RunKind::Daemon, Pending::editor());
    a.on_event(Ev::Timer);
    assert!(hidden(&a) && a.pending.is_none());
    assert_eq!(code(&a), 0, "the daemon keeps running");
}

#[test]
fn daemon_capture_cycle() {
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    a.on_create(Hwnd::default());
    assert!(hidden(&a));
    // A hotkey press opens the editor; a second press while it is open is ignored.
    assert!(a.on_hot(HotEvent::Capture));
    queue(Ok(synthetic_shot()));
    assert!(a.on_event(Ev::Timer));
    assert!(matches!(a.st, State::Edit(_)));
    assert!(a.on_hot(HotEvent::Capture));
    assert!(a.pending.is_none());
    // Select, copy... (Geometry here), back to hidden: the daemon stays.
    ed(&mut a).tasks = vec![Task::Geometry];
    drag(&mut a, (10, 10), (60, 60));
    key(&mut a, key::RETURN);
    assert!(hidden(&a));
    assert_eq!(code(&a), 0);
    // Quit closes it.
    assert!(!a.on_hot(HotEvent::Quit));
}

// ---- recording ---------------------------------------------------------------

/// Centre of the first toolbar item `f` matches (lays the toolbar out first).
#[cfg(windows)]
fn item(a: &mut App, f: impl Fn(&toolbar::Kind) -> bool) -> (i32, i32) {
    let m = a.mouse;
    ed(a).prepare(None, m, Instant::now());
    let tb = ed(a).toolbar.as_ref().expect("toolbar");
    let it = tb.items.iter().find(|i| f(&i.kind)).expect("toolbar item");
    ((it.r.x + it.r.w / 2.0) as i32, (it.r.y + it.r.h / 2.0) as i32)
}

#[cfg(windows)]
fn rec_phase(a: &App) -> Option<rec_ui::Phase> {
    a.rec.as_ref().map(|r| r.phase())
}

/// The Record button (or the mode switch, or 2) turns video mode on: the
/// setup panel replaces the toolbar, centred under the selection; its
/// choices start a recording of the selection (physical px).
#[cfg(windows)]
#[test]
fn record_panel_choices_start_a_recording() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    a.kind = RunKind::Daemon;
    let (x, y) = button(&mut a, Act::Record);
    click(&mut a, x, y);
    assert!(ed(&mut a).rec.open && !ed(&mut a).palette_open);
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let tb = ed(&mut a).toolbar.clone().unwrap();
    assert!(tb.video && tb.bar.w == panel::PANEL_W, "the panel replaces the toolbar");
    assert_eq!(tb.bar.x + tb.bar.w / 2.0, 400.0, "centred under the selection");
    assert_eq!(tb.bar.y, 508.0, "8 px below it");
    assert!(!tb.items.iter().any(|i| i.kind == toolbar::Kind::Btn(Act::Copy)), "no toolbar buttons");
    // Space (the palette) does nothing here.
    key(&mut a, key::SPACE);
    assert!(!ed(&mut a).palette_open && ed(&mut a).rec.open);
    let seg = |a: &mut App, act: Act| item(a, |k| matches!(k, toolbar::Kind::Seg { act: x, .. } if *x == act));
    let (sx, sy) = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecSystem, .. }));
    click(&mut a, sx, sy);
    assert!(!ed(&mut a).rec.opts.system);
    let (mx, my) = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecMic, .. }));
    click(&mut a, mx, my);
    assert!(ed(&mut a).rec.opts.mic);
    // The microphone list: pick the second device.
    let (bx, by) = item(&mut a, |k| matches!(k, toolbar::Kind::DevBox { dev: toolbar::Dev::Mic, .. }));
    click(&mut a, bx, by);
    assert_eq!(ed(&mut a).rec.dev_open, Some(toolbar::Dev::Mic));
    let (dx, dy) = item(&mut a, |k| matches!(k, toolbar::Kind::DevItem { i: 2, .. }));
    click(&mut a, dx, dy);
    assert!(ed(&mut a).rec.dev_open.is_none());
    assert_eq!(ed(&mut a).rec.opts.mic_dev, "{mic-2}");
    assert_eq!(ed(&mut a).rec.mic_label(), "Headset");
    // The camera: on shows the bubble over the selection, off closes it.
    let (cx, cy) = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    click(&mut a, cx, cy);
    assert!(ed(&mut a).rec.opts.camera);
    assert!(ed(&mut a).rec.cam.shown.is_some());
    click(&mut a, cx, cy);
    assert!(!ed(&mut a).rec.opts.camera && ed(&mut a).rec.cam.shown.is_none());
    // Esc closes a list first, then cancels (the panel's Close).
    click(&mut a, bx, by);
    key(&mut a, key::ESCAPE);
    assert!(matches!(a.st, State::Edit(_)) && ed(&mut a).rec.open && ed(&mut a).rec.dev_open.is_none());
    // No countdown, then Start.
    let (zx, zy) = seg(&mut a, Act::RecCountdown(false));
    click(&mut a, zx, zy);
    let (tx, ty) = item(&mut a, |k| matches!(k, toolbar::Kind::Primary { .. }));
    click(&mut a, tx, ty);
    assert!(hidden(&a), "the overlay is gone while recording");
    let spec = a.rec_spec.clone().expect("recording started");
    assert_eq!(spec.area, (100, 100, 600, 400));
    assert_eq!((spec.format, spec.fps, spec.system_audio), (crate::rec::RecFormat::Mp4, 30, false));
    assert_eq!(spec.mic.as_deref(), Some("{mic-2}"));
    assert_eq!(rec_phase(&a), Some(rec_ui::Phase::Recording), "Countdown 0: at once");
    assert!(!a.idle(), "recording is not idle");
    // The choices are remembered for the next capture; the microphone is
    // no longer metered (the panel is gone).
    assert_eq!(a.rec_opts.as_ref().map(|o| (o.countdown, o.mic)), Some((false, true)));
}

/// The mode switch: 2 / 1 and its two buttons; the size label sits right
/// of it; Close cancels the capture.
#[cfg(windows)]
#[test]
fn mode_switch_keys_and_close() {
    let mut a = app(Some(r(300.0, 200.0, 800.0, 400.0)));
    key(&mut a, '2' as u32);
    assert!(ed(&mut a).rec.open, "2: video");
    key(&mut a, '1' as u32);
    assert!(!ed(&mut a).rec.open, "1: screenshot");
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let tb = ed(&mut a).toolbar.clone().unwrap();
    let mode = tb.mode.expect("the mode switch shows in screenshot mode too");
    assert!(mode.y1() <= 200.0 - 8.0 + 0.01 && mode.x == 300.0, "above the selection's top-left: {mode:?}");
    let label = ed(&mut a).scene.as_ref().unwrap().label.clone().unwrap();
    assert_eq!(label.lead, Some(mode), "the size label follows it");
    // Its Video button.
    let (vx, vy) = item(&mut a, |k| matches!(k, toolbar::Kind::Mode { video: true, .. }));
    click(&mut a, vx, vy);
    assert!(ed(&mut a).rec.open);
    let (sx, sy) = item(&mut a, |k| matches!(k, toolbar::Kind::Mode { video: false, .. }));
    click(&mut a, sx, sy);
    assert!(!ed(&mut a).rec.open);
    // Ctrl+2 is not the mode key; in a run with tasks there is no video mode.
    key_m(&mut a, '2' as u32, CTRL);
    assert!(!ed(&mut a).rec.open);
    // Close (video mode, top right of the selection) cancels.
    key(&mut a, '2' as u32);
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let close = ed(&mut a).toolbar.as_ref().unwrap().items.iter().find(|i| i.kind == toolbar::Kind::Close).unwrap().r;
    assert!((close.x1() - 1100.0).abs() < 0.01 && close.y1() <= 192.01, "at the selection's top right: {close:?}");
    click(&mut a, (close.x + 5.0) as i32, (close.y + 5.0) as i32);
    assert_eq!(code(&a), 2, "cancelled");
    let mut a = app(Some(r(300.0, 200.0, 800.0, 400.0)));
    ed(&mut a).tasks = vec![Task::Geometry];
    key(&mut a, '2' as u32);
    assert!(!ed(&mut a).rec.open);
}

/// The source: F the monitor, W a window picked like window snapping (a
/// click takes it), R keeps the region.
/// Video mode needs recording (`rec_ui::supported`: not on macOS in tests).
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn source_keys_pick_screen_window_region() {
    let mut a = app(Some(r(300.0, 200.0, 400.0, 300.0)));
    key(&mut a, '2' as u32);
    key(&mut a, 'F' as u32);
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Screen);
    assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 1440.0, 900.0)), "the whole monitor");
    key(&mut a, 'R' as u32);
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Region);
    assert_eq!(ed(&mut a).sel, Some(r(0.0, 0.0, 1440.0, 900.0)), "kept");
    // W: windows to pick from (snapping off in the config or not).
    ed(&mut a).cfg.snap_windows = false;
    let win = crate::snap::Node::new(crate::snap::R::new(120, 90, 1320, 810), vec![]);
    FAKE_SNAP.with(|s| *s.borrow_mut() = Some(crate::snap::Snapshot::build(&[win], (0, 0), (1440, 900))));
    mv(&mut a, 400, 300);
    key(&mut a, 'W' as u32);
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Window);
    assert!(ed(&mut a).sel.is_none() && ed(&mut a).snap.is_some(), "picking a window");
    assert_eq!(ed(&mut a).snap_rect(), Some(r(120.0, 90.0, 1200.0, 720.0)));
    click(&mut a, 400, 300);
    assert_eq!(ed(&mut a).sel, Some(r(120.0, 90.0, 1200.0, 720.0)), "the window");
    assert!(ed(&mut a).rec.open, "still in video mode");
    // Not in screenshot mode: F/W/R are the tools' keys there.
    key(&mut a, '1' as u32);
    key(&mut a, 'R' as u32);
    assert_eq!(ed(&mut a).tool, Some(Tool::Rect));
    // The redesign's tool keys there too (L line, E ellipse, X pixelate);
    // in video mode they draw nothing.
    for (k, t) in [('L', Tool::Line), ('E', Tool::Ellipse), ('X', Tool::Pixelate)] {
        key(&mut a, k as u32);
        assert_eq!(ed(&mut a).tool, Some(t), "{k}");
    }
    key(&mut a, '2' as u32);
    let before = ed(&mut a).tool;
    key(&mut a, 'L' as u32);
    assert_eq!(ed(&mut a).tool, before, "video mode: no tool change");
    key(&mut a, 'F' as u32);
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Screen, "F is the source again");
}

/// A custom area is a region: after Window (or Screen) a drag of a new
/// area, or moving or resizing the picked one, switches the Record
/// panel's source to Region; a click that picks a window keeps Window.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_dragged_area_makes_the_source_region() {
    let mut a = app(Some(r(300.0, 200.0, 400.0, 300.0)));
    key(&mut a, '2' as u32);
    let win = crate::snap::Node::new(crate::snap::R::new(120, 90, 1320, 810), vec![]);
    FAKE_SNAP.with(|s| *s.borrow_mut() = Some(crate::snap::Snapshot::build(&[win], (0, 0), (1440, 900))));
    mv(&mut a, 400, 300);
    key(&mut a, 'W' as u32);
    click(&mut a, 400, 300);
    assert_eq!(ed(&mut a).sel, Some(r(120.0, 90.0, 1200.0, 720.0)), "the window");
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Window, "a click keeps Window");
    // Resize the window's area by its bottom-right handle: a region.
    drag(&mut a, (1320, 810), (1000, 600));
    assert_eq!(ed(&mut a).sel, Some(r(120.0, 90.0, 880.0, 510.0)));
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Region);
    // Window again, then a new area dragged outside it.
    mv(&mut a, 400, 300);
    key(&mut a, 'W' as u32);
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Window);
    drag(&mut a, (20, 20), (100, 80));
    assert_eq!(ed(&mut a).sel, Some(r(20.0, 20.0, 80.0, 60.0)));
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Region, "a drag is a region");
    // Screen, then the monitor's area moved... it is the whole monitor
    // (nothing to move), so a press and release in it keeps Screen.
    key(&mut a, 'F' as u32);
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Screen);
    drag(&mut a, (700, 450), (700, 450));
    assert_eq!(ed(&mut a).rec.source, rec_ui::RecSource::Screen, "nothing changed");
}

/// A GIF: the sound rows say "Not recorded in GIF" and are disabled; the
/// countdown gives way to the frame rate (10 / 15 / 24) and a max length
/// (15 / 30 / 60 s) the recording stops at by itself.
#[cfg(windows)]
#[test]
fn gif_mode_rows_frame_rate_and_max_length() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 300.0)));
    a.kind = RunKind::Daemon;
    key(&mut a, '2' as u32);
    let seg = |a: &mut App, act: Act| item(a, |k| matches!(k, toolbar::Kind::Seg { act: x, .. } if *x == act));
    let (gx, gy) = seg(&mut a, Act::RecFormat(crate::rec::RecFormat::Gif));
    click(&mut a, gx, gy);
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let tb = ed(&mut a).toolbar.clone().unwrap();
    for act in [Act::RecMic, Act::RecSystem] {
        let row = tb.items.iter().find(|i| matches!(i.kind, toolbar::Kind::Row { act: x, .. } if x == act)).unwrap();
        assert!(row.disabled && matches!(row.kind, toolbar::Kind::Row { value: Some(rec_ui::NOT_IN_GIF), dim: true, .. }), "{act:?}");
        let sw = tb.items.iter().find(|i| matches!(i.kind, toolbar::Kind::Switch { act: x, .. } if x == act)).unwrap();
        assert!(sw.disabled && tb.act_at(Pt::new(sw.r.x + 10.0, sw.r.y + 10.0)).is_none());
    }
    assert!(!tb.items.iter().any(|i| matches!(i.kind, toolbar::Kind::Seg { act: Act::RecCountdown(_), .. })), "no countdown");
    let fps: Vec<Act> = tb.items.iter().filter_map(|i| match i.kind {
        toolbar::Kind::Seg { act: a @ Act::RecFps(_), .. } => Some(a),
        _ => None,
    }).collect();
    assert_eq!(fps, [Act::RecFps(10), Act::RecFps(15), Act::RecFps(24)]);
    let (fx, fy) = seg(&mut a, Act::RecFps(24));
    click(&mut a, fx, fy);
    let (lx, ly) = seg(&mut a, Act::RecGifMax(15));
    click(&mut a, lx, ly);
    assert_eq!((ed(&mut a).rec.opts.fps(), ed(&mut a).rec.opts.gif_max), (24, 15));
    // The estimate toast says the GIF's rate.
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let toast = ed(&mut a).scene.as_ref().unwrap().toast.clone().expect("the estimate");
    assert!(toast.text.starts_with("300p \u{b7} 24 fps \u{b7} est. "), "{}", toast.text);
    key(&mut a, key::RETURN);
    let spec = a.rec_spec.clone().unwrap();
    assert_eq!((spec.format, spec.fps, spec.mic.is_none(), spec.system_audio), (crate::rec::RecFormat::Gif, 24, true, false));
    // It stops by itself after 15 s of recording (paused time aside).
    let t = Instant::now();
    let run = a.rec.as_mut().unwrap();
    assert_eq!(run.phase(), rec_ui::Phase::Countdown);
    run.tick(t + Duration::from_secs(3));
    assert_eq!(run.phase(), rec_ui::Phase::Recording);
    run.tick(t + Duration::from_secs(17));
    assert_eq!(run.phase(), rec_ui::Phase::Recording, "14 s recorded");
    run.tick(t + Duration::from_secs(19));
    assert_eq!(run.phase(), rec_ui::Phase::Stopping, "15 s: auto-stop");
    assert_eq!(run.log.last(), Some(&rec_ui::Effect::Stop));
}

/// The microphone is metered only while the panel shows it on: opened
/// with the panel, its level drawn as bars (amber at the top when it
/// clips), released when the microphone, MP4 or video mode goes.
/// Video mode needs recording (`rec_ui::supported`: not on macOS in tests).
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn mic_meter_opens_with_the_panel_only() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 300.0)));
    ed(&mut a).rec.opts.mic = true;
    a.pump();
    assert!(ed(&mut a).rec.meter.is_none(), "screenshot mode: no microphone open");
    key(&mut a, '2' as u32);
    a.pump();
    assert_eq!(ed(&mut a).rec.meter.as_ref().map(|m| m.device().to_string()).as_deref(), Some(""), "the default microphone");
    ed(&mut a).rec.meter.as_ref().unwrap().set_peak(1.0);
    ed(&mut a).rec.meter_at = a_second_ago();
    a.pump();
    assert_eq!(ed(&mut a).rec.bars, (7, true), "loud: all bars, clipping");
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let tb = ed(&mut a).toolbar.clone().unwrap();
    assert!(tb.items.iter().any(|i| i.kind == toolbar::Kind::Meter { lit: 7, clip: true }));
    // Polled at 20 Hz: not before 50 ms.
    ed(&mut a).rec.meter.as_ref().unwrap().set_peak(0.0);
    ed(&mut a).rec.meter_at = Instant::now();
    a.pump();
    assert_eq!(ed(&mut a).rec.bars, (7, true));
    // Another microphone: reopened on it.
    ed(&mut a).rec.opts.mic_dev = "{mic-2}".into();
    a.pump();
    assert_eq!(ed(&mut a).rec.meter.as_ref().unwrap().device(), "{mic-2}");
    // GIF, microphone off, screenshot mode: released.
    ed(&mut a).rec.opts.format = crate::rec::RecFormat::Gif;
    a.pump();
    assert!(ed(&mut a).rec.meter.is_none());
    ed(&mut a).rec.opts.format = crate::rec::RecFormat::Mp4;
    a.pump();
    assert!(ed(&mut a).rec.meter.is_some());
    key(&mut a, '1' as u32);
    a.pump();
    assert!(ed(&mut a).rec.meter.is_none());
}

/// The camera in the panel: device list and shape; the bubble follows
/// the selection and is handed to the recording on Start.
#[cfg(windows)]
#[test]
fn record_panel_camera_bubble() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    a.kind = RunKind::Daemon;
    key(&mut a, '2' as u32);
    let cam = |a: &mut App| item(a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    let (cx, cy) = cam(&mut a);
    click(&mut a, cx, cy);
    let shown = ed(&mut a).rec.cam.shown.clone().expect("bubble shown");
    assert_eq!(shown.0, "", "the first camera");
    assert_eq!(shown.1, crate::cam::Look::new(crate::cam::Shape::Circle, crate::cam::CamSize::M));
    assert_eq!(shown.2, (100, 100, 600, 400), "anchored to the selection");
    // Pick the second camera from its list.
    let (bx, by) = item(&mut a, |k| matches!(k, toolbar::Kind::DevBox { dev: toolbar::Dev::Cam, .. }));
    click(&mut a, bx, by);
    assert_eq!(ed(&mut a).rec.dev_open, Some(toolbar::Dev::Cam));
    let (dx, dy) = item(&mut a, |k| matches!(k, toolbar::Kind::DevItem { dev: toolbar::Dev::Cam, i: 2, .. }));
    click(&mut a, dx, dy);
    assert_eq!(ed(&mut a).rec.opts.cam_dev, "{cam-2}");
    assert_eq!(ed(&mut a).rec.cam_label(), "USB Webcam");
    // Square.
    let (sx, sy) = item(&mut a, |k| matches!(k, toolbar::Kind::Seg { act: Act::RecShape(crate::cam::Shape::Rounded), .. }));
    click(&mut a, sx, sy);
    assert_eq!(ed(&mut a).rec.cam.shown.as_ref().unwrap().1.shape, crate::cam::Shape::Rounded);
    // Screenshot mode closes the bubble; video mode brings it back.
    key(&mut a, '1' as u32);
    assert!(ed(&mut a).rec.cam.shown.is_none());
    key(&mut a, '2' as u32);
    assert!(ed(&mut a).rec.cam.shown.is_some());
    let (tx, ty) = item(&mut a, |k| matches!(k, toolbar::Kind::Primary { .. }));
    click(&mut a, tx, ty);
    assert!(hidden(&a));
    let handed = a.rec_cam.clone().expect("the recording has the bubble");
    assert_eq!((handed.0.as_str(), handed.1.shape, handed.2), ("{cam-2}", crate::cam::Shape::Rounded, (100, 100, 600, 400)));
    let o = a.rec_opts.clone().unwrap();
    assert!(o.camera && o.cam_dev == "{cam-2}" && o.cam_shape == crate::cam::Shape::Rounded);
}

/// No camera: the Camera row is disabled and says why on hover; a config
/// with the camera on shows no bubble.
#[cfg(windows)]
#[test]
fn record_panel_without_a_camera() {
    rec_ui::FAKE_CAMS.with(|c| c.borrow_mut().clear());
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    ed(&mut a).rec.opts.camera = true;
    key(&mut a, '2' as u32);
    assert!(ed(&mut a).rec.open);
    assert!(ed(&mut a).rec.cam.shown.is_none(), "no bubble without a camera");
    let (cx, cy) = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    let tb = ed(&mut a).toolbar.clone().unwrap();
    let i = tb.index_at(Pt::new(cx as f32, cy as f32)).unwrap();
    assert!(tb.items[i].disabled);
    click(&mut a, cx, cy);
    assert!(ed(&mut a).rec.cam.shown.is_none());
    let row = tb.items.iter().find(|i| matches!(i.kind, toolbar::Kind::Row { act: Act::RecCamera, .. })).unwrap();
    assert!(matches!(row.kind, toolbar::Kind::Row { value: Some(crate::cam::NO_CAMERA), .. }), "the row says so");
    mv(&mut a, cx, cy);
    ed(&mut a).hover_at = Instant::now() - Duration::from_secs(2);
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let tip = ed(&mut a).scene.as_ref().and_then(|s| s.tip.as_ref()).map(|t| (t.label, t.below));
    assert_eq!(tip, Some((crate::cam::NO_CAMERA, true)));
    rec_ui::FAKE_CAMS.with(|c| *c.borrow_mut() = rec_ui::fake_cams());
}

/// A fake camera: solid pictures about 100 a second.
#[cfg(windows)]
struct SolidCam;

#[cfg(windows)]
impl crate::cam::CamSource for SolidCam {
    fn size(&self) -> (u32, u32) {
        (64, 48)
    }

    fn read(&mut self, into: &mut crate::cam::CamFrame) -> anyhow::Result<bool> {
        std::thread::sleep(Duration::from_millis(10));
        (into.w, into.h) = (64, 48);
        into.bgra.clear();
        into.bgra.extend_from_slice(&[40u8, 160, 90, 255].repeat(64 * 48));
        Ok(true)
    }
}

/// Bubbles in `a` open fake cameras; `fail` makes the camera fail with it
/// once opened.
#[cfg(windows)]
fn fake_camera(a: &mut App, fail: Option<&'static str>) {
    ed(a).rec.cam.fake = Some(Box::new(move |_dev: &str| -> crate::cam::Opener {
        Box::new(move || match fail {
            Some(e) => anyhow::bail!(e),
            None => Ok(Box::new(SolidCam) as Box<dyn crate::cam::CamSource>),
        })
    }));
}

#[cfg(windows)]
fn overlaps(a: capture::IRect, b: capture::IRect) -> bool {
    a.0 < b.0 + b.2 as i32 && b.0 < a.0 + a.2 as i32 && a.1 < b.1 + b.3 as i32 && b.1 < a.1 + a.3 as i32
}

/// The real bubble (a fake camera): opened with the camera switch, kept
/// off the panel and the mode switch, the grip hint under it; the wheel
/// over it steps its size (the panel follows), a drag shows it at 40 %
/// and the drop puts it in the nearest corner, 24 px in; another camera
/// reopens it, Start hands it to the recording.
#[cfg(windows)]
#[test]
fn record_panel_bubble_scroll_drag_and_handover() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEWHEEL};
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    a.kind = RunKind::Daemon;
    fake_camera(&mut a, None);
    key(&mut a, '2' as u32);
    let (cx, cy) = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    click(&mut a, cx, cy);
    assert_eq!(ed(&mut a).rec.cam.opened, 1);
    a.pump();
    let (pos, d) = {
        let e = ed(&mut a);
        let d = crate::cam::side(e.rec.opts.cam_size, 1.0);
        (e.rec.cam.bubble.as_ref().and_then(|b| b.pos()).unwrap(), d)
    };
    for t in ed(&mut a).toolbar_screen_rects() {
        assert!(!overlaps((pos.0, pos.1, d, d), t), "bubble {pos:?} over the panel {t:?}");
    }
    // The grip hint: centred under the bubble, 12 px below it.
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let grip = ed(&mut a).toolbar.as_ref().unwrap().items.iter().find(|i| i.kind == toolbar::Kind::Grip).expect("the grip hint").r;
    assert!((grip.x + grip.w / 2.0 - (pos.0 as f32 + d as f32 / 2.0)).abs() < 0.6 && (grip.y - (pos.1 + d as i32 + 12) as f32).abs() < 0.6, "{grip:?} {pos:?}");
    // The wheel: a size up, the panel's choice follows.
    let hwnd = ed(&mut a).rec.cam.bubble.as_ref().unwrap().hwnd();
    unsafe { SendMessageW(hwnd, WM_MOUSEWHEEL, Some(WPARAM((120usize) << 16)), Some(LPARAM(0))) };
    a.pump();
    assert_eq!(ed(&mut a).rec.opts.cam_size, crate::cam::CamSize::L, "160 → 200");
    assert_eq!(ed(&mut a).rec.cam.bubble.as_ref().unwrap().look().unwrap().size, crate::cam::CamSize::L);
    unsafe { SendMessageW(hwnd, WM_MOUSEWHEEL, Some(WPARAM(((-120i16 as u16) as usize) << 16)), Some(LPARAM(0))) };
    unsafe { SendMessageW(hwnd, WM_MOUSEWHEEL, Some(WPARAM(((-120i16 as u16) as usize) << 16)), Some(LPARAM(0))) };
    a.pump();
    assert_eq!(ed(&mut a).rec.opts.cam_size, crate::cam::CamSize::S, "down to 120");
    // A drag: see-through until dropped; the drop settles in a corner.
    unsafe { SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(LPARAM(0))) };
    assert!(ed(&mut a).rec.cam.bubble.as_ref().unwrap().dragging());
    a.pump();
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    assert!(!ed(&mut a).toolbar.as_ref().unwrap().items.iter().any(|i| i.kind == toolbar::Kind::Grip), "no grip while dragged");
    unsafe { SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(LPARAM(0))) };
    let b = ed(&mut a).rec.cam.bubble.as_ref().unwrap();
    assert!(!b.dragging());
    let p = b.pos().unwrap();
    let ds = crate::cam::side(crate::cam::CamSize::S, 1.0) as i32;
    assert_eq!(p, crate::cam::settle(p, (100, 100, 600, 400), ds, 1.0), "at rest in a corner");
    // Another camera: reopened; Start hands it over (no new camera).
    let (bx, by) = item(&mut a, |k| matches!(k, toolbar::Kind::DevBox { dev: toolbar::Dev::Cam, .. }));
    click(&mut a, bx, by);
    let (dx, dy) = item(&mut a, |k| matches!(k, toolbar::Kind::DevItem { dev: toolbar::Dev::Cam, i: 2, .. }));
    click(&mut a, dx, dy);
    assert_eq!(ed(&mut a).rec.cam.opened, 2);
    let (tx, ty) = item(&mut a, |k| matches!(k, toolbar::Kind::Primary { .. }));
    click(&mut a, tx, ty);
    assert!(hidden(&a));
    assert!(a.rec_bubble.is_some(), "the recording has the bubble");
    assert_eq!(a.rec_opts.as_ref().unwrap().cam_size, crate::cam::CamSize::S, "the wheel's size is remembered");
}

/// The camera fails once open: a toast says why, the camera switches off
/// and the bubble closes.
#[cfg(windows)]
#[test]
fn record_panel_camera_error_is_a_toast() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    fake_camera(&mut a, Some(crate::cam::ACCESS_OFF));
    key(&mut a, '2' as u32);
    let (cx, cy) = item(&mut a, |k| matches!(k, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
    click(&mut a, cx, cy);
    let t = Instant::now();
    while ed(&mut a).rec.opts.camera && t.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(10));
        a.pump();
    }
    let e = ed(&mut a);
    assert!(!e.rec.opts.camera, "the camera is switched off");
    assert!(e.rec.cam.bubble.is_none() && e.rec.cam.shown.is_none());
    assert_eq!(e.notice.as_ref().map(|n| n.text.as_str()), Some(crate::cam::ACCESS_OFF));
}

/// Enter in record mode after leaving video mode, with no countdown and
/// the camera on: the camera opens and the recording starts once its
/// first picture is in.
#[cfg(windows)]
#[test]
fn record_mode_enter_waits_for_the_first_picture() {
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    assert!(a.on_hot(HotEvent::Record));
    queue(Ok(synthetic_shot()));
    a.on_event(Ev::Timer);
    drag(&mut a, (200, 150), (520, 390));
    key(&mut a, '1' as u32);
    assert!(!ed(&mut a).rec.open);
    ed(&mut a).rec.opts.camera = true;
    ed(&mut a).rec.opts.countdown = false;
    fake_camera(&mut a, None);
    a.pump();
    assert_eq!(ed(&mut a).rec.cam.opened, 0, "no bubble outside video mode");
    key(&mut a, key::RETURN);
    assert!(hidden(&a));
    assert_eq!(rec_phase(&a), Some(rec_ui::Phase::Recording), "no countdown");
    assert_eq!(a.rec_bubble, Some(true), "the bubble shows a picture as the recording starts");
}

/// The cameras are listed on a worker: until they come the Camera row
/// says "Looking for cameras…", is disabled and has no tooltip; then it
/// is the Camera switch.
#[cfg(windows)]
#[test]
fn record_panel_lists_cameras_in_the_background() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    let (tx, rx) = std::sync::mpsc::channel();
    ed(&mut a).rec.cams_rx = Some(rx);
    key(&mut a, '2' as u32);
    assert!(ed(&mut a).rec.cams_looking());
    let row = |a: &mut App| {
        let m = a.mouse;
        ed(a).prepare(None, m, Instant::now());
        let tb = ed(a).toolbar.clone().unwrap();
        tb.items.into_iter().find(|i| matches!(i.kind, toolbar::Kind::Switch { act: Act::RecCamera, .. })).unwrap()
    };
    let it = row(&mut a);
    assert!(matches!(it.kind, toolbar::Kind::Switch { label: toolbar::LOOKING_CAMS, .. }) && it.disabled);
    assert!(animating(ed(&mut a), None, Instant::now()), "ticks until the list comes");
    let (cx, cy) = ((it.r.x + it.r.w / 2.0) as i32, (it.r.y + it.r.h / 2.0) as i32);
    mv(&mut a, cx, cy);
    ed(&mut a).hover_at = Instant::now() - Duration::from_secs(2);
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    assert!(ed(&mut a).scene.as_ref().unwrap().tip.is_none(), "no \"No camera found\" while looking");
    tx.send(rec_ui::fake_cams()).unwrap();
    a.pump();
    assert!(!ed(&mut a).rec.cams_looking() && ed(&mut a).rec.has_cams());
    let it = row(&mut a);
    assert!(matches!(it.kind, toolbar::Kind::Switch { label: "Camera", .. }) && !it.disabled);
}

// The record hotkey: idle → selection in record mode (popover open,
/// Enter records); again → stop; the end makes the daemon idle.
#[cfg(windows)]
#[test]
fn record_hotkey_selects_records_and_stops() {
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    a.cfg.rec_countdown = false;
    assert!(a.on_hot(HotEvent::Record));
    assert!(a.pending.as_ref().is_some_and(|p| p.record));
    queue(Ok(synthetic_shot()));
    a.on_event(Ev::Timer);
    assert!(ed(&mut a).rec.mode && ed(&mut a).rec.open);
    drag(&mut a, (200, 150), (520, 390));
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    assert!(ed(&mut a).toolbar.as_ref().is_some_and(|t| t.video), "the panel shows with the selection");
    key(&mut a, key::RETURN);
    assert!(hidden(&a));
    assert_eq!(a.rec_spec.as_ref().unwrap().area, (200, 150, 320, 240));
    assert_eq!(rec_phase(&a), Some(rec_ui::Phase::Recording), "no countdown");
    // A capture now would be recorded: ignored.
    assert!(a.on_hot(HotEvent::Capture));
    assert!(a.pending.is_none());
    // The update gate waits for the recording.
    let (tx, rx) = std::sync::mpsc::channel();
    a.on_hot(HotEvent::RestartWhenIdle(tx));
    a.idle_gate();
    assert!(rx.try_recv().is_err(), "recording: no reply");
    // The record hotkey again stops it; the file arrives; idle.
    assert!(a.on_hot(HotEvent::Record));
    assert_eq!(rec_phase(&a), Some(rec_ui::Phase::Stopping));
    assert_eq!(a.rec.as_ref().unwrap().log, [rec_ui::Effect::Go, rec_ui::Effect::Stop]);
    a.rec.as_mut().unwrap().fake_end = Some(rec_ui::End::Cancelled);
    a.pump();
    assert!(a.rec.is_none() && a.idle());
    assert_eq!(rx.try_recv(), Ok(()), "idle now: the update may install");
}

/// The tray's "Stop recording" stops it (and cancels a countdown); with
/// nothing recording it does nothing.
#[cfg(windows)]
#[test]
fn tray_stop_recording() {
    let mut a = app(Some(r(10.0, 10.0, 300.0, 200.0)));
    a.kind = RunKind::Daemon;
    assert!(a.on_hot(HotEvent::StopRecording));
    assert!(matches!(a.st, State::Edit(_)), "nothing to stop");
    ed(&mut a).rec.open = true;
    key(&mut a, key::RETURN);
    assert_eq!(rec_phase(&a), Some(rec_ui::Phase::Countdown));
    assert!(a.on_hot(HotEvent::StopRecording));
    assert_eq!(a.rec.as_ref().unwrap().log, [rec_ui::Effect::Cancel]);
    // A second stop (bar and tray at once) is one.
    assert!(a.on_hot(HotEvent::StopRecording));
    assert_eq!(a.rec.as_ref().unwrap().log.len(), 1);
}

/// The record hotkey while the overlay is open, or an update installing,
/// starts nothing; keyboard Record opens the popover only with a selection.
#[test]
fn record_hotkey_refused_while_busy() {
    let mut a = app(None);
    assert!(a.on_hot(HotEvent::Record));
    assert!(a.pending.is_none(), "overlay open");
    a.st = State::Hidden;
    a.updating = true;
    assert!(a.on_hot(HotEvent::Record));
    assert!(a.pending.is_none(), "update installing");
    a.updating = false;
    a.on_hot(HotEvent::Record);
    assert_eq!(a.pending.is_some(), rec_ui::supported());
    // A config reload forgets the popover's last choices.
    a.rec_opts = Some(rec_ui::RecOpts::from_config(&a.cfg));
    a.reload_config(a.cfg.clone());
    assert!(a.rec_opts.is_none());
}

/// A one-shot run (`rustshot gui`, no tasks) records too and exits once
/// the recording is over, like a capture: 0 saved ("saved: <path>"),
/// 1 failed, 2 cancelled; a recording that cannot start exits 1.
#[cfg(windows)]
#[test]
fn one_shot_recording_exits_with_its_result() {
    let saved = rec_ui::End::Saved { path: "C:/shots/a.mp4".into(), warnings: vec![] };
    for (end, want) in [(saved, 0), (rec_ui::End::Failed("disk full".into()), 1), (rec_ui::End::Cancelled, 2)] {
        let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
        assert!(a.one_shot());
        a.exit_code.store(-1, Ordering::SeqCst);
        ed(&mut a).rec.open = true;
        key(&mut a, key::RETURN);
        assert!(hidden(&a) && a.rec.is_some(), "recording");
        a.pump();
        assert_eq!(code(&a), -1, "still running: no exit yet");
        // The bar's Stop (no tray, no hotkeys in a one-shot run).
        a.rec.as_mut().unwrap().stop(Instant::now());
        a.rec.as_mut().unwrap().fake_end = Some(end.clone());
        a.pump();
        assert!(a.rec.is_none());
        assert_eq!(code(&a), want, "{end:?}");
    }
    let mut a = app(None);
    a.st = State::Hidden;
    a.exit_code.store(-1, Ordering::SeqCst);
    a.recording_started(Err(anyhow::anyhow!("no H.264 encoder is installed")));
    assert_eq!(code(&a), 1);
    assert!(a.rec.is_none());
}

/// A run with tasks (`gui --copy`) has no Record: neither the keymap's
/// `record` nor Enter (even in record mode) starts one.
#[cfg(windows)]
#[test]
fn record_is_unavailable_in_a_run_with_tasks() {
    let mut a = app(Some(r(100.0, 100.0, 600.0, 400.0)));
    ed(&mut a).tasks = vec![Task::Geometry];
    ed(&mut a).keys = Keymap::from_config(&[("record".to_string(), "Ctrl+R".to_string())].into_iter().collect()).0;
    key_m(&mut a, 'R' as u32, CTRL);
    assert!(!ed(&mut a).rec.open, "no popover");
    ed(&mut a).rec.mode = true;
    key(&mut a, key::RETURN);
    assert!(a.rec.is_none() && a.rec_spec.is_none(), "Enter ran the tasks instead");
    assert!(hidden(&a));
}

/// Enter in record mode with nothing selected records the monitor under
/// the pointer, not the whole desktop.
#[cfg(windows)]
#[test]
fn record_mode_enter_without_selection_records_the_monitor_under_the_pointer() {
    let mut a = quiet(preview_app_with(theme::DARK, None, two_monitor_shot()));
    a.kind = RunKind::Daemon;
    ed(&mut a).rec = RecPop::new(RecOpts::from_config(&a.cfg), true);
    mv(&mut a, 1500, 300);
    key(&mut a, key::RETURN);
    assert_eq!(a.rec_spec.as_ref().expect("recording").area, (1200, 0, 1200, 900));
}

/// The recording geometry carries each monitor's own UI scale.
#[test]
fn rec_geom_has_each_monitors_scale() {
    let mut a = quiet(preview_app_with(theme::DARK, Some(r(10.0, 300.0, 100.0, 100.0)), two_monitor_shot()));
    let mon = |x, y, w, h, scale| capture::MonInfo { x, y, w, h, scale, primary: false };
    let mons = [mon(0, 200, 1200, 700, 1.0), mon(1200, 0, 1200, 900, 2.0)];
    let g = ed(&mut a).rec_geom(&mons);
    assert_eq!(g.monitors, vec![((0, 200, 1200, 700), 1.0), ((1200, 0, 1200, 900), 2.0)]);
    assert_eq!(g.area_scale(), 1.0);
    // No system monitors (tests): the shot's scale.
    let g = ed(&mut a).rec_geom(&[]);
    assert!(g.monitors.iter().all(|m| m.1 == g.scale));
}

/// Without an MP4 encoder (Linux without ffmpeg) the popover records GIF,
/// whatever the config says, until one is found.
#[test]
fn without_mp4_the_popover_records_gif() {
    let mut opts = rec_ui::RecOpts::from_config(&Config::default());
    opts.format = crate::rec::RecFormat::Mp4;
    rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::No));
    let p = RecPop::new(opts.clone(), false);
    rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::Yes));
    assert_eq!(p.mp4, rec_ui::Mp4::No);
    assert_eq!(p.opts.format, crate::rec::RecFormat::Gif);
    let p = RecPop::new(opts, false);
    assert_eq!(p.mp4, rec_ui::Mp4::Yes);
    assert_eq!(p.opts.format, crate::rec::RecFormat::Mp4);
}

/// While ffmpeg is being asked the popover records GIF and MP4 waits;
/// once it answers yes, the configured MP4 is chosen again.
#[test]
fn while_mp4_is_checked_the_popover_waits_then_takes_the_config() {
    let mut opts = rec_ui::RecOpts::from_config(&Config::default());
    opts.format = crate::rec::RecFormat::Mp4;
    rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::Checking));
    let mut p = RecPop::new(opts.clone(), false);
    assert_eq!(p.mp4, rec_ui::Mp4::Checking);
    assert_eq!(p.opts.format, crate::rec::RecFormat::Gif);
    assert!(!p.poll_mp4(), "still asking");
    rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::Yes));
    assert!(p.poll_mp4());
    assert_eq!((p.mp4, p.opts.format), (rec_ui::Mp4::Yes, crate::rec::RecFormat::Mp4));
    assert!(!p.poll_mp4(), "once");
    // No MP4 after all: GIF stays.
    rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::Checking));
    let mut p = RecPop::new(opts, false);
    rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::No));
    assert!(p.poll_mp4());
    rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::Yes));
    assert_eq!((p.mp4, p.opts.format), (rec_ui::Mp4::No, crate::rec::RecFormat::Gif));
}

// ---- the redesign's global actions ------------------------------------------

/// Capture full screen: everything, saved at once (the shutter clicks
/// when it is on); Repeat last capture: that region again, saved; with
/// nothing to repeat it is a plain capture.
#[test]
fn full_screen_and_repeat_last_save_at_once() {
    let dir = TempDir::new("full");
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    a.cfg.save_path = dir.0.display().to_string();
    a.cfg.save_subfolder = false;
    a.cfg.shutter_sound = true;
    crate::sound::take_played();
    assert!(a.on_hot(HotEvent::RepeatLast));
    assert!(a.pending.as_ref().is_some_and(|p| !p.accept_on_select && p.region.is_none()), "nothing to repeat yet");
    a.pending = None;
    assert!(a.on_hot(HotEvent::Pressed(3)), "the full-screen slot");
    queue(Ok(synthetic_shot()));
    a.on_event(Ev::Timer);
    assert!(hidden(&a), "saved without the overlay staying up");
    assert_eq!(a.last_region, Some((0, 0, 1440, 900)));
    assert_eq!(crate::sound::take_played(), 1);
    let saved: Vec<_> = std::fs::read_dir(&dir.0).unwrap().flatten().collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    // A region, then repeat it.
    a.last_region = Some((100, 50, 320, 240));
    a.cfg.shutter_sound = false;
    a.on_hot(HotEvent::Pressed(5));
    let p = a.pending.as_ref().unwrap();
    assert_eq!((p.region, p.accept_on_select), (Some((100, 50, 320, 240)), true));
    assert!(matches!(p.tasks[..], [Task::Save { path: None, ask: false }]));
    queue(Ok(synthetic_shot()));
    a.on_event(Ev::Timer);
    assert!(hidden(&a));
    assert_eq!(a.last_region, Some((100, 50, 320, 240)));
    assert_eq!(crate::sound::take_played(), 0, "sound off");
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}

/// Capture window: snapping on for that capture even with
/// `snap_windows` off.
#[test]
fn capture_window_forces_snapping() {
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    a.cfg.snap_windows = false;
    a.on_hot(HotEvent::Pressed(4));
    assert!(a.pending.as_ref().is_some_and(|p| p.pick_window));
    queue(Ok(synthetic_shot()));
    a.on_event(Ev::Timer);
    assert!(ed(&mut a).cfg.snap_windows, "this capture snaps");
    assert!(!a.cfg.snap_windows, "the setting is unchanged");
}

/// A global hotkey pressed in the overlay is the overlay's key: the
/// capture chord set to Ctrl+Shift+S is Save As there; Quit still quits.
#[test]
fn global_chords_reach_the_overlay() {
    let mut a = app(Some(r(10.0, 10.0, 200.0, 100.0)));
    a.kind = RunKind::Daemon;
    assert_eq!(a.cfg.capture_hotkey, "Shift+Meta+X", "the default");
    a.cfg.capture_hotkey = "Ctrl+Shift+S".into();
    // A chord bound in the overlay ("]" = size up) does its overlay job.
    ed(&mut a).tool = Some(Tool::Rect);
    let before = ed(&mut a).mem.size(Tool::Rect);
    a.cfg.capture_full_hotkey = "]".into();
    a.on_hot(HotEvent::Pressed(3));
    assert!(matches!(a.st, State::Edit(_)) && a.pending.is_none(), "no capture over the open one");
    assert!(ed(&mut a).mem.size(Tool::Rect) > before, "the overlay got ]");
    // Ctrl+Shift+S: Save As (a Save that asks).
    a.on_hot(HotEvent::Pressed(0));
    let State::Finish(job) = &a.st else { panic!("Save As ran") };
    assert!(matches!(job.tasks[..], [Task::Save { path: None, ask: true }]), "{:?}", job.tasks);
    let mut a = app(Some(r(10.0, 10.0, 200.0, 100.0)));
    a.kind = RunKind::Daemon;
    assert!(!a.on_hot(HotEvent::Pressed(1)), "Quit quits from the overlay too");
}

/// Copy to clipboard after capture: a daemon save also copies.
#[test]
fn copy_after_capture_joins_the_save() {
    let mut a = app(Some(r(10.0, 10.0, 200.0, 100.0)));
    a.kind = RunKind::Daemon;
    ed(&mut a).cfg.copy_after_capture = true;
    ed(&mut a).cfg.save_dialog = false;
    // Stop before the export runs (it would write and copy).
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    e.tasks = vec![Task::Save { path: None, ask: false }];
    a.finish(&mut e, false);
    let State::Finish(job) = &a.st else { panic!("finish job") };
    assert_eq!(job.tasks, vec![Task::Save { path: None, ask: false }, Task::Copy]);
}
