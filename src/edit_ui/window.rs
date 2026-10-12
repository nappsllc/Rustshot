//! The Edit window's driver: events to the canvas, the list, the zoom
//! control and the questions; the editor's requests carried out (exports
//! on worker threads); frames.

use super::draw::{self, Hit, Lay};
use super::{Btn, Edit, Msg};
use crate::editor::host::Request;
use crate::editor::style::ToastKind;
use crate::keymap::Action;
use crate::objects::{FRect, Pt};
use crate::pixbuf::PixBuf;
use crate::ui::paint::WTok;
use crate::ui::{FocusState, Input, Ui};
use crate::wind::{self, Cursor, Driver, Ev, Hwnd, Mods, key};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// A worker's result: a message, and the uploaded link to keep.
struct Done {
    text: String,
    kind: ToastKind,
    uploaded: Option<String>,
}

pub struct Window {
    hwnd: Hwnd,
    pub e: Edit,
    canvas: PixBuf,
    out: PixBuf,
    pub(super) rx: Receiver<Msg>,
    done: (Sender<Done>, Receiver<Done>),
    /// The pointer (window px), the button held, the press's target.
    mouse: Pt,
    held: bool,
    press: Option<Hit>,
    last_row_click: Option<(usize, Instant)>,
    hover: Option<Hit>,
    /// How long the last frame took to compose (the perf measurements).
    pub last_frame: Duration,
    /// Tests: the scale to draw at (no window to ask).
    pub scale: Option<f32>,
}

impl Window {
    pub fn new(e: Edit, rx: Receiver<Msg>) -> Window {
        Window {
            hwnd: Hwnd::default(),
            e,
            canvas: PixBuf::default(),
            out: PixBuf::default(),
            rx,
            done: std::sync::mpsc::channel(),
            mouse: Pt::new(-1.0, -1.0),
            held: false,
            press: None,
            last_row_click: None,
            hover: None,
            last_frame: Duration::ZERO,
            scale: None,
        }
    }

    fn k(&self) -> f32 {
        if let Some(k) = self.scale {
            return k;
        }
        if cfg!(test) && self.hwnd == Hwnd::default() { 1.0 } else { wind::scale(self.hwnd) }
    }

    /// The frame as composed (tests).
    #[cfg(test)]
    pub fn pixels(&self) -> &PixBuf {
        &self.canvas
    }

    /// The window was closed: what the tools remember is written.
    pub fn finish(&mut self) {
        self.e.canvas.flush(true);
    }

    fn lay(&self) -> Lay {
        let k = self.k();
        let (w, h) = (self.canvas.width() as f32 / k, self.canvas.height() as f32 / k);
        draw::layout(&self.e, w, h, self.e.canvas.bar_width())
    }

    /// Tell the canvas where it is.
    fn place(&mut self) {
        let k = self.k();
        let lay = self.lay();
        let p = |r: FRect| crate::ui::paint::px(r, k);
        let win = FRect { x: 0.0, y: 0.0, w: self.canvas.width() as f32, h: self.canvas.height() as f32 };
        self.e.canvas.set_rects(p(lay.canvas), p(lay.bar), win, k);
    }

    /// Compose the frame.
    pub fn render(&mut self) {
        if self.canvas.width() == 0 {
            return;
        }
        let t0 = Instant::now();
        self.place();
        let k = self.k();
        let lay = self.lay();
        let th = *self.e.canvas.theme();
        let tok = WTok::of(&th);
        let now = Instant::now();
        let (w, _) = self.canvas.dimensions();
        {
            let mut fb = crate::uifb::Fb::new(self.canvas.as_raw_mut(), w as usize);
            let edge = if tok.dark { tok.bd } else { C4_SHADOW };
            let _dp = crate::perf::span(crate::perf::K::Damage);
            self.e.canvas.paint(&mut fb, tok.cv, edge, now);
        }
        let inp = Input::default();
        let mut focus = FocusState::default();
        let mut clip = crate::ui::SystemClipboard;
        let fb = crate::uifb::Fb::new(self.canvas.as_raw_mut(), w as usize);
        let area = FRect { x: 0.0, y: 0.0, w: lay.w, h: lay.h };
        let mut ui = Ui::new(fb, &th, &inp, &mut focus, &mut clip, k, area);
        let _lp = crate::perf::span(crate::perf::K::Label);
        draw::paint_strip(&mut ui, &lay, &tok);
        draw::paint_rail(&mut ui, &self.e, &lay, &tok, self.hover);
        draw::paint_status(&mut ui, &self.e, &lay, &tok);
        draw::paint_zoom(&mut ui, &self.e, &lay, &tok, self.hover);
        drop(_lp);
        self.e.canvas.paint_bar(&mut ui.fb, now);
        draw::paint_toast(&mut ui, &self.e, &lay, now);
        draw::paint_dialog(&mut ui, &self.e, &lay, &tok, self.hover);
        ui.finish();
        self.last_frame = t0.elapsed();
        if crate::perf::on() {
            crate::perf::note(&format!("edit-frame {:.3}", self.last_frame.as_secs_f64() * 1000.0));
        }
    }

