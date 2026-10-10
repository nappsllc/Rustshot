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

use super::{CamDevice, CamFrame, CamSource, Fmt, Matrix, Mode, ACCESS_OFF};
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
        return Err("Media Foundation is not available (on Windows N install the Media Feature Pack)".into());
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
                let name = alloc_string(a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME).unwrap_or_else(|| "Camera".into());
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

/// An open camera: its media source and a source reader on it.
pub struct WinCam {
    source: IMFMediaSource,
    reader: IMFSourceReader,
    mode: Mode,
    matrix: Matrix,
    /// Row pitch from the media type (when a buffer gives none).
    stride: usize,
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
            bail!(if devs.is_empty() { super::NO_CAMERA } else { "The chosen camera is not connected" });
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
                let mode = mode_of(&cur).unwrap_or(modes[i]);
                let matrix = match cur.GetUINT32(&MF_MT_YUV_MATRIX) {
                    Ok(m) if m == MFVideoTransferMatrix_BT709.0 as u32 => Matrix::Bt709,
                    _ => Matrix::Bt601,
                };
                let row = if mode.fmt == Fmt::Yuy2 { mode.w as usize * 2 } else { mode.w as usize };
                let stride = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).map_or(row, |s| (s as i32).unsigned_abs() as usize).max(row);
                Ok(WinCam { source: source.clone(), reader, mode, matrix, stride })
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
        self.mode
    }

    fn convert(&self, data: &[u8], pitch: usize, out: &mut Vec<u8>) {
        let (w, h) = (self.mode.w, self.mode.h);
        match self.mode.fmt {
            Fmt::Yuy2 => super::yuy2_to_bgra(data, pitch, w, h, self.matrix, out),
            _ => {
                let y_len = pitch * h as usize;
                let (y, uv) = data.split_at(y_len.min(data.len()));
                super::nv12_to_bgra(y, uv, pitch, w, h, self.matrix, out);
            }
        }
    }
}

impl CamSource for WinCam {
    fn size(&self) -> (u32, u32) {
        (self.mode.w & !1, if self.mode.fmt == Fmt::Nv12 { self.mode.h & !1 } else { self.mode.h })
    }

    fn read(&mut self) -> Result<Option<CamFrame>> {
        let mut flags = 0u32;
        let mut sample = None;
        unsafe { self.reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)) }
            .map_err(explain)
            .context("read from the camera")?;
        if flags & (MF_SOURCE_READERF_ERROR.0 as u32 | MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
            bail!("The camera stopped");
        }
        let Some(sample) = sample else { return Ok(None) };
        let buf = unsafe { sample.GetBufferByIndex(0) }.context("camera buffer")?;
        let mut bgra = Vec::new();
        // A 2D buffer knows its pitch; otherwise the media type's stride.
        if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
            let (mut p, mut pitch) = (std::ptr::null_mut(), 0i32);
            if unsafe { b2.Lock2D(&mut p, &mut pitch) }.is_ok() {
                let pitch_abs = pitch.unsigned_abs() as usize;
                let rows = if self.mode.fmt == Fmt::Nv12 { self.mode.h as usize * 3 / 2 } else { self.mode.h as usize };
                if pitch > 0 && !p.is_null() {
                    let data = unsafe { std::slice::from_raw_parts(p, pitch_abs * rows) };
                    self.convert(data, pitch_abs, &mut bgra);
                }
                let _ = unsafe { b2.Unlock2D() };
                if pitch > 0 {
                    let (w, h) = self.size();
                    return Ok(Some(CamFrame { w, h, bgra }));
                }
            }
        }
        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
        unsafe { buf.Lock(&mut p, None, Some(&mut len)) }.context("lock the camera buffer")?;
        if !p.is_null() {
            let data = unsafe { std::slice::from_raw_parts(p, len as usize) };
            self.convert(data, self.stride, &mut bgra);
        }
        let _ = unsafe { buf.Unlock() };
        let (w, h) = self.size();
        Ok(Some(CamFrame { w, h, bgra }))
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
