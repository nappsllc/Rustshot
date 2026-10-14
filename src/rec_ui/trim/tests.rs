use super::*;
use crate::wind::key;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn handles_keep_their_order_and_a_minimum() {
    let mut t = Trim::new(ms(10_000), 30);
    assert!(!t.trims());
    assert_eq!(t.choice(), Choice::Full, "nothing cut: kept as it is");
    t.set_start(ms(2_000));
    t.set_end(ms(7_500));
    assert_eq!((t.start, t.end, t.selected()), (ms(2_000), ms(7_500), ms(5_500)));
    assert!(t.trims());
    assert_eq!(t.choice(), Choice::Trimmed { start: ms(2_000), end: ms(7_500) });
    // Past each other: stopped MIN_LEN apart.
    t.set_start(ms(9_000));
    assert_eq!(t.start, ms(7_500) - MIN_LEN);
    t.set_end(ms(100));
    assert_eq!(t.end, t.start + MIN_LEN);
    // Never past the recording.
    t.set_end(ms(60_000));
    assert_eq!(t.end, ms(10_000));
}

#[test]
fn presses_take_the_handle_within_reach_else_the_head() {
    let l = layout();
    let mut t = Trim::new(ms(10_000), 30);
    t.set_start(ms(2_000));
    t.set_end(ms(8_000));
    let sx = x_of(&l, t.len, t.start);
    assert_eq!(near_handle(&l, &t, sx + REACH - 1.0), Some(Grab::Start));
    assert_eq!(near_handle(&l, &t, x_of(&l, t.len, t.end) - 3.0), Some(Grab::End));
    assert_eq!(near_handle(&l, &t, x_of(&l, t.len, ms(5_000))), None);
    // A drag of the start handle.
    t.press(time_at(&l, t.len, sx), Some(Grab::Start));
    t.drag(time_at(&l, t.len, x_of(&l, t.len, ms(3_000))));
    t.release();
    assert!((t.start.as_millis() as i64 - 3_000).abs() < 20, "{:?}", t.start);
    assert_eq!(t.head, t.start, "the preview follows the start");
    // Elsewhere: the playhead.
    t.press(ms(6_000), None);
    assert_eq!((t.grab, t.head), (Some(Grab::Head), ms(6_000)));
    t.release();
    assert_eq!(t.grab, None);
    // Close handles: the nearer.
    let mut c = Trim::new(ms(10_000), 30);
    c.set_start(ms(5_000));
    c.set_end(ms(5_200));
    let mid = (x_of(&l, c.len, c.start) + x_of(&l, c.len, c.end)) / 2.0;
    assert_eq!(near_handle(&l, &c, mid - 1.0), Some(Grab::Start));
    assert_eq!(near_handle(&l, &c, mid + 1.0), Some(Grab::End));
}

#[test]
fn playing_runs_through_the_selection_once() {
    let t0 = Instant::now();
    let mut t = Trim::new(ms(10_000), 30);
    t.set_start(ms(1_000));
    t.set_end(ms(3_000));
    t.seek(ms(9_000));
    t.toggle_play(t0);
    assert!(t.playing());
    assert_eq!(t.head, ms(1_000), "outside the selection: from its start");
    assert!(t.tick(t0 + ms(500)));
    assert_eq!(t.head, ms(1_500));
    t.toggle_play(t0 + ms(600));
    assert!(!t.playing(), "paused");
    assert!(!t.tick(t0 + ms(900)), "paused: no ticking");
    t.toggle_play(t0 + ms(1_000));
    assert!(!t.tick(t0 + ms(5_000)), "past the end: it stops");
    assert!(t.head < ms(3_000) && t.head >= ms(3_000) - t.frame);
    // Play again: from the start.
    t.toggle_play(t0 + ms(6_000));
    assert_eq!(t.head, ms(1_000));
}

#[test]
fn keys() {
    let now = Instant::now();
    let mut t = Trim::new(ms(10_000), 25);
    assert_eq!(t.frame, ms(40));
    t.key(key::RIGHT, false, now);
    assert_eq!(t.head, ms(40), "a frame");
    t.key(key::RIGHT, true, now);
    assert_eq!(t.head, ms(1_040), "Shift: a second");
    t.key(0x49, false, now); // I
    assert_eq!(t.start, ms(1_040));
    t.seek(ms(4_000));
    t.key(0x4F, false, now); // O
    assert_eq!(t.end, ms(4_040), "the head's frame stays in");
    t.key(key::HOME, false, now);
    assert_eq!(t.head, t.start);
    t.key(key::SPACE, false, now);
    assert!(t.playing());
    assert_eq!(t.key(key::RETURN, false, now), Some(Choice::Trimmed { start: ms(1_040), end: ms(4_040) }));
    assert_eq!(t.key(key::ESCAPE, false, now), Some(Choice::Full));
    assert_eq!(t.key(0x41, false, now), None);
}

