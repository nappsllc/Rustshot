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
#[cfg(test)]
pub(crate) use super::worker::Cmd;
pub(crate) use super::worker::{Opener, Parts, Worker};
use super::{border, countdown, BarSpot, Effect, End, Geom, MicFlags};
use crate::editor::style::Ui;
use crate::objects::Pt;
use crate::rec::gif::GifEncoder;
use crate::rec::win::{self as recwin, mf, MfEncoder, WasapiCapture, WasapiLoopback};
use crate::rec::{AudioSource, RecFormat, RecSpec, VideoEncoder};
use crate::theme::Theme;
use anyhow::{bail, Context, Result};
use std::cell::RefCell;
use std::sync::mpsc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
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
    dib: Dib,
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
        Ok(Layer { hwnd, shown: false, dib: Dib::default() })
    }

    /// Show `bgra` (`w`x`h`, premultiplied) at (x, y).
    fn put(&mut self, x: i32, y: i32, w: u32, h: u32, bgra: &[u8]) {
        if w == 0 || h == 0 || bgra.len() < (w * h * 4) as usize {
            self.hide();
            return;
        }
        self.dib.put(self.hwnd, x, y, w, h, bgra);
        unsafe {
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

/// A memory DC with a 32-bit DIB section, kept from one
/// `UpdateLayeredWindow` to the next (made again only when the size
/// changes): a window repainted many times a second allocates nothing.
#[derive(Default)]
pub(crate) struct Dib {
    /// (memory DC, bitmap, the DC's bitmap before, bits, w, h)
    cur: Option<(HDC, HBITMAP, HGDIOBJ, *mut u8, u32, u32)>,
}

impl Dib {
    fn ensure(&mut self, w: u32, h: u32) -> Option<(HDC, *mut u8)> {
        if let Some((dc, _, _, bits, cw, ch)) = self.cur
            && (cw, ch) == (w, h)
        {
            return Some((dc, bits));
        }
        self.release();
        unsafe {
            let mem = CreateCompatibleDC(None);
            if mem.is_invalid() {
                return None;
            }
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits = std::ptr::null_mut();
            match CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bmp) if !bits.is_null() => {
                    let old = SelectObject(mem, HGDIOBJ(bmp.0));
                    self.cur = Some((mem, bmp, old, bits as *mut u8, w, h));
                    Some((mem, bits as *mut u8))
                }
                Ok(bmp) => {
                    let _ = DeleteObject(HGDIOBJ(bmp.0));
                    let _ = DeleteDC(mem);
                    None
                }
                Err(_) => {
                    let _ = DeleteDC(mem);
                    None
                }
            }
        }
    }

    fn release(&mut self) {
        if let Some((mem, bmp, old, ..)) = self.cur.take() {
            unsafe {
                SelectObject(mem, old);
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                let _ = DeleteDC(mem);
            }
        }
    }

    /// Put `bgra` (`w`x`h`, premultiplied BGRA) on layered window `hwnd`
    /// at (x, y).
    pub(crate) fn put(&mut self, hwnd: HWND, x: i32, y: i32, w: u32, h: u32, bgra: &[u8]) {
        let _ = self.put_alpha(hwnd, x, y, w, h, bgra, 255);
    }

    /// [`Dib::put`] with the whole layer at constant alpha `alpha` (a
    /// pin's opacity). False when nothing was put up (the size is out of
    /// range, or the DIB section could not be made).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn put_alpha(&mut self, hwnd: HWND, x: i32, y: i32, w: u32, h: u32, bgra: &[u8], alpha: u8) -> bool {
        let n = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4));
        let Some(n) = n.filter(|&n| n > 0 && bgra.len() >= n && w <= i32::MAX as u32 && h <= i32::MAX as u32) else { return false };
        let Some((mem, bits)) = self.ensure(w, h) else { return false };
        unsafe {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits, n);
            let screen = GetDC(None);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let (pos, size, src) = (POINT { x, y }, SIZE { cx: w as i32, cy: h as i32 }, POINT::default());
            let _ = UpdateLayeredWindow(
                hwnd,
                Some(screen),
                Some(&pos),
                Some(&size),
                Some(mem),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen);
        }
        true
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        self.release();
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
    /// Show it around `area`, on each monitor `(rect, thickness)` it is on.
    pub(crate) fn show(area: crate::capture::IRect, monitors: &[(crate::capture::IRect, i32)]) -> Result<Border> {
        let mut strips = Vec::new();
        let red = super::RED;
        for (x, y, w, h) in border::strips(area, monitors) {
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
    spot: Option<BarSpot>,
    drawn: Option<(BarView, Option<BarAct>, Option<BarAct>)>,
}

impl BarWin {
    /// A bar whose clicks queue for the thread's tick; `nudge` gets a
    /// `WM_TIMER` after input (0: none).
    pub(crate) fn new(nudge: isize) -> Result<BarWin> {
        let layer = Layer::new(false)?;
        BAR.with(|b| *b.borrow_mut() = BarInput { hwnd: layer.hwnd.0 as isize, nudge, ..Default::default() });
        Ok(BarWin { layer, spot: None, drawn: None })
    }

    /// Draw `v` (when it changed), at the DPI of the monitor it is on;
    /// placed by `geom` the first time and again when its size changes.
    pub(crate) fn show(&mut self, geom: &Geom, th: &Theme, v: &BarView) {
        if v.hidden {
            self.layer.hide();
            self.drawn = None; // drawn again when shown
            return;
        }
        let font = Some(crate::fonts::ui_font());
        let spot = super::spot_bar(geom, self.spot, |s| {
            let p = bar::layout(&Ui { th, s, font }, v).panel;
            (p.w.round() as i32, p.h.round() as i32)
        });
        let moved = self.spot != Some(spot);
        self.spot = Some(spot);
        let ui = Ui { th, s: spot.scale, font };
        let l = bar::layout(&ui, v);
        let (hover, pressed) = BAR.with(|b| {
            let mut b = b.borrow_mut();
            b.layout = Some(l.clone());
            (b.hover, b.pressed)
        });
        let state = (v.clone(), hover, pressed);
        if !moved && self.drawn.as_ref() == Some(&state) {
            return;
        }
        let m = (bar::MARGIN * spot.scale).round() as i32;
        let pos = (spot.pos.0 - m, spot.pos.1 - m);
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
    /// The number and the arc (in 60ths of a second) on screen.
    drawn: Option<(u32, u16)>,
}

impl CountWin {
    fn show(&mut self, geom: &Geom, th: &Theme, n: Option<(u32, f32)>) {
        let key = n.map(|(n, f)| (n, (f * 60.0).round() as u16));
        if key == self.drawn {
            return;
        }
        self.drawn = key;
        let Some((n, frac)) = n else {
            self.layer.hide();
            return;
        };
        // At the DPI of the monitor the area (and so the panel) is on.
        let s = geom.area_scale();
        let ui = Ui { th, s, font: Some(crate::fonts::ui_font()) };
        let (w, h) = countdown::size(s);
        let px = super::render_layer(w as usize, h as usize, |f| countdown::draw(f, &ui, n, frac));
        let (x, y) = super::centre_on(geom.area, (w as i32, h as i32));
        self.layer.put(x, y, w, h, &px);
    }
}

/// Open the capture, the sound and the encoder for `spec` (the encoder
/// creates `.part`). Sound that cannot be opened is left out and said.
fn prepare(spec: &RecSpec, mic: MicFlags) -> Result<Parts> {
    if let Some(dir) = spec.out.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    // MP4: NV12 at the encoder's size, made on the GPU where it can be.
    let nv12 = (spec.format == RecFormat::Mp4).then(|| recwin::nv12::fit(spec.area.2, spec.area.3));
    let (src, _) = recwin::open_source(spec.area, nv12)?;
    let (w, h) = src.size();
    let mut audio: Vec<Box<dyn AudioSource>> = Vec::new();
    let mut warnings = Vec::new();
    if spec.format == RecFormat::Mp4 {
        if spec.system_audio {
            match WasapiLoopback::open() {
                Ok(a) => audio.push(Box::new(mic.system(a))),
                Err(e) => warnings.push(format!("system sound not recorded: {e:#}")),
            }
        }
        if let Some(id) = &spec.mic {
            match WasapiCapture::open(Some(id)) {
                Ok(a) => audio.push(Box::new(mic.opened(a))),
                Err(e) => warnings.push(format!("microphone not recorded: {e:#}")),
            }
        }
    }
    for w in &warnings {
        eprintln!("rustshot: {w}");
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
    Ok(Parts { src, audio, enc, warnings, late: Vec::new() })
}

/// The windows and the worker of one recording.
pub struct Recorder {
    worker: Worker,
    geom: Geom,
    th: Theme,
    border: Option<Border>,
    bar: Option<BarWin>,
    count: Option<CountWin>,
    /// The camera bubble: up until the recording is over.
    bubble: Option<crate::cam::Bubble>,
}

impl Recorder {
    pub fn start(
        spec: RecSpec,
        geom: Geom,
        th: Theme,
        mic: MicFlags,
        nudge: HWND,
        bubble: Option<crate::cam::Bubble>,
    ) -> Result<Recorder> {
        let mut r = Recorder::start_with(spec, geom, th, nudge, Box::new(move |s: &RecSpec| prepare(s, mic)))?;
        r.bubble = bubble;
        Ok(r)
    }

    /// [`Recorder::start`] with the parts opened by `open`. When a window
    /// or the worker cannot be made, the ones already up are taken down
    /// (dropped) before the error returns.
    pub(crate) fn start_with(spec: RecSpec, geom: Geom, th: Theme, nudge: HWND, open: Opener) -> Result<Recorder> {
        let border = Border::show(geom.area, &geom.border_monitors())?;
        let bar = BarWin::new(nudge.0 as isize)?;
        let count = CountWin { layer: Layer::new(true)?, drawn: None };
        let worker = Worker::spawn(spec, open)?;
        Ok(Recorder { worker, geom, th, border: Some(border), bar: Some(bar), count: Some(count), bubble: None })
    }

    pub fn send(&mut self, e: Effect) {
        self.worker.send(e);
    }

    pub fn take_clicks(&mut self) -> Vec<BarAct> {
        if self.bar.is_some() { BarWin::take_clicks() } else { Vec::new() }
    }

    pub fn poll_end(&mut self) -> Option<End> {
        match self.worker.rx.try_recv() {
            // A camera that stopped meanwhile is said with the file.
            Ok(e) => Some(super::with_camera_error(e, self.bubble.as_ref().and_then(|b| b.error()))),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(End::Failed("the recording thread stopped".into())),
        }
    }

    /// Hide the camera bubble or show it again (the pill's camera button).
    pub fn set_cam_hidden(&mut self, hidden: bool) {
        if let Some(b) = &mut self.bubble {
            b.set_hidden(hidden);
        }
    }

    pub fn show(&mut self, v: &BarView, count: Option<(u32, f32)>) {
        if let Some(b) = &mut self.bar {
            b.show(&self.geom, &self.th, v);
        }
        if let Some(c) = &mut self.count {
            c.show(&self.geom, &self.th, count);
        }
    }

    /// The recording is over: take the windows down.
    pub fn close(&mut self) {
        self.bubble = None;
        self.count = None;
        self.bar = None;
        self.border = None;
    }
}

#[cfg(test)]
mod tests;
