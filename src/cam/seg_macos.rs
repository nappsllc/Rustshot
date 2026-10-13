//! macOS 12 and later: the person mask from Vision's
//! `VNGeneratePersonSegmentationRequest` (balanced quality, one byte a
//! pixel), the same request reused from one picture to the next (Vision
//! keeps it steady across frames). It takes the camera thread's small
//! RGBA picture ([`SegInput`]) as a `CGImage`; the mask's square is cut
//! out as for the embedded model ([`super::seg::crop_square`]). Older
//! systems have no such request: the embedded model is used if the build
//! has one.

use super::seg::{crop_square, SegInput, Segment, SqMask};
use crate::macos_objc::{array, dict, items, load, new_obj, Id, Pool};
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

/// `VNGeneratePersonSegmentationRequestQualityLevelBalanced`.
const BALANCED: usize = 1;
/// `kCVPixelFormatType_OneComponent8` ('L008').
const ONE_COMPONENT_8: u32 = 0x4C30_3038;

const VISION: &core::ffi::CStr = c"/System/Library/Frameworks/Vision.framework/Vision";

/// Whether this macOS has Vision's person segmentation (12+).
pub fn available() -> bool {
    static HAVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HAVE.get_or_init(|| {
        load(VISION);
        !objc_cls(c"VNGeneratePersonSegmentationRequest").is_null() && !objc_cls(c"VNImageRequestHandler").is_null()
    })
}

/// The pixels handed to CoreGraphics: freed when it lets go of them.
unsafe extern "C" fn release_pixels(info: *mut c_void, _data: *const c_void, _size: usize) {
    unsafe { drop(Box::from_raw(info as *mut Vec<u8>)) }
}

/// An RGBA `w`x`h` `CGImage` of `rgba` (+1; release with `CGImageRelease`).
unsafe fn cg_image(rgba: &[u8], w: usize, h: usize) -> *mut c_void {
    let pixels = Box::new(rgba[..w * h * 4].to_vec());
    let (data, len) = (pixels.as_ptr(), pixels.len());
    let info = Box::into_raw(pixels) as *mut c_void;
    unsafe {
        let provider = CGDataProviderCreateWithData(info, data.cast(), len, release_pixels);
        if provider.is_null() {
            release_pixels(info, data.cast(), len);
            return core::ptr::null_mut();
        }
        let space = CGColorSpaceCreateDeviceRGB();
        // As the OCR's: RGBA bytes, alpha last (255).
        let image = CGImageCreate(w, h, 8, 32, w * 4, space, 0x3001, provider, core::ptr::null(), true, 0);
        CGDataProviderRelease(provider);
        if !space.is_null() {
            CGColorSpaceRelease(space);
        }
        image
    }
}

/// Vision's person segmentation, one request kept for the stream.
pub struct Vision {
    req: Id,
    mask: Vec<f32>,
}

// The request is used from the segmentation thread alone.
unsafe impl Send for Vision {}

impl Vision {
    pub fn new() -> Option<Vision> {
        if !available() {
            return None;
        }
        let _pool = Pool::new();
        unsafe {
            let req = new_obj(objc_cls(c"VNGeneratePersonSegmentationRequest"))?;
            let _: () = msg1(req.ptr(), objc_sel(c"setQualityLevel:"), BALANCED);
            let _: () = msg1(req.ptr(), objc_sel(c"setOutputPixelFormat:"), ONE_COMPONENT_8);
            Some(Vision { req, mask: Vec::new() })
        }
    }
}

impl Segment for Vision {
    fn input_size(&self) -> (usize, usize) {
        (256, 144)
    }

    fn segment(&mut self, input: &SegInput, out: &mut SqMask) -> bool {
        let Some(sq) = input.sq else { return false };
        let (iw, ih) = (input.w, input.h);
        if iw == 0 || ih == 0 || input.rgba.len() < iw * ih * 4 {
            return false;
        }
        let Ok(m) = crate::rec::macos::media::media() else { return false };
        let _pool = Pool::new();
        unsafe {
            let image = cg_image(&input.rgba, iw, ih);
            if image.is_null() {
                return false;
            }
            let cls = objc_cls(c"VNImageRequestHandler");
            let alloc: *mut c_void = msg0(cls, objc_sel(c"alloc"));
            // A failed init releases the allocation itself.
            let handler = if alloc.is_null() { None } else { Id::owned(msg2(alloc, objc_sel(c"initWithCGImage:options:"), image, dict(&[]))) };
            CGImageRelease(image);
            let Some(handler) = handler else { return false };
            let mut err: *mut c_void = core::ptr::null_mut();
            let ok: i8 = msg2(handler.ptr(), objc_sel(c"performRequests:error:"), array(&[self.req.ptr()]), &mut err as *mut *mut c_void);
            if ok == 0 {
                return false;
            }
            let results: *mut c_void = msg0(self.req.ptr(), objc_sel(c"results"));
            let Some(&obs) = items(results).first() else { return false };
            let pb: *mut c_void = msg0(obs, objc_sel(c"pixelBuffer"));
            if pb.is_null() {
                return false;
            }
            const READ_ONLY: u64 = 1;
            if (m.cv.CVPixelBufferLockBaseAddress)(pb, READ_ONLY) != 0 {
                return false;
            }
            let (w, h) = ((m.cv.CVPixelBufferGetWidth)(pb), (m.cv.CVPixelBufferGetHeight)(pb));
            let (base, stride) = ((m.cv.CVPixelBufferGetBaseAddress)(pb), (m.cv.CVPixelBufferGetBytesPerRow)(pb));
            let ok = !base.is_null() && w > 0 && h > 0 && stride >= w;
            if ok {
                let bytes = std::slice::from_raw_parts(base, stride * (h - 1) + w);
                self.mask.clear();
                for y in 0..h {
                    self.mask.extend(bytes[y * stride..][..w].iter().map(|&b| b as f32 * (1.0 / 255.0)));
                }
            }
            (m.cv.CVPixelBufferUnlockBaseAddress)(pb, READ_ONLY);
            if !ok {
                return false;
            }
            crop_square(&self.mask, (w, h), (iw, ih), sq, out);
            true
        }
    }
}
