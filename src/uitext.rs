//! UI strings the baked Inter subset has no glyphs for (anything outside
//! ASCII and a few UI symbols: accented Latin, Greek, Cyrillic, CJK,
//! Arabic, Hebrew, Thai, Devanagari...) go through the OS text stack
//! ([`crate::text::UiFace`]: GDI + Uniscribe on Windows, with its font
//! linking, shaping and bidi; CoreText on macOS; on Linux the
//! fontconfig-chosen font for each character, through ab_glyph, with a
//! simplified bidi and no shaping: Arabic letters draw in their isolated
//! forms and Indic / Thai marks unordered). A whole string goes one way,
//! so it is shaped and ordered as a unit.
//!
//! The OS face is sized to the baked font's em and its baseline put on
//! the baked font's, so a fallback label lines up with its neighbours.
//! Widths and rasterised coverage are cached per (string, size,
//! direction): the UI redraws the same labels every frame, and a hit
//! costs a hash lookup and the blend.

use crate::fonts::UiFont;
use crate::objects::Pt;
use crate::text::{UiFace, UI_EM_RATIO};
use std::sync::{Mutex, OnceLock};

/// Whether `s` needs the OS text stack (a character the baked font lacks).
pub fn needed(font: &UiFont, s: &str) -> bool {
    !s.is_ascii() && s.chars().any(|c| !c.is_control() && !font.has_glyph(c))
}

/// One cached raster: coverage `a` (`w` wide, 0..=255) with its top-left
/// at (`x0`, `y0`) from the string's rounded top-left.
struct Run {
    x0: i32,
    y0: i32,
    w: usize,
    a: Vec<u8>,
}

/// One cached string at one size and direction: its width, and its
/// raster once drawn.
struct Entry {
    s: String,
    px: u32,
    rtl: bool,
    w: f32,
    run: Option<Run>,
    /// Last use (measured or drawn): eviction keeps the recent ones.
    used: u64,
}

impl Entry {
    /// What it holds, roughly (the byte bound).
    fn bytes(&self) -> usize {
        64 + self.s.len() + self.run.as_ref().map_or(0, |r| r.a.len())
    }
}

/// Least recently used out: a hash of (string, size, direction) → its
/// entry; a hash shared by two keys keeps the latest.
#[derive(Default)]
struct Cache {
    entries: Vec<Entry>,
    index: std::collections::HashMap<u64, usize>,
    bytes: usize,
    tick: u64,
    /// [`crate::text::font_generation`] of what is kept: when it moves (a
    /// font text lacked has been found), all of it is drawn again.
    fonts: u64,
}

fn key_hash(s: &str, px: f32, rtl: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (s, px.to_bits(), rtl).hash(&mut h);
    h.finish()
}

impl Cache {
    /// The entry for the key, marked used now.
    fn find(&mut self, s: &str, px: f32, rtl: bool) -> Option<&mut Entry> {
        self.tick += 1;
        let tick = self.tick;
        let &i = self.index.get(&key_hash(s, px, rtl))?;
        let e = &mut self.entries[i];
        if e.s != s || e.px != px.to_bits() || e.rtl != rtl {
            return None;
        }
        e.used = tick;
        Some(e)
    }

