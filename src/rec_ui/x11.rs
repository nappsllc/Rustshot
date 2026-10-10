//! X11: the windows around a recording (red border strips, the control
//! bar, the countdown) and the pipeline's parts for a spec.
//!
//! The windows are override-redirect (no decorations, above normal
//! windows) on the recorder's own Display connection, used from the UI
//! thread only; the bar's input is read when the owner ticks
//! ([`Recorder::take_clicks`]). X11 cannot keep a window out of the
//! capture, so nothing is drawn over the recorded area: the border strips
//! lie outside it (a side that touches the screen edge has none) and the
//! bar goes beside it (inside only when it fits nowhere else, e.g. a full
//! screen recording). The countdown is taken down before the first frame.
//!
//! With a compositor the bar and countdown use a 32-bit ARGB visual;
//! without one they are cut to shape (alpha ≥ 50%) with the Shape
//! extension. The border and countdown take no input (an empty input
//! shape).

use super::bar::{self, BarAct, BarView};
use super::worker::{Opener, Parts, Worker};
use super::{border, countdown, BarSpot, Effect, End, Geom};
use crate::editor::style::Ui;
use crate::objects::Pt;
use crate::rec::gif::GifEncoder;
use crate::rec::linux::ffmpeg::{self, FfmpegEncoder};
use crate::rec::linux::pulse_list;
use crate::rec::linux::xshm::XShmSource;
use crate::rec::{FrameSource, RecFormat, RecSpec, VideoEncoder};
use crate::theme::Theme;
use anyhow::{anyhow, bail, Context, Result};
use core::ffi::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_ushort, c_void};
use std::sync::mpsc;

#[repr(C)]
struct XSetWindowAttributes {
    background_pixmap: c_ulong,
    background_pixel: c_ulong,
    border_pixmap: c_ulong,
    border_pixel: c_ulong,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: c_int,
    event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: c_int,
    colormap: c_ulong,
    cursor: c_ulong,
}

#[repr(C)]
struct XVisualInfo {
    visual: *mut c_void,
    visualid: c_ulong,
    screen: c_int,
    depth: c_int,
    class: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    colormap_size: c_int,
    bits_per_rgb: c_int,
}

/// Button, motion and crossing events share this head.
#[repr(C)]
#[derive(Clone, Copy)]
struct XPointerEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    window: c_ulong,
    root: c_ulong,
    subwindow: c_ulong,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    button: c_uint,
}

#[repr(C)]
union XEvent {
    type_: c_int,
    pointer: XPointerEvent,
    pad: [c_long; 24],
}

#[repr(C)]
struct XRectangle {
    x: c_short,
    y: c_short,
    width: c_ushort,
    height: c_ushort,
}

/// The head of XImage, for its `data` field (as in capture_linux).
#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut c_char,
}

