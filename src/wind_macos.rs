//! macOS overlay window: a borderless, keyable `NSWindow` whose layer is
//! presented from raw CoreGraphics images, a hand-rolled Cocoa event pump
//! (`NSEvent` → `Ev`, the same variable timer cadence as wind_win's
//! `SetTimer` via `wind::tick_ms`) and the main-thread Carbon hotkey bridge:
//! registered OS hotkeys only dispatch through the process' event loop, so
//! `hotkey_macos` parses on its own thread and hands registration/dispatch
//! over via `HotkeyHook` instead of owning a `CFRunLoop` nobody spins.
//! Also the decorated `run_window` (titled NSWindow; main thread only).

use super::*;

use crate::capture::{CGPoint, CGRect, CGSize};
use core::ffi::{c_char, c_void, CStr};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// --- frameworks -----------------------------------------------------------

// AppKit/Foundation/QuartzCore are used only for Objective-C classes (every
// call goes through objc_msgSend); an item-less extern block still emits its
// -framework flag at link time, and the frameworks must be loaded at process
// start so the classes exist before `main`.
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
    fn CGEventSourceFlagsState(state_id: u32) -> u64;
    fn CGDataProviderCreateWithData(
        info: *mut c_void,
        data: *const c_void,
        size: usize,
        release: unsafe extern "C" fn(*mut c_void, *const c_void, usize),
    ) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
        provider: *mut c_void,
        decode: *const f64,
        interpolate: bool,
        intent: i32,
    ) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> usize;
    fn objc_allocateClassPair(
        superclass: *mut c_void,
        name: *const c_char,
        extra_bytes: usize,
    ) -> *mut c_void;
    fn objc_registerClassPair(class: *mut c_void);
    fn class_addMethod(
        class: *mut c_void,
        sel: usize,
        imp: *const c_void,
        types: *const c_char,
    ) -> i8;
    fn objc_msgSend();
}

// --- objc runtime helpers -------------------------------------------------

/// Look up a class by name (null if the framework owning it never loaded).
pub fn objc_cls(name: &CStr) -> *mut c_void {
    unsafe { objc_getClass(name.as_ptr()) }
}

/// Register or look up a selector; the runtime caches these process-wide.
pub fn objc_sel(name: &CStr) -> usize {
    unsafe { sel_registerName(name.as_ptr()) }
}

/// `+[NSString stringWithUTF8String:]` — autoreleased, so only valid inside
/// an autorelease pool (the pump keeps one alive on every pass).
pub fn ns_string(s: &str) -> *mut c_void {
    unsafe {
        let bytes = std::ffi::CString::new(s).unwrap_or_default();
        let cls = objc_cls(c"NSString");
        msg1(cls, objc_sel(c"stringWithUTF8String:"), bytes.as_ptr())
    }
}

// The C symbol is an untyped variadic-ish function pointer: every call first
// transmutes it to the exact method signature (integer payloads travel in
// registers either way on arm64/x86-64; BOOL reads the low byte, structs
// follow the platform C ABI because the payloads are repr(C)).

pub unsafe fn msg0<R: Copy>(recv: *mut c_void, sel: usize) -> R {
    unsafe {
        let f: unsafe extern "C" fn(*mut c_void, usize) -> R =
            core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(recv, sel)
    }
}

pub unsafe fn msg1<A1: Copy, R: Copy>(recv: *mut c_void, sel: usize, a1: A1) -> R {
    unsafe {
        let f: unsafe extern "C" fn(*mut c_void, usize, A1) -> R =
            core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(recv, sel, a1)
    }
}

pub unsafe fn msg2<A1: Copy, A2: Copy, R: Copy>(recv: *mut c_void, sel: usize, a1: A1, a2: A2) -> R {
    unsafe {
        let f: unsafe extern "C" fn(*mut c_void, usize, A1, A2) -> R =
            core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(recv, sel, a1, a2)
    }
}

pub unsafe fn msg3<A1: Copy, A2: Copy, A3: Copy, R: Copy>(
    recv: *mut c_void,
    sel: usize,
    a1: A1,
    a2: A2,
    a3: A3,
) -> R {
    unsafe {
        let f: unsafe extern "C" fn(*mut c_void, usize, A1, A2, A3) -> R =
            core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(recv, sel, a1, a2, a3)
    }
}

pub unsafe fn msg4<A1: Copy, A2: Copy, A3: Copy, A4: Copy, R: Copy>(
    recv: *mut c_void,
    sel: usize,
    a1: A1,
    a2: A2,
    a3: A3,
    a4: A4,
) -> R {
    unsafe {
        let f: unsafe extern "C" fn(*mut c_void, usize, A1, A2, A3, A4) -> R =
            core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(recv, sel, a1, a2, a3, a4)
    }
}

/// Manual autorelease pool: AppKit hands back autoreleased objects on every
/// call, and the pump wants them reclaimed per pass instead of accumulating.
struct Pool(*mut c_void);

impl Pool {
    fn new() -> Self {
        unsafe {
            let cls = objc_cls(c"NSAutoreleasePool");
            if cls.is_null() {
                return Pool(core::ptr::null_mut());
            }
            // `+new` (alloc + init): `init` sent to the class itself
            // returned the class, whose `drain` does not exist.
            Pool(msg0(cls, objc_sel(c"new")))
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _: () = msg0(self.0, objc_sel(c"drain"));
            }
        }
    }
}

// --- main-thread Carbon hotkey bridge -------------------------------------

/// Function table published by `hotkey_macos`. Its parsing thread cannot own
/// an event queue: `RegisterEventHotKey` events only dispatch on the main
/// thread's run loop, so wind's pump accepts this hook and calls it every
/// pass instead (`ReceiveNextEvent` on any other thread never fires).
pub struct HotkeyHook {
    /// Called once on the pump thread before the first loop pass.
    pub install: fn(),
    /// Drain pending Carbon hotkey events without blocking.
    pub pump: fn(),
    /// Called once when the pump loop ends.
    pub shutdown: fn(),
}

/// Slot the parsing thread writes and `run` reads (taken at most once).
static PENDING_HOOK: Mutex<Option<HotkeyHook>> = Mutex::new(None);

/// Hand a hook over from the hotkey parsing thread to the window pump.
pub fn publish_hotkeys(hook: HotkeyHook) {
    let mut slot = PENDING_HOOK.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(hook);
}

/// Live modifier state via `CGEventSourceFlagsState` (session-state flags:
/// Shift 1<<17, Control 1<<18, Option 1<<19, Command 1<<20). `Mods` has no
/// meta field, so Command is folded into `ctrl`.
pub fn current_mods() -> Mods {
    unsafe {
        let f = CGEventSourceFlagsState(1); // kCGEventSourceStateCombinedSessionState
        Mods {
            shift: f & (1 << 17) != 0,
            // ⌘ (bit 20) works like Ctrl so ⌘Z/⌘C/⌘S/⌘U match native apps.
            ctrl: f & (1 << 18) != 0 || f & (1 << 20) != 0,
            alt: f & (1 << 19) != 0,
        }
    }
}

// --- process-wide state ---------------------------------------------------

/// Set by `close` (always inside a driver callback on the pump thread; an
/// atomic also makes a cross-thread close safe, like wind_linux's).
static QUIT: AtomicBool = AtomicBool::new(false);
/// Set by `invalidate`/`show_at`, consumed once per loop pass.
static PRESENT: AtomicBool = AtomicBool::new(false);
/// Window of the running overlay (0 = none); read by `capture_macos`.
pub static MAIN_WINDOW: AtomicUsize = AtomicUsize::new(0);
/// Cached overlay class (NSWindow subclass), created on first use.
/// NSWindowCollectionBehavior of the overlay and the recording windows:
/// CanJoinAllSpaces (1 << 0) | Stationary (1 << 4) | IgnoresCycle (1 << 6)
/// | FullScreenAuxiliary (1 << 8).
const COLLECT_OVERLAY: u64 = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);

static OVERLAY_CLASS: AtomicUsize = AtomicUsize::new(0);

std::thread_local! {
    /// The overlay as last placed by `show_at` (pump thread only): its
    /// top-left in pixels, its height in points and the pixels per point
    /// (`capture::backing`). NSEvent locations are window-local points
    /// with y up; the driver's are global pixels with y down.
    static WIN_FRAME: Cell<(f64, f64, f64, f64)> = const { Cell::new((0.0, 0.0, 0.0, 1.0)) };
}

// --- public API -----------------------------------------------------------

/// Whether `hwnd` is the running overlay's window (not 0, not stale).
fn is_overlay(hwnd: Hwnd) -> bool {
    hwnd.0 != 0 && MAIN_WINDOW.load(Ordering::SeqCst) == hwnd.0
}

