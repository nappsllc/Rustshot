//! macOS: the camera through an `AVCaptureSession` (BGRA frames from an
//! `AVCaptureVideoDataOutput`, 1280×720 when the camera offers it; access
//! asked for on first use, `NSCameraUsageDescription`) and the camera
//! bubble, a transparent borderless panel ([`crate::wind::Layer`]) with
//! the same look and behaviour as on Windows: shapes, sizes, mirror,
//! accent ring and shadow ([`super::compose`]), kept relative to the area,
//! dragged and snapped to its corners, off the overlay's toolbar.
//!
//! The bubble is placed in points and drawn at the display's pixel
//! density; its API takes the editor's pixels (points × the backing
//! factor, see `capture::pts`) and converts them on the way in. Its window lives on the main
//! thread (AppKit); a camera thread opens the camera, fits each picture to
//! the bubble and asks the main thread to repaint (through the main
//! dispatch queue, which the overlay's event pump serves). The bubble is
//! recorded (it is not in the recording's filter); the camera is open only
//! while it exists. Center Stage and Portrait apply to the feed by
//! themselves.

use super::{CamDevice, CamFrame, CamSource, Look, Template};
use crate::capture::IRect;
use crate::macos_objc as objc;
use crate::rec::macos::media::{self, Capture, SampleSink};
use crate::theme::Theme;
use crate::wind::{msg1, objc_cls, objc_sel, AuxKind, Layer};
use anyhow::{anyhow, Context, Result};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use crate::chan::{self as mpsc, Receiver};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The cameras.
pub fn list() -> Vec<CamDevice> {
    match media::media() {
        Ok(m) => media::devices(m.k.AVMediaTypeVideo).into_iter().map(|(id, name)| CamDevice { id, name }).collect(),
        Err(_) => Vec::new(),
    }
}

#[derive(Default)]
struct Latest {
    frame: CamFrame,
    seq: u64,
}

#[derive(Default)]
struct CamShared {
    latest: Mutex<Latest>,
    cv: Condvar,
}

/// An open camera.
pub struct MacCam {
    _cap: Capture,
    shared: Arc<CamShared>,
    pref: (u32, u32),
    seen: u64,
}

impl MacCam {
    /// Camera `id` ("" = the default one), at `pref` when it offers it.
    pub fn open(id: &str, pref: (u32, u32)) -> Result<MacCam> {
        let m = media::media()?;
        media::av_access(m.k.AVMediaTypeVideo, crate::rec::macos::CAMERA_DENIED.get())?;
        let _pool = objc::Pool::new();
        unsafe {
            let dev = media::device(m.k.AVMediaTypeVideo, id)
                .ok_or_else(|| if id.is_empty() { anyhow!(super::NO_CAMERA) } else { anyhow!("camera {id} not found") })?;
            let out = objc::new_obj(objc_cls(c"AVCaptureVideoDataOutput")).ok_or_else(|| anyhow!("create the camera output"))?;
            let settings =
                objc::dict(&[(m.k.kCVPixelBufferPixelFormatTypeKey, objc::num_i64(i64::from(crate::rec::macos::BGRA)))]);
            let _: () = msg1(out.ptr(), objc_sel(c"setVideoSettings:"), settings);
            let _: () = msg1(out.ptr(), objc_sel(c"setAlwaysDiscardsLateVideoFrames:"), 1i8);
            let shared = Arc::new(CamShared::default());
            let sh = shared.clone();
            let sink = SampleSink::new(
                move |sbuf, _| {
                    let mut l = lock(&sh.latest);
                    if let Some((w, h)) = media::copy_pixels(m, sbuf, &mut l.frame.bgra) {
                        (l.frame.w, l.frame.h) = (w, h);
                        l.seq += 1;
                        sh.cv.notify_all();
                    }
                },
                None,
            )?;
            let preset = if pref == (1280, 720) { m.k.AVCaptureSessionPreset1280x720 } else { core::ptr::null_mut() };
            let cap = Capture::start(dev, out, sink, preset).context("open the camera")?;
            Ok(MacCam { _cap: cap, shared, pref, seen: 0 })
        }
    }
}

impl CamSource for MacCam {
    fn size(&self) -> (u32, u32) {
        let l = lock(&self.shared.latest);
        if l.seq > 0 { (l.frame.w, l.frame.h) } else { self.pref }
    }

