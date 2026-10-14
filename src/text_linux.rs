//! Linux annotation text: the first system font that parses (Inter, which
//! is Latin only, when none does), rasterised with ab_glyph. Serif and
//! mono, bold and italic are other files (DejaVu, Liberation, Noto,
//! FreeFont, else what `fc-match` names); a style without its own file is
//! synthesized from the regular face: bold by a second pass at an offset
//! (the stroke grown), italic by shearing the outline 12 degrees.

use crate::objects::Pt;
use crate::raster::Coverage;
use crate::text::Family;
use ab_glyph::{Font, FontArc, OutlineCurve, PxScale, ScaleFont};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.subset.ttf");

const DIRS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/",
    "/usr/share/fonts/TTF/",
    "/usr/share/fonts/dejavu/",
    "/usr/share/fonts/noto/",
    "/usr/share/fonts/truetype/noto/",
    "/usr/share/fonts/truetype/liberation/",
    "/usr/share/fonts/liberation/",
    "/usr/share/fonts/truetype/freefont/",
    "/usr/share/fonts/gnu-free/",
];

/// File names of a family's regular, bold, italic and bold italic faces,
/// best first.
fn names(f: Family) -> [&'static [&'static str]; 4] {
    match f {
        Family::Sans => [
            &["DejaVuSans.ttf", "NotoSans-Regular.ttf", "LiberationSans-Regular.ttf", "FreeSans.ttf"],
            &["DejaVuSans-Bold.ttf", "NotoSans-Bold.ttf", "LiberationSans-Bold.ttf", "FreeSansBold.ttf"],
            &["DejaVuSans-Oblique.ttf", "NotoSans-Italic.ttf", "LiberationSans-Italic.ttf", "FreeSansOblique.ttf"],
            &["DejaVuSans-BoldOblique.ttf", "NotoSans-BoldItalic.ttf", "LiberationSans-BoldItalic.ttf", "FreeSansBoldOblique.ttf"],
        ],
        Family::Serif => [
            &["DejaVuSerif.ttf", "NotoSerif-Regular.ttf", "LiberationSerif-Regular.ttf", "FreeSerif.ttf"],
            &["DejaVuSerif-Bold.ttf", "NotoSerif-Bold.ttf", "LiberationSerif-Bold.ttf", "FreeSerifBold.ttf"],
            &["DejaVuSerif-Italic.ttf", "NotoSerif-Italic.ttf", "LiberationSerif-Italic.ttf", "FreeSerifItalic.ttf"],
            &["DejaVuSerif-BoldItalic.ttf", "NotoSerif-BoldItalic.ttf", "LiberationSerif-BoldItalic.ttf", "FreeSerifBoldItalic.ttf"],
        ],
        Family::Mono => [
            &["DejaVuSansMono.ttf", "NotoSansMono-Regular.ttf", "LiberationMono-Regular.ttf", "FreeMono.ttf"],
            &["DejaVuSansMono-Bold.ttf", "NotoSansMono-Bold.ttf", "LiberationMono-Bold.ttf", "FreeMonoBold.ttf"],
            &["DejaVuSansMono-Oblique.ttf", "LiberationMono-Italic.ttf", "FreeMonoOblique.ttf"],
            &["DejaVuSansMono-BoldOblique.ttf", "LiberationMono-BoldItalic.ttf", "FreeMonoBoldOblique.ttf"],
        ],
    }
}

/// Lines and strikes are drawn by `text.rs`.
pub const NATIVE_DECO: bool = false;

/// DejaVu Sans's line (ascent + descent) per em (UI text matches the
/// baked font's em size).
pub const EM_RATIO: f32 = 2384.0 / 2048.0;

/// The font dropdown's name for `f`.
pub fn family_label(f: Family) -> &'static str {
    match f {
        Family::Sans => tr!("font.sans"),
        Family::Serif => tr!("font.serif"),
        Family::Mono => tr!("font.mono"),
    }
}

/// What `fc-match` gives for `pattern` (a file path), if it runs.
fn fc_match(pattern: &str) -> Option<String> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}", pattern]).output().ok()?;
    let p = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !p.is_empty()).then_some(p)
}

