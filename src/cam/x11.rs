//! Linux: the camera bubble window (X11, and XWayland on Wayland). The
//! same behaviour as the Windows bubble (`bubble.rs`): shapes, sizes,
//! mirror, accent ring and shadow ([`super::compose`]), kept relative to
//! the area, dragged and snapped to its corners, off the overlay's toolbar
//! and popover; the camera is open only while the bubble exists.
//!
//! X11: an override-redirect window on a 32-bit ARGB visual (premultiplied
//! pixels straight from `compose`) when a compositor runs, else the
//! default visual cut to the shape (XShape bounding). Its input shape is
//! the bubble's shape, so clicks on the shadow and corners go through.
//! Dragging is done by hand from pointer events, with snapping.
//!
//! Wayland (XWayland): a normal undecorated always-on-top toplevel; the
//! compositor has the last word on where it goes (it may ignore the place
//! asked for), and dragging hands the move to it (`_NET_WM_MOVERESIZE`):
//! no snapping and no remembered place there. The overlay (an override-
//! redirect window) may cover it while the Record popover is open.
//!
//! The window has its own thread and Display connection (Xlib was
//! initialised for threads); a camera thread opens the camera, fits each
//! picture to the bubble and leaves the newest for the window thread,
//! which paints it at most about 30 times a second.

use super::{CamFrame, CamSource, Look, Template};
use crate::capture::IRect;
use crate::rec_ui::x11::{self as xw, Disp, Kind, Win};
use crate::theme::Theme;
use anyhow::{bail, Context, Result};
use core::ffi::{c_int, c_short};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Opens the camera on the camera thread (tests inject their own); the
/// flag is set when the bubble closes meanwhile (the camera portal's
/// dialog is then called off).
pub(crate) type Opener = Box<dyn FnOnce(&AtomicBool) -> Result<Box<dyn CamSource>> + Send>;

/// How long closing waits for the camera thread to let go of the camera,
/// as on Windows: not at all in the app (the thread stops after its
/// current read, 200 ms at most, and releases the camera on its own, which
/// must not hold up the UI thread at Stop); tests wait to check the release.
#[cfg(not(test))]
const CLOSE_WAIT: Duration = Duration::ZERO;
#[cfg(test)]
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Pictures closer together than this are skipped (about 30 fps at most).
const FRAME_GAP: Duration = Duration::from_millis(25);
/// How often the window thread looks for changes when nothing happens.
const TICK_MS: c_int = 10;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where the bubble is and how it looks (shared with the owner).
struct Geo {
    area: IRect,
    scale: f32,
    look: Look,
    th: Theme,
    avoid: Vec<IRect>,
    screen: IRect,
    /// Top-left of the shape (physical px; the window is `pad` larger).
    pos: (i32, i32),
    tpl: Template,
    /// Bumped when the template changes (the input shape is set again)
    /// and when the window must move or repaint.
    tpl_gen: u64,
    paint_gen: u64,
    /// Pressed: (pointer at the press, pos at the press).
    drag: Option<((i32, i32), (i32, i32))>,
    /// The picture shown (fitted to `tpl.d`).
    frame: Option<CamFrame>,
    /// Where the compositor put the shape's top-left (a managed window on
    /// Wayland: `ConfigureNotify`).
    placed: Option<(i32, i32)>,
}

impl Geo {
    fn home(&self) -> (i32, i32) {
        let d = self.tpl.d as i32;
        let p = super::place(super::remembered(), self.area, d, self.scale);
        super::clear_of(p, self.area, d, self.scale, &self.avoid, self.screen)
    }
}

struct Shared {
    geo: Mutex<Geo>,
    /// The newest fitted picture, not yet shown, and a spare buffer.
    fresh: Mutex<Option<CamFrame>>,
    spare: Mutex<Option<CamFrame>>,
    error: Mutex<Option<String>>,
    stop: AtomicBool,
    side: AtomicU32,
    picture: AtomicBool,
    /// The window thread could not make the window.
    failed: Mutex<Option<String>>,
}

/// The camera bubble on screen (see the module docs).
pub struct Bubble {
    shared: Arc<Shared>,
    device: String,
    ui: Option<std::thread::JoinHandle<()>>,
    cam_done: Option<Receiver<()>>,
}

