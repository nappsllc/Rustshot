//! The user's Desktop folder, for "Save to Desktop".
//!
//! - Windows: the Desktop known folder (`SHGetKnownFolderPath`), which
//!   follows a desktop moved by OneDrive or by hand; `%USERPROFILE%\Desktop`
//!   when that fails.
//! - Linux: `XDG_DESKTOP_DIR` from `$XDG_CONFIG_HOME/user-dirs.dirs` (or
//!   `~/.config/user-dirs.dirs`), localised names included; `~/Desktop`
//!   without one.
//! - macOS: `~/Desktop`.
//!
//! The lookups are plain functions of what they read (the known folder,
//! environment variables, the file), so tests run every platform's.

use std::path::{Path, PathBuf};

/// What a lookup may read: the Windows known folder, an environment
/// variable, a file's text.
pub struct Probe<'a> {
    pub known: &'a dyn Fn() -> Option<PathBuf>,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub read: &'a dyn Fn(&Path) -> Option<String>,
}

impl Probe<'_> {
    fn var(&self, k: &str) -> Option<String> {
        (self.env)(k).filter(|v| !v.trim().is_empty())
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: the Desktop this thread's exports see (`Some(None)`: none).
    pub static TEST_DIR: std::cell::RefCell<Option<Option<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

/// The Desktop folder (not created here); `None` when there is no home.
pub fn dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(d) = TEST_DIR.with(|t| t.borrow().clone()) {
        return d;
    }
    let env = |k: &str| std::env::var(k).ok();
    let read = |p: &Path| std::fs::read_to_string(p).ok();
    let probe = Probe { known: &known_folder, env: &env, read: &read };
    #[cfg(windows)]
    return windows_dir(&probe);
    #[cfg(target_os = "macos")]
    return mac_dir(&probe);
    #[cfg(not(any(windows, target_os = "macos")))]
    return xdg_dir(&probe);
}

/// Windows: the known folder, else `%USERPROFILE%\Desktop`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_dir(p: &Probe) -> Option<PathBuf> {
    (p.known)().filter(|d| !d.as_os_str().is_empty()).or_else(|| p.var("USERPROFILE").map(|h| PathBuf::from(h).join("Desktop")))
}

/// macOS: `~/Desktop`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mac_dir(p: &Probe) -> Option<PathBuf> {
    p.var("HOME").map(|h| PathBuf::from(h).join("Desktop"))
}

/// Linux (XDG): `XDG_DESKTOP_DIR` from `user-dirs.dirs`, else `~/Desktop`.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
pub fn xdg_dir(p: &Probe) -> Option<PathBuf> {
    let home = p.var("HOME")?;
    let cfg = p.var("XDG_CONFIG_HOME").filter(|c| c.starts_with('/')).map_or_else(|| PathBuf::from(&home).join(".config"), PathBuf::from);
    let from_file = (p.read)(&cfg.join("user-dirs.dirs")).and_then(|t| user_dir(&t, "XDG_DESKTOP_DIR", &home));
    Some(from_file.unwrap_or_else(|| PathBuf::from(&home).join("Desktop")))
}

/// `key`'s folder in a `user-dirs.dirs` text: `KEY="$HOME/Folder"` or an
/// absolute `KEY="/path"` (`#` comments; the last assignment wins).
pub fn user_dir(text: &str, key: &str, home: &str) -> Option<PathBuf> {
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(v) = line.strip_prefix(key).and_then(|r| r.trim_start().strip_prefix('=')) else { continue };
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
        // Shell-style escapes (`\"`, `\\`, `\$`).
        let mut s = String::with_capacity(v.len());
        let mut chars = v.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                s.extend(chars.next());
            } else {
                s.push(c);
            }
        }
        let path = if let Some(rest) = s.strip_prefix("$HOME").or_else(|| s.strip_prefix("${HOME}")) {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() { PathBuf::from(home) } else { PathBuf::from(home).join(rest) }
        } else if s.starts_with('/') {
            PathBuf::from(&s)
        } else {
            continue; // the spec allows only these two forms
        };
        found = Some(path);
    }
    found
}

/// The Desktop known folder (Windows).
#[cfg(windows)]
fn known_folder() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        s.filter(|s| !s.is_empty()).map(PathBuf::from)
    }
}

