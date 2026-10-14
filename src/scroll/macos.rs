//! macOS (compile-only for now): a scroll-wheel CGEvent posted at the
//! region's centre, which is where the window to scroll is (the pointer
//! is moved there first; it goes back at the end).

use super::Wheel;
use anyhow::{bail, Result};
use core::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateScrollWheelEvent2(source: *const c_void, units: u32, count: u32, w1: i32, w2: i32, w3: i32) -> *mut c_void;
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(e: *mut c_void) -> CGPoint;
    fn CGEventPost(tap: u32, e: *mut c_void);
    fn CGWarpMouseCursorPosition(p: CGPoint) -> i32;
    fn CFRelease(p: *const c_void);
}

/// `kCGHIDEventTap`, `kCGScrollEventUnitLine`.
const HID_TAP: u32 = 0;
const UNIT_LINE: u32 = 1;

pub fn available() -> bool {
    false // not offered until it is tested on a Mac
}

pub struct MacWheel {
    at: CGPoint,
    home: CGPoint,
}

// SAFETY: plain coordinates.
unsafe impl Send for MacWheel {}

impl MacWheel {
    /// `at` in physical px of the main display's space.
    pub fn at(at: (i32, i32)) -> Result<MacWheel> {
        if !available() {
            bail!(super::text::UNSUPPORTED);
        }
        let k = crate::capture::backing();
        let at = CGPoint { x: at.0 as f64 / k, y: at.1 as f64 / k };
        // SAFETY: a fresh event to read the pointer from, released.
        let home = unsafe {
            let e = CGEventCreate(core::ptr::null_mut());
            let p = CGEventGetLocation(e);
            CFRelease(e);
            p
        };
        Ok(MacWheel { at, home })
    }
}

impl Wheel for MacWheel {
    fn scroll(&mut self, notches: u32) -> Result<()> {
        // SAFETY: plain CoreGraphics calls; each event is released.
        unsafe {
            CGWarpMouseCursorPosition(self.at);
            let e = CGEventCreateScrollWheelEvent2(core::ptr::null(), UNIT_LINE, 1, -(3 * notches as i32), 0, 0);
            if e.is_null() {
                bail!("cannot make a scroll event");
            }
            CGEventPost(HID_TAP, e);
            CFRelease(e);
        }
        Ok(())
    }
}

impl Drop for MacWheel {
    fn drop(&mut self) {
        // SAFETY: a plain CoreGraphics call.
        unsafe {
            CGWarpMouseCursorPosition(self.home);
        }
    }
}
