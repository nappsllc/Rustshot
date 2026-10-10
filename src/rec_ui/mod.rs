//! Recording UI around the pipeline in `crate::rec`: the choices the
//! Record popover makes ([`RecOpts`]), the recording state machine
//! ([`Machine`]: idle → countdown → recording ⇄ paused → stopping → idle),
//! where the control bar goes ([`place_bar`]) and a running recording
//! ([`Run`]). The windows (red border, control bar, 3-2-1 countdown) are
//! per platform: Windows (`win.rs`) and X11 (`x11.rs`); the worker thread
//! that drives the pipeline is shared (`worker.rs`). Elsewhere (macOS,
//! Wayland sessions) the Record button is hidden.
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))] // no recording UI on macOS yet

pub mod bar;
pub mod border;
pub mod countdown;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub(crate) use win::Dib;
#[cfg(any(windows, target_os = "linux"))]
mod worker;
#[cfg(target_os = "linux")]
mod x11;

use crate::cam::{CamDevice, CamSize, Look, Shape};
use crate::capture::IRect;
use crate::config::Config;
use crate::rec::{AudioSource, RecFormat, RecSpec};
use crate::uifb::{Fb, C4};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Whether this platform records (shows the Record button): Windows, and
/// Linux in an X11 session (a Wayland session needs the ScreenCast portal,
/// which comes later; XShm would see only XWayland windows there).
pub fn supported() -> bool {
    #[cfg(any(windows, test))]
    {
        cfg!(any(windows, target_os = "linux"))
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        static X11: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *X11.get_or_init(|| {
            x11_session(std::env::var_os("WAYLAND_DISPLAY").as_deref(), std::env::var("XDG_SESSION_TYPE").ok().as_deref())
        })
    }
    #[cfg(all(not(windows), not(target_os = "linux"), not(test)))]
    {
        false
    }
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
/// An X11 session: no Wayland display, and the session type (when set)
/// is not Wayland.
pub fn x11_session(wayland_display: Option<&std::ffi::OsStr>, session_type: Option<&str>) -> bool {
    wayland_display.is_none_or(|d| d.is_empty()) && !session_type.is_some_and(|t| t.eq_ignore_ascii_case("wayland"))
}

/// Why MP4 is off on Linux without ffmpeg (the MP4 segment's tooltip).
pub const NO_MP4: &str = "MP4 needs ffmpeg (install it, or wait for the built-in fallback)";

#[cfg(test)]
thread_local! {
    /// Whether tests see an MP4 encoder.
    pub static FAKE_MP4: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether MP4 can be recorded: on Linux only with ffmpeg installed
/// (else GIF only, the MP4 choice disabled with [`NO_MP4`]); Windows checks
/// Media Foundation when the recording starts.
pub fn mp4_available() -> bool {
    #[cfg(test)]
    {
        FAKE_MP4.with(|c| c.get())
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        crate::rec::linux::ffmpeg::find().is_some()
    }
    #[cfg(all(not(target_os = "linux"), not(test)))]
    {
        true
    }
}

/// The recording border's colour (`border 2 px red`).
pub const RED: C4 = C4::rgb(0xF0, 0x3E, 0x3E);

/// The Record popover's choices, seeded from the config.
#[derive(Clone, Debug, PartialEq)]
pub struct RecOpts {
    pub format: RecFormat,
    /// MP4 frame rate: 30 or 60.
    pub fps_mp4: u32,
    /// GIF frame rate: 10 or 15.
    pub fps_gif: u32,
    pub system: bool,
    pub mic: bool,
    /// Microphone endpoint id; "" = the default device.
    pub mic_dev: String,
    /// Camera bubble.
    pub camera: bool,
    /// Camera id; "" = the first camera.
    pub cam_dev: String,
    pub cam_shape: Shape,
    pub cam_size: CamSize,
    pub cam_mirror: bool,
}

impl RecOpts {
    pub fn from_config(cfg: &Config) -> RecOpts {
        RecOpts {
            format: RecFormat::from_config(&cfg.rec_format),
            fps_mp4: if cfg.rec_fps >= 60 { 60 } else { 30 },
            fps_gif: if cfg.rec_gif_fps <= 10 { 10 } else { 15 },
            system: cfg.rec_system_audio,
            mic: cfg.rec_mic,
            mic_dev: cfg.rec_mic_device.clone(),
            camera: cfg.rec_camera,
            cam_dev: cfg.rec_camera_device.clone(),
            cam_shape: Shape::from_config(&cfg.rec_camera_shape),
            cam_size: CamSize::from_config(&cfg.rec_camera_size),
            cam_mirror: cfg.rec_camera_mirror,
        }
    }

    /// How the camera bubble looks.
    pub fn cam_look(&self) -> Look {
        Look { shape: self.cam_shape, size: self.cam_size, mirror: self.cam_mirror }
    }

    /// The frame rate of the chosen format.
    pub fn fps(&self) -> u32 {
        match self.format {
            RecFormat::Mp4 => self.fps_mp4,
            RecFormat::Gif => self.fps_gif,
        }
    }

    /// The two frame rates the popover offers for the chosen format.
    pub fn fps_choices(&self) -> [u32; 2] {
        match self.format {
            RecFormat::Mp4 => [30, 60],
            RecFormat::Gif => [10, 15],
        }
    }

    pub fn set_fps(&mut self, n: u32) {
        if self.fps_choices().contains(&n) {
            match self.format {
                RecFormat::Mp4 => self.fps_mp4 = n,
                RecFormat::Gif => self.fps_gif = n,
            }
        }
    }

    /// The pipeline's spec: `area` saved where a capture would be saved
    /// now, with these choices (a GIF has no sound).
    pub fn spec(&self, cfg: &Config, area: IRect, now: crate::export::Tm) -> RecSpec {
        let mut c = cfg.clone();
        c.rec_format = self.format.ext().into();
        let mut spec = RecSpec::from_config(&c, area, now);
        let mp4 = self.format == RecFormat::Mp4;
        spec.fps = self.fps();
        spec.system_audio = mp4 && self.system;
        spec.mic = (mp4 && self.mic).then(|| self.mic_dev.clone());
        spec
    }
}

/// The microphones on offer as (endpoint id, name).
pub fn mics() -> Vec<(String, String)> {
    #[cfg(test)]
    {
        vec![("{mic-1}".into(), "USB microphone".into()), ("{mic-2}".into(), "Headset".into())]
    }
    #[cfg(all(windows, not(test)))]
    {
        crate::rec::win::wasapi::list_mics()
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        crate::rec::linux::pulse_list::sources().into_iter().map(|s| (s.name, s.description)).collect()
    }
    #[cfg(all(not(windows), not(target_os = "linux"), not(test)))]
    {
        Vec::new()
    }
}

/// The cameras tests see by default.
#[cfg(test)]
pub fn fake_cams() -> Vec<CamDevice> {
    vec![
        CamDevice { id: "{cam-1}".into(), name: "Integrated Camera".into() },
        CamDevice { id: "{cam-2}".into(), name: "USB Webcam".into() },
    ]
}

#[cfg(test)]
thread_local! {
    /// The cameras tests see (empty for a camera-less machine).
    pub static FAKE_CAMS: std::cell::RefCell<Vec<CamDevice>> = std::cell::RefCell::new(fake_cams());
}

/// The cameras on offer (none on a machine without one, or without Media
/// Foundation).
pub fn cameras() -> Vec<CamDevice> {
    #[cfg(test)]
    {
        FAKE_CAMS.with(|c| c.borrow().clone())
    }
    #[cfg(not(test))]
    {
        crate::cam::list()
    }
}

/// [`cameras`] listed on a worker thread (it can take a moment: Media
/// Foundation starts and asks every device); the list arrives on the
/// receiver. Tests list their fake cameras at once.
pub fn cameras_async() -> std::sync::mpsc::Receiver<Vec<CamDevice>> {
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(test)]
    {
        let _ = tx.send(cameras());
    }
    #[cfg(not(test))]
    {
        let spawned = std::thread::Builder::new().name("camera-list".into()).spawn(move || {
            let _ = tx.send(cameras());
        });
        if let Err(e) = spawned {
            eprintln!("rustshot: cannot list cameras: {e}");
        }
    }
    rx
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// The 3-2-1 countdown before the first frame.
pub const COUNTDOWN: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Countdown,
    Recording,
    Paused,
    /// Stop (or cancel) sent; waiting for the file.
    Stopping,
}

/// What the caller must tell the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Start capturing now.
    Go,
    Pause,
    Resume,
    /// Stop and finish the file.
    Stop,
    /// Stopped during the countdown: nothing was recorded.
    Cancel,
}

