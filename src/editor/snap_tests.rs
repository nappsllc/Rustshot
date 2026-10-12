//! Window snapping in the overlay: hovering outlines the window (or popup,
//! or part after Tab) under the pointer with its size, a click selects it,
//! a drag still draws, Tab / Shift+Tab step in and out, the Tab hint until
//! it is used, the desktop as the monitor, and repaints that cover every
//! changed pixel; plus the preview PNGs.

use super::tests::{assert_dirty_covers, edit_of, preview_app, preview_app_with, settled, two_monitor_shot, synthetic_shot};
use super::*;
use crate::snap::{Node, Snapshot, R};
use crate::wind::key;

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };

fn fr(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

fn n(x: i32, y: i32, w: i32, h: i32, kids: Vec<Node>) -> Node {
    Node::new(R::xywh(x, y, w, h), kids)
}

/// The synthetic desktop's windows: a menu popup in front, then the app
/// window (title bar, sidebar with items, content with three cards).
fn windows() -> Vec<Node> {
    let items: Vec<Node> = (0..8).map(|i| n(140, 160 + i * 40, 180, 22, vec![])).collect();
    let sidebar = n(120, 130, 220, 680, items);
    let cards = vec![n(370, 540, 330, 220, vec![]), n(740, 540, 320, 220, vec![]), n(1100, 540, 190, 220, vec![])];
    let content = n(340, 130, 980, 680, cards);
    let title = n(120, 90, 1200, 40, vec![]);
    let app = n(120, 90, 1200, 720, vec![title, sidebar, content]);
    let menu = n(1000, 120, 220, 260, vec![]);
    vec![menu, app]
}

fn snapshot(shot: &Shot) -> Snapshot {
    Snapshot::build(&windows(), shot.origin, shot.size)
}

/// The preview editor with nothing selected and the windows listed.
fn app() -> App {
    let mut a = preview_app(theme::DARK, None);
    a.focus_tries = u8::MAX;
    let e = edit_of(&mut a);
    e.snap = Some(snapshot(&e.shot));
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
fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

/// What the overlay outlines (and shows plain) now.
fn outlined(a: &mut App) -> Option<FRect> {
    let m = a.mouse;
    let e = ed(a);
    e.prepare(None, m, Instant::now());
    let sc = e.scene.as_ref().unwrap();
    let r = sc.snap.as_ref().map(|s| s.r);
    assert_eq!(sc.sel, e.sel.or(r), "the backdrop is plain inside the outline");
    r
}

const APP: FRect = FRect { x: 120.0, y: 90.0, w: 1200.0, h: 720.0 };
const MENU: FRect = FRect { x: 1000.0, y: 120.0, w: 220.0, h: 260.0 };
const SIDEBAR: FRect = FRect { x: 120.0, y: 130.0, w: 220.0, h: 680.0 };

#[test]
fn hover_outlines_the_window_under_the_pointer() {
    let mut a = app();
    mv(&mut a, 700, 300);
    assert_eq!(outlined(&mut a), Some(APP));
    assert_eq!(ed(&mut a).sel, None, "nothing selected yet");
    // The popup on its own, over the window.
    mv(&mut a, 1100, 200);
    assert_eq!(outlined(&mut a), Some(MENU));
    // The desktop: the monitor (the "whole screen").
    mv(&mut a, 40, 40);
    assert_eq!(outlined(&mut a), Some(fr(0.0, 0.0, 1440.0, 900.0)));
    // The badge says the size, above the window.
    mv(&mut a, 700, 300);
    outlined(&mut a);
    let e = ed(&mut a);
    let sc = e.scene.as_ref().unwrap();
    let badge = *sc.rects(compose::El::Snap).last().unwrap();
    assert!(badge.y0 < 90 - 20 && badge.x0 <= 120, "above the window's top-left: {badge:?}");
}

#[test]
fn a_click_selects_exactly_the_window_and_it_adjusts_afterwards() {
    let mut a = app();
    mv(&mut a, 700, 300);
    down(&mut a, 700, 300);
    assert_eq!(outlined(&mut a), Some(APP), "still outlined while pressed");
    up(&mut a, 700, 300);
    assert_eq!(ed(&mut a).sel, Some(APP));
    assert_eq!(outlined(&mut a), None, "a selection: no more snapping");
    assert!(ed(&mut a).toolbar.is_some(), "the toolbar as for a drawn selection");
    // Arrows and handles adjust it.
    key_m(&mut a, key::RIGHT, NONE);
    assert_eq!(ed(&mut a).sel, Some(fr(121.0, 90.0, 1200.0, 720.0)));
    down(&mut a, 1321, 810);
    mv(&mut a, 1300, 800);
    up(&mut a, 1300, 800);
    assert_eq!(ed(&mut a).sel, Some(fr(121.0, 90.0, 1179.0, 710.0)), "the SE handle");
    // A popup click selects the popup.
    let mut a = app();
    mv(&mut a, 1100, 200);
    down(&mut a, 1100, 200);
    up(&mut a, 1100, 200);
    assert_eq!(ed(&mut a).sel, Some(MENU));
}

#[test]
fn a_wobble_under_4_px_is_a_click_and_a_drag_draws_its_own_rect() {
    let mut a = app();
    mv(&mut a, 700, 300);
    down(&mut a, 700, 300);
    mv(&mut a, 702, 302); // 2.8 px
    mv(&mut a, 703, 300); // 3 px
    up(&mut a, 703, 300);
    assert_eq!(ed(&mut a).sel, Some(APP), "still a click");
    let mut a = app();
    mv(&mut a, 700, 300);
    down(&mut a, 700, 300);
    mv(&mut a, 704, 300);
    assert_eq!(outlined(&mut a), None, "dragging: no snapping");
    mv(&mut a, 900, 450);
    up(&mut a, 900, 450);
    assert_eq!(ed(&mut a).sel, Some(fr(700.0, 300.0, 200.0, 150.0)), "the dragged rect, not the window");
}

#[test]
fn tab_steps_into_the_parts_under_the_pointer_and_shift_tab_back_out() {
    let mut a = app();
    mv(&mut a, 200, 205); // over the second sidebar item
    assert_eq!(outlined(&mut a), Some(APP));
    key_m(&mut a, key::TAB, NONE);
    assert_eq!(outlined(&mut a), Some(SIDEBAR));
    key_m(&mut a, key::TAB, NONE);
    assert_eq!(outlined(&mut a), Some(fr(140.0, 200.0, 180.0, 22.0)), "the item");
    key_m(&mut a, key::TAB, NONE);
    assert_eq!(outlined(&mut a), Some(fr(140.0, 200.0, 180.0, 22.0)), "nothing deeper");
    // Moving over the content keeps the depth (one part in).
    key_m(&mut a, key::TAB, SHIFT);
    assert_eq!(outlined(&mut a), Some(SIDEBAR));
    mv(&mut a, 800, 300);
    assert_eq!(outlined(&mut a), Some(fr(340.0, 130.0, 980.0, 680.0)), "the content pane");
    mv(&mut a, 800, 600);
    assert_eq!(outlined(&mut a), Some(fr(340.0, 130.0, 980.0, 680.0)), "over a card: still one part in");
    key_m(&mut a, key::TAB, NONE);
    assert_eq!(outlined(&mut a), Some(fr(740.0, 540.0, 320.0, 220.0)), "the card");
    mv(&mut a, 200, 215);
    assert_eq!(outlined(&mut a), Some(fr(140.0, 200.0, 180.0, 22.0)), "as deep as the sidebar goes");
    key_m(&mut a, key::TAB, SHIFT);
    // Another window starts from the whole window.
    mv(&mut a, 1100, 200);
    assert_eq!(outlined(&mut a), Some(MENU));
    key_m(&mut a, key::TAB, NONE);
    assert_eq!(outlined(&mut a), Some(MENU), "a popup has no parts");
    mv(&mut a, 800, 300);
    assert_eq!(outlined(&mut a), Some(APP));
    key_m(&mut a, key::TAB, SHIFT);
    assert_eq!(outlined(&mut a), Some(APP), "Shift+Tab at the window stays");
    // A click selects the part outlined.
    mv(&mut a, 200, 205);
    key_m(&mut a, key::TAB, NONE);
    down(&mut a, 200, 205);
    up(&mut a, 200, 205);
    assert_eq!(ed(&mut a).sel, Some(SIDEBAR));
    // Tab with a selection does nothing to it; Esc still cancels.
    key_m(&mut a, key::TAB, NONE);
    assert_eq!(ed(&mut a).sel, Some(SIDEBAR));
    key_m(&mut a, key::ESCAPE, NONE);
    assert!(matches!(a.st, State::Hidden), "cancelled");
    let mut a = app();
    mv(&mut a, 200, 205);
    key_m(&mut a, key::TAB, NONE);
    key_m(&mut a, key::ESCAPE, NONE);
    assert!(matches!(a.st, State::Hidden), "Esc cancels while snapping too");
}

#[test]
fn the_tab_hint_shows_until_tab_is_first_used() {
    let mut a = app();
    let hint = |a: &mut App| {
        outlined(a);
        ed(a).scene.as_ref().unwrap().snap.as_ref().map(|s| s.hint)
    };
    mv(&mut a, 1100, 200);
    assert_eq!(hint(&mut a), Some(false), "a popup has no parts: no hint");
    mv(&mut a, 700, 300);
    assert_eq!(hint(&mut a), Some(true));
    // The badge grows to hold it.
    let wide = *ed(&mut a).scene.as_ref().unwrap().rects(compose::El::Snap).last().unwrap();
    key_m(&mut a, key::TAB, NONE);
    assert_eq!(hint(&mut a), Some(false), "used once: gone");
    assert!(!ed(&mut a).cfg.snap_tab_hint);
    assert_eq!(ed(&mut a).pending.get("snap_tab_hint"), Some(&Some("false".to_string())), "written with the tools' settings");
    key_m(&mut a, key::TAB, SHIFT);
    assert_eq!(hint(&mut a), Some(false));
    let narrow = *ed(&mut a).scene.as_ref().unwrap().rects(compose::El::Snap).last().unwrap();
    assert!(narrow.w() < wide.w());
    // The next capture remembers it.
    key_m(&mut a, key::ESCAPE, NONE);
    assert!(!a.cfg.snap_tab_hint, "copied back like the tools' settings");
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    FAKE_SNAP.with(|g| *g.borrow_mut() = Some(snapshot(&synthetic_shot())));
    a.pending = Some(Pending::editor());
    a.mouse = (700, 300);
    a.on_event(Ev::Timer);
    assert_eq!(hint(&mut a), Some(false));
}

#[test]
fn a_capture_lists_the_windows_once_and_outlines_the_one_under_the_pointer() {
    let mut a = preview_app(theme::DARK, None);
    a.st = State::Hidden;
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    FAKE_SNAP.with(|g| *g.borrow_mut() = Some(snapshot(&synthetic_shot())));
    a.pending = Some(Pending::editor());
    a.mouse = (700, 300);
    a.on_event(Ev::Timer);
    assert!(FAKE_SNAP.with(|g| g.borrow().is_none()), "taken");
    assert_eq!(outlined(&mut a), Some(APP), "before the pointer moves");
    // Off in the config: no snapshot, a click selects everything.
    let mut a = preview_app(theme::DARK, None);
    a.st = State::Hidden;
    a.cfg.snap_windows = false;
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    FAKE_SNAP.with(|g| *g.borrow_mut() = Some(snapshot(&synthetic_shot())));
    a.pending = Some(Pending::editor());
    a.on_event(Ev::Timer);
    FAKE_SNAP.with(|g| *g.borrow_mut() = None);
    assert!(ed(&mut a).snap.is_none());
    mv(&mut a, 700, 300);
    assert_eq!(outlined(&mut a), None);
    down(&mut a, 700, 300);
    up(&mut a, 700, 300);
    assert_eq!(ed(&mut a).sel, Some(fr(0.0, 0.0, 1440.0, 900.0)));
    // A capture with a region given never lists windows.
    let mut a = preview_app(theme::DARK, None);
    a.st = State::Hidden;
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    FAKE_SNAP.with(|g| *g.borrow_mut() = Some(snapshot(&synthetic_shot())));
    let mut p = Pending::editor();
    p.region = Some((10, 10, 300, 200));
    a.pending = Some(p);
    a.on_event(Ev::Timer);
    assert!(ed(&mut a).snap.is_none());
    FAKE_SNAP.with(|g| *g.borrow_mut() = None);
}

#[test]
fn no_snapping_with_a_tool_and_clicks_on_the_desktop_take_the_monitor() {
    let mut a = app();
    mv(&mut a, 700, 300);
    key_m(&mut a, 'R' as u32, NONE);
    assert_eq!(outlined(&mut a), None, "a tool is drawing on the whole screen");
    // Two monitors: the desktop is the monitor under the pointer.
    let shot = two_monitor_shot();
    let mut a = preview_app_with(theme::LIGHT, None, shot.clone());
    ed(&mut a).snap = Some(Snapshot::build(&[n(1300, 100, 600, 400, vec![])], shot.origin, shot.size));
    mv(&mut a, 300, 500);
    assert_eq!(outlined(&mut a), Some(fr(0.0, 200.0, 1200.0, 700.0)));
    mv(&mut a, 1250, 800);
    assert_eq!(outlined(&mut a), Some(fr(1200.0, 0.0, 1200.0, 900.0)));
    mv(&mut a, 1500, 300);
    assert_eq!(outlined(&mut a), Some(fr(1300.0, 100.0, 600.0, 400.0)));
    mv(&mut a, 300, 500);
    down(&mut a, 300, 500);
    up(&mut a, 300, 500);
    assert_eq!(ed(&mut a).sel, Some(fr(0.0, 200.0, 1200.0, 700.0)), "the monitor");
}

/// Hover changes repaint only what changed: the old and new outlines and
/// badges and the area whose dimming flips; never the whole frame.
#[test]
fn hover_repaints_cover_every_changed_pixel() {
    for th in [theme::DARK, theme::LIGHT] {
        let mut a = app();
        ed(&mut a).th = th;
        // Snapping alone (the magnifier and the hint have their own tests).
        ed(&mut a).cfg.magnifier = false;
        ed(&mut a).assist.dismiss_hint(Instant::now());
        mv(&mut a, 700, 300);
        settled(&mut a);
        // The window's dimming flips: most of the frame, inherently.
        assert_dirty_covers(&mut a, "to the popup", |a| mv(a, 1100, 200));
        let f = assert_dirty_covers(&mut a, "same popup", |a| mv(a, 1110, 210));
        assert_eq!(f, 0.0, "nothing changed");
        assert_dirty_covers(&mut a, "back to the window", |a| mv(a, 200, 205));
        assert_dirty_covers(&mut a, "tab in", |a| key_m(a, key::TAB, NONE));
        let f = assert_dirty_covers(&mut a, "tab deeper", |a| key_m(a, key::TAB, NONE));
        assert!(f < 0.2, "the sidebar to its item: about the sidebar ({f})");
        assert_dirty_covers(&mut a, "the next item", |a| mv(a, 200, 245));
        assert_dirty_covers(&mut a, "to the desktop", |a| mv(a, 30, 30));
        assert_dirty_covers(&mut a, "pressed", |a| down(a, 30, 30));
        assert_dirty_covers(&mut a, "clicked: the monitor selected", |a| up(a, 30, 30));
    }
}

/// The hover frame composes byte-exactly through any tiling (the GDI
/// path paints it in rects).
#[test]
fn hover_frames_tile_exactly() {
    let mut a = app();
    mv(&mut a, 200, 205);
    key_m(&mut a, key::TAB, NONE);
    let want = settled(&mut a);
    super::tests::assert_tiles_match(&mut a, &want, "snap part", 0x51ED_270B);
}

#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_snap_preview_pngs() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
        let shot = |a: &mut App, name: &str| {
            let png = settled(a).to_png().unwrap();
            std::fs::write(dir.join(format!("snap-{tn}-{name}.png")), png).unwrap();
        };
        let mut a = app();
        ed(&mut a).th = th;
        mv(&mut a, 700, 300);
        shot(&mut a, "window");
        mv(&mut a, 1100, 200);
        shot(&mut a, "popup");
        mv(&mut a, 200, 205);
        key_m(&mut a, key::TAB, NONE);
        shot(&mut a, "part");
        key_m(&mut a, key::TAB, NONE);
        shot(&mut a, "part-deeper");
        mv(&mut a, 30, 30);
        shot(&mut a, "desktop");
        // The badge with its Tab hint (first time), and a window at the
        // top of the screen (badge inside).
        let mut a = app();
        ed(&mut a).th = th;
        ed(&mut a).snap = Some(Snapshot::build(
            &[n(0, 0, 1440, 860, vec![n(0, 0, 1440, 48, vec![]), n(0, 48, 300, 812, vec![])])],
            (0, 0),
            (1440, 900),
        ));
        mv(&mut a, 700, 400);
        shot(&mut a, "maximised-badge-inside");
        let mut a = app();
        ed(&mut a).th = th;
        ed(&mut a).cfg.snap_tab_hint = false;
        mv(&mut a, 700, 300);
        shot(&mut a, "window-no-hint");
        // After the click.
        down(&mut a, 700, 300);
        up(&mut a, 700, 300);
        shot(&mut a, "clicked");
    }
}
