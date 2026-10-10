//! Windows: the layered windows around a recording (red border strips,
//! the control bar, the countdown), all top-most tool windows that never
//! take focus and are excluded from screen capture
//! (`WDA_EXCLUDEFROMCAPTURE`), and the worker thread that opens the
//! pipeline, runs the [`Session`] and finishes the file.
//!
//! The windows live on the caller's (UI) thread; their input is queued in
//! a thread-local and a `WM_TIMER` is posted to the caller's window so it
//! ticks at once ([`Recorder::take_clicks`]).

use super::bar::{self, BarAct, BarView};
use super::{border, countdown, Effect, End, Geom, Muted};
use crate::editor::style::Ui;
use crate::objects::Pt;
use crate::rec::gif::GifEncoder;
use crate::rec::win::{self as recwin, mf, MfEncoder, WasapiCapture, WasapiLoopback};
use crate::rec::{AudioSource, FrameSource, RecFormat, RecSpec, Session, VideoEncoder};
use crate::theme::Theme;
use anyhow::{bail, Context, Result};
use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("rustshot_rec");
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Input on the control bar, queued for the owner's tick.
#[derive(Default)]
struct BarInput {
    hwnd: isize,
    /// The owner's window: gets a `WM_TIMER` after input.
    nudge: isize,
    layout: Option<bar::BarLayout>,
    hover: Option<BarAct>,
    pressed: Option<BarAct>,
    clicks: Vec<BarAct>,
    tracking: bool,
}

thread_local! {
    static BAR: RefCell<BarInput> = RefCell::new(BarInput::default());
}

fn lp_pt(lp: LPARAM) -> Pt {
    let x = (lp.0 & 0xFFFF) as u16 as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
    Pt::new(x, y)
}