/// The face file of `f` in a style (`i`: 0 regular, 1 bold, 2 italic,
/// 3 bold italic) and whether it is that style.
fn find(f: Family, i: usize) -> Option<(FontArc, bool)> {
    let bold = i & 1 == 1;
    let italic = i & 2 == 2;
    let read = |p: &str| std::fs::read(p).ok().and_then(|b| FontArc::try_from_vec(b).ok());
    for n in names(f)[i] {
        if let Some(font) = DIRS.iter().find_map(|d| read(&format!("{d}{n}"))) {
            return Some((font, true));
        }
    }
    // Fontconfig's match: the style only when its file name says so.
    let fam = match f {
        Family::Sans => "sans-serif",
        Family::Serif => "serif",
        Family::Mono => "monospace",
    };
    let pat = format!("{fam}{}{}", if bold { ":weight=bold" } else { "" }, if italic { ":slant=italic" } else { "" });
    let path = fc_match(&pat)?;
    let l = path.to_ascii_lowercase();
    let styled = (!bold || l.contains("bold") || l.contains("heavy") || l.contains("black"))
        && (!italic || l.contains("italic") || l.contains("oblique"));
    read(&path).map(|font| (font, styled))
}

/// A face as loaded, with what is synthesized.
#[derive(Clone)]
pub struct Face {
    font: FontArc,
    /// Bold drawn by a second pass at an offset (no bold file).
    synth_bold: bool,
    /// Italic drawn sheared (no italic file).
    synth_italic: bool,
    /// UI text (`Some(right to left)`): characters the face lacks come
    /// from the font fontconfig picks for them, and lines are put in
    /// visual order ([`crate::text::visual_order`]).
    ui: Option<bool>,
}

/// Faces found so far, by family and style.
type Found = Mutex<HashMap<(Family, usize), Option<(FontArc, bool)>>>;

fn cache() -> &'static Found {
    static C: OnceLock<Found> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The face of `f` in style `i`: found once (the files read and
/// `fc-match` run without the lock held), then kept.
fn found(f: Family, i: usize) -> Option<(FontArc, bool)> {
    if let Some(v) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&(f, i)) {
        return v.clone();
    }
    let v = find(f, i);
    cache().lock().unwrap_or_else(|e| e.into_inner()).entry((f, i)).or_insert(v).clone()
}

/// fontconfig's language for the UI language (`:lang=`), so a font
/// collection gives the face of that language: Simplified or Traditional
/// Chinese, Japanese, Korean Han forms.
fn fc_lang() -> String {
    fc_lang_of(crate::i18n::current())
}

fn fc_lang_of(tag: &str) -> String {
    match tag {
        "zh-Hans" => "zh-cn".into(),
        "zh-Hant" => "zh-tw".into(),
        c => c.split('-').next().unwrap_or("en").to_ascii_lowercase(),
    }
}

/// The file and face index fontconfig picks for `pattern`.
fn fc_match_face(pattern: &str) -> Option<(String, u32)> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}\n%{index}", pattern]).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let mut it = s.lines();
    let file = it.next().filter(|p| !p.is_empty())?.to_string();
    let index = it.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    out.status.success().then_some((file, index))
}

/// A cache key: a 256-codepoint block (or one character, where the
/// block's font lacks it) in a language.
type FbKey = (u32, String);

#[derive(Default)]
struct Fallbacks {
    /// What each key resolved to (an index into `fonts`, or none).
    keys: HashMap<FbKey, Option<usize>>,
    /// Keys asked for and not answered yet.
    pending: std::collections::HashSet<FbKey>,
    /// (file, face index) → the face.
    fonts: Vec<((String, u32), FontArc)>,
}

fn fallbacks() -> std::sync::MutexGuard<'static, Fallbacks> {
    static C: OnceLock<Mutex<Fallbacks>> = OnceLock::new();
    C.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Bumped when a fallback font is found: UI text drawn before is stale.
