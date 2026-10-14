//! Translations as files: `lang/<tag>.txt` in the data folders
//! ([`crate::datadir::data_dirs`]), read at run time, never embedded. The
//! format is i18n/en.txt's (the template): UTF-8, `key = value` lines,
//! `key[one]` / `key[other]`... for plural forms, `@name` / `@dir` /
//! `@plural` metadata, `#` comments, `\n` `\t` `\s` `\\` escapes.
//!
//! Parsing never fails: a bad line is skipped (and logged), and so is a
//! value whose `{placeholders}` do not match English's (it falls back to
//! English): a placeholder English lacks, or, for a plain string, one it
//! has left out (a plural form may leave out `{n}`: "one file").

use super::table;
use std::path::{Path, PathBuf};

/// A language as read: its metadata and its values in key order (empty:
/// English).
pub struct Parsed {
    pub name: String,
    pub rtl: bool,
    pub plural: String,
    pub values: Vec<&'static str>,
    /// What was skipped, as `file:line: why` (tests; the log has them).
    pub problems: Vec<String>,
}

const FORMS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// A tag that can name a language file (`de`, `pt-BR`, `zh-Hant`).
#[cfg(test)]
pub fn valid_tag(t: &str) -> bool {
    !t.is_empty() && t.len() <= 16 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !t.starts_with('-')
}

/// The language files in `dirs` (highest priority first): tag → its
/// files, lowest priority first (later ones override earlier entries).
/// English files are the template, not a language. (Tests: lang/'s
/// files; the app looks a picked language's file up by its tag.)
#[cfg(test)]
pub fn discover(dirs: &[PathBuf]) -> Vec<(String, Vec<PathBuf>)> {
    let mut out: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for d in dirs.iter().rev() {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
        files.sort();
        for p in files {
            let Some(tag) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            if !valid_tag(&tag) || tag.eq_ignore_ascii_case("en") {
                continue;
            }
            match out.iter_mut().find(|(t, _)| t.eq_ignore_ascii_case(&tag)) {
                Some((_, v)) => v.push(p),
                None => out.push((tag, vec![p])),
            }
        }
    }
    out.sort_by_key(|a| a.0.to_ascii_lowercase());
    out
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('s') => out.push(' '),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The `{name}` placeholders of `s`.
fn placeholders(s: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        rest = &rest[i + 1..];
        if let Some(j) = rest.find('}') {
            let name = &rest[..j];
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                v.push(name);
                rest = &rest[j + 1..];
            }
        }
    }
    v
}

/// Read the files of one language (lowest priority first) into its table.
/// A language file larger than this is not read (a real one is ~100 KB).
pub const MAX_FILE: u64 = 2 << 20;
/// Problems kept per language (the rest counted).
const MAX_PROBLEMS: usize = 20;

/// A language file's text: UTF-8 (a BOM is fine), at most [`MAX_FILE`];
/// UTF-16 (Notepad's "Unicode") and other encodings are refused, lone CRs
/// are line breaks.
pub fn read_text(f: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(f).map_err(|e| e.to_string())?;
    let mut b = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut b).map_err(|e| e.to_string())?;
    if b.len() as u64 > MAX_FILE {
        return Err(format!("larger than {} MB: not read", MAX_FILE >> 20));
    }
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        return Err("UTF-16: save it as UTF-8".into());
    }
    let s = String::from_utf8(b).map_err(|e| format!("not UTF-8 (byte {})", e.utf8_error().valid_up_to()))?;
    Ok(if s.contains('\r') { s.replace("\r\n", "\n").replace('\r', "\n") } else { s })
}

