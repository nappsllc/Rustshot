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
//! One sheet at a time ([`BUSY`]). The sheet needs a window that stays
//! while it may still ask for the data: a share runs on its own thread
//! with its own (invisible) owner window. The caller waits until the share
//! ends or a time limit passes; the process waits for such callers before
//! it exits ([`wait_idle`]), so a one-shot capture keeps the sheet up too.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(all(windows, not(test)))]
mod win;
#[cfg(all(windows, test))]
#[allow(dead_code)] // its pure parts (the IIDs, the vtables, the timing) are tested
mod win;

#[cfg(all(target_os = "macos", not(test)))]
mod macos;

/// The title the share sheet shows for the package (Windows).
#[cfg_attr(not(windows), allow(dead_code))]
pub const TITLE: &str = "Rustshot capture";

/// What a second share says while a sheet is up.
pub const BUSY: &str = "A share sheet is already open";

/// How long a share may keep its caller (and the process) waiting.
pub const MAX_WAIT: Duration = Duration::from_secs(10 * 60);

/// [`MAX_WAIT`], or `RUSTSHOT_SHARE_MAX_SECS` seconds (live tests; at most
/// an hour).
pub fn max_wait() -> Duration {
    parse_max(std::env::var("RUSTSHOT_SHARE_MAX_SECS").ok().as_deref())
}

fn parse_max(v: Option<&str>) -> Duration {
    v.and_then(|s| s.trim().parse::<u64>().ok()).map_or(MAX_WAIT, |s| Duration::from_secs(s.min(3600)))
}

/// Whether this OS has a share sheet (Windows, macOS; not Linux).
pub const fn supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

/// Callers waiting for their share.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// One waiting caller, counted in [`ACTIVE`] while it lives.
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

/// How many callers wait for their share.
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

