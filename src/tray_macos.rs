//! The macOS menu-bar icon: an `NSStatusItem` with the app icon (the blue
//! tile, the red dot while recording) and an `NSMenu` of the shared model
//! (`tray::menu`), rebuilt each time it opens. Everything runs on the main thread:
//! the calls here queue their work on the main queue, which the overlay's
//! pump serves.
#![cfg_attr(test, allow(dead_code))] // tests never put an icon in the menu bar

use crate::hotkey::HotEvent;
use crate::macos_objc::{self as oc, Id, Pool};
use crate::tray::{self, MenuItem};
use crate::wind::{msg0, msg1, msg2, msg3, ns_string, objc_cls, objc_sel};
use core::ffi::c_void;
use std::cell::RefCell;
use std::sync::Mutex;
use crate::chan::Sender;

static TX: Mutex<Option<Sender<HotEvent>>> = Mutex::new(None);

/// The status item and its menu (main thread only).
struct State {
    bar: *mut c_void,
    item: Id,
    menu: Id,
    target: Id,
    /// The menu's items now (an `NSMenuItem`'s tag indexes them).
    items: Vec<MenuItem>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The glyph's size in points (drawn at 2x for Retina).
const PT: f64 = 18.0;

#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    w: f64,
    h: f64,
}

/// Start the icon (once the main thread's pump runs); events go to `tx`.
pub fn spawn(tx: Sender<HotEvent>) {
    *TX.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    oc::on_main(core::ptr::null_mut(), refresh_main);
}

/// The recording state or `show_tray` changed: the icon follows.
pub fn refresh() {
    if TX.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        oc::on_main(core::ptr::null_mut(), refresh_main);
    }
}

/// The recorded time changed: the tooltip says it.
pub fn refresh_tip() {
    if TX.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        oc::on_main(core::ptr::null_mut(), tip_main);
    }
}

extern "C" fn refresh_main(_: *mut c_void) {
    let _pool = Pool::new();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        match (tray::is_visible(), s.is_some()) {
            (true, false) => *s = unsafe { create() },
            (false, true) => {
                if let Some(st) = s.take() {
                    unsafe {
                        let _: () = msg1(st.bar, objc_sel(c"removeStatusItem:"), st.item.ptr());
                    }
                }
            }
            _ => {}
        }
        if let Some(st) = s.as_ref() {
            unsafe {
                set_image(st);
                set_tip(st);
            }
        }
    });
}

extern "C" fn tip_main(_: *mut c_void) {
    let _pool = Pool::new();
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            unsafe { set_tip(st) }
        }
    });
}

unsafe fn button(st: &State) -> *mut c_void {
    unsafe { msg0(st.item.ptr(), objc_sel(c"button")) }
}

unsafe fn create() -> Option<State> {
    unsafe {
        let bar: *mut c_void = msg0(objc_cls(c"NSStatusBar"), objc_sel(c"systemStatusBar"));
        if bar.is_null() {
            eprintln!("rustshot: no menu bar (NSStatusBar) for the tray icon");
            return None;
        }
        // NSVariableStatusItemLength.
        let item = Id::retain(msg1(bar, objc_sel(c"statusItemWithLength:"), -1.0f64))?;
        let menu = oc::new_obj(objc_cls(c"NSMenu"))?;
        let _: () = msg1(menu.ptr(), objc_sel(c"setAutoenablesItems:"), 0i8);
        let cls = oc::make_class(
            c"RustshotTrayTarget",
            &[c"NSMenuDelegate"],
            &[
                (c"act:", act as *const c_void, c"v@:@"),
                (c"menuNeedsUpdate:", needs_update as *const c_void, c"v@:@"),
            ],
        );
        let target = oc::new_obj(cls)?;
        let _: () = msg1(menu.ptr(), objc_sel(c"setDelegate:"), target.ptr());
        let _: () = msg1(item.ptr(), objc_sel(c"setMenu:"), menu.ptr());
        let st = State { bar, item, menu, target, items: Vec::new() };
        Some(st)
    }
}

/// The icon as a PNG, `n`x`n` px.
fn icon_png(n: u32, recording: bool) -> Option<Vec<u8>> {
    let rgba = tray::tray_icon_rgba(n, recording);
    crate::pngfile::encode(n, n, crate::pngfile::Color::Rgba, &rgba).ok()
}

