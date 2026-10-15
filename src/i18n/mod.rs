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
        names()[self.0 as usize]
    }

    /// The string in the current language (English when it has none).
    /// (Never inlined: a thousand call sites stay a call each.)
    #[inline(never)]
    pub fn get(self) -> &'static str {
        let lang = current_index();
        let v = table_of(lang)[self.0 as usize];
        if v.is_empty() { table_of(0)[self.0 as usize] } else { v }
    }

    /// The current language has this string itself (English always does):
    /// not the English one a missing key falls back to.
    pub fn own(self) -> bool {
        let lang = current_index();
        lang == 0 || !table_of(lang)[self.0 as usize].is_empty()
    }

    /// The plural form for `n` in the current language (its `other`, then
    /// English, when it lacks that form).
    #[inline(never)]
    pub fn plural(self, n: i64) -> &'static str {
        let lang = current_index();
        let pick = |lang: usize| -> &'static str {
            let v = table_of(lang)[self.0 as usize];
            if v.is_empty() {
                return "";
            }
            let c = plural::Rule::from_name(plural_name(lang)).category(n.unsigned_abs()) as usize;
            match v.split('\u{1f}').nth(c) {
                Some(f) if !f.is_empty() => f,
                _ => v.split('\u{1f}').nth(5).unwrap_or(""),
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
    names().binary_search(&k).ok().filter(|&i| !table::PLURAL[i]).map(|i| Key(i as u16))
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

/// Right to left, a run of numbers joined by separators (`1180 × 760`,
/// `724, 120`, `3/4`, `10–20`), a signed number (`-5`) or one grouped by
/// thin spaces (`1 234`) is wrapped in a left-to-right isolate (LRI …
/// PDI), so it keeps its order inside right-to-left text instead of
/// reading backwards (`760 × 1180`). Strings that already hold an isolate
/// are left as they are. [`fmt`] does this for every filled-in string
/// while the UI runs right to left; the text renderers draw the isolates
/// as nothing (crate::text::is_bidi_control).
pub fn isolate_numbers(s: &str) -> String {
    const LRI: char = '\u{2066}';
    const PDI: char = '\u{2069}';
    if !s.bytes().any(|b| b.is_ascii_digit()) || s.contains([LRI, '\u{2067}', '\u{2068}', PDI]) {
        return s.to_string();
    }
    let c: Vec<char> = s.chars().collect();
    let digit = |i: usize| c.get(i).is_some_and(|c| c.is_ascii_digit());
    let space = |i: usize| c.get(i).is_some_and(|&c| matches!(c, ' ' | '\u{a0}' | '\u{2009}' | '\u{202f}'));
    // A number from `i` (digits, inner `.`/`,`/thin-space groups, a `%`):
    // its end and whether it holds a group space.
    let number = |i: usize| -> Option<(usize, bool)> {
        if !digit(i) {
            return None;
        }
        let (mut j, mut grouped) = (i, false);
        loop {
            while digit(j) {
                j += 1;
            }
            match c.get(j) {
                Some('.' | ',') if digit(j + 1) => j += 1,
                Some('\u{2009}' | '\u{202f}') if digit(j + 1) => {
                    grouped = true;
                    j += 1;
                }
                _ => break,
            }
        }
        if c.get(j) == Some(&'%') {
            j += 1;
        }
        Some((j, grouped))
    };
    // A separator from `i` and the number after it: that number's end.
    let joined = |i: usize| -> Option<(usize, bool)> {
        let mut j = i;
        while space(j) && j < i + 2 {
            j += 1;
        }
        if !c.get(j).is_some_and(|&c| matches!(c, '×' | 'x' | ',' | ':' | '/' | '–' | '-')) {
            return None;
        }
        j += 1;
        while space(j) && j < i + 4 {
            j += 1;
        }
        number(j)
    };
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < c.len() {
        let word = i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_');
        let sign = matches!(c[i], '-' | '+' | '\u{2212}') && digit(i + 1) && !word;
        let start = if sign { i + 1 } else { i };
        let Some((mut end, mut grouped)) = (!word || sign).then(|| number(start)).flatten() else {
            // Not a number's start: the character, or a word's whole digit run.
            let alnum = c[i].is_alphanumeric();
            out.push(c[i]);
            i += 1;
            while alnum && digit(i) {
                out.push(c[i]);
                i += 1;
            }
            continue;
        };
        let mut n = 1;
        while let Some((e, g)) = joined(end) {
            (end, grouped, n) = (e, grouped || g, n + 1);
        }
        let wrap = sign || grouped || n > 1;
        if wrap {
            out.push(LRI);
        }
        out.extend(&c[i..end]);
        if wrap {
            out.push(PDI);
        }
        i = end;
    }
    out
}

/// `s` with each `{name}` replaced by its argument (unknown ones kept);
/// right to left, number pairs kept in order ([`isolate_numbers`]).
#[inline(never)]
pub fn fmt(s: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 16);
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        // (No `if let` guard: the Flatpak SDK's rustc predates them.)
        let hit = after.find('}').and_then(|j| args.iter().find(|(n, _)| *n == &after[..j]).map(|(_, v)| (v, j)));
        match hit {
            Some((v, j)) => {
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
    if is_rtl() { isolate_numbers(&out) } else { out }
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
    /// English and the test languages: made at first use.
    parsed: OnceLock<files::Parsed>,
    /// A language's file once read (null until then); replaced (the old
    /// one leaked: a reload is rare) when its files change.
    live: std::sync::atomic::AtomicPtr<files::Parsed>,
    /// Its files as last read or looked at, and when (a check at most
    /// once a second).
    seen: std::sync::Mutex<Option<(Vec<Sig>, std::time::Instant)>>,
}

/// A file's identity for reloading: its path, size and modification time
/// (none: missing).
type Sig = (std::path::PathBuf, Option<(u64, std::time::SystemTime)>);

impl Known {
    const fn new(code: &'static str, name: &'static str, rtl: bool) -> Known {
        Known { code, name, rtl, parsed: OnceLock::new(), live: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()), seen: std::sync::Mutex::new(None) }
    }
}

/// The languages: English, then every language of lang/ (whether its file
/// is here yet or not), by tag; tests add the pseudo-languages.
fn registry() -> &'static [Known] {
    static R: OnceLock<Vec<Known>> = OnceLock::new();
    R.get_or_init(|| {
        let mut v = vec![Known::new("en", "English", false)];
        v.extend(table::AVAILABLE.iter().map(|&(code, name, rtl)| Known::new(code, name, rtl)));
        #[cfg(test)]
        v.extend([
            Known::new("qps", "Pseudo (accented, long)", false),
            Known::new("qps-rtl", "Pseudo (right to left)", true),
            Known::new("qps-ar", "Pseudo (Arabic letters)", true),
            // A downloadable language (the download tests).
            Known::new("zz-test", "Zet", false),
            Known::new("zz-ui", "Zet UI", false),
            Known::new("zz-two", "Zet Two", false),
            // A catalog the fit tests' loader reads (`load_fit_langs`).
            Known::new("zz-fit", "Zet Fit", false),
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
        let mut parts = s.split('\0');
        let mut values: Vec<&'static str> = parts.by_ref().take(table::N).collect();
        values.resize(table::N, "");
        let mut names: Vec<&'static str> = parts.take(table::N).collect();
        names.resize(table::N, "");
        let _ = NAMES.set(names);
        files::Parsed { name: "English".into(), rtl: false, plural: "one_other".into(), values, problems: Vec::new() }
    })
}

static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();

/// The key names, sorted (from English's table: `table::KEYS` is for the
/// compile-time checks only).
pub(crate) fn names() -> &'static [&'static str] {
    english();
    NAMES.get().map_or(&[], |v| v)
}

/// The `{placeholders}` of key `i`'s English string (all its forms).
pub(crate) fn placeholders_of(i: usize) -> Vec<&'static str> {
    let v = english().values[i];
    let mut out = Vec::new();
    let mut rest = v;
    while let Some(a) = rest.find('{') {
        rest = &rest[a + 1..];
        if let Some(b) = rest.find('}') {
            let name = &rest[..b];
            if !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                out.push(name);
                rest = &rest[b + 1..];
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
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
        let sc = pseudo::Script::of(k.code);
        return Some(k.parsed.get_or_init(|| {
            let values = english()
                .values
                .iter()
                .map(|v| -> &'static str {
                    let forms: Vec<String> = v.split('\u{1f}').map(|f| if f.is_empty() { String::new() } else { pseudo::pseudo(f, sc) }).collect();
                    Box::leak(forms.join("\u{1f}").into_boxed_str())
                })
                .collect();
            files::Parsed { name: k.name.into(), rtl, plural: "one_other".into(), values, problems: Vec::new() }
        }));
    }
    let p = k.live.load(Ordering::Acquire);
    // SAFETY: set only from a leaked Box (never freed).
    (!p.is_null()).then(|| unsafe { &*p })
}

