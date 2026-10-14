//! The trim window: a fixed-size `run_window` on its own thread, its
//! pictures from a decoding worker (thumbnails first, then whatever the
//! playhead asks for; a newer request replaces an older one).

use super::{apply, layout, near_handle, text, time_at, x_of, Choice, Grab, Lay, Outcome, Trim, H, THUMBS, W};
use crate::chan::{self, Receiver, Sender, TryRecvError};
use crate::objects::FRect;
use crate::pixbuf::PixBuf;
use crate::theme::Theme;
use crate::ui::compact::Tok;
use crate::ui::{FocusState, Input, KeyIn, Ui};
use crate::uifb::{Fb, C4};
use crate::wind::{self, Cursor, Driver, Ev, Hwnd, WindowSpec};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// What the worker sends.
enum Pic {
    /// The decoder opened: the recording's length.
    Ready(Duration),
    Thumb(usize, PixBuf),
    Frame(Duration, PixBuf),
    /// No decoder: no pictures.
    None,
}

/// The picture the window wants next (only the latest counts).
#[derive(Default)]
struct Want {
    frame: Option<(Duration, (u32, u32))>,
    thumbs: Option<(u32, u32)>,
    quit: bool,
}

type Shared = Arc<(Mutex<Want>, Condvar)>;

fn worker(path: PathBuf, sh: Shared, tx: Sender<Pic>) {
    #[cfg(windows)]
    crate::rec::win::com_init();
    let mut dec = match crate::rec::cut::decode::open(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rustshot: trim preview: {e:#}");
            let _ = tx.send(Pic::None);
            return;
        }
    };
    let len = dec.length();
    let _ = tx.send(Pic::Ready(len));
    let mut thumbs_done = 0usize;
    loop {
        let (frame, thumbs) = {
            let (m, cv) = &*sh;
            let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
            while !g.quit && g.frame.is_none() && (g.thumbs.is_none() || thumbs_done >= THUMBS) {
                g = cv.wait(g).unwrap_or_else(|e| e.into_inner());
            }
            if g.quit {
                return;
            }
            (g.frame.take(), g.thumbs)
        };
        // The playhead first; a thumbnail at a time in between.
        if let Some((t, max)) = frame {
            if let Some(p) = dec.frame_at(t, max) {
                let _ = tx.send(Pic::Frame(t, p));
            }
            continue;
        }
        if let Some(max) = thumbs
            && thumbs_done < THUMBS
        {
            let t = Duration::from_secs_f64(len.as_secs_f64() * (thumbs_done as f64 + 0.5) / THUMBS as f64);
            if let Some(p) = dec.frame_at(t, max) {
                let _ = tx.send(Pic::Thumb(thumbs_done, crop_to(p, max)));
            }
            thumbs_done += 1;
        }
    }
}

/// `p` cut to fill `size` (centred).
fn crop_to(p: PixBuf, size: (u32, u32)) -> PixBuf {
    let (w, h) = p.dimensions();
    let k = (size.0 as f32 / w as f32).max(size.1 as f32 / h as f32);
    let scaled = if (k - 1.0).abs() > 0.01 {
        crate::scale::fit(&p, ((w as f32 * k).ceil() as u32).max(1), ((h as f32 * k).ceil() as u32).max(1))
    } else {
        p
    };
    let (sw, sh) = scaled.dimensions();
    let (cw, ch) = (size.0.min(sw), size.1.min(sh));
    scaled.crop((sw - cw) / 2, (sh - ch) / 2, cw, ch)
}

/// Copy `p` (RGBA) to (x, y) physical px of `fb`, clipped.
fn blit(fb: &mut Fb, p: &PixBuf, x: i32, y: i32, alpha: f32) {
    let (bx0, by0, bx1, by1) = fb.bounds();
    let (w, h) = p.dimensions();
    let k = alpha.clamp(0.0, 1.0);
    for py in 0..h as i32 {
        let ty = y + py;
        if ty < by0 || ty >= by1 {
            continue;
        }
        for px in 0..w as i32 {
            let tx = x + px;
            if tx < bx0 || tx >= bx1 {
                continue;
            }
            let i = (py as usize * w as usize + px as usize) * 4;
            let q = &p.as_raw()[i..i + 4];
            fb.blend_px(tx, ty, C4::new(q[0], q[1], q[2], (q[3] as f32 * k) as u8));
        }
    }
}