    fn read(&mut self, into: &mut CamFrame) -> Result<bool> {
        let mut l = lock(&self.shared.latest);
        let end = Instant::now() + Duration::from_millis(200);
        while l.seq == self.seen {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(false);
            }
            l = self.shared.cv.wait_timeout(l, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        self.seen = l.seq;
        // The old buffer goes back to the camera to fill.
        std::mem::swap(into, &mut l.frame);
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// The bubble
// ---------------------------------------------------------------------------

/// Opens the camera on the camera thread.
type Opener = Box<dyn FnOnce() -> Result<Box<dyn CamSource>> + Send>;

/// Pictures closer together than this are skipped (about 30 fps at most).
const FRAME_GAP: Duration = Duration::from_millis(25);

/// Shared with the camera thread.
#[derive(Default)]
struct Shared {
    fresh: Mutex<Option<CamFrame>>,
    spare: Mutex<Option<CamFrame>>,
    error: Mutex<Option<String>>,
    stop: AtomicBool,
    /// The picture's side in pixels.
    side: AtomicU32,
    /// The background effect ([`super::bg::Bg`]).
    bg: AtomicU8,
    picture: AtomicBool,
    /// A repaint is queued on the main thread.
    queued: AtomicBool,
}

/// Where the bubble is and how it looks (main thread).
struct Geo {
    area: IRect,
    look: Look,
    th: Theme,
    avoid: Vec<IRect>,
    screen: IRect,
    /// Top-left of the shape (points).
    pos: (i32, i32),
    /// Pressed: (pointer at the press, pos at the press).
    drag: Option<((f64, f64), (i32, i32))>,
    /// Pixels per point it is drawn at.
    px: f32,
    /// The UI size's multiplier (logical px per point).
    ui: f32,
    /// The pixels' template (at `px` × `ui`).
    tpl: Template,
    frame: Option<CamFrame>,
    /// Hidden by the recording's camera button (the camera stays open).
    hidden: bool,
}

impl Geo {
    /// The shape's side in points.
    fn d(&self) -> i32 {
        super::side(self.look.size, self.ui) as i32
    }

    fn home(&self) -> (i32, i32) {
        let d = self.d();
        let p = super::place(super::remembered(), self.area, d, self.ui);
        super::clear_of(p, self.area, d, self.ui, &self.avoid, self.screen)
    }
}

struct Ui {
    geo: RefCell<Geo>,
    layer: RefCell<Layer>,
    shared: Arc<Shared>,
    px: RefCell<Vec<u8>>,
}

std::thread_local! {
    /// The bubbles on screen (main thread), for repaints the camera asks for.
    static LIVE: RefCell<Vec<Weak<Ui>>> = const { RefCell::new(Vec::new()) };
}

/// Pixels per point where `area` is.
fn scale_of(area: IRect) -> f32 {
    let (cx, cy) = (f64::from(area.0) + f64::from(area.2) / 2.0, f64::from(area.1) + f64::from(area.3) / 2.0);
    (crate::rec::macos::backing_scale_at(cx, cy) as f32).clamp(1.0, 4.0)
}

impl Ui {
    /// Take a new picture, if any, and draw.
    fn refresh(&self) {
        if let Some(f) = lock(&self.shared.fresh).take() {
            let mut g = self.geo.borrow_mut();
            if f.w == g.tpl.d {
                if let Some(old) = g.frame.replace(f) {
                    *lock(&self.shared.spare) = Some(old);
                }
            } else {
                *lock(&self.shared.spare) = Some(f); // fitted before a size change
            }
        }
        self.paint();
    }

    fn paint(&self) {
        let mut layer = self.layer.borrow_mut();
        if lock(&self.shared.error).is_some() || self.geo.borrow().hidden {
            layer.hide();
            return;
        }
        let g = self.geo.borrow();
        let mut px = self.px.borrow_mut();
        super::compose(&g.tpl, g.frame.as_ref(), g.look.mirror, &mut px);
        if g.drag.is_some() {
            super::fade_layer(&mut px, super::DRAG_OPACITY);
        }
        let s = f64::from(g.px);
        let pad = f64::from(g.tpl.pad) / s;
        let at = (f64::from(g.pos.0) - pad, f64::from(g.pos.1) - pad);
        layer.put(at, s, g.tpl.w, g.tpl.w, &px);
    }

    /// A new template for the look and scale (the picture is fitted again).
    fn retemplate(&self, g: &mut Geo) {
        let d = g.tpl.d;
        g.tpl = super::template(g.look, g.px * g.ui, &g.th);
        if g.tpl.d != d {
            g.frame = None;
        }
        self.shared.side.store(g.tpl.d, Ordering::SeqCst);
    }

    fn pointer(&self, ev: crate::wind::AuxEv) {
        {
            let mut g = self.geo.borrow_mut();
            match ev.kind {
                AuxKind::Down => g.drag = Some((ev.global, g.pos)),
                AuxKind::Drag => {
                    let Some((c0, p0)) = g.drag else { return };
                    let at = (p0.0 + (ev.global.0 - c0.0).round() as i32, p0.1 + (ev.global.1 - c0.1).round() as i32);
                    g.pos = super::snap(at, g.area, g.d(), g.ui);
                }
                AuxKind::Up => {
                    if g.drag.take().is_none() {
                        return;
                    }
                    // At rest in the nearest corner, opaque again.
                    let d = g.d();
                    g.pos = super::settle(g.pos, g.area, d, g.ui);
                    super::remember(super::rel_of(g.pos, g.area, d, g.ui));
                }
            }
        }
        self.paint();
    }
}

/// On the main thread: repaint the bubbles that have a new picture.
extern "C" fn repaint_live(_: *mut core::ffi::c_void) {
    let live: Vec<Rc<Ui>> = LIVE.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|w| w.strong_count() > 0);
        l.iter().filter_map(Weak::upgrade).collect()
    });
    for ui in live {
        ui.shared.queued.store(false, Ordering::SeqCst);
        ui.refresh();
    }
}

