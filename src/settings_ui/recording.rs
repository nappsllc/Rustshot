//! Settings → **Recording** and **Camera**: the defaults video mode's
//! panel starts from (Recording: format, frame rate, MP4 quality, a GIF's
//! max length, the 3-2-1 countdown, on Linux the MP4 encoder with a line
//! saying what is installed, then system sound and the microphone with its
//! device; Camera: the bubble's device, shape, size, mirror, background
//! (blurred or not; the model's download, or why it cannot be had) and ring). The record hotkey is in Shortcuts ›
//! Recording.
//!
//! [`RecForm`] is the part of the settings form this tab edits (every
//! `rec_*` key and `record_hotkey`); [`Lists`] holds the microphones and
//! cameras, listed on a worker thread when the tab is first shown (Media
//! Foundation and PulseAudio can take a moment), and the encoder status.

use super::draw::{srow, srow_ex};
use crate::config::{self, Config};
use crate::keymap::Chord;
use crate::ui::compact::Tok;
use crate::ui::Ui;
use std::path::PathBuf;
use crate::chan::Receiver;
use crate::i18n::{Key, Label};

/// (label, config value) tables.
const FORMATS: [(&str, &str); 2] = [("MP4", "mp4"), ("GIF", "gif")];
const MP4_FPS: [(Key, u32); 2] = [(key!("settings.recording.fps30"), 30), (key!("settings.recording.fps60"), 60)];
const GIF_FPS: [(Key, u32); 3] = [(key!("settings.recording.fps10"), 10), (key!("settings.recording.fps15"), 15), (key!("settings.recording.fps24"), 24)];
/// A GIF stops by itself at its max length (`rec_gif_max`, seconds).
const GIF_MAX: [(Key, u32); 3] = [(key!("settings.recording.s15"), 15), (key!("settings.recording.s30"), 30), (key!("settings.recording.s60"), 60)];
const QUALITIES: [(Key, &str); 3] = [(key!("settings.recording.low"), "low"), (key!("settings.recording.medium"), "medium"), (key!("settings.recording.high"), "high")];
const SHAPES: [(Key, &str); 3] = [(key!("settings.camera.circle"), "circle"), (key!("settings.camera.rounded"), "rounded"), (key!("settings.camera.square"), "square")];
/// The bubble's side (logical px), as video mode's panel and the wheel
/// size it: S / M / L / XL.
const SIZES: [(&str, &str); 4] = [("120", "s"), ("160", "m"), ("200", "l"), ("240", "xl")];
/// The camera's background (`rec_camera_bg`).
pub const BACKGROUNDS: [(Key, &str); 3] = [(key!("settings.camera.bg_off"), "off"), (key!("settings.camera.bg_light"), "light"), (key!("settings.camera.bg_strong"), "strong")];
pub const ENCODERS: [(&str, &str); 3] = [("Auto", "auto"), ("ffmpeg", "ffmpeg"), ("OpenH264", "openh264")];
/// The ring colors on offer after "Accent" (the default drawing palette).
const RINGS: [(Key, &str); 10] = [
    (key!("color.red"), "#f04438"),
    (key!("color.orange"), "#ff8a1f"),
    (key!("color.yellow"), "#ffc532"),
    (key!("color.green"), "#2dc06f"),
    (key!("color.cyan"), "#19b5d6"),
    (key!("color.blue"), "#3b82f6"),
    (key!("color.purple"), "#8b5cf6"),
    (key!("color.pink"), "#ec4899"),
    (key!("color.white"), "#ffffff"),
    (key!("color.black"), "#111318"),
];

const DEFAULT_MIC: Key = key!("settings.recording.default_mic");
const DEFAULT_CAM: Key = key!("settings.camera.default_camera");

fn index_of<L, T: PartialEq + Copy>(list: &[(L, T)], v: T) -> usize {
    list.iter().position(|(_, x)| *x == v).unwrap_or(0)
}

fn index_str<L>(list: &[(L, &str)], v: &str) -> usize {
    list.iter().position(|(_, x)| v.trim().eq_ignore_ascii_case(x)).unwrap_or(0)
}

