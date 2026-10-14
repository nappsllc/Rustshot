//! X11: scroll the window under the region with XTest's wheel buttons
//! (4 up, 5 down; `libXtst`, loaded when used). XTest presses go to the
//! window under the pointer, so the pointer is moved to the region's
//! centre once, at the start, and each press is sent only while it is
//! still there with the same top-level window under it: the user moving
//! the pointer away (or another window coming over) ends the run, keeping
//! what was stitched, and never scrolls something else. The pointer goes
//! back where it was at the end unless the user has moved it. Rustshot's
//! own windows are never the target.
//! Wayland has no way for one app to scroll another's window: not
//! offered there.

use super::{text, Interrupted, Wheel};
use crate::rec::linux::dl::Lib;
use anyhow::{anyhow, bail, Result};
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(dpy: *mut c_void) -> c_int;
    fn XDefaultScreen(dpy: *mut c_void) -> c_int;
    fn XRootWindow(dpy: *mut c_void, screen: c_int) -> c_ulong;
    fn XQueryPointer(
        dpy: *mut c_void,
        w: c_ulong,
        root_return: *mut c_ulong,
        child_return: *mut c_ulong,
        root_x: *mut c_int,
        root_y: *mut c_int,
        win_x: *mut c_int,
        win_y: *mut c_int,
        mask_return: *mut c_uint,
    ) -> c_int;
    fn XTranslateCoordinates(
        dpy: *mut c_void,
        src: c_ulong,
        dst: c_ulong,
        x: c_int,
        y: c_int,
        dx: *mut c_int,
        dy: *mut c_int,
        child: *mut c_ulong,
    ) -> c_int;
    #[allow(clippy::too_many_arguments)]
    fn XWarpPointer(
        dpy: *mut c_void,
        src_w: c_ulong,
        dest_w: c_ulong,
        src_x: c_int,
        src_y: c_int,
        src_width: c_uint,
        src_height: c_uint,
        dest_x: c_int,
        dest_y: c_int,
    ) -> c_int;
    fn XQueryKeymap(dpy: *mut c_void, keys: *mut c_char) -> c_int;
    fn XKeysymToKeycode(dpy: *mut c_void, keysym: c_ulong) -> u8;
    fn XFlush(dpy: *mut c_void) -> c_int;
    fn XSync(dpy: *mut c_void, discard: c_int) -> c_int;
}

/// `XTestFakeButtonEvent(dpy, button, is_press, delay)`.
type FakeButton = unsafe extern "C" fn(*mut c_void, c_uint, c_int, c_ulong) -> c_int;

const XK_ESCAPE: c_ulong = 0xFF1B;
/// The wheel's "down" button.
const WHEEL_DOWN: c_uint = 5;

/// An X11 session (not Wayland) with XTest.
pub fn available() -> bool {
    let wl = std::env::var_os("WAYLAND_DISPLAY");
    let st = std::env::var("XDG_SESSION_TYPE").ok();
    crate::rec_ui::x11_session(wl.as_deref(), st.as_deref()) && std::env::var_os("DISPLAY").is_some() && xtst().is_ok()
}

fn xtst() -> Result<(Lib, FakeButton)> {
    let lib = Lib::open("libXtst.so.6")?;
    // SAFETY: XTestFakeButtonEvent's C type.
    let f = unsafe { lib.sym::<FakeButton>("XTestFakeButtonEvent")? };
    Ok((lib, f))
}

/// The display connection the wheel owns (one thread at a time).
struct Dpy(*mut c_void);

// SAFETY: opened for the wheel and used only by the thread that owns it.
unsafe impl Send for Dpy {}

impl Drop for Dpy {
    fn drop(&mut self) {
        // SAFETY: opened by us.
        unsafe {
            XCloseDisplay(self.0);
        }
    }
}

pub struct XWheel {
    dpy: Dpy,
    root: c_ulong,
    /// The top-level window under the region's centre.
    target: c_ulong,
    at: (i32, i32),
    /// Where the pointer was before.
    home: (i32, i32),
    /// The pointer was put over the region (once, at the first step).
    moved: bool,
    fake: FakeButton,
    _lib: Lib,
}

/// The top-level window (child of the root) at (x, y), and the pointer.
unsafe fn top_at(dpy: *mut c_void, root: c_ulong, x: i32, y: i32) -> c_ulong {
    let (mut dx, mut dy, mut child) = (0, 0, 0);
    unsafe { XTranslateCoordinates(dpy, root, root, x, y, &mut dx, &mut dy, &mut child) };
    child
}

