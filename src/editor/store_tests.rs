//! Microsoft Store screenshots of the overlay (docs/store/screenshots):
//! 1920 x 1080 frames over a sample desktop with real text, so Text mode
//! and Quick Redact have words to show. `RUSTSHOT_PREVIEW_DIR=<dir>
//! cargo test store_screenshots -- --ignored` writes `<dir>/en/store-*.png`
//! (and `<dir>/<tag>/` for each catalog `RUSTSHOT_FIT_LANGS` names; `all`:
//! every `lang/*.txt`). The sample desktop stays English.

use super::tests::{edit_of, preview_app_with, settled_frame};
use super::*;
use crate::ocr::{Line, Word};
use crate::uifb::{Fb, text_width};

const W: u32 = 1920;
const H: u32 = 1080;
/// The message's body text: left edge, first line's top, line pitch, size.
const BODY: (f32, f32, f32, f32) = (520.0, 300.0, 40.0, 21.0);
const LINES: [&str; 6] = [
    "Hi team, the March invoice was charged twice.",
    "Contact: jane.cooper@example.com or +1 415 555 0132",
    "Card on file: 4111 1111 1111 1111",
    "Server 203.0.113.42 returned 502 after the deploy.",
    "API key: 7f3c9a1e5b2d8c4f6a0e9b1d",
    "Can we refund the second charge today?",
];
/// The area Text mode and Quick Redact read (the message).
const TEXT_SEL: FRect = FRect { x: 490.0, y: 170.0, w: 1100.0, h: 380.0 };

fn ink(c: [u8; 3]) -> C4 {
    C4::rgb(c[0], c[1], c[2])
}

