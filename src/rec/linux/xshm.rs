//! X11 frames: `XShmGetImage` of the recorded area into a shared-memory
//! image (plain `XGetImage` when the server has no MIT-SHM, e.g. over
//! ssh, or once a shared-memory grab raised an X error), with the pointer
//! drawn in from `XFixesGetCursorImage`. X errors never end the process
//! (see `guarded`); a grab that fails keeps the last frame.
//!
//! The source has its own Display connection (Xlib is thread-initialised
//! by `wind::init_x11`); it is opened on the recording worker and used on
//! the capture thread, one at a time.

use crate::capture::IRect;
use crate::rec::{Frame, FrameSource};
use anyhow::{anyhow, bail, Result};
use core::ffi::{c_char, c_int, c_short, c_uint, c_ulong, c_ushort, c_void};
use std::time::{Duration, Instant};

/// The head of XImage (ABI-stable), as in capture_linux.
#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut u8,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
}

#[repr(C)]
struct XShmSegmentInfo {
    shmseg: c_ulong,
    shmid: c_int,
    shmaddr: *mut c_char,
    read_only: c_int,
}

#[repr(C)]
struct XFixesCursorImage {
    x: c_short,
    y: c_short,
    width: c_ushort,
    height: c_ushort,
    xhot: c_ushort,
    yhot: c_ushort,
    cursor_serial: c_ulong,
    /// Premultiplied ARGB, one per `unsigned long`.
    pixels: *mut c_ulong,
}

#[allow(clashing_extern_declarations)]
#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(dpy: *mut c_void) -> c_int;
    fn XDefaultScreen(dpy: *mut c_void) -> c_int;
    fn XRootWindow(dpy: *mut c_void, screen: c_int) -> c_ulong;
    fn XDefaultVisual(dpy: *mut c_void, screen: c_int) -> *mut c_void;
    fn XDefaultDepth(dpy: *mut c_void, screen: c_int) -> c_int;
    fn XDisplayWidth(dpy: *mut c_void, screen: c_int) -> c_int;
    fn XDisplayHeight(dpy: *mut c_void, screen: c_int) -> c_int;
    fn XGetImage(
        dpy: *mut c_void,
        drawable: c_ulong,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        plane_mask: c_ulong,
        format: c_int,
    ) -> *mut XImage;
    fn XDestroyImage(image: *mut XImage) -> c_int;
    fn XFree(data: *mut c_void) -> c_int;
}

// X errors. Xlib's default handler exits the process, and a grab can fail
// at any time (the screen shrank under the area, the server refused the
// shared segment), so every request here runs in `guarded`: the shared
// recording handler of `wind::x_guarded` (also used by the hotkey grabs),
// serialised process-wide.
unsafe fn guarded<T>(dpy: *mut c_void, f: impl FnOnce() -> T) -> (T, bool) {
    // SAFETY: `dpy` is this source's own Display, used by one thread at a time.
    unsafe { crate::wind::x_guarded(dpy, f) }
}

#[link(name = "Xext")]
unsafe extern "C" {
    fn XShmQueryExtension(dpy: *mut c_void) -> c_int;
    fn XShmCreateImage(
        dpy: *mut c_void,
        visual: *mut c_void,
        depth: c_uint,
        format: c_int,
        data: *mut c_char,
        shminfo: *mut XShmSegmentInfo,
        width: c_uint,
        height: c_uint,
    ) -> *mut XImage;
    fn XShmAttach(dpy: *mut c_void, shminfo: *mut XShmSegmentInfo) -> c_int;
    fn XShmDetach(dpy: *mut c_void, shminfo: *mut XShmSegmentInfo) -> c_int;
    fn XShmGetImage(dpy: *mut c_void, d: c_ulong, image: *mut XImage, x: c_int, y: c_int, plane_mask: c_ulong) -> c_int;
}

#[link(name = "Xfixes")]
unsafe extern "C" {
    fn XFixesQueryExtension(dpy: *mut c_void, event_base: *mut c_int, error_base: *mut c_int) -> c_int;
    fn XFixesGetCursorImage(dpy: *mut c_void) -> *mut XFixesCursorImage;
}