unsafe fn bar_input(hwnd: HWND, msg: u32, lp: LPARAM) {
    let nudge = BAR.with(|b| {
        let mut b = b.borrow_mut();
        if b.hwnd != hwnd.0 as isize {
            return None;
        }
        let at = b.layout.as_ref().and_then(|l| bar::hit(l, lp_pt(lp)));
        let before = (b.hover, b.pressed, b.clicks.len());
        match msg {
            WM_MOUSEMOVE => {
                if !b.tracking {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    b.tracking = unsafe { TrackMouseEvent(&mut t) }.is_ok();
                }
                b.hover = at;
            }
            WM_LBUTTONDOWN => {
                unsafe { SetCapture(hwnd) };
                b.pressed = at;
            }
            WM_LBUTTONUP => {
                let _ = unsafe { ReleaseCapture() };
                if let Some(a) = at
                    && b.pressed == Some(a)
                {
                    b.clicks.push(a);
                }
                b.pressed = None;
            }
            WM_MOUSELEAVE => {
                b.tracking = false;
                b.hover = None;
            }
            _ => {}
        }
        ((b.hover, b.pressed, b.clicks.len()) != before).then_some(b.nudge)
    });
    if let Some(n) = nudge
        && n != 0
    {
        let _ = unsafe { PostMessageW(Some(HWND(n as *mut _)), WM_TIMER, WPARAM(1), LPARAM(0)) };
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR => {
                if let Ok(c) = LoadCursorW(None, IDC_ARROW) {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSELEAVE => {
                bar_input(hwnd, msg, lp);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn register() -> Result<()> {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *OK.get_or_init(|| unsafe {
        let Ok(inst) = GetModuleHandleW(None) else { return false };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc) != 0
    });
    if !ok {
        bail!("register the recording window class");
    }
    Ok(())
}

/// A top-most layered tool window showing premultiplied BGRA pixels.
struct Layer {
    hwnd: HWND,
    shown: bool,
}

impl Layer {
    fn new(click_through: bool) -> Result<Layer> {
        register()?;
        let mut ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
        if click_through {
            ex |= WS_EX_TRANSPARENT;
        }
        let hwnd = unsafe {
            let inst = GetModuleHandleW(None)?;
            CreateWindowExW(ex, CLASS, w!("Rustshot recording"), WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None)
        }
        .context("create a recording window")?;
        // Never in the recording (nor in screenshots); Windows 10 2004+.
        if let Err(e) = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } {
            eprintln!("rustshot: cannot exclude the recording frame from capture: {e}");
        }
        Ok(Layer { hwnd, shown: false })
    }

    /// Show `bgra` (`w`x`h`, premultiplied) at (x, y).
    fn put(&mut self, x: i32, y: i32, w: u32, h: u32, bgra: &[u8]) {
        if w == 0 || h == 0 || bgra.len() < (w * h * 4) as usize {
            self.hide();
            return;
        }
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits = std::ptr::null_mut();
            if let Ok(bmp) = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                && !bits.is_null()
            {
                std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, (w * h * 4) as usize);
                let old = SelectObject(mem, HGDIOBJ(bmp.0));
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let (pos, size, src) = (POINT { x, y }, SIZE { cx: w as i32, cy: h as i32 }, POINT::default());
                let _ = UpdateLayeredWindow(
                    self.hwnd,
                    Some(screen),
                    Some(&pos),
                    Some(&size),
                    Some(mem),
                    Some(&src),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(mem, old);
                let _ = DeleteObject(HGDIOBJ(bmp.0));
            }
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            if !self.shown {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.shown = true;
            }
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    fn hide(&mut self) {
        if self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// The red border around `area`.
pub(crate) struct Border {
    /// Held: dropping them takes the strips down.
    _strips: Vec<Layer>,
}

impl Border {
    /// Show it around `area` (inside `screen` bounds), `t` px thick.
    pub(crate) fn show(area: crate::capture::IRect, t: i32, screen: crate::capture::IRect) -> Result<Border> {
        let mut strips = Vec::new();
        let red = super::RED;
        for (x, y, w, h) in border::strips(area, t, screen) {
            let mut l = Layer::new(true)?;
            let px = [red.b, red.g, red.r, 255].repeat((w * h) as usize);
            l.put(x, y, w, h, &px);
            strips.push(l);
        }
        Ok(Border { _strips: strips })
    }
}

/// The control bar window.
pub(crate) struct BarWin {
    layer: Layer,
    pos: Option<(i32, i32)>,
    drawn: Option<(BarView, Option<BarAct>, Option<BarAct>)>,
}

impl BarWin {
    /// A bar whose clicks queue for the thread's tick; `nudge` gets a
    /// `WM_TIMER` after input (0: none).
    pub(crate) fn new(nudge: isize) -> Result<BarWin> {
        let layer = Layer::new(false)?;
        BAR.with(|b| *b.borrow_mut() = BarInput { hwnd: layer.hwnd.0 as isize, nudge, ..Default::default() });
        Ok(BarWin { layer, pos: None, drawn: None })
    }

    /// Draw `v` (when it changed), placed by `geom` the first time.
    pub(crate) fn show(&mut self, geom: &Geom, th: &Theme, v: &BarView) {
        let ui = Ui { th, s: geom.scale, font: Some(crate::fonts::ui_font()) };
        let l = bar::layout(&ui, v);
        let (hover, pressed) = BAR.with(|b| {
            let mut b = b.borrow_mut();
            b.layout = Some(l.clone());
            (b.hover, b.pressed)
        });
        let state = (v.clone(), hover, pressed);
        if self.drawn.as_ref() == Some(&state) {
            return;
        }
        let pos = *self.pos.get_or_insert_with(|| {
            let s = geom.scale;
            let gap = (8.0 * s).round() as i32;
            let (pw, ph) = (l.panel.w.round() as i32, l.panel.h.round() as i32);
            let (x, y) = super::place_bar(geom.area, &geom.monitors, (pw, ph), gap, gap);
            let m = (bar::MARGIN * s).round() as i32;
            (x - m, y - m)
        });
        let (w, h) = l.size;
        let px = super::render_layer(w as usize, h as usize, |f| bar::draw(f, &ui, &l, v, hover, pressed));
        self.layer.put(pos.0, pos.1, w, h, &px);
        self.drawn = Some(state);
    }

    pub(crate) fn take_clicks() -> Vec<BarAct> {
        BAR.with(|b| std::mem::take(&mut b.borrow_mut().clicks))
    }
}

impl Drop for BarWin {
    fn drop(&mut self) {
        BAR.with(|b| *b.borrow_mut() = BarInput::default());
    }
}

/// The countdown window.
struct CountWin {
    layer: Layer,
    drawn: Option<u32>,
}

impl CountWin {
    fn show(&mut self, geom: &Geom, th: &Theme, n: Option<u32>) {
        if n == self.drawn {
            return;
        }
        self.drawn = n;
        let Some(n) = n else {
            self.layer.hide();
            return;
        };
        let ui = Ui { th, s: geom.scale, font: Some(crate::fonts::ui_font()) };
        let (w, h) = countdown::size(geom.scale);
        let px = super::render_layer(w as usize, h as usize, |f| countdown::draw(f, &ui, n));
        let (x, y) = super::centre_on(geom.area, (w as i32, h as i32));
        self.layer.put(x, y, w, h, &px);
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

enum Cmd {
    Go,
    Pause,
    Resume,
    Stop,
    Cancel,
}

type Parts = (Box<dyn FrameSource>, Vec<Box<dyn AudioSource>>, Box<dyn VideoEncoder>);

/// Open the capture, the sound and the encoder for `spec` (the encoder
/// creates `.part`). Sound that cannot be opened is left out (and said).
fn prepare(spec: &RecSpec, muted: Arc<AtomicBool>) -> Result<Parts> {
    if let Some(dir) = spec.out.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let (src, _) = recwin::open_source(spec.area)?;
    let (w, h) = src.size();
    let mut audio: Vec<Box<dyn AudioSource>> = Vec::new();
    if spec.format == RecFormat::Mp4 {
        if spec.system_audio {
            match WasapiLoopback::open() {
                Ok(a) => audio.push(Box::new(a)),
                Err(e) => eprintln!("rustshot: system sound is not recorded: {e:#}"),
            }
        }
        if let Some(id) = &spec.mic {
            match WasapiCapture::open(Some(id)) {
                Ok(a) => audio.push(Box::new(Muted { inner: a, muted })),
                Err(e) => eprintln!("rustshot: the microphone is not recorded: {e:#}"),
            }
        }
    }
    let part = spec.part_path();
    let enc: Box<dyn VideoEncoder> = match spec.format {
        RecFormat::Mp4 => {
            if !mf::available() {
                bail!("Media Foundation is not available (Windows N editions: install the Media Feature Pack)");
            }
            if !mf::encoders_available().0 {
                bail!("no H.264 encoder is installed");
            }
            let e = MfEncoder::create(&part, w, h, spec.capture_fps(), spec.quality, !audio.is_empty())?;
            let hw = if e.hardware() { "hardware" } else { "software" };
            eprintln!("rustshot: recording {w}x{h} with {} ({hw})", e.encoder_name());
            Box::new(e)
        }
        RecFormat::Gif => Box::new(GifEncoder::create(&part, w, h, spec.capture_fps())?),
    };
    Ok((src, audio, enc))
}

fn work(spec: RecSpec, muted: Arc<AtomicBool>, rx: Receiver<Cmd>) -> End {
    recwin::com_init();
    let part = spec.part_path();
    let (src, mut audio, enc) = match prepare(&spec, muted) {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return End::Failed(format!("{e:#}"));
        }
    };
    loop {
        match rx.recv() {
            Ok(Cmd::Go) => break,
            Ok(Cmd::Cancel | Cmd::Stop) | Err(_) => {
                drop(enc); // closes `.part`
                let _ = std::fs::remove_file(&part);
                return End::Cancelled;
            }
            Ok(Cmd::Pause | Cmd::Resume) => {}
        }
    }
    // Sound captured during the countdown is not part of the recording.
    let mut scratch = Vec::new();
    for a in &mut audio {
        while a.read(&mut scratch) > 0 {
            scratch.clear();
        }
    }
    let session = match Session::start(spec, src, audio, enc) {
        Ok(s) => s,
        Err(e) => return End::Failed(format!("{e:#}")),
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Cmd::Pause) => session.pause(),
            Ok(Cmd::Resume) => session.resume(),
            Ok(Cmd::Go) => {}
            Ok(Cmd::Stop | Cmd::Cancel) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                // The encoder failed: stop now and keep what was written.
                if session.stats().failed {
                    break;
                }
            }
        }
    }
    match session.stop() {
        Ok(p) => End::Saved(p),
        Err(e) => End::Failed(format!("{e:#}")),
    }
}

struct Worker {
    tx: Sender<Cmd>,
    rx: Receiver<End>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn spawn(spec: RecSpec, muted: Arc<AtomicBool>) -> Result<Worker> {
        let (tx, crx) = mpsc::channel();
        let (etx, rx) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("rec-worker".into())
            .spawn(move || {
                let _ = etx.send(work(spec, muted, crx));
            })
            .context("start the recording thread")?;
        Ok(Worker { tx, rx, join: Some(join) })
    }
}

impl Drop for Worker {
    /// The process is going away mid-recording: finish the file first.
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// The windows and the worker of one recording.
pub struct Recorder {
    worker: Worker,
    geom: Geom,
    th: Theme,
    border: Option<Border>,
    bar: Option<BarWin>,
    count: Option<CountWin>,
}

impl Recorder {
    pub fn start(spec: RecSpec, geom: Geom, th: Theme, muted: Arc<AtomicBool>, nudge: HWND) -> Result<Recorder> {
        let t = border::thickness(geom.scale);
        let border = Border::show(geom.area, t, border::bounds(&geom.monitors))?;
        let bar = BarWin::new(nudge.0 as isize)?;
        let count = CountWin { layer: Layer::new(true)?, drawn: None };
        let worker = Worker::spawn(spec, muted)?;
        Ok(Recorder { worker, geom, th, border: Some(border), bar: Some(bar), count: Some(count) })
    }

    pub fn send(&mut self, e: Effect) {
        let c = match e {
            Effect::Go => Cmd::Go,
            Effect::Pause => Cmd::Pause,
            Effect::Resume => Cmd::Resume,
            Effect::Stop => Cmd::Stop,
            Effect::Cancel => Cmd::Cancel,
        };
        let _ = self.worker.tx.send(c);
    }

    pub fn take_clicks(&mut self) -> Vec<BarAct> {
        if self.bar.is_some() { BarWin::take_clicks() } else { Vec::new() }
    }

    pub fn poll_end(&mut self) -> Option<End> {
        match self.worker.rx.try_recv() {
            Ok(e) => Some(e),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(End::Failed("the recording thread stopped".into())),
        }
    }

    pub fn show(&mut self, v: &BarView, count: Option<u32>) {
        if let Some(b) = &mut self.bar {
            b.show(&self.geom, &self.th, v);
        }
        if let Some(c) = &mut self.count {
            c.show(&self.geom, &self.th, count);
        }
    }

    /// The recording is over: take the windows down.
    pub fn close(&mut self) {
        self.count = None;
        self.bar = None;
        self.border = None;
    }
}

#[cfg(test)]
mod tests;