/// Language `i`'s files, lowest priority first: the downloaded copy
/// ([`fetch::cached`]), then the data folders' own `lang/<tag>.txt` (a
/// package's or the user's fixes) over it.
fn files_of(i: usize) -> Vec<std::path::PathBuf> {
    let k = known(i);
    // The usable copy (the override folder's, else the cache's when it is
    // this version's: [`crate::datadir::cached`]); the override folder's
    // own file on top when the cache has one too (a translator's fixes).
    let mut paths: Vec<std::path::PathBuf> = fetch::cached(k.code).into_iter().collect();
    if let Some(p) = files::override_lang_dir().map(|d| d.join(format!("{}.txt", k.code))).filter(|p| p.is_file())
        && !paths.contains(&p)
    {
        paths.push(p);
    }
    paths
}

fn sigs(paths: &[std::path::PathBuf]) -> Vec<Sig> {
    paths.iter().map(|p| (p.clone(), std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH))))).collect()
}

/// Read language `i`'s files when they are new or changed (size or
/// modification time), looking at most once a second unless `force` (a
/// download just landed). A read whose files changed while it ran (one
/// being written) is not used: the next look reads it again. (The cache
/// is published whole, written then renamed: see [`fetch::Fetch`].)
/// True when the language is loaded.
fn refresh(i: usize, force: bool) -> bool {
    let k = known(i);
    let mut seen = lock(&k.seen);
    let have = !k.live.load(Ordering::Acquire).is_null();
    if !force && let Some((_, at)) = &*seen
        && at.elapsed() < std::time::Duration::from_secs(1)
    {
        return have;
    }
    let paths = files_of(i);
    let before = sigs(&paths);
    if have && !force && seen.as_ref().is_some_and(|(s, _)| *s == before) {
        *seen = Some((before, std::time::Instant::now()));
        return true;
    }
    if paths.is_empty() {
        *seen = Some((before, std::time::Instant::now()));
        return have;
    }
    let p = files::load(k.code, &paths);
    let after = sigs(&paths);
    if after != before {
        // Being written: look again next time.
        *seen = None;
        return have;
    }
    *seen = Some((after, std::time::Instant::now()));
    // The fonts its scripts need, found in the background (Linux).
    crate::text::prefetch_fonts(&p.values.concat());
    k.live.store(Box::into_raw(Box::new(p)), Ordering::Release);
    if have {
        bump();
    }
    true
}

