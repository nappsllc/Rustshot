//! The mouse pointer at capture time (Settings › Capture "Include mouse
//! pointer"): its picture with straight alpha and where its top-left is
//! (global physical px, the hotspot taken off). Added to the capture as
//! an object ([`crate::objects::Obj::Cursor`]), never into the pixels.
//!
//! - Windows: `GetCursorInfo` + `GetIconInfo` (the hotspot), drawn with
//!   `DrawIconEx` over black and over white (the alpha is what differs).
//! - Linux (X11): `XFixesGetCursorImage` (libXfixes, loaded when there;
//!   none on Wayland, where the portal's own picture is all there is).
//! - macOS: none yet (compile only).

use crate::pixbuf::PixBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Pointer {
    pub img: PixBuf,
    /// The picture's top-left, global physical px.
    pub at: (i32, i32),
}

/// The largest pointer picture taken (px a side).
#[cfg_attr(test, allow(dead_code))]
pub const MAX_SIDE: u32 = 256;

/// The picture from one drawn over black and one over white (`BGRA`
/// rows, alpha ignored): straight-alpha RGBA. A pixel that inverts what
/// is under it (the I-beam) comes out opaque.
pub fn from_black_white(w: u32, h: u32, black: &[u8], white: &[u8]) -> Option<PixBuf> {
    let n = (w as usize).checked_mul(h as usize)?;
    if black.len() < n * 4 || white.len() < n * 4 || w == 0 || h == 0 {
        return None;
    }
    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let (b, wt) = (&black[i * 4..i * 4 + 3], &white[i * 4..i * 4 + 3]);
        // alpha = 1 - (white - black), per channel, averaged
        let diff: i32 = (0..3).map(|c| (wt[c] as i32 - b[c] as i32).max(0)).sum::<i32>() / 3;
        let a = (255 - diff).clamp(0, 255);
        let o = &mut out[i * 4..i * 4 + 4];
        if a == 0 {
            continue;
        }
        for (c, &ch) in [2usize, 1, 0].iter().enumerate() {
            o[c] = ((b[ch] as i32 * 255 + a / 2) / a).clamp(0, 255) as u8;
        }
        o[3] = a as u8;
    }
    PixBuf::from_raw(w, h, out)
}

/// The pointer now, if it is showing.
pub fn grab() -> Option<Pointer> {
    #[cfg(test)]
    return FAKE.with(|f| f.borrow().clone());
    #[cfg(not(test))]
    imp::grab()
}

#[cfg(test)]
thread_local! {
    pub static FAKE: std::cell::RefCell<Option<Pointer>> = const { std::cell::RefCell::new(None) };
}

#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
mod imp {
    use super::*;
    use std::mem::size_of;
    use windows::Win32::Graphics::Gdi::{
        BITMAP, BITMAPINFO, BI_RGB, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetObjectW, HGDIOBJ, SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, HICON, ICONINFO};

    pub fn grab() -> Option<Pointer> {
        unsafe {
            let mut ci = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
            if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 || ci.hCursor.is_invalid() {
                return None;
            }
            let icon = HICON(ci.hCursor.0);
            let mut ii = ICONINFO::default();
            GetIconInfo(icon, &mut ii).ok()?;
            // The size: the color bitmap's, else half the mask's height
            // (a monochrome cursor stacks its AND and XOR masks).
            let mut bm = BITMAP::default();
            let (mut w, mut h) = (32u32, 32u32);
            let src = if !ii.hbmColor.is_invalid() { ii.hbmColor } else { ii.hbmMask };
            if !src.is_invalid() && GetObjectW(HGDIOBJ(src.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) != 0 {
                w = bm.bmWidth.max(1) as u32;
                h = bm.bmHeight.max(1) as u32;
                if ii.hbmColor.is_invalid() {
                    h = (h / 2).max(1);
                }
            }
            for b in [ii.hbmMask, ii.hbmColor] {
                if !b.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(b.0));
                }
            }
            let (w, h) = (w.min(MAX_SIDE), h.min(MAX_SIDE));
            let draw = |fill: u8| -> Option<Vec<u8>> {
                let dc = CreateCompatibleDC(None);
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w as i32;
                bmi.bmiHeader.biHeight = -(h as i32);
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB.0;
                let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
                let Ok(dib) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                    let _ = DeleteDC(dc);
                    return None;
                };
                let n = (w * h * 4) as usize;
                let buf = std::slice::from_raw_parts_mut(bits as *mut u8, n);
                buf.fill(fill);
                let old = SelectObject(dc, HGDIOBJ(dib.0));
                let ok = DrawIconEx(dc, 0, 0, icon, 0, 0, 0, None, DI_NORMAL).is_ok();
                let _ = GdiFlush();
                let out = ok.then(|| buf.to_vec());
                let _ = SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(dib.0));
                let _ = DeleteDC(dc);
                out
            };
            let (black, white) = (draw(0)?, draw(255)?);
            let img = from_black_white(w, h, &black, &white)?;
            let at = (ci.ptScreenPos.x - ii.xHotspot as i32, ci.ptScreenPos.y - ii.yHotspot as i32);
            Some(Pointer { img, at })
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
mod imp {
    use super::*;
    use core::ffi::{c_char, c_int, c_short, c_ulong, c_ushort, c_void};