#[allow(clashing_extern_declarations)]
#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(dpy: *mut c_void) -> c_int;
    fn XDefaultScreen(dpy: *mut c_void) -> c_int;
    fn XRootWindow(dpy: *mut c_void, screen: c_int) -> c_ulong;
    fn XDefaultVisual(dpy: *mut c_void, screen: c_int) -> *mut c_void;
    fn XDefaultDepth(dpy: *mut c_void, screen: c_int) -> c_int;
    fn XMatchVisualInfo(dpy: *mut c_void, screen: c_int, depth: c_int, class: c_int, info: *mut XVisualInfo) -> c_int;
    fn XCreateColormap(dpy: *mut c_void, w: c_ulong, visual: *mut c_void, alloc: c_int) -> c_ulong;
    fn XFreeColormap(dpy: *mut c_void, cmap: c_ulong) -> c_int;
    fn XInternAtom(dpy: *mut c_void, name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XGetSelectionOwner(dpy: *mut c_void, selection: c_ulong) -> c_ulong;
    fn XCreateWindow(
        dpy: *mut c_void,
        parent: c_ulong,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        border_width: c_uint,
        depth: c_int,
        class: c_uint,
        visual: *mut c_void,
        valuemask: c_ulong,
        attributes: *mut XSetWindowAttributes,
    ) -> c_ulong;
    fn XDestroyWindow(dpy: *mut c_void, w: c_ulong) -> c_int;
    fn XMapRaised(dpy: *mut c_void, w: c_ulong) -> c_int;
    fn XRaiseWindow(dpy: *mut c_void, w: c_ulong) -> c_int;
    fn XUnmapWindow(dpy: *mut c_void, w: c_ulong) -> c_int;
    fn XMoveResizeWindow(dpy: *mut c_void, w: c_ulong, x: c_int, y: c_int, width: c_uint, height: c_uint) -> c_int;
    fn XCreateGC(dpy: *mut c_void, d: c_ulong, valuemask: c_ulong, values: *mut c_void) -> *mut c_void;
    fn XFreeGC(dpy: *mut c_void, gc: *mut c_void) -> c_int;
    fn XCreateImage(
        dpy: *mut c_void,
        visual: *mut c_void,
        depth: c_uint,
        format: c_int,
        offset: c_int,
        data: *mut c_char,
        width: c_uint,
        height: c_uint,
        bitmap_pad: c_int,
        bytes_per_line: c_int,
    ) -> *mut XImage;
    fn XPutImage(
        dpy: *mut c_void,
        d: c_ulong,
        gc: *mut c_void,
        image: *mut XImage,
        src_x: c_int,
        src_y: c_int,
        dest_x: c_int,
        dest_y: c_int,
        width: c_uint,
        height: c_uint,
    ) -> c_int;
    fn XDestroyImage(image: *mut XImage) -> c_int;
    fn XPending(dpy: *mut c_void) -> c_int;
    fn XNextEvent(dpy: *mut c_void, event: *mut XEvent) -> c_int;
    fn XFlush(dpy: *mut c_void) -> c_int;
    fn XSync(dpy: *mut c_void, discard: c_int) -> c_int;
}

#[link(name = "Xext")]
unsafe extern "C" {
    fn XShapeCombineRectangles(
        dpy: *mut c_void,
        dest: c_ulong,
        dest_kind: c_int,
        x_off: c_int,
        y_off: c_int,
        rectangles: *mut XRectangle,
        n_rects: c_int,
        op: c_int,
        ordering: c_int,
    );
}

const TRUE_COLOR: c_int = 4;
const INPUT_OUTPUT: c_uint = 1;
const Z_PIXMAP: c_int = 2;
const CW_BACK_PIXEL: c_ulong = 1 << 1;
const CW_BORDER_PIXEL: c_ulong = 1 << 3;
const CW_OVERRIDE_REDIRECT: c_ulong = 1 << 9;
const CW_EVENT_MASK: c_ulong = 1 << 11;
const CW_COLORMAP: c_ulong = 1 << 13;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const LEAVE_WINDOW_MASK: c_long = 1 << 5;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const EXPOSURE_MASK: c_long = 1 << 15;
const BUTTON_PRESS: c_int = 4;
const BUTTON_RELEASE: c_int = 5;
const MOTION_NOTIFY: c_int = 6;
const LEAVE_NOTIFY: c_int = 8;
const EXPOSE: c_int = 12;
const SHAPE_BOUNDING: c_int = 0;
const SHAPE_INPUT: c_int = 2;
const SHAPE_SET: c_int = 0;
const UNSORTED: c_int = 0;

/// The recorder's Display: the screen, and the ARGB visual when a
/// compositor runs (else windows are cut to shape).
struct Disp {
    dpy: *mut c_void,
    screen: c_int,
    root: c_ulong,
    argb: Option<(*mut c_void, c_ulong)>,
}