#[test]
fn times_read_with_tenths() {
    assert_eq!(fmt_time(ms(0)), "0:00.0");
    assert_eq!(fmt_time(ms(3_250)), "0:03.2");
    assert_eq!(fmt_time(ms(754_900)), "12:34.9");
    assert_eq!(fmt_time(ms(3_723_400)), "1:02:03.4");
}

#[test]
fn layout_fits_the_window() {
    let l = layout();
    assert!(l.preview.y >= 0.0 && l.preview.h > 200.0);
    assert!(l.preview.y1() < l.play.y && l.play.y1() < l.strip.y && l.strip.y1() + 20.0 <= l.footer);
    assert!(l.strip.x1() <= W as f32 - PAD + 0.5);
    assert_eq!(time_at(&l, ms(10_000), l.strip.x), ms(0));
    assert_eq!(time_at(&l, ms(10_000), l.strip.x1() + 50.0), ms(10_000));
    assert!((x_of(&l, ms(10_000), ms(5_000)) - (l.strip.x + l.strip.w / 2.0)).abs() < 0.01);
}

/// The window drawn (dark and light, previews under RUSTSHOT_PREVIEW_DIR):
/// the buttons are there, Save trimmed only does something once trimmed.
#[cfg(not(target_os = "macos"))]
#[test]
fn trim_window_preview() {
    use crate::ui::{FocusState, Input};
    for (name, th) in [("dark", &crate::theme::DARK), ("light", &crate::theme::LIGHT)] {
        let mut t = Trim::new(ms(12_000), 30);
        t.set_start(ms(1_500));
        t.set_end(ms(9_000));
        t.seek(ms(4_200));
        let thumb = crate::pixbuf::PixBuf::from_pixel(72, 56, [70, 110, 160, 255]);
        let pics = window::Pics {
            ready: Some(true),
            thumbs: (0..THUMBS).map(|_| Some(thumb.clone())).collect(),
            frame: Some((ms(4_200), crate::pixbuf::PixBuf::from_pixel(600, 320, [40, 60, 90, 255]))),
        };
        let mut focus = FocusState::default();
        let img = crate::ui::preview::render(W, H, 1.0, th, &Input::default(), &mut focus, |ui| {
            assert_eq!(window::paint(ui, &mut t, &pics, Instant::now()), None, "nothing pressed");
        });
        for id in ["trim.play", "trim.save", "trim.keep"] {
            assert!(focus.rect_of(id).is_some(), "{name}: {id}");
        }
        // The selection's start handle is the accent.
        let l = layout();
        let (x, y) = (x_of(&l, t.len, t.start), l.strip.y + 4.0);
        let p = img.get_pixel(x as u32, y as u32);
        let ac = th.accent;
        assert!(p[0].abs_diff(ac.r) < 8 && p[1].abs_diff(ac.g) < 8 && p[2].abs_diff(ac.b) < 8, "{name}: handle {p:?}");
        crate::ui::preview::save(&format!("trim_window_{name}.png"), &img);
    }
}

/// The window fits in English and the long pseudo-languages (accented,
/// right to left, Arabic where it is shaped), at 1× and 2×: every control
/// inside, none over another, no text cut.
#[cfg(not(target_os = "macos"))]
#[test]
fn trim_window_fits() {
    use crate::ui::{preview, FocusState, Input};
    for lang in preview::LANGS {
        crate::i18n::with_lang(lang, || {
            for k in [1.0f32, 2.0] {
                for ready in [None, Some(false), Some(true)] {
                    let mut t = Trim::new(ms(3_723_400), 30);
                    t.set_start(ms(1_500));
                    t.seek(ms(4_200));
                    let pics = window::Pics { ready, ..Default::default() };
                    let mut focus = FocusState::default();
                    let img = preview::render(W, H, k, &crate::theme::DARK, &Input::default(), &mut focus, |ui| {
                        window::paint(ui, &mut t, &pics, Instant::now());
                    });
                    preview::assert_fits(&focus, W, H, k, &format!("trim {lang} {ready:?} {k}x"));
                    if k == 1.0 && ready == Some(false) {
                        preview::save(&format!("trim_window_{lang}.png"), &img);
                    }
                }
            }
        });
    }
}

