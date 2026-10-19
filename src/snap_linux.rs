//! Window snapping on X11: the window manager's stacking list
//! (`_NET_CLIENT_LIST_STACKING`, each client with its frame:
//! `_NET_FRAME_EXTENTS` added, `_GTK_FRAME_EXTENTS` shadows taken off)
//! plus the mapped override-redirect windows (menus, tooltips) on top;
//! without a window manager, the mapped children of the root in stacking
//! order. Parts are mapped subwindows (`XQueryTree`; many toolkits draw
//! everything in one window, which then has none). Wayland: no window
//! positions, so no snapping (`None`).
//!
//! Everything runs on a private Display inside `wind::x_guarded`: a window
//! that goes away halfway is skipped, never fatal.

use super::{x11_rect, Node, MAX_DEPTH, MAX_NODES, R};
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::atomic::Ordering;

#[repr(C)]
struct XWindowAttributes {
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    depth: c_int,
    visual: *mut c_void,
    root: c_ulong,
    class: c_int,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: c_int,
    colormap: c_ulong,
    map_installed: c_int,
    map_state: c_int,
    all_event_masks: c_long,
    your_event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: c_int,
    screen: *mut c_void,
}

// Same Xlib symbols as wind/capture declare (see wind_linux.rs).
#[allow(clashing_extern_declarations)]
#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(dpy: *mut c_void) -> c_int;
    fn XDefaultScreen(dpy: *mut c_void) -> c_int;
    fn XRootWindow(dpy: *mut c_void, screen: c_int) -> c_ulong;
    fn XInternAtom(dpy: *mut c_void, name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XFree(data: *mut c_void) -> c_int;
    fn XGetWindowProperty(
        dpy: *mut c_void,
        w: c_ulong,
        property: c_ulong,
        long_offset: c_long,
        long_length: c_long,
        delete: c_int,
        req_type: c_ulong,
        actual_type: *mut c_ulong,
        actual_format: *mut c_int,
        nitems: *mut c_ulong,
        bytes_after: *mut c_ulong,
        prop: *mut *mut u8,
    ) -> c_int;
    fn XQueryTree(
        dpy: *mut c_void,
        w: c_ulong,
        root: *mut c_ulong,
        parent: *mut c_ulong,
        children: *mut *mut c_ulong,
        n: *mut c_uint,
    ) -> c_int;
    fn XGetWindowAttributes(dpy: *mut c_void, w: c_ulong, attrs: *mut XWindowAttributes) -> c_int;
    fn XTranslateCoordinates(
        dpy: *mut c_void,
        src: c_ulong,
        dst: c_ulong,
        x: c_int,
        y: c_int,
        dx: *mut c_int,
        dy: *mut c_int,
        child: *mut c_ulong,
    ) -> c_int;
}

const IS_VIEWABLE: c_int = 2;
const ANY_PROPERTY_TYPE: c_ulong = 0;

struct X {
    dpy: *mut c_void,
    root: c_ulong,
    pid: c_ulong,
    net_frame: c_ulong,
    gtk_frame: c_ulong,
    wm_type: c_ulong,
    desktop: c_ulong,
    own_pid: Option<u32>,
    overlay: c_ulong,
    budget: usize,
}

impl X {
    /// A property as C longs (format 32), `None` when unset.
    unsafe fn longs(&self, w: c_ulong, prop: c_ulong, max: c_long) -> Option<Vec<c_long>> {
        if prop == 0 {
            return None;
        }
        let (mut ty, mut fmt, mut n, mut after) = (0 as c_ulong, 0 as c_int, 0 as c_ulong, 0 as c_ulong);
        let mut data: *mut u8 = core::ptr::null_mut();
        let ok = unsafe {
            XGetWindowProperty(self.dpy, w, prop, 0, max, 0, ANY_PROPERTY_TYPE, &mut ty, &mut fmt, &mut n, &mut after, &mut data)
        };
        if ok != 0 || data.is_null() {
            return None;
        }
        let out = (fmt == 32).then(|| unsafe { core::slice::from_raw_parts(data as *const c_long, n as usize) }.to_vec());
        unsafe { XFree(data.cast()) };
        out.filter(|v| !v.is_empty())
    }

    unsafe fn extents(&self, w: c_ulong, prop: c_ulong) -> Option<[i32; 4]> {
        let v = unsafe { self.longs(w, prop, 4) }?;
        (v.len() >= 4).then(|| [v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32])
    }

    unsafe fn attrs(&self, w: c_ulong) -> Option<XWindowAttributes> {
        let mut a: XWindowAttributes = unsafe { core::mem::zeroed() };
        (unsafe { XGetWindowAttributes(self.dpy, w, &mut a) } != 0).then_some(a)
    }

    unsafe fn children(&self, w: c_ulong) -> Vec<c_ulong> {
        let (mut root, mut parent, mut kids, mut n) = (0, 0, core::ptr::null_mut(), 0 as c_uint);
        if unsafe { XQueryTree(self.dpy, w, &mut root, &mut parent, &mut kids, &mut n) } == 0 || kids.is_null() {
            return Vec::new();
        }
        let v = unsafe { core::slice::from_raw_parts(kids, n as usize) }.to_vec();
        unsafe { XFree(kids.cast()) };
        v
    }

    /// Where `w`'s origin (inside its border) is on the root.
    unsafe fn origin(&self, w: c_ulong) -> Option<(i32, i32)> {
        let (mut x, mut y, mut child) = (0, 0, 0);
        (unsafe { XTranslateCoordinates(self.dpy, w, self.root, 0, 0, &mut x, &mut y, &mut child) } != 0).then_some((x, y))
    }

