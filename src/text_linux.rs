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

/// The font fontconfig picks for `c` when the UI face lacks it, with a
/// number naming it: asked once per 256-codepoint block (then per
/// character where the block's font lacks one); each file read once.
fn fallback_font(c: char) -> Option<(usize, FontArc)> {
    type Fonts = Mutex<(HashMap<u32, Option<usize>>, Vec<(String, FontArc)>)>;
    static C: OnceLock<Fonts> = OnceLock::new();
    let cache = C.get_or_init(|| Mutex::new((HashMap::new(), Vec::new())));
    let lock = || cache.lock().unwrap_or_else(|e| e.into_inner());
    let has = |id: usize| {
        let g = lock();
        let f = g.1[id].1.clone();
        (f.glyph_id(c).0 != 0).then_some((id, f))
    };
    for key in [(c as u32 >> 8) | 1 << 31, c as u32] {
        let known = lock().0.get(&key).copied();
        match known {
            Some(Some(id)) => match has(id) {
                Some(hit) => return Some(hit),
                None => continue,
            },
            Some(None) => continue,
            None => {}
        }
        // `fc-match` and the file read run without the lock.
        let found = fc_match(&format!("sans-serif:charset={:x}", c as u32)).and_then(|p| {
            if let Some(id) = lock().1.iter().position(|(q, _)| *q == p) {
                return Some(id);
            }
            let f = std::fs::read(&p).ok().and_then(|b| FontArc::try_from_vec(b).ok())?;
            let mut g = lock();
            g.1.push((p, f));
            Some(g.1.len() - 1)
        });
        lock().0.insert(key, found);
        if let Some(hit) = found.and_then(has) {
            return Some(hit);
        }
    }
    None
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
            let fb = if self.font.glyph_id(c).0 != 0 { None } else { fallback_font(c) };
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
