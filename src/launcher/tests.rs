use super::*;
use std::sync::Mutex;

/// The tests that run a [`Delay`] share its Cancel flag: one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// What a countdown showed.
#[derive(Default)]
struct Shown(Vec<Option<u64>>);

impl Show for Shown {
    fn left(&mut self, n: Option<u64>) {
        self.0.push(n);
    }
}

#[test]
fn choices_round_trip_through_the_config() {
    let cfg = Config::default();
    assert_eq!(Plan::from_config(&cfg), Plan { source: Source::Region, delay: 0, after: After::Edit });
    for plan in [
        Plan { source: Source::Window, delay: 5, after: After::Pin },
        Plan { source: Source::Screen, delay: 60, after: After::Save },
        Plan { source: Source::Region, delay: 7, after: After::Copy },
    ] {
        // The edits the window writes, applied to a config file's text.
        let text = plan.config_edits().iter().fold(crate::config::to_toml(&cfg), |t, (k, v)| crate::config::with_top_raw(&t, k, v.as_deref()));
        let back = crate::config::parse_config(&text).expect("parses");
        assert_eq!(Plan::from_config(&back), plan, "{text}");
        // And the whole config round-trips with them.
        assert_eq!(crate::config::parse_config(&crate::config::to_toml(&back)).unwrap(), back);
    }
    // Out of range or unknown: the file does not parse (as other keys).
    assert!(crate::config::parse_config("launcher_delay = 61").is_err());
    assert!(crate::config::parse_config("launcher_source = \"monitor\"").is_err());
    assert!(crate::config::parse_config("launcher_after = \"upload\"").is_err());
    // A hand-edited odd value in a config still reads safely.
    let odd = Config { launcher_source: "nope".into(), launcher_after: "".into(), launcher_delay: 999, ..Config::default() };
    assert_eq!(Plan::from_config(&odd), Plan { source: Source::Region, delay: MAX_DELAY, after: After::Edit });
    for s in Source::ALL {
        assert_eq!(Source::from_id(s.id()), Some(s));
    }
    for a in After::ALL {
        assert_eq!(After::from_id(a.id()), Some(a));
    }
}

/// Each choice starts the capture it says: the source picks how the
/// overlay opens, After capture what follows (at once unless Edit).
#[test]
fn plans_start_their_captures() {
    let p = |source, after| Plan { source, delay: 0, after }.pending();
    let e = p(Source::Region, After::Edit);
    assert!(e.tasks.is_empty() && !e.accept_on_select && !e.select_all && !e.pick_window);
    let c = p(Source::Region, After::Copy);
    assert_eq!(c.tasks, vec![Task::Copy]);
    assert!(c.accept_on_select);
    let s = p(Source::Screen, After::Save);
    assert!(s.select_all && s.accept_on_select);
    assert_eq!(s.tasks, vec![Task::Save { path: None, ask: false }]);
    let w = p(Source::Window, After::Pin);
    assert!(w.pick_window && w.accept_on_select);
    assert_eq!(w.tasks, vec![Task::Pin]);
    let se = p(Source::Screen, After::Edit);
    assert!(se.select_all && !se.accept_on_select, "the whole screen in the editor");
    assert_eq!(Plan::region_in(3), Plan { source: Source::Region, delay: 3, after: After::Edit });
    assert_eq!(Plan::region_in(600).delay, MAX_DELAY);
}

/// The countdown (an injected clock): whole seconds, rounded up, each
/// shown once; due exactly at the delay.
#[test]
fn the_countdown_ticks_by_the_second() {
    let t0 = Instant::now();
    let ms = |n: u64| t0 + Duration::from_millis(n);
    let plan = Plan::region_in(3);
    let mut cd = Countdown::new(plan, t0);
    assert_eq!(cd.tick(t0), Tick::Wait(Some(3)));
    assert_eq!(cd.tick(ms(400)), Tick::Wait(None));
    assert_eq!(cd.left(ms(999)), 3);
    assert_eq!(cd.tick(ms(1000)), Tick::Wait(Some(2)));
    assert_eq!(cd.tick(ms(1900)), Tick::Wait(None));
    assert_eq!(cd.tick(ms(2001)), Tick::Wait(Some(1)));
    assert_eq!(cd.tick(ms(2999)), Tick::Wait(None));
    assert_eq!(cd.tick(ms(3000)), Tick::Fire(plan));
    assert_eq!(cd.left(ms(5000)), 0);
    assert_eq!(pill_text(3), "Capturing in 3 s");
    let n = pill(3);
    assert_eq!(n.actions, vec![crate::notice::NoteAct::CancelDelay]);
    assert!(n.corner, "bottom-right");
    assert_eq!(crate::notice::NoteAct::CancelDelay.label(), "Cancel");
}