fn camera_thread(open: Opener, sh: Arc<Shared>, done: mpsc::Sender<()>) {
    let ask = |sh: &Shared| {
        if !sh.queued.swap(true, Ordering::SeqCst) {
            objc::on_main(core::ptr::null_mut(), repaint_live);
        }
    };
    let r = (|| -> Result<()> {
        let mut src = open()?;
        let mut last: Option<Instant> = None;
        let (mut raw, mut tmp) = (CamFrame::default(), Vec::new());
        let mut fx = super::bg::Effect::default();
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
            fx.apply(super::bg::Bg::from_u8(sh.bg.load(Ordering::SeqCst)), &raw, &mut sq);
            if let Some(old) = lock(&sh.fresh).replace(sq) {
                *lock(&sh.spare) = Some(old);
            }
            sh.picture.store(true, Ordering::SeqCst);
            ask(&sh);
        }
        Ok(())
    })();
    if let Err(e) = r
        && !sh.stop.load(Ordering::SeqCst)
    {
        let msg = format!("{e:#}");
        eprintln!("rustshot: camera: {msg}");
        *lock(&sh.error) = Some(msg);
        ask(&sh);
    }
    let _ = done.send(());
}

/// The camera bubble on screen (see the module docs). Main thread only.
pub struct Bubble {
    ui: Rc<Ui>,
    device: String,
    _cam_done: Option<Receiver<()>>,
}

