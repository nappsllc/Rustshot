//! The launcher window: three labelled segmented choices (Capture,
//! Delay with its 0–60 s stepper, After capture) and Cancel / Capture,
//! in the Settings board's compact controls and tokens (dark and light).

use super::{After, Plan, Source, DELAYS, H, MAX_DELAY, W};
use crate::objects::FRect;
use crate::pixbuf::PixBuf;
use crate::theme::Theme;
use crate::ui::compact::Tok;
use crate::ui::{FocusState, Input, Ui};
use crate::wind::{self, Cursor, Driver, Ev, Hwnd, WindowSpec};
use crate::chan::Receiver;

/// The board's paddings (logical px).
pub const PAD: f32 = 20.0;
const LABEL_H: f32 = 20.0;
const CTL: f32 = 30.0;
const GROUP_GAP: f32 = 12.0;
const STEP_W: f32 = 28.0;
const VALUE_W: f32 = 40.0;

/// The window's form.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub plan: Plan,
}

/// What a frame asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Capture,
    Cancel,
}

impl Form {
    pub fn new(plan: Plan) -> Form {
        Form { plan }
    }

    /// The delay segment shown chosen (past the end: a custom delay).
    pub fn delay_sel(&self) -> usize {
        DELAYS.iter().position(|d| *d == self.plan.delay).unwrap_or(DELAYS.len())
    }

    /// The stepper: one second down / up, within 0–60.
    pub fn step(&mut self, up: bool) {
        let d = self.plan.delay;
        self.plan.delay = if up { (d + 1).min(MAX_DELAY) } else { d.saturating_sub(1) };
    }
}

/// The labels of the delay choices.
const DELAY_LABELS: [crate::i18n::Key; 4] = [key!("launcher.s0"), key!("launcher.s3"), key!("launcher.s5"), key!("launcher.s10")];

fn phys(ui: &Ui, r: FRect) -> FRect {
    let k = ui.k;
    FRect { x: (r.x * k).round(), y: (r.y * k).round(), w: (r.w * k).round(), h: (r.h * k).round() }
}

/// Paint the form (and its controls) over the whole window.
pub fn paint(ui: &mut Ui, f: &mut Form) -> Option<Click> {
    let th = *ui.theme;
    let tok = Tok::of(&th);
    ui.text_px = 12.0 * crate::ui::compact::CSS;
    ui.radius = 7.0;
    ui.ctl_h = CTL;
    let (w, h) = (W as f32, H as f32);
    let bg = phys(ui, FRect { x: 0.0, y: 0.0, w, h });
    ui.fb.fill_rect(bg.x as i32, bg.y as i32, bg.w as i32 + 1, bg.h as i32 + 1, tok.win);
    let mut click = None;
    let inner = w - 2.0 * PAD;
    let mut y = 16.0;
    let group = |ui: &mut Ui, label: &str, y: &mut f32, draw: &mut dyn FnMut(&mut Ui, FRect)| {
        ui.text_at(12.0, label, ui.px(PAD + 1.0), ui.px(*y + LABEL_H / 2.0), tok.tx2, false);
        let r = FRect { x: PAD, y: *y + LABEL_H, w: inner, h: CTL };
        draw(ui, r);
        *y += LABEL_H + CTL + GROUP_GAP;
    };
    // What to capture.
    let mut src = Source::ALL.iter().position(|s| *s == f.plan.source).unwrap_or(0);
    group(ui, tr!("launcher.capture"), &mut y, &mut |ui, r| {
        let labels: Vec<&str> = Source::ALL.iter().map(|s| s.label()).collect();
        ui.area(r, |ui| ui.seg("launcher.source", &labels, &mut src, &tok));
    });
    f.plan.source = Source::ALL[src.min(Source::ALL.len() - 1)];
    // After how long: the choices, and the stepper for any 0–60 s.
    let mut dsel = f.delay_sel();
    let mut stepped = None;
    group(ui, tr!("launcher.delay"), &mut y, &mut |ui, r| {
        ui.area(r, |ui| {
            ui.row(|ui| {
                if ui.seg("launcher.delay", &DELAY_LABELS.map(|k| k.get()), &mut dsel, &tok) {
                    stepped = Some(None);
                }
                let sx = r.x1() - (2.0 * STEP_W + VALUE_W);
                let minus = FRect { x: sx, y: r.y, w: STEP_W, h: CTL };
                let plus = FRect { x: sx + STEP_W + VALUE_W, y: r.y, w: STEP_W, h: CTL };
                for (id, r, up, icon) in [("launcher.less", minus, false, "minus"), ("launcher.more", plus, true, "plus")] {
                    ui.place(r);
                    if ui.cbutton_ex(id, "", None, false, 13.0, &tok) {
                        stepped = Some(Some(up));
                    }
                    let (s, k) = (ui.px(14.0), if ui.enabled { 1.0 } else { 0.4 });
                    let (cx, cy) = (ui.px(r.x + r.w / 2.0), ui.px(r.y + r.h / 2.0));
                    crate::icon_path::draw_icon(&mut ui.fb, icon, cx - s / 2.0, cy - s / 2.0, s, tok.tx.fade(k));
                }
            });
        });
        let v = tr!("launcher.seconds", n = f.plan.delay);
        let vx = r.x1() - STEP_W - VALUE_W / 2.0 - ui.text_w(12.5, &v) / 2.0;
        ui.text_at(12.5, &v, ui.px(vx), ui.px(r.y + CTL / 2.0), tok.tx, false);
    });
    match stepped {
        Some(None) => f.plan.delay = DELAYS[dsel.min(DELAYS.len() - 1)],
        Some(Some(up)) => f.step(up),
        None => {}
    }
    // What follows.
    let mut after = After::ALL.iter().position(|a| *a == f.plan.after).unwrap_or(0);
    group(ui, tr!("settings.capture.after_capture"), &mut y, &mut |ui, r| {
        let labels: Vec<&str> = After::ALL.iter().map(|a| a.label()).collect();
        ui.area(r, |ui| ui.seg("launcher.after", &labels, &mut after, &tok));
    });
    f.plan.after = After::ALL[after.min(After::ALL.len() - 1)];
    // The buttons, bottom-right.
    let line = phys(ui, FRect { x: 0.0, y: h - 60.0, w, h: 1.0 });
    ui.fb.fill_rect(line.x as i32, line.y as i32, line.w as i32, ui.k.round().max(1.0) as i32, tok.sep);
    let bar = FRect { x: PAD, y: h - 60.0 + 16.0, w: inner, h: 28.0 };
    ui.area(bar, |ui| {
        ui.ctl_h = 28.0;
        ui.row_rtl(|ui| {
            if ui.cbutton("launcher.capture", tr!("launcher.capture"), Some("region"), true, &tok) {
                click = Some(Click::Capture);
            }
            if ui.cbutton("launcher.cancel", tr!("common.cancel"), None, false, &tok) {
                click = Some(Click::Cancel);
            }
        });
    });
    click
}

