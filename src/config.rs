use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use crate::uisize::UiSize;

/// Settings, mirroring Flameshot's key names where they apply.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Directory to save captures into. Empty = `<Pictures>/rustshot`.
    pub save_path: String,
    /// Put each capture in a subfolder of `save_path` named by
    /// `subfolder_pattern` (one folder per day by default).
    pub save_subfolder: bool,
    /// Pattern for that subfolder (same tokens as `filename_pattern`;
    /// `/` nests folders, `..` is dropped).
    pub subfolder_pattern: String,
    /// strftime-like pattern, e.g. `%F_%H-%M`.
    pub filename_pattern: String,
    /// Format for Ctrl+S / direct captures: "png", "jpg" or "bmp".
    pub save_format: String,
    /// JPEG quality, 1-100.
    pub jpeg_quality: u8,
    /// Ctrl+S always asks with a Save As dialog (the old behaviour).
    pub save_dialog: bool,
    /// Accent override for the overlay UI (Flameshot `uiColor`); empty =
    /// the theme's own accent.
    pub ui_color: String,
    /// Overlay theme: "auto" (follow the OS), "dark" or "light".
    pub theme: String,
    /// UI language: empty for the system's, else a code of `i18n/` ("de",
    /// "pt-BR"; Settings › General › Language).
    pub language: String,
    /// UI size: "auto" (each monitor's OS scale) or a percentage of it
    /// (75, 90, 100, 125, 150, 200; see `uisize`).
    pub ui_scale: UiSize,
    /// Darkness of the dimmed area outside the selection (0-255).
    pub contrast_opacity: u8,
    pub draw_color: String,
    pub draw_thickness: f32,
    pub draw_marker_size: f32,
    pub draw_pixelate_size: f32,
    pub draw_font_size: f32,
    pub undo_limit: usize,
    /// Global hotkey that starts a capture (daemon mode).
    pub capture_hotkey: String,
    /// Global hotkey that quits the daemon.
    pub quit_hotkey: String,
    /// Global hotkey that captures every monitor at once and saves it (no
    /// overlay).
    pub capture_full_hotkey: String,
    /// Global hotkey that opens the overlay picking a window (a click on
    /// the outlined window selects it).
    pub capture_window_hotkey: String,
    /// Global hotkey that captures the last capture's region again (same
    /// monitor) and saves it.
    pub repeat_hotkey: String,
    /// Global hotkey that picks an area and copies its text (OCR; no
    /// annotation step).
    pub capture_text_hotkey: String,
    /// Show the notification-area (tray) icon; off, the daemon runs
    /// without one (hotkeys and a second launch still capture).
    pub show_tray: bool,
    /// Also copy the image to the clipboard when a capture is saved or
    /// uploaded.
    pub copy_after_capture: bool,
    /// A short shutter sound when a capture is taken.
    pub shutter_sound: bool,
    /// After a screenshot is copied or saved, a thumbnail card bottom-right
    /// (Copy, Save, Pin; drag it out for the file) instead of the toast.
    pub show_thumbnail: bool,
    /// Keep a history of captures (`%LOCALAPPDATA%\rustshot-data\history`: the
    /// unannotated original, its annotations, a thumbnail and what is
    /// known about it) for the History and Edit windows.
    pub history: bool,
    /// History entries older than this many days are removed (1-3650).
    pub history_days: u32,
    /// The history is kept under this many MB, oldest entries out first.
    pub history_max_mb: u32,
    /// Global hotkey that opens the History window.
    pub history_hotkey: String,
    /// The Edit window's Save: "" asks (Overwrite the original / Save as
    /// new file), "overwrite" or "new" once "Don't ask again" was ticked.
    pub edit_save: String,
    /// The overlay's Copy text button: "select" (Text mode: pick words,
    /// then copy) or "all" (copy everything at once). The global Copy
    /// text hotkey always copies all.
    pub copy_text_mode: String,
    /// Quick Redact's kinds ("all", or a list such as "emails,phones";
    /// [`crate::ocr::redact::parse_kinds`]); the Redact menu remembers them.
    pub redact_kinds: String,
    /// What Quick Redact puts over a find: "solid" (an opaque fill in the
    /// surrounding color) or "blur" (a coarse mosaic under a strong blur).
    pub redact_style: String,
    /// The last selection mode: "rect" or "freeform" (a lasso).
    pub selection_mode: String,
    /// Palette shown in the color picker.
    pub user_colors: Vec<String>,
    /// Imgur client id used for uploads.
    pub upload_client_id: String,
    /// Copy the resulting URL to the clipboard after upload.
    pub copy_url_after_upload: bool,
    /// Where uploads go: "imgur" or "s3" (an S3-compatible bucket, see
    /// `crate::s3`).
    pub upload_destination: String,
    /// S3: the endpoint (`https://<account>.r2.cloudflarestorage.com`,
    /// `https://s3.<region>.amazonaws.com`), the signing region ("auto"
    /// for R2), the bucket and the access key id. The secret access key
    /// is never here: it is in the OS credential store (`crate::keystore`).
    pub s3_endpoint: String,
    pub s3_region: String,
    pub s3_bucket: String,
    pub s3_access_key_id: String,
    /// S3: the public URL base links start with ("" = the object's URL).
    pub s3_public_url: String,
    /// S3: the key prefix, with strftime tokens (`%Y/%m/%d/`).
    pub s3_key_prefix: String,
    /// S3: path-style URLs (`<endpoint>/<bucket>/<key>`); off,
    /// virtual-hosted (`<bucket>.<endpoint>/<key>`).
    pub s3_path_style: bool,
    /// Upload a recording as soon as it is saved (the toast's Copy link
    /// then copies at once).
    pub upload_recordings: bool,
    /// If true, GUI captures only the monitor under the cursor even when
    /// all monitors share the same scale factor.
    pub capture_active_monitor: bool,
    /// Before a selection exists, the overlay outlines the window under
    /// the pointer and a click selects it (Tab: its parts). X11, Windows
    /// and macOS; Wayland has no window positions.
    pub snap_windows: bool,
    /// Show "Tab: parts of the window" while snapping, until Tab is first
    /// used (then written as `false`).
    pub snap_tab_hint: bool,
    /// Let the daemon check GitHub for a newer release once a day.
    pub check_updates: bool,
    /// Download data files from this version's GitHub release when they
    /// are first needed (a language file, the camera blur's model). Off:
    /// Settings says which file to get and where to put it.
    pub download_data: bool,
    /// A release version the user chose to skip; the daily check ignores it
    /// (a manual check still offers it). Empty = none.
    pub skip_version: String,
    /// Overlay renderer on Windows: "gdi" (screenshot kept in GDI bitmaps,
    /// only changed rects repainted; the default) or "software" (whole
    /// frame composed in memory). Linux/macOS always use software.
    pub renderer: String,
    /// Recording: default format, "mp4" or "gif".
    pub rec_format: String,
    /// Recording: MP4 frame rate, 30 or 60.
    pub rec_fps: u32,
    /// Recording: GIF frame rate, 10, 15 or 24.
    pub rec_gif_fps: u32,
    /// Recording: MP4 quality, "low", "medium" or "high".
    pub rec_quality: String,
    /// Recording: capture system sound (MP4 only).
    pub rec_system_audio: bool,
    /// Recording: capture the microphone (MP4 only).
    pub rec_mic: bool,
    /// Recording: microphone device id; empty = the default device.
    pub rec_mic_device: String,
    /// Recording: show the camera bubble.
    pub rec_camera: bool,
    /// Recording: camera device id; empty = the default camera.
    pub rec_camera_device: String,
    /// Recording: bubble shape, "circle", "rounded" or "square".
    pub rec_camera_shape: String,
    /// Recording: bubble size, "s", "m", "l" or "xl" (120/160/200/240
    /// logical px).
    pub rec_camera_size: String,
    /// Recording: mirror the camera image.
    pub rec_camera_mirror: bool,
    /// Recording: 3-2-1 countdown before the first frame.
    pub rec_countdown: bool,
    /// Global hotkey that starts (and stops) a recording.
    pub record_hotkey: String,
    /// Linux MP4 encoder: "auto", "ffmpeg" or "openh264".
    pub rec_linux_encoder: String,
    /// Recording: the camera bubble's ring color (`#rrggbb`); empty = the
    /// theme's accent.
    pub rec_camera_ring_color: String,
    /// Recording: the ring's width in logical px (0 = no ring).
    pub rec_camera_ring_width: u32,
    /// Recording: a GIF stops by itself after this many seconds (15, 30
    /// or 60; video mode's panel and Settings › Recording "Max length").
    pub rec_gif_max: u32,
    /// Recording: the camera bubble's background, "off", "light" or
    /// "strong" (blurred behind the person).
    pub rec_camera_bg: String,
    /// Recording: a ring where the pointer clicks, drawn into the frames.
    pub rec_show_clicks: bool,
    /// Recording: the last key chords at the bottom centre ("Ctrl + S").
    pub rec_show_keys: bool,
    /// With `rec_show_keys`: every key, not only chords with Ctrl, Alt or
    /// Win (never in a password field where that can be told).
    pub rec_show_all_keys: bool,
    /// After Stop: "off" (save at once) or "ask" (the trim window).
    pub rec_trim: String,
    /// The capture launcher's last choices: "region", "screen" or
    /// "window"; the delay in seconds (0-60); "edit", "copy", "save" or
    /// "pin" after the capture.
    pub launcher_source: String,
    pub launcher_delay: u32,
    pub launcher_after: String,
    /// The color picker's recent custom colors, newest first (at most
    /// [`RECENT_MAX`]).
    pub recent_colors: Vec<String>,
    /// Each drawing tool's last size (`tool_<id>_size`, ids in
    /// [`TOOL_IDS`]); a tool without one starts from the `draw_*` default.
    pub tool_sizes: BTreeMap<String, f32>,
    /// Each drawing tool's last color (`tool_<id>_color`).
    pub tool_colors: BTreeMap<String, String>,
    /// Filled or outlined, for the shapes that can be either
    /// (`tool_<id>_fill`, ids in [`FILL_IDS`]).
    pub tool_fills: BTreeMap<String, bool>,
    /// The callout shape new bubbles get (`tool_callout_style`: rounded,
    /// cloud or ellipse; empty: rounded).
    pub callout_style: String,
    /// The text and callout tools' font family (`tool_<id>_font`: sans,
    /// serif or mono; ids in [`FMT_IDS`]).
    pub tool_fonts: BTreeMap<String, String>,
    /// Their style words (`tool_text_style`, `tool_callout_text_style`:
    /// "bold,italic,underline,strike", those on; "" for none).
    pub tool_text_styles: BTreeMap<String, String>,
    /// The text tool's alignment (`tool_text_align`: left, center, right).
    pub tool_aligns: BTreeMap<String, String>,
    /// Editor shortcut overrides from the `[shortcuts]` table
    /// (`action id -> "Ctrl+Shift+S"`; see `keymap`).
    pub shortcuts: BTreeMap<String, String>,
    /// The overlay's pixel magnifier and crosshair while a region is
    /// being picked (Settings › Overlay "Show magnifier").
    pub magnifier: bool,
    /// The magnifier is square (default: round).
    pub magnifier_square: bool,
    /// Releasing the mouse on a new region runs this at once and closes:
    /// "off", "copy" or "save" (Shift while releasing skips it).
    pub accept_on_select: String,
    /// The hint card when the overlay opens (Settings › Overlay).
    pub overlay_hints: bool,
    /// Settings › Capture "Trim margins automatically": every new region
    /// or window selection is tightened to its content (`trim`).
    pub trim_margins: bool,
    /// Settings › Capture "HDR screenshots" (Windows): "tonemap" (an HDR
    /// monitor is read in FP16 and tone-mapped to SDR) or "keep" (as the
    /// system hands it over).
    pub hdr_screenshots: String,
    /// Settings › Capture "Add a border": its color (`#rrggbb`) and width
    /// in logical px (0 = no border, 1-8), around every exported image.
    pub border_color: String,
    pub border_width: u32,
    /// Settings › Saving "Save at": "screen" (the screen's pixels, as
    /// captured), "logical" (1×: a high-DPI capture scaled down by its
    /// scale, area-averaged) or "custom" (`save_scale_percent`, 10-100 %).
    /// Copy, Save and Upload get it; a pin and History keep the original size.
    pub save_scale: String,
    pub save_scale_percent: u32,
    /// Windows: the daemon reminds how to pick Rustshot for Win+Shift+S
    /// while it is offered but not chosen (false: "Don't show again").
    pub screenclip_hint: bool,
    /// Windows: the user knows that with the tray icon off nothing runs in
    /// the background (they turned it off in this version, or saw the
    /// one-time notice an older file with `show_tray = false` gets).
    pub tray_off_mode_seen: bool,
}

/// The widest export border (logical px).
pub const BORDER_MAX: u32 = 8;

/// The drawing tools whose size and color are remembered
/// (`tool_<id>_size`, `tool_<id>_color`).
pub const TOOL_IDS: [&str; 12] = ["pencil", "line", "arrow", "rect", "ellipse", "marker", "text", "counter", "pixelate", "callout", "magnify", "blur"];

/// The tools that draw filled or outlined (`tool_<id>_fill`).
pub const FILL_IDS: [&str; 3] = ["rect", "ellipse", "callout"];

/// The tools that type styled text (`tool_<id>_font` and their style).
pub const FMT_IDS: [&str; 2] = ["text", "callout"];

/// What a remembered text-style key holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FmtKey {
    Font,
    Style,
    Align,
}

/// The config key of a text style part of tool `id` (`tool_text_font`,
/// `tool_text_style`, `tool_text_align`, `tool_callout_font`,
/// `tool_callout_text_style`: the callout's `tool_callout_style` is its
/// bubble shape).
pub fn fmt_key_name(id: &str, what: FmtKey) -> String {
    match (id, what) {
        (id, FmtKey::Font) => format!("tool_{id}_font"),
        ("callout", FmtKey::Style) => "tool_callout_text_style".into(),
        (id, FmtKey::Style) => format!("tool_{id}_style"),
        (id, FmtKey::Align) => format!("tool_{id}_align"),
    }
}

/// A text-style key split into (id, what).
fn fmt_key(key: &str) -> Option<(&'static str, FmtKey)> {
    FMT_IDS
        .iter()
        .flat_map(|id| [(*id, FmtKey::Font), (*id, FmtKey::Style), (*id, FmtKey::Align)])
        .filter(|(id, w)| *w != FmtKey::Align || *id == "text")
        .find(|(id, w)| fmt_key_name(id, *w) == key)
}

/// Every remembered tool key (`tool_<id>_size`, `_color`, `_fill`, the
/// text styles): what Settings' "Reset tool sizes & colors" removes.
pub fn tool_key_names() -> Vec<String> {
    let mut v: Vec<String> = TOOL_IDS.iter().flat_map(|id| [format!("tool_{id}_size"), format!("tool_{id}_color")]).collect();
    v.extend(FILL_IDS.iter().map(|id| format!("tool_{id}_fill")));
    v.push("tool_callout_style".into());
    for id in FMT_IDS {
        v.push(fmt_key_name(id, FmtKey::Font));
        v.push(fmt_key_name(id, FmtKey::Style));
    }
    v.push(fmt_key_name("text", FmtKey::Align));
    v
}

/// How many recent custom colors are kept.
pub const RECENT_MAX: usize = 8;

/// The camera ring's default width (logical px) and the widest it gets.
pub const RING_WIDTH_DEFAULT: u32 = 2; // recorder redesign: 2 px accent ring (+ 1 px dark halo)
pub const RING_WIDTH_MAX: u32 = 16;

