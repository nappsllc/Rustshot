//! HDR on Windows: which outputs are in HDR (`IDXGIOutput6::GetDesc1`,
//! color space `DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020`), their SDR
//! white level (`DisplayConfigGetDeviceInfo`), and a screenshot of their
//! part of a rect: Desktop Duplication in FP16 (`IDXGIOutput5::
//! DuplicateOutput1` with `R16G16B16A16_FLOAT`: scRGB), tone-mapped by
//! [`super::ToneMap`].
//!
//! The process keeps one warm duplication per HDR output ([`warm_up`]
//! makes them at start; a capture makes one for an output that turned HDR
//! since, and re-makes one whose access was lost) with a GPU copy of the
//! latest desktop image it delivered. A duplication queues every change
//! since its last frame, so a capture asks for a new frame for at most
//! [`WAIT_MS`] and, when none comes (nothing changed: a static desktop),
//! the copy is current. A capture therefore never waits more than about
//! [`BUDGET_MS`] per HDR output; an output with no image yet keeps its GDI
//! pixels (logged). Without an HDR output in the rect nothing here runs
//! past the output list (a DXGI factory and a `GetDesc1` per output).

use super::ToneMap;
use crate::capture::IRect;
use crate::pixbuf::PixBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::*;

/// How long a capture waits for a new frame of a warm duplication.
pub const WAIT_MS: u32 = 50;
/// The most a capture spends on one HDR output (a new duplication's first
/// frame included).
pub const BUDGET_MS: u64 = 60;

/// What an HDR output says about itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hdr {
    /// SDR content's white (nits).
    pub white_nits: f32,
}

impl Hdr {
    /// Its mapping: SDR white at the display's SDR content brightness.
    pub fn tone_map(&self) -> ToneMap {
        ToneMap::new(self.white_nits)
    }
}

/// `Some` when `output` is in HDR (the PQ / BT.2020 color space).
pub fn output_hdr(output: &IDXGIOutput) -> Option<Hdr> {
    let o6: IDXGIOutput6 = output.cast().ok()?;
    let d = unsafe { o6.GetDesc1() }.ok()?;
    (d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020).then(|| Hdr { white_nits: sdr_white_nits(&d.DeviceName) })
}

/// The active display paths, asked again when the topology changed
/// between sizing the buffers and filling them.
fn display_paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    for _ in 0..4 {
        unsafe {
            let (mut np, mut nm) = (0u32, 0u32);
            if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
                return None;
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
            match QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) {
                ERROR_SUCCESS => {
                    paths.truncate(np as usize);
                    return Some(paths);
                }
                ERROR_INSUFFICIENT_BUFFER => continue,
                _ => return None,
            }
        }
    }
    None
}

/// The SDR white level of the display whose GDI name is `gdi` (nits;
/// Windows' 80 when it cannot say).
pub fn sdr_white_nits(gdi: &[u16; 32]) -> f32 {
    let Some(paths) = display_paths() else { return super::DEFAULT_WHITE_NITS };
    let name_len = gdi.iter().position(|&c| c == 0).unwrap_or(gdi.len());
    for p in &paths {
        unsafe {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: p.sourceInfo.adapterId,
                    id: p.sourceInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let n = &src.viewGdiDeviceName;
            let len = n.iter().position(|&c| c == 0).unwrap_or(n.len());
            if n[..len] != gdi[..name_len] {
                continue;
            }
            let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: std::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: p.targetInfo.adapterId,
                    id: p.targetInfo.id,
                },
                SDRWhiteLevel: 0,
            };
            if DisplayConfigGetDeviceInfo(&mut white.header) == 0 && white.SDRWhiteLevel > 0 {
                // In thousandths of 80 nits.
                return white.SDRWhiteLevel as f32 / 1000.0 * super::SCRGB_NITS;
            }
        }
    }
    super::DEFAULT_WHITE_NITS
}

fn intersect(a: IRect, r: &RECT) -> Option<IRect> {
    let x0 = a.0.max(r.left);
    let y0 = a.1.max(r.top);
    let x1 = (a.0 + a.2 as i32).min(r.right);
    let y1 = (a.1 + a.3 as i32).min(r.bottom);
    (x1 > x0 && y1 > y0).then(|| (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// A D3D11 device on `adapter` (FP16 duplication needs nothing more).
pub fn device(adapter: &IDXGIAdapter1) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let (mut dev, mut ctx) = (None, None);
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut dev),
            None,
            Some(&mut ctx),
        )?;
    }
    Ok((dev.ok_or_else(windows::core::Error::empty)?, ctx.ok_or_else(windows::core::Error::empty)?))
}