    /// The logical-px hit under window point `p` (physical px).
    fn hit_at(&self, p: Pt) -> Option<Hit> {
        let k = self.k();
        let lay = self.lay();
        let th = *self.e.canvas.theme();
        let inp = Input::default();
        let mut focus = FocusState::default();
        let mut clip = crate::ui::SystemClipboard;
        let mut px = [0u8; 4];
        let fb = crate::uifb::Fb::new(&mut px, 1);
        let ui = Ui::new(fb, &th, &inp, &mut focus, &mut clip, k, FRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
        draw::hit(&ui, &self.e, &lay, p.x / k, p.y / k)
    }

    fn dialog_btn(&mut self, b: Btn) {
        let rem = self.e.remembered();
        for d in self.e.flow.press(b, rem) {
            self.e.perform(d);
        }
    }

    /// The editor asked for something.
    fn handle_requests(&mut self) {
        while let Some(r) = self.e.canvas.take_request() {
            match r {
                Request::Save | Request::Close => {
                    for d in self.e.request(r) {
                        self.e.perform(d);
                    }
                }
                Request::SaveAs | Request::SaveDesktop | Request::Copy | Request::Upload | Request::CopyText | Request::Pin => self.export(r),
            }
        }
    }

    /// Save As, Save to Desktop, Copy, Upload, Pin (a worker: dialogs,
    /// encoding and uploads take a while).
    fn export(&mut self, r: Request) {
        use crate::export::Task;
        let img = self.e.canvas.export();
        let objs = self.e.canvas.objects().to_vec();
        if r == Request::Pin {
            #[cfg(not(test))]
            let at = crate::capture::cursor_pos();
            #[cfg(test)]
            let at = (0, 0);
            if !crate::pin::open(std::sync::Arc::new(img), at, *self.e.canvas.theme()) {
                self.e.note("Pin to screen is not available here", ToastKind::Error);
            }
            return;
        }
        let task = match r {
            Request::SaveAs => Task::Save { path: None, ask: true },
            Request::SaveDesktop => Task::SaveDesktop,
            Request::Upload => Task::Upload,
            Request::CopyText => Task::Text,
            _ => Task::Copy,
        };
        if r == Request::Upload {
            self.e.note("Uploading\u{2026}", ToastKind::Info);
        }
        if r == Request::CopyText {
            let tx = self.done.0.clone();
            std::thread::spawn(move || {
                let res = crate::ocr::recognize(&img);
                let note = crate::ocr::finish(&res);
                let _ = tx.send(Done { text: note.text, kind: note.kind, uploaded: None });
            });
            return;
        }
        let (store, id, cfg) = (self.e.store.clone(), self.e.entry.id.clone(), self.e.cfg.clone());
        let saves = matches!(r, Request::SaveAs | Request::SaveDesktop);
        let tx = self.done.0.clone();
        std::thread::spawn(move || {
            let res = crate::export::run_export(&img, (0, 0), std::slice::from_ref(&task), &cfg);
            // A save is an edit of the entry: its objects and the file.
            if saves && let Some(p) = res.done.iter().find_map(|d| match d {
                crate::export::Done::Saved(p) | crate::export::Done::SavedDesktop(p) => Some(p.clone()),
                _ => None,
            }) {
                let bytes = std::fs::metadata(&p).map_or(0, |m| m.len());
                let _ = store.save_edit(&id, &objs, Some((p, bytes)));
                crate::history::changed();
            }
            let done = match res.upload.map(|rx| rx.recv()) {
                Some(Ok(Ok(url))) => {
                    let copied = cfg.copy_url_after_upload && crate::export::copy_text_to_clipboard(&url).is_ok();
                    Done { text: if copied { "Uploaded \u{2014} link copied".into() } else { "Uploaded".into() }, kind: ToastKind::Success, uploaded: Some(url) }
                }
                Some(Ok(Err(e))) => Done { text: format!("Upload failed: {e}"), kind: ToastKind::Error, uploaded: None },
                Some(Err(_)) => Done { text: "Upload failed".into(), kind: ToastKind::Error, uploaded: None },
                None => match crate::notice::compose(&res.done, false) {
                    Some(n) => Done { text: n.text, kind: n.kind, uploaded: None },
                    None => Done { text: String::new(), kind: ToastKind::Info, uploaded: None },
                },
            };
            let _ = tx.send(Done { ..done });
            let _ = saves;
        });
        if saves {
            // The file is written on the worker; the objects are the saved ones.
            self.e.canvas.mark_saved();
        }
    }

    fn settle(&mut self) {
        self.handle_requests();
        if self.e.closing {
            wind::close(self.hwnd);
        }
    }

    /// A key; true when the window (not the editor) took it.
    fn window_key(&mut self, vk: u32, mods: Mods) -> bool {
        if self.e.flow.dialog.is_some() {
            let rem = self.e.remembered();
            let ds = match vk {
                key::ESCAPE => self.e.flow.escape(),
                key::RETURN => self.e.flow.enter(rem),
                _ => return true,
            };
            for d in ds {
                self.e.perform(d);
            }
            return true;
        }
        match self.e.canvas.keymap().resolve_edit_window(vk, mods) {
            Some(Action::ZoomIn) => self.e.canvas.zoom_step(1),
            Some(Action::ZoomOut) => self.e.canvas.zoom_step(-1),
            Some(Action::ZoomFit) => self.e.canvas.zoom_fit(),
            Some(Action::ZoomActual) => self.e.canvas.zoom_actual(),
            _ => {
                // Ctrl+W closes (asking first).
                if vk == 'W' as u32 && mods.ctrl && !mods.alt && !mods.shift {
                    let unsaved = self.e.canvas.unsaved();
                    for d in self.e.flow.close(unsaved) {
                        self.e.perform(d);
                    }
                    return true;
                }
                return false;
            }
        }
        true
    }

    fn click(&mut self, h: Hit, mods: Mods, now: Instant) {
        match h {
            Hit::ZoomOut => self.e.canvas.zoom_step(-1),
            Hit::ZoomIn => self.e.canvas.zoom_step(1),
            Hit::Fit => self.e.canvas.zoom_fit(),
            Hit::Actual => self.e.canvas.zoom_actual(),
            Hit::Collapse => self.e.collapsed = !self.e.collapsed,
            Hit::Row(i) => {
                let double = self.last_row_click.is_some_and(|(j, at)| j == i && now.duration_since(at) < Duration::from_millis(450));
                self.last_row_click = Some((i, now));
                if double && self.e.collapsed {
                    self.e.collapsed = false;
                }
                self.e.canvas.select(i, mods.ctrl);
            }
            Hit::Dialog(b) => self.dialog_btn(b),
        }
    }

    /// One event; true: a frame is due.
    pub fn event(&mut self, ev: Ev) -> bool {
        let now = Instant::now();
        match ev {
            Ev::Resize(w, h) => {
                self.canvas = PixBuf::new(w.max(1), h.max(1));
                self.place();
            }
            Ev::Close => {
                let unsaved = self.e.canvas.unsaved();
                for d in self.e.flow.close(unsaved) {
                    self.e.perform(d);
                }
            }
            Ev::Down { x, y } => {
                let p = Pt::new(x as f32, y as f32);
                self.mouse = p;
                self.held = true;
                let h = self.hit_at(p);
                if self.e.flow.dialog.is_some() {
                    self.press = h;
                    return true;
                }
                if let Some(Hit::Row(i)) = h
                    && !self.e.collapsed
                {
                    self.e.rail_drag = Some(super::RailDrag { obj: i, y0: p.y / self.k(), to: None });
                }
                if h.is_some() {
                    self.press = h;
                } else {
                    self.press = None;
                    self.e.canvas.press(p);
                }
            }
            Ev::Middle { down, x, y } => {
                let p = Pt::new(x as f32, y as f32);
                if down {
                    self.e.canvas.middle_press(p);
                } else {
                    self.e.canvas.release(p);
                }
            }
            Ev::Move { x, y } => {
                let p = Pt::new(x as f32, y as f32);
                self.mouse = p;
                let k = self.k();
                // Reordering in the list.
                if let Some(d) = self.e.rail_drag.as_mut()
                    && self.held
                {
                    let ly = p.y / k;
                    if (ly - d.y0).abs() > 4.0 || d.to.is_some() {
                        let lay = draw::layout(&self.e, self.canvas.width() as f32 / k, self.canvas.height() as f32 / k, self.e.canvas.bar_width());
                        let to = lay.rows.iter().min_by(|a, b| (a.0.y + a.0.h / 2.0 - ly).abs().total_cmp(&(b.0.y + b.0.h / 2.0 - ly).abs())).map(|r| r.1);
                        if let Some(d) = self.e.rail_drag.as_mut() {
                            d.to = to;
                        }
                    }
                    return true;
                }
                let hov = if self.e.flow.dialog.is_some() || !self.held { self.hit_at(p) } else { None };
                let changed = hov != self.hover;
                self.hover = hov;
                let c = if self.e.flow.dialog.is_some() { false } else { self.e.canvas.moved(p, self.held && self.press.is_none()) };
                return c || changed;
            }
            Ev::Up { x, y } => {
                let p = Pt::new(x as f32, y as f32);
                self.held = false;
                if let Some(d) = self.e.rail_drag.take()
                    && let Some(to) = d.to
                {
                    self.e.canvas.reorder(d.obj, to);
                    self.press = None;
                    return true;
                }
                match self.press.take() {
                    Some(h) => {
                        if self.hit_at(p) == Some(h) {
                            self.click(h, Mods::current_or_default(), now);
                        }
                    }
                    None => self.e.canvas.release(p),
                }
            }
            Ev::Wheel { delta, x, y } => {
                let p = Pt::new(x as f32, y as f32);
                if self.e.flow.dialog.is_some() || !self.e.canvas.on_canvas(p) {
                    return false;
                }
                let m = Mods::current_or_default();
                let notches = delta as f32 / 120.0;
                if m.ctrl {
                    self.e.canvas.zoom_by(1.25f32.powf(notches), p);
                } else if m.alt {
                    self.e.canvas.wheel_size(delta);
                } else if m.shift {
                    self.e.canvas.pan_by(notches * 96.0 * self.k(), 0.0);
                } else {
                    self.e.canvas.pan_by(0.0, notches * 96.0 * self.k());
                }
            }
            Ev::Key { vk, up, repeat, mods } => {
                if !up && self.window_key(vk, mods) {
                    self.settle();
                    return true;
                }
                if self.e.flow.dialog.is_none() {
                    self.e.canvas.key(vk, up, repeat, mods);
                }
            }
            Ev::Char(c) => {
                if self.e.flow.dialog.is_none() {
                    self.e.canvas.char_input(c);
                }
            }
            Ev::Focus(_) => {}
            Ev::Timer => return self.tick(),
        }
        self.settle();
        true
    }

    /// Timer: raise requests, workers, tool memory, animations.
    fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(Msg::Raise) = self.rx.try_recv() {
            #[cfg(windows)]
            wind::raise(self.hwnd);
        }
        while let Ok(d) = self.done.1.try_recv() {
            if let Some(u) = d.uploaded {
                let u2 = u.clone();
                let _ = self.e.store.update(&self.e.entry.id, move |m| m.uploaded = Some(u2));
                self.e.entry.meta.uploaded = Some(u);
                crate::history::changed();
            }
            if !d.text.is_empty() {
                self.e.note(d.text, d.kind);
            }
            changed = true;
        }
        self.e.canvas.flush(false);
        let now = Instant::now();
        let toast = self.e.toast.as_ref().is_some_and(|(_, _, at)| at.elapsed() < Duration::from_millis(2500));
        if self.e.toast.is_some() && !toast {
            self.e.toast = None;
            changed = true;
        }
        let anim = self.e.canvas.animating(now) || toast;
        wind::set_fast_timer(self.hwnd, anim);
        changed || anim
    }
}

/// The light board's soft edge under the picture.
const C4_SHADOW: crate::uifb::C4 = crate::uifb::C4 { r: 0, g: 0, b: 0, a: 22 };

impl Driver for Window {
    fn on_create(&mut self, hwnd: Hwnd) {
        self.hwnd = hwnd;
    }

    fn on_event(&mut self, ev: Ev) -> bool {
        let timer = matches!(ev, Ev::Timer);
        let mv = matches!(ev, Ev::Move { .. });
        let due = self.event(ev);
        if due {
            self.render();
            if mv {
                wind::invalidate(self.hwnd);
            }
        }
        if timer { due } else { true }
    }

    fn frame(&mut self) -> Option<&mut PixBuf> {
        if self.canvas.width() == 0 {
            return None;
        }
        if self.out.dimensions() != self.canvas.dimensions() {
            self.out = PixBuf::new(self.canvas.width(), self.canvas.height());
        }
        self.out.as_raw_mut().copy_from_slice(self.canvas.as_raw());
        Some(&mut self.out)
    }

    fn cursor(&self) -> Cursor {
        if self.e.flow.dialog.is_some() {
            return Cursor::Arrow;
        }
        self.e.canvas.cursor(self.mouse)
    }
}