/// The values this tab edits.
#[derive(Clone, Debug, PartialEq)]
pub struct RecForm {
    /// [`FORMATS`] index.
    pub format: usize,
    /// MP4 frame rate ([`MP4_FPS`] index) and GIF frame rate ([`GIF_FPS`]).
    pub fps: usize,
    pub gif_fps: usize,
    /// A GIF's max length ([`GIF_MAX`] index).
    pub gif_max: usize,
    /// [`QUALITIES`] index.
    pub quality: usize,
    pub system: bool,
    pub mic: bool,
    /// Microphone id; "" = the default device.
    pub mic_dev: String,
    pub camera: bool,
    /// Camera id; "" = the first camera.
    pub cam_dev: String,
    pub shape: usize,
    pub size: usize,
    pub mirror: bool,
    /// [`BACKGROUNDS`] index.
    pub bg: usize,
    /// `#rrggbb`, or "" for the theme's accent.
    pub ring: String,
    /// Logical px, 0 = no ring.
    pub ring_w: u8,
    pub countdown: bool,
    pub hotkey: Option<Chord>,
    /// [`ENCODERS`] index.
    pub encoder: usize,
    /// Show clicks / keystrokes / every key in the recording.
    pub show_clicks: bool,
    pub show_keys: bool,
    pub all_keys: bool,
    /// [`TRIMS`] index (Trim after recording).
    pub trim: usize,
}

/// Trim after recording (`rec_trim`).
pub const TRIMS: [(Key, &str); 2] = [(key!("settings.recording.trim_off"), "off"), (key!("settings.recording.trim_ask"), "ask")];

impl RecForm {
    pub fn new(cfg: &Config) -> RecForm {
        RecForm {
            format: index_str(&FORMATS, &cfg.rec_format),
            fps: index_of(&MP4_FPS, cfg.rec_fps),
            gif_fps: index_of(&GIF_FPS, cfg.rec_gif_fps),
            gif_max: GIF_MAX.iter().position(|(_, x)| *x == cfg.rec_gif_max).unwrap_or(1),
            quality: index_str(&QUALITIES, &cfg.rec_quality),
            system: cfg.rec_system_audio,
            mic: cfg.rec_mic,
            mic_dev: cfg.rec_mic_device.clone(),
            camera: cfg.rec_camera,
            cam_dev: cfg.rec_camera_device.clone(),
            shape: index_str(&SHAPES, &cfg.rec_camera_shape),
            size: index_str(&SIZES, &cfg.rec_camera_size),
            mirror: cfg.rec_camera_mirror,
            bg: index_str(&BACKGROUNDS, &cfg.rec_camera_bg),
            ring: cfg.rec_camera_ring_color.trim().to_ascii_lowercase(),
            ring_w: cfg.rec_camera_ring_width.min(config::RING_WIDTH_MAX) as u8,
            countdown: cfg.rec_countdown,
            hotkey: Chord::parse(&cfg.record_hotkey),
            encoder: index_str(&ENCODERS, &cfg.rec_linux_encoder),
            show_clicks: cfg.rec_show_clicks,
            show_keys: cfg.rec_show_keys,
            all_keys: cfg.rec_show_all_keys,
            trim: index_str(&TRIMS, &cfg.rec_trim),
        }
    }