// System V shared memory (libc).
unsafe extern "C" {
    fn shmget(key: c_int, size: usize, flags: c_int) -> c_int;
    fn shmat(id: c_int, addr: *const c_void, flags: c_int) -> *mut c_void;
    fn shmdt(addr: *const c_void) -> c_int;
    fn shmctl(id: c_int, cmd: c_int, buf: *mut c_void) -> c_int;
}

const Z_PIXMAP: c_int = 2;
const ALL_PLANES: c_ulong = !0;
const IPC_PRIVATE: c_int = 0;
const IPC_CREAT: c_int = 0o1000;
const IPC_RMID: c_int = 0;
const LSB_FIRST: c_int = 0;

/// The shared-memory image (MIT-SHM).
struct Shm {
    img: *mut XImage,
    info: Box<XShmSegmentInfo>,
}

/// Frames of an area of the X screen.
pub struct XShmSource {
    dpy: *mut c_void,
    root: c_ulong,
    area: IRect,
    shm: Option<Shm>,
    cursor: bool,
    /// Said that grabs fail.
    warned: bool,
    /// The last frame (returned again when a grab fails).
    last: Option<Frame>,
}

// SAFETY: the Display and the image are used by one thread at a time (the
// worker opens the source, then the capture thread owns it); Xlib was
// initialised for threads.
unsafe impl Send for XShmSource {}