/// What a remembered tool key holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ToolKey {
    Size,
    Color,
    Fill,
}

/// `tool_<id>_size` / `tool_<id>_color` / `tool_<id>_fill` split into
/// (id, what), for a known tool id.
fn tool_key(key: &str) -> Option<(&str, ToolKey)> {
    let rest = key.strip_prefix("tool_")?;
    if let Some(id) = rest.strip_suffix("_fill") {
        return FILL_IDS.contains(&id).then_some((id, ToolKey::Fill));
    }
    let (id, what) = match rest.strip_suffix("_size") {
        Some(id) => (id, ToolKey::Size),
        None => (rest.strip_suffix("_color")?, ToolKey::Color),
    };
    TOOL_IDS.contains(&id).then_some((id, what))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            save_path: String::new(),
            save_subfolder: true,
            subfolder_pattern: "%F".into(),
            filename_pattern: "%F_%H-%M".into(),
            save_format: "png".into(),
            jpeg_quality: 90,
            save_dialog: false,
            ui_color: String::new(),
            theme: "auto".into(),
            language: String::new(),
            ui_scale: UiSize::Auto,
            contrast_opacity: 148,
            draw_color: "#f04438".into(),
            draw_thickness: 3.0,
            draw_marker_size: 15.0,
            draw_pixelate_size: 12.0,
            draw_font_size: 16.0,
            undo_limit: 100,
            capture_hotkey: "Shift+Meta+X".into(),
            quit_hotkey: "Ctrl+Alt+Shift+Q".into(),
            capture_full_hotkey: "Shift+Meta+F".into(),
            capture_window_hotkey: "Shift+Meta+E".into(),
            repeat_hotkey: String::new(),
            capture_text_hotkey: "Shift+Meta+Z".into(),
            show_tray: true,
            copy_after_capture: false,
            shutter_sound: false,
            show_thumbnail: true,
            magnifier: true,
            magnifier_square: false,
            accept_on_select: "off".into(),
            overlay_hints: true,
            trim_margins: false,
            hdr_screenshots: "tonemap".into(),
            border_color: "#000000".into(),
            border_width: 0,
            save_scale: "screen".into(),
            save_scale_percent: 50,
            screenclip_hint: true,
            tray_off_mode_seen: false,
            history: true,
            history_days: 30,
            history_max_mb: 2048,
            history_hotkey: "Shift+Meta+H".into(),
            edit_save: String::new(),
            copy_text_mode: "select".into(),
            redact_kinds: "all".into(),
            redact_style: "solid".into(),
            selection_mode: "rect".into(),
            user_colors: [
                "#f04438", "#ff8a1f", "#ffc532", "#2dc06f", "#19b5d6", "#3b82f6", "#8b5cf6",
                "#ec4899", "#ffffff", "#111318",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            upload_client_id: "313baf0c7b4d3ff".into(),
            copy_url_after_upload: true,
            upload_destination: "imgur".into(),
            s3_endpoint: String::new(),
            s3_region: crate::s3::DEFAULT_REGION.into(),
            s3_bucket: String::new(),
            s3_access_key_id: String::new(),
            s3_public_url: String::new(),
            s3_key_prefix: crate::s3::DEFAULT_PREFIX.into(),
            s3_path_style: true,
            upload_recordings: false,
            capture_active_monitor: false,
            snap_windows: true,
            snap_tab_hint: true,
            check_updates: true,
            download_data: true,
            skip_version: String::new(),
            renderer: "gdi".into(),
            rec_format: "mp4".into(),
            rec_fps: 30,
            rec_gif_fps: 15,
            rec_quality: "medium".into(),
            rec_system_audio: true,
            rec_mic: false,
            rec_mic_device: String::new(),
            rec_camera: false,
            rec_camera_device: String::new(),
            rec_camera_shape: "circle".into(),
            rec_camera_size: "m".into(),
            rec_camera_mirror: true,
            rec_countdown: true,
            record_hotkey: "Shift+Meta+G".into(),
            rec_linux_encoder: "auto".into(),
            rec_camera_ring_color: String::new(),
            rec_camera_ring_width: RING_WIDTH_DEFAULT,
            rec_gif_max: 30,
            rec_camera_bg: "off".into(),
            rec_show_clicks: false,
            rec_show_keys: false,
            rec_show_all_keys: false,
            rec_trim: "off".into(),
            launcher_source: "region".into(),
            launcher_delay: 0,
            launcher_after: "edit".into(),
            recent_colors: Vec::new(),
            tool_sizes: BTreeMap::new(),
            tool_colors: BTreeMap::new(),
            tool_fills: BTreeMap::new(),
            callout_style: String::new(),
            tool_fonts: BTreeMap::new(),
            tool_text_styles: BTreeMap::new(),
            tool_aligns: BTreeMap::new(),
            shortcuts: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Whether captures use the GDI overlay renderer: Windows only, and
    /// any value but "software" (unknown values fall back to gdi).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn use_gdi(&self) -> bool {
        cfg!(windows) && !self.renderer.trim().eq_ignore_ascii_case("software")
    }
}

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    if let Ok(v) = std::env::var("APPDATA")
        && !v.is_empty()
    {
        return PathBuf::from(v).join("rustshot");
    }
    #[cfg(unix)]
    if let Ok(v) = std::env::var("XDG_CONFIG_HOME")
        && !v.is_empty()
    {
        return PathBuf::from(v).join("rustshot");
    }
    #[cfg(unix)]
    if let Ok(v) = std::env::var("HOME")
        && !v.is_empty()
    {
        return PathBuf::from(v).join(".config").join("rustshot");
    }
    PathBuf::from(".")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Make sure `config.toml` exists (written with defaults if missing); returns its path.
pub fn ensure_config_file() -> std::io::Result<PathBuf> {
    let path = config_path();
    std::fs::create_dir_all(config_dir())?;
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(to_toml(&Config::default()).as_bytes())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    Ok(path)
}

pub fn load() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => match parse_config(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("warning: ignoring invalid config {}: {e}", path.display());
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

/// The config file at `path` (normally [`config_path`]): `Ok(None)` when
/// there is none, `Err` (e.g. "line 3: expected key = value") when it
/// cannot be read or does not parse. Unlike [`load`] it never falls back
/// to the defaults, so a caller cannot write them over a broken file.
pub fn read_at(path: &std::path::Path) -> Result<Option<Config>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_config(&text).map(Some),
        // A file where a folder of the path should be: no config there either.
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => Ok(None),
        Err(e) => Err(format!("cannot read it: {e}")),
    }
}

/// Write `cfg` to `path` (normally [`config_path`]; its directory is
/// created): the whole file, as [`to_toml`] renders it, atomically.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no Settings window yet
pub fn save_at(path: &std::path::Path, cfg: &Config) -> std::io::Result<()> {
    with_config_lock(path, || {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(path, to_toml(cfg).as_bytes())
    })
}

/// The advisory lock file next to the config file `path`
/// (`config.toml.lock`).
pub fn lock_path(path: &std::path::Path) -> std::path::PathBuf {
    let name = path.file_name().map_or_else(|| "config.toml".into(), |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!("{name}.lock"))
}

/// Run `f`, a read-modify-write of the config file at `path`, holding
/// the process-wide config mutex and an advisory lock on [`lock_path`]
/// (`LockFileEx` on Windows, `flock` on Unix, through `File::lock`), so
/// neither another thread nor another rustshot process (`rustshot
/// settings`, an update dialog) can write in between. Re-entrant on one
/// thread (a nested call just runs `f`). Without a lock file (its folder
/// cannot be created) only this process is serialised.
pub fn with_config_lock<R>(path: &std::path::Path, f: impl FnOnce() -> R) -> R {
    use std::cell::Cell;
    static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
    thread_local!(static HELD: Cell<bool> = const { Cell::new(false) });
    if HELD.with(Cell::get) {
        return f();
    }
    let _g = M.lock().unwrap_or_else(|e| e.into_inner());
    let lock = lock_path(path);
    if let Some(dir) = lock.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock).ok();
    let locked = file.as_ref().is_some_and(|f| f.lock().is_ok());
    HELD.with(|h| h.set(true));
    // Released even when `f` panics.
    struct Release<'a>(Option<&'a std::fs::File>, bool);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            HELD.with(|h| h.set(false));
            if let (Some(f), true) = (self.0, self.1) {
                let _ = f.unlock();
            }
        }
    }
    let _r = Release(file.as_ref(), locked);
    f()
}

/// Whether rewriting the config file `text` with [`to_toml`] would lose
/// something: comments, keys rustshot does not know, values written
/// differently, or a file that does not parse. Keys that are merely missing
/// or in another order do not count.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn rewrite_loses(text: &str) -> bool {
    let Ok(cfg) = parse_config(text) else { return true };
    let ours = to_toml(&cfg);
    let kept: std::collections::HashSet<&str> = ours.lines().map(str::trim).collect();
    text.lines().map(str::trim).any(|l| !l.is_empty() && !kept.contains(l))
}

/// "Skip this version": set `skip_version` in the config file at `path`
/// (normally [`config_path`]), editing only that line (comments, unknown
/// keys and even an invalid rest of the file stay as they are; a missing
/// file is created with defaults). Written atomically.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: no update dialog yet
pub fn save_skip_version_at(path: &std::path::Path, version: &str) -> std::io::Result<()> {
    save_top_key_at(path, "skip_version", version)
}

/// Set the top-level string `key` to `value` in the config file at
/// `path`, keeping everything else as written (`skip_version`, the Linux
/// recording encoder chosen in the first-use dialog).
pub fn save_top_key_at(path: &std::path::Path, key: &str, value: &str) -> std::io::Result<()> {
    with_config_lock(path, || {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => to_toml(&Config::default()),
            Err(e) => return Err(e),
        };
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(path, with_top_key(&text, key, value).as_bytes())
    })
}

/// `text` with its top-level `skip_version` line replaced by `version`
/// (inserted before the first `[table]`, or appended, when absent).
#[cfg(test)]
pub fn with_skip_version(text: &str, version: &str) -> String {
    with_top_key(text, "skip_version", version)
}

/// `text` with its top-level `key` line set to the string `value` (see
/// [`with_skip_version`]).
pub fn with_top_key(text: &str, key: &str, value: &str) -> String {
    with_top_raw(text, key, Some(&toml_str(value)))
}

/// `text` with its top-level `key` line set to the TOML value `raw`
/// (written as is: `"a string"`, `4`, `["#fff"]`), or removed (`None`).
/// The line keeps its place; a new key goes right after the last
/// top-level key line (never between a table and the comment that heads
/// it, nor after the table, where it would belong to it); with no keys
/// yet, before the first table's leading comment block. Duplicates
/// collapse into one; comments, other keys and the line endings (LF or
/// CRLF) stay as written.
pub fn with_top_raw(text: &str, key: &str, raw: Option<&str>) -> String {
    let crlf = text.contains("\r\n");
    let nl = if crlf { "\r\n" } else { "\n" };
    let line = raw.map(|v| format!("{key} = {v}"));
    let mut out = String::with_capacity(text.len() + line.as_ref().map_or(0, |l| l.len() + 4));
    // A removal has nothing to insert.
    let mut done = line.is_none();
    let mut top = true;
    // `out.len()` after the last top-level key line.
    let mut last_key: Option<usize> = None;
    // Where the run of comment lines just before this line starts.
    let mut block: Option<usize> = None;
    // Where a new key goes when the first table is reached: (position,
    // after a key line, lines between it and the table).
    let mut at_table: Option<(usize, bool, bool)> = None;
    for raw in text.split_inclusive('\n') {
        let body = strip_comment(raw).trim();
        if top && body.starts_with('[') {
            top = false;
            at_table = Some(match last_key {
                Some(p) => (p, true, out.len() > p),
                None => (block.unwrap_or(out.len()), false, true),
            });
        }
        let is_key = top && body.split_once('=').is_some_and(|(k, _)| k.trim() == key);
        if top {
            if body.is_empty() {
                // A comment line extends the block; a blank one ends it.
                if raw.trim().is_empty() {
                    block = None;
                } else {
                    block.get_or_insert(out.len());
                }
            } else {
                block = None;
            }
        }
        if is_key {
            if let (false, Some(l)) = (done, &line) {
                out.push_str(l);
                out.push_str(if raw.ends_with('\n') { nl } else { "" });
                done = true;
                last_key = Some(out.len());
            }
            continue;
        }
        out.push_str(raw);
        if top && !body.is_empty() {
            last_key = Some(out.len());
        }
    }
    if let (false, Some(l)) = (done, &line) {
        let (pos, blank) = match at_table {
            // A blank line between the new key and the table (or its
            // comment) unless one is there already.
            Some((p, after_key, gap)) => (p, !(after_key && gap)),
            None => (last_key.unwrap_or(out.len()), false),
        };
        let lead = if pos > 0 && !out[..pos].ends_with('\n') { nl } else { "" };
        let ins = format!("{lead}{l}{nl}{}", if blank { nl } else { "" });
        out.insert_str(pos, &ins);
    }
    out
}

/// Set (`Some(raw TOML value)`) or remove (`None`) top-level keys of the
/// config file at `path`, editing only their lines ([`with_top_raw`]);
/// written atomically. A missing file is created with the defaults first.
/// A file that does not parse is left alone (an `InvalidData` error): a
/// remembered size is never worth touching a file the user is fixing.
pub fn edit_top_keys_at(path: &std::path::Path, edits: &[(String, Option<String>)]) -> std::io::Result<()> {
    with_config_lock(path, || {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => to_toml(&Config::default()),
            Err(e) => return Err(e),
        };
        if let Err(e) = parse_config(&text) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("config.toml does not parse ({e})")));
        }
        let out = edits.iter().fold(text, |t, (k, v)| with_top_raw(&t, k, v.as_deref()));
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(path, out.as_bytes())
    })
}

/// Copy the value of the remembered key `key` (a `tool_<id>_size` or
/// `_color`, `recent_colors`, the camera ring's) from `from` to `to`.
pub fn copy_key(from: &Config, to: &mut Config, key: &str) {
    match key {
        "recent_colors" => to.recent_colors = from.recent_colors.clone(),
        "rec_camera_ring_color" => to.rec_camera_ring_color = from.rec_camera_ring_color.clone(),
        "rec_camera_ring_width" => to.rec_camera_ring_width = from.rec_camera_ring_width,
        "rec_camera_bg" => to.rec_camera_bg = from.rec_camera_bg.clone(),
        "ui_scale" => to.ui_scale = from.ui_scale,
        "snap_tab_hint" => to.snap_tab_hint = from.snap_tab_hint,
        "redact_kinds" => to.redact_kinds = from.redact_kinds.clone(),
        "selection_mode" => to.selection_mode = from.selection_mode.clone(),
        "tool_callout_style" => to.callout_style = from.callout_style.clone(),
        k if fmt_key(k).is_some() => {
            let (id, what) = fmt_key(k).expect("a style key");
            let (src, dst) = match what {
                FmtKey::Font => (&from.tool_fonts, &mut to.tool_fonts),
                FmtKey::Style => (&from.tool_text_styles, &mut to.tool_text_styles),
                FmtKey::Align => (&from.tool_aligns, &mut to.tool_aligns),
            };
            match src.get(id) {
                Some(v) => {
                    dst.insert(id.into(), v.clone());
                }
                None => {
                    dst.remove(id);
                }
            }
        }
        k => match tool_key(k) {
            Some((id, ToolKey::Fill)) => match from.tool_fills.get(id) {
                Some(v) => {
                    to.tool_fills.insert(id.into(), *v);
                }
                None => {
                    to.tool_fills.remove(id);
                }
            },
            Some((id, ToolKey::Size)) => match from.tool_sizes.get(id) {
                Some(v) => {
                    to.tool_sizes.insert(id.into(), *v);
                }
                None => {
                    to.tool_sizes.remove(id);
                }
            },
            Some((id, ToolKey::Color)) => match from.tool_colors.get(id) {
                Some(v) => {
                    to.tool_colors.insert(id.into(), v.clone());
                }
                None => {
                    to.tool_colors.remove(id);
                }
            },
            None => {}
        },
    }
}

