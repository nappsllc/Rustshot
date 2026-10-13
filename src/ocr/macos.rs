//! macOS: Vision's `VNRecognizeTextRequest` (accurate level, language
//! correction on) over a `CGImage` of the area, run synchronously with
//! `-[VNImageRequestHandler performRequests:error:]` on the OCR worker.
//! Each observation is one line; each of its words gets its own box from
//! `-[VNRecognizedText boundingBoxForRange:error:]` (normalized, origin
//! at the bottom left), the line's box where that fails.

use super::{Line, OcrError, Word};
use crate::macos_objc::{Id, Pool, array, dict, error_text, has_method, items, load, msg_big0, string_of};
use crate::pixbuf::PixBuf;
use crate::wind::{msg0, msg1, msg2, objc_cls, objc_sel};
use core::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDataProviderCreateWithData(
        info: *mut c_void,
        data: *const c_void,
        size: usize,
        release: unsafe extern "C" fn(*mut c_void, *const c_void, usize),
    ) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
        provider: *mut c_void,
        decode: *const f64,
        interpolate: bool,
        intent: i32,
    ) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// The pixels handed to CoreGraphics: freed when it lets go of them.
unsafe extern "C" fn release_pixels(info: *mut c_void, _data: *const c_void, _size: usize) {
    unsafe { drop(Box::from_raw(info as *mut Vec<u8>)) }
}

/// An opaque RGBA `CGImage` of `img` (+1; release with `CGImageRelease`).
unsafe fn cg_image(img: &PixBuf) -> *mut c_void {
    let (w, h) = img.dimensions();
    let mut rgba = img.as_raw().clone();
    for px in rgba.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    let pixels = Box::new(rgba);
    let (data, len) = (pixels.as_ptr(), pixels.len());
    let info = Box::into_raw(pixels) as *mut c_void;
    unsafe {
        let provider = CGDataProviderCreateWithData(info, data.cast(), len, release_pixels);
        if provider.is_null() {
            release_pixels(info, data.cast(), len);
            return core::ptr::null_mut();
        }
        let space = CGColorSpaceCreateDeviceRGB();
        // premultipliedLast | byteOrder32Big: RGBA bytes (alpha is 255).
        let image = CGImageCreate(w as usize, h as usize, 8, 32, w as usize * 4, space, 0x3001, provider, core::ptr::null(), true, 0);
        CGDataProviderRelease(provider);
        if !space.is_null() {
            CGColorSpaceRelease(space);
        }
        image
    }
}

pub fn recognize_lines(img: &PixBuf) -> Result<Vec<Line>, OcrError> {
    let (img, k) = super::prepare(img, None);
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    let _pool = Pool::new();
    unsafe {
        load(c"/System/Library/Frameworks/Vision.framework/Vision");
        let req_cls = objc_cls(c"VNRecognizeTextRequest");
        let handler_cls = objc_cls(c"VNImageRequestHandler");
        if req_cls.is_null() || handler_cls.is_null() {
            return Err(OcrError::Failed("Vision text recognition is not available".into()));
        }
        let req = crate::macos_objc::new_obj(req_cls).ok_or_else(|| OcrError::Failed("no Vision request".into()))?;
        let _: () = msg1(req.ptr(), objc_sel(c"setRecognitionLevel:"), 0isize); // accurate
        let _: () = msg1(req.ptr(), objc_sel(c"setUsesLanguageCorrection:"), 1i8);
        if has_method(req_cls, c"setAutomaticallyDetectsLanguage:") {
            let _: () = msg1(req.ptr(), objc_sel(c"setAutomaticallyDetectsLanguage:"), 1i8);
        }
        let image = cg_image(&img);
        if image.is_null() {
            return Err(OcrError::Failed("cannot make an image".into()));
        }
        let alloc: *mut c_void = msg0(handler_cls, objc_sel(c"alloc"));
        // A failed `init…` releases the allocation itself (Cocoa's
        // initializer contract), so a nil result leaks nothing; releasing
        // `alloc` here would be a double release.
        let handler = if alloc.is_null() { None } else { Id::owned(msg2(alloc, objc_sel(c"initWithCGImage:options:"), image, dict(&[]))) };
        CGImageRelease(image);
        let handler = handler.ok_or_else(|| OcrError::Failed("no Vision handler".into()))?;
        let mut err: *mut c_void = core::ptr::null_mut();
        let ok: i8 = msg2(handler.ptr(), objc_sel(c"performRequests:error:"), array(&[req.ptr()]), &mut err as *mut *mut c_void);
        if ok == 0 {
            return Err(OcrError::Failed(error_text(err)));
        }
        let results: *mut c_void = msg0(req.ptr(), objc_sel(c"results"));
        let s = k as f64;
        let mut lines = Vec::new();
        for obs in items(results) {
            let cands: *mut c_void = msg1(obs, objc_sel(c"topCandidates:"), 1usize);
            let Some(best) = items(cands).first().copied() else { continue };
            let text = string_of(msg0(best, objc_sel(c"string")));
            let line_box: CGRect = msg_big0(obs, objc_sel(c"boundingBox"));
            let to_word = |text: String, r: CGRect| {
                let top = (1.0 - (r.y + r.h)) * ih / s;
                Word { text, x: (r.x * iw / s) as f32, y: top as f32, w: (r.w * iw / s) as f32, h: (r.h * ih / s) as f32 }
            };
            let mut words = Vec::new();
            for (word, range) in word_ranges(&text) {
                let mut err: *mut c_void = core::ptr::null_mut();
                let rect_obs: *mut c_void = msg2(best, objc_sel(c"boundingBoxForRange:error:"), range, &mut err as *mut *mut c_void);
                let r = if rect_obs.is_null() { line_box } else { msg_big0::<CGRect>(rect_obs, objc_sel(c"boundingBox")) };
                words.push(to_word(word, r));
            }
            lines.push(Line { words });
        }
        Ok(lines)
    }
}

/// An `NSRange` (UTF-16 units).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NsRange {
    loc: usize,
    len: usize,
}

/// The words of `s` (split at whitespace) with their UTF-16 ranges.
pub(super) fn word_ranges(s: &str) -> Vec<(String, NsRange)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut cur: Option<(String, usize, usize)> = None;
    for c in s.chars() {
        let n = c.len_utf16();
        if c.is_whitespace() {
            if let Some((w, loc, len)) = cur.take() {
                out.push((w, NsRange { loc, len }));
            }
        } else {
            let e = cur.get_or_insert_with(|| (String::new(), at, 0));
            e.0.push(c);
            e.2 += n;
        }
        at += n;
    }
    if let Some((w, loc, len)) = cur {
        out.push((w, NsRange { loc, len }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_get_utf16_ranges() {
        let r = word_ranges("Hi  \u{1F600}x a");
        let got: Vec<(&str, usize, usize)> = r.iter().map(|(w, n)| (w.as_str(), n.loc, n.len)).collect();
        assert_eq!(got, [("Hi", 0, 2), ("\u{1F600}x", 4, 3), ("a", 8, 1)]);
        assert!(word_ranges("   ").is_empty());
    }
}