/// Recording phases and the timer the control bar shows (paused time
/// excluded). Pure: the caller passes the time and carries out effects.
#[derive(Clone, Debug)]
pub struct Machine {
    phase: Phase,
    /// When the countdown ends.
    go_at: Option<Instant>,
    /// When capturing started.
    rec_at: Option<Instant>,
    paused_at: Option<Instant>,
    paused_total: Duration,
}

impl Default for Machine {
    fn default() -> Self {
        Machine::new()
    }
}

impl Machine {
    pub fn new() -> Machine {
        Machine { phase: Phase::Idle, go_at: None, rec_at: None, paused_at: None, paused_total: Duration::ZERO }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Idle → countdown (`countdown`), or straight to recording ([`Effect::Go`]).
    pub fn start(&mut self, countdown: bool, now: Instant) -> Option<Effect> {
        if self.phase != Phase::Idle {
            return None;
        }
        *self = Machine::new();
        if countdown {
            self.phase = Phase::Countdown;
            self.go_at = Some(now + COUNTDOWN);
            None
        } else {
            self.go(now)
        }
    }

    fn go(&mut self, now: Instant) -> Option<Effect> {
        self.phase = Phase::Recording;
        self.rec_at = Some(now);
        Some(Effect::Go)
    }

    /// The countdown ends on time.
    pub fn tick(&mut self, now: Instant) -> Option<Effect> {
        match (self.phase, self.go_at) {
            (Phase::Countdown, Some(t)) if now >= t => self.go(now),
            _ => None,
        }
    }

    /// The number the countdown shows now (3, 2, 1), outside it `None`.
    pub fn countdown(&self, now: Instant) -> Option<u32> {
        let t = self.go_at.filter(|_| self.phase == Phase::Countdown)?;
        let left = t.saturating_duration_since(now);
        Some((left.as_millis().div_ceil(1000) as u32).clamp(1, COUNTDOWN.as_secs() as u32))
    }

    /// Pause/Resume: recording ⇄ paused.
    pub fn toggle_pause(&mut self, now: Instant) -> Option<Effect> {
        match self.phase {
            Phase::Recording => {
                self.phase = Phase::Paused;
                self.paused_at = Some(now);
                Some(Effect::Pause)
            }
            Phase::Paused => {
                self.phase = Phase::Recording;
                if let Some(p) = self.paused_at.take() {
                    self.paused_total += now.saturating_duration_since(p);
                }
                Some(Effect::Resume)
            }
            _ => None,
        }
    }

    /// Stop: the bar, the record hotkey, the tray. During the countdown it
    /// cancels; twice is once.
    pub fn stop(&mut self, now: Instant) -> Option<Effect> {
        match self.phase {
            Phase::Countdown => {
                self.phase = Phase::Stopping;
                Some(Effect::Cancel)
            }
            Phase::Recording | Phase::Paused => {
                if let Some(p) = self.paused_at.take() {
                    self.paused_total += now.saturating_duration_since(p);
                }
                self.paused_at = Some(now); // the timer freezes
                self.phase = Phase::Stopping;
                Some(Effect::Stop)
            }
            Phase::Idle | Phase::Stopping => None,
        }
    }

    /// The pipeline is done (file saved, cancelled, or it failed): idle.
    pub fn finish(&mut self) {
        self.phase = Phase::Idle;
    }

    /// Recorded time: since capturing started, paused time excluded.
    pub fn elapsed(&self, now: Instant) -> Duration {
        let Some(t0) = self.rec_at else { return Duration::ZERO };
        let end = self.paused_at.unwrap_or(now);
        end.saturating_duration_since(t0).saturating_sub(self.paused_total)
    }
}

/// "0:07", "12:34", "1:02:03".
pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

pub fn contains(m: IRect, x: i32, y: i32) -> bool {
    x >= m.0 && y >= m.1 && x < m.0 + m.2 as i32 && y < m.1 + m.3 as i32
}

fn x1(r: IRect) -> i32 {
    r.0 + r.2 as i32
}

fn y1(r: IRect) -> i32 {
    r.1 + r.3 as i32
}

/// Top-left of a `w`x`h` control bar for a recording of `area` (all
/// physical px): just outside the area, `gap` from it, on the side with
/// the most room left beside it on the monitor there (ties: below, above,
/// right, left),
/// kept `margin` inside that monitor. When it fits on no side (a full
/// screen recording) it goes inside the area at its bottom centre (it is
/// excluded from the capture).
pub fn place_bar(area: IRect, monitors: &[IRect], (w, h): (i32, i32), gap: i32, margin: i32) -> (i32, i32) {
    let (cx, cy) = (area.0 + area.2 as i32 / 2, area.1 + area.3 as i32 / 2);
    let clamp = |v: i32, lo: i32, hi: i32| v.clamp(lo, hi.max(lo));
    let mon = |x: i32, y: i32| monitors.iter().copied().find(|m| contains(*m, x, y));
    let mut best: Option<(i32, (i32, i32))> = None;
    // Below, above, right, left: probe a point just outside each edge.
    let sides: [(i32, i32); 4] = [(cx, y1(area) + gap), (cx, area.1 - gap - 1), (x1(area) + gap, cy), (area.0 - gap - 1, cy)];
    for (i, (px, py)) in sides.into_iter().enumerate() {
        let Some(m) = mon(px, py) else { continue };
        // Room = what is left on that side once the bar is there.
        let (room, pos) = match i {
            0 => (y1(m) - y1(area) - h, (clamp(cx - w / 2, m.0 + margin, x1(m) - margin - w), y1(area) + gap)),
            1 => (area.1 - m.1 - h, (clamp(cx - w / 2, m.0 + margin, x1(m) - margin - w), area.1 - gap - h)),
            2 => (x1(m) - x1(area) - w, (x1(area) + gap, clamp(y1(area) - h, m.1 + margin, y1(m) - margin - h))),
            _ => (area.0 - m.0 - w, (area.0 - gap - w, clamp(y1(area) - h, m.1 + margin, y1(m) - margin - h))),
        };
        let fits = pos.0 >= m.0 + margin && pos.1 >= m.1 + margin && pos.0 + w <= x1(m) - margin && pos.1 + h <= y1(m) - margin;
        if fits && best.is_none_or(|(r, _)| room > r) {
            best = Some((room, pos));
        }
    }
    if let Some((_, p)) = best {
        return p;
    }
    // Inside, bottom centre (clamped to the area's monitor).
    let m = mon(cx, cy).unwrap_or(area);
    let x = clamp(cx - w / 2, m.0 + margin, x1(m) - margin - w);
    let y = clamp(y1(area) - gap - h, m.1 + margin, y1(m) - margin - h);
    (x, y)
}

/// Where the control bar is: the UI scale it is drawn at, its panel's
/// size at that scale and the panel's top-left (physical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarSpot {
    pub scale: f32,
    pub size: (i32, i32),
    pub pos: (i32, i32),
}

