//! Windows: the camera bubble window. A top-most layered tool window that
//! never takes focus, shows premultiplied BGRA through
//! `UpdateLayeredWindow` and is NOT excluded from capture (it is meant to
//! be in the recording). It lives on the caller's (UI) thread; a camera
//! thread opens the camera, reads it, fits each picture to the bubble and
//! posts it (at most about 30 a second); the window composes it with the
//! shape, ring and shadow ([`super::compose`]) when the message arrives.
//!
//! Dragging moves it; within [`super::SNAP`] of a corner position of the
//! area it snaps there, and where it is dropped is remembered relative to
//! the area ([`super::remember`]).
//!
//! The camera is open only while the bubble exists: dropping the
//! [`Bubble`] stops the camera thread (it releases the camera within a
//! frame) and destroys the window.

use super::{CamFrame, CamSource, Look, Template};
use crate::capture::IRect;
use crate::rec::win::com_init;
use crate::theme::Theme;
use anyhow::{bail, Context, Result};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("rustshot_cam");
/// A picture is waiting in [`Shared::frame`].
const WM_FRAME: u32 = WM_APP + 1;
/// The camera failed ([`Shared::error`]).
const WM_FAIL: u32 = WM_APP + 2;
/// Pictures closer together than this are skipped (about 30 fps at most).
const FRAME_GAP: Duration = Duration::from_millis(25);
/// How long closing waits for the camera thread to let go of the camera.
/// Not at all in the app: the thread stops within a frame and releases
/// the camera on its own (that takes a C920 about 300 ms), which must not
/// hold up the UI thread at Stop. Tests wait to check the release.
#[cfg(not(test))]
const CLOSE_WAIT: Duration = Duration::ZERO;
#[cfg(test)]
const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// Opens the camera on the camera thread (tests inject their own).
pub(crate) type Opener = Box<dyn FnOnce() -> Result<Box<dyn CamSource>> + Send>;

