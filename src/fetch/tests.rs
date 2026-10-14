use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::{mpsc, Condvar};
use std::time::Duration;

/// What the stand-in server answers for a URL.
#[derive(Clone)]
enum Resp {
    Body(Vec<u8>),
    /// Half the body, then the connection drops.
    Partial(Vec<u8>),
    Fail(String),
}

#[derive(Default)]
struct Mock {
    routes: Mutex<HashMap<String, Resp>>,
    calls: Mutex<HashMap<String, usize>>,
    /// While true, asset downloads wait (SHA256SUMS does not).
    held: Mutex<bool>,
    cv: Condvar,
    waiting: AtomicUsize,
}

impl Mock {
    fn set(&self, url: &str, r: Resp) {
        lock(&self.routes).insert(url.into(), r);
    }
    fn calls(&self, url: &str) -> usize {
        lock(&self.calls).get(url).copied().unwrap_or(0)
    }
    fn release(&self) {
        *lock(&self.held) = false;
        self.cv.notify_all();
    }
}

impl Transport for Arc<Mock> {
    fn get(&self, url: &str, dest: &Path, _max: u64, progress: &dyn Fn(u64, Option<u64>) -> bool) -> Result<(), String> {
        *lock(&self.calls).entry(url.into()).or_default() += 1;
        if !url.ends_with(crate::update::SUMS_NAME) {
            self.waiting.fetch_add(1, Ordering::SeqCst);
            let mut h = lock(&self.held);
            while *h {
                h = self.cv.wait(h).unwrap_or_else(|e| e.into_inner());
            }
        }
        let r = lock(&self.routes).get(url).cloned().unwrap_or(Resp::Fail("HTTP 404".into()));
        let write = |b: &[u8]| -> Result<bool, String> {
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(|e| e.to_string())?;
            use std::io::Write;
            for (i, c) in b.chunks(4096).enumerate() {
                f.write_all(c).map_err(|e| e.to_string())?;
                if !progress(((i + 1) * 4096).min(b.len()) as u64, Some(b.len() as u64)) {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        let r = match r {
            Resp::Body(b) => match write(&b)? {
                true => Ok(()),
                false => Err(crate::update::CANCELLED.into()),
            },
            Resp::Partial(b) => {
                write(&b[..b.len() / 2])?;
                Err("the connection was reset".into())
            }
            Resp::Fail(e) => Err(e),
        };
        if r.is_err() {
            let _ = std::fs::remove_file(dest);
        }
        r
    }
}

const V: &str = "9.8.7";

fn tmp(name: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!("rustshot-fetch-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A data file of `data`, as a manifest entry.
fn entry(asset: &'static str, rel: &'static str, data: &[u8], strict: bool) -> &'static DataFile {
    let sha256: &'static str = Box::leak(crate::sha256::hex(&crate::sha256::digest(data)).into_boxed_str());
    Box::leak(Box::new(DataFile { asset, rel, size: data.len() as u64, sha256, strict }))
}

fn sums_url() -> String {
    format!("{}v{V}/SHA256SUMS", crate::update::DOWNLOAD_PREFIX)
}

fn sums_for(files: &[(&DataFile, &[u8])]) -> Vec<u8> {
    files.iter().map(|(f, d)| format!("{}  {}\n", crate::sha256::hex(&crate::sha256::digest(d)), f.asset)).collect::<String>().into_bytes()
}

fn fetcher(cache: &Path, over: Option<PathBuf>) -> (Arc<Fetcher>, Arc<Mock>) {
    let m = Arc::new(Mock::default());
    let fe = Fetcher::new(Some(cache.to_path_buf()), over, V, Box::new(Arc::clone(&m)));
    fe.set_allowed(true);
    (Arc::new(fe), m)
}

fn ensure_wait(fe: &Arc<Fetcher>, f: &'static DataFile) -> Result<PathBuf, String> {
    let (tx, rx) = mpsc::channel();
    fe.ensure(f, Box::new(move |r| tx.send(r).unwrap()));
    rx.recv_timeout(Duration::from_secs(20)).expect("the download ends")
}

/// Nothing but finished files in the cache (no `.part` left behind).
fn leftovers(dir: &Path) -> Vec<String> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.path().is_dir() {
            v.extend(leftovers(&e.path()));
        } else if e.file_name().to_string_lossy().ends_with(".part") {
            v.push(e.path().display().to_string());
        }
    }
    v
}

const DE: &[u8] = b"# Deutsch\nsettings.title = Einstellungen\n";

#[test]
fn downloads_checks_and_keeps() {
    let cache = tmp("ok");
    let f = entry("lang-de.txt", "lang/de.txt", DE, false);
    let (fe, m) = fetcher(&cache, None);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Body(DE.to_vec()));
    assert_eq!(fe.state(f), State::Missing);
    let g0 = fe.generation();
    let p = ensure_wait(&fe, f).unwrap();
    assert_eq!(p, cache.join("lang").join("de.txt"));
    assert_eq!(std::fs::read(&p).unwrap(), DE);
    assert_eq!(fe.state(f), State::Ready(p.clone()));
    assert!(fe.generation() > g0, "windows redraw");
    assert!(leftovers(&cache).is_empty(), "{:?}", leftovers(&cache));
    // Here: no second download.
    assert_eq!(ensure_wait(&fe, f).unwrap(), p);
    assert_eq!(m.calls(&f.url(V)), 1);
    assert_eq!(f.url(V), format!("https://github.com/nappsllc/rustshot/releases/download/v{V}/lang-de.txt"));
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn a_file_with_another_hash_is_refused() {
    let cache = tmp("hash");
    let f = entry("lang-de.txt", "lang/de.txt", DE, false);
    let (fe, m) = fetcher(&cache, None);
    // The release's file is not the one this version expects, though its
    // SHA256SUMS line matches the manifest.
    let other = b"# Deutsch\nsettings.title = Einstellungem\n";
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Body(other.to_vec()));
    let e = ensure_wait(&fe, f).unwrap_err();
    assert!(e.contains("SHA-256"), "{e}");
    assert!(!cache.join("lang/de.txt").exists());
    assert!(leftovers(&cache).is_empty());
    assert_eq!(fe.state(f), State::Failed(e));
    // SHA256SUMS itself disagreeing with the manifest: refused before the file.
    let (fe, m) = fetcher(&cache, None);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, other)])));
    m.set(&f.url(V), Resp::Body(DE.to_vec()));
    let e = ensure_wait(&fe, f).unwrap_err();
    assert!(e.contains("not the one this version expects"), "{e}");
    assert_eq!(m.calls(&f.url(V)), 0);
    // Not listed, or listed twice.
    let (fe, m) = fetcher(&cache, None);
    m.set(&sums_url(), Resp::Body(b"00  other.txt\n".to_vec()));
    assert!(ensure_wait(&fe, f).unwrap_err().contains("not listed"));
    let (fe, m) = fetcher(&cache, None);
    let mut twice = sums_for(&[(f, DE)]);
    twice.extend(sums_for(&[(f, other)]));
    m.set(&sums_url(), Resp::Body(twice));
    assert!(ensure_wait(&fe, f).unwrap_err().contains("twice"));
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn not_found_offline_and_retry() {
    let cache = tmp("404");
    let f = entry("lang-de.txt", "lang/de.txt", DE, false);
    let (fe, m) = fetcher(&cache, None);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    // (no route for the file: 404)
    let e = ensure_wait(&fe, f).unwrap_err();
    assert!(e.contains("does not have the file") && e.contains("404"), "{e}");
    assert!(matches!(fe.state(f), State::Failed(_)));
    // Offline: even SHA256SUMS fails.
    m.set(&sums_url(), Resp::Fail("couldn't connect to github.com".into()));
    let e = ensure_wait(&fe, f).unwrap_err();
    assert!(e.starts_with("no connection to github.com (SHA256SUMS: couldn't connect"), "{e}");
    // Back online: a retry works and clears the failure.
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Body(DE.to_vec()));
    assert!(ensure_wait(&fe, f).is_ok());
    assert!(matches!(fe.state(f), State::Ready(_)));
    assert!(leftovers(&cache).is_empty());
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn a_partial_download_is_never_seen() {
    let cache = tmp("partial");
    let f = entry("lang-de.txt", "lang/de.txt", DE, false);
    let (fe, m) = fetcher(&cache, None);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Partial(DE.to_vec()));
    let e = ensure_wait(&fe, f).unwrap_err();
    assert!(e.contains("reset"), "{e}");
    assert!(!cache.join("lang/de.txt").exists());
    assert!(leftovers(&cache).is_empty());
    // A transport that says "done" early: the size check refuses it.
    let (fe, m) = fetcher(&cache, None);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Body(DE[..10].to_vec()));
    assert!(ensure_wait(&fe, f).is_err());
    assert!(!cache.join("lang/de.txt").exists());
    // Larger than the manifest says: stopped.
    let (fe, m) = fetcher(&cache, None);
    let big = vec![b'x'; 64 * 1024];
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Body(big));
    assert!(ensure_wait(&fe, f).unwrap_err().contains("larger than expected"));
    assert!(leftovers(&cache).is_empty());
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn one_download_for_many_requests() {
    let cache = tmp("once");
    let f = entry("lang-de.txt", "lang/de.txt", DE, false);
    let (fe, m) = fetcher(&cache, None);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Body(DE.to_vec()));
    *lock(&m.held) = true;
    let (tx, rx) = mpsc::channel();
    for _ in 0..4 {
        let tx = tx.clone();
        fe.ensure(f, Box::new(move |r| tx.send(r).unwrap()));
    }
    // The download is underway (and waits) before it is let go.
    let t = std::time::Instant::now();
    while m.waiting.load(Ordering::SeqCst) == 0 && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(fe.state(f), State::Downloading { got: 0, .. }));
    m.release();
    for _ in 0..4 {
        assert!(rx.recv_timeout(Duration::from_secs(20)).unwrap().is_ok());
    }
    assert_eq!(m.calls(&f.url(V)), 1, "a single download");
    assert_eq!(m.calls(&sums_url()), 1);
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn downloads_off() {
    let cache = tmp("off");
    let f = entry("selfie-test.bin", "models/selfie.bin", b"model", true);
    let (fe, m) = fetcher(&cache, None);
    fe.set_allowed(false);
    assert_eq!(ensure_wait(&fe, f).unwrap_err(), DISABLED);
    assert_eq!(m.calls(&sums_url()), 0, "nothing is asked for");
    assert_eq!(fe.state(f), State::Off(cache.join("models/selfie.bin")));
    // Put there by hand: wrong, then right.
    std::fs::create_dir_all(cache.join("models")).unwrap();
    std::fs::write(cache.join("models/selfie.bin"), b"modem!").unwrap();
    assert_eq!(fe.state(f), State::Damaged(cache.join("models/selfie.bin")));
    std::fs::write(cache.join("models/selfie.bin"), b"model").unwrap();
    assert_eq!(fe.state(f), State::Ready(cache.join("models/selfie.bin")));
    let _ = std::fs::remove_dir_all(&cache);
}

