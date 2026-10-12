//! "Open with…": the capture as a file, handed to the system's app
//! chooser. Windows: `SHOpenWithDialog` (`OAIF_ALLOW_REGISTRATION |
//! OAIF_EXEC`: pick an app, optionally keep it, open) on an STA thread,
//! owned by the window that asked. Linux: the desktop portal's
//! `OpenURI.OpenFile` with `ask` (its chooser; the file passed as a
//! descriptor), else `xdg-open` (the default app, no chooser: the portal
//! is not there). macOS: `open` (the default app).
//!
//! A capture that was saved opens as that file; one that was not is
//! written to a private folder first: `%TEMP%\rustshot-open` (Windows,
//! per user), `$XDG_RUNTIME_DIR/rustshot-open` (Linux; else under the
//! cache folder), `$TMPDIR/rustshot-open` (macOS, per user). On Unix the
//! folder is 0700 and must be ours (a pre-made one that is not is
//! refused) and the files 0600. They are removed once a day old, when the
//! daemon starts ([`cleanup`]).

use crate::pixbuf::PixBuf;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Temporary files older than this go when the daemon starts.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// Where unsaved captures are written to be opened.
pub fn temp_dir() -> PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let abs = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
        if let Some(r) = abs("XDG_RUNTIME_DIR").filter(|p| p.is_dir()) {
            return r.join("rustshot-open");
        }
        if let Some(c) = abs("XDG_CACHE_HOME").or_else(|| abs("HOME").map(|h| h.join(".cache"))) {
            return c.join("rustshot").join("open");
        }
    }
    std::env::temp_dir().join("rustshot-open")
}

/// Whether a folder with these facts may hold captures: a real folder
/// (not a link), ours, closed to everyone else.
#[cfg_attr(not(unix), allow(dead_code))] // Unix folders (tests everywhere)
pub fn private_ok(is_dir: bool, is_link: bool, owner: u32, me: u32, mode: u32) -> Result<(), String> {
    if is_link || !is_dir {
        return Err("it is not a folder".into());
    }
    if owner != me {
        return Err("it belongs to another user".into());
    }
    if mode & 0o077 != 0 {
        return Err(format!("others can open it (mode {:o})", mode & 0o777));
    }
    Ok(())
}

/// Make `dir` (0700 on Unix) and check it is private.
fn private_dir(dir: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        if let Some(p) = dir.parent() {
            std::fs::create_dir_all(p)?;
        }
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let m = std::fs::symlink_metadata(dir)?;
        // SAFETY: geteuid has no preconditions.
        let me = unsafe { geteuid() };
        private_ok(m.is_dir(), m.file_type().is_symlink(), m.uid(), me, m.mode()).map_err(|e| anyhow::anyhow!("{} is not safe for captures: {e}", dir.display()))
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)?;
        Ok(())
    }
}

/// Write `img` as a PNG named by `pattern` (the filename pattern) into
/// the private folder `dir`; its path.
pub fn write_temp(img: &PixBuf, dir: &Path, pattern: &str) -> anyhow::Result<PathBuf> {
    private_dir(dir)?;
    let name = crate::export::format_filename(pattern);
    let name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let name = if name.trim().is_empty() { "capture".to_string() } else { name };
    let path = crate::export::unique_path(&dir.join(format!("{name}.png")));
    let png = img.to_png()?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(&path)?, &png)?;
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

/// Show the system's app chooser for `file`, owned by window `owner`
/// (a raw handle; Windows), and wait until it is closed: on a worker.
/// `Err`: what to tell the user.
pub fn open_now(file: &Path, owner: Option<usize>) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = owner;
        OPENED.with(|o| o.borrow_mut().push(file.to_path_buf()));
        Ok(())
    }
    #[cfg(not(test))]
    imp::open(file, owner).map_err(|e| format!("Open with failed: {e}"))
}

/// [`open_now`] on its own thread (the overlay's thread must not wait),
/// a failure said in a balloon.
pub fn open(file: &Path) {
    #[cfg(test)]
    let _ = open_now(file, None);
    #[cfg(not(test))]
    {
        let f = file.to_path_buf();
        let _ = std::thread::Builder::new().name("rustshot-open-with".into()).spawn(move || {
            if let Err(e) = open_now(&f, None) {
                eprintln!("rustshot: {e}");
                crate::tray::notify(&e);
            }
        });
    }
}