/// Between the camera thread and the window.
struct Shared {
    frame: Mutex<Option<CamFrame>>,
    error: Mutex<Option<String>>,
    stop: AtomicBool,
    /// The bubble's side in physical px (what pictures are fitted to).
    side: AtomicU32,
    hwnd: isize,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The window's state, on its thread.
struct State {
    hwnd: isize,
    /// Gets a `WM_TIMER` when the camera fails, so the owner looks.
    nudge: isize,
    area: IRect,
    scale: f32,
    look: Look,
    th: Theme,
    /// Top-left of the shape (physical px; the window is `pad` larger).
    pos: (i32, i32),
    tpl: Template,
    frame: Option<CamFrame>,
    /// Pressed: (cursor, pos) at the press.
    drag: Option<((i32, i32), (i32, i32))>,
    px: Vec<u8>,
    shared: Arc<Shared>,
    shown: bool,
}

thread_local! {
    static BUBBLE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Run `f` on the state, unless it is gone or already borrowed (a
/// message sent while the state is in use, e.g. from `SetWindowPos`).
fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    BUBBLE.with(|b| {
        let mut b = b.try_borrow_mut().ok()?;
        b.as_mut().map(f)
    })
}

fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

fn hwnd_of(h: isize) -> HWND {
    HWND(h as *mut _)
}

/// Move the window so the shape's top-left is at `pos`.
fn move_to(hwnd: isize, pos: (i32, i32), pad: i32) {
    unsafe {
        let _ = SetWindowPos(
            hwnd_of(hwnd),
            Some(HWND_TOPMOST),
            pos.0 - pad,
            pos.1 - pad,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Compose the current picture and put it on the window.
fn paint() {
    let job = with(|st| {
        super::compose(&st.tpl, st.frame.as_ref(), st.look.mirror, &mut st.px);
        let pad = st.tpl.pad as i32;
        (st.hwnd, (st.pos.0 - pad, st.pos.1 - pad), st.tpl.w, std::mem::take(&mut st.px), !st.shown)
    });
    let Some((hwnd, (x, y), w, px, first)) = job else { return };
    crate::rec_ui::update_layered(hwnd_of(hwnd), x, y, w, w, &px);
    unsafe {
        if first {
            let _ = ShowWindow(hwnd_of(hwnd), SW_SHOWNOACTIVATE);
        }
        // Over the overlay and the recording frame.
        let _ = SetWindowPos(hwnd_of(hwnd), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
    with(|st| {
        st.px = px;
        st.shown = true;
    });
}

fn end_drag() {
    with(|st| {
        if st.drag.take().is_some() {
            super::remember(super::rel_of(st.pos, st.area, st.tpl.d as i32, st.scale));
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR => {
                if let Ok(c) = LoadCursorW(None, IDC_SIZEALL) {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                SetCapture(hwnd);
                let at = cursor();
                with(|st| st.drag = Some((at, st.pos)));
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let at = cursor();
                let mv = with(|st| {
                    let (c0, p0) = st.drag?;
                    let p = (p0.0 + at.0 - c0.0, p0.1 + at.1 - c0.1);
                    st.pos = super::snap(p, st.area, st.tpl.d as i32, st.scale);
                    Some((st.hwnd, st.pos, st.tpl.pad as i32))
                })
                .flatten();
                if let Some((h, p, pad)) = mv {
                    move_to(h, p, pad);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                end_drag();
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                end_drag();
                LRESULT(0)
            }
            WM_FRAME => {
                let got = with(|st| {
                    if let Some(f) = lock(&st.shared.frame).take() {
                        // A picture fitted before a size change is dropped.
                        if f.w == st.tpl.d {
                            st.frame = Some(f);
                        }
                        true
                    } else {
                        false
                    }
                });
                if got == Some(true) {
                    paint();
                }
                LRESULT(0)
            }
            WM_FAIL => {
                let nudge = with(|st| {
                    st.shown = false;
                    st.nudge
                });
                let _ = ShowWindow(hwnd, SW_HIDE);
                if let Some(n) = nudge
                    && n != 0
                {
                    let _ = PostMessageW(Some(hwnd_of(n)), WM_TIMER, WPARAM(1), LPARAM(0));
                }
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
            hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc) != 0
    });
    if !ok {
        bail!("register the camera window class");
    }
    Ok(())
}

fn post(hwnd: isize, msg: u32) {
    let _ = unsafe { PostMessageW(Some(hwnd_of(hwnd)), msg, WPARAM(0), LPARAM(0)) };
}

/// Open the camera, then read it until told to stop; the camera is
/// released (the source dropped) before `done` is signalled.
fn camera_thread(open: Opener, sh: Arc<Shared>, done: mpsc::Sender<()>) {
    com_init();
    let r = (|| -> Result<()> {
        let mut src = open()?;
        let mut last: Option<Instant> = None;
        while !sh.stop.load(Ordering::SeqCst) {
            let Some(f) = src.read()? else { continue };
            let now = Instant::now();
            if last.is_some_and(|t| now.saturating_duration_since(t) < FRAME_GAP) {
                continue;
            }
            last = Some(now);
            let sq = super::fit_square(&f, sh.side.load(Ordering::SeqCst));
            *lock(&sh.frame) = Some(sq);
            post(sh.hwnd, WM_FRAME);
        }
        Ok(())
    })();
    if let Err(e) = r
        && !sh.stop.load(Ordering::SeqCst)
    {
        let msg = format!("{e:#}");
        eprintln!("rustshot: camera: {msg}");
        *lock(&sh.error) = Some(msg);
        post(sh.hwnd, WM_FAIL);
    }
    let _ = done.send(());
}

/// The camera bubble on screen (see the module docs).
pub struct Bubble {
    hwnd: HWND,
    shared: Arc<Shared>,
    done: Option<Receiver<()>>,
    #[cfg_attr(test, allow(dead_code))]
    device: String,
}

impl Bubble {
    /// Show the bubble for camera `device` ("" = the first) with `look`,
    /// placed where it was last left relative to `area` (physical px) at
    /// UI scale `scale`, its ring in `th`'s accent. `nudge` gets a
    /// `WM_TIMER` when the camera fails (see [`Bubble::error`]).
    #[cfg_attr(test, allow(dead_code))] // tests use fake cameras
    pub fn show(look: Look, device: &str, area: IRect, scale: f32, th: &Theme, nudge: HWND) -> Result<Bubble> {
        let id = device.to_string();
        let open: Opener = Box::new(move || super::open(&id, super::PREF));
        Bubble::show_with(look, device, area, scale, th, nudge, open)
    }

    /// [`Bubble::show`] with the camera opened by `open`.
    pub(crate) fn show_with(
        look: Look,
        device: &str,
        area: IRect,
        scale: f32,
        th: &Theme,
        nudge: HWND,
        open: Opener,
    ) -> Result<Bubble> {
        if BUBBLE.with(|b| b.borrow().is_some()) {
            bail!("a camera bubble is already shown");
        }
        register()?;
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
        let hwnd = unsafe {
            let inst = GetModuleHandleW(None)?;
            CreateWindowExW(ex, CLASS, w!("Rustshot camera"), WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None)
        }
        .context("create the camera window")?;
        let tpl = super::template(look, scale, th);
        let shared = Arc::new(Shared {
            frame: Mutex::new(None),
            error: Mutex::new(None),
            stop: AtomicBool::new(false),
            side: AtomicU32::new(tpl.d),
            hwnd: hwnd.0 as isize,
        });
        let pos = super::place(super::remembered(), area, tpl.d as i32, scale);
        BUBBLE.with(|b| {
            *b.borrow_mut() = Some(State {
                hwnd: hwnd.0 as isize,
                nudge: nudge.0 as isize,
                area,
                scale,
                look,
                th: *th,
                pos,
                tpl,
                frame: None,
                drag: None,
                px: Vec::new(),
                shared: shared.clone(),
                shown: false,
            })
        });
        let (tx, rx) = mpsc::channel();
        let sh = shared.clone();
        let mut bubble = Bubble { hwnd, shared, done: None, device: device.to_string() };
        std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || camera_thread(open, sh, tx))
            .context("start the camera thread")?;
        bubble.done = Some(rx);
        // The placeholder until the first picture.
        paint();
        Ok(bubble)
    }

    /// The camera it shows ("" = the first).
    #[cfg_attr(test, allow(dead_code))]
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Why the camera stopped (the bubble is hidden then).
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.error).clone()
    }

    /// The shape's top-left on screen (physical px).
    #[cfg_attr(not(test), allow(dead_code))] // tests
    pub fn pos(&self) -> Option<(i32, i32)> {
        with(|st| st.pos)
    }

    /// A camera picture is shown (tests).
    #[cfg(test)]
    pub(crate) fn has_picture(&self) -> bool {
        with(|st| st.frame.is_some()).unwrap_or(false)
    }

    /// The window (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Follow `area` (the selection being changed) at UI scale `scale`:
    /// placed where it was left relative to the area. Not while dragged.
    pub fn set_anchor(&mut self, area: IRect, scale: f32) {
        let r = with(|st| {
            if (st.area, st.scale) == (area, scale) || st.drag.is_some() {
                return None;
            }
            let rescale = st.scale != scale;
            st.area = area;
            st.scale = scale;
            if rescale {
                st.tpl = super::template(st.look, scale, &st.th);
                st.frame = None;
                st.shared.side.store(st.tpl.d, Ordering::SeqCst);
            }
            st.pos = super::place(super::remembered(), area, st.tpl.d as i32, scale);
            Some((rescale, st.hwnd, st.pos, st.tpl.pad as i32))
        })
        .flatten();
        match r {
            Some((true, ..)) => paint(),
            Some((false, h, p, pad)) => move_to(h, p, pad),
            None => {}
        }
    }

    /// Change the shape, size or mirroring (and the theme's accent).
    pub fn set_look(&mut self, look: Look, th: &Theme) {
        let changed = with(|st| {
            if st.look == look && st.th == *th {
                return false;
            }
            let resized = st.look.size != look.size;
            st.look = look;
            st.th = *th;
            st.tpl = super::template(look, st.scale, th);
            st.shared.side.store(st.tpl.d, Ordering::SeqCst);
            if resized {
                // Keep it at its corner of the area.
                st.frame = None;
                st.pos = super::place(super::remembered(), st.area, st.tpl.d as i32, st.scale);
            }
            true
        });
        if changed == Some(true) {
            paint();
        }
    }
}

impl Drop for Bubble {
    /// Stop the camera (the thread lets go of it within a frame; see
    /// [`CLOSE_WAIT`]) and take the window down.
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(rx) = self.done.take()
            && !CLOSE_WAIT.is_zero()
        {
            let _ = rx.recv_timeout(CLOSE_WAIT);
        }
        BUBBLE.with(|b| b.borrow_mut().take());
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
