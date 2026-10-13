//! The Settings window (the Settings board, 680 × 520): a rail with
//! **General** (theme, accent, start at login, tray icon, copy after
//! capture, shutter sound, where and how captures are saved),
//! **Capture** (daily subfolders, the file-name pattern editor, Save As,
//! format and JPEG quality, the overlay's UI size, snapping and renderer,
//! "Reset tool sizes & colors"), **Recording** (the Record popover's
//! defaults: [`recording`]), **Key bindings** (the global hotkeys and the
//! overlay keymap), **Upload** (Imgur) and **About**, with a footer of
//! **OK** / **Cancel** / **Apply** (the board leaves it out; the window
//! keeps its explicit save). Drawn in [`draw`]. Opened from the tray
//! ("Settings…") and by `rustshot settings` ([`show`], [`run_here`]).
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

/// The rail's sections (icon, name); About sits at the bottom.
pub const SECTIONS: [(&str, &str); 6] = [
    ("set-sliders", "General"),
    ("set-camera", "Capture"),
    ("video", "Recording"),
    ("set-keyboard", "Key bindings"),
    ("cloud", "Upload"),
    ("info", "About"),
];
/// Section indices.
pub const GENERAL: usize = 0;
pub const CAPTURE: usize = 1;
pub const RECORDING: usize = 2;
pub const KEYS: usize = 3;
pub const UPLOAD: usize = 4;
pub const ABOUT: usize = 5;

/// (label, config value)
const THEMES: [(&str, &str); 3] = [("System", "auto"), ("Dark", "dark"), ("Light", "light")];
const RENDERERS: [(&str, &str); 2] = [("GDI", "gdi"), ("Software", "software")];
const FORMATS: [(&str, &str); 3] = [("PNG", "png"), ("JPG", "jpg"), ("BMP", "bmp")];
/// Capture › Accept on select (`accept_on_select`).
pub const ACCEPTS: [(&str, &str); 3] = [("Off", "off"), ("Copy", "copy"), ("Save", "save")];
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
    /// `config.toml` does not parse (the error): Open config file, Reset
    /// to defaults…, Close; nothing else until it is fixed.
    Broken(String),
    /// [`RESET`] over [`Modal::Broken`] (its error, to go back to).
    Reset(String),
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
    /// Registered, but Windows still opens the Snipping Tool: a note says
    /// to pick Rustshot when Windows asks.
    pub screenclip_wait: bool,
    pub form: Form,
    /// The rail's section ([`SECTIONS`] index).
    pub section: usize,
    /// Each section's scroll offset and content height (logical px).
    scroll: [f32; SECTIONS.len()],
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
}

impl Settings {
    pub fn new(cfg: Config, autostart: Option<bool>, path: PathBuf) -> Settings {
        Settings {
            form: Form::new(&cfg, autostart),
            loaded: cfg,
            autostart,
            screenclip: None,
            screenclip_wait: false,
            section: GENERAL,
            scroll: [0.0; SECTIONS.len()],
            content_h: [0.0; SECTIONS.len()],
            modal: None,
            confirmed: false,
            pattern: Pattern::File,
            path,
            renderer: cfg!(windows),
            tray_row: cfg!(windows),
            stale: false,
            pending: Vec::new(),
            folder_probe: None,
            hotkey_failures: Vec::new(),
            tools_reset: false,
            rec_lists: recording::Lists::default(),
            upload: upload::State::default(),
            set_autostart: crate::autostart::set,
            set_screenclip: crate::screenclip_win::set,
        }
    }

