//! Window snapping on macOS: `CGWindowListCopyWindowInfo` (on-screen
//! windows, front to back, no desktop elements), kept by
//! [`super::mac_keep`] (normal windows, panels, popup menus and tooltips;
//! not ours, the Dock or the menu bar). `kCGWindowBounds` is in global
//! points; the shared code's pixels are points × the backing factor
//! (`capture::pts`). macOS lists no parts of a window.

use super::{mac_keep, Node, R};
use crate::capture::CGRect;
use core::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
    fn CGRectMakeWithDictionaryRepresentation(dict: *const c_void, rect: *mut CGRect) -> bool;
    static kCGWindowBounds: *const c_void;
    static kCGWindowLayer: *const c_void;
    static kCGWindowOwnerPID: *const c_void;
    static kCGWindowAlpha: *const c_void;
    static kCGWindowSharingState: *const c_void;
    static kCGWindowOwnerName: *const c_void;
}

#[allow(clashing_extern_declarations)]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(a: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(a: *const c_void, i: isize) -> *const c_void;
    fn CFDictionaryGetValue(d: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(n: *const c_void, ty: isize, out: *mut c_void) -> bool;
    fn CFRelease(cf: *const c_void);
    fn CFStringGetCString(s: *const c_void, buf: *mut u8, len: isize, encoding: u32) -> bool;
}

const ON_SCREEN_ONLY: u32 = 1 << 0;
const EXCLUDE_DESKTOP: u32 = 1 << 4;
const NUMBER_SINT64: isize = 4;
const NUMBER_DOUBLE: isize = 13;

unsafe fn int_of(d: *const c_void, key: *const c_void) -> Option<i64> {
    let n = unsafe { CFDictionaryGetValue(d, key) };
    let mut v = 0i64;
    (!n.is_null() && unsafe { CFNumberGetValue(n, NUMBER_SINT64, (&mut v as *mut i64).cast()) }).then_some(v)
}

unsafe fn float_of(d: *const c_void, key: *const c_void) -> Option<f64> {
    let n = unsafe { CFDictionaryGetValue(d, key) };
    let mut v = 0f64;
    (!n.is_null() && unsafe { CFNumberGetValue(n, NUMBER_DOUBLE, (&mut v as *mut f64).cast()) }).then_some(v)
}

/// Nothing to load ahead here.
pub fn warm() {}

/// Every window that counts, front to back (no parts); `own_pid`'s left out.
pub fn windows(own_pid: Option<u32>, within: R) -> Option<Vec<Node>> {
    // The shot clips them (`Snapshot::build`); nothing is skipped early here.
    let _ = within;
    let k = crate::capture::backing();
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0);
        if list.is_null() {
            return None;
        }
        let mut out = Vec::new();
        for i in 0..CFArrayGetCount(list) {
            let d = CFArrayGetValueAtIndex(list, i);
            if d.is_null() {
                continue;
            }
            let b = CFDictionaryGetValue(d, kCGWindowBounds);
            let mut r = CGRect::default();
            if b.is_null() || !CGRectMakeWithDictionaryRepresentation(b, &mut r) {
                continue;
            }
            let layer = int_of(d, kCGWindowLayer).unwrap_or(0);
            let alpha = float_of(d, kCGWindowAlpha).unwrap_or(1.0);
            let own = own_pid.is_some_and(|p| int_of(d, kCGWindowOwnerPID) == Some(i64::from(p)));
            if !mac_keep(layer, alpha, own, r.size.width, r.size.height) {
                continue;
            }
            let (x, y, w, h) = crate::capture::pts::rect_px((r.origin.x, r.origin.y, r.size.width, r.size.height), k);
            out.push(Node::new(R::xywh(x, y, w as i32, h as i32), Vec::new()));
        }
        CFRelease(list);
        Some(out)
    }
}

/// `kCGWindowSharingNone`: the window asked to be left out of captures
/// (`NSWindow.sharingType = .none`).
const SHARING_NONE: i64 = 0;
const UTF8: u32 = 0x0800_0100;

/// The owning app's name is Rustshot (another Rustshot process: its
/// notices are left out of captures on purpose).
unsafe fn rustshot_owner(d: *const c_void) -> bool {
    let s = unsafe { CFDictionaryGetValue(d, kCGWindowOwnerName) };
    let mut buf = [0u8; 64];
    if s.is_null() || !unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } {
        return false;
    }
    let n = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    buf[..n].eq_ignore_ascii_case(b"rustshot")
}

/// Every window that counts, front to back, those that asked not to be
/// captured (`kCGWindowSharingState` none) marked with their corner
/// radius (10 pt for a normal window, square for panels and menus); see
/// [`super::hidden_tops`].
pub fn hidden_tops(own_pid: Option<u32>, within: R) -> Option<Vec<crate::hidden::Top>> {
    let _ = within;
    let k = crate::capture::backing();
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0);
        if list.is_null() {
            return None;
        }
        let mut out = Vec::new();
        for i in 0..CFArrayGetCount(list) {
            let d = CFArrayGetValueAtIndex(list, i);
            if d.is_null() {
                continue;
            }
            let b = CFDictionaryGetValue(d, kCGWindowBounds);
            let mut r = CGRect::default();
            if b.is_null() || !CGRectMakeWithDictionaryRepresentation(b, &mut r) {
                continue;
            }
            let layer = int_of(d, kCGWindowLayer).unwrap_or(0);
            let alpha = float_of(d, kCGWindowAlpha).unwrap_or(1.0);
            if !mac_keep(layer, alpha, false, r.size.width, r.size.height) {
                continue;
            }
            let (x, y, w, h) = crate::capture::pts::rect_px((r.origin.x, r.origin.y, r.size.width, r.size.height), k);
            let rect = R::xywh(x, y, w as i32, h as i32);
            let hidden = int_of(d, kCGWindowSharingState) == Some(SHARING_NONE);
            if hidden {
                let own = own_pid.is_some_and(|p| int_of(d, kCGWindowOwnerPID) == Some(i64::from(p)));
                if own || rustshot_owner(d) {
                    continue;
                }
            }
            let radius = if layer == 0 { 10.0 * k as f32 } else { 0.0 };
            out.push(crate::hidden::Top { r: rect, hidden: hidden.then_some(radius) });
        }
        CFRelease(list);
        Some(out)
    }
}