    #[repr(C)]
    struct XFixesCursorImage {
        x: c_short,
        y: c_short,
        width: c_ushort,
        height: c_ushort,
        xhot: c_ushort,
        yhot: c_ushort,
        cursor_serial: c_ulong,
        pixels: *mut c_ulong,
        atom: c_ulong,
        name: *const c_char,
    }

    #[allow(clashing_extern_declarations)]
    #[link(name = "X11")]
    unsafe extern "C" {
        fn XOpenDisplay(name: *const c_char) -> *mut c_void;
        fn XCloseDisplay(dpy: *mut c_void) -> c_int;
        fn XFree(data: *mut c_void) -> c_int;
    }

    type GetImage = unsafe extern "C" fn(*mut c_void) -> *mut XFixesCursorImage;

    pub fn grab() -> Option<Pointer> {
        if crate::rec::linux::portal::wayland_now() {
            return None;
        }
        static LIB: std::sync::OnceLock<Option<crate::rec::linux::dl::Lib>> = std::sync::OnceLock::new();
        let lib = LIB.get_or_init(|| crate::rec::linux::dl::Lib::open("libXfixes.so.3").ok()).as_ref()?;
        // SAFETY: libXfixes' C signature.
        let get: GetImage = unsafe { lib.sym("XFixesGetCursorImage") }.ok()?;
        crate::wind::init_x11();
        unsafe {
            let dpy = XOpenDisplay(core::ptr::null());
            if dpy.is_null() {
                return None;
            }
            let ci = get(dpy);
            let out = if ci.is_null() {
                None
            } else {
                let c = &*ci;
                let (w, h) = (c.width as u32, c.height as u32);
                let mut px = Vec::with_capacity((w * h * 4) as usize);
                if w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE && !c.pixels.is_null() {
                    // Premultiplied ARGB in the low 32 bits of each long.
                    for i in 0..(w * h) as usize {
                        let v = *c.pixels.add(i) as u32;
                        let a = v >> 24;
                        let un = |ch: u32| if a == 0 { 0 } else { ((ch * 255 + a / 2) / a).min(255) as u8 };
                        px.extend_from_slice(&[un((v >> 16) & 255), un((v >> 8) & 255), un(v & 255), a as u8]);
                    }
                }
                let at = (c.x as i32 - c.xhot as i32, c.y as i32 - c.yhot as i32);
                let img = PixBuf::from_raw(w, h, px);
                XFree(ci as *mut c_void);
                img.map(|img| Pointer { img, at })
            };
            XCloseDisplay(dpy);
            out
        }
    }
}

#[cfg(all(not(any(windows, target_os = "linux")), not(test)))]
mod imp {
    use super::*;

    /// macOS: not yet (NSCursor.currentSystemCursor needs the main thread).
    pub fn grab() -> Option<Pointer> {
        None
    }
}

/// The pointer as the object a capture at `origin` (global physical px)
/// gets.
pub fn object(origin: (i32, i32)) -> Option<crate::objects::Obj> {
    let p = grab()?;
    let at = crate::objects::Pt::new((p.at.0 - origin.0) as f32, (p.at.1 - origin.1) as f32);
    Some(crate::objects::Obj::Cursor { at, img: std::sync::Arc::new(p.img) })
}

/// A capture with no editing step (full screen, `--raw`): the pointer
/// drawn into it.
pub fn bake_into(img: &mut PixBuf, origin: (i32, i32)) {
    if let Some(o) = object(origin) {
        o.render(img, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real pointer (Windows): a picture with opaque pixels, its
    /// top-left within a picture's size of where the pointer is.
    #[cfg(windows)]
    #[test]
    #[ignore = "live display access"]
    fn live_pointer_grab() {
        let p = super::imp::grab().expect("a pointer showing");
        let (w, h) = p.img.dimensions();
        assert!((8..=MAX_SIDE).contains(&w) && (8..=MAX_SIDE).contains(&h), "{w}x{h}");
        assert!(p.img.as_raw().as_chunks::<4>().0.iter().any(|q| q[3] == 255), "opaque pixels");
        let (cx, cy) = crate::capture::cursor_pos();
        assert!((cx - p.at.0).abs() <= w as i32 && (cy - p.at.1).abs() <= h as i32, "{:?} vs {:?}", p.at, (cx, cy));
        eprintln!("pointer {w}x{h} at {:?}", p.at);
    }

    #[test]
    fn alpha_from_black_and_white() {
        // BGRA: opaque red, half-transparent white, transparent, an inverting pixel
        let black = [0, 0, 255, 0, 128, 128, 128, 0, 0, 0, 0, 0, 255, 255, 255, 0];
        let white = [0, 0, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 0, 0, 0, 0];
        let p = from_black_white(4, 1, &black, &white).unwrap();
        assert_eq!(p.get_pixel(0, 0), [255, 0, 0, 255]);
        let q = p.get_pixel(1, 0);
        assert!(q[3].abs_diff(128) <= 1 && q[0] >= 250, "{q:?}");
        assert_eq!(p.get_pixel(2, 0)[3], 0);
        assert_eq!(p.get_pixel(3, 0)[3], 255, "inverting: opaque");
        assert!(from_black_white(0, 1, &[], &[]).is_none());
        assert!(from_black_white(3, 2, &black, &white).is_none(), "short buffers");
    }
}
