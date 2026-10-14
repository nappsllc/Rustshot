use super::*;
use crate::rec::Nv12 as Nv12Pic;

const ALL: MarkOpts = MarkOpts { clicks: true, keys: true, all_keys: true };
const CHORDS: MarkOpts = MarkOpts { clicks: true, keys: true, all_keys: false };

/// The keys `vks` pressed together: the modifiers among them held while
/// the others go down.
fn press(m: &Marks, vks: &[u32], t: Instant) {
    let has = |a: &[u32]| vks.iter().any(|v| a.contains(v));
    let held = Held { ctrl: has(&[0x11, 0xA2, 0xA3]), alt: has(&[0x12, 0xA4, 0xA5]), shift: has(&[0x10, 0xA0, 0xA1]), win: has(&[0x5B, 0x5C]) };
    for &v in vks.iter().filter(|v| !is_modifier(**v)) {
        m.key(v, held, false, t);
    }
}

/// Pausing (and the secure desktop) forgets what was shown and takes
/// nothing until resumed: a password typed while paused never shows.
#[test]
fn a_pause_forgets_and_ignores() {
    let t = Instant::now();
    let m = Marks::new(ALL);
    press(&m, &[0xA2, 0x53], t);
    m.click(1, 1, Button::Left, t);
    m.suspend(true);
    assert!(m.scene(t).is_empty(), "nothing while paused");
    press(&m, &[0x50], t);
    m.click(2, 2, Button::Left, t);
    m.suspend(false);
    assert!(m.scene(t).is_empty(), "what came while paused is gone");
    press(&m, &[0x51], t);
    assert_eq!(m.scene(t).pill.map(|p| p.0).as_deref(), Some("Q"));
}

#[test]
fn chords_read_like_shortcuts() {
    let h = |ctrl, alt, shift, win| Held { ctrl, alt, shift, win };
    assert_eq!(chord_text(h(true, false, true, false), 0x53, false).as_deref(), Some("Ctrl + Shift + S"));
    assert_eq!(chord_text(h(false, true, false, false), 0x73, false).as_deref(), Some("Alt + F4"));
    let win = format!("{} + E", crate::keymap::META_NAME);
    assert_eq!(chord_text(h(false, false, false, true), 0x45, false), Some(win));
    // Without Ctrl, Alt or Win: only with every key shown.
    assert_eq!(chord_text(h(false, false, true, false), 0x41, false), None, "Shift + A is typing");
    assert_eq!(chord_text(Held::default(), 0x41, false), None);
    assert_eq!(chord_text(Held::default(), 0x41, true).as_deref(), Some("A"));
    assert_eq!(chord_text(h(false, false, true, false), 0x41, true).as_deref(), Some("Shift + A"));
    // A modifier alone, or an unnamed key, is never a chord.
    assert_eq!(chord_text(h(true, false, false, false), 0xA2, true), None);
    assert_eq!(chord_text(h(true, false, false, false), 0xFF, true), None);
    assert_eq!(key_label(0x26).as_deref(), Some("\u{2191}"));
    assert_eq!(key_label(0xBF).as_deref(), Some("/"));
}

#[test]
fn keys_build_the_pill_and_fade() {
    let t0 = Instant::now();
    let m = Marks::new(CHORDS);
    // Typing shows nothing.
    press(&m, &[0x48], t0);
    press(&m, &[0xA0, 0x49], t0);
    assert_eq!(m.scene(t0).pill, None);
    press(&m, &[0xA2, 0xA0, 0x53], t0);
    let s = m.scene(t0 + Duration::from_millis(100));
    assert_eq!(s.pill, Some(("Ctrl + Shift + S".to_string(), 1.0)));
    // Still at 1.5 s, half gone 150 ms later, gone after the fade.
    assert_eq!(m.scene(t0 + KEY_FADE_AFTER).pill.map(|p| p.1), Some(1.0));
    let half = m.scene(t0 + KEY_FADE_AFTER + KEY_FADE / 2).pill.expect("fading").1;
    assert!((half - 0.5).abs() < 0.02, "{half}");
    assert_eq!(m.scene(t0 + KEY_FADE_AFTER + KEY_FADE + Duration::from_millis(1)).pill, None);
}

#[test]
fn the_pill_keeps_the_last_chords_and_counts_repeats() {
    let t0 = Instant::now();
    let m = Marks::new(CHORDS);
    let ms = |n: u64| t0 + Duration::from_millis(n);
    press(&m, &[0xA2, 0x43], ms(0));
    press(&m, &[0xA2, 0x56], ms(200));
    press(&m, &[0xA2, 0x56], ms(400));
    assert_eq!(m.scene(ms(450)).pill.unwrap().0, "Ctrl + C   Ctrl + V \u{d7}2");
    press(&m, &[0xA2, 0x5A], ms(600));
    press(&m, &[0xA2, 0x59], ms(800));
    assert_eq!(m.scene(ms(850)).pill.unwrap().0, "Ctrl + V \u{d7}2   Ctrl + Z   Ctrl + Y", "the last three");
    // After a pause the pill starts over.
    press(&m, &[0xA4, 0x09], ms(800) + KEY_FADE_AFTER + Duration::from_millis(10));
    assert_eq!(m.scene(ms(800) + KEY_FADE_AFTER + Duration::from_millis(20)).pill.unwrap().0, "Alt + Tab");
}

