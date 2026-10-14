//! The selection helpers: magnifier sampling, caption and placement, the
//! crosshair, H copying the color, the color ring's hit tests and flows,
//! accept on select (Shift skips it, window clicks, the CLI's pending),
//! the hint card's keys and dismissal, repaints that cover every changed
//! pixel, the preview PNGs and the 4K pointer-move bench.

use super::super::tests::{assert_dirty_covers, assert_tiles_match, edit_of, preview_app, preview_app_with, settled, synthetic_shot};
use super::*;
use crate::wind::key;

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
const ALT: Mods = Mods { shift: false, ctrl: false, alt: true };

fn fr(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

fn app(sel: Option<FRect>) -> App {
    let mut a = preview_app(theme::DARK, sel);
    a.focus_tries = u8::MAX;
    a
}

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
}

fn mv(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Move { x, y });
}

fn rdown(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Right { down: true, x, y });
}

fn rup(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Right { down: false, x, y });
}

fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

/// Drags see `m` held until the guard drops.
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

fn scene(a: &mut App) -> &compose::Scene {
    a.frame();
    ed(a).scene.as_ref().expect("scene")
}

fn pixel(e: &Edit, x: u32, y: u32) -> C4 {
    let i = (y as usize * e.base.width() as usize + x as usize) * 4;
    let p = &e.base.as_raw()[i..i + 4];
    C4::rgb(p[0], p[1], p[2])
}

// ---- magnifier geometry ----------------------------------------------------

#[test]
fn magnifier_is_140_px_of_17_pixels_at_8x() {
    let (cell, rim) = mag_geom(1.0);
    assert_eq!((cell, rim), (8.0, 2.0));
    assert_eq!(cell * MAG_N as f32 + 2.0 * rim, 140.0);
    let (cell, rim) = mag_geom(1.5);
    assert_eq!((cell, rim), (12.0, 3.0));
    assert_eq!(cell * MAG_N as f32 + 2.0 * rim, 210.0);
    assert!(mag_geom(1.0).0 >= MAG_GRID_AT, "grid lines at 8x");
    assert!(mag_geom(0.5).0 < MAG_GRID_AT, "none below 6x");
}

#[test]
fn magnifier_flips_to_stay_on_screen_and_off_the_pointer() {
    let area = fr(0.0, 0.0, 1440.0, 900.0);
    let (r, cap) = mag_rect(Pt::new(100.0, 100.0), area, 1.0, 120.0, &[]);
    assert_eq!((r.x, r.y, r.w, r.h), (124.0, 124.0, 140.0, 140.0), "below right, 24 px off");
    assert_eq!((cap.y, cap.h), (124.0 + 140.0 + 6.0, 22.0), "the caption under it");
    assert_eq!(cap.x + cap.w / 2.0, r.x + r.w / 2.0);
    // Right edge: left of the pointer; bottom: above it.
    let (r, _) = mag_rect(Pt::new(1430.0, 100.0), area, 1.0, 120.0, &[]);
    assert!(r.x1() <= 1430.0 - 24.0 && r.y > 100.0);
    let (r, cap) = mag_rect(Pt::new(100.0, 890.0), area, 1.0, 120.0, &[]);
    assert!(r.x > 100.0 && cap.y1() <= 890.0 - 24.0);
    let (r, cap) = mag_rect(Pt::new(1439.0, 899.0), area, 1.0, 120.0, &[]);
    assert!(r.x1() < 1439.0 && cap.y1() < 899.0, "the corner: up and left");
    // Everywhere: on the monitor, never over the pointer.
    for y in (0..900).step_by(37) {
        for x in (0..1440).step_by(41) {
            let p = Pt::new(x as f32 + 0.5, y as f32 + 0.5);
            for s in [1.0, 1.5, 2.0] {
                let (r, cap) = mag_rect(p, area, s, 130.0 * s, &[]);
                for b in [r, cap] {
                    assert!(b.x >= 0.0 && b.y >= 0.0 && b.x1() <= 1440.0 && b.y1() <= 900.0, "{p:?} {s}: {b:?} off screen");
                    assert!(!toolbar::hit(b, p), "{p:?} {s}: {b:?} covers the pointer");
                }
            }
        }
    }
    // A second monitor: it stays on the one under the pointer.
    let right = fr(1440.0, 0.0, 1920.0, 1080.0);
    let (r, _) = mag_rect(Pt::new(1450.0, 500.0), right, 1.0, 120.0, &[]);
    assert!(r.x >= 1440.0);
    // Off the toolbar when another side has room.
    let bar = fr(620.0, 330.0, 400.0, 44.0);
    let (r, cap) = mag_rect(Pt::new(600.0, 300.0), area, 1.0, 120.0, &[bar]);
    let both = toolbar::union(r, cap);
    assert!(both.x1() <= bar.x || both.x >= bar.x1() || both.y1() <= bar.y || both.y >= bar.y1(), "{both:?} over the bar");
}

#[test]
fn caption_and_hex() {
    assert_eq!(mag_text(12, 345, C4::rgb(10, 11, 255)), "12, 345 \u{b7} #0a0bff");
    assert_eq!(hex(C4::new(255, 0, 128, 7)), "#ff0080");
}

// ---- magnifier in the overlay ---------------------------------------------