impl Bubble {
    /// Show the bubble for camera `device` ("" = the first) with `look`,
    /// placed where it was last left relative to `area` (pixels). `_scale`
    /// and `_nudge` are not needed here: the bubble is placed in points
    /// and drawn at its display's density, and it repaints itself.
    pub fn show(look: Look, device: &str, area: IRect, _scale: f32, th: &Theme, _nudge: crate::wind::Hwnd) -> Result<Bubble> {
        let area = crate::capture::to_points_i(area);
        let layer = Layer::new(false, false)?;
        let px = scale_of(area);
        let mut geo = Geo {
            area,
            look,
            th: *th,
            avoid: Vec::new(),
            screen: area,
            pos: (0, 0),
            drag: None,
            px,
            ui: crate::uisize::mult(),
            tpl: super::template(look, px * crate::uisize::mult(), th),
            frame: None,
            hidden: false,
        };
        geo.pos = geo.home();
        let shared = Arc::new(Shared::default());
        shared.side.store(geo.tpl.d, Ordering::SeqCst);
        shared.bg.store(look.bg.to_u8(), Ordering::SeqCst);
        let ui = Rc::new(Ui { geo: RefCell::new(geo), layer: RefCell::new(layer), shared: shared.clone(), px: RefCell::new(Vec::new()) });
        let weak = Rc::downgrade(&ui);
        ui.layer.borrow().on_pointer(move |ev| {
            if let Some(ui) = weak.upgrade() {
                ui.pointer(ev);
            }
        });
        LIVE.with(|l| l.borrow_mut().push(Rc::downgrade(&ui)));
        ui.paint();
        let id = device.to_string();
        let open: Opener = Box::new(move || super::open(&id, super::PREF));
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || camera_thread(open, shared, tx))
            .context("start the camera thread")?;
        Ok(Bubble { ui, device: device.to_string(), _cam_done: Some(rx) })
    }

    /// The camera it shows ("" = the first).
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Why the camera stopped (the bubble is hidden then).
    pub fn error(&self) -> Option<String> {
        lock(&self.ui.shared.error).clone()
    }

    /// The shape's top-left on screen (pixels).
    #[allow(dead_code)] // Windows tests use it; kept alike
    pub fn pos(&self) -> Option<(i32, i32)> {
        let (x, y) = self.ui.geo.borrow().pos;
        let (px, py, _, _) = crate::capture::from_points((f64::from(x), f64::from(y), 0.0, 0.0));
        Some((px, py))
    }

    /// The window's id: the recording keeps it in (see `rec::macos::sck`).
    pub fn window_id(&self) -> u32 {
        self.ui.layer.borrow().id()
    }

    /// The look it has now.
    #[allow(dead_code)] // Windows tests use it; kept alike
    pub fn look(&self) -> Option<Look> {
        Some(self.ui.geo.borrow().look)
    }

    /// Being dragged (drawn at [`super::DRAG_OPACITY`]).
    #[allow(dead_code)] // kept alike
    pub fn dragging(&self) -> bool {
        self.ui.geo.borrow().drag.is_some()
    }

    /// Hide it during the recording (the camera button) or show it again.
    pub fn set_hidden(&mut self, hidden: bool) {
        let changed = {
            let mut g = self.ui.geo.borrow_mut();
            std::mem::replace(&mut g.hidden, hidden) != hidden
        };
        if changed {
            self.ui.paint();
        }
    }

    /// A camera picture came.
    pub fn has_picture(&self) -> bool {
        self.ui.shared.picture.load(Ordering::SeqCst)
    }

    /// Wait up to `timeout` for the camera's first picture (and show it);
    /// false when none came (or the camera failed).
    pub fn wait_picture(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if self.has_picture() {
                self.ui.refresh();
                return true;
            }
            if self.error().is_some() || Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Follow `area` (placed where it was left relative to it). Not while
    /// dragged.
    pub fn set_anchor(&mut self, area: IRect, _scale: f32) {
        let area = crate::capture::to_points_i(area);
        let px = scale_of(area);
        {
            let mut g = self.ui.geo.borrow_mut();
            if (g.area, g.px) == (area, px) || g.drag.is_some() {
                return;
            }
            if g.px != px {
                g.px = px;
                self.ui.retemplate(&mut g);
            }
            g.area = area;
            g.pos = g.home();
        }
        self.ui.paint();
    }

    /// Keep off `avoid` (the overlay's toolbar and Record popover), going
    /// outside a small area within `screen` if it must. Not while dragged.
    pub fn set_avoid(&mut self, avoid: &[IRect], screen: IRect) {
        // Pixels in, points here.
        let avoid: Vec<IRect> = avoid.iter().map(|&r| crate::capture::to_points_i(r)).collect();
        let screen = crate::capture::to_points_i(screen);
        self.set_avoid_pt(&avoid, screen);
    }

    fn set_avoid_pt(&mut self, avoid: &[IRect], screen: IRect) {
        {
            let mut g = self.ui.geo.borrow_mut();
            if (g.avoid.as_slice(), g.screen) == (avoid, screen) || g.drag.is_some() {
                return;
            }
            g.avoid = avoid.to_vec();
            g.screen = screen;
            let pos = g.home();
            if pos == g.pos {
                return;
            }
            g.pos = pos;
        }
        self.ui.paint();
    }

    /// Keep off nothing any more (the overlay is gone).
    pub fn clear_avoid(&mut self) {
        let screen = self.ui.geo.borrow().screen;
        self.set_avoid_pt(&[], screen);
    }

    /// Change the shape, size or mirroring (and the theme's accent).
    pub fn set_look(&mut self, look: Look, th: &Theme) {
        self.ui.shared.bg.store(look.bg.to_u8(), Ordering::SeqCst);
        {
            let mut g = self.ui.geo.borrow_mut();
            if g.look == look && g.th == *th {
                return;
            }
            if (Look { bg: g.look.bg, ..look }) == g.look && g.th == *th {
                // Only the background: the camera thread's next picture.
                g.look = look;
                return;
            }
            let resized = g.look.size != look.size;
            g.look = look;
            g.th = *th;
            self.ui.retemplate(&mut g);
            if resized {
                g.pos = g.home();
            }
        }
        self.ui.paint();
    }
}

impl Drop for Bubble {
    /// Stop the camera (its thread lets go of it after its current read)
    /// and take the window down.
    fn drop(&mut self) {
        self.ui.shared.stop.store(true, Ordering::SeqCst);
        let me = Rc::as_ptr(&self.ui);
        LIVE.with(|l| l.borrow_mut().retain(|w| w.as_ptr() != me));
    }
}