    fn insert(&mut self, mut e: Entry) {
        self.tick += 1;
        e.used = self.tick;
        let h = key_hash(&e.s, f32::from_bits(e.px), e.rtl);
        self.bytes += e.bytes();
        match self.index.get(&h) {
            Some(&i) => {
                self.bytes -= self.entries[i].bytes();
                self.entries[i] = e;
            }
            None => {
                self.index.insert(h, self.entries.len());
                self.entries.push(e);
            }
        }
        if self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            self.evict();
        }
    }

    /// Keep the more recently used half.
    fn evict(&mut self) {
        let mut v = std::mem::take(&mut self.entries);
        v.sort_unstable_by_key(|e| std::cmp::Reverse(e.used));
        let mut bytes = 0;
        let keep = v.iter().take(v.len() / 2).take_while(|e| {
            bytes += e.bytes();
            bytes <= MAX_BYTES / 2
        }).count();
        v.truncate(keep);
        self.index = v.iter().enumerate().map(|(i, e)| (key_hash(&e.s, f32::from_bits(e.px), e.rtl), i)).collect();
        self.bytes = v.iter().map(Entry::bytes).sum();
        self.entries = v;
    }

    /// The raster to set for an entry that has its width.
    fn set_run(&mut self, s: &str, px: f32, rtl: bool, r: Run) {
        let n = r.a.len();
        if let Some(e) = self.find(s, px, rtl) {
            e.run = Some(r);
            self.bytes += n;
            if self.bytes > MAX_BYTES {
                self.evict();
            }
        }
    }
}

/// Kept entries: at most this many, holding at most this many bytes
/// (strings and rasters).
const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: usize = 8 << 20;

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut c = C.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    let g = crate::text::font_generation();
    if c.fonts != g {
        c.entries.clear();
        c.index.clear();
        c.bytes = 0;
        c.fonts = g;
    }
    c
}

/// The OS font size whose em is the baked font's at `px`.
fn os_px(font: &UiFont, px: f32) -> f32 {
    font.em(px) * UI_EM_RATIO
}

fn face(rtl: bool) -> &'static UiFace {
    static F: [OnceLock<UiFace>; 2] = [OnceLock::new(), OnceLock::new()];
    F[rtl as usize].get_or_init(|| UiFace::new(rtl))
}

/// Advance width of `s` at the baked font's `px`.
pub fn width(font: &UiFont, s: &str, px: f32) -> f32 {
    let rtl = crate::i18n::is_rtl();
    if let Some(e) = cache().find(s, px, rtl) {
        return e.w;
    }
    let w = face(rtl).width(s, os_px(font, px));
    cache().insert(Entry { s: s.to_string(), px: px.to_bits(), rtl, w, run: None, used: 0 });
    w
}

/// Draw `s` with its top-left at (x, y) as the baked font would place it:
/// `f(x, y, coverage)` per pixel. Returns the advance width.
pub fn draw(font: &UiFont, s: &str, px: f32, x: f32, y: f32, f: &mut dyn FnMut(i32, i32, f32)) -> f32 {
    let rtl = crate::i18n::is_rtl();
    let w = width(font, s, px);
    let (ox, oy) = (x.round() as i32, y.round() as i32);
    {
        let mut c = cache();
        if let Some(r) = c.find(s, px, rtl).and_then(|e| e.run.as_ref()) {
            blit(r, ox, oy, f);
            return w;
        }
    }
    let r = raster(font, s, px, rtl, w);
    blit(&r, ox, oy, f);
    cache().set_run(s, px, rtl, r);
    w
}

fn blit(r: &Run, ox: i32, oy: i32, f: &mut dyn FnMut(i32, i32, f32)) {
    for (i, &v) in r.a.iter().enumerate() {
        if v > 0 {
            f(ox + r.x0 + (i % r.w) as i32, oy + r.y0 + (i / r.w) as i32, v as f32 / 255.0);
        }
    }
}