/// Ask for a repaint of the window's client area. An unknown or stale
/// handle is a no-op.
pub fn invalidate(hwnd: Hwnd) {
    if !with_slot(hwnd, |s| s.present = true) && is_overlay(hwnd) {
        PRESENT.store(true, Ordering::SeqCst);
    }
}

/// The event loops read `tick_ms(win)` each pass; nothing to re-arm.
pub fn retime(_hwnd: Hwnd, _ms: u64) {}

/// Position + show the overlay at an exact rect (the shared code's pixels,
/// see `capture_macos`; placed in points) and take focus.
pub fn show_at(hwnd: Hwnd, x: i32, y: i32, w: i32, h: i32) {
    if hwnd.0 == 0 {
        return;
    }
    unsafe {
        let window = hwnd.0 as *mut c_void;
        let k = crate::capture::backing();
        let (px, py, pw, ph) = crate::capture::pts::rect_pt((x, y, w.max(0) as u32, h.max(0) as u32), k);
        WIN_FRAME.with(|f| f.set((f64::from(x), f64::from(y), ph, k)));
        let size = CGSize { width: pw, height: ph };
        let _: () = msg1(window, objc_sel(c"setContentSize:"), size);
        // Quartz top-down origin → Cocoa bottom-up frame (a borderless
        // window's frame equals its content rect).
        let origin = CGPoint { x: px, y: crate::capture::main_display_height() - py - ph };
        let _: () = msg1(window, objc_sel(c"setFrameOrigin:"), origin);
        let app = objc_cls(c"NSApplication");
        let app: *mut c_void = msg0(app, objc_sel(c"sharedApplication"));
        let _: () = msg1(app, objc_sel(c"activateIgnoringOtherApps:"), 1i64);
        let _: () = msg1(
            window,
            objc_sel(c"makeKeyAndOrderFront:"),
            core::ptr::null_mut::<c_void>(),
        );
    }
    // A re-positioned window must repaint even when no input follows.
    PRESENT.store(true, Ordering::SeqCst);
}

pub fn hide(hwnd: Hwnd) {
    if hwnd.0 == 0 {
        return;
    }
    unsafe {
        let _: () = msg1(
            hwnd.0 as *mut c_void,
            objc_sel(c"orderOut:"),
            core::ptr::null_mut::<c_void>(),
        );
    }
}

/// Never destroy the window from here: callers run inside driver callbacks
/// on the pump thread, exactly like `DestroyWindow` on Win32 (whose effect
/// is also deferred until the current dispatch returns). An unknown or
/// stale handle is a no-op.
pub fn close(hwnd: Hwnd) {
    if !with_slot(hwnd, |s| s.quit = true) && is_overlay(hwnd) {
        QUIT.store(true, Ordering::SeqCst);
    }
}

/// Activation policy + `finishLaunching`, once per process whichever of
/// the overlay and `run_window` comes first. Accessory policy: no Dock
/// tile, no menu bar (the overlay is the whole UI; activation still works
/// for a borderless window).
unsafe fn launch_app(app: *mut c_void) {
    static LAUNCHED: std::sync::Once = std::sync::Once::new();
    LAUNCHED.call_once(|| unsafe {
        let _: () = msg1(app, objc_sel(c"setActivationPolicy:"), 1i64);
        let _: () = msg0(app, objc_sel(c"finishLaunching"));
    });
}

/// Create the (initially hidden) overlay and pump messages until quit.
/// Returns when the window is destroyed.
pub fn run(driver: &mut dyn Driver) -> i32 {
    QUIT.store(false, Ordering::SeqCst);
    PRESENT.store(false, Ordering::SeqCst);
    // Outer pool: setup and teardown autoreleased objects (the loop below
    // keeps its own per pass). Drops on every return path.
    let pool = Pool::new();
    let code = pump_loop(driver);
    drop(pool);
    code
}

// --- pump -----------------------------------------------------------------

fn pump_loop(driver: &mut dyn Driver) -> i32 {
    unsafe {
        let app_cls = objc_cls(c"NSApplication");
        if app_cls.is_null() {
            eprintln!("failed to load AppKit (NSApplication missing)");
            return 1;
        }
        let app: *mut c_void = msg0(app_cls, objc_sel(c"sharedApplication"));
        if app.is_null() {
            eprintln!("failed to create NSApplication");
            return 1;
        }
        launch_app(app);
        let window = create_window();
        if window.is_null() {
            eprintln!("failed to create overlay window");
            return 1;
        }
        WIN_FRAME.with(|f| f.set((0.0, 0.0, 0.0, 1.0)));
        MAIN_WINDOW.store(window as usize, Ordering::SeqCst);
        driver.on_create(Hwnd(window as usize));

        let mut hook: Option<HotkeyHook> = None;
        let mut cursor: Option<Cursor> = None;
        let mut key: Option<bool> = None;
        let me = Hwnd(window as usize);
        let mut next_tick = Instant::now() + Duration::from_millis(tick_ms(me));
        loop {
            if QUIT.load(Ordering::SeqCst) {
                break;
            }
            // The parsing thread publishes asynchronously: take the hook the
            // first pass it exists, then feed Carbon before Cocoa each pass
            // so hotkey events are never swallowed by the Cocoa drain.
            if hook.is_none() {
                let taken = PENDING_HOOK
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if let Some(h) = taken {
                    (h.install)();
                    hook = Some(h);
                }
            }
            if let Some(h) = hook.as_ref() {
                (h.pump)();
            }
            // Honour a slow->fast switch made during the previous pass.
            next_tick = next_tick.min(Instant::now() + Duration::from_millis(tick_ms(me)));
            let pool = Pool::new();
            let mut repaint = false;
            // Block only for the rest of the current tick slice on the first wait,
            // then drain the queue without waiting until it runs dry.
            let mut first = true;
            while !QUIT.load(Ordering::SeqCst) {
                let wait = if first {
                    first = false;
                    next_tick
                        .saturating_duration_since(Instant::now())
                        .as_secs_f64()
                } else {
                    0.0
                };
                let deadline: *mut c_void = msg1(
                    objc_cls(c"NSDate"),
                    objc_sel(c"dateWithTimeIntervalSinceNow:"),
                    wait,
                );
                let mode = ns_string("kCFRunLoopDefaultMode");
                let ev: *mut c_void = msg4(
                    app,
                    objc_sel(c"nextEventMatchingMask:untilDate:mode:dequeue:"),
                    u64::MAX, // NSEventMaskAny
                    deadline,
                    mode,
                    1i64, // dequeue
                );
                if ev.is_null() {
                    break; // timeout or queue empty
                }
                // The recording's control bar and the camera bubble.
                if aux_event(ev) {
                    continue;
                }
                if dispatch_nsevent(ev, driver) {
                    repaint = true;
                }
            }
            if !QUIT.load(Ordering::SeqCst) && Instant::now() >= next_tick {
                next_tick = Instant::now() + Duration::from_millis(tick_ms(me));
                // Idle ticks (nothing animating) cost no frame.
                repaint |= driver.on_event(Ev::Timer);
            }
            repaint |= PRESENT.swap(false, Ordering::SeqCst);
            if repaint && !QUIT.load(Ordering::SeqCst)
                && let Some(fb) = driver.frame()
            {
                present(window, &*fb);
            }
            // Focus lost (another app or window is key) or back: the
            // pointer shows again, or may hide.
            let is_key: i8 = msg0(window, objc_sel(c"isKeyWindow"));
            if !QUIT.load(Ordering::SeqCst) && key != Some(is_key != 0) {
                key = Some(is_key != 0);
                driver.on_event(Ev::Focus(is_key != 0));
            }
            let want = driver.cursor();
            if cursor != Some(want) {
                set_cursor(want);
                cursor = Some(want);
            }
            drop(pool);
        }
        if let Some(h) = hook {
            (h.shutdown)();
        }
        // A hidden pointer is app-wide on macOS: never leave it hidden.
        set_cursor(Cursor::Arrow);
        driver.on_quit();
        forget_tick(me);
        let _: () = msg1(
            window,
            objc_sel(c"orderOut:"),
            core::ptr::null_mut::<c_void>(),
        );
        let _: () = msg0(window, objc_sel(c"close"));
        let _: () = msg0(window, objc_sel(c"release"));
        MAIN_WINDOW.store(0, Ordering::SeqCst);
        WIN_FRAME.with(|f| f.set((0.0, 0.0, 0.0, 1.0)));
        0
    }
}

// --- window ---------------------------------------------------------------

