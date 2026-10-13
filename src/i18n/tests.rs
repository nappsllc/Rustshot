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
    assert_eq!(table::LANGS[0].code, "en");
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
    assert_eq!(qps, pseudo::pseudo(en, false));
    assert!(with_lang("qps-rtl", is_rtl));
    assert!(!with_lang("qps", is_rtl));
    assert_eq!(tr!("settings.general.language"), en, "back to English after with_lang");
}

#[test]
fn system_tags_match_languages() {
    let tags = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(best_match(&tags(&["en-US"])), 0);
    assert_eq!(best_match(&tags(&["C", "POSIX"])), 0);
    assert_eq!(best_match(&tags(&["xx-YY"])), 0, "unknown: English");
    for (i, d) in table::LANGS.iter().enumerate() {
        assert_eq!(best_match(&tags(&[d.code])), i, "{}", d.code);
        assert_eq!(best_match(&tags(&[&format!("{}.UTF-8", d.code.replace('-', "_"))])), i, "{}", d.code);
    }
}

#[test]
fn catalogs_agree_with_english() {
    // Every language's keys exist in English, with the same placeholders
    // (each plural form a subset of English's), and real names.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n");
    let en: std::collections::HashMap<&str, usize> = table::KEYS.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    for d in &table::LANGS {
        assert!(!d.name.is_empty() && d.name != d.code, "{}: @name", d.code);
        let mut text = String::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".txt") && name.split('.').next() == Some(d.code) {
                text += &std::fs::read_to_string(e.path()).unwrap();
                text.push('\n');
            }
        }
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('@')) {
            let (k, v) = line.split_once('=').unwrap();
            let base = k.trim().split('[').next().unwrap();
            let i = *en.get(base).unwrap_or_else(|| panic!("{}: {base} is not an English key", d.code));
            let ph = |s: &str| {
                let mut v: Vec<String> = Vec::new();
                let mut r = s;
                while let Some(a) = r.find('{') {
                    r = &r[a + 1..];
                    if let Some(b) = r.find('}') {
                        v.push(r[..b].to_string());
                        r = &r[b + 1..];
                    }
                }
                v
            };
            for p in ph(v) {
                assert!(table::PH[i].contains(&p.as_str()), "{}: {base} has {{{p}}}, English does not", d.code);
            }
            if !table::PLURAL[i] && d.code != "en" {
                let mut want: Vec<&str> = table::PH[i].to_vec();
                let mut got = ph(v);
                want.sort();
                got.sort();
                got.dedup();
                assert_eq!(got, want, "{}: {base} placeholders", d.code);
            }
        }
    }
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
    let one = trn!("test.files", 1);
    let many = trn!("test.files", 5);
    assert_eq!(one, "1 file");
    assert_eq!(many, "5 files");
    let p = with_lang("qps", || trn!("test.files", 1));
    assert!(p.contains('1') && p.starts_with('['), "{p}");
}
