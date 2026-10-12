//! X11: the pins on their own thread and Display connection, each a
//! managed toplevel without decorations (`_MOTIF_WM_HINTS`) kept above
//! other windows (`_NET_WM_STATE_ABOVE`), skipped by the taskbar and the
//! pager (see `rec_ui::x11::Kind::Managed`). Override-redirect would be
//! wrong here: such a window never gets the keyboard (Esc), and the
//! window manager could never stack or move it. The position asked for
//! goes in `WM_NORMAL_HINTS` (`USPosition`), so the window manager keeps
//! it where it was captured.
//!
//! A drag moves the window (on Wayland, under XWayland, the compositor
//! moves it: `_NET_WM_MOVERESIZE`), buttons 4/5 zoom it about the pointer,
//! with Ctrl the opacity (an ARGB window's pixels, so it needs a
//! compositor), a double-click or Esc closes it, Ctrl+C / Ctrl+S copy and
//! save it, and button 3 opens the drawn menu ([`super::render_menu`]) in
//! an override-redirect window that grabs the pointer until a choice or a
//! click elsewhere.

use super::{apply, dragged, is_double, is_drag, menu, menu_at, menu_layout, render, render_menu, resample, scaled_size, zoom_about, zoom_step, Cmd as PinCmd, Frame, MenuLayout, View};
use crate::objects::Pt;
use crate::pixbuf::PixBuf;
use crate::rec_ui::x11::{self as x, Disp, Kind, Win};
use crate::theme::Theme;
use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The windows' name (what `xprop`/`xwininfo` and the Docker harness see).
pub const TITLE: &str = "Rustshot pin";
const TICK: Duration = Duration::from_millis(16);
const REPLY: Duration = Duration::from_millis(1000);

const KEY_PRESS: c_int = 2;
const BUTTON1_MASK: c_uint = 1 << 8;
const CONTROL_MASK: c_uint = 1 << 2;
const SHIFT_MASK: c_uint = 1 << 0;

/// Tests: a pin's window, view and origin.
#[cfg(test)]
type Shown = (c_ulong, View, (i32, i32));

enum Cmd {
    Open(Arc<PixBuf>, (i32, i32), Theme, Sender<bool>),
    CloseAll(Sender<()>),
    /// Tests: each pin's window, view and origin.
    #[cfg(test)]
    Query(Sender<Vec<Shown>>),
    /// Tests: wheel `notches` (Ctrl: `ctrl`) at `at` over pin `i`.
    #[cfg(test)]
    Wheel(usize, i32, bool, (i32, i32), Sender<()>),
}

static TX: OnceLock<Option<Mutex<Sender<Cmd>>>> = OnceLock::new();

