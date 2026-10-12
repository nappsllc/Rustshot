//! macOS: the windows around a recording (red border strips, the control
//! bar, the countdown), transparent non-activating panels
//! ([`crate::wind::Layer`]) above other windows, and the worker that opens
//! the pipeline: ScreenCaptureKit frames and system sound, the
//! microphone, `AVAssetWriter` (MP4) or the GIF encoder.
//!
//! The recording filters our application's windows out of the capture
//! (all but the camera bubble), so the bar sits beside the area or, when
//! there is no room, inside it as on Windows.
//!
//! Main thread: AppKit windows can only be made and changed there. The
//! daemon's main thread runs the overlay's event pump (`wind_macos`), and
//! [`super::Run`] lives in the overlay's driver, so these windows are made,
//! updated and dropped on it. Their pointer events are taken out of that
//! pump before the overlay sees them (`wind_macos::aux_event`) and queued
//! for the driver's next tick ([`Recorder::take_clicks`]). Geometry is in
//! points (the space of the overlay and the area); the pixels are drawn at
//! the display's density. The editor hands over pixels (points × the
//! backing factor, see `capture::pts`): [`Recorder::start`] converts its
//! geometry to points once ([`Geom::in_points`]), and [`prepare`] the
//! recorded area for ScreenCaptureKit.

use super::bar::{self, BarAct, BarView};
pub(crate) use super::worker::{Opener, Parts, Worker};
use super::{border, countdown, BarSpot, Effect, End, Geom, MicFlags};
use crate::editor::style::Ui;
use crate::objects::Pt;
use crate::rec::gif::GifEncoder;
use crate::rec::macos::{self as recmac, avwriter::AvWriterEncoder, mic, sck};
use crate::rec::{AudioSource, FrameSource, RecFormat, RecSpec, VideoEncoder};
use crate::theme::Theme;
use crate::wind::{AuxKind, Layer};
use anyhow::{Context, Result};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

/// Pixels per point at (x, y) points.
fn scale_at(x: i32, y: i32) -> f32 {
    (recmac::backing_scale_at(f64::from(x), f64::from(y)) as f32).clamp(1.0, 4.0)
}

/// Input on the control bar, queued for the owner's tick.
#[derive(Default)]
struct BarInput {
    layout: Option<bar::BarLayout>,
    /// Pixels per point it is drawn at.
    scale: f64,
    pressed: Option<BarAct>,
    clicks: Vec<BarAct>,
}

/// The control bar window.
struct BarWin {
    layer: Layer,
    input: Rc<RefCell<BarInput>>,
    spot: Option<BarSpot>,
    drawn: Option<(BarView, Option<BarAct>, f32)>,
}

impl BarWin {
    fn new() -> Result<BarWin> {
        let layer = Layer::new(false, true)?;
        let input = Rc::new(RefCell::new(BarInput::default()));
        let inp = input.clone();
        layer.on_pointer(move |ev| {
            let mut b = inp.borrow_mut();
            let p = Pt::new((ev.local.0 * b.scale) as f32, (ev.local.1 * b.scale) as f32);
            let at = b.layout.as_ref().and_then(|l| bar::hit(l, p));
            match ev.kind {
                AuxKind::Down => b.pressed = at,
                AuxKind::Drag => {}
                AuxKind::Up => {
                    if let Some(a) = at
                        && b.pressed == Some(a)
                    {
                        b.clicks.push(a);
                    }
                    b.pressed = None;
                }
            }
        });
        Ok(BarWin { layer, input, spot: None, drawn: None })
    }

