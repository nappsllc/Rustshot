use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    /// Palette shown in the color picker.
    pub user_colors: Vec<String>,
    /// Imgur client id used for uploads.
    pub upload_client_id: String,
    /// Copy the resulting URL to the clipboard after upload.
    pub copy_url_after_upload: bool,
    /// If true, GUI captures only the monitor under the cursor even when
    /// all monitors share the same scale factor.
    pub capture_active_monitor: bool,
    /// Let the daemon check GitHub for a newer release once a day.
    pub check_updates: bool,
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
    /// Recording: GIF frame rate, 10 or 15.
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
    /// Recording: bubble size, "s", "m" or "l" (160/240/320 logical px).
    pub rec_camera_size: String,
    /// Recording: mirror the camera image.
    pub rec_camera_mirror: bool,
    /// Recording: 3-2-1 countdown before the first frame.
    pub rec_countdown: bool,
    /// Global hotkey that starts (and stops) a recording.
    pub record_hotkey: String,
    /// Linux MP4 encoder: "auto", "ffmpeg" or "openh264".
    pub rec_linux_encoder: String,
    /// Recording: the camera bubble's ring colour (`#rrggbb`); empty = the
    /// theme's accent.
    pub rec_camera_ring_color: String,
    /// Recording: the ring's width in logical px (0 = no ring).
    pub rec_camera_ring_width: u32,
    /// The colour picker's recent custom colours, newest first (at most
    /// [`RECENT_MAX`]).
    pub recent_colors: Vec<String>,
    /// Each drawing tool's last size (`tool_<id>_size`, ids in
    /// [`TOOL_IDS`]); a tool without one starts from the `draw_*` default.
    pub tool_sizes: BTreeMap<String, f32>,
    /// Each drawing tool's last colour (`tool_<id>_color`).
    pub tool_colors: BTreeMap<String, String>,
    /// Editor shortcut overrides from the `[shortcuts]` table
    /// (`action id -> "Ctrl+Shift+S"`; see `keymap`).
    pub shortcuts: BTreeMap<String, String>,
}

/// The drawing tools whose size and colour are remembered
/// (`tool_<id>_size`, `tool_<id>_color`).
pub const TOOL_IDS: [&str; 9] = ["pencil", "line", "arrow", "rect", "ellipse", "marker", "text", "counter", "pixelate"];

/// How many recent custom colours are kept.
pub const RECENT_MAX: usize = 8;

/// The camera ring's default width (logical px) and the widest it gets.
pub const RING_WIDTH_DEFAULT: u32 = 3;
pub const RING_WIDTH_MAX: u32 = 16;

