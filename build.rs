// Bakes the UI font (Inter subset) into static tables, and embeds the app
// icon (resource ID 1) and version info into rustshot.exe. The icon is
// Windows MSVC targets only and never fails the build (a missing rc.exe
// just means no icon). The embedded data (font outlines, the segmentation
// model) is DEFLATE-compressed here with the crate's own compressor and
// inflated on first use.
#[path = "src/deflate.rs"]
mod deflate;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/deflate.rs");
    // The camera blur's segmentation model is embedded when it is there
    // (`assets/selfie.bin`, converted by tools/convert-selfie); the
    // `selfie_model` feature insists on it, `RUSTSHOT_NO_SELFIE=1` leaves
    // it out.
    println!("cargo:rustc-check-cfg=cfg(selfie_embedded)");
    println!("cargo:rerun-if-changed=assets/selfie.bin");
    println!("cargo:rerun-if-env-changed=RUSTSHOT_NO_SELFIE");
    let have = std::path::Path::new("assets/selfie.bin").is_file();
    let off = std::env::var_os("RUSTSHOT_NO_SELFIE").is_some_and(|v| v == "1");
    if std::env::var_os("CARGO_FEATURE_SELFIE_MODEL").is_some() && !have {
        panic!("the selfie_model feature needs assets/selfie.bin (see tools/convert-selfie)");
    }
    if have && !off {
        println!("cargo:rustc-cfg=selfie_embedded");
        compress_model();
    }
    println!("cargo:rerun-if-changed=packaging/icons/rustshot.ico");
    println!("cargo:rerun-if-env-changed=RC");
    bake_font();
    pack_i18n();
    // Check the *target*, not the host: cross-target checks run build.rs on the host.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    if let Err(e) = embed() {
        println!("cargo:warning=rustshot: not embedding icon/version info: {e}");
    }
}

fn embed() -> Result<(), String> {
    let rc = find_rc().ok_or("rc.exe not found (Windows SDK)")?;
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let ico = manifest.join("packaging").join("icons").join("rustshot.ico");
    if !ico.exists() {
        return Err(format!("{} missing", ico.display()));
    }
    let ico = ico.to_string_lossy().replace('\\', "\\\\");
    let ver = env::var("CARGO_PKG_VERSION").map_err(|e| e.to_string())?;
    let num = |k: &str| env::var(k).unwrap_or_else(|_| "0".into());
    let (maj, min, pat) = (
        num("CARGO_PKG_VERSION_MAJOR"),
        num("CARGO_PKG_VERSION_MINOR"),
        num("CARGO_PKG_VERSION_PATCH"),
    );
    let rc_src = format!(
        r#"#pragma code_page(65001)
1 ICON "{ico}"

1 VERSIONINFO
FILEVERSION {maj},{min},{pat},0
PRODUCTVERSION {maj},{min},{pat},0
FILEOS 0x4
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "PowerNapps"
      VALUE "FileDescription", "Rustshot"
      VALUE "FileVersion", "{ver}"
      VALUE "InternalName", "rustshot"
      VALUE "LegalCopyright", "© 2026 PowerNapps"
      VALUE "OriginalFilename", "rustshot.exe"
      VALUE "ProductName", "Rustshot"
      VALUE "ProductVersion", "{ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let rc_path = out_dir.join("rustshot.rc");
    let res_path = out_dir.join("rustshot.res");
    fs::write(&rc_path, rc_src).map_err(|e| e.to_string())?;
    let out = Command::new(&rc)
        .args(["/nologo", "/fo"])
        .arg(&res_path)
        .arg(&rc_path)
        .output()
        .map_err(|e| format!("running {}: {e}", rc.display()))?;
    if !out.status.success() {
        return Err(format!(
            "rc.exe failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    println!("cargo:rustc-link-arg-bins={}", res_path.display());
    Ok(())
}

fn find_rc() -> Option<PathBuf> {
    if let Some(p) = env::var_os("RC") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let pf = env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let bin = pf.join("Windows Kits").join("10").join("bin");
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for e in fs::read_dir(&bin).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("10.") {
            continue;
        }
        let rc = e.path().join("x64").join("rc.exe");
        if !Path::new(&rc).exists() {
            continue;
        }
        let key: Vec<u32> = name.split('.').filter_map(|p| p.parse().ok()).collect();
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, rc));
        }
    }
    best.map(|(_, p)| p)
}

