//! Annotation text (user-typed, any script): measured and rasterised by the
//! OS text stack so shaping and font fallback come for free: GDI on Windows
//! (`text_win.rs`), CoreText on macOS (`text_macos.rs`); Linux rasterises a
//! system font with ab_glyph (`text_linux.rs`). `px` is the line's
//! ascent-to-descent height, as before.
//!
//! Lines are split on `\n`, stacked `line_height` apart from `top`, and each
//! line starts at `top.x` (moved right by the alignment, [`Align`]).
//! Coverage is blended with the object color by [`Surf::blend_px`] once
//! per pixel through one [`Mask`] for the whole text; the backend only
//! produces grayscale coverage.
//!
//! A [`TextFmt`] styles a whole text: the family, bold and italic pick
//! the face (GDI weight / italic, CoreText traits; Linux a bold / italic
//! file, synthesized when there is none: emboldened by a second pass at an
//! offset, sheared 12 degrees), underline and strikethrough are GDI's own
//! on Windows and lines drawn here elsewhere.

use crate::objects::Pt;
use crate::raster::{Blend, Mask, Surf};
use crate::uifb::C4;

#[cfg(windows)]
#[path = "text_win.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "text_linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "text_macos.rs"]
mod imp;

/// A text's face family: the OS UI font, a serif, a monospace (Windows:
/// Segoe UI / Georgia / Consolas; macOS: Helvetica / Times / Menlo;
/// Linux: DejaVu Sans / Serif / Sans Mono or what fontconfig matches).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Family {
    #[default]
    Sans,
    Serif,
    Mono,
}

impl Family {
    pub const ALL: [Family; 3] = [Family::Sans, Family::Serif, Family::Mono];

    /// The annotations.json / config name.
    pub fn id(self) -> &'static str {
        match self {
            Family::Sans => "sans",
            Family::Serif => "serif",
            Family::Mono => "mono",
        }
    }

    pub fn from_id(s: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.id() == s)
    }

    /// The font dropdown's name for it (the face this OS draws).
    pub fn label(self) -> &'static str {
        imp::family_label(self)
    }
}

/// How the lines of a multi-line text line up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    pub const ALL: [Align; 3] = [Align::Left, Align::Center, Align::Right];

    pub fn id(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }

    pub fn from_id(s: &str) -> Option<Align> {
        Align::ALL.into_iter().find(|a| a.id() == s)
    }
}

/// One style for a whole text or callout (no rich runs): the family,
/// bold, italic, underline, strikethrough and (text only) the alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct TextFmt {
    pub family: Family,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub align: Align,
}

impl TextFmt {
    /// Plain: what every text was before formatting (and what a file
    /// without the fields reads as).
    pub fn is_plain(&self) -> bool {
        *self == TextFmt::default()
    }

    /// The style words (`bold,italic,underline,strike`, those set, in
    /// that order; empty when none): `tool_text_style` in the config.
    pub fn style_id(&self) -> String {
        let mut v = Vec::new();
        for (on, w) in [(self.bold, "bold"), (self.italic, "italic"), (self.underline, "underline"), (self.strike, "strike")] {
            if on {
                v.push(w);
            }
        }
        v.join(",")
    }

    /// `self` with the style words of `s` (commas or spaces between
    /// them; unknown words ignored); the family and alignment kept.
    pub fn with_style(self, s: &str) -> TextFmt {
        let mut f = TextFmt { bold: false, italic: false, underline: false, strike: false, ..self };
        for w in s.split([',', ' ']).map(str::trim) {
            match w {
                "bold" => f.bold = true,
                "italic" => f.italic = true,
                "underline" => f.underline = true,
                "strike" | "strikethrough" => f.strike = true,
                _ => {}
            }
        }
        f
    }

    /// A small number naming the style (raster and wrap caches).
    pub fn key(&self) -> u32 {
        self.family as u32
            | (self.bold as u32) << 2
            | (self.italic as u32) << 3
            | (self.underline as u32) << 4
            | (self.strike as u32) << 5
            | (self.align as u32) << 6
    }
}

/// Vertical metrics of one line at a pixel size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    /// Baseline below the line top.
    pub ascent: f32,
    /// Below the baseline (positive).
    pub descent: f32,
    /// Distance between consecutive line tops.
    pub line_height: f32,
}

/// The annotation font in one style ([`TextFmt`]). Cheap to clone.
#[derive(Clone)]
pub struct AnnotFont {
    face: imp::Face,
    /// The weight when not bold (400 regular, 600 the callout's semibold).
    base: u16,
    fmt: TextFmt,
}

