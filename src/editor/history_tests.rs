//! The overlay keeps its captures in the history: the selection without
//! its objects as `original.png`, the objects moved into its coordinates,
//! and nothing at all for a cancelled capture, Copy text, or with
//! `history = false`.

use super::tests::{edit_of, preview_app};
use super::*;
use crate::history::{Store, tests::TempRoot};
use crate::objects::Pt;

const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };

fn app(sel: FRect, root: &std::path::Path) -> App {
    let mut a = preview_app(theme::DARK, Some(sel));
    a.focus_tries = u8::MAX;
    a.kind = RunKind::Daemon;
    a.history_root = Some(root.to_path_buf());
    a
}

fn arrow(x: f32, y: f32) -> Obj {
    Obj::Arrow { a: Pt::new(x, y), b: Pt::new(x + 60.0, y + 40.0), color: C4::rgb(240, 68, 56), width: 4.0 }
}

#[test]
fn a_pinned_capture_is_kept_plain_with_its_objects() {
    let _ = (crate::pin::take_opened(), crate::notice::take_shown());
    let root = TempRoot::new("overlay");
    let sel = FRect { x: 100.5, y: 80.0, w: 300.0, h: 200.0 };
    let mut a = app(sel, &root.0);
    let plain = crop_to_image(&edit_of(&mut a).base, sel);
    edit_of(&mut a).commit_object(arrow(150.0, 120.0));
    a.on_event(Ev::Key { vk: 'P' as u32, up: false, repeat: false, mods: CTRL });
    assert!(matches!(a.st, State::Hidden), "exported");
    let id = a.last_entry.clone().expect("kept");
    crate::history::flush(Duration::from_secs(20));
    let st = Store::new(&root.0);
    let e = st.get(&id).expect("written");
    assert_eq!(st.original(&e).unwrap(), plain, "the selection without its objects");
    // The crop starts at the floored selection: x 100.
    assert_eq!(st.objects(&e).unwrap(), vec![arrow(50.0, 40.0)]);
    assert_eq!((e.meta.w, e.meta.h), (plain.width(), plain.height()));
    assert_eq!(e.meta.objects, ["Arrow"]);
    assert!(!e.meta.edited && e.meta.saved.is_none());
    let _ = crate::pin::take_opened();
}

#[test]
fn nothing_is_kept_without_a_picture() {
    let root = TempRoot::new("none");
    let sel = FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 };
    // Cancelled.
    let mut a = app(sel, &root.0);
    a.on_event(Ev::Key { vk: wind::key::ESCAPE, up: false, repeat: false, mods: Mods::default() });
    assert!(a.last_entry.is_none());
    // History off.
    let mut a = app(sel, &root.0);
    edit_of(&mut a).cfg.history = false;
    a.on_event(Ev::Key { vk: 'P' as u32, up: false, repeat: false, mods: CTRL });
    assert!(a.last_entry.is_none());
    let _ = crate::pin::take_opened();
    crate::history::flush(Duration::from_secs(20));
    assert!(Store::new(&root.0).list().is_empty());
    assert!(history_keeps(&[Task::Copy]) && history_keeps(&[Task::Save { path: None, ask: false }]));
    assert!(!history_keeps(&[Task::Text]) && !history_keeps(&[Task::Raw, Task::Geometry]) && !history_keeps(&[]));
}
