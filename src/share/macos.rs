//! macOS: `NSSharingServicePicker` with the file's URL, shown on the main
//! thread at the pointer (the toolbar button or menu row just clicked)
//! from a tiny clear window. A delegate says when the share ends: the
//! picker closed without a choice (`didChooseSharingService:` nil), or the
//! chosen service shared or failed. The caller (a worker) waits for that,
//! at most `max`; a process about to exit serves the main thread's events
//! meanwhile ([`serve`], from `share::wait_idle`), so a one-shot capture
//! shows the picker too. The anchor window is ordered out at the end.

use crate::capture::{CGPoint, CGRect, CGSize};
use crate::macos_objc::{Id, Pool, array, is_main_thread, make_class, new_obj, on_main};
use crate::wind::{msg0, msg1, msg2, msg3, msg4, ns_string, objc_cls, objc_sel};
use core::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::{Duration, Instant};

/// The anchor window, the picker and its delegate (one share at a time:
/// `share::Sheet`).
static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static PICKER: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static DELEGATE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// The share ended (or could not start).
static DONE: AtomicBool = AtomicBool::new(false);

/// `NSMinYEdge`: the picker opens below the anchor.
const MIN_Y_EDGE: usize = 1;

/// ObjC `YES` / `NO` (a signed char).
const YES: i8 = 1;
const NO: i8 = 0;

pub fn share(file: &Path, max: Duration) -> Result<(), String> {
    DONE.store(false, Ordering::SeqCst);
    let ctx = Box::into_raw(Box::new(file.to_path_buf())) as *mut c_void;
    on_main(ctx, show);
    let t0 = Instant::now();
    while !DONE.load(Ordering::SeqCst) && t0.elapsed() < max {
        std::thread::sleep(Duration::from_millis(50));
    }
    on_main(core::ptr::null_mut(), hide);
    Ok(())
}

/// Serve the main thread's events for `d` (the picker runs there) when
/// called on it; else just wait.
pub fn serve(d: Duration) {
    if !is_main_thread() {
        std::thread::sleep(d);
        return;
    }
    let _pool = Pool::new();
    unsafe {
        let app: *mut c_void = msg0(objc_cls(c"NSApplication"), objc_sel(c"sharedApplication"));
        let until: *mut c_void = msg1(objc_cls(c"NSDate"), objc_sel(c"dateWithTimeIntervalSinceNow:"), d.as_secs_f64());
        let mode = ns_string("kCFRunLoopDefaultMode");
        let ev: *mut c_void = msg4(app, objc_sel(c"nextEventMatchingMask:untilDate:mode:dequeue:"), u64::MAX, until, mode, YES);
        if !ev.is_null() {
            let _: () = msg1(app, objc_sel(c"sendEvent:"), ev);
        }
    }
}

extern "C" fn did_choose(_this: *mut c_void, _cmd: usize, _picker: *mut c_void, service: *mut c_void) {
    // No service: the picker was closed without a choice.
    if service.is_null() {
        DONE.store(true, Ordering::SeqCst);
    }
}

extern "C" fn delegate_for(this: *mut c_void, _cmd: usize, _picker: *mut c_void, _service: *mut c_void) -> *mut c_void {
    this
}

extern "C" fn did_share(_this: *mut c_void, _cmd: usize, _service: *mut c_void, _items: *mut c_void) {
    DONE.store(true, Ordering::SeqCst);
}

extern "C" fn did_fail(_this: *mut c_void, _cmd: usize, _service: *mut c_void, _items: *mut c_void, _err: *mut c_void) {
    DONE.store(true, Ordering::SeqCst);
}

/// The picker's (and the chosen service's) delegate, made once.
fn delegate() -> *mut c_void {
    let d = DELEGATE.load(Ordering::SeqCst);
    if !d.is_null() {
        return d;
    }
    let choose: extern "C" fn(*mut c_void, usize, *mut c_void, *mut c_void) = did_choose;
    let for_service: extern "C" fn(*mut c_void, usize, *mut c_void, *mut c_void) -> *mut c_void = delegate_for;
    let shared: extern "C" fn(*mut c_void, usize, *mut c_void, *mut c_void) = did_share;
    let failed: extern "C" fn(*mut c_void, usize, *mut c_void, *mut c_void, *mut c_void) = did_fail;
    let cls = make_class(
        c"RustshotShareDelegate",
        &[c"NSSharingServicePickerDelegate", c"NSSharingServiceDelegate"],
        &[
            (c"sharingServicePicker:didChooseSharingService:", choose as *const c_void, c"v@:@@"),
            (c"sharingServicePicker:delegateForSharingService:", for_service as *const c_void, c"@@:@@"),
            (c"sharingService:didShareItems:", shared as *const c_void, c"v@:@@"),
            (c"sharingService:didFailToShareItems:error:", failed as *const c_void, c"v@:@@@"),
        ],
    );
    // Kept for the process (the picker holds its delegate weakly).
    let obj = unsafe { new_obj(cls) }.map_or(core::ptr::null_mut(), |o: Id| core::mem::ManuallyDrop::new(o).ptr());
    DELEGATE.store(obj, Ordering::SeqCst);
    obj
}

