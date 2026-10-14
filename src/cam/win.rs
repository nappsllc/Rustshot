//! Windows camera capture through Media Foundation: video capture devices
//! from `MFEnumDeviceSources`, a synchronous `IMFSourceReader` on the one
//! chosen, in one of its own NV12 or YUY2 modes (no decoder or converter
//! in between; MJPG-only cameras are refused, see [`super::pick_mode`]),
//! converted to BGRA here.
//!
//! The DLLs are loaded at run time like the MP4 encoder's
//! (`crate::rec::win::mf`); [`mf::available`] starts Media Foundation.
//! Everything runs in the multithreaded apartment (the bubble's camera
//! thread, or a short-lived MTA thread for the device list).

use super::{CamDevice, CamFrame, CamSource, Fmt, Matrix, Mode, Yuv, ACCESS_OFF};
use crate::rec::win::{com_init, in_mta, mf};
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::c_void;
use std::sync::OnceLock;
use windows::core::{Interface, GUID, HRESULT, PWSTR};
use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

type FnCreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type FnEnumDeviceSources = unsafe extern "system" fn(*mut c_void, *mut *mut *mut c_void, *mut u32) -> HRESULT;
type FnReaderFromSource = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;

struct Api {
    create_attributes: FnCreateAttributes,
    enum_device_sources: FnEnumDeviceSources,
    reader_from_source: FnReaderFromSource,
}

fn api() -> Result<&'static Api> {
    static API: OnceLock<std::result::Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| unsafe { load() }).as_ref().map_err(|e| anyhow!("{e}"))
}

unsafe fn load() -> std::result::Result<Api, String> {
    // Loads mfplat / mfreadwrite and calls MFStartup.
    if !mf::available() {
        return Err(crate::tr!("err.media_foundation").into());
    }
    unsafe {
        let lib = |name: windows::core::PCWSTR| LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32);
        let plat = lib(windows::core::w!("mfplat.dll")).map_err(|_| "mfplat.dll is missing")?;
        let mfdll = lib(windows::core::w!("mf.dll")).map_err(|_| "mf.dll is missing")?;
        let rw = lib(windows::core::w!("mfreadwrite.dll")).map_err(|_| "mfreadwrite.dll is missing")?;
        macro_rules! sym {
            ($m:expr, $name:literal, $t:ty) => {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, $t>(
                    GetProcAddress($m, windows::core::s!($name)).ok_or_else(|| format!("{} not found", $name))?,
                )
            };
        }
        Ok(Api {
            create_attributes: sym!(plat, "MFCreateAttributes", FnCreateAttributes),
            enum_device_sources: sym!(mfdll, "MFEnumDeviceSources", FnEnumDeviceSources),
            reader_from_source: sym!(rw, "MFCreateSourceReaderFromMediaSource", FnReaderFromSource),
        })
    }
}

/// A camera error said the way the user can act on it.
fn explain(e: windows::core::Error) -> anyhow::Error {
    if e.code() == E_ACCESSDENIED {
        anyhow!(ACCESS_OFF)
    } else if e.code() == MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED || e.code() == MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED {
        anyhow!("The camera was disconnected or taken by another app")
    } else {
        anyhow::Error::new(e)
    }
}

unsafe fn alloc_string(a: &IMFActivate, key: &GUID) -> Option<String> {
    unsafe {
        let mut p = PWSTR::null();
        let mut n = 0u32;
        a.GetAllocatedString(key, &mut p, &mut n).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s
    }
}

