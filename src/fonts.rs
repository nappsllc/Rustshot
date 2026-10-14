//! UI font: Inter Medium (Latin + UI symbols, see tools/subset-font.sh),
//! baked by build.rs into static tables (metrics, cmap, advances, kerning,
//! outlines) and rasterised with [`Coverage`]: no font parser at run time.
//! Layout and coverage match what ab_glyph produced from the TTF (no
//! hinting; `px` is the ascent-to-descent height, as `ab_glyph::PxScale`).
//! Annotation text, which may be in any script, is in `text.rs`.

use crate::objects::Pt;
use crate::raster::Coverage;

/// One baked glyph: advance and bounding box in font units (bbox is
/// `[x_min, y_max, x_max, y_min]`, all zero for an empty glyph), and its
/// outline as `n_ops` segment codes from op index `op`, consuming points
/// from point index `pt`.
pub struct G {
    pub adv: u16,
    pub bbox: [i16; 4],
    pub op: u16,
    pub n_ops: u16,
    pub pt: u16,
}

#[allow(dead_code, clippy::all)]
mod baked {
    use super::G;
    include!(concat!(env!("OUT_DIR"), "/font_baked.rs"));
}

/// The outlines, inflated from [`baked::OUTLINES_Z`] on first use: `ops`
/// packs 2-bit segment codes four per byte (0 move, 1 line, 2 quad); `pts`
/// holds the x, y pairs they consume (1, 1 and 2 points), in half font
/// units.
struct Outlines {
    ops: Vec<u8>,
    pts: Vec<i16>,
}

fn outlines() -> &'static Outlines {
    static O: std::sync::OnceLock<Outlines> = std::sync::OnceLock::new();
    O.get_or_init(|| {
        let (no, np) = (baked::N_OPS_BYTES, baked::N_PTS);
        let (b, _) = crate::deflate::inflate(baked::OUTLINES_Z, no + 2 * np).expect("baked font outlines inflate");
        assert_eq!(b.len(), no + 2 * np, "baked font outlines");
        let (lo, hi) = b[no..].split_at(np);
        let mut last = [0i16; 2];
        let pts = (0..np)
            .map(|i| {
                let z = lo[i] as u16 | (hi[i] as u16) << 8;
                let d = (z >> 1) as i16 ^ -((z & 1) as i16);
                last[i % 2] = last[i % 2].wrapping_add(d);
                last[i % 2]
            })
            .collect();
        Outlines { ops: b[..no].to_vec(), pts }
    })
}

/// The baked overlay chrome font (Inter Medium).
pub struct UiFont {
    _private: (),
}

pub static UI: UiFont = UiFont { _private: () };

/// The overlay chrome font.
pub fn ui_font() -> &'static UiFont {
    &UI
}

impl UiFont {
    /// Pixels per font unit at `px` (ab_glyph's `h_scale_factor`).
    fn k(&self, px: f32) -> f32 {
        px / (baked::ASCENT as f32 - baked::DESCENT as f32)
    }

    /// Glyph slot for `ch` (0, .notdef, when unmapped).
    fn slot(&self, ch: char) -> usize {
        baked::CMAP
            .binary_search_by_key(&ch, |&(c, _)| c)
            .map_or(0, |i| baked::CMAP[i].1 as usize)
    }

    /// Whether `ch` has its own glyph.
    pub fn has_glyph(&self, ch: char) -> bool {
        self.slot(ch) != 0
    }

    fn kern(&self, a: usize, b: usize) -> f32 {
        baked::KERN
            .binary_search_by_key(&(a as u16, b as u16), |&(l, r, _)| (l, r))
            .map_or(0.0, |i| baked::KERN[i].2 as f32)
    }

    pub fn ascent(&self, px: f32) -> f32 {
        self.k(px) * baked::ASCENT as f32
    }

    /// Negative: below the baseline.
    pub fn descent(&self, px: f32) -> f32 {
        self.k(px) * baked::DESCENT as f32
    }

    #[cfg(test)]
    pub fn line_gap(&self, px: f32) -> f32 {
        self.k(px) * baked::LINE_GAP as f32
    }

    #[cfg(test)]
    pub fn units_per_em(&self) -> u16 {
        baked::UNITS_PER_EM
    }