    /// Show the Win+Shift+S toggle at `state` (as the registry has it;
    /// `None` hides it); `waiting`: Windows has not been told to use us.
    pub fn with_screenclip(mut self, state: Option<bool>, waiting: bool) -> Settings {
        self.screenclip = state;
        self.form.screenclip = state;
        self.screenclip_wait = waiting;
        self
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
            Some(_) => self.pending.push(msg),
        }
    }

    fn show_pending(&mut self) {
        if self.modal.is_none() && !self.pending.is_empty() {
            self.modal = Some(Modal::Error(std::mem::take(&mut self.pending).join("\n")));
        }
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

    /// The note under hotkey `slot`'s row while it is not registered:
    /// "Not registered — using Ctrl+Alt+Shift+Q" (the chord kept in its
    /// place), or "Not registered".
    pub fn hotkey_slot_note(&self, slot: usize) -> Option<String> {
        let f = self.hotkey_failures.iter().find(|f| f.slot == slot)?;
        Some(match &f.kept {
            Some(k) => format!("Not registered \u{2014} using {}", hotkey::chord_label(k)),
            None if f.cause == hotkey::Cause::InUse => "Already used by another app. Choose another.".into(),
            None => "Not registered".into(),
        })
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
            (Click::Escape, Some(_)) => Click::Back,
            (Click::Escape, None) => Click::Cancel,
            (Click::Enter, Some(Modal::Rewrite { .. })) => Click::Save,
            (Click::Enter, Some(Modal::Error(_))) => Click::Back,
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
            let cfg = self.form.apply(&self.loaded, &onto);
            Some((config::save_at(&path, &cfg), cfg))
        });
        let Some((res, cfg)) = saved else { return vec![] };
        if let Err(e) = res {
            self.modal = Some(Modal::Error(format!("Could not save {}: {e}", self.path.display())));
            return vec![];
        }
        self.confirmed = true; // the file is ours now
        self.loaded = cfg.clone();
        // The typed S3 secret goes to the OS credential store, never the file.
        let secret = std::mem::take(&mut self.form.up.secret.text);
        // The form shows what was saved (merged with the file, trimmed).
        self.form = self.form_for(&cfg);
        let mut acts = vec![Act::Saved(Box::new(cfg.clone()))];
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
                    self.screenclip_wait = on;
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
    /// Go to this section ([`show_at`]).
    Section(usize),
}

/// The open window's inbox, if any.
static OPEN: Mutex<Option<Sender<Msg>>> = Mutex::new(None);
/// The section a window opened by [`show_at`] starts in.
static START_SECTION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(GENERAL);
/// The daemon's event sender: told to reload after a save.
static DAEMON: Mutex<Option<Sender<HotEvent>>> = Mutex::new(None);

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

/// Another window changed `config.toml` (History's "Keep a history of
/// captures", the Edit window's "Don't ask again"): the daemon reloads.
pub fn tell_daemon_reload() {
    reload_daemon();
}

/// Open the window at `section` (History's "History settings" link goes
/// to Capture), or bring the open one there.
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

/// Show `text` in the open Settings window (over the form, with OK);
/// false when no window is open (the caller tells the user some other way).
pub fn notify(text: &str) -> bool {
    lock(&OPEN).as_ref().is_some_and(|tx| tx.send(Msg::Note(text.to_string())).is_ok())
}

/// Open the window on its own thread, or bring the open one to the front.
pub fn show() {
    #[cfg(target_os = "macos")]
    std::thread::spawn(crate::actions::open_config); // never block the caller
    #[cfg(not(target_os = "macos"))]
    if let Some(rx) = claim() {
        std::thread::spawn(move || run(rx));
    }
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
        .with_screenclip(crate::screenclip_win::state(), crate::screenclip_win::awaiting_choice());
    s.hotkey_failures = hotkey::outstanding();
    s.section = START_SECTION.swap(GENERAL, std::sync::atomic::Ordering::SeqCst).min(SECTIONS.len() - 1);
    let th = crate::theme::resolve(&s.loaded);
    // Opened without a daemon (`rustshot settings`): the file's UI size.
    // In the daemon an overlay may have set it and still have the write
    // pending, so the file is older: `init` leaves a set size alone.
    if !s.stale {
        uisize::init(s.loaded.ui_scale);
    }
    let mut w = Window::new(s, th, rx);
    let spec = WindowSpec { title: "Rustshot Settings".into(), w: W, h: H, resizable: false, min: (W, H) };
    if let Err(e) = wind::run_window(spec, &mut w) {
        eprintln!("rustshot: settings window: {e:#}");
    }
    let mut open = lock(&OPEN);
    let mut late = false;
    let mut notes = Vec::new();
    for m in w.rx.try_iter() {
        match m {
            Msg::Raise => late = true,
            Msg::Note(t) => notes.push(t),
            Msg::Section(_) => late = true,
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
        }
    }

    /// Draw a frame from the current input; repeat while it produced a
    /// click (the content changes) or the kit asks for it.
    fn render(&mut self) {
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
                // Upload's Test connection answered.
                let (ch, acts) = self.s.poll_upload();
                changed |= ch;
                for a in acts {
                    self.perform(a);
                }
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
}

#[cfg(test)]
mod tests;