#[test]
fn password_fields_and_options_hide_keys() {
    let t = Instant::now();
    let m = Marks::new(ALL);
    m.key(0x41, Held::default(), true, t);
    assert_eq!(m.scene(t).pill, None, "never in a password field");
    m.key(0x41, Held::default(), false, t);
    assert_eq!(m.scene(t).pill.unwrap().0, "A", "every key, asked for");
    let off = Marks::new(MarkOpts { clicks: true, keys: false, all_keys: true });
    press(&off, &[0xA2, 0x53], t);
    assert_eq!(off.scene(t).pill, None, "keystrokes off");
    let no_clicks = Marks::new(MarkOpts { clicks: false, keys: true, all_keys: false });
    no_clicks.click(5, 5, Button::Left, t);
    assert!(no_clicks.scene(t).rings.is_empty(), "clicks off");
}

#[test]
fn clicks_pulse_and_go() {
    let t0 = Instant::now();
    let m = Marks::new(ALL);
    m.click(100, 50, Button::Left, t0);
    m.click(30, 40, Button::Right, t0 + Duration::from_millis(200));
    let s = m.scene(t0 + Duration::from_millis(300));
    assert_eq!(s.rings.len(), 2);
    assert!((s.rings[0].1 - 300.0 / 450.0).abs() < 0.01);
    assert_eq!(m.scene(t0 + RING_LIFE + Duration::from_millis(1)).rings.len(), 1, "the first is gone");
    assert!(m.scene(t0 + Duration::from_secs(5)).is_empty());
}

fn gray(w: u32, h: u32) -> Frame {
    Frame::bgra(w, h, [80u8, 80, 80, 255].repeat((w * h) as usize), Duration::ZERO)
}

fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * f.w + x) * 4) as usize;
    f.bgra[i..i + 4].try_into().unwrap()
}

#[test]
fn rings_and_the_pill_are_drawn_into_bgra_frames() {
    let (w, h) = (400u32, 300u32);
    let area = (1000, 500, w, h);
    let t = Instant::now();
    let m = Marks::new(ALL);
    m.click(1000 + 200, 500 + 100, Button::Left, t);
    m.click(1000 + 60, 500 + 60, Button::Right, t);
    press(&m, &[0xA2, 0x53], t);
    let mut f = gray(w, h);
    let mut p = Painter::new(1.0);
    p.draw(&mut f, area, &m.scene(t + Duration::from_millis(50)));
    // The left ring is yellow-ish on its circle (r ≈ 6 + 20 · ease(0.11)).
    let e = 1.0 - (1.0 - 50.0 / 450.0f32).powi(2);
    let r = (RING_R0 + (RING_R1 - RING_R0) * e).round() as u32;
    let on = px(&f, 200 + r, 100);
    assert!(on[2] > 150 && on[1] > 130 && on[0] < 120, "left ring {on:?}");
    let right = px(&f, 60 + r, 60);
    assert!(right[0] > 150 && right[2] < 120, "right ring is blue {right:?}");
    assert_eq!(px(&f, 300, 30), [80, 80, 80, 255], "elsewhere untouched");
    // The pill: dark at the bottom centre, with white text in it.
    let y = h - PILL_BOTTOM as u32 - PILL_H as u32 / 2;
    let row: Vec<[u8; 4]> = (0..w).map(|x| px(&f, x, y)).collect();
    assert!(row[(w / 2) as usize - 60][0] < 40 || row.iter().any(|p| p[0] < 40), "the pill's panel");
    assert!(row.iter().any(|p| p[0] > 200), "its text");
    assert_eq!(px(&f, 5, y), [80, 80, 80, 255], "only at the centre");
}