impl AnnotFont {
    /// The platform font; `None` when none is available (text objects then
    /// draw nothing).
    pub fn load() -> Option<Self> {
        imp::Face::load().map(|face| AnnotFont { face, base: 400, fmt: TextFmt::default() })
    }

    fn with(&self, base: u16, fmt: TextFmt) -> Self {
        let weight = if fmt.bold { base.max(700) } else { base };
        let face = imp::Face::new(fmt.family, weight, fmt.italic, fmt.underline, fmt.strike);
        AnnotFont { face, base, fmt }
    }

    /// The same font at semibold where the OS has it (GDI FW_SEMIBOLD,
    /// Helvetica Neue Medium on macOS; Linux keeps its regular face);
    /// bold stays bold.
    pub fn semibold(&self) -> Self {
        self.with(self.base.max(600), self.fmt)
    }

    /// This font in style `fmt` (its family, bold, italic, underline,
    /// strikethrough and alignment).
    pub fn styled(&self, fmt: TextFmt) -> Self {
        if fmt == self.fmt {
            return self.clone();
        }
        self.with(self.base, fmt)
    }

    /// The style this font draws.
    #[allow(dead_code)]
    pub fn fmt(&self) -> TextFmt {
        self.fmt
    }

    /// A number naming the face and style (raster caches).
    pub fn key(&self) -> u32 {
        self.fmt.key() | (self.base as u32) << 16
    }

    pub fn metrics(&self, px: f32) -> LineMetrics {
        let (ascent, descent, gap) = self.face.metrics(px);
        LineMetrics { ascent, descent, line_height: (ascent + descent + gap).max(px * 1.1) }
    }

    /// Advance width of one line (no `\n`); caret offsets measure prefixes.
    pub fn line_width(&self, line: &str, px: f32) -> f32 {
        if line.is_empty() { 0.0 } else { self.face.width(line, px) }
    }

    /// Where the caret before byte `idx` of `line` goes, from the line's
    /// left. Windows asks Uniscribe (`ScriptStringCPtoX`), so the caret
    /// sits where the character is drawn in right-to-left text (Arabic,
    /// Hebrew) and mixed runs. Linux (ab_glyph lays glyphs out left to
    /// right in logical order) and macOS take the prefix's width: right for
    /// left-to-right text, approximate for right-to-left text on macOS
    /// (CoreText reorders it when drawing).
    pub fn caret_x(&self, line: &str, idx: usize, px: f32) -> f32 {
        let idx = idx.min(line.len());
        #[cfg(windows)]
        if let Some(x) = self.face.caret_x(line, idx, px) {
            return x;
        }
        self.line_width(line.get(..idx).unwrap_or(""), px)
    }

    /// Where a line `line_w` wide starts in a text `w` wide, as aligned
    /// (whole pixels).
    pub fn line_x(&self, w: f32, line_w: f32) -> f32 {
        match self.fmt.align {
            Align::Left => 0.0,
            Align::Center => ((w - line_w) / 2.0).round(),
            Align::Right => (w - line_w).round(),
        }
    }

    /// The caret before byte `idx` of a (multi-line) `text`: its top-left
    /// from the text's top-left, the lines aligned.
    pub fn caret_at(&self, text: &str, idx: usize, px: f32) -> Pt {
        let (w, _, m) = self.measure(text, px);
        let idx = idx.min(text.len());
        let mut start = 0;
        for (row, line) in text.split('\n').enumerate() {
            let end = start + line.len();
            if idx <= end {
                let x = self.line_x(w, self.line_width(line, px)) + self.caret_x(line, idx - start, px);
                return Pt::new(x, row as f32 * m.line_height);
            }
            start = end + 1;
        }
        Pt::new(0.0, 0.0)
    }

    /// Widest line and total height of `text`, with the line metrics.
    pub fn measure(&self, text: &str, px: f32) -> (f32, f32, LineMetrics) {
        let m = self.metrics(px);
        let (mut w, mut n) = (0.0f32, 0);
        for line in text.split('\n') {
            w = w.max(self.line_width(line, px));
            n += 1;
        }
        (w, m.line_height * n as f32, m)
    }

