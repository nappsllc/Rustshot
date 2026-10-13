//! UI translations. Every user-visible string goes through [`tr!`] (or
//! [`trn!`] for a count, [`key!`] where a constant needs to hold one) with
//! a key of `lang/en.txt`. English is the source and the fallback and the
//! only language in the binary: build.rs packs it (DEFLATE-compressed,
//! inflated on first use). Every other language is a file read at run
//! time, `lang/<tag>.txt` in the data folders ([`files`],
//! [`crate::datadir`]): plain `key = value` lines that translators can
//! edit in place; a key a file lacks, or gets wrong, is English. Keys are
//! checked at compile time: an unknown key, or a named argument the
//! English string has no `{placeholder}` for, does not build.
//!
//! The language is Settings › General › Language (`language` in
//! config.toml): empty for the system's, else a tag. Switching it bumps
//! [`generation`]; open windows re-lay themselves out on their next timer
//! tick and right-to-left languages mirror the decorated windows
//! ([`is_rtl`]).
//!
//! Tests also have two pseudo-languages made from English (never shipped):
//! `qps` (accented, 40 % longer, bracketed: overflow and concatenation
//! show) and `qps-rtl` (Hebrew letters, right to left, 40 % longer:
//! mirroring and the OS text path).

pub mod fetch;
pub mod files;
pub mod plural;
#[cfg(test)]
pub mod pseudo;
mod sys;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[allow(dead_code, clippy::all)]
mod table {
    include!(concat!(env!("OUT_DIR"), "/i18n_table.rs"));
}

/// A translatable string: an index into the key table, checked when the
/// constant is evaluated ([`Key::of`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Key(u16);

const fn cmp(a: &str, b: &str) -> i32 {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut i = 0;
    while i < a.len() && i < b.len() {
        if a[i] != b[i] {
            return a[i] as i32 - b[i] as i32;
        }
        i += 1;
    }
    a.len() as i32 - b.len() as i32
}

const fn find(k: &str) -> Option<usize> {
    let (mut lo, mut hi) = (0usize, table::N);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let c = cmp(table::KEYS[mid], k);
        if c == 0 {
            return Some(mid);
        }
        if c < 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

impl Key {
    /// The plain (not plural) key `k`: a compile error in a constant when
    /// lang/en.txt has no such key.
    pub const fn of(k: &str) -> Key {
        match find(k) {
            Some(i) if !table::PLURAL[i] => Key(i as u16),
            Some(_) => panic!("a plural i18n key: use trn!"),
            None => panic!("unknown i18n key: add it to lang/en.txt"),
        }
    }

    /// The plural key `k` (`k[one]`, `k[other]`... in lang/en.txt).
    pub const fn of_plural(k: &str) -> Key {
        match find(k) {
            Some(i) if table::PLURAL[i] => Key(i as u16),
            Some(_) => panic!("not a plural i18n key: use tr!"),
            None => panic!("unknown i18n key: add it to lang/en.txt"),
        }
    }

    /// Whether the English string has `{name}` (compile-time argument checks).
    pub const fn has_ph(self, name: &str) -> bool {
        let ph = table::PH[self.0 as usize];
        let mut i = 0;
        while i < ph.len() {
            if cmp(ph[i], name) == 0 {
                return true;
            }
            i += 1;
        }
        false
    }

    /// The key's name (`settings.general.language`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn name(self) -> &'static str {
        table::KEYS[self.0 as usize]
    }

    /// The string in the current language (English when it has none).
    pub fn get(self) -> &'static str {
        let lang = current_index();
        let v = table_of(lang)[self.0 as usize];
        if v.is_empty() { table_of(0)[self.0 as usize] } else { v }
    }

    /// The plural form for `n` in the current language (its `other`, then
    /// English, when it lacks that form).
    pub fn plural(self, n: i64) -> &'static str {
        let lang = current_index();
        let pick = |lang: usize| -> &'static str {
            let v = table_of(lang)[self.0 as usize];
            if v.is_empty() {
                return "";
            }
            let forms: Vec<&'static str> = v.split('\u{1f}').collect();
            let c = plural::Rule::from_name(plural_name(lang)).category(n.unsigned_abs()) as usize;
            match forms.get(c) {
                Some(f) if !f.is_empty() => f,
                _ => forms.get(5).copied().unwrap_or(""),
            }
        };
        let s = pick(lang);
        if s.is_empty() { pick(0) } else { s }
    }
}