/// A file kept from another version counts when it has this version's
/// hash; one that differs is fetched again (and replaced).
#[test]
fn kept_files_by_hash() {
    let cache = tmp("kept");
    let f = entry("lang-de.txt", "lang/de.txt", DE, false);
    std::fs::create_dir_all(cache.join("lang")).unwrap();
    std::fs::write(cache.join("lang/de.txt"), DE).unwrap();
    let (fe, m) = fetcher(&cache, None);
    assert!(ensure_wait(&fe, f).is_ok());
    assert_eq!(m.calls(&sums_url()), 0, "same hash: no download");
    std::fs::write(cache.join("lang/de.txt"), b"# Deutsch (0.3)\n").unwrap();
    assert_eq!(fe.state(f), State::Missing);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, DE)])));
    m.set(&f.url(V), Resp::Body(DE.to_vec()));
    assert!(ensure_wait(&fe, f).is_ok());
    assert_eq!(std::fs::read(cache.join("lang/de.txt")).unwrap(), DE);
    let _ = std::fs::remove_dir_all(&cache);
}

/// The override folder: a language file there is taken as it is (a
/// translator's copy); the model only when it matches.
#[test]
fn override_folder() {
    let cache = tmp("ovc");
    let over = tmp("ovo");
    let lang = entry("lang-de.txt", "lang/de.txt", DE, false);
    let model = entry("selfie-test.bin", "models/selfie.bin", b"model", true);
    std::fs::create_dir_all(over.join("lang")).unwrap();
    std::fs::create_dir_all(over.join("models")).unwrap();
    std::fs::write(over.join("lang/de.txt"), b"edited").unwrap();
    std::fs::write(over.join("models/selfie.bin"), b"modem!").unwrap();
    let (fe, _m) = fetcher(&cache, Some(over.clone()));
    assert_eq!(fe.state(lang), State::Ready(over.join("lang/de.txt")));
    assert_eq!(fe.state(model), State::Missing);
    std::fs::write(over.join("models/selfie.bin"), b"model").unwrap();
    assert_eq!(fe.state(model), State::Ready(over.join("models/selfie.bin")));
    // Paths of our own only.
    assert_eq!(fe.find_rel("../lang/de.txt"), None);
    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir_all(&over);
}