#[test]
fn a_delay_shows_then_captures() {
    let _l = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let t0 = Instant::now();
    let s = |n: u64| t0 + Duration::from_secs(n);
    let mut d = Delay::default();
    let mut shown = Shown::default();
    // No delay: at once, nothing shown.
    let now = d.start(Plan { delay: 0, ..Plan::region_in(0) }, t0, &mut shown).expect("at once");
    assert!(now.tasks.is_empty());
    assert!(shown.0.is_empty() && !d.active());
    // Five seconds: 5, 4, ... 1, then the capture (the pill gone first).
    let plan = Plan { source: Source::Screen, delay: 5, after: After::Copy };
    assert!(d.start(plan, t0, &mut shown).is_none());
    assert!(d.active());
    for i in 1..5 {
        assert!(d.poll(s(i), &mut shown).is_none());
        assert!(d.poll(s(i) + Duration::from_millis(300), &mut shown).is_none());
    }
    let p = d.poll(s(5), &mut shown).expect("due");
    assert!(p.select_all && p.tasks == vec![Task::Copy]);
    assert_eq!(shown.0, vec![Some(5), Some(4), Some(3), Some(2), Some(1), None]);
    assert!(!d.active());
    assert!(d.poll(s(6), &mut shown).is_none());
}

/// Cancel (the pill's button from any thread, or the tray's) calls it
/// off: no capture, the pill and the tip go.
#[test]
fn a_delay_can_be_cancelled() {
    let _l = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let t0 = Instant::now();
    let mut d = Delay::default();
    let mut shown = Shown::default();
    d.start(Plan::region_in(10), t0, &mut shown);
    // The pill's Cancel (another thread).
    std::thread::spawn(cancel).join().unwrap();
    assert!(d.poll(t0 + Duration::from_secs(1), &mut shown).is_none());
    assert!(!d.active());
    assert!(d.poll(t0 + Duration::from_secs(11), &mut shown).is_none(), "never fires");
    assert_eq!(shown.0, vec![Some(10), None]);
    // The tray's Cancel.
    d.start(Plan::region_in(3), t0, &mut shown);
    assert!(d.cancel(&mut shown));
    assert!(!d.cancel(&mut shown), "nothing left to cancel");
    assert_eq!(shown.0[2..], [Some(3), None]);
    // A new launch replaces one that counts.
    d.start(Plan::region_in(10), t0, &mut shown);
    d.start(Plan::region_in(3), t0, &mut shown);
    assert_eq!(d.poll(t0 + Duration::from_secs(3), &mut shown).map(|p| p.tasks), Some(Vec::new()));
}

#[cfg(not(target_os = "macos"))]
mod window_tests {
    use super::super::window::{paint, Click, Form};
    use super::*;
    use crate::theme::{DARK, LIGHT};
    use crate::ui::{preview, FocusState, Input};
    use crate::wind::Ev;

    fn render(f: &mut Form, th: &crate::theme::Theme, k: f32, input: &Input, focus: &mut FocusState) -> (crate::pixbuf::PixBuf, Option<Click>) {
        preview::render(W, H, k, th, &Input::default(), focus, |ui| {
            paint(ui, f);
        });
        let mut click = None;
        let img = preview::render(W, H, k, th, input, focus, |ui| click = paint(ui, f));
        (img, click)
    }