/// A key reads as its string: `&key` passes for a `&str`, `key.len()`,
/// `format!("{key}")` and comparisons with strings work.
impl std::ops::Deref for Key {
    type Target = str;
    fn deref(&self) -> &str {
        self.get()
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.get())
    }
}

impl From<Key> for String {
    fn from(k: Key) -> String {
        k.get().to_string()
    }
}

impl From<Key> for &'static str {
    fn from(k: Key) -> &'static str {
        k.get()
    }
}

impl PartialEq<str> for Key {
    fn eq(&self, o: &str) -> bool {
        self.get() == o
    }
}

impl PartialEq<&str> for Key {
    fn eq(&self, o: &&str) -> bool {
        self.get() == *o
    }
}

impl PartialEq<String> for Key {
    fn eq(&self, o: &String) -> bool {
        self.get() == o
    }
}

impl PartialEq<Key> for &str {
    fn eq(&self, o: &Key) -> bool {
        *self == o.get()
    }
}

impl PartialEq<Key> for String {
    fn eq(&self, o: &Key) -> bool {
        self == o.get()
    }
}

/// A label in a table: a [`Key`], or a `&'static str` that is the same in
/// every language (a format name, a number).
pub trait Label: Copy {
    fn text(self) -> &'static str;
}

impl Label for Key {
    fn text(self) -> &'static str {
        self.get()
    }
}

impl Label for &'static str {
    fn text(self) -> &'static str {
        self
    }
}

/// The key named `k` at run time (keys built from ids); None when there
/// is none.
pub fn lookup(k: &str) -> Option<Key> {
    table::KEYS.binary_search(&k).ok().filter(|&i| !table::PLURAL[i]).map(|i| Key(i as u16))
}

/// The string of the key named `k` at run time; the key itself when there
/// is none (a test checks every name these are called with exists).
#[allow(dead_code)]
pub fn t(k: &str) -> &'static str {
    match lookup(k) {
        Some(key) => key.get(),
        None => {
            debug_assert!(false, "unknown i18n key {k}");
            "?"
        }
    }
}