/// Place the control bar, whose panel is `size(s)` at UI scale `s`: at
/// the scale of the area's monitor first and, when it lands on a monitor
/// with another scale (mixed DPI), again at that one's. `prev` is kept
/// while the panel's size at its scale is unchanged; when it changes (the
/// timer passed an hour) the bar is placed again.
pub fn spot_bar(geom: &Geom, prev: Option<BarSpot>, size: impl Fn(f32) -> (i32, i32)) -> BarSpot {
    if let Some(p) = prev
        && size(p.scale) == p.size
    {
        return p;
    }
    let rects = geom.rects();
    let at = |s: f32| {
        let sz = size(s);
        let gap = (8.0 * s).round() as i32;
        BarSpot { scale: s, size: sz, pos: place_bar(geom.area, &rects, sz, gap, gap) }
    };
    let first = at(geom.area_scale());
    let s = geom.scale_at(first.pos.0 + first.size.0 / 2, first.pos.1 + first.size.1 / 2);
    if s == first.scale { first } else { at(s) }
}

/// Where the countdown goes: centred on the area.
pub fn centre_on(area: IRect, (w, h): (i32, i32)) -> (i32, i32) {
    (area.0 + (area.2 as i32 - w) / 2, area.1 + (area.3 as i32 - h) / 2)
}

