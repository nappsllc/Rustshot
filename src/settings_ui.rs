//! The Settings window (the Settings board, 680 × 520): a rail of short
//! sections, each fitting the body without scrolling ([`SECTIONS`]):
//! **General** (theme, accent, launch at login, tray icon, update
//! checks), **Capture** (Win+Shift+S, snapping, Accept on select, what
//! happens after a capture), **Overlay** (magnifier, hints, UI size,
//! renderer, "Reset tool sizes & colors"), **Saving** (folder, Save As,
//! format and JPEG quality), **File names** (daily subfolders and the
//! file-name pattern editor), **History**, **Recording** and **Camera**
//! (the Record popover's defaults: [`recording`]), **Upload** (Imgur),
//! then under *Shortcuts* **Global**, **Recording** (the record hotkey and
//! the while-recording keys), **Overlay tools**, **Overlay actions**, and
//! **About**, with a footer of **OK** / **Cancel** / **Apply** (the board
//! leaves it out; the window keeps its explicit save). Drawn in [`draw`].
//! Opened from the tray ("Settings…") and by `rustshot settings`
//! ([`show`], [`run_here`]).
//!
//! The form ([`Form`]) is a plain copy of the settings it edits; saving
//! writes only the values the user changed onto the file as it is on disk
//! then (so a `skip_version` written meanwhile survives) and rewrites the
//! whole `config.toml` ([`config::save_at`]). A file with comments or
//! unknown keys would lose them: the first save asks first ([`REWRITE`]).
//! A file that does not parse is never replaced by the defaults behind the
//! user's back: the window shows the error ([`broken_text`]) and offers to
//! open the file or, after a second question ([`RESET`]), to reset it
//! (the old file is kept as `config.toml.bak`, `.bak.1`, ...).
//! After a save the daemon reloads the config (re-registers its hotkeys,
//! applies `check_updates` and `show_tray`); every later capture uses it.
//!
//! Drawn with the `ui` kit in a `wind::run_window` window on its own
//! thread, one window at a time: another request brings it to the front.
//! A hotkey the daemon could not register after a save is reported in the
//! open window ([`notify`]) and noted under its row while it stays
//! unregistered; Apply then asks the daemon to try again. macOS opens windows only on the main thread,
//! so there the config file opens in the default editor instead.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use crate::config::{self, Config};
use crate::export;
use crate::hotkey::{self, HotEvent};
use crate::keymap::{Action, Chord, Keymap};
use crate::objects::FRect;
use crate::pixbuf::PixBuf;
use crate::screenclip_win::Handler;
use crate::theme::Theme;
use crate::ui::{FocusState, Input, TextState, Ui};
use crate::uisize::{self, UiSize};
use crate::wind::{self, Cursor, Driver, Ev, Hwnd, WindowSpec};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender};

/// Window size, logical px.
pub const W: u32 = 680;
pub const H: u32 = 520;

pub(crate) mod draw;
mod recording;
mod upload;

/// The rail's sections (icon, name); About sits at the bottom. Each one
/// fits the body without scrolling (`every_section_fits`).
pub const SECTIONS: [(&str, &str); 15] = [
    ("set-sliders", "General"),
    ("set-camera", "Capture"),
    ("region", "Overlay"),
    ("set-folder", "Saving"),
    ("text", "File names"),
    ("history", "History"),
    ("video", "Recording"),
    ("round", "Camera"),
    ("cloud", "Upload"),
    ("layers", "S3 storage"),
    ("set-keyboard", "Global"),
    ("film", "Recording"),
    ("pencil", "Overlay tools"),
    ("copy", "Overlay actions"),
    ("info", "About"),
];
/// The header's title of each section: the rail's name, with its group
/// for the shortcut sections.
pub const TITLES: [&str; SECTIONS.len()] = [
    "General",
    "Capture",
    "Overlay",
    "Saving",
    "File names",
    "History",
    "Recording",
    "Camera",
    "Upload",
    "S3 storage",
    "Shortcuts \u{b7} Global",
    "Shortcuts \u{b7} Recording",
    "Shortcuts \u{b7} Overlay tools",
    "Shortcuts \u{b7} Overlay actions",
    "About",
];
/// Section indices.
pub const GENERAL: usize = 0;
pub const CAPTURE: usize = 1;
pub const OVERLAY: usize = 2;
pub const SAVING: usize = 3;
pub const FILE_NAMES: usize = 4;
pub const HISTORY: usize = 5;
pub const RECORDING: usize = 6;
pub const CAMERA: usize = 7;
pub const UPLOAD: usize = 8;
/// An S3-compatible bucket's settings (Upload › Upload to S3-compatible).
pub const S3_STORAGE: usize = 9;
/// The shortcut sections: the global hotkeys, the record hotkey and the
/// while-recording keys, the overlay's tool keys, its action keys.
pub const KEYS_GLOBAL: usize = 10;
pub const KEYS_RECORDING: usize = 11;
pub const KEYS_TOOLS: usize = 12;
pub const KEYS_ACTIONS: usize = 13;
pub const ABOUT: usize = 14;
/// The rail's group labels: (first section, label).
pub const RAIL_GROUPS: [(usize, &str); 1] = [(KEYS_GLOBAL, "Shortcuts")];

/// One of the shortcut sections (key rows, Reset to defaults).
pub fn is_keys(section: usize) -> bool {
    (KEYS_GLOBAL..=KEYS_ACTIONS).contains(&section)
}

/// The section that lists global hotkey `slot` (see [`GLOBALS`]): the
/// record hotkey is with the while-recording keys.
pub fn section_of_slot(slot: usize) -> usize {
    if slot == 2 { KEYS_RECORDING } else { KEYS_GLOBAL }
}

/// The section that lists overlay or while-recording action `a`.
pub fn section_of_action(a: Action) -> usize {
    if a.while_recording() {
        KEYS_RECORDING
    } else if draw::TOOL_ROWS.iter().any(|(_, acts)| acts.contains(&a)) {
        KEYS_TOOLS
    } else {
        KEYS_ACTIONS
    }
}

/// The section with key box `field` ("global3", "key_rec_pause"; the
/// hotkey dialog's "Change shortcuts…"); Global when it is none of them.
pub fn section_of_field(field: &str) -> usize {
    if let Some(slot) = field.strip_prefix("global").and_then(|n| n.parse::<usize>().ok()) {
        return section_of_slot(slot);
    }
    field.strip_prefix("key_").and_then(Action::from_id).map_or(KEYS_GLOBAL, section_of_action)
}

/// (label, config value)
const THEMES: [(&str, &str); 3] = [("System", "auto"), ("Dark", "dark"), ("Light", "light")];
const RENDERERS: [(&str, &str); 2] = [("GDI", "gdi"), ("Software", "software")];
const FORMATS: [(&str, &str); 3] = [("PNG", "png"), ("JPG", "jpg"), ("BMP", "bmp")];
/// Capture › Accept on select (`accept_on_select`).
pub const ACCEPTS: [(&str, &str); 3] = [("Off", "off"), ("Copy", "copy"), ("Save", "save")];
/// Capture › Copy text (copy_text_mode): the overlay's button copies all,
/// or opens Text mode to pick words (the hotkey always copies all).
pub const COPY_TEXT: [(&str, &str); 2] = [("Copy all at once", "all"), ("Let me select", "select")];
/// Capture › Redact style (`redact_style`): an opaque fill, or a mosaic
/// under a blur.
pub const REDACT_STYLES: [(&str, &str); 2] = [("Solid", "solid"), ("Blur", "blur")];
/// Saving "Save at" (`save_scale`): the screen's pixels, 1× (the
/// logical size of a high-DPI capture) or a custom percent.
pub const SAVE_AT: [(&str, &str); 3] = [("Screen pixels", "screen"), ("1\u{d7}", "logical"), ("Custom", "custom")];
/// Capture › HDR screenshots (`hdr_screenshots`; Windows).
pub const HDR_MODES: [(&str, &str); 2] = [("Tone-map to SDR", "tonemap"), ("Keep as captured", "keep")];
/// Capture › Add a border: the width a new border starts at (logical px).
pub const BORDER_DEFAULT: u8 = 2;
const DEFAULT_FILENAME: &str = "%F_%H-%M";
/// Capture › History: how long and how much it keeps (value, label).
pub const HISTORY_DAYS: [(u32, &str); 4] = [(7, "7 days"), (30, "30 days"), (90, "90 days"), (365, "1 year")];
pub const HISTORY_MB: [(u32, &str); 4] = [(1024, "1 GB"), (2048, "2 GB"), (5120, "5 GB"), (10240, "10 GB")];

/// The accent swatches: Periwinkle (the theme's, `ui_color` empty), then
/// [`crate::theme::ACCENTS`], then the custom color.
pub const ACCENT_CUSTOM: usize = 3;

/// The global hotkeys of Key bindings, in board order: (label, hotkey
/// slot: 0 capture, 1 quit, 2 record, 3 full screen, 4 window, 5 repeat,
/// 6 copy text, 7 History).
pub const GLOBALS: [(&str, usize); 8] = [
    ("Capture region", 0),
    ("Capture full screen", 3),
    ("Capture window", 4),
    ("Record video", 2),
    ("Repeat last capture", 5),
    ("Copy text from area", 6),
    ("Open History", 7),
    ("Quit Rustshot", 1),
];

/// The question when "Show icon in tray" is turned off: the way back in.
/// (Windows: off means no background process at all.)
#[cfg(windows)]
pub const HIDE_TRAY: &str = "Rustshot won't run in the background. Win+Shift+S and Print Screen still open it if Rustshot is set as your screenshot app (Settings > Capture); other global shortcuts stop working. To open Settings, start Rustshot from the Start menu.";
#[cfg(not(windows))]
pub const HIDE_TRAY: &str = "The tray icon will be hidden. To open Settings later, start Rustshot again from your applications menu (or run `rustshot settings`). Your shortcuts keep working.";
/// [`HIDE_TRAY`]'s extra line while Win+Shift+S does not open Rustshot.
pub const HIDE_TRAY_SETUP: &str = "Rustshot isn't set as your screenshot app yet \u{2014} Set up\u{2026}";
/// The "Show icon in tray" row's help (Windows).
pub const TRAY_HELP: &str = "Off: no background process; only Win+Shift+S / Print Screen (when set up)";
/// Where the icon off keeps the daemon (the Store build, which cannot
/// take Win+Shift+S): the question and the row's help.
pub const HIDE_TRAY_KEEP: &str = "The tray icon will be hidden. Rustshot keeps running in the background and your shortcuts keep working. To open Settings, start Rustshot again from the Start menu.";
pub const TRAY_HELP_KEEP: &str = "Off: no icon; Rustshot keeps running and its shortcuts work";
/// The one-time notice for a file from before the icon off meant "no
/// background process" (the first start of this version).
pub const TRAY_OFF_NOTICE: &str = "Rustshot no longer runs in the background while the tray icon is off. Win+Shift+S and Print Screen can still open it.";

