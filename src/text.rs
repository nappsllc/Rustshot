//! Annotation text (user-typed, any script): measured and rasterised by the
//! OS text stack so shaping and font fallback come for free: GDI on Windows
//! (`text_win.rs`), CoreText on macOS (`text_macos.rs`); Linux rasterises a
//! system font with ab_glyph (`text_linux.rs`). `px` is the line's
//! ascent-to-descent height, as before.
//!
//! Lines are split on `\n`, stacked `line_height` apart from `top`, and each
//! line starts at `top.x`. Coverage is blended with the object colour by
//! [`Surf::blend_px`] once per pixel through one [`Mask`] for the whole
//! text; the backend only produces grayscale coverage.

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

/// The annotation font. Cheap to clone.
#[derive(Clone)]
pub struct AnnotFont(imp::Face);

impl AnnotFont {
    /// The platform font; `None` when none is available (text objects then
    /// draw nothing).
    pub fn load() -> Option<Self> {
        imp::Face::load().map(AnnotFont)
    }

    /// The same font at semibold where the OS has it (GDI FW_SEMIBOLD,
    /// Helvetica Neue Medium on macOS; Linux keeps its one face).
    pub fn semibold(&self) -> Self {
        AnnotFont(self.0.semibold())
    }

    pub fn metrics(&self, px: f32) -> LineMetrics {
        let (ascent, descent, gap) = self.0.metrics(px);
        LineMetrics { ascent, descent, line_height: (ascent + descent + gap).max(px * 1.1) }
    }

    /// Advance width of one line (no `\n`); caret offsets measure prefixes.
    pub fn line_width(&self, line: &str, px: f32) -> f32 {
        if line.is_empty() { 0.0 } else { self.0.width(line, px) }
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
        if let Some(x) = self.0.caret_x(line, idx, px) {
            return x;
        }
        self.line_width(line.get(..idx).unwrap_or(""), px)
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

    /// `text`'s coverage drawn with its top-left at the origin (its own
    /// box, unclipped): what [`TextCov::blend`] draws anywhere at a whole
    /// pixel offset, so a text drawn again and again is rasterised once.
    pub fn coverage(&self, text: &str, px: f32) -> TextCov {
        let (w, h, m) = self.measure(text, px);
        let pad = (px / 3.0).ceil() + 3.0;
        let (x0, y0) = (-pad as i32, -pad as i32);
        let (x1, y1) = ((w + pad).ceil() as i32, (h + pad).ceil() as i32);
        let (cw, ch) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
        let mut a = vec![0u8; cw * ch];
        let mut y = 0.0;
        for line in text.split('\n') {
            if !line.trim().is_empty() {
                self.0.draw(line, px, Pt::new(0.0, y), m.ascent, (x0, y0, x1, y1), &mut |x, y, cov| {
                    if x >= x0 && y >= y0 && x < x1 && y < y1 && cov > 0.0 {
                        let i = (y - y0) as usize * cw + (x - x0) as usize;
                        let v = (cov.min(1.0) * 255.0).round() as u8;
                        if v > a[i] {
                            a[i] = v;
                        }
                    }
                });
            }
            y += m.line_height;
        }
        TextCov { x0, y0, w: cw, h: ch, a }
    }

    /// Draw `text` with its top-left at `top`, clipped to the surface.
    pub fn render(&self, sf: &mut Surf, text: &str, px: f32, top: Pt, color: C4) {
        let (w, h, m) = self.measure(text, px);
        // One mask for every glyph of every line (max coverage), blended
        // once: touching or overlapping glyphs of a translucent colour do
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
        let mut y = top.y;
        for line in text.split('\n') {
            if !line.trim().is_empty() {
                self.0.draw(line, px, Pt::new(top.x, y), m.ascent, clip, &mut |x, y, cov| mask.put(x, y, cov));
            }
            y += m.line_height;
        }
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
        // A window over part of the text paints what the full surface has there.
        let f = font();
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
                assert_eq!(win[a..a + 4], full[b..b + 4], "({x}, {y})");
            }
        }
        assert!(win.iter().any(|&v| v != 0));
    }
}