/// Wait (at most `max`) until no caller waits for a share: the process
/// is about to end. True when none is left. On macOS the main thread
/// serves its events meanwhile (the picker runs there).
pub fn wait_idle(max: Duration) -> bool {
    let t0 = Instant::now();
    while active() > 0 {
        if t0.elapsed() >= max {
            return false;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        macos::serve(Duration::from_millis(50));
        #[cfg(not(all(target_os = "macos", not(test))))]
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// The one sheet this process may have up ([`BUSY`]): held from the
/// request until the share ends or its caller is let go.
pub struct Sheet(());

#[cfg(not(test))]
static SHEET_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    // Per test thread (the tests run side by side).
    static SHEET_UP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Sheet {
    /// The sheet, unless one is up.
    pub fn claim() -> Option<Sheet> {
        #[cfg(not(test))]
        let free = SHEET_UP.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok();
        #[cfg(test)]
        let free = !SHEET_UP.with(|s| s.replace(true));
        // (Not `then_some`: a Sheet made and dropped would free the flag.)
        if free { Some(Sheet(())) } else { None }
    }
}

impl Drop for Sheet {
    fn drop(&mut self) {
        #[cfg(not(test))]
        SHEET_UP.store(false, Ordering::SeqCst);
        #[cfg(test)]
        SHEET_UP.with(|s| s.set(false));
    }
}

/// What a share hands over: the picture's file (PNG, also set as the
/// bitmap) or another file (a recording).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))] // Windows (tests everywhere)
pub enum Kind {
    Picture,
    File,
}

/// Which kind `file` is: a PNG, JPEG or BMP is also offered as a bitmap.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn kind_of(file: &Path) -> Kind {
    let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp") { Kind::Picture } else { Kind::File }
}

/// `p` without a `\\?\` (or `\\?\UNC\`) prefix, which `StorageFile`
/// refuses.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn plain_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p.to_path_buf()
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: the files "shared" (no sheet ever comes up).
    pub static SHARED: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Show the share sheet for `file` near window `owner` (a raw handle,
/// Windows; else the pointer's monitor) and wait until the share ends (or
/// its time limit): on a worker. `Err`: what to tell the user.
pub fn share_now(file: &Path, owner: Option<usize>) -> Result<(), String> {
    if !supported() {
        return Err("Share is not available here".into());
    }
    if !file.is_file() {
        return Err("Share failed: the file is gone".into());
    }
    let Some(sheet) = Sheet::claim() else { return Err(BUSY.into()) };
    let _running = Running::start();
    #[cfg(test)]
    {
        let _ = (owner, sheet);
        SHARED.with(|s| s.borrow_mut().push(file.to_path_buf()));
        Ok(())
    }
    #[cfg(not(test))]
    {
        let file: PathBuf = plain_path(&std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf()));
        imp(file, owner, sheet)
    }
}

/// The share on its own thread, which lets the caller go (the result)
/// when the share ends or at the time limit, and stays while the sheet
/// may still ask for the data.
#[cfg(all(windows, not(test)))]
fn imp(file: PathBuf, owner: Option<usize>, sheet: Sheet) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let kind = kind_of(&file);
    let max = max_wait();
    std::thread::Builder::new()
        .name("rustshot-share-sheet".into())
        .spawn(move || win::share(&file, owner, kind, max, win::Release::new(tx, sheet)))
        .map_err(|e| format!("Share failed: {e}"))?;
    rx.recv().unwrap_or_else(|_| Err("Share failed".into()))
}

#[cfg(all(target_os = "macos", not(test)))]
fn imp(file: PathBuf, _owner: Option<usize>, sheet: Sheet) -> Result<(), String> {
    let r = macos::share(&file, max_wait());
    drop(sheet);
    r
}

#[cfg(all(not(any(windows, target_os = "macos")), not(test)))]
fn imp(_file: PathBuf, _owner: Option<usize>, _sheet: Sheet) -> Result<(), String> {
    Err("Share is not available here".into())
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
        // Also busy work: a daemon that quits when idle (tray off) waits.
        crate::busy::spawn(move || {
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

    /// The tests that claim the sheet or share run one at a time: the
    /// flag is per thread, but a harness on one thread (`--test-threads=1`)
    /// runs them all on it.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(|e| e.into_inner())
    }

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
        let _serial = serial();
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

    /// One sheet at a time: a share while one is up says so (and shows
    /// nothing); the sheet is free again when its holder goes.
    #[test]
    fn a_second_share_is_refused_while_a_sheet_is_up() {
        let dir = std::env::temp_dir().join(format!("rustshot-share-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.png");
        std::fs::write(&f, b"x").unwrap();
        let _serial = serial();
        SHARED.with(|s| s.borrow_mut().clear());
        let up = Sheet::claim().expect("free");
        assert!(Sheet::claim().is_none());
        if supported() {
            assert_eq!(share_now(&f, None), Err(BUSY.to_string()));
            assert!(SHARED.with(|s| s.borrow().is_empty()), "nothing shown");
        }
        // (Released on every OS: Linux has no sheet, but the flag is the same.)
        drop(up);
        if supported() {
            assert_eq!(share_now(&f, None), Ok(()));
        }
        assert!(Sheet::claim().is_some(), "free after the share");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The test knob for the time limit is bounded (no overflow).
    #[test]
    fn the_time_limit_is_bounded() {
        assert_eq!(parse_max(None), MAX_WAIT);
        assert_eq!(parse_max(Some("8")), Duration::from_secs(8));
        assert_eq!(parse_max(Some("18446744073709551615")), Duration::from_secs(3600));
        assert_eq!(parse_max(Some("x")), MAX_WAIT);
    }

    /// Verbatim paths lose their prefix (StorageFile refuses them).
    #[test]
    fn verbatim_paths_are_made_plain() {
        assert_eq!(plain_path(Path::new(r"\\?\C:\a\b.png")), PathBuf::from(r"C:\a\b.png"));
        assert_eq!(plain_path(Path::new(r"\\?\UNC\srv\share\b.png")), PathBuf::from(r"\\srv\share\b.png"));
        assert_eq!(plain_path(Path::new(r"C:\a.png")), PathBuf::from(r"C:\a.png"));
    }

    /// A gone file is refused (a nice message); a real one is "shared".
    #[test]
    fn share_now_checks_the_file() {
        let dir = std::env::temp_dir().join(format!("rustshot-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.png");
        std::fs::write(&f, b"x").unwrap();
        let _serial = serial();
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
