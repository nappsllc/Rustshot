use super::*;

#[test]
fn keys_are_sorted_and_found() {
    assert!(table::KEYS.windows(2).all(|w| w[0] < w[1]), "sorted, unique");
    for (i, k) in table::KEYS.iter().enumerate() {
        assert_eq!(find(k), Some(i), "{k}");
    }
    assert_eq!(find("no.such.key"), None);
    assert_eq!(lookup("settings.general.language").map(Key::name), Some("settings.general.language"));
}

#[test]
fn english_is_first_and_complete() {
    assert_eq!(registry()[0].code, "en");
    assert_eq!(languages()[0].code, "en");
    for (i, v) in table_of(0).iter().enumerate() {
        assert!(!v.is_empty(), "English lacks {}", table::KEYS[i]);
    }
    assert_eq!(tr!("settings.general.language"), "Language");
}

#[test]
fn placeholders_are_filled() {
    assert_eq!(fmt("Saved {name} to {dir}", &[("name", &"a.png"), ("dir", &"D:")]), "Saved a.png to D:");
    assert_eq!(fmt("{x} {missing} {", &[("x", &1)]), "1 {missing} {");
    assert_eq!(fmt("{}", &[]), "{}");
}

#[test]
fn pseudo_languages_derive_from_english() {
    let en = tr!("settings.general.language");
    let qps = with_lang("qps", || tr!("settings.general.language"));
    assert_eq!(qps, pseudo::pseudo(en, pseudo::Script::Accented));
    assert!(with_lang("qps-ar", is_rtl));
    assert!(with_lang("qps-ar", || tr!("settings.general.language")).contains('ب') || with_lang("qps-ar", || tr!("settings.general.language")).chars().any(|c| ('\u{600}'..='\u{6ff}').contains(&c)));
    assert!(with_lang("qps-rtl", is_rtl));
    assert!(!with_lang("qps", is_rtl));
    assert_eq!(tr!("settings.general.language"), en, "back to English after with_lang");
}

/// Headings' capitals: Turkish dotted and dotless i, other scripts as
/// they are; only ASCII capitals are tracked letter by letter.
#[test]
fn headings_upper_case_by_language() {
    assert_eq!(upper_in("en", "file names"), "FILE NAMES");
    assert_eq!(upper_in("tr", "dil ayarı"), "D\u{130}L AYARI");
    assert_eq!(upper_in("az-Latn", "i"), "\u{130}");
    assert_eq!(upper_in("de", "größe"), "GRÖSSE");
    assert_eq!(upper_in("ar", "عام"), "عام");
    assert!(tracks("APPEARANCE"));
    assert!(!tracks("GRÖSSE") && !tracks("عام"));
}