fn fp16_texture(device: &ID3D11Device, w: u32, h: u32, staging: bool) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: if staging { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
        BindFlags: 0,
        CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex)) }?;
    tex.ok_or_else(windows::core::Error::empty)
}

/// A CPU-readable FP16 texture of `w` x `h`.
pub fn fp16_staging(device: &ID3D11Device, w: u32, h: u32) -> windows::core::Result<ID3D11Texture2D> {
    fp16_texture(device, w, h, true)
}

/// Duplicate `output` in FP16 (scRGB) on `device`; `None` when the system
/// will not (then it is read as before).
pub fn duplicate_fp16(output: &IDXGIOutput, device: &ID3D11Device) -> Option<IDXGIOutputDuplication> {
    let o5: IDXGIOutput5 = output.cast().ok()?;
    let dup = unsafe { o5.DuplicateOutput1(device, 0, &[DXGI_FORMAT_R16G16B16A16_FLOAT]) }.ok()?;
    let d = unsafe { dup.GetDesc() };
    let upright = d.Rotation == DXGI_MODE_ROTATION_IDENTITY || d.Rotation == DXGI_MODE_ROTATION_UNSPECIFIED;
    (d.ModeDesc.Format == DXGI_FORMAT_R16G16B16A16_FLOAT && upright).then_some(dup)
}

/// One HDR output's warm duplication and its latest desktop image.
struct Warm {
    name: [u16; 32],
    desk: RECT,
    output: IDXGIOutput,
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    dup: Option<IDXGIOutputDuplication>,
    /// The latest desktop image (FP16, the whole output, on the GPU).
    last: Option<ID3D11Texture2D>,
    /// The fallback was logged since the last image.
    logged: bool,
}

/// The warm duplications (D3D11 devices are free-threaded; the contexts
/// and duplications are only used under the lock).
struct Warms(Vec<Warm>);
// SAFETY: every use is under `WARM`'s lock, one thread at a time.
unsafe impl Send for Warms {}

static WARM: Mutex<Warms> = Mutex::new(Warms(Vec::new()));

impl Warm {
    fn new(adapter: &IDXGIAdapter1, output: IDXGIOutput, name: [u16; 32], desk: RECT) -> Option<Warm> {
        let (dev, ctx) = device(adapter).ok()?;
        let dup = duplicate_fp16(&output, &dev)?;
        Some(Warm { name, desk, output, dev, ctx, dup: Some(dup), last: None, logged: false })
    }

    /// Take a new frame into `last` if one comes within `wait_ms`
    /// (re-duplicating after lost access); true when `last` is there.
    fn update(&mut self, wait_ms: u32) -> bool {
        if self.dup.is_none() {
            self.dup = duplicate_fp16(&self.output, &self.dev);
            self.last = None;
        }
        let Some(dup) = self.dup.clone() else { return false };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res = None;
        match unsafe { dup.AcquireNextFrame(wait_ms, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return self.last.is_some(),
            Err(e) => {
                if e.code() == DXGI_ERROR_ACCESS_LOST {
                    eprintln!("rustshot: HDR display duplication lost; re-created at the next capture");
                }
                self.dup = None;
                self.last = None;
                return false;
            }
        }
        // Copied out before the frame goes back, and the frame's texture
        // dropped before it is released.
        if info.LastPresentTime != 0
            && let Some(tex) = res.take().and_then(|r| r.cast::<ID3D11Texture2D>().ok())
        {
            let (w, h) = ((self.desk.right - self.desk.left) as u32, (self.desk.bottom - self.desk.top) as u32);
            if self.last.is_none() {
                self.last = fp16_texture(&self.dev, w, h, false).ok();
            }
            if let Some(last) = &self.last {
                unsafe { self.ctx.CopyResource(last, &tex) };
            }
            drop(tex);
        }
        drop(res);
        let _ = unsafe { dup.ReleaseFrame() };
        self.last.is_some()
    }

    /// The part `isect` (virtual-screen px) of `last`, tone-mapped (RGBA).
    fn read(&self, isect: IRect, tm: ToneMap) -> Option<PixBuf> {
        let last = self.last.as_ref()?;
        let (ix, iy, iw, ih) = isect;
        let stg = fp16_staging(&self.dev, iw, ih).ok()?;
        let bx = D3D11_BOX {
            left: (ix - self.desk.left) as u32,
            top: (iy - self.desk.top) as u32,
            front: 0,
            right: (ix - self.desk.left) as u32 + iw,
            bottom: (iy - self.desk.top) as u32 + ih,
            back: 1,
        };
        unsafe { self.ctx.CopySubresourceRegion(&stg, 0, 0, 0, 0, last, 0, Some(&bx)) };
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.ctx.Map(&stg, 0, D3D11_MAP_READ, 0, Some(&mut m)) }.ok()?;
        let mut rgba = vec![0u8; iw as usize * ih as usize * 4];
        {
            let src = unsafe { mapped_fp16(&m, iw, ih) };
            let pitch = m.RowPitch as usize / 2;
            let rows = rgba.chunks_mut(iw as usize * 4).enumerate().map(|(y, d)| (&src[y * pitch..y * pitch + iw as usize * 4], d)).collect();
            tm.rows(rows, false);
        }
        unsafe { self.ctx.Unmap(&stg, 0) };
        PixBuf::from_raw(iw, ih, rgba)
    }
}