fn camera_thread(open: Opener, sh: Arc<Shared>, done: mpsc::Sender<()>) {
    let r = (|| -> Result<()> {
        let mut src = open(&sh.stop)?;
        let mut last: Option<Instant> = None;
        let (mut raw, mut tmp) = (CamFrame::default(), Vec::new());
        while !sh.stop.load(Ordering::SeqCst) {
            if !src.read(&mut raw)? {
                continue;
            }
            let now = Instant::now();
            if last.is_some_and(|t| now.saturating_duration_since(t) < FRAME_GAP) {
                continue;
            }
            last = Some(now);
            let mut sq = lock(&sh.spare).take().unwrap_or_default();
            super::fit_square_into(&raw, sh.side.load(Ordering::SeqCst), &mut tmp, &mut sq);
            if let Some(old) = lock(&sh.fresh).replace(sq) {
                *lock(&sh.spare) = Some(old);
            }
        }
        Ok(())
    })();
    if let Err(e) = r
        && !sh.stop.load(Ordering::SeqCst)
    {
        let msg = format!("{e:#}");
        eprintln!("rustshot: camera: {msg}");
        *lock(&sh.error) = Some(msg);
    }
    let _ = done.send(());
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
}

/// The input shape: the bubble's shape (mask ≥ 50%) inside the window.
fn input_runs(t: &Template) -> Vec<(i16, i16, u16, u16)> {
    let d = t.d as usize;
    let mut px = vec![0u8; d * d * 4];
    for (i, m) in t.mask.iter().enumerate() {
        px[i * 4 + 3] = *m;
    }
    xw::alpha_runs(&px, d, d).into_iter().map(|(x, y, w, h)| (x + t.pad as i16, y + t.pad as i16, w, h)).collect()
}