/// Bake the embedded Inter subset into `$OUT_DIR/font_baked.rs`: metrics,
/// cmap, advances, bounding boxes, kerning pairs and outlines (lines and
/// quadratics in half font units, exactly the curves ab_glyph produces), so
/// the UI font needs no font parser at run time. `OPS` packs 2-bit segment
/// codes four per byte (0 move, 1 line, 2 quad); `PTS` holds the x, y
/// pairs they consume (1, 1 and 2 points).
fn bake_font() {
    use ab_glyph::{Font, FontRef, GlyphId, OutlineCurve, Point};
    use std::fmt::Write as _;
    const SRC: &str = "assets/fonts/Inter-Medium.subset.ttf";
    println!("cargo:rerun-if-changed={SRC}");
    let bytes = fs::read(SRC).expect("read the Inter subset");
    let font = FontRef::try_from_slice(&bytes).expect("parse the Inter subset");

    let mut cmap: Vec<(char, GlyphId)> = font
        .codepoint_ids()
        .filter(|(g, _)| g.0 != 0)
        .map(|(g, c)| (c, g))
        .collect();
    cmap.sort_by_key(|&(c, _)| c);
    cmap.dedup_by_key(|&mut (c, _)| c);
    // Slots: .notdef first (what unmapped characters draw), then each
    // mapped glyph once.
    let mut gids: Vec<GlyphId> = vec![GlyphId(0)];
    for &(c, g) in &cmap {
        assert_eq!(font.glyph_id(c), g, "cmap mismatch for {c:?}");
        if !gids.contains(&g) {
            gids.push(g);
        }
    }
    assert!(gids.len() <= 256, "more than 256 glyphs: widen the slot type");
    let slot = |g: GlyphId| gids.iter().position(|&x| x == g).unwrap();
    let int = |v: f32| -> i16 {
        assert!(v.fract() == 0.0 && v.abs() < 32767.0, "value {v} is not an i16");
        v as i16
    };
    // Half units: implied on-curve points are midpoints of integer points.
    let half = |p: Point, pts: &mut Vec<i16>| {
        pts.push(int(p.x * 2.0));
        pts.push(int(p.y * 2.0));
    };

    let mut glyphs = String::new();
    let mut ops: Vec<u8> = Vec::new();
    let mut pts: Vec<i16> = Vec::new();
    for &g in &gids {
        let adv = font.h_advance_unscaled(g);
        assert!(adv.fract() == 0.0 && (0.0..65535.0).contains(&adv), "advance {adv}");
        let (op0, pt0) = (ops.len(), pts.len());
        let bbox = match font.outline(g) {
            Some(o) => {
                let mut cur: Option<Point> = None;
                for c in &o.curves {
                    let (p0, end) = match *c {
                        OutlineCurve::Line(a, b) | OutlineCurve::Quad(a, _, b) => (a, b),
                        OutlineCurve::Cubic(..) => panic!("cubic curve in a TrueType font"),
                    };
                    if cur != Some(p0) {
                        ops.push(0);
                        half(p0, &mut pts);
                    }
                    match *c {
                        OutlineCurve::Line(_, b) => {
                            ops.push(1);
                            half(b, &mut pts);
                        }
                        OutlineCurve::Quad(_, ctl, b) => {
                            ops.push(2);
                            half(ctl, &mut pts);
                            half(b, &mut pts);
                        }
                        OutlineCurve::Cubic(..) => unreachable!(),
                    }
                    cur = Some(end);
                }
                // ab_glyph's bounds are (x_min, y_max)..(x_max, y_min).
                let b = o.bounds;
                [int(b.min.x), int(b.min.y), int(b.max.x), int(b.max.y)]
            }
            None => [0; 4],
        };
        writeln!(
            glyphs,
            "    G {{ adv: {adv}, bbox: {bbox:?}, op: {op0}, n_ops: {}, pt: {} }},",
            ops.len() - op0,
            pt0 / 2
        )
        .unwrap();
    }
    assert!(ops.len() < 65536 && pts.len() / 2 < 65536);

    let mut kern = Vec::new();
    for (i, &a) in gids.iter().enumerate() {
        for (j, &b) in gids.iter().enumerate() {
            let k = font.kern_unscaled(a, b);
            if k != 0.0 {
                kern.push((i, j, int(k)));
            }
        }
    }

    let mut out = String::new();
    let w = &mut out;
    writeln!(w, "// @generated by build.rs from {SRC}; do not edit.").unwrap();
    writeln!(w, "pub const UNITS_PER_EM: u16 = {};", font.units_per_em().unwrap() as u16).unwrap();
    writeln!(w, "pub const ASCENT: i16 = {};", int(font.ascent_unscaled())).unwrap();
    writeln!(w, "pub const DESCENT: i16 = {};", int(font.descent_unscaled())).unwrap();
    writeln!(w, "pub const LINE_GAP: i16 = {};", int(font.line_gap_unscaled())).unwrap();
    writeln!(w, "/// (codepoint, glyph slot), sorted by codepoint.").unwrap();
    writeln!(w, "pub static CMAP: [(char, u8); {}] = [", cmap.len()).unwrap();
    for &(c, g) in &cmap {
        writeln!(w, "    ({c:?}, {}),", slot(g)).unwrap();
    }
    writeln!(w, "];\npub static GLYPHS: [G; {}] = [\n{glyphs}];", gids.len()).unwrap();
    writeln!(w, "/// (left slot, right slot, adjustment), sorted.").unwrap();
    writeln!(w, "pub static KERN: [(u8, u8, i16); {}] = {kern:?};", kern.len()).unwrap();
    let packed: Vec<u8> = ops
        .chunks(4)
        .map(|c| c.iter().enumerate().fold(0u8, |acc, (i, &o)| acc | (o << (2 * i))))
        .collect();
    // OPS and PTS go in DEFLATE-compressed (fonts.rs inflates them once):
    // the packed ops, then the points as zigzag deltas from the previous
    // coordinate of the same axis, low bytes then high bytes.
    let mut blob = packed.clone();
    let (mut lo, mut hi) = (Vec::new(), Vec::new());
    let mut last = [0i16; 2];
    for (i, &v) in pts.iter().enumerate() {
        let d = v.wrapping_sub(last[i % 2]);
        last[i % 2] = v;
        let z = ((d << 1) ^ (d >> 15)) as u16;
        lo.push(z as u8);
        hi.push((z >> 8) as u8);
    }
    blob.extend(lo);
    blob.extend(hi);
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let z = deflate::deflate(&blob, deflate::Effort::BEST);
    fs::write(out_dir.join("font_outlines.z"), &z).expect("write font_outlines.z");
    writeln!(w, "pub const N_OPS_BYTES: usize = {};", packed.len()).unwrap();
    writeln!(w, "pub const N_PTS: usize = {};", pts.len()).unwrap();
    writeln!(w, "/// The packed ops, then the point coordinates (see build.rs).").unwrap();
    writeln!(w, r#"pub static OUTLINES_Z: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/font_outlines.z"));"#).unwrap();
    fs::write(out_dir.join("font_baked.rs"), out).expect("write font_baked.rs");
}

/// `assets/selfie.bin`, DEFLATE-compressed into `$OUT_DIR/selfie.bin.z`
/// for `cam::seg`, as two planes: the bytes at even offsets, then at odd
/// ones (the f16 weights' low bytes, nearly random, apart from their high
/// bytes, sign and exponent, which compress).
fn compress_model() {
    let raw = fs::read("assets/selfie.bin").expect("read assets/selfie.bin");
    let mut planes: Vec<u8> = raw.iter().step_by(2).copied().collect();
    planes.extend(raw.iter().skip(1).step_by(2));
    let z = deflate::deflate(&planes, deflate::Effort::BEST);
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("selfie.bin.z"), &z).expect("write selfie.bin.z");
    fs::write(out_dir.join("selfie_len.rs"), format!("{}", raw.len())).expect("write selfie_len.rs");
}

