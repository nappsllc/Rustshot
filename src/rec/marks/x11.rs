//! X11: XInput2 raw events (`libXi`, loaded when used) on the root
//! window, read on a thread with its own Display: raw key presses and
//! releases, and raw button presses (the pointer's place is asked then).
//! Raw events are only listened to (nothing is grabbed, nothing passes
//! through Rustshot). Key codes become Win32-numbered keys through their
//! unshifted keysym. Wayland gives no app the input of others: not
//! available there (the recording says so once saved).

use super::{Button, Hooks, Marks};
use crate::rec::linux::dl::Lib;
use anyhow::{bail, Context, Result};
use core::ffi::{c_char, c_int, c_uchar, c_uint, c_ulong, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[repr(C)]
struct XIEventMask {
    deviceid: c_int,
    mask_len: c_int,
    mask: *mut c_uchar,
}

/// `XGenericEventCookie`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Cookie {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    extension: c_int,
    evtype: c_int,
    cookie: c_uint,
    data: *mut c_void,
}

/// The head of `XIRawEvent` (what is read of it).
#[repr(C)]
struct RawHead {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    extension: c_int,
    evtype: c_int,
    time: c_ulong,
    deviceid: c_int,
    sourceid: c_int,
    detail: c_int,
}

#[repr(C)]
union XEvent {
    type_: c_int,
    cookie: Cookie,
    pad: [core::ffi::c_long; 24],
}

#[allow(clashing_extern_declarations)]
#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(dpy: *mut c_void) -> c_int;
    fn XDefaultScreen(dpy: *mut c_void) -> c_int;
    fn XRootWindow(dpy: *mut c_void, screen: c_int) -> c_ulong;
    fn XQueryExtension(dpy: *mut c_void, name: *const c_char, opcode: *mut c_int, event: *mut c_int, error: *mut c_int) -> c_int;
    fn XPending(dpy: *mut c_void) -> c_int;
    fn XNextEvent(dpy: *mut c_void, event: *mut XEvent) -> c_int;
    fn XGetEventData(dpy: *mut c_void, cookie: *mut Cookie) -> c_int;
    fn XFreeEventData(dpy: *mut c_void, cookie: *mut Cookie);
    fn XConnectionNumber(dpy: *mut c_void) -> c_int;
    fn XFlush(dpy: *mut c_void) -> c_int;
    fn XkbKeycodeToKeysym(dpy: *mut c_void, keycode: u8, group: c_uint, level: c_uint) -> c_ulong;
    fn XQueryKeymap(dpy: *mut c_void, keys: *mut c_char) -> c_int;
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
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
}

/// `XISelectEvents(dpy, win, masks, n)`.
type SelectEvents = unsafe extern "C" fn(*mut c_void, c_ulong, *mut XIEventMask, c_int) -> c_int;
/// `XIQueryVersion(dpy, &major, &minor)`.
type QueryVersion = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int) -> c_int;

const GENERIC_EVENT: c_int = 35;
const XI_ALL_MASTER_DEVICES: c_int = 1;
const XI_RAW_KEY_PRESS: c_int = 13;
const XI_RAW_BUTTON_PRESS: c_int = 15;

