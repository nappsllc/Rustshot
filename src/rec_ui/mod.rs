//! Recording UI around the pipeline in `crate::rec`: the choices the
//! Record popover makes ([`RecOpts`]), the recording state machine
//! ([`Machine`]: idle → countdown → recording ⇄ paused → stopping → idle),
//! where the control bar goes ([`place_bar`]) and a running recording
//! ([`Run`]). The windows (red border, control bar, 3-2-1 countdown) are
//! per platform: Windows (`win.rs`), X11 (`x11.rs`) and macOS (`macos.rs`);
//! the worker thread that drives the pipeline is shared (`worker.rs`). On
//! Wayland it needs the ScreenCast portal; on macOS ScreenCaptureKit
//! (12.3+; the Record button is hidden before).
#![cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))] // no recording UI elsewhere

pub mod bar;
pub mod border;
pub mod countdown;
pub mod meter;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub(crate) use win::Dib;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod worker;
#[cfg(target_os = "linux")]
pub(crate) mod x11;

use crate::cam::{CamDevice, CamSize, Look, Shape};
use crate::capture::IRect;
use crate::config::Config;
use crate::rec::{AudioSource, RecFormat, RecSpec};
use crate::uifb::{Fb, C4};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Whether this platform records (shows the Record button): Windows;
/// Linux: in an X11 session always (XShm); on Wayland and in Flatpak when
/// the ScreenCast portal answers and libpipewire loads (XShm would see
/// only XWayland windows there). Never waits: while the portal is being
/// asked (in the background, from the first call or [`warm_mp4`]) it says
/// no. macOS: when ScreenCaptureKit loads (12.3+).
pub fn supported() -> bool {
    #[cfg(any(windows, test))]
    {
        cfg!(any(windows, target_os = "linux"))
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        use crate::rec::linux::{pipewire, portal};
        static PORTAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*PORTAL.get_or_init(portal::needs_portal_now) {
            return true;
        }
        // Not known yet (the probe runs, 2 s at most): no Record button yet.
        portal::screencast_status() == Some(true) && pipewire::available()
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        crate::rec::macos::available()
    }
    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos"), not(test)))]
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

/// Why MP4 is off (the MP4 segment's tooltip).
pub const NO_MP4: crate::i18n::Key = key!("rec.no_mp4");

/// The MP4 segment's tooltip while ffmpeg is being asked.
pub const CHECKING_MP4: crate::i18n::Key = key!("rec.checking_mp4");

/// Whether MP4 can be recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))] // Yes elsewhere
pub enum Mp4 {
    Yes,
    /// GIF only: the MP4 choice is disabled with [`NO_MP4`].
    No,
    /// Not known yet (Linux: ffmpeg is being asked, in the background).
    Checking,
}

#[cfg(test)]
thread_local! {
    /// What tests see.
    pub static FAKE_MP4: std::cell::Cell<Mp4> = const { std::cell::Cell::new(Mp4::Yes) };
}

/// Whether MP4 can be recorded, without waiting. On Linux it always can:
/// with ffmpeg when it encodes H.264 (`ffmpeg -encoders`, a VA-API-only
/// ffmpeg tried once; cached, each probe stopped after 3 s, asked on a
/// background thread: [`Mp4::Checking`] until it answers, [`warm_mp4`]
/// asks at start), with a downloaded OpenH264, or else after the first-use
/// dialog when the recording starts (see [`linux_encoder`]). Windows
/// checks Media Foundation when the recording starts.
pub fn mp4_status() -> Mp4 {
    #[cfg(test)]
    {
        FAKE_MP4.with(|c| c.get())
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        use crate::rec::linux::{ffmpeg, openh264};
        if openh264::installed().is_some() {
            return Mp4::Yes;
        }
        // Without a usable ffmpeg the first-use dialog offers OpenH264,
        // when there is a build for this machine.
        let ask = if crate::rec::openh264_pins::this_machine().is_some() { Mp4::Yes } else { Mp4::No };
        match ffmpeg::find().map(|exe| ffmpeg::caps_now(&exe)) {
            Some(None) => Mp4::Checking,
            Some(Some(c)) if c.h264.is_some() => Mp4::Yes,
            _ => ask,
        }
    }
    #[cfg(all(not(target_os = "linux"), not(test)))]
    {
        Mp4::Yes
    }
}

/// Linux: ask ffmpeg what it can do now, in the background (the daemon
/// starts this so the Record popover knows when it opens).
pub fn warm_mp4() {
    #[cfg(all(target_os = "linux", not(test)))]
    {
        if let Some(exe) = crate::rec::linux::ffmpeg::find() {
            crate::rec::linux::ffmpeg::warm(&exe);
        }
        // An OpenH264 download cut short (killed, crashed) is not reused.
        crate::rec::linux::openh264::sweep();
        // Wayland: ask whether the ScreenCast portal answers (in the
        // background), so it is known when the overlay opens.
        if crate::rec::linux::portal::needs_portal_now() {
            let _ = crate::rec::linux::portal::screencast_status();
        }
    }
}

/// A recording the user chose not to start after all (the first-use
/// encoder dialog's Cancel / Install ffmpeg): ends like a countdown cancel.
#[derive(Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // only Linux asks
pub struct NotStarted;

impl std::fmt::Display for NotStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the recording was not started")
    }
}