static FONT_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn font_generation() -> u64 {
    FONT_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// The worker that runs `fc-match` and reads the font files: the UI
/// thread never waits for either.
fn queue() -> &'static Mutex<crate::chan::Sender<(char, FbKey)>> {
    static Q: OnceLock<Mutex<crate::chan::Sender<(char, FbKey)>>> = OnceLock::new();
    Q.get_or_init(|| {
        let (tx, rx) = crate::chan::channel::<(char, FbKey)>();
        let _ = std::thread::Builder::new().name("rustshot-fallback-fonts".into()).spawn(move || {
            while let Ok((c, key)) = rx.recv() {
                let found = fc_match_face(&format!("sans-serif:charset={:x}:lang={}", c as u32, key.1)).and_then(|(p, i)| {
                    if let Some(id) = fallbacks().fonts.iter().position(|(q, _)| *q == (p.clone(), i)) {
                        return Some(id);
                    }
                    let b = std::fs::read(&p).ok()?;
                    let f = ab_glyph::FontVec::try_from_vec_and_index(b, i).ok().map(FontArc::new)?;
                    let mut g = fallbacks();
                    g.fonts.push(((p, i), f));
                    Some(g.fonts.len() - 1)
                });
                {
                    let mut g = fallbacks();
                    g.pending.remove(&key);
                    g.keys.insert(key, found);
                }
                if found.is_some() {
                    FONT_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // The windows draw again (with the font now).
                    crate::i18n::bump();
                }
            }
        });
        Mutex::new(tx)
    })
}

/// A fallback for `c`: found, none, or still being looked for.
enum Fb {
    Font(usize, FontArc),
    None,
    Pending,
}

/// The font fontconfig picks for `c` when the UI face lacks it (asked
/// once per 256-codepoint block and language, then per character where
/// the block's font lacks one), from the worker: [`Fb::Pending`] until it
/// has answered.
fn fallback_font(c: char) -> Fb {
    let lang = fc_lang();
    for key in [((c as u32 >> 8) | 1 << 31, lang.clone()), (c as u32, lang)] {
        let mut g = fallbacks();
        match g.keys.get(&key).copied() {
            Some(Some(id)) => {
                let f = g.fonts[id].1.clone();
                if f.glyph_id(c).0 != 0 {
                    return Fb::Font(id, f);
                }
                continue;
            }
            Some(None) => continue,
            None => {}
        }
        if g.pending.insert(key.clone()) {
            drop(g);
            let _ = queue().lock().unwrap_or_else(|e| e.into_inner()).send((c, key));
        }
        return Fb::Pending;
    }
    Fb::None
}

/// Look up, in the background, the fonts the characters of `s` need
/// (a language's strings when it loads).
pub fn prefetch(s: &str) {
    let face = &ui_face_font();
    let mut seen = std::collections::HashSet::new();
    for c in s.chars().filter(|&c| !c.is_ascii() && !c.is_control() && !crate::text::is_bidi_control(c)) {
        if seen.insert(c as u32 >> 8) && face.glyph_id(c).0 == 0 {
            let _ = fallback_font(c);
        }
    }
}

fn ui_face_font() -> FontArc {
    static F: OnceLock<FontArc> = OnceLock::new();
    F.get_or_init(|| Face::new(Family::Sans, 500, false, false, false).font).clone()
}

/// Wait until every fallback asked for has been answered (tests).
#[cfg(test)]
pub fn wait_fallbacks() {
    let t = std::time::Instant::now();
    while !fallbacks().pending.is_empty() && t.elapsed() < std::time::Duration::from_secs(30) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Find every family's faces on a thread of their own (once), so the
/// first bold or serif text does not wait for the disk or `fc-match`.
fn warm() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("rustshot-fonts".into()).spawn(|| {
            for f in Family::ALL {
                for i in 0..4 {
                    let _ = found(f, i);
                }
            }
        });
    });
}

/// The shear of a synthesized italic: tan(12 degrees).
const SHEAR: f32 = 0.212_556_56;

impl Face {
    pub fn load() -> Option<Self> {
        let f = Face::new(Family::Sans, 400, false, false, false);
        warm();
        Some(f)
    }

    /// `family` at `weight` (700 and up: bold), italic; the line styles are
    /// drawn by `text.rs`.
    pub fn new(family: Family, weight: u16, italic: bool, _underline: bool, _strike: bool) -> Face {
        let bold = weight >= 700;
        let want = bold as usize | (italic as usize) << 1;
        // The styled file, else the family's regular one, else the sans
        // one, else Inter; what the file lacks is synthesized.
        let (font, styled) = found(family, want)
            .filter(|(_, s)| *s)
            .or_else(|| found(family, 0).map(|(f, _)| (f, false)))
            .or_else(|| found(Family::Sans, 0).map(|(f, _)| (f, false)))
            .unwrap_or_else(|| (FontArc::try_from_slice(INTER).expect("bundled Inter parses"), false));
        let plain = want == 0 || styled;
        Face { font, synth_bold: bold && !plain, synth_italic: italic && !plain, ui: None }
    }

