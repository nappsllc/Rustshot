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

#[test]
fn plurals_pick_forms() {
    let one = trn!("test.files", 1);
    let many = trn!("test.files", 5);
    assert_eq!(one, "1 file");
    assert_eq!(many, "5 files");
    let p = with_lang("qps", || trn!("test.files", 1));
    assert!(p.contains('1') && p.starts_with('['), "{p}");
}
