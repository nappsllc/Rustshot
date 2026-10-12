//! Windows annotation text: GDI with "Segoe UI" (Georgia for serif,
//! Consolas for mono). `DrawTextW` brings Uniscribe shaping (Arabic,
//! Indic) and font linking (CJK) for free. Each line is drawn
//! white-on-black into a 32-bit DIB with grayscale anti-aliasing; the gray
//! level is the coverage. Bold, italic, underline and strikeout are the
//! font's own (`CreateFontW`).

use crate::objects::Pt;
use crate::text::Family;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, GdiFlush,
    GetTextMetricsW, SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS,
    DRAW_TEXT_FORMAT, DT_CALCRECT, DT_NOCLIP, DT_NOPREFIX, DT_SINGLELINE, FW_NORMAL, HDC, HFONT,
    HGDIOBJ, OUT_TT_PRECIS, TEXTMETRICW, TRANSPARENT,
};

/// GDI draws underline and strikeout itself.
pub const NATIVE_DECO: bool = true;

/// The face name of a family.
fn face_name(f: Family) -> PCWSTR {
    match f {
        Family::Sans => w!("Segoe UI"),
        Family::Serif => w!("Georgia"),
        Family::Mono => w!("Consolas"),
    }
}

/// The font dropdown's name for `f`.
pub fn family_label(f: Family) -> &'static str {
    match f {
        Family::Sans => "Segoe UI",
        Family::Serif => "Georgia",
        Family::Mono => "Consolas",
    }
}

/// The annotation face: a family at a GDI weight (400 regular, 600
/// semibold, 700 bold), italic, underlined, struck out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Face {
    weight: i32,
    family: Family,
    italic: bool,
    underline: bool,
    strike: bool,
}

impl Face {
    /// The style bits after the weight (the DC cache's key).
    fn bits(&self) -> i32 {
        self.family as i32 | (self.italic as i32) << 2 | (self.underline as i32) << 3 | (self.strike as i32) << 4
    }
}

/// A memory DC with the face selected at a pixel size; freed on drop.
struct FontDc {
    dc: HDC,
    font: HFONT,
    old: HGDIOBJ,
}

impl FontDc {
    fn new(px: f32, face: &Face) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            // Positive height: the cell (ascent + descent) height, which is
            // what `px` meant for the old ab_glyph renderer.
            let font = CreateFontW(
                px.round().max(1.0) as i32,
                0,
                0,
                0,
                face.weight,
                face.italic as u32,
                face.underline as u32,
                face.strike as u32,
                DEFAULT_CHARSET,
                OUT_TT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                DEFAULT_PITCH.0 as u32,
                face_name(face.family),
            );
            if font.is_invalid() {
                let _ = DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, font.into());
            Some(FontDc { dc, font, old })
        }
    }

    fn text(&self, line: &str, r: &mut RECT, flags: DRAW_TEXT_FORMAT) {
        let mut wide: Vec<u16> = line.encode_utf16().collect();
        unsafe {
            DrawTextW(self.dc, &mut wide, r, flags | DT_SINGLELINE | DT_NOPREFIX | DT_NOCLIP);
        }
    }
}

/// The DC cache key: pixel size, weight, style bits.
type DcKey = (i32, i32, i32);