/// Odd files read as problems, not as junk: too large, UTF-16, not
/// UTF-8, separators in a value; lone CRs are line breaks.
#[test]
fn odd_language_files() {
    let dir = std::env::temp_dir().join(format!("rustshot-oddlang-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let w = |n: &str, b: &[u8]| {
        let p = dir.join(n);
        std::fs::write(&p, b).unwrap();
        p
    };
    let big = w("big.txt", &vec![b'#'; (files::MAX_FILE + 1) as usize]);
    assert!(files::read_text(&big).unwrap_err().contains("MB"));
    let u16 = w("u16.txt", &[0xFF, 0xFE, b'a', 0]);
    assert!(files::read_text(&u16).unwrap_err().contains("UTF-16"));
    let bad = w("bad.txt", &[b'a', 0xC3, b'(']);
    assert!(files::read_text(&bad).unwrap_err().contains("UTF-8"));
    let cr = w("cr.txt", b"@name = Zet\rsettings.general.language = Taal\rsettings.general.theme = a\x1fb\r");
    let p = files::load("zz", &[cr]);
    assert_eq!(p.name, "Zet");
    let i = lookup("settings.general.language").unwrap().0 as usize;
    assert_eq!(p.values[i], "Taal");
    let j = lookup("settings.general.theme").unwrap().0 as usize;
    assert_eq!(p.values[j], "", "a separator in a value: English");
    assert!(p.problems.iter().any(|s| s.contains("control")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn system_tags_match_languages() {
    let tags = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let codes = ["en", "de", "pt-BR", "pt-PT", "zh-Hans", "zh-Hant", "nb", "sr-Latn", "ar"];
    let m = |t: &[&str]| codes[best_in(&tags(t), &codes)];
    assert_eq!(m(&["en-US"]), "en");
    assert_eq!(m(&["C", "POSIX"]), "en");
    assert_eq!(m(&["xx-YY"]), "en", "unknown: English");
    assert_eq!(m(&["de_AT.UTF-8"]), "de");
    assert_eq!(m(&["xx", "de-CH"]), "de", "the first one known");
    assert_eq!(m(&["pt-PT"]), "pt-PT");
    assert_eq!(m(&["pt"]), "pt-BR");
    assert_eq!(m(&["zh-TW"]), "zh-Hant");
    assert_eq!(m(&["zh-CN"]), "zh-Hans");
    assert_eq!(m(&["no_NO"]), "nb");
    assert_eq!(m(&["sr-Cyrl-RS"]), "sr-Latn");
    assert_eq!(m(&["ar-EG"]), "ar");
    for c in codes {
        assert_eq!(m(&[c]), c);
    }
    // The real list: every embedded language is found by its own tag.
    for (i, k) in registry().iter().enumerate().filter(|(_, k)| !k.code.starts_with("qps")) {
        assert_eq!(best_match(&tags(&[k.code])), i, "{}", k.code);
    }
}

/// Every translation in lang/ reads without a problem: known keys, the
/// English placeholders, a name of its own (build.rs lists it).
#[test]
fn catalogs_agree_with_english() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lang");
    for (tag, files) in files::discover(&[dir]) {
        let p = files::load(&tag, &files);
        assert!(p.problems.is_empty(), "{tag}: {:#?}", p.problems);
        assert!(p.name != tag, "{tag}: @name");
        assert!(table::AVAILABLE.iter().any(|a| a.0 == tag), "{tag} is listed");
    }
}

/// A language file is read defensively: bad lines are skipped, a value
/// with the wrong placeholders falls back to English, later files win.
#[test]
fn language_files_are_read_defensively() {
    let dir = std::env::temp_dir().join(format!("rustshot-lang-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("base.txt");
    let over = dir.join("over.txt");
    std::fs::write(
        &base,
        "\u{feff}# test\n@name = Testisch\n@dir = rtl\n@plural = polish\nsettings.general.language = Sprache\nno equals sign\nno.such.key = x\n\
         settings.error.save = Speichern {path} fehlgeschlagen: {e}\nsettings.error.autostart = Fehler {bogus}\nsettings.broken = Fehler ohne e\n\
         history.count[one] = {n} Aufnahme\nhistory.count[few] = {n} Aufnahmen\nhistory.count[many] = {n} Aufnahmen!\nhistory.count[other] = {n} Aufnahme(n)\n\
         history.count = kein Plural\nsettings.general.theme[one] = plural?\nhistory.count[bogus] = x\nsettings.general.accent = \\s spaced \\\\ \\q\n",
    )
    .unwrap();
    std::fs::write(&over, "settings.general.language = Sprache!\n").unwrap();
    let p = files::load("xx", &[base, over]);
    let v = |k: &str| p.values[lookup(k).map(|k| k.0 as usize).or_else(|| find(k)).unwrap()];
    assert_eq!((p.name.as_str(), p.rtl, p.plural.as_str()), ("Testisch", true, "polish"));
    assert_eq!(v("settings.general.language"), "Sprache!", "the later file wins");
    assert_eq!(v("settings.error.save"), "Speichern {path} fehlgeschlagen: {e}");
    assert_eq!(v("settings.error.autostart"), "", "an unknown placeholder: English");
    assert_eq!(v("settings.broken"), "", "a left-out placeholder: English");
    assert_eq!(v("history.count"), "\u{1f}{n} Aufnahme\u{1f}\u{1f}{n} Aufnahmen\u{1f}{n} Aufnahmen!\u{1f}{n} Aufnahme(n)");
    assert_eq!(v("settings.general.theme"), "", "not plural in English");
    assert_eq!(v("settings.general.accent"), "  spaced \\ \\q", "escapes; an unknown one kept");
    assert_eq!(p.problems.len(), 7, "{:#?}", p.problems);
    let _ = std::fs::remove_dir_all(&dir);
    // Nothing at all: no panic, all English.
    let none = files::load("yy", &[dir.join("gone.txt")]);
    assert!(none.values.iter().all(|v| v.is_empty()) && none.problems.len() == 1);
}

#[test]
fn language_files_are_found_by_tag() {
    let root = std::env::temp_dir().join(format!("rustshot-langdirs-{}", std::process::id()));
    let (hi, lo) = (root.join("hi"), root.join("lo"));
    std::fs::create_dir_all(&hi).unwrap();
    std::fs::create_dir_all(&lo).unwrap();
    for (d, n) in [(&hi, "de.txt"), (&lo, "de.txt"), (&lo, "pt-BR.txt"), (&lo, "en.txt"), (&lo, "bad tag.txt"), (&lo, "notes.md")] {
        std::fs::write(d.join(n), "").unwrap();
    }
    let found = files::discover(&[hi.clone(), lo.clone(), root.join("missing")]);
    let tags: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(tags, ["de", "pt-BR"], "English is the template; bad names skipped");
    assert_eq!(found[0].1, [lo.join("de.txt"), hi.join("de.txt")], "lowest priority first");
    let _ = std::fs::remove_dir_all(&root);
}

/// Picking a language that is not here downloads it (a fake fetcher):
/// Downloading, then loaded and shown; a failure is kept for Retry.
#[test]
fn a_picked_language_is_downloaded() {
    use std::sync::Arc;
    struct Fake {
        dir: std::path::PathBuf,
        fail: std::sync::atomic::AtomicBool,
        asked: std::sync::Mutex<Vec<String>>,
    }
    impl fetch::Fetch for Fake {
        fn ensure(&self, asset: &str, rel: &str, done: Box<dyn FnOnce(Result<std::path::PathBuf, String>) + Send>) {
            self.asked.lock().unwrap().push(asset.to_string());
            if self.fail.swap(false, std::sync::atomic::Ordering::SeqCst) {
                return done(Err("offline".into()));
            }
            let p = self.dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "@name = Zet\nsettings.general.language = Taal\n").unwrap();
            done(Ok(p));
        }
        fn cached(&self, rel: &str) -> Option<std::path::PathBuf> {
            Some(self.dir.join(rel)).filter(|p| p.is_file())
        }
    }
    let dir = std::env::temp_dir().join(format!("rustshot-fetch-{}", std::process::id()));
    let fake = Arc::new(Fake { dir: dir.clone(), fail: true.into(), asked: Default::default() });
    fetch::set_fetcher(fake.clone());
    assert_eq!(fetch::status("zz-test"), fetch::Status::Missing);
    fetch::request("zz-test");
    assert_eq!(fetch::status("zz-test"), fetch::Status::Failed("offline".into()));
    let g = generation();
    fetch::request("zz-test");
    assert_eq!(fetch::status("zz-test"), fetch::Status::Here);
    assert!(generation() > g, "the windows lay themselves out again");
    assert_eq!(*fake.asked.lock().unwrap(), ["lang-zz-test.txt", "lang-zz-test.txt"]);
    assert_eq!(with_lang("zz-test", || tr!("settings.general.language")), "Taal");
    assert_eq!(with_lang("zz-test", || tr!("settings.general.theme")), "Theme", "missing keys: English");
    fetch::request("zz-test");
    assert_eq!(fake.asked.lock().unwrap().len(), 2, "here: not asked again");
    // A newer file (another size) is read again when it lands.
    std::fs::write(dir.join(fetch::rel("zz-test")), "@name = Zet\nsettings.general.language = Taal kiezen\n").unwrap();
    assert!(reload("zz-test"));
    assert_eq!(with_lang("zz-test", || tr!("settings.general.language")), "Taal kiezen");
    // A failure with one fetcher is asked again when another is set (the
    // first tr! may run before the real one is wired in: set_fetcher
    // retries the language in force).
    fake.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    fetch::request("zz-two");
    assert!(matches!(fetch::status("zz-two"), fetch::Status::Failed(_)));
    fetch::retry("zz-two");
    assert_eq!(fetch::status("zz-two"), fetch::Status::Here, "asked again");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The code with the test modules cut off (`#[cfg(test)] mod ...` to the
/// end of the file, as the crate writes them).
fn non_test(src: &str) -> String {
    let mut out = String::new();
    let mut lines = src.lines().peekable();
    while let Some(l) = lines.next() {
        if l.trim() == "#[cfg(test)]" {
            // Past further attributes: a module is cut to its end (the
            // file's end: the crate puts test modules last).
            let mut look = lines.clone();
            while look.peek().is_some_and(|n| n.trim_start().starts_with("#[")) {
                look.next();
            }
            if look.peek().is_some_and(|n| {
                let t = n.trim_start();
                t.starts_with("mod ") || t.starts_with("pub mod ") || t.starts_with("pub(crate) mod ")
            }) {
                break;
            }
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// Words that are the same in every language: product, format and
/// codec names, key caps.
const SAME_EVERYWHERE: &[&str] = &[
    "Rustshot", "PNG", "JPG", "BMP", "MP4", "GIF", "GDI", "ffmpeg", "OpenH264", "Imgur", "Ctrl", "Alt", "Shift", "Win", "Cmd", "Super", "Tab", "Enter",
    "Esc", "Space", "Segoe UI", "Georgia", "Consolas", "Helvetica", "Times", "Menlo", "Aa", "Auto", "px", "powernapps.net",
];

/// UI text passed straight to a drawing, control or notice call is a
/// `tr!`, never a literal: every UI code path is scanned for literals in
/// those shapes (`text(ui, 12.0, "…"`, `.cbutton("id", "…"`, `Toast::new("…"`,
/// `label: "…"`, `=> "…"` arms of `label`/`tip` functions ...).
#[test]
fn no_untranslated_ui_literals() {
    // (what comes before, arguments to skip): the literal follows.
    const SHAPES: &[(&str, usize)] = &[
        ("text(ui, ", 1),
        ("text(f, ui, ", 1),
        (".text_at(", 1),
        ("text_w(", 1),
        ("tw(ui, ", 1),
        ("tw(&ui, ", 1),
        ("draw_text(", 2),
        ("Toast::new(", 0),
        ("Note::plain(", 0),
        ("Note::new(", 0),
        ("notify(", 0),
        ("say(", 0),
        (".cbutton(", 1),
        (".cbutton_ex(", 1),
        (".clink(", 1),
        (".clink_in(", 1),
        (".checkbox(", 1),
        (".button(", 1),
        ("button(ui, ", 1),
        ("srow(ui, tok, ", 0),
        ("srow_ex(ui, tok, ", 0),
        (".sec(", 0),
        (".label(", 0),
        (".note(", 0),
        (".heading(", 0),
        (".paragraph(", 0),
        (".cbutton_width(", 0),
        (".button_width(", 0),
        ("Kind::Label(", 0),
        ("label: ", 0),
        ("tip: ", 0),
        ("title: ", 0),
        ("caption: ", 0),
        ("group(ui, ", 0),
    ];
    // The string literal at the start of `s` (after spaces), if any.
    fn literal(s: &str) -> Option<&str> {
        let s = s.trim_start();
        let body = s.strip_prefix('"')?;
        let mut esc = false;
        for (i, c) in body.char_indices() {
            match c {
                '\\' if !esc => esc = true,
                '"' if !esc => return Some(&body[..i]),
                _ => esc = false,
            }
        }
        None
    }
    // `s` after `n` top-level commas (quotes and brackets respected).
    fn skip_args(s: &str, n: usize) -> Option<&str> {
        let (mut depth, mut quote, mut esc, mut left) = (0i32, false, false, n);
        if n == 0 {
            return Some(s);
        }
        for (i, c) in s.char_indices() {
            match c {
                '\\' if quote && !esc => {
                    esc = true;
                    continue;
                }
                '"' if !esc => quote = !quote,
                '(' | '[' | '{' if !quote => depth += 1,
                ')' | ']' | '}' if !quote => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                }
                ',' if !quote && depth == 0 => {
                    left -= 1;
                    if left == 0 {
                        return Some(&s[i + 1..]);
                    }
                }
                _ => {}
            }
            esc = false;
        }
        None
    }
    let words = |s: &str| {
        let mut run = 0;
        s.chars().any(|c| {
            run = if c.is_ascii_alphabetic() { run + 1 } else { 0 };
            run >= 2
        })
    };
    let label_fns = ["fn label(", "fn fixed_label(", "fn tip(", "fn title("];
    let mut bad = Vec::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                if name != "tests" {
                    stack.push(p);
                }
                continue;
            }
            if !name.ends_with(".rs") || name.contains("test") {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            let mut in_label_fn = false;
            let code = non_test(&src);
            for (i, line) in code.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") {
                    continue;
                }
                if t.contains("fn ") && (t.starts_with("fn ") || t.starts_with("pub") || t.starts_with("const fn ")) {
                    in_label_fn = label_fns.iter().any(|f| t.contains(f));
                }
                let mut found: Vec<&str> = Vec::new();
                for &(needle, skip) in SHAPES {
                    for (at, _) in line.match_indices(needle) {
                        // A whole word: `text(` is not `context(`.
                        let before = line[..at].chars().next_back();
                        if !needle.starts_with(['.', ' ']) && before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        if let Some(l) = skip_args(&line[at + needle.len()..], skip).and_then(literal) {
                            found.push(l);
                        }
                        // A row's help: `srow(ui, tok, label, Some("…")`.
                        if needle.starts_with("srow")
                            && let Some(help) = skip_args(&line[at + needle.len()..], 1).and_then(|r| r.trim_start().strip_prefix("Some("))
                        {
                            found.extend(literal(help));
                        }
                    }
                }
                if in_label_fn {
                    for (at, _) in line.match_indices("=> ") {
                        found.extend(literal(&line[at + 3..]));
                    }
                }
                for f in found {
                    let rest: String = SAME_EVERYWHERE.iter().fold(f.to_string(), |s, w| s.replace(w, ""));
                    if words(&rest) {
                        bad.push(format!("{}:{}: \"{f}\"", p.strip_prefix(&root).unwrap().display(), i + 1));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} UI literals not through tr!:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn plurals_pick_forms() {
    let one = trn!("history.count", 1);
    let many = trn!("history.count", 5);
    assert_eq!(one, "1 capture");
    assert_eq!(many, "5 captures");
    let p = with_lang("qps", || trn!("history.count", 1));
    assert!(p.contains('1') && p.starts_with('['), "{p}");
}
