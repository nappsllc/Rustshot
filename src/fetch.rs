//! Data files downloaded on demand from this version's GitHub release:
//! the camera blur's model (when the blur is first turned on) and the
//! language files (when a language is chosen). Nothing is sent but the
//! request for the file.
//!
//! Each file this version knows is in its manifest ([`MODEL`], [`LANGS`]
//! from build.rs) with its release asset name, size and SHA-256. A
//! download ([`ensure`]) takes `SHA256SUMS` and the asset from
//! `https://github.com/nappsllc/rustshot/releases/download/v<version>/`
//! through the updater's guarded transport (that URL prefix only,
//! redirects only to GitHub's asset hosts), refuses a `SHA256SUMS` whose
//! entry is not the manifest's, caps the size, checks the SHA-256, and
//! renames the file into the cache ([`crate::datadir::cache_dir`]) only
//! then: a partial download is never seen. One download per file at a
//! time; later callers wait for it. A file kept from another version is
//! used as long as it has this version's SHA-256 (no download).
//!
//! A build whose own version has no release yet (a development build, a
//! pre-release) or whose release lacks the file: the newest published
//! release is asked for (the updater's API request) and, when it lists an
//! asset of the same name and SHA-256, the file comes from there. The
//! SHA-256 built into this version is what decides, wherever it comes
//! from. Downloads stay off until the config says otherwise
//! ([`set_allowed`]; [`global`] reads it once).

use crate::datadir;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// A data file of this version.
#[derive(Debug, PartialEq, Eq)]
pub struct DataFile {
    /// The release asset's name.
    pub asset: &'static str,
    /// Where it goes under a data folder (`lang/de.txt`).
    pub rel: &'static str,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: &'static str,
    /// Checked wherever it is found (the model); otherwise a copy in the
    /// override folder is taken as it is (a translator's file).
    pub strict: bool,
}

include!(concat!(env!("OUT_DIR"), "/lang_manifest.rs"));

/// The person-segmentation model (`models/selfie.bin` in the source tree;
/// the release asset is named after its hash). The test
/// `model_manifest_matches_the_file` says when it changes.
pub static MODEL: DataFile = DataFile {
    asset: "selfie-f11cb3ec.bin",
    rel: "models/selfie.bin",
    size: 219_338,
    sha256: "f11cb3ecb40759537b48076331e702dadaafe1fca7fb50e7a2c15e65cf577e76",
    strict: true,
};

/// Every file this version may download.
pub fn manifest() -> impl Iterator<Item = &'static DataFile> {
    std::iter::once(&MODEL).chain(LANGS.iter())
}

pub fn by_rel(rel: &str) -> Option<&'static DataFile> {
    manifest().find(|f| f.rel == rel)
}

pub fn by_asset(asset: &str) -> Option<&'static DataFile> {
    manifest().find(|f| f.asset == asset)
}

/// The languages this version has files for (their tags, sorted; English
/// is built in and not among them).
#[cfg_attr(not(test), allow(dead_code))] // Settings › Language (the i18n branch)
pub fn lang_tags() -> impl Iterator<Item = &'static str> {
    LANGS.iter().filter_map(|f| f.rel.strip_prefix("lang/")?.strip_suffix(".txt"))
}

/// The error when downloads are off (`download_data = false`).
pub const DISABLED: &str = "downloads are off";

impl DataFile {
    /// Its URL in the release of `version`.
    pub fn url(&self, version: &str) -> String {
        format!("{}v{version}/{}", crate::update::DOWNLOAD_PREFIX, self.asset)
    }

    /// `p` has this file's size and SHA-256 (remembered by path, length
    /// and modification time: asking again costs a `stat`).
    pub fn matches(&self, p: &Path) -> bool {
        let Ok(md) = std::fs::metadata(p) else { return false };
        if !md.is_file() || md.len() != self.size {
            return false;
        }
        let stamp = (md.len(), md.modified().ok());
        let key = (p.to_path_buf(), self.sha256);
        let seen = seen();
        if let Some(&(len, t, ok)) = lock(seen).get(&key)
            && (len, t) == stamp
        {
            return ok;
        }
        let ok = self.check(p).is_ok();
        lock(seen).insert(key, (stamp.0, stamp.1, ok));
        ok
    }

    /// Forget what [`DataFile::matches`] remembered of this file (its
    /// bytes turned out not to load: the next look hashes it again).
    pub fn forget(&self) {
        lock(seen()).retain(|(_, sha), _| *sha != self.sha256);
        global().bump();
    }