/// `s` with each `{name}` replaced by its argument (unknown ones kept).
pub fn fmt(s: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 16);
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}').map(|j| (&after[..j], j)) {
            Some((name, j)) if let Some((_, v)) = args.iter().find(|(n, _)| *n == name) => {
                let _ = write!(out, "{v}");
                rest = &after[j + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The key `$k` as a [`Key`] constant (for tables of strings).
#[macro_export]
macro_rules! key {
    ($k:literal) => {{
        const K: $crate::i18n::Key = $crate::i18n::Key::of($k);
        K
    }};
}

/// The string of key `$k` in the current language (`&'static str`), or
/// with `{name}` placeholders filled (`name = value` or just `name` for a
/// variable of that name; a `String`).
#[macro_export]
macro_rules! tr {
    ($k:literal) => {
        $crate::key!($k).get()
    };
    ($k:literal, $($name:ident $(= $v:expr)?),+ $(,)?) => {{
        const K: $crate::i18n::Key = $crate::i18n::Key::of($k);
        const _: () = { $( assert!(K.has_ph(stringify!($name)), concat!("i18n: no {{", stringify!($name), "}} in ", $k)); )+ };
        $crate::i18n::fmt(K.get(), &[$((stringify!($name), &$crate::__tr_arg!($name $(= $v)?) as &dyn ::std::fmt::Display)),+])
    }};
}

/// The plural form of key `$k` for count `$n` (also `{n}` in the string),
/// placeholders filled as in [`tr!`]; a `String`.
#[macro_export]
macro_rules! trn {
    ($k:literal, $n:expr $(, $name:ident $(= $v:expr)?)* $(,)?) => {{
        const K: $crate::i18n::Key = $crate::i18n::Key::of_plural($k);
        const _: () = { $( assert!(K.has_ph(stringify!($name)), concat!("i18n: no {{", stringify!($name), "}} in ", $k)); )* };
        let n = $n;
        $crate::i18n::fmt(K.plural(n as i64), &[("n", &n as &dyn ::std::fmt::Display) $(, (stringify!($name), &$crate::__tr_arg!($name $(= $v)?) as &dyn ::std::fmt::Display))*])
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __tr_arg {
    ($name:ident) => {
        $name
    };
    ($name:ident = $v:expr) => {
        $v
    };
}

// ---------------------------------------------------------------- tables

/// A known language: its tag, its own name, its direction (all
/// embedded: build.rs reads them from lang/), and its strings once read.
struct Known {
    code: &'static str,
    name: &'static str,
    rtl: bool,
    /// Set once its file is read (English and the test languages at first
    /// use; a downloaded language when its file is there).
    parsed: OnceLock<files::Parsed>,
}

/// The languages: English, then every language of lang/ (whether its file
/// is here yet or not), by tag; tests add the pseudo-languages.
fn registry() -> &'static [Known] {
    static R: OnceLock<Vec<Known>> = OnceLock::new();
    R.get_or_init(|| {
        let mut v = vec![Known { code: "en", name: "English", rtl: false, parsed: OnceLock::new() }];
        v.extend(table::AVAILABLE.iter().map(|&(code, name, rtl)| Known { code, name, rtl, parsed: OnceLock::new() }));
        #[cfg(test)]
        v.extend([
            Known { code: "qps", name: "Pseudo (accented, long)", rtl: false, parsed: OnceLock::new() },
            Known { code: "qps-rtl", name: "Pseudo (right to left)", rtl: true, parsed: OnceLock::new() },
            // A downloadable language (the download tests).
            Known { code: "zz-test", name: "Zet", rtl: false, parsed: OnceLock::new() },
            Known { code: "zz-ui", name: "Zet UI", rtl: false, parsed: OnceLock::new() },
        ]);
        v
    })
}

fn known(i: usize) -> &'static Known {
    let r = registry();
    &r[i.min(r.len() - 1)]
}

/// English's table, inflated once.
fn english() -> &'static files::Parsed {
    known(0).parsed.get_or_init(|| {
        let raw = match crate::deflate::inflate(table::EN_Z, table::EN_RAW_LEN) {
            Ok((b, _)) if b.len() == table::EN_RAW_LEN => b,
            _ => vec![0; table::N],
        };
        let s: &'static str = Box::leak(String::from_utf8(raw).unwrap_or_default().into_boxed_str());
        let mut values: Vec<&'static str> = s.split('\0').take(table::N).collect();
        values.resize(table::N, "");
        files::Parsed { name: "English".into(), rtl: false, plural: "one_other".into(), values, problems: Vec::new() }
    })
}

/// Language `i` if its strings are here (English always; a test
/// pseudo-language made on first use; a downloaded language once
/// [`try_load`] found its file).
fn loaded(i: usize) -> Option<&'static files::Parsed> {
    if i == 0 {
        return Some(english());
    }
    let k = known(i);
    #[cfg(test)]
    if k.code.starts_with("qps") {
        let rtl = k.rtl;
        return Some(k.parsed.get_or_init(|| {
            let values = english()
                .values
                .iter()
                .map(|v| -> &'static str {
                    let forms: Vec<String> = v.split('\u{1f}').map(|f| if f.is_empty() { String::new() } else { pseudo::pseudo(f, rtl) }).collect();
                    Box::leak(forms.join("\u{1f}").into_boxed_str())
                })
                .collect();
            files::Parsed { name: k.name.into(), rtl, plural: "one_other".into(), values, problems: Vec::new() }
        }));
    }
    k.parsed.get()
}

/// Read language `i`'s file if one is here and it is not read yet: the
/// downloaded copy ([`fetch::cached`]), with the data folders' own
/// `lang/<tag>.txt` (a package's or the user's fixes) over it. True when
/// the language is loaded now.
fn try_load(i: usize) -> bool {
    if loaded(i).is_some() {
        return true;
    }
    let k = known(i);
    let mut paths: Vec<std::path::PathBuf> = fetch::cached(k.code).into_iter().collect();
    for (tag, p) in files::discover(&files::lang_dirs()) {
        if tag.eq_ignore_ascii_case(k.code) {
            paths.extend(p);
        }
    }
    if paths.is_empty() {
        return false;
    }
    let _ = k.parsed.set(files::load(k.code, &paths));
    true
}

