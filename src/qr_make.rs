//! The Make QR code window (the tray, the launcher): a text field
//! (prefilled from the clipboard's text), the code live below it (the
//! in-repo encoder: byte mode, level M, versions 1–40, the mask of least
//! penalty) and Copy image / Save… / Pin. In the Settings board's compact
//! controls and tokens, dark and light. Not on macOS (no windows off the
//! main thread there).

use crate::objects::FRect;
use crate::pixbuf::PixBuf;
use crate::qr::{self, Ecl, QrCode};
use crate::ui::compact::Tok;
use crate::ui::{TextState, Ui};
use std::sync::Mutex;

/// The window (logical px).
pub const W: u32 = 400;
pub const H: u32 = 460;
const PAD: f32 = 20.0;
const LABEL_H: f32 = 20.0;
const CTL: f32 = 30.0;
/// The preview's box.
const BOX: f32 = 260.0;
/// Pixels per module of what Copy, Save and Pin hand out, and its quiet
/// zone (modules).
pub const EXPORT_SCALE: u32 = 10;
pub const QUIET: u32 = 4;

/// The form.
pub struct Form {
    pub text: TextState,
    /// The code for `text` as last encoded.
    code: Option<(String, Option<QrCode>)>,
    /// What the last button did ("Copied", an error).
    pub status: Option<(String, bool)>,
}

/// What a frame asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Copy,
    Save,
    Pin,
    Close,
}

impl Form {
    pub fn new(text: &str) -> Form {
        // Only a sensible prefill: one paragraph of the clipboard.
        let pre: String = text.trim().chars().take(2000).collect();
        let mut f = Form { text: TextState::new(&pre), code: None, status: None };
        f.text.select_all();
        f
    }

    /// The code for the text (`None`: empty, or too long).
    pub fn code(&mut self) -> Option<&QrCode> {
        let s = self.text.text.clone();
        if self.code.as_ref().is_none_or(|(k, _)| *k != s) {
            let c = if s.is_empty() { None } else { qr::encode_text(&s, Ecl::M) };
            self.code = Some((s, c));
        }
        self.code.as_ref().and_then(|(_, c)| c.as_ref())
    }

    /// The picture Copy, Save and Pin hand out.
    pub fn image(&mut self) -> Option<PixBuf> {
        self.code().map(|c| qr::render(c, EXPORT_SCALE, QUIET))
    }
}

fn phys(ui: &Ui, r: FRect) -> FRect {
    let k = ui.k;
    FRect { x: (r.x * k).round(), y: (r.y * k).round(), w: (r.w * k).round(), h: (r.h * k).round() }
}