// ---------------------------------------------------------------------------
// Layered-window pixels
// ---------------------------------------------------------------------------

/// Draw with `draw` into a `w`x`h` transparent layer: premultiplied BGRA
/// (what `UpdateLayeredWindow` takes). The rasterizer blends onto an
/// opaque target, so the layer is drawn over black and over white: the
/// difference is the coverage, the black one the premultiplied colour.
pub fn render_layer(w: usize, h: usize, draw: impl Fn(&mut Fb)) -> Vec<u8> {
    let mut black = [0u8, 0, 0, 255].repeat(w * h);
    let mut white = [255u8, 255, 255, 255].repeat(w * h);
    draw(&mut Fb::new(&mut black, w));
    draw(&mut Fb::new(&mut white, w));
    let mut out = vec![0u8; w * h * 4];
    for ((o, b), wh) in out.as_chunks_mut::<4>().0.iter_mut().zip(black.as_chunks::<4>().0).zip(white.as_chunks::<4>().0) {
        let a = 255 - (0..3).map(|i| wh[i].saturating_sub(b[i])).max().unwrap_or(0);
        // Premultiplied colour never exceeds alpha.
        *o = [b[2].min(a), b[1].min(a), b[0].min(a), a];
    }
    out
}

/// A premultiplied BGRA layer over an opaque `bg`, as RGBA (previews, tests).
#[cfg(test)]
pub fn layer_over(bgra: &[u8], bg: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bgra.len());
    for p in bgra.as_chunks::<4>().0 {
        let a = p[3] as u32;
        let ch = |c: u8, b: u8| (c as u32 + b as u32 * (255 - a) / 255).min(255) as u8;
        out.extend_from_slice(&[ch(p[2], bg[0]), ch(p[1], bg[1]), ch(p[0], bg[2]), 255]);
    }
    out
}