/// The plural categories (CLDR), in the order a plural entry stores them.
const PLURAL_FORMS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// One catalog file: its `@` metadata and `key = value` entries (plural
/// forms as `key[one]`: (key, form, value, line)), in file order.
struct Catalog {
    code: String,
    meta: std::collections::BTreeMap<String, String>,
    entries: Vec<(String, Option<usize>, String, usize)>,
}

/// `\n`, `\t`, `\\` and `\s` (a space, for an edge the trim would eat).
fn unescape(v: &str, at: &str) -> String {
    let mut out = String::new();
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
            other => panic!("{at}: unknown escape \\{}", other.map(String::from).unwrap_or_default()),
        }
    }
    out
}

fn read_catalog(path: &Path) -> Catalog {
    let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
    let code = stem.split('.').next().unwrap_or_default().to_string();
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut cat = Catalog { code, meta: Default::default(), entries: Vec::new() };
    let mut seen = std::collections::HashSet::new();
    for (n, line) in text.lines().enumerate() {
        let at = format!("{}:{}", path.display(), n + 1);
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').unwrap_or_else(|| panic!("{at}: expected `key = value`"));
        let (k, v) = (k.trim(), unescape(v.trim(), &at));
        // NUL and 0x1F separate the packed table's values and plural forms.
        if let Some(c) = v.chars().find(|&c| c.is_control() && c != '\n' && c != '\t') {
            panic!("{at}: control character U+{:04X} in a value", c as u32);
        }
        if let Some(m) = k.strip_prefix('@') {
            cat.meta.insert(m.to_string(), v);
            continue;
        }
        let (base, form) = match k.split_once('[') {
            Some((b, f)) => {
                let f = f.strip_suffix(']').unwrap_or_else(|| panic!("{at}: unclosed ["));
                let i = PLURAL_FORMS.iter().position(|&p| p == f).unwrap_or_else(|| panic!("{at}: unknown plural form {f}"));
                (b, Some(i))
            }
            None => (k, None),
        };
        assert!(
            !base.is_empty() && base.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)),
            "{at}: bad key {base:?} (a-z 0-9 . _ -)"
        );
        assert!(seen.insert(k.to_string()), "{at}: duplicate key {k}");
        cat.entries.push((base.to_string(), form, v, n + 1));
    }
    cat
}