    /// Hash `p` now (no memo).
    pub fn check(&self, p: &Path) -> Result<(), String> {
        let d = crate::sha256::file_digest(p).map_err(|e| format!("read {}: {e}", p.display()))?;
        let len = std::fs::metadata(p).map_err(|e| e.to_string())?.len();
        if len == self.size && crate::sha256::hex(&d) == self.sha256 { Ok(()) } else { Err(format!("{} failed its SHA-256 check", self.asset)) }
    }
}

type Seen = HashMap<(PathBuf, &'static str), (u64, Option<std::time::SystemTime>, bool)>;

/// [`DataFile::matches`]' memo.
fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// `rel` (`lang/de.txt`) under `d`, with this OS's separators.
fn under(d: &Path, rel: &str) -> PathBuf {
    let mut p = d.to_path_buf();
    p.extend(rel.split('/'));
    p
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// How a file is fetched: the guarded release download
/// ([`crate::export::download_to`]), or a test's stand-in.
pub trait Transport: Send + Sync {
    /// Download `url` to `dest` (created; a partial file deleted on
    /// failure), at most `max` bytes; `progress(got, total)` returning
    /// false stops it. It always ends: a stalled or slow download fails
    /// (the release transport's time limits).
    fn get(&self, url: &str, dest: &Path, max: u64, progress: &dyn Fn(u64, Option<u64>) -> bool) -> Result<(), String>;
}

struct Web;

impl Transport for Web {
    fn get(&self, url: &str, dest: &Path, max: u64, progress: &dyn Fn(u64, Option<u64>) -> bool) -> Result<(), String> {
        crate::export::download_to(url, dest, Some(max), progress)
    }
}

/// Where a file stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Here (verified).
    Ready(PathBuf),
    /// Not here, not asked for yet.
    Missing,
    /// On its way: bytes so far of the total.
    Downloading { got: u64, total: u64 },
    /// The last download failed (why).
    Failed(String),
    /// Not here and downloads are off: to be put at the path by hand.
    Off(PathBuf),
    /// Downloads off and the file there is not this version's (damaged).
    Damaged(PathBuf),
}

type Done = Box<dyn FnOnce(Result<PathBuf, String>) + Send>;

struct Job {
    got: u64,
    waiters: Vec<Done>,
}

/// The downloads of one cache.
pub struct Fetcher {
    cache: Option<PathBuf>,
    over: Option<PathBuf>,
    version: String,
    net: Box<dyn Transport>,
    allowed: AtomicBool,
    jobs: Mutex<HashMap<&'static str, Job>>,
    failed: Mutex<HashMap<&'static str, String>>,
    generation: AtomicU64,
    seq: AtomicU64,
    /// The newest published release's version, for a file this version's
    /// own release does not have (`None`: no fallback).
    fallback: Option<Box<dyn Fn() -> Result<String, String> + Send + Sync>>,
}

impl Fetcher {
    pub fn new(cache: Option<PathBuf>, over: Option<PathBuf>, version: &str, net: Box<dyn Transport>) -> Fetcher {
        Fetcher {
            cache,
            over,
            version: version.to_string(),
            net,
            // Off until the config is read (fail closed).
            allowed: AtomicBool::new(false),
            jobs: Mutex::default(),
            failed: Mutex::default(),
            generation: AtomicU64::new(0),
            seq: AtomicU64::new(0),
            fallback: None,
        }
    }

    /// With `latest` (the newest release's version) for files this
    /// version's own release does not have.
    pub fn with_fallback(mut self, latest: impl Fn() -> Result<String, String> + Send + Sync + 'static) -> Fetcher {
        self.fallback = Some(Box::new(latest));
        self
    }

    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// The usable copy of `rel`: the override folder's (taken as it is,
    /// unless the manifest's file is strict and it does not match), else
    /// the cache's when it has this version's SHA-256. A file not in the
    /// manifest is only taken from the override folder.
    pub fn find_rel(&self, rel: &str) -> Option<PathBuf> {
        if !datadir::safe_rel(rel) {
            return None;
        }
        let f = by_rel(rel);
        if let Some(p) = self.over.as_ref().map(|d| under(d, rel)).filter(|p| p.is_file()) {
            match f {
                Some(f) if f.strict => {
                    if f.matches(&p) {
                        return Some(p);
                    }
                }
                _ => return Some(p),
            }
        }
        f.and_then(|f| self.find_in_cache(f))
    }

    fn find(&self, f: &DataFile) -> Option<PathBuf> {
        let o = self.over.as_ref().map(|d| under(d, f.rel)).filter(|p| p.is_file());
        if let Some(p) = o
            && (!f.strict || f.matches(&p))
        {
            return Some(p);
        }
        self.find_in_cache(f)
    }

    fn find_in_cache(&self, f: &DataFile) -> Option<PathBuf> {
        self.cache.as_ref().map(|d| under(d, f.rel)).filter(|p| f.matches(p))
    }

    /// Where `f` goes in the cache.
    pub fn cache_path(&self, f: &DataFile) -> Option<PathBuf> {
        self.cache.as_ref().map(|d| under(d, f.rel))
    }

    pub fn state(&self, f: &'static DataFile) -> State {
        if let Some(p) = self.find(f) {
            return State::Ready(p);
        }
        if let Some(j) = lock(&self.jobs).get(f.asset) {
            return State::Downloading { got: j.got, total: f.size };
        }
        if let Some(e) = lock(&self.failed).get(f.asset) {
            return State::Failed(e.clone());
        }
        if !self.allowed() {
            // (No cache folder at all: an empty path; nothing to check.)
            let p = self.cache_path(f).unwrap_or_default();
            return if !p.as_os_str().is_empty() && p.is_file() { State::Damaged(p) } else { State::Off(p) };
        }
        State::Missing
    }

    pub fn allowed(&self) -> bool {
        self.allowed.load(Ordering::Relaxed)
    }

    pub fn set_allowed(&self, on: bool) {
        if self.allowed.swap(on, Ordering::Relaxed) != on {
            self.bump();
        }
    }

    /// Bumped whenever a download starts, moves on or ends (windows that
    /// show a [`State`] redraw when it changes).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// `f` here: `done` gets its path, now when it already is, else once a
    /// download (on its own thread; one per file) ends. Never blocks.
    /// `done` runs on the caller's thread when the answer is immediate
    /// (the file is here, downloads are off), else on the download's
    /// thread: a caller must not hold a lock that `done` takes.
    pub fn ensure(self: &Arc<Self>, f: &'static DataFile, done: Done) {
        if let Some(p) = self.find(f) {
            return done(Ok(p));
        }
        if !self.allowed() {
            return done(Err(DISABLED.into()));
        }
        let here = {
            let mut jobs = lock(&self.jobs);
            if let Some(j) = jobs.get_mut(f.asset) {
                j.waiters.push(done);
                return;
            }
            // A download that ended (renamed, gone from `jobs`) between
            // the look above and this lock: no second one.
            match self.find(f) {
                Some(p) => Err((p, done)),
                None => {
                    jobs.insert(f.asset, Job { got: 0, waiters: vec![done] });
                    Ok(())
                }
            }
        };
        if let Err((p, done)) = here {
            return done(Ok(p));
        }
        lock(&self.failed).remove(f.asset);
        self.bump();
        let me = Arc::clone(self);
        let run = move || {
            let r = me.download(f).map_err(|e| friendly(&e));
            if let Err(e) = &r {
                eprintln!("rustshot: couldn't download {}: {e}", f.asset);
                lock(&me.failed).insert(f.asset, e.clone());
            } else {
                eprintln!("rustshot: downloaded {} (SHA-256 verified)", f.asset);
            }
            let waiters = lock(&me.jobs).remove(f.asset).map(|j| j.waiters).unwrap_or_default();
            me.bump();
            for w in waiters {
                w(r.clone());
            }
        };
        if let Err(e) = std::thread::Builder::new().name("rustshot-fetch".into()).spawn(run) {
            let waiters = lock(&self.jobs).remove(f.asset).map(|j| j.waiters).unwrap_or_default();
            let e = format!("start the download: {e}");
            lock(&self.failed).insert(f.asset, e.clone());
            self.bump();
            for w in waiters {
                w(Err(e.clone()));
            }
        }
    }

    /// SHA256SUMS, then the file, into a temporary name beside its place;
    /// checked, then renamed there.
    fn download(&self, f: &'static DataFile) -> Result<PathBuf, String> {
        let dest = self.cache_path(f).ok_or("there is no folder to keep downloads in")?;
        let dir = dest.parent().ok_or("unexpected download path")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        sweep(dir);
        match self.download_from(f, &self.version, &dest, dir) {
            Err(Fetch::NotThere(why)) => {
                // This version's release lacks it: the newest release, if
                // it has the very same file.
                let Some(latest) = &self.fallback else { return Err(why) };
                let v = match latest() {
                    Ok(v) if v != self.version && crate::update::parse_version(&v).is_some() => v,
                    _ => return Err(why),
                };
                eprintln!("rustshot: {}: {why}; trying release v{v}", f.asset);
                self.download_from(f, &v, &dest, dir).map_err(|e| match e {
                    Fetch::NotThere(_) => why,
                    Fetch::Err(e) => e,
                })
            }
            r => r.map_err(|e| match e {
                Fetch::NotThere(e) | Fetch::Err(e) => e,
            }),
        }
    }

    /// [`Fetcher::download`] from release `version`.
    fn download_from(&self, f: &'static DataFile, version: &str, dest: &Path, dir: &Path) -> Result<PathBuf, Fetch> {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let tag = format!("{}.{}", std::process::id(), n);
        let sums_tmp = dir.join(format!(".{}.sums.{tag}.part", f.asset));
        let tmp = dir.join(format!(".{}.{tag}.part", f.asset));
        let r = (|| -> Result<(), Fetch> {
            let sums_url = format!("{}v{version}/{}", crate::update::DOWNLOAD_PREFIX, crate::update::SUMS_NAME);
            let too_big = AtomicBool::new(false);
            let r = self.net.get(&sums_url, &sums_tmp, crate::update::SUMS_MAX, &|got, _| {
                let big = got > crate::update::SUMS_MAX;
                too_big.fetch_or(big, Ordering::Relaxed);
                !big
            });
            if too_big.load(Ordering::Relaxed) {
                return Err(Fetch::Err("SHA256SUMS is unexpectedly large".into()));
            }
            r.map_err(|e| {
                let e = format!("SHA256SUMS: {e}");
                if is_404(&e) { Fetch::NotThere(e) } else { Fetch::Err(e) }
            })?;
            let text = std::fs::read(&sums_tmp).map_err(|e| Fetch::Err(e.to_string()))?;
            let (sums, poisoned) = crate::update::parse_sums_checked(&String::from_utf8_lossy(&text));
            if poisoned.iter().any(|p| p == f.asset) {
                return Err(Fetch::Err(format!("SHA256SUMS lists {} twice", f.asset)));
            }
            let want = sums.get(f.asset).ok_or_else(|| Fetch::NotThere(format!("{} is not listed in SHA256SUMS", f.asset)))?;
            if crate::sha256::hex(want) != f.sha256 {
                return Err(Fetch::Err(format!("the release's {} is not the one this version expects", f.asset)));
            }
            let too_big = AtomicBool::new(false);
            let r = self.net.get(&f.url(version), &tmp, f.size, &|got, _| {
                let big = got > f.size;
                too_big.fetch_or(big, Ordering::Relaxed);
                if let Some(j) = lock(&self.jobs).get_mut(f.asset) {
                    j.got = got.min(f.size);
                }
                self.bump();
                !big
            });
            if too_big.load(Ordering::Relaxed) {
                return Err(Fetch::Err(format!("{} is larger than expected", f.asset)));
            }
            r.map_err(Fetch::Err)?;
            f.check(&tmp).map_err(Fetch::Err)?;
            std::fs::rename(&tmp, dest).map_err(|e| Fetch::Err(format!("keep {}: {e}", dest.display())))
        })();
        let _ = std::fs::remove_file(&sums_tmp);
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        r.map(|()| dest.to_path_buf())
    }
}

/// A download's failure: the release does not have the file (another
/// release may), or anything else.
enum Fetch {
    NotThere(String),
    Err(String),
}

fn is_404(e: &str) -> bool {
    e.contains("HTTP 404") || e.contains("error: 404") || e.contains("returned error: 404")
}

/// Temporary files (`.<asset>….part`) a download killed midway left in
/// `dir`, an hour old or more, are deleted.
fn sweep(dir: &Path) {
    let old = std::time::Duration::from_secs(3600);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with('.') && name.ends_with(".part")) {
            continue;
        }
        let aged = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|a| a >= old);
        if aged {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A transport error as the user reads it: no connection, a timeout, or
/// a file this version's release does not have, in words (the raw
/// error in parentheses); anything else as it is.
fn friendly(e: &str) -> String {
    // WinHTTP's codes (0x80072EE7 and so on) and curl's exit messages.
    let offline = ["80072ee7", "80072efd", "80072efe", "80072f30", "could not resolve host", "failed to connect", "couldn't connect"];
    let timeout = ["80072ee2", "timed out", "timeout"];
    let low = e.to_ascii_lowercase();
    let why = if offline.iter().any(|k| low.contains(k)) {
        "no connection to github.com"
    } else if timeout.iter().any(|k| low.contains(k)) {
        "github.com did not answer in time"
    } else if is_404(e) || e.contains("is not listed in SHA256SUMS") {
        "this version's release does not have the file"
    } else {
        return e.to_string();
    };
    format!("{why} ({e})")
}

/// The app's fetcher: the real cache and transport, this version.
/// Downloads start as the config file says (`download_data`), read once
/// here: every process (the daemon, a one-shot capture, a Settings
/// window of its own, the CLI) begins from it, never from "on"; a reload
/// or a save sets it again ([`set_allowed`]). Tests: no cache, no
/// override folder, downloads off.
pub fn global() -> &'static Arc<Fetcher> {
    static G: OnceLock<Arc<Fetcher>> = OnceLock::new();
    G.get_or_init(|| {
        #[cfg(test)]
        return Arc::new(Fetcher::new(None, None, env!("CARGO_PKG_VERSION"), Box::new(Web)));
        #[cfg(not(test))]
        {
            let f = Fetcher::new(datadir::cache_dir(), datadir::override_dir(), env!("CARGO_PKG_VERSION"), Box::new(Web)).with_fallback(crate::update::latest_version);
            f.allowed.store(initial_allowed(crate::config::read_at(&crate::config::config_path())), Ordering::Relaxed);
            Arc::new(f)
        }
    })
}