#[test]
fn magnifier_samples_the_pixels_around_the_pointer() {
    let mut a = app(None);
    mv(&mut a, 400, 600);
    let sc = scene(&mut a).clone();
    let m = sc.mag.as_ref().expect("magnifier before a selection");
    let e = ed(&mut a);
    let want = pixel(e, 400, 600);
    assert_eq!(m.px.len(), MAG_N * MAG_N);
    assert_eq!(m.px[MAG_N * MAG_N / 2], want, "the middle is the pointer's pixel");
    assert_eq!(m.px[0], pixel(e, 400 - 8, 600 - 8), "row by row from the top left");
    assert_eq!(m.text, mag_text(400, 600, want));
    assert!(!m.square, "round by default");
    let c = sc.cross.clone().expect("crosshair");
    assert_eq!((c.x, c.y, c.lw), (400, 600, 1));
    // Off the image: black.
    let px = ed(&mut a).pixels_around((0, 0), MAG_N);
    assert_eq!(px[0], C4::rgb(0, 0, 0));
    assert_eq!(px[MAG_N * MAG_N / 2], pixel(ed(&mut a), 0, 0));
    // Square when set.
    ed(&mut a).cfg.magnifier_square = true;
    mv(&mut a, 401, 600);
    assert!(scene(&mut a).mag.as_ref().unwrap().square);
}

#[test]
fn magnifier_shows_while_selecting_only() {
    let sel = fr(200.0, 150.0, 500.0, 300.0);
    // Before a selection: yes; turned off: no.
    let mut a = app(None);
    mv(&mut a, 300, 300);
    assert!(scene(&mut a).mag.is_some());
    ed(&mut a).cfg.magnifier = false;
    mv(&mut a, 301, 300);
    let sc = scene(&mut a);
    assert!(sc.mag.is_none() && sc.cross.is_none());
    // Drawing the region: yes, following the pointer.
    let mut a = app(None);
    a.on_event(Ev::Down { x: 100, y: 100 });
    mv(&mut a, 300, 250);
    let sc = scene(&mut a);
    assert_eq!(sc.cross.as_ref().map(|c| (c.x, c.y)), Some((300, 250)));
    a.on_event(Ev::Up { x: 300, y: 250 });
    assert!(scene(&mut a).mag.is_none(), "a selection, idle: the toolbar instead");
    // Resizing it: yes; moving it: no.
    let mut a = app(Some(sel));
    a.on_event(Ev::Down { x: 700, y: 450 });
    mv(&mut a, 720, 470);
    assert!(scene(&mut a).mag.is_some(), "resize by the SE handle");
    a.on_event(Ev::Up { x: 720, y: 470 });
    a.on_event(Ev::Down { x: 400, y: 300 });
    mv(&mut a, 420, 310);
    assert!(scene(&mut a).mag.is_none(), "move");
    a.on_event(Ev::Up { x: 420, y: 310 });
    // Annotating: no.
    let mut a = app(Some(sel));
    ed(&mut a).tool = Some(Tool::Rect);
    a.on_event(Ev::Down { x: 300, y: 200 });
    mv(&mut a, 350, 260);
    assert!(scene(&mut a).mag.is_none(), "drawing a rectangle");
    // A tool before any selection: no (a press would draw).
    let mut a = app(None);
    ed(&mut a).tool = Some(Tool::Path);
    mv(&mut a, 300, 300);
    assert!(scene(&mut a).mag.is_none());
}

#[test]
fn h_copies_the_color_under_the_pointer() {
    let mut a = app(None);
    mv(&mut a, 500, 600);
    key_m(&mut a, 'H' as u32, NONE);
    let want = hex(pixel(ed(&mut a), 500, 600));
    let e = ed(&mut a);
    assert_eq!(e.assist.copied.as_deref(), Some(want.as_str()));
    let t = e.notice.as_ref().expect("toast");
    assert_eq!(t.text, format!("Copied {want}"));
    // A selection, idle: no magnifier, nothing copied.
    let mut a = app(Some(fr(10.0, 10.0, 300.0, 200.0)));
    mv(&mut a, 500, 600);
    key_m(&mut a, 'H' as u32, NONE);
    assert!(ed(&mut a).assist.copied.is_none());
    // Remapped: the new key copies, H is free.
    let mut a = app(None);
    ed(&mut a).keys = Keymap::from_config(&[("copy_color".to_string(), "J".to_string())].into_iter().collect()).0;
    mv(&mut a, 500, 600);
    key_m(&mut a, 'H' as u32, NONE);
    assert!(ed(&mut a).assist.copied.is_none());
    key_m(&mut a, 'J' as u32, NONE);
    assert!(ed(&mut a).assist.copied.is_some());
}

#[test]
fn magnifier_repaints_cover_every_changed_pixel() {
    let mut a = app(None);
    mv(&mut a, 300, 300);
    // The magnifier alone (the hint card's fade has its own test).
    ed(&mut a).assist.dismiss_hint(Instant::now());
    settled(&mut a);
    for (i, (x, y)) in [(305, 302), (900, 700), (1430, 890), (5, 5), (700, 450)].into_iter().enumerate() {
        let share = assert_dirty_covers(&mut a, &format!("hover {i}"), |a| mv(a, x, y));
        assert!(share < 0.15, "hover {i}: {share} of the image repainted");
    }
    let want = settled(&mut a);
    assert_tiles_match(&mut a, &want, "magnifier", 7);
    // While drawing the region.
    a.on_event(Ev::Down { x: 100, y: 100 });
    for (x, y) in [(200, 160), (420, 300), (430, 305)] {
        assert_dirty_covers(&mut a, "new sel", |a| mv(a, x, y));
    }
    // Resizing a selection: the crosshair is left out under the
    // toolbar, which follows the selection.
    let sel = fr(200.0, 150.0, 600.0, 300.0);
    let mut b = app(Some(sel));
    settled(&mut b);
    b.on_event(Ev::Down { x: 800, y: 450 });
    for (x, y) in [(810, 470), (700, 520), (1000, 380), (1003, 382)] {
        assert_dirty_covers(&mut b, "resize", |b| mv(b, x, y));
    }
    let sc = ed(&mut b).scene.clone().unwrap();
    let c = sc.cross.as_ref().expect("crosshair while resizing");
    assert!(!c.skip.is_empty(), "the toolbar's bounds");
    assert!(assist::cross_bounds(c, sc.size).iter().all(|r| c.skip.iter().all(|k| !r.intersects(k))));
    let want = settled(&mut b);
    assert_tiles_match(&mut b, &want, "resize crosshair", 5);
    // Square.
    ed(&mut a).cfg.magnifier_square = true;
    assert_dirty_covers(&mut a, "square", |a| mv(a, 431, 305));
    let want = settled(&mut a);
    assert_tiles_match(&mut a, &want, "square magnifier", 11);
}