extern "C" fn show(ctx: *mut c_void) {
    // SAFETY: `ctx` is the box made by `share`, handed over once.
    let path = unsafe { Box::from_raw(ctx as *mut PathBuf) };
    let _pool = Pool::new();
    if !unsafe { open_picker(&path) } {
        DONE.store(true, Ordering::SeqCst);
    }
}

/// Show the picker for `path` at the pointer; false when it could not.
unsafe fn open_picker(path: &Path) -> bool {
    unsafe {
        let url: *mut c_void = msg1(objc_cls(c"NSURL"), objc_sel(c"fileURLWithPath:"), ns_string(&path.to_string_lossy()));
        if url.is_null() {
            return false;
        }
        let items = array(&[url]);
        let picker: *mut c_void = msg0(objc_cls(c"NSSharingServicePicker"), objc_sel(c"alloc"));
        let picker: *mut c_void = msg1(picker, objc_sel(c"initWithItems:"), items);
        if picker.is_null() {
            return false;
        }
        let d = delegate();
        if !d.is_null() {
            let _: () = msg1(picker, objc_sel(c"setDelegate:"), d);
        }
        // The pointer (screen points, y up), the anchor 2 x 2 points there.
        let at: CGPoint = msg0(objc_cls(c"NSEvent"), objc_sel(c"mouseLocation"));
        let frame = CGRect { origin: CGPoint { x: at.x - 1.0, y: at.y - 1.0 }, size: CGSize { width: 2.0, height: 2.0 } };
        let mut win = WINDOW.load(Ordering::SeqCst);
        if win.is_null() {
            win = msg0(objc_cls(c"NSWindow"), objc_sel(c"alloc"));
            win = msg4(win, objc_sel(c"initWithContentRect:styleMask:backing:defer:"), frame, 0u64, 2u64, NO);
            if win.is_null() {
                let _: () = msg0(picker, objc_sel(c"release"));
                return false;
            }
            let _: () = msg1(win, objc_sel(c"setReleasedWhenClosed:"), NO);
            let _: () = msg1(win, objc_sel(c"setOpaque:"), NO);
            let clear: *mut c_void = msg0(objc_cls(c"NSColor"), objc_sel(c"clearColor"));
            let _: () = msg1(win, objc_sel(c"setBackgroundColor:"), clear);
            let _: () = msg1(win, objc_sel(c"setIgnoresMouseEvents:"), YES);
            let _: () = msg1(win, objc_sel(c"setLevel:"), 3i64); // floating
            WINDOW.store(win, Ordering::SeqCst);
        } else {
            let _: () = msg2(win, objc_sel(c"setFrame:display:"), frame, NO);
        }
        let _: () = msg0(win, objc_sel(c"orderFrontRegardless"));
        let view: *mut c_void = msg0(win, objc_sel(c"contentView"));
        let anchor = CGRect { origin: CGPoint::default(), size: CGSize { width: 2.0, height: 2.0 } };
        let _: () = msg3(picker, objc_sel(c"showRelativeToRect:ofView:preferredEdge:"), anchor, view, MIN_Y_EDGE);
        let old = PICKER.swap(picker, Ordering::SeqCst);
        if !old.is_null() {
            let _: () = msg0(old, objc_sel(c"release"));
        }
        true
    }
}

/// The share is over: the anchor window goes off screen.
extern "C" fn hide(_ctx: *mut c_void) {
    let win = WINDOW.load(Ordering::SeqCst);
    if !win.is_null() {
        unsafe {
            let _: () = msg1(win, objc_sel(c"orderOut:"), core::ptr::null_mut::<c_void>());
        }
    }
}