#[cfg(not(windows))]
fn known_folder() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn probe_with<R>(known: Option<&str>, env: &[(&str, &str)], files: &[(&str, &str)], f: impl FnOnce(&Probe) -> R) -> R {
        let env: BTreeMap<String, String> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let files: BTreeMap<PathBuf, String> = files.iter().map(|(k, v)| (PathBuf::from(k), v.to_string())).collect();
        let known = known.map(PathBuf::from);
        let k = move || known.clone();
        let e = |n: &str| env.get(n).cloned();
        let r = |p: &Path| files.get(p).cloned();
        f(&Probe { known: &k, env: &e, read: &r })
    }

    #[test]
    fn windows_takes_the_known_folder_then_the_profile() {
        let od = r"C:\Users\me\OneDrive\Desktop";
        let got = probe_with(Some(od), &[("USERPROFILE", r"C:\Users\me")], &[], windows_dir);
        assert_eq!(got, Some(PathBuf::from(od)), "a redirected desktop");
        let got = probe_with(None, &[("USERPROFILE", r"C:\Users\me")], &[], windows_dir);
        assert_eq!(got, Some(PathBuf::from(r"C:\Users\me").join("Desktop")));
        assert_eq!(probe_with(Some(""), &[], &[], windows_dir), None);
    }

    #[test]
    fn xdg_reads_user_dirs() {
        let home = "/home/me";
        let file = "# written by xdg-user-dirs-update\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DESKTOP_DIR=\"$HOME/Рабочий стол\"\n";
        let got = probe_with(None, &[("HOME", home)], &[("/home/me/.config/user-dirs.dirs", file)], xdg_dir);
        assert_eq!(got, Some(PathBuf::from("/home/me/Рабочий стол")), "a localised name");
        // XDG_CONFIG_HOME moves the file.
        let got = probe_with(
            None,
            &[("HOME", home), ("XDG_CONFIG_HOME", "/cfg")],
            &[("/cfg/user-dirs.dirs", "XDG_DESKTOP_DIR=\"/data/desk\"\n"), ("/home/me/.config/user-dirs.dirs", file)],
            xdg_dir,
        );
        assert_eq!(got, Some(PathBuf::from("/data/desk")));
        // No file, or no entry: ~/Desktop.
        assert_eq!(probe_with(None, &[("HOME", home)], &[], xdg_dir), Some(PathBuf::from("/home/me/Desktop")));
        let got = probe_with(None, &[("HOME", home)], &[("/home/me/.config/user-dirs.dirs", "XDG_MUSIC_DIR=\"$HOME/M\"\n")], xdg_dir);
        assert_eq!(got, Some(PathBuf::from("/home/me/Desktop")));
        assert_eq!(probe_with(None, &[], &[], xdg_dir), None, "no home");
    }

    #[test]
    fn user_dirs_lines() {
        let h = "/home/me";
        assert_eq!(user_dir("XDG_DESKTOP_DIR=\"$HOME/\"", "XDG_DESKTOP_DIR", h), Some(PathBuf::from(h)), "disabled: the home");
        assert_eq!(user_dir("XDG_DESKTOP_DIR=\"${HOME}/D\"", "XDG_DESKTOP_DIR", h), Some(PathBuf::from("/home/me/D")));
        assert_eq!(user_dir("XDG_DESKTOP_DIR=\"$HOME/My \\\"D\\\"\"", "XDG_DESKTOP_DIR", h), Some(PathBuf::from("/home/me/My \"D\"")));
        assert_eq!(user_dir("#XDG_DESKTOP_DIR=\"/x\"\nXDG_DESKTOP_DIR=\"relative\"", "XDG_DESKTOP_DIR", h), None);
        assert_eq!(user_dir("XDG_DESKTOP_DIR=\"/a\"\nXDG_DESKTOP_DIR=\"/b\"", "XDG_DESKTOP_DIR", h), Some(PathBuf::from("/b")), "the last wins");
        assert_eq!(user_dir("XDG_DESKTOP_DIRX=\"/a\"", "XDG_DESKTOP_DIR", h), None);
    }

    #[test]
    fn mac_is_home_desktop() {
        assert_eq!(probe_with(None, &[("HOME", "/Users/me")], &[], mac_dir), Some(PathBuf::from("/Users/me/Desktop")));
    }

    /// The real lookup finds a folder on this machine (CI runners have a
    /// home; Windows' known folder exists).
    #[test]
    fn this_machine_has_one() {
        let d = dir().expect("a desktop folder");
        assert!(d.is_absolute(), "{d:?}");
    }
}