/// One frame into `canvas`; the click (Esc: Cancel, Enter: Capture) and
/// whether the kit wants another frame.
pub fn frame(canvas: &mut PixBuf, th: &Theme, input: &Input, focus: &mut FocusState, f: &mut Form, k: f32, clip: &mut dyn crate::ui::Clipboard) -> (Option<Click>, bool) {
    let (w, h) = canvas.dimensions();
    let area = FRect { x: 0.0, y: 0.0, w: w as f32 / k, h: h as f32 / k };
    let fb = crate::uifb::Fb::new(canvas.as_raw_mut(), w as usize);
    let mut ui = Ui::new(fb, th, input, focus, clip, k, area);
    let click = paint(&mut ui, f);
    let out = ui.finish();
    let click = click.or(if out.escape {
        Some(Click::Cancel)
    } else if out.enter {
        Some(Click::Capture)
    } else {
        None
    });
    (click, out.redraw)
}

/// The window driver.
pub(super) struct Window {
    hwnd: Hwnd,
    f: Form,
    th: Theme,
    input: Input,
    focus: FocusState,
    canvas: PixBuf,
    out: PixBuf,
    rx: Receiver<()>,
    closing: bool,
    /// Capture was clicked: launched after the window is destroyed.
    chosen: Option<Plan>,
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
            let (click, redraw) = frame(&mut self.canvas, &self.th, &self.input, &mut self.focus, &mut self.f, k, &mut clip);
            self.input.end_frame();
            match click {
                Some(c) => self.click(c),
                None if redraw => continue,
                None => {}
            }
            return;
        }
    }

    /// Capture or Cancel: the window closes; Capture's plan is launched
    /// only once it is gone ([`super::after_close`]).
    pub(super) fn click(&mut self, c: Click) {
        self.closing = true;
        if c == Click::Capture {
            self.chosen = Some(self.f.plan);
        }
    }

    fn settle(&mut self) {
        if self.closing {
            wind::close(self.hwnd);
        }
    }

    #[cfg(test)]
    pub(super) fn for_tests(plan: Plan) -> Window {
        let (_tx, rx) = crate::chan::channel();
        Window::new(Form::new(plan), crate::theme::DARK, rx)
    }

    #[cfg(test)]
    pub(super) fn chosen(&self) -> Option<Plan> {
        self.chosen
    }

    fn new(f: Form, th: Theme, rx: Receiver<()>) -> Window {
        Window {
            hwnd: Hwnd::default(),
            f,
            th,
            input: Input::default(),
            focus: FocusState::default(),
            canvas: PixBuf::default(),
            out: PixBuf::default(),
            rx,
            closing: false,
            chosen: None,
            sized_for: 0.0,
        }
    }
}

impl Driver for Window {
    fn title(&self) -> Option<String> {
        Some(tr!("launcher.window_title").into())
    }

    fn on_create(&mut self, hwnd: Hwnd) {
        self.hwnd = hwnd;
    }

    fn on_event(&mut self, ev: Ev) -> bool {
        // Right to left: the pointer mirrored, as the frame is.
        let ev = crate::ui::rtl_ev(ev, self.canvas.width());
        match ev {
            Ev::Resize(w, h) => {
                self.canvas = PixBuf::new(w.max(1), h.max(1));
                self.render();
            }
            Ev::Close => self.closing = true,
            Ev::Timer => {
                let raise = self.rx.try_iter().count() > 0;
                if raise {
                    #[cfg(windows)]
                    wind::raise(self.hwnd);
                }
                wind::follow_scale(self.hwnd, &mut self.sized_for, W, H);
                self.settle();
                return false;
            }
            Ev::Move { .. } => {
                self.input.feed(&ev);
                self.render();
                if !self.closing {
                    wind::invalidate(self.hwnd);
                }
            }
            _ => {
                self.input.feed(&ev);
                self.render();
            }
        }
        self.settle();
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

/// Run the window on this thread until it closes.
pub fn run(rx: Receiver<()>) {
    let cfg = crate::config::load();
    let th = crate::theme::resolve(&cfg);
    let mut w = Window::new(Form::new(Plan::from_config(&cfg)), th, rx);
    let spec = WindowSpec { title: tr!("launcher.window_title").into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::MinClose };
    if let Err(e) = wind::run_window(spec, &mut w) {
        eprintln!("rustshot: launcher window: {e:#}");
    }
    super::release();
    // Destroyed: give it time to leave the screen, then capture.
    super::after_close(w.chosen, || std::thread::sleep(super::SETTLE));
}