fn sender() -> Option<Sender<Cmd>> {
    let tx = TX.get_or_init(|| {
        std::env::var_os("DISPLAY")?;
        let (tx, rx) = mpsc::channel();
        let (ok_tx, ok_rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("pins".into()).spawn(move || run(rx, ok_tx));
        (spawned.is_ok() && ok_rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false)).then(|| Mutex::new(tx))
    });
    tx.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

fn ask<T>(make: impl FnOnce(Sender<T>) -> Cmd, dflt: T) -> T {
    let Some(tx) = sender() else { return dflt };
    let (rtx, rrx) = mpsc::channel();
    if tx.send(make(rtx)).is_err() {
        return dflt;
    }
    rrx.recv_timeout(REPLY).unwrap_or(dflt)
}

pub fn open(img: Arc<PixBuf>, at: (i32, i32), th: Theme) -> bool {
    ask(|r| Cmd::Open(img, at, th, r), false)
}

pub fn close_all() {
    if TX.get().is_some_and(|t| t.is_some()) {
        ask(Cmd::CloseAll, ());
    }
}

struct Pin {
    win: Win,
    img: Arc<PixBuf>,
    th: Theme,
    s: f32,
    view: View,
    /// The image's top-left on screen (physical px).
    origin: (i32, i32),
    /// The window's pixels at the zoom (fully opaque) and their size.
    px: Vec<u8>,
    size: (u32, u32),
    /// A press: the pointer (root px) and the origin then.
    press: Option<((i32, i32), (i32, i32))>,
    moved: bool,
    /// The last press (time, root px), for a double-click.
    last: Option<(Instant, (i32, i32))>,
}

impl Pin {
    fn zoomed(&self) -> (u32, u32) {
        scaled_size(self.img.dimensions(), self.view.zoom)
    }

    fn centre(&self) -> (i32, i32) {
        let (w, h) = self.zoomed();
        (self.origin.0 + w as i32 / 2, self.origin.1 + h as i32 / 2)
    }

    fn redraw(&mut self) {
        let (w, h) = self.zoomed();
        let (ww, wh, px) =
            if (w, h) == self.img.dimensions() { render(&self.img, &self.th, self.s) } else { render(&resample(&self.img, w, h), &self.th, self.s) };
        self.px = px;
        self.size = (ww, wh);
        self.show();
    }

    /// The pixels at the opacity, where the origin says.
    fn show(&mut self) {
        let (x, y) = Frame::at(self.s).window_pos(self.origin);
        let a = self.view.alpha() as u32;
        if a == 255 {
            self.win.put(x, y, self.size.0, self.size.1, &self.px);
        } else {
            let faded: Vec<u8> = self.px.iter().map(|&c| ((c as u32 * a + 127) / 255) as u8).collect();
            self.win.put(x, y, self.size.0, self.size.1, &faded);
        }
    }

    fn place(&mut self) {
        let (x, y) = Frame::at(self.s).window_pos(self.origin);
        self.win.place(x, y, self.size.0, self.size.1);
    }

    fn set_view(&mut self, v: View, about: (i32, i32)) {
        let z0 = self.view.zoom;
        self.view = v;
        if v.zoom != z0 {
            self.origin = zoom_about(self.origin, about, z0, v.zoom);
            self.redraw();
        } else {
            self.show();
        }
    }

    fn wheel(&mut self, notches: i32, ctrl: bool, at: (i32, i32)) {
        let v = if ctrl {
            View { opacity: super::opacity_step(self.view.opacity, notches), ..self.view }
        } else {
            View { zoom: zoom_step(self.view.zoom, notches), ..self.view }
        };
        if v != self.view {
            self.set_view(v, at);
        }
    }
}

/// The open menu: its window, layout, the pin it is for and the row lit.
struct Menu {
    win: Win,
    l: MenuLayout,
    pin: c_ulong,
    at: (i32, i32),
    hover: Option<PinCmd>,
}

struct State {
    d: Disp,
    pins: Vec<Pin>,
    menu: Option<Menu>,
    /// Keycodes of Esc, C and S.
    keys: (c_uint, c_uint, c_uint),
}

fn run(rx: Receiver<Cmd>, ok: Sender<bool>) {
    let d = match Disp::open() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rustshot: no pin windows: {e:#}");
            let _ = ok.send(false);
            return;
        }
    };
    // SAFETY: an open Display.
    let keys = unsafe { (XKeysymToKeycode(d.dpy, 0xFF1B) as c_uint, XKeysymToKeycode(d.dpy, 0x63) as c_uint, XKeysymToKeycode(d.dpy, 0x73) as c_uint) };
    let _ = ok.send(true);
    let mut st = State { d, pins: Vec::new(), menu: None, keys };
    loop {
        let cmd = if st.pins.is_empty() {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(TICK) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        if let Some(c) = cmd {
            st.handle(c);
            while let Ok(c) = rx.try_recv() {
                st.handle(c);
            }
        }
        st.events();
    }
}

impl State {
    fn report(&self) {
        super::set_count(self.pins.len());
    }

    fn handle(&mut self, c: Cmd) {
        match c {
            Cmd::Open(img, at, th, r) => {
                let _ = r.send(self.open(img, at, th));
            }
            Cmd::CloseAll(r) => {
                self.menu = None;
                self.pins.clear();
                x::flush(&self.d);
                self.report();
                let _ = r.send(());
            }
            #[cfg(test)]
            Cmd::Query(r) => {
                let _ = r.send(self.pins.iter().map(|p| (p.win.win, p.view, p.origin)).collect());
            }
            #[cfg(test)]
            Cmd::Wheel(i, n, ctrl, at, r) => {
                if let Some(p) = self.pins.get_mut(i) {
                    p.wheel(n, ctrl, at);
                }
                let _ = r.send(());
            }
        }
    }

    fn open(&mut self, img: Arc<PixBuf>, at: (i32, i32), th: Theme) -> bool {
        let win = match Win::with_kind(&self.d, true, None, Kind::Managed(TITLE)) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("rustshot: no pin window: {e:#}");
                return false;
            }
        };
        let s = scale(&self.d);
        let mut pin = Pin { win, img, th, s, view: View::default(), origin: at, px: Vec::new(), size: (0, 0), press: None, moved: false, last: None };
        let (w, h) = Frame::at(s).window(pin.zoomed());
        let (wx, wy) = Frame::at(s).window_pos(at);
        // SAFETY: an open Display and its new window.
        unsafe {
            XSelectInput(
                self.d.dpy,
                pin.win.win,
                KEY_PRESS_MASK | BUTTON_PRESS_MASK | BUTTON_RELEASE_MASK | POINTER_MOTION_MASK | EXPOSURE_MASK | STRUCTURE_NOTIFY_MASK,
            );
            let mut hints: XSizeHints = core::mem::zeroed();
            hints.flags = US_POSITION | P_POSITION;
            (hints.x, hints.y, hints.width, hints.height) = (wx, wy, w as c_int, h as c_int);
            XSetWMNormalHints(self.d.dpy, pin.win.win, &mut hints);
        }
        pin.redraw();
        self.pins.push(pin);
        self.report();
        true
    }

    fn close(&mut self, win: c_ulong) {
        self.pins.retain(|p| p.win.win != win);
        x::flush(&self.d);
        self.report();
    }

    fn events(&mut self) {
        while let Some(ev) = x::next_event(&self.d) {
            // SAFETY: every event starts with its type.
            let ty = unsafe { ev.type_ };
            if ty == x::EV_CONFIGURE {
                // SAFETY: a ConfigureNotify.
                let c = unsafe { ev.configure };
                self.moved_to(c.window);
                continue;
            }
            if ty == x::EV_EXPOSE {
                // SAFETY: Expose starts like a pointer event (type, serial,
                // send_event, display, window).
                let w = unsafe { ev.pointer.window };
                if let Some(p) = self.pins.iter_mut().find(|p| p.win.win == w) {
                    p.show();
                }
                continue;
            }
            if ![KEY_PRESS, x::EV_BUTTON_PRESS, x::EV_BUTTON_RELEASE, x::EV_MOTION].contains(&ty) {
                continue;
            }
            // SAFETY: a pointer or key event (the same head).
            let e = unsafe { ev.pointer };
            if self.menu.is_some() {
                self.menu_event(ty, &e);
                continue;
            }
            match ty {
                KEY_PRESS => self.key(&e),
                x::EV_BUTTON_PRESS => self.press(&e),
                x::EV_BUTTON_RELEASE => {
                    if let Some(p) = self.pins.iter_mut().find(|p| p.win.win == e.window)
                        && e.button == 1
                    {
                        p.press = None;
                    }
                }
                _ => self.motion(&e),
            }
        }
    }

    /// The window manager moved pin `win`: where it really is now.
    fn moved_to(&mut self, win: c_ulong) {
        let dpy = self.d.dpy;
        let root = self.d.root();
        let Some(p) = self.pins.iter_mut().find(|p| p.win.win == win) else { return };
        if p.press.is_some() && !crate::rec::linux::portal::wayland_now() {
            return; // our own drag
        }
        let (mut x, mut y, mut child) = (0 as c_int, 0 as c_int, 0 as c_ulong);
        // SAFETY: an open Display, a live window.
        if unsafe { XTranslateCoordinates(dpy, win, root, 0, 0, &mut x, &mut y, &mut child) } != 0 {
            let i = Frame::at(p.s).inset();
            p.origin = (x + i, y + i);
        }
    }

    fn key(&mut self, e: &x::XPointerEvent) {
        let (esc, c, s) = self.keys;
        let ctrl = e.state & CONTROL_MASK != 0;
        let cmd = if e.button == esc {
            Some(PinCmd::Close)
        } else if ctrl && e.button == c {
            Some(PinCmd::Copy)
        } else if ctrl && e.button == s {
            Some(if e.state & SHIFT_MASK != 0 { PinCmd::SaveAs } else { PinCmd::Save })
        } else {
            None
        };
        if let Some(cmd) = cmd {
            self.run(e.window, cmd);
        }
    }

    fn press(&mut self, e: &x::XPointerEvent) {
        let at = (e.x_root, e.y_root);
        let Some(p) = self.pins.iter_mut().find(|p| p.win.win == e.window) else { return };
        match e.button {
            1 => {
                let now = Instant::now();
                if let Some((t, q)) = p.last.take()
                    && is_double(now.duration_since(t).as_millis(), (at.0 - q.0, at.1 - q.1))
                {
                    let w = p.win.win;
                    self.close(w);
                    return;
                }
                p.last = Some((now, at));
                p.press = Some((at, p.origin));
                p.moved = false;
            }
            3 => {
                let (w, v) = (p.win.win, p.view);
                self.open_menu(w, v, at);
            }
            4 | 5 => p.wheel(if e.button == 4 { 1 } else { -1 }, e.state & CONTROL_MASK != 0, at),
            _ => {}
        }
    }

    fn motion(&mut self, e: &x::XPointerEvent) {
        let at = (e.x_root, e.y_root);
        let wayland = crate::rec::linux::portal::wayland_now();
        let d = &self.d;
        let Some(p) = self.pins.iter_mut().find(|p| p.win.win == e.window) else { return };
        let Some((press, o)) = p.press else { return };
        if e.state & BUTTON1_MASK == 0 || !(p.moved || is_drag(press, at)) {
            return;
        }
        p.moved = true;
        if wayland {
            // XWayland: only the compositor can move a toplevel.
            p.press = None;
            p.win.start_move(d, at);
            return;
        }
        p.origin = dragged(o, press, at);
        p.place();
        x::flush(d);
    }

    fn open_menu(&mut self, pin: c_ulong, v: View, at: (i32, i32)) {
        let s = self.pins.iter().find(|p| p.win.win == pin).map_or(1.0, |p| p.s);
        let th = self.pins.iter().find(|p| p.win.win == pin).map_or(crate::theme::DARK, |p| p.th);
        let l = menu_layout(&menu(v, self.pins.len()), s);
        let Ok(mut win) = Win::with_kind(&self.d, true, None, Kind::Overlay) else { return };
        win.set_name("Rustshot pin menu", "POPUP_MENU");
        // The panel's top-left at the pointer (its shadow room around it).
        let pos = (at.0 - l.panel.x as i32, at.1 - l.panel.y as i32);
        let px = render_menu(&l, &th, s, None);
        win.put(pos.0, pos.1, l.size.0, l.size.1, &px);
        // SAFETY: an open Display, the mapped menu window.
        unsafe {
            XGrabPointer(
                self.d.dpy,
                win.win,
                1,
                (BUTTON_PRESS_MASK | BUTTON_RELEASE_MASK | POINTER_MOTION_MASK) as c_uint,
                GRAB_MODE_ASYNC,
                GRAB_MODE_ASYNC,
                0,
                0,
                0,
            );
        }
        x::flush(&self.d);
        self.menu = Some(Menu { win, l, pin, at: pos, hover: None });
        let _ = th;
    }

    fn menu_event(&mut self, ty: c_int, e: &x::XPointerEvent) {
        let Some(m) = self.menu.as_mut() else { return };
        // Relative to the menu window, wherever the event was.
        let p = Pt::new((e.x_root - m.at.0) as f32, (e.y_root - m.at.1) as f32);
        let inside = p.x >= 0.0 && p.y >= 0.0 && p.x < m.l.size.0 as f32 && p.y < m.l.size.1 as f32;
        match ty {
            x::EV_MOTION => {
                let h = menu_at(&m.l, p);
                if h != m.hover {
                    m.hover = h;
                    let (s, th) = self.pins.iter().find(|q| q.win.win == m.pin).map_or((1.0, crate::theme::DARK), |q| (q.s, q.th));
                    let px = render_menu(&m.l, &th, s, h);
                    m.win.put(m.at.0, m.at.1, m.l.size.0, m.l.size.1, &px);
                }
            }
            x::EV_BUTTON_PRESS if !inside => self.close_menu(),
            x::EV_BUTTON_RELEASE if inside => {
                if let Some(c) = menu_at(&m.l, p) {
                    let pin = m.pin;
                    self.close_menu();
                    self.run(pin, c);
                }
            }
            KEY_PRESS if e.button == self.keys.0 => self.close_menu(),
            _ => {}
        }
    }

    fn close_menu(&mut self) {
        // SAFETY: an open Display.
        unsafe {
            XUngrabPointer(self.d.dpy, 0);
        }
        self.menu = None;
        x::flush(&self.d);
    }

    fn run(&mut self, pin: c_ulong, c: PinCmd) {
        match c {
            PinCmd::Close => self.close(pin),
            PinCmd::CloseAll => {
                self.pins.clear();
                x::flush(&self.d);
                self.report();
            }
            PinCmd::Copy | PinCmd::Save | PinCmd::SaveAs => {
                let Some(p) = self.pins.iter().find(|p| p.win.win == pin) else { return };
                let (img, at, centre, th) = (p.img.clone(), p.origin, p.centre(), p.th);
                super::export(&img, at, centre, c, th);
            }
            PinCmd::Zoom100 | PinCmd::Opacity(_) => {
                if let Some(p) = self.pins.iter_mut().find(|p| p.win.win == pin)
                    && let Some(v) = apply(p.view, c)
                {
                    let about = p.centre();
                    p.set_view(v, about);
                }
            }
        }
        x::flush(&self.d);
    }
}