fn create_window() -> *mut c_void {
    unsafe {
        let class = overlay_class();
        if class.is_null() {
            return core::ptr::null_mut();
        }
        let w: *mut c_void = msg0(class, objc_sel(c"alloc"));
        // The designated initializer (there is no public
        // `initWithStyleMask:backing:defer:`); the frame is set later.
        let w: *mut c_void = msg4(
            w,
            objc_sel(c"initWithContentRect:styleMask:backing:defer:"),
            CGRect::default(),
            0u64, // NSWindowStyleMaskBorderless (no chrome, outer == inner)
            2u64, // NSBackingStoreBuffered
            0i8,  // defer = NO
        );
        if w.is_null() {
            return core::ptr::null_mut();
        }
        let _: () = msg1(w, objc_sel(c"setReleasedWhenClosed:"), 0i64);
        // Floating keeps us above normal windows without reaching the
        // screensaver/menu-bar levels (kCGFloatingWindowLevelKey = 5 → 3).
        let level = CGWindowLevelForKey(5) as i64;
        let _: () = msg1(w, objc_sel(c"setLevel:"), level);
        // NSWindowCollectionBehavior: CanJoinAllSpaces (1 << 0) | Stationary
        // (1 << 4) | IgnoresCycle (1 << 6) | FullScreenAuxiliary (1 << 8):
        // on every Space and over full-screen apps (WS_EX_TOPMOST parity),
        // not moved by Exposé, not in the Cmd-` window cycle.
        let _: () = msg1(w, objc_sel(c"setCollectionBehavior:"), COLLECT_OVERLAY);
        let _: () = msg1(w, objc_sel(c"setAcceptsMouseMovedEvents:"), 1i64);
        let _: () = msg1(w, objc_sel(c"setHasShadow:"), 0i64);
        let bg: *mut c_void = msg0(objc_cls(c"NSColor"), objc_sel(c"blackColor"));
        let _: () = msg1(w, objc_sel(c"setBackgroundColor:"), bg);
        let view: *mut c_void = msg0(objc_cls(c"NSView"), objc_sel(c"alloc"));
        let view: *mut c_void = msg1(view, objc_sel(c"initWithFrame:"), CGRect::default());
        if view.is_null() {
            let _: () = msg0(w, objc_sel(c"close"));
            let _: () = msg0(w, objc_sel(c"release"));
            return core::ptr::null_mut();
        }
        let _: () = msg1(view, objc_sel(c"setWantsLayer:"), 1i64);
        let _: () = msg1(w, objc_sel(c"setContentView:"), view);
        let _: () = msg0(view, objc_sel(c"release")); // the window retains it
        w
    }
}

/// NSWindow subclass that answers YES to `canBecomeKeyWindow`: borderless
/// windows refuse key status by default, and without a key window the
/// window server never routes keyboard events to this process. Built once.
fn overlay_class() -> *mut c_void {
    unsafe {
        let cached = OVERLAY_CLASS.load(Ordering::SeqCst);
        if cached != 0 {
            return cached as *mut c_void;
        }
        let base = objc_cls(c"NSWindow");
        if base.is_null() {
            return core::ptr::null_mut();
        }
        let class = objc_allocateClassPair(base, c"RustshotOverlay".as_ptr(), 0);
        if class.is_null() {
            return objc_cls(c"RustshotOverlay"); // another call won the race
        }
        let imp: unsafe extern "C" fn(*mut c_void, usize) -> i8 = can_become_key;
        if class_addMethod(
            class,
            objc_sel(c"canBecomeKeyWindow"),
            imp as *const c_void,
            c"c@:".as_ptr(), // BOOL return, self, _cmd
        ) == 0
        {
            eprintln!("warning: could not override canBecomeKeyWindow");
        }
        objc_registerClassPair(class);
        OVERLAY_CLASS.store(class as usize, Ordering::SeqCst);
        class
    }
}

/// `-canBecomeKeyWindow` → YES.
unsafe extern "C" fn can_become_key(_this: *mut c_void, _cmd: usize) -> i8 {
    1
}

// --- decorated window (run_window) ----------------------------------------

/// Per-thread state of a running `run_window` (win 0 = none). `close` and
/// `invalidate` route here for that window, so a dialog never touches the
/// overlay's QUIT/PRESENT flags.
#[derive(Clone, Copy)]
struct WinSlot {
    win: usize,
    quit: bool,
    present: bool,
    /// The close button fired `performClose:` (turned into `Ev::Close`).
    close_req: bool,
}

const NO_SLOT: WinSlot = WinSlot { win: 0, quit: false, present: false, close_req: false };

std::thread_local! {
    static WINDOWED: Cell<WinSlot> = const { Cell::new(NO_SLOT) };
}

/// Apply `f` to this thread's `run_window` slot when `hwnd` is that window.
fn with_slot(hwnd: Hwnd, f: impl FnOnce(&mut WinSlot)) -> bool {
    WINDOWED.with(|c| {
        let mut s = c.get();
        if s.win == 0 || s.win != hwnd.0 {
            return false;
        }
        f(&mut s);
        c.set(s);
        true
    })
}

#[cfg_attr(not(test), allow(unused_imports))]
pub use window::{resize, run_window, scale};

/// The decorated window proper (no caller outside tests until the update
/// dialog / Settings land, hence the dead-code allowance). Compile-checked
/// only so far: it has not been run on a Mac yet.
mod window {
    #![cfg_attr(not(test), allow(dead_code))]
    use super::*;

