//! X11: the notice in an override-redirect window (no decorations, above
//! normal windows, never focused) on its own thread and Display
//! connection, bottom-centre of the monitor's work area (`_NET_WORKAREA`
//! clipped to the monitor). With a compositor it is a 32-bit ARGB window
//! and fades; without one it is cut to shape (alpha ≥ 50%, see
//! `rec_ui::x11::Win`). A note without buttons lets clicks through (an
//! empty input shape); one with buttons (after a recording) takes them and
//! holds while the pointer is on it.
//!
//! X11 cannot keep a window out of a screenshot, so [`hide`] waits for the
//! thread to unmap it: a new capture never has the last notice in it.
//!
//! Wayland is left to stdout and `tray::notify`: an XWayland toplevel
//! cannot place itself (the compositor would put the toast anywhere, often
//! centred), cannot be click-through, and is focused when mapped, which a
//! toast must never be.

use super::{act_at, act_layout, hold, note_size, opacity, place_note, render_note, work_area, ActLayout, Next, Note, NoteAct, FADE_IN};
use crate::capture::IRect;
use crate::objects::Pt;
use crate::rec_ui::x11::{self as x, Disp, Kind, Win};
use crate::theme::Theme;
use core::ffi::{c_char, c_int, c_long, c_ulong, c_void};
use crate::chan::{self as mpsc, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The window's name and type (what the Docker harness looks for).
pub const TITLE: &str = "Rustshot notice";
const TICK: Duration = Duration::from_millis(16);
/// How long a caller waits for the thread to answer.
const REPLY: Duration = Duration::from_millis(500);

enum Cmd {
    Show(Note, (i32, i32), Theme, Sender<bool>),
    Replace(Note, Sender<bool>),
    Hide(Sender<()>),
    /// An action finished on another thread (an upload).
    Later(Note),
    /// Tests: what is up (text, takes clicks, the X window).
    #[cfg(test)]
    Query(Sender<Option<(String, bool, c_ulong)>>),
}

/// The notice thread's queue; None when there is no X display (Wayland, or
/// it could not be opened).
static TX: OnceLock<Option<Mutex<Sender<Cmd>>>> = OnceLock::new();

fn sender() -> Option<Sender<Cmd>> {
    let tx = TX.get_or_init(|| {
        if crate::rec::linux::portal::wayland_now() || std::env::var_os("DISPLAY").is_none() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let (ok_tx, ok_rx) = mpsc::channel();
        let me = tx.clone();
        let spawned = std::thread::Builder::new().name("notice".into()).spawn(move || run(rx, me, ok_tx));
        (spawned.is_ok() && ok_rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false)).then(|| Mutex::new(tx))
    });
    tx.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

/// Send `make(reply)` and wait for the answer (`dflt` when there is no
/// thread or it does not answer).
fn ask<T>(make: impl FnOnce(Sender<T>) -> Cmd, dflt: T) -> T {
    let Some(tx) = sender() else { return dflt };
    let (rtx, rrx) = mpsc::channel();
    if tx.send(make(rtx)).is_err() {
        return dflt;
    }
    rrx.recv_timeout(REPLY).unwrap_or(dflt)
}

pub fn show(note: Note, at: (i32, i32), th: Theme) -> bool {
    ask(|r| Cmd::Show(note, at, th, r), false)
}

pub fn replace(note: Note) -> bool {
    ask(|r| Cmd::Replace(note, r), false)
}

/// Take the note down and wait until it is unmapped (a capture follows).
pub fn hide() {
    // Never start the thread just to hide nothing.
    if TX.get().is_some_and(|t| t.is_some()) {
        ask(Cmd::Hide, ());
    }
}

#[cfg(test)]
fn query() -> Option<(String, bool, c_ulong)> {
    ask(Cmd::Query, None)
}

struct Up {
    note: Note,
    th: Theme,
    s: f32,
    /// The monitor's work area it was placed in.
    work: IRect,
    pos: (i32, i32),
    since: Instant,
    /// Opacity of the frame on screen (None: none drawn yet).
    k: Option<f32>,
    layout: Option<ActLayout>,
    hover: Option<NoteAct>,
    dirty: bool,
    /// Its age while held (see [`hold`]).
    frozen: Option<Duration>,
}

struct State {
    d: Disp,
    win: Win,
    up: Option<Up>,
    tx: Sender<Cmd>,
}

fn run(rx: Receiver<Cmd>, tx: Sender<Cmd>, ok: Sender<bool>) {
    let opened = Disp::open().and_then(|d| {
        let mut win = Win::with_kind(&d, true, None, Kind::Overlay)?;
        win.set_name(TITLE, "NOTIFICATION");
        Ok((d, win))
    });
    let (d, win) = match opened {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rustshot: no notice window: {e:#}");
            let _ = ok.send(false);
            return;
        }
    };
    let _ = ok.send(true);
    let mut st = State { d, win, up: None, tx };
    loop {
        // Idle: wait for a note; up: tick the fade every 16 ms.
        let cmd = if st.up.is_some() {
            match rx.recv_timeout(TICK) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };
        if let Some(c) = cmd {
            st.handle(c);
            while let Ok(c) = rx.try_recv() {
                st.handle(c);
            }
        }
        st.events();
        st.tick();
    }
}

impl State {
    fn handle(&mut self, c: Cmd) {
        match c {
            Cmd::Show(note, at, th, r) => {
                let (work, s) = monitor_at(&self.d, at);
                self.put_up(note, th, s, work, Instant::now());
                let _ = r.send(true);
            }
            Cmd::Replace(note, r) => {
                let _ = r.send(self.replace(note));
            }
            Cmd::Hide(r) => {
                self.take_down();
                let _ = r.send(());
            }
            Cmd::Later(note) => {
                if !self.replace(note.clone()) {
                    crate::tray::notify(&note.text);
                }
            }
            #[cfg(test)]
            Cmd::Query(r) => {
                let _ = r.send(self.up.as_ref().map(|u| (u.note.text.clone(), u.layout.is_some(), self.win.win)));
            }
        }
        self.tick();
    }

    fn put_up(&mut self, note: Note, th: Theme, s: f32, work: IRect, since: Instant) {
        let size = note_size(&note, s);
        let pos = place_note(work, &note, size);
        let layout = (!note.actions.is_empty()).then(|| act_layout(&note, s));
        // Clicks through a plain note; the whole window takes them with buttons.
        match &layout {
            Some(_) => self.win.set_input(&[(0, 0, size.0.min(u16::MAX as u32) as u16, size.1.min(u16::MAX as u32) as u16)]),
            None => self.win.set_input(&[]),
        }
        self.up = Some(Up { note, th, s, work, pos, since, k: None, layout, hover: None, dirty: false, frozen: None });
    }

    fn replace(&mut self, note: Note) -> bool {
        let Some(up) = self.up.as_ref() else { return false };
        let (th, s, work) = (up.th, up.s, up.work);
        let now = Instant::now();
        // On screen already: no second fade-in.
        self.put_up(note, th, s, work, now.checked_sub(FADE_IN).unwrap_or(now));
        true
    }

    fn take_down(&mut self) {
        self.up = None;
        super::forget_picture();
        self.win.hide();
        x::flush(&self.d);
    }

    /// Pointer input on a note with buttons.
    fn events(&mut self) {
        while let Some(ev) = x::next_event(&self.d) {
            // SAFETY: every event starts with its type.
            let ty = unsafe { ev.type_ };
            if ty == x::EV_EXPOSE {
                if let Some(up) = self.up.as_mut() {
                    up.dirty = true;
                }
                continue;
            }
            if ![x::EV_MOTION, x::EV_LEAVE, x::EV_BUTTON_RELEASE].contains(&ty) {
                continue;
            }
            // SAFETY: a pointer event (motion, crossing, button).
            let p = unsafe { ev.pointer };
            let at = Pt::new(p.x as f32, p.y as f32);
            if ty == x::EV_BUTTON_RELEASE {
                if p.button == 1 {
                    self.click(at);
                }
                continue;
            }
            let Some(up) = self.up.as_mut() else { continue };
            let Some(l) = &up.layout else { continue };
            let h = if ty == x::EV_MOTION { act_at(l, at) } else { None };
            if h != up.hover {
                up.hover = h;
                up.dirty = true;
            }
        }
    }

    /// A click on a button: carry it out; the note says how it went (or goes).
    fn click(&mut self, p: Pt) {
        let Some(up) = self.up.as_ref() else { return };
        let Some(a) = up.layout.as_ref().and_then(|l| act_at(l, p)) else { return };
        let (note, th) = (up.note.clone(), up.th);
        let tx = self.tx.clone();
        match super::run_note_action(&note, a, th, move |n| {
            let _ = tx.send(Cmd::Later(n));
        }) {
            Next::Keep => {}
            Next::Show(n) => {
                self.replace(n);
            }
            // Open / Show in folder, Pin, Close: done with the note.
            Next::Hide => self.take_down(),
        }
    }

    /// Draw the next frame of the fade, or take the note down once it is over.
    fn tick(&mut self) {
        let Some(up) = self.up.as_mut() else { return };
        // Held while the pointer is on its buttons (or the card).
        up.since = hold(up.since, Instant::now(), up.note.ttl, up.hover.is_some(), &mut up.frozen);
        let Some(k) = opacity(up.since.elapsed(), up.note.ttl) else {
            self.take_down();
            return;
        };
        if !up.dirty && up.k.is_some_and(|old| (old - k).abs() < 0.004) {
            return; // holding: no new frame
        }
        up.k = Some(k);
        up.dirty = false;
        let (w, h, px) = render_note(&up.note, &up.th, up.s, k, up.hover);
        self.win.put(up.pos.0, up.pos.1, w, h, &px);
    }
}

#[allow(clashing_extern_declarations)]
#[link(name = "X11")]
unsafe extern "C" {
    fn XInternAtom(dpy: *mut c_void, name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XFree(data: *mut c_void) -> c_int;
    fn XResourceManagerString(dpy: *mut c_void) -> *mut c_char;
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
}

/// A root window property as C longs (format 32); empty when unset.
fn root_longs(d: &Disp, name: &str, max: c_long) -> Vec<c_long> {
    let c = std::ffi::CString::new(name).expect("no NUL");
    // SAFETY: an open Display, its root window; the data is copied, then freed.
    unsafe {
        let prop = XInternAtom(d.dpy, c.as_ptr(), 1);
        if prop == 0 {
            return Vec::new();
        }
        let (mut ty, mut fmt, mut n, mut after) = (0 as c_ulong, 0 as c_int, 0 as c_ulong, 0 as c_ulong);
        let mut data: *mut u8 = core::ptr::null_mut();
        if XGetWindowProperty(d.dpy, d.root(), prop, 0, max, 0, 0, &mut ty, &mut fmt, &mut n, &mut after, &mut data) != 0 || data.is_null() {
            return Vec::new();
        }
        let out = if fmt == 32 { core::slice::from_raw_parts(data as *const c_long, n as usize).to_vec() } else { Vec::new() };
        XFree(data.cast());
        out
    }
}

/// The desktop's work area (`_NET_WORKAREA` of `_NET_CURRENT_DESKTOP`):
/// the screen less docks and panels, as the window manager says.
fn net_workarea(d: &Disp) -> Option<IRect> {
    let areas = root_longs(d, "_NET_WORKAREA", 4 * 64);
    let cur = root_longs(d, "_NET_CURRENT_DESKTOP", 1).first().copied().unwrap_or(0).max(0) as usize;
    let (desks, _) = areas.as_chunks::<4>();
    let a = desks.get(cur).or_else(|| desks.first())?;
    (a[2] > 0 && a[3] > 0).then(|| (a[0] as i32, a[1] as i32, a[2] as u32, a[3] as u32))
}

/// The work area of the monitor at `at` and the UI scale (`Xft.dpi`, the
/// UI size on top).
fn monitor_at(d: &Disp, at: (i32, i32)) -> (IRect, f32) {
    let mons: Vec<IRect> = crate::capture::monitors().unwrap_or_default().iter().map(|m| (m.x, m.y, m.w, m.h)).collect();
    let work = work_area(&mons, net_workarea(d), at);
    // SAFETY: an open Display; the string is Xlib's.
    let dpi = unsafe {
        let rm = XResourceManagerString(d.dpy);
        if rm.is_null() { None } else { crate::wind::xft_scale(&core::ffi::CStr::from_ptr(rm).to_string_lossy()) }
    };
    (work, crate::uisize::apply(dpi.unwrap_or(1.0)))
}

/// The real window (under Xvfb in the Docker harness; skipped without a
/// display).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::style::ToastKind;

    #[test]
    fn the_notice_window_comes_and_goes() {
        if !crate::wind::has_display() || crate::rec::linux::portal::wayland_now() {
            eprintln!("skipped: no X display");
            return;
        }
        let note = |t: &str| Note { ttl: Duration::from_millis(400), ..Note::plain(t, ToastKind::Info) };
        hide();
        assert!(!replace(note("x")), "nothing up yet");
        assert!(show(note("Uploading\u{2026}"), (10, 10), crate::theme::DARK));
        let (text, clicks, win) = query().expect("up");
        assert_eq!((text.as_str(), clicks), ("Uploading\u{2026}", false), "a plain note lets clicks through");
        assert_ne!(win, 0);
        assert!(replace(note("Link copied \u{b7} i.imgur.com/x.png")));
        assert_eq!(query().map(|q| q.0).as_deref(), Some("Link copied \u{b7} i.imgur.com/x.png"));
        let rec = super::super::recording_saved(std::path::Path::new("/tmp/a.gif"), Duration::from_secs(3), 1000, &[], false);
        assert!(replace(rec));
        assert_eq!(query().map(|q| q.1), Some(true), "buttons: it takes clicks");
        hide();
        assert_eq!(query(), None, "hidden at once");
        // It fades out on its own.
        assert!(show(note("Copied to clipboard"), (10, 10), crate::theme::LIGHT));
        let t0 = Instant::now();
        while query().is_some() && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(query(), None, "gone after its time and the fade");
    }
}
