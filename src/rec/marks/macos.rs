//! macOS (compile-only for now): a listen-only CGEventTap for mouse
//! downs and key downs/ups, on a thread running its own CFRunLoop until
//! the recording ends. Needs the Input Monitoring permission; without it
//! the tap is not made and the recording says so.

use super::{Button, Held, Hooks, Marks};
use anyhow::{anyhow, Result};
use core::ffi::c_void;
use std::sync::Arc;
use std::time::Instant;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

type TapCallback = unsafe extern "C" fn(proxy: *mut c_void, ty: u32, event: *mut c_void, user: *mut c_void) -> *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, user: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(e: *mut c_void) -> CGPoint;
    fn CGEventGetIntegerValueField(e: *mut c_void, field: u32) -> i64;
    fn CGEventGetFlags(e: *mut c_void) -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: *mut c_void, order: isize) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: *mut c_void);
    fn CFRelease(p: *const c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

const SESSION_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const LISTEN_ONLY: u32 = 1;
const LEFT_DOWN: u32 = 1;
const RIGHT_DOWN: u32 = 3;
const KEY_DOWN: u32 = 10;
const KEYCODE_FIELD: u32 = 9;

/// macOS virtual key codes (ANSI) → Win32-numbered keys.
fn vk_of(code: i64) -> u32 {
    const LETTERS: [(i64, u8); 26] = [
        (0, b'A'), (11, b'B'), (8, b'C'), (2, b'D'), (14, b'E'), (3, b'F'), (5, b'G'), (4, b'H'), (34, b'I'),
        (38, b'J'), (40, b'K'), (37, b'L'), (46, b'M'), (45, b'N'), (31, b'O'), (35, b'P'), (12, b'Q'),
        (15, b'R'), (1, b'S'), (17, b'T'), (32, b'U'), (9, b'V'), (13, b'W'), (7, b'X'), (16, b'Y'), (6, b'Z'),
    ];
    const DIGITS: [i64; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    if let Some((_, c)) = LETTERS.iter().find(|(k, _)| *k == code) {
        return *c as u32;
    }
    if let Some(d) = DIGITS.iter().position(|k| *k == code) {
        return 0x30 + d as u32;
    }
    match code {
        36 => 0x0D,
        48 => 0x09,
        49 => 0x20,
        51 => 0x08,
        53 => 0x1B,
        117 => 0x2E,
        123 => 0x25,
        124 => 0x27,
        125 => 0x28,
        126 => 0x26,
        _ => 0,
    }
}

/// The chord modifiers of a key event's flags (Shift 1<<17, Control
/// 1<<18, Option 1<<19, Command 1<<20). Option alone types characters
/// (Option+2 is € on some layouts): it counts only with Control or Command.
fn held_of(f: u64) -> Held {
    let (ctrl, cmd) = (f >> 18 & 1 == 1, f >> 20 & 1 == 1);
    Held { ctrl, alt: f >> 19 & 1 == 1 && (ctrl || cmd), shift: f >> 17 & 1 == 1, win: cmd }
}

struct Tap {
    marks: Arc<Marks>,
}

unsafe extern "C" fn callback(_proxy: *mut c_void, ty: u32, event: *mut c_void, user: *mut c_void) -> *mut c_void {
    // SAFETY: `user` is the Tap the thread keeps alive while the loop runs.
    let tap = unsafe { &mut *(user as *mut Tap) };
    let now = Instant::now();
    unsafe {
        match ty {
            LEFT_DOWN | RIGHT_DOWN => {
                let p = CGEventGetLocation(event);
                let k = crate::capture::backing();
                let b = if ty == LEFT_DOWN { Button::Left } else { Button::Right };
                tap.marks.click((p.x * k) as i32, (p.y * k) as i32, b, now);
            }
            KEY_DOWN => {
                let vk = vk_of(CGEventGetIntegerValueField(event, KEYCODE_FIELD));
                if vk != 0 {
                    tap.marks.key(vk, held_of(CGEventGetFlags(event)), false, now);
                }
            }
            _ => {}
        }
    }
    event
}

/// The run loop to stop (a raw pointer, handed between threads once).
struct Loop(*mut c_void);

// SAFETY: CFRunLoopStop may be called from any thread.
unsafe impl Send for Loop {}

struct MacHooks {
    rl: Loop,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Hooks for MacHooks {}

impl Drop for MacHooks {
    fn drop(&mut self) {
        let Some(j) = self.join.take() else { return };
        // Asked again until it ends (a stop before the loop ran is lost).
        for _ in 0..100 {
            // SAFETY: the loop of our thread, still running or done.
            unsafe { CFRunLoopStop(self.rl.0) };
            if j.is_finished() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if j.is_finished() {
            let _ = j.join();
        }
    }
}

pub fn start(marks: Arc<Marks>) -> Result<Box<dyn Hooks>> {
    let (tx, rx) = crate::chan::channel::<Option<Loop>>();
    let join = std::thread::Builder::new().name("rec-marks".into()).spawn(move || {
        let mut tap = Box::new(Tap { marks });
        let mask: u64 = (1 << LEFT_DOWN) | (1 << RIGHT_DOWN) | (1 << KEY_DOWN);
        // SAFETY: the tap's user pointer outlives the run loop below.
        unsafe {
            let port = CGEventTapCreate(SESSION_TAP, HEAD_INSERT, LISTEN_ONLY, mask, callback, &mut *tap as *mut Tap as *mut c_void);
            if port.is_null() {
                let _ = tx.send(None);
                return;
            }
            let src = CFMachPortCreateRunLoopSource(core::ptr::null(), port, 0);
            let rl = CFRunLoopGetCurrent();
            CFRunLoopAddSource(rl, src, kCFRunLoopCommonModes);
            let _ = tx.send(Some(Loop(rl)));
            CFRunLoopRun();
            CFRelease(src);
            CFRelease(port);
        }
        drop(tap);
    })?;
    match rx.recv() {
        Ok(Some(rl)) => Ok(Box::new(MacHooks { rl, join: Some(join) })),
        _ => {
            let _ = join.join();
            Err(anyhow!("allow Rustshot under Privacy & Security › Input Monitoring"))
        }
    }
}