fn window_thread(sh: Arc<Shared>, managed: bool, ready: mpsc::Sender<Result<()>>) {
    let made = (|| -> Result<(Disp, Win)> {
        let d = Disp::open()?;
        let kind = if managed { Kind::Managed("Rustshot camera") } else { Kind::Overlay };
        let w = Win::with_kind(&d, true, None, kind)?;
        Ok((d, w))
    })();
    let (disp, mut win) = match made {
        Ok(x) => {
            let _ = ready.send(Ok(()));
            x
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let (mut tpl_gen, mut paint_gen) = (u64::MAX, u64::MAX);
    let mut px = Vec::new();
    let mut hidden = false;
    let fd = xw::conn_fd(&disp);
    while !sh.stop.load(Ordering::SeqCst) {
        // Pointer: drag (X11) or hand the move to the compositor (Wayland).
        while let Some(ev) = xw::next_event(&disp) {
            // SAFETY: every event starts with its type; pointer events
            // share the head read here.
            let (ty, p) = unsafe { (ev.type_, ev.pointer) };
            let mut g = lock(&sh.geo);
            match ty {
                xw::EV_BUTTON_PRESS if p.button == 1 => {
                    if win.is_managed() {
                        drop(g);
                        win.start_move(&disp, (p.x_root, p.y_root));
                        continue;
                    }
                    g.drag = Some(((p.x_root, p.y_root), g.pos));
                }
                xw::EV_MOTION => {
                    if let Some((c0, p0)) = g.drag {
                        let at = (p0.0 + p.x_root - c0.0, p0.1 + p.y_root - c0.1);
                        g.pos = super::snap(at, g.area, g.tpl.d as i32, g.scale);
                        g.paint_gen += 1;
                    }
                }
                xw::EV_BUTTON_RELEASE if p.button == 1 => {
                    if g.drag.take().is_some() {
                        super::remember(super::rel_of(g.pos, g.area, g.tpl.d as i32, g.scale));
                    }
                }
                xw::EV_EXPOSE => g.paint_gen += 1,
                xw::EV_CONFIGURE if win.is_managed() => {
                    // SAFETY: a ConfigureNotify event.
                    let c = unsafe { ev.configure };
                    let pad = g.tpl.pad as i32;
                    g.placed = Some((c.x + pad, c.y + pad));
                }
                _ => {}
            }
        }
        // A new picture.
        if let Some(f) = lock(&sh.fresh).take() {
            let mut g = lock(&sh.geo);
            if f.w == g.tpl.d {
                if let Some(old) = g.frame.replace(f) {
                    *lock(&sh.spare) = Some(old);
                }
                sh.picture.store(true, Ordering::SeqCst);
                g.paint_gen += 1;
            } else {
                *lock(&sh.spare) = Some(f); // fitted before a size change
            }
        }
        if lock(&sh.error).is_some() {
            if !hidden {
                win.hide();
                hidden = true;
            }
        } else {
            let g = lock(&sh.geo);
            if g.tpl_gen != tpl_gen {
                tpl_gen = g.tpl_gen;
                win.set_input(&input_runs(&g.tpl));
            }
            if g.paint_gen != paint_gen {
                paint_gen = g.paint_gen;
                super::compose(&g.tpl, g.frame.as_ref(), g.look.mirror, &mut px);
                let pad = g.tpl.pad as i32;
                let w = g.tpl.w;
                // A managed window is placed once; the compositor keeps it.
                let (x, y) = (g.pos.0 - pad, g.pos.1 - pad);
                drop(g);
                win.put(x, y, w, w, &px);
            }
        }
        xw::flush(&disp);
        let mut p = PollFd { fd, events: 1, revents: 0 };
        // SAFETY: one valid pollfd.
        unsafe {
            poll(&mut p, 1, TICK_MS);
        }
    }
    drop(win);
    drop(disp);
}

impl Bubble {
    /// Show the bubble for camera `device` ("" = the first) with `look`,
    /// placed where it was last left relative to `area` (physical px) at
    /// UI scale `scale`, its ring in `th`'s accent. `_nudge` is unused on
    /// Linux (the owner looks at [`Bubble::error`] on its ticks).
    #[cfg_attr(test, allow(dead_code))]
    pub fn show(look: Look, device: &str, area: IRect, scale: f32, th: &Theme, _nudge: crate::wind::Hwnd) -> Result<Bubble> {
        let id = device.to_string();
        let open: Opener = Box::new(move |stop| super::open_or_cancel(&id, super::PREF, stop));
        Bubble::show_with(look, device, area, scale, th, _nudge, open)
    }

    /// [`Bubble::show`] with the camera opened by `open`.
    pub(crate) fn show_with(
        look: Look,
        device: &str,
        area: IRect,
        scale: f32,
        th: &Theme,
        _nudge: crate::wind::Hwnd,
        open: Opener,
    ) -> Result<Bubble> {
        let tpl = super::template(look, scale, th);
        let mut geo = Geo {
            area,
            scale,
            look,
            th: *th,
            avoid: Vec::new(),
            screen: area,
            pos: (0, 0),
            tpl,
            tpl_gen: 0,
            paint_gen: 0,
            drag: None,
            frame: None,
            placed: None,
        };
        geo.pos = geo.home();
        let side = geo.tpl.d;
        let shared = Arc::new(Shared {
            geo: Mutex::new(geo),
            fresh: Mutex::new(None),
            spare: Mutex::new(None),
            error: Mutex::new(None),
            stop: AtomicBool::new(false),
            side: AtomicU32::new(side),
            picture: AtomicBool::new(false),
            failed: Mutex::new(None),
        });
        let managed = crate::rec::linux::portal::wayland_now();
        let (tx, rx) = mpsc::channel();
        let sh = shared.clone();
        let ui = std::thread::Builder::new()
            .name("camera-bubble".into())
            .spawn(move || window_thread(sh, managed, tx))
            .context("start the camera window thread")?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = ui.join();
                return Err(e.context("create the camera window"));
            }
            Err(_) => bail!("the camera window did not open"),
        }
        let (dtx, drx) = mpsc::channel();
        let sh = shared.clone();
        let mut b = Bubble { shared, device: device.to_string(), ui: Some(ui), cam_done: None };
        std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || camera_thread(open, sh, dtx))
            .context("start the camera thread")?;
        b.cam_done = Some(drx);
        Ok(b)
    }

    /// The camera it shows ("" = the first).
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Why the camera stopped (the bubble is hidden then).
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.error).clone().or_else(|| lock(&self.shared.failed).clone())
    }

    /// The shape's top-left on screen (physical px).
    #[allow(dead_code)] // Windows tests use it; kept alike
    pub fn pos(&self) -> Option<(i32, i32)> {
        Some(lock(&self.shared.geo).pos)
    }

    /// Where the compositor put the bubble's shape (Wayland), once it said.
    pub fn placed(&self) -> Option<IRect> {
        let g = lock(&self.shared.geo);
        g.placed.map(|(x, y)| (x, y, g.tpl.d, g.tpl.d))
    }

    /// A camera picture is shown.
    pub fn has_picture(&self) -> bool {
        self.shared.picture.load(Ordering::SeqCst)
    }

    /// Wait up to `timeout` for the camera's first picture; false when
    /// none came (or the camera failed).
    pub fn wait_picture(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if self.has_picture() {
                return true;
            }
            if self.error().is_some() || Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Follow `area` at UI scale `scale` (placed where it was left
    /// relative to it). Not while dragged.
    pub fn set_anchor(&mut self, area: IRect, scale: f32) {
        let mut g = lock(&self.shared.geo);
        if (g.area, g.scale) == (area, scale) || g.drag.is_some() {
            return;
        }
        if g.scale != scale {
            g.tpl = super::template(g.look, scale, &g.th);
            g.frame = None;
            g.tpl_gen += 1;
            self.shared.side.store(g.tpl.d, Ordering::SeqCst);
        }
        g.area = area;
        g.scale = scale;
        g.pos = g.home();
        g.paint_gen += 1;
    }

    /// Keep off `avoid` (the overlay's toolbar and Record popover), going
    /// outside a small area within `screen` if it must. Not while dragged.
    pub fn set_avoid(&mut self, avoid: &[IRect], screen: IRect) {
        let mut g = lock(&self.shared.geo);
        if (g.avoid.as_slice(), g.screen) == (avoid, screen) || g.drag.is_some() {
            return;
        }
        g.avoid = avoid.to_vec();
        g.screen = screen;
        let pos = g.home();
        if pos != g.pos {
            g.pos = pos;
            g.paint_gen += 1;
        }
    }

    /// Keep off nothing any more (the overlay is gone).
    pub fn clear_avoid(&mut self) {
        let screen = lock(&self.shared.geo).screen;
        self.set_avoid(&[], screen);
    }

    /// Change the shape, size or mirroring (and the theme's accent).
    pub fn set_look(&mut self, look: Look, th: &Theme) {
        let mut g = lock(&self.shared.geo);
        if g.look == look && g.th == *th {
            return;
        }
        let resized = g.look.size != look.size;
        g.look = look;
        g.th = *th;
        g.tpl = super::template(look, g.scale, th);
        g.tpl_gen += 1;
        self.shared.side.store(g.tpl.d, Ordering::SeqCst);
        if resized {
            g.frame = None;
            g.pos = g.home();
        }
        g.paint_gen += 1;
    }
}

impl Drop for Bubble {
    /// Stop the camera (its thread lets go of it after its current read;
    /// waited for up to [`CLOSE_WAIT`]) and take the window down (its
    /// thread is joined; it stops within one tick).
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(rx) = self.cam_done.take()
            && !CLOSE_WAIT.is_zero()
        {
            let _ = rx.recv_timeout(CLOSE_WAIT);
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_shape_is_the_bubble_not_its_shadow() {
        let look = Look::new(super::super::Shape::Circle, super::super::CamSize::S);
        let t = super::super::template(look, 1.0, &crate::theme::DARK);
        let runs = input_runs(&t);
        let (pad, d) = (t.pad as i16, t.d as i16);
        assert!(runs.iter().all(|r| r.0 >= pad && r.1 >= pad && r.0 + r.2 as i16 <= pad + d && r.1 < pad + d));
        // The middle row spans (nearly) the whole circle; the top row little.
        let mid = runs.iter().find(|r| r.1 == pad + d / 2).unwrap();
        assert!(mid.2 as i16 >= d - 2);
        let top = runs.iter().find(|r| r.1 == pad).map_or(0, |r| r.2);
        assert!((top as i16) < d / 4);
    }
}