unsafe fn set_image(st: &State) {
    unsafe {
        let b = button(st);
        if b.is_null() {
            return;
        }
        let Some(png) = icon_png((PT * 2.0) as u32, tray::is_recording()) else { return };
        let data: *mut c_void = msg0::<*mut c_void>(objc_cls(c"NSData"), objc_sel(c"alloc"));
        let data: *mut c_void = msg2(data, objc_sel(c"initWithBytes:length:"), png.as_ptr(), png.len());
        let Some(data) = Id::owned(data) else { return };
        let img: *mut c_void = msg0(objc_cls(c"NSImage"), objc_sel(c"alloc"));
        let Some(img) = Id::owned(msg1(img, objc_sel(c"initWithData:"), data.ptr())) else { return };
        let _: () = msg1(img.ptr(), objc_sel(c"setSize:"), NSSize { w: PT, h: PT });
        let _: () = msg1(b, objc_sel(c"setImage:"), img.ptr());
    }
}

unsafe fn set_tip(st: &State) {
    unsafe {
        let b = button(st);
        if !b.is_null() {
            let _: () = msg1(b, objc_sel(c"setToolTip:"), ns_string(&tray::tip_text()));
        }
    }
}

/// Fill `menu` from the model (it is about to open).
unsafe fn rebuild(st: &mut State) {
    unsafe {
        let menu = st.menu.ptr();
        let _: () = msg0(menu, objc_sel(c"removeAllItems"));
        st.items = tray::current_menu();
        // Tags are the flat list's indexes (submenu items included).
        let add = |menu: *mut c_void, i: usize, it: MenuItem| -> Option<Id> {
            if it == MenuItem::Separator {
                let sep: *mut c_void = msg0(objc_cls(c"NSMenuItem"), objc_sel(c"separatorItem"));
                let _: () = msg1(menu, objc_sel(c"addItem:"), sep);
                return None;
            }
            let mi: *mut c_void = msg0(objc_cls(c"NSMenuItem"), objc_sel(c"alloc"));
            let mi: *mut c_void =
                msg3(mi, objc_sel(c"initWithTitle:action:keyEquivalent:"), ns_string(&it.label()), objc_sel(c"act:"), ns_string(""));
            let mi = Id::owned(mi)?;
            let _: () = msg1(mi.ptr(), objc_sel(c"setTarget:"), st.target.ptr());
            let _: () = msg1(mi.ptr(), objc_sel(c"setTag:"), i as isize);
            if it.checked() == Some(true) {
                let _: () = msg1(mi.ptr(), objc_sel(c"setState:"), 1isize); // NSControlStateValueOn
            }
            let _: () = msg1(menu, objc_sel(c"addItem:"), mi.ptr());
            Some(mi)
        };
        let items = st.items.clone();
        for (i, kids) in tray::nest(&items) {
            let Some(mi) = add(menu, i, items[i]) else { continue };
            if kids.is_empty() {
                continue;
            }
            // "Recent uploads": its submenu (the item keeps it).
            let sub: *mut c_void = msg0(objc_cls(c"NSMenu"), objc_sel(c"alloc"));
            let sub: *mut c_void = msg1(sub, objc_sel(c"initWithTitle:"), ns_string(&items[i].label()));
            let Some(sub) = Id::owned(sub) else { continue };
            for k in kids {
                add(sub.ptr(), k, items[k]);
            }
            let _: () = msg1(mi.ptr(), objc_sel(c"setSubmenu:"), sub.ptr());
        }
    }
}

unsafe extern "C" fn needs_update(_this: *mut c_void, _cmd: usize, _menu: *mut c_void) {
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            unsafe { rebuild(st) }
        }
    });
}

unsafe extern "C" fn act(_this: *mut c_void, _cmd: usize, sender: *mut c_void) {
    if sender.is_null() {
        return;
    }
    let tag: isize = unsafe { msg0(sender, objc_sel(c"tag")) };
    let item = STATE.with(|s| s.borrow().as_ref().and_then(|st| st.items.get(tag as usize).copied()));
    let Some(item) = item else { return };
    let Some(tx) = TX.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
    if let Err(e) = tray::activate(item, &tx) {
        tray::notify(&e);
    }
}
