//! Copy text (OCR) through the editor: the key, its alternative chord, the
//! toolbar button and the global hotkey close the overlay at once; the
//! text is recognized off the UI thread (a fake recognizer here), copied
//! (a fake clipboard) and the notice says how it went.

use super::tests::{edit_of, preview_app, synthetic_shot};
use super::*;

fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
    FRect { x, y, w, h }
}

const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
const CTRL_SHIFT: Mods = Mods { shift: true, ctrl: true, alt: false };

fn app(sel: Option<FRect>) -> App {
    let mut a = preview_app(theme::DARK, sel);
    a.focus_tries = u8::MAX;
    a
}

fn key_m(a: &mut App, vk: u32, mods: Mods) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods });
}

fn click(a: &mut App, x: i32, y: i32) {
    a.on_event(Ev::Down { x, y });
    a.on_event(Ev::Up { x, y });
}

fn hidden(a: &App) -> bool {
    matches!(a.st, State::Hidden)
}

fn texts(notes: &[crate::notice::Note]) -> Vec<&str> {
    notes.iter().map(|n| n.text.as_str()).collect()
}

/// Tick until the recognition is over (it never blocks the UI thread).
fn settle(a: &mut App) {
    let t0 = Instant::now();
    while a.ocr.is_some() {
        assert!(!a.idle(), "busy while the text is recognized");
        assert!(t0.elapsed() < Duration::from_secs(10), "recognition never finished");
        std::thread::sleep(Duration::from_millis(5));
        a.on_event(Ev::Timer);
    }
}

/// Run Copy text in the daemon with `fake` as the recognizer, `start`
/// closing the overlay: what was copied and the notices.
fn daemon_ocr(fake: crate::ocr::Fake, start: impl FnOnce(&mut App)) -> (App, Vec<String>, Vec<crate::notice::Note>) {
    crate::ocr::FAKE.with(|f| f.set(Some(fake)));
    crate::ocr::COPIED.with(|c| c.borrow_mut().clear());
    let _ = (crate::tray::take_notes(), crate::notice::take_shown(), crate::sound::take_played());
    let mut a = app(Some(r(100.0, 80.0, 300.0, 200.0)));
    a.kind = RunKind::Daemon;
    a.cfg.shutter_sound = true;
    start(&mut a);
    assert!(hidden(&a), "the overlay closed at once");
    settle(&mut a);
    crate::ocr::FAKE.with(|f| f.set(None));
    assert_eq!(crate::sound::take_played(), 0, "no shutter for text");
    let copied = crate::ocr::COPIED.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let shown = crate::notice::take_shown();
    assert!(shown.iter().all(|(_, at)| *at == (250, 180)), "at the selection's centre: {shown:?}");
    (a, copied, shown.into_iter().map(|(n, _)| n).collect())
}

#[test]
fn copy_text_key_recognizes_and_copies_the_selection() {
    let (a, copied, shown) = daemon_ocr(
        |img| {
            assert_eq!(img.dimensions(), (300, 200), "the selection");
            Ok("Hello Rustshot 123\nsecond line".into())
        },
        |a| key_m(a, 'O' as u32, NONE),
    );
    assert_eq!(copied, ["Hello Rustshot 123\nsecond line"]);
    assert_eq!(texts(&shown), ["Copied text \u{b7} 2 lines"]);
    assert_eq!(shown[0].kind, ToastKind::Success);
    assert_eq!(a.last_region, Some((100, 80, 300, 200)));
    assert!(crate::tray::take_notes().is_empty());
}

#[test]
fn copy_text_alt_chord_and_nothing_found() {
    let (_, copied, shown) = daemon_ocr(|_| Ok("  \n".into()), |a| key_m(a, 'C' as u32, CTRL_SHIFT));
    assert!(copied.is_empty(), "nothing to copy: the clipboard is left alone");
    assert_eq!(texts(&shown), ["No text found"]);
}