    /// Write the values that differ from `was` (the form of the config
    /// the window loaded) into `c`; the others keep `c`'s spelling.
    pub fn apply(&self, was: &RecForm, c: &mut Config) {
        if self.format != was.format {
            c.rec_format = FORMATS[self.format].1.into();
        }
        if self.fps != was.fps {
            c.rec_fps = MP4_FPS[self.fps].1;
        }
        if self.gif_fps != was.gif_fps {
            c.rec_gif_fps = GIF_FPS[self.gif_fps].1;
        }
        if self.gif_max != was.gif_max {
            c.rec_gif_max = GIF_MAX[self.gif_max].1;
        }
        if self.quality != was.quality {
            c.rec_quality = QUALITIES[self.quality].1.into();
        }
        if self.system != was.system {
            c.rec_system_audio = self.system;
        }
        if self.mic != was.mic {
            c.rec_mic = self.mic;
        }
        if self.mic_dev != was.mic_dev {
            c.rec_mic_device = self.mic_dev.clone();
        }
        if self.camera != was.camera {
            c.rec_camera = self.camera;
        }
        if self.cam_dev != was.cam_dev {
            c.rec_camera_device = self.cam_dev.clone();
        }
        if self.shape != was.shape {
            c.rec_camera_shape = SHAPES[self.shape].1.into();
        }
        if self.size != was.size {
            c.rec_camera_size = SIZES[self.size].1.into();
        }
        if self.mirror != was.mirror {
            c.rec_camera_mirror = self.mirror;
        }
        if self.bg != was.bg {
            c.rec_camera_bg = BACKGROUNDS[self.bg].1.into();
        }
        if self.ring != was.ring {
            c.rec_camera_ring_color = self.ring.clone();
        }
        if self.ring_w != was.ring_w {
            c.rec_camera_ring_width = self.ring_w as u32;
        }
        if self.countdown != was.countdown {
            c.rec_countdown = self.countdown;
        }
        if self.hotkey != was.hotkey {
            c.record_hotkey = self.hotkey.map(|k| k.display()).unwrap_or_default();
        }
        if self.encoder != was.encoder {
            c.rec_linux_encoder = ENCODERS[self.encoder].1.into();
        }
        if self.show_clicks != was.show_clicks {
            c.rec_show_clicks = self.show_clicks;
        }
        if self.show_keys != was.show_keys {
            c.rec_show_keys = self.show_keys;
        }
        if self.all_keys != was.all_keys {
            c.rec_show_all_keys = self.all_keys;
        }
        if self.trim != was.trim {
            c.rec_trim = TRIMS[self.trim].1.into();
        }
    }

    fn gif(&self) -> bool {
        self.format == 1
    }

    /// The ring color choices ("Accent", the palette, and the color in
    /// the file when it is none of them) and the current one's index.
    pub fn ring_items(&self) -> (Vec<String>, usize) {
        let mut items: Vec<String> = std::iter::once(tr!("settings.recording.ring_accent").to_string()).chain(RINGS.iter().map(|(l, _)| l.to_string())).collect();
        let sel = if self.ring.is_empty() {
            0
        } else if let Some(i) = RINGS.iter().position(|(_, v)| *v == self.ring) {
            i + 1
        } else {
            items.push(tr!("settings.recording.ring_custom", color = self.ring));
            items.len() - 1
        };
        (items, sel)
    }

    /// Pick ring item `i` of [`RecForm::ring_items`].
    pub fn set_ring(&mut self, i: usize) {
        match i {
            0 => self.ring.clear(),
            i if i <= RINGS.len() => self.ring = RINGS[i - 1].1.into(),
            _ => {} // the custom color: kept
        }
    }
}

/// Device dropdown items: the default first, then the devices, then the
/// saved id when it is not among them; and the index of `cur`.
pub fn device_items(default: &str, devs: Option<&[(String, String)]>, cur: &str) -> (Vec<String>, Vec<String>, usize) {
    let mut names = vec![default.to_string()];
    let mut ids = vec![String::new()];
    for (id, name) in devs.unwrap_or_default() {
        ids.push(id.clone());
        names.push(name.clone());
    }
    let sel = match ids.iter().position(|i| i == cur) {
        Some(i) => i,
        None => {
            // Unplugged, or the list is not here yet: the saved id stays.
            names.push(if devs.is_some() { tr!("settings.recording.saved_device_gone").into() } else { tr!("settings.recording.saved_device").into() });
            ids.push(cur.to_string());
            ids.len() - 1
        }
    };
    (names, ids, sel)
}

/// The Linux encoder line: what `rec_linux_encoder = sel` uses now
/// (`ffmpeg`: where ffmpeg is; `openh264`: whether the library is there).
pub fn encoder_status(sel: usize, ffmpeg: Option<&std::path::Path>, openh264: bool) -> String {
    let ff = ffmpeg.map(|p| tr!("settings.recording.ffmpeg_found", path = p.display()));
    let oh = openh264.then(|| tr!("settings.recording.openh264_downloaded").to_string());
    let line = match ENCODERS.get(sel).map_or("auto", |e| e.1) {
        "ffmpeg" => ff.or(oh),
        "openh264" => oh.or(ff),
        _ => ff.or(oh),
    };
    line.unwrap_or_else(|| tr!("settings.recording.no_encoder").into())
}