    #[cfg(target_arch = "x86_64")]
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_msgSend_stret();
    }

    /// A message returning a CGRect. 32 bytes come back through a hidden
    /// pointer on x86-64, which needs `objc_msgSend_stret`; arm64 returns
    /// it via x8 through plain `objc_msgSend`.
    unsafe fn msg_rect(recv: *mut c_void, sel: usize) -> CGRect {
        unsafe {
            #[cfg(target_arch = "x86_64")]
            {
                let f: unsafe extern "C" fn(*mut c_void, usize) -> CGRect =
                    core::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
                f(recv, sel)
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                msg0(recv, sel)
            }
        }
    }

    /// A message taking and returning a CGRect (see `msg_rect`).
    unsafe fn msg_rect1(recv: *mut c_void, sel: usize, a1: CGRect) -> CGRect {
        unsafe {
            #[cfg(target_arch = "x86_64")]
            {
                let f: unsafe extern "C" fn(*mut c_void, usize, CGRect) -> CGRect =
                    core::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
                f(recv, sel, a1)
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                msg1(recv, sel, a1)
            }
        }
    }

    /// Height in points the frame adds above the content (the title bar).
    unsafe fn title_bar_height(win: *mut c_void) -> f64 {
        unsafe {
            let frame = msg_rect(win, objc_sel(c"frame"));
            let content = msg_rect1(win, objc_sel(c"contentRectForFrameRect:"), frame);
            (frame.size.height - content.size.height).max(0.0)
        }
    }

    /// Backing scale of a window (2.0 on Retina): points -> pixels.
    fn backing(hwnd: Hwnd) -> f32 {
        if hwnd.0 == 0 {
            return 1.0;
        }
        let s: f64 = unsafe { msg0(hwnd.0 as *mut c_void, objc_sel(c"backingScaleFactor")) };
        if s > 0.0 { s as f32 } else { 1.0 }
    }

    /// UI scale of a `run_window` window: its backing scale times the UI
    /// size (its content is the spec's size times the UI size, in points).
    pub fn scale(hwnd: Hwnd) -> f32 {
        crate::uisize::apply(backing(hwnd))
    }

    /// Give a `run_window` window the content size `w`×`h` (logical px)
    /// at the current UI size; `Ev::Resize` follows from the pump.
    pub fn resize(hwnd: Hwnd, w: u32, h: u32) {
        if hwnd.0 == 0 {
            return;
        }
        let m = f64::from(crate::uisize::mult());
        let size = CGSize { width: f64::from(w) * m, height: f64::from(h) * m };
        unsafe {
            let _: () = msg1(hwnd.0 as *mut c_void, objc_sel(c"setContentSize:"), size);
        }
    }

    static WINDOW_CLASS: AtomicUsize = AtomicUsize::new(0);

    /// NSWindow subclass whose `performClose:` (the close button's action)
    /// only records the request: the driver decides, as on Win32/X11.
    fn window_class() -> *mut c_void {
        unsafe {
            let cached = WINDOW_CLASS.load(Ordering::SeqCst);
            if cached != 0 {
                return cached as *mut c_void;
            }
            let base = objc_cls(c"NSWindow");
            if base.is_null() {
                return core::ptr::null_mut();
            }
            let class = objc_allocateClassPair(base, c"RustshotWindow".as_ptr(), 0);
            if class.is_null() {
                return objc_cls(c"RustshotWindow"); // registered already
            }
            let imp: unsafe extern "C" fn(*mut c_void, usize, *mut c_void) = perform_close;
            class_addMethod(class, objc_sel(c"performClose:"), imp as *const c_void, c"v@:@".as_ptr());
            objc_registerClassPair(class);
            WINDOW_CLASS.store(class as usize, Ordering::SeqCst);
            class
        }
    }

    /// `-performClose:` → flag `Ev::Close` for the pump.
    unsafe extern "C" fn perform_close(this: *mut c_void, _cmd: usize, _sender: *mut c_void) {
        with_slot(Hwnd(this as usize), |s| s.close_req = true);
    }

    /// Content size in points (the content view's bounds).
    unsafe fn content_size(window: *mut c_void) -> (f64, f64) {
        unsafe {
            let view: *mut c_void = msg0(window, objc_sel(c"contentView"));
            if view.is_null() {
                return (0.0, 0.0);
            }
            let r = msg_rect(view, objc_sel(c"bounds"));
            (r.size.width, r.size.height)
        }
    }

    fn phys(size: (f64, f64), s: f64) -> (u32, u32) {
        (((size.0 * s).round() as u32).max(1), ((size.1 * s).round() as u32).max(1))
    }

    /// Open a normal titled NSWindow centred on the display under the
    /// pointer and pump events until the driver calls `wind::close(hwnd)`.
    /// The close button only sends `Ev::Close`; `Ev::Resize` (pixels)
    /// follows `on_create` and every size/backing-scale change; mouse
    /// coordinates are client pixels, top-left origin.
    ///
    /// Limitation: AppKit only works on the main thread, so this refuses to
    /// run elsewhere — and the daemon's main thread is busy in the overlay
    /// pump, so dialogs on macOS need that pump to host them (later work).
    pub fn run_window(spec: WindowSpec, driver: &mut dyn Driver) -> anyhow::Result<()> {
        let _outer = Pool::new();
        unsafe {
            let is_main: i8 = msg0(objc_cls(c"NSThread"), objc_sel(c"isMainThread"));
            if is_main == 0 {
                anyhow::bail!("run_window: AppKit windows must be created on the main thread");
            }
            let app_cls = objc_cls(c"NSApplication");
            if app_cls.is_null() {
                anyhow::bail!("failed to load AppKit (NSApplication missing)");
            }
            let app: *mut c_void = msg0(app_cls, objc_sel(c"sharedApplication"));
            launch_app(app);
            let class = window_class();
            if class.is_null() {
                anyhow::bail!("failed to create the window class");
            }
            // Logical px are points times the UI size.
            let m = f64::from(crate::uisize::mult());
            let (w, h) = (spec.w.max(1) as f64 * m, spec.h.max(1) as f64 * m);
            let content = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: w, height: h } };
            // Titled | Closable (| Miniaturizable) (| Resizable): the traffic
            // lights; a button the window does not have shows disabled.
            // Content under the title bar: | FullSizeContentView (our
            // surface runs to the top edge; the content view is the whole
            // window, the title bar transparent over its top 28 pt).
            let under = driver.under_titlebar();
            let style: u64 = 1
                | 2
                | if spec.controls.minimize() { 4 } else { 0 }
                | if spec.resizable { 8 } else { 0 }
                | if under { 1 << 15 } else { 0 };
            let win: *mut c_void = msg0(class, objc_sel(c"alloc"));
            let win: *mut c_void = msg4(
                win,
                objc_sel(c"initWithContentRect:styleMask:backing:defer:"),
                content,
                style,
                2u64, // NSBackingStoreBuffered
                0i64, // defer = NO
            );
            if win.is_null() {
                anyhow::bail!("failed to create NSWindow");
            }
            let _: () = msg1(win, objc_sel(c"setReleasedWhenClosed:"), 0i64);
            // Still set when hidden: the Window menu, Mission Control and
            // the Dock's window list show it.
            let _: () = msg1(win, objc_sel(c"setTitle:"), ns_string(&spec.title));
            if under {
                // The header row is the visual title (NSWindowTitleHidden).
                let _: () = msg1(win, objc_sel(c"setTitleVisibility:"), 1i64);
                let _: () = msg1(win, objc_sel(c"setTitlebarAppearsTransparent:"), 1i64);
            }
            if spec.resizable {
                let min = CGSize { width: spec.min.0 as f64 * m, height: spec.min.1 as f64 * m };
                let _: () = msg1(win, objc_sel(c"setContentMinSize:"), min);
            }
            let _: () = msg1(win, objc_sel(c"setAcceptsMouseMovedEvents:"), 1i64);
            let view: *mut c_void = msg0(objc_cls(c"NSView"), objc_sel(c"alloc"));
            let view: *mut c_void = msg1(view, objc_sel(c"initWithFrame:"), content);
            if !view.is_null() {
                let _: () = msg1(view, objc_sel(c"setWantsLayer:"), 1i64);
                let _: () = msg1(win, objc_sel(c"setContentView:"), view);
                let _: () = msg0(view, objc_sel(c"release")); // the window retains it
            }
            // Centre on the display under the pointer (Quartz points, y
            // down); the title bar adds to the frame above the content.
            let (cx, cy) = crate::capture::cursor_pos();
            let mons = crate::capture::monitors().unwrap_or_default();
            if let Some(m) = mons.iter().find(|m| m.contains(cx, cy)).or(mons.first()) {
                let outer_h = h + title_bar_height(win);
                // Monitors are in pixels; the window is placed in points.
                let (mx, my, mw, mh) = crate::capture::to_points_i((m.x, m.y, m.w, m.h));
                let (x, y) = centre_in((mx, my, mw as i32, mh as i32), w as i32, outer_h.round() as i32);
                let top_left = CGPoint { x: x as f64, y: crate::capture::main_display_height() - y as f64 };
                let _: () = msg1(win, objc_sel(c"setFrameTopLeftPoint:"), top_left);
            } else {
                let _: () = msg0(win, objc_sel(c"center"));
            }
            let hwnd = Hwnd(win as usize);
            let mut s = backing(hwnd) as f64;
            set_layer_scale(win, s);
            let prev = WINDOWED.with(|c| c.replace(WinSlot { win: hwnd.0, ..NO_SLOT }));
            let quit = || WINDOWED.with(|c| c.get().quit);
            let mut size = phys(content_size(win), s);
            let mut lang = crate::i18n::generation();
            let mut title = crate::wind::Title::new(&spec.title);
            driver.on_create(hwnd);
            if !quit() {
                driver.on_event(Ev::Resize(size.0, size.1));
            }
            let mut frame = None;
            if !quit() {
                // Themed before it is first shown.
                frame = driver.frame_colors();
                apply_frame(win, frame, under);
                let _: () = msg1(app, objc_sel(c"activateIgnoringOtherApps:"), 1i64);
                let _: () = msg1(win, objc_sel(c"makeKeyAndOrderFront:"), core::ptr::null_mut::<c_void>());
            }
            let mut key = false;
            let mut pressed = false;
            let mut cursor: Option<Cursor> = None;
            let mut next_tick = Instant::now() + Duration::from_millis(tick_ms(hwnd));
            while !quit() {
                next_tick = next_tick.min(Instant::now() + Duration::from_millis(tick_ms(hwnd)));
                let pool = Pool::new();
                let mut repaint = false;
                let mut first = true;
                while !quit() {
                    let wait = if first {
                        first = false;
                        next_tick.saturating_duration_since(Instant::now()).as_secs_f64()
                    } else {
                        0.0
                    };
                    let deadline: *mut c_void =
                        msg1(objc_cls(c"NSDate"), objc_sel(c"dateWithTimeIntervalSinceNow:"), wait);
                    let ev: *mut c_void = msg4(
                        app,
                        objc_sel(c"nextEventMatchingMask:untilDate:mode:dequeue:"),
                        u64::MAX,
                        deadline,
                        ns_string("kCFRunLoopDefaultMode"),
                        1i64,
                    );
                    if ev.is_null() {
                        break;
                    }
                    let target: *mut c_void = msg0(ev, objc_sel(c"window"));
                    let ty: u64 = msg0(ev, objc_sel(c"type"));
                    let ours = target == win;
                    // Content under the title bar: a press in its strip
                    // moves the window unless one of our buttons is there.
                    let strip = if ours && under && ty == LEFT_DOWN { strip_press(ev, &*driver, content_size(win), s) } else { None };
                    if ours && strip != Some(true) {
                        repaint |= dispatch_window(ev, ty, driver, content_size(win), s, &mut pressed);
                    }
                    // Keys stay ours (no responder chain: it would beep),
                    // as does a press on our button in the strip (or the
                    // title bar would start a drag); everything else also
                    // goes to AppKit so the title bar, traffic lights and
                    // resize edges work.
                    if !(ours && (ty == KEY_DOWN || ty == KEY_UP || strip == Some(false))) {
                        let _: () = msg1(app, objc_sel(c"sendEvent:"), ev);
                    }
                }
                let mut close_req = false;
                with_slot(hwnd, |v| close_req = core::mem::take(&mut v.close_req));
                if close_req {
                    driver.on_event(Ev::Close);
                    repaint = true;
                }
                let now_s = backing(hwnd) as f64;
                if now_s != s {
                    s = now_s;
                    set_layer_scale(win, s);
                }
                let now = phys(content_size(win), s);
                if !quit() && now != size {
                    size = now;
                    driver.on_event(Ev::Resize(now.0, now.1));
                    repaint = true;
                }
                let is_key: i8 = msg0(win, objc_sel(c"isKeyWindow"));
                if !quit() && (is_key != 0) != key {
                    key = is_key != 0;
                    driver.on_event(Ev::Focus(key));
                    repaint = true;
                }
                if !quit() && Instant::now() >= next_tick {
                    next_tick = Instant::now() + Duration::from_millis(tick_ms(hwnd));
                    // The UI language switched: lay out again, retitle.
                    if let Some(t) = crate::wind::relocalize(&mut lang, driver, size) {
                        if let Some(t) = title.retitle(t) {
                            let _: () = msg1(win, objc_sel(c"setTitle:"), ns_string(&t));
                        }
                        repaint = true;
                    }
                    repaint |= driver.on_event(Ev::Timer);
                }
                with_slot(hwnd, |v| repaint |= core::mem::take(&mut v.present));
                if repaint && !quit() {
                    let want = driver.frame_colors();
                    if want != frame {
                        frame = want;
                        apply_frame(win, frame, under);
                    }
                    // Unsaved changes: the document-edited dot in the
                    // close button (the title text stays as it is).
                    if title.set_edited(driver.edited()).is_some() {
                        let _: () = msg1(win, objc_sel(c"setDocumentEdited:"), i64::from(title.is_edited()));
                    }
                }
                if repaint
                    && !quit()
                    && let Some(fb) = driver.frame()
                {
                    present(win, &*fb);
                }
                let want = driver.cursor();
                if cursor != Some(want) {
                    set_cursor(want);
                    cursor = Some(want);
                }
                drop(pool);
            }
            driver.on_quit();
            forget_tick(hwnd);
            let _: () = msg1(win, objc_sel(c"orderOut:"), core::ptr::null_mut::<c_void>());
            let _: () = msg0(win, objc_sel(c"close"));
            let _: () = msg0(win, objc_sel(c"release"));
            WINDOWED.with(|c| c.set(prev));
            Ok(())
        }
    }

    /// The title bar in the design's colours: transparent over a window
    /// background of the caption colour, in the dark or light appearance
    /// of OUR theme (traffic lights and title text to match). A window
    /// whose content runs `under` it (board 12) keeps it transparent over
    /// our own surface; the others keep the content view below it.
    /// `None`: AppKit's defaults.
    unsafe fn apply_frame(win: *mut c_void, c: Option<FrameColors>, under: bool) {
        unsafe {
            let _: () = msg1(win, objc_sel(c"setTitlebarAppearsTransparent:"), i64::from(c.is_some() || under));
            let appearance: *mut c_void = match c {
                Some(c) => msg1(
                    objc_cls(c"NSAppearance"),
                    objc_sel(c"appearanceNamed:"),
                    ns_string(if c.dark { "NSAppearanceNameDarkAqua" } else { "NSAppearanceNameAqua" }),
                ),
                None => core::ptr::null_mut(),
            };
            let _: () = msg1(win, objc_sel(c"setAppearance:"), appearance);
            let bg: *mut c_void = match c {
                Some(c) => {
                    let [r, g, b] = c.caption.map(|v| f64::from(v) / 255.0);
                    msg4(objc_cls(c"NSColor"), objc_sel(c"colorWithSRGBRed:green:blue:alpha:"), r, g, b, 1.0f64)
                }
                None => msg0(objc_cls(c"NSColor"), objc_sel(c"windowBackgroundColor")),
            };
            if !bg.is_null() {
                let _: () = msg1(win, objc_sel(c"setBackgroundColor:"), bg);
            }
        }
    }

    /// The layer shows frames at pixel density (2x images on Retina).
    unsafe fn set_layer_scale(win: *mut c_void, s: f64) {
        unsafe {
            let view: *mut c_void = msg0(win, objc_sel(c"contentView"));
            if view.is_null() {
                return;
            }
            let layer: *mut c_void = msg0(view, objc_sel(c"layer"));
            if !layer.is_null() {
                let _: () = msg1(layer, objc_sel(c"setContentsScale:"), s);
            }
        }
    }

    /// Mouse events in client pixels (top-left origin); keys as the overlay.
    /// Only the content rect is client area: moves, presses and wheel over
    /// the title bar or frame are left to AppKit. A press that started
    /// inside keeps its drags and release wherever they land (SetCapture
    /// parity).
    unsafe fn dispatch_window(
        ev: *mut c_void,
        ty: u64,
        driver: &mut dyn Driver,
        content: (f64, f64),
        s: f64,
        pressed: &mut bool,
    ) -> bool {
        unsafe {
            let (content_w, content_h) = content;
            let pos = || {
                let p: CGPoint = msg0(ev, objc_sel(c"locationInWindow"));
                // Window coordinates are y-up with the content at the bottom.
                let inside = p.x >= 0.0 && p.x < content_w && p.y >= 0.0 && p.y < content_h;
                ((p.x * s).round() as i32, ((content_h - p.y) * s).round() as i32, inside)
            };
            match ty {
                MOUSE_MOVED | LEFT_DRAGGED => {
                    let (x, y, inside) = pos();
                    if inside || (ty == LEFT_DRAGGED && *pressed) {
                        driver.on_event(Ev::Move { x, y });
                    }
                    false
                }
                LEFT_DOWN => {
                    let (x, y, inside) = pos();
                    if !inside {
                        return false;
                    }
                    *pressed = true;
                    driver.on_event(Ev::Down { x, y });
                    true
                }
                LEFT_UP => {
                    if !core::mem::take(pressed) {
                        return false;
                    }
                    let (x, y, _) = pos();
                    driver.on_event(Ev::Up { x, y });
                    true
                }
                SCROLL_WHEEL => {
                    let (x, y, inside) = pos();
                    if !inside {
                        return false;
                    }
                    let delta = wheel_delta(ev);
                    driver.on_event(Ev::Wheel { delta, x, y });
                    true
                }
                KEY_DOWN | KEY_UP => dispatch_nsevent(ev, driver),
                _ => false,
            }
        }
    }

    /// A press in the transparent title bar's strip of a window whose
    /// content runs under it (the top 28 pt, [`Overlap::MACOS`]): `true`
    /// when it is the title bar's (the traffic lights' zone, or a spot the
    /// driver leaves to it: AppKit moves the window), `false` when it is
    /// ours (a button reaching into the strip). `None`: not in the strip.
    unsafe fn strip_press(ev: *mut c_void, driver: &dyn Driver, content: (f64, f64), s: f64) -> Option<bool> {
        unsafe {
            let p: CGPoint = msg0(ev, objc_sel(c"locationInWindow"));
            let top = content.1 - p.y;
            let o = Overlap::MACOS;
            if !(0.0..f64::from(o.top)).contains(&top) || !(0.0..content.0).contains(&p.x) {
                return None;
            }
            if p.x < f64::from(o.left) {
                return Some(true);
            }
            Some(driver.drags_window((p.x * s).round() as i32, (top * s).round() as i32))
        }
    }
}