    /// The em size at `px` (UI text on the OS stack asks for this em).
    pub fn em(&self, px: f32) -> f32 {
        self.k(px) * baked::UNITS_PER_EM as f32
    }

    /// Ascent minus descent: `px` up to float rounding.
    pub fn height(&self, px: f32) -> f32 {
        self.ascent(px) - self.descent(px)
    }

    /// Advance of `ch` alone at `px`.
    #[cfg(test)]
    pub fn advance(&self, ch: char, px: f32) -> f32 {
        self.k(px) * baked::GLYPHS[self.slot(ch)].adv as f32
    }

    /// Pen positions from `start`: `f(slot, pen x)` per character; returns
    /// the final pen. (Accumulated from `start` in order, the same float
    /// sums the old ab_glyph loops made.)
    fn layout(&self, s: &str, px: f32, start: f32, mut f: impl FnMut(usize, f32)) -> f32 {
        let k = self.k(px);
        let mut pen = start;
        let mut prev: Option<usize> = None;
        // Bidi format characters (isolates round number pairs): no
        // glyph, no width, no effect on the kerning.
        for ch in s.chars().filter(|&c| !crate::text::is_bidi_control(c)) {
            let g = self.slot(ch);
            if let Some(p) = prev {
                pen += k * self.kern(p, g);
            }
            f(g, pen);
            pen += k * baked::GLYPHS[g].adv as f32;
            prev = Some(g);
        }
        pen
    }

    /// Advance width of `s` at `px` (on the OS text stack when the
    /// baked glyphs do not cover it: `uitext.rs`).
    pub fn width(&self, s: &str, px: f32) -> f32 {
        if crate::uitext::needed(self, s) {
            return crate::uitext::width(self, s, px);
        }
        self.baked_width(s, px)
    }

    /// The width of `full[..end]` as part of `full` (a caret's x, a
    /// selection's edge): measured with the face that draws all of `full`,
    /// so an ASCII prefix of a string the OS draws is measured by the OS.
    pub fn prefix_width(&self, full: &str, end: usize, px: f32) -> f32 {
        let p = &full[..end];
        if crate::uitext::needed(self, full) {
            return crate::uitext::width(self, p, px);
        }
        self.baked_width(p, px)
    }

    /// [`UiFont::width`] with the baked glyphs only (.notdef for others).
    pub fn baked_width(&self, s: &str, px: f32) -> f32 {
        self.layout(s, px, 0.0, |_, _| {})
    }

    /// Rasterise `s` with the top-left corner at (x, y): calls `f(x, y,
    /// coverage)` for every pixel of each glyph's box (coverage 0..=1, or a
    /// little above 1 where contours overlap). Returns the advance width.
    pub fn draw(&self, s: &str, px: f32, x: f32, y: f32, mut f: impl FnMut(i32, i32, f32)) -> f32 {
        if crate::uitext::needed(self, s) {
            return crate::uitext::draw(self, s, px, x, y, &mut f);
        }
        self.baked_draw(s, px, x, y, f)
    }

    /// [`UiFont::draw`] with the baked glyphs only (.notdef for others).
    pub fn baked_draw(&self, s: &str, px: f32, x: f32, y: f32, mut f: impl FnMut(i32, i32, f32)) -> f32 {
        let k = self.k(px);
        let baseline = y + self.ascent(px);
        let end = self.layout(s, px, x, |g, pen| {
            draw_glyph(&baked::GLYPHS[g], k, Pt::new(pen, baseline), &mut f);
        });
        end - x
    }
}