/// An asset outside the manifest, or at the wrong place, is refused.
#[test]
fn ensure_takes_manifest_files_only() {
    let (tx, rx) = mpsc::channel();
    ensure("rustshot-9.9.9-setup.exe", "models/selfie.bin", move |r| tx.send(r).unwrap());
    assert!(rx.recv().unwrap().unwrap_err().contains("not a data file"));
    let (tx, rx) = mpsc::channel();
    ensure(MODEL.asset, "lang/de.txt", move |r| tx.send(r).unwrap());
    assert!(rx.recv().unwrap().is_err());
}

#[test]
fn manifest_is_consistent() {
    assert_eq!(by_rel("models/selfie.bin"), Some(&MODEL));
    assert_eq!(by_asset(MODEL.asset), Some(&MODEL));
    let mut assets: Vec<_> = manifest().map(|f| f.asset).collect();
    assets.sort_unstable();
    assets.dedup();
    assert_eq!(assets.len(), manifest().count(), "asset names are unique");
    for f in manifest() {
        assert!(crate::datadir::safe_rel(f.rel), "{}", f.rel);
        assert!(crate::update::is_safe_download_url(&f.url("1.2.3")), "{}", f.url("1.2.3"));
        assert_eq!(f.sha256.len(), 64);
        assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }
    for (tag, f) in lang_tags().zip(LANGS) {
        assert_eq!(f.asset, format!("lang-{tag}.txt"));
        assert_eq!(f.rel, format!("lang/{tag}.txt"));
        assert!(!f.strict);
    }
    // What build.rs listed is the source tree's lang/ folder, byte for byte.
    for f in LANGS {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(f.rel);
        assert!(f.check(&p).is_ok(), "{}", p.display());
    }
    assert_eq!(size_label(MODEL.size), "215 KB");
}