// ---- color ring --------------------------------------------------------------

#[test]
fn ring_hit_testing() {
    let c = Pt::new(500.0, 400.0);
    let n = 10;
    assert_eq!(ring_hit(c, c, n, 1.0, true), Some(RingHit::Center));
    assert_eq!(ring_hit(c, Pt::new(510.0, 410.0), n, 1.0, true), Some(RingHit::Center), "dead zone");
    // Straight up is the first swatch, clockwise from there.
    assert_eq!(ring_hit(c, Pt::new(500.0, 360.0), n, 1.0, true), Some(RingHit::Swatch(0)));
    assert_eq!(ring_hit(c, Pt::new(500.0, 440.0), n, 1.0, true), Some(RingHit::Swatch(n / 2)), "straight down");
    for i in 0..n {
        let q = ring_swatch(c, i, n, 1.0);
        assert!(((q.x - c.x).hypot(q.y - c.y) - RING_R).abs() < 0.01);
        assert_eq!(ring_hit(c, q, n, 1.0, false), Some(RingHit::Swatch(i)), "on swatch {i}");
        assert_eq!(ring_hit(c, q, n, 1.0, true), Some(RingHit::Swatch(i)), "towards swatch {i}");
        // Far out in its direction: the gesture still picks it; a click does not.
        let far = Pt::new(c.x + (q.x - c.x) * 3.0, c.y + (q.y - c.y) * 3.0);
        assert_eq!(ring_hit(c, far, n, 1.0, true), Some(RingHit::Swatch(i)));
        assert_eq!(ring_hit(c, far, n, 1.0, false), None);
        // Halfway out (past the dead zone): towards it.
        let half = Pt::new(c.x + (q.x - c.x) * 0.5, c.y + (q.y - c.y) * 0.5);
        assert_eq!(ring_hit(c, half, n, 1.0, true), Some(RingHit::Swatch(i)));
    }
    // 12 swatches at 2x: the same at twice the radius.
    let q = ring_swatch(c, 3, 12, 2.0);
    assert!((q.x - (c.x + 2.0 * RING_R)).abs() < 0.01 && (q.y - c.y).abs() < 0.01, "3 o'clock is the fourth of 12");
    assert_eq!(ring_hit(c, q, 12, 2.0, false), Some(RingHit::Swatch(3)));
    // Kept on the monitor.
    let area = fr(0.0, 0.0, 1440.0, 900.0);
    let k = ring_center(Pt::new(5.0, 895.0), area, 1.0);
    assert!(k.x - (RING_R + 13.0 + 9.0) >= 0.0 && k.y + RING_R + 13.0 + 9.0 <= 900.0);
    assert_eq!(ring_center(Pt::new(700.0, 400.0), area, 1.0), Pt::new(700.0, 400.0));
}

/// Press, drag towards swatch `i`, release: the tool takes the color
/// (remembered); the selection's objects when there are some.
#[test]
fn ring_press_drag_release_picks() {
    let mut a = app(None);
    let colors = palette_colors(&ed(&mut a).cfg);
    rdown(&mut a, 700, 400);
    let r = ed(&mut a).assist.ring.clone().expect("ring open");
    assert_eq!(r.c, Pt::new(700.0, 400.0));
    assert_eq!(r.colors.len(), colors.len().min(RING_MAX));
    assert_eq!(r.cur, Some(0), "the pencil's red is the palette's first");
    assert!(scene(&mut a).ring.is_some() && scene(&mut a).mag.is_none(), "the ring, no magnifier over it");
    let q = ring_swatch(r.c, 5, r.colors.len(), 1.0);
    mv(&mut a, q.x as i32, q.y as i32);
    assert_eq!(ed(&mut a).assist.ring.as_ref().unwrap().hot, Some(RingHit::Swatch(5)), "highlighted");
    assert_eq!(scene(&mut a).ring.as_ref().unwrap().hot, Some(RingHit::Swatch(5)));
    rup(&mut a, q.x as i32, q.y as i32);
    let e = ed(&mut a);
    assert!(e.assist.ring.is_none(), "closed");
    assert_eq!(e.color(), colors[5]);
    assert!(e.pending.touched().any(|k| k == "tool_pencil_color"), "remembered");

    // With objects selected: theirs.
    let sel = fr(100.0, 100.0, 800.0, 600.0);
    let mut a = app(Some(sel));
    let e = ed(&mut a);
    e.objects.push(Obj::Rect { r: fr(200.0, 200.0, 100.0, 80.0), color: colors[0], width: 3.0 });
    e.dirty = true;
    e.tool = Some(Tool::Select);
    e.obj.sel = vec![0];
    rdown(&mut a, 600, 400);
    assert!(ed(&mut a).assist.ring.is_some(), "open off the object");
    let c = ed(&mut a).assist.ring.as_ref().unwrap().c;
    let q = ring_swatch(c, 2, colors.len(), 1.0);
    mv(&mut a, q.x as i32, q.y as i32);
    rup(&mut a, q.x as i32, q.y as i32);
    assert_eq!(ed(&mut a).sel_color(), Some(colors[2]));
}