// ---------------------------------------------------------------------------
// Audio mute
// ---------------------------------------------------------------------------

/// An audio source the control bar can mute: silence (same timing) while
/// `muted` is set.
pub struct Muted<A> {
    pub inner: A,
    pub muted: Arc<AtomicBool>,
}

impl<A: AudioSource> AudioSource for Muted<A> {
    fn rate(&self) -> u32 {
        self.inner.rate()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn read(&mut self, out: &mut Vec<f32>) -> usize {
        self.read_timed(out).0
    }

    fn read_timed(&mut self, out: &mut Vec<f32>) -> (usize, Option<Instant>) {
        let start = out.len();
        let r = self.inner.read_timed(out);
        if self.muted.load(Ordering::Relaxed) {
            out[start..].fill(0.0);
        }
        r
    }
}

// ---------------------------------------------------------------------------
// A running recording
// ---------------------------------------------------------------------------

/// How a recording ended.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// The file, and what was left out of it ("microphone not recorded:
    /// <reason>").
    Saved { path: PathBuf, warnings: Vec<String> },
    /// It could not start, or the file could not be finished.
    Failed(String),
    /// Stopped during the countdown.
    Cancelled,
}

/// `end` with "camera stopped: <reason>" among the warnings of a saved
/// file when the camera bubble failed during the recording (it hid then).
#[cfg_attr(not(windows), allow(dead_code))] // no camera bubble elsewhere yet
pub fn with_camera_error(end: End, cam: Option<String>) -> End {
    match (end, cam) {
        (End::Saved { path, mut warnings }, Some(e)) => {
            warnings.push(format!("camera stopped: {e}"));
            End::Saved { path, warnings }
        }
        (end, _) => end,
    }
}