#[test]
fn errors_in_words() {
    assert_eq!(friendly("SHA256SUMS: WinHttpSendRequest: 0x80072EE7"), "no connection to github.com (SHA256SUMS: WinHttpSendRequest: 0x80072EE7)");
    assert!(friendly("curl: (28) Operation timed out").starts_with("github.com did not answer in time"));
    assert!(friendly("HTTP 404").starts_with("this version's release does not have the file"));
    assert!(friendly("selfie-f11cb3ec.bin is not listed in SHA256SUMS").starts_with("this version's release does not have the file"));
    assert_eq!(friendly("lang-de.txt failed its SHA-256 check"), "lang-de.txt failed its SHA-256 check");
}

/// Downloads start off (fail closed) until the config says otherwise:
/// a new fetcher refuses, the config's switch decides, a broken file is
/// off.
#[test]
fn downloads_start_off_until_the_config_says() {
    let d = tmp("off-first");
    let m = Arc::new(Mock::default());
    let fe = Arc::new(Fetcher::new(Some(d.clone()), None, V, Box::new(Arc::clone(&m))));
    let f = entry("first-off.bin", "models/first-off.bin", b"abc", true);
    assert!(!fe.allowed());
    assert!(matches!(fe.state(f), State::Off(_)));
    assert_eq!(ensure_wait(&fe, f), Err(DISABLED.into()));
    assert_eq!(m.calls(&sums_url()), 0, "nothing was asked of the network");
    use crate::config::Config;
    assert!(initial_allowed(Ok(None)), "no config yet: the default (on)");
    assert!(initial_allowed(Ok(Some(Config::default()))));
    assert!(!initial_allowed(Ok(Some(Config { download_data: false, ..Config::default() }))));
    assert!(!initial_allowed(Err("line 3: expected key = value".into())), "a broken file: off");
}