// --- event pump -----------------------------------------------------------

/// NSEventType values (stable ABI numbering).
const LEFT_DOWN: u64 = 1;
const LEFT_UP: u64 = 2;
const RIGHT_DOWN: u64 = 3;
const RIGHT_UP: u64 = 4;
const RIGHT_DRAGGED: u64 = 7;
const MOUSE_MOVED: u64 = 5;
const LEFT_DRAGGED: u64 = 6;
const KEY_DOWN: u64 = 10;
const KEY_UP: u64 = 11;
const SCROLL_WHEEL: u64 = 22;

/// A scroll event's vertical delta in Win32 units: ±120 a notch, away
/// from the user positive whatever the natural-scrolling setting.
/// A wheel reports lines (a notch is one); a trackpad or Magic Mouse
/// reports precise deltas in points, many events a gesture: ten points
/// make a line, so a swipe is notches a piece at a time, not one (or
/// dozens) every event. Never zero for a nonzero scroll.
unsafe fn wheel_delta(ev: *mut c_void) -> i32 {
    unsafe {
        let dy: f64 = msg0(ev, objc_sel(c"scrollingDeltaY"));
        let precise: i8 = msg0(ev, objc_sel(c"hasPreciseScrollingDeltas"));
        let inverted: i8 = msg0(ev, objc_sel(c"isDirectionInvertedFromDevice"));
        wheel_units(dy, precise != 0, inverted != 0)
    }
}