/// Right to left the window is mirrored (Play and the times on the
/// right, the buttons on the left) but its media is not: the preview, the
/// filmstrip with its selection and playhead, and the Play glyph are the
/// left-to-right pixels at their mirrored places, and a press lands at
/// the time it shows over.
#[cfg(not(target_os = "macos"))]
#[test]
fn right_to_left_mirrors_the_window_not_its_media() {
    use crate::pixbuf::PixBuf;
    use crate::ui::{FocusState, Input};
    use crate::wind::Ev;
    // Pictures that read one way: a left-to-right ramp, each thumbnail
    // its own colour.
    let ramp = |w: u32, h: u32, g: u8| {
        let d = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, g, (y * 255 / h) as u8, 255])).collect();
        PixBuf::from_raw(w, h, d).unwrap()
    };
    let pics = window::Pics {
        ready: Some(true),
        thumbs: (0..THUMBS).map(|i| Some(ramp(72, 56, (i * 30) as u8))).collect(),
        frame: Some((ms(4_200), ramp(600, 320, 90))),
    };
    let trim = || {
        let mut t = Trim::new(ms(12_000), 30);
        t.set_start(ms(1_500));
        t.set_end(ms(9_000));
        t.seek(ms(4_200));
        t
    };
    let frame = |rtl: bool, input: &Input| {
        let run = || {
            let mut t = trim();
            let mut focus = FocusState::default();
            let img = crate::ui::preview::render(W, H, 1.0, &crate::theme::DARK, input, &mut focus, |ui| {
                window::paint(ui, &mut t, &pics, Instant::now());
            });
            (img, t)
        };
        if rtl { crate::i18n::with_rtl(run) } else { run() }
    };
    let (ltr, _) = frame(false, &Input::default());
    let (rtl, _) = frame(true, &Input::default());
    crate::ui::preview::save("trim_window_rtl_forced.png", &rtl);
    let crop = |img: &PixBuf, (x0, y0, x1, y1): (u32, u32, u32, u32)| -> Vec<[u8; 4]> {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| img.get_pixel(x, y)).collect()
    };
    let at = |r: FRect, grow: f32| (((r.x - grow).max(0.0)) as u32, ((r.y - grow).max(0.0)) as u32, ((r.x1() + grow).min(W as f32)) as u32, ((r.y1() + grow).min(H as f32)) as u32);
    let mirror = |(x0, y0, x1, y1): (u32, u32, u32, u32)| (W - x1, y0, W - x0, y1);
    let l = layout();
    for (what, r, grow) in [("the preview", l.preview, -4.0), ("the filmstrip", l.strip, 0.0), ("Play", l.play, 0.0)] {
        let b = at(r, grow);
        let a = crop(&ltr, b);
        let row: Vec<[u8; 4]> = a.iter().rev().copied().collect();
        assert_ne!(a, row, "{what}: something that reads one way");
        assert!(a == crop(&rtl, mirror(b)), "{what} at {b:?} reads left to right, at its mirrored place");
    }
    // The frame is mirrored: Play is not on the left any more.
    let play = at(l.play, 0.0);
    assert!(crop(&ltr, play) != crop(&rtl, play), "Play moved to the right");
    // A press where 6 s shows (on screen, the strip reads left to right)
    // takes the playhead there.
    let x = x_of(&l, ms(12_000), ms(6_000)).round() as i32;
    let y = (l.strip.y + l.strip.h / 2.0) as i32;
    let mut inp = Input::default();
    crate::i18n::with_rtl(|| inp.feed(&crate::ui::rtl_ev(Ev::Down { x, y }, W)));
    let (_, t) = frame(true, &inp);
    assert_eq!(t.grab, Some(Grab::Head));
    assert!((t.head.as_millis() as i64 - 6_000).abs() < 60, "the head at {:?}", t.head);
}

#[test]
fn applying_a_choice() {
    let dir = std::env::temp_dir().join(format!("rustshot-trim-apply-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("rec.gif");
    let mut enc = crate::rec::gif::GifEncoder::new(std::io::Cursor::new(Vec::new()), 20, 20, 10).unwrap();
    for k in 0..10u64 {
        use crate::rec::VideoEncoder;
        let px = [k as u8 * 20, 30, 30, 255].repeat(400);
        enc.push_video(&crate::rec::Frame::bgra(20, 20, px, ms(k * 100))).unwrap();
    }
    std::fs::write(&p, enc.into_inner().unwrap().into_inner()).unwrap();
    assert_eq!(apply(&p, Choice::Full), Outcome::Full);
    assert_eq!(apply(&p, Choice::Trimmed { start: ms(200), end: ms(600) }), Outcome::Trimmed(p.clone(), ms(400)));
    assert_eq!(crate::rec::cut::length(&p).unwrap(), ms(400), "the trimmed file has the recording's name");
    assert!(!dir.join("rec-trimmed.gif").exists());
    assert!(matches!(apply(&dir.join("gone.gif"), Choice::Trimmed { start: ms(0), end: ms(100) }), Outcome::Failed(_)));
    let _ = std::fs::remove_dir_all(&dir);
}
