//! Settings → **Recording**: the defaults the Record popover starts from
//! (format, frame rate, MP4 quality, system sound and microphone with its
//! device, the camera bubble's device, shape, size, mirror and ring), the
//! 3-2-1 countdown, the record hotkey and, on Linux, the MP4 encoder with
//! a line saying what is installed.
//!
//! [`RecForm`] is the part of the settings form this tab edits (every
//! `rec_*` key and `record_hotkey`); [`Lists`] holds the microphones and
//! cameras, listed on a worker thread when the tab is first shown (Media
//! Foundation and PulseAudio can take a moment), and the encoder status.

use super::{field, BARE_NOTE, LABEL_W};
use crate::config::{self, Config};
use crate::keymap::Chord;
use crate::objects::FRect;
use crate::ui::layout::{GAP, H as ROW};
use crate::ui::Ui;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// (label, config value) tables.
const FORMATS: [(&str, &str); 2] = [("MP4", "mp4"), ("GIF", "gif")];
const MP4_FPS: [(&str, u32); 2] = [("30 fps", 30), ("60 fps", 60)];
const GIF_FPS: [(&str, u32); 2] = [("10 fps", 10), ("15 fps", 15)];
const QUALITIES: [(&str, &str); 3] = [("Low", "low"), ("Medium", "medium"), ("High", "high")];
const SHAPES: [(&str, &str); 3] = [("Circle", "circle"), ("Rounded square", "rounded"), ("Square", "square")];
const SIZES: [(&str, &str); 3] = [("Small", "s"), ("Medium", "m"), ("Large", "l")];
pub const ENCODERS: [(&str, &str); 3] = [("Auto", "auto"), ("ffmpeg", "ffmpeg"), ("OpenH264", "openh264")];
/// The ring colours on offer after "Accent" (the default drawing palette).
const RINGS: [(&str, &str); 10] = [
    ("Red", "#f04438"),
    ("Orange", "#ff8a1f"),
    ("Yellow", "#ffc532"),
    ("Green", "#2dc06f"),
    ("Cyan", "#19b5d6"),
    ("Blue", "#3b82f6"),
    ("Purple", "#8b5cf6"),
    ("Pink", "#ec4899"),
    ("White", "#ffffff"),
    ("Black", "#111318"),
];

const DEFAULT_MIC: &str = "Default microphone";
const DEFAULT_CAM: &str = "Default camera";

fn index_of<T: PartialEq + Copy>(list: &[(&str, T)], v: T) -> usize {
    list.iter().position(|(_, x)| *x == v).unwrap_or(0)
}

fn index_str(list: &[(&str, &str)], v: &str) -> usize {
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
    /// `#rrggbb`, or "" for the theme's accent.
    pub ring: String,
    /// Logical px, 0 = no ring.
    pub ring_w: u8,
    pub countdown: bool,
    pub hotkey: Option<Chord>,
    /// [`ENCODERS`] index.
    pub encoder: usize,
}

impl RecForm {
    pub fn new(cfg: &Config) -> RecForm {
        RecForm {
            format: index_str(&FORMATS, &cfg.rec_format),
            fps: index_of(&MP4_FPS, cfg.rec_fps),
            gif_fps: index_of(&GIF_FPS, cfg.rec_gif_fps),
            quality: index_str(&QUALITIES, &cfg.rec_quality),
            system: cfg.rec_system_audio,
            mic: cfg.rec_mic,
            mic_dev: cfg.rec_mic_device.clone(),
            camera: cfg.rec_camera,
            cam_dev: cfg.rec_camera_device.clone(),
            shape: index_str(&SHAPES, &cfg.rec_camera_shape),
            size: index_str(&SIZES, &cfg.rec_camera_size),
            mirror: cfg.rec_camera_mirror,
            ring: cfg.rec_camera_ring_color.trim().to_ascii_lowercase(),
            ring_w: cfg.rec_camera_ring_width.min(config::RING_WIDTH_MAX) as u8,
            countdown: cfg.rec_countdown,
            hotkey: Chord::parse(&cfg.record_hotkey),
            encoder: index_str(&ENCODERS, &cfg.rec_linux_encoder),
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
    }

    fn gif(&self) -> bool {
        self.format == 1
    }

    /// The ring colour choices ("Accent", the palette, and the colour in
    /// the file when it is none of them) and the current one's index.
    pub fn ring_items(&self) -> (Vec<String>, usize) {
        let mut items: Vec<String> = std::iter::once("Accent".to_string()).chain(RINGS.iter().map(|(l, _)| l.to_string())).collect();
        let sel = if self.ring.is_empty() {
            0
        } else if let Some(i) = RINGS.iter().position(|(_, v)| *v == self.ring) {
            i + 1
        } else {
            items.push(format!("Custom {}", self.ring));
            items.len() - 1
        };
        (items, sel)
    }

    /// Pick ring item `i` of [`RecForm::ring_items`].
    pub fn set_ring(&mut self, i: usize) {
        match i {
            0 => self.ring.clear(),
            i if i <= RINGS.len() => self.ring = RINGS[i - 1].1.into(),
            _ => {} // the custom colour: kept
        }
    }

    /// A problem with the record hotkey next to the General tab's ones.
    pub fn hotkey_note(&self, capture: Option<Chord>, quit: Option<Chord>) -> Option<&'static str> {
        let k = self.hotkey?;
        if Some(k) == capture {
            Some("Record and Capture use the same keys.")
        } else if Some(k) == quit {
            Some("Record and Quit Rustshot use the same keys.")
        } else {
            let modded = k.ctrl || k.alt || k.shift || k.meta;
            let special = (0x70..=0x7B).contains(&k.vk) || k.vk == crate::wind::key::PRINTSCREEN;
            (!(modded || special)).then_some(BARE_NOTE)
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
            names.push(if devs.is_some() { "Saved device (not connected)".into() } else { "Saved device".into() });
            ids.push(cur.to_string());
            ids.len() - 1
        }
    };
    (names, ids, sel)
}