/// An unshifted keysym → Win32-numbered key (0: not one we name).
pub fn vk_of_keysym(ks: c_ulong) -> u32 {
    match ks {
        0x61..=0x7a => ks as u32 - 0x20, // a..z
        0x41..=0x5a | 0x30..=0x39 => ks as u32,
        0x20 => 0x20,
        0xff0d | 0xff8d => 0x0D, // Return, KP_Enter
        0xff1b => 0x1B,
        0xff08 => 0x08,
        0xff09 | 0xfe20 => 0x09,
        0xffff => 0x2E,
        0xff63 => 0x2D,
        0xff50 => 0x24,
        0xff57 => 0x23,
        0xff55 => 0x21,
        0xff56 => 0x22,
        0xff51 => 0x25,
        0xff52 => 0x26,
        0xff53 => 0x27,
        0xff54 => 0x28,
        0xff61 => 0x2C,
        0xffbe..=0xffd5 => 0x70 + (ks - 0xffbe) as u32, // F1..F24
        0xffe1 | 0xffe2 => 0xA0,                         // Shift
        0xffe3 | 0xffe4 => 0xA2,                         // Control
        0xffe7..=0xffea => 0xA4,                         // Meta, Alt
        0xffeb | 0xffec => 0x5B,                         // Super
        0xffe5 => 0x14,                                  // Caps_Lock
        0x3d => 0xBB,
        0x2d => 0xBD,
        0x5b => 0xDB,
        0x5d => 0xDD,
        0x3b => 0xBA,
        0x2c => 0xBC,
        0x2e => 0xBE,
        0x2f => 0xBF,
        0x60 => 0xC0,
        0x5c => 0xDC,
        0x27 => 0xDE,
        0xffab => 0x6B,
        0xffad => 0x6D,
        0xffaa => 0x6A,
        0xffaf => 0x6F,
        0xffb0..=0xffb9 => 0x60 + (ks - 0xffb0) as u32,
        _ => 0,
    }
}

struct XHooks {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Hooks for XHooks {}

impl Drop for XHooks {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Display, root and the XInput opcode; the library is kept open.
struct Conn {
    dpy: *mut c_void,
    root: c_ulong,
    opcode: c_int,
    _xi: Lib,
}

// SAFETY: made here and moved once to the listening thread, which alone
// uses it from then on.
unsafe impl Send for Conn {}

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: opened by `open`.
        unsafe {
            XCloseDisplay(self.dpy);
        }
    }
}

fn open(opts: super::MarkOpts) -> Result<Conn> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()) {
        bail!("Wayland does not let apps see other apps' input");
    }
    let xi = Lib::open("libXi.so.6")?;
    // SAFETY: XISelectEvents' C type.
    let select: SelectEvents = unsafe { xi.sym("XISelectEvents")? };
    // SAFETY: XIQueryVersion's C type.
    let version: QueryVersion = unsafe { xi.sym("XIQueryVersion")? };
    crate::wind::init_x11();
    // SAFETY: a fresh connection of ours.
    unsafe {
        let dpy = XOpenDisplay(core::ptr::null());
        if dpy.is_null() {
            bail!("cannot open the X display");
        }
        let root = XRootWindow(dpy, XDefaultScreen(dpy));
        let (mut op, mut ev, mut err) = (0, 0, 0);
        if XQueryExtension(dpy, c"XInputExtension".as_ptr(), &mut op, &mut ev, &mut err) == 0 {
            XCloseDisplay(dpy);
            bail!("the X server has no XInput extension");
        }
        // XI 2.0 semantics: no raw events while another client grabs the
        // keyboard (a screen locker's password prompt is never seen).
        let (mut major, mut minor) = (2, 0);
        if version(dpy, &mut major, &mut minor) != 0 || major < 2 {
            XCloseDisplay(dpy);
            bail!("the X server has no XInput 2");
        }
        let mut bits = [0u8; 4];
        let mut set = |e: c_int| bits[(e >> 3) as usize] |= 1 << (e & 7);
        if opts.keys {
            set(XI_RAW_KEY_PRESS);
        }
        if opts.clicks {
            set(XI_RAW_BUTTON_PRESS);
        }
        let mut mask = XIEventMask { deviceid: XI_ALL_MASTER_DEVICES, mask_len: bits.len() as c_int, mask: bits.as_mut_ptr() };
        select(dpy, root, &mut mask, 1);
        XFlush(dpy);
        Ok(Conn { dpy, root, opcode: op, _xi: xi })
    }
}

pub fn start(marks: Arc<Marks>) -> Result<Box<dyn Hooks>> {
    let conn = open(marks.opts)?;
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let join = std::thread::Builder::new()
        .name("rec-marks".into())
        .spawn(move || listen(conn, &marks, &s2))
        .context("start the input thread")?;
    Ok(Box::new(XHooks { stop, join: Some(join) }))
}

/// The key codes of each chord modifier (by their unshifted keysym; the
/// AltGr key is ISO_Level3_Shift, no modifier here).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ModCodes {
    ctrl: Vec<u8>,
    alt: Vec<u8>,
    shift: Vec<u8>,
    win: Vec<u8>,
}