/// Whether downloads start allowed, from the config file as read: its
/// `download_data`; no file yet, the default (on); a broken one, off
/// (fail closed).
pub fn initial_allowed(read: Result<Option<crate::config::Config>, String>) -> bool {
    match read {
        Ok(c) => c.unwrap_or_default().download_data,
        Err(_) => false,
    }
}

/// `rel` (e.g. `lang/de.txt`) here, as the release asset `asset`: `on_done`
/// gets its path (at once when it is already here), or why it could not
/// be had. An asset not in this version's manifest, or not at `rel`, is
/// refused. Never blocks. `on_done` runs at once on the caller's thread
/// when the answer is immediate, else on the download's thread: do not
/// hold a lock `on_done` takes while calling this.
pub fn ensure(asset: &str, rel: &str, on_done: impl FnOnce(Result<PathBuf, String>) + Send + 'static) {
    match by_asset(asset).filter(|f| f.rel == rel) {
        Some(f) => global().ensure(f, Box::new(on_done)),
        None => on_done(Err(format!("{asset} is not a data file of this version"))),
    }
}

/// [`Fetcher::state`] of the app's fetcher.
pub fn state(f: &'static DataFile) -> State {
    global().state(f)
}

/// `download_data` (Settings › General).
pub fn set_allowed(on: bool) {
    #[cfg(test)]
    LAST_SET.with(|l| l.set(Some(on)));
    global().set_allowed(on);
}