/// The TOML text of a size value (`4`, `2.5`).
pub fn toml_num(v: f32) -> String {
    format!("{v}")
}

/// The TOML text of an array of strings.
pub fn toml_strings(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|s| toml_str(s)).collect::<Vec<_>>().join(", "))
}

/// One batch of key edits for [`edit_top_keys_at`], and who waits for it.
type KeyJob = (std::path::PathBuf, Vec<(String, Option<String>)>, Option<crate::chan::Sender<()>>);

/// What the writer thread does with one batch.
type EditFn = fn(&std::path::Path, &[(String, Option<String>)]);

/// [`edit_top_keys_at`], failures reported on stderr (a remembered size is
/// not worth a dialog).
fn write_key_edits(p: &std::path::Path, e: &[(String, Option<String>)]) {
    if let Err(err) = edit_top_keys_at(p, e) {
        eprintln!("rustshot: could not save tool settings to {}: {err}", p.display());
    }
}

/// How long a waiting [`queue_key_edits`] waits for the writer.
const KEY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Apply `edits` to the config file at `path` off the calling thread, in
/// order with earlier batches (one writer thread for the process); with
/// `wait`, return once it is written (or after [`KEY_WAIT`]). Tests write
/// synchronously (the writer itself is tested through [`KeyWriter`]).
pub fn queue_key_edits(path: &std::path::Path, edits: Vec<(String, Option<String>)>, wait: bool) {
    if edits.is_empty() {
        return;
    }
    if cfg!(test) {
        write_key_edits(path, &edits);
        return;
    }
    static W: std::sync::Mutex<KeyWriter> = std::sync::Mutex::new(KeyWriter::new(write_key_edits));
    // Sent under the lock (batches stay in order), waited for without it.
    let done = W.lock().unwrap_or_else(|e| e.into_inner()).send(path, edits, wait);
    KeyWriter::wait(done, KEY_WAIT);
}

/// The background writer behind [`queue_key_edits`]: one thread, started
/// on the first batch, applying batches in the order they were sent.
struct KeyWriter {
    tx: Option<crate::chan::Sender<KeyJob>>,
    run: EditFn,
}

impl KeyWriter {
    const fn new(run: EditFn) -> Self {
        KeyWriter { tx: None, run }
    }

    /// Queue one batch (starting the thread if needed); with `wait`, the
    /// receiver that hears when it is written. A writer that is gone
    /// (it never should be) is dropped and the batch is written here.
    fn send(
        &mut self,
        path: &std::path::Path,
        edits: Vec<(String, Option<String>)>,
        wait: bool,
    ) -> Option<crate::chan::Receiver<()>> {
        let run = self.run;
        let tx = self.tx.get_or_insert_with(|| {
            let (tx, rx) = crate::chan::channel::<KeyJob>();
            std::thread::spawn(move || {
                for (p, e, done) in rx {
                    run(&p, &e);
                    if let Some(d) = done {
                        let _ = d.send(());
                    }
                }
            });
            tx
        });
        let (dtx, drx) = crate::chan::channel();
        match tx.send((path.to_path_buf(), edits, wait.then_some(dtx))) {
            Ok(()) => wait.then_some(drx),
            Err(crate::chan::SendError((p, e, _))) => {
                self.tx = None;
                run(&p, &e);
                None
            }
        }
    }

    /// Wait up to `timeout` for a batch `send` returned a receiver for;
    /// false when it timed out.
    fn wait(done: Option<crate::chan::Receiver<()>>, timeout: std::time::Duration) -> bool {
        done.is_none_or(|rx| rx.recv_timeout(timeout).is_ok())
    }
}

/// Replace `path` with `bytes` via a uniquely named temp file in the same
/// directory and a rename, so a crash never leaves a half-written file. A
/// symlinked `path` is written through to its target (the link stays); on
/// Unix the file keeps its permission bits.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let path = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path).or_else(|_| {
            // Dangling link: its target, relative to the link's directory.
            let t = std::fs::read_link(path)?;
            Ok::<_, std::io::Error>(path.parent().map_or(t.clone(), |d| d.join(&t)))
        })?,
        _ => path.to_path_buf(),
    };
    let name = path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    let r = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        #[cfg(unix)]
        if let Ok(m) = std::fs::metadata(&path) {
            f.set_permissions(m.permissions())?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// A TOML basic string.
pub fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Validate that the config parses; used by `rustshot config --check`.
pub fn check() -> Result<()> {
    let path = config_path();
    if !path.exists() {
        println!("config file does not exist (defaults will be used): {}", path.display());
        return Ok(());
    }
    let text = std::fs::read_to_string(&path).context("read config")?;
    let cfg = parse_config(&text)
        .map_err(anyhow::Error::msg)
        .context("parse config")?;
    println!("config OK: {}", path.display());
    println!("save_path = {:?}", cfg.save_path);
    println!("filename_pattern = {:?}", cfg.filename_pattern);
    println!("capture_hotkey = {:?}", cfg.capture_hotkey);
    if let Some(w) = renderer_warning(&cfg.renderer) {
        println!("warning: {w}");
    }
    for w in shortcut_warnings(&cfg).into_iter().chain(hotkey_warnings(&cfg)) {
        println!("warning: {w}");
    }
    Ok(())
}

/// Global hotkeys (`capture_hotkey`, `quit_hotkey`, `record_hotkey`) that
/// do not parse as a chord. Warnings rather than parse errors, so configs
/// that load today keep loading; an empty value means unbound.
pub fn hotkey_warnings(cfg: &Config) -> Vec<String> {
    [
        ("capture_hotkey", &cfg.capture_hotkey),
        ("quit_hotkey", &cfg.quit_hotkey),
        ("record_hotkey", &cfg.record_hotkey),
        ("capture_full_hotkey", &cfg.capture_full_hotkey),
        ("capture_window_hotkey", &cfg.capture_window_hotkey),
        ("repeat_hotkey", &cfg.repeat_hotkey),
        ("capture_text_hotkey", &cfg.capture_text_hotkey),
        ("history_hotkey", &cfg.history_hotkey),
    ]
    .into_iter()
        .filter(|(_, v)| !v.trim().is_empty() && crate::keymap::Chord::parse(v).is_none())
        .map(|(k, v)| format!("{k} = {v:?} is not a valid hotkey; it will not be registered"))
        .collect()
}

/// Global hotkeys whose default moved: (config key, former defaults).
/// 0.2.0 moved Record off Win+Shift+R and Copy text off Win+Shift+T
/// (Windows holds both); 0.1.3 moved Capture window off Win+Shift+W.
pub const FORMER_DEFAULTS: [(&str, &[&str]); 3] = [
    ("record_hotkey", &["Shift+Meta+R"]),
    ("capture_text_hotkey", &["Shift+Meta+T"]),
    ("capture_window_hotkey", &["Shift+Meta+W"]),
];

/// The global hotkey field of `key` (one of [`FORMER_DEFAULTS`]').
fn former_field<'a>(cfg: &'a mut Config, key: &str) -> Option<&'a mut String> {
    match key {
        "record_hotkey" => Some(&mut cfg.record_hotkey),
        "capture_text_hotkey" => Some(&mut cfg.capture_text_hotkey),
        "capture_window_hotkey" => Some(&mut cfg.capture_window_hotkey),
        _ => None,
    }
}

/// Global hotkeys still on a former default ([`FORMER_DEFAULTS`]; the
/// chord compared, not the text, so "Meta+Shift+R" is "Shift+Meta+R"):
/// the old file kept the default of its day, so they get today's.
/// A key that was never a default is left alone. Never a clash: when
/// another global already has today's default, the first of the slot's
/// alternates (then the spare keys) no global has is used instead, or
/// none (unset) when every one is taken. Returns (key, old, new).
pub fn migrate_old_defaults(cfg: &mut Config) -> Vec<(&'static str, String, String)> {
    let def = Config::default();
    let mut out = Vec::new();
    for (key, olds) in FORMER_DEFAULTS {
        let new = former_field(&mut def.clone(), key).map(|s| s.clone()).unwrap_or_default();
        let slot = former_slot(key);
        let Some(v) = former_field(cfg, key) else { continue };
        let Some(c) = crate::keymap::Chord::parse(v) else { continue };
        if !olds.iter().any(|o| crate::keymap::Chord::parse(o) == Some(c)) {
            continue;
        }
        let wanted = crate::hotkey::wanted(cfg);
        let taken = |s: &str| {
            let k = crate::keymap::Chord::parse(s);
            (0..crate::hotkey::SLOTS).any(|j| j != slot && k.is_some() && crate::keymap::Chord::parse(&wanted[j]) == k)
        };
        let pick = std::iter::once(new.as_str())
            .chain(crate::hotkey::alternates(slot).iter().copied())
            .chain(crate::hotkey::FREE_POOL)
            .find(|s| !taken(s))
            .unwrap_or_default()
            .to_string();
        let Some(v) = former_field(cfg, key) else { continue };
        out.push((key, std::mem::replace(v, pick.clone()), pick));
    }
    out
}

/// The hotkey slot of a [`FORMER_DEFAULTS`] key.
fn former_slot(key: &str) -> usize {
    match key {
        "record_hotkey" => 2,
        "capture_window_hotkey" => 4,
        _ => 6, // capture_text_hotkey
    }
}

/// [`migrate_old_defaults`] for the daemon's `cfg`, read from `path`: the
/// moved keys are written back to the file (only their lines, atomically)
/// and logged. A file that cannot be written keeps them for next time;
/// this run uses the new keys anyway.
pub fn migrate_at(path: &std::path::Path, cfg: &mut Config) {
    let moved = migrate_old_defaults(cfg);
    if moved.is_empty() {
        return;
    }
    let edits: Vec<(String, Option<String>)> = moved.iter().map(|(k, _, new)| (k.to_string(), Some(toml_str(new)))).collect();
    let res = edit_top_keys_at(path, &edits);
    let def = crate::hotkey::defaults();
    for (k, old, new) in &moved {
        let why = if new.is_empty() {
            " (today's and every spare key are used by other Rustshot hotkeys: unset; pick one in Settings \u{203a} Key bindings)".to_string()
        } else if !def.iter().any(|d| crate::hotkey::chord_label(d) == crate::hotkey::chord_label(new)) {
            " (today's default is another Rustshot hotkey's)".to_string()
        } else {
            String::new()
        };
        let now = if new.is_empty() { "none".to_string() } else { crate::hotkey::chord_label(new) };
        eprintln!(
            "rustshot: {k} {} was an old default; it is {now} now{why}{}",
            crate::hotkey::chord_label(old),
            if res.is_ok() { " (config.toml updated)" } else { "" }
        );
    }
    if let Err(e) = res {
        eprintln!("rustshot: could not update {}: {e}", path.display());
    }
}

/// `[shortcuts]` problems: unknown actions, bad chords, and chords bound
/// to several actions (the first in table order wins).
pub fn shortcut_warnings(cfg: &Config) -> Vec<String> {
    let (km, mut out) = crate::keymap::Keymap::from_config(&cfg.shortcuts);
    for (c, acts) in km.conflicts() {
        let ids: Vec<&str> = acts.iter().map(|a| a.id()).collect();
        out.push(format!(
            "[shortcuts] {} is bound to {}; {} wins",
            c.display(),
            ids.join(", "),
            ids[0]
        ));
    }
    out
}

/// A warning for a `renderer` value other than "gdi" / "software" (the
/// overlay then uses gdi on Windows).
pub fn renderer_warning(v: &str) -> Option<String> {
    let t = v.trim();
    (!t.eq_ignore_ascii_case("gdi") && !t.eq_ignore_ascii_case("software"))
        .then(|| format!("unknown renderer {v:?} (expected \"gdi\" or \"software\"); using \"gdi\" on Windows"))
}