/// This version's release lacks the file (not released yet; released
/// without it): the newest release's, when it lists the same SHA-256.
#[test]
fn falls_back_to_the_newest_release_with_the_same_file() {
    let d = tmp("fallback");
    let data = b"the same model bytes";
    let f = entry("fb-11223344.bin", "models/fb.bin", data, true);
    let m = Arc::new(Mock::default());
    let fe = Fetcher::new(Some(d.clone()), None, V, Box::new(Arc::clone(&m))).with_fallback(|| Ok("1.2.3".into()));
    fe.set_allowed(true);
    let fe = Arc::new(fe);
    let old = |name: &str| format!("{}v1.2.3/{name}", crate::update::DOWNLOAD_PREFIX);
    // This version: no release (404); the newest: has it.
    m.set(&old("SHA256SUMS"), Resp::Body(sums_for(&[(f, data)])));
    m.set(&old(f.asset), Resp::Body(data.to_vec()));
    let p = ensure_wait(&fe, f).expect("from v1.2.3");
    assert_eq!(std::fs::read(&p).unwrap(), data);
    assert_eq!(m.calls(&sums_url()), 1);
    // Released without it (not listed): the newest again.
    let d2 = tmp("fallback2");
    let m2 = Arc::new(Mock::default());
    let fe2 = Fetcher::new(Some(d2.clone()), None, V, Box::new(Arc::clone(&m2))).with_fallback(|| Ok("1.2.3".into()));
    fe2.set_allowed(true);
    let fe2 = Arc::new(fe2);
    m2.set(&sums_url(), Resp::Body(b"0000000000000000000000000000000000000000000000000000000000000000  other.bin\n".to_vec()));
    m2.set(&old("SHA256SUMS"), Resp::Body(sums_for(&[(f, data)])));
    m2.set(&old(f.asset), Resp::Body(data.to_vec()));
    assert!(ensure_wait(&fe2, f).is_ok());
    // The newest has another file under that name: refused (the built-in
    // SHA-256 decides), and the error is this version's.
    let d3 = tmp("fallback3");
    let m3 = Arc::new(Mock::default());
    let fe3 = Fetcher::new(Some(d3.clone()), None, V, Box::new(Arc::clone(&m3))).with_fallback(|| Ok("1.2.3".into()));
    fe3.set_allowed(true);
    let fe3 = Arc::new(fe3);
    m3.set(&old("SHA256SUMS"), Resp::Body(sums_for(&[(f, b"other bytes")])));
    let e = ensure_wait(&fe3, f).unwrap_err();
    assert!(e.contains("not the one this version expects"), "{e}");
    assert_eq!(m3.calls(&old(f.asset)), 0, "never downloaded");
    // No newer answer, or a nonsense version: this version's error, in words.
    for latest in [Err("offline".to_string()), Ok(V.to_string()), Ok("1.2/../x".to_string())] {
        let d4 = tmp("fallback4");
        let m4 = Arc::new(Mock::default());
        let l = latest.clone();
        let fe4 = Fetcher::new(Some(d4.clone()), None, V, Box::new(Arc::clone(&m4))).with_fallback(move || l.clone());
        fe4.set_allowed(true);
        let e = ensure_wait(&Arc::new(fe4), f).unwrap_err();
        assert!(e.starts_with("this version's release does not have the file"), "{e}");
    }
}

/// A transport that gives up (a stall, a timeout) ends the job as Failed
/// with Retry: never stuck "downloading".
#[test]
fn a_stalled_download_ends_failed() {
    let d = tmp("stall");
    let (fe, m) = fetcher(&d, None);
    let f = entry("stall.bin", "models/stall.bin", b"xyz", true);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, b"xyz")])));
    m.set(&f.url(V), Resp::Fail("download stalled (timed out)".into()));
    let e = ensure_wait(&fe, f).unwrap_err();
    assert!(e.starts_with("github.com did not answer in time"), "{e}");
    assert!(matches!(fe.state(f), State::Failed(_)));
    assert!(lock(&fe.jobs).is_empty());
}

/// Temporary files of a download killed midway are swept when the next
/// one starts (an hour old), fresh ones (another process's) are kept.
#[test]
fn stale_part_files_are_swept() {
    let d = tmp("sweep");
    let dir = d.join("models");
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join(".sweep.bin.1.0.part");
    let fresh = dir.join(".sweep.bin.2.0.part");
    let mine = dir.join("keep.bin");
    for p in [&old, &fresh, &mine] {
        std::fs::write(p, b"x").unwrap();
    }
    let t = std::time::SystemTime::now() - Duration::from_secs(7200);
    std::fs::File::options().write(true).open(&old).unwrap().set_modified(t).unwrap();
    let (fe, m) = fetcher(&d, None);
    let f = entry("sweep.bin", "models/sweep.bin", b"abc", true);
    m.set(&sums_url(), Resp::Body(sums_for(&[(f, b"abc")])));
    m.set(&f.url(V), Resp::Body(b"abc".to_vec()));
    ensure_wait(&fe, f).unwrap();
    assert!(!old.exists() && fresh.exists() && mine.exists());
}

#[test]
fn time_limits_scale_with_the_size() {
    use crate::update::download_time_limit as t;
    assert_eq!(t(None).as_secs(), 600);
    assert_eq!(t(Some(219_338)).as_secs(), 60 + 219_338 / 4096);
    assert_eq!(t(Some(64 * 1024)).as_secs(), 76);
    assert_eq!(t(Some(1 << 40)).as_secs(), 600);
}