/// Language `i` is loaded (its files read, or read now if they are here
/// or changed: a look at most once a second).
fn try_load(i: usize) -> bool {
    if i == 0 {
        return true;
    }
    #[cfg(test)]
    if known(i).code.starts_with("qps") {
        return true;
    }
    refresh(i, false)
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
    known(if cfg!(test) { 0 } else { best_match(&preferred()) }).code
}

/// The system's preferred languages, asked at most every 10 s (Settings
/// shows it every frame).
fn preferred() -> Vec<String> {
    static P: std::sync::Mutex<Option<(Vec<String>, std::time::Instant)>> = std::sync::Mutex::new(None);
    let mut p = lock(&P);
    match &*p {
        Some((v, at)) if at.elapsed() < std::time::Duration::from_secs(10) => v.clone(),
        _ => {
            let v = sys::preferred();
            *p = Some((v.clone(), std::time::Instant::now()));
            v
        }
    }
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
    thread_local!(static IN_INIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    // A `tr!` while reading the config (on this thread) is English: the
    // OnceLock below would wait for itself.
    if IN_INIT.with(|f| f.replace(true)) {
        let _ = CUR.compare_exchange(usize::MAX, 0, Ordering::SeqCst, Ordering::SeqCst);
        return;
    }
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            IN_INIT.with(|f| f.set(false));
        }
    }
    let _done = Done;
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
/// Read language `tag`'s files again now (its download just landed).
pub(crate) fn reload(tag: &str) -> bool {
    index_of(tag).is_some_and(|i| i != 0 && refresh(i, true))
}

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
    best_match(&preferred())
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

/// `s` in capitals for the current language (headings): Turkish and
/// Azerbaijani map i to İ and ı to I; scripts without case are unchanged.
pub fn upper(s: &str) -> String {
    upper_in(current(), s)
}

/// [`upper`] for language `tag`.
pub fn upper_in(tag: &str, s: &str) -> String {
    let p = tag.split('-').next().unwrap_or("");
    if p.eq_ignore_ascii_case("tr") || p.eq_ignore_ascii_case("az") {
        s.chars().map(|c| match c {
            'i' => "\u{130}".to_string(),
            'ı' => "I".to_string(),
            c => c.to_uppercase().collect(),
        }).collect()
    } else {
        s.to_uppercase()
    }
}