/// Hand-rolled parser for the flat `key = value` subset of TOML this config
/// uses: quoted strings, numbers, booleans, and arrays of strings.
pub fn parse_config(text: &str) -> Result<Config, String> {
    let mut cfg = Config::default();
    // Current `[table]`: "" = top level; keys of other tables are ignored.
    let mut table = String::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            table = name.trim().to_string();
            continue;
        }
        let Some((key, val)) = split_key(line) else {
            return Err(format!("line {}: expected key = value", idx + 1));
        };
        let key = key.trim();
        let val = val.trim();
        let bad = |kind: &str, v: &str| format!("line {}: invalid {kind} for '{key}': {v}", idx + 1);
        if table == "shortcuts" {
            let key = if key.starts_with('"') { as_string(key).map_err(|e| bad("key", &e))? } else { key.to_string() };
            let v = as_string(val).map_err(|e| bad("string", &e))?;
            cfg.shortcuts.insert(key, v);
            continue;
        }
        if !table.is_empty() {
            continue;
        }
        match key {
            "save_path" => cfg.save_path = as_string(val).map_err(|e| bad("string", &e))?,
            "filename_pattern" => {
                cfg.filename_pattern = as_string(val).map_err(|e| bad("string", &e))?
            }
            "ui_color" => cfg.ui_color = as_string(val).map_err(|e| bad("string", &e))?,
            "theme" => cfg.theme = as_string(val).map_err(|e| bad("string", &e))?,
            "language" => cfg.language = as_string(val).map_err(|e| bad("string", &e))?,
            "ui_scale" => {
                let v = if val.starts_with('"') { as_string(val).map_err(|e| bad("string", &e))? } else { val.to_string() };
                cfg.ui_scale = UiSize::parse(&v)
                    .ok_or_else(|| bad("value (expected \"auto\", 75, 90, 100, 125, 150 or 200)", val))?
            }
            "draw_color" => cfg.draw_color = as_string(val).map_err(|e| bad("string", &e))?,
            "capture_hotkey" => {
                cfg.capture_hotkey = as_string(val).map_err(|e| bad("string", &e))?
            }
            "quit_hotkey" => cfg.quit_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
            "capture_full_hotkey" => cfg.capture_full_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
            "capture_window_hotkey" => cfg.capture_window_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
            "repeat_hotkey" => cfg.repeat_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
            "capture_text_hotkey" => cfg.capture_text_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
            "show_tray" => cfg.show_tray = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "copy_after_capture" => cfg.copy_after_capture = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "shutter_sound" => cfg.shutter_sound = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "show_thumbnail" => cfg.show_thumbnail = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "magnifier" => cfg.magnifier = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "magnifier_square" => cfg.magnifier_square = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "accept_on_select" => {
                cfg.accept_on_select = one_of(val, &["off", "copy", "save"]).map_err(|e| bad("value", &e))?
            }
            "overlay_hints" => cfg.overlay_hints = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "trim_margins" => cfg.trim_margins = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "hdr_screenshots" => cfg.hdr_screenshots = one_of(val, &["tonemap", "keep"]).map_err(|e| bad("value", &e))?,
            "border_color" => {
                let v = as_string(val).map_err(|e| bad("string", &e))?;
                parse_color(&v).ok_or_else(|| bad("color (#rrggbb)", &v))?;
                cfg.border_color = v.trim().to_string();
            }
            "border_width" => {
                cfg.border_width =
                    as_u64(val).ok().filter(|w| *w <= BORDER_MAX as u64).ok_or_else(|| bad("number (0-8)", val))? as u32
            }
            "save_scale" => cfg.save_scale = one_of(val, &["screen", "logical", "custom"]).map_err(|e| bad("value", &e))?,
            "save_scale_percent" => {
                cfg.save_scale_percent =
                    as_u64(val).ok().filter(|p| (10..=100).contains(p)).ok_or_else(|| bad("number (10-100)", val))? as u32
            }
            "screenclip_hint" => cfg.screenclip_hint = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "tray_off_mode_seen" => cfg.tray_off_mode_seen = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "history" => cfg.history = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "history_days" => {
                cfg.history_days =
                    as_u64(val).ok().filter(|d| (1..=3650).contains(d)).ok_or_else(|| bad("number (1-3650)", val))? as u32
            }
            "history_max_mb" => {
                cfg.history_max_mb =
                    as_u64(val).ok().filter(|d| (1..=1_048_576).contains(d)).ok_or_else(|| bad("number (MB)", val))? as u32
            }
            "history_hotkey" => cfg.history_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
            "edit_save" => cfg.edit_save = one_of(val, &["", "overwrite", "new"]).map_err(|e| bad("value", &e))?,
            "copy_text_mode" => cfg.copy_text_mode = one_of(val, &["select", "all"]).map_err(|e| bad("value", &e))?,
            "redact_kinds" => cfg.redact_kinds = as_string(val).map_err(|e| bad("string", &e))?,
            "redact_style" => cfg.redact_style = one_of(val, &["solid", "blur"]).map_err(|e| bad("value", &e))?,
            "selection_mode" => cfg.selection_mode = one_of(val, &["rect", "freeform"]).map_err(|e| bad("value", &e))?,
            "upload_client_id" => {
                cfg.upload_client_id = as_string(val).map_err(|e| bad("string", &e))?
            }
            "contrast_opacity" => {
                cfg.contrast_opacity = as_u64(val)
                    .map_err(|e| bad("number", &e))?
                    .try_into()
                    .map_err(|_| bad("number", "out of range"))?
            }
            "save_subfolder" => {
                cfg.save_subfolder = as_bool(val).map_err(|e| bad("boolean", &e))?
            }
            "subfolder_pattern" => {
                cfg.subfolder_pattern = as_string(val).map_err(|e| bad("string", &e))?
            }
            "save_format" => {
                let v = as_string(val).map_err(|e| bad("string", &e))?;
                let v = v.trim().to_ascii_lowercase();
                if !matches!(v.as_str(), "png" | "jpg" | "jpeg" | "bmp") {
                    return Err(bad("value (expected \"png\", \"jpg\" or \"bmp\")", &v));
                }
                cfg.save_format = v;
            }
            "jpeg_quality" => {
                cfg.jpeg_quality = as_u64(val)
                    .ok()
                    .filter(|q| (1..=100).contains(q))
                    .ok_or_else(|| bad("number (1-100)", val))? as u8
            }
            "save_dialog" => cfg.save_dialog = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "draw_thickness" => cfg.draw_thickness = as_f64(val)? as f32,
            "draw_marker_size" => cfg.draw_marker_size = as_f64(val)? as f32,
            "draw_pixelate_size" => cfg.draw_pixelate_size = as_f64(val)? as f32,
            "draw_font_size" => cfg.draw_font_size = as_f64(val)? as f32,
            "undo_limit" => {
                cfg.undo_limit = as_u64(val)
                    .map_err(|e| bad("number", &e))?
                    .try_into()
                    .map_err(|_| bad("number", "out of range"))?
            }
            "user_colors" => cfg.user_colors = as_array(val).map_err(|e| bad("array", &e))?,
            "upload_destination" => cfg.upload_destination = one_of(val, &["imgur", "s3"]).map_err(|e| bad("value", &e))?,
            "s3_endpoint" => cfg.s3_endpoint = as_string(val).map_err(|e| bad("string", &e))?,
            "s3_region" => cfg.s3_region = as_string(val).map_err(|e| bad("string", &e))?,
            "s3_bucket" => cfg.s3_bucket = as_string(val).map_err(|e| bad("string", &e))?,
            "s3_access_key_id" => cfg.s3_access_key_id = as_string(val).map_err(|e| bad("string", &e))?,
            "s3_public_url" => cfg.s3_public_url = as_string(val).map_err(|e| bad("string", &e))?,
            "s3_key_prefix" => cfg.s3_key_prefix = as_string(val).map_err(|e| bad("string", &e))?,
            "s3_path_style" => cfg.s3_path_style = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "upload_recordings" => cfg.upload_recordings = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "copy_url_after_upload" => {
                cfg.copy_url_after_upload =
                    as_bool(val).map_err(|e| bad("boolean", &e))?
            }
            "capture_active_monitor" => {
                cfg.capture_active_monitor = as_bool(val).map_err(|e| bad("boolean", &e))?
            }
            "snap_windows" => cfg.snap_windows = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "snap_tab_hint" => cfg.snap_tab_hint = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "check_updates" => {
                cfg.check_updates = as_bool(val).map_err(|e| bad("boolean", &e))?
            }
            "download_data" => cfg.download_data = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "skip_version" => {
                cfg.skip_version = as_string(val).map_err(|e| bad("string", &e))?
            }
            "renderer" => cfg.renderer = as_string(val).map_err(|e| bad("string", &e))?,
            "rec_format" => cfg.rec_format = one_of(val, &["mp4", "gif"]).map_err(|e| bad("value", &e))?,
            "rec_fps" => cfg.rec_fps = fps_of(val, &[30, 60]).map_err(|e| bad("number", &e))?,
            "rec_gif_fps" => cfg.rec_gif_fps = fps_of(val, &[10, 15, 24]).map_err(|e| bad("number", &e))?,
            "rec_quality" => {
                cfg.rec_quality = one_of(val, &["low", "medium", "high"]).map_err(|e| bad("value", &e))?
            }
            "rec_system_audio" => cfg.rec_system_audio = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "rec_mic" => cfg.rec_mic = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "rec_mic_device" => cfg.rec_mic_device = as_string(val).map_err(|e| bad("string", &e))?,
            "rec_camera" => cfg.rec_camera = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "rec_camera_device" => cfg.rec_camera_device = as_string(val).map_err(|e| bad("string", &e))?,
            "rec_camera_shape" => {
                cfg.rec_camera_shape =
                    one_of(val, &["circle", "rounded", "square"]).map_err(|e| bad("value", &e))?
            }
            "rec_camera_size" => cfg.rec_camera_size = one_of(val, &["s", "m", "l", "xl"]).map_err(|e| bad("value", &e))?,
            "rec_camera_mirror" => cfg.rec_camera_mirror = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "rec_countdown" => cfg.rec_countdown = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "record_hotkey" => cfg.record_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
            "rec_linux_encoder" => {
                cfg.rec_linux_encoder =
                    one_of(val, &["auto", "ffmpeg", "openh264"]).map_err(|e| bad("value", &e))?
            }
            "rec_camera_ring_color" => {
                cfg.rec_camera_ring_color = as_string(val).map_err(|e| bad("string", &e))?
            }
            "rec_camera_ring_width" => {
                cfg.rec_camera_ring_width = as_u64(val)
                    .ok()
                    .filter(|w| *w <= RING_WIDTH_MAX as u64)
                    .ok_or_else(|| bad(&format!("number (0-{RING_WIDTH_MAX})"), val))? as u32
            }
            // Recorder redesign.
            "rec_gif_max" => cfg.rec_gif_max = fps_of(val, &[15, 30, 60]).map_err(|e| bad("number (15, 30 or 60)", &e))?,
            "rec_camera_bg" => cfg.rec_camera_bg = one_of(val, &["off", "light", "strong"]).map_err(|e| bad("value", &e))?,
            "rec_show_clicks" => cfg.rec_show_clicks = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "rec_show_keys" => cfg.rec_show_keys = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "rec_show_all_keys" => cfg.rec_show_all_keys = as_bool(val).map_err(|e| bad("boolean", &e))?,
            "rec_trim" => cfg.rec_trim = one_of(val, &["off", "ask"]).map_err(|e| bad("value", &e))?,
            "recent_colors" => cfg.recent_colors = as_array(val).map_err(|e| bad("array", &e))?,
            "launcher_source" => {
                cfg.launcher_source = one_of(val, &["region", "screen", "window"]).map_err(|e| bad("value", &e))?
            }
            "launcher_delay" => {
                cfg.launcher_delay = as_u64(val).ok().filter(|d| *d <= 60).ok_or_else(|| bad("number (0-60)", val))? as u32
            }
            "launcher_after" => {
                cfg.launcher_after = one_of(val, &["edit", "copy", "save", "pin"]).map_err(|e| bad("value", &e))?
            }
            "tool_callout_style" => {
                cfg.callout_style = one_of(val, &["rounded", "cloud", "ellipse"]).map_err(|e| bad("value", &e))?
            }
            k if fmt_key(k).is_some() => {
                let (id, what) = fmt_key(k).expect("a style key");
                match what {
                    FmtKey::Font => {
                        cfg.tool_fonts.insert(id.into(), one_of(val, &["sans", "serif", "mono"]).map_err(|e| bad("value", &e))?);
                    }
                    FmtKey::Style => {
                        cfg.tool_text_styles.insert(id.into(), as_string(val).map_err(|e| bad("string", &e))?);
                    }
                    FmtKey::Align => {
                        cfg.tool_aligns.insert(id.into(), one_of(val, &["left", "center", "right"]).map_err(|e| bad("value", &e))?);
                    }
                }
            }
            k => match tool_key(k) {
                Some((id, ToolKey::Fill)) => {
                    cfg.tool_fills.insert(id.to_string(), as_bool(val).map_err(|e| bad("boolean", &e))?);
                }
                Some((id, ToolKey::Size)) => {
                    let v = as_f64(val).ok().filter(|v| v.is_finite() && *v > 0.0).ok_or_else(|| bad("number", val))?;
                    cfg.tool_sizes.insert(id.to_string(), v as f32);
                }
                Some((id, ToolKey::Color)) => {
                    cfg.tool_colors.insert(id.to_string(), as_string(val).map_err(|e| bad("string", &e))?);
                }
                None => {} // unknown keys are ignored, as with serde's default
            },
        }
    }
    Ok(cfg)
}