unsafe fn pointer(dpy: *mut c_void, root: c_ulong) -> ((i32, i32), c_ulong) {
    let (mut r, mut child, mut rx, mut ry, mut wx, mut wy, mut m) = (0, 0, 0, 0, 0, 0, 0);
    unsafe { XQueryPointer(dpy, root, &mut r, &mut child, &mut rx, &mut ry, &mut wx, &mut wy, &mut m) };
    ((rx, ry), child)
}

#[link(name = "X11")]
unsafe extern "C" {
    fn XInternAtom(dpy: *mut c_void, name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XFree(data: *mut c_void) -> c_int;
    fn XQueryTree(dpy: *mut c_void, w: c_ulong, root: *mut c_ulong, parent: *mut c_ulong, children: *mut *mut c_ulong, n: *mut c_uint) -> c_int;
    #[allow(clippy::too_many_arguments)]
    fn XGetWindowProperty(
        dpy: *mut c_void,
        w: c_ulong,
        property: c_ulong,
        long_offset: c_long,
        long_length: c_long,
        delete: c_int,
        req_type: c_ulong,
        actual_type: *mut c_ulong,
        actual_format: *mut c_int,
        nitems: *mut c_ulong,
        bytes_after: *mut c_ulong,
        prop: *mut *mut u8,
    ) -> c_int;
}

/// `_NET_WM_PID` of `w`, or of one of its children (a window manager's
/// frame holds the client), when set.
unsafe fn pid_of(dpy: *mut c_void, w: c_ulong) -> Option<u32> {
    unsafe {
        let atom = XInternAtom(dpy, c"_NET_WM_PID".as_ptr(), 1);
        if atom == 0 {
            return None;
        }
        let read = |w: c_ulong| -> Option<u32> {
            let (mut ty, mut fmt, mut n, mut after, mut p) = (0, 0, 0, 0, core::ptr::null_mut());
            let ok = XGetWindowProperty(dpy, w, atom, 0, 1, 0, 6 /* XA_CARDINAL */, &mut ty, &mut fmt, &mut n, &mut after, &mut p) == 0;
            let v = (ok && !p.is_null() && n == 1 && fmt == 32).then(|| *(p as *const c_ulong) as u32);
            if !p.is_null() {
                XFree(p.cast());
            }
            v
        };
        if let Some(v) = read(w) {
            return Some(v);
        }
        let (mut r, mut parent, mut kids, mut n) = (0, 0, core::ptr::null_mut(), 0);
        if XQueryTree(dpy, w, &mut r, &mut parent, &mut kids, &mut n) == 0 || kids.is_null() {
            return None;
        }
        let found = std::slice::from_raw_parts(kids, n as usize).iter().find_map(|&k| read(k));
        XFree(kids.cast());
        found
    }
}

impl XWheel {
    pub fn at(at: (i32, i32)) -> Result<XWheel> {
        XWheel::find(at, false)
    }

    /// [`XWheel::at`]; `own`: one of ours will do (the live test's).
    fn find(at: (i32, i32), own: bool) -> Result<XWheel> {
        let (lib, fake) = xtst().map_err(|e| anyhow!("{}: {e:#}", text::UNSUPPORTED))?;
        crate::wind::init_x11();
        // SAFETY: a fresh connection, used by this wheel only.
        unsafe {
            let dpy = XOpenDisplay(core::ptr::null());
            if dpy.is_null() {
                bail!("cannot open the X display");
            }
            let dpy = Dpy(dpy);
            let root = XRootWindow(dpy.0, XDefaultScreen(dpy.0));
            let target = top_at(dpy.0, root, at.0, at.1);
            if target == 0 {
                bail!(text::NO_TARGET);
            }
            // Never one of Rustshot's own windows (an overlay not yet gone).
            if !own && pid_of(dpy.0, target) == Some(std::process::id()) {
                bail!(text::NO_TARGET);
            }
            let (home, _) = pointer(dpy.0, root);
            Ok(XWheel { dpy, root, target, at, home, moved: false, fake, _lib: lib })
        }
    }

    /// The pointer is over the region's centre with the target under it.
    unsafe fn still_there(&self) -> bool {
        let (p, under) = unsafe { pointer(self.dpy.0, self.root) };
        p == self.at && under == self.target
    }
}

impl Wheel for XWheel {
    fn scroll(&mut self, notches: u32) -> Result<()> {
        // SAFETY: our connection, on the owning thread.
        unsafe {
            if !self.moved {
                // Once, at the start: over the region, the same window there.
                if top_at(self.dpy.0, self.root, self.at.0, self.at.1) != self.target {
                    return Err(Interrupted(text::CHANGED).into());
                }
                XWarpPointer(self.dpy.0, 0, self.root, 0, 0, 0, 0, self.at.0, self.at.1);
                XSync(self.dpy.0, 0);
                self.moved = true;
            }
            for _ in 0..notches {
                // Checked right before each press: the user moving the
                // pointer away (or another window coming over) ends the
                // run, never scrolls something else.
                if !self.still_there() {
                    return Err(Interrupted(text::POINTER_LEFT).into());
                }
                (self.fake)(self.dpy.0, WHEEL_DOWN, 1, 0);
                (self.fake)(self.dpy.0, WHEEL_DOWN, 0, 0);
                XSync(self.dpy.0, 0);
            }
        }
        Ok(())
    }
}

impl Drop for XWheel {
    fn drop(&mut self) {
        // SAFETY: our connection. The pointer goes back where it was, unless
        // the user has moved it since.
        unsafe {
            if self.moved && pointer(self.dpy.0, self.root).0 == self.at {
                XWarpPointer(self.dpy.0, 0, self.root, 0, 0, 0, 0, self.home.0, self.home.1);
            }
            XFlush(self.dpy.0);
        }
    }
}

/// Esc held now (`XQueryKeymap`; no grab).
pub fn esc_down() -> bool {
    thread_local! {
        static D: std::cell::Cell<*mut c_void> = const { std::cell::Cell::new(core::ptr::null_mut()) };
    }
    D.with(|d| {
        // SAFETY: a connection of this thread's, kept for its lifetime.
        unsafe {
            if d.get().is_null() {
                d.set(XOpenDisplay(core::ptr::null()));
            }
            let dpy = d.get();
            if dpy.is_null() {
                return false;
            }
            let code = XKeysymToKeycode(dpy, XK_ESCAPE) as usize;
            let mut keys = [0 as c_char; 32];
            XQueryKeymap(dpy, keys.as_mut_ptr());
            code != 0 && (keys[code / 8] as u8) & (1 << (code % 8)) != 0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::{run, Ctl, End, Pace};
    use crate::rec_ui::x11 as xw;
    use std::time::{Duration, Instant};

    /// Live (X11, the Docker harness's Xvfb): a window of our own whose
    /// content scrolls on wheel button 5 is captured whole through XTest.
    /// `cargo test live_x11_scroll_capture -- --ignored --nocapture`
    #[test]
    #[ignore = "live: X11 display (Docker)"]
    fn live_x11_scroll_capture() {
        if std::env::var_os("DISPLAY").is_none() || !super::available() {
            eprintln!("SKIP: no X11 display with XTest");
            return;
        }
        let _g = crate::wind::test_window_lock();
        const CONTENT: u32 = 1200;
        let (x, y, w, h) = (200i32, 150i32, 360u32, 240u32);
        let page: Vec<u8> = (0..CONTENT)
            .flat_map(|r| {
                let band = r / 20;
                (0..w).flat_map(move |c| {
                    let mark = c < 30 + band * 37 % 280;
                    if mark { [(band * 53 % 200) as u8, (band * 97 % 220) as u8, ((r % 20) * 10) as u8, 255] } else { [255, 255, 255, 255] }
                })
            })
            .collect();
        let d = xw::Disp::open().expect("display");
        let mut win = xw::Win::with_kind(&d, true, None, xw::Kind::Overlay).expect("window");
        let n = (w * 4) as usize;
        let mut off = 0u32;
        let draw = |win: &mut xw::Win, off: u32| win.put(x, y, w, h, &page[off as usize * n..(off + h) as usize * n]);
        draw(&mut win, off);
        xw::flush(&d);
        std::thread::sleep(Duration::from_millis(300));
        let area = (x, y, w, h);
        let mut wheel = super::XWheel::find((x + w as i32 / 2, y + h as i32 / 2), true).expect("the wheel");
        let job = std::thread::spawn(move || {
            let ctl = Ctl::default();
            let mut grab = || super::super::grab_area(area);
            run(&mut grab, &mut wheel, &ctl, Pace::default(), &|| false)
        });
        let t0 = Instant::now();
        while !job.is_finished() && t0.elapsed() < Duration::from_secs(60) {
            while let Some(ev) = xw::next_event(&d) {
                // SAFETY: every event starts with its type; buttons share the pointer head.
                let (ty, p) = unsafe { (ev.type_, ev.pointer) };
                if ty == xw::EV_BUTTON_PRESS && p.button == 5 {
                    off = (off + 40).min(CONTENT - h);
                    draw(&mut win, off);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let (pic, end) = job.join().expect("the run").expect("scrolled");
        println!("X11 scrolling capture: {end:?} {}x{} in {:?}", pic.width(), pic.height(), t0.elapsed());
        assert_eq!(end, End::Bottom);
        assert_eq!(pic.height(), CONTENT, "the whole content");
        // The window shows BGRA; the capture is RGBA.
        let shown: Vec<u8> = page.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let same = pic.as_raw() == &shown;
        assert!(same, "exactly the page");
    }
}