impl ModCodes {
    pub(super) fn of(keysym: &dyn Fn(u8) -> c_ulong) -> ModCodes {
        let mut m = ModCodes::default();
        for code in 8..=255u8 {
            match keysym(code) {
                0xffe3 | 0xffe4 => m.ctrl.push(code),
                0xffe7..=0xffea => m.alt.push(code),
                0xffe1 | 0xffe2 => m.shift.push(code),
                0xffeb | 0xffec => m.win.push(code),
                _ => {}
            }
        }
        m
    }

    /// The modifiers down in `keys` (an `XQueryKeymap` bit field), read
    /// at the key: never derived from earlier events, so a release lost
    /// to another client's grab cannot leave one stuck.
    pub(super) fn held(&self, keys: &[u8; 32]) -> super::Held {
        let down = |codes: &[u8]| codes.iter().any(|&c| keys[c as usize / 8] & (1 << (c % 8)) != 0);
        super::Held { ctrl: down(&self.ctrl), alt: down(&self.alt), shift: down(&self.shift), win: down(&self.win) }
    }
}

fn listen(c: Conn, marks: &Marks, stop: &AtomicBool) {
    // SAFETY: the connection is this thread's alone.
    unsafe {
        let mods = ModCodes::of(&|code| XkbKeycodeToKeysym(c.dpy, code, 0, 0));
        let fd = XConnectionNumber(c.dpy);
        while !stop.load(Ordering::SeqCst) {
            if XPending(c.dpy) == 0 {
                let mut p = PollFd { fd, events: 1, revents: 0 };
                poll(&mut p, 1, 100);
                continue;
            }
            let mut ev: XEvent = core::mem::zeroed();
            XNextEvent(c.dpy, &mut ev);
            if ev.type_ != GENERIC_EVENT || ev.cookie.extension != c.opcode {
                continue;
            }
            let mut ck = ev.cookie;
            if XGetEventData(c.dpy, &mut ck) == 0 || ck.data.is_null() {
                continue;
            }
            let raw = &*(ck.data as *const RawHead);
            let now = Instant::now();
            match ck.evtype {
                XI_RAW_KEY_PRESS => {
                    let ks = XkbKeycodeToKeysym(c.dpy, raw.detail as u8, 0, 0);
                    let vk = vk_of_keysym(ks);
                    if vk != 0 && !super::is_modifier(vk) {
                        let mut keys = [0u8; 32];
                        XQueryKeymap(c.dpy, keys.as_mut_ptr().cast());
                        // No password field is known on X11: only chords
                        // show unless the user asked for every key.
                        marks.key(vk, mods.held(&keys), false, now);
                    }
                }
                XI_RAW_BUTTON_PRESS if raw.detail == 1 || raw.detail == 3 => {
                    let (mut r, mut ch, mut x, mut y, mut wx, mut wy, mut m) = (0, 0, 0, 0, 0, 0, 0);
                    XQueryPointer(c.dpy, c.root, &mut r, &mut ch, &mut x, &mut y, &mut wx, &mut wy, &mut m);
                    let b = if raw.detail == 1 { Button::Left } else { Button::Right };
                    marks.click(x, y, b, now);
                }
                _ => {}
            }
            XFreeEventData(c.dpy, &mut ck);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn keysyms_map_to_shared_keys() {
        use super::vk_of_keysym as v;
        assert_eq!(v(0x73), 0x53, "s is S");
        assert_eq!(v(0xffe3), 0xA2, "Control_L");
        assert_eq!(v(0xffbe), 0x70, "F1");
        assert_eq!(v(0x2f), 0xBF, "slash");
        assert_eq!(v(0x1234), 0);
        assert_eq!(super::super::key_label(v(0x2f)).as_deref(), Some("/"));
    }

    /// Modifiers come from the key map at the key; AltGr
    /// (ISO_Level3_Shift) is none.
    #[test]
    fn modifiers_are_read_from_the_keymap() {
        // A German-like map: 37 Control_L, 64 Alt_L, 108 ISO_Level3_Shift,
        // 50 Shift_L, 133 Super_L.
        let m = super::ModCodes::of(&|c| match c {
            37 => 0xffe3,
            64 => 0xffe9,
            108 => 0xfe03,
            50 => 0xffe1,
            133 => 0xffeb,
            _ => 0x61,
        });
        let keys = |codes: &[u8]| {
            let mut k = [0u8; 32];
            for &c in codes {
                k[c as usize / 8] |= 1 << (c % 8);
            }
            k
        };
        let h = m.held(&keys(&[108]));
        assert_eq!(h, super::super::Held::default(), "AltGr is no chord");
        let h = m.held(&keys(&[37, 50]));
        assert!(h.ctrl && h.shift && !h.alt && !h.win);
        assert!(m.held(&keys(&[133])).win);
    }

    /// Live (X11, the Docker harness's Xvfb only: it fakes input): XTest
    /// presses Ctrl+S and clicks, the raw-event listener files them.
    /// `cargo test live_x11_marks -- --ignored --nocapture`
    #[test]
    #[ignore = "live: X11 display (Docker), fakes input"]
    fn live_x11_marks_listen_to_raw_events() {
        use super::super::{Button, MarkOpts, Marks};
        use core::ffi::{c_int, c_uint, c_ulong, c_void};
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        // Only in a container's own X server: never fake input on a desktop.
        if std::env::var_os("DISPLAY").is_none() || std::env::var_os("RS_SUMMARY").is_none() && !std::path::Path::new("/.dockerenv").exists() {
            eprintln!("SKIP: not in the Docker harness");
            return;
        }
        #[link(name = "X11")]
        unsafe extern "C" {
            fn XKeysymToKeycode(dpy: *mut c_void, keysym: c_ulong) -> u8;
        }
        type FakeKey = unsafe extern "C" fn(*mut c_void, c_uint, c_int, c_ulong) -> c_int;
        type FakeButton = unsafe extern "C" fn(*mut c_void, c_uint, c_int, c_ulong) -> c_int;
        let lib = crate::rec::linux::dl::Lib::open("libXtst.so.6").expect("libXtst");
        let (key, button): (FakeKey, FakeButton) = unsafe { (lib.sym("XTestFakeKeyEvent").unwrap(), lib.sym("XTestFakeButtonEvent").unwrap()) };
        let marks = Arc::new(Marks::new(MarkOpts { clicks: true, keys: true, all_keys: false }));
        let hooks = super::start(marks.clone()).expect("listening");
        std::thread::sleep(Duration::from_millis(300));
        unsafe {
            let dpy = super::XOpenDisplay(core::ptr::null());
            assert!(!dpy.is_null());
            let (ctrl, s) = (XKeysymToKeycode(dpy, 0xffe3) as c_uint, XKeysymToKeycode(dpy, 0x73) as c_uint);
            // Ctrl is held while the listener reads the key map.
            for (k, down) in [(ctrl, 1), (s, 1), (s, 0)] {
                key(dpy, k, down, 0);
            }
            super::XFlush(dpy);
            std::thread::sleep(Duration::from_millis(300));
            key(dpy, ctrl, 0, 0);
            super::XFlush(dpy);
            std::thread::sleep(Duration::from_millis(100));
            // A typed letter: not shown (no Ctrl, Alt or Win).
            key(dpy, s, 1, 0);
            key(dpy, s, 0, 0);
            button(dpy, 3, 1, 0);
            button(dpy, 3, 0, 0);
            super::XFlush(dpy);
            super::XCloseDisplay(dpy);
        }
        let t0 = Instant::now();
        let mut scene = marks.scene(Instant::now());
        while (scene.pill.is_none() || scene.rings.is_empty()) && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
            scene = marks.scene(Instant::now());
        }
        drop(hooks);
        println!("X11 marks: {scene:?}");
        assert_eq!(scene.pill.map(|p| p.0).as_deref(), Some("Ctrl + S"));
        assert_eq!(scene.rings.first().map(|r| r.0.button), Some(Button::Right));
    }
}
