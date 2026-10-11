//! macOS capture: Quartz display enumeration, `CGWindowListCreateImage`
//! desktop snapshots and the global cursor position (replaces xcap/GDI).
//!
//! Units: Quartz works in points (origin at the top-left of the main
//! display, y down). Everything this module hands to the shared code —
//! [`monitors`], [`cursor_pos`], the rects [`gdi_capture`] takes — is in
//! pixels: points × [`backing`] (see [`super::pts`]), so a Retina screen
//! is captured and edited at its full density. `wind_macos`, the recorder
//! and the camera bubble convert back to points with [`to_points`] /
//! [`to_points_i`] / [`from_points`] where they call AppKit.

use super::*;

use core::ffi::c_void;
use std::sync::atomic::Ordering;

// --- CoreGraphics geometry ------------------------------------------------

/// Quartz point (window/server coordinates, y down).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

/// Quartz size (points).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

/// Quartz rectangle in points (origin + size).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

// --- FFI ------------------------------------------------------------------

// `cargo check` compiles this per cross-target without linking; the symbols
// resolve on a real mac against the named system frameworks.
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(max: u32, ids: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(id: u32) -> CGRect;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayCopyDisplayMode(id: u32) -> *mut c_void;
    fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeGetWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
    // Deprecated in macOS 14 (ScreenCaptureKit replaces it) but still the
    // only FFI-only desktop snapshot API; never compiled, never warned here.
    fn CGWindowListCreateImage(
        rect: CGRect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> *mut c_void;
    fn CGImageGetWidth(image: *mut c_void) -> usize;
    fn CGImageGetHeight(image: *mut c_void) -> usize;
    fn CGImageGetBitsPerPixel(image: *mut c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *mut c_void) -> usize;
    fn CGImageGetBitmapInfo(image: *mut c_void) -> u32;
    fn CGImageGetDataProvider(image: *mut c_void) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
    fn CGDataProviderCopyData(provider: *mut c_void) -> *mut c_void;
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFDataGetLength(data: *const c_void) -> i64;
}

// --- public API -----------------------------------------------------------

/// No per-process DPI switch exists on macOS: Quartz reports points
/// everywhere; this module scales them to pixels itself.
pub fn enable_dpi_awareness() {}

/// Pixels per point of display `id` (`CGDisplayModeGetPixelWidth /
/// GetWidth`: 2 on Retina, also in scaled modes; 1 when unknown).
/// `CGDisplayPixelsWide` would not do: it reports points in HiDPI modes.
pub fn display_scale(id: u32) -> f64 {
    unsafe {
        let m = CGDisplayCopyDisplayMode(id);
        if m.is_null() {
            return 1.0;
        }
        let s = super::pts::mode_scale(CGDisplayModeGetPixelWidth(m), CGDisplayModeGetWidth(m));
        CGDisplayModeRelease(m);
        s
    }
}

/// The active displays and their bounds (points).
fn displays() -> Vec<(u32, CGRect)> {
    unsafe {
        let mut ids = [0u32; 32];
        let mut count = 0u32;
        if CGGetActiveDisplayList(32, ids.as_mut_ptr(), &mut count) != 0 {
            return Vec::new();
        }
        ids[..(count as usize).min(32)].iter().map(|&id| (id, CGDisplayBounds(id))).collect()
    }
}

/// Pixels per point of the shared code's coordinates: the largest scale
/// of the active displays (see [`super::pts`]).
pub fn backing() -> f64 {
    let scales: Vec<f64> = displays().iter().map(|d| display_scale(d.0)).collect();
    super::pts::factor(&scales)
}

/// A rect of the shared code (pixels) in global points.
pub fn to_points(r: IRect) -> (f64, f64, f64, f64) {
    super::pts::rect_pt(r, backing())
}

/// [`to_points`] in whole points.
pub fn to_points_i(r: IRect) -> IRect {
    super::pts::rect_pt_i(r, backing())
}

/// A rect in global points as the shared code's pixels.
pub fn from_points(r: (f64, f64, f64, f64)) -> IRect {
    super::pts::rect_px(r, backing())
}

/// Active displays via `CGGetActiveDisplayList`, in pixels (points ×
/// [`backing`]; every monitor's `scale` is that factor, the density its
/// shot is drawn at), sorted by (x, y) so `screenN` numbering in the CLI
/// stays stable across calls.
pub fn monitors() -> Result<Vec<MonInfo>> {
    let ds = displays();
    if ds.is_empty() {
        return Err(anyhow!("no monitors found"));
    }
    let k = super::pts::factor(&ds.iter().map(|d| display_scale(d.0)).collect::<Vec<_>>());
    let primary = unsafe { CGMainDisplayID() };
    let mut out: Vec<MonInfo> = ds
        .iter()
        .map(|&(id, b)| {
            let (x, y, w, h) = super::pts::rect_px((b.origin.x, b.origin.y, b.size.width, b.size.height), k);
            MonInfo { x, y, w, h, scale: k as f32, primary: id == primary }
        })
        .collect();
    out.sort_by_key(|m| (m.x, m.y));
    Ok(out)
}

/// Capture a virtual-screen rect (pixels, see the module docs) with
/// `CGWindowListCreateImage`. Always returns a `w x h` buffer: the
/// snapshot comes back at each display's native resolution and is
/// resampled to the request when that differs (a 1× display beside a
/// Retina one).
pub fn gdi_capture(x: i32, y: i32, w: u32, h: u32) -> Result<PixBuf> {
    if w == 0 || h == 0 {
        return Err(anyhow!("empty capture region"));
    }
    let (px, py, pw, ph) = to_points((x, y, w, h));
    unsafe {
        let rect = CGRect { origin: CGPoint { x: px, y: py }, size: CGSize { width: pw, height: ph } };
        let image = CGWindowListCreateImage(
            rect,
            1, // kCGWindowListOptionOnScreenOnly
            0, // kCGNullWindowID
            0, // kCGWindowImageDefault
        );
        if image.is_null() {
            return Err(anyhow!("screen capture failed"));
        }
        let out = decode(image, w, h);
        CGImageRelease(image);
        out
    }
}

/// Global cursor position in pixels (`CGEventCreate` never fails in
/// practice; fall back to the main display origin if it somehow does).
pub fn cursor_pos() -> (i32, i32) {
    unsafe {
        let ev = CGEventCreate(core::ptr::null_mut());
        if ev.is_null() {
            return (0, 0);
        }
        let p = CGEventGetLocation(ev);
        CFRelease(ev);
        let k = backing();
        (super::pts::px(p.x, k), super::pts::px(p.y, k))
    }
}

/// Height of the main display in points — the flip factor Cocoa needs to
/// map Quartz top-down rects to `NSWindow` bottom-up frames (used by
/// `wind_macos`, which would otherwise have to re-declare CGDisplayBounds).
pub fn main_display_height() -> f64 {
    unsafe {
        let b = CGDisplayBounds(CGMainDisplayID());
        b.size.height
    }
}

/// Activate the process and raise our overlay so it receives key events.
/// Only ever called from driver callbacks on the pump thread (inside the
/// loop's autorelease pool); a zero `MAIN_WINDOW` means `run` has exited.
pub fn focus_our_window() {
    use crate::wind::{msg0, msg1, objc_cls, objc_sel};

    unsafe {
        if crate::wind::MAIN_WINDOW.load(Ordering::SeqCst) == 0 {
            return;
        }
        let cls = objc_cls(c"NSApplication");
        if cls.is_null() {
            return;
        }
        let app: *mut c_void = msg0(cls, objc_sel(c"sharedApplication"));
        if app.is_null() {
            return;
        }
        let _: () = msg1(app, objc_sel(c"activateIgnoringOtherApps:"), 1i64);
        let _: () = msg1(
            app,
            objc_sel(c"makeKeyAndOrderFront:"),
            core::ptr::null_mut::<c_void>(),
        );
    }
}

// --- image decode ---------------------------------------------------------

/// Read a `CGImage` into a `w x h` RGBA8 buffer (alpha forced opaque).
unsafe fn decode(image: *mut c_void, w: u32, h: u32) -> Result<PixBuf> {
    unsafe {
        let (iw, ih) = (CGImageGetWidth(image), CGImageGetHeight(image));
        if iw == 0 || ih == 0 {
            return Err(anyhow!("capture produced an empty image"));
        }
        if CGImageGetBitsPerPixel(image) != 32 {
            return Err(anyhow!("unsupported capture format"));
        }
        let bpr = CGImageGetBytesPerRow(image);
        let data = CGDataProviderCopyData(CGImageGetDataProvider(image));
        if data.is_null() {
            return Err(anyhow!("capture pixel data unavailable"));
        }
        let len = CFDataGetLength(data) as usize;
        let src = CFDataGetBytePtr(data);
        let out = if src.is_null() || bpr < iw * 4 || len < bpr.saturating_mul(ih) {
            Err(anyhow!("capture pixel buffer too small"))
        } else {
            convert(src, bpr, iw, ih, w, h, channel_map(CGImageGetBitmapInfo(image)))
        };
        CFRelease(data);
        out
    }
}

/// 32bpp memory layout of a `CGImage`: offsets of R, G, B in each pixel.
/// (`kCGImageAlphaInfoMask` = 0x1F, `kCGBitmapByteOrder32Big` = 0x3000;
/// Apple QA1708 pins the four host-order combinations.)
fn channel_map(info: u32) -> [usize; 3] {
    let alpha_first = matches!(info & 0x1F, 2 | 4 | 6);
    let big = info & 0x3000 == 0x3000;
    match (alpha_first, big) {
        (true, false) => [2, 1, 0],  // B,G,R,A (what CGWindowList hands back)
        (true, true) => [1, 2, 3],   // A,R,G,B
        (false, true) => [0, 1, 2],  // R,G,B,A
        (false, false) => [3, 2, 1], // A,B,G,R
    }
}

/// Convert `src` (stride `bpr`, `iw x ih`, 32 bpp) to a `w x h` PixBuf.
/// An exact integer ratio becomes a box average (retina 2x → points 1x,
/// keeping edges clean); anything else falls back to nearest-neighbour.
fn convert(
    src: *const u8,
    bpr: usize,
    iw: usize,
    ih: usize,
    w: u32,
    h: u32,
    map: [usize; 3],
) -> Result<PixBuf> {
    let (w, h) = (w as usize, h as usize);
    let mut buf = vec![0u8; w * h * 4];
    let (bx, by) = if iw.is_multiple_of(w) && ih.is_multiple_of(h) && w > 0 && h > 0 {
        (iw / w, ih / h)
    } else {
        (1, 1)
    };
    for dy in 0..h {
        for dx in 0..w {
            let (x0, y0) = if bx > 1 || by > 1 {
                (dx * bx, dy * by)
            } else {
                (dx * iw / w, dy * ih / h)
            };
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for oy in 0..by {
                let row = unsafe { src.add((y0 + oy) * bpr) };
                for ox in 0..bx {
                    let px = unsafe { row.add((x0 + ox) * 4) };
                    r += unsafe { *px.add(map[0]) } as u32;
                    g += unsafe { *px.add(map[1]) } as u32;
                    b += unsafe { *px.add(map[2]) } as u32;
                }
            }
            let n = (bx * by) as u32;
            let i = (dy * w + dx) * 4;
            buf[i] = (r / n) as u8;
            buf[i + 1] = (g / n) as u8;
            buf[i + 2] = (b / n) as u8;
            buf[i + 3] = 255; // screen content is opaque; premultiplied A is noise
        }
    }
    PixBuf::from_raw(w as u32, h as u32, buf).ok_or_else(|| anyhow!("bad capture buffer"))
}
