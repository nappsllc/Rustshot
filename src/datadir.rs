//! Where Rustshot keeps the data files it downloads on demand (see
//! [`crate::fetch`]): the camera blur's model (`models/selfie.bin`) and the
//! language files (`lang/<tag>.txt`).
//!
//! - The cache, where downloads go ([`cache_dir`]): Windows
//!   `%LOCALAPPDATA%\rustshot-data`, Linux `$XDG_DATA_HOME/rustshot`
//!   (`~/.local/share/rustshot`), macOS `~/Library/Application
//!   Support/rustshot`. A file put there by hand (downloads off) counts
//!   once it has the expected SHA-256.
//! - `RUSTSHOT_DATA_DIR`, the user's override folder (a translator's
//!   working copy, the source tree): looked in first; a language file
//!   there is taken as it is, the model is still checked.

use std::path::PathBuf;

/// The override folder's variable.
pub const DIR_VAR: &str = "RUSTSHOT_DATA_DIR";

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The download cache (None: no home folder to keep it in).
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_from(&env_path)
}

fn cache_dir_from(get: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    #[cfg(windows)]
    let base = get("LOCALAPPDATA").or_else(|| get("USERPROFILE").map(|h| h.join("AppData").join("Local")));
    #[cfg(target_os = "macos")]
    let base = get("HOME").map(|h| h.join("Library").join("Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = get("XDG_DATA_HOME").or_else(|| get("HOME").map(|h| h.join(".local").join("share")));
    // (`rustshot-data` on Windows: `%LOCALAPPDATA%\rustshot` is the
    // installer's program folder.)
    base.map(|b| b.join(if cfg!(windows) { "rustshot-data" } else { "rustshot" }))
}

/// The user's override folder (`RUSTSHOT_DATA_DIR`), if set.
pub fn override_dir() -> Option<PathBuf> {
    env_path(DIR_VAR)
}

/// The folders looked in, override first, then the cache.
#[cfg_attr(not(test), allow(dead_code))] // the i18n branch lists language files
pub fn data_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = override_dir().into_iter().collect();
    v.extend(cache_dir().filter(|c| !v.contains(c)));
    v
}

/// `rel` is a plain relative path of our own: `folder/name`, ASCII, no
/// traversal, no Windows device name (`CON`, `nul.txt`, `COM1`…) and no
/// trailing dot (Win32 drops it: `de.txt.` is `de.txt`).
pub fn safe_rel(rel: &str) -> bool {
    let device = |s: &str| {
        let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit())
    };
    let ok = |s: &str| !s.is_empty() && !s.starts_with('.') && !s.ends_with('.') && !device(s) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    rel.split('/').count() <= 2 && rel.split('/').all(ok)
}

/// The usable copy of `rel` (e.g. `"lang/de.txt"`), if there is one:
/// the override folder's (taken as it is, unless `rel` is a file that is
/// always checked: the model), else the cache's when it has the SHA-256
/// this version expects ([`crate::fetch::manifest`]). Never downloads
/// (that is [`crate::fetch::ensure`]).
#[cfg_attr(not(test), allow(dead_code))] // the language files (the i18n branch)
pub fn cached(rel: &str) -> Option<PathBuf> {
    crate::fetch::global().find_rel(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_per_os() {
        let env = |k: &str| match k {
            "LOCALAPPDATA" => Some(PathBuf::from(r"C:\Users\u\AppData\Local")),
            "HOME" => Some(PathBuf::from("/home/u")),
            _ => None,
        };
        let d = cache_dir_from(&env).unwrap();
        #[cfg(windows)]
        assert_eq!(d, PathBuf::from(r"C:\Users\u\AppData\Local\rustshot-data"));
        #[cfg(target_os = "macos")]
        assert_eq!(d, PathBuf::from("/home/u/Library/Application Support/rustshot"));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert_eq!(d, PathBuf::from("/home/u/.local/share/rustshot"));
            let xdg = |k: &str| (k == "XDG_DATA_HOME").then(|| PathBuf::from("/x"));
            assert_eq!(cache_dir_from(&xdg), Some(PathBuf::from("/x/rustshot")));
        }
        assert_eq!(cache_dir_from(&|_| None), None);
    }

    #[test]
    fn looked_in_override_then_cache() {
        let v = data_dirs();
        assert_eq!(v.last(), cache_dir().as_ref());
        assert!(v.len() <= 2);
    }

    #[test]
    fn rel_paths_are_plain() {
        assert!(safe_rel("lang/de.txt"));
        assert!(safe_rel("models/selfie.bin"));
        assert!(safe_rel("lang/pt-BR.txt"));
        assert!(safe_rel("lang/console.txt") && safe_rel("lang/com10.txt"));
        for bad in ["", "../x", "lang/../x", "/etc/passwd", r"lang\de.txt", "a/b/c", "lang/.hidden", "lang/", "C:/x", "lang/CON.txt", "lang/nul", "lang/com1.txt", "LPT9/x", "lang/aux.tar.gz", "lang/de.txt.", "lang./de.txt"] {
            assert!(!safe_rel(bad), "{bad}");
        }
    }
}