/// The Linux encoder line: what `rec_linux_encoder = sel` uses now
/// (`ffmpeg`: where ffmpeg is; `openh264`: whether the library is there).
pub fn encoder_status(sel: usize, ffmpeg: Option<&std::path::Path>, openh264: bool) -> String {
    let ff = ffmpeg.map(|p| format!("ffmpeg found at {}", p.display()));
    let oh = openh264.then(|| "OpenH264 downloaded".to_string());
    let line = match ENCODERS.get(sel).map_or("auto", |e| e.1) {
        "ffmpeg" => ff.or(oh),
        "openh264" => oh.or(ff),
        _ => ff.or(oh),
    };
    line.unwrap_or_else(|| "Not available: Rustshot asks at the first MP4 recording".into())
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
            let (tx, rx) = std::sync::mpsc::channel();
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
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
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

/// `s` cut with an ellipsis to fit `w` logical px.
fn fit(ui: &Ui, s: &str, w: f32) -> String {
    if ui.text_width(s) <= w {
        return s.to_string();
    }
    let mut out: String = s.to_string();
    while !out.is_empty() && ui.text_width(&format!("{out}…")) > w {
        out.pop();
    }
    format!("{}…", out.trim_end())
}

/// A dropdown over `(names, ids)` setting `cur` to the picked id.
fn device_dropdown(ui: &mut Ui, id: &str, w: f32, (names, ids, sel): (Vec<String>, Vec<String>, usize), cur: &mut String) {
    let text_w = w - 50.0; // the box's padding and chevron
    let shown: Vec<String> = names.iter().map(|n| fit(ui, n, text_w)).collect();
    let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
    let mut i = sel;
    if ui.width(w).dropdown(id, &refs, &mut i) {
        *cur = ids[i].clone();
    }
}

fn labels<T: Copy, const N: usize>(list: &[(&'static str, T); N]) -> [&'static str; N] {
    list.map(|(l, _)| l)
}

/// The tab; returns where its last row ends (logical px).
pub fn tab(ui: &mut Ui, s: &mut super::Settings, body: FRect) -> f32 {
    s.rec_lists.start();
    let slot_note = s.hotkey_slot_note(2);
    let lists = &s.rec_lists;
    let found = lists.found.as_ref();
    let show_encoder = lists.linux;
    let status = found.map(|f| encoder_status(s.form.rec.encoder, f.ffmpeg.as_deref(), f.openh264));
    let (capture, quit) = (s.form.capture, s.form.quit);
    let f = &mut s.form.rec;
    let note = f.hotkey_note(capture, quit);
    // One note row at most: a clash says more than "not registered".
    let note: Option<String> = note.map(str::to_string).or(slot_note);
    let mics = device_items(DEFAULT_MIC, found.map(|f| f.mics.as_slice()), &f.mic_dev);
    let cams = device_items(DEFAULT_CAM, found.map(|f| f.cams.as_slice()), &f.cam_dev);
    let no_cams = found.is_some_and(|f| f.cams.is_empty());
    let rest = body.w - (LABEL_W + GAP);
    ui.area(body, |ui| {
        ui.lay.gap = 6.0;
        let on = ui.enabled;
        ui.heading("Video");
        field(ui, "Format", |ui| {
            ui.width(96.0).dropdown("rec_format", &labels(&FORMATS), &mut f.format);
            if f.gif() {
                ui.width(100.0).dropdown("rec_gif_fps", &labels(&GIF_FPS), &mut f.gif_fps);
            } else {
                ui.width(100.0).dropdown("rec_fps", &labels(&MP4_FPS), &mut f.fps);
            }
            ui.space(4.0);
            ui.enabled = on && !f.gif();
            ui.label("Quality");
            ui.width(110.0).dropdown("rec_quality", &labels(&QUALITIES), &mut f.quality);
            ui.enabled = on;
        });
        field(ui, "Countdown", |ui| {
            ui.toggle("rec_countdown", &mut f.countdown);
            ui.space(20.0);
            ui.label("Record hotkey");
            ui.allow_meta = true;
            ui.key_capture("record_hotkey", &mut f.hotkey);
            ui.allow_meta = false;
        });
        if let Some(n) = &note {
            ui.row(|ui| {
                ui.space(LABEL_W + GAP);
                ui.height(18.0).error_note(n);
            });
        }
        if show_encoder {
            field(ui, "MP4 encoder", |ui| {
                ui.width(120.0).dropdown("rec_linux_encoder", &labels(&ENCODERS), &mut f.encoder);
                let t = status.as_deref().unwrap_or("Looking…");
                let t = fit(ui, t, rest - 120.0 - GAP);
                ui.note(&t);
            });
        }

        ui.heading("Sound");
        field(ui, "System sound", |ui| {
            let gap = ui.lay.gap;
            ui.enabled = on && !f.gif();
            ui.toggle("rec_system_audio", &mut f.system);
            ui.space(20.0);
            ui.label("Microphone");
            ui.toggle("rec_mic", &mut f.mic);
            if f.gif() {
                ui.enabled = on;
                ui.note("GIFs have no sound");
            } else {
                ui.enabled = on && f.mic;
                let used = 36.0 + gap + 20.0 + ui.text_width("Microphone").ceil() + gap + 36.0 + gap;
                device_dropdown(ui, "rec_mic_device", (rest - used).floor(), mics, &mut f.mic_dev);
            }
            ui.enabled = on;
        });

        ui.heading("Camera bubble");
        field(ui, "Camera", |ui| {
            ui.enabled = on && !no_cams;
            ui.toggle("rec_camera", &mut f.camera);
            ui.enabled = on && !no_cams && f.camera;
            if no_cams {
                ui.note("No camera found");
            } else {
                let w = (rest - 36.0 - ui.lay.gap).floor();
                device_dropdown(ui, "rec_camera_device", w, cams, &mut f.cam_dev);
            }
            ui.enabled = on;
        });
        ui.enabled = on && !no_cams;
        field(ui, "Shape", |ui| {
            ui.width(150.0).dropdown("rec_camera_shape", &labels(&SHAPES), &mut f.shape);
            ui.space(4.0);
            ui.label("Size");
            ui.width(110.0).dropdown("rec_camera_size", &labels(&SIZES), &mut f.size);
            ui.space(4.0);
            ui.label("Mirror");
            ui.toggle("rec_camera_mirror", &mut f.mirror);
        });
        field(ui, "Ring", |ui| {
            let (items, sel) = f.ring_items();
            let refs: Vec<&str> = items.iter().map(String::as_str).collect();
            let mut i = sel;
            // A swatch of the colour left of the list.
            let sw = ui.alloc(Some(20.0), ROW);
            let c = match config::parse_color(&f.ring) {
                Some((r, g, b, _)) => crate::uifb::C4::rgb(r, g, b),
                None => ui.theme.accent,
            };
            let d = ui.px(16.0);
            let (cx, cy) = (sw.x + sw.w / 2.0, sw.y + sw.h / 2.0);
            let fade = if ui.enabled && f.ring_w > 0 { 1.0 } else { 0.4 };
            ui.fb.fill_circle(cx, cy, d / 2.0 + ui.px(1.0), ui.theme.border.fade(fade));
            ui.fb.fill_circle(cx, cy, d / 2.0, c.fade(fade));
            if ui.width(150.0).dropdown("rec_camera_ring_color", &refs, &mut i) {
                f.set_ring(i);
            }
            ui.space(4.0);
            ui.label("Width");
            ui.slider("rec_camera_ring_width", &mut f.ring_w, 0, config::RING_WIDTH_MAX as u8);
        });
        ui.enabled = on;
        ui.cursor_y() - ui.lay.gap
    })
}

#[cfg(test)]
mod tests;