/// The video capture devices' activation objects (call in the MTA).
fn devices(api: &Api) -> Result<Vec<IMFActivate>> {
    unsafe {
        let mut attrs = std::ptr::null_mut();
        (api.create_attributes)(&mut attrs, 1).ok()?;
        if attrs.is_null() {
            bail!("MFCreateAttributes returned nothing");
        }
        let attrs = IMFAttributes::from_raw(attrs);
        attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        let (mut arr, mut n) = (std::ptr::null_mut::<*mut c_void>(), 0u32);
        (api.enum_device_sources)(attrs.as_raw(), &mut arr, &mut n).ok().map_err(explain)?;
        let mut out = Vec::new();
        if !arr.is_null() {
            for i in 0..n as usize {
                let p = *arr.add(i);
                if !p.is_null() {
                    out.push(IMFActivate::from_raw(p));
                }
            }
            CoTaskMemFree(Some(arr as *const c_void));
        }
        Ok(out)
    }
}

/// The cameras (empty when there are none or Media Foundation is missing).
pub fn list() -> Vec<CamDevice> {
    let Ok(api) = api() else { return Vec::new() };
    in_mta(|| {
        let devs = devices(api).unwrap_or_else(|e| {
            eprintln!("rustshot: cannot list cameras: {e:#}");
            Vec::new()
        });
        devs.iter()
            .filter_map(|a| unsafe {
                let id = alloc_string(a, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK)?;
                let name = alloc_string(a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME).unwrap_or_else(|| tr!("tip.camera").into());
                Some(CamDevice { id, name })
            })
            .collect()
    })
}

fn fmt_of(g: &GUID) -> Fmt {
    if *g == MFVideoFormat_NV12 {
        Fmt::Nv12
    } else if *g == MFVideoFormat_YUY2 {
        Fmt::Yuy2
    } else if *g == MFVideoFormat_MJPG {
        Fmt::Mjpg
    } else {
        Fmt::Other
    }
}

fn mode_of(t: &IMFMediaType) -> Option<Mode> {
    unsafe {
        let fmt = fmt_of(&t.GetGUID(&MF_MT_SUBTYPE).ok()?);
        let fs = t.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let fr = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(30 << 32 | 1);
        let (num, den) = ((fr >> 32) as u32, (fr as u32).max(1));
        Some(Mode { fmt, w: (fs >> 32) as u32, h: fs as u32, fps: num as f32 / den as f32 })
    }
}

/// How the current media type lays its pictures out.
#[derive(Clone, Copy, Debug)]
struct Pixels {
    /// The format and the visible size (the display aperture's, when the
    /// frame is padded beyond it).
    mode: Mode,
    /// The frame's height as coded (≥ `mode.h`).
    coded_h: u32,
    yuv: Yuv,
    /// Row pitch from the media type (for a buffer that gives none).
    stride: usize,
    /// The media type's stride is negative: rows run bottom to top.
    bottom_up: bool,
}

fn pixels_of(t: &IMFMediaType) -> Option<Pixels> {
    let coded = mode_of(t)?;
    unsafe {
        let mut mode = coded;
        // A display aperture inside a padded frame (1920x1088 showing 1080).
        let mut area = MFVideoArea::default();
        let n = std::mem::size_of::<MFVideoArea>() as u32;
        let blob = std::slice::from_raw_parts_mut(&mut area as *mut MFVideoArea as *mut u8, n as usize);
        if t.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, blob, None).is_ok() {
            let (aw, ah) = (area.Area.cx.max(0) as u32, area.Area.cy.max(0) as u32);
            if aw >= 2 && ah >= 2 && aw <= coded.w && ah <= coded.h {
                (mode.w, mode.h) = (aw, ah);
            }
        }
        let matrix = match t.GetUINT32(&MF_MT_YUV_MATRIX) {
            Ok(m) if m == MFVideoTransferMatrix_BT709.0 as u32 => Matrix::Bt709,
            _ => Matrix::Bt601,
        };
        let full = t.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE).is_ok_and(|r| r == MFNominalRange_0_255.0 as u32);
        let row = if coded.fmt == Fmt::Yuy2 { coded.w as usize * 2 } else { coded.w as usize };
        let s = t.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(row as i32);
        Some(Pixels {
            mode,
            coded_h: coded.h,
            yuv: Yuv { matrix, full },
            stride: (s.unsigned_abs() as usize).max(row),
            bottom_up: s < 0,
        })
    }
}