/// The mapped FP16 texture `m` (`w` x `h`) as halves, row pitch apart.
///
/// # Safety
/// `m` is a live mapping of a `R16G16B16A16_FLOAT` texture of that size,
/// and the slice is not used after it is unmapped.
pub unsafe fn mapped_fp16<'a>(m: &D3D11_MAPPED_SUBRESOURCE, w: u32, h: u32) -> &'a [u16] {
    let pitch = m.RowPitch as usize / 2;
    let len = if h == 0 { 0 } else { pitch * (h as usize - 1) + w as usize * 4 };
    unsafe { std::slice::from_raw_parts(m.pData as *const u16, len) }
}

/// The attached HDR outputs now: (adapter, output, GDI name, rect, HDR).
fn hdr_outputs() -> Vec<(IDXGIAdapter1, IDXGIOutput, [u16; 32], RECT, Hdr)> {
    let mut out = Vec::new();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    let mut ai = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(ai) } {
        ai += 1;
        let mut oi = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(oi) } {
            oi += 1;
            let Ok(d) = (unsafe { output.GetDesc() }) else { continue };
            if !d.AttachedToDesktop.as_bool() {
                continue;
            }
            if let Some(h) = output_hdr(&output) {
                out.push((adapter.clone(), output, d.DeviceName, d.DesktopCoordinates, h));
            }
        }
    }
    out
}

/// Keep `warms` to the HDR outputs there are now (new ones duplicated,
/// gone ones dropped); with each one's mapping (by name and rect).
fn refresh(warms: &mut Vec<Warm>) -> Vec<([u16; 32], RECT, ToneMap)> {
    let now = hdr_outputs();
    warms.retain(|w| now.iter().any(|(_, _, n, r, _)| *n == w.name && *r == w.desk));
    let mut maps = Vec::new();
    for (adapter, output, name, desk, h) in now {
        if !warms.iter().any(|w| w.name == name && w.desk == desk)
            && let Some(w) = Warm::new(&adapter, output, name, desk)
        {
            warms.push(w);
        }
        maps.push((name, desk, h.tone_map()));
    }
    maps
}

/// Duplicate the HDR outputs on a background thread and take their first
/// images, off the first capture's path (DXGI's first load is ~40 ms; a
/// new duplication's first frame up to a few hundred ms).
pub fn warm_up() {
    let _ = std::thread::Builder::new().name("hdr-warm".into()).spawn(|| {
        if !super::enabled() {
            return;
        }
        let Ok(mut g) = WARM.lock() else { return };
        refresh(&mut g.0);
        for w in g.0.iter_mut() {
            w.update(300);
        }
    });
}