/// A global row's note while another app holds its key.
pub const IN_USE: &str = "In use by another app";

/// The file-name editor's token buttons: (label, inserted text), shown in
/// columns (top to bottom, then the next column).
pub const TOKENS: [(&str, &str); 15] = [
    ("Century (00-99)", "%C"),
    ("Day (001-366)", "%j"),
    ("Day (01-31)", "%d"),
    ("Day of Month (1-31)", "%e"),
    ("Full Date (%Y-%m-%d)", "%F"),
    ("Full Date (%d-%m-%Y)", "%d-%m-%Y"),
    ("Hour (00-23)", "%H"),
    ("Hour (01-12)", "%I"),
    ("Minute (00-59)", "%M"),
    ("Month (01-12)", "%m"),
    ("Second (00-59)", "%S"),
    ("Week (01-53)", "%V"),
    ("Week Day (1-7)", "%u"),
    ("Year (00-99)", "%y"),
    ("Year (2000)", "%Y"),
];

/// Asked once before the first save over a file rustshot would not write
/// that way (comments, unknown keys).
pub const REWRITE: &str = "Saving will rewrite config.toml and remove comments.";

/// The question before "Reset to defaults…" replaces a broken file.
pub const RESET: &str = "Replace config.toml with default settings? Your current values will be lost.";

/// The error state's text for a `config.toml` that does not parse (`e`
/// as [`config::read_at`] words it: "line 3: expected key = value").
pub fn broken_text(e: &str) -> String {
    format!("config.toml has an error ({e}). Fix it or reset it.")
}

/// Where "Reset to defaults…" keeps the old file: `config.toml.bak`, or
/// `config.toml.bak.1`, `.bak.2`, ... when that is taken (backups never
/// overwrite each other).
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "config.toml".into());
    name.push(".bak");
    let first = path.with_file_name(&name);
    if !first.exists() {
        return first;
    }
    (1u32..)
        .map(|n| {
            let mut s = name.clone();
            s.push(format!(".{n}"));
            path.with_file_name(s)
        })
        .find(|p| !p.exists())
        .expect("a free name")
}

fn theme_index(v: &str) -> usize {
    THEMES.iter().position(|(_, t)| v.trim().eq_ignore_ascii_case(t)).unwrap_or(0)
}

fn format_index(cfg: &Config) -> usize {
    match export::Format::from_config(cfg) {
        export::Format::Png => 0,
        export::Format::Jpeg => 1,
        export::Format::Bmp => 2,
    }
}

/// The folder field's text: `save_path`, or the folder an empty one means.
fn folder_text(cfg: &Config) -> String {
    if cfg.save_path.trim().is_empty() {
        export::default_save_dir(cfg).display().to_string()
    } else {
        cfg.save_path.clone()
    }
}

fn chord_text(c: Option<Chord>) -> String {
    c.map(|c| c.display()).unwrap_or_default()
}

/// `ui_color` → the swatch it is (0 Periwinkle, 1 Teal, 2 Amber,
/// [`ACCENT_CUSTOM`]).
fn accent_index(v: &str) -> usize {
    let Some((r, g, b, _)) = config::parse_color(v) else { return 0 };
    let c = crate::uifb::C4::rgb(r, g, b);
    crate::theme::ACCENTS.iter().position(|a| a.1 == c).map_or(ACCENT_CUSTOM, |i| i + 1)
}

/// A swatch's `ui_color` (lowercase `#rrggbb`); None for the custom one.
fn accent_value(i: usize) -> Option<String> {
    match i {
        0 => Some(String::new()),
        ACCENT_CUSTOM => None,
        i => crate::theme::ACCENTS.get(i - 1).map(|a| format!("#{:02x}{:02x}{:02x}", a.1.r, a.1.g, a.1.b)),
    }
}

/// The values the window edits.
#[derive(Clone, Debug)]
pub struct Form {
    pub theme: usize,
    /// Accent swatch (`ui_color`), and the custom color's text.
    pub accent: usize,
    pub accent_hex: TextState,
    /// The UI size ([`uisize::LABELS`] index).
    pub ui_size: usize,
    pub renderer: usize,
    /// Snap to windows (`snap_windows`).
    pub snap: bool,
    /// Start at login; `None` hides it (store installs).
    pub autostart: Option<bool>,
    /// Win+Shift+S and Print Screen open Rustshot (the `ms-screenclip:`
    /// handler, Windows); `None` hides it (other systems, MSIX).
    pub screenclip: Option<bool>,
    pub check_updates: bool,
    pub show_tray: bool,
    pub copy_after: bool,
    pub shutter: bool,
    /// "Show thumbnail after capture" (`show_thumbnail`).
    pub thumbnail: bool,
    /// Capture › History: keep it, for how long ([`HISTORY_DAYS`] index;
    /// past the end: another value), up to how much ([`HISTORY_MB`]).
    pub history: bool,
    pub history_days: usize,
    pub history_size: usize,
    pub capture: Option<Chord>,
    pub quit: Option<Chord>,
    pub full: Option<Chord>,
    pub window: Option<Chord>,
    pub repeat: Option<Chord>,
    /// Open the History window.
    pub history_key: Option<Chord>,
    /// Copy text from an area (OCR).
    pub text: Option<Chord>,
    pub folder: TextState,
    pub subfolder: bool,
    pub subfolder_pattern: TextState,
    pub format: usize,
    pub quality: u8,
    pub ask: bool,
    pub filename: TextState,
    /// The Upload section's values (`upload_*`, `s3_*`,
    /// `copy_url_after_upload`; the typed S3 secret, never written here).
    pub up: upload::UpForm,
    pub keys: Keymap,
    /// The Recording section's values (every `rec_*` key, `record_hotkey`).
    pub rec: recording::RecForm,
    /// Capture › Overlay: the magnifier, round or square, and Accept on
    /// select ([`ACCEPTS`] index); General: the overlay's hint card.
    pub magnifier: bool,
    pub magnifier_square: bool,
    pub accept: usize,
    pub hints: bool,
    /// Capture › Overlay: the Copy text button ([COPY_TEXT] index).
    pub copy_text: usize,
    /// Capture › Overlay: Quick Redact's style ([`REDACT_STYLES`] index).
    pub redact_style: usize,
    /// Capture › Image: trim margins of new selections (`trim_margins`),
    /// the export border (on, its width 1-8 and color: `border_width`,
    /// `border_color`), HDR screenshots ([`HDR_MODES`] index).
    pub trim: bool,
    pub border: bool,
    pub border_w: u8,
    pub border_hex: TextState,
    pub hdr: usize,
    /// Saving "Save at" ([`SAVE_AT`] index) and its custom
    /// percent (10-100).
    pub save_at: usize,
    pub save_pct: u8,
}

impl Form {
    pub fn new(cfg: &Config, autostart: Option<bool>) -> Form {
        let accent = accent_index(&cfg.ui_color);
        Form {
            theme: theme_index(&cfg.theme),
            accent,
            accent_hex: TextState::new(if accent == ACCENT_CUSTOM { cfg.ui_color.trim() } else { "" }),
            ui_size: cfg.ui_scale.index(),
            renderer: !cfg.use_gdi() as usize,
            snap: cfg.snap_windows,
            autostart,
            screenclip: None,
            check_updates: cfg.check_updates,
            show_tray: cfg.show_tray,
            copy_after: cfg.copy_after_capture,
            shutter: cfg.shutter_sound,
            thumbnail: cfg.show_thumbnail,
            history: cfg.history,
            history_days: HISTORY_DAYS.iter().position(|d| d.0 == cfg.history_days).unwrap_or(usize::MAX),
            history_size: HISTORY_MB.iter().position(|m| m.0 == cfg.history_max_mb).unwrap_or(usize::MAX),
            capture: Chord::parse(&cfg.capture_hotkey),
            quit: Chord::parse(&cfg.quit_hotkey),
            full: Chord::parse(&cfg.capture_full_hotkey),
            window: Chord::parse(&cfg.capture_window_hotkey),
            repeat: Chord::parse(&cfg.repeat_hotkey),
            text: Chord::parse(&cfg.capture_text_hotkey),
            history_key: Chord::parse(&cfg.history_hotkey),
            folder: TextState::new(&folder_text(cfg)),
            subfolder: cfg.save_subfolder,
            subfolder_pattern: TextState::new(&cfg.subfolder_pattern),
            format: format_index(cfg),
            quality: cfg.jpeg_quality.clamp(1, 100),
            ask: cfg.save_dialog,
            filename: TextState::new(&cfg.filename_pattern),
            up: upload::UpForm::new(cfg),
            keys: Keymap::from_config(&cfg.shortcuts).0,
            rec: recording::RecForm::new(cfg),
            magnifier: cfg.magnifier,
            magnifier_square: cfg.magnifier_square,
            accept: ACCEPTS.iter().position(|a| cfg.accept_on_select.trim().eq_ignore_ascii_case(a.1)).unwrap_or(0),
            hints: cfg.overlay_hints,
            copy_text: COPY_TEXT.iter().position(|a| cfg.copy_text_mode.trim() == a.1).unwrap_or(1),
            redact_style: REDACT_STYLES.iter().position(|a| cfg.redact_style.trim() == a.1).unwrap_or(0),
            trim: cfg.trim_margins,
            border: cfg.border_width > 0,
            border_w: if cfg.border_width > 0 { cfg.border_width.min(config::BORDER_MAX) as u8 } else { BORDER_DEFAULT },
            border_hex: TextState::new(cfg.border_color.trim()),
            hdr: HDR_MODES.iter().position(|m| cfg.hdr_screenshots.trim() == m.1).unwrap_or(0),
            save_at: SAVE_AT.iter().position(|m| cfg.save_scale.trim() == m.1).unwrap_or(0),
            save_pct: cfg.save_scale_percent.clamp(10, 100) as u8,
        }
    }

    /// The global hotkey of `slot` (see [`GLOBALS`]).
    pub fn global(&self, slot: usize) -> Option<Chord> {
        match slot {
            0 => self.capture,
            1 => self.quit,
            2 => self.rec.hotkey,
            3 => self.full,
            4 => self.window,
            6 => self.text,
            7 => self.history_key,
            _ => self.repeat,
        }
    }

    pub fn global_mut(&mut self, slot: usize) -> &mut Option<Chord> {
        match slot {
            0 => &mut self.capture,
            1 => &mut self.quit,
            2 => &mut self.rec.hotkey,
            3 => &mut self.full,
            4 => &mut self.window,
            6 => &mut self.text,
            7 => &mut self.history_key,
            _ => &mut self.repeat,
        }
    }