/// What the worker found.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub mics: Vec<(String, String)>,
    pub cams: Vec<(String, String)>,
    /// Linux: ffmpeg's path and whether OpenH264 is downloaded.
    pub ffmpeg: Option<PathBuf>,
    pub openh264: bool,
}

/// The tab's lists, asked for once.
pub struct Lists {
    pub found: Option<Found>,
    rx: Option<Receiver<Found>>,
    /// Show the Linux encoder row.
    pub linux: bool,
}

#[cfg_attr(not(target_os = "linux"), allow(clippy::derivable_impls))] // `linux` is true there
impl Default for Lists {
    fn default() -> Lists {
        Lists { found: None, rx: None, linux: cfg!(target_os = "linux") }
    }
}

impl Lists {
    /// Start listing (once), on a worker thread; tests list their fakes now.
    pub fn start(&mut self) {
        if self.found.is_some() || self.rx.is_some() {
            return;
        }
        #[cfg(test)]
        {
            self.found = Some(find());
        }
        #[cfg(not(test))]
        {
            let (tx, rx) = crate::chan::channel();
            let spawned = std::thread::Builder::new().name("rec-devices".into()).spawn(move || {
                let _ = tx.send(find());
            });
            match spawned {
                Ok(_) => self.rx = Some(rx),
                Err(e) => {
                    eprintln!("rustshot: cannot list recording devices: {e}");
                    self.found = Some(Found::default());
                }
            }
        }
    }

    /// The worker answered: returns true once (repaint).
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(f) => {
                self.found = Some(f);
                self.rx = None;
                true
            }
            Err(crate::chan::TryRecvError::Empty) => false,
            Err(crate::chan::TryRecvError::Disconnected) => {
                self.found = Some(Found::default());
                self.rx = None;
                true
            }
        }
    }
}

fn find() -> Found {
    let cams = crate::rec_ui::cameras().into_iter().map(|c| (c.id, c.name)).collect();
    #[cfg(all(target_os = "linux", not(test)))]
    let (ffmpeg, openh264) = (crate::rec::linux::ffmpeg::find(), crate::rec::linux::openh264::installed().is_some());
    #[cfg(not(all(target_os = "linux", not(test))))]
    let (ffmpeg, openh264) = (None, false);
    Found { mics: crate::rec_ui::mics(), cams, ffmpeg, openh264 }
}

/// A dropdown over `(names, ids)` setting `cur` to the picked id; names
/// are cut to fit `w`.
fn device_dropdown(ui: &mut Ui, id: &str, w: f32, (names, ids, sel): (Vec<String>, Vec<String>, usize), cur: &mut String) {
    let text_w = w - 46.0; // the box's padding and chevron
    let shown: Vec<String> = names.iter().map(|n| ui.fit_text(12.0, n, text_w)).collect();
    let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
    let mut i = sel;
    if ui.width(w).dropdown(id, &refs, &mut i) {
        *cur = ids[i].clone();
    }
}

fn labels<L: Label, T: Copy, const N: usize>(list: &[(L, T); N]) -> [&'static str; N] {
    list.map(|(l, _)| l.text())
}

/// Device dropdowns' width (logical px).
const DEVICE_W: f32 = 210.0;