/// The parts of `area` (virtual-screen px) that lie on HDR outputs, read
/// in FP16 and tone-mapped (RGBA), with where each goes. Empty when
/// tone-mapping is off ([`super::enabled`]) or no HDR output shows the
/// area; an output with no image within [`BUDGET_MS`] is left out, logged
/// (the caller keeps its pixels as captured).
pub fn grab(area: IRect) -> Vec<(IRect, PixBuf)> {
    let mut out = Vec::new();
    if !super::enabled() || area.2 == 0 || area.3 == 0 {
        return out;
    }
    let Ok(mut g) = WARM.lock() else { return out };
    let maps = refresh(&mut g.0);
    for w in g.0.iter_mut() {
        let Some(isect) = intersect(area, &w.desk) else { continue };
        let Some(&(_, _, tm)) = maps.iter().find(|m| m.0 == w.name && m.1 == w.desk) else { continue };
        let t0 = Instant::now();
        let wait = if w.last.is_some() { WAIT_MS } else { BUDGET_MS as u32 };
        let mut have = w.update(wait);
        // Access lost on the way: one more try in what is left.
        if !have && w.dup.is_none() {
            let left = Duration::from_millis(BUDGET_MS).saturating_sub(t0.elapsed()).as_millis() as u32;
            if left > 5 {
                have = w.update(left);
            }
        }
        match have.then(|| w.read(isect, tm)).flatten() {
            Some(img) => {
                w.logged = false;
                out.push((isect, img));
            }
            None if !w.logged => {
                w.logged = true;
                eprintln!("rustshot: the HDR display has no image yet; captured as is (washed out)");
            }
            None => {}
        }
    }
    out
}

/// Copy `img` into `dst` (RGBA, `size`) with its top-left at `at`
/// (clipped).
pub fn paste(dst: &mut PixBuf, at: (i32, i32), img: &PixBuf) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (img.width() as i32, img.height() as i32);
    let (x0, y0) = (at.0.max(0), at.1.max(0));
    let (x1, y1) = ((at.0 + sw).min(dw), (at.1 + sh).min(dh));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    let d = dst.as_raw_mut();
    for y in y0..y1 {
        let s = ((y - at.1) as usize * sw as usize + (x0 - at.0) as usize) * 4;
        let o = (y as usize * dw as usize + x0 as usize) * 4;
        d[o..o + n].copy_from_slice(&img.as_raw()[s..s + n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_clips_to_the_image() {
        let mut dst = PixBuf::from_pixel(4, 3, [0, 0, 0, 255]);
        let src = PixBuf::from_pixel(2, 2, [9, 8, 7, 255]);
        paste(&mut dst, (3, -1), &src);
        assert_eq!(dst.get_pixel(3, 0), [9, 8, 7, 255]);
        assert_eq!(dst.get_pixel(3, 1), [0, 0, 0, 255]);
        assert_eq!(dst.get_pixel(2, 0), [0, 0, 0, 255]);
        paste(&mut dst, (10, 10), &src);
        paste(&mut dst, (0, 1), &src);
        assert_eq!((dst.get_pixel(0, 1), dst.get_pixel(1, 2)), ([9, 8, 7, 255], [9, 8, 7, 255]));
    }

    /// Each display's SDR white and HDR state; on an HDR display a
    /// 64 x 64 capture through the warm duplication, twice (the second
    /// from a static desktop), each within the budget.
    ///
    ///     cargo test live_outputs_report -- --ignored --nocapture
    #[test]
    #[ignore = "live display access (duplicates an HDR desktop)"]
    fn live_outputs_report_their_hdr_state() {
        let t0 = Instant::now();
        let none = grab((0, 0, 64, 64));
        eprintln!("grab: {:?} ({} parts)", t0.elapsed(), none.len());
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return };
        let mut ai = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(ai) } {
            ai += 1;
            let mut oi = 0;
            while let Ok(output) = unsafe { adapter.EnumOutputs(oi) } {
                oi += 1;
                let d = unsafe { output.GetDesc() }.unwrap();
                let white = sdr_white_nits(&d.DeviceName);
                assert!((40.0..=2000.0).contains(&white), "{white}");
                let hdr = output_hdr(&output);
                eprintln!("output {}: SDR white {white} nits, HDR {hdr:?}", String::from_utf16_lossy(&d.DeviceName));
                if hdr.is_some() {
                    let r = d.DesktopCoordinates;
                    for i in 0..2 {
                        let t = Instant::now();
                        let shots = grab((r.left, r.top, 64, 64));
                        let ms = t.elapsed().as_millis();
                        eprintln!("HDR grab {i}: {ms} ms, {} parts", shots.len());
                        assert!(shots.iter().all(|(_, p)| p.dimensions() == (64, 64)));
                        assert!(i == 0 || ms <= BUDGET_MS as u128 + 40, "{ms} ms");
                    }
                }
            }
        }
    }
}