#[test]
fn ring_click_mode_and_cancel() {
    let mut a = app(Some(fr(100.0, 100.0, 900.0, 600.0)));
    a.frame(); // the toolbar: the centre opens the wheel
    let before = ed(&mut a).color();
    // Click (no move): it stays open.
    rdown(&mut a, 600, 400);
    rup(&mut a, 600, 400);
    let r = ed(&mut a).assist.ring.clone().expect("still open");
    assert!(!r.held && r.custom);
    // Hover highlights only on a swatch now.
    let q = ring_swatch(r.c, 1, r.colors.len(), 1.0);
    mv(&mut a, q.x as i32, q.y as i32);
    assert_eq!(ed(&mut a).assist.ring.as_ref().unwrap().hot, Some(RingHit::Swatch(1)));
    mv(&mut a, (r.c.x + 3.0 * (q.x - r.c.x)) as i32, (r.c.y + 3.0 * (q.y - r.c.y)) as i32);
    assert_eq!(ed(&mut a).assist.ring.as_ref().unwrap().hot, None);
    // A left click on a swatch picks it.
    a.on_event(Ev::Down { x: q.x as i32, y: q.y as i32 });
    a.on_event(Ev::Up { x: q.x as i32, y: q.y as i32 });
    assert!(ed(&mut a).assist.ring.is_none());
    assert_eq!(ed(&mut a).color(), r.colors[1]);
    assert!(ed(&mut a).sel.is_some(), "the press did not start a selection");

    // Esc closes it, nothing changes, the capture stays.
    ed(&mut a).set_tool_color(before, Instant::now());
    rdown(&mut a, 600, 400);
    key_m(&mut a, key::ESCAPE, NONE);
    assert!(ed(&mut a).assist.ring.is_none());
    assert!(matches!(a.st, State::Edit(_)), "Esc closed the ring only");
    assert_eq!(ed(&mut a).color(), before);
    // Out to a swatch and back to the centre: cancelled.
    rdown(&mut a, 600, 400);
    let q = ring_swatch(Pt::new(600.0, 400.0), 4, r.colors.len(), 1.0);
    mv(&mut a, q.x as i32, q.y as i32);
    mv(&mut a, 600, 400);
    rup(&mut a, 600, 400);
    assert!(ed(&mut a).assist.ring.is_none());
    assert_eq!(ed(&mut a).color(), before);
    // A click outside closes it.
    rdown(&mut a, 600, 400);
    rup(&mut a, 600, 400);
    a.on_event(Ev::Down { x: 50, y: 50 });
    a.on_event(Ev::Up { x: 50, y: 50 });
    assert!(ed(&mut a).assist.ring.is_none());
    assert_eq!(ed(&mut a).color(), before);
    // The centre: the color wheel.
    rdown(&mut a, 600, 400);
    rup(&mut a, 600, 400);
    a.on_event(Ev::Down { x: 600, y: 400 });
    a.on_event(Ev::Up { x: 600, y: 400 });
    assert!(ed(&mut a).assist.ring.is_none());
    assert!(ed(&mut a).pick.is_some(), "the existing color picker");
}

#[test]
fn ring_stays_shut_where_it_does_not_apply() {
    // The select tool on an object.
    let sel = fr(100.0, 100.0, 800.0, 600.0);
    let mut a = app(Some(sel));
    let e = ed(&mut a);
    e.objects.push(Obj::Rect { r: fr(200.0, 200.0, 100.0, 80.0), color: C4::rgb(240, 68, 56), width: 3.0 });
    e.dirty = true;
    e.tool = Some(Tool::Select);
    rdown(&mut a, 200, 240);
    assert!(ed(&mut a).assist.ring.is_none(), "on an object with the select tool");
    // Pixelate has no color.
    ed(&mut a).tool = Some(Tool::Pixelate);
    rdown(&mut a, 600, 500);
    assert!(ed(&mut a).assist.ring.is_none());
    // Typing a text.
    ed(&mut a).tool = Some(Tool::Text);
    a.on_event(Ev::Down { x: 600, y: 500 });
    a.on_event(Ev::Up { x: 600, y: 500 });
    assert!(ed(&mut a).text.is_some());
    rdown(&mut a, 650, 500);
    assert!(ed(&mut a).assist.ring.is_none());
    // Before a selection: the centre opens the picker on its own, for
    // the current tool.
    let mut a = app(None);
    rdown(&mut a, 600, 400);
    rup(&mut a, 600, 400);
    a.on_event(Ev::Down { x: 600, y: 400 });
    a.on_event(Ev::Up { x: 600, y: 400 });
    assert!(ed(&mut a).pick.is_some() && ed(&mut a).assist.ring.is_none());
    let sc = scene(&mut a).clone();
    let tb = ed(&mut a).toolbar.clone().expect("the picker alone");
    assert!(tb.docked && tb.pop.is_some() && sc.bar.is_some());
    // A recent swatch would apply; a hue drag does: the pencil changes.
    let before = ed(&mut a).color();
    let hue = tb.items.iter().find(|i| i.kind == toolbar::Kind::PickHue).unwrap().r;
    let (hx, hy) = ((hue.x + 4.0) as i32, (hue.y + hue.h / 2.0) as i32);
    a.on_event(Ev::Down { x: hx, y: hy });
    a.on_event(Ev::Up { x: hx, y: hy });
    assert_ne!(ed(&mut a).color(), before, "the picker sets the current tool's color");
    // A press outside closes it (keeping the color), and the toolbar is gone.
    a.on_event(Ev::Down { x: 50, y: 850 });
    a.on_event(Ev::Up { x: 50, y: 850 });
    assert!(ed(&mut a).pick.is_none());
    scene(&mut a);
    assert!(ed(&mut a).toolbar.is_none());
}