impl Disp {
    fn open() -> Result<Disp> {
        crate::wind::init_x11();
        unsafe {
            let dpy = XOpenDisplay(core::ptr::null());
            if dpy.is_null() {
                bail!("cannot open the X display");
            }
            let screen = XDefaultScreen(dpy);
            let root = XRootWindow(dpy, screen);
            let name = format!("_NET_WM_CM_S{screen}\0");
            let cm = XInternAtom(dpy, name.as_ptr() as *const c_char, 0);
            let composited = cm != 0 && XGetSelectionOwner(dpy, cm) != 0;
            let mut argb = None;
            if composited {
                let mut vi: XVisualInfo = core::mem::zeroed();
                if XMatchVisualInfo(dpy, screen, 32, TRUE_COLOR, &mut vi) != 0 && !vi.visual.is_null() {
                    argb = Some((vi.visual, XCreateColormap(dpy, root, vi.visual, 0)));
                }
            }
            Ok(Disp { dpy, screen, root, argb })
        }
    }
}

impl Drop for Disp {
    fn drop(&mut self) {
        unsafe {
            if let Some((_, cmap)) = self.argb {
                XFreeColormap(self.dpy, cmap);
            }
            XCloseDisplay(self.dpy);
        }
    }
}

/// Rows of `bgra` (`w`×`h`) cut into runs of pixels at least half opaque,
/// as (x, y, w, h) rectangles one pixel high (a window's shape).
pub(crate) fn alpha_runs(bgra: &[u8], w: usize, h: usize) -> Vec<(i16, i16, u16, u16)> {
    let mut out = Vec::new();
    for y in 0..h {
        let row = &bgra[y * w * 4..(y + 1) * w * 4];
        let mut x = 0;
        while x < w {
            if row[x * 4 + 3] >= 128 {
                let x0 = x;
                while x < w && row[x * 4 + 3] >= 128 {
                    x += 1;
                }
                out.push((x0 as i16, y as i16, (x - x0) as u16, 1));
            } else {
                x += 1;
            }
        }
    }
    out
}

/// An override-redirect window showing premultiplied BGRA pixels.
struct Win {
    dpy: *mut c_void,
    win: c_ulong,
    gc: *mut c_void,
    visual: *mut c_void,
    depth: c_int,
    argb: bool,
    mapped: bool,
    at: (i32, i32, u32, u32),
}

impl Win {
    /// `input`: gets pointer events (else click-through). `solid`: a
    /// plain colour window (the border), drawn by the server.
    fn new(d: &Disp, input: bool, solid: Option<u32>) -> Result<Win> {
        unsafe {
            let mut a: XSetWindowAttributes = core::mem::zeroed();
            a.override_redirect = 1;
            a.border_pixel = 0;
            a.background_pixel = solid.map_or(0, c_ulong::from);
            a.event_mask = if input {
                BUTTON_PRESS_MASK | BUTTON_RELEASE_MASK | POINTER_MOTION_MASK | LEAVE_WINDOW_MASK | EXPOSURE_MASK
            } else {
                EXPOSURE_MASK
            };
            let mut mask = CW_BACK_PIXEL | CW_BORDER_PIXEL | CW_OVERRIDE_REDIRECT | CW_EVENT_MASK;
            let argb = d.argb.filter(|_| solid.is_none());
            let (visual, depth) = match argb {
                Some((v, cmap)) => {
                    a.colormap = cmap;
                    mask |= CW_COLORMAP;
                    (v, 32)
                }
                None => (XDefaultVisual(d.dpy, d.screen), XDefaultDepth(d.dpy, d.screen)),
            };
            let win = XCreateWindow(d.dpy, d.root, 0, 0, 1, 1, 0, depth, INPUT_OUTPUT, visual, mask, &mut a);
            if win == 0 {
                bail!("create a recording window");
            }
            if !input {
                XShapeCombineRectangles(d.dpy, win, SHAPE_INPUT, 0, 0, core::ptr::null_mut(), 0, SHAPE_SET, UNSORTED);
            }
            let gc = XCreateGC(d.dpy, win, 0, core::ptr::null_mut());
            Ok(Win { dpy: d.dpy, win, gc, visual, depth, argb: argb.is_some(), mapped: false, at: (0, 0, 0, 0) })
        }
    }