/// [`wheel_delta`]'s arithmetic: `dy` in lines, or in points when
/// `precise`; `inverted` undoes natural scrolling.
fn wheel_units(dy: f64, precise: bool, inverted: bool) -> i32 {
    let lines = if precise { dy / 10.0 } else { dy };
    let mut delta = (lines * 120.0).round() as i32;
    if delta == 0 && dy != 0.0 {
        delta = if dy < 0.0 { -1 } else { 1 };
    }
    if inverted { -delta } else { delta }
}
// FlagsChanged (12) is deliberately absent: modifier-only changes are not
// keys on any backend (wind_linux ignores them the same way).

/// `locationInWindow` is window-local with y up from the bottom-left;
/// events are reported in Quartz global space (y down).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NSRange {
    location: u64,
    length: u64,
}

/// Map one NSEvent onto `Ev` calls; returns whether a repaint is due (input
/// and timer repaint, WM_MOUSEMOVE-style motion never does).
fn dispatch_nsevent(ev: *mut c_void, driver: &mut dyn Driver) -> bool {
    unsafe {
        let ty: u64 = msg0(ev, objc_sel(c"type"));
        match ty {
            MOUSE_MOVED | LEFT_DRAGGED | RIGHT_DRAGGED => {
                let p: CGPoint = msg0(ev, objc_sel(c"locationInWindow"));
                let (x, y) = client_pos(p);
                driver.on_event(Ev::Move { x, y });
                false
            }
            RIGHT_DOWN | RIGHT_UP => {
                let p: CGPoint = msg0(ev, objc_sel(c"locationInWindow"));
                let (x, y) = client_pos(p);
                driver.on_event(Ev::Right { down: ty == RIGHT_DOWN, x, y });
                true
            }
            LEFT_DOWN | LEFT_UP => {
                let p: CGPoint = msg0(ev, objc_sel(c"locationInWindow"));
                let (x, y) = client_pos(p);
                if ty == LEFT_DOWN {
                    driver.on_event(Ev::Down { x, y });
                } else {
                    driver.on_event(Ev::Up { x, y });
                }
                true
            }
            KEY_DOWN => {
                key_down(ev, driver);
                true
            }
            KEY_UP => {
                let flags: u64 = msg0(ev, objc_sel(c"modifierFlags"));
                let code: u16 = msg0(ev, objc_sel(c"keyCode"));
                let vk = vk_of_kvk(code);
                if vk != 0 {
                    driver.on_event(Ev::Key {
                        vk,
                        up: true,
                        repeat: false,
                        mods: mods_of(flags),
                    });
                }
                true
            }
            SCROLL_WHEEL => {
                let delta = wheel_delta(ev);
                let p: CGPoint = msg0(ev, objc_sel(c"locationInWindow"));
                let (x, y) = client_pos(p);
                driver.on_event(Ev::Wheel { delta, x, y });
                true
            }
            _ => false,
        }
    }
}

fn key_down(ev: *mut c_void, driver: &mut dyn Driver) {
    unsafe {
        let flags: u64 = msg0(ev, objc_sel(c"modifierFlags"));
        let repeat: i8 = msg0(ev, objc_sel(c"isARepeat"));
        let code: u16 = msg0(ev, objc_sel(c"keyCode"));
        let mods = mods_of(flags);
        let vk = vk_of_kvk(code);
        if vk != 0 {
            driver.on_event(Ev::Key {
                vk,
                up: false,
                repeat: repeat != 0,
                mods,
            });
        }
        // One Char per UTF-16 unit of `characters` (Win32 WM_CHAR parity);
        // empty for arrows/navigation keys, control codes for Ctrl combos.
        let chars: *mut c_void = msg0(ev, objc_sel(c"characters"));
        if !chars.is_null() {
            let len: u64 = msg0(chars, objc_sel(c"length"));
            if len > 0 {
                let mut buf = vec![0u16; len as usize];
                let _: () = msg2(
                    chars,
                    objc_sel(c"getCharacters:range:"),
                    buf.as_mut_ptr(),
                    NSRange {
                        location: 0,
                        length: len,
                    },
                );
                for &c in &buf {
                    driver.on_event(Ev::Char(c));
                }
            }
        }
    }
}

/// A window-local event location (points, y up) → global pixels, y down.
fn client_pos(p: CGPoint) -> (i32, i32) {
    let (fx, fy, fh, k) = WIN_FRAME.with(|f| f.get());
    ((fx + p.x * k).round() as i32, (fy + (fh - p.y) * k).round() as i32)
}

/// NSEventModifierFlag bits: shift 1<<17, control 1<<18, option 1<<19,
/// command 1<<20. `Mods` has no meta field, so Command is folded into `ctrl`
/// (⌘Z/⌘C/⌘S/⌘U act like Ctrl, as in native apps).
fn mods_of(flags: u64) -> Mods {
    Mods {
        shift: flags & (1 << 17) != 0,
        ctrl: flags & (1 << 18) != 0 || flags & (1 << 20) != 0,
        alt: flags & (1 << 19) != 0,
    }
}

/// Hardware keycode (Events.h `kVK_*`) → Win32-style virtual key (the
/// shared numbering from `wind::key` / `key_vk`); 0 = unmapped.
fn vk_of_kvk(code: u16) -> u32 {
    match code {
        // Letters (irregular on purpose — this is the physical layout).
        0x00 => 0x41, // A
        0x01 => 0x53, // S
        0x02 => 0x44, // D
        0x03 => 0x46, // F
        0x04 => 0x48, // H
        0x05 => 0x47, // G
        0x06 => 0x5A, // Z
        0x07 => 0x58, // X
        0x08 => 0x43, // C
        0x09 => 0x56, // V
        0x0B => 0x42, // B
        0x0C => 0x51, // Q
        0x0D => 0x57, // W
        0x0E => 0x45, // E
        0x0F => 0x52, // R
        0x10 => 0x59, // Y
        0x11 => 0x54, // T
        0x1F => 0x4F, // O
        0x20 => 0x55, // U
        0x22 => 0x49, // I
        0x23 => 0x50, // P
        0x25 => 0x4C, // L
        0x26 => 0x4A, // J
        0x28 => 0x4B, // K
        0x2D => 0x4E, // N
        0x2E => 0x4D, // M
        // Digit row.
        0x1D => 0x30, // 0
        0x12 => 0x31, // 1
        0x13 => 0x32, // 2
        0x14 => 0x33, // 3
        0x15 => 0x34, // 4
        0x17 => 0x35, // 5
        0x16 => 0x36, // 6
        0x1A => 0x37, // 7
        0x1C => 0x38, // 8
        0x19 => 0x39, // 9
        // Punctuation row.
        0x18 => 0xBB, // =
        0x1B => 0xBD, // -
        0x1E => 0xDD, // ]
        0x21 => 0xDB, // [
        0x27 => 0xDE, // '
        0x29 => 0xBA, // ;
        0x2A => 0xDC, // backslash
        0x2B => 0xBC, // ,
        0x2C => 0xBF, // /
        0x2F => 0xBE, // .
        0x32 => 0xC0, // `
        // Editing/navigation.
        0x24 => key::RETURN,
        0x30 => key::TAB,
        0x31 => key::SPACE,
        0x33 => key::BACK,
        0x35 => key::ESCAPE,
        0x72 => key::INSERT, // Help
        0x73 => key::HOME,
        0x74 => key::PAGEUP,
        0x75 => key::DELETE, // forward delete
        0x77 => key::END,
        0x79 => key::PAGEDOWN,
        0x7B => key::LEFT,
        0x7C => key::RIGHT,
        0x7D => key::DOWN,
        0x7E => key::UP,
        0x69 => key::PRINTSCREEN, // F13 plays the PrtSc role on mac keyboards
        // Function row (deliberately not sequential).
        0x7A => 0x70, // F1
        0x78 => 0x71, // F2
        0x63 => 0x72, // F3
        0x76 => 0x73, // F4
        0x60 => 0x74, // F5
        0x61 => 0x75, // F6
        0x62 => 0x76, // F7
        0x64 => 0x77, // F8
        0x65 => 0x78, // F9
        0x6D => 0x79, // F10
        0x67 => 0x7A, // F11
        0x6F => 0x7B, // F12
        // Numeric keypad.
        0x52 => 0x60,
        0x53 => 0x61,
        0x54 => 0x62,
        0x55 => 0x63,
        0x56 => 0x64,
        0x57 => 0x65,
        0x58 => 0x66,
        0x59 => 0x67,
        0x5B => 0x68,
        0x5C => 0x69,
        0x41 => 0x6E, // .
        0x43 => 0x6A, // *
        0x45 => 0x6B, // +
        0x4E => 0x6D, // -
        0x4B => 0x6F, // /
        0x4C => key::RETURN, // keypad enter
        0x47 => 0x0C, // clear
        _ => 0,
    }
}

