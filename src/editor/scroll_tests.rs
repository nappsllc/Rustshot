//! Scrolling capture from the overlay: Shift+S (or the size label menu's
//! row) hides the overlay and asks for a scrolling capture of the
//! selection; without a selection it does nothing.

use super::tests::{edit_of, preview_app_with, settled};
use super::*;

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };

fn fr(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

fn shot() -> Shot {
    let (w, h) = (1200u32, 800u32);
    Shot { origin: (1920, 0), size: (w, h), scale: 1.0, image: PixBuf::from_pixel(w, h, [250, 250, 250, 255]), monitors: vec![(0, 0, w, h)] }
}

fn app_with(sel: Option<FRect>) -> App {
    let mut a = preview_app_with(theme::DARK, sel, shot());
    a.focus_tries = u8::MAX;
    a
}

fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

fn click(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Move { x, y });
    a.on_event(Ev::Down { x, y });
    a.on_event(Ev::Up { x, y });
}

#[test]
fn shift_s_scrolls_the_selection() {
    let mut a = app_with(Some(fr(100.0, 120.0, 640.0, 480.0)));
    key_m(&mut a, 'S' as u32, SHIFT);
    assert!(matches!(a.st, State::Hidden), "the overlay is gone");
    assert_eq!(a.scroll_asked, Some((1920 + 100, 120, 640, 480)), "the selection, in screen px");
    // S alone is still the spotlight.
    let mut b = app_with(Some(fr(100.0, 120.0, 640.0, 480.0)));
    key_m(&mut b, 'S' as u32, NONE);
    assert!(matches!(b.st, State::Edit(_)));
    assert_eq!(b.scroll_asked, None);
}

#[test]
fn without_a_selection_nothing_scrolls() {
    let mut a = app_with(None);
    key_m(&mut a, 'S' as u32, SHIFT);
    assert!(matches!(a.st, State::Edit(_)), "still open");
    assert_eq!(a.scroll_asked, None);
}

#[test]
fn the_size_label_menu_has_a_scrolling_capture_row() {
    if !crate::scroll::supported() {
        return;
    }
    let mut a = app_with(Some(fr(100.0, 100.0, 800.0, 500.0)));
    settled(&mut a);
    let label = edit_of(&mut a).label_at.expect("the label is drawn");
    click(&mut a, (label.x + label.w / 2.0) as i32, (label.y + label.h / 2.0) as i32);
    settled(&mut a);
    let tb = edit_of(&mut a).toolbar.clone().expect("toolbar");
    let row = tb.items.iter().find(|i| matches!(i.kind, toolbar::Kind::MenuItem { act: Act::Scroll, .. })).expect("the Scrolling capture row");
    if let toolbar::Kind::MenuItem { label, key, .. } = row.kind {
        assert_eq!(label, tr!("tip.scroll_capture"));
        assert_eq!(tb.texts[key], format!("{}+S", chrome::SHIFT), "its key");
    }
    let r = row.r;
    click(&mut a, (r.x + r.w / 2.0) as i32, (r.y + r.h / 2.0) as i32);
    assert_eq!(a.scroll_asked, Some((1920 + 100, 100, 800, 500)));
}
