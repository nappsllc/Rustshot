//! Windows: `Windows.Media.Ocr` through hand-rolled WinRT vtables (the
//! `windows` crate supplies only `RoInitialize`, `RoGetActivationFactory`,
//! `HSTRING` and the GUIDs' type).
//!
//! `OcrEngine::TryCreateFromUserProfileLanguages()` reads a
//! `SoftwareBitmap` (BGRA8) filled through `LockBuffer` and
//! `IMemoryBufferByteAccess`; `RecognizeAsync` is waited on by polling
//! `IAsyncInfo::Status` (no completion handler object to implement).
//! The calls take the vtable slot of each method: IUnknown 0-2,
//! IInspectable 3-5, then the interface's own methods from 6.

use super::{Line, OcrError, Word};
use crate::pixbuf::PixBuf;
use core::ffi::c_void;
use std::time::{Duration, Instant};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoGetActivationFactory, RoInitialize, RoUninitialize};
use windows::core::{GUID, HRESULT, HSTRING, IInspectable, Interface};

const IID_OCR_ENGINE_STATICS: GUID = GUID::from_u128(0x5bffa85a_3384_3540_9940_699120d428a8);
const IID_SOFTWARE_BITMAP_FACTORY: GUID = GUID::from_u128(0xc99feb69_2d62_4d47_a6b3_4fdb6a07fdf8);
const IID_MEMORY_BUFFER: GUID = GUID::from_u128(0xfbc4dd2a_245b_11e4_af98_689423260cf8);
const IID_MEMORY_BUFFER_BYTE_ACCESS: GUID = GUID::from_u128(0x5b0d3235_4dba_4d44_865e_8f1d0e4fd04d);
const IID_CLOSABLE: GUID = GUID::from_u128(0x30d5a829_7fa4_4026_83bb_d75bae4ea99e);
const IID_ASYNC_INFO: GUID = GUID::from_u128(0x00000036_0000_0000_c000_000000000046);

/// `BitmapPixelFormat::Bgra8`, `BitmapBufferAccessMode::Write`.
const BGRA8: i32 = 87;
const ACCESS_WRITE: i32 = 2;
/// `AsyncStatus`.
const STARTED: i32 = 0;
const COMPLETED: i32 = 1;
const CANCELED: i32 = 2;

use super::TIMEOUT;

/// A `Windows.Foundation.Rect`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// `BitmapPlaneDescription`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct Plane {
    start: i32,
    width: i32,
    height: i32,
    stride: i32,
}

/// An owned COM interface pointer, released on drop.
struct Com(*mut c_void);

impl Com {
    fn new(p: *mut c_void) -> Option<Com> {
        (!p.is_null()).then(|| Com(p))
    }

    /// Method `slot` of the vtable, as `F`.
    unsafe fn slot<F: Copy>(&self, slot: usize) -> F {
        debug_assert_eq!(size_of::<F>(), size_of::<usize>());
        unsafe {
            let vt = *(self.0 as *const *const usize);
            core::mem::transmute_copy(&*vt.add(slot))
        }
    }

    fn query(&self, iid: &GUID) -> Result<Com, OcrError> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT = self.slot(0);
            f(self.0, iid, &mut out)
        };
        check(hr, "QueryInterface")?;
        Com::new(out).ok_or_else(|| fail("QueryInterface returned nothing"))
    }

    /// A getter returning an interface (`slot`, no arguments).
    fn get_obj(&self, slot: usize, what: &str) -> Result<Option<Com>, OcrError> {
        let mut out = core::ptr::null_mut();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT = self.slot(slot);
            f(self.0, &mut out)
        };
        check(hr, what)?;
        Ok(Com::new(out))
    }

    /// A getter returning a value (`slot`, no arguments).
    fn get<T: Copy + Default>(&self, slot: usize, what: &str) -> Result<T, OcrError> {
        let mut out = T::default();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut T) -> HRESULT = self.slot(slot);
            f(self.0, &mut out)
        };
        check(hr, what)?;
        Ok(out)
    }

    /// A getter returning an `HSTRING`.
    fn get_text(&self, slot: usize) -> Result<String, OcrError> {
        let mut out = HSTRING::new();
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut HSTRING) -> HRESULT = self.slot(slot);
            f(self.0, &mut out)
        };
        check(hr, "Text")?;
        Ok(out.to_string_lossy())
    }

    /// `IClosable::Close` (where the object has it).
    fn close(&self) {
        if let Ok(c) = self.query(&IID_CLOSABLE) {
            unsafe {
                let f: unsafe extern "system" fn(*mut c_void) -> HRESULT = c.slot(6);
                debug_hr(f(c.0), "Close");
            }
        }
    }

    /// The items of an `IVectorView<T>` (T an interface): `get_Size` is
    /// slot 7, `GetAt` slot 6.
    fn items(&self) -> Result<Vec<Com>, OcrError> {
        let n: u32 = self.get(7, "Size")?;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let mut p = core::ptr::null_mut();
            let hr = unsafe {
                let f: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT = self.slot(6);
                f(self.0, i, &mut p)
            };
            check(hr, "GetAt")?;
            out.extend(Com::new(p));
        }
        Ok(out)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void) -> u32 = self.slot(2);
            f(self.0);
        }
    }
}