    /// `onto` with the values the user changed from `loaded` (the config
    /// the form was made from). A value left as it was keeps `onto`'s, so
    /// spellings the form cannot show (an unknown theme, a hotkey it
    /// cannot parse) and edits made to the file meanwhile survive.
    pub fn apply(&self, loaded: &Config, onto: &Config) -> Config {
        let was = Form::new(loaded, None);
        let mut c = onto.clone();
        if self.theme != was.theme {
            c.theme = THEMES[self.theme].1.into();
        }
        if self.accent != was.accent || (self.accent == ACCENT_CUSTOM && self.accent_hex.text.trim() != was.accent_hex.text.trim()) {
            match accent_value(self.accent) {
                Some(v) => c.ui_color = v,
                // A custom color that does not parse is not written.
                None if config::parse_color(self.accent_hex.text.trim()).is_some() => c.ui_color = self.accent_hex.text.trim().to_string(),
                None => {}
            }
        }
        if self.ui_size != was.ui_size {
            c.ui_scale = UiSize::from_index(self.ui_size);
        }
        if self.renderer != was.renderer {
            c.renderer = RENDERERS[self.renderer].1.into();
        }
        if self.snap != was.snap {
            c.snap_windows = self.snap;
        }
        if self.check_updates != was.check_updates {
            c.check_updates = self.check_updates;
        }
        if self.show_tray != was.show_tray {
            c.show_tray = self.show_tray;
        }
        if self.copy_after != was.copy_after {
            c.copy_after_capture = self.copy_after;
        }
        if self.shutter != was.shutter {
            c.shutter_sound = self.shutter;
        }
        if self.history != was.history {
            c.history = self.history;
        }
        if self.history_days != was.history_days && let Some(d) = HISTORY_DAYS.get(self.history_days) {
            c.history_days = d.0;
        }
        if self.history_size != was.history_size && let Some(m) = HISTORY_MB.get(self.history_size) {
            c.history_max_mb = m.0;
        }
        if self.thumbnail != was.thumbnail {
            c.show_thumbnail = self.thumbnail;
        }
        if self.capture != was.capture {
            c.capture_hotkey = chord_text(self.capture);
        }
        if self.quit != was.quit {
            c.quit_hotkey = chord_text(self.quit);
        }
        if self.full != was.full {
            c.capture_full_hotkey = chord_text(self.full);
        }
        if self.window != was.window {
            c.capture_window_hotkey = chord_text(self.window);
        }
        if self.repeat != was.repeat {
            c.repeat_hotkey = chord_text(self.repeat);
        }
        if self.history_key != was.history_key {
            c.history_hotkey = chord_text(self.history_key);
        }
        if self.text != was.text {
            c.capture_text_hotkey = chord_text(self.text);
        }
        if self.folder.text.trim() != was.folder.text.trim() {
            c.save_path = self.folder.text.trim().to_string();
        }
        if self.subfolder != was.subfolder {
            c.save_subfolder = self.subfolder;
        }
        if self.subfolder_pattern.text != was.subfolder_pattern.text {
            c.subfolder_pattern = self.subfolder_pattern.text.clone();
        }
        if self.format != was.format {
            c.save_format = FORMATS[self.format].1.into();
        }
        if self.quality != was.quality {
            c.jpeg_quality = self.quality;
        }
        if self.ask != was.ask {
            c.save_dialog = self.ask;
        }
        if self.filename.text != was.filename.text {
            c.filename_pattern = self.filename.text.clone();
        }
        self.up.apply(&was.up, &mut c);
        self.rec.apply(&was.rec, &mut c);
        if self.magnifier != was.magnifier {
            c.magnifier = self.magnifier;
        }
        if self.magnifier_square != was.magnifier_square {
            c.magnifier_square = self.magnifier_square;
        }
        if self.accept != was.accept && let Some(a) = ACCEPTS.get(self.accept) {
            c.accept_on_select = a.1.into();
        }
        if self.hints != was.hints {
            c.overlay_hints = self.hints;
        }
        if self.copy_text != was.copy_text && let Some(a) = COPY_TEXT.get(self.copy_text) {
            c.copy_text_mode = a.1.into();
        }
        if self.redact_style != was.redact_style && let Some(a) = REDACT_STYLES.get(self.redact_style) {
            c.redact_style = a.1.into();
        }
        if self.trim != was.trim {
            c.trim_margins = self.trim;
        }
        if self.save_at != was.save_at && let Some(m) = SAVE_AT.get(self.save_at) {
            c.save_scale = m.1.into();
        }
        if self.save_pct != was.save_pct {
            c.save_scale_percent = u32::from(self.save_pct.clamp(10, 100));
        }
        if self.border != was.border || (self.border && self.border_w != was.border_w) {
            c.border_width = if self.border { u32::from(self.border_w.clamp(1, config::BORDER_MAX as u8)) } else { 0 };
        }
        // A color that does not parse is not written.
        let hex = self.border_hex.text.trim();
        if hex != was.border_hex.text.trim() && config::parse_color(hex).is_some() {
            c.border_color = hex.to_string();
        }
        if self.hdr != was.hdr && let Some(m) = HDR_MODES.get(self.hdr) {
            c.hdr_screenshots = m.1.into();
        }
        let changed: Vec<Action> = Action::ALL.into_iter().filter(|a| self.keys.chords(*a) != was.keys.chords(*a)).collect();
        if !changed.is_empty() {
            let mut km = Keymap::from_config(&onto.shortcuts).0;
            for a in changed {
                km.set(a, self.keys.chords(a).to_vec());
            }
            let mut sc = km.to_config();
            // Entries for actions this build does not know stay as they are.
            for (k, v) in &onto.shortcuts {
                if Action::from_id(k.trim()).is_none() {
                    sc.insert(k.clone(), v.clone());
                }
            }
            c.shortcuts = sc;
        }
        c
    }

    /// Insert a token at the caret of `target` (replacing its selection).
    pub fn insert(&mut self, target: Pattern, token: &str) {
        match target {
            Pattern::File => self.filename.insert(token),
            Pattern::Sub => self.subfolder_pattern.insert(token),
        }
    }

    /// Where Ctrl+S would save now, with the form's values over `base`.
    pub fn preview(&self, base: &Config, now: export::Tm) -> PathBuf {
        export::auto_save_path(&self.apply(base, base), now)
    }

    /// Every global hotkey and overlay key back to the defaults (the
    /// Key bindings header's "Reset to defaults").
    pub fn reset_keys(&mut self) {
        let d = Config::default();
        self.keys = Keymap::defaults();
        for slot in 0..hotkey::SLOTS {
            *self.global_mut(slot) = Chord::parse(&hotkey::wanted(&d)[slot]);
        }
    }

    /// The overlay actions bound to more than one action's chord.
    pub fn conflicting(&self) -> Vec<Action> {
        let mut v: Vec<Action> = self.keys.conflicts().into_iter().flat_map(|(_, v)| v).collect();
        v.extend(self.keys.plain_typing());
        v
    }

    /// The message under an overlay action in a conflict: "Already used
    /// by Pencil. Choose another." (the other action's name); None
    /// otherwise.
    pub fn conflict_note(&self, a: Action) -> Option<String> {
        if self.keys.plain_typing().contains(&a) {
            return Some("Needs Ctrl or Alt: a plain key would type. Choose another.".into());
        }
        let all = self.keys.conflicts();
        let (_, acts) = all.iter().find(|(c, acts)| acts.contains(&a) && self.keys.chords(a).first() == Some(c))?;
        let other = acts.iter().find(|x| **x != a)?;
        Some(format!("Already used by {}. Choose another.", other.label()))
    }

    /// The problem with while-recording chord `a`, if any: the same chord
    /// as another of them, or as a global hotkey (the global keeps it), or a
    /// bare key that every app would lose while recording.
    pub fn rec_note(&self, a: Action) -> Option<String> {
        if let Some(n) = self.conflict_note(a) {
            return Some(n);
        }
        let c = *self.keys.chords(a).first()?;
        if let Some((label, _)) = GLOBALS.iter().find(|(_, s)| self.global(*s) == Some(c)) {
            return Some(format!("Already used by {label}. Choose another."));
        }
        let modded = c.ctrl || c.alt || c.shift || c.meta;
        let special = (0x70..=0x7B).contains(&c.vk) || c.vk == wind::key::PRINTSCREEN;
        (!(modded || special)).then(|| BARE_NOTE.to_string())
    }

    /// The problem with global hotkey `slot`, if any: the same chord as
    /// another global (the one listed first keeps it in the message), or a
    /// bare key that every app would lose.
    pub fn global_note(&self, slot: usize) -> Option<String> {
        let c = self.global(slot)?;
        if let Some((label, _)) = GLOBALS.iter().find(|(_, s)| *s != slot && self.global(*s) == Some(c)) {
            return Some(format!("Already used by {label}. Choose another."));
        }
        let modded = c.ctrl || c.alt || c.shift || c.meta;
        let special = (0x70..=0x7B).contains(&c.vk) || c.vk == wind::key::PRINTSCREEN; // F1-F12
        (!(modded || special)).then(|| BARE_NOTE.to_string())
    }
}

#[cfg(windows)]
const BARE_NOTE: &str = "Without Ctrl, Alt, Shift or Win it takes that key from every app.";
#[cfg(target_os = "macos")]
const BARE_NOTE: &str = "Without Ctrl, Alt, Shift or Cmd it takes that key from every app.";
#[cfg(not(any(windows, target_os = "macos")))]
const BARE_NOTE: &str = "Without Ctrl, Alt, Shift or Super it takes that key from every app.";

/// The folder text, looked at without touching the disk.
#[derive(Clone, Debug, PartialEq)]
enum FolderKind {
    /// Nothing to say (empty, or a network share: never probed).
    Fine,
    Note(String),
    /// An absolute folder: its drive (Windows) or top-level folder must
    /// be probed ([`probe_root`]).
    Root(PathBuf),
}

fn folder_kind(t: &str) -> FolderKind {
    use std::path::Component;
    if t.is_empty() {
        return FolderKind::Fine;
    }
    let p = Path::new(t);
    if !p.has_root() {
        return FolderKind::Note("A relative folder is saved under the folder Rustshot was started in.".into());
    }
    if p.is_relative() {
        // Windows: "\shots" has a root but no drive.
        return FolderKind::Note("A folder without a drive is saved relative to the root of the current drive.".into());
    }
    // The drive (Windows) or the top-level folder (elsewhere: /media, /mnt).
    let mut root = PathBuf::new();
    for c in p.components() {
        if let Component::Prefix(x) = c
            && matches!(x.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..))
        {
            return FolderKind::Fine; // a network share: not probed (it can take seconds)
        }
        root.push(c);
        if cfg!(windows) && matches!(c, Component::RootDir) || matches!(c, Component::Normal(_)) {
            break;
        }
    }
    FolderKind::Root(root)
}

/// Whether `root` (from [`folder_kind`]) is there. A mapped network drive
/// is not probed (that can block for seconds).
fn probe_root(root: &Path) -> Option<String> {
    #[cfg(windows)]
    if remote_drive(root) {
        return None;
    }
    (!root.exists()).then(|| {
        let name = root.display().to_string();
        if cfg!(windows) { format!("Drive {} is not on this computer.", name.trim_end_matches('\\')) } else { format!("{name} does not exist.") }
    })
}