/// Where the recording is on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Geom {
    /// Recorded area, physical virtual-screen px.
    pub area: IRect,
    /// Every monitor (same coordinates) and its UI scale (physical px per
    /// logical px).
    pub monitors: Vec<(IRect, f32)>,
    /// UI scale off every monitor.
    pub scale: f32,
}

impl Geom {
    /// The monitors' rects.
    pub fn rects(&self) -> Vec<IRect> {
        self.monitors.iter().map(|m| m.0).collect()
    }

    /// The UI scale of the monitor at (x, y).
    pub fn scale_at(&self, x: i32, y: i32) -> f32 {
        self.monitors.iter().find(|m| contains(m.0, x, y)).map_or(self.scale, |m| m.1)
    }

    /// The UI scale where the area is (its centre).
    pub fn area_scale(&self) -> f32 {
        self.scale_at(self.area.0 + self.area.2 as i32 / 2, self.area.1 + self.area.3 as i32 / 2)
    }

    /// Each monitor with the border's thickness there.
    pub fn border_monitors(&self) -> Vec<(IRect, i32)> {
        self.monitors.iter().map(|&(m, s)| (m, border::thickness(s))).collect()
    }
}

/// A recording from the Start button to the saved file: the state
/// machine, the pipeline worker and the windows around the area.
pub struct Run {
    m: Machine,
    muted: Arc<AtomicBool>,
    has_mic: bool,
    #[cfg(windows)]
    ui: Option<win::Recorder>,
    #[cfg(target_os = "linux")]
    ui: Option<x11::Recorder>,
    /// Effects carried out (tests run without windows or a worker).
    #[cfg(test)]
    pub(crate) log: Vec<Effect>,
    /// An end reported without a worker (tests).
    #[cfg(test)]
    pub(crate) fake_end: Option<End>,
}

impl Run {
    /// Start recording `spec` (the pipeline opens in the background; with
    /// `countdown` it captures after 3-2-1). `nudge` gets a `WM_TIMER`
    /// when the control bar is clicked, so the owner ticks at once.
    #[cfg(windows)]
    #[cfg_attr(test, allow(dead_code))] // tests record through `Run::fake`
    /// `bubble`, the camera bubble, stays up until the recording is over.
    pub fn start(
        spec: RecSpec,
        geom: Geom,
        th: crate::theme::Theme,
        countdown: bool,
        nudge: crate::wind::Hwnd,
        bubble: Option<crate::cam::Bubble>,
    ) -> anyhow::Result<Run> {
        let muted = Arc::new(AtomicBool::new(false));
        let has_mic = spec.mic.is_some();
        let ui = win::Recorder::start(spec, geom, th, muted.clone(), nudge, bubble)?;
        let mut run = Run {
            m: Machine::new(),
            muted,
            has_mic,
            ui: Some(ui),
            #[cfg(test)]
            log: Vec::new(),
            #[cfg(test)]
            fake_end: None,
        };
        let now = Instant::now();
        if let Some(e) = run.m.start(countdown, now) {
            run.apply(e);
        }
        run.show(now);
        Ok(run)
    }

