//! Windows: the camera bubble window. A top-most layered tool window that
//! never takes focus, shows premultiplied BGRA through
//! `UpdateLayeredWindow` and is NOT excluded from capture (it is meant to
//! be in the recording). It lives on the caller's (UI) thread; a camera
//! thread opens the camera, reads it, fits each picture to the bubble and
//! posts it (at most about 30 a second); the window composes it with the
//! shape, ring and shadow ([`super::compose`]) when the message arrives.
//! The picture buffers, the scratch and the DIB section are reused from
//! one picture to the next.
//!
//! Dragging moves it; within [`super::SNAP`] of a corner position of the
//! area it snaps there, and where it is dropped is remembered relative to
//! the area ([`super::remember`]). While the overlay's toolbar and Record
//! popover are up it keeps off them ([`Bubble::set_avoid`],
//! [`super::clear_of`]).
//!
//! The camera is open only while the bubble exists: dropping the
//! [`Bubble`] stops the camera thread (it releases the camera within a
//! frame) and destroys the window.

use super::{CamFrame, CamSource, Look, Template};
use crate::capture::IRect;
use crate::rec::win::com_init;
use crate::rec_ui::Dib;
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
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Opens the camera on the camera thread (tests inject their own).
pub(crate) type Opener = Box<dyn FnOnce() -> Result<Box<dyn CamSource>> + Send>;

/// Between the camera thread and the window.
struct Shared {
    /// The newest fitted picture, not yet shown.
    frame: Mutex<Option<CamFrame>>,
    /// A picture the window is done with: the camera thread fits the next
    /// one into its buffer.
    spare: Mutex<Option<CamFrame>>,
    error: Mutex<Option<String>>,
    stop: AtomicBool,
    /// The bubble's side in physical px (what pictures are fitted to).
    side: AtomicU32,
    /// The window; 0 once it is being destroyed. Posting holds the lock,
    /// so nothing reaches a destroyed (or reused) handle.
    hwnd: Mutex<isize>,
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
    /// Rects to keep off (the overlay's toolbar and popover) and the
    /// screen the bubble may go to around a small area.
    avoid: Vec<IRect>,
    screen: IRect,
    /// Top-left of the shape (physical px; the window is `pad` larger).
    pos: (i32, i32),
    tpl: Template,
    frame: Option<CamFrame>,
    /// Pressed: (cursor, pos) at the press.
    drag: Option<((i32, i32), (i32, i32))>,
    px: Vec<u8>,
    dib: Dib,
    shared: Arc<Shared>,
    shown: bool,
}