// --- presentation ---------------------------------------------------------

/// Push an unpremultiplied RGBA framebuffer into the window's layer. The
/// loop never spins NSWindow's display cycle, so the transaction is
/// committed *and flushed* — without a run loop, flush is what actually
/// hands the new contents to the compositor.
fn present(window: *mut c_void, fb: &PixBuf) {
    let (w, h) = fb.dimensions();
    if w == 0 || h == 0 {
        return;
    }
    unsafe {
        let mut rgba = fb.as_raw().clone();
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255; // frames are opaque; be explicit like wind_win
        }
        let (len, cap) = (rgba.len(), rgba.capacity());
        let data = rgba.as_mut_ptr();
        std::mem::forget(rgba);
        let info = Box::into_raw(Box::new([len, cap])) as *mut c_void;
        let provider = CGDataProviderCreateWithData(info, data.cast(), len, release_pixels);
        if provider.is_null() {
            release_pixels(info, data.cast(), len);
            return;
        }
        let space = CGColorSpaceCreateDeviceRGB();
        let image = CGImageCreate(
            w as usize,
            h as usize,
            8,
            32,
            (w * 4) as usize,
            space,
            // premultipliedLast | byteOrder32Big = RGBA bytes in memory
            // (Apple QA1708); alpha is 255 everywhere, so premultiplied
            // and straight layouts coincide.
            0x3001,
            provider,
            core::ptr::null(),
            true,
            0, // kCGRenderingIntentDefault
        );
        CGDataProviderRelease(provider);
        if !space.is_null() {
            CGColorSpaceRelease(space);
        }
        if image.is_null() {
            return;
        }
        let view: *mut c_void = msg0(window, objc_sel(c"contentView"));
        let layer: *mut c_void = if view.is_null() {
            core::ptr::null_mut()
        } else {
            msg0(view, objc_sel(c"layer"))
        };
        if !layer.is_null() {
            let tx = objc_cls(c"CATransaction");
            let _: () = msg0(tx, objc_sel(c"begin"));
            let _: () = msg1(tx, objc_sel(c"setDisableActions:"), 0i64);
            // The frame is in pixels, the layer in points.
            let k = WIN_FRAME.with(|f| f.get().3);
            let _: () = msg1(layer, objc_sel(c"setContentsScale:"), k);
            let _: () = msg1(layer, objc_sel(c"setContents:"), image);
            let _: () = msg0(tx, objc_sel(c"commit"));
            let _: () = msg0(tx, objc_sel(c"flush"));
        }
        CGImageRelease(image);
    }
}

/// Provider release callback: rebuilds the boxed `[len, cap]` and pixel Vec
/// `present` handed over, freeing them exactly when CoreGraphics is done.
unsafe extern "C" fn release_pixels(info: *mut c_void, data: *const c_void, size: usize) {
    unsafe {
        let cap = Box::from_raw(info as *mut [usize; 2])[1];
        drop(Vec::from_raw_parts(data as *mut u8, size, cap));
    }
}

// --- cursor ---------------------------------------------------------------

std::thread_local! {
    /// `[NSCursor hide]` is in effect (from us): hide and unhide nest
    /// app-wide, so each hide is balanced by exactly one unhide.
    static POINTER_HIDDEN: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Cache `Cursor` → an `NSCursor` class selector; the cache lives one run.
/// `Cursor::Hidden` hides the pointer (once); any other shows it again.
fn set_cursor(c: Cursor) {
    unsafe {
        let cls = objc_cls(c"NSCursor");
        if cls.is_null() {
            return;
        }
        let hidden = POINTER_HIDDEN.with(|h| h.get());
        if c == Cursor::Hidden {
            if !hidden {
                let _: () = msg0(cls, objc_sel(c"hide"));
                POINTER_HIDDEN.with(|h| h.set(true));
            }
            return;
        }
        if hidden {
            let _: () = msg0(cls, objc_sel(c"unhide"));
            POINTER_HIDDEN.with(|h| h.set(false));
        }
        let (want, alt): (&CStr, &CStr) = match c {
            Cursor::Hidden => return,
            Cursor::Arrow => (c"arrowCursor", c"arrowCursor"),
            Cursor::Cross => (c"crosshairCursor", c"crosshairCursor"),
            Cursor::IBeam => (c"IBeamCursor", c"IBeamCursor"),
            Cursor::SizeNS => (c"resizeUpDownCursor", c"arrowCursor"),
            Cursor::SizeWE => (c"resizeLeftRightCursor", c"arrowCursor"),
            Cursor::SizeNWSE => (c"resizeNorthWestSouthEastCursor", c"resizeLeftRightCursor"),
            Cursor::SizeNESW => (c"resizeNorthEastSouthWestCursor", c"resizeLeftRightCursor"),
            Cursor::Move => (c"openHandCursor", c"arrowCursor"),
        };
        // The diagonal class methods only exist on newer SDKs; ask first.
        let want_sel = objc_sel(want);
        let responds: i8 = msg1(cls, objc_sel(c"respondsToSelector:"), want_sel);
        let sel = if responds != 0 { want_sel } else { objc_sel(alt) };
        let obj: *mut c_void = msg0(cls, sel);
        if !obj.is_null() {
            let _: () = msg0(obj, objc_sel(c"set"));
        }
    }
}

// --- auxiliary windows (recording border, bar, countdown; camera bubble) ---

use std::cell::RefCell;
use std::rc::Rc;

/// A pointer event on an auxiliary window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxKind {
    Down,
    Drag,
    Up,
}

/// [`AuxKind`] at `local` (points from the window's top-left) and
/// `global` (Quartz points).
#[derive(Clone, Copy, Debug)]
pub struct AuxEv {
    pub kind: AuxKind,
    pub local: (f64, f64),
    pub global: (f64, f64),
}

/// Where a layer window is: x, y, w, h in Quartz points.
type Frame = Rc<Cell<(f64, f64, f64, f64)>>;
type AuxFn = Rc<RefCell<dyn FnMut(AuxEv)>>;

std::thread_local! {
    /// The auxiliary windows taking pointer events (pump thread only).
    static AUX: RefCell<Vec<(usize, Frame, AuxFn)>> = const { RefCell::new(Vec::new()) };
}

/// NSEventType values of pointer events: left/right down, up (1–4), moved
/// (5), left/right dragged (6, 7), entered/exited (8, 9), scroll wheel (22),
/// other button down/up/dragged (25–27).
fn is_mouse_event(ty: u64) -> bool {
    matches!(ty, 1..=9 | 22 | 25..=27)
}

/// Hand an event for an auxiliary window to its handler; true when it was
/// for one (the overlay's driver never sees those).
fn aux_event(ev: *mut c_void) -> bool {
    let target: *mut c_void = unsafe { msg0(ev, objc_sel(c"window")) };
    if target.is_null() {
        return false;
    }
    let hit = AUX.with(|a| a.borrow().iter().find(|e| e.0 == target as usize).map(|e| (e.1.clone(), e.2.clone())));
    let Some((frame, f)) = hit else { return false };
    let ty: u64 = unsafe { msg0(ev, objc_sel(c"type")) };
    let kind = match ty {
        LEFT_DOWN => AuxKind::Down,
        LEFT_DRAGGED => AuxKind::Drag,
        LEFT_UP => AuxKind::Up,
        // Other pointer events on it: taken (the overlay must not see a
        // click or a move meant for the bar or the bubble).
        t if is_mouse_event(t) => return true,
        // Everything else (window, app-defined, system events): AppKit's.
        _ => {
            unsafe {
                let app: *mut c_void = msg0(objc_cls(c"NSApplication"), objc_sel(c"sharedApplication"));
                let _: () = msg1(app, objc_sel(c"sendEvent:"), ev);
            }
            return true;
        }
    };
    let p: CGPoint = unsafe { msg0(ev, objc_sel(c"locationInWindow")) };
    let (x, y, _, h) = frame.get();
    let local = (p.x, h - p.y);
    let ev = AuxEv { kind, local, global: (x + local.0, y + local.1) };
    if let Ok(mut f) = f.try_borrow_mut() {
        f(ev);
    }
    true
}

/// A borderless, transparent, non-activating panel above other windows
/// (and above the overlay), on every Space: the recording's border, bar
/// and countdown, and the camera bubble. Main thread only.
pub struct Layer {
    win: *mut c_void,
    frame: Frame,
    shown: bool,
}