    /// The UI face: per-character fallback fonts and visual order, the
    /// paragraph right to left when `rtl`.
    pub fn with_rtl(self, rtl: bool) -> Face {
        Face { ui: Some(rtl), ..self }
    }

    /// `line` in visual order as runs of one font each: (fallback font
    /// number, or None for this face; the font; its scale relative to
    /// `px`, so every font has this face's em; the characters).
    fn ui_runs(&self, line: &str, rtl: bool) -> Vec<(Option<usize>, FontArc, f32, Vec<char>)> {
        let em = |f: &FontArc| f.height_unscaled() / f.units_per_em().unwrap_or(1000.0);
        let own = em(&self.font);
        let mut runs: Vec<(Option<usize>, FontArc, f32, Vec<char>)> = Vec::new();
        for c in crate::text::visual_order(line, rtl) {
            if c.is_control() {
                continue;
            }
            let (c, fb) = if self.font.glyph_id(c).0 != 0 {
                (c, None)
            } else {
                match fallback_font(c) {
                    Fb::Font(id, f) => (c, Some((id, f))),
                    Fb::None => (c, None),
                    // Not found yet: a gap, not a box (drawn again when it is).
                    Fb::Pending => (' ', None),
                }
            };
            match runs.last_mut() {
                Some((id, _, _, cs)) if *id == fb.as_ref().map(|f| f.0) => cs.push(c),
                _ => match fb {
                    Some((id, f)) => {
                        let k = em(&f) / own;
                        runs.push((Some(id), f, k, vec![c]))
                    }
                    None => runs.push((None, self.font.clone(), 1.0, vec![c])),
                },
            }
        }
        runs
    }

    /// A synthesized bold's offset at `px`.
    fn bold_dx(&self, px: f32) -> f32 {
        if self.synth_bold { (px / 18.0).max(0.75) } else { 0.0 }
    }

    pub fn metrics(&self, px: f32) -> (f32, f32, f32) {
        let f = self.font.as_scaled(PxScale::from(px));
        (f.ascent(), -f.descent(), f.line_gap())
    }

    pub fn width(&self, line: &str, px: f32) -> f32 {
        if let Some(rtl) = self.ui {
            return self
                .ui_runs(line, rtl)
                .iter()
                .map(|(_, font, k, cs)| {
                    let f = font.as_scaled(PxScale::from(px * k));
                    cs.iter().map(|&c| f.h_advance(font.glyph_id(c))).sum::<f32>()
                })
                .sum();
        }
        let f = self.font.as_scaled(PxScale::from(px));
        let dx = self.bold_dx(px);
        line.chars().map(|c| f.h_advance(self.font.glyph_id(c)) + dx).sum()
    }

    pub fn draw(
        &self,
        line: &str,
        px: f32,
        top: Pt,
        ascent: f32,
        clip: (i32, i32, i32, i32),
        f: &mut dyn FnMut(i32, i32, f32),
    ) {
        let (bx0, by0, bx1, by1) = clip;
        if let Some(rtl) = self.ui {
            let mut put = |px: i32, py: i32, cov: f32| {
                if px >= bx0 && py >= by0 && px < bx1 && py < by1 {
                    f(px, py, cov);
                }
            };
            let y = top.y + ascent;
            let mut x = top.x;
            for (_, font, k, cs) in self.ui_runs(line, rtl) {
                let scale = PxScale::from(px * k);
                let sf = font.as_scaled(scale);
                for c in cs {
                    let gid = font.glyph_id(c);
                    if let Some(og) = font.outline_glyph(gid.with_scale_and_position(scale, ab_glyph::point(x, y))) {
                        let b = og.px_bounds();
                        og.draw(|gx, gy, cov| put(b.min.x.floor() as i32 + gx as i32, b.min.y.floor() as i32 + gy as i32, cov));
                    }
                    x += sf.h_advance(gid);
                }
            }
            return;
        }
        let scale = PxScale::from(px);
        let sf = self.font.as_scaled(scale);
        let y = top.y + ascent;
        let mut x = top.x;
        let dx = self.bold_dx(px);
        let mut put = |px: i32, py: i32, cov: f32| {
            if px >= bx0 && py >= by0 && px < bx1 && py < by1 {
                f(px, py, cov);
            }
        };
        for ch in line.chars() {
            let gid = self.font.glyph_id(ch);
            if self.synth_bold || self.synth_italic {
                // The outline sheared (italic) and drawn twice (bold).
                let offs = [0.0, dx];
                for off in &offs[..if self.synth_bold { 2 } else { 1 }] {
                    self.draw_synth(gid, sf.scale_factor().horizontal, Pt::new(x + off, y), &mut put);
                }
            } else {
                let glyph = gid.with_scale_and_position(scale, ab_glyph::point(x, y));
                if let Some(og) = self.font.outline_glyph(glyph) {
                    let b = og.px_bounds();
                    og.draw(|gx, gy, cov| put(b.min.x.floor() as i32 + gx as i32, b.min.y.floor() as i32 + gy as i32, cov));
                }
            }
            x += sf.h_advance(gid) + dx;
        }
    }