/// `GetDriveTypeW(root) == DRIVE_REMOTE`: a mapped network drive.
#[cfg(windows)]
fn remote_drive(root: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows::core::PCWSTR;
    const DRIVE_REMOTE: u32 = 4;
    let mut w: Vec<u16> = root.as_os_str().to_string_lossy().encode_utf16().collect();
    if w.last() != Some(&(b'\\' as u16)) {
        w.push(b'\\' as u16);
    }
    w.push(0);
    unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) == DRIVE_REMOTE }
}

/// A word about the folder that does not stop the save: a relative
/// path, or a drive (root folder) this computer does not have.
#[cfg(test)]
fn folder_note(t: &str) -> Option<String> {
    match folder_kind(t) {
        FolderKind::Fine => None,
        FolderKind::Note(n) => Some(n),
        FolderKind::Root(r) => probe_root(&r),
    }
}

/// The pattern field token buttons insert into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    File,
    Sub,
}

/// A question over the form.
#[derive(Clone, Debug, PartialEq)]
enum Modal {
    /// [`REWRITE`]; `close`: OK (not Apply) asked to save.
    Rewrite { close: bool },
    /// A save failed, or a message for the user (a hotkey the daemon
    /// could not register).
    Error(String),
    /// Hotkeys the daemon could not register as saved (or that fell
    /// back): OK, and "Change shortcuts…", which goes to Key bindings
    /// with `field`'s key box recording (the first row needing a new key).
    Hotkeys { text: String, field: Option<String> },
    /// [`HIDE_TRAY`]: "Show icon in tray" was just turned off.
    HideTray,
    /// [`TRAY_OFF_NOTICE`]: Set up Win+Shift+S / Show tray icon again / OK.
    TrayOffNotice,
    /// `config.toml` does not parse (the error): Open config file, Reset
    /// to defaults…, Close; nothing else until it is fixed.
    Broken(String),
    /// [`RESET`] over [`Modal::Broken`] (its error, to go back to).
    Reset(String),
    /// How to make Windows open Rustshot for Win+Shift+S
    /// ([`crate::screenclip_win::GUIDE_TITLE`]); `broken`: the choice
    /// names an app that is gone; `dont_show`: "Don't show again" ticked.
    Guide { broken: bool, dont_show: bool },
}

/// What the user did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    Ok,
    Cancel,
    Apply,
    /// The rewrite question's Save.
    Save,
    /// The rewrite question's Cancel, or the error's OK.
    Back,
    /// The hotkey dialog's "Change shortcuts…".
    ChangeShortcuts,
    /// The tray-icon-off question's "Turn off" / "Cancel".
    HideTray,
    KeepTray,
    /// Its "Set up…": the Win+Shift+S setup ([`Settings::screenclip_setup`]).
    ScreenclipSetup,
    /// The tray-off notice's "Show tray icon again" (saved, the daemon
    /// starts) and "OK" (the window closes).
    ShowTrayAgain,
    NoticeOk,
    Browse,
    OpenConfig,
    Restore,
    Clear,
    /// Key bindings: "Reset to defaults" (every key and global hotkey).
    ResetAll,
    /// "Reset tool sizes & colors": the tools' remembered settings go
    /// (from the file, at once).
    ResetTools,
    /// Capture › History: "Clear history…".
    ClearHistory,
    /// The broken file's "Reset to defaults…".
    ResetDefaults,
    /// The reset question's Replace.
    Replace,
    /// The broken file's Close.
    Close,
    /// About: "Check for updates".
    CheckUpdates,
    /// About: one of [`crate::actions::LINKS`].
    Link(&'static str),
    /// Upload: "Test connection" (the form's bucket, on a worker).
    TestUpload,
    /// The Win+Shift+S guide's "Open Default apps".
    OpenDefaultApps,
    /// The guide's "Remind me later".
    RemindLater,
    /// The guide's "Don't show again" checkbox.
    DontShow,
    /// Capture: "Set to a missing app — click to fix" (opens the guide).
    FixScreenclip,
    /// Esc not taken by a control.
    Escape,
    /// Enter not taken by a control.
    Enter,
}

/// What the window must do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Close,
    /// The config was written: the daemon reloads it.
    Saved(Box<Config>),
    /// Show the folder picker.
    Browse,
    OpenConfig,
    /// Nothing to save, but a hotkey is not registered: the daemon tries
    /// again (it reloads the config).
    Reload,
    /// The update dialog, checking now.
    CheckUpdates,
    /// History's "Clear history?" (in the History window).
    ClearHistory,
    /// Open this About link in the browser.
    Link(&'static str),
    /// "Show icon in tray" was turned on again: start the daemon unless
    /// one runs (Windows: with it off none did).
    StartDaemon,
    /// Open Windows' Default apps (the guide's button).
    OpenDefaultApps,
}

/// The window's state: the form, what it was loaded from, the section and
/// the open question.
pub struct Settings {
    /// The config as last loaded or saved.
    loaded: Config,
    /// Start at login as it is now (`None`: hidden).
    autostart: Option<bool>,
    /// The Win+Shift+S handler as registered now (`None`: hidden).
    screenclip: Option<bool>,
    /// Which app Win+Shift+S opens, as last read (the row's line under
    /// the toggle; read again when the window gets the focus back).
    pub handler: Handler,
    /// The guide's Open Default apps was clicked: when the window gets
    /// the focus back and Windows now opens Rustshot, a toast says so.
    guide_away: bool,
    /// The guide waits for the open question to close.
    guide_queued: bool,
    /// Default apps did not open: the guide says so (in red).
    pub guide_error: Option<String>,
    /// A short message at the bottom of the window, and when it came.
    pub toast: Option<(String, std::time::Instant)>,
    pub form: Form,
    /// The rail's section ([`SECTIONS`] index).
    pub section: usize,
    /// Each section's content height as last drawn (logical px; it fits
    /// the body, nothing scrolls).
    content_h: [f32; SECTIONS.len()],
    modal: Option<Modal>,
    /// The rewrite question was answered (or the file is ours now).
    confirmed: bool,
    /// Token buttons insert here.
    pattern: Pattern,
    /// The config file.
    path: PathBuf,
    /// Show the renderer choice (Windows).
    renderer: bool,
    /// Show "Show icon in tray" (Windows: the only tray so far).
    tray_row: bool,
    /// The form was not made from the file (it did not parse at open):
    /// rebuilt from it once it does.
    stale: bool,
    /// Messages waiting for the open question to close.
    pending: Vec<String>,
    /// Some of them are hotkey notes: the dialog they make has "Change
    /// shortcuts…" (to this key box, when one needs a new key).
    pending_keys: Option<Option<String>>,
    /// The key box to start recording in at the next frame of its
    /// shortcut section ("Change shortcuts…").
    pub record_field: Option<String>,
    /// A control to focus (with the ring) at the next
    /// frame of its section.
    pub focus_field: Option<String>,
    /// The icon off means no background process here
    /// ([`crate::tray::hidden_means_no_daemon`]; not the Store build).
    pub no_background: bool,
    /// The drive (top-level folder) last probed and its note: probed
    /// again only when the root changes, never while typing in it.
    folder_probe: Option<(PathBuf, Option<String>)>,
    /// The hotkeys the daemon could not register as saved
    /// ([`hotkey::outstanding`]).
    pub hotkey_failures: Vec<hotkey::Failure>,
    /// The tools' sizes and colors were reset (a note says so).
    tools_reset: bool,
    /// The Recording section's device lists and encoder status.
    rec_lists: recording::Lists,
    /// The Upload section's stored-secret check and Test connection.
    pub upload: upload::State,
    set_autostart: fn(bool) -> Result<(), String>,
    set_screenclip: fn(bool) -> Result<(), String>,
    read_handler: fn() -> Handler,
}

/// How long the toast shows.
pub const TOAST_MS: u64 = 2500;

impl Settings {
    pub fn new(cfg: Config, autostart: Option<bool>, path: PathBuf) -> Settings {
        Settings {
            form: Form::new(&cfg, autostart),
            loaded: cfg,
            autostart,
            screenclip: None,
            handler: Handler::None,
            guide_away: false,
            guide_queued: false,
            guide_error: None,
            toast: None,
            section: GENERAL,
            content_h: [0.0; SECTIONS.len()],
            modal: None,
            confirmed: false,
            pattern: Pattern::File,
            path,
            renderer: cfg!(windows),
            tray_row: cfg!(windows),
            stale: false,
            pending: Vec::new(),
            pending_keys: None,
            record_field: None,
            focus_field: None,
            no_background: crate::tray::hidden_means_no_daemon(),
            folder_probe: None,
            hotkey_failures: Vec::new(),
            tools_reset: false,
            rec_lists: recording::Lists::default(),
            upload: upload::State::default(),
            set_autostart: crate::autostart::set,
            set_screenclip: crate::screenclip_win::set,
            read_handler: crate::screenclip_win::handler,
        }
    }

    /// Show the Win+Shift+S toggle at `state` (as the registry has it;
    /// `None` hides it); `handler`: which app Win+Shift+S opens now.
    pub fn with_screenclip(mut self, state: Option<bool>, handler: Handler) -> Settings {
        self.screenclip = state;
        self.form.screenclip = state;
        self.handler = handler;
        self
    }

    /// Show the Win+Shift+S guide (now, or once the open question
    /// closes); nothing when Windows already opens Rustshot or the toggle
    /// is not on.
    pub fn open_guide(&mut self) {
        if self.screenclip != Some(true) {
            return;
        }
        self.handler = (self.read_handler)();
        if self.handler == Handler::Ours {
            return;
        }
        let broken = matches!(self.handler, Handler::Broken { .. });
        match &self.modal {
            None => {
                self.modal = Some(Modal::Guide { broken, dont_show: false });
                self.guide_error = None;
            }
            Some(Modal::Guide { .. }) => {}
            Some(_) => self.guide_queued = true,
        }
    }

    /// The guide's text: its title and body.
    pub fn guide_texts(&self, broken: bool) -> (&'static str, String) {
        (crate::screenclip_win::GUIDE_TITLE, crate::screenclip_win::guide_text(broken))
    }

    /// The registration as it is now (another flow may have turned it on:
    /// [`crate::screenclip_win::show_setup_guide`]); the form's toggle
    /// follows unless the user changed it.
    pub fn sync_screenclip(&mut self, state: Option<bool>) {
        if self.form.screenclip == self.screenclip {
            self.form.screenclip = state;
        }
        self.screenclip = state;
    }

    /// The red line's click: turn the registration on (as Apply would)
    /// when it is off, then the guide.
    fn fix_screenclip(&mut self) {
        if self.screenclip == Some(false) {
            match (self.set_screenclip)(true) {
                Ok(()) => {
                    self.screenclip = Some(true);
                    self.form.screenclip = Some(true);
                }
                Err(e) => {
                    self.modal = Some(Modal::Error(format!("Could not change Win+Shift+S: {e}")));
                    return;
                }
            }
        }
        self.open_guide();
    }