impl Layer {
    /// `click_through`: pointer events pass through it; `private`: kept
    /// out of screen capture by other apps (`NSWindowSharingNone`; our own
    /// recording filters our windows out anyway).
    pub fn new(click_through: bool, private: bool) -> anyhow::Result<Layer> {
        if !crate::macos_objc::is_main_thread() {
            anyhow::bail!("recording windows must be made on the main thread");
        }
        unsafe {
            let app: *mut c_void = msg0(objc_cls(c"NSApplication"), objc_sel(c"sharedApplication"));
            if app.is_null() {
                anyhow::bail!("AppKit is not available");
            }
            launch_app(app);
            let rect = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 1.0, height: 1.0 } };
            let w: *mut c_void = msg0(objc_cls(c"NSPanel"), objc_sel(c"alloc"));
            let w: *mut c_void = msg4(
                w,
                objc_sel(c"initWithContentRect:styleMask:backing:defer:"),
                rect,
                1u64 << 7, // borderless | NSWindowStyleMaskNonactivatingPanel
                2u64,      // NSBackingStoreBuffered
                0i8,
            );
            if w.is_null() {
                anyhow::bail!("create a recording window");
            }
            let _: () = msg1(w, objc_sel(c"setReleasedWhenClosed:"), 0i8);
            let _: () = msg1(w, objc_sel(c"setOpaque:"), 0i8);
            let clear: *mut c_void = msg0(objc_cls(c"NSColor"), objc_sel(c"clearColor"));
            let _: () = msg1(w, objc_sel(c"setBackgroundColor:"), clear);
            let _: () = msg1(w, objc_sel(c"setHasShadow:"), 0i8);
            let _: () = msg1(w, objc_sel(c"setHidesOnDeactivate:"), 0i8);
            // Above the overlay (floating level).
            let _: () = msg1(w, objc_sel(c"setLevel:"), CGWindowLevelForKey(5) as i64 + 1);
            let _: () = msg1(w, objc_sel(c"setCollectionBehavior:"), COLLECT_OVERLAY);
            let _: () = msg1(w, objc_sel(c"setIgnoresMouseEvents:"), i8::from(click_through));
            if private {
                let _: () = msg1(w, objc_sel(c"setSharingType:"), 0u64); // NSWindowSharingNone
            }
            let view: *mut c_void = msg0(objc_cls(c"NSView"), objc_sel(c"alloc"));
            let view: *mut c_void = msg1(view, objc_sel(c"initWithFrame:"), rect);
            if !view.is_null() {
                // Layer-hosting (setLayer: before setWantsLayer:): AppKit
                // leaves our layer's contents alone (a layer-backed view's
                // layer is AppKit's to redraw).
                let layer: *mut c_void = msg0(objc_cls(c"CALayer"), objc_sel(c"layer"));
                if !layer.is_null() {
                    let _: () = msg1(view, objc_sel(c"setLayer:"), layer);
                }
                let _: () = msg1(view, objc_sel(c"setWantsLayer:"), 1i8);
                let _: () = msg1(w, objc_sel(c"setContentView:"), view);
                let _: () = msg0(view, objc_sel(c"release"));
            }
            Ok(Layer { win: w, frame: Rc::new(Cell::new((0.0, 0.0, 1.0, 1.0))), shown: false })
        }
    }

    /// The window's number (its `CGWindowID`, what ScreenCaptureKit lists).
    pub fn id(&self) -> u32 {
        let n: i64 = unsafe { msg0(self.win, objc_sel(c"windowNumber")) };
        n as u32
    }

    /// Pointer events on it go to `f` (until it is dropped).
    pub fn on_pointer(&self, f: impl FnMut(AuxEv) + 'static) {
        let f: AuxFn = Rc::new(RefCell::new(f));
        let me = self.win as usize;
        AUX.with(|a| {
            let mut a = a.borrow_mut();
            a.retain(|e| e.0 != me);
            a.push((me, self.frame.clone(), f));
        });
    }

    fn place(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.frame.set((x, y, w, h));
        let cocoa = CGRect {
            origin: CGPoint { x, y: crate::capture::main_display_height() - y - h },
            size: CGSize { width: w, height: h },
        };
        unsafe {
            let _: () = msg2(self.win, objc_sel(c"setFrame:display:"), cocoa, 0i8);
            if !self.shown {
                let _: () = msg0(self.win, objc_sel(c"orderFrontRegardless"));
                self.shown = true;
            }
        }
    }

    /// A solid color over (x, y, w, h) points.
    pub fn fill(&mut self, (x, y, w, h): (f64, f64, f64, f64), rgb: [u8; 3]) {
        unsafe {
            let c = |v: u8| f64::from(v) / 255.0;
            let col: *mut c_void =
                msg4(objc_cls(c"NSColor"), objc_sel(c"colorWithSRGBRed:green:blue:alpha:"), c(rgb[0]), c(rgb[1]), c(rgb[2]), 1.0f64);
            let _: () = msg1(self.win, objc_sel(c"setBackgroundColor:"), col);
            let _: () = msg1(self.win, objc_sel(c"setOpaque:"), 1i8);
        }
        self.place(x, y, w, h);
    }

    /// Show premultiplied BGRA pixels (`w`×`h`) with their top-left at
    /// (x, y) points, at `scale` pixels per point.
    pub fn put(&mut self, (x, y): (f64, f64), scale: f64, w: u32, h: u32, bgra: &[u8]) {
        let n = w as usize * h as usize * 4;
        if w == 0 || h == 0 || bgra.len() < n {
            self.hide();
            return;
        }
        let s = if scale > 0.0 { scale } else { 1.0 };
        unsafe {
            let mut px = bgra[..n].to_vec();
            let (len, cap, data) = (px.len(), px.capacity(), px.as_mut_ptr());
            std::mem::forget(px);
            let info = Box::into_raw(Box::new([len, cap])) as *mut c_void;
            let provider = CGDataProviderCreateWithData(info, data.cast(), len, release_pixels);
            if provider.is_null() {
                release_pixels(info, data.cast(), len);
                return;
            }
            let space = CGColorSpaceCreateDeviceRGB();
            // kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little:
            // B, G, R, A bytes, premultiplied.
            let image =
                CGImageCreate(w as usize, h as usize, 8, 32, w as usize * 4, space, 0x2002, provider, core::ptr::null(), false, 0);
            CGDataProviderRelease(provider);
            if !space.is_null() {
                CGColorSpaceRelease(space);
            }
            if image.is_null() {
                return;
            }
            // Size the window (and with it the layer) first, then give the
            // layer its picture: contents never shown stretched to the old
            // size.
            self.place(x, y, f64::from(w) / s, f64::from(h) / s);
            let view: *mut c_void = msg0(self.win, objc_sel(c"contentView"));
            let layer: *mut c_void = if view.is_null() { core::ptr::null_mut() } else { msg0(view, objc_sel(c"layer")) };
            if !layer.is_null() {
                let tx = objc_cls(c"CATransaction");
                let _: () = msg0(tx, objc_sel(c"begin"));
                let _: () = msg1(tx, objc_sel(c"setDisableActions:"), 1i8);
                let _: () = msg1(layer, objc_sel(c"setContentsScale:"), s);
                let _: () = msg1(layer, objc_sel(c"setContents:"), image);
                let _: () = msg0(tx, objc_sel(c"commit"));
            }
            CGImageRelease(image);
        }
        unsafe {
            // No display cycle runs: hand the new contents over now.
            let _: () = msg0(objc_cls(c"CATransaction"), objc_sel(c"flush"));
        }
    }

    pub fn hide(&mut self) {
        if self.shown {
            unsafe {
                let _: () = msg1(self.win, objc_sel(c"orderOut:"), core::ptr::null_mut::<c_void>());
            }
            self.shown = false;
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        let me = self.win as usize;
        AUX.with(|a| a.borrow_mut().retain(|e| e.0 != me));
        unsafe {
            let _: () = msg1(self.win, objc_sel(c"orderOut:"), core::ptr::null_mut::<c_void>());
            let _: () = msg0(self.win, objc_sel(c"close"));
            let _: () = msg0(self.win, objc_sel(c"release"));
        }
    }
}

#[cfg(test)]
mod wheel_tests {
    use super::wheel_units;

    #[test]
    fn wheel_lines_and_trackpad_points_become_notches() {
        assert_eq!(wheel_units(1.0, false, false), 120, "a wheel notch");
        assert_eq!(wheel_units(-3.0, false, false), -360);
        assert_eq!(wheel_units(10.0, true, false), 120, "ten trackpad points a line");
        assert_eq!(wheel_units(2.5, true, false), 30);
        assert_eq!(wheel_units(0.01, true, false), 1, "never zero for a scroll");
        assert_eq!(wheel_units(0.0, true, false), 0, "a horizontal-only scroll is none");
        assert_eq!(wheel_units(10.0, true, true), -120, "natural scrolling undone");
    }
}
