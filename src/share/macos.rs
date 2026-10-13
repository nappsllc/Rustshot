//! macOS: `NSSharingServicePicker` with the file's URL, shown on the main
//! thread at the pointer (the toolbar button or menu row just clicked)
//! from a tiny clear window. The picker runs on its own; the call returns
//! once it is up. The last window and picker are kept (one of each) until
//! the next share.

use crate::capture::{CGPoint, CGRect, CGSize};
use crate::macos_objc::{Pool, array, on_main};
use crate::wind::{msg0, msg1, msg3, msg4, ns_string, objc_cls, objc_sel};
use core::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicPtr, Ordering};

/// The anchor window and the picker of the last share.
static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static PICKER: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// `NSMinYEdge`: the picker opens below the anchor.
const MIN_Y_EDGE: usize = 1;

pub fn share(file: &Path) -> Result<(), String> {
    let ctx = Box::into_raw(Box::new(file.to_path_buf())) as *mut c_void;
    on_main(ctx, show);
    Ok(())
}

extern "C" fn show(ctx: *mut c_void) {
    // SAFETY: `ctx` is the box made by `share`, handed over once.
    let path = unsafe { Box::from_raw(ctx as *mut PathBuf) };
    let _pool = Pool::new();
    unsafe {
        let url: *mut c_void = msg1(objc_cls(c"NSURL"), objc_sel(c"fileURLWithPath:"), ns_string(&path.to_string_lossy()));
        if url.is_null() {
            return;
        }
        let items = array(&[url]);
        let picker: *mut c_void = msg0(objc_cls(c"NSSharingServicePicker"), objc_sel(c"alloc"));
        let picker: *mut c_void = msg1(picker, objc_sel(c"initWithItems:"), items);
        if picker.is_null() {
            return;
        }
        // The pointer (screen points, y up), the anchor 2 x 2 points there.
        let at: CGPoint = msg0(objc_cls(c"NSEvent"), objc_sel(c"mouseLocation"));
        let frame = CGRect { origin: CGPoint { x: at.x - 1.0, y: at.y - 1.0 }, size: CGSize { width: 2.0, height: 2.0 } };
        let mut win = WINDOW.load(Ordering::SeqCst);
        if win.is_null() {
            win = msg0(objc_cls(c"NSWindow"), objc_sel(c"alloc"));
            win = msg4(win, objc_sel(c"initWithContentRect:styleMask:backing:defer:"), frame, 0u64, 2u64, 0i8);
            if win.is_null() {
                let _: () = msg0(picker, objc_sel(c"release"));
                return;
            }
            let _: () = msg1(win, objc_sel(c"setReleasedWhenClosed:"), 0i64);
            let _: () = msg1(win, objc_sel(c"setOpaque:"), 0i64);
            let clear: *mut c_void = msg0(objc_cls(c"NSColor"), objc_sel(c"clearColor"));
            let _: () = msg1(win, objc_sel(c"setBackgroundColor:"), clear);
            let _: () = msg1(win, objc_sel(c"setIgnoresMouseEvents:"), 1i64);
            let _: () = msg1(win, objc_sel(c"setLevel:"), 3i64); // floating
            WINDOW.store(win, Ordering::SeqCst);
        } else {
            msg2_frame(win, frame);
        }
        let _: () = msg0(win, objc_sel(c"orderFrontRegardless"));
        let view: *mut c_void = msg0(win, objc_sel(c"contentView"));
        let anchor = CGRect { origin: CGPoint::default(), size: CGSize { width: 2.0, height: 2.0 } };
        let _: () = msg3(picker, objc_sel(c"showRelativeToRect:ofView:preferredEdge:"), anchor, view, MIN_Y_EDGE);
        let old = PICKER.swap(picker, Ordering::SeqCst);
        if !old.is_null() {
            let _: () = msg0(old, objc_sel(c"release"));
        }
    }
}

/// `-[NSWindow setFrame:display:]` (`display` NO).
unsafe fn msg2_frame(win: *mut c_void, frame: CGRect) {
    unsafe {
        let _: () = crate::wind::msg2(win, objc_sel(c"setFrame:display:"), frame, 0i8);
    }
}