/// One glyph at `pos` (pen on the baseline), `k` pixels per font unit:
/// ab_glyph's `px_bounds` and `OutlinedGlyph::draw`, on baked data.
fn draw_glyph(g: &G, k: f32, pos: Pt, f: &mut impl FnMut(i32, i32, f32)) {
    if g.n_ops == 0 {
        return;
    }
    let [x_min, y_max, x_max, y_min] = g.bbox.map(|v| v as f32);
    let (xt, xf) = (pos.x.trunc(), pos.x.fract());
    let (yt, yf) = (pos.y.trunc(), pos.y.fract());
    let min = Pt::new((x_min * k + xf).floor() + xt, (y_max * -k + yf).floor() + yt);
    let max = Pt::new((x_max * k + xf).ceil() + xt, (y_min * -k + yf).ceil() + yt);
    let (w, h) = ((max.x - min.x) as usize, (max.y - min.y) as usize);
    let off = Pt::new(pos.x - min.x, pos.y - min.y);
    let mut cov = Coverage::new(w, h);
    let o = outlines();
    let mut pi = g.pt as usize;
    let mut next = || {
        let p = Pt::new(
            o.pts[2 * pi] as f32 * 0.5 * k + off.x,
            o.pts[2 * pi + 1] as f32 * 0.5 * -k + off.y,
        );
        pi += 1;
        p
    };
    let mut cur = Pt::new(0.0, 0.0);
    for i in g.op as usize..(g.op + g.n_ops) as usize {
        match (o.ops[i / 4] >> (2 * (i % 4))) & 3 {
            0 => cur = next(),
            1 => {
                let p = next();
                cov.line(cur, p);
                cur = p;
            }
            _ => {
                let (c, p) = (next(), next());
                cov.quad(cur, c, p);
                cur = p;
            }
        }
    }
    let (bx, by) = (min.x as i32, min.y as i32);
    cov.for_each(|gx, gy, c| f(bx + gx as i32, by + gy as i32, c));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::{Font, FontArc, PxScale, ScaleFont};

    static INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.subset.ttf");

    fn reference() -> FontArc {
        FontArc::try_from_slice(INTER).expect("embedded Inter parses")
    }

    const SIZES: [f32; 10] = [11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0];
    const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

    fn all_chars() -> Vec<char> {
        baked::CMAP.iter().map(|&(c, _)| c).collect()
    }

    #[test]
    fn ui_font_has_ui_glyphs() {
        for ch in ['A', 'g', '0', '×', '·', '…', '—', '⇧', '⌘', '⌥', '⏎', '←', '↓'] {
            assert!(UI.has_glyph(ch), "missing {ch:?}");
        }
        // Latin, Greek, Cyrillic and Vietnamese are baked; CJK is not.
        for ch in ['é', 'ß', 'ł', 'ş', 'Δ', 'Ж', 'ї', 'ữ', '°', '²', '€', '›'] {
            assert!(UI.has_glyph(ch), "missing {ch:?}");
        }
        assert!(!UI.has_glyph('保'));
    }

    #[test]
    fn cmap_matches_the_ttf() {
        let f = reference();
        let mapped: Vec<char> = {
            let mut v: Vec<char> = f.codepoint_ids().filter(|(g, _)| g.0 != 0).map(|(_, c)| c).collect();
            v.sort();
            v.dedup();
            v
        };
        assert_eq!(all_chars(), mapped);
        assert!(baked::CMAP.windows(2).all(|w| w[0].0 < w[1].0), "sorted");
        assert!(baked::KERN.windows(2).all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1)), "sorted");
        assert_eq!(UI.units_per_em() as f32, f.units_per_em().unwrap());
    }

    /// Every advance, the metrics and whole-string widths equal ab_glyph's
    /// (which the old code summed without kerning; the subset has no
    /// `kern` table, so the baked pair list is empty and changes nothing).
    #[test]
    fn measurement_matches_ab_glyph() {
        let f = reference();
        let chars = all_chars();
        let all: String = chars.iter().collect();
        for size in SIZES {
            for s in SCALES {
                let px = size * s;
                let sf = f.as_scaled(PxScale { x: px, y: px });
                assert_eq!(UI.ascent(px), sf.ascent(), "ascent {px}");
                assert_eq!(UI.descent(px), sf.descent(), "descent {px}");
                assert_eq!(UI.line_gap(px), sf.line_gap(), "line gap {px}");
                for &ch in chars.iter().chain(&['Ж', '\u{1F600}']) {
                    assert_eq!(UI.advance(ch, px), sf.h_advance(f.glyph_id(ch)), "{ch:?} at {px}");
                }
                let mut w = 0.0f32;
                let mut prev = None;
                for ch in all.chars() {
                    let g = f.glyph_id(ch);
                    if let Some(p) = prev {
                        w += sf.kern(p, g);
                    }
                    w += sf.h_advance(g);
                    prev = Some(g);
                }
                assert_eq!(UI.baked_width(&all, px), w, "width at {px}");
                let old: f32 = all.chars().map(|c| sf.h_advance(f.glyph_id(c))).sum();
                assert_eq!(UI.baked_width(&all, px), old, "old (kerning-free) width at {px}");
            }
        }
        assert!(baked::KERN.is_empty(), "the subset gained kerning: compare against kerned widths only");
    }

    /// The old `Fb::draw_text` glyph loop on ab_glyph, as a coverage grid.
    fn ab_glyph_grid(f: &FontArc, px: f32, s: &str, x: f32, y: f32, w: usize, h: usize) -> Vec<f32> {
        let sf = f.as_scaled(PxScale { x: px, y: px });
        let baseline = y + sf.ascent();
        let mut pen = x;
        let mut grid = vec![0.0f32; w * h];
        for ch in s.chars() {
            let glyph = ab_glyph::Glyph {
                id: f.glyph_id(ch),
                scale: PxScale { x: px, y: px },
                position: ab_glyph::point(pen, baseline),
            };
            if let Some(o) = sf.outline_glyph(glyph) {
                let b = o.px_bounds();
                o.draw(|gx, gy, c| {
                    let (sx, sy) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h && c > 0.0 {
                        grid[sy as usize * w + sx as usize] += c;
                    }
                });
            }
            pen += sf.h_advance(f.glyph_id(ch));
        }
        grid
    }

    fn baked_grid(px: f32, s: &str, x: f32, y: f32, w: usize, h: usize) -> Vec<f32> {
        let mut grid = vec![0.0f32; w * h];
        UI.baked_draw(s, px, x, y, |sx, sy, c| {
            if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h && c > 0.0 {
                grid[sy as usize * w + sx as usize] += c;
            }
        });
        grid
    }

    /// Coverage per pixel within 8/255 of ab_glyph's for every glyph of the
    /// subset and a sample string, at every size, scale and a few subpixel
    /// origins (in practice identical: same curves, same arithmetic).
    #[test]
    fn coverage_matches_ab_glyph() {
        let f = reference();
        let all: String = all_chars().into_iter().chain(['Ж']).collect();
        let samples = [all.as_str(), "Copy  Save  Ctrl+Shift+S  3 line  960 × 540 · 1 ⌘⇧⌥⏎"];
        let to8 = |c: f32| (c.min(1.0) * 255.0) as i32;
        let mut worst = 0;
        for size in SIZES {
            for s in SCALES {
                let px = size * s;
                for (ox, oy) in [(2.0, 3.0), (2.25, 3.5), (7.6, 1.3)] {
                    for text in samples {
                        let w = (UI.baked_width(text, px) + 2.0 * px) as usize;
                        let h = (px * 2.0) as usize;
                        let a = ab_glyph_grid(&f, px, text, ox, oy, w, h);
                        let b = baked_grid(px, text, ox, oy, w, h);
                        for (i, (&ca, &cb)) in a.iter().zip(&b).enumerate() {
                            let d = (to8(ca) - to8(cb)).abs();
                            worst = worst.max(d);
                            assert!(d <= 8, "{px}px at ({ox}, {oy}), pixel {i}: {ca} vs {cb}");
                        }
                    }
                }
            }
        }
        eprintln!("worst coverage difference: {worst}/255");
    }

    #[test]
    fn baked_tables_are_compact() {
        let bytes = std::mem::size_of_val(&baked::CMAP)
            + std::mem::size_of_val(&baked::GLYPHS)
            + std::mem::size_of_val(&baked::KERN)
            + baked::OUTLINES_Z.len();
        eprintln!("baked font tables: {bytes} bytes ({} compressed outline bytes)", baked::OUTLINES_Z.len());
        // Latin, Greek, Cyrillic and Vietnamese: about 42 KB.
        assert!(bytes < 48 * 1024, "{bytes} bytes");
    }

    /// The outlines inflate quickly enough to do at startup.
    #[test]
    fn outlines_inflate_fast() {
        let t = std::time::Instant::now();
        let (b, _) = crate::deflate::inflate(baked::OUTLINES_Z, 1 << 20).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        eprintln!("font outlines: {} -> {} bytes in {ms:.3} ms", baked::OUTLINES_Z.len(), b.len());
        assert_eq!(b.len(), baked::N_OPS_BYTES + 2 * baked::N_PTS);
        assert!(ms < 5.0 || cfg!(debug_assertions), "{ms} ms");
        assert_eq!(outlines().pts.len(), baked::N_PTS);
    }
}