impl State {
    /// Where the bubble goes: where it was left relative to the area, off
    /// the rects to avoid.
    fn home(&self) -> (i32, i32) {
        let d = self.tpl.d as i32;
        let p = super::place(super::remembered(), self.area, d, self.scale);
        super::clear_of(p, self.area, d, self.scale, &self.avoid, self.screen)
    }
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
        let (x, y, w) = (st.pos.0 - pad, st.pos.1 - pad, st.tpl.w);
        st.dib.put(hwnd_of(st.hwnd), x, y, w, w, &st.px);
        let first = !st.shown;
        st.shown = true;
        (st.hwnd, first)
    });
    let Some((hwnd, first)) = job else { return };
    unsafe {
        if first {
            let _ = ShowWindow(hwnd_of(hwnd), SW_SHOWNOACTIVATE);
        }
        // Over the overlay and the recording frame.
        let _ = SetWindowPos(hwnd_of(hwnd), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Take the waiting picture (if any) and show it; true when one was taken.
fn take_frame() -> bool {
    let got = with(|st| {
        let Some(f) = lock(&st.shared.frame).take() else { return false };
        // A picture fitted before a size change is dropped.
        if f.w == st.tpl.d {
            if let Some(old) = st.frame.replace(f) {
                *lock(&st.shared.spare) = Some(old);
            }
        } else {
            *lock(&st.shared.spare) = Some(f);
        }
        true
    });
    if got == Some(true) {
        paint();
        return true;
    }
    false
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
                take_frame();
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

/// Post `msg` to the window unless it is being destroyed.
fn post(sh: &Shared, msg: u32) {
    let h = lock(&sh.hwnd);
    if *h != 0 {
        let _ = unsafe { PostMessageW(Some(hwnd_of(*h)), msg, WPARAM(0), LPARAM(0)) };
    }
}

/// Open the camera, then read it until told to stop; the camera is
/// released (the source dropped) before `done` is signalled. The camera's
/// picture, the scratch and the fitted pictures are reused.
fn camera_thread(open: Opener, sh: Arc<Shared>, done: mpsc::Sender<()>) {
    com_init();
    let r = (|| -> Result<()> {
        let mut src = open()?;
        let mut last: Option<Instant> = None;
        let (mut raw, mut tmp) = (CamFrame::default(), Vec::new());
        while !sh.stop.load(Ordering::SeqCst) {
            if !src.read(&mut raw)? {
                continue;
            }
            let now = Instant::now();
            if last.is_some_and(|t| now.saturating_duration_since(t) < FRAME_GAP) {
                continue;
            }
            last = Some(now);
            // The window's old picture (or the one it never took) is
            // filled again.
            let mut sq = lock(&sh.spare).take().or_else(|| lock(&sh.frame).take()).unwrap_or_default();
            super::fit_square_into(&raw, sh.side.load(Ordering::SeqCst), &mut tmp, &mut sq);
            if let Some(old) = lock(&sh.frame).replace(sq) {
                *lock(&sh.spare) = Some(old);
            }
            post(&sh, WM_FRAME);
        }
        Ok(())
    })();
    if let Err(e) = r
        && !sh.stop.load(Ordering::SeqCst)
    {
        let msg = format!("{e:#}");
        eprintln!("rustshot: camera: {msg}");
        *lock(&sh.error) = Some(msg);
        post(&sh, WM_FAIL);
    }
    let _ = done.send(());
}

/// The camera bubble on screen (see the module docs).
pub struct Bubble {
    hwnd: HWND,
    shared: Arc<Shared>,
    done: Option<Receiver<()>>,
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
            spare: Mutex::new(None),
            error: Mutex::new(None),
            stop: AtomicBool::new(false),
            side: AtomicU32::new(tpl.d),
            hwnd: Mutex::new(hwnd.0 as isize),
        });
        let mut st = State {
            hwnd: hwnd.0 as isize,
            nudge: nudge.0 as isize,
            area,
            scale,
            look,
            th: *th,
            avoid: Vec::new(),
            screen: area,
            pos: (0, 0),
            tpl,
            frame: None,
            drag: None,
            px: Vec::new(),
            dib: Dib::default(),
            shared: shared.clone(),
            shown: false,
        };
        st.pos = st.home();
        BUBBLE.with(|b| *b.borrow_mut() = Some(st));
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

    /// A camera picture is shown.
    pub fn has_picture(&self) -> bool {
        with(|st| st.frame.is_some()).unwrap_or(false)
    }

    /// Wait up to `timeout` for the camera's first picture and show it
    /// (so a recording does not start on the placeholder); false when none
    /// came (or the camera failed).
    pub fn wait_picture(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        loop {
            take_frame();
            if self.has_picture() {
                return true;
            }
            if self.error().is_some() || Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
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
            st.pos = st.home();
            Some((rescale, st.hwnd, st.pos, st.tpl.pad as i32))
        })
        .flatten();
        match r {
            Some((true, ..)) => paint(),
            Some((false, h, p, pad)) => move_to(h, p, pad),
            None => {}
        }
    }

    /// Keep off `avoid` (the overlay's toolbar and Record popover,
    /// physical px; none once they are gone), going outside a small area
    /// within `screen` if it must. Not while dragged.
    pub fn set_avoid(&mut self, avoid: &[IRect], screen: IRect) {
        let r = with(|st| {
            if (st.avoid.as_slice(), st.screen) == (avoid, screen) || st.drag.is_some() {
                return None;
            }
            st.avoid = avoid.to_vec();
            st.screen = screen;
            let pos = st.home();
            (pos != st.pos).then(|| {
                st.pos = pos;
                (st.hwnd, pos, st.tpl.pad as i32)
            })
        })
        .flatten();
        if let Some((h, p, pad)) = r {
            move_to(h, p, pad);
        }
    }

    /// Keep off nothing any more (the overlay is gone).
    pub fn clear_avoid(&mut self) {
        let screen = with(|st| st.screen);
        if let Some(screen) = screen {
            self.set_avoid(&[], screen);
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
                st.pos = st.home();
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
        // The camera thread posts nothing from now on.
        *lock(&self.shared.hwnd) = 0;
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