#[cfg(test)]
thread_local! {
    static LAST_SET: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Tests: the last [`set_allowed`] on this thread (and forget it).
#[cfg(test)]
pub fn take_last_set() -> Option<bool> {
    LAST_SET.with(|l| l.take())
}

pub fn generation() -> u64 {
    global().generation()
}

/// `rustshot data [--fetch]`: each data file of this version, where it
/// is, or what to download from where and where to save it; `fetch`
/// downloads the missing ones (waiting for them). 0 when every file is
/// here (or, without `fetch`, always), 1 otherwise.
pub fn cli(fetch: bool) -> i32 {
    let g = global();
    let ver = env!("CARGO_PKG_VERSION");
    let mut missing = 0;
    for f in manifest() {
        let mut st = g.state(f);
        if fetch && !matches!(st, State::Ready(_)) {
            let (tx, rx) = crate::chan::channel();
            g.ensure(f, Box::new(move |r| {
                let _ = tx.send(r);
            }));
            let _ = rx.recv();
            st = g.state(f);
            if !g.allowed() && !matches!(st, State::Ready(_)) {
                eprintln!("{}: not downloaded: {DISABLED} (download_data = false)", f.asset);
            }
        }
        let to = g.cache_path(f).map_or_else(|| format!("{} in the folder RUSTSHOT_DATA_DIR names", f.rel), |p| p.display().to_string());
        match st {
            State::Ready(p) => println!("{}: {} (SHA-256 verified)", f.asset, p.display()),
            State::Failed(e) => {
                missing += 1;
                println!("{}: couldn't download: {e} (from {}; save it as {to})", f.asset, f.url(ver));
            }
            State::Damaged(p) => {
                missing += 1;
                println!("{}: {} is damaged (SHA-256 differs); download {} again", f.asset, p.display(), f.url(ver));
            }
            _ => {
                missing += 1;
                println!("{}: not here; download {} and save it as {to}", f.asset, f.url(ver));
            }
        }
    }
    i32::from(fetch && missing > 0)
}

/// "220 KB".
pub fn size_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) } else { format!("{} KB", bytes.div_ceil(1024)) }
}

#[cfg(test)]
mod tests;