/// The `{name}` placeholders of `s`, sorted.
fn placeholders(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        rest = &rest[i + 1..];
        if let Some(j) = rest.find('}') {
            let name = &rest[..j];
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                v.push(name.to_string());
                rest = &rest[j + 1..];
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

/// The languages of `lang/<tag>.txt` other than English: (tag, its own
/// name, right to left), read leniently from each file's `@name` / `@dir`
/// (a translation file never breaks the build; the app checks it when it
/// loads it).
fn available_languages(all: &[PathBuf]) -> Vec<(String, String, bool)> {
    let mut v = Vec::new();
    for p in all {
        let Some(tag) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        let ok = !tag.is_empty() && tag.len() <= 16 && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok || tag.split('.').next() == Some("en") {
            continue;
        }
        let text = String::from_utf8_lossy(&fs::read(p).unwrap_or_default()).into_owned();
        let meta = |m: &str| {
            text.lines().find_map(|l| {
                let (k, v) = l.trim_start_matches('\u{feff}').split_once('=')?;
                (k.trim() == m).then(|| v.trim().to_string())
            })
        };
        let name = meta("@name").filter(|n| !n.is_empty()).unwrap_or_else(|| tag.to_string());
        let rtl = meta("@dir").is_some_and(|d| d.eq_ignore_ascii_case("rtl"));
        v.push((tag.to_string(), name, rtl));
    }
    v.sort_by_key(|a| a.0.to_ascii_lowercase());
    v
}

/// Pack the English catalog (`lang/en.txt`, and `en.<part>.txt` beside it)
/// into `$OUT_DIR/i18n_table.rs` (the sorted keys, their placeholders,
/// which are plural; the other languages' tags and names) and
/// `$OUT_DIR/i18n.bin` (the values in key order, NUL-terminated, a plural
/// entry's six CLDR forms joined by 0x1F, DEFLATE-compressed). Other
/// languages are not embedded: their files are release assets the app
/// downloads when one is picked (`src/i18n/files.rs`).
fn pack_i18n() {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    println!("cargo:rerun-if-changed=lang");
    let mut all: Vec<PathBuf> = fs::read_dir("lang")
        .expect("read lang/")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .collect();
    all.sort();
    let others = available_languages(&all);
    let mut files: Vec<PathBuf> =
        all.iter().filter(|p| p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.split('.').next()) == Some("en")).cloned().collect();
    files.sort();
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    // `<lang>.<part>.txt` is more of `<lang>.txt`.
    let mut cats: Vec<Catalog> = Vec::new();
    for c in files.iter().map(|p| read_catalog(p)) {
        match cats.iter_mut().find(|o| o.code == c.code) {
            Some(o) => {
                for e in c.entries {
                    let dup = o.entries.iter().any(|x| x.0 == e.0 && x.1 == e.1);
                    assert!(!dup, "lang/{}*.txt: duplicate key {}", c.code, e.0);
                    o.entries.push(e);
                }
                o.meta.extend(c.meta);
            }
            None => cats.push(c),
        }
    }
    // Only English is embedded (the source and the fallback); every other
    // language is a file the app loads at run time (`src/i18n/files.rs`).
    let en_i = cats.iter().position(|c| c.code == "en").expect("lang/en.txt");
    let en = cats.remove(en_i);
    cats = vec![en];
    // Each English key's forms (a plain entry in slot 5, "other").
    let mut keys: BTreeMap<String, [Option<String>; 6]> = BTreeMap::new();
    let mut plural: BTreeMap<String, bool> = BTreeMap::new();
    for (k, form, v, line) in &cats[0].entries {
        if let Some(&p) = plural.get(k) {
            assert_eq!(p, form.is_some(), "lang/en.txt:{line}: {k} is both plain and plural");
        }
        plural.insert(k.clone(), form.is_some());
        assert!(!v.is_empty(), "lang/en.txt:{line}: empty value for {k}");
        keys.entry(k.clone()).or_default()[form.unwrap_or(5)] = Some(v.clone());
    }
    for (k, f) in &keys {
        if plural[k] {
            assert!(f[1].is_some() && f[5].is_some(), "lang/en.txt: plural {k} needs [one] and [other]");
        }
    }
    let names: Vec<&String> = keys.keys().collect();
    assert!(names.len() < 65536, "too many i18n keys");
    let ph: Vec<Vec<String>> = keys
        .values()
        .map(|f| {
            let mut all: Vec<String> = f.iter().flatten().flat_map(|v| placeholders(v)).collect();
            all.sort();
            all.dedup();
            all
        })
        .collect();

    let mut blob: Vec<u8> = Vec::new();
    let mut langs = String::new();
    for cat in &cats {
        let mut vals: BTreeMap<&str, [Option<&str>; 6]> = BTreeMap::new();
        for (k, form, v, line) in &cat.entries {
            let Some(&is_plural) = plural.get(k) else {
                println!("cargo:warning=i18n/{}.txt:{line}: {k} is not an English key", cat.code);
                continue;
            };
            if is_plural != form.is_some() {
                println!("cargo:warning=i18n/{}.txt:{line}: {k} is plural in one file only", cat.code);
                continue;
            }
            vals.entry(k).or_default()[form.unwrap_or(5)] = Some(v);
        }
        let mut raw: Vec<u8> = Vec::new();
        for k in keys.keys() {
            if let Some(f) = vals.get(k.as_str()) {
                if plural[k] {
                    let forms: Vec<&str> = f.iter().map(|v| v.unwrap_or("")).collect();
                    raw.extend(forms.join("\u{1f}").bytes());
                } else {
                    raw.extend(f[5].unwrap_or("").bytes());
                }
            }
            raw.push(0);
        }
        // Then the key names (the run time reads files and dynamic keys
        // by name; compressed here they cost a few KB, not an array).
        for k in keys.keys() {
            raw.extend(k.bytes());
            raw.push(0);
        }
        let z = deflate::deflate(&raw, deflate::Effort::BEST);
        writeln!(langs, "pub const EN_RAW_LEN: usize = {};", raw.len()).unwrap();
        blob.extend(z);
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("i18n.bin"), &blob).expect("write i18n.bin");
    let mut w = String::new();
    writeln!(w, "// @generated by build.rs from lang/*.txt; do not edit.").unwrap();
    writeln!(w, "/// The other languages of lang/: (tag, name in itself, right to left).").unwrap();
    writeln!(w, "pub static AVAILABLE: [(&str, &str, bool); {}] = {others:?};", others.len()).unwrap();
    writeln!(w, "pub const N: usize = {};", names.len()).unwrap();
    writeln!(w, "/// The English keys, sorted (byte order).").unwrap();
    writeln!(w, "pub const KEYS: [&str; N] = {names:?};").unwrap();
    writeln!(w, "/// Each key's `{{placeholders}}` (compile-time checks only).").unwrap();
    let ph: Vec<String> = ph.iter().map(|p| format!("&{p:?}")).collect();
    writeln!(w, "pub const PH: [&[&str]; N] = [{}];", ph.join(", ")).unwrap();
    let pl: Vec<bool> = keys.keys().map(|k| plural[k]).collect();
    writeln!(w, "/// Whether each key is a plural entry.").unwrap();
    writeln!(w, "pub const PLURAL: [bool; N] = {pl:?};").unwrap();
    w.push_str(&langs);
    writeln!(w, "/// English, DEFLATE-compressed: each key's value in key order, NUL-terminated (plural forms joined by 0x1F).").unwrap();
    writeln!(w, r#"pub static EN_Z: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/i18n.bin"));"#).unwrap();
    fs::write(out_dir.join("i18n_table.rs"), w).expect("write i18n_table.rs");
}