/// The Recording section (the board's rows): the video, then the sound.
pub fn section(ui: &mut Ui, s: &mut super::Settings, tok: &Tok) {
    s.rec_lists.start();
    let lists = &s.rec_lists;
    let found = lists.found.as_ref();
    let show_encoder = lists.linux;
    let status = found.map(|f| encoder_status(s.form.rec.encoder, f.ffmpeg.as_deref(), f.openh264));
    let f = &mut s.form.rec;
    let mics = device_items(&DEFAULT_MIC, found.map(|f| f.mics.as_slice()), &f.mic_dev);
    let on = ui.enabled;
    let gif = f.gif();
    srow(ui, tok, tr!("settings.recording.format"), None, |ui| {
        ui.seg("rec_format", &labels(&FORMATS), &mut f.format, tok);
    });
    srow(ui, tok, tr!("settings.recording.frame_rate"), None, |ui| {
        if f.gif() {
            ui.seg("rec_gif_fps", &labels(&GIF_FPS), &mut f.gif_fps, tok);
        } else {
            ui.seg("rec_fps", &labels(&MP4_FPS), &mut f.fps, tok);
        }
    });
    ui.enabled = on && !gif;
    srow(ui, tok, tr!("settings.recording.quality"), gif.then_some(tr!("settings.recording.mp4_only")), |ui| {
        ui.seg("rec_quality", &labels(&QUALITIES), &mut f.quality, tok);
    });
    ui.enabled = on && gif;
    srow(ui, tok, tr!("settings.recording.max_length"), (!gif).then_some(tr!("settings.recording.gif_only")), |ui| {
        ui.seg("rec_gif_max", &labels(&GIF_MAX), &mut f.gif_max, tok);
    });
    ui.enabled = on;
    srow(ui, tok, tr!("settings.recording.countdown"), Some(tr!("settings.recording.countdown.help")), |ui| {
        ui.switch("rec_countdown", &mut f.countdown, tok);
    });
    if show_encoder {
        let t = status.as_deref().unwrap_or(tr!("settings.recording.looking"));
        srow(ui, tok, tr!("settings.recording.mp4_encoder"), Some(t), |ui| {
            let mut items = labels(&ENCODERS);
            items[0] = tr!("settings.recording.encoder_auto");
            ui.width(120.0).dropdown("rec_linux_encoder", &items, &mut f.encoder);
        });
    }
    let warn = extras(ui, f, tok);

    ui.sec(tr!("settings.recording.sound"), tok);
    ui.enabled = on && !gif;
    srow(ui, tok, tr!("settings.recording.system_sound"), gif.then_some(tr!("settings.recording.gif_no_sound")), |ui| {
        ui.switch("rec_system_audio", &mut f.system, tok);
    });
    srow(ui, tok, tr!("settings.recording.microphone"), None, |ui| {
        ui.switch("rec_mic", &mut f.mic, tok);
        ui.enabled = on && !gif && f.mic;
        device_dropdown(ui, "rec_mic_device", DEVICE_W, mics, &mut f.mic_dev);
    });
    ui.enabled = on;
    if warn {
        s.toast = Some((crate::rec::marks::text::all_keys_warning(), std::time::Instant::now()));
    }
}

/// The rows' height: a little under the others' (the section fits the
/// body with the Linux encoder row and a GIF's notes).
const EXTRA_H: f32 = 32.0;

/// The Recording section's clicks & keys and trim rows (with the video's).
/// True when All keys was just turned on (its warning is said then, in
/// full, and stays short under the row).
fn extras(ui: &mut Ui, f: &mut RecForm, tok: &Tok) -> bool {
    use crate::rec::marks::text as mt;
    let on = ui.enabled;
    let warn = f.show_keys && f.all_keys;
    let note = if warn { mt::ALL_KEYS_SHORT } else { mt::SHOW_NOTE };
    let mut turned_on = false;
    srow_ex(ui, tok, mt::SHOW.get(), Some(note.get()), warn, EXTRA_H, |ui| {
        // Right to left: All keys, Keystrokes, Clicks.
        ui.enabled = on && f.show_keys;
        turned_on = ui.checkbox("rec_show_all_keys", mt::ALL_KEYS.get(), &mut f.all_keys, tok) && f.all_keys;
        ui.enabled = on;
        ui.checkbox("rec_show_keys", mt::KEYS.get(), &mut f.show_keys, tok);
        ui.checkbox("rec_show_clicks", mt::CLICKS.get(), &mut f.show_clicks, tok);
    });
    ui.enabled = on;
    srow_ex(ui, tok, tr!("settings.recording.trim"), Some(tr!("settings.recording.trim.help")), false, EXTRA_H, |ui| {
        ui.seg("rec_trim", &labels(&TRIMS), &mut f.trim, tok);
    });
    turned_on
}

