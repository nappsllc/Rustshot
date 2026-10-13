//! "Share…": the capture (or a recording's file) handed to the system's
//! share sheet. Windows: `IDataTransferManagerInterop::ShowShareUIForWindow`
//! through hand-rolled WinRT vtables ([`win`]): the package has the title
//! "Rustshot capture", the file as a `StorageFile` and, for a picture, the
//! bitmap. macOS: `NSSharingServicePicker` at the pointer (the button just
//! clicked). Linux has no standard share sheet: the action is not offered
//! ([`supported`]).
//!
//! The file is the one the export saved, else a PNG in Open with's private
//! temporary folder ([`crate::open_with::temp_dir`], cleaned a day later).
//! The sheet needs a window that stays until it closes: a share runs on its
//! own thread with its own (invisible) owner window, and the process waits
//! for running shares before it exits ([`wait_idle`]), so a one-shot capture
//! keeps the sheet up too.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(all(windows, not(test)))]
mod win;
#[cfg(all(windows, test))]
#[allow(dead_code)] // its pure parts (the IIDs, the vtables) are tested
mod win;

#[cfg(all(target_os = "macos", not(test)))]
mod macos;

/// The title the share sheet shows for the package (Windows).
#[cfg_attr(not(windows), allow(dead_code))]
pub const TITLE: &str = "Rustshot capture";

/// How long a share may keep its owner window (and the process) alive.
pub const MAX_WAIT: Duration = Duration::from_secs(10 * 60);

/// [`MAX_WAIT`], or `RUSTSHOT_SHARE_MAX_SECS` seconds (live tests).
pub fn max_wait() -> Duration {
    std::env::var("RUSTSHOT_SHARE_MAX_SECS").ok().and_then(|s| s.parse().ok()).map_or(MAX_WAIT, Duration::from_secs)
}

/// Whether this OS has a share sheet (Windows, macOS; not Linux).
pub const fn supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

/// Shares still showing their sheet.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// One running share, counted in [`ACTIVE`] while it lives.
struct Running;