/// The UI scale: `Xft.dpi` with the UI size on top.
fn scale(d: &Disp) -> f32 {
    // SAFETY: an open Display; the string is Xlib's.
    let dpi = unsafe {
        let rm = XResourceManagerString(d.dpy);
        if rm.is_null() { None } else { crate::wind::xft_scale(&core::ffi::CStr::from_ptr(rm).to_string_lossy()) }
    };
    crate::uisize::apply(dpi.unwrap_or(1.0))
}

const KEY_PRESS_MASK: c_long = 1 << 0;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const EXPOSURE_MASK: c_long = 1 << 15;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const GRAB_MODE_ASYNC: c_int = 1;
const US_POSITION: c_long = 1 << 0;
const P_POSITION: c_long = 1 << 2;

#[repr(C)]
struct XSizeHints {
    flags: c_long,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    min_width: c_int,
    min_height: c_int,
    max_width: c_int,
    max_height: c_int,
    width_inc: c_int,
    height_inc: c_int,
    min_aspect: [c_int; 2],
    max_aspect: [c_int; 2],
    base_width: c_int,
    base_height: c_int,
    win_gravity: c_int,
}

#[allow(clashing_extern_declarations)]
#[link(name = "X11")]
unsafe extern "C" {
    fn XSelectInput(dpy: *mut c_void, w: c_ulong, mask: c_long) -> c_int;
    fn XSetWMNormalHints(dpy: *mut c_void, w: c_ulong, hints: *mut XSizeHints);
    fn XKeysymToKeycode(dpy: *mut c_void, keysym: c_ulong) -> u8;
    fn XResourceManagerString(dpy: *mut c_void) -> *mut c_char;
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
    #[allow(clippy::too_many_arguments)]
    fn XGrabPointer(
        dpy: *mut c_void,
        w: c_ulong,
        owner_events: c_int,
        mask: c_uint,
        pointer_mode: c_int,
        keyboard_mode: c_int,
        confine_to: c_ulong,
        cursor: c_ulong,
        time: c_ulong,
    ) -> c_int;
    fn XUngrabPointer(dpy: *mut c_void, time: c_ulong) -> c_int;
}

/// The real windows (under Xvfb in the Docker harness; skipped without a
/// display).
#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> Vec<Shown> {
        ask(Cmd::Query, Vec::new())
    }

    #[test]
    fn pin_windows_come_zoom_and_go() {
        if !crate::wind::has_display() {
            eprintln!("skipped: no X display");
            return;
        }
        close_all();
        let img = Arc::new(crate::pin::tests::sample_shot(120, 80));
        assert!(open(img.clone(), (100, 100), crate::theme::DARK));
        assert!(open(img, (300, 100), crate::theme::LIGHT));
        let q = query();
        assert_eq!(q.len(), 2);
        assert!(q.iter().all(|p| p.0 != 0 && p.1 == View::default()));
        ask(|r| Cmd::Wheel(0, 2, false, (100, 100), r), ());
        ask(|r| Cmd::Wheel(0, -3, true, (0, 0), r), ());
        let q = query();
        assert_eq!(q[0].1, View { zoom: 121, opacity: 70 });
        assert_eq!(q[0].2, (100, 100), "zoomed about its corner");
        assert_eq!(crate::pin::COUNT.load(std::sync::atomic::Ordering::SeqCst), 2);
        close_all();
        assert!(query().is_empty());
        assert_eq!(crate::pin::COUNT.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