/// Run `f` with a DC holding `face` at `px`: the last few are kept
/// (creating a font and a DC per measure cost tens of microseconds, and a
/// wrap measures many times). A nested call gets a fresh one.
fn with_dc<R>(px: f32, face: &Face, f: impl FnOnce(&FontDc) -> R) -> Option<R> {
    thread_local! {
        static DCS: std::cell::RefCell<Vec<(DcKey, FontDc)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let key = (px.round().max(1.0) as i32, face.weight, face.bits());
    DCS.with(|c| match c.try_borrow_mut() {
        Ok(mut v) => {
            if let Some(i) = v.iter().position(|(k, _)| *k == key) {
                let e = v.remove(i);
                v.push(e);
            } else {
                let fd = FontDc::new(px, face)?;
                if v.len() >= 6 {
                    v.remove(0);
                }
                v.push((key, fd));
            }
            v.last().map(|(_, fd)| f(fd))
        }
        Err(_) => FontDc::new(px, face).map(|fd| f(&fd)),
    })
}

/// A strong right-to-left character (Hebrew, Arabic and their
/// presentation forms, Syriac, Thaana, N'Ko...).
fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

impl Drop for FontDc {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.font.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

impl Face {
    pub fn load() -> Option<Self> {
        let f = Face::new(Family::Sans, FW_NORMAL.0 as u16, false, false, false);
        FontDc::new(16.0, &f).map(|_| f)
    }

    /// `family` at GDI `weight`, italic, underlined, struck out.
    pub fn new(family: Family, weight: u16, italic: bool, underline: bool, strike: bool) -> Face {
        Face { weight: weight as i32, family, italic, underline, strike }
    }

    pub fn metrics(&self, px: f32) -> (f32, f32, f32) {
        let tm = with_dc(px, self, |fd| {
            let mut tm = TEXTMETRICW::default();
            unsafe { GetTextMetricsW(fd.dc, &mut tm) }.as_bool().then_some(tm)
        });
        match tm.flatten() {
            Some(tm) => (tm.tmAscent as f32, tm.tmDescent as f32, tm.tmExternalLeading as f32),
            None => (px * 0.8, px * 0.2, 0.0),
        }
    }

    /// The caret before byte `idx` (from the line's left) as Uniscribe
    /// lays the line out (the leading edge of the character there, or the
    /// trailing edge of the last one at the end); None when it cannot.
    pub fn caret_x(&self, line: &str, idx: usize, px: f32) -> Option<f32> {
        use windows::Win32::Globalization::{ScriptStringAnalyse, ScriptStringCPtoX, ScriptStringFree};
        const SSA_FALLBACK: u32 = 0x20;
        const SSA_GLYPHS: u32 = 0x80;
        const SSA_LINK: u32 = 0x1000;
        if line.is_empty() || idx == 0 && !line.chars().any(is_rtl) {
            return Some(0.0);
        }
        let wide: Vec<u16> = line.encode_utf16().collect();
        let cp = line.get(..idx)?.encode_utf16().count();
        let n = wide.len() as i32;
        with_dc(px, self, |fd| unsafe {
            let mut ssa: *mut core::ffi::c_void = std::ptr::null_mut();
            ScriptStringAnalyse(
                fd.dc,
                wide.as_ptr() as *const _,
                n,
                n * 3 / 2 + 16,
                -1, // Unicode
                SSA_GLYPHS | SSA_FALLBACK | SSA_LINK,
                0,
                None,
                None,
                None,
                None,
                std::ptr::null(),
                &mut ssa,
            )
            .ok()?;
            let x = if cp < wide.len() { ScriptStringCPtoX(ssa, cp as i32, false) } else { ScriptStringCPtoX(ssa, n - 1, true) };
            let _ = ScriptStringFree(&mut ssa);
            x.ok().map(|x| x as f32)
        })
        .flatten()
    }

    pub fn width(&self, line: &str, px: f32) -> f32 {
        with_dc(px, self, |fd| {
            let mut r = RECT::default();
            fd.text(line, &mut r, DT_CALCRECT);
            (r.right - r.left).max(0) as f32
        })
        .unwrap_or(0.0)
    }

    pub fn draw(
        &self,
        line: &str,
        px: f32,
        top: Pt,
        _ascent: f32,
        clip: (i32, i32, i32, i32),
        f: &mut dyn FnMut(i32, i32, f32),
    ) {
        with_dc(px, self, |fd| Self::draw_with(fd, line, px, top, clip, f));
    }

    fn draw_with(fd: &FontDc, line: &str, px: f32, top: Pt, clip: (i32, i32, i32, i32), f: &mut dyn FnMut(i32, i32, f32)) {
        let mut ext = RECT::default();
        fd.text(line, &mut ext, DT_CALCRECT);
        // Overhang (italic-ish fallback glyphs, combining marks) beyond the
        // logical box.
        let pad = (px / 3.0).ceil() as i32 + 2;
        let (ox, oy) = (top.x.round() as i32, top.y.round() as i32);
        let x0 = (ox - pad).max(clip.0);
        let y0 = (oy - pad).max(clip.1);
        let x1 = (ox + ext.right + pad).min(clip.2);
        let y1 = (oy + ext.bottom + pad).min(clip.3);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        unsafe {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // top-down
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(fd.dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                return;
            };
            if bits.is_null() {
                let _ = DeleteObject(bmp.into());
                return;
            }
            let old = SelectObject(fd.dc, bmp.into());
            SetBkMode(fd.dc, TRANSPARENT);
            SetTextColor(fd.dc, COLORREF(0x00FF_FFFF));
            let mut r = RECT { left: ox - x0, top: oy - y0, right: ox - x0 + ext.right, bottom: oy - y0 + ext.bottom };
            fd.text(line, &mut r, DRAW_TEXT_FORMAT(0));
            let _ = GdiFlush();
            // A fresh DIB section is zero-filled: black, i.e. no coverage.
            let px_data = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
            for (i, p) in px_data.as_chunks::<4>().0.iter().enumerate() {
                let v = p[0].max(p[1]).max(p[2]);
                if v > 0 {
                    let i = i as i32;
                    f(x0 + i % w, y0 + i / w, v as f32 / 255.0);
                }
            }
            SelectObject(fd.dc, old);
            let _ = DeleteObject(bmp.into());
        }
    }
}
