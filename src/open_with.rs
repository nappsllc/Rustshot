//! "Open with…": the capture as a file, handed to the system's app
//! chooser. Windows: `SHOpenWithDialog` (`OAIF_ALLOW_REGISTRATION |
//! OAIF_EXEC`: pick an app, optionally keep it, open). Linux: the desktop
//! portal's `OpenURI.OpenFile` with `ask` (its chooser; the file passed as
//! a descriptor), else `xdg-open` (the default app, no chooser). macOS:
//! `open` (the default app).
//!
//! A capture that was saved opens as that file; one that was not is
//! written to `<temp>/rustshot-open/` first. Those files are removed once
//! a day old, when the daemon starts ([`cleanup`]).

use crate::pixbuf::PixBuf;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Temporary files older than this go when the daemon starts.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// Where unsaved captures are written to be opened.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("rustshot-open")
}

/// Write `img` as a PNG named by `pattern` (the filename pattern) into
/// `dir`; its path.
pub fn write_temp(img: &PixBuf, dir: &Path, pattern: &str) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = crate::export::format_filename(pattern);
    let name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let name = if name.trim().is_empty() { "capture".to_string() } else { name };
    let path = crate::export::unique_path(&dir.join(format!("{name}.png")));
    std::fs::write(&path, img.to_png()?)?;
    Ok(path)
}

/// Remove the files in `dir` last changed more than `max_age` before
/// `now`; how many went.
pub fn cleanup_at(dir: &Path, max_age: Duration, now: SystemTime) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let old = m.modified().ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > max_age);
        if old && std::fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// The daemon started: yesterday's files go.
pub fn cleanup() {
    cleanup_at(&temp_dir(), MAX_AGE, SystemTime::now());
}