/// The values of language `lang` in key order ("": English's); English's
/// while its file is not here.
fn table_of(lang: usize) -> &'static [&'static str] {
    &loaded(lang).unwrap_or_else(english).values
}

fn plural_name(lang: usize) -> &'static str {
    &loaded(lang).unwrap_or_else(english).plural
}

// ---------------------------------------------------------------- language

/// A selectable language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lang {
    pub code: &'static str,
    /// Its own name for itself ("Deutsch").
    pub name: &'static str,
    pub rtl: bool,
}

fn lang_at(i: usize) -> Lang {
    let k = known(i);
    Lang { code: k.code, name: k.name, rtl: k.rtl }
}

/// The languages Settings offers: English, then every language of the
/// release, by tag (downloaded when picked; tests: the pseudo-languages).
pub fn languages() -> Vec<Lang> {
    (0..registry().len()).map(lang_at).collect()
}

/// The language the system's settings pick ([`best_match`]; English in
/// tests).
pub fn system_language() -> &'static str {
    known(if cfg!(test) { 0 } else { best_match(&sys::preferred()) }).code
}

/// Language `code`'s own name (the tag when it is not known).
pub fn name_of(code: &str) -> &'static str {
    index_of(code).map_or("?", |i| known(i).name)
}

/// Whether language `code`'s strings are here (English always).
pub fn is_loaded(code: &str) -> bool {
    index_of(code).is_some_and(try_load)
}

/// The language folders looked in for local files (`lang/` of each data
/// folder, highest priority first).
#[allow(dead_code)]
pub fn data_dirs() -> Vec<std::path::PathBuf> {
    files::lang_dirs()
}

/// The index of language `code` (exact, case-insensitive).
fn index_of(code: &str) -> Option<usize> {
    registry().iter().position(|k| k.code.eq_ignore_ascii_case(code))
}

/// The best language for BCP 47 tags in preference order (`de-AT`,
/// `pt_BR.UTF-8`, `zh-Hant-TW`): an exact match, then the script or
/// region forms of Chinese, Portuguese, Norwegian and Serbian, then the
/// first language of the same primary subtag; English when none matches.
pub fn best_match(tags: &[String]) -> usize {
    best_in(tags, &registry().iter().map(|k| k.code).collect::<Vec<_>>())
}

/// [`best_match`] among `codes` (English first).
pub fn best_in(tags: &[String], codes: &[&str]) -> usize {
    let real = || codes.iter().copied().enumerate().filter(|(_, c)| !c.starts_with("qps"));
    for t in tags {
        let t = t.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
        if t.is_empty() || t == "c" || t == "posix" {
            continue;
        }
        if let Some((i, _)) = real().find(|(_, c)| c.eq_ignore_ascii_case(&t)) {
            return i;
        }
        let parts: Vec<&str> = t.split('-').collect();
        let want: Option<&str> = match parts[0] {
            "zh" if parts.iter().any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo")) => Some("zh-hant"),
            "zh" => Some("zh-hans"),
            "pt" if parts.get(1) == Some(&"pt") => Some("pt-pt"),
            "pt" => Some("pt-br"),
            "no" | "nn" => Some("nb"),
            "sr" | "sh" => Some("sr-latn"),
            _ => None,
        };
        if let Some(w) = want
            && let Some((i, _)) = real().find(|(_, c)| c.eq_ignore_ascii_case(w))
        {
            return i;
        }
        if let Some((i, _)) = real().find(|(_, c)| c.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(parts[0]))) {
            return i;
        }
    }
    0
}

static CUR: AtomicUsize = AtomicUsize::new(usize::MAX);
static GEN: AtomicU64 = AtomicU64::new(0);