    /// Each line's glyphs and its underline / strikethrough (where the
    /// backend does not draw them) as `f(x, y, coverage)`, the text's
    /// top-left at `top`, clipped to `clip`.
    fn draw_lines(&self, text: &str, px: f32, top: Pt, clip: (i32, i32, i32, i32), f: &mut dyn FnMut(i32, i32, f32)) {
        let (w, _, m) = self.measure(text, px);
        let deco = !imp::NATIVE_DECO && (self.fmt.underline || self.fmt.strike);
        let t = (px / 14.0).round().max(1.0) as i32;
        let mut y = top.y;
        for line in text.split('\n') {
            if !line.trim().is_empty() {
                let lw = self.line_width(line, px);
                let x = top.x + self.line_x(w, lw);
                self.face.draw(line, px, Pt::new(x, y), m.ascent, clip, f);
                if deco {
                    let base = y + m.ascent;
                    let mut bar = |y0: f32| {
                        let (x0, x1) = (x.round() as i32, (x + lw).round() as i32);
                        let y0 = y0.round() as i32;
                        for yy in y0.max(clip.1)..(y0 + t).min(clip.3) {
                            for xx in x0.max(clip.0)..x1.min(clip.2) {
                                f(xx, yy, 1.0);
                            }
                        }
                    };
                    if self.fmt.underline {
                        bar(base + (m.descent * 0.35).max(1.0));
                    }
                    if self.fmt.strike {
                        bar(base - m.ascent * 0.32 - t as f32 / 2.0);
                    }
                }
            }
            y += m.line_height;
        }
    }

    /// `text`'s coverage drawn with its top-left at the origin (its own
    /// box, unclipped): what [`TextCov::blend`] draws anywhere at a whole
    /// pixel offset, so a text drawn again and again is rasterised once.
    pub fn coverage(&self, text: &str, px: f32) -> TextCov {
        let (w, h, _) = self.measure(text, px);
        let pad = (px / 3.0).ceil() + 3.0;
        let (x0, y0) = (-pad as i32, -pad as i32);
        let (x1, y1) = ((w + pad).ceil() as i32, (h + pad).ceil() as i32);
        let (cw, ch) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
        let mut a = vec![0u8; cw * ch];
        self.draw_lines(text, px, Pt::new(0.0, 0.0), (x0, y0, x1, y1), &mut |x, y, cov| {
            if x >= x0 && y >= y0 && x < x1 && y < y1 && cov > 0.0 {
                let i = (y - y0) as usize * cw + (x - x0) as usize;
                let v = (cov.min(1.0) * 255.0).round() as u8;
                if v > a[i] {
                    a[i] = v;
                }
            }
        });
        TextCov { x0, y0, w: cw, h: ch, a }
    }

    /// Draw `text` with its top-left at `top`, clipped to the surface.
    pub fn render(&self, sf: &mut Surf, text: &str, px: f32, top: Pt, color: C4) {
        let (w, h, _) = self.measure(text, px);
        // One mask for every glyph of every line (max coverage), blended
        // once: touching or overlapping glyphs of a translucent color do
        // not darken where they meet. The pad covers overhang past the
        // advance box (as the backends' own pad does).
        let pad = (px / 3.0).ceil() + 3.0;
        let mut mask = Mask::new(
            sf,
            (top.x - pad).floor() as i32,
            (top.y - pad).floor() as i32,
            (top.x + w + pad).ceil() as i32,
            (top.y + h + pad).ceil() as i32,
        );
        let clip = sf.bounds();
        self.draw_lines(text, px, top, clip, &mut |x, y, cov| mask.put(x, y, cov));
        mask.blend(sf, color, Blend::Normal);
    }
}

/// A text's coverage at the origin ([`AnnotFont::coverage`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TextCov {
    pub x0: i32,
    pub y0: i32,
    pub w: usize,
    pub h: usize,
    pub a: Vec<u8>,
}