fn phys(ui: &Ui, r: FRect) -> FRect {
    let k = ui.k;
    FRect { x: (r.x * k).round(), y: (r.y * k).round(), w: (r.w * k).round(), h: (r.h * k).round() }
}

/// The window's pictures.
#[derive(Default)]
pub(super) struct Pics {
    /// Known once the decoder opened (`Some(false)`: no decoder).
    pub ready: Option<bool>,
    pub thumbs: Vec<Option<PixBuf>>,
    pub frame: Option<(Duration, PixBuf)>,
}

/// Paint one frame; the choice when a button closes the window.
pub(super) fn paint(ui: &mut Ui, t: &mut Trim, pics: &Pics, now: Instant) -> Option<Choice> {
    let th = *ui.theme;
    let tok = Tok::of(&th);
    ui.text_px = 12.0 * crate::ui::compact::CSS;
    ui.radius = 7.0;
    ui.ctl_h = 32.0;
    let l = layout();
    let bg = phys(ui, FRect { x: 0.0, y: 0.0, w: W as f32, h: H as f32 });
    ui.fb.fill_rect(bg.x as i32, bg.y as i32, bg.w as i32 + 1, bg.h as i32 + 1, tok.win);
    // The preview.
    let pr = phys(ui, l.preview);
    let well = if th.dark { C4::rgb(14, 15, 18) } else { C4::rgb(226, 228, 233) };
    ui.fb.fill_rounded(pr.x as i32, pr.y as i32, pr.w as i32, pr.h as i32, ui.px(8.0), well);
    match (&pics.frame, pics.ready) {
        (Some((_, p)), _) => {
            let (w, h) = p.dimensions();
            let x = pr.x as i32 + (pr.w as i32 - w as i32) / 2;
            let y = pr.y as i32 + (pr.h as i32 - h as i32) / 2;
            blit(&mut ui.fb, p, x, y, 1.0);
        }
        (None, r) => {
            let s = if r == Some(false) { text::NO_PREVIEW.get() } else { text::LOADING.get() };
            let w = ui.px(ui.text_w(12.0, s));
            ui.text_at(12.0, s, pr.x + (pr.w - w) / 2.0, pr.y + pr.h / 2.0, tok.tx2, false);
        }
    }
    // Play / Pause, the times.
    let mut toggled = false;
    ui.area(l.play, |ui| {
        ui.place(l.play);
        if ui.cbutton_ex("trim.play", "", None, false, 13.0, &tok) {
            toggled = true;
        }
    });
    // Its glyph (Play / Pause), as the launcher's steppers draw theirs.
    let icon = if t.playing() { "pause" } else { "play" };
    let (s, cx, cy) = (ui.px(16.0), ui.px(l.play.x + l.play.w / 2.0), ui.px(l.play.y + l.play.h / 2.0));
    crate::icon_path::draw_icon(&mut ui.fb, icon, cx - s / 2.0, cy - s / 2.0, s, tok.tx);
    if toggled {
        t.toggle_play(now);
    }
    let cy = ui.px(l.play.y + l.play.h / 2.0);
    let times = format!("{} / {}", super::fmt_time(t.head), super::fmt_time(t.len));
    ui.text_at(12.5, &times, ui.px(l.play.x1() + 12.0), cy, tok.tx, false);
    let sel = tr!("trim.selected", time = super::fmt_time(t.selected()));
    let sw = ui.text_w(12.5, &sel);
    ui.text_at(12.5, &sel, ui.px(l.strip.x1() - sw), cy, tok.tx2, false);
    // The timeline.
    strip(ui, &l, t, pics, &tok);
    let hint_y = ui.px(l.strip.y1() + 10.0);
    ui.text_at(11.0, text::HINT.get(), ui.px(l.strip.x), hint_y, tok.tx3, false);
    // The buttons.
    let line = phys(ui, FRect { x: 0.0, y: l.footer, w: W as f32, h: 1.0 });
    ui.fb.fill_rect(line.x as i32, line.y as i32, line.w as i32, ui.k.round().max(1.0) as i32, tok.sep);
    let bar = FRect { x: super::PAD, y: l.footer + 16.0, w: W as f32 - 2.0 * super::PAD, h: 28.0 };
    let mut out = None;
    let can = t.trims();
    ui.area(bar, |ui| {
        ui.ctl_h = 28.0;
        ui.row_rtl(|ui| {
            let on = ui.enabled;
            ui.enabled = on && can;
            if ui.cbutton("trim.save", text::SAVE.get(), Some("save"), true, &tok) {
                out = Some(t.choice());
            }
            ui.enabled = on;
            if ui.cbutton("trim.keep", text::KEEP.get(), None, false, &tok) {
                out = Some(Choice::Full);
            }
        });
    });
    out
}