/// A heading's capitals are tracked letter by letter only when they are
/// plain ASCII (the baked font); any other string is drawn whole, so the
/// OS shaper sees the word (Arabic joining, Indic marks). Never in a
/// right-to-left frame: letters placed one by one there would run right
/// to left (an English fallback heading spelled backwards).
pub fn tracks(up: &str) -> bool {
    up.is_ascii() && !is_rtl()
}

/// The language the setting asks for (by code), whether its file is here
/// or not.
pub(crate) fn wanted() -> &'static str {
    let i = CUR.load(Ordering::Relaxed);
    if i == usize::MAX { "en" } else { known(i).code }
}

/// The current language's code (the one in force: English shows until
/// its file is here).
pub fn current() -> &'static str {
    lang_at(current_index()).code
}

/// Whether the current language is written right to left (the decorated
/// windows mirror, the toolbar runs right to left).
pub fn is_rtl() -> bool {
    #[cfg(test)]
    if FORCE_RTL.with(|f| f.get()) {
        return true;
    }
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

#[cfg(test)]
thread_local! {
    /// [`with_rtl`]: this thread lays out right to left in its language.
    static FORCE_RTL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with this thread laid out right to left in its own (English)
/// strings: a mirrored frame whose text and pictures can be compared
/// pixel for pixel with the left-to-right one.
#[cfg(test)]
pub fn with_rtl<R>(f: impl FnOnce() -> R) -> R {
    let old = FORCE_RTL.with(|o| o.replace(true));
    struct Back(bool);
    impl Drop for Back {
        fn drop(&mut self) {
            FORCE_RTL.with(|o| o.set(self.0));
        }
    }
    let _back = Back(old);
    f()
}

/// Whether `s` is a whole string of the current language's catalog (its
/// own or the English one a missing key falls back to; every plural
/// form): the fit tests' cut labels, help lines and placeholders.
#[cfg(test)]
pub fn is_catalog_text(s: &str) -> bool {
    thread_local!(static SETS: std::cell::RefCell<std::collections::HashMap<usize, std::collections::HashSet<&'static str>>> = Default::default());
    let lang = current_index();
    SETS.with(|m| {
        let mut m = m.borrow_mut();
        let set = m.entry(lang).or_insert_with(|| {
            let mut set = std::collections::HashSet::new();
            for i in 0..names().len() {
                let v = table_of(lang)[i];
                let v = if v.is_empty() { table_of(0)[i] } else { v };
                set.extend(v.split('\u{1f}').filter(|f| !f.is_empty()));
            }
            set
        });
        set.contains(s)
    })
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

/// The variable naming real catalogs for the fit tests: comma-separated
/// tags (`de,fi,ar`), each read from the source tree's `lang/<tag>.txt`
/// (docs/translating.md), or `all` for every file there.
#[cfg(test)]
pub const FIT_VAR: &str = "RUSTSHOT_FIT_LANGS";

/// The real languages [`FIT_VAR`] names, loaded from the source tree's
/// `lang/` once per test run (a tag this build does not list, or whose
/// file is not there, fails the test that asked).
#[cfg(test)]
pub fn fit_langs() -> Vec<&'static str> {
    static L: OnceLock<Vec<&'static str>> = OnceLock::new();
    L.get_or_init(|| {
        let list = std::env::var(FIT_VAR).unwrap_or_default();
        load_fit_langs(&list, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lang"))
    })
    .clone()
}

/// Load the languages `list` names (comma-separated tags; English and the
/// pseudo-languages skipped) from the catalogs in `dir`, for this run.
#[cfg(test)]
fn load_fit_langs(list: &str, dir: &std::path::Path) -> Vec<&'static str> {
    let found = files::discover(&[dir.to_path_buf()]);
    let all: String;
    let list = if list.trim().eq_ignore_ascii_case("all") {
        all = found.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(",");
        &all
    } else {
        list
    };
    let mut out: Vec<&'static str> = Vec::new();
    for t in list.split(',').map(str::trim).filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("en")) {
        let i = index_of(t).unwrap_or_else(|| panic!("{FIT_VAR}: this build has no language {t} (add lang/{t}.txt, then build again)"));
        let k = known(i);
        if out.contains(&k.code) || k.code.starts_with("qps") {
            continue;
        }
        let paths = found.iter().find(|(tag, _)| tag.eq_ignore_ascii_case(k.code)).map(|(_, p)| p.clone()).unwrap_or_default();
        assert!(!paths.is_empty(), "{FIT_VAR}: no {}.txt in {}", k.code, dir.display());
        let p = files::load(k.code, &paths);
        k.live.store(Box::into_raw(Box::new(p)), Ordering::Release);
        out.push(k.code);
    }
    out
}

#[cfg(test)]
mod tests;