    fn place(&mut self, x: i32, y: i32, w: u32, h: u32) {
        unsafe {
            if self.at != (x, y, w, h) {
                XMoveResizeWindow(self.dpy, self.win, x, y, w.max(1), h.max(1));
                self.at = (x, y, w, h);
            }
            if self.mapped {
                XRaiseWindow(self.dpy, self.win);
            } else {
                XMapRaised(self.dpy, self.win);
                self.mapped = true;
            }
        }
    }

    /// Show `bgra` (`w`×`h`, premultiplied) at (x, y).
    fn put(&mut self, x: i32, y: i32, w: u32, h: u32, bgra: &[u8]) {
        if w == 0 || h == 0 || bgra.len() < (w * h * 4) as usize {
            self.hide();
            return;
        }
        unsafe {
            if !self.argb {
                let mut rects: Vec<XRectangle> = alpha_runs(bgra, w as usize, h as usize)
                    .into_iter()
                    .map(|(x, y, width, height)| XRectangle { x, y, width, height })
                    .collect();
                let n = rects.len() as c_int;
                XShapeCombineRectangles(self.dpy, self.win, SHAPE_BOUNDING, 0, 0, rects.as_mut_ptr(), n, SHAPE_SET, UNSORTED);
            }
            self.place(x, y, w, h);
            let mut px = bgra[..(w * h * 4) as usize].to_vec();
            let img = XCreateImage(self.dpy, self.visual, self.depth as c_uint, Z_PIXMAP, 0, px.as_mut_ptr() as *mut c_char, w, h, 32, 0);
            if !img.is_null() {
                XPutImage(self.dpy, self.win, self.gc, img, 0, 0, 0, 0, w, h);
                (*img).data = core::ptr::null_mut(); // ours, not Xlib's to free
                XDestroyImage(img);
            }
            XFlush(self.dpy);
        }
    }

    fn hide(&mut self) {
        if self.mapped {
            unsafe {
                XUnmapWindow(self.dpy, self.win);
                XFlush(self.dpy);
            }
            self.mapped = false;
        }
    }
}

impl Drop for Win {
    fn drop(&mut self) {
        unsafe {
            XFreeGC(self.dpy, self.gc);
            XDestroyWindow(self.dpy, self.win);
            XFlush(self.dpy);
        }
    }
}

/// The control bar: its window, where it is, what it shows and its input.
struct BarWin {
    win: Win,
    spot: Option<BarSpot>,
    layout: Option<bar::BarLayout>,
    hover: Option<BarAct>,
    pressed: Option<BarAct>,
    clicks: Vec<BarAct>,
    drawn: Option<(BarView, Option<BarAct>, Option<BarAct>)>,
}

impl BarWin {
    fn show(&mut self, geom: &Geom, th: &Theme, v: &BarView) {
        let font = Some(crate::fonts::ui_font());
        let spot = super::spot_bar(geom, self.spot, |s| {
            let p = bar::layout(&Ui { th, s, font }, v).panel;
            (p.w.round() as i32, p.h.round() as i32)
        });
        let moved = self.spot != Some(spot);
        self.spot = Some(spot);
        let ui = Ui { th, s: spot.scale, font };
        let l = bar::layout(&ui, v);
        let state = (v.clone(), self.hover, self.pressed);
        self.layout = Some(l.clone());
        if !moved && self.drawn.as_ref() == Some(&state) {
            return;
        }
        let m = (bar::MARGIN * spot.scale).round() as i32;
        let (w, h) = l.size;
        let (hover, pressed) = (self.hover, self.pressed);
        let px = super::render_layer(w as usize, h as usize, |f| bar::draw(f, &ui, &l, v, hover, pressed));
        self.win.put(spot.pos.0 - m, spot.pos.1 - m, w, h, &px);
        self.drawn = Some(state);
    }