/// `area` clipped to a `sw`×`sh` screen; `None` when nothing is left.
pub fn clip_area(area: IRect, sw: i32, sh: i32) -> Option<IRect> {
    let (x0, y0) = (area.0.max(0), area.1.max(0));
    let (x1, y1) = ((area.0 + area.2 as i32).min(sw), (area.1 + area.3 as i32).min(sh));
    (x1 > x0 && y1 > y0).then(|| (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
}

impl XShmSource {
    /// Frames of `area` (root-window pixels; clipped to the screen).
    pub fn open(area: IRect) -> Result<XShmSource> {
        crate::wind::init_x11();
        unsafe {
            let dpy = XOpenDisplay(core::ptr::null());
            if dpy.is_null() {
                bail!("cannot open the X display");
            }
            let screen = XDefaultScreen(dpy);
            let root = XRootWindow(dpy, screen);
            let (sw, sh) = (XDisplayWidth(dpy, screen), XDisplayHeight(dpy, screen));
            let Some(area) = clip_area(area, sw, sh) else {
                XCloseDisplay(dpy);
                bail!("the recording area is off the screen");
            };
            let depth = XDefaultDepth(dpy, screen);
            if depth != 24 && depth != 32 {
                XCloseDisplay(dpy);
                bail!("the X screen has depth {depth}; recording needs 24 or 32");
            }
            let (mut eb, mut er) = (0, 0);
            let cursor = XFixesQueryExtension(dpy, &mut eb, &mut er) != 0;
            let mut src = XShmSource { dpy, root, area, shm: None, cursor, warned: false, last: None };
            if XShmQueryExtension(dpy) != 0 {
                match src.attach(XDefaultVisual(dpy, screen), depth as c_uint) {
                    Ok(s) => src.shm = Some(s),
                    Err(e) => eprintln!("rustshot: MIT-SHM unavailable ({e:#}); recording with XGetImage"),
                }
            }
            if !cursor {
                eprintln!("rustshot: XFixes unavailable; the pointer is not recorded");
            }
            Ok(src)
        }
    }

    unsafe fn attach(&self, visual: *mut c_void, depth: c_uint) -> Result<Shm> {
        unsafe {
            let mut info = Box::new(XShmSegmentInfo { shmseg: 0, shmid: -1, shmaddr: core::ptr::null_mut(), read_only: 0 });
            let (w, h) = (self.area.2, self.area.3);
            let img = XShmCreateImage(self.dpy, visual, depth, Z_PIXMAP, core::ptr::null_mut(), &mut *info, w, h);
            if img.is_null() {
                bail!("XShmCreateImage failed");
            }
            let fail = |img: *mut XImage, msg: &str| {
                XDestroyImage(img);
                anyhow!("{msg}")
            };
            if (*img).bits_per_pixel != 32 || (*img).byte_order != LSB_FIRST {
                return Err(fail(img, "the screen is not 32-bit little-endian"));
            }
            let size = (*img).bytes_per_line as usize * (*img).height as usize;
            info.shmid = shmget(IPC_PRIVATE, size, IPC_CREAT | 0o600);
            if info.shmid < 0 {
                return Err(fail(img, "shmget failed"));
            }
            let addr = shmat(info.shmid, core::ptr::null(), 0);
            if addr as isize == -1 {
                shmctl(info.shmid, IPC_RMID, core::ptr::null_mut());
                return Err(fail(img, "shmat failed"));
            }
            info.shmaddr = addr as *mut c_char;
            (*img).data = addr as *mut u8;
            // Refused asynchronously (BadAccess: e.g. a server in another
            // container cannot map our segment): an X error, not a result.
            let (ok, err) = guarded(self.dpy, || XShmAttach(self.dpy, &mut *info) != 0);
            let attached = ok && !err;
            // Removed now: it goes away once both sides detach (or exit).
            shmctl(info.shmid, IPC_RMID, core::ptr::null_mut());
            if !attached {
                (*img).data = core::ptr::null_mut();
                XDestroyImage(img);
                shmdt(addr);
                bail!("XShmAttach failed");
            }
            Ok(Shm { img, info })
        }
    }

    /// Grab the area now (BGRA, alpha 255). An X error on the shared-memory
    /// grab drops MIT-SHM for plain `XGetImage`; when that fails too (the
    /// area left the screen), `None`: the last frame stays.
    fn grab(&mut self) -> Option<Vec<u8>> {
        if self.shm.is_some() {
            let (out, err) = unsafe { guarded(self.dpy, || self.grab_with_shm()) };
            if !err {
                return out;
            }
            eprintln!("rustshot: XShmGetImage failed; recording with XGetImage");
            self.drop_shm();
        }
        let (out, err) = unsafe { guarded(self.dpy, || self.grab_plain()) };
        if err && !self.warned {
            self.warned = true;
            eprintln!("rustshot: XGetImage failed (the area left the screen?); the picture stays still");
        }
        out.filter(|_| !err)
    }

    fn grab_with_shm(&self) -> Option<Vec<u8>> {
        let (x, y, ..) = self.area;
        let s = self.shm.as_ref()?;
        unsafe {
            if XShmGetImage(self.dpy, self.root, s.img, x, y, ALL_PLANES) == 0 {
                return None;
            }
            self.copy_out(s.img, false)
        }
    }

    fn grab_plain(&self) -> Option<Vec<u8>> {
        let (x, y, w, h) = self.area;
        unsafe {
            let i = XGetImage(self.dpy, self.root, x, y, w, h, ALL_PLANES, Z_PIXMAP);
            if i.is_null() {
                return None;
            }
            self.copy_out(i, true)
        }
    }

    fn drop_shm(&mut self) {
        if let Some(mut s) = self.shm.take() {
            unsafe {
                // The detach may fail (the segment is gone): guarded. XShm
                // images free only the struct, not the shared data.
                guarded(self.dpy, || XShmDetach(self.dpy, &mut *s.info));
                XDestroyImage(s.img);
                shmdt(s.info.shmaddr as *const c_void);
            }
        }
    }

    /// The area's pixels out of `img` (destroyed when `owned`).
    unsafe fn copy_out(&self, img: *mut XImage, owned: bool) -> Option<Vec<u8>> {
        let (w, h) = (self.area.2, self.area.3);
        unsafe {
            let ok = (*img).bits_per_pixel == 32 && (*img).byte_order == LSB_FIRST && !(*img).data.is_null();
            let out = ok.then(|| {
                let stride = (*img).bytes_per_line as usize;
                let row = w as usize * 4;
                let data = core::slice::from_raw_parts((*img).data, stride * h as usize);
                let mut out = Vec::with_capacity(row * h as usize);
                for r in 0..h as usize {
                    out.extend_from_slice(&data[r * stride..r * stride + row]);
                }
                for p in out.as_chunks_mut::<4>().0 {
                    p[3] = 255;
                }
                out
            });
            if owned {
                XDestroyImage(img);
            }
            out
        }
    }

    /// Draw the pointer into `bgra`.
    fn draw_cursor(&self, bgra: &mut [u8]) {
        if !self.cursor {
            return;
        }
        unsafe {
            let c = XFixesGetCursorImage(self.dpy);
            if c.is_null() {
                return;
            }
            let (cw, ch) = ((*c).width as usize, (*c).height as usize);
            if !(*c).pixels.is_null() && cw > 0 && ch > 0 {
                let px = core::slice::from_raw_parts((*c).pixels, cw * ch);
                let argb: Vec<u32> = px.iter().map(|&p| p as u32).collect();
                let at = (i32::from((*c).x) - i32::from((*c).xhot), i32::from((*c).y) - i32::from((*c).yhot));
                blend_cursor(bgra, self.area, &argb, cw, ch, at);
            }
            XFree(c as *mut c_void);
        }
    }
}

/// Blend a premultiplied ARGB cursor image (`cw`×`ch`, top-left at `at` in
/// screen pixels) over the BGRA frame of `area`.
pub fn blend_cursor(bgra: &mut [u8], area: IRect, argb: &[u32], cw: usize, ch: usize, at: (i32, i32)) {
    let (fw, fh) = (area.2 as i32, area.3 as i32);
    let (ox, oy) = (at.0 - area.0, at.1 - area.1);
    for cy in 0..ch as i32 {
        let y = oy + cy;
        if y < 0 || y >= fh {
            continue;
        }
        for cx in 0..cw as i32 {
            let x = ox + cx;
            if x < 0 || x >= fw {
                continue;
            }
            let p = argb[cy as usize * cw + cx as usize];
            let a = p >> 24;
            if a == 0 {
                continue;
            }
            let i = (y as usize * fw as usize + x as usize) * 4;
            let Some(d) = bgra.get_mut(i..i + 3) else { continue };
            let inv = 255 - a;
            for (k, c) in [(p & 0xFF), (p >> 8 & 0xFF), (p >> 16 & 0xFF)].into_iter().enumerate() {
                d[k] = (c + (u32::from(d[k]) * inv + 127) / 255).min(255) as u8;
            }
        }
    }
}

impl FrameSource for XShmSource {
    fn next(&mut self, deadline: Instant) -> Option<Frame> {
        let _ = deadline; // a grab takes a few ms; there is nothing to wait for
        match self.grab() {
            Some(mut bgra) => {
                // SAFETY: requests on our Display, errors recorded.
                unsafe { guarded(self.dpy, || self.draw_cursor(&mut bgra)) };
                let f = Frame::bgra(self.area.2, self.area.3, bgra, Duration::ZERO);
                self.last = Some(f.clone());
                Some(f)
            }
            None => self.last.clone(),
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.area.2, self.area.3)
    }
}

impl Drop for XShmSource {
    fn drop(&mut self) {
        self.drop_shm();
        unsafe {
            XCloseDisplay(self.dpy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_are_clipped_to_the_screen() {
        assert_eq!(clip_area((10, 20, 100, 50), 1920, 1080), Some((10, 20, 100, 50)));
        assert_eq!(clip_area((-10, -20, 100, 50), 1920, 1080), Some((0, 0, 90, 30)));
        assert_eq!(clip_area((1900, 1000, 100, 100), 1920, 1080), Some((1900, 1000, 20, 80)));
        assert_eq!(clip_area((2000, 0, 100, 100), 1920, 1080), None);
    }

    #[test]
    fn the_cursor_is_blended_and_clipped() {
        // A 4x2 grey frame at (100, 100); a 2x2 cursor at (103, 99): only
        // its bottom-left pixel lands on the frame (x 3, y 0).
        let area = (100, 100, 4, 2);
        let mut f = [128u8, 128, 128, 255].repeat(8);
        // Opaque red, half-transparent white (premultiplied 0x80808080), clear, opaque blue.
        let cur = [0xFFFF0000, 0x00000000, 0x80808080, 0xFF0000FF];
        blend_cursor(&mut f, area, &cur, 2, 2, (103, 99));
        assert_eq!(&f[12..16], &[128 + 64, 128 + 64, 128 + 64, 255], "half white over grey");
        assert_eq!(&f[..12], &[128, 128, 128, 255].repeat(3)[..]);
        let mut f = [0u8, 0, 0, 255].repeat(8);
        blend_cursor(&mut f, area, &cur, 2, 2, (100, 100));
        assert_eq!(&f[..4], &[0, 0, 255, 255], "red is BGRA 0,0,255");
        assert_eq!(&f[16..20], &[128, 128, 128, 255]);
        assert_eq!(&f[20..24], &[255, 0, 0, 255], "blue");
    }
}