pub fn load(tag: &str, files: &[PathBuf]) -> Parsed {
    let mut forms: Vec<[Option<String>; 6]> = vec![Default::default(); table::N];
    let mut p = Parsed { name: tag.to_string(), rtl: false, plural: "one_other".into(), values: Vec::new(), problems: Vec::new() };
    for f in files {
        match read_text(f) {
            Ok(t) => parse_into(&t, &f.display().to_string(), &mut forms, &mut p),
            Err(e) => p.problems.push(format!("{}: {e}", f.display())),
        }
    }
    if p.problems.len() > MAX_PROBLEMS {
        let more = p.problems.len() - MAX_PROBLEMS;
        p.problems.truncate(MAX_PROBLEMS);
        p.problems.push(format!("and {more} more"));
    }
    for w in &p.problems {
        eprintln!("rustshot: language file {w}");
    }
    // One leaked buffer per language: values in key order, plural forms
    // joined by 0x1F (as English's table).
    let mut all = String::new();
    let mut spans = Vec::with_capacity(table::N);
    for (i, f) in forms.iter().enumerate() {
        let a = all.len();
        if table::PLURAL[i] {
            if f.iter().any(Option::is_some) {
                all.push_str(&f.iter().map(|v| v.as_deref().unwrap_or("")).collect::<Vec<_>>().join("\u{1f}"));
            }
        } else if let Some(v) = &f[5] {
            all.push_str(v);
        }
        spans.push((a, all.len()));
    }
    let s: &'static str = Box::leak(all.into_boxed_str());
    p.values = spans.into_iter().map(|(a, b)| &s[a..b]).collect();
    p
}

/// One file's lines into `forms` (later files win); metadata into `p`.
pub fn parse_into(text: &str, file: &str, forms: &mut [[Option<String>; 6]], p: &mut Parsed) {
    for (n, line) in text.lines().enumerate() {
        let at = || format!("{file}:{}", n + 1);
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            p.problems.push(format!("{}: not `key = value`", at()));
            continue;
        };
        let (k, v) = (k.trim(), unescape(v.trim()));
        // NUL and 0x1F separate the table's values and plural forms.
        if v.contains(['\0', '\u{1f}']) {
            p.problems.push(format!("{}: a control character in {k}", at()));
            continue;
        }
        if let Some(m) = k.strip_prefix('@') {
            match m {
                "name" if !v.is_empty() => p.name = v,
                "dir" => p.rtl = v.eq_ignore_ascii_case("rtl"),
                "plural" => p.plural = v,
                _ => {}
            }
            continue;
        }
        let (base, form) = match k.split_once('[') {
            Some((b, f)) => match f.strip_suffix(']').and_then(|f| FORMS.iter().position(|x| *x == f)) {
                Some(i) => (b.trim(), Some(i)),
                None => {
                    p.problems.push(format!("{}: unknown plural form in {k}", at()));
                    continue;
                }
            },
            None => (k, None),
        };
        let Ok(i) = super::names().binary_search(&base) else {
            p.problems.push(format!("{}: {base} is not a key of this version", at()));
            continue;
        };
        if table::PLURAL[i] != form.is_some() {
            p.problems.push(format!("{}: {base} is {} in English", at(), if table::PLURAL[i] { "plural" } else { "not plural" }));
            continue;
        }
        if v.is_empty() {
            continue; // English stands
        }
        let en = super::placeholders_of(i);
        let ph = placeholders(&v);
        if let Some(bad) = ph.iter().find(|x| !en.contains(x)) {
            p.problems.push(format!("{}: {base} has {{{bad}}}, which English has not: English is used", at()));
            continue;
        }
        if form.is_none()
            && let Some(miss) = en.iter().find(|x| !ph.contains(x))
        {
            p.problems.push(format!("{}: {base} leaves out {{{miss}}}: English is used", at()));
            continue;
        }
        forms[i][form.unwrap_or(5)] = Some(v);
    }
}

/// The language folders: `lang/` in each data folder, highest priority first.
pub fn lang_dirs() -> Vec<PathBuf> {
    crate::datadir::data_dirs().into_iter().map(|d: PathBuf| Path::new(&d).join("lang")).collect()
}

/// The user's own language folder (`RUSTSHOT_DATA_DIR`'s `lang/`), if set:
/// its files are taken as they are, over the downloaded copy.
pub fn override_lang_dir() -> Option<PathBuf> {
    crate::datadir::override_dir().map(|d| d.join("lang"))
}
