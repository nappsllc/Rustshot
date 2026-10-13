//! X11: scroll the window under the region with XTest's wheel buttons
//! (4 up, 5 down; `libXtst`, loaded when used). XTest presses go to the
//! window under the pointer, so the pointer is moved to the region's
//! centre first, and each press is sent only while the top-level window
//! under the pointer is still the one that was there at the start (the
//! user moving the pointer elsewhere stops the run instead of scrolling
//! something else). The pointer goes back where it was at the end.
//! Wayland has no way for one app to scroll another's window: not
//! offered there.

use super::{text, Wheel};
use crate::rec::linux::dl::Lib;
use anyhow::{anyhow, bail, Result};
use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

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

impl XWheel {
    pub fn at(at: (i32, i32)) -> Result<XWheel> {
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
            let (home, _) = pointer(dpy.0, root);
            Ok(XWheel { dpy, root, target, at, home, fake, _lib: lib })
        }
    }
}

impl Wheel for XWheel {
    fn scroll(&mut self, notches: u32) -> Result<()> {
        // SAFETY: our connection, on the owning thread.
        unsafe {
            let (p, under) = pointer(self.dpy.0, self.root);
            if p != self.at {
                // First step (or the user moved it): over the region again,
                // only while the same window is there.
                if top_at(self.dpy.0, self.root, self.at.0, self.at.1) != self.target {
                    bail!("the window under the selection changed");
                }
                XWarpPointer(self.dpy.0, 0, self.root, 0, 0, 0, 0, self.at.0, self.at.1);
            } else if under != self.target {
                bail!("the window under the selection changed");
            }
            for _ in 0..notches {
                (self.fake)(self.dpy.0, WHEEL_DOWN, 1, 0);
                (self.fake)(self.dpy.0, WHEEL_DOWN, 0, 0);
            }
            XFlush(self.dpy.0);
        }
        Ok(())
    }
}

impl Drop for XWheel {
    fn drop(&mut self) {
        // SAFETY: our connection; the pointer goes back where it was.
        unsafe {
            XWarpPointer(self.dpy.0, 0, self.root, 0, 0, 0, 0, self.home.0, self.home.1);
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
