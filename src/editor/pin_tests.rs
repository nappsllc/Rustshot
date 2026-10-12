//! Pin to screen and the thumbnail card, driven the way the window
//! drives the editor: Ctrl+P and the toolbar's pin button, a one-shot run
//! waiting for its pins, the tray's Close all pins, and the card a saved
//! capture gets (or the toast, with the setting off). Pins and notes are
//! recorded (`pin::take_opened`, `notice::take_shown`), never shown.

use super::tests::{edit_of, preview_app};
use super::*;
use crate::wind::key;

fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };

fn app(sel: FRect, kind: RunKind) -> App {
    let mut a = preview_app(theme::DARK, Some(sel));
    a.focus_tries = u8::MAX;
    a.kind = kind;
    a
}

fn ed(a: &mut App) -> &mut Edit {
    edit_of(a)
}

fn ctrl(a: &mut App, c: char) {
    a.on_event(Ev::Key { vk: c as u32, up: false, repeat: false, mods: CTRL });
}

fn hidden(a: &App) -> bool {
    matches!(a.st, State::Hidden)
}

/// A fresh temp folder for exports (removed on drop).
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("rustshot-pin-{}-{tag}", std::process::id()));
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

/// Ctrl+P: the overlay closes and the selection, as exported, is pinned
/// where it was; nothing is copied or saved, so no toast or card.
#[test]
fn ctrl_p_pins_the_selection_where_it_was() {
    let _ = (crate::pin::take_opened(), crate::notice::take_shown());
    let mut a = app(r(100.0, 80.0, 300.0, 200.0), RunKind::Daemon);
    ctrl(&mut a, 'P');
    assert!(hidden(&a), "the overlay closed");
    assert_eq!(crate::pin::take_opened(), [((300, 200), (100, 80))]);
    assert!(crate::notice::take_shown().is_empty(), "a pin needs no toast");
    assert_eq!(action_of(Act::Pin), Some(Action::Pin), "the tooltip shows Ctrl+P");
}

/// The toolbar's pin button does the same.
#[test]
fn the_pin_button_pins() {
    let _ = crate::pin::take_opened();
    let mut a = app(r(40.0, 30.0, 900.0, 500.0), RunKind::Daemon);
    let m = a.mouse;
    ed(&mut a).prepare(None, m, Instant::now());
    let b = ed(&mut a).toolbar.as_ref().expect("toolbar").items.iter().find(|i| i.kind == toolbar::Kind::Btn(Act::Pin)).expect("pin button").r;
    let (x, y) = ((b.x + b.w / 2.0) as i32, (b.y + b.h / 2.0) as i32);
    a.on_event(Ev::Down { x, y });
    a.on_event(Ev::Up { x, y });
    assert!(hidden(&a));
    assert_eq!(crate::pin::take_opened(), [((900, 500), (40, 30))]);
}

/// A one-shot run (`rustshot gui`) that pinned stays up, hidden, until its
/// last pin is closed; then it ends like a capture (code 0).
#[test]
fn a_one_shot_run_waits_for_its_pins() {
    let _ = crate::pin::take_opened();
    let mut a = app(r(100.0, 80.0, 300.0, 200.0), RunKind::OneShot);
    ctrl(&mut a, 'P');
    assert!(hidden(&a) && a.wait_pins, "waiting for the pin");
    assert_eq!(crate::pin::count(), 1);
    a.on_event(Ev::Timer);
    assert!(a.wait_pins, "still up");
    crate::pin::close_all();
    a.on_event(Ev::Timer);
    assert!(!a.wait_pins, "closed with the last pin");
    assert_eq!(a.exit_code.load(Ordering::SeqCst), 0);
}

/// The tray's Close all pins reaches the daemon at any time, an open
/// capture left alone.
#[test]
fn close_all_pins_from_the_tray() {
    let _ = crate::pin::take_opened();
    assert!(crate::pin::open(std::sync::Arc::new(PixBuf::new(4, 4)), (0, 0), theme::DARK));
    let mut a = app(r(10.0, 10.0, 50.0, 50.0), RunKind::Daemon);
    assert!(a.on_hot(crate::hotkey::HotEvent::ClosePins));
    assert_eq!(crate::pin::count(), 0);
    assert!(!hidden(&a), "the capture is still open");
}

/// A saved screenshot in the daemon gets the thumbnail card (the capture,
/// where it was, the file it saved); with the setting off, the toast.
#[test]
fn a_saved_capture_gets_the_thumbnail_card() {
    let dir = TempDir::new("thumb");
    let _ = crate::notice::take_shown();
    let save = |a: &mut App, thumb: bool| {
        ed(a).tasks = vec![Task::Save { path: Some(dir.0.clone()), ask: false }];
        ed(a).cfg.filename_pattern = "shot".into();
        ed(a).cfg.show_thumbnail = thumb;
        a.on_event(Ev::Key { vk: key::RETURN, up: false, repeat: false, mods: Mods::default() });
        let shown = crate::notice::take_shown();
        assert_eq!(shown.len(), 1, "{shown:?}");
        shown.into_iter().next().unwrap()
    };
    let mut a = app(r(100.0, 80.0, 300.0, 200.0), RunKind::Daemon);
    let (n, at) = save(&mut a, true);
    let t = n.thumb.as_ref().expect("a thumbnail");
    assert_eq!((t.img.dimensions(), t.at), ((300, 200), (100, 80)));
    assert_eq!(at, (250, 180), "on the monitor under the capture");
    assert_eq!(n.text, "Saved \u{b7} shot.png");
    assert_eq!(n.file.as_deref(), Some(dir.0.join("shot.png").as_path()));
    assert_eq!(n.ttl, crate::notice::ACTION_TTL);
    let mut a = app(r(100.0, 80.0, 300.0, 200.0), RunKind::Daemon);
    let (n, _) = save(&mut a, false);
    assert!(n.thumb.is_none() && n.actions.is_empty(), "the toast");
    assert_eq!(n.text, "Saved \u{b7} shot_1.png");
}
