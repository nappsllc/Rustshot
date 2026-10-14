//! The mouse pointer object (Settings › Capture "Include mouse pointer")
//! and the Read QR code hotkey through the daemon.

use super::tests::{edit_of, preview_app, synthetic_shot};
use super::*;

fn app(sel: Option<FRect>) -> App {
    let mut a = preview_app(theme::DARK, sel);
    a.focus_tries = u8::MAX;
    a
}

fn hidden(a: &App) -> bool {
    matches!(a.st, State::Hidden)
}

fn pointer_pic() -> PixBuf {
    let mut p = PixBuf::new(12, 20);
    for (i, px) in p.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i as u32 % 12, i as u32 / 12);
        if x <= y / 2 {
            *px = [255, 0, 255, 255];
        }
    }
    p
}

/// A capture with the setting on gets the pointer as an object where it
/// was (capture px), from the start: no undo takes it away, Delete does.
#[test]
fn the_pointer_is_an_object_from_the_start() {
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    a.cfg.include_cursor = true;
    crate::pointer::FAKE.with(|f| *f.borrow_mut() = Some(crate::pointer::Pointer { img: pointer_pic(), at: (300, 200) }));
    assert!(a.on_hot(HotEvent::Capture));
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    a.on_event(Ev::Timer);
    crate::pointer::FAKE.with(|f| *f.borrow_mut() = None);
    let _ = a.frame(); // bakes the objects
    let e = edit_of(&mut a);
    let origin = e.shot.origin;
    let at = Pt::new((300 - origin.0) as f32, (200 - origin.1) as f32);
    assert!(matches!(&e.objects[..], [Obj::Cursor { at: p, img }] if *p == at && img.dimensions() == (12, 20)), "{:?}", e.objects.len());
    // Drawn into what the capture exports.
    let px = e.composed().get_pixel(at.x as u32 + 1, at.y as u32 + 18);
    assert_eq!(px, [255, 0, 255, 255]);
    // No undo before it.
    e.undo();
    assert_eq!(e.objects.len(), 1);
    // Moved like any object.
    let mut o = e.objects[0].clone();
    crate::objects::edit::translate(&mut o, 5.0, 7.0);
    assert!(matches!(o, Obj::Cursor { at: p, .. } if p == Pt::new(at.x + 5.0, at.y + 7.0)));
    assert_eq!(crate::objects::edit::boxes(&o), crate::objects::edit::Boxes::None, "never resized");
    // Off: nothing.
    let mut b = app(None);
    b.st = State::Hidden;
    b.kind = RunKind::Daemon;
    crate::pointer::FAKE.with(|f| *f.borrow_mut() = Some(crate::pointer::Pointer { img: pointer_pic(), at: (300, 200) }));
    assert!(b.on_hot(HotEvent::Capture));
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(synthetic_shot())));
    b.on_event(Ev::Timer);
    crate::pointer::FAKE.with(|f| *f.borrow_mut() = None);
    assert!(edit_of(&mut b).objects.is_empty());
}

/// The Read QR code hotkey (slot 8, no default chord): pick an area; its
/// codes are read off the UI thread and the notice shows the link with
/// Copy and Open link (nothing opened).
#[test]
fn read_qr_hotkey_picks_an_area_then_reads() {
    assert!(Config::default().read_qr_hotkey.is_empty(), "no default chord");
    let mut a = app(None);
    a.st = State::Hidden;
    a.kind = RunKind::Daemon;
    assert!(a.on_hot(HotEvent::Pressed(8)));
    let p = a.pending.as_ref().expect("a capture");
    assert!(p.accept_on_select && matches!(p.tasks[..], [Task::Qr]));
    let mut shot = synthetic_shot();
    let code = crate::qr::render(&crate::qr::encode_text("https://example.com/qr", crate::qr::Ecl::M).unwrap(), 4, 4);
    let w = shot.image.width() as usize;
    for y in 0..code.height() as usize {
        for x in 0..code.width() as usize {
            let s = (y * code.width() as usize + x) * 4;
            let d = ((y + 300) * w + x + 300) * 4;
            let px: [u8; 4] = code.as_raw()[s..s + 4].try_into().unwrap();
            shot.image.as_raw_mut()[d..d + 4].copy_from_slice(&px);
        }
    }
    let _ = crate::notice::take_shown();
    FAKE_GRAB.with(|g| *g.borrow_mut() = Some(Ok(shot)));
    a.on_event(Ev::Timer);
    assert!(matches!(a.st, State::Edit(_)), "the overlay is up to pick the area");
    a.on_event(Ev::Down { x: 280, y: 280 });
    a.on_event(Ev::Move { x: 400, y: 400 });
    a.on_event(Ev::Move { x: 450, y: 450 });
    a.on_event(Ev::Up { x: 450, y: 450 });
    assert!(hidden(&a), "done when the selection ends");
    let t0 = Instant::now();
    let shown = loop {
        a.on_event(Ev::Timer);
        let s = crate::notice::take_shown();
        if !s.is_empty() {
            break s;
        }
        assert!(t0.elapsed() < Duration::from_secs(20), "the read never finished");
        std::thread::sleep(Duration::from_millis(10));
    };
    let n = &shown[0].0;
    assert!(n.text.contains("https://example.com/qr"), "{}", n.text);
    assert_eq!(n.actions, vec![crate::notice::NoteAct::QrCopy, crate::notice::NoteAct::QrOpen]);
    assert!(crate::qr_ui::OPENED.with(|o| o.borrow().is_empty()), "never opened by itself");
}