/// The file "Open with…" opens for an export of `img`: `saved` (what the
/// same export just saved) when there is one, else a new temporary file.
pub fn file_for(img: &PixBuf, saved: Option<&Path>, cfg: &crate::config::Config) -> anyhow::Result<PathBuf> {
    match saved.filter(|p| p.is_file()) {
        Some(p) => Ok(p.to_path_buf()),
        None => write_temp(img, &temp_dir(), &cfg.filename_pattern),
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: the files "opened" (no chooser ever comes up).
    pub static OPENED: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Show the system's app chooser for `file` (it opens on its own thread:
/// the dialog is modal).
pub fn open(file: &Path) {
    #[cfg(test)]
    OPENED.with(|o| o.borrow_mut().push(file.to_path_buf()));
    #[cfg(not(test))]
    {
        let f = file.to_path_buf();
        let _ = std::thread::Builder::new().name("rustshot-open-with".into()).spawn(move || {
            if let Err(e) = imp::open(&f) {
                eprintln!("rustshot: open with: {e}");
                crate::actions::open_path(&f);
            }
        });
    }
}

#[cfg(all(windows, not(test)))]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{SHOpenWithDialog, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO};

    pub fn open(file: &std::path::Path) -> Result<(), String> {
        let wide: Vec<u16> = file.as_os_str().encode_wide().chain(Some(0)).collect();
        let info = OPENASINFO { pcszFile: PCWSTR(wide.as_ptr()), pcszClass: PCWSTR::null(), oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_EXEC };
        // SAFETY: `wide` outlives the (modal) call.
        match unsafe { SHOpenWithDialog(None, &info) } {
            Ok(()) => Ok(()),
            // The user closed the chooser: not an error.
            Err(e) if e.code().0 as u32 == 0x800704C7 => Ok(()),
            Err(e) => Err(e.message()),
        }
    }
}

/// The portal's `OpenFile` call for descriptor 0 (`ask`: its chooser).
#[cfg(any(target_os = "linux", test))]
pub fn portal_open_file(token: &str) -> crate::dbus::Message {
    use crate::dbus::{Message, Value};
    let opts = Value::dict_sv(vec![("ask", Value::Bool(true)), ("handle_token", Value::str(token))]);
    Message::call(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.OpenURI",
        "OpenFile",
        vec![Value::str(""), Value::Fd(0), opts],
    )
}

#[cfg(all(target_os = "linux", not(test)))]
mod imp {
    use crate::dbus::{Bus, Conn};
    use std::time::Duration;

    /// Through the portal's chooser, else `xdg-open`.
    pub fn open(file: &std::path::Path) -> Result<(), String> {
        if portal(file).is_ok() {
            return Ok(());
        }
        std::process::Command::new("xdg-open").arg(file).spawn().map(|_| ()).map_err(|e| format!("xdg-open: {e}"))
    }

    fn portal(file: &std::path::Path) -> anyhow::Result<()> {
        let mut c = Conn::session_within(Duration::from_secs(2))?;
        let f = std::fs::File::open(file)?;
        let token = format!("rustshot_open_{}", std::process::id());
        let rule = crate::dbus::signal_rule("", "", "org.freedesktop.portal.Request", "Response");
        crate::dbus::add_match(&mut c, &rule)?;
        let mut m = super::portal_open_file(&token);
        m.push_fd(f.into());
        c.call(m, crate::dbus::CALL_TIMEOUT)?;
        // Stay connected while the chooser is up (the request ends with us).
        let _ = c.wait_signal(&|m| m.is("org.freedesktop.portal.Request", "Response"), Duration::from_secs(300));
        Ok(())
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod imp {
    pub fn open(file: &std::path::Path) -> Result<(), String> {
        std::process::Command::new("open").arg(file).spawn().map(|_| ()).map_err(|e| format!("open: {e}"))
    }
}

#[cfg(all(not(any(windows, target_os = "linux", target_os = "macos")), not(test)))]
mod imp {
    pub fn open(_file: &std::path::Path) -> Result<(), String> {
        Err("not available here".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustshot-openwith-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// An unsaved capture becomes a PNG named by the pattern (a second
    /// one gets its own name); files a day old go, newer ones stay.
    #[test]
    fn temp_files_come_and_go() {
        let dir = scratch("life");
        let img = PixBuf::from_pixel(8, 6, [1, 2, 3, 255]);
        let a = write_temp(&img, &dir, "shot").unwrap();
        let b = write_temp(&img, &dir, "shot").unwrap();
        assert_eq!(a.file_name().unwrap(), "shot.png");
        assert_ne!(a, b);
        assert_eq!(PixBuf::from_png(&std::fs::read(&a).unwrap()).unwrap().dimensions(), (8, 6));
        // Path separators in a pattern stay in the file's name.
        let c = write_temp(&img, &dir, "a/b:c").unwrap();
        assert_eq!(c.parent(), Some(dir.as_path()));
        // `a` is two days old.
        let old = SystemTime::now() - Duration::from_secs(2 * 24 * 3600);
        std::fs::File::options().write(true).open(&a).unwrap().set_modified(old).unwrap();
        assert_eq!(cleanup_at(&dir, MAX_AGE, SystemTime::now()), 1);
        assert!(!a.exists() && b.exists() && c.exists());
        // A day later the rest goes too.
        assert_eq!(cleanup_at(&dir, MAX_AGE, SystemTime::now() + Duration::from_secs(25 * 3600)), 2);
        assert_eq!(cleanup_at(&dir.join("missing"), MAX_AGE, SystemTime::now()), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A capture this export saved opens as that file; otherwise a
    /// temporary one; Open with in an export does that (no chooser here).
    #[test]
    fn the_saved_file_is_opened_as_is() {
        let dir = scratch("saved");
        std::fs::create_dir_all(&dir).unwrap();
        let saved = dir.join("mine.png");
        std::fs::write(&saved, b"x").unwrap();
        let img = PixBuf::from_pixel(4, 4, [9, 9, 9, 255]);
        let cfg = crate::config::Config::default();
        assert_eq!(file_for(&img, Some(&saved), &cfg).unwrap(), saved);
        let t = file_for(&img, Some(&dir.join("gone.png")), &cfg).unwrap();
        assert!(t.starts_with(temp_dir()) && t.is_file());
        let _ = std::fs::remove_file(&t);
        OPENED.with(|o| o.borrow_mut().clear());
        let tasks = [crate::export::Task::Save { path: Some(dir.join("both.png")), ask: false }, crate::export::Task::OpenWith];
        let res = crate::export::run_export(&img, (0, 0), &tasks, &cfg);
        assert!(!res.error, "{:?}", res.messages);
        assert_eq!(OPENED.with(|o| o.borrow().clone()), vec![dir.join("both.png")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The portal call: OpenFile(parent "", the file as fd 0, ask).
    #[test]
    fn the_portal_is_asked_to_choose() {
        let m = portal_open_file("tok1");
        assert_eq!(m.signature(), "sha{sv}");
        assert_eq!(m.member.as_deref(), Some("OpenFile"));
        assert_eq!(m.iface.as_deref(), Some("org.freedesktop.portal.OpenURI"));
        let opts = format!("{:?}", m.body[2]);
        assert!(opts.contains("\"ask\"") && opts.contains("Bool(true)") && opts.contains("tok1"), "{opts}");
    }
}