impl std::error::Error for NotStarted {}

/// Linux: the MP4 encoder for `spec` (by `rec_linux_encoder`: ffmpeg,
/// else the downloaded OpenH264, else the first-use dialog asks). The
/// dialog can turn the recording into a GIF, or call it off
/// ([`NotStarted`]). Blocks while the dialog is open; `ffmpeg::caps` is
/// known by now (the popover waits for it).
#[cfg(target_os = "linux")]
fn linux_encoder(spec: &mut RecSpec) -> anyhow::Result<crate::rec::linux::native::Pick> {
    use crate::rec::linux::codec_dialog::{self, Choice};
    use crate::rec::linux::native::{pick, Pick};
    use crate::rec::linux::openh264::{self, Installed};
    use crate::rec::linux::ffmpeg;
    if spec.format != RecFormat::Mp4 {
        return Ok(Pick::Gif);
    }
    let cfg = crate::config::load();
    let ff = ffmpeg::find().is_some_and(|e| ffmpeg::caps(&e).h264.is_some());
    let lib = openh264::check();
    if let Installed::Bad(why) = &lib {
        eprintln!("rustshot: {why}; it was deleted");
    }
    match pick(&cfg.rec_linux_encoder, ff, &lib) {
        Pick::Ask => {}
        p => return Ok(p),
    }
    match codec_dialog::ask() {
        Choice::DownloadOpenH264 => Ok(Pick::OpenH264),
        Choice::Gif => {
            *spec = spec.as_gif(cfg.rec_gif_fps);
            Ok(Pick::Gif)
        }
        Choice::InstallFfmpeg | Choice::Cancel => Err(anyhow::Error::new(NotStarted)),
    }
}

/// The recording border's color (`border 2 px red`).
pub const RED: C4 = C4::rgb(0xF0, 0x3E, 0x3E);
/// The boards' recording red (`--rec`): the Start and Stop buttons, the
/// timer's dot, a muted microphone or hidden camera.
pub const REC: C4 = C4::rgb(0xFF, 0x5A, 0x5A);
/// The paused dot and a clipping level meter (`#FFB454`).
pub const AMBER: C4 = C4::rgb(0xFF, 0xB4, 0x54);
/// Text on the red buttons (`#1A0A0A`).
pub const ON_REC: C4 = C4::rgb(0x1A, 0x0A, 0x0A);

/// The sound rows' value in GIF mode.
pub const NOT_IN_GIF: crate::i18n::Key = key!("rec.not_in_gif");

/// What the Record panel records: the monitor (F), a window picked like
/// window snapping (W), or the selected region (R).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecSource {
    Screen,
    Window,
    Region,
}

/// The key that picks `s` in the Record panel (built in, like the mode
/// keys 1 and 2).
pub fn source_key(s: RecSource) -> &'static str {
    match s {
        RecSource::Screen => "F",
        RecSource::Window => "W",
        RecSource::Region => "R",
    }
}

/// The GIF frame rates on offer, and its length caps (seconds).
pub const GIF_FPS: [u32; 3] = [10, 15, 24];
pub const GIF_MAX: [u32; 3] = [15, 30, 60];
/// Regions narrower than this (logical px) get the compact pill.
pub const COMPACT_W: f32 = 520.0;
/// The toast when the microphone is silent this long while recording.
pub const NO_MIC_INPUT: crate::i18n::Key = key!("rec.no_mic_input");
pub const SILENCE: Duration = Duration::from_secs(5);
/// Peaks below this count as silence (about -50 dBFS).
const SILENT_PEAK: f32 = 0.003;

/// The Record panel's choices, seeded from the config.
#[derive(Clone, Debug, PartialEq)]
pub struct RecOpts {
    pub format: RecFormat,
    /// MP4 frame rate: 30 or 60.
    pub fps_mp4: u32,
    /// GIF frame rate: 10, 15 or 24.
    pub fps_gif: u32,
    /// A GIF stops by itself after this many seconds (15, 30 or 60).
    pub gif_max: u32,
    /// 3-2-1 before the first frame (the panel's Countdown 0 / 3s).
    pub countdown: bool,
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
    /// The bubble's ring color; `None` = the theme's accent.
    pub cam_ring: Option<crate::uifb::C4>,
    /// The ring's width in logical px (0 = no ring).
    pub cam_ring_w: u32,
    /// The background behind the person (`rec_camera_bg`).
    pub cam_bg: crate::cam::bg::Bg,
}