    /// Draw `v` (when it changed), placed by `geom` (points) the first time
    /// and again when its size changes.
    fn show(&mut self, geom: &Geom, th: &Theme, v: &BarView) {
        let font = Some(crate::fonts::ui_font());
        let spot = super::spot_bar_with(
            geom,
            self.spot,
            |s| {
                let p = bar::layout(&Ui { th, s, font }, v).panel;
                (p.w.round() as i32, p.h.round() as i32)
            },
            |area, rects, sz, gap| super::place_bar(area, rects, sz, gap, gap),
        );
        let moved = self.spot != Some(spot);
        self.spot = Some(spot);
        // Pixels per point there; the UI size on top (`spot.scale`).
        let px = scale_at(spot.pos.0 + spot.size.0 / 2, spot.pos.1 + spot.size.1 / 2);
        let s = px * spot.scale;
        let ui = Ui { th, s, font };
        let l = bar::layout(&ui, v);
        let pressed = {
            let mut b = self.input.borrow_mut();
            b.layout = Some(l.clone());
            b.scale = f64::from(px);
            b.pressed
        };
        let state = (v.clone(), pressed, s);
        if !moved && self.drawn.as_ref() == Some(&state) {
            return;
        }
        let (w, h) = l.size;
        let pixels = super::render_layer(w as usize, h as usize, |f| bar::draw(f, &ui, &l, v, None, pressed));
        let m = f64::from(bar::MARGIN * spot.scale);
        let at = (f64::from(spot.pos.0) - m, f64::from(spot.pos.1) - m);
        self.layer.put(at, f64::from(px), w, h, &pixels);
        self.drawn = Some(state);
    }

    fn take_clicks(&mut self) -> Vec<BarAct> {
        std::mem::take(&mut self.input.borrow_mut().clicks)
    }
}

/// The countdown window.
struct CountWin {
    layer: Layer,
    /// The number and the arc (in 60ths of a second) on screen.
    drawn: Option<(u32, u16)>,
}

impl CountWin {
    fn show(&mut self, geom: &Geom, th: &Theme, n: Option<(u32, f32)>) {
        let key = n.map(|(n, f)| (n, (f * 60.0).round() as u16));
        if key == self.drawn {
            return;
        }
        self.drawn = key;
        let Some((n, frac)) = n else {
            self.layer.hide();
            return;
        };
        let a = geom.area;
        // Pixels per point there, the UI size on top.
        let k = scale_at(a.0 + a.2 as i32 / 2, a.1 + a.3 as i32 / 2);
        let s = k * geom.area_scale();
        let ui = Ui { th, s, font: Some(crate::fonts::ui_font()) };
        let (w, h) = countdown::size(s);
        let px = super::render_layer(w as usize, h as usize, |f| countdown::draw(f, &ui, n, frac));
        let pts = ((f64::from(w) / f64::from(k)).round() as i32, (f64::from(h) / f64::from(k)).round() as i32);
        let (x, y) = super::centre_on(a, pts);
        self.layer.put((f64::from(x), f64::from(y)), f64::from(k), w, h, &px);
    }
}

/// The red border: one opaque strip window per side.
fn show_border(geom: &Geom) -> Result<Vec<Layer>> {
    let t = border::thickness(1.0); // points
    let mons: Vec<_> = geom.monitors.iter().map(|m| (m.0, t)).collect();
    let red = super::RED;
    let mut out = Vec::new();
    for (x, y, w, h) in border::strips(geom.area, &mons) {
        let mut l = Layer::new(true, true)?;
        l.fill((f64::from(x), f64::from(y), f64::from(w), f64::from(h)), [red.r, red.g, red.b]);
        out.push(l);
    }
    Ok(out)
}

