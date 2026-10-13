//! Getting a language's file: languages other than English are release
//! assets, downloaded when one is picked (or is the system's) and kept in
//! the data cache. The download itself is the data-files layer's
//! (`datadir` / `fetch::ensure`, with SHA256SUMS checks); this module
//! talks to it through [`Fetch`], so tests use a fake, and keeps each
//! language's [`Status`] for Settings ("Downloading…", an error and Retry).
//!
//! Names: language `<tag>` is the release asset `lang-<tag>.txt`, cached
//! as `lang/<tag>.txt` (relative to the cache folder).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// The data-files layer as this module needs it.
pub trait Fetch: Send + Sync {
    /// Make release asset `asset` the cache file `rel`, then call `done`
    /// (on any thread) with its path or why it could not.
    fn ensure(&self, asset: &str, rel: &str, done: Box<dyn FnOnce(Result<PathBuf, String>) + Send>);
    /// The cached copy of `rel`, if it is there.
    fn cached(&self, rel: &str) -> Option<PathBuf>;
}

/// Until the data-files layer is wired in: the cache is read, nothing is
/// downloaded.
struct NoDownloads;

impl Fetch for NoDownloads {
    fn ensure(&self, _asset: &str, _rel: &str, done: Box<dyn FnOnce(Result<PathBuf, String>) + Send>) {
        done(Err(crate::tr!("i18n.download_unavailable").into()));
    }
    fn cached(&self, rel: &str) -> Option<PathBuf> {
        crate::datadir::cached(rel)
    }
}

/// The fetcher in use (a clone: never called with the lock held).
fn fetcher() -> Arc<dyn Fetch> {
    slot().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn slot() -> &'static Mutex<Arc<dyn Fetch>> {
    static F: OnceLock<Mutex<Arc<dyn Fetch>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(Arc::new(NoDownloads)))
}

/// Use `f` for downloads (the data-files layer at startup; tests a fake).
#[allow(dead_code)]
pub fn set_fetcher(f: Arc<dyn Fetch>) {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) = f;
}

/// The release asset of language `tag`.
pub fn asset(tag: &str) -> String {
    format!("lang-{tag}.txt")
}

/// Language `tag`'s file in the cache folder.
pub fn rel(tag: &str) -> String {
    format!("lang/{tag}.txt")
}

/// The cached file of language `tag`, if it is there.
pub fn cached(tag: &str) -> Option<PathBuf> {
    fetcher().cached(&rel(tag))
}

/// Where a language's file stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Its strings are loaded (English always).
    Here,
    /// Not downloaded (nor asked for).
    Missing,
    Downloading,
    /// The last download failed: why (Settings offers Retry).
    Failed(String),
}

fn states() -> std::sync::MutexGuard<'static, HashMap<String, Status>> {
    static S: OnceLock<Mutex<HashMap<String, Status>>> = OnceLock::new();
    S.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Set language `tag`'s status (tests: the states Settings shows).
#[cfg(test)]
pub fn set_status(tag: &str, s: Status) {
    states().insert(tag.to_ascii_lowercase(), s);
}

/// Language `tag`'s status.
pub fn status(tag: &str) -> Status {
    if super::is_loaded(tag) {
        return Status::Here;
    }
    states().get(&tag.to_ascii_lowercase()).cloned().unwrap_or(Status::Missing)
}

/// Download language `tag` unless it is here or on its way (a failed one
/// is tried again: Retry). When it lands it is loaded, and every window
/// lays itself out again ([`super::generation`]).
pub fn request(tag: &str) {
    let key = tag.to_ascii_lowercase();
    if super::is_loaded(tag) || matches!(states().get(&key), Some(Status::Downloading)) {
        return;
    }
    states().insert(key.clone(), Status::Downloading);
    super::bump();
    let t = tag.to_string();
    let done: Box<dyn FnOnce(Result<PathBuf, String>) + Send> = Box::new(move |r| {
        let st = match r {
            Ok(_) if super::is_loaded(&t) => Status::Here,
            Ok(p) => Status::Failed(crate::tr!("i18n.unreadable", path = p.display())),
            Err(e) => Status::Failed(e),
        };
        if st == Status::Here {
            super::downloaded(&t);
        }
        states().insert(key, st);
        super::bump();
    });
    fetcher().ensure(&asset(tag), &rel(tag), done);
}