#[test]
fn nv12_frames_get_a_drawn_copy() {
    let (w, h) = (320u32, 240u32);
    let shared = Arc::new(vec![100u8; crate::rec::yuv::size(w, h)]);
    // The picture is scaled: the area is twice as large.
    let mut f = Frame { w: w * 2, h: h * 2, bgra: Vec::new(), ts: Duration::ZERO, nv12: Some(Nv12Pic { w, h, data: shared.clone() }) };
    let t = Instant::now();
    let m = Marks::new(ALL);
    m.click(200, 200, Button::Left, t); // (100, 100) in the picture
    let mut p = Painter::new(1.0);
    p.draw(&mut f, (0, 0, w * 2, h * 2), &m.scene(t + Duration::from_millis(200)));
    assert!(shared.iter().all(|&v| v == 100), "the source's picture is untouched");
    let out = &f.nv12.as_ref().unwrap().data;
    let e = 1.0 - (1.0 - 200.0 / 450.0f32).powi(2);
    let r = ((RING_R0 + (RING_R1 - RING_R0) * e) * 0.5).round() as usize;
    let yv = out[100 * w as usize + 100 + r];
    assert!(yv > 120, "the ring is bright in Y: {yv}");
    assert_eq!(out[10 * w as usize + 10], 100, "elsewhere untouched");
}

#[test]
fn yuv_matches_the_encoders_conversion() {
    assert_eq!(yuv(0, 0, 0), [16, 128, 128]);
    assert_eq!(yuv(255, 255, 255), [235, 128, 128]);
    let [y, u, v] = yuv(255, 0, 0);
    assert!((y as i32 - 63).abs() <= 1 && (u as i32 - 102).abs() <= 1 && (v as i32 - 240).abs() <= 1, "{y} {u} {v}");
}

/// A source of gray frames.
struct Gray(u32, u32);

impl FrameSource for Gray {
    fn next(&mut self, _deadline: Instant) -> Option<Frame> {
        Some(gray(self.0, self.1))
    }
    fn size(&self) -> (u32, u32) {
        (self.0, self.1)
    }
}

#[test]
fn only_an_armed_recording_is_wrapped() {
    let (src, late) = wrap(Box::new(Gray(8, 8)), (0, 0, 8, 8));
    assert!(late.is_none(), "nothing armed: the source as it is");
    assert_eq!(src.size(), (8, 8));
    arm(MarkOpts::default());
    assert!(wrap(Box::new(Gray(8, 8)), (0, 0, 8, 8)).1.is_none(), "all off is not armed");
    arm(CHORDS);
    let (_src, late) = wrap(Box::new(Gray(8, 8)), (0, 0, 8, 8));
    assert!(late.is_some(), "armed: wrapped");
    assert!(wrap(Box::new(Gray(8, 8)), (0, 0, 8, 8)).1.is_none(), "taken once");
}

#[test]
fn a_marked_source_draws_what_the_marks_hold() {
    let m = Arc::new(Marks::new(ALL));
    let mut src = Marked::new(Box::new(Gray(200, 150)), m.clone(), (0, 0, 200, 150), 1.0);
    src.tried = true; // no hooks in tests
    m.click(100, 75, Button::Left, Instant::now());
    let f = src.next(Instant::now()).expect("frame");
    assert!(f.bgra.as_chunks::<4>().0.iter().any(|p| p[2] > 150), "a ring");
    assert_eq!(src.size(), (200, 150));
}

/// Perf: drawing costs nothing without marks, and a ring plus the pill
/// stay well under 0.2 ms a 1080p frame (BGRA; NV12 copies the picture
/// once while something shows). Release:
/// `cargo test --release marks_perf -- --ignored --nocapture`
#[test]
#[ignore = "benchmark"]
fn marks_perf() {
    let (w, h) = (1920u32, 1080u32);
    let m = Marks::new(ALL);
    let mut p = Painter::new(1.0);
    let mut f = gray(w, h);
    let area = (0, 0, w, h);
    let n = 200;
    let t = Instant::now();
    for _ in 0..n {
        let s = m.scene(Instant::now());
        p.draw(&mut f, area, &s);
    }
    let idle = t.elapsed() / n;
    let t0 = Instant::now();
    m.click(900, 500, Button::Left, t0);
    press(&m, &[0xA2, 0xA0, 0x53], t0);
    let t = Instant::now();
    for _ in 0..n {
        p.draw(&mut f, area, &m.scene(t0 + Duration::from_millis(100)));
    }
    let busy = t.elapsed() / n;
    let nv = Arc::new(vec![100u8; crate::rec::yuv::size(w, h)]);
    let mut fv = Frame { w, h, bgra: Vec::new(), ts: Duration::ZERO, nv12: Some(Nv12Pic { w, h, data: nv.clone() }) };
    let t = Instant::now();
    for _ in 0..n {
        fv.nv12.as_mut().unwrap().data = nv.clone();
        p.draw(&mut fv, area, &m.scene(t0 + Duration::from_millis(100)));
    }
    let busy_nv = t.elapsed() / n;
    println!("marks per 1080p frame: idle {idle:?}, ring + pill {busy:?} (BGRA), {busy_nv:?} (NV12, with the copy)");
    if !cfg!(debug_assertions) {
        assert!(idle < Duration::from_micros(20), "{idle:?}");
        assert!(busy < Duration::from_micros(200), "{busy:?}");
    }
}