/// A support inbox on a neutral wallpaper, drawn with the UI font.
fn store_shot() -> Shot {
    let mut img = PixBuf::from_pixel(W, H, [0, 0, 0, 255]);
    for y in 0..H {
        let t = y as f32 / H as f32;
        let c = [(64.0 + 34.0 * t) as u8, (72.0 + 36.0 * t) as u8, (88.0 + 40.0 * t) as u8];
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            img.as_raw_mut()[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let font = fonts::ui_font();
    {
        let mut fb = Fb::new(img.as_raw_mut(), W as usize);
        let dark = ink([28, 32, 40]);
        let grey = ink([108, 116, 130]);
        // The window and its title bar.
        fb.fill_rounded(220, 120, 1480, 840, 10.0, ink([250, 250, 252]));
        fb.fill_rect(220, 130, 1480, 34, ink([236, 239, 243]));
        fb.fill_rounded(220, 120, 1480, 30, 10.0, ink([236, 239, 243]));
        fb.draw_text(font, 15.0, "Inbox \u{b7} Support", 244.0, 132.0, dark);
        // Minimize, maximize, close.
        fb.fill_rect(1586, 142, 12, 1, grey);
        fb.stroke_rect(1622.0, 136.0, 11.0, 11.0, 1.0, grey);
        let mut sf = fb.surf();
        sf.stroke_polyline(&[Pt::new(1658.0, 136.0), Pt::new(1669.0, 147.0)], 1.2, grey, crate::raster::Blend::Normal);
        sf.stroke_polyline(&[Pt::new(1669.0, 136.0), Pt::new(1658.0, 147.0)], 1.2, grey, crate::raster::Blend::Normal);
        // The sidebar.
        fb.fill_rect(220, 164, 250, 796, ink([243, 245, 248]));
        fb.fill_rounded(232, 182, 226, 36, 8.0, ink([222, 228, 250]));
        for (i, (name, n)) in [("Inbox", "12"), ("Starred", ""), ("Sent", ""), ("Drafts", "2"), ("Archive", ""), ("Billing", "4"), ("Operations", "")].iter().enumerate() {
            let y = 190.0 + i as f32 * 44.0;
            fb.fill_circle(258.0, y + 10.0, 5.0, ink([[92, 108, 230], [240, 170, 70], [110, 190, 140], [160, 168, 180]][i % 4]));
            fb.draw_text(font, 17.0, name, 276.0, y, dark);
            if !n.is_empty() {
                fb.draw_text(font, 15.0, n, 430.0, y + 1.0, grey);
            }
        }
        // The message.
        fb.draw_text(font, 30.0, "Re: Double charge on the March invoice", 520.0, 186.0, dark);
        fb.fill_circle(535.0, 254.0, 14.0, ink([240, 170, 70]));
        fb.draw_text(font, 14.0, "JC", 526.0, 246.0, ink([255, 255, 255]));
        fb.draw_text(font, 17.0, "Jane Cooper  \u{b7}  Today, 9:41", 560.0, 244.0, grey);
        let (x, y0, pitch, px) = BODY;
        for (i, l) in LINES.iter().enumerate() {
            fb.draw_text(font, px, l, x, y0 + i as f32 * pitch, dark);
        }
        // A chart card and a details card.
        fb.fill_rounded(500, 590, 600, 330, 12.0, ink([241, 243, 247]));
        fb.draw_text(font, 18.0, "Refunds this week", 524.0, 608.0, dark);
        let bars = [120, 180, 90, 210, 160, 240, 140];
        for (i, h) in bars.iter().enumerate() {
            let c = if i == 5 { [92, 108, 230] } else { [168, 180, 230] };
            fb.fill_rounded(540 + i as i32 * 76, 890 - h, 44, *h, 6.0, ink(c));
            fb.draw_text(font, 13.0, ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][i], 548.0 + i as f32 * 76.0, 896.0, grey);
        }
        fb.fill_rounded(1130, 590, 540, 330, 12.0, ink([241, 243, 247]));
        fb.draw_text(font, 18.0, "Ticket details", 1154.0, 608.0, dark);
        for (i, (k, v)) in [("Status", "Open"), ("Priority", "High"), ("Assignee", "Sam Lee"), ("Plan", "Business"), ("Opened", "Mar 14, 2026")].iter().enumerate() {
            let y = 660.0 + i as f32 * 48.0;
            fb.draw_text(font, 17.0, k, 1154.0, y, grey);
            fb.draw_text(font, 17.0, v, 1330.0, y, dark);
            if i < 4 {
                fb.fill_rect(1154, y as i32 + 34, 492, 1, ink([224, 228, 234]));
            }
        }
    }
    Shot { origin: (0, 0), size: (W, H), scale: 1.0, image: img, monitors: vec![(0, 0, W, H)] }
}

/// The words of the message where [`store_shot`] drew them, relative to
/// [`TEXT_SEL`] (a fake recognizer: what Windows OCR would return).
fn store_doc(_: &PixBuf) -> Result<Vec<Line>, crate::ocr::OcrError> {
    let font = fonts::ui_font();
    let (x0, y0, pitch, px) = BODY;
    let space = text_width(font, px, " ");
    let line = |y: f32, text: &str, px: f32, x0: f32| {
        let mut x = x0;
        let mut words = Vec::new();
        for t in text.split(' ') {
            let w = text_width(font, px, t);
            words.push(Word { text: t.into(), x: x - TEXT_SEL.x, y: y - TEXT_SEL.y, w, h: px * 1.25 });
            x += w + space;
        }
        Line { words }
    };
    let mut v = vec![line(186.0, "Re: Double charge on the March invoice", 30.0, 520.0)];
    v.extend(LINES.iter().enumerate().map(|(i, l)| line(y0 + i as f32 * pitch, l, px, x0)));
    Ok(v)
}

fn settle_ocr(a: &mut App) {
    let t0 = Instant::now();
    while edit_of(a).snip.ocr.is_some() {
        assert!(t0.elapsed() < Duration::from_secs(10), "recognition never finished");
        std::thread::sleep(Duration::from_millis(2));
        a.on_event(Ev::Timer);
    }
}

fn key(a: &mut App, vk: u32, shift: bool) {
    a.on_event(Ev::Key { vk, up: false, repeat: false, mods: Mods { shift, ctrl: false, alt: false } });
}

fn drag(a: &mut App, from: Pt, to: Pt) {
    a.on_event(Ev::Down { x: from.x as i32, y: from.y as i32 });
    a.on_event(Ev::Move { x: ((from.x + to.x) / 2.0) as i32, y: ((from.y + to.y) / 2.0) as i32 });
    a.on_event(Ev::Move { x: to.x as i32, y: to.y as i32 });
    a.on_event(Ev::Up { x: to.x as i32, y: to.y as i32 });
}

fn app(th: Theme, sel: FRect) -> App {
    let mut a = preview_app_with(th, Some(sel), store_shot());
    a.focus_tries = u8::MAX;
    a
}

/// The top-left of word `w` on body line `l` (screen px), plus (dx, dy).
fn word_at(l: usize, w: usize, dx: f32) -> Pt {
    let font = fonts::ui_font();
    let (x0, y0, pitch, px) = BODY;
    let prefix: String = LINES[l].split(' ').take(w).map(|t| format!("{t} ")).collect();
    Pt::new(x0 + text_width(font, px, &prefix) + dx, y0 + l as f32 * pitch + px * 0.6)
}

#[test]
#[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
fn store_screenshots() {
    let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
    for lang in crate::ui::preview::store_langs() {
        let dir = std::path::PathBuf::from(&dir).join(lang);
        std::fs::create_dir_all(&dir).unwrap();
        crate::i18n::with_lang(lang, || store_screenshots_in(&dir));
    }
}

/// The five overlay screenshots, in this thread's language, into `dir`.
fn store_screenshots_in(dir: &std::path::Path) {
    let save = |a: &mut App, name: &str| {
        let img = settled_frame(a);
        assert_eq!((img.width(), img.height()), (W, H));
        std::fs::write(dir.join(name), img.to_png().unwrap()).unwrap();
    };
    let red = C4::rgb(240, 68, 56);
    let tip = |a: &mut App, act: Act| {
        settled_frame(a);
        let e = edit_of(a);
        e.hover = e.toolbar.as_ref().and_then(|t| t.items.iter().position(|i| i.kind == toolbar::Kind::Btn(act)));
        e.hover_at = Instant::now() - Duration::from_secs(2);
    };

    // 1. Dark: annotations over the message (a box, an arrow, numbered
    // tags, a highlight and a callout), Copy's tooltip.
    let sel = FRect { x: 480.0, y: 166.0, w: 1210.0, h: 770.0 };
    let mut a = app(theme::DARK, sel);
    let e = edit_of(&mut a);
    e.tool = Some(Tool::Arrow);
    e.objects.push(Obj::Marker { a: Pt::new(516.0, 393.0), b: Pt::new(860.0, 393.0), color: C4::new(255, 230, 0, 160), width: 30.0 });
    e.objects.push(Obj::Rect { r: FRect { x: 904.0, y: 632.0, w: 76.0, h: 286.0 }, color: red, width: 3.0 });
    e.objects.push(Obj::Arrow { a: Pt::new(1580.0, 560.0), b: Pt::new(1400.0, 700.0), color: red, width: 4.0 });
    e.objects.push(Obj::Counter { c: Pt::new(498.0, 392.0), n: 1, color: red, d: 30.0, tail: None });
    e.objects.push(Obj::Counter { c: Pt::new(904.0, 632.0), n: 2, color: red, d: 30.0, tail: None });
    e.objects.push(Obj::Callout {
        fmt: Default::default(),
        r: FRect { x: 1120.0, y: 450.0, w: 360.0, h: 84.0 },
        tail: Pt::new(990.0, 650.0),
        text: "Saturday's spike: the double charges".into(),
        color: red,
        size: 20.0,
        style: crate::objects::CalloutStyle::Rounded,
        fill: true,
    });
    e.dirty = true;
    tip(&mut a, Act::Copy);
    save(&mut a, "store-overlay-dark.png");

    // 2. Light: an ellipse, a blurred name, a loupe on the chart and a
    // text note; Save's tooltip.
    let sel = FRect { x: 480.0, y: 560.0, w: 1210.0, h: 380.0 };
    let mut a = app(theme::LIGHT, sel);
    let e = edit_of(&mut a);
    e.tool = Some(Tool::Text);
    e.objects.push(Obj::Ellipse { r: FRect { x: 1316.0, y: 698.0, w: 66.0, h: 40.0 }, color: red, width: 3.0 });
    e.objects.push(Obj::Blur { r: FRect { x: 1324.0, y: 750.0, w: 84.0, h: 30.0 }, sigma: 5.0 });
    e.objects.push(Obj::Magnify { src: Pt::new(942.0, 662.0), dst: Pt::new(1575.0, 820.0), radius: 70.0, zoom: 2.0, color: C4::rgb(92, 108, 230), width: 3.0 });
    e.objects.push(Obj::Text { fmt: Default::default(), pos: Pt::new(760.0, 604.0), text: "Refunds peaked on Saturday".into(), color: red, size: 22.0 });
    e.dirty = true;
    tip(&mut a, Act::Save);
    save(&mut a, "store-overlay-light.png");

    // 3. Text mode: the recognized words, two lines selected.
    crate::ocr::FAKE_LINES.with(|f| f.set(Some(store_doc)));
    let mut a = app(theme::DARK, TEXT_SEL);
    key(&mut a, 'O' as u32, false);
    settle_ocr(&mut a);
    assert!(edit_of(&mut a).snip.text.is_some(), "Text mode");
    drag(&mut a, word_at(1, 1, 2.0), word_at(2, 6, 40.0));
    save(&mut a, "store-text-mode.png");

    // 4. Quick Redact (light, Blur style): the e-mail, phone, card, IP
    // and key covered.
    let mut a = app(theme::LIGHT, TEXT_SEL);
    edit_of(&mut a).cfg.redact_style = "blur".into();
    key(&mut a, 'R' as u32, true);
    settle_ocr(&mut a);
    save(&mut a, "store-redacted.png");
    crate::ocr::FAKE_LINES.with(|f| f.set(None));

    // 5. Video mode: the setup panel with the microphone's level and the
    // camera bubble (drawn where its own window would be).
    let vsel = FRect { x: 480.0, y: 166.0, w: 1210.0, h: 770.0 };
    let mut a = app(theme::DARK, vsel);
    let e = edit_of(&mut a);
    e.set_video(true);
    e.rec.opts.mic = true;
    e.rec.opts.camera = true;
    e.rec.meter = Some(rec_ui::meter::MicMeter::open(""));
    e.rec.bars = (5, false);
    let th = theme::DARK;
    let look = e.rec.opts.cam_look();
    let d = crate::cam::side(look.size, 1.0) as i32;
    let at = crate::cam::place(crate::cam::remembered(), (vsel.x as i32, vsel.y as i32, vsel.w as u32, vsel.h as u32), d, 1.0);
    e.rec.grip = Some(FRect { x: at.0 as f32, y: at.1 as f32, w: d as f32, h: d as f32 });
    a.frame();
    let e = edit_of(&mut a);
    let tb = e.toolbar.clone().expect("panel");
    e.hover = tb.items.iter().position(|i| matches!(i.kind, toolbar::Kind::Primary { .. }));
    let mut png = settled_frame(&mut a);
    let t = crate::cam::template(look, 1.0, &th);
    let mut px = Vec::new();
    crate::cam::compose(&t, None, false, &mut px);
    for y in 0..t.w as i32 {
        for x in 0..t.w as i32 {
            let (dx, dy) = (at.0 - t.pad as i32 + x, at.1 - t.pad as i32 + y);
            if dx < 0 || dy < 0 || dx >= W as i32 || dy >= H as i32 {
                continue;
            }
            let s = &px[((y as u32 * t.w + x as u32) * 4) as usize..][..4];
            let al = s[3] as u32;
            let dp = png.get_pixel(dx as u32, dy as u32);
            let ch = |c: u8, b: u8| (c as u32 + b as u32 * (255 - al) / 255).min(255) as u8;
            let i = ((dy as u32 * W + dx as u32) * 4) as usize;
            png.as_raw_mut()[i..i + 4].copy_from_slice(&[ch(s[2], dp[0]), ch(s[1], dp[1]), ch(s[0], dp[2]), 255]);
        }
    }
    std::fs::write(dir.join("store-video-panel.png"), png.to_png().unwrap()).unwrap();
}

/// Quick Redact finds every kind the sample message carries.
#[test]
fn the_store_sample_has_something_to_redact() {
    let lines = store_doc(&PixBuf::from_pixel(1, 1, [0; 4])).unwrap();
    let hits = crate::ocr::redact::find(&lines, &crate::ocr::redact::Kind::ALL);
    let kinds: std::collections::BTreeSet<_> = hits.iter().map(|h| format!("{:?}", h.kind)).collect();
    for k in ["Email", "Phone", "Card", "Ip", "Url"] {
        assert!(kinds.contains(k), "{k} in {kinds:?}");
    }
}