/// `s` rasterised by the OS face, its top-left at the origin, its
/// baseline on the baked font's.
fn raster(font: &UiFont, s: &str, px: f32, rtl: bool, w: f32) -> Run {
    let face = face(rtl);
    let opx = os_px(font, px);
    let (asc, desc) = face.metrics(opx);
    let top = (font.ascent(px) - asc).round();
    let pad = (opx / 3.0).ceil() as i32 + 3;
    let (x0, y0) = (-pad, top as i32 - pad);
    let (x1, y1) = (w.ceil() as i32 + pad, (top + asc + desc).ceil() as i32 + pad);
    let (cw, ch) = ((x1 - x0).max(1) as usize, (y1 - y0).max(1) as usize);
    let mut a = vec![0u8; cw * ch];
    face.draw(s, opx, Pt::new(0.0, top), (x0, y0, x1, y1), &mut |x, y, cov| {
        if x >= x0 && y >= y0 && x < x1 && y < y1 && cov > 0.0 {
            let i = (y - y0) as usize * cw + (x - x0) as usize;
            a[i] = a[i].max((cov.min(1.0) * 255.0).round() as u8);
        }
    });
    // Trim the empty margins.
    let rows: Vec<usize> = (0..ch).filter(|&r| a[r * cw..(r + 1) * cw].iter().any(|&v| v > 0)).collect();
    let cols: Vec<usize> = (0..cw).filter(|&c| (0..ch).any(|r| a[r * cw + c] > 0)).collect();
    let (Some(&r0), Some(&r1), Some(&c0), Some(&c1)) = (rows.first(), rows.last(), cols.first(), cols.last()) else {
        return Run { x0: 0, y0: 0, w: 1, a: Vec::new() };
    };
    let (tw, th) = (c1 - c0 + 1, r1 - r0 + 1);
    let mut t = Vec::with_capacity(tw * th);
    for r in r0..=r1 {
        t.extend_from_slice(&a[r * cw + c0..r * cw + c1 + 1]);
    }
    Run { x0: x0 + c0 as i32, y0: y0 + r0 as i32, w: tw, a: t }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::UI;

    fn lit(s: &str, px: f32) -> (usize, f32, (i32, i32, i32, i32)) {
        let mut n = 0;
        let mut b = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let w = draw(&UI, s, px, 10.0, 10.0, &mut |x, y, c| {
            if c > 0.3 {
                n += 1;
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        });
        (n, w, b)
    }

    #[test]
    fn only_strings_with_missing_glyphs_fall_back() {
        assert!(!needed(&UI, "Save as\u{2026} \u{00d7}"));
        assert!(needed(&UI, "Größe"));
        assert!(needed(&UI, "Сохранить"));
        assert!(needed(&UI, "保存"));
    }

    /// Every script draws something on the OS stack, about as tall as
    /// the baked font's text and on its baseline, and a redraw is a hit.
    #[test]
    fn scripts_render_and_cache() {
        if cfg!(target_os = "linux") && std::process::Command::new("fc-match").arg("-v").output().is_err() {
            eprintln!("skipping: no fontconfig");
            return;
        }
        let (_, _, base) = {
            let mut b = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            UI.draw("Hxg", 20.0, 10.0, 10.0, |x, y, c| {
                if c > 0.3 {
                    b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
                }
            });
            (0, 0.0, b)
        };
        for s in ["Größe ändern", "Сохранить", "Αποθήκευση", "שמור", "حفظ الصورة", "保存截图", "スクリーンショット", "저장", "บันทึก", "सहेजें"] {
            // (Linux finds the fonts in the background.)
            crate::text::prefetch_fonts(s);
            crate::text::wait_fonts();
            let (n, w, b) = lit(s, 20.0);
            if n == 0 && cfg!(target_os = "linux") {
                eprintln!("no font for {s} here");
                continue;
            }
            assert!(n > 20 && w > 10.0, "{s}: {n} px, width {w}");
            // Within a few px of the baked text's vertical extent.
            assert!(b.1 >= base.1 - 6 && b.3 <= base.3 + 7, "{s}: rows {}..{} vs {}..{}", b.1, b.3, base.1, base.3);
            assert!(cache().find(s, 20.0, false).is_some_and(|e| e.run.is_some()), "{s}: cached");
            let again = lit(s, 20.0);
            assert_eq!(again.0, n, "{s}: same pixels from the cache");
        }
    }

    /// Arabic joins when drawn whole: a word is narrower than its letters
    /// drawn one by one (isolated forms). Windows and macOS shape; Linux
    /// has no shaper (see the module notes).
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn arabic_joins_when_drawn_whole() {
        let word = "\u{628}\u{628}\u{628}\u{628}";
        let one = width(&UI, "\u{628}", 20.0);
        let w = width(&UI, word, 20.0);
        assert!(w < one * 4.0 - 2.0, "joined {w} vs isolated {}", one * 4.0);
    }

    /// Eviction keeps what is in use: strings measured every "frame" stay
    /// while more than the cache holds stream past; and the bytes stay
    /// bounded.
    #[test]
    fn cache_keeps_recently_used() {
        let mut c = Cache::default();
        let e = |s: String| Entry { s, px: 1.0f32.to_bits(), rtl: false, w: 1.0, run: None, used: 0 };
        for i in 0..20 {
            c.insert(e(format!("hot {i}")));
        }
        for n in 0..3 * MAX_ENTRIES {
            for i in 0..20 {
                assert!(c.find(&format!("hot {i}"), 1.0, false).is_some(), "hot {i} kept after {n}");
            }
            c.insert(e(format!("cold {n}")));
            assert!(c.entries.len() <= MAX_ENTRIES && c.bytes <= MAX_BYTES);
        }
        let big = Entry { run: Some(Run { x0: 0, y0: 0, w: 1, a: vec![0; MAX_BYTES / 3] }), ..e("big".into()) };
        c.insert(big);
        c.insert(Entry { run: Some(Run { x0: 0, y0: 0, w: 1, a: vec![0; MAX_BYTES / 3] }), ..e("big2".into()) });
        c.insert(Entry { run: Some(Run { x0: 0, y0: 0, w: 1, a: vec![0; MAX_BYTES / 3] }), ..e("big3".into()) });
        assert!(c.bytes <= MAX_BYTES, "{}", c.bytes);
    }

    /// A preview of the scripts beside baked text (RUSTSHOT_PREVIEW_DIR).
    #[test]
    fn preview_scripts() {
        use crate::uifb::{Fb, C4};
        let (w, h) = (640u32, 300u32);
        let mut img = crate::pixbuf::PixBuf::from_pixel(w, h, [32, 33, 38, 255]);
        let mut fb = Fb::new(img.as_raw_mut(), w as usize);
        let lines = [
            "Save as  Größe ändern  Ελληνικά",
            "Save as  Сохранить как  Українська",
            "Save as  保存截图  スクリーンショット  저장",
            "Save as  حفظ الصورة  שמור בשם",
            "Save as  บันทึกภาพ  सहेजें",
        ];
        for (i, l) in lines.iter().enumerate() {
            let y = 14.0 + i as f32 * 56.0;
            fb.fill_rect(10, y as i32 + 15, 620, 1, C4::rgb(90, 60, 60)); // the baked baseline
            fb.draw_text(&UI, 19.0, "Save as", 14.0, y, C4::rgb(236, 238, 243));
            let x = 14.0 + UI.width("Save as  ", 19.0);
            fb.draw_text(&UI, 19.0, &l["Save as  ".len()..], x, y, C4::rgb(236, 238, 243));
            fb.draw_text(&UI, 12.5, l, 14.0, y + 28.0, C4::rgb(180, 186, 200));
        }
        crate::ui::preview::save("uitext_scripts.png", &img);
    }

    #[test]
    fn widths_follow_length() {
        let a = width(&UI, "Größe", 14.0);
        let b = width(&UI, "Größe Größe", 14.0);
        assert!(a > 10.0 && b > a * 1.7, "{a} {b}");
    }

    #[test]
    fn visual_order_reverses_rtl_runs() {
        use crate::text::visual_order;
        let s = |v: Vec<char>| v.into_iter().collect::<String>();
        assert_eq!(s(visual_order("abc", false)), "abc");
        assert_eq!(s(visual_order("אבג", false)), "גבא");
        assert_eq!(s(visual_order("אב 12 גד", false)), "דג 12 בא");
        assert_eq!(s(visual_order("abc", true)), "abc");
        assert_eq!(s(visual_order("שמור (file.png)", true)), "(file.png) רומש");
        assert_eq!(s(visual_order("[אב]", true)), "[בא]");
    }
}