    /// Default apps did not open (`text`: [`crate::screenclip_win::open_failed_text`]):
    /// the guide shows it in red, and nothing waits for the user to come back.
    pub fn default_apps_failed(&mut self, text: String) {
        self.guide_away = false;
        if matches!(self.modal, Some(Modal::Guide { .. })) {
            self.guide_error = Some(text);
        } else {
            self.notify(text);
        }
    }

    /// Close the guide however it closes (Remind me later, Esc, the
    /// window's close button, or Rustshot chosen); a ticked "Don't show
    /// again" writes `screenclip_hint = false` (that line only). A write
    /// that fails leaves the error showing.
    fn close_guide(&mut self) -> Vec<Act> {
        let Some(Modal::Guide { dont_show, .. }) = self.modal else { return vec![] };
        self.modal = None;
        self.guide_away = false;
        self.guide_error = None;
        if dont_show {
            let edit = [("screenclip_hint".to_string(), Some("false".to_string()))];
            match config::edit_top_keys_at(&self.path, &edit) {
                Ok(()) => self.loaded.screenclip_hint = false,
                Err(e) => {
                    self.modal = Some(Modal::Error(format!("Could not save {}: {e}", self.path.display())));
                    return vec![];
                }
            }
        }
        self.show_pending();
        vec![]
    }

    /// The window has the focus again (back from Default apps): while the
    /// guide is up or its Open Default apps was clicked, read which app
    /// Win+Shift+S opens; once it is Rustshot the guide closes and a toast
    /// says so. Other focus changes read nothing (the read may load a
    /// package's resources). Returns whether anything changed.
    pub fn refocused(&mut self) -> bool {
        let guide = matches!(self.modal, Some(Modal::Guide { .. }));
        if self.screenclip != Some(true) || !(guide || self.guide_away) {
            return false;
        }
        let was = std::mem::replace(&mut self.handler, (self.read_handler)());
        if self.handler == Handler::Ours && was != Handler::Ours {
            if matches!(self.modal, Some(Modal::Guide { .. })) {
                self.close_guide();
            }
            self.guide_away = false;
            self.guide_queued = false;
            self.toast = Some((crate::screenclip_win::NOW_OURS.into(), std::time::Instant::now()));
            return true;
        }
        let broken_now = matches!(self.handler, Handler::Broken { .. });
        if let Some(Modal::Guide { broken, .. }) = &mut self.modal {
            *broken = broken_now;
        }
        self.handler != was
    }

    /// The toast is showing (the window animates until it goes).
    pub fn toast_live(&mut self) -> bool {
        let live = self.toast.as_ref().is_some_and(|(_, at)| at.elapsed() < std::time::Duration::from_millis(TOAST_MS));
        if !live {
            self.toast = None;
        }
        live
    }

    /// The form for `cfg`, keeping the toggles that are not in the file.
    fn form_for(&self, cfg: &Config) -> Form {
        Form { screenclip: self.form.screenclip, ..Form::new(cfg, self.form.autostart) }
    }

    /// The window for the config file at `path` as it is now. A file that
    /// does not parse opens in the error state ([`Modal::Broken`]).
    pub fn open(path: PathBuf, autostart: Option<bool>) -> Settings {
        match config::read_at(&path) {
            Ok(cfg) => Settings::new(cfg.unwrap_or_default(), autostart, path),
            Err(e) => {
                let mut s = Settings::new(Config::default(), autostart, path);
                s.stale = true;
                s.modal = Some(Modal::Broken(e));
                s
            }
        }
    }

    /// The theme the window shows: the form's theme and accent (so they
    /// preview before Apply).
    pub fn preview_theme(&self) -> Theme {
        crate::theme::resolve(&self.form.apply(&self.loaded, &self.loaded))
    }

    /// Showing the broken-file error (or its reset question).
    pub fn broken(&self) -> bool {
        matches!(self.modal, Some(Modal::Broken(_) | Modal::Reset(_)))
    }

    /// While the broken-file error shows: read the file again; once it
    /// parses the window works again (a form made before it broke is
    /// kept, one that never saw it is rebuilt). Returns whether anything
    /// changed.
    pub fn recheck(&mut self) -> bool {
        let Some(Modal::Broken(old)) = &self.modal else { return false };
        match config::read_at(&self.path) {
            Ok(cfg) => {
                if self.stale {
                    let cfg = cfg.unwrap_or_default();
                    self.form = self.form_for(&cfg);
                    self.loaded = cfg;
                    self.stale = false;
                }
                self.modal = None;
                self.show_pending();
                true
            }
            Err(e) if e != *old => {
                self.modal = Some(Modal::Broken(e));
                true
            }
            Err(_) => false,
        }
    }

    /// Show `msg` (e.g. a hotkey the daemon could not register) once no
    /// other question is open.
    pub fn notify(&mut self, msg: String) {
        match &mut self.modal {
            None => self.modal = Some(Modal::Error(msg)),
            Some(Modal::Error(e)) => {
                e.push('\n');
                e.push_str(&msg);
            }
            Some(Modal::Hotkeys { text, .. }) => {
                text.push('\n');
                text.push_str(&msg);
            }
            Some(_) => self.pending.push(msg),
        }
    }

    /// Show the hotkey note `msg` ([`Modal::Hotkeys`]; `field`: the key
    /// box of the first row that needs a new key) once no other question
    /// is open. Joins an error or hotkey dialog already showing.
    pub fn notify_hotkeys(&mut self, msg: String, field: Option<String>) {
        match &mut self.modal {
            None => self.modal = Some(Modal::Hotkeys { text: msg, field }),
            Some(Modal::Error(e)) => {
                let text = format!("{e}\n{msg}");
                self.modal = Some(Modal::Hotkeys { text, field });
            }
            Some(Modal::Hotkeys { text, field: f }) => {
                text.push('\n');
                text.push_str(&msg);
                if f.is_none() {
                    *f = field;
                }
            }
            Some(_) => {
                self.pending.push(msg);
                let f = self.pending_keys.take().flatten();
                self.pending_keys = Some(f.or(field));
            }
        }
    }

    fn show_pending(&mut self) {
        if self.modal.is_none() && !self.pending.is_empty() {
            let text = std::mem::take(&mut self.pending).join("\n");
            self.modal = Some(match self.pending_keys.take() {
                Some(field) => Modal::Hotkeys { text, field },
                None => Modal::Error(text),
            });
        }
        if self.modal.is_none() && std::mem::take(&mut self.guide_queued) {
            self.open_guide();
        }
    }

    /// "Show icon in tray" was just turned off: ask first, saying how to
    /// get back in ([`HIDE_TRAY`]).
    pub fn ask_hide_tray(&mut self) {
        if self.modal.is_none() {
            self.modal = Some(Modal::HideTray);
        }
    }

    /// Win+Shift+S / Print Screen open Rustshot: registered and chosen.
    /// (`None` where it cannot be: nothing to set up.)
    pub fn screenclip_ready(&self) -> Option<bool> {
        self.screenclip.map(|on| on && self.handler == Handler::Ours)
    }

    /// The tray-icon-off question: [`HIDE_TRAY`], and while Win+Shift+S
    /// does not open Rustshot, [`HIDE_TRAY_SETUP`].
    pub fn hide_tray_text(&self) -> String {
        if !self.no_background {
            return HIDE_TRAY_KEEP.to_string();
        }
        match self.screenclip_ready() {
            Some(false) => format!("{HIDE_TRAY}\n{HIDE_TRAY_SETUP}"),
            _ => HIDE_TRAY.to_string(),
        }
    }

    /// "Set up…": the icon stays for now; Capture opens at "Use
    /// Win+Shift+S and Print Screen", which is turned on if it is off, and
    /// the Win+Shift+S guide shows over it (what
    /// [`crate::screenclip_win::show_setup_guide`] does from outside).
    pub fn screenclip_setup(&mut self) {
        if matches!(self.modal, Some(Modal::HideTray | Modal::TrayOffNotice)) {
            self.modal = None;
        }
        self.form.show_tray = true;
        self.section = CAPTURE;
        self.focus_field = Some("screenclip".into());
        if self.screenclip == Some(false) {
            match (self.set_screenclip)(true) {
                Ok(()) => {
                    self.screenclip = Some(true);
                    self.form.screenclip = Some(true);
                }
                Err(e) => {
                    self.modal = Some(Modal::Error(format!("Could not change Win+Shift+S: {e}")));
                    return;
                }
            }
        }
        self.open_guide();
        self.show_pending();
    }