/// Near a monitor edge the ring sits off the press point; the dead zone
/// stays around the press: a click there waits (click-click), nothing is
/// highlighted or picked until the pointer moves out.
#[test]
fn ring_at_an_edge_waits_for_a_click() {
    let mut a = app(None);
    let before = ed(&mut a).color();
    rdown(&mut a, 5, 450);
    let r = ed(&mut a).assist.ring.clone().unwrap();
    assert!(r.c.x > 80.0, "kept on the monitor");
    assert_eq!(r.hot, None, "nothing highlighted at first");
    mv(&mut a, 7, 451);
    assert_eq!(ed(&mut a).assist.ring.as_ref().unwrap().hot, None, "still in the dead zone");
    rup(&mut a, 7, 451);
    let r = ed(&mut a).assist.ring.clone().expect("click mode");
    assert!(!r.held);
    assert_eq!(ed(&mut a).color(), before, "nothing picked");
    // Then a click on a swatch.
    let q = ring_swatch(r.c, 2, r.colors.len(), 1.0);
    a.on_event(Ev::Down { x: q.x as i32, y: q.y as i32 });
    assert_eq!(ed(&mut a).color(), r.colors[2]);
    // A drag out from the press does pick on release.
    let mut a = app(None);
    rdown(&mut a, 5, 450);
    let r = ed(&mut a).assist.ring.clone().unwrap();
    let q = ring_swatch(r.c, 0, r.colors.len(), 1.0);
    mv(&mut a, q.x as i32, q.y as i32);
    rup(&mut a, q.x as i32, q.y as i32);
    assert_eq!(ed(&mut a).color(), r.colors[0]);
    assert!(ed(&mut a).assist.ring.is_none());
}

// ---- accept on select ----------------------------------------------------------

/// The edit with a new region just drawn (no export runs).
fn drawn(a: &mut App, moved: bool) -> Edit {
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    e.interact = Interact::NewSel { anchor: Pt::new(10.0, 10.0), moved };
    if moved {
        e.sel = Some(fr(10.0, 10.0, 200.0, 100.0));
    }
    end_interaction(&mut e);
    e
}

#[test]
fn accept_on_select_copies_or_saves_unless_alt() {
    let mut a = app(None);
    let e = drawn(&mut a, true);
    assert!(!e.done, "off by default");

    let mut a = app(None);
    a.cfg.accept_on_select = "copy".into();
    let mut c = Config { accept_on_select: "copy".into(), ..Config::default() };
    ed(&mut a).assist = Assist::new(&c, Instant::now());
    let e = drawn(&mut a, true);
    assert!(e.done && !e.cancelled);
    assert!(matches!(e.tasks[..], [Task::Copy]));

    let mut a = app(None);
    c.accept_on_select = "save".into();
    c.save_dialog = true;
    ed(&mut a).cfg.save_dialog = true;
    ed(&mut a).assist = Assist::new(&c, Instant::now());
    let e = drawn(&mut a, true);
    assert!(e.done && matches!(e.tasks[..], [Task::Save { path: None, ask: true }]));

    // Alt on release: the toolbar, to adjust.
    let mut a = app(None);
    ed(&mut a).assist.accept = Some(AcceptOn::Copy);
    let e = {
        let _s = hold(ALT);
        drawn(&mut a, true)
    };
    assert!(!e.done && e.tasks.is_empty() && e.sel.is_some());
    // Shift (a square region) is accepted.
    let mut a = app(None);
    ed(&mut a).assist.accept = Some(AcceptOn::Copy);
    let e = {
        let _s = hold(SHIFT);
        drawn(&mut a, true)
    };
    assert!(e.done);

    // A plain click (everything, no window under it): not accepted.
    let mut a = app(None);
    ed(&mut a).assist.accept = Some(AcceptOn::Copy);
    let e = drawn(&mut a, false);
    assert!(!e.done && e.sel.is_some() && e.tasks.is_empty());
    // A slip: under 8 x 8 logical px.
    let mut a = app(None);
    ed(&mut a).assist.accept = Some(AcceptOn::Copy);
    let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
    let mut e = *e;
    e.interact = Interact::NewSel { anchor: Pt::new(10.0, 10.0), moved: true };
    e.sel = Some(fr(10.0, 10.0, 7.0, 40.0));
    end_interaction(&mut e);
    assert!(!e.done, "7 px wide");

    // The command's tasks win.
    let mut a = app(None);
    ed(&mut a).assist.accept = Some(AcceptOn::Copy);
    ed(&mut a).tasks = vec![Task::Geometry];
    let e = drawn(&mut a, true);
    assert!(e.done && matches!(e.tasks[..], [Task::Geometry]));

    // Record mode records instead.
    let mut a = app(None);
    ed(&mut a).assist.accept = Some(AcceptOn::Copy);
    ed(&mut a).rec.mode = true;
    let e = drawn(&mut a, true);
    assert!(!e.done);
    assert_eq!(AcceptOn::from_config(" Save "), Some(AcceptOn::Save));
    assert_eq!(AcceptOn::from_config("off"), None);
    assert_eq!(AcceptOn::from_config("bogus"), None);
}