/// The thumbnails, the selection, its handles and the playhead; presses
/// and drags on it.
fn strip(ui: &mut Ui, l: &Lay, t: &mut Trim, pics: &Pics, tok: &Tok) {
    let s = phys(ui, l.strip);
    let well = if ui.theme.dark { C4::rgb(14, 15, 18) } else { C4::rgb(226, 228, 233) };
    ui.fb.fill_rounded(s.x as i32, s.y as i32, s.w as i32, s.h as i32, ui.px(6.0), well);
    let cell = s.w / THUMBS as f32;
    for (i, p) in pics.thumbs.iter().enumerate() {
        if let Some(p) = p {
            blit(&mut ui.fb, p, (s.x + i as f32 * cell).round() as i32, s.y as i32, 1.0);
        }
    }
    // Outside the selection: dimmed.
    let (sx, ex) = (ui.px(x_of(l, t.len, t.start)), ui.px(x_of(l, t.len, t.end)));
    let dim = C4::new(0, 0, 0, 150);
    ui.fb.fill_rect(s.x as i32, s.y as i32, (sx - s.x).round() as i32, s.h as i32, dim);
    ui.fb.fill_rect(ex.round() as i32, s.y as i32, (s.x1() - ex).round() as i32, s.h as i32, dim);
    // The selection's frame and handles (accent).
    let ac = tok.ac;
    let bw = ui.px(2.0).round().max(1.0) as i32;
    ui.fb.fill_rect(sx.round() as i32, s.y as i32, (ex - sx).round() as i32, bw, ac);
    ui.fb.fill_rect(sx.round() as i32, (s.y1()) as i32 - bw, (ex - sx).round() as i32, bw, ac);
    let hw = ui.px(super::HANDLE_W);
    for x in [sx, ex] {
        let x0 = x - hw / 2.0;
        ui.fb.fill_rounded(x0.round() as i32, s.y as i32 - bw, hw.round() as i32, s.h as i32 + 2 * bw, ui.px(3.0), ac);
        let gx = (x0 + hw / 2.0).round() as i32;
        let gh = ui.px(14.0) as i32;
        ui.fb.fill_rect(gx - bw / 2, (s.y + s.h / 2.0) as i32 - gh / 2, bw.max(1), gh, tok.acfg);
    }
    // The playhead.
    let hx = ui.px(x_of(l, t.len, t.head)).round() as i32;
    let white = if ui.theme.dark { C4::rgb(255, 255, 255) } else { tok.tx };
    ui.fb.fill_rect(hx - bw / 2, s.y as i32 - ui.px(4.0) as i32, bw.max(2), s.h as i32 + ui.px(8.0) as i32, white);
    ui.fb.fill_circle(hx as f32, s.y - ui.px(5.0), ui.px(4.0), white);
    // Input: a press within the strip (and a little around it).
    let inp = ui.input;
    let k = ui.k;
    let hit = FRect { x: l.strip.x - super::REACH, y: l.strip.y - 10.0, w: l.strip.w + 2.0 * super::REACH, h: l.strip.h + 20.0 };
    let lx = inp.mouse.x / k;
    if let Some(p) = inp.pressed
        && crate::ui::contains(hit, crate::objects::Pt::new(p.x / k, p.y / k))
    {
        let x = p.x / k;
        t.press(time_at(l, t.len, x), near_handle(l, t, x));
    } else if inp.held && t.grab.is_some() && inp.moved {
        t.drag(time_at(l, t.len, lx));
    }
    if inp.released.is_some() && t.grab.is_some() {
        if t.grab != Some(Grab::Head) || inp.moved {
            t.drag(time_at(l, t.len, lx));
        }
        t.release();
    }
}

/// The window driver.
struct Window {
    hwnd: Hwnd,
    t: Trim,
    th: Theme,
    input: Input,
    focus: FocusState,
    canvas: PixBuf,
    out: PixBuf,
    pics: Pics,
    rx: Receiver<Pic>,
    want: Shared,
    choice: Option<Choice>,
    sized_for: f32,
    /// The picture last asked for (time, size), so it is asked once.
    asked: Option<(Duration, (u32, u32))>,
    fast: bool,
}