    /// The "Show icon in tray" row's help.
    pub fn tray_help(&self) -> &'static str {
        if self.no_background { TRAY_HELP } else { TRAY_HELP_KEEP }
    }

    /// The one-time tray-off notice ([`run_tray_off_notice`]).
    pub fn show_tray_off_notice(&mut self) {
        self.modal = Some(Modal::TrayOffNotice);
    }

    /// The dialog's "Change shortcuts…": the shortcut section that lists
    /// the row ([`section_of_field`]), its key box recording.
    fn change_shortcuts(&mut self) {
        if let Some(Modal::Hotkeys { field, .. }) = self.modal.take() {
            self.section = field.as_deref().map_or(KEYS_GLOBAL, section_of_field);
            self.record_field = field;
        }
        self.show_pending();
    }

    /// The file on disk now, if it parses: `Ok(None)` when there is none.
    /// When it does not, the window switches to the error state.
    fn disk(&mut self) -> Result<Option<Config>, ()> {
        config::read_at(&self.path).map_err(|e| {
            self.modal = Some(Modal::Broken(e));
        })
    }

    /// The folder's note ([`folder_kind`]); the drive is probed once per
    /// root.
    fn folder_note(&mut self) -> Option<String> {
        match folder_kind(self.form.folder.text.trim()) {
            FolderKind::Fine => None,
            FolderKind::Note(n) => Some(n),
            FolderKind::Root(r) => {
                if let Some((root, note)) = &self.folder_probe
                    && *root == r
                {
                    return note.clone();
                }
                let note = probe_root(&r);
                self.folder_probe = Some((r, note.clone()));
                note
            }
        }
    }

    /// A hotkey is not registered as saved: Apply and OK ask the daemon to
    /// try again even with nothing to save.
    pub fn retry_hotkeys(&self) -> bool {
        !self.hotkey_failures.is_empty()
    }

    /// The daemon's failure for hotkey `slot`, while the form still has
    /// the saved chord (a new key recorded in its place hides it).
    fn slot_failure(&self, slot: usize) -> Option<&hotkey::Failure> {
        let f = self.hotkey_failures.iter().find(|f| f.slot == slot)?;
        let saved = hotkey::wanted(&self.loaded).get(slot).and_then(|s| Chord::parse(s));
        (slot >= hotkey::SLOTS || self.form.global(slot) == saved).then_some(f)
    }

    /// The note under hotkey `slot`'s row while it is not registered as
    /// saved: another app holds it ("In use by another app — using
    /// Win+Shift+Y for now" after a fallback, "… — using Ctrl+Alt+Shift+Q"
    /// with the chord kept, "In use by another app. Win+Shift+Y is free."),
    /// else "Not registered — using …" or "Not registered".
    pub fn hotkey_slot_note(&self, slot: usize) -> Option<String> {
        let f = self.slot_failure(slot)?;
        let label = hotkey::chord_label;
        Some(match (&f.alt, &f.kept, f.cause) {
            (Some(a), _, _) => format!("{IN_USE} \u{2014} using {} for now", label(a)),
            (None, Some(k), hotkey::Cause::InUse) => format!("{IN_USE} \u{2014} using {}", label(k)),
            (None, Some(k), _) => format!("Not registered \u{2014} using {}", label(k)),
            (None, None, hotkey::Cause::InUse) => match &f.free {
                Some(free) => format!("{IN_USE}. {} is free.", label(free)),
                None => format!("{IN_USE}. Choose another."),
            },
            (None, None, _) => "Not registered".into(),
        })
    }

    /// Hotkey `slot`'s keys show red: another global has the chord, or
    /// another app holds it.
    pub fn global_err(&self, slot: usize) -> bool {
        self.form.global_note(slot).is_some_and(|n| n.starts_with("Already"))
            || self.slot_failure(slot).is_some_and(|f| f.cause == hotkey::Cause::InUse)
    }

    /// The message under global hotkey `slot`: a clash with another
    /// global first, then a failed registration, then a bare key.
    pub fn global_message(&self, slot: usize) -> Option<String> {
        let note = self.form.global_note(slot);
        match note {
            Some(n) if n.starts_with("Already") => Some(n),
            n => self.hotkey_slot_note(slot).or(n),
        }
    }

    /// "Show icon in tray" is off and nothing can start a capture but a
    /// second launch: no capture hotkey, or one that did not register.
    pub fn tray_warning(&self) -> bool {
        !self.form.show_tray && (self.form.capture.is_none() || self.hotkey_failures.iter().any(|f| f.slot == 0 && f.kept.is_none()))
    }

    /// Something differs from what is saved.
    pub fn dirty(&self) -> bool {
        let now = self.form.apply(&self.loaded, &self.loaded);
        config::to_toml(&now) != config::to_toml(&self.loaded)
            || !self.form.up.secret.text.is_empty()
            || self.form.autostart != self.autostart
            || self.form.screenclip != self.screenclip
    }

    /// The file has content a rewrite would drop and nobody said yes yet.
    fn needs_confirm(&self) -> bool {
        !self.confirmed && std::fs::read_to_string(&self.path).is_ok_and(|t| config::rewrite_loses(&t))
    }

    pub fn on(&mut self, c: Click) -> Vec<Act> {
        let c = match (c, &self.modal) {
            (Click::Escape, Some(Modal::Broken(_))) => Click::Close,
            (Click::Escape, Some(Modal::Guide { .. })) => Click::RemindLater,
            (Click::Enter, Some(Modal::Guide { .. })) => Click::OpenDefaultApps,
            (Click::Cancel, Some(Modal::Guide { .. })) => {
                let mut acts = self.close_guide();
                // "Don't show again" could not be saved: the error stays.
                if !matches!(self.modal, Some(Modal::Error(_))) {
                    acts.push(Act::Close);
                }
                return acts;
            }
            (Click::Escape, Some(_)) => Click::Back,
            (Click::Escape, None) => Click::Cancel,
            (Click::Enter, Some(Modal::Rewrite { .. })) => Click::Save,
            (Click::Enter, Some(Modal::HideTray)) => Click::HideTray,
            (Click::Enter, Some(Modal::TrayOffNotice)) => Click::NoticeOk,
            (Click::Enter, Some(Modal::Error(_) | Modal::Hotkeys { .. })) => Click::Back,
            // Nothing happens to a broken file by a stray Enter.
            (Click::Enter, Some(Modal::Broken(_) | Modal::Reset(_))) => return vec![],
            (Click::Enter, None) => Click::Ok,
            // While the file is broken only its own buttons work.
            (Click::OpenConfig | Click::ResetDefaults | Click::Close | Click::Cancel, Some(Modal::Broken(_))) => c,
            (Click::Replace | Click::Back, Some(Modal::Reset(_))) => c,
            (Click::Cancel, Some(Modal::Reset(_))) => Click::Close, // the window's close button
            (_, Some(Modal::Broken(_) | Modal::Reset(_))) => return vec![],
            (c, _) => c,
        };
        match c {
            Click::Ok if !self.dirty() && self.retry_hotkeys() => vec![Act::Reload, Act::Close],
            Click::Ok if !self.dirty() => vec![Act::Close],
            Click::Ok => self.save(true),
            Click::Apply if self.dirty() => self.save(false),
            Click::Apply if self.retry_hotkeys() => vec![Act::Reload],
            Click::Apply => vec![],
            Click::Cancel => vec![Act::Close],
            Click::Save => {
                let close = matches!(self.modal, Some(Modal::Rewrite { close: true }));
                self.confirmed = true;
                self.modal = None;
                self.commit(close)
            }
            Click::Back => {
                self.modal = match self.modal.take() {
                    Some(Modal::Reset(e)) => Some(Modal::Broken(e)),
                    // Esc on the hide-the-icon question: it stays.
                    Some(Modal::HideTray) => {
                        self.form.show_tray = true;
                        None
                    }
                    _ => None,
                };
                self.show_pending();
                vec![]
            }
            Click::ResetDefaults => {
                if let Some(Modal::Broken(e)) = &self.modal {
                    self.modal = Some(Modal::Reset(e.clone()));
                }
                vec![]
            }
            Click::Replace => self.reset(),
            Click::ScreenclipSetup => {
                self.screenclip_setup();
                vec![]
            }
            Click::ShowTrayAgain => {
                self.modal = None;
                self.form.show_tray = true;
                self.save(true)
            }
            Click::NoticeOk => {
                self.modal = None;
                vec![Act::Close]
            }
            Click::HideTray | Click::KeepTray => {
                if matches!(self.modal, Some(Modal::HideTray)) {
                    self.modal = None;
                    if c == Click::KeepTray {
                        self.form.show_tray = true;
                    }
                }
                self.show_pending();
                vec![]
            }
            Click::ChangeShortcuts => {
                self.change_shortcuts();
                vec![]
            }
            Click::Close => vec![Act::Close],
            Click::Browse => vec![Act::Browse],
            Click::OpenConfig => vec![Act::OpenConfig],
            Click::CheckUpdates => vec![Act::CheckUpdates],
            Click::Link(u) => vec![Act::Link(u)],
            Click::Restore => {
                self.form.filename.set(DEFAULT_FILENAME);
                vec![]
            }
            Click::Clear => {
                self.form.filename.set("");
                vec![]
            }
            Click::ResetAll => {
                self.form.reset_keys();
                vec![]
            }
            Click::ResetTools => self.reset_tools(),
            Click::ClearHistory => vec![Act::ClearHistory],
            Click::TestUpload => {
                self.upload.start_test(&self.form.up);
                vec![]
            }
            Click::OpenDefaultApps => {
                // The guide stays: its steps are still needed over there.
                self.guide_away = true;
                vec![Act::OpenDefaultApps]
            }
            Click::RemindLater => self.close_guide(),
            Click::DontShow => {
                if let Some(Modal::Guide { dont_show, .. }) = &mut self.modal {
                    *dont_show = !*dont_show;
                }
                vec![]
            }
            Click::FixScreenclip => {
                self.fix_screenclip();
                vec![]
            }
            Click::Escape | Click::Enter => vec![],
        }
    }

    fn save(&mut self, close: bool) -> Vec<Act> {
        if self.disk().is_err() {
            return vec![]; // never merged onto the defaults
        }
        if self.needs_confirm() {
            self.modal = Some(Modal::Rewrite { close });
            return vec![];
        }
        self.commit(close)
    }

    /// Write the changed values onto the file as it is now.
    fn commit(&mut self, close: bool) -> Vec<Act> {
        // The file as it is now (it may have changed, or broken, since),
        // read and written under the config lock: nothing (the overlay's
        // remembered sizes, another process) writes in between.
        let path = self.path.clone();
        let saved = config::with_config_lock(&path, || {
            let onto = self.disk().ok()?.unwrap_or_else(|| self.loaded.clone());
            let mut cfg = self.form.apply(&self.loaded, &onto);
            // Turned off here: the user saw what it means now.
            if !cfg.show_tray && self.no_background {
                cfg.tray_off_mode_seen = true;
            }
            Some((config::save_at(&path, &cfg), cfg))
        });
        let Some((res, cfg)) = saved else { return vec![] };
        if let Err(e) = res {
            self.modal = Some(Modal::Error(format!("Could not save {}: {e}", self.path.display())));
            return vec![];
        }
        self.confirmed = true; // the file is ours now
        let tray_on = cfg.show_tray && !self.loaded.show_tray;
        self.loaded = cfg.clone();
        // The typed S3 secret goes to the OS credential store, never the file.
        let secret = std::mem::take(&mut self.form.up.secret.text);
        // The form shows what was saved (merged with the file, trimmed).
        self.form = self.form_for(&cfg);
        let mut acts = vec![Act::Saved(Box::new(cfg.clone()))];
        if tray_on && self.no_background {
            acts.push(Act::StartDaemon);
        }
        // Kept on a worker (a locked keyring may ask for its password);
        // OK closes the window once it is kept (`poll_upload`).
        let mut close = close;
        if !secret.trim().is_empty() {
            match upload::slot_of(&crate::s3::Target::from_config(&cfg), &cfg.s3_access_key_id) {
                Some(slot) => {
                    self.upload.save(slot, secret, close);
                    close = false;
                }
                None => {
                    // Kept typed, to try again.
                    self.form.up.secret.set(&secret);
                    self.modal = Some(Modal::Error("Enter the bucket and the access key ID first: the secret access key is kept for them.".into()));
                    return acts;
                }
            }
        }
        if let (Some(on), true) = (self.form.autostart, self.form.autostart != self.autostart) {
            match (self.set_autostart)(on) {
                Ok(()) => self.autostart = Some(on),
                Err(e) => {
                    self.modal = Some(Modal::Error(format!("Could not change Launch at login: {e}")));
                    return acts;
                }
            }
        }
        if let (Some(on), true) = (self.form.screenclip, self.form.screenclip != self.screenclip) {
            match (self.set_screenclip)(on) {
                Ok(()) => {
                    self.screenclip = Some(on);
                    self.handler = (self.read_handler)();
                    // Turned on: Windows still has to be told, by the
                    // user. The guide shows (and OK leaves the window
                    // open for it).
                    if on {
                        self.open_guide();
                        if matches!(self.modal, Some(Modal::Guide { .. })) {
                            return acts;
                        }
                    }
                }
                Err(e) => {
                    self.modal = Some(Modal::Error(format!("Could not change Win+Shift+S: {e}")));
                    return acts;
                }
            }
        }
        if close {
            acts.push(Act::Close);
        }
        acts
    }

    /// The Upload section's workers answered: whether to repaint, and what
    /// the window must do (close once OK's secret is kept). A secret that
    /// could not be kept goes back into its field, with the reason.
    pub fn poll_upload(&mut self) -> (bool, Vec<Act>) {
        let p = self.upload.poll();
        let mut acts = Vec::new();
        match p.saved {
            Some((Ok(()), _, close)) if close => acts.push(Act::Close),
            Some((Err(e), secret, _)) => {
                self.form.up.secret.set(&secret);
                self.notify(e);
            }
            _ => {}
        }
        (p.changed, acts)
    }

    /// Remove every `tool_<id>_size` / `tool_<id>_color` from the file
    /// (only those lines; a file that does not parse is left alone and the
    /// window shows its error); the daemon reloads, so the next capture
    /// starts every tool from the defaults.
    fn reset_tools(&mut self) -> Vec<Act> {
        if !self.path.exists() {
            self.tools_reset = true;
            return vec![];
        }
        let edits: Vec<(String, Option<String>)> = config::tool_key_names().into_iter().map(|k| (k, None)).collect();
        match config::edit_top_keys_at(&self.path, &edits) {
            Ok(()) => {
                self.loaded.tool_sizes.clear();
                self.loaded.tool_colors.clear();
                self.loaded.tool_fills.clear();
                self.tools_reset = true;
                vec![Act::Reload]
            }
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                let _ = self.disk(); // the error state
                vec![]
            }
            Err(e) => {
                self.modal = Some(Modal::Error(format!("Could not reset the tools in {}: {e}", self.path.display())));
                vec![]
            }
        }
    }

    /// The reset question's Replace: read the file again (fixed
    /// meanwhile: back to the form, nothing written); else keep it as
    /// `config.toml.bak` (or the next free `.bak.N`), then write the
    /// defaults.
    fn reset(&mut self) -> Vec<Act> {
        let Some(Modal::Reset(e)) = &self.modal else { return vec![] };
        // `recheck` works from the error state.
        self.modal = Some(Modal::Broken(e.clone()));
        if self.recheck() && !self.broken() {
            return vec![];
        }
        let Some(Modal::Broken(e)) = &self.modal else { return vec![] };
        self.modal = Some(Modal::Reset(e.clone()));
        let bak = backup_path(&self.path);
        let cfg = Config::default();
        // The backup and the rewrite under the config lock.
        let path = self.path.clone();
        let res = config::with_config_lock(&path, || {
            if path.exists()
                && let Err(e) = std::fs::copy(&path, &bak)
            {
                return Err(format!("Could not back up config.toml to {}: {e}", bak.display()));
            }
            config::save_at(&path, &cfg).map_err(|e| format!("Could not save {}: {e}", path.display()))
        });
        if let Err(e) = res {
            self.modal = Some(Modal::Error(e));
            return vec![];
        }
        self.modal = None;
        self.confirmed = true;
        self.stale = false;
        self.form = self.form_for(&cfg);
        self.loaded = cfg.clone();
        self.show_pending();
        vec![Act::Saved(Box::new(cfg))]
    }
}