#[test]
fn accept_on_select_through_the_overlay() {
    // A drag through the window events closes the overlay at once.
    let mut a = app(None);
    ed(&mut a).assist.accept = Some(AcceptOn::Copy);
    ed(&mut a).tasks = vec![Task::Geometry];
    a.on_event(Ev::Down { x: 10, y: 10 });
    mv(&mut a, 60, 40);
    mv(&mut a, 110, 60);
    a.on_event(Ev::Up { x: 110, y: 60 });
    assert!(matches!(a.st, State::Hidden), "exported and closed");
    // A window click while snapping selects that window and accepts it.
    let mut a = app(None);
    let e = ed(&mut a);
    e.assist.accept = Some(AcceptOn::Copy);
    let win = crate::snap::Node::new(crate::snap::R::xywh(120, 90, 1200, 720), vec![]);
    e.snap = Some(crate::snap::Snapshot::build(&[win], e.shot.origin, e.shot.size));
    mv(&mut a, 600, 400);
    let mut e = {
        let State::Edit(e) = std::mem::replace(&mut a.st, State::Hidden) else { unreachable!() };
        *e
    };
    e.canvas_press(Pt::new(600.0, 400.0));
    e.pointer_up();
    assert!(e.done && e.sel == Some(fr(120.0, 90.0, 1200.0, 720.0)) && matches!(e.tasks[..], [Task::Copy]));
}

#[test]
fn begin_capture_takes_the_cli_accept() {
    let queue = |shot: Shot| FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(shot)));
    let waiting = |p: Pending| {
        let mut a = app(None);
        a.st = State::Hidden;
        a.kind = RunKind::OneShot;
        a.pending = Some(p);
        a
    };
    // No region: the overlay waits, accepting the region drawn.
    queue(synthetic_shot());
    let mut p = Pending::editor();
    p.accept_new = Some(AcceptOn::Copy);
    let mut a = waiting(p);
    a.on_event(Ev::Timer);
    assert_eq!(ed(&mut a).assist.accept, Some(AcceptOn::Copy));
    // The setting, without the flag.
    queue(synthetic_shot());
    let mut a = waiting(Pending::editor());
    a.cfg.accept_on_select = "save".into();
    a.on_event(Ev::Timer);
    assert_eq!(ed(&mut a).assist.accept, Some(AcceptOn::Save));
    // A region with the flag: exported at once (its tasks here).
    queue(synthetic_shot());
    let mut p = Pending::editor();
    p.accept_new = Some(AcceptOn::Copy);
    p.accept_on_select = true;
    p.region = Some((0, 0, 64, 32));
    p.tasks = vec![Task::Geometry];
    let mut a = waiting(p);
    a.on_event(Ev::Timer);
    assert!(matches!(a.st, State::Hidden));
}

// ---- hint card --------------------------------------------------------------