/// A failure we go on after (Close, Cancel): only told in debug builds.
fn debug_hr(hr: HRESULT, what: &str) {
    if cfg!(debug_assertions) && hr.is_err() {
        eprintln!("rustshot: ocr: {what} failed: {}", hr.message().trim());
    }
}

fn fail(s: impl Into<String>) -> OcrError {
    OcrError::Failed(s.into())
}

fn check(hr: HRESULT, what: &str) -> Result<(), OcrError> {
    if hr.is_ok() { Ok(()) } else { Err(fail(format!("{what}: {}", hr.message().trim()))) }
}

/// The activation factory of runtime class `class`, as interface `iid`.
fn factory(class: &str, iid: &GUID) -> Result<Com, OcrError> {
    let f: IInspectable = unsafe { RoGetActivationFactory(&HSTRING::from(class)) }.map_err(|e| fail(format!("{class}: {}", e.message().trim())))?;
    // Borrowed from `f` (which releases its own reference on drop).
    let borrowed = core::mem::ManuallyDrop::new(Com(f.as_raw()));
    borrowed.query(iid)
}

/// The WinRT apartment of this (worker) thread, left on drop.
struct Apartment(bool);

impl Apartment {
    fn enter() -> Apartment {
        Apartment(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { RoUninitialize() };
        }
    }
}

/// RGBA rows into a BGRA plane with `stride` bytes per row.
pub(super) fn fill_bgra(dst: &mut [u8], stride: usize, img: &PixBuf) {
    let w = img.width() as usize;
    if w == 0 {
        return;
    }
    for (y, row) in img.as_raw().chunks_exact(w * 4).enumerate() {
        let Some(out) = dst.get_mut(y * stride..y * stride + w * 4) else { break };
        for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(row.as_chunks::<4>().0) {
            *o = [s[2], s[1], s[0], 255];
        }
    }
}

/// A BGRA8 `SoftwareBitmap` holding `img`.
fn software_bitmap(img: &PixBuf) -> Result<Com, OcrError> {
    if img.width() == 0 || img.height() == 0 {
        return Err(fail("empty area"));
    }
    let fac = factory("Windows.Graphics.Imaging.SoftwareBitmap", &IID_SOFTWARE_BITMAP_FACTORY)?;
    let mut bmp = core::ptr::null_mut();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, i32, i32, i32, *mut *mut c_void) -> HRESULT = fac.slot(6);
        f(fac.0, BGRA8, img.width() as i32, img.height() as i32, &mut bmp)
    };
    check(hr, "SoftwareBitmap")?;
    let bmp = Com::new(bmp).ok_or_else(|| fail("no SoftwareBitmap"))?;
    // ISoftwareBitmap::LockBuffer (slot 15).
    let mut buf = core::ptr::null_mut();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> HRESULT = bmp.slot(15);
        f(bmp.0, ACCESS_WRITE, &mut buf)
    };
    check(hr, "LockBuffer")?;
    let buf = Com::new(buf).ok_or_else(|| fail("no bitmap buffer"))?;
    // IBitmapBuffer::GetPlaneDescription (slot 7).
    let mut plane = Plane::default();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, i32, *mut Plane) -> HRESULT = buf.slot(7);
        f(buf.0, 0, &mut plane)
    };
    check(hr, "GetPlaneDescription")?;
    {
        // IMemoryBuffer::CreateReference (slot 6), then the bytes.
        let mem = buf.query(&IID_MEMORY_BUFFER)?;
        let r = mem.get_obj(6, "CreateReference")?.ok_or_else(|| fail("no buffer reference"))?;
        let access = r.query(&IID_MEMORY_BUFFER_BYTE_ACCESS)?;
        let (mut p, mut cap) = (core::ptr::null_mut::<u8>(), 0u32);
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32) -> HRESULT = access.slot(3);
            f(access.0, &mut p, &mut cap)
        };
        check(hr, "GetBuffer")?;
        if p.is_null() || plane.start < 0 || plane.stride < img.width() as i32 * 4 {
            return Err(fail("unexpected bitmap layout"));
        }
        let start = plane.start as usize;
        let need = start + plane.stride as usize * (img.height() as usize - 1) + img.width() as usize * 4;
        if need > cap as usize {
            return Err(fail("bitmap buffer too small"));
        }
        let dst = unsafe { std::slice::from_raw_parts_mut(p.add(start), cap as usize - start) };
        fill_bgra(dst, plane.stride as usize, img);
        r.close();
    }
    buf.close();
    Ok(bmp)
}