/// An open camera: its media source and a source reader on it.
pub struct WinCam {
    source: IMFMediaSource,
    reader: IMFSourceReader,
    px: Pixels,
    /// Pictures dropped because their buffer was too short.
    drops: u64,
}

// SAFETY: the media source and the source reader are free-threaded; the
// camera is used by one thread at a time (the bubble's camera thread,
// which joined the MTA).
unsafe impl Send for WinCam {}

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

impl WinCam {
    /// Open camera `id` ("" = the first) in its mode nearest `pref`. Call
    /// on a thread in the MTA (the camera thread).
    pub fn open(id: &str, pref: (u32, u32)) -> Result<WinCam> {
        let api = api()?;
        com_init();
        let devs = devices(api)?;
        let act = if id.is_empty() {
            devs.first()
        } else {
            devs.iter().find(|a| unsafe { alloc_string(a, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK) }.as_deref() == Some(id))
        };
        let Some(act) = act else {
            bail!("{}", if devs.is_empty() { super::NO_CAMERA.get() } else { tr!("cam.not_connected") });
        };
        unsafe {
            let source: IMFMediaSource = act.ActivateObject().map_err(explain).context("open the camera")?;
            let cam = (|| -> Result<WinCam> {
                let mut r = std::ptr::null_mut();
                (api.reader_from_source)(source.as_raw(), std::ptr::null_mut(), &mut r).ok().map_err(explain)?;
                if r.is_null() {
                    bail!("no source reader");
                }
                let reader = IMFSourceReader::from_raw(r);
                let mut types = Vec::new();
                let mut modes = Vec::new();
                for i in 0..512 {
                    let Ok(t) = reader.GetNativeMediaType(STREAM, i) else { break };
                    if let Some(m) = mode_of(&t) {
                        modes.push(m);
                        types.push(t);
                    }
                }
                let i = super::pick_mode(&modes, pref).map_err(|e| anyhow!(e))?;
                reader.SetCurrentMediaType(STREAM, None, &types[i]).map_err(explain).context("set the camera mode")?;
                let cur = reader.GetCurrentMediaType(STREAM).unwrap_or_else(|_| types[i].clone());
                let px = pixels_of(&cur).or_else(|| pixels_of(&types[i])).context("read the camera mode")?;
                Ok(WinCam { source: source.clone(), reader, px, drops: 0 })
            })();
            if cam.is_err() {
                let _ = source.Shutdown();
                let _ = act.ShutdownObject();
            }
            cam
        }
    }

    /// The mode in use.
    #[cfg_attr(not(test), allow(dead_code))] // tests
    pub fn mode(&self) -> Mode {
        self.px.mode
    }

    /// Pictures dropped so far, and how the pixels are read (tests).
    #[cfg(test)]
    pub fn stats(&self) -> (u64, Yuv, u32, usize) {
        (self.drops, self.px.yuv, self.px.coded_h, self.px.stride)
    }

    /// The camera changed its media type mid-stream: read it again.
    fn reread_type(&mut self) -> Result<()> {
        let cur = unsafe { self.reader.GetCurrentMediaType(STREAM) }.map_err(explain).context("read the new camera mode")?;
        let px = pixels_of(&cur).context("read the new camera mode")?;
        if !matches!(px.mode.fmt, Fmt::Nv12 | Fmt::Yuy2) {
            bail!(super::NO_FORMAT);
        }
        eprintln!("rustshot: camera mode changed to {:?}", px.mode);
        self.px = px;
        Ok(())
    }