fn words(parts: &[HintPart]) -> String {
    parts
        .iter()
        .map(|p| match p {
            HintPart::T(s) | HintPart::K(s) => s.as_str(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn hint_reads_the_keymap() {
    let km = Keymap::defaults();
    assert_eq!(words(&hint_parts(&km, false)), "Drag to select \u{b7} Ctrl C copy \u{b7} Ctrl S save \u{b7} Esc cancel \u{b7} right-click colors".replace("Ctrl C", &chord_text("Ctrl+C")).replace("Ctrl S", &chord_text("Ctrl+S")));
    let parts = hint_parts(&km, false);
    assert!(parts.contains(&HintPart::K(chord_text("Ctrl+C"))) && parts.contains(&HintPart::K("Esc".into())));
    assert!(words(&hint_parts(&km, true)).starts_with("Click a window or drag to select \u{b7}"));
    // Remapped and unbound keys.
    let (km, _) = Keymap::from_config(&[("copy", "Ctrl+K"), ("cancel", "Q"), ("save", "")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect());
    let parts = hint_parts(&km, false);
    assert!(parts.contains(&HintPart::K(chord_text("Ctrl+K"))));
    assert!(parts.contains(&HintPart::K("Q".into())));
    assert!(!parts.iter().any(|p| *p == HintPart::T("save".into())), "unbound: left out");
    assert!(!parts.contains(&HintPart::K("Esc".into())));
}

fn chord_text(s: &str) -> String {
    chrome::chord_caps(&crate::keymap::Chord::parse(s).unwrap()).join("+")
}

#[test]
fn hint_shows_at_open_and_goes_on_move_key_or_time() {
    let mut a = app(None);
    settled(&mut a);
    let sc = ed(&mut a).scene.as_ref().unwrap();
    let h = sc.hint.as_ref().expect("the hint when the overlay opens");
    assert!(words(&h.parts).starts_with("Drag to select"));
    // The window's first move only marks where the pointer is.
    mv(&mut a, 300, 300);
    assert!(ed(&mut a).assist.hint_live(Instant::now()));
    mv(&mut a, 301, 301);
    assert!(ed(&mut a).assist.hint_live(Instant::now()), "a jitter");
    mv(&mut a, 320, 300);
    assert!(!ed(&mut a).assist.hint_live(Instant::now()), "moved: gone");
    settled(&mut a);
    assert!(ed(&mut a).scene.as_ref().unwrap().hint.is_none(), "faded out");
    // A key.
    let mut a = app(None);
    key_m(&mut a, 'J' as u32, NONE);
    assert!(!ed(&mut a).assist.hint_live(Instant::now()));
    // Three seconds.
    let mut a = app(None);
    let e = ed(&mut a);
    let now = Instant::now();
    assert!(e.assist.hint_end > now + Duration::from_millis(HINT_MS - 500));
    assert!(!e.assist.hint_due(now), "no fast ticks for 3 s");
    e.assist.hint_end = now + Duration::from_millis(100);
    assert!(e.assist.hint_due(now) && animating(e, None, now), "ticks fast to fade on time");
    e.assist.hint_end = now;
    settled(&mut a);
    assert!(ed(&mut a).scene.as_ref().unwrap().hint.is_none());
    // Off in Settings: never.
    let c = Config { overlay_hints: false, ..Config::default() };
    let mut a = app(None);
    ed(&mut a).assist = Assist::new(&c, Instant::now());
    settled(&mut a);
    assert!(ed(&mut a).scene.as_ref().unwrap().hint.is_none());
}

#[test]
fn helpers_are_never_in_the_capture() {
    let area = fr(100.0, 100.0, 600.0, 400.0);
    let mut a = app(None);
    let plain = ed(&mut a).export(area);
    // The hint card and the magnifier (with its crosshair) on screen.
    mv(&mut a, 400, 300);
    mv(&mut a, 401, 300);
    settled(&mut a);
    let sc = ed(&mut a).scene.clone().unwrap();
    assert!(sc.hint.is_some() && sc.mag.is_some() && sc.cross.is_some(), "all up");
    let f = settled(&mut a);
    assert_ne!(crop_to_image(&f, area).as_raw(), plain.as_raw(), "they are drawn over the area");
    assert_eq!(ed(&mut a).export(area).as_raw(), plain.as_raw(), "not in the capture");
    // The color ring over it.
    rdown(&mut a, 400, 300);
    settled(&mut a);
    assert!(ed(&mut a).scene.as_ref().unwrap().ring.is_some());
    assert_eq!(ed(&mut a).export(area).as_raw(), plain.as_raw());
}

// ---- the crosshair's centre ----------------------------------------------------

/// The frame at pointer `p` with the magnifier on or off (hint gone).
fn cross_frame(th: Theme, p: (i32, i32), on: bool) -> PixBuf {
    let mut a = preview_app(th, None);
    ed(&mut a).cfg.magnifier = on;
    ed(&mut a).assist.dismiss_hint(Instant::now());
    mv(&mut a, p.0, p.1);
    settled(&mut a)
}

/// The system pointer is hidden under the crosshair, so the pointer's
/// own pixel must stay in sight: the crossing is left out (the pixel as
/// it is without the crosshair), the four arms start right next to it.
#[test]
fn crosshair_leaves_the_pointers_pixel_in_sight() {
    let p = (702, 548);
    for th in [theme::DARK, theme::LIGHT] {
        let (on, off) = (cross_frame(th, p, true), cross_frame(th, p, false));
        let px = |f: &PixBuf, x: i32, y: i32| f.get_pixel(x as u32, y as u32);
        assert_eq!(px(&on, p.0, p.1), px(&off, p.0, p.1), "the pointer's pixel, untouched");
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-40, 0), (40, 0), (0, -40), (0, 40)] {
            assert_ne!(px(&on, p.0 + dx, p.1 + dy), px(&off, p.0 + dx, p.1 + dy), "an arm at ({dx}, {dy})");
        }
        // The diagonal neighbours: no line there.
        assert_eq!(px(&on, p.0 + 1, p.1 + 1), px(&off, p.0 + 1, p.1 + 1));
    }
    // The parts: four arms, none over the crossing, at any line width.
    for lw in [1, 2, 3] {
        let c = CrossUi { x: 100, y: 80, lw, skip: Vec::new() };
        let parts = c.parts((400, 300));
        assert_eq!(parts.len(), 4);
        let hole = PxRect::new(100, 80, 100 + lw, 80 + lw);
        assert!(parts.iter().all(|r| !r.intersects(&hole)), "lw {lw}");
        let area: i32 = parts.iter().map(|r| r.w() * r.h()).sum();
        assert_eq!(area, lw * 400 + lw * 300 - 2 * lw * lw, "lw {lw}: both lines but the crossing");
    }
    // At the image's edge: the arms that fit.
    assert_eq!(CrossUi { x: 0, y: 0, lw: 1, skip: Vec::new() }.parts((400, 300)).len(), 2);
}

// ---- preview PNGs -------------------------------------------------------------

/// `RUSTSHOT_PREVIEW_DIR=<dir> cargo test assist_previews`: the round and
/// square magnifier, the ring and the hint card, dark and light.
#[test]
fn assist_previews() {
    for (th, name) in [(theme::DARK, "dark"), (theme::LIGHT, "light")] {
        let mut shots: Vec<(String, PixBuf)> = Vec::new();
        for square in [false, true] {
            let mut a = preview_app(th, None);
            ed(&mut a).cfg.magnifier_square = square;
            ed(&mut a).assist.dismiss_hint(Instant::now());
            mv(&mut a, 702, 548);
            let f = settled(&mut a);
            let m = ed(&mut a).scene.as_ref().unwrap().mag.clone().expect("magnifier");
            // The loupe's middle shows the pointer's pixel (zoomed).
            let (cx, cy) = ((m.r.x + m.r.w / 2.0) as u32, (m.r.y + m.r.h / 2.0) as u32);
            let i = (cy as usize * f.width() as usize + cx as usize) * 4;
            let want = pixel(ed(&mut a), 702, 548);
            assert_eq!(&f.as_raw()[i..i + 3], &[want.r, want.g, want.b], "{name} square={square}");
            shots.push((format!("assist_mag_{}_{name}.png", if square { "square" } else { "round" }), f));
        }
        let mut a = preview_app(th, Some(fr(240.0, 140.0, 960.0, 540.0)));
        rdown(&mut a, 720, 420);
        let r = ed(&mut a).assist.ring.clone().unwrap();
        let q = ring_swatch(r.c, 3, r.colors.len(), 1.0);
        mv(&mut a, q.x as i32, q.y as i32);
        let f = settled(&mut a);
        assert!(ed(&mut a).scene.as_ref().unwrap().ring.is_some());
        shots.push((format!("assist_ring_{name}.png"), f));
        let mut a = preview_app(th, None);
        ed(&mut a).cfg.magnifier = false;
        let f = settled(&mut a);
        assert!(ed(&mut a).scene.as_ref().unwrap().hint.is_some());
        shots.push((format!("assist_hint_{name}.png"), f));
        // The crosshair's centre, 12x (the system pointer is hidden there):
        // the pointer's pixel between the four arms.
        let f = cross_frame(th, (702, 548), true);
        let (n, k) = (33u32, 12u32);
        let mut z = PixBuf::new(n * k, n * k);
        for y in 0..n * k {
            for x in 0..n * k {
                let c = f.get_pixel(702 - n / 2 + x / k, 548 - n / 2 + y / k);
                let i = ((y * n * k + x) * 4) as usize;
                z.as_raw_mut()[i..i + 4].copy_from_slice(&c);
            }
        }
        shots.push((format!("assist_cross_center_{name}.png"), z));
        if let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (n, f) in shots {
                std::fs::write(dir.join(n), f.to_png().unwrap()).unwrap();
            }
        }
    }
}

// ---- bench -------------------------------------------------------------------

/// Pointer moves at 3840x2160 before a selection (hover) and while the
/// region is drawn: the driver's move handling, `prepare` + dirty rects,
/// and composing the damage, with the magnifier off and on.
/// `cargo test --release perf_magnifier_bench -- --ignored --nocapture`
#[test]
#[ignore = "benchmark; run in release"]
fn perf_magnifier_bench() {
    let (w, h) = (3840u32, 2160u32);
    let src = synthetic_shot();
    let mut img = PixBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (y as usize * w as usize + x as usize) * 4;
            let j = ((y % src.size.1) as usize * src.size.0 as usize + (x % src.size.0) as usize) * 4;
            img.as_raw_mut()[i..i + 4].copy_from_slice(&src.image.as_raw()[j..j + 4]);
        }
    }
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f64 * q) as usize]
    };
    for (name, on, drag) in [("hover, magnifier off", false, false), ("hover, magnifier on", true, false), ("new sel, magnifier off", false, true), ("new sel, magnifier on", true, true)] {
        let shot = Shot { origin: (0, 0), size: (w, h), scale: 1.5, image: img.clone(), monitors: vec![(0, 0, w, h)] };
        let mut a = preview_app_with(theme::DARK, None, shot);
        ed(&mut a).cfg.magnifier = on;
        ed(&mut a).assist.dismiss_hint(Instant::now());
        settled(&mut a);
        if drag {
            a.on_event(Ev::Down { x: 400, y: 300 });
        }
        let mut buf = Vec::new();
        let (mut t, mut tm, mut td, mut tc, mut area) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for k in 0..300 {
            let (x, y) = (500 + (k % 150) * 7, 400 + (k % 150) * 4);
            let t0 = Instant::now();
            a.mouse = (x, y);
            let p = Pt::new(x as f32, y as f32);
            if drag {
                a.pointer_move(x, y);
            } else {
                let e = ed(&mut a);
                let _ = e.track_hover(p) | e.track_assist(p);
            }
            let e = ed(&mut a);
            let prev = e.scene.clone();
            let t1 = Instant::now();
            e.prepare(None, (x, y), Instant::now());
            let rects = compose::merge_rects(e.dirty_rects(prev.as_ref()));
            let t2 = Instant::now();
            let bd = PixBufBackdrop { img: e.composed(), dim: e.th.dim };
            for r in &rects {
                let n = r.w() as usize * r.h() as usize * 4;
                buf.resize(n, 0);
                e.compose_rect(*r, &bd, &mut buf[..n], crate::raster::Order::Bgra);
            }
            let t3 = Instant::now();
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
            t.push(ms(t0, t3));
            tm.push(ms(t0, t1));
            td.push(ms(t1, t2));
            tc.push(ms(t2, t3));
            area.push(rects.iter().map(|r| r.w() as f64 * r.h() as f64).sum::<f64>());
        }
        println!(
            "BENCH 4K {name}: median {:.3} p90 {:.3} max {:.3} ms | move {:.3} prepare+damage {:.3} compose {:.3} ms | damage {:.0} px",
            pct(&mut t.clone(), 0.5),
            pct(&mut t.clone(), 0.9),
            pct(&mut t, 1.0),
            pct(&mut tm, 0.5),
            pct(&mut td, 0.5),
            pct(&mut tc, 0.5),
            pct(&mut area, 0.5)
        );
    }
}