/// Wait for an `IAsyncOperation<T>` and take its result (slot 8).
fn wait(op: &Com) -> Result<Option<Com>, OcrError> {
    let info = op.query(&IID_ASYNC_INFO)?;
    let t0 = Instant::now();
    loop {
        let st: i32 = info.get(7, "Status")?;
        match st {
            STARTED if t0.elapsed() > TIMEOUT => {
                unsafe {
                    let f: unsafe extern "system" fn(*mut c_void) -> HRESULT = info.slot(9);
                    debug_hr(f(info.0), "Cancel");
                }
                return Err(OcrError::TimedOut);
            }
            STARTED => std::thread::sleep(Duration::from_millis(5)),
            COMPLETED => break,
            CANCELED => return Err(fail("cancelled")),
            _ => {
                let hr: HRESULT = info.get(8, "ErrorCode")?;
                return Err(fail(hr.message().trim().to_string()));
            }
        }
    }
    op.get_obj(8, "GetResults")
}

/// The engine's largest image side (`OcrEngine.MaxImageDimension`).
fn max_side(statics: &Com) -> Option<u32> {
    statics.get::<u32>(6, "MaxImageDimension").ok().filter(|m| *m > 0)
}

pub fn recognize_lines(img: &PixBuf) -> Result<Vec<Line>, OcrError> {
    let _apt = Apartment::enter();
    let statics = factory("Windows.Media.Ocr.OcrEngine", &IID_OCR_ENGINE_STATICS)?;
    let max = max_side(&statics);
    // TryCreateFromUserProfileLanguages (slot 10): null without a language.
    let engine = statics.get_obj(10, "TryCreateFromUserProfileLanguages")?.ok_or(OcrError::NoLanguage)?;
    if let Some(m) = max
        && img.width().max(img.height()) > m
    {
        return Err(fail(crate::tr!("err.ocr_too_large", px = m)));
    }
    let (img, k) = super::prepare(img, max);
    let bmp = software_bitmap(&img)?;
    // IOcrEngine::RecognizeAsync (slot 6).
    let mut op = core::ptr::null_mut();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT = engine.slot(6);
        f(engine.0, bmp.0, &mut op)
    };
    check(hr, "RecognizeAsync")?;
    let op = Com::new(op).ok_or_else(|| fail("no operation"))?;
    let Some(result) = wait(&op)? else { return Ok(Vec::new()) };
    // IOcrResult::Lines (6) → IOcrLine::Words (6) → IOcrWord::BoundingRect (6), Text (7).
    let Some(lines) = result.get_obj(6, "Lines")? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for l in lines.items()? {
        let mut line = Line::default();
        if let Some(words) = l.get_obj(6, "Words")? {
            for w in words.items()? {
                let r: Rect = w.get(6, "BoundingRect")?;
                let s = k as f32;
                line.words.push(Word { text: w.get_text(7)?, x: r.x / s, y: r.y / s, w: r.w / s, h: r.h / s });
            }
        }
        out.push(line);
    }
    Ok(out)
}

#[cfg(test)]
mod null_tests {
    /// A null interface (TryCreateFromUserProfileLanguages without an OCR
    /// language, as on a fresh Windows Sandbox) is `None`, and nothing is
    /// released: the old eager `then_some(Com(p))` dropped a `Com(null)`
    /// and crashed Text mode and Quick Redact.
    #[test]
    fn a_null_interface_is_none_and_never_released() {
        assert!(super::Com::new(core::ptr::null_mut()).is_none());
    }
}