impl Window {
    /// Ask the worker for what the window shows now.
    fn ask(&mut self) {
        let k = self.sized_for.max(1.0);
        let l = layout();
        let pv = (((l.preview.w - 8.0) * k) as u32, ((l.preview.h - 8.0) * k) as u32);
        let thumb = (((l.strip.w / THUMBS as f32) * k).ceil() as u32, (l.strip.h * k) as u32);
        let (m, cv) = &*self.want;
        let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
        if g.thumbs.is_none() {
            g.thumbs = Some(thumb);
        }
        // A picture near the head (a tenth of a second while still).
        let step = if self.t.playing() { Duration::from_millis(33) } else { Duration::from_millis(1) };
        let at = Duration::from_nanos((self.t.head.as_nanos() / step.as_nanos() * step.as_nanos()) as u64);
        if self.asked != Some((at, pv)) {
            self.asked = Some((at, pv));
            g.frame = Some((at, pv));
        }
        cv.notify_one();
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(Pic::Ready(len)) => {
                    self.pics.ready = Some(true);
                    if (len.as_millis() as i64 - self.t.len.as_millis() as i64).abs() > 50 && !len.is_zero() {
                        let fps = (1.0 / self.t.frame.as_secs_f64()).round() as u32;
                        self.t = Trim::new(len, fps);
                    }
                    changed = true;
                }
                Ok(Pic::Thumb(i, p)) => {
                    if self.pics.thumbs.len() < THUMBS {
                        self.pics.thumbs.resize(THUMBS, None);
                    }
                    self.pics.thumbs[i] = Some(p);
                    changed = true;
                }
                Ok(Pic::Frame(t, p)) => {
                    self.pics.frame = Some((t, p));
                    changed = true;
                }
                Ok(Pic::None) => {
                    self.pics.ready = Some(false);
                    changed = true;
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return changed,
            }
        }
    }

    fn render(&mut self) {
        for _ in 0..4 {
            if self.choice.is_some() || self.canvas.width() == 0 {
                return;
            }
            let k = wind::follow_scale(self.hwnd, &mut self.sized_for, W, H);
            // The window's own keys first.
            let now = Instant::now();
            let mut rest = self.input.clone();
            rest.keys.clear();
            for key in &self.input.keys {
                match *key {
                    KeyIn::Key { vk, mods } if !(self.focus.focused_id().is_some() && matches!(vk, 0x0D | 0x20)) && vk != 0x09 => {
                        if let Some(c) = self.t.key(vk, mods.shift, now) {
                            self.choice = Some(c);
                        }
                    }
                    k => rest.keys.push(k),
                }
            }
            if self.choice.is_some() {
                return;
            }
            let (w, h) = self.canvas.dimensions();
            let area = FRect { x: 0.0, y: 0.0, w: w as f32 / k, h: h as f32 / k };
            let mut clip = crate::ui::SystemClipboard;
            let fb = Fb::new(self.canvas.as_raw_mut(), w as usize);
            let mut ui = Ui::new(fb, &self.th, &rest, &mut self.focus, &mut clip, k, area);
            let c = paint(&mut ui, &mut self.t, &self.pics, now);
            let out = ui.finish();
            self.input.end_frame();
            if c.is_some() {
                self.choice = c;
                return;
            }
            self.ask();
            if !out.redraw {
                return;
            }
        }
    }

    fn settle(&mut self) {
        if self.choice.is_some() {
            wind::close(self.hwnd);
        }
        let fast = self.t.playing();
        if fast != self.fast {
            self.fast = fast;
            wind::set_fast_timer(self.hwnd, fast);
        }
    }
}

impl Driver for Window {
    fn on_create(&mut self, hwnd: Hwnd) {
        self.hwnd = hwnd;
    }

    fn on_event(&mut self, ev: Ev) -> bool {
        match ev {
            Ev::Resize(w, h) => {
                self.canvas = PixBuf::new(w.max(1), h.max(1));
                self.render();
            }
            Ev::Close => self.choice = Some(Choice::Full),
            Ev::Timer => {
                let pics = self.poll();
                let playing = self.t.tick(Instant::now());
                wind::follow_scale(self.hwnd, &mut self.sized_for, W, H);
                if pics || playing || self.fast {
                    self.render();
                }
                self.settle();
                return pics || playing || self.fast;
            }
            Ev::Move { .. } => {
                self.input.feed(&ev);
                self.render();
                wind::invalidate(self.hwnd);
            }
            _ => {
                self.input.feed(&ev);
                self.render();
            }
        }
        self.settle();
        self.choice.is_none()
    }