#[cfg(all(windows, not(test)))]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::{SHOpenWithDialog, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO};

    pub fn open(file: &std::path::Path, owner: Option<usize>) -> Result<(), String> {
        let wide: Vec<u16> = file.as_os_str().encode_wide().chain(Some(0)).collect();
        let info = OPENASINFO { pcszFile: PCWSTR(wide.as_ptr()), pcszClass: PCWSTR::null(), oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_EXEC };
        // The shell's dialogs want a single-threaded apartment; balanced
        // below when this call set one up (S_OK or S_FALSE).
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        let hwnd = owner.filter(|h| *h != 0).map(|h| HWND(h as *mut core::ffi::c_void));
        // SAFETY: `wide` outlives the (modal) call.
        let r = match unsafe { SHOpenWithDialog(hwnd, &info) } {
            Ok(()) => Ok(()),
            // The user closed the chooser: not an error.
            Err(e) if e.code().0 as u32 == 0x800704C7 => Ok(()),
            Err(e) => Err(e.message()),
        };
        if com.is_ok() {
            unsafe { CoUninitialize() };
        }
        r
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

    /// Through the portal's chooser; `xdg-open` (no chooser) only where
    /// there is no portal to ask.
    pub fn open(file: &std::path::Path, _owner: Option<usize>) -> Result<(), String> {
        match portal(file) {
            Ok(()) => Ok(()),
            Err(Missing::No) => {
                let st = std::process::Command::new("xdg-open").arg(file).status().map_err(|e| format!("xdg-open: {e}"))?;
                if st.success() { Ok(()) } else { Err(format!("xdg-open: {st}")) }
            }
            Err(Missing::Failed(e)) => Err(e),
        }
    }

    enum Missing {
        /// No portal (or no OpenURI): the default app instead.
        No,
        Failed(String),
    }

    fn portal(file: &std::path::Path) -> Result<(), Missing> {
        let mut c = Conn::session_within(Duration::from_secs(2)).map_err(|_| Missing::No)?;
        let f = std::fs::File::open(file).map_err(|e| Missing::Failed(e.to_string()))?;
        let token = format!("rustshot_open_{}", std::process::id());
        let rule = crate::dbus::signal_rule("", "", "org.freedesktop.portal.Request", "Response");
        crate::dbus::add_match(&mut c, &rule).map_err(|_| Missing::No)?;
        let mut m = super::portal_open_file(&token);
        m.push_fd(f.into());
        if let Err(e) = c.call(m, crate::dbus::CALL_TIMEOUT) {
            let s = e.to_string();
            // Not there (no portal, no OpenURI): fall back; else say why.
            return Err(if s.contains("ServiceUnknown") || s.contains("UnknownMethod") || s.contains("UnknownInterface") || s.contains("NameHasNoOwner") {
                Missing::No
            } else {
                Missing::Failed(s)
            });
        }
        // Stay connected while the chooser is up (the request ends with us).
        let _ = c.wait_signal(&|m| m.is("org.freedesktop.portal.Request", "Response"), Duration::from_secs(300));
        Ok(())
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod imp {
    pub fn open(file: &std::path::Path, _owner: Option<usize>) -> Result<(), String> {
        let st = std::process::Command::new("open").arg(file).status().map_err(|e| format!("open: {e}"))?;
        if st.success() { Ok(()) } else { Err(format!("open: {st}")) }
    }
}

#[cfg(all(not(any(windows, target_os = "linux", target_os = "macos")), not(test)))]
mod imp {
    pub fn open(_file: &std::path::Path, _owner: Option<usize>) -> Result<(), String> {
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

    /// Only a real folder of ours that no one else can open holds captures.
    #[test]
    fn the_folder_must_be_private() {
        assert_eq!(private_ok(true, false, 1000, 1000, 0o40700), Ok(()));
        assert!(private_ok(true, true, 1000, 1000, 0o700).is_err(), "a link");
        assert!(private_ok(false, false, 1000, 1000, 0o700).is_err(), "a file");
        assert!(private_ok(true, false, 0, 1000, 0o700).unwrap_err().contains("another user"));
        assert!(private_ok(true, false, 1000, 1000, 0o755).unwrap_err().contains("755"));
        assert!(private_ok(true, false, 1000, 1000, 0o710).is_err());
    }

    /// Unix: the folder is made 0700, the files 0600; a pre-made open
    /// folder is refused.
    #[cfg(unix)]
    #[test]
    fn temp_files_are_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perms");
        let img = PixBuf::from_pixel(2, 2, [0, 0, 0, 255]);
        let f = write_temp(&img, &dir, "p").unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(write_temp(&img, &dir, "p").is_err(), "an open folder is refused");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