    /// One of ours, or the desktop.
    unsafe fn skip(&self, w: c_ulong) -> bool {
        if w == self.overlay {
            return true;
        }
        let own = self.own_pid.is_some_and(|p| unsafe { self.longs(w, self.pid, 1) }.is_some_and(|v| v[0] as u32 == p));
        let desktop = self.desktop != 0 && unsafe { self.longs(w, self.wm_type, 8) }.is_some_and(|v| v.contains(&(self.desktop as c_long)));
        own || desktop
    }

    /// The mapped subwindows of `w` (inside origin `o` on the root),
    /// front to back.
    unsafe fn parts(&mut self, w: c_ulong, o: (i32, i32), depth: usize) -> Vec<Node> {
        if depth > MAX_DEPTH || self.budget == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for k in unsafe { self.children(w) }.into_iter().rev() {
            if self.budget == 0 {
                break;
            }
            let Some(a) = (unsafe { self.attrs(k) }).filter(|a| a.map_state == IS_VIEWABLE) else { continue };
            let bw = a.border_width;
            let r = R::xywh(o.0 + a.x, o.1 + a.y, a.width + 2 * bw, a.height + 2 * bw);
            if r.is_empty() {
                continue;
            }
            self.budget -= 1;
            let inner = (o.0 + a.x + bw, o.1 + a.y + bw);
            let kids = unsafe { self.parts(k, inner, depth + 1) };
            out.push(Node::new(r, kids));
        }
        out
    }

    /// A managed client: its rect with the frame, and its parts.
    unsafe fn client(&mut self, w: c_ulong) -> Option<Node> {
        let a = unsafe { self.attrs(w) }.filter(|a| a.map_state == IS_VIEWABLE)?;
        if unsafe { self.skip(w) } {
            return None;
        }
        let o = unsafe { self.origin(w) }?;
        let client = R::xywh(o.0, o.1, a.width, a.height);
        let r = x11_rect(client, unsafe { self.extents(w, self.net_frame) }, unsafe { self.extents(w, self.gtk_frame) });
        let kids = unsafe { self.parts(w, o, 1) };
        Some(Node::new(r, kids))
    }

    /// A child of the root as it is (no window manager, or a popup).
    unsafe fn bare(&mut self, w: c_ulong, a: &XWindowAttributes) -> Option<Node> {
        if unsafe { self.skip(w) } {
            return None;
        }
        let bw = a.border_width;
        let outer = R::xywh(a.x, a.y, a.width + 2 * bw, a.height + 2 * bw);
        let r = x11_rect(outer, None, unsafe { self.extents(w, self.gtk_frame) });
        let kids = unsafe { self.parts(w, (a.x + bw, a.y + bw), 1) };
        Some(Node::new(r, kids))
    }

    unsafe fn list(&mut self) -> Vec<Node> {
        let stacking = unsafe { XInternAtom(self.dpy, c"_NET_CLIENT_LIST_STACKING".as_ptr(), 1) };
        let clients = unsafe { self.longs(self.root, stacking, 1 << 16) }.unwrap_or_default();
        let tree = unsafe { self.children(self.root) };
        let mut out = Vec::new();
        // Front first: the root's children and the stacking list are
        // bottom to top.
        for &w in tree.iter().rev() {
            let Some(a) = (unsafe { self.attrs(w) }).filter(|a| a.map_state == IS_VIEWABLE) else { continue };
            if !clients.is_empty() && a.override_redirect == 0 {
                continue; // a frame: its client comes from the stacking list
            }
            out.extend(unsafe { self.bare(w, &a) });
        }
        for &c in clients.iter().rev() {
            out.extend(unsafe { self.client(c as c_ulong) });
        }
        out
    }
}

/// Nothing to load ahead here.
pub fn warm() {}

/// X11 and Wayland have no "hidden from capture" window flag.
pub fn hidden_tops(_own_pid: Option<u32>, _within: R) -> Option<Vec<crate::hidden::Top>> {
    None
}

/// Every window that counts, front to back, with its parts; `own_pid`'s
/// windows (and the overlay) left out. `None` on Wayland or without an X
/// display.
pub fn windows(own_pid: Option<u32>, within: R) -> Option<Vec<Node>> {
    // The shot clips them (`Snapshot::build`); nothing is skipped early here.
    let _ = within;
    if crate::rec::linux::portal::wayland_now() {
        return None;
    }
    crate::wind::init_x11();
    unsafe {
        let dpy = XOpenDisplay(core::ptr::null());
        if dpy.is_null() {
            return None;
        }
        let atom = |n: &core::ffi::CStr| XInternAtom(dpy, n.as_ptr(), 1);
        let mut x = X {
            dpy,
            root: XRootWindow(dpy, XDefaultScreen(dpy)),
            pid: atom(c"_NET_WM_PID"),
            net_frame: atom(c"_NET_FRAME_EXTENTS"),
            gtk_frame: atom(c"_GTK_FRAME_EXTENTS"),
            wm_type: atom(c"_NET_WM_WINDOW_TYPE"),
            desktop: atom(c"_NET_WM_WINDOW_TYPE_DESKTOP"),
            own_pid,
            overlay: crate::wind::MAIN_WINDOW.load(Ordering::SeqCst),
            budget: MAX_NODES,
        };
        let (list, _errors) = crate::wind::x_guarded(dpy, || x.list());
        XCloseDisplay(dpy);
        Some(list)
    }
}