/// The Camera section: the bubble a recording shows.
pub fn camera(ui: &mut Ui, s: &mut super::Settings, tok: &Tok) {
    s.rec_lists.start();
    let found = s.rec_lists.found.as_ref();
    // Downloads as the form has them (Settings › General, not saved yet).
    let downloads = s.form.download_data;
    let f = &mut s.form.rec;
    let cams = device_items(&DEFAULT_CAM, found.map(|f| f.cams.as_slice()), &f.cam_dev);
    let no_cams = found.is_some_and(|f| f.cams.is_empty());
    let on = ui.enabled;
    ui.sec(tr!("settings.camera.camera_bubble"), tok);
    ui.enabled = on && !no_cams;
    srow(ui, tok, tr!("settings.camera.camera"), no_cams.then_some(tr!("settings.camera.no_camera")), |ui| {
        ui.switch("rec_camera", &mut f.camera, tok);
        if !no_cams {
            ui.enabled = on && f.camera;
            device_dropdown(ui, "rec_camera_device", DEVICE_W, cams, &mut f.cam_dev);
        }
    });
    ui.enabled = on && !no_cams;
    srow(ui, tok, tr!("settings.camera.shape"), None, |ui| {
        ui.seg("rec_camera_shape", &labels(&SHAPES), &mut f.shape, tok);
    });
    srow(ui, tok, tr!("settings.camera.size"), Some(tr!("settings.camera.size.help")), |ui| {
        ui.seg("rec_camera_size", &labels(&SIZES), &mut f.size, tok);
    });
    srow(ui, tok, tr!("settings.camera.mirror"), Some(tr!("settings.camera.mirror.help")), |ui| {
        ui.switch("rec_camera_mirror", &mut f.mirror, tok);
    });
    use crate::cam::bg::{Blur, DAMAGED, UNAVAILABLE};
    let state = crate::cam::bg::state();
    let note = state.settings_note();
    // The choices are always there (Off too, whatever the model's state);
    // the note says what a blur needs. Downloads off or a damaged file:
    // a short note on the row, then which file goes where (the row's
    // width, the lines it needs, in the error color).
    let long = matches!(state, Blur::Off { .. } | Blur::Damaged(_));
    let failed = matches!(state, Blur::Failed(_));
    let short = match &state {
        Blur::Off { .. } => UNAVAILABLE.get().to_string(),
        Blur::Damaged(_) => DAMAGED.get().to_string(),
        _ => note.clone(),
    };
    ui.enabled = on && !no_cams;
    let h = if failed { 16.0 + 13.0 * 3.0 + 6.0 } else { super::draw::ROW_H };
    srow_ex(ui, tok, tr!("settings.camera.background"), Some(&short), failed || long, h, |ui| {
        // A blur chosen: the model comes (if it is not here, and the
        // form has downloads on) while the window stays responsive; a
        // failed download is retried.
        let was = f.bg;
        if failed && downloads && ui.clink("rec_camera_bg_retry", tr!("settings.general.language.retry"), 12.0, false, tok) {
            crate::cam::bg::request_model();
        }
        ui.seg("rec_camera_bg", &labels(&BACKGROUNDS), &mut f.bg, tok);
        if f.bg != was && BACKGROUNDS[f.bg].1 != "off" && downloads {
            crate::cam::bg::request_model();
        }
    });
    if let Some(detail) = state.settings_detail() {
        let w = ui.bounds().w - 14.0;
        let n = super::draw::wrap_at_breaks(&detail, w, |s| ui.text_w(11.0, s)).len();
        let h = (16.0 + 13.0 * n as f32 + 6.0).max(super::draw::ROW_H);
        srow_ex(ui, tok, "", Some(&detail), true, h, |_| {});
    }
    ring_row(ui, f, tok, on, no_cams);
}

/// The camera's Ring row (after Background).
fn ring_row(ui: &mut Ui, f: &mut RecForm, tok: &Tok, on: bool, no_cams: bool) {
    ui.enabled = on && !no_cams;
    srow(ui, tok, tr!("settings.camera.ring"), Some(tr!("settings.camera.ring.help")), |ui| {
        ui.width(120.0).slider("rec_camera_ring_width", &mut f.ring_w, 0, config::RING_WIDTH_MAX as u8);
        let (items, sel) = f.ring_items();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let mut i = sel;
        if ui.width(130.0).dropdown("rec_camera_ring_color", &refs, &mut i) {
            f.set_ring(i);
        }
        let c = match config::parse_color(&f.ring) {
            Some((r, g, b, _)) => crate::uifb::C4::rgb(r, g, b),
            None => ui.theme.accent,
        };
        let shown = ui.enabled && f.ring_w > 0;
        ui.swatch("rec_ring_swatch", Some(if shown { c } else { c.fade(0.4) }), false, tok);
    });
    ui.enabled = on;
}

#[cfg(test)]
mod tests;