    fn frame(&mut self) -> Option<&mut PixBuf> {
        if self.canvas.width() == 0 || self.choice.is_some() {
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

    fn on_quit(&mut self) {
        let (m, cv) = &*self.want;
        m.lock().unwrap_or_else(|e| e.into_inner()).quit = true;
        cv.notify_all();
    }
}

/// Open the window for `path` on a thread of its own; the outcome comes on
/// the receiver once it is closed (and the cut made).
pub fn open(path: &Path, th: Theme) -> Option<Receiver<Outcome>> {
    let (otx, orx) = chan::channel();
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("rustshot-trim".into())
        .spawn(move || {
            let c = run(&path, th);
            let _ = otx.send(apply(&path, c));
        })
        .ok()?;
    Some(orx)
}

/// Run the window on this thread until it closes; the choice.
fn run(path: &Path, th: Theme) -> Choice {
    let (mut w, join) = Window::open(path, th);
    let spec = WindowSpec { title: text::TITLE.get().into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::MinClose };
    if let Err(e) = wind::run_window(spec, &mut w) {
        eprintln!("rustshot: trim window: {e:#}");
    }
    w.stop_worker(join);
    w.choice.unwrap_or(Choice::Full)
}

impl Window {
    /// The window for `path`, and its decoding worker (started).
    fn open(path: &Path, th: Theme) -> (Window, Option<std::thread::JoinHandle<()>>) {
        let len = crate::rec::cut::length(path).unwrap_or(Duration::from_secs(1));
        let (tx, rx) = chan::channel();
        let want: Shared = Arc::default();
        let w2 = want.clone();
        let p2 = path.to_path_buf();
        let join = std::thread::Builder::new().name("trim-decode".into()).spawn(move || worker(p2, w2, tx)).ok();
        let fps = if crate::rec::cut::Kind::of(path) == Some(crate::rec::cut::Kind::Gif) { 24 } else { 30 };
        let w = Window {
            hwnd: Hwnd::default(),
            t: Trim::new(len, fps),
            th,
            input: Input::default(),
            focus: FocusState::default(),
            canvas: PixBuf::default(),
            out: PixBuf::default(),
            pics: Pics::default(),
            rx,
            want,
            choice: None,
            sized_for: 0.0,
            asked: None,
            fast: false,
        };
        (w, join)
    }

    /// The window is gone: the worker stops.
    fn stop_worker(&self, join: Option<std::thread::JoinHandle<()>>) {
        let (m, cv) = &*self.want;
        m.lock().unwrap_or_else(|e| e.into_inner()).quit = true;
        cv.notify_all();
        if let Some(j) = join {
            let _ = j.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live: the real window on a GIF: it draws, the thumbnails and the
    /// preview arrive from the worker, closing keeps the full length.
    /// `cargo test live_trim_window -- --ignored --nocapture`
    #[test]
    #[ignore = "live: opens the trim window"]
    fn live_trim_window_shows_pictures() {
        if !wind::has_display() {
            return;
        }
        let _g = wind::test_window_lock();
        let dir = std::env::temp_dir().join(format!("rustshot-trim-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("clip.gif");
        let mut enc = crate::rec::gif::GifEncoder::new(std::io::Cursor::new(Vec::new()), 160, 90, 10).unwrap();
        for k in 0..30u64 {
            use crate::rec::VideoEncoder;
            let px = [(k * 8) as u8, 120, 200 - (k * 5) as u8, 255].repeat(160 * 90);
            enc.push_video(&crate::rec::Frame::bgra(160, 90, px, Duration::from_millis(k * 100))).unwrap();
        }
        std::fs::write(&p, enc.into_inner().unwrap().into_inner()).unwrap();
        let (mut w, join) = Window::open(&p, crate::theme::DARK);
        let spec = WindowSpec { title: text::TITLE.get().into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::MinClose };
        let frames = {
            let mut ac = wind::AutoClose::new(&mut w, 2000);
            wind::run_window(spec, &mut ac).expect("trim window");
            ac.frames
        };
        w.stop_worker(join);
        println!("{frames} frames, {} thumbnails, preview {}", w.pics.thumbs.iter().flatten().count(), w.pics.frame.is_some());
        assert!(frames > 0, "it drew");
        assert_eq!(w.pics.ready, Some(true));
        assert!(w.pics.thumbs.iter().flatten().count() == THUMBS, "every thumbnail");
        assert!(w.pics.frame.is_some(), "the preview");
        assert_eq!(w.t.len, Duration::from_secs(3));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