/// Paint the window.
pub fn paint(ui: &mut Ui, f: &mut Form) -> Option<Click> {
    let th = *ui.theme;
    let tok = Tok::of(&th);
    ui.text_px = 12.0 * crate::ui::compact::CSS;
    ui.radius = 7.0;
    ui.ctl_h = CTL;
    let (w, h) = (W as f32, H as f32);
    let bg = phys(ui, FRect { x: 0.0, y: 0.0, w, h });
    ui.fb.fill_rect(bg.x as i32, bg.y as i32, bg.w as i32 + 1, bg.h as i32 + 1, tok.win);
    let inner = w - 2.0 * PAD;
    let mut click = None;
    // The text.
    ui.text_at(12.0, tr!("qr.text_label"), ui.px(PAD + 1.0), ui.px(16.0 + LABEL_H / 2.0), tok.tx2, false);
    let field = FRect { x: PAD, y: 16.0 + LABEL_H, w: inner, h: CTL };
    if ui.area(field, |ui| ui.text_field("qr.text", &mut f.text)) {
        f.status = None;
    }
    // The code, in a white box (it reads on any theme).
    let by = field.y1() + 14.0;
    let b = FRect { x: (w - BOX) / 2.0, y: by, w: BOX, h: BOX };
    let pb = phys(ui, b);
    let ok = f.code().is_some();
    let rad = 10.0 * ui.k;
    let fill = if ok { crate::uifb::C4::rgb(255, 255, 255) } else { tok.sf2 };
    ui.fb.fill_rounded(pb.x as i32, pb.y as i32, pb.w as i32, pb.h as i32, rad, fill);
    let (bytes, empty) = (f.text.text.len(), f.text.text.is_empty());
    let info = match f.code() {
        Some(code) => {
            let n = code.size() as u32 + 2 * QUIET;
            let s = ((pb.w as u32).saturating_sub(8) / n).max(1);
            let side = n * s;
            let (x0, y0) = (pb.x as i32 + (pb.w as i32 - side as i32) / 2, pb.y as i32 + (pb.h as i32 - side as i32) / 2);
            for y in 0..code.size() {
                for x in 0..code.size() {
                    if code.get(x, y) {
                        let (px, py) = (x0 + ((x as u32 + QUIET) * s) as i32, y0 + ((y as u32 + QUIET) * s) as i32);
                        ui.fb.fill_rect(px, py, s as i32, s as i32, crate::uifb::C4::rgb(0, 0, 0));
                    }
                }
            }
            (tr!("qr.info", v = code.version, s = code.size(), b = bytes), false)
        }
        None if empty => (tr!("qr.empty").to_string(), false),
        None => (tr!("qr.too_long").to_string(), true),
    };
    if !ok {
        let tw = ui.text_w(12.0, &info.0);
        let c = if info.1 { tok.er } else { tok.tx2 };
        ui.text_at(12.0, &info.0, pb.x + (pb.w - ui.px(tw)) / 2.0, pb.y + pb.h / 2.0, c, false);
    }
    // The line under the box: the code's size, or what a button did.
    let line = match &f.status {
        Some((s, err)) => (s.clone(), *err),
        None if ok => info,
        None => (String::new(), false),
    };
    if !line.0.is_empty() {
        let s = ui.fit_text(11.5, &line.0, inner);
        let tw = ui.text_w(11.5, &s);
        ui.text_at(11.5, &s, ui.px((w - tw) / 2.0), ui.px(b.y1() + 16.0), if line.1 { tok.er } else { tok.tx3 }, false);
    }
    // The buttons.
    let sep = phys(ui, FRect { x: 0.0, y: h - 60.0, w, h: 1.0 });
    ui.fb.fill_rect(sep.x as i32, sep.y as i32, sep.w as i32, ui.k.round().max(1.0) as i32, tok.sep);
    let bar = FRect { x: PAD, y: h - 60.0 + 16.0, w: inner, h: 28.0 };
    // A long language: the icons go first, then Close (Esc and the title
    // bar close too).
    let labels = [tr!("qr.copy_image"), tr!("qr.save"), tr!("qr.pin")];
    let gap = crate::ui::layout::GAP;
    let need = |icons: bool| labels.iter().map(|l| ui.cbutton_width(l, icons) + gap).sum::<f32>();
    let close_w = ui.cbutton_width(tr!("qr.close"), false) + 2.0 * gap;
    let icons = need(true) + close_w <= inner;
    let close = need(icons) + close_w <= inner;
    let icon = |i: &'static str| icons.then_some(i);
    ui.area(bar, |ui| {
        ui.ctl_h = 28.0;
        ui.row_rtl(|ui| {
            let on = ui.enabled;
            ui.enabled = on && ok;
            if ui.cbutton("qr.copy", labels[0], icon("copy"), true, &tok) {
                click = Some(Click::Copy);
            }
            if ui.cbutton("qr.save", labels[1], icon("save"), false, &tok) {
                click = Some(Click::Save);
            }
            if ui.cbutton("qr.pin", labels[2], icon("pin"), false, &tok) {
                click = Some(Click::Pin);
            }
            ui.enabled = on;
        });
    });
    if !close {
        return click;
    }
    ui.area(FRect { x: PAD, y: bar.y, w: close_w, h: 28.0 }, |ui| {
        ui.ctl_h = 28.0;
        ui.row(|ui| {
            if ui.cbutton("qr.close", tr!("qr.close"), None, false, &tok) {
                click = Some(Click::Close);
            }
        });
    });
    click
}

#[cfg(not(target_os = "macos"))]
mod window {
    use super::*;
    use crate::theme::Theme;
    use crate::ui::{FocusState, Input};
    use crate::wind::{self, Cursor, Driver, Ev, Hwnd, WindowSpec};

    pub(super) struct Window {
        hwnd: Hwnd,
        f: Form,
        th: Theme,
        input: Input,
        focus: FocusState,
        canvas: PixBuf,
        out: PixBuf,
        rx: crate::chan::Receiver<()>,
        closing: bool,
        sized_for: f32,
    }

    impl Window {
        fn render(&mut self) {
            for _ in 0..4 {
                if self.closing || self.canvas.width() == 0 {
                    return;
                }
                let k = wind::follow_scale(self.hwnd, &mut self.sized_for, W, H);
                let mut clip = crate::ui::SystemClipboard;
                let (cw, ch) = self.canvas.dimensions();
                let area = FRect { x: 0.0, y: 0.0, w: cw as f32 / k, h: ch as f32 / k };
                let fb = crate::uifb::Fb::new(self.canvas.as_raw_mut(), cw as usize);
                let mut ui = Ui::new(fb, &self.th, &self.input, &mut self.focus, &mut clip, k, area);
                let click = paint(&mut ui, &mut self.f);
                let out = ui.finish();
                self.input.end_frame();
                let click = click.or(if out.escape { Some(Click::Close) } else if out.enter { Some(Click::Copy) } else { None });
                match click {
                    Some(c) => {
                        self.click(c);
                        continue;
                    }
                    None if out.redraw => continue,
                    None => return,
                }
            }
        }