impl RecOpts {
    pub fn from_config(cfg: &Config) -> RecOpts {
        RecOpts {
            format: RecFormat::from_config(&cfg.rec_format),
            fps_mp4: if cfg.rec_fps >= 60 { 60 } else { 30 },
            fps_gif: GIF_FPS.into_iter().find(|f| *f >= cfg.rec_gif_fps).unwrap_or(24),
            gif_max: GIF_MAX.into_iter().find(|m| *m >= cfg.rec_gif_max).unwrap_or(60),
            countdown: cfg.rec_countdown,
            system: cfg.rec_system_audio,
            mic: cfg.rec_mic,
            mic_dev: cfg.rec_mic_device.clone(),
            camera: cfg.rec_camera,
            cam_dev: cfg.rec_camera_device.clone(),
            cam_shape: Shape::from_config(&cfg.rec_camera_shape),
            cam_size: CamSize::from_config(&cfg.rec_camera_size),
            cam_mirror: cfg.rec_camera_mirror,
            cam_ring: crate::config::parse_color(&cfg.rec_camera_ring_color).map(|(r, g, b, _)| crate::uifb::C4::rgb(r, g, b)),
            cam_ring_w: cfg.rec_camera_ring_width.min(crate::config::RING_WIDTH_MAX),
            cam_bg: crate::cam::bg::Bg::from_config(&cfg.rec_camera_bg),
        }
    }

    /// How the camera bubble looks.
    pub fn cam_look(&self) -> Look {
        Look {
            shape: self.cam_shape,
            size: self.cam_size,
            mirror: self.cam_mirror,
            ring: self.cam_ring,
            ring_w: self.cam_ring_w.min(crate::config::RING_WIDTH_MAX) as u8,
            bg: self.cam_bg,
        }
    }

    /// The frame rate of the chosen format.
    pub fn fps(&self) -> u32 {
        match self.format {
            RecFormat::Mp4 => self.fps_mp4,
            RecFormat::Gif => self.fps_gif,
        }
    }