    /// One glyph's outline at pen `pos` (on the baseline), `k` px per
    /// font unit, sheared when italic is synthesized.
    fn draw_synth(&self, gid: ab_glyph::GlyphId, k: f32, pos: Pt, put: &mut dyn FnMut(i32, i32, f32)) {
        let Some(o) = self.font.outline(gid) else { return };
        let sh = if self.synth_italic { SHEAR } else { 0.0 };
        let map = |p: ab_glyph::Point| Pt::new(pos.x + (p.x + sh * p.y) * k, pos.y - p.y * k);
        let b = o.bounds;
        let (xs, ys) = ([b.min.x + sh * b.min.y, b.max.x + sh * b.max.y, b.min.x + sh * b.max.y, b.max.x + sh * b.min.y], [b.min.y, b.max.y]);
        let x0 = (pos.x + xs.iter().cloned().fold(f32::MAX, f32::min) * k).floor() - 1.0;
        let x1 = (pos.x + xs.iter().cloned().fold(f32::MIN, f32::max) * k).ceil() + 1.0;
        let y0 = (pos.y - ys[1] * k).floor() - 1.0;
        let y1 = (pos.y - ys[0] * k).ceil() + 1.0;
        let (w, h) = ((x1 - x0).max(0.0) as usize, (y1 - y0).max(0.0) as usize);
        if w == 0 || h == 0 || w * h > 4_000_000 {
            return;
        }
        let mut cov = Coverage::new(w, h);
        let o0 = |p: Pt| Pt::new(p.x - x0, p.y - y0);
        for c in &o.curves {
            match *c {
                OutlineCurve::Line(a, b) => cov.line(o0(map(a)), o0(map(b))),
                OutlineCurve::Quad(a, b, c) => cov.quad(o0(map(a)), o0(map(b)), o0(map(c))),
                OutlineCurve::Cubic(a, b, c, d) => {
                    // Flattened: 8 segments.
                    let (a, b, c, d) = (o0(map(a)), o0(map(b)), o0(map(c)), o0(map(d)));
                    let mut prev = a;
                    for i in 1..=8 {
                        let t = i as f32 / 8.0;
                        let u = 1.0 - t;
                        let p = Pt::new(
                            u * u * u * a.x + 3.0 * u * u * t * b.x + 3.0 * u * t * t * c.x + t * t * t * d.x,
                            u * u * u * a.y + 3.0 * u * u * t * b.y + 3.0 * u * t * t * c.y + t * t * t * d.y,
                        );
                        cov.line(prev, p);
                        prev = p;
                    }
                }
            }
        }
        let (ox, oy) = (x0 as i32, y0 as i32);
        cov.for_each(|gx, gy, c| put(ox + gx as i32, oy + gy as i32, c.min(1.0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fontconfig is asked in the UI language, so a CJK collection gives
    /// that language's face (the Han forms differ).
    #[test]
    fn fallbacks_ask_in_the_ui_language() {
        assert_eq!(fc_lang_of("zh-Hans"), "zh-cn");
        assert_eq!(fc_lang_of("zh-Hant"), "zh-tw");
        assert_eq!(fc_lang_of("ja"), "ja");
        assert_eq!(fc_lang_of("pt-BR"), "pt");
        // A collection's face for the language (when Noto CJK is here).
        let (Some(sc), Some(tc)) = (fc_match_face("sans-serif:charset=4e2d:lang=zh-cn"), fc_match_face("sans-serif:charset=4e2d:lang=zh-tw")) else {
            eprintln!("skipping: no fontconfig");
            return;
        };
        if sc.0 == tc.0 && sc.0.ends_with(".ttc") {
            assert_ne!(sc.1, tc.1, "{sc:?} {tc:?}: another face of the collection");
        }
    }
}