        fn click(&mut self, c: Click) {
            if c == Click::Close {
                self.closing = true;
                return;
            }
            let Some(img) = self.f.image() else { return };
            self.f.status = Some(match c {
                Click::Copy => match crate::export::copy_to_clipboard(&img) {
                    Ok(()) => (tr!("qr.copied").to_string(), false),
                    Err(e) => (format!("Copy failed: {e:#}"), true),
                },
                Click::Save => {
                    let cfg = crate::config::load();
                    let res = crate::export::run_export(&img, (0, 0), &[crate::export::Task::Save { path: None, ask: true }], &cfg);
                    match crate::notice::compose(&res.done, false) {
                        Some(n) => (n.text, n.kind == crate::editor::style::ToastKind::Error),
                        None => (String::new(), false),
                    }
                }
                Click::Pin => {
                    let at = crate::capture::cursor_pos();
                    if crate::pin::open(std::sync::Arc::new(img), at, self.th) {
                        (String::new(), false)
                    } else {
                        ("Pin to screen is not available here".to_string(), true)
                    }
                }
                Click::Close => unreachable!(),
            })
            .filter(|s| !s.0.is_empty());
        }

        pub(super) fn new(f: Form, th: Theme, rx: crate::chan::Receiver<()>) -> Window {
            Window { hwnd: Hwnd::default(), f, th, input: Input::default(), focus: FocusState::default(), canvas: PixBuf::default(), out: PixBuf::default(), rx, closing: false, sized_for: 0.0 }
        }
    }

    impl Driver for Window {
        fn on_create(&mut self, hwnd: Hwnd) {
            self.hwnd = hwnd;
            self.focus.focus("qr.text");
        }

        fn on_event(&mut self, ev: Ev) -> bool {
            match ev {
                Ev::Resize(w, h) => {
                    self.canvas = PixBuf::new(w.max(1), h.max(1));
                    self.render();
                }
                Ev::Close => self.closing = true,
                Ev::Timer => {
                    if self.rx.try_iter().count() > 0 {
                        #[cfg(windows)]
                        wind::raise(self.hwnd);
                    }
                    wind::follow_scale(self.hwnd, &mut self.sized_for, W, H);
                    if self.closing {
                        wind::close(self.hwnd);
                    }
                    return false;
                }
                _ => {
                    self.input.feed(&ev);
                    self.render();
                }
            }
            if self.closing {
                wind::close(self.hwnd);
            }
            !self.closing
        }

        fn frame(&mut self) -> Option<&mut PixBuf> {
            if self.canvas.width() == 0 || self.closing {
                return None;
            }
            if self.out.dimensions() != self.canvas.dimensions() {
                self.out = PixBuf::new(self.canvas.width(), self.canvas.height());
            }
            self.out.as_raw_mut().copy_from_slice(self.canvas.as_raw());
            Some(&mut self.out)
        }

        fn cursor(&self) -> Cursor {
            Cursor::Arrow
        }

        fn frame_colors(&self) -> Option<wind::FrameColors> {
            Some(crate::ui::paint::WTok::of(&self.th).frame())
        }
    }

    pub(super) fn run(rx: crate::chan::Receiver<()>) {
        let cfg = crate::config::load();
        let th = crate::theme::resolve(&cfg);
        let pre = crate::ui::Clipboard::get(&mut crate::ui::SystemClipboard).unwrap_or_default();
        let mut w = Window::new(Form::new(&pre), th, rx);
        let spec = WindowSpec { title: tr!("qr.make_title").into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::MinClose };
        if let Err(e) = wind::run_window(spec, &mut w) {
            eprintln!("rustshot: QR code window: {e:#}");
        }
        super::release();
    }
}