// --- Window -----------------------------------------------------------------

/// What the open window is told from other threads.
enum Msg {
    /// Come to the front (another request to open it).
    Raise,
    /// Show this to the user ([`notify`]).
    Note(String),
    /// A hotkey note ([`notify_hotkeys`]): the text and the key box to
    /// record in.
    Hotkeys(String, Option<String>),
    /// Go to this section ([`show_at`]).
    Section(usize),
    /// Show the Win+Shift+S guide ([`show_guide`]).
    Guide,
    /// The guide's Open Default apps did not open it (the text to show).
    DefaultAppsFailed(String),
}

/// The open window's inbox, if any.
static OPEN: Mutex<Option<Sender<Msg>>> = Mutex::new(None);
/// The section a window opened by [`show_at`] starts in.
static START_SECTION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(GENERAL);
/// The daemon's event sender: told to reload after a save.
static DAEMON: Mutex<Option<Sender<HotEvent>>> = Mutex::new(None);

/// Whether the window is open in this process (an on-demand run waits
/// for it before it quits).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn is_open() -> bool {
    lock(&OPEN).is_some()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Register the daemon's event sender (saves make it reload the config).
pub fn set_daemon(tx: Sender<HotEvent>) {
    *lock(&DAEMON) = Some(tx);
}

/// Ask the daemon (if any) to read the config again.
fn reload_daemon() {
    if let Some(d) = lock(&DAEMON).as_ref() {
        let _ = d.send(HotEvent::ReloadConfig);
    }
}

/// Start `rustshot daemon` beside this window's process unless one runs.
fn start_daemon() {
    #[cfg(windows)]
    if !crate::instance::daemon_running() {
        let r = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| crate::proc_win::spawn_detached(&exe, &[std::ffi::OsStr::new("daemon")], &[]));
        if let Err(e) = r {
            eprintln!("rustshot: could not start the daemon: {e}");
        }
    }
}

/// Another window changed `config.toml` (History's "Keep a history of
/// captures", the Edit window's "Don't ask again"): the daemon reloads.
pub fn tell_daemon_reload() {
    reload_daemon();
}

/// Open the window at `section` (History's "History settings" link goes
/// to [`HISTORY`]), or bring the open one there.
pub fn show_at(section: usize) {
    #[cfg(not(target_os = "macos"))]
    {
        let open = lock(&OPEN);
        if let Some(tx) = open.as_ref()
            && tx.send(Msg::Section(section)).is_ok()
        {
            let _ = tx.send(Msg::Raise);
            return;
        }
        START_SECTION.store(section, std::sync::atomic::Ordering::SeqCst);
    }
    let _ = section;
    show();
}