impl TextCov {
    /// Blend `color` through the coverage with the text's top-left at
    /// `at`, once per pixel, clipped to the surface.
    pub fn blend(&self, sf: &mut Surf, at: (i32, i32), color: C4) {
        let (ox, oy) = (self.x0 + at.0, self.y0 + at.1);
        let mut mask = Mask::new(sf, ox, oy, ox + self.w as i32, oy + self.h as i32);
        for (r, row) in self.a.chunks_exact(self.w.max(1)).enumerate() {
            for (i, &v) in row.iter().enumerate() {
                if v > 0 {
                    mask.put(ox + i as i32, oy + r as i32, v as f32 / 255.0);
                }
            }
        }
        mask.blend(sf, color, Blend::Normal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> AnnotFont {
        AnnotFont::load().expect("annotation font")
    }

    fn lit(text: &str, px: f32) -> usize {
        let f = font();
        let (w, h, _) = f.measure(text, px);
        let (w, h) = (w as u32 + 40, h as u32 + 20);
        let mut d = vec![0u8; (w * h * 4) as usize];
        f.render(&mut Surf::new(&mut d, w, h), text, px, Pt::new(10.0, 5.0), C4::rgb(255, 255, 255));
        d.as_chunks::<4>().0.iter().filter(|p| p[0] > 64).count()
    }

    #[test]
    fn renders_every_script() {
        for s in ["Hello, world", "Привет, мир", "مرحبا بالعالم", "你好世界"] {
            let n = lit(s, 24.0);
            assert!(n > 30, "{s:?}: only {n} lit pixels");
        }
    }

    #[test]
    fn measure_is_monotonic_in_length() {
        let f = font();
        for s in ["Annotation text", "Привет, мир", "Mixed: abc 123 xyz"] {
            let mut prev = 0.0f32;
            let mut end = 0;
            for (i, ch) in s.char_indices() {
                end = i + ch.len_utf8();
                let w = f.line_width(&s[..end], 20.0);
                assert!(w >= prev, "{s:?} up to {end}: {w} < {prev}");
                prev = w;
            }
            assert!(prev > 20.0 && end == s.len(), "{s:?}: {prev}");
        }
    }

    /// The caret: after the prefix in left-to-right text; in Hebrew and
    /// Arabic (Windows: Uniscribe) it starts at the right and moves left.
    #[test]
    fn the_caret_follows_the_text_direction() {
        let f = font();
        let ltr = "Hello";
        for i in 0..=ltr.len() {
            let want = f.line_width(&ltr[..i], 24.0);
            assert!((f.caret_x(ltr, i, 24.0) - want).abs() <= 1.0, "{i}");
        }
        assert_eq!(f.caret_x(ltr, 99, 24.0), f.caret_x(ltr, ltr.len(), 24.0), "past the end: the end");
        #[cfg(windows)]
        for rtl in ["\u{5e9}\u{5dc}\u{5d5}\u{5dd}", "\u{645}\u{631}\u{62d}\u{628}\u{627}"] {
            let w = f.line_width(rtl, 24.0);
            let xs: Vec<f32> = rtl.char_indices().map(|(i, _)| i).chain([rtl.len()]).map(|i| f.caret_x(rtl, i, 24.0)).collect();
            assert!((xs[0] - w).abs() <= 2.0, "the start is at the right: {xs:?} of {w}");
            assert!(xs[xs.len() - 1].abs() <= 2.0, "the end is at the left: {xs:?}");
            assert!(xs.windows(2).all(|p| p[1] <= p[0]), "moves left: {xs:?}");
        }
    }

    #[test]
    fn metrics_scale_with_size() {
        let f = font();
        let (a, b) = (f.metrics(12.0), f.metrics(36.0));
        assert!(a.ascent > 5.0 && a.descent > 0.5, "{a:?}");
        assert!(b.line_height > a.line_height * 2.5, "{a:?} {b:?}");
        assert!(a.line_height >= 12.0 * 1.1);
        let (w1, h1, _) = f.measure("ab\ncdef", 20.0);
        let (w2, h2, _) = f.measure("cdef", 20.0);
        assert_eq!(w1, w2);
        assert_eq!(h1, 2.0 * h2);
    }

    #[test]
    fn clipped_render_matches_full_render() {
        // A window over part of the text paints what the full surface has
        // there, in every style.
        let base = font();
        let styles = [
            TextFmt::default(),
            TextFmt { bold: true, italic: true, underline: true, strike: true, ..Default::default() },
            TextFmt { family: Family::Serif, align: Align::Center, underline: true, ..Default::default() },
            TextFmt { family: Family::Mono, align: Align::Right, strike: true, ..Default::default() },
        ];
        for fmt in styles {
            let f = base.styled(fmt);
            let (w, h) = (200u32, 60u32);
            let mut full = vec![0u8; (w * h * 4) as usize];
            let c = C4::new(250, 60, 20, 210);
            f.render(&mut Surf::new(&mut full, w, h), "Hello Wy\nПривет", 20.0, Pt::new(7.3, 4.6), c);
            let (cx, cy, cw, ch) = (31u32, 9u32, 50u32, 30u32);
            let mut win = vec![0u8; (cw * ch * 4) as usize];
            f.render(
                &mut Surf::with_origin(&mut win, cw, ch, cx as i32, cy as i32, crate::raster::Order::Rgba),
                "Hello Wy\nПривет",
                20.0,
                Pt::new(7.3, 4.6),
                c,
            );
            for y in 0..ch {
                for x in 0..cw {
                    let a = ((y * cw + x) * 4) as usize;
                    let b = (((y + cy) * w + x + cx) * 4) as usize;
                    assert_eq!(win[a..a + 4], full[b..b + 4], "{fmt:?} ({x}, {y})");
                }
            }
            assert!(win.iter().any(|&v| v != 0));
        }
    }

    /// `text` rendered white on black with `fmt` (its own box).
    fn shot(fmt: TextFmt, text: &str) -> Vec<u8> {
        let f = font().styled(fmt);
        let (w, h) = (260u32, 110u32);
        let mut d = vec![0u8; (w * h * 4) as usize];
        f.render(&mut Surf::new(&mut d, w, h), text, 28.0, Pt::new(12.0, 8.0), C4::rgb(255, 255, 255));
        d.as_chunks::<4>().0.iter().map(|p| p[0]).collect()
    }

    /// Every style draws differently from plain text: bold more ink,
    /// italic leaning (more ink to the right at the top), underline and
    /// strikethrough a full-width row, the other families other glyphs.
    #[test]
    fn styles_change_the_pixels() {
        let text = "Hamburger";
        let plain = shot(TextFmt::default(), text);
        let ink = |v: &[u8]| v.iter().map(|&p| p as u64).sum::<u64>();
        let differs = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
        let bold = shot(TextFmt { bold: true, ..Default::default() }, text);
        assert!(ink(&bold) > ink(&plain) * 11 / 10, "bold has more ink: {} vs {}", ink(&bold), ink(&plain));
        let italic = shot(TextFmt { italic: true, ..Default::default() }, text);
        assert!(differs(&italic, &plain) > 200, "italic differs");
        for fmt in [TextFmt { underline: true, ..Default::default() }, TextFmt { strike: true, ..Default::default() }] {
            let v = shot(fmt, text);
            // A row lit across most of the word's width that plain lacks.
            let w = 260usize;
            let row = (0..110usize).find(|&y| {
                let lit = |img: &[u8]| (12..12 + 100).filter(|&x| img[y * w + x] > 128).count();
                lit(&v) >= 95 && lit(&plain) < 60
            });
            assert!(row.is_some(), "{fmt:?}: no line across the text");
        }
        for fam in [Family::Serif, Family::Mono] {
            let v = shot(TextFmt { family: fam, ..Default::default() }, text);
            assert!(differs(&v, &plain) > 200, "{fam:?} draws another face");
        }
    }

    /// Lines line up left, centred or right within the widest one; the
    /// caret follows.
    #[test]
    fn alignment_moves_shorter_lines() {
        let base = font();
        let text = "a long first line\nshort";
        let (w, _, m) = base.measure(text, 20.0);
        let sw = base.line_width("short", 20.0);
        for (align, want) in [(Align::Left, 0.0), (Align::Center, ((w - sw) / 2.0).round()), (Align::Right, (w - sw).round())] {
            let f = base.styled(TextFmt { align, ..Default::default() });
            let at = f.caret_at(text, text.len() - 5, 20.0);
            assert_eq!(at, Pt::new(want, m.line_height), "{align:?}");
            assert_eq!(f.caret_at(text, 0, 20.0), Pt::new(0.0, 0.0), "the widest line starts at 0");
        }
    }

    #[test]
    fn style_words_round_trip() {
        let f = TextFmt { bold: true, strike: true, family: Family::Mono, ..Default::default() };
        assert_eq!(f.style_id(), "bold,strike");
        assert_eq!(TextFmt { family: Family::Mono, ..Default::default() }.with_style("bold, strike nonsense"), f);
        assert_eq!(TextFmt::default().style_id(), "");
        assert!(TextFmt::default().is_plain() && !f.is_plain());
        for fam in Family::ALL {
            assert_eq!(Family::from_id(fam.id()), Some(fam));
            assert!(!fam.label().is_empty());
        }
        for a in Align::ALL {
            assert_eq!(Align::from_id(a.id()), Some(a));
        }
        let keys: std::collections::BTreeSet<u32> =
            [TextFmt::default(), f, TextFmt { italic: true, ..Default::default() }, TextFmt { align: Align::Right, ..Default::default() }].iter().map(TextFmt::key).collect();
        assert_eq!(keys.len(), 4);
    }
}