/// The open window's "come to the front".
static OPEN: Mutex<Option<crate::chan::Sender<()>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the window is open in this process (an on-demand run waits
/// for it before it quits).
pub fn is_open() -> bool {
    lock(&OPEN).is_some()
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn release() {
    *lock(&OPEN) = None;
}

/// Open the window (or raise the open one), on its own thread.
pub fn show() {
    #[cfg(target_os = "macos")]
    crate::tray::notify(tr!("qr.unavailable_mac"));
    #[cfg(not(target_os = "macos"))]
    {
        let mut open = lock(&OPEN);
        if let Some(tx) = open.as_ref()
            && tx.send(()).is_ok()
        {
            return;
        }
        let (tx, rx) = crate::chan::channel();
        *open = Some(tx);
        drop(open);
        let spawned = std::thread::Builder::new().name("rustshot-qr".into()).spawn(move || window::run(rx));
        if spawned.is_err() {
            release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{DARK, LIGHT};
    use crate::ui::{FocusState, Input, preview};
    use crate::wind::Ev;

    fn render(f: &mut Form, th: &crate::theme::Theme, k: f32, input: &Input, focus: &mut FocusState) -> (PixBuf, Option<Click>) {
        preview::render(W, H, k, th, &Input::default(), focus, |ui| {
            paint(ui, f);
        });
        let mut click = None;
        let img = preview::render(W, H, k, th, input, focus, |ui| click = paint(ui, f));
        (img, click)
    }

    fn click(f: &mut Form, focus: &mut FocusState, id: &str) -> Option<Click> {
        render(f, &DARK, 1.0, &Input::default(), focus);
        let r = focus.rect_of(id).unwrap_or_else(|| panic!("{id} drawn"));
        let (x, y) = ((r.x + r.w / 2.0) as i32, (r.y + r.h / 2.0) as i32);
        let mut input = Input::default();
        for e in [Ev::Move { x, y }, Ev::Down { x, y }, Ev::Up { x, y }] {
            input.feed(&e);
        }
        render(f, &DARK, 1.0, &input, focus).1
    }

    #[test]
    fn the_code_follows_the_text_and_reads_back() {
        let mut f = Form::new("  https://example.com/ \n");
        assert_eq!(f.text.text, "https://example.com/", "trimmed");
        let img = f.image().unwrap();
        assert_eq!(qr::read(&img), vec!["https://example.com/".to_string()]);
        f.text.set("");
        assert!(f.code().is_none() && f.image().is_none());
        f.text.set(&"x".repeat(2400));
        assert!(f.code().is_none(), "too long");
        f.text.set("Grüße");
        assert_eq!(qr::read(&f.image().unwrap()), vec!["Grüße".to_string()]);
    }

    #[test]
    fn the_buttons_answer_and_need_a_code() {
        let mut f = Form::new("hello");
        let mut focus = FocusState::default();
        assert_eq!(click(&mut f, &mut focus, "qr.copy"), Some(Click::Copy));
        assert_eq!(click(&mut f, &mut focus, "qr.save"), Some(Click::Save));
        assert_eq!(click(&mut f, &mut focus, "qr.pin"), Some(Click::Pin));
        assert_eq!(click(&mut f, &mut focus, "qr.close"), Some(Click::Close));
        f.text.set("");
        assert_eq!(click(&mut f, &mut focus, "qr.copy"), None, "disabled without a code");
    }

    #[test]
    fn previews() {
        for (name, th) in [("dark", DARK), ("light", LIGHT)] {
            for k in [1.0f32, 2.0] {
                for (what, text) in [("link", "https://github.com/rustshot/rustshot"), ("empty", ""), ("long", &*"y".repeat(2500))] {
                    let mut f = Form::new(text);
                    let mut focus = FocusState::default();
                    let (img, _) = render(&mut f, &th, k, &Input::default(), &mut focus);
                    assert_eq!(img.dimensions(), ((W as f32 * k) as u32, (H as f32 * k) as u32));
                    for id in ["qr.text", "qr.copy", "qr.save", "qr.pin", "qr.close"] {
                        let r = focus.rect_of(id).unwrap_or_else(|| panic!("{id}"));
                        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x1() <= W as f32 * k + 0.5 && r.y1() <= H as f32 * k + 0.5, "{id} {r:?} inside");
                    }

                    if what == "link" {
                        // What shows on screen reads back.
                        assert_eq!(qr::read(&img), vec!["https://github.com/rustshot/rustshot".to_string()], "{name} {k}");
                    }
                    preview::save(&format!("qr_make_{what}_{name}_{k}x.png"), &img);
                }
            }
        }
    }

    /// The window fits in English and the long pseudo-languages, at 1x and
    /// 1.5x, in each state (previews per language).
    #[test]
    fn fits_in_long_languages() {
        for lang in preview::LANGS {
            crate::i18n::with_lang(lang, || {
                for k in [1.0f32, 1.5] {
                    for (what, text) in [("link", "https://github.com/rustshot/rustshot"), ("empty", ""), ("long", &*"y".repeat(2500))] {
                        let mut f = Form::new(text);
                        let mut focus = FocusState::default();
                        let (img, _) = render(&mut f, &DARK, k, &Input::default(), &mut focus);
                        preview::assert_fits(&focus, W, H, k, &format!("qr window {what} {lang} {k}x"));
                        if k == 1.0 {
                            preview::save(&format!("qr_make_{what}_{lang}.png"), &img);
                        }
                    }
                }
            });
        }
    }
}