    /// X11: as on Windows, but ffmpeg records the sound itself, so the
    /// microphone cannot be muted from the bar (it has no Mute button).
    #[cfg(target_os = "linux")]
    #[cfg_attr(test, allow(dead_code))] // tests record through `Run::fake`
    pub fn start(
        spec: RecSpec,
        geom: Geom,
        th: crate::theme::Theme,
        countdown: bool,
        _nudge: crate::wind::Hwnd,
        bubble: Option<crate::cam::Bubble>,
    ) -> anyhow::Result<Run> {
        let muted = Arc::new(AtomicBool::new(false));
        let ui = x11::Recorder::start(spec, geom, th, bubble)?;
        let mut run = Run {
            m: Machine::new(),
            muted,
            has_mic: false,
            ui: Some(ui),
            #[cfg(test)]
            log: Vec::new(),
            #[cfg(test)]
            fake_end: None,
        };
        let now = Instant::now();
        if let Some(e) = run.m.start(countdown, now) {
            run.apply(e);
        }
        run.show(now);
        Ok(run)
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    pub fn start(
        _spec: RecSpec,
        _geom: Geom,
        _th: crate::theme::Theme,
        _countdown: bool,
        _nudge: crate::wind::Hwnd,
        _bubble: Option<crate::cam::Bubble>,
    ) -> anyhow::Result<Run> {
        anyhow::bail!("recording is not supported on this platform yet")
    }

    /// A recording with no worker or windows (tests).
    #[cfg(test)]
    pub fn fake(countdown: bool, mic: bool, now: Instant) -> Run {
        let mut run = Run {
            m: Machine::new(),
            muted: Arc::new(AtomicBool::new(false)),
            has_mic: mic,
            #[cfg(any(windows, target_os = "linux"))]
            ui: None,
            log: Vec::new(),
            fake_end: None,
        };
        if let Some(e) = run.m.start(countdown, now) {
            run.apply(e);
        }
        run
    }

    #[cfg_attr(not(test), allow(dead_code))] // tests
    pub fn phase(&self) -> Phase {
        self.m.phase()
    }

    fn apply(&mut self, e: Effect) {
        #[cfg(not(any(windows, target_os = "linux", test)))]
        let _ = e;
        #[cfg(test)]
        self.log.push(e);
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(ui) = &mut self.ui {
            ui.send(e);
        }
    }

    pub fn stop(&mut self, now: Instant) {
        if let Some(e) = self.m.stop(now) {
            self.apply(e);
            self.show(now);
        }
    }

    pub fn toggle_pause(&mut self, now: Instant) {
        if let Some(e) = self.m.toggle_pause(now) {
            self.apply(e);
            self.show(now);
        }
    }

    pub fn toggle_mute(&mut self) {
        if self.has_mic {
            self.muted.fetch_xor(true, Ordering::SeqCst);
        }
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    /// The control bar's state now.
    pub fn view(&self, now: Instant) -> bar::BarView {
        bar::BarView {
            // Whole seconds: the bar is redrawn only when its text changes.
            elapsed: Duration::from_secs(self.m.elapsed(now).as_secs()),
            paused: self.m.phase() == Phase::Paused,
            mic: self.has_mic.then(|| self.muted()),
            stopping: self.m.phase() == Phase::Stopping,
        }
    }

    /// Advance: the countdown, clicks on the control bar, the pipeline's
    /// end (returned once, when the recording is over and its windows gone).
    pub fn tick(&mut self, now: Instant) -> Option<End> {
        if let Some(e) = self.m.tick(now) {
            self.apply(e);
        }
        #[cfg(any(windows, target_os = "linux"))]
        let clicks = self.ui.as_mut().map(|u| u.take_clicks()).unwrap_or_default();
        #[cfg(not(any(windows, target_os = "linux")))]
        let clicks: Vec<bar::BarAct> = Vec::new();
        for c in clicks {
            match c {
                bar::BarAct::Pause => self.toggle_pause(now),
                bar::BarAct::Mute => self.toggle_mute(),
                bar::BarAct::Stop => self.stop(now),
            }
        }
        let end = self.poll_end();
        if let Some(end) = end {
            self.m.finish();
            #[cfg(any(windows, target_os = "linux"))]
            if let Some(ui) = &mut self.ui {
                ui.close();
            }
            return Some(end);
        }
        self.show(now);
        None
    }

    fn poll_end(&mut self) -> Option<End> {
        #[cfg(test)]
        if let Some(e) = self.fake_end.take() {
            return Some(e);
        }
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(ui) = &mut self.ui {
            return ui.poll_end();
        }
        None
    }

    /// Bring the windows up to date (border, bar, countdown).
    fn show(&mut self, now: Instant) {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let view = self.view(now);
            let count = self.m.countdown(now);
            if let Some(ui) = &mut self.ui {
                ui.show(&view, count);
            }
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = now;
    }
}

#[cfg(test)]
mod tests;