    fn event(&mut self, e: &XPointerEvent) {
        let at = self.layout.as_ref().and_then(|l| bar::hit(l, Pt::new(e.x as f32, e.y as f32)));
        match e.type_ {
            BUTTON_PRESS if e.button == 1 => self.pressed = at,
            BUTTON_RELEASE if e.button == 1 => {
                if let Some(a) = at
                    && self.pressed == Some(a)
                {
                    self.clicks.push(a);
                }
                self.pressed = None;
            }
            MOTION_NOTIFY => self.hover = at,
            LEAVE_NOTIFY => self.hover = None,
            EXPOSE => self.drawn = None,
            _ => {}
        }
    }
}

/// The countdown window.
struct CountWin {
    win: Win,
    drawn: Option<u32>,
}

impl CountWin {
    fn show(&mut self, geom: &Geom, th: &Theme, n: Option<u32>) {
        if n == self.drawn {
            return;
        }
        self.drawn = n;
        let Some(n) = n else {
            self.win.hide();
            return;
        };
        let s = geom.area_scale();
        let ui = Ui { th, s, font: Some(crate::fonts::ui_font()) };
        let (w, h) = countdown::size(s);
        let px = super::render_layer(w as usize, h as usize, |f| countdown::draw(f, &ui, n));
        let (x, y) = super::centre_on(geom.area, (w as i32, h as i32));
        self.win.put(x, y, w, h, &px);
    }
}

/// Open the capture and the encoder for `spec` (ffmpeg records the sound;
/// what cannot be recorded is left out and said).
fn prepare(spec: &RecSpec) -> Result<Parts> {
    if let Some(dir) = spec.out.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let src = XShmSource::open(spec.area)?;
    let (w, h) = src.size();
    let mut warnings = Vec::new();
    let part = spec.part_path();
    let enc: Box<dyn VideoEncoder> = match spec.format {
        RecFormat::Mp4 => {
            let exe = ffmpeg::find().ok_or_else(|| anyhow!("{}", super::NO_MP4))?;
            let caps = ffmpeg::caps(&exe);
            let (mut system, mut mic) = (None, None);
            if spec.system_audio || spec.mic.is_some() {
                let all = if caps.pulse && caps.aac { pulse_list::list() } else { None };
                let why = |e: String| {
                    if caps.pulse && caps.aac { e } else { format!("{} cannot record PulseAudio sound", exe.display()) }
                };
                if spec.system_audio {
                    match pulse_list::system_source(all.as_deref()) {
                        Ok(s) => system = Some(s),
                        Err(e) => warnings.push(format!("system sound not recorded: {}", why(e))),
                    }
                }
                if let Some(id) = &spec.mic {
                    match pulse_list::mic_source(all.as_deref(), id) {
                        Ok(s) => mic = Some(s),
                        Err(e) => warnings.push(format!("microphone not recorded: {}", why(e))),
                    }
                }
            }
            let e = FfmpegEncoder::create(&part, w, h, spec.capture_fps(), spec.quality, system, mic)?;
            eprintln!("rustshot: recording {w}x{h} with ffmpeg ({})", e.codec().name());
            Box::new(e)
        }
        RecFormat::Gif => Box::new(GifEncoder::create(&part, w, h, spec.capture_fps())?),
    };
    for w in &warnings {
        eprintln!("rustshot: {w}");
    }
    let src: Box<dyn FrameSource> = Box::new(src);
    Ok(Parts { src, audio: Vec::new(), enc, warnings })
}

/// The windows and the worker of one recording.
pub struct Recorder {
    worker: Worker,
    geom: Geom,
    th: Theme,
    border: Vec<Win>,
    bar: Option<BarWin>,
    count: Option<CountWin>,
    /// The camera bubble (none on Linux yet): up until the recording is over.
    bubble: Option<crate::cam::Bubble>,
    /// Last: the windows above go first.
    disp: Disp,
}

