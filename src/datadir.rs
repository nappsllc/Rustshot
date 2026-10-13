//! Where Rustshot's data files live (translations in `lang/`, models...),
//! highest priority first; [`find`] takes the first that has a file.
//!
//! - `RUSTSHOT_DATA_DIR` when set (tests, development, a portable copy).
//! - Windows: the folder of rustshot.exe (installer, portable zip, the
//!   MSIX package root).
//! - macOS: `Rustshot.app/Contents/Resources`, then the executable's folder.
//! - Linux: `$XDG_DATA_HOME/rustshot` (`~/.local/share/rustshot`: user
//!   overrides), the AppImage's, Snap's and Flatpak's `share/rustshot`,
//!   each `$XDG_DATA_DIRS` entry's `rustshot` (default
//!   `/usr/local/share:/usr/share`), then `../share/rustshot` beside the
//!   executable.

use std::path::{Path, PathBuf};

/// The data folders, highest priority first (existing or not).
pub fn data_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("RUSTSHOT_DATA_DIR").filter(|d| !d.is_empty()) {
        v.push(PathBuf::from(d));
    }
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    #[cfg(windows)]
    v.extend(exe_dir);
    #[cfg(target_os = "macos")]
    if let Some(d) = exe_dir {
        v.push(d.join("..").join("Resources"));
        v.push(d);
    }
    #[cfg(target_os = "linux")]
    {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        match env("XDG_DATA_HOME") {
            Some(d) => v.push(d.join("rustshot")),
            None => v.extend(env("HOME").map(|h| h.join(".local/share/rustshot"))),
        }
        v.extend(env("APPDIR").map(|d| d.join("usr/share/rustshot")));
        v.extend(env("SNAP").map(|d| d.join("usr/share/rustshot")));
        if std::path::Path::new("/.flatpak-info").exists() {
            v.push(PathBuf::from("/app/share/rustshot"));
        }
        let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        v.extend(dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("rustshot")));
        v.extend(exe_dir.map(|d| d.join("../share/rustshot")));
    }
    let mut seen = Vec::new();
    v.retain(|d| {
        let new = !seen.contains(d);
        seen.push(d.clone());
        new
    });
    v
}

/// The per-user folder downloaded data files are kept in
/// (`%LOCALAPPDATA%\\rustshot\\data`, `~/Library/Application
/// Support/rustshot/data`, `$XDG_DATA_HOME/rustshot/data`).
pub fn cache_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    #[cfg(windows)]
    let base = env("LOCALAPPDATA");
    #[cfg(target_os = "macos")]
    let base = env("HOME").map(|h| h.join("Library/Application Support"));
    #[cfg(target_os = "linux")]
    let base = env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")));
    base.map(|b| b.join("rustshot").join("data"))
}

/// The downloaded copy of `rel` in [`cache_dir`], if it is there.
pub fn cached(rel: &str) -> Option<PathBuf> {
    cache_dir().map(|d| d.join(rel)).filter(|p| p.is_file())
}

/// The first data folder's `rel` that exists.
#[allow(dead_code)]
pub fn find(rel: &str) -> Option<PathBuf> {
    data_dirs().into_iter().map(|d| d.join(rel)).find(|p| p.exists())
}