impl Running {
    fn start() -> Running {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// How many shares are showing their sheet.
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

/// Wait (at most `max`) until no share is showing its sheet: the process
/// is about to end. True when none is left.
pub fn wait_idle(max: Duration) -> bool {
    let t0 = Instant::now();
    while active() > 0 {
        if t0.elapsed() >= max {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// What a share hands over: the picture's file (PNG, also set as the
/// bitmap) or another file (a recording).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Picture,
    File,
}

/// Which kind `file` is: a PNG, JPEG or BMP is also offered as a bitmap.
pub fn kind_of(file: &Path) -> Kind {
    let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp") { Kind::Picture } else { Kind::File }
}

#[cfg(test)]
thread_local! {
    /// Tests: the files "shared" (no sheet ever comes up).
    pub static SHARED: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Show the share sheet for `file` near window `owner` (a raw handle,
/// Windows; else the pointer's monitor) and wait until it closes: on a
/// worker. `Err`: what to tell the user.
pub fn share_now(file: &Path, owner: Option<usize>) -> Result<(), String> {
    if !supported() {
        return Err("Share is not available here".into());
    }
    if !file.is_file() {
        return Err("Share failed: the file is gone".into());
    }
    let _running = Running::start();
    #[cfg(test)]
    {
        let _ = owner;
        SHARED.with(|s| s.borrow_mut().push(file.to_path_buf()));
        Ok(())
    }
    #[cfg(not(test))]
    {
        let file: PathBuf = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        imp(&file, owner, kind_of(&file)).map_err(|e| format!("Share failed: {e}"))
    }
}

#[cfg(all(windows, not(test)))]
fn imp(file: &Path, owner: Option<usize>, kind: Kind) -> Result<(), String> {
    win::share(file, owner, kind, max_wait())
}

#[cfg(all(target_os = "macos", not(test)))]
fn imp(file: &Path, _owner: Option<usize>, _kind: Kind) -> Result<(), String> {
    macos::share(file)
}

#[cfg(all(not(any(windows, target_os = "macos")), not(test)))]
fn imp(_file: &Path, _owner: Option<usize>, _kind: Kind) -> Result<(), String> {
    Err("not available here".into())
}

/// [`share_now`] on its own thread (the overlay's thread must not wait;
/// the overlay may be gone before the sheet is), a failure said in a
/// balloon.
pub fn share(file: &Path) {
    #[cfg(test)]
    let _ = share_now(file, None);
    #[cfg(not(test))]
    {
        let f = file.to_path_buf();
        // Counted before the thread starts: an exit right after waits.
        let running = Running::start();
        let _ = std::thread::Builder::new().name("rustshot-share".into()).spawn(move || {
            let r = share_now(&f, None);
            drop(running);
            if let Err(e) = r {
                eprintln!("rustshot: {e}");
                crate::tray::notify(&e);
            }
        });
    }
}

/// The file a share of `img` hands over: what the same export saved, else
/// a temporary PNG (Open with's private folder).
pub fn file_for(img: &crate::pixbuf::PixBuf, saved: Option<&Path>, cfg: &crate::config::Config) -> anyhow::Result<PathBuf> {
    crate::open_with::file_for(img, saved, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows and macOS share; Linux has no standard sheet.
    #[test]
    fn availability_per_os() {
        assert_eq!(supported(), cfg!(windows) || cfg!(target_os = "macos"));
        if cfg!(target_os = "linux") {
            assert!(!supported());
        }
    }

    /// Pictures go as a bitmap too; recordings as a file only.
    #[test]
    fn pictures_and_files() {
        assert_eq!(kind_of(Path::new("a/shot.PNG")), Kind::Picture);
        assert_eq!(kind_of(Path::new("shot.jpg")), Kind::Picture);
        assert_eq!(kind_of(Path::new("rec.mp4")), Kind::File);
        assert_eq!(kind_of(Path::new("rec.gif")), Kind::File);
        assert_eq!(kind_of(Path::new("noext")), Kind::File);
    }

    /// A running share keeps [`wait_idle`] waiting; done, it returns.
    #[test]
    fn exit_waits_for_running_shares() {
        let before = active();
        let r = Running::start();
        assert_eq!(active(), before + 1);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            drop(r);
        });
        assert!(wait_idle(Duration::from_secs(5)));
        t.join().unwrap();
        let _held = Running::start();
        assert!(!wait_idle(Duration::from_millis(60)), "gives up after max");
    }

    /// An export's Share hands over the file it saved, else a temporary
    /// PNG in Open with's private folder.
    #[test]
    fn exports_share_the_saved_file_or_a_temporary_one() {
        if !supported() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("rustshot-share-exp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = crate::pixbuf::PixBuf::from_pixel(4, 4, [9, 9, 9, 255]);
        let cfg = crate::config::Config::default();
        SHARED.with(|s| s.borrow_mut().clear());
        let tasks = [crate::export::Task::Save { path: Some(dir.join("both.png")), ask: false }, crate::export::Task::Share];
        let res = crate::export::run_export(&img, (0, 0), &tasks, &cfg);
        assert!(!res.error, "{:?}", res.messages);
        let res = crate::export::run_export(&img, (0, 0), &[crate::export::Task::Share], &cfg);
        assert!(!res.error, "{:?}", res.messages);
        let shared = SHARED.with(|s| s.borrow().clone());
        assert_eq!(shared[0], dir.join("both.png"));
        assert!(shared[1].starts_with(crate::open_with::temp_dir()) && shared[1].is_file());
        let _ = std::fs::remove_file(&shared[1]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A gone file is refused (a nice message); a real one is "shared".
    #[test]
    fn share_now_checks_the_file() {
        let dir = std::env::temp_dir().join(format!("rustshot-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.png");
        std::fs::write(&f, b"x").unwrap();
        SHARED.with(|s| s.borrow_mut().clear());
        if supported() {
            assert_eq!(share_now(&f, None), Ok(()));
            assert!(share_now(&dir.join("gone.png"), None).unwrap_err().contains("gone"));
            assert_eq!(SHARED.with(|s| s.borrow().clone()), vec![f.clone()]);
        } else {
            assert!(share_now(&f, None).is_err());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