    fn click(f: &mut Form, focus: &mut FocusState, id: &str, at: f32) -> Option<Click> {
        render(f, &DARK, 1.0, &Input::default(), focus);
        let r = focus.rect_of(id).unwrap_or_else(|| panic!("{id} drawn"));
        let (x, y) = ((r.x + r.w * at) as i32, (r.y + r.h / 2.0) as i32);
        let mut input = Input::default();
        for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
            input.feed(&e);
        }
        render(f, &DARK, 1.0, &input, focus).1
    }

    /// Clicks change the choices; the stepper goes past the presets
    /// (0–60); Capture and Cancel answer.
    #[test]
    fn the_window_changes_the_plan() {
        let mut f = Form::new(Plan::default());
        let mut focus = FocusState::default();
        assert_eq!(click(&mut f, &mut focus, "launcher.source", 0.5), None);
        assert_eq!(f.plan.source, Source::Screen, "the middle segment");
        click(&mut f, &mut focus, "launcher.after", 0.9);
        assert_eq!(f.plan.after, After::Pin);
        click(&mut f, &mut focus, "launcher.delay", 0.95);
        assert_eq!(f.plan.delay, 10);
        assert_eq!(f.delay_sel(), 3);
        click(&mut f, &mut focus, "launcher.more", 0.5);
        assert_eq!(f.plan.delay, 11);
        assert_eq!(f.delay_sel(), DELAYS.len(), "a custom delay: no preset lit");
        for _ in 0..80 {
            f.step(true);
        }
        assert_eq!(f.plan.delay, MAX_DELAY);
        f.plan.delay = 1;
        click(&mut f, &mut focus, "launcher.less", 0.5);
        click(&mut f, &mut focus, "launcher.less", 0.5);
        assert_eq!(f.plan.delay, 0);
        assert_eq!(click(&mut f, &mut focus, "launcher.capture", 0.5), Some(Click::Capture));
        assert_eq!(click(&mut f, &mut focus, "launcher.cancel", 0.5), Some(Click::Cancel));
        if cfg!(not(target_os = "macos")) {
            assert_eq!(click(&mut f, &mut focus, "launcher.qr", 0.5), Some(Click::MakeQr));
            let (qr, cancel) = (focus.rect_of("launcher.qr").unwrap(), focus.rect_of("launcher.cancel").unwrap());
            assert!(qr.x1() + 8.0 <= cancel.x, "{qr:?} {cancel:?}");
        }
    }

    /// Capture only closes the window: the plan goes out after the window
    /// is gone and the settle has run, never while it is on screen.
    #[test]
    fn the_plan_waits_for_the_window_to_go() {
        use super::super::window::Window;
        LAUNCHED.with(|l| l.borrow_mut().clear());
        let plan = Plan { source: Source::Screen, delay: 0, after: After::Copy };
        let mut w = Window::for_tests(plan);
        w.click(Click::Capture);
        assert_eq!(w.chosen(), Some(plan));
        assert!(LAUNCHED.with(|l| l.borrow().is_empty()), "not while the window exists");
        let settled = std::cell::Cell::new(false);
        after_close(w.chosen(), || {
            assert!(LAUNCHED.with(|l| l.borrow().is_empty()), "the settle comes first");
            settled.set(true);
        });
        assert!(settled.get());
        assert_eq!(LAUNCHED.with(|l| l.borrow().clone()), vec![plan]);
        // Cancel: nothing at all.
        let mut w = Window::for_tests(plan);
        w.click(Click::Cancel);
        after_close(w.chosen(), || panic!("no settle without a plan"));
        assert_eq!(LAUNCHED.with(|l| l.borrow().len()), 1);
        assert!(SETTLE >= Duration::from_millis(200), "past the DWM close fade");
    }

    /// The board's look, dark and light, at 1x and 2x (PNGs under
    /// RUSTSHOT_PREVIEW_DIR); the controls fit the 360 × 260 window.
    #[test]
    fn launcher_previews() {
        let mut f = Form::new(Plan { source: Source::Region, delay: 3, after: After::Edit });
        for (name, th) in [("dark", DARK), ("light", LIGHT)] {
            for k in [1.0f32, 2.0] {
                let mut focus = FocusState::default();
                let (img, _) = render(&mut f, &th, k, &Input::default(), &mut focus);
                assert_eq!(img.dimensions(), ((W as f32 * k) as u32, (H as f32 * k) as u32));
                for id in ["launcher.source", "launcher.delay", "launcher.less", "launcher.more", "launcher.after", "launcher.capture", "launcher.cancel"] {
                    let r = focus.rect_of(id).unwrap_or_else(|| panic!("{id}"));
                    assert!(r.x >= 0.0 && r.y >= 0.0 && r.x1() <= W as f32 * k + 0.5 && r.y1() <= H as f32 * k + 0.5, "{id} {r:?} inside");
                }
                // The delay presets end before the stepper starts.
                let (seg, less) = (focus.rect_of("launcher.delay").unwrap(), focus.rect_of("launcher.less").unwrap());
                assert!(seg.x1() <= less.x, "{seg:?} {less:?}");
                preview::save(&format!("launcher_{name}_{k}x.png"), &img);
            }
        }
        // A custom delay.
        let mut f = Form::new(Plan { source: Source::Window, delay: 17, after: After::Save });
        let mut focus = FocusState::default();
        let (img, _) = render(&mut f, &DARK, 1.0, &Input::default(), &mut focus);
        preview::save("launcher_dark_custom.png", &img);
    }

    /// The window fits in English and the long pseudo-languages, at 1x
    /// and 2x (previews per language).
    #[test]
    fn fits_in_long_languages() {
        for lang in preview::LANGS {
            crate::i18n::with_lang(lang, || {
                for k in [1.0f32, 2.0] {
                    let mut f = Form::new(Plan { source: Source::Region, delay: 3, after: After::Edit });
                    let mut focus = FocusState::default();
                    let (img, _) = render(&mut f, &DARK, k, &Input::default(), &mut focus);
                    preview::assert_fits(&focus, W, H, k, &format!("launcher {lang} {k}x"));
                    if k == 1.0 {
                        preview::save(&format!("launcher-{lang}.png"), &img);
                    }
                }
            });
        }
    }
}