#[test]
fn copy_text_button_says_why_it_failed() {
    let (_, copied, shown) = daemon_ocr(
        |_| Err(crate::ocr::OcrError::NoTesseract),
        |a| {
            let _ = a.frame(); // lays the toolbar out
            let tb = edit_of(a).toolbar.as_ref().expect("toolbar");
            let b = tb.items.iter().find(|i| i.kind == toolbar::Kind::Btn(Act::CopyText)).expect("Copy text button").r;
            click(a, (b.x + b.w / 2.0) as i32, (b.y + b.h / 2.0) as i32);
        },
    );
    assert!(copied.is_empty());
    assert_eq!(texts(&shown), ["Install tesseract for text recognition"]);
    assert_eq!(shown[0].kind, ToastKind::Error);
}

#[test]
fn copy_text_tooltip_shows_its_key() {
    let km = Keymap::defaults();
    let (label, keys) = chrome::act_tip(Act::CopyText, &km);
    assert_eq!((label, keys), ("Copy text", vec!["O".to_string()]));
    assert_eq!(chrome::act_icon(Act::CopyText), "ocr");
    assert!(crate::icon_path::ICONS.iter().any(|(n, _)| *n == "ocr"));
}

/// The copy-text hotkey (slot 6): pick an area; its text is copied as
/// soon as the selection ends (no annotation step).
#[test]
fn copy_text_hotkey_picks_an_area_then_recognizes() {
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    assert!(a.on_hot(HotEvent::Pressed(6)));
    let p = a.pending.as_ref().expect("a capture");
    assert!(p.accept_on_select && matches!(p.tasks[..], [Task::Text]) && p.region.is_none());
    crate::ocr::FAKE.with(|f| f.set(Some(|img| Ok(format!("{}x{}", img.width(), img.height())))));
    crate::ocr::COPIED.with(|c| c.borrow_mut().clear());
    let _ = crate::notice::take_shown();
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    a.on_event(Ev::Timer);
    assert!(matches!(a.st, State::Edit(_)), "the overlay is up to pick the area");
    a.on_event(Ev::Down { x: 100, y: 100 });
    a.on_event(Ev::Move { x: 180, y: 120 });
    a.on_event(Ev::Move { x: 260, y: 140 });
    a.on_event(Ev::Up { x: 260, y: 140 });
    assert!(hidden(&a), "done when the selection ends");
    settle(&mut a);
    crate::ocr::FAKE.with(|f| f.set(None));
    assert_eq!(crate::ocr::COPIED.with(|c| c.borrow().clone()), ["160x40"]);
    let shown: Vec<_> = crate::notice::take_shown().into_iter().map(|n| n.0).collect();
    assert_eq!(texts(&shown), ["Copied text \u{b7} 1 line"]);
}

/// Preview PNGs under `RUSTSHOT_PREVIEW_DIR`: the toolbar with Copy text
/// hovered (its tooltip), and the notices it ends with (dark and light).
#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn render_copy_text_previews() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let big = r(100.0, 80.0, 1240.0, 700.0);
    for (tn, th, bg) in [("dark", theme::DARK, [38u8, 44, 58]), ("light", theme::LIGHT, [214, 222, 232])] {
        let mut a = super::tests::preview_app_with(th, Some(big), synthetic_shot());
        a.focus_tries = u8::MAX;
        super::tests::settled_frame(&mut a);
        let e = edit_of(&mut a);
        e.hover = e.toolbar.as_ref().unwrap().items.iter().position(|it| it.kind == toolbar::Kind::Btn(Act::CopyText));
        e.hover_at = Instant::now() - Duration::from_secs(2);
        let img = super::tests::settled_frame(&mut a);
        std::fs::write(dir.join(format!("{tn}-toolbar-copy-text-tip.png")), img.to_png().unwrap()).unwrap();
        let notes = [
            ("copied-text", crate::ocr::note(&Ok("a\nb\nc".into()), Ok(()))),
            ("no-text", crate::ocr::note(&Ok(String::new()), Ok(()))),
            ("no-tesseract", crate::ocr::note(&Err(crate::ocr::OcrError::NoTesseract), Ok(()))),
        ];
        for s in [1.0f32, 2.0] {
            for (name, note) in &notes {
                let (w, h, px) = crate::notice::render(note, &th, s, 1.0);
                let img = PixBuf::from_raw(w, h, crate::rec_ui::layer_over(&px, bg)).unwrap();
                std::fs::write(dir.join(format!("{tn}-notice-{name}@{s}x.png")), img.to_png().unwrap()).unwrap();
            }
        }
    }
}