/// Render the config as the same flat TOML subset.
pub fn to_toml(c: &Config) -> String {
    fn q(s: &str) -> String {
        toml_str(s)
    }
    let colors: Vec<String> = c.user_colors.iter().map(|s| q(s)).collect();
    let mut out = format!(
        "save_path = {}\n\
         save_subfolder = {}\n\
         subfolder_pattern = {}\n\
         filename_pattern = {}\n\
         save_format = {}\n\
         jpeg_quality = {}\n\
         save_dialog = {}\n\
         ui_color = {}\n\
         theme = {}\n\
         ui_scale = {}\n\
         contrast_opacity = {}\n\
         draw_color = {}\n\
         draw_thickness = {}\n\
         draw_marker_size = {}\n\
         draw_pixelate_size = {}\n\
         draw_font_size = {}\n\
         undo_limit = {}\n\
         capture_hotkey = {}\n\
         quit_hotkey = {}\n\
         capture_full_hotkey = {}\n\
         capture_window_hotkey = {}\n\
         repeat_hotkey = {}\n\
         capture_text_hotkey = {}\n\
         show_tray = {}\n\
         copy_after_capture = {}\n\
         shutter_sound = {}\n\
         user_colors = [{}]\n\
         upload_client_id = {}\n\
         copy_url_after_upload = {}\n\
         capture_active_monitor = {}\n\
         snap_windows = {}\n\
         snap_tab_hint = {}\n\
         check_updates = {}\n\
         download_data = {}\n\
         skip_version = {}\n\
         renderer = {}\n\
         rec_format = {}\n\
         rec_fps = {}\n\
         rec_gif_fps = {}\n\
         rec_quality = {}\n\
         rec_system_audio = {}\n\
         rec_mic = {}\n\
         rec_mic_device = {}\n\
         rec_camera = {}\n\
         rec_camera_device = {}\n\
         rec_camera_shape = {}\n\
         rec_camera_size = {}\n\
         rec_camera_mirror = {}\n\
         rec_countdown = {}\n\
         record_hotkey = {}\n\
         rec_linux_encoder = {}\n\
         rec_camera_ring_color = {}\n\
         rec_camera_ring_width = {}\n\
         recent_colors = [{}]\n",
        q(&c.save_path),
        c.save_subfolder,
        q(&c.subfolder_pattern),
        q(&c.filename_pattern),
        q(&c.save_format),
        c.jpeg_quality,
        c.save_dialog,
        q(&c.ui_color),
        q(&c.theme),
        c.ui_scale.to_toml(),
        c.contrast_opacity,
        q(&c.draw_color),
        c.draw_thickness,
        c.draw_marker_size,
        c.draw_pixelate_size,
        c.draw_font_size,
        c.undo_limit,
        q(&c.capture_hotkey),
        q(&c.quit_hotkey),
        q(&c.capture_full_hotkey),
        q(&c.capture_window_hotkey),
        q(&c.repeat_hotkey),
        q(&c.capture_text_hotkey),
        c.show_tray,
        c.copy_after_capture,
        c.shutter_sound,
        colors.join(", "),
        q(&c.upload_client_id),
        c.copy_url_after_upload,
        c.capture_active_monitor,
        c.snap_windows,
        c.snap_tab_hint,
        c.check_updates,
        c.download_data,
        q(&c.skip_version),
        q(&c.renderer),
        q(&c.rec_format),
        c.rec_fps,
        c.rec_gif_fps,
        q(&c.rec_quality),
        c.rec_system_audio,
        c.rec_mic,
        q(&c.rec_mic_device),
        c.rec_camera,
        q(&c.rec_camera_device),
        q(&c.rec_camera_shape),
        q(&c.rec_camera_size),
        c.rec_camera_mirror,
        c.rec_countdown,
        q(&c.record_hotkey),
        q(&c.rec_linux_encoder),
        q(&c.rec_camera_ring_color),
        c.rec_camera_ring_width,
        c.recent_colors.iter().map(|s| q(s)).collect::<Vec<_>>().join(", "),
    );
    out.push_str(&format!("rec_gif_max = {}
", c.rec_gif_max));
    out.push_str(&format!("rec_camera_bg = {}\n", q(&c.rec_camera_bg)));
    out.push_str(&format!(
        "rec_show_clicks = {}\nrec_show_keys = {}\nrec_show_all_keys = {}\nrec_trim = {}\n",
        c.rec_show_clicks,
        c.rec_show_keys,
        c.rec_show_all_keys,
        q(&c.rec_trim)
    ));
    out.push_str(&format!("show_thumbnail = {}\n", c.show_thumbnail));
    out.push_str(&format!("language = {}\n", q(&c.language)));
    out.push_str(&format!(
        "magnifier = {}\nmagnifier_square = {}\naccept_on_select = {}\noverlay_hints = {}\n",
        c.magnifier,
        c.magnifier_square,
        q(&c.accept_on_select),
        c.overlay_hints
    ));
    out.push_str(&format!("screenclip_hint = {}\n", c.screenclip_hint));
    out.push_str(&format!("tray_off_mode_seen = {}\n", c.tray_off_mode_seen));
    out.push_str(&format!(
        "trim_margins = {}\nhdr_screenshots = {}\nborder_color = {}\nborder_width = {}\n",
        c.trim_margins,
        q(&c.hdr_screenshots),
        q(&c.border_color),
        c.border_width
    ));
    out.push_str(&format!("save_scale = {}\nsave_scale_percent = {}\n", q(&c.save_scale), c.save_scale_percent));
    out.push_str(&format!(
        "launcher_source = {}\nlauncher_delay = {}\nlauncher_after = {}\n",
        q(&c.launcher_source),
        c.launcher_delay,
        q(&c.launcher_after)
    ));
    out.push_str(&format!(
        "upload_destination = {}\ns3_endpoint = {}\ns3_region = {}\ns3_bucket = {}\ns3_access_key_id = {}\ns3_public_url = {}\ns3_key_prefix = {}\ns3_path_style = {}\nupload_recordings = {}\n",
        q(&c.upload_destination),
        q(&c.s3_endpoint),
        q(&c.s3_region),
        q(&c.s3_bucket),
        q(&c.s3_access_key_id),
        q(&c.s3_public_url),
        q(&c.s3_key_prefix),
        c.s3_path_style,
        c.upload_recordings
    ));
    out.push_str(&format!(
        "history = {}\nhistory_days = {}\nhistory_max_mb = {}\nhistory_hotkey = {}\nedit_save = {}\n",
        c.history,
        c.history_days,
        c.history_max_mb,
        q(&c.history_hotkey),
        q(&c.edit_save)
    ));
    out.push_str(&format!(
        "copy_text_mode = {}\nredact_kinds = {}\nredact_style = {}\nselection_mode = {}\n",
        q(&c.copy_text_mode),
        q(&c.redact_kinds),
        q(&c.redact_style),
        q(&c.selection_mode)
    ));
    for id in TOOL_IDS {
        if let Some(v) = c.tool_sizes.get(id) {
            out.push_str(&format!("tool_{id}_size = {v}\n"));
        }
        if let Some(v) = c.tool_colors.get(id) {
            out.push_str(&format!("tool_{id}_color = {}\n", q(v)));
        }
        if let Some(v) = c.tool_fills.get(id) {
            out.push_str(&format!("tool_{id}_fill = {v}\n"));
        }
    }
    if !c.callout_style.is_empty() {
        out.push_str(&format!("tool_callout_style = {}\n", q(&c.callout_style)));
    }
    for id in FMT_IDS {
        for (map, what) in [(&c.tool_fonts, FmtKey::Font), (&c.tool_text_styles, FmtKey::Style), (&c.tool_aligns, FmtKey::Align)] {
            if let Some(v) = map.get(id) {
                out.push_str(&format!("{} = {}\n", fmt_key_name(id, what), q(v)));
            }
        }
    }
    if !c.shortcuts.is_empty() {
        out.push_str("
[shortcuts]
");
        for (k, v) in &c.shortcuts {
            // A key that is not a bare TOML key (e.g. one with `#` or `[`) is quoted.
            let bare = !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            let k = if bare { k.clone() } else { q(k) };
            out.push_str(&format!("{k} = {}
", q(v)));
        }
    }
    out
}

/// Split `key = value` at the first `=` outside a quoted key.
fn split_key(line: &str) -> Option<(&str, &str)> {
    let mut esc = false;
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            _ if esc => esc = false,
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            '=' if !in_str => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Cut a trailing `# comment`, respecting quotes.
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in line.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn as_string(v: &str) -> Result<String, String> {
    let v = v.trim();
    if v.len() < 2 || !v.starts_with('"') || !v.ends_with('"') {
        return Err(format!("expected quoted string, got {v}"));
    }
    let inner = &v[1..v.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(o) => out.push(o),
            None => return Err("trailing backslash".into()),
        }
    }
    Ok(out)
}

/// A quoted string that is one of `allowed` (trimmed, case-insensitive);
/// returned lowercase.
fn one_of(v: &str, allowed: &[&str]) -> Result<String, String> {
    let s = as_string(v)?.trim().to_ascii_lowercase();
    if allowed.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(format!("{v} (expected one of {})", allowed.join(", ")))
    }
}

/// A frame rate that is one of `allowed`.
fn fps_of(v: &str, allowed: &[u32]) -> Result<u32, String> {
    let n = as_u64(v)?;
    allowed
        .iter()
        .copied()
        .find(|&a| u64::from(a) == n)
        .ok_or_else(|| format!("{v} (expected one of {allowed:?})"))
}

fn as_u64(v: &str) -> Result<u64, String> {
    v.trim()
        .parse::<u64>()
        .map_err(|_| format!("expected number, got {v}"))
}

fn as_f64(v: &str) -> Result<f64, String> {
    v.trim()
        .parse::<f64>()
        .map_err(|_| format!("expected number, got {v}"))
}

fn as_bool(v: &str) -> Result<bool, String> {
    match v.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("expected true/false, got {other}")),
    }
}

fn as_array(v: &str) -> Result<Vec<String>, String> {
    let v = v.trim();
    if v.len() < 2 || !v.starts_with('[') || !v.ends_with(']') {
        return Err(format!("expected [array], got {v}"));
    }
    let inner = &v[1..v.len() - 1];
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut esc = false;
    for c in inner.chars() {
        if esc {
            cur.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' if in_str => {
                cur.push(c);
                esc = true;
            }
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            ',' if !in_str => {
                let item = cur.trim();
                if !item.is_empty() {
                    out.push(as_string(item)?);
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let item = cur.trim();
    if !item.is_empty() {
        out.push(as_string(item)?);
    }
    if in_str {
        return Err("unterminated string in array".into());
    }
    Ok(out)
}

/// Parse `#rrggbb` / `#rgb` into (r, g, b, a).
pub fn parse_color(s: &str) -> Option<(u8, u8, u8, u8)> {
    let s = s.trim();
    let hex = s.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some((r, g, b, 255))
        }
        6 | 8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = if hex.len() == 8 {
                u8::from_str_radix(&hex[6..8], 16).ok()?
            } else {
                255
            };
            Some((r, g, b, a))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_at_never_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("rustshot-readat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        let _ = std::fs::remove_file(&p);
        assert_eq!(read_at(&p), Ok(None), "missing");
        std::fs::write(&p, "theme = \"dark\"
").unwrap();
        assert_eq!(read_at(&p).unwrap().unwrap().theme, "dark");
        std::fs::write(&p, "theme = \"dark\"
oops
").unwrap();
        assert_eq!(read_at(&p), Err("line 2: expected key = value".into()));
        assert!(read_at(&dir).is_err(), "a directory cannot be read");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skip_version_edits_only_its_line() {
        let text = "# my config\nsave_format = \"jpg\" # comment\nskip_version = \"0.1.0\"\nunknown = 1\n\n[shortcuts]\nsave = \"Ctrl+S\"\n";
        let out = with_skip_version(text, "0.1.2");
        assert_eq!(out, text.replace("\"0.1.0\"", "\"0.1.2\""));
        let cfg = parse_config(&out).unwrap();
        assert_eq!((cfg.skip_version.as_str(), cfg.save_format.as_str()), ("0.1.2", "jpg"));
        assert_eq!(cfg.shortcuts.get("save").map(String::as_str), Some("Ctrl+S"));
    }

    #[test]
    fn skip_version_inserted_when_missing() {
        // Before the first table (a key after it would belong to the table).
        let out = with_skip_version("theme = \"dark\"\n[shortcuts]\nundo = \"Z\"\n", "0.1.2");
        assert_eq!(out, "theme = \"dark\"\nskip_version = \"0.1.2\"\n\n[shortcuts]\nundo = \"Z\"\n");
        assert_eq!(parse_config(&out).unwrap().skip_version, "0.1.2");
        // Appended; a missing final newline is supplied; CRLF kept.
        assert_eq!(with_skip_version("theme = \"dark\"", "1.0.0"), "theme = \"dark\"\nskip_version = \"1.0.0\"\n");
        assert_eq!(with_skip_version("a = 1\r\n", "1.0.0"), "a = 1\r\nskip_version = \"1.0.0\"\r\n");
        assert_eq!(with_skip_version("", "1.0.0"), "skip_version = \"1.0.0\"\n");
        // A `skip_version` inside a table is not the top-level key.
        let t = "[other]\nskip_version = \"x\"\n";
        assert_eq!(with_skip_version(t, "2.0.0"), format!("skip_version = \"2.0.0\"\n\n{t}"));
        // Duplicates collapse to one line; a commented-out key is left alone.
        let d = with_skip_version("# skip_version = \"0\"\nskip_version = \"a\"\nskip_version = \"b\"\n", "3.0.0");
        assert_eq!(d, "# skip_version = \"0\"\nskip_version = \"3.0.0\"\n");
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rustshot-cfg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Only the target file is in `dir` (no temp file left behind).
    fn only(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        // The advisory lock's sidecar stays (see `with_config_lock`).
        v.retain(|n| !n.ends_with(".lock"));
        v.sort();
        v
    }

    #[test]
    fn write_atomic_replaces_the_file() {
        let dir = scratch("atomic");
        let p = dir.join("config.toml");
        std::fs::write(&p, "old").unwrap();
        write_atomic(&p, b"new").unwrap();
        write_atomic(&p, b"newer").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "newer");
        assert_eq!(only(&dir), ["config.toml"]);
        // A missing file is created.
        write_atomic(&dir.join("fresh.toml"), b"x").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("fresh.toml")).unwrap(), "x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_mode_and_writes_through_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("atomic-unix");
        let real = dir.join("real.toml");
        std::fs::write(&real, "old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink("real.toml", &link).unwrap();
        write_atomic(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link stays");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(only(&dir), ["config.toml", "real.toml"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn write_atomic_writes_through_symlinks() {
        let dir = scratch("atomic-link");
        let real = dir.join("real.toml");
        std::fs::write(&real, "old").unwrap();
        let link = dir.join("config.toml");
        // Needs Developer Mode or admin rights; skip when not allowed.
        if let Err(e) = std::os::windows::fs::symlink_file(&real, &link) {
            eprintln!("skipped write_atomic_writes_through_symlinks: cannot create a symlink ({e})");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        write_atomic(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link stays");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        assert_eq!(only(&dir), ["config.toml", "real.toml"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_default_hotkeys_migrate_by_chord() {
        // The user's file: Record on the pre-0.2.0 default, written in
        // another order; full screen on today's default; window on a key
        // they chose; Copy text on the old T.
        let text = "# mine\nrecord_hotkey = \"Meta+Shift+R\"\ncapture_full_hotkey = \"Shift+Meta+F\"\ncapture_window_hotkey = \"Ctrl+Alt+W\"\ncapture_text_hotkey = \"shift+win+t\"\nbogus = 1\n";
        let mut cfg = parse_config(text).unwrap();
        let moved = migrate_old_defaults(&mut cfg);
        assert_eq!(
            moved,
            [
                ("record_hotkey", "Meta+Shift+R".to_string(), "Shift+Meta+G".to_string()),
                ("capture_text_hotkey", "shift+win+t".to_string(), "Shift+Meta+Z".to_string()),
            ]
        );
        assert_eq!((cfg.record_hotkey.as_str(), cfg.capture_text_hotkey.as_str()), ("Shift+Meta+G", "Shift+Meta+Z"));
        assert_eq!((cfg.capture_full_hotkey.as_str(), cfg.capture_window_hotkey.as_str()), ("Shift+Meta+F", "Ctrl+Alt+W"), "not former defaults");
        // Once: nothing moves the second time.
        assert!(migrate_old_defaults(&mut cfg).is_empty());
        // Win+Shift+W was Capture window's default.
        let mut w = parse_config("capture_window_hotkey = \"Shift+Meta+W\"\n").unwrap();
        assert_eq!(migrate_old_defaults(&mut w).len(), 1);
        assert_eq!(w.capture_window_hotkey, "Shift+Meta+E");
        // Today's default is another global's (the user's full screen on
        // G): the slot's first free alternate instead, never a clash.
        let mut c = Config { record_hotkey: "Shift+Meta+R".into(), capture_full_hotkey: "Shift+Meta+G".into(), ..Config::default() };
        assert_eq!(migrate_old_defaults(&mut c), [("record_hotkey", "Shift+Meta+R".to_string(), "Shift+Meta+J".to_string())]);
        let w = crate::hotkey::wanted(&c);
        for i in 0..w.len() {
            for j in 0..i {
                assert!(w[i].is_empty() || w[i] != w[j], "{w:?}");
            }
        }
        // Every candidate taken: unset.
        let mut c = Config {
            capture_hotkey: "Shift+Meta+Z".into(),
            capture_window_hotkey: "Shift+Meta+Q".into(),
            capture_full_hotkey: "Shift+Meta+Y".into(),
            history_hotkey: "Shift+Meta+J".into(),
            record_hotkey: "Shift+Meta+E".into(),
            repeat_hotkey: "Shift+Meta+G".into(),
            capture_text_hotkey: "Shift+Meta+T".into(),
            ..Config::default()
        };
        assert_eq!(migrate_old_defaults(&mut c), [("capture_text_hotkey", "Shift+Meta+T".to_string(), String::new())]);
        // Unbound, invalid or another former default's key: left alone.
        let mut c = Config { record_hotkey: String::new(), capture_text_hotkey: "Nope+".into(), capture_window_hotkey: "Shift+Meta+R".into(), ..Config::default() };
        assert!(migrate_old_defaults(&mut c).is_empty());
        // The file: only the moved lines change.
        let dir = scratch("migrate");
        let p = dir.join("config.toml");
        std::fs::write(&p, text).unwrap();
        let mut cfg = read_at(&p).unwrap().unwrap();
        migrate_at(&p, &mut cfg);
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "# mine\nrecord_hotkey = \"Shift+Meta+G\"\ncapture_full_hotkey = \"Shift+Meta+F\"\ncapture_window_hotkey = \"Ctrl+Alt+W\"\ncapture_text_hotkey = \"Shift+Meta+Z\"\nbogus = 1\n"
        );
        assert_eq!(read_at(&p).unwrap().unwrap(), cfg);
        // A file with nothing to move is not touched.
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        migrate_at(&p, &mut cfg);
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_skip_version_edits_the_file_in_place() {
        let dir = scratch("skip");
        let p = dir.join("config.toml");
        std::fs::write(&p, "# mine
theme = \"dark\"
skip_version = \"0.1.0\"
bogus = 1
").unwrap();
        save_skip_version_at(&p, "9.9.9").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "# mine
theme = \"dark\"
skip_version = \"9.9.9\"
bogus = 1
");
        assert_eq!(only(&dir), ["config.toml"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_skip_version_creates_a_missing_file() {
        let dir = scratch("skip-missing");
        let p = dir.join("sub").join("config.toml");
        save_skip_version_at(&p, "9.9.9").unwrap();
        let cfg = parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(cfg.skip_version, "9.9.9");
        assert_eq!(cfg.save_format, Config::default().save_format, "the rest are defaults");
        assert_eq!(only(&dir.join("sub")), ["config.toml"]);
        // A path that cannot be a file reports the error.
        assert!(save_skip_version_at(&dir, "1.0.0").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_writes_the_whole_config_atomically() {
        let dir = scratch("save");
        let p = dir.join("sub").join("config.toml");
        let cfg = Config {
            save_format: "jpg".into(),
            jpeg_quality: 70,
            save_path: "D:\\shots \"x\"".into(),
            shortcuts: [("save".to_string(), "Ctrl+Alt+S".to_string())].into_iter().collect(),
            ..Config::default()
        };
        save_at(&p, &cfg).unwrap();
        save_at(&p, &cfg).unwrap();
        let back = parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(to_toml(&back), to_toml(&cfg), "every value survives");
        assert_eq!(back.save_path, cfg.save_path);
        assert_eq!(only(&dir.join("sub")), ["config.toml"], "no temp file left");
        // An unwritable target fails without touching anything.
        assert!(save_at(&dir, &cfg).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewrite_loss_detection() {
        let ours = to_toml(&Config::default());
        assert!(!rewrite_loses(&ours));
        assert!(!rewrite_loses(&ours.replace('\n', "\r\n")), "line endings only");
        assert!(!rewrite_loses("theme = \"dark\"\n\nsave_format = \"jpg\"\n"), "missing keys");
        assert!(!rewrite_loses(""));
        assert!(rewrite_loses(&format!("# mine\n{ours}")), "comment");
        assert!(rewrite_loses("theme = \"dark\" # why\n"), "inline comment");
        assert!(rewrite_loses("bogus = 1\n"), "unknown key");
        assert!(rewrite_loses("[other]\nx = 1\n"), "unknown table");
        assert!(rewrite_loses("theme = 1\n"), "does not parse");
        assert!(!rewrite_loses("[shortcuts]\nsave = \"Ctrl+Alt+S\"\n"));
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#ff0000"), Some((255, 0, 0, 255)));
        assert_eq!(parse_color("#f00"), Some((255, 0, 0, 255)));
        assert_eq!(parse_color("#10203040"), Some((16, 32, 48, 64)));
        assert_eq!(parse_color("red"), None);
    }

    #[test]
    fn shortcuts_table() {
        let text = "theme = \"dark\"\n[shortcuts]\nsave_as = \"Ctrl+Alt+S\"\ncopy = \"\" # unbound\nbogus = \"N\"\n[other]\ntheme = \"light\"\n";
        let cfg = parse_config(text).unwrap();
        assert_eq!(cfg.theme, "dark", "keys of other tables are ignored");
        assert_eq!(cfg.shortcuts.len(), 3);
        assert_eq!(cfg.shortcuts["save_as"], "Ctrl+Alt+S");
        assert_eq!(cfg.shortcuts["copy"], "");
        let back = parse_config(&to_toml(&cfg)).unwrap();
        assert_eq!(back.shortcuts, cfg.shortcuts);
        assert!(!to_toml(&Config::default()).contains("[shortcuts]"));
        // Keys that are not bare TOML keys are quoted and read back.
        let odd = Config {
            shortcuts: [("a#b", "X"), ("[c]", "Y"), ("q\"=k", "Z"), ("save", "Ctrl+S")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Config::default()
        };
        let text = to_toml(&odd);
        assert!(text.contains("\"a#b\" = \"X\"") && text.contains("\"[c]\" = \"Y\"") && text.contains("\nsave = "));
        assert_eq!(parse_config(&text).unwrap().shortcuts, odd.shortcuts);
        assert!(!rewrite_loses(&text));
        let w = shortcut_warnings(&cfg);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("bogus"));
        let clash = Config {
            shortcuts: [("cancel".to_string(), "P".to_string())].into_iter().collect(),
            ..Config::default()
        };
        let w = shortcut_warnings(&clash);
        assert_eq!(w, ["[shortcuts] P is bound to tool_pencil, cancel; tool_pencil wins"]);
    }

    /// `ui_scale`: "auto" by default; a step of the list as a number or a
    /// string; anything else refused; round-trips and line-edits.
    #[test]
    fn ui_scale_parses_validates_and_roundtrips() {
        assert_eq!(Config::default().ui_scale, UiSize::Auto);
        assert!(to_toml(&Config::default()).contains("ui_scale = \"auto\"\n"));
        for (text, want) in [
            ("ui_scale = \"auto\"", UiSize::Auto),
            ("ui_scale = \"Auto\"", UiSize::Auto),
            ("ui_scale = 75", UiSize::Pct(75)),
            ("ui_scale = 90", UiSize::Pct(90)),
            ("ui_scale = 100", UiSize::Pct(100)),
            ("ui_scale = 125  # bigger", UiSize::Pct(125)),
            ("ui_scale = \"150\"", UiSize::Pct(150)),
            ("ui_scale = \"200%\"", UiSize::Pct(200)),
        ] {
            assert_eq!(parse_config(text).unwrap().ui_scale, want, "{text}");
        }
        for bad in ["ui_scale = 80", "ui_scale = 0", "ui_scale = 1.5", "ui_scale = \"big\"", "ui_scale = true", "ui_scale = 300"] {
            let e = parse_config(bad).unwrap_err();
            assert!(e.contains("ui_scale") && e.contains("\"auto\""), "{bad}: {e}");
        }
        for s in std::iter::once(UiSize::Auto).chain(crate::uisize::STEPS.map(UiSize::Pct)) {
            let text = to_toml(&Config { ui_scale: s, ..Config::default() });
            assert_eq!(parse_config(&text).unwrap().ui_scale, s);
            assert!(!rewrite_loses(&text));
        }
        // The overlay's line edit (Ctrl+=) keeps the rest of the file.
        let text = "# mine\ntheme = \"dark\"\n";
        let out = with_top_raw(text, "ui_scale", Some(&UiSize::Pct(125).to_toml()));
        assert!(out.starts_with("# mine\ntheme = \"dark\"\n"), "{out}");
        assert_eq!(parse_config(&out).unwrap().ui_scale, UiSize::Pct(125));
        let back = with_top_raw(&out, "ui_scale", Some(&UiSize::Auto.to_toml()));
        assert_eq!(parse_config(&back).unwrap().ui_scale, UiSize::Auto);
        let mut to = Config::default();
        copy_key(&Config { ui_scale: UiSize::Pct(90), ..Config::default() }, &mut to, "ui_scale");
        assert_eq!(to.ui_scale, UiSize::Pct(90));
    }

    #[test]
    fn default_config_roundtrips() {
        let text = to_toml(&Config::default());
        let back = parse_config(&text).unwrap();
        assert_eq!(back.filename_pattern, "%F_%H-%M");
        assert_eq!(back.draw_thickness, 3.0);
        assert_eq!(back.contrast_opacity, 148);
        assert_eq!(back.user_colors.len(), 10);
        assert!(back.copy_url_after_upload);
        assert!(back.check_updates);
        let off = to_toml(&Config { check_updates: false, ..Config::default() });
        assert!(!parse_config(&off).unwrap().check_updates);
        assert_eq!(back.skip_version, "");
        let skip = to_toml(&Config { skip_version: "0.1.2".into(), ..Config::default() });
        assert_eq!(parse_config(&skip).unwrap().skip_version, "0.1.2");
        assert_eq!(back.renderer, "gdi");
        let sw = to_toml(&Config { renderer: "software".into(), ..Config::default() });
        assert_eq!(parse_config(&sw).unwrap().renderer, "software");
        assert!(!parse_config(&sw).unwrap().use_gdi());
        assert_eq!(back.use_gdi(), cfg!(windows));
        let odd = Config { renderer: "vulkan".into(), ..Config::default() };
        assert_eq!(odd.use_gdi(), cfg!(windows), "unknown values fall back to gdi");
        assert!(renderer_warning("vulkan").is_some_and(|w| w.contains("\"vulkan\"")));
        for ok in ["gdi", "software", " Software "] {
            assert_eq!(renderer_warning(ok), None, "{ok}");
        }
    }

    /// The redesign's keys: the new global hotkeys (Win+Shift+X/F/E/R —
    /// the boards' Ctrl+Shift chords are common app keys —, repeat unbound), the tray, copy-after-capture and the
    /// shutter sound; a file without them gets the defaults, one with them
    /// keeps its values.
    #[test]
    fn overlay_helper_keys_roundtrip() {
        let d = Config::default();
        assert!(d.magnifier && !d.magnifier_square && d.accept_on_select == "off" && d.overlay_hints);
        let c = Config { magnifier: false, magnifier_square: true, accept_on_select: "save".into(), overlay_hints: false, ..Config::default() };
        let text = to_toml(&c);
        assert!(text.contains("magnifier = false\n") && text.contains("accept_on_select = \"save\"\n"), "{text}");
        assert_eq!(parse_config(&text).unwrap(), c);
        assert!(!rewrite_loses(&text));
        assert!(!rewrite_loses("accept_on_select = \"copy\"\nmagnifier_square = true\n"), "hand-written keys survive");
        assert!(parse_config("accept_on_select = \"maybe\"").is_err());
    }

    /// Trim margins, HDR screenshots and the export border: defaults,
    /// round trip, and values out of range refused.
    #[test]
    fn capture_extras_roundtrip() {
        let d = Config::default();
        assert!(!d.trim_margins && d.hdr_screenshots == "tonemap" && d.border_width == 0 && d.border_color == "#000000");
        let c = Config { trim_margins: true, hdr_screenshots: "keep".into(), border_color: "#ff8800".into(), border_width: 3, ..Config::default() };
        let text = to_toml(&c);
        assert!(text.contains("trim_margins = true\n") && text.contains("hdr_screenshots = \"keep\"\n"), "{text}");
        assert!(text.contains("border_color = \"#ff8800\"\n") && text.contains("border_width = 3\n"), "{text}");
        assert_eq!(parse_config(&text).unwrap(), c);
        assert!(!rewrite_loses(&text));
        for bad in ["border_width = 9", "border_width = -1", "border_color = \"red\"", "hdr_screenshots = \"hdr\""] {
            assert!(parse_config(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_config("border_width = 8").unwrap().border_width, 8);
    }

    /// Settings › Saving "Save at": screen pixels by default, kept.
    #[test]
    fn save_scale_keys() {
        let d = Config::default();
        assert_eq!((d.save_scale.as_str(), d.save_scale_percent), ("screen", 50));
        for (mode, pct) in [("logical", 50), ("custom", 75), ("custom", 10), ("screen", 100)] {
            let c = Config { save_scale: mode.into(), save_scale_percent: pct, ..Config::default() };
            let text = to_toml(&c);
            assert!(text.contains(&format!("save_scale = \"{mode}\"\nsave_scale_percent = {pct}\n")), "{text}");
            assert_eq!(parse_config(&text).unwrap(), c);
            assert!(!rewrite_loses(&text));
        }
        for bad in ["save_scale = \"2x\"", "save_scale_percent = 5", "save_scale_percent = 150", "save_scale_percent = \"50\""] {
            assert!(parse_config(bad).is_err(), "{bad}");
        }
    }

    /// The Win+Shift+S guide's "Don't show again": on by default, kept.
    #[test]
    fn tray_off_mode_seen_roundtrip() {
        assert!(!Config::default().tray_off_mode_seen);
        let c = Config { tray_off_mode_seen: true, ..Config::default() };
        let text = to_toml(&c);
        assert!(text.contains("tray_off_mode_seen = true\n"), "{text}");
        assert_eq!(parse_config(&text).unwrap(), c);
        assert!(parse_config("tray_off_mode_seen = 1").is_err());
    }

    #[test]
    fn screenclip_hint_roundtrip() {
        assert!(Config::default().screenclip_hint);
        let c = Config { screenclip_hint: false, ..Config::default() };
        let text = to_toml(&c);
        assert!(text.contains("screenclip_hint = false\n"), "{text}");
        assert_eq!(parse_config(&text).unwrap(), c);
        assert!(!rewrite_loses(&text));
        assert!(!parse_config("screenclip_hint = false").unwrap().screenclip_hint);
        assert!(parse_config("screenclip_hint = 2").is_err());
    }

    #[test]
    fn redesign_keys_defaults_and_roundtrip() {
        let d = Config::default();
        let hk = [d.capture_hotkey.as_str(), d.capture_full_hotkey.as_str(), d.capture_window_hotkey.as_str(), d.record_hotkey.as_str()];
        assert_eq!(hk, ["Shift+Meta+X", "Shift+Meta+F", "Shift+Meta+E", "Shift+Meta+G"]);
        for h in hk {
            let c = crate::keymap::Chord::parse(h).unwrap();
            assert!(c.meta && c.shift && !c.ctrl && !c.alt, "{h}: Win+Shift");
        }
        assert_eq!(d.repeat_hotkey, "");
        assert_eq!(d.capture_text_hotkey, "Shift+Meta+Z", "Copy text: Win+Shift+Z");
        assert!(d.show_tray && !d.copy_after_capture && !d.shutter_sound);
        let c = Config {
            capture_full_hotkey: "Ctrl+Alt+F".into(),
            capture_window_hotkey: "".into(),
            repeat_hotkey: "Ctrl+Alt+Shift+P".into(),
            capture_text_hotkey: "Ctrl+Alt+T".into(),
            show_tray: false,
            copy_after_capture: true,
            shutter_sound: true,
            ..Config::default()
        };
        assert_eq!(parse_config(&to_toml(&c)).unwrap(), c);
        assert!(!rewrite_loses(&to_toml(&c)));
        // An older file (written before these keys) keeps its own hotkeys.
        let old = parse_config("capture_hotkey = \"Shift+Meta+X\"\nrecord_hotkey = \"Meta+Shift+R\"\n").unwrap();
        assert_eq!((old.capture_hotkey.as_str(), old.record_hotkey.as_str()), ("Shift+Meta+X", "Meta+Shift+R"));
        assert_eq!(old.capture_full_hotkey, "Shift+Meta+F");
        let older = parse_config("capture_hotkey = \"Ctrl+Shift+S\"
record_hotkey = \"Ctrl+Shift+R\"
").unwrap();
        assert_eq!((older.capture_hotkey.as_str(), older.record_hotkey.as_str()), ("Ctrl+Shift+S", "Ctrl+Shift+R"), "custom keys kept");
        // 0.1.3 moved Capture window off Win+Shift+W (taken on a clean
        // Windows); a file with 0.1.2's default keeps it.
        let w = parse_config("capture_window_hotkey = \"Shift+Meta+W\"\n").unwrap();
        assert_eq!(w.capture_window_hotkey, "Shift+Meta+W", "a written key is kept");
        // 0.2.0 moved Record to Win+Shift+G and Copy text to Win+Shift+Z
        // (Win+Shift+R and T are taken on a clean Windows): a file with the
        // old keys keeps them, a file without them gets the new ones.
        let rt = parse_config("record_hotkey = \"Shift+Meta+R\"\ncapture_text_hotkey = \"Shift+Meta+T\"\n").unwrap();
        assert_eq!((rt.record_hotkey.as_str(), rt.capture_text_hotkey.as_str()), ("Shift+Meta+R", "Shift+Meta+T"), "written keys are kept");
        let none = parse_config("theme = \"dark\"\n").unwrap();
        assert_eq!((none.record_hotkey.as_str(), none.capture_text_hotkey.as_str()), ("Shift+Meta+G", "Shift+Meta+Z"));
        assert!(parse_config("show_tray = 1").is_err());
        // The thumbnail after a capture: on by default, round-trips off.
        assert!(d.show_thumbnail);
        let off = Config { show_thumbnail: false, ..Config::default() };
        assert_eq!(parse_config(&to_toml(&off)).unwrap(), off);
        assert!(!parse_config("show_thumbnail = false").unwrap().show_thumbnail);
        assert!(parse_config("show_thumbnail = \"no\"").is_err());
        // History: on, 30 days, 2 GB, Win+Shift+H; the Edit window asks.
        assert!(d.history && d.history_days == 30 && d.history_max_mb == 2048 && d.edit_save.is_empty());
        assert_eq!(d.history_hotkey, "Shift+Meta+H");
        let h = Config { history: false, history_days: 7, history_max_mb: 500, history_hotkey: "Ctrl+Alt+H".into(), edit_save: "new".into(), ..Config::default() };
        assert_eq!(parse_config(&to_toml(&h)).unwrap(), h);
        assert!(!rewrite_loses(&to_toml(&h)));
        assert!(parse_config("history_days = 0").is_err() && parse_config("edit_save = \"maybe\"").is_err());
        // Copy text lets the user select; Redact finds every kind; regions.
        assert!(d.copy_text_mode == "select" && d.redact_kinds == "all" && d.selection_mode == "rect");
        assert_eq!(d.redact_style, "solid");
        let s = Config { copy_text_mode: "all".into(), redact_kinds: "emails,ips".into(), redact_style: "blur".into(), selection_mode: "freeform".into(), ..Config::default() };
        assert_eq!(parse_config(&to_toml(&s)).unwrap(), s);
        assert!(!rewrite_loses(&to_toml(&s)));
        assert!(parse_config("copy_text_mode = \"some\"").is_err() && parse_config("selection_mode = \"circle\"").is_err());
        let w = hotkey_warnings(&Config { history_hotkey: "Nope+".into(), ..Config::default() });
        assert!(w.len() == 1 && w[0].starts_with("history_hotkey"), "{w:?}");
        let w = hotkey_warnings(&Config { repeat_hotkey: "Nope+".into(), ..Config::default() });
        assert!(w.len() == 1 && w[0].starts_with("repeat_hotkey"), "{w:?}");
        let w = hotkey_warnings(&Config { capture_text_hotkey: "Nope+".into(), ..Config::default() });
        assert!(w.len() == 1 && w[0].starts_with("capture_text_hotkey"), "{w:?}");
        assert_eq!(old.capture_text_hotkey, "Shift+Meta+Z", "an older file gets the default");
    }

    #[test]
    fn parses_comments_and_escapes() {
        let text = concat!(
            "# full-line comment\n",
            "ui_color = \"#ffffff\"  # inline comment after a color\n",
            "save_path = \"C:\\\\shots #1\"\n",
            "user_colors = [\"#fff\", \"#000\"]\n",
            "unknown_key = 42\n",
        );
        let cfg = parse_config(text).unwrap();
        assert_eq!(cfg.ui_color, "#ffffff");
        assert_eq!(cfg.save_path, "C:\\shots #1");
        assert_eq!(cfg.user_colors, vec!["#fff", "#000"]);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(parse_config("draw_thickness = \"x\"").is_err());
        assert!(parse_config("copy_url_after_upload = 1").is_err());
        assert!(parse_config("contrast_opacity = 999").is_err());
        assert!(parse_config("not a assignment").is_err());
        assert!(parse_config("save_format = \"gif\"").is_err());
        assert!(parse_config("jpeg_quality = 0").is_err());
        assert!(parse_config("jpeg_quality = 101").is_err());
        assert!(parse_config("save_subfolder = 1").is_err());
    }

    #[test]
    fn recording_keys_defaults_and_roundtrip() {
        let d = Config::default();
        assert_eq!((d.rec_format.as_str(), d.rec_fps, d.rec_gif_fps, d.rec_quality.as_str()), ("mp4", 30, 15, "medium"));
        assert!(d.rec_system_audio && !d.rec_mic && d.rec_mic_device.is_empty());
        assert!(!d.rec_camera && d.rec_camera_device.is_empty());
        assert_eq!((d.rec_camera_shape.as_str(), d.rec_camera_size.as_str()), ("circle", "m"));
        assert!(d.rec_camera_mirror && d.rec_countdown);
        assert_eq!(d.record_hotkey, "Shift+Meta+G");
        assert!(crate::keymap::Chord::parse(&d.record_hotkey).is_some());
        assert_eq!(d.rec_linux_encoder, "auto");
        assert_eq!(parse_config(&to_toml(&d)).unwrap(), d);
        let c = Config {
            rec_format: "gif".into(),
            rec_fps: 60,
            rec_gif_fps: 10,
            rec_quality: "high".into(),
            rec_system_audio: false,
            rec_mic: true,
            rec_mic_device: "{0.0.1.00000000}.{abc} \"Mic\"".into(),
            rec_camera: true,
            rec_camera_device: "\\\\?\\usb#cam".into(),
            rec_camera_shape: "rounded".into(),
            rec_camera_size: "l".into(),
            rec_camera_mirror: false,
            rec_countdown: false,
            record_hotkey: "Ctrl+Alt+R".into(),
            rec_linux_encoder: "openh264".into(),
            ..Config::default()
        };
        assert_eq!(parse_config(&to_toml(&c)).unwrap(), c);
        assert!(!rewrite_loses(&to_toml(&c)));
        // Global hotkeys: bad chords warn (the file still loads), empty = unbound.
        assert!(hotkey_warnings(&d).is_empty() && hotkey_warnings(&c).is_empty());
        let h = parse_config("capture_hotkey = \"Ctrl+Nope\"\nquit_hotkey = \"\"\nrecord_hotkey = \"Shift+\"\n").unwrap();
        let w = hotkey_warnings(&h);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].starts_with("capture_hotkey = \"Ctrl+Nope\"") && w[1].starts_with("record_hotkey"), "{w:?}");
        // Enumerations are trimmed and case-folded.
        let t = parse_config(concat!(
            "rec_format = \" GIF \"\n",
            "rec_quality = \"Low\"\n",
            "rec_camera_shape = \"SQUARE\"\n",
            "rec_camera_size = \"S\"\n",
            "rec_linux_encoder = \"FFmpeg\"\n",
        ))
        .unwrap();
        assert_eq!(
            [t.rec_format, t.rec_quality, t.rec_camera_shape, t.rec_camera_size, t.rec_linux_encoder],
            ["gif", "low", "square", "s", "ffmpeg"]
        );
    }

    #[test]
    fn recording_keys_reject_bad_values() {
        for bad in [
            "rec_format = \"webm\"",
            "rec_format = mp4",
            "rec_fps = 25",
            "rec_fps = 15",
            "rec_fps = \"30\"",
            "rec_gif_fps = 30",
            "rec_gif_fps = -1",
            "rec_quality = \"ultra\"",
            "rec_system_audio = 1",
            "rec_mic = \"yes\"",
            "rec_mic_device = 3",
            "rec_camera = 0",
            "rec_camera_device = x",
            "rec_camera_shape = \"hexagon\"",
            "rec_camera_size = \"xxl\"",
            "rec_camera_mirror = on",
            "rec_countdown = 1",
            "record_hotkey = R",
            "rec_linux_encoder = \"vaapi\"",
        ] {
            let e = parse_config(bad).expect_err(bad);
            let key = bad.split(' ').next().unwrap();
            assert!(e.contains(&format!("'{key}'")), "{bad}: {e}");
        }
        for ok in ["rec_fps = 60", "rec_gif_fps = 10", "rec_camera_shape = \"circle\"", "rec_linux_encoder = \"auto\""] {
            assert!(parse_config(ok).is_ok(), "{ok}");
        }
    }

    /// The camera bubble's background: off by default, light or strong,
    /// written and read back; anything else refused.
    #[test]
    fn camera_background_key() {
        assert_eq!(Config::default().rec_camera_bg, "off");
        assert!(to_toml(&Config::default()).contains("rec_camera_bg = \"off\"\n"));
        for v in ["off", "light", "strong"] {
            let c = parse_config(&format!("rec_camera_bg = \"{v}\"")).unwrap();
            assert_eq!(c.rec_camera_bg, v);
            assert_eq!(parse_config(&to_toml(&c)).unwrap(), c);
        }
        assert_eq!(parse_config("rec_camera_bg = \"Strong\"").unwrap().rec_camera_bg, "strong");
        for bad in ["rec_camera_bg = \"blur\"", "rec_camera_bg = 1"] {
            assert!(parse_config(bad).expect_err(bad).contains("'rec_camera_bg'"), "{bad}");
        }
        let mut to = Config::default();
        copy_key(&Config { rec_camera_bg: "light".into(), ..Config::default() }, &mut to, "rec_camera_bg");
        assert_eq!(to.rec_camera_bg, "light");
    }

    /// Show clicks / keystrokes / all keys (off by default) and Trim
    /// after recording (off or ask), written and read back.
    #[test]
    fn recording_extras_keys() {
        let d = Config::default();
        assert!(!d.rec_show_clicks && !d.rec_show_keys && !d.rec_show_all_keys);
        assert_eq!(d.rec_trim, "off");
        let t = to_toml(&d);
        for line in ["rec_show_clicks = false\n", "rec_show_keys = false\n", "rec_show_all_keys = false\n", "rec_trim = \"off\"\n"] {
            assert!(t.contains(line), "{line}");
        }
        let c = Config { rec_show_clicks: true, rec_show_keys: true, rec_show_all_keys: true, rec_trim: "ask".into(), ..d };
        assert_eq!(parse_config(&to_toml(&c)).unwrap(), c);
        assert_eq!(parse_config("rec_trim = \"Ask\"").unwrap().rec_trim, "ask");
        for bad in ["rec_trim = \"always\"", "rec_show_keys = \"yes please\""] {
            assert!(parse_config(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn tool_keys_parse_and_roundtrip() {
        let text = "tool_line_size = 5\ntool_line_color = \"#00ff00\"\ntool_marker_size = 2.5\ntool_counter_size = 9\n\
                    tool_bogus_size = 1\nrecent_colors = [\"#112233\", \"#44556680\"]\n\
                    rec_camera_ring_color = \"#ff0000\"\nrec_camera_ring_width = 0\n";
        let c = parse_config(text).unwrap();
        assert_eq!(c.tool_sizes.get("line"), Some(&5.0));
        assert_eq!(c.tool_sizes.get("marker"), Some(&2.5));
        assert_eq!(c.tool_sizes.get("counter"), Some(&9.0), "a known id");
        assert!(!c.tool_sizes.contains_key("bogus"), "unknown tools are ignored like unknown keys");
        assert_eq!(c.tool_colors.get("line").map(String::as_str), Some("#00ff00"));
        assert_eq!(c.recent_colors, ["#112233", "#44556680"]);
        assert_eq!((c.rec_camera_ring_color.as_str(), c.rec_camera_ring_width), ("#ff0000", 0));
        let back = parse_config(&to_toml(&c)).unwrap();
        assert_eq!(back, c, "every value survives");
        assert!(!rewrite_loses(&to_toml(&c)));
        let d = Config::default();
        assert_eq!((d.rec_camera_ring_color.as_str(), d.rec_camera_ring_width), ("", 2));
        assert!(d.recent_colors.is_empty() && d.tool_sizes.is_empty() && !to_toml(&d).contains("tool_"));
        for bad in ["tool_line_size = \"5\"", "tool_line_size = -1", "tool_line_size = 0", "tool_text_color = 3", "recent_colors = \"#fff\"", "rec_camera_ring_width = 17", "rec_camera_ring_width = -1", "rec_camera_ring_color = 1"] {
            let e = parse_config(bad).expect_err(bad);
            let key = bad.split(' ').next().unwrap();
            assert!(e.contains(&format!("'{key}'")), "{bad}: {e}");
        }
    }

    #[test]
    fn fill_keys_parse_roundtrip_and_copy() {
        let c = parse_config("tool_rect_fill = true\ntool_ellipse_fill = false\ntool_line_fill = true\ntool_counter_color = \"#123456\"\n").unwrap();
        assert_eq!(c.tool_fills.get("rect"), Some(&true));
        assert_eq!(c.tool_fills.get("ellipse"), Some(&false));
        assert!(!c.tool_fills.contains_key("line"), "only the shapes have a fill");
        assert_eq!(c.tool_colors.get("counter").map(String::as_str), Some("#123456"));
        let out = to_toml(&c);
        assert!(out.contains("tool_rect_fill = true\n") && out.contains("tool_ellipse_fill = false\n"), "{out}");
        assert_eq!(parse_config(&out).unwrap(), c);
        assert!(parse_config("tool_rect_fill = 1").expect_err("not a bool").contains("'tool_rect_fill'"));
        let mut to = Config::default();
        copy_key(&c, &mut to, "tool_rect_fill");
        assert_eq!(to.tool_fills.get("rect"), Some(&true));
        copy_key(&Config::default(), &mut to, "tool_rect_fill");
        assert!(to.tool_fills.is_empty(), "a removed key goes");
        let names = tool_key_names();
        assert!(names.contains(&"tool_rect_fill".to_string()) && names.contains(&"tool_counter_size".to_string()));
        assert!(names.iter().all(|k| tool_key(k).is_some() || fmt_key(k).is_some() || k == "tool_callout_style"), "{names:?}");
    }

    /// The text tools' style keys: parsed, written back, copied, reset.
    #[test]
    fn text_style_keys_parse_roundtrip_and_copy() {
        let text = "tool_text_font = \"serif\"\ntool_text_style = \"bold,underline\"\ntool_text_align = \"center\"\ntool_callout_font = \"MONO\"\ntool_callout_text_style = \"italic\"\ntool_callout_style = \"cloud\"\n";
        let c = parse_config(text).unwrap();
        assert_eq!(c.tool_fonts.get("text").map(String::as_str), Some("serif"));
        assert_eq!(c.tool_fonts.get("callout").map(String::as_str), Some("mono"), "case-insensitive");
        assert_eq!(c.tool_text_styles.get("text").map(String::as_str), Some("bold,underline"));
        assert_eq!(c.tool_text_styles.get("callout").map(String::as_str), Some("italic"));
        assert_eq!(c.tool_aligns.get("text").map(String::as_str), Some("center"));
        assert_eq!(c.callout_style, "cloud", "the bubble's shape is apart");
        assert_eq!(parse_config(&to_toml(&c)).unwrap(), c);
        assert!(parse_config("tool_text_font = \"fantasy\"").expect_err("not a family").contains("'tool_text_font'"));
        assert!(parse_config("tool_text_align = \"justify\"").is_err());
        assert!(parse_config("tool_callout_align = \"left\"").unwrap().tool_aligns.is_empty(), "a callout has no alignment key");
        let mut to = Config::default();
        for k in ["tool_text_font", "tool_text_style", "tool_text_align", "tool_callout_text_style"] {
            copy_key(&c, &mut to, k);
        }
        assert_eq!((to.tool_fonts.get("text"), to.tool_aligns.get("text")), (Some(&"serif".to_string()), Some(&"center".to_string())));
        assert_eq!(to.tool_text_styles, c.tool_text_styles);
        copy_key(&Config::default(), &mut to, "tool_text_font");
        assert!(!to.tool_fonts.contains_key("text"), "a removed key goes");
        let names = tool_key_names();
        for k in ["tool_text_font", "tool_text_style", "tool_text_align", "tool_callout_font", "tool_callout_text_style", "tool_blur_size"] {
            assert!(names.iter().any(|n| n == k), "{k} is reset with the tools");
        }
    }

    #[test]
    fn line_edits_keep_the_rest_of_the_file() {
        let text = "# mine\ntheme = \"dark\" # why\ntool_line_size = 3\nbogus = 1\n\n[shortcuts]\nsave = \"Ctrl+S\"\n";
        // Set in place.
        let out = with_top_raw(text, "tool_line_size", Some("7"));
        assert_eq!(out, text.replace("tool_line_size = 3", "tool_line_size = 7"));
        // Added after the last top-level line, before the table.
        let out = with_top_raw(text, "tool_arrow_color", Some("\"#00ff00\""));
        assert_eq!(out, "# mine\ntheme = \"dark\" # why\ntool_line_size = 3\nbogus = 1\ntool_arrow_color = \"#00ff00\"\n\n[shortcuts]\nsave = \"Ctrl+S\"\n");
        // Removed; a missing key removed changes nothing.
        let out = with_top_raw(text, "tool_line_size", None);
        assert_eq!(out, text.replace("tool_line_size = 3\n", ""));
        assert_eq!(with_top_raw(text, "tool_text_size", None), text);
        // A key inside a table is not the top-level one.
        let t = "[shortcuts]\ntool_line_size = \"x\"\n";
        assert_eq!(with_top_raw(t, "tool_line_size", Some("2")), format!("tool_line_size = 2\n\n{t}"));
        assert_eq!(with_top_raw(t, "tool_line_size", None), t);
        // CRLF kept, several keys in a row, arrays.
        let crlf = "theme = \"dark\"\r\n\r\n[shortcuts]\r\nsave = \"Ctrl+S\"\r\n";
        let out = [("tool_line_size", "4"), ("recent_colors", "[\"#fff\"]")].iter().fold(crlf.to_string(), |t, (k, v)| with_top_raw(&t, k, Some(v)));
        assert_eq!(out, "theme = \"dark\"\r\ntool_line_size = 4\r\nrecent_colors = [\"#fff\"]\r\n\r\n[shortcuts]\r\nsave = \"Ctrl+S\"\r\n");
        let c = parse_config(&out).unwrap();
        assert_eq!((c.tool_sizes.get("line"), c.recent_colors.as_slice()), (Some(&4.0), &["#fff".to_string()][..]));
        // An empty file, one without a final newline.
        assert_eq!(with_top_raw("", "a", Some("1")), "a = 1\n");
        assert_eq!(with_top_raw("theme = \"dark\"", "a", Some("1")), "theme = \"dark\"\na = 1\n");
    }

    #[test]
    fn key_edits_on_disk() {
        let dir = scratch("keyedits");
        let p = dir.join("config.toml");
        // A missing file: the defaults plus the key.
        edit_top_keys_at(&p, &[("tool_line_size".into(), Some("6".into()))]).unwrap();
        let c = parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!((c.tool_sizes.get("line"), c.theme.as_str()), (Some(&6.0), "auto"));
        // Comments survive; several edits at once; removals.
        std::fs::write(&p, "# keep me\ntheme = \"light\"\ntool_line_size = 6\n").unwrap();
        let edits = [("tool_line_size".to_string(), None), ("tool_pencil_color".to_string(), Some(toml_str("#010203")))];
        edit_top_keys_at(&p, &edits).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "# keep me\ntheme = \"light\"\ntool_pencil_color = \"#010203\"\n");
        assert_eq!(only(&dir), ["config.toml"], "no temp file left");
        // A file that does not parse is refused and left as it is.
        std::fs::write(&p, "theme = \"dark\"\noops\n").unwrap();
        let e = edit_top_keys_at(&p, &[("tool_line_size".into(), Some("2".into()))]).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "theme = \"dark\"\noops\n");
        // Through the queue (tests: synchronous).
        std::fs::write(&p, "theme = \"dark\"\n").unwrap();
        queue_key_edits(&p, vec![("recent_colors".into(), Some(toml_strings(&["#abcdef".into()])))], true);
        assert_eq!(parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap().recent_colors, ["#abcdef"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_key_moves_one_remembered_value() {
        let mut from = Config::default();
        from.tool_sizes.insert("line".into(), 9.0);
        from.tool_colors.insert("text".into(), "#123456".into());
        from.recent_colors = vec!["#fff".into()];
        from.rec_camera_ring_width = 7;
        let mut to = Config::default();
        to.tool_sizes.insert("arrow".into(), 2.0);
        for k in ["tool_line_size", "tool_text_color", "recent_colors", "rec_camera_ring_width", "tool_arrow_size"] {
            copy_key(&from, &mut to, k);
        }
        assert_eq!(to.tool_sizes.get("line"), Some(&9.0));
        assert!(!to.tool_sizes.contains_key("arrow"), "gone from `from`: gone");
        assert_eq!(to.tool_colors.get("text").map(String::as_str), Some("#123456"));
        assert_eq!((to.recent_colors.len(), to.rec_camera_ring_width), (1, 7));
    }

    #[test]
    fn saving_keys_roundtrip() {
        let d = Config::default();
        assert!(d.save_subfolder);
        assert_eq!(d.subfolder_pattern, "%F");
        assert_eq!(d.save_format, "png");
        assert_eq!(d.jpeg_quality, 90);
        assert!(!d.save_dialog);
        let back = parse_config(&to_toml(&d)).unwrap();
        assert!(back.save_subfolder);
        assert_eq!(back.subfolder_pattern, "%F");
        assert_eq!(back.save_format, "png");
        assert_eq!(back.jpeg_quality, 90);
        assert!(!back.save_dialog);
        let c = Config {
            save_subfolder: false,
            subfolder_pattern: "%Y/%m".into(),
            save_format: "jpg".into(),
            jpeg_quality: 1,
            save_dialog: true,
            ..Config::default()
        };
        let back = parse_config(&to_toml(&c)).unwrap();
        assert!(!back.save_subfolder);
        assert_eq!(back.subfolder_pattern, "%Y/%m");
        assert_eq!(back.save_format, "jpg");
        assert_eq!(back.jpeg_quality, 1);
        assert!(back.save_dialog);
        assert_eq!(parse_config("save_format = \" BMP \"").unwrap().save_format, "bmp");
        assert_eq!(parse_config("jpeg_quality = 100").unwrap().jpeg_quality, 100);
    }

    #[test]
    fn new_keys_go_after_the_last_key_not_into_a_table_comment() {
        // The comment heading the first table stays with it.
        let t = "theme = \"dark\"\n\n# Keyboard shortcuts\n[shortcuts]\nsave = \"S\"\n";
        assert_eq!(
            with_top_raw(t, "tool_line_size", Some("4")),
            "theme = \"dark\"\ntool_line_size = 4\n\n# Keyboard shortcuts\n[shortcuts]\nsave = \"S\"\n"
        );
        // No blank line: still after the key, the comment stays on the table.
        let t = "theme = \"dark\"\n# Keyboard shortcuts\n[shortcuts]\n";
        assert_eq!(with_top_raw(t, "a", Some("1")), "theme = \"dark\"\na = 1\n# Keyboard shortcuts\n[shortcuts]\n");
        // After the last key even with comments after it at the top level.
        let t = "a = 1\n# about b\nb = 2\n# trailing note\n\n# Table\n[t]\n";
        assert_eq!(with_top_raw(t, "c", Some("3")), "a = 1\n# about b\nb = 2\nc = 3\n# trailing note\n\n# Table\n[t]\n");
        // No keys: before the table's leading comment block (a file
        // header above it stays first).
        let t = "# rustshot settings\n\n# Keyboard shortcuts\n# (any order)\n[shortcuts]\n";
        assert_eq!(
            with_top_raw(t, "a", Some("1")),
            "# rustshot settings\n\na = 1\n\n# Keyboard shortcuts\n# (any order)\n[shortcuts]\n"
        );
        let t = "# Keyboard shortcuts\r\n[shortcuts]\r\n";
        assert_eq!(with_top_raw(t, "a", Some("1")), "a = 1\r\n\r\n# Keyboard shortcuts\r\n[shortcuts]\r\n");
        // No table: after the last key; a file of comments only: appended.
        assert_eq!(with_top_raw("a = 1\n# end\n", "b", Some("2")), "a = 1\nb = 2\n# end\n");
        assert_eq!(with_top_raw("# only a comment\n", "b", Some("2")), "# only a comment\nb = 2\n");
        // The defaults file: the new key lands after a key line, and every
        // comment that headed a table still does.
        let d = to_toml(&Config::default());
        let out = with_top_raw(&d, "tool_line_size", Some("9"));
        assert_eq!(parse_config(&out).unwrap().tool_sizes.get("line"), Some(&9.0));
        let (before, after) = out.split_once("tool_line_size = 9\n").unwrap();
        let prev = before.lines().last().unwrap_or("");
        assert!(!strip_comment(prev).trim().is_empty(), "after a key line: {prev:?}");
        assert!(!after.trim_start().starts_with('['), "not straight into a table");
        for (a, b) in d.lines().zip(d.lines().skip(1)) {
            if b.starts_with('[') && a.starts_with('#') {
                assert!(out.contains(&format!("{a}\n{b}")), "{a:?} still heads {b:?}");
            }
        }
    }

    /// Many threads editing different keys at once: every edit lands (the
    /// read-modify-writes never interleave).
    #[test]
    fn concurrent_key_writers_lose_nothing() {
        let dir = scratch("concurrent");
        let p = dir.join("config.toml");
        std::fs::write(&p, "# mine\ntheme = \"dark\"\n\n[shortcuts]\nsave = \"Ctrl+S\"\n").unwrap();
        let ids = ["pencil", "line", "arrow", "rect", "ellipse", "marker", "text", "counter"];
        std::thread::scope(|s| {
            for (n, id) in ids.iter().enumerate() {
                let p = &p;
                s.spawn(move || {
                    for i in 1..=20 {
                        let v = (n * 100 + i) as f32;
                        edit_top_keys_at(p, &[(format!("tool_{id}_size"), Some(toml_num(v)))]).unwrap();
                    }
                });
            }
            // Settings saving the whole file at the same time keeps them too
            // (it merges onto the file as read under the lock).
            let p = &p;
            s.spawn(move || {
                for _ in 0..10 {
                    with_config_lock(p, || {
                        let cfg = parse_config(&std::fs::read_to_string(p).unwrap()).unwrap();
                        save_at(p, &cfg).unwrap();
                    });
                }
            });
        });
        let c = parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap();
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(c.tool_sizes.get(*id), Some(&((n * 100 + 20) as f32)), "{id}");
        }
        assert_eq!(c.shortcuts.get("save").map(String::as_str), Some("Ctrl+S"));
        assert!(lock_path(&p).exists(), "the sidecar lock file");
        assert_eq!(only(&dir), ["config.toml"], "no temp file left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The advisory lock keeps out a writer of another process: while the
    /// sidecar is locked (as `rustshot settings` would hold it), an edit
    /// waits, then goes through.
    #[test]
    fn a_locked_sidecar_holds_writers_back() {
        let dir = scratch("flock");
        let p = dir.join("config.toml");
        std::fs::write(&p, "theme = \"dark\"\n").unwrap();
        let other = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(&p)).unwrap();
        other.lock().unwrap();
        let (tx, rx) = crate::chan::channel();
        let q = p.clone();
        let h = std::thread::spawn(move || {
            edit_top_keys_at(&q, &[("tool_line_size".into(), Some("5".into()))]).unwrap();
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "waits for the lock");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "theme = \"dark\"\n");
        other.unlock().unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10)).expect("written once unlocked");
        h.join().unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("tool_line_size = 5"));
        // Re-entrant on one thread: a nested write does not deadlock.
        with_config_lock(&p, || save_top_key_at(&p, "skip_version", "1.2.3").unwrap());
        assert_eq!(parse_config(&std::fs::read_to_string(&p).unwrap()).unwrap().skip_version, "1.2.3");
        drop(other);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The background writer (as the app runs it, here with its own thread
    /// and an in-memory "file"): batches apply in the order sent, `wait`
    /// returns once written or after its timeout, and a writer that is gone
    /// falls back to writing on the caller's thread. Nothing here touches
    /// the disk, takes the config lock or sleeps (the slow write is held on
    /// a gate the test opens), so the outcome does not depend on the
    /// machine or on other tests. The old version wrote config.toml 31
    /// times through `edit_top_keys_at`, each under `with_config_lock`'s
    /// process-wide mutex and with an fsync; the concurrent-writers and
    /// locked-sidecar tests hold that mutex for 170 fsynced writes and a
    /// blocked lock, so on a slow CI runner the queue missed its 10 s wait.
    #[test]
    fn key_writer_thread_order_wait_and_fallback() {
        use std::sync::Mutex;
        // (thread, value) per edit as applied, and the "file".
        static SEEN: Mutex<Vec<(std::thread::ThreadId, String)>> = Mutex::new(Vec::new());
        static FILE: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
        // Held by the test while the slow batch must not finish.
        static GATE: Mutex<()> = Mutex::new(());
        fn record(_: &std::path::Path, e: &[(String, Option<String>)]) {
            if e.iter().any(|(k, _)| k == "tool_counter_size") {
                drop(GATE.lock().unwrap_or_else(|p| p.into_inner()));
            }
            let me = std::thread::current().id();
            for (k, v) in e {
                let v = v.clone().unwrap_or_default();
                SEEN.lock().unwrap().push((me, v.clone()));
                let mut f = FILE.lock().unwrap();
                f.retain(|(fk, _)| fk != k);
                f.push((k.clone(), v));
            }
        }
        let get = |k: &str| FILE.lock().unwrap().iter().find(|(fk, _)| fk == k).map(|(_, v)| v.clone());
        let p = std::path::Path::new("config.toml");
        let long = std::time::Duration::from_secs(60); // only ever reached if the writer is broken
        let mut w = KeyWriter::new(record);
        for i in 1..=30 {
            assert!(w.send(p, vec![("tool_line_size".into(), Some(i.to_string()))], false).is_none());
        }
        let done = w.send(p, vec![("tool_rect_size".into(), Some("31".into()))], true);
        assert!(KeyWriter::wait(done, long), "written");
        let seen = std::mem::take(&mut *SEEN.lock().unwrap());
        let want: Vec<String> = (1..=31).map(|i| i.to_string()).collect();
        assert_eq!(seen.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(), want, "in the order sent");
        let main = std::thread::current().id();
        let writer = seen[0].0;
        assert!(writer != main && seen.iter().all(|(t, _)| *t == writer), "all on one writer thread");
        assert_eq!((get("tool_line_size"), get("tool_rect_size")), (Some("30".into()), Some("31".into())));
        // A slow write: `wait` gives up after its timeout (the write is
        // held until after it returns), and the write still happens.
        let gate = GATE.lock().unwrap();
        let slow = vec![("tool_counter_size".into(), Some("1".into())), ("tool_text_size".into(), Some("40".into()))];
        let done = w.send(p, slow, true);
        assert!(!KeyWriter::wait(done, std::time::Duration::from_millis(50)), "timed out");
        assert_eq!(get("tool_text_size"), None, "not written yet");
        drop(gate);
        let done = w.send(p, vec![("tool_pencil_size".into(), Some("2".into()))], true);
        assert!(KeyWriter::wait(done, long));
        assert_eq!((get("tool_text_size"), get("tool_pencil_size")), (Some("40".into()), Some("2".into())), "in order");
        // The writer is gone (its receiver dropped): written here, at once.
        let (tx, rx) = crate::chan::channel::<KeyJob>();
        drop(rx);
        w.tx = Some(tx);
        SEEN.lock().unwrap().clear();
        assert!(w.send(p, vec![("tool_arrow_size".into(), Some("7".into()))], true).is_none());
        assert!(w.tx.is_none(), "dropped; the next batch starts a new thread");
        assert_eq!(get("tool_arrow_size"), Some("7".into()));
        assert_eq!(SEEN.lock().unwrap()[0].0, main, "on the caller's thread");
        let done = w.send(p, vec![("tool_arrow_size".into(), Some("8".into()))], true);
        assert!(KeyWriter::wait(done, long) && w.tx.is_some());
        assert_eq!(get("tool_arrow_size"), Some("8".into()));
    }

    /// The writer as the app runs it, with the real file writer: a waited
    /// batch lands in config.toml and keeps the rest of the file.
    #[test]
    fn key_writer_writes_the_config_file() {
        let dir = scratch("writer");
        let p = dir.join("config.toml");
        std::fs::write(&p, "# keep\ntheme = \"dark\"\n").unwrap();
        let mut w = KeyWriter::new(write_key_edits);
        let edits = vec![("tool_line_size".into(), Some("30".into())), ("tool_rect_size".into(), Some("31".into()))];
        let done = w.send(&p, edits, true);
        assert!(KeyWriter::wait(done, std::time::Duration::from_secs(60)), "written");
        let text = std::fs::read_to_string(&p).unwrap();
        let c = parse_config(&text).unwrap();
        assert_eq!((c.tool_sizes.get("line"), c.tool_sizes.get("rect")), (Some(&30.0), Some(&31.0)));
        assert!(text.starts_with("# keep\n") && c.theme == "dark", "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