impl Recorder {
    pub fn start(spec: RecSpec, geom: Geom, th: Theme, bubble: Option<crate::cam::Bubble>) -> Result<Recorder> {
        let mut r = Recorder::start_with(spec, geom, th, Box::new(prepare))?;
        r.bubble = bubble;
        Ok(r)
    }

    pub(crate) fn start_with(spec: RecSpec, geom: Geom, th: Theme, open: Opener) -> Result<Recorder> {
        let disp = Disp::open()?;
        let mut border = Vec::new();
        let red = super::RED;
        let pixel = (u32::from(red.r) << 16) | (u32::from(red.g) << 8) | u32::from(red.b);
        for (x, y, w, h) in border::strips_outside(geom.area, &geom.border_monitors()) {
            let mut win = Win::new(&disp, false, Some(pixel))?;
            win.place(x, y, w, h);
            border.push(win);
        }
        let bar = BarWin {
            win: Win::new(&disp, true, None)?,
            spot: None,
            layout: None,
            hover: None,
            pressed: None,
            clicks: Vec::new(),
            drawn: None,
        };
        let count = CountWin { win: Win::new(&disp, false, None)?, drawn: None };
        unsafe {
            XFlush(disp.dpy);
        }
        let worker = Worker::spawn(spec, open)?;
        Ok(Recorder { worker, geom, th, border, bar: Some(bar), count: Some(count), bubble: None, disp })
    }

    pub fn send(&mut self, e: Effect) {
        if e == Effect::Go
            && let Some(c) = &mut self.count
        {
            // Off the screen before the first frame is grabbed.
            c.win.hide();
            c.drawn = None;
            unsafe {
                XSync(self.disp.dpy, 0);
            }
        }
        self.worker.send(e);
    }

    /// Read the bar's input; the clicks since the last call.
    pub fn take_clicks(&mut self) -> Vec<BarAct> {
        unsafe {
            while XPending(self.disp.dpy) > 0 {
                let mut ev: XEvent = core::mem::zeroed();
                XNextEvent(self.disp.dpy, &mut ev);
                let p = ev.pointer;
                if let Some(b) = &mut self.bar
                    && p.window == b.win.win
                {
                    b.event(&p);
                }
            }
        }
        self.bar.as_mut().map(|b| std::mem::take(&mut b.clicks)).unwrap_or_default()
    }

    pub fn poll_end(&mut self) -> Option<End> {
        match self.worker.rx.try_recv() {
            Ok(e) => Some(e),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(End::Failed("the recording thread stopped".into())),
        }
    }

    pub fn show(&mut self, v: &BarView, count: Option<u32>) {
        if let Some(b) = &mut self.bar {
            b.show(&self.geom, &self.th, v);
        }
        if let Some(c) = &mut self.count {
            c.show(&self.geom, &self.th, count);
        }
        // Stay above windows raised meanwhile.
        for w in &mut self.border {
            let (x, y, ww, h) = w.at;
            w.place(x, y, ww, h);
        }
        unsafe {
            XFlush(self.disp.dpy);
        }
    }

    /// The recording is over: take the windows down.
    pub fn close(&mut self) {
        self.bubble = None;
        self.count = None;
        self.bar = None;
        self.border.clear();
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_runs_follow_alpha() {
        // 4x2: row 0 = [opaque, opaque, clear, half]; row 1 = all clear.
        let mut px = vec![0u8; 4 * 2 * 4];
        for (i, a) in [255u8, 255, 0, 128].into_iter().enumerate() {
            px[i * 4 + 3] = a;
        }
        assert_eq!(alpha_runs(&px, 4, 2), [(0, 0, 2, 1), (3, 0, 1, 1)]);
    }
}