    /// Convert the picture in `data` (rows `stride` apart, bottom up when
    /// `flip`) into `out`; false (counted as a drop) when `data` is too
    /// short for it.
    fn convert(&mut self, data: &[u8], stride: usize, flip: bool, out: &mut CamFrame) -> bool {
        let Pixels { mode, coded_h, yuv, .. } = self.px;
        let (w, h) = self.size();
        let Some(l) = super::layout(mode.fmt, w, h, coded_h, stride, data.len()) else {
            self.drops += 1;
            if self.drops == 1 || self.drops.is_multiple_of(100) {
                eprintln!("rustshot: camera picture dropped ({} bytes, {w}x{h} at pitch {stride}); {} so far", data.len(), self.drops);
            }
            return false;
        };
        let data = &data[..l.len];
        let converted = match mode.fmt {
            Fmt::Yuy2 => super::yuy2_to_bgra(data, l.stride, w, h, yuv, &mut out.bgra),
            _ => {
                let (y, uv) = data.split_at(l.uv);
                super::nv12_to_bgra(y, uv, l.stride, w, h, yuv, &mut out.bgra)
            }
        };
        if converted.is_none() {
            return false;
        }
        if flip {
            super::flip_rows(&mut out.bgra, w, h);
        }
        (out.w, out.h) = (w, h);
        true
    }
}

impl CamSource for WinCam {
    fn size(&self) -> (u32, u32) {
        let m = self.px.mode;
        (m.w & !1, if m.fmt == Fmt::Nv12 { m.h & !1 } else { m.h })
    }

    fn read(&mut self, into: &mut CamFrame) -> Result<bool> {
        let mut flags = 0u32;
        let mut sample = None;
        unsafe { self.reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)) }
            .map_err(explain)
            .context("read from the camera")?;
        if flags & (MF_SOURCE_READERF_ERROR.0 as u32 | MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
            bail!("The camera stopped");
        }
        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
            self.reread_type()?;
        }
        let Some(sample) = sample else { return Ok(false) };
        let buf = unsafe { sample.GetBufferByIndex(0) }.context("camera buffer")?;
        // A 2D buffer says where its rows are and how long it really is.
        if let Ok(b2) = buf.cast::<IMF2DBuffer2>() {
            let (mut row0, mut pitch, mut start, mut len) = (std::ptr::null_mut(), 0i32, std::ptr::null_mut(), 0u32);
            if unsafe { b2.Lock2DSize(MF2DBuffer_LockFlags_Read, &mut row0, &mut pitch, &mut start, &mut len) }.is_ok() {
                let ok = if row0.is_null() || start.is_null() || pitch == 0 {
                    None
                } else if pitch > 0 {
                    // From the top row to the end of the buffer.
                    let skip = (row0 as usize).checked_sub(start as usize).filter(|&s| s <= len as usize);
                    skip.map(|s| {
                        let data = unsafe { std::slice::from_raw_parts(row0 as *const u8, len as usize - s) };
                        self.convert(data, pitch as usize, false, into)
                    })
                } else {
                    // Bottom up: the buffer starts with the last row.
                    let data = unsafe { std::slice::from_raw_parts(start as *const u8, len as usize) };
                    Some(self.convert(data, pitch.unsigned_abs() as usize, true, into))
                };
                let _ = unsafe { b2.Unlock2D() };
                if let Some(ok) = ok {
                    return Ok(ok);
                }
            }
        }
        // Contiguous: rows at the media type's stride.
        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
        unsafe { buf.Lock(&mut p, None, Some(&mut len)) }.context("lock the camera buffer")?;
        let ok = !p.is_null() && {
            let data = unsafe { std::slice::from_raw_parts(p as *const u8, len as usize) };
            let (stride, flip) = (self.px.stride, self.px.bottom_up);
            self.convert(data, stride, flip, into)
        };
        let _ = unsafe { buf.Unlock() };
        Ok(ok)
    }
}

impl Drop for WinCam {
    /// Release the camera now (its light goes off).
    fn drop(&mut self) {
        unsafe {
            let _ = self.source.Shutdown();
        }
    }
}