/// A window opened by [`show_guide`] starts on the guide.
static START_GUIDE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Open the window at Capture with the Win+Shift+S guide over it (the
/// daemon's reminder), or show it in the open one.
pub fn show_guide() {
    #[cfg(not(target_os = "macos"))]
    {
        let open = lock(&OPEN);
        if let Some(tx) = open.as_ref()
            && tx.send(Msg::Guide).is_ok()
        {
            let _ = tx.send(Msg::Raise);
            return;
        }
        START_GUIDE.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    show_at(CAPTURE);
}

/// Show `text` in the open Settings window (over the form, with OK);
/// false when no window is open (the caller tells the user some other way).
pub fn notify(text: &str) -> bool {
    lock(&OPEN).as_ref().is_some_and(|tx| tx.send(Msg::Note(text.to_string())).is_ok())
}

/// The hotkey dialog ([`Modal::Hotkeys`]) in the open Settings window;
/// false when none is open.
pub fn notify_hotkeys(text: &str, field: Option<String>) -> bool {
    lock(&OPEN).as_ref().is_some_and(|tx| tx.send(Msg::Hotkeys(text.to_string(), field.clone())).is_ok())
}

/// A hotkey note with keys to change: the hotkey dialog in the Settings
/// window, opened at the shortcut section with `field` when it is not
/// open. (macOS and tests: a tray note.)
pub fn show_hotkeys(text: &str, field: Option<String>) {
    if notify_hotkeys(text, field.clone()) {
        return;
    }
    if cfg!(test) || cfg!(target_os = "macos") {
        crate::tray::notify(text);
        return;
    }
    let section = field.as_deref().map_or(KEYS_GLOBAL, section_of_field);
    *lock(&START_NOTE) = Some((text.to_string(), field));
    show_at(section);
}

/// The hotkey note a window opened by [`show_hotkeys`] starts with.
static START_NOTE: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);

/// Open the window on its own thread, or bring the open one to the front.
pub fn show() {
    #[cfg(target_os = "macos")]
    std::thread::spawn(crate::actions::open_config); // never block the caller
    #[cfg(not(target_os = "macos"))]
    if let Some(rx) = claim() {
        std::thread::spawn(move || run(rx));
    }
}

/// A window opened by [`run_tray_off_notice`] starts with the notice.
static START_TRAY_NOTICE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The first start of this version with an older file's `show_tray =
/// false`: the window here with [`TRAY_OFF_NOTICE`] over it.
pub fn run_tray_off_notice() {
    START_TRAY_NOTICE.store(true, std::sync::atomic::Ordering::SeqCst);
    run_here();
}

/// `rustshot settings` with no daemon running: the window on this thread
/// (returns when it closes).
pub fn run_here() {
    #[cfg(target_os = "macos")]
    crate::actions::open_config();
    #[cfg(not(target_os = "macos"))]
    if let Some(rx) = claim() {
        run(rx);
    }
}

/// Take the single-window slot; `None` (and the open window raised) when
/// a window is already open.
fn claim() -> Option<Receiver<Msg>> {
    let mut open = lock(&OPEN);
    if let Some(tx) = open.as_ref()
        && tx.send(Msg::Raise).is_ok()
    {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    *open = Some(tx);
    Some(rx)
}

fn run(rx: Receiver<Msg>) {
    let autostart = crate::update::managed_install().is_none().then(crate::autostart::is_enabled);
    let mut s = Settings::open(config::config_path(), autostart)
        .with_screenclip(crate::screenclip_win::state(), crate::screenclip_win::handler());
    s.hotkey_failures = hotkey::outstanding();
    s.section = START_SECTION.swap(GENERAL, std::sync::atomic::Ordering::SeqCst).min(SECTIONS.len() - 1);
    if START_TRAY_NOTICE.swap(false, std::sync::atomic::Ordering::SeqCst) {
        s.show_tray_off_notice();
    } else if let Some((text, field)) = lock(&START_NOTE).take() {
        s.notify_hotkeys(text, field);
    } else if START_GUIDE.swap(false, std::sync::atomic::Ordering::SeqCst) {
        s.open_guide();
    }
    let th = crate::theme::resolve(&s.loaded);
    // Opened without a daemon (`rustshot settings`): the file's UI size.
    // In the daemon an overlay may have set it and still have the write
    // pending, so the file is older: `init` leaves a set size alone.
    if !s.stale {
        uisize::init(s.loaded.ui_scale);
    }
    let mut w = Window::new(s, th, rx);
    let spec = WindowSpec { title: "Rustshot Settings".into(), w: W, h: H, resizable: false, min: (W, H), controls: wind::Controls::MinClose };
    if let Err(e) = wind::run_window(spec, &mut w) {
        eprintln!("rustshot: settings window: {e:#}");
    }
    let mut open = lock(&OPEN);
    let mut late = false;
    let mut notes = Vec::new();
    for m in w.rx.try_iter() {
        match m {
            Msg::Raise => late = true,
            Msg::Note(t) | Msg::Hotkeys(t, _) => notes.push(t),
            Msg::Section(_) => late = true,
            Msg::Guide => {
                START_GUIDE.store(true, std::sync::atomic::Ordering::SeqCst);
                late = true;
            }
            Msg::DefaultAppsFailed(t) => notes.push(t),
        }
    }
    *open = None;
    drop(open);
    // Told while closing (OK saves, then closes): the tray says it instead.
    for t in notes {
        crate::tray::notify(&t);
    }
    if late {
        show(); // a request raced the close
    }
}

/// The `wind` driver: renders on every input event (the controls are
/// immediate-mode) and runs the [`Settings`] actions.
struct Window {
    hwnd: Hwnd,
    s: Settings,
    th: Theme,
    input: Input,
    focus: FocusState,
    canvas: PixBuf,
    out: PixBuf,
    /// Requests to come to the front, and messages.
    rx: Receiver<Msg>,
    /// When the broken file was last read again.
    rechecked: std::time::Instant,
    /// The folder picker's answer (it runs on its own thread).
    picked: (Sender<Option<PathBuf>>, Receiver<Option<PathBuf>>),
    picking: bool,
    closing: bool,
    /// The UI scale the window was sized for (0 = not known yet).
    sized_for: f32,
    /// What [`Window::report_focus`] last wrote.
    reported: Option<String>,
}

impl Window {
    fn new(s: Settings, th: Theme, rx: Receiver<Msg>) -> Window {
        Window {
            hwnd: Hwnd::default(),
            s,
            th,
            input: Input::default(),
            focus: FocusState::default(),
            canvas: PixBuf::default(),
            out: PixBuf::default(),
            rx,
            rechecked: std::time::Instant::now(),
            picked: std::sync::mpsc::channel(),
            picking: false,
            closing: false,
            sized_for: 0.0,
            reported: None,
        }
    }

    fn perform(&mut self, a: Act) {
        match a {
            Act::Close => self.closing = true,
            Act::Saved(cfg) => {
                self.th = crate::theme::resolve(&cfg);
                // A new UI size: the next frame sees the new scale and
                // resizes the window (`follow_scale`), the overlay lays
                // out at its next capture.
                uisize::set(cfg.ui_scale);
                reload_daemon();
            }
            Act::Reload => reload_daemon(),
            // In the daemon (or an on-demand run) the reload did it.
            Act::StartDaemon if lock(&DAEMON).is_none() => start_daemon(),
            Act::StartDaemon => {}
            Act::Browse if !self.picking => {
                self.picking = true;
                let start = PathBuf::from(self.s.form.folder.text.trim());
                let tx = self.picked.0.clone();
                let owner = OwnerHandle::of(self.hwnd);
                std::thread::spawn(move || {
                    let _ = tx.send(crate::ui::folder_dialog::pick_folder(owner.get(), &start));
                });
            }
            Act::Browse => {}
            Act::OpenConfig => {
                std::thread::spawn(crate::actions::open_config);
            }
            Act::CheckUpdates => {
                std::thread::spawn(crate::actions::check_updates);
            }
            Act::ClearHistory => crate::history_ui::show_clear(),
            Act::Link(u) => {
                std::thread::spawn(move || crate::actions::open_link(u));
            }
            Act::OpenDefaultApps => {
                std::thread::spawn(|| {
                    if let Err(e) = crate::screenclip_win::open_default_apps() {
                        eprintln!("rustshot: opening Default apps: {e}");
                        if let Some(tx) = lock(&OPEN).as_ref() {
                            let _ = tx.send(Msg::DefaultAppsFailed(crate::screenclip_win::open_failed_text(&e)));
                        }
                    }
                });
            }
        }
    }

    /// Draw a frame from the current input; repeat while it produced a
    /// click (the content changes) or the kit asks for it.
    fn render(&mut self) {
        self.render_frames();
        self.report_focus();
    }

    /// End-to-end tests: `RUSTSHOT_TEST_FOCUS_FILE` gets the focused
    /// control's id ([`crate::ui::id_of`] its name, decimal; `none`) after
    /// every render, so a test can Tab to a control by name instead of
    /// counting the rows before it.
    fn report_focus(&mut self) {
        static PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        let Some(path) = PATH.get_or_init(|| std::env::var_os("RUSTSHOT_TEST_FOCUS_FILE").map(PathBuf::from)) else { return };
        let now = self.focus.focused_id().map_or_else(|| "none".to_string(), |id| id.to_string());
        if self.reported.as_deref() != Some(now.as_str()) {
            // A temporary file renamed over the old one: a reader never
            // sees half a number.
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, &now).and_then(|()| std::fs::rename(&tmp, path)).is_ok() {
                self.reported = Some(now);
            }
        }
    }

    fn render_frames(&mut self) {
        for _ in 0..4 {
            if self.closing || self.canvas.width() == 0 {
                return;
            }
            let k = wind::follow_scale(self.hwnd, &mut self.sized_for, W, H);
            // The form's theme and accent show before Apply.
            self.th = self.s.preview_theme();
            let click = frame(&mut self.canvas, &self.th, &self.input, &mut self.focus, &mut self.s, k);
            self.input.end_frame();
            let (click, redraw) = click;
            let Some(c) = click else {
                if redraw {
                    continue;
                }
                return;
            };
            for a in self.s.on(c) {
                self.perform(a);
            }
        }
    }

    fn settle(&mut self) {
        if self.closing {
            wind::close(self.hwnd);
        }
    }
}

/// Paint one frame of `s` into `canvas` (window background first); returns
/// the click (Esc/Enter included) and whether the kit wants another frame.
fn frame(canvas: &mut PixBuf, th: &Theme, input: &Input, focus: &mut FocusState, s: &mut Settings, k: f32) -> (Option<Click>, bool) {
    let (w, h) = canvas.dimensions();
    let bg = th.surface.with_alpha(255);
    for p in canvas.as_raw_mut().as_chunks_mut::<4>().0 {
        *p = [bg.r, bg.g, bg.b, 255];
    }
    let area = FRect { x: 0.0, y: 0.0, w: w as f32 / k, h: h as f32 / k };
    let mut clip = crate::ui::SystemClipboard;
    let fb = crate::uifb::Fb::new(canvas.as_raw_mut(), w as usize);
    let mut ui = Ui::new(fb, th, input, focus, &mut clip, k, area);
    let click = draw::paint(&mut ui, s);
    let out = ui.finish();
    let click = click.or(if out.escape {
        Some(Click::Escape)
    } else if out.enter {
        Some(Click::Enter)
    } else {
        None
    });
    (click, out.redraw)
}

/// A window handle that may cross to the folder-picker thread.
#[derive(Clone, Copy)]
struct OwnerHandle(usize);

impl OwnerHandle {
    fn of(h: Hwnd) -> OwnerHandle {
        #[cfg(not(target_os = "macos"))]
        return OwnerHandle(h.0 as usize);
        #[cfg(target_os = "macos")]
        return OwnerHandle(h.0);
    }

    fn get(self) -> Option<Hwnd> {
        #[cfg(windows)]
        let h = windows::Win32::Foundation::HWND(self.0 as *mut core::ffi::c_void);
        #[cfg(not(windows))]
        let h = Hwnd(self.0 as _);
        (self.0 != 0).then_some(h)
    }
}

impl Driver for Window {
    fn on_create(&mut self, hwnd: Hwnd) {
        self.hwnd = hwnd;
    }

    fn on_event(&mut self, ev: Ev) -> bool {
        match ev {
            Ev::Resize(w, h) => {
                self.canvas = PixBuf::new(w.max(1), h.max(1));
                self.render();
            }
            Ev::Close => {
                for a in self.s.on(Click::Cancel) {
                    self.perform(a);
                }
            }
            Ev::Timer => {
                let mut changed = false;
                let mut raise = false;
                while let Ok(m) = self.rx.try_recv() {
                    match m {
                        Msg::Raise => raise = true,
                        Msg::Section(n) => {
                            self.s.section = n.min(SECTIONS.len() - 1);
                            changed = true;
                        }
                        Msg::Note(t) => {
                            self.s.notify(t);
                            changed = true;
                        }
                        Msg::Hotkeys(t, field) => {
                            self.s.notify_hotkeys(t, field);
                            raise = true;
                            changed = true;
                        }
                        Msg::Guide => {
                            self.s.section = CAPTURE;
                            self.s.sync_screenclip(crate::screenclip_win::state());
                            self.s.open_guide();
                            changed = true;
                        }
                        Msg::DefaultAppsFailed(t) => {
                            self.s.default_apps_failed(t);
                            changed = true;
                        }
                    }
                }
                if raise {
                    #[cfg(windows)]
                    wind::raise(self.hwnd);
                }
                // The UI size changed elsewhere (the overlay's Ctrl+=).
                wind::follow_scale(self.hwnd, &mut self.sized_for, W, H);
                // The broken file fixed in an editor: the window works again.
                if self.s.broken() && self.rechecked.elapsed() >= std::time::Duration::from_millis(500) {
                    self.rechecked = std::time::Instant::now();
                    if self.s.recheck() {
                        self.th = crate::theme::resolve(&self.s.loaded);
                        changed = true;
                    }
                }
                // A hotkey registered (or failed) since: its note follows.
                let failures = hotkey::outstanding();
                if failures != self.s.hotkey_failures {
                    self.s.hotkey_failures = failures;
                    changed = true;
                }
                // The Recording section's devices were listed.
                changed |= self.s.rec_lists.poll();
                // Upload's workers answered (Test connection, the secret).
                let (ch, acts) = self.s.poll_upload();
                changed |= ch;
                for a in acts {
                    self.perform(a);
                }
                // The toast fades: frames while it shows, one after.
                let toast = self.s.toast.is_some();
                let live = self.s.toast_live();
                changed |= toast;
                wind::set_fast_timer(self.hwnd, live);
                while let Ok(p) = self.picked.1.try_recv() {
                    self.picking = false;
                    if let Some(p) = p {
                        self.s.form.folder.set(&p.display().to_string());
                        changed = true;
                    }
                }
                if changed {
                    self.render();
                }
                self.settle();
                return changed && !self.closing;
            }
            Ev::Move { .. } => {
                self.input.feed(&ev);
                self.render();
                if !self.closing {
                    wind::invalidate(self.hwnd); // hover changes
                }
            }
            // Back (from Default apps, say): Win+Shift+S's app read again.
            Ev::Focus(true) => {
                self.input.feed(&ev);
                if self.s.refocused() && self.s.toast.is_some() {
                    wind::set_fast_timer(self.hwnd, true);
                }
                self.render();
            }
            _ => {
                self.input.feed(&ev);
                self.render();
            }
        }
        self.settle();
        !self.closing
    }

    fn frame(&mut self) -> Option<&mut PixBuf> {
        if self.canvas.width() == 0 || self.closing {
            return None;
        }
        if self.out.dimensions() != self.canvas.dimensions() {
            self.out = PixBuf::new(self.canvas.width(), self.canvas.height());
        }
        self.out.as_raw_mut().copy_from_slice(self.canvas.as_raw());
        Some(&mut self.out)
    }

    fn cursor(&self) -> Cursor {
        Cursor::Arrow
    }

    /// The title bar follows the theme the window shows (a Theme click
    /// recolours it before Apply, as it does the rest of the window).
    fn frame_colors(&self) -> Option<wind::FrameColors> {
        Some(crate::ui::paint::WTok::of(&self.th).frame())
    }
}

#[cfg(test)]
mod tests;