    /// The frame rates offered for the chosen format.
    pub fn fps_choices(&self) -> &'static [u32] {
        match self.format {
            RecFormat::Mp4 => &[30, 60],
            RecFormat::Gif => &GIF_FPS,
        }
    }

    /// How long the recording may run before it stops by itself: a GIF's
    /// max length; an MP4 runs until stopped.
    pub fn max_len(&self) -> Option<Duration> {
        (self.format == RecFormat::Gif).then(|| Duration::from_secs(u64::from(self.gif_max)))
    }

    /// The estimate toast while the panel is open, for a `w`x`h` (physical
    /// px) recording: "1080p · 30 fps · est. 2.1 MB / min". MP4 from its
    /// bit rate (and 128 kbit/s of sound when there is any); GIF from a
    /// rough 0.05 bytes per pixel and frame (screen content, frames that
    /// repeat are only a delay).
    pub fn estimate(&self, (w, h): (u32, u32), q: crate::rec::Quality) -> String {
        let fps = self.fps();
        let (eh, bytes_s) = match self.format {
            RecFormat::Mp4 => {
                let (ew, eh) = crate::rec::mp4_fit(w, h);
                let sound = if self.system || self.mic { 128_000.0 } else { 0.0 };
                (eh, (f64::from(crate::rec::bitrate(ew, eh, fps, q)) + sound) / 8.0)
            }
            RecFormat::Gif => (h, f64::from(w) * f64::from(h) * f64::from(fps) * 0.05),
        };
        let mb = bytes_s * 60.0 / 1_000_000.0;
        let mb = if mb >= 10.0 { format!("{mb:.0}") } else { format!("{mb:.1}") };
        tr!("rec.estimate", height = eh, fps, mb)
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
    #[cfg(all(target_os = "macos", not(test)))]
    {
        crate::rec::macos::mic::list()
    }
    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos"), not(test)))]
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
pub fn cameras_async() -> crate::chan::Receiver<Vec<CamDevice>> {
    let (tx, rx) = crate::chan::channel();
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
/// How long before the first frame the X11 countdown is taken down: more
/// than a compositor frame (about 16 ms at 60 Hz), so it is never recorded.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const HIDE_BEFORE_GO: Duration = Duration::from_millis(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// Waiting for the screen to be chosen (Wayland: the portal's dialog is
    /// open on the recording worker); the countdown comes after.
    Asking,
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

    /// Idle → asking: the screen is being chosen ([`Machine::asked`] goes on).
    #[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
    pub fn ask(&mut self) {
        if self.phase == Phase::Idle {
            *self = Machine::new();
            self.phase = Phase::Asking;
        }
    }

    /// Asking → the countdown or recording, as [`Machine::start`].
    #[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
    pub fn asked(&mut self, countdown: bool, now: Instant) -> Option<Effect> {
        if self.phase != Phase::Asking {
            return None;
        }
        self.phase = Phase::Idle;
        self.start(countdown, now)
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

    /// The countdown's number as shown on X11: taken down [`HIDE_BEFORE_GO`]
    /// before the first frame, so a compositor has repainted without it by
    /// then (it would be recorded otherwise).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // X11 only; tested everywhere
    pub fn countdown_shown(&self, now: Instant) -> Option<u32> {
        let t = self.go_at.filter(|_| self.phase == Phase::Countdown)?;
        if t.saturating_duration_since(now) <= HIDE_BEFORE_GO {
            return None;
        }
        self.countdown(now)
    }

    /// The countdown's number and how much of its second is left (1 → 0:
    /// the 4 px arc drains over each second), outside it `None`.
    pub fn countdown_frac(&self, now: Instant) -> Option<(u32, f32)> {
        let n = self.countdown(now)?;
        let left = self.go_at?.saturating_duration_since(now).as_secs_f32();
        let frac = (left - (n - 1) as f32).clamp(0.0, 1.0);
        Some((n, frac))
    }

    /// The length cap reached (a GIF's max length; paused time does not
    /// count): stop, once.
    pub fn auto_stop(&mut self, max: Option<Duration>, now: Instant) -> Option<Effect> {
        let max = max?;
        if self.phase == Phase::Recording && self.elapsed(now) >= max { self.stop(now) } else { None }
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

    /// Stop: the bar, the record hotkey, the tray. While asking or during
    /// the countdown it cancels; twice is once.
    pub fn stop(&mut self, now: Instant) -> Option<Effect> {
        match self.phase {
            Phase::Asking | Phase::Countdown => {
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
#[cfg_attr(not(windows), allow(dead_code))] // Windows; X11 uses `place_bar_away`
pub fn place_bar(area: IRect, monitors: &[IRect], (w, h): (i32, i32), gap: i32, margin: i32) -> (i32, i32) {
    if let Some(p) = place_bar_beside(area, monitors, (w, h), gap, margin) {
        return p;
    }
    let (cx, cy) = (area.0 + area.2 as i32 / 2, area.1 + area.3 as i32 / 2);
    let clamp = |v: i32, lo: i32, hi: i32| v.clamp(lo, hi.max(lo));
    let mon = |x: i32, y: i32| monitors.iter().copied().find(|m| contains(*m, x, y));
    // Inside, bottom centre (clamped to the area's monitor).
    let m = mon(cx, cy).unwrap_or(area);
    let x = clamp(cx - w / 2, m.0 + margin, x1(m) - margin - w);
    let y = clamp(y1(area) - gap - h, m.1 + margin, y1(m) - margin - h);
    (x, y)
}

/// Where the bar goes when it must stay out of the recorded area (X11
/// cannot keep a window out of the capture): beside the area as in
/// [`place_bar`], else at the bottom centre of another monitor that the
/// area does not touch; `None` when there is no such place (a full-screen
/// recording on the only monitor: see [`corner_pill`]).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // X11 only; tested everywhere
pub fn place_bar_away(area: IRect, monitors: &[IRect], (w, h): (i32, i32), gap: i32, margin: i32) -> Option<(i32, i32)> {
    if let Some(p) = place_bar_beside(area, monitors, (w, h), gap, margin) {
        return Some(p);
    }
    let apart = |m: &IRect| x1(*m) <= area.0 || m.0 >= x1(area) || y1(*m) <= area.1 || m.1 >= y1(area);
    monitors
        .iter()
        .filter(|m| apart(m) && m.2 as i32 >= w + 2 * margin && m.3 as i32 >= h + 2 * margin)
        .map(|m| (m.0 + (m.2 as i32 - w) / 2, y1(*m) - margin - h))
        .next()
}

/// The compact pill's top-left: inside the area's top-right corner,
/// `margin` in (it is recorded there, so it stays small and out of the way).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn corner_pill(area: IRect, (w, _h): (i32, i32), margin: i32) -> (i32, i32) {
    ((x1(area) - margin - w).max(area.0), area.1 + margin)
}

/// [`place_bar`]'s sides: `None` when the bar fits beside the area on none.
fn place_bar_beside(area: IRect, monitors: &[IRect], (w, h): (i32, i32), gap: i32, margin: i32) -> Option<(i32, i32)> {
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
    best.map(|(_, p)| p)
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
#[cfg_attr(not(windows), allow(dead_code))]
pub fn spot_bar(geom: &Geom, prev: Option<BarSpot>, size: impl Fn(f32) -> (i32, i32)) -> BarSpot {
    spot_bar_with(geom, prev, size, |area, rects, sz, gap| place_bar(area, rects, sz, gap, gap))
}

/// [`spot_bar`] with the placement given (`place(area, monitors, size,
/// gap)`; the gap is also the margin).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn spot_bar_with(
    geom: &Geom,
    prev: Option<BarSpot>,
    size: impl Fn(f32) -> (i32, i32),
    place: impl Fn(IRect, &[IRect], (i32, i32), i32) -> (i32, i32),
) -> BarSpot {
    if let Some(p) = prev
        && size(p.scale) == p.size
    {
        return p;
    }
    let rects = geom.rects();
    let at = |s: f32| {
        let sz = size(s);
        let gap = (8.0 * s).round() as i32;
        BarSpot { scale: s, size: sz, pos: place(geom.area, &rects, sz, gap) }
    };
    let first = at(geom.area_scale());
    let s = geom.scale_at(first.pos.0 + first.size.0 / 2, first.pos.1 + first.size.1 / 2);
    if s == first.scale { first } else { at(s) }
}

/// The hint when the desktop put the camera bubble outside the recording.
pub const BUBBLE_OUTSIDE: crate::i18n::Key = key!("rec.bubble_outside");
/// The hint when it put the control bar inside the recording.
pub const BAR_INSIDE: crate::i18n::Key = key!("rec.bar_inside");

/// Wayland, where the compositor places the recording's windows: a hint
/// when the camera bubble (its shape, as placed) is not within the
/// recorded `area`, or the control bar's panel overlaps it. `None` while a
/// place is not known.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub fn placement_hint(area: IRect, bar: Option<IRect>, bubble: Option<IRect>) -> Option<&'static str> {
    let within = |r: IRect| r.0 >= area.0 && r.1 >= area.1 && x1(r) <= x1(area) && y1(r) <= y1(area);
    let overlaps = |r: IRect| r.0 < x1(area) && area.0 < x1(r) && r.1 < y1(area) && area.1 < y1(r);
    if bubble.is_some_and(|b| !within(b)) {
        return Some(BUBBLE_OUTSIDE.get());
    }
    if bar.is_some_and(overlaps) {
        return Some(BAR_INSIDE.get());
    }
    None
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
/// difference is the coverage, the black one the premultiplied color.
pub fn render_layer(w: usize, h: usize, draw: impl Fn(&mut Fb)) -> Vec<u8> {
    let mut black = [0u8, 0, 0, 255].repeat(w * h);
    let mut white = [255u8, 255, 255, 255].repeat(w * h);
    draw(&mut Fb::new(&mut black, w));
    draw(&mut Fb::new(&mut white, w));
    let mut out = vec![0u8; w * h * 4];
    for ((o, b), wh) in out.as_chunks_mut::<4>().0.iter_mut().zip(black.as_chunks::<4>().0).zip(white.as_chunks::<4>().0) {
        let a = 255 - (0..3).map(|i| wh[i].saturating_sub(b[i])).max().unwrap_or(0);
        // Premultiplied color never exceeds alpha.
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

/// Whether the pill shows only the timer and Stop: the region is narrower
/// than [`COMPACT_W`] logical px (`w` physical px at UI scale `scale`), or
/// the user asked for it (the timer click / H).
pub fn compact_pill(w: u32, scale: f32, by_user: bool) -> bool {
    by_user || (w as f32 / scale.max(0.01)) < COMPACT_W
}

/// The sound as the control bar sees it: whether the microphone's source
/// opened (set by the worker once it did: only then is there a Mute
/// button), whether it is muted, its latest peak (silence detection), and
/// the same for the system sound.
#[derive(Clone, Debug, Default)]
pub struct MicFlags {
    pub muted: Arc<AtomicBool>,
    pub open: Arc<AtomicBool>,
    /// The microphone's latest peak (`f32` bits), muted or not.
    pub level: Arc<std::sync::atomic::AtomicU32>,
    pub sys_muted: Arc<AtomicBool>,
    pub sys_open: Arc<AtomicBool>,
}

impl MicFlags {
    /// `inner` (the microphone, which opened) to record, mutable from the bar.
    pub fn opened<A>(&self, inner: A) -> Muted<A> {
        self.open.store(true, Ordering::SeqCst);
        Muted { inner, muted: self.muted.clone(), level: Some(self.level.clone()) }
    }

    /// `inner` (the system sound, which opened) to record, mutable from the bar.
    pub fn system<A>(&self, inner: A) -> Muted<A> {
        self.sys_open.store(true, Ordering::SeqCst);
        Muted { inner, muted: self.sys_muted.clone(), level: None }
    }

    /// The microphone's latest peak, 0..=1.
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

/// An audio source the control bar can mute: silence (same timing) while
/// `muted` is set; its peak goes to `level` (before muting).
pub struct Muted<A> {
    pub inner: A,
    pub muted: Arc<AtomicBool>,
    pub level: Option<Arc<std::sync::atomic::AtomicU32>>,
}

/// The largest magnitude among `s` (0 for none).
pub fn peak_of(s: &[f32]) -> f32 {
    s.iter().fold(0.0f32, |m, v| m.max(v.abs())).min(1.0)
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
        if let Some(l) = &self.level
            && out.len() > start
        {
            l.store(peak_of(&out[start..]).to_bits(), Ordering::Relaxed);
        }
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
            warnings.push(tr!("rec.warn.camera_stopped", e));
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
    /// The UI size's multiplier, already in every scale above (the
    /// border follows the OS scale only).
    pub mult: f32,
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

    /// macOS: this geometry (pixels = points × `k`, see
    /// `capture::pts`) in whole points, every scale the UI size's
    /// multiplier (logical px per point): what the recording windows are
    /// placed in.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn in_points(&self, k: f64) -> Geom {
        let p = |r: IRect| crate::capture::pts::rect_pt_i(r, k);
        let m = crate::uisize::mult();
        Geom { area: p(self.area), monitors: self.monitors.iter().map(|x| (p(x.0), m)).collect(), scale: m, mult: m }
    }

    /// Each monitor with the border's thickness there: by the monitor's
    /// own scale, not the UI size (a line, as on macOS where it is in
    /// points).
    #[cfg_attr(target_os = "macos", allow(dead_code))] // macOS draws it in points
    pub fn border_monitors(&self) -> Vec<(IRect, i32)> {
        self.monitors
            .iter()
            .map(|&(m, s)| {
                // Back to the OS scale (rounded: the division's noise must not tip a .5).
                let os = (s / self.mult.max(f32::EPSILON) * 1000.0).round() / 1000.0;
                (m, border::thickness(os))
            })
            .collect()
    }
}

/// A recording from the Start button to the saved file: the state
/// machine, the pipeline worker and the windows around the area.
pub struct Run {
    m: Machine,
    /// Mute, and whether the microphone opened (no Mute button before).
    mic: MicFlags,
    #[cfg(windows)]
    ui: Option<win::Recorder>,
    #[cfg(target_os = "linux")]
    ui: Option<x11::Recorder>,
    #[cfg(target_os = "macos")]
    ui: Option<macos::Recorder>,
    /// Whether 3-2-1 comes before the first frame (once asking is over).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    countdown: bool,
    /// The placement hint was handed out (once per recording).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    hint_taken: bool,
    /// It stops by itself after this long (a GIF's max length).
    max: Option<Duration>,
    /// With a camera bubble: whether it is hidden (the pill's camera button).
    cam: Option<bool>,
    /// The region is narrower than [`COMPACT_W`]: the compact pill.
    narrow: bool,
    /// The user toggled the compact pill (the timer, H).
    compact_user: bool,
    /// The user hid the pill (the tray, the while-recording chord).
    pill_hidden: bool,
    /// Discard pressed: the file is deleted once it is finished.
    discard: bool,
    /// The microphone has been silent since (while recording).
    silent_since: Option<Instant>,
    /// "No mic input" was said (once per recording) / waits to be taken.
    mic_noted: bool,
    notice: Option<crate::i18n::Key>,
    /// Effects carried out (tests run without windows or a worker).
    #[cfg(test)]
    pub(crate) log: Vec<Effect>,
    /// An end reported without a worker (tests).
    #[cfg(test)]
    pub(crate) fake_end: Option<End>,
    /// The parts are open, without a worker (tests).
    #[cfg(test)]
    pub(crate) fake_ready: bool,
}

impl Run {
    /// A recording's state before its windows: `cam` when it has a bubble,
    /// `narrow` for a region below [`COMPACT_W`].
    fn base(mic: MicFlags, countdown: bool, cam: bool, narrow: bool) -> Run {
        Run {
            m: Machine::new(),
            mic,
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            ui: None,
            countdown,
            hint_taken: false,
            max: None,
            cam: cam.then_some(false),
            narrow,
            compact_user: false,
            pill_hidden: false,
            discard: false,
            silent_since: None,
            mic_noted: false,
            notice: None,
            #[cfg(test)]
            log: Vec::new(),
            #[cfg(test)]
            fake_end: None,
            #[cfg(test)]
            fake_ready: false,
        }
    }

    /// Whether `geom`'s region gets the compact pill.
    fn narrow_of(geom: &Geom) -> bool {
        compact_pill(geom.area.2, geom.area_scale(), false)
    }

    /// Stop by itself after `max` (a GIF's max length).
    pub fn with_max(mut self, max: Option<Duration>) -> Run {
        self.max = max;
        self
    }

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
        let mic = MicFlags::default();
        let mut run = Run::base(mic.clone(), countdown, bubble.is_some(), Run::narrow_of(&geom));
        run.ui = Some(win::Recorder::start(spec, geom, th, mic, nudge, bubble)?);
        let now = Instant::now();
        if let Some(e) = run.m.start(countdown, now) {
            run.apply(e);
        }
        run.show(now);
        Ok(run)
    }

    /// Linux: as on Windows (the sound is read in-app, so the microphone
    /// can be muted from the bar once it opened). On Wayland and in
    /// Flatpak the portal asks which screen first, on the recording worker
    /// (the UI, hotkeys and tray stay live): the recording is
    /// [`Phase::Asking`] meanwhile, the bar says so, and Stop (bar, hotkey,
    /// tray) calls the dialog off; closing the dialog calls the recording
    /// off. The countdown comes once the screen is chosen.
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
        use crate::rec::linux::portal;
        let mut spec = spec;
        let enc = linux_encoder(&mut spec)?;
        let asking = portal::needs_portal_now();
        let mic = MicFlags::default();
        let mut run = Run::base(mic.clone(), countdown, bubble.is_some(), Run::narrow_of(&geom));
        run.ui = Some(x11::Recorder::start(spec, geom, th, mic, enc, bubble, asking)?);
        let now = Instant::now();
        if asking {
            run.m.ask();
        } else if let Some(e) = run.m.start(countdown, now) {
            run.apply(e);
        }
        run.show(now);
        Ok(run)
    }

    /// macOS: as on Windows; the windows are made on the main thread (the
    /// overlay's), the stream opens on the worker (ScreenCaptureKit asks
    /// for Screen Recording permission the first time).
    #[cfg(target_os = "macos")]
    #[cfg_attr(test, allow(dead_code))] // tests record through `Run::fake`
    pub fn start(
        spec: RecSpec,
        geom: Geom,
        th: crate::theme::Theme,
        countdown: bool,
        _nudge: crate::wind::Hwnd,
        bubble: Option<crate::cam::Bubble>,
    ) -> anyhow::Result<Run> {
        let mic = MicFlags::default();
        let mut run = Run::base(mic.clone(), countdown, bubble.is_some(), Run::narrow_of(&geom));
        run.ui = Some(macos::Recorder::start(spec, geom, th, mic, bubble)?);
        let now = Instant::now();
        if let Some(e) = run.m.start(countdown, now) {
            run.apply(e);
        }
        run.show(now);
        Ok(run)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
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
        let flags = MicFlags { open: Arc::new(AtomicBool::new(mic)), ..MicFlags::default() };
        let mut run = Run::base(flags, countdown, false, false);
        if let Some(e) = run.m.start(countdown, now) {
            run.apply(e);
        }
        run
    }

    /// [`Run::fake`] with a camera bubble and a region `narrow` or not.
    #[cfg(test)]
    pub fn fake_with(countdown: bool, mic: bool, cam: bool, narrow: bool, now: Instant) -> Run {
        let mut run = Run::fake(countdown, mic, now);
        run.cam = cam.then_some(false);
        run.narrow = narrow;
        run
    }

    /// [`Run::fake`] waiting for the screen to be chosen (Wayland).
    #[cfg(test)]
    pub fn fake_asking(countdown: bool) -> Run {
        let mut run = Run::fake(false, false, Instant::now());
        run.m = Machine::new();
        run.log.clear();
        run.countdown = countdown;
        run.m.ask();
        run
    }

    /// Tests: the microphone flags (its level, as the worker sets it).
    #[cfg(test)]
    pub(crate) fn mic_flags(&self) -> &MicFlags {
        &self.mic
    }

    #[cfg_attr(not(test), allow(dead_code))] // tests
    pub fn phase(&self) -> Phase {
        self.m.phase()
    }

    fn apply(&mut self, e: Effect) {
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos", test)))]
        let _ = e;
        #[cfg(test)]
        self.log.push(e);
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
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

    /// Stop and throw the recording away (the pill's Discard).
    pub fn discard(&mut self, now: Instant) {
        if matches!(self.m.phase(), Phase::Recording | Phase::Paused) {
            self.discard = true;
        }
        self.stop(now);
    }

    pub fn toggle_pause(&mut self, now: Instant) {
        if let Some(e) = self.m.toggle_pause(now) {
            self.apply(e);
            self.show(now);
        }
    }

    pub fn toggle_mute(&mut self) {
        if self.has_mic() {
            self.mic.muted.fetch_xor(true, Ordering::SeqCst);
        }
    }

    /// The system sound off/on, when it is recorded.
    pub fn toggle_system(&mut self) {
        if self.mic.sys_open.load(Ordering::SeqCst) {
            self.mic.sys_muted.fetch_xor(true, Ordering::SeqCst);
        }
    }

    /// Hide the camera bubble or show it again, when there is one.
    pub fn toggle_camera(&mut self) {
        let Some(hidden) = self.cam.as_mut() else { return };
        *hidden = !*hidden;
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        let h = *hidden;
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if let Some(ui) = &mut self.ui {
            ui.set_cam_hidden(h);
        }
    }

    /// The compact pill and back (the timer, H). A narrow region stays
    /// compact.
    pub fn toggle_compact(&mut self) {
        self.compact_user = !self.compact_user;
    }

    /// Hide the control pill or show it again (the tray, the while-recording
    /// chord): the recording goes on; the tray and the hotkeys still stop it.
    pub fn toggle_pill(&mut self, now: Instant) {
        self.pill_hidden = !self.pill_hidden;
        self.show(now);
    }

    /// What the tray menu shows of this recording.
    pub fn menu_state(&self) -> crate::tray::RecMenu {
        let v = self.view(Instant::now());
        crate::tray::RecMenu { paused: v.paused, mic: v.mic, cam: v.cam, pill_hidden: self.pill_hidden }
    }

    /// The microphone is being recorded (its source opened).
    fn has_mic(&self) -> bool {
        self.mic.open.load(Ordering::SeqCst)
    }

    pub fn muted(&self) -> bool {
        self.mic.muted.load(Ordering::SeqCst)
    }

    /// The time recorded (frozen once stopped).
    pub fn length(&self, now: Instant) -> Duration {
        self.m.elapsed(now)
    }

    /// The countdown is up (it is redrawn about 60 times a second: its arc).
    pub fn counting(&self) -> bool {
        self.m.phase() == Phase::Countdown
    }

    /// A toast for the user (the microphone has been silent for 5 s:
    /// "No mic input"), once.
    pub fn take_notice(&mut self) -> Option<&'static str> {
        self.notice.take().map(crate::i18n::Key::get)
    }

    /// The control bar's state now.
    pub fn view(&self, now: Instant) -> bar::BarView {
        let sys = self.mic.sys_open.load(Ordering::SeqCst).then(|| self.mic.sys_muted.load(Ordering::SeqCst));
        bar::BarView {
            // Whole seconds: the bar is redrawn only when its text changes.
            elapsed: Duration::from_secs(self.m.elapsed(now).as_secs()),
            paused: self.m.phase() == Phase::Paused,
            mic: self.has_mic().then(|| self.muted()),
            cam: self.cam,
            system: sys,
            stopping: self.m.phase() == Phase::Stopping,
            compact: self.narrow || self.compact_user,
            asking: self.m.phase() == Phase::Asking,
            hidden: self.pill_hidden,
        }
    }

    /// Watch the microphone while recording: silent (unmuted) for
    /// [`SILENCE`] → "No mic input", once.
    fn watch_mic(&mut self, now: Instant) {
        let live = self.m.phase() == Phase::Recording && self.has_mic() && !self.muted();
        if !live || self.mic.peak() >= SILENT_PEAK {
            self.silent_since = None;
            return;
        }
        let since = *self.silent_since.get_or_insert(now);
        if !self.mic_noted && now.saturating_duration_since(since) >= SILENCE {
            self.mic_noted = true;
            self.notice = Some(NO_MIC_INPUT);
        }
    }

    /// Advance: the countdown, clicks on the control bar, the pipeline's
    /// end (returned once, when the recording is over and its windows gone).
    pub fn tick(&mut self, now: Instant) -> Option<End> {
        // The screen was chosen: on to the countdown.
        if self.m.phase() == Phase::Asking
            && self.ready()
            && let Some(e) = self.m.asked(self.countdown, now)
        {
            self.apply(e);
        }
        if let Some(e) = self.m.tick(now) {
            self.apply(e);
        }
        // A GIF's max length.
        if let Some(e) = self.m.auto_stop(self.max, now) {
            self.apply(e);
        }
        self.watch_mic(now);
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        let clicks = self.ui.as_mut().map(|u| u.take_clicks()).unwrap_or_default();
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        let clicks: Vec<bar::BarAct> = Vec::new();
        for c in clicks {
            self.act(c, now);
        }
        let end = self.poll_end();
        if let Some(end) = end {
            self.m.finish();
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            if let Some(ui) = &mut self.ui {
                ui.close();
            }
            return Some(self.discarded(end));
        }
        self.show(now);
        None
    }

    /// Carry out a click on the pill.
    pub fn act(&mut self, a: bar::BarAct, now: Instant) {
        match a {
            bar::BarAct::Pause => self.toggle_pause(now),
            bar::BarAct::Mute => self.toggle_mute(),
            bar::BarAct::Stop => self.stop(now),
            bar::BarAct::Camera => self.toggle_camera(),
            bar::BarAct::System => self.toggle_system(),
            bar::BarAct::Discard => self.discard(now),
            bar::BarAct::Compact => self.toggle_compact(),
        }
    }

    /// A discarded recording's file is deleted: it ends as cancelled.
    fn discarded(&self, end: End) -> End {
        match end {
            End::Saved { path, .. } if self.discard => {
                if let Err(e) = std::fs::remove_file(&path) {
                    eprintln!("rustshot: could not delete the discarded {}: {e}", path.display());
                }
                End::Cancelled
            }
            e => e,
        }
    }

    /// The recording's parts are open (the screen chosen).
    fn ready(&self) -> bool {
        #[cfg(test)]
        if self.fake_ready {
            return true;
        }
        #[cfg(target_os = "linux")]
        if let Some(ui) = &self.ui {
            return ui.ready();
        }
        false
    }

    /// A hint about where the desktop put the recording's windows (Wayland:
    /// the camera bubble outside the recorded area, or the bar inside it),
    /// once per recording.
    pub fn take_hint(&mut self) -> Option<&'static str> {
        if self.hint_taken {
            return None;
        }
        #[cfg(target_os = "linux")]
        let hint = self.ui.as_mut().and_then(|u| u.placement_hint());
        #[cfg(not(target_os = "linux"))]
        let hint: Option<&'static str> = None;
        self.hint_taken = hint.is_some();
        hint
    }

    fn poll_end(&mut self) -> Option<End> {
        #[cfg(test)]
        if let Some(e) = self.fake_end.take() {
            return Some(e);
        }
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if let Some(ui) = &mut self.ui {
            return ui.poll_end();
        }
        None
    }

    /// Bring the windows up to date (border, bar, countdown).
    fn show(&mut self, now: Instant) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        {
            let view = self.view(now);
            // Windows and macOS keep the countdown out of the capture.
            #[cfg(any(windows, target_os = "macos"))]
            let count = self.m.countdown_frac(now);
            #[cfg(target_os = "linux")]
            let count = self.m.countdown_shown(now).and(self.m.countdown_frac(now));
            if let Some(ui) = &mut self.ui {
                ui.show(&view, count);
            }
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        let _ = now;
    }
}

#[cfg(test)]
mod tests;