fn current_index() -> usize {
    #[cfg(test)]
    if let Some(i) = OVERRIDE.with(|o| o.get()) {
        return i;
    }
    let i = CUR.load(Ordering::Relaxed);
    if i != usize::MAX {
        return i;
    }
    init();
    CUR.load(Ordering::Relaxed).min(registry().len() - 1)
}

/// The first use in a process reads config.toml's `language` (tests stay
/// in English unless they set one).
fn init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let setting = if cfg!(test) {
            "en".to_string()
        } else {
            crate::config::read_at(&crate::config::config_path()).ok().flatten().map(|c| c.language).unwrap_or_default()
        };
        *lock(&SETTING) = setting.clone();
        let i = resolve(&setting);
        let _ = CUR.compare_exchange(usize::MAX, i, Ordering::SeqCst, Ordering::SeqCst);
        // Not here yet (the system's language on a first run, or a file
        // the cache lost): English until it is downloaded.
        if !cfg!(test) && !try_load(i) {
            fetch::request(known(i).code);
        }
    });
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The `language` setting in force (empty: the system's).
static SETTING: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// A language the system's setting switched to when its download landed,
/// for the daemon's one notice ([`take_auto_switch`]).
static AUTO: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

/// Something changed that windows show: they lay themselves out again.
pub(crate) fn bump() {
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// Language `tag`'s file landed (and is loaded): if it is the language in
/// force, every window shows it now; if the system's setting chose it,
/// [`take_auto_switch`] reports it once.
pub(crate) fn downloaded(tag: &str) {
    let Some(i) = index_of(tag) else { return };
    if CUR.load(Ordering::SeqCst) == i {
        let setting = lock(&SETTING).clone();
        if setting.trim().is_empty() || setting.trim().eq_ignore_ascii_case("system") {
            *lock(&AUTO) = Some(known(i).name);
        }
    }
    bump();
}

/// The language the system's setting just switched to (its own name),
/// once: the daemon says "Rustshot is now in …" with "Use English".
#[allow(dead_code)]
pub fn take_auto_switch() -> Option<&'static str> {
    lock(&AUTO).take()
}

/// The language a `language` setting means: a known code, else the
/// system's.
fn resolve(setting: &str) -> usize {
    let s = setting.trim();
    if !s.is_empty()
        && !s.eq_ignore_ascii_case("system")
        && let Some(i) = index_of(s)
    {
        return i;
    }
    // Tests never follow the machine's language.
    if cfg!(test) {
        return 0;
    }
    best_match(&sys::preferred())
}

/// Switch to the `language` setting (empty: the system's); returns
/// whether the language changed (and [`generation`] moved on).
pub fn set_language(setting: &str) -> bool {
    current_index();
    *lock(&SETTING) = setting.to_string();
    let i = resolve(setting);
    // Not downloaded yet: English until it lands (then the windows switch).
    if !cfg!(test) && !try_load(i) {
        fetch::request(known(i).code);
    }
    let old = CUR.swap(i, Ordering::SeqCst);
    if old != i {
        GEN.fetch_add(1, Ordering::SeqCst);
        true
    } else {
        false
    }
}

/// The current language's code (the one in force: English shows until
/// its file is here).
#[allow(dead_code)]
pub fn current() -> &'static str {
    lang_at(current_index()).code
}

/// Whether the current language is written right to left (the decorated
/// windows mirror, the toolbar runs right to left).
pub fn is_rtl() -> bool {
    let i = current_index();
    loaded(i).is_some() && known(i).rtl
}

/// Bumped by every language switch: a window that drew with another
/// generation lays itself out again.
pub fn generation() -> u64 {
    GEN.load(Ordering::Relaxed)
}

#[cfg(test)]
thread_local! {
    /// [`with_lang`]'s language on this thread (tests run in parallel).
    static OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Run `f` with this thread in language `code`, then switch back.
#[cfg(test)]
pub fn with_lang<R>(code: &str, f: impl FnOnce() -> R) -> R {
    let i = index_of(code).unwrap_or_else(|| panic!("no language {code}"));
    let old = OVERRIDE.with(|o| o.replace(Some(i)));
    struct Back(Option<usize>);
    impl Drop for Back {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _back = Back(old);
    f()
}

#[cfg(test)]
mod tests;