/// Open the capture, the sound and the encoder for `spec` (the encoder
/// creates `.part`); `keep` are our windows that are recorded (the camera
/// bubble). Sound that cannot be opened is left out and said.
fn prepare(spec: &RecSpec, mic_flags: MicFlags, keep: &[u32]) -> Result<Parts> {
    if let Some(dir) = spec.out.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let mp4 = spec.format == RecFormat::Mp4;
    // The area is in the editor's pixels; the stream wants points.
    let area = crate::capture::to_points_i(spec.area);
    let opened = sck::open(area, spec.capture_fps(), mp4, mp4 && spec.system_audio, keep)?;
    let (w, h) = opened.video.size();
    let mut audio: Vec<Box<dyn AudioSource>> = Vec::new();
    let mut warnings = Vec::new();
    let mut late: Vec<super::worker::Late> = Vec::new();
    match opened.audio {
        Some(Ok(a)) => {
            late.push(dropped_note("system sound", a.feed.clone()));
            audio.push(Box::new(mic_flags.system(a)));
        }
        Some(Err(e)) => warnings.push(format!("system sound not recorded: {e}")),
        None => {}
    }
    if mp4 && let Some(id) = &spec.mic {
        match mic::open(id) {
            Ok(a) => {
                late.push(dropped_note("microphone", a.feed.clone()));
                audio.push(Box::new(mic_flags.opened(a)));
            }
            Err(e) => warnings.push(format!("microphone not recorded: {e:#}")),
        }
    }
    for w in &warnings {
        eprintln!("rustshot: {w}");
    }
    let part = spec.part_path();
    let enc: Box<dyn VideoEncoder> = match spec.format {
        RecFormat::Mp4 => Box::new(AvWriterEncoder::create(&part, w, h, spec.capture_fps(), spec.quality, !audio.is_empty())?),
        RecFormat::Gif => Box::new(GifEncoder::create(&part, w, h, spec.capture_fps())?),
    };
    Ok(Parts { src: Box::new(opened.video), audio, enc, warnings, late })
}

/// Said with the saved file when some of `what`'s sound was dropped
/// (unreadable buffers, unusable timestamps).
fn dropped_note(what: &'static str, feed: std::sync::Arc<crate::rec::macos::media::PcmFeed>) -> super::worker::Late {
    Box::new(move || feed.problem().map(|e| format!("{what} partly not recorded: {e}")))
}

/// The windows and the worker of one recording.
pub struct Recorder {
    worker: Worker,
    geom: Geom,
    th: Theme,
    border: Vec<Layer>,
    bar: Option<BarWin>,
    count: Option<CountWin>,
    /// The camera bubble: up until the recording is over.
    bubble: Option<crate::cam::Bubble>,
}

impl Recorder {
    /// Show the border and the bar, then open the pipeline on the worker
    /// (our windows are on screen by then, so the stream's filter finds our
    /// application). When a window or the worker cannot be made, the ones
    /// already up are taken down (dropped) before the error returns.
    pub fn start(spec: RecSpec, geom: Geom, th: Theme, mic: MicFlags, bubble: Option<crate::cam::Bubble>) -> Result<Recorder> {
        // `geom` is in pixels; the windows are placed in points.
        let geom = geom.in_points(crate::capture::backing());
        let border = show_border(&geom)?;
        let bar = BarWin::new()?;
        let count = CountWin { layer: Layer::new(true, true)?, drawn: None };
        let keep: Vec<u32> = bubble.iter().map(|b| b.window_id()).collect();
        let open: Opener = Box::new(move |s: &RecSpec| prepare(s, mic, &keep));
        let worker = Worker::spawn(spec, open)?;
        Ok(Recorder { worker, geom, th, border, bar: Some(bar), count: Some(count), bubble })
    }

    pub fn send(&mut self, e: Effect) {
        self.worker.send(e);
    }

    pub fn take_clicks(&mut self) -> Vec<BarAct> {
        self.bar.as_mut().map(|b| b.take_clicks()).unwrap_or_default()
    }

    pub fn poll_end(&mut self) -> Option<End> {
        match self.worker.rx.try_recv() {
            // A camera that stopped meanwhile is said with the file.
            Ok(e) => Some(super::with_camera_error(e, self.bubble.as_ref().and_then(|b| b.error()))),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(End::Failed("the recording thread stopped".into())),
        }
    }

    /// Hide the camera bubble or show it again (the pill's camera button).
    pub fn set_cam_hidden(&mut self, hidden: bool) {
        if let Some(b) = &mut self.bubble {
            b.set_hidden(hidden);
        }
    }

    pub fn show(&mut self, v: &BarView, count: Option<(u32, f32)>) {
        if let Some(b) = &mut self.bar {
            b.show(&self.geom, &self.th, v);
        }
        if let Some(c) = &mut self.count {
            c.show(&self.geom, &self.th, count);
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