/// `tool_<id>_size` / `tool_<id>_color` split into (id, is_size), for a
/// known tool id.
fn tool_key(key: &str) -> Option<(&str, bool)> {
    let rest = key.strip_prefix("tool_")?;
    let (id, size) = match rest.strip_suffix("_size") {
        Some(id) => (id, true),
        None => (rest.strip_suffix("_color")?, false),
    };
    TOOL_IDS.contains(&id).then_some((id, size))
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
            contrast_opacity: 148,
            draw_color: "#f04438".into(),
            draw_thickness: 3.0,
            draw_marker_size: 15.0,
            draw_pixelate_size: 12.0,
            draw_font_size: 16.0,
            undo_limit: 100,
            capture_hotkey: "Shift+Meta+X".into(),
            quit_hotkey: "Ctrl+Alt+Shift+Q".into(),
            user_colors: [
                "#f04438", "#ff8a1f", "#ffc532", "#2dc06f", "#19b5d6", "#3b82f6", "#8b5cf6",
                "#ec4899", "#ffffff", "#111318",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            upload_client_id: "313baf0c7b4d3ff".into(),
            copy_url_after_upload: true,
            capture_active_monitor: false,
            check_updates: true,
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
            record_hotkey: "Meta+Shift+R".into(),
            rec_linux_encoder: "auto".into(),
            rec_camera_ring_color: String::new(),
            rec_camera_ring_width: RING_WIDTH_DEFAULT,
            recent_colors: Vec::new(),
            tool_sizes: BTreeMap::new(),
            tool_colors: BTreeMap::new(),
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
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    write_atomic(path, to_toml(cfg).as_bytes())
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
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => to_toml(&Config::default()),
        Err(e) => return Err(e),
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    write_atomic(path, with_top_key(&text, key, value).as_bytes())
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
/// The line keeps its place; a new key goes after the last top-level line
/// (before the first `[table]`, a key after which would belong to it).
/// Duplicates collapse into one; comments, other keys and the line
/// endings (LF or CRLF) stay as written.
pub fn with_top_raw(text: &str, key: &str, raw: Option<&str>) -> String {
    let crlf = text.contains("\r\n");
    let nl = if crlf { "\r\n" } else { "\n" };
    let line = raw.map(|v| format!("{key} = {v}"));
    let mut out = String::with_capacity(text.len() + line.as_ref().map_or(0, |l| l.len() + 2));
    // A removal has nothing to insert.
    let mut done = line.is_none();
    let mut top = true;
    // `out.len()` after the last non-blank top-level line.
    let mut last = 0;
    for raw in text.split_inclusive('\n') {
        let body = strip_comment(raw).trim();
        if top && body.starts_with('[') {
            top = false;
            if let (false, Some(l)) = (done, &line) {
                // After the last top-level line; a blank line before the
                // table when there is none yet.
                let gap = out.len() > last;
                let ins = if gap { format!("{l}{nl}") } else { format!("{l}{nl}{nl}") };
                out.insert_str(last, &ins);
                done = true;
            }
        }
        let is_key = top && body.split_once('=').is_some_and(|(k, _)| k.trim() == key);
        if is_key {
            if let (false, Some(l)) = (done, &line) {
                out.push_str(l);
                out.push_str(if raw.ends_with('\n') { nl } else { "" });
                done = true;
            }
        } else {
            out.push_str(raw);
        }
        if top && !raw.trim().is_empty() {
            last = out.len();
        }
    }
    if let (false, Some(l)) = (done, &line) {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(nl);
        }
        out.push_str(l);
        out.push_str(nl);
    }
    out
}

/// Set (`Some(raw TOML value)`) or remove (`None`) top-level keys of the
/// config file at `path`, editing only their lines ([`with_top_raw`]);
/// written atomically. A missing file is created with the defaults first.
/// A file that does not parse is left alone (an `InvalidData` error): a
/// remembered size is never worth touching a file the user is fixing.
pub fn edit_top_keys_at(path: &std::path::Path, edits: &[(String, Option<String>)]) -> std::io::Result<()> {
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
}

/// Copy the value of the remembered key `key` (a `tool_<id>_size` or
/// `_color`, `recent_colors`, the camera ring's) from `from` to `to`.
pub fn copy_key(from: &Config, to: &mut Config, key: &str) {
    match key {
        "recent_colors" => to.recent_colors = from.recent_colors.clone(),
        "rec_camera_ring_color" => to.rec_camera_ring_color = from.rec_camera_ring_color.clone(),
        "rec_camera_ring_width" => to.rec_camera_ring_width = from.rec_camera_ring_width,
        k => match tool_key(k) {
            Some((id, true)) => match from.tool_sizes.get(id) {
                Some(v) => {
                    to.tool_sizes.insert(id.into(), *v);
                }
                None => {
                    to.tool_sizes.remove(id);
                }
            },
            Some((id, false)) => match from.tool_colors.get(id) {
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

/// The TOML text of a string value.
pub fn toml_string(s: &str) -> String {
    toml_str(s)
}

/// The TOML text of an array of strings.
pub fn toml_strings(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|s| toml_str(s)).collect::<Vec<_>>().join(", "))
}

/// One batch of key edits for [`edit_top_keys_at`], and who waits for it.
type KeyJob = (std::path::PathBuf, Vec<(String, Option<String>)>, Option<std::sync::mpsc::Sender<()>>);

/// Apply `edits` to the config file at `path` off the calling thread, in
/// order with earlier batches (one writer thread for the process); with
/// `wait`, return once it is written. Failures are reported on stderr (a
/// remembered size is not worth a dialog). Tests write synchronously.
pub fn queue_key_edits(path: &std::path::Path, edits: Vec<(String, Option<String>)>, wait: bool) {
    if edits.is_empty() {
        return;
    }
    let run = |p: &std::path::Path, e: &[(String, Option<String>)]| {
        if let Err(err) = edit_top_keys_at(p, e) {
            eprintln!("rustshot: could not save tool settings to {}: {err}", p.display());
        }
    };
    if cfg!(test) {
        run(path, &edits);
        return;
    }
    static TX: std::sync::Mutex<Option<std::sync::mpsc::Sender<KeyJob>>> = std::sync::Mutex::new(None);
    let mut tx = TX.lock().unwrap_or_else(|e| e.into_inner());
    let sender = tx.get_or_insert_with(|| {
        let (tx, rx) = std::sync::mpsc::channel::<KeyJob>();
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
    let (dtx, drx) = std::sync::mpsc::channel();
    if sender.send((path.to_path_buf(), edits.clone(), wait.then_some(dtx))).is_err() {
        // The writer is gone (it never should be): write here.
        *tx = None;
        drop(tx);
        run(path, &edits);
        return;
    }
    drop(tx);
    if wait {
        let _ = drx.recv_timeout(std::time::Duration::from_secs(5));
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
fn toml_str(s: &str) -> String {
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
    [("capture_hotkey", &cfg.capture_hotkey), ("quit_hotkey", &cfg.quit_hotkey), ("record_hotkey", &cfg.record_hotkey)]
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty() && crate::keymap::Chord::parse(v).is_none())
        .map(|(k, v)| format!("{k} = {v:?} is not a valid hotkey; it will not be registered"))
        .collect()
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
            "draw_color" => cfg.draw_color = as_string(val).map_err(|e| bad("string", &e))?,
            "capture_hotkey" => {
                cfg.capture_hotkey = as_string(val).map_err(|e| bad("string", &e))?
            }
            "quit_hotkey" => cfg.quit_hotkey = as_string(val).map_err(|e| bad("string", &e))?,
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
            "copy_url_after_upload" => {
                cfg.copy_url_after_upload =
                    as_bool(val).map_err(|e| bad("boolean", &e))?
            }
            "capture_active_monitor" => {
                cfg.capture_active_monitor = as_bool(val).map_err(|e| bad("boolean", &e))?
            }
            "check_updates" => {
                cfg.check_updates = as_bool(val).map_err(|e| bad("boolean", &e))?
            }
            "skip_version" => {
                cfg.skip_version = as_string(val).map_err(|e| bad("string", &e))?
            }
            "renderer" => cfg.renderer = as_string(val).map_err(|e| bad("string", &e))?,
            "rec_format" => cfg.rec_format = one_of(val, &["mp4", "gif"]).map_err(|e| bad("value", &e))?,
            "rec_fps" => cfg.rec_fps = fps_of(val, &[30, 60]).map_err(|e| bad("number", &e))?,
            "rec_gif_fps" => cfg.rec_gif_fps = fps_of(val, &[10, 15]).map_err(|e| bad("number", &e))?,
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
            "rec_camera_size" => cfg.rec_camera_size = one_of(val, &["s", "m", "l"]).map_err(|e| bad("value", &e))?,
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
            "recent_colors" => cfg.recent_colors = as_array(val).map_err(|e| bad("array", &e))?,
            k => match tool_key(k) {
                Some((id, true)) => {
                    let v = as_f64(val).ok().filter(|v| v.is_finite() && *v > 0.0).ok_or_else(|| bad("number", val))?;
                    cfg.tool_sizes.insert(id.to_string(), v as f32);
                }
                Some((id, false)) => {
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
         contrast_opacity = {}\n\
         draw_color = {}\n\
         draw_thickness = {}\n\
         draw_marker_size = {}\n\
         draw_pixelate_size = {}\n\
         draw_font_size = {}\n\
         undo_limit = {}\n\
         capture_hotkey = {}\n\
         quit_hotkey = {}\n\
         user_colors = [{}]\n\
         upload_client_id = {}\n\
         copy_url_after_upload = {}\n\
         capture_active_monitor = {}\n\
         check_updates = {}\n\
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
        c.contrast_opacity,
        q(&c.draw_color),
        c.draw_thickness,
        c.draw_marker_size,
        c.draw_pixelate_size,
        c.draw_font_size,
        c.undo_limit,
        q(&c.capture_hotkey),
        q(&c.quit_hotkey),
        colors.join(", "),
        q(&c.upload_client_id),
        c.copy_url_after_upload,
        c.capture_active_monitor,
        c.check_updates,
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
    for id in TOOL_IDS {
        if let Some(v) = c.tool_sizes.get(id) {
            out.push_str(&format!("tool_{id}_size = {v}\n"));
        }
        if let Some(v) = c.tool_colors.get(id) {
            out.push_str(&format!("tool_{id}_color = {}\n", q(v)));
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
        assert_eq!(d.record_hotkey, "Meta+Shift+R");
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
            "rec_camera_size = \"xl\"",
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
        assert_eq!((d.rec_camera_ring_color.as_str(), d.rec_camera_ring_width), ("", 3));
        assert!(d.recent_colors.is_empty() && d.tool_sizes.is_empty() && !to_toml(&d).contains("tool_"));
        for bad in ["tool_line_size = \"5\"", "tool_line_size = -1", "tool_line_size = 0", "tool_text_color = 3", "recent_colors = \"#fff\"", "rec_camera_ring_width = 17", "rec_camera_ring_width = -1", "rec_camera_ring_color = 1"] {
            let e = parse_config(bad).expect_err(bad);
            let key = bad.split(' ').next().unwrap();
            assert!(e.contains(&format!("'{key}'")), "{bad}: {e}");
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
        let edits = [("tool_line_size".to_string(), None), ("tool_pencil_color".to_string(), Some(toml_string("#010203")))];
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
}
