//! HDR on Windows: which outputs are in HDR (`IDXGIOutput6::GetDesc1`,
//! color space `DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020`), their SDR
//! white level (`DisplayConfigGetDeviceInfo`), and a screenshot of their
//! part of a rect: one frame of Desktop Duplication in FP16
//! (`IDXGIOutput5::DuplicateOutput1` with `R16G16B16A16_FLOAT`: scRGB),
//! tone-mapped by [`super::ToneMap`] with the picture's own peak.
//! Without an HDR output in the rect nothing here runs past the output
//! list (a DXGI factory and a `GetDesc1` per output).

use super::ToneMap;
use crate::capture::IRect;
use crate::pixbuf::PixBuf;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::*;

/// What an HDR output says about itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hdr {
    /// SDR content's white (nits).
    pub white_nits: f32,
    /// The display's peak (nits; 0 when it does not say).
    pub max_nits: f32,
}

impl Hdr {
    /// The mapping for a recording: up to the display's peak (fixed, so
    /// frames do not pump).
    pub fn tone_map(&self) -> ToneMap {
        let peak = if self.max_nits > 0.0 { self.max_nits } else { 1000.0 };
        ToneMap::new(self.white_nits, peak / super::SCRGB_NITS)
    }
}

/// `Some` when `output` is in HDR (the PQ / BT.2020 color space).
pub fn output_hdr(output: &IDXGIOutput) -> Option<Hdr> {
    let o6: IDXGIOutput6 = output.cast().ok()?;
    let d = unsafe { o6.GetDesc1() }.ok()?;
    (d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020)
        .then(|| Hdr { white_nits: sdr_white_nits(&d.DeviceName), max_nits: d.MaxLuminance })
}

/// The SDR white level of the display whose GDI name is `gdi` (nits;
/// Windows' 80 when it cannot say).
pub fn sdr_white_nits(gdi: &[u16; 32]) -> f32 {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return super::DEFAULT_WHITE_NITS;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) != ERROR_SUCCESS {
            return super::DEFAULT_WHITE_NITS;
        }
        let name_len = gdi.iter().position(|&c| c == 0).unwrap_or(gdi.len());
        for p in paths.iter().take(np as usize) {
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
        super::DEFAULT_WHITE_NITS
    }
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

/// A CPU-readable FP16 texture of `w` x `h`.
pub fn fp16_staging(device: &ID3D11Device, w: u32, h: u32) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex)) }?;
    tex.ok_or_else(windows::core::Error::empty)
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

/// Read the mapped FP16 staging texture `m` (`w` x `h`) as u16 pixels.
///
/// # Safety
/// `m` is a live mapping of a `R16G16B16A16_FLOAT` texture of that size.
pub unsafe fn read_fp16(m: &D3D11_MAPPED_SUBRESOURCE, w: u32, h: u32) -> Vec<u16> {
    let row = w as usize * 4;
    let mut out = vec![0u16; row * h as usize];
    for (y, dst) in out.chunks_exact_mut(row.max(1)).enumerate() {
        let src = unsafe { std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize) as *const u16, row) };
        dst.copy_from_slice(src);
    }
    out
}

/// The parts of `area` (virtual-screen px) that lie on HDR outputs, read
/// in FP16 and tone-mapped (RGBA), with where each goes. Empty when
/// tone-mapping is off ([`super::enabled`]) or no HDR output shows the
/// area; an output that cannot be read is left out (the caller keeps its
/// pixels as captured).
pub fn grab(area: IRect) -> Vec<(IRect, PixBuf)> {
    let mut out = Vec::new();
    if !super::enabled() || area.2 == 0 || area.3 == 0 {
        return out;
    }
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    let mut ai = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(ai) } {
        ai += 1;
        let mut oi = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(oi) } {
            oi += 1;
            let Ok(desc) = (unsafe { output.GetDesc() }) else { continue };
            let Some(isect) = intersect(area, &desc.DesktopCoordinates).filter(|_| desc.AttachedToDesktop.as_bool()) else { continue };
            let Some(hdr) = output_hdr(&output) else { continue };
            match grab_output(&adapter, &output, &desc.DesktopCoordinates, isect, hdr) {
                Some(img) => out.push((isect, img)),
                None => eprintln!("rustshot: could not read the HDR display in FP16; captured as is"),
            }
        }
    }
    out
}

fn grab_output(adapter: &IDXGIAdapter1, output: &IDXGIOutput, desk: &RECT, isect: IRect, hdr: Hdr) -> Option<PixBuf> {
    let (dev, ctx) = device(adapter).ok()?;
    let dup = duplicate_fp16(output, &dev)?;
    let (ix, iy, iw, ih) = isect;
    let stg = fp16_staging(&dev, iw, ih).ok()?;
    // The first frame of a new duplication is the whole desktop; a
    // display that presents nothing may take a moment.
    let until = Instant::now() + Duration::from_millis(500);
    let px = loop {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res = None;
        let left = until.saturating_duration_since(Instant::now()).as_millis() as u32;
        if unsafe { dup.AcquireNextFrame(left.max(1), &mut info, &mut res) }.is_err() {
            return None;
        }
        let tex = res.and_then(|r| r.cast::<ID3D11Texture2D>().ok()).filter(|_| info.LastPresentTime != 0);
        if let Some(tex) = tex {
            let bx = D3D11_BOX {
                left: (ix - desk.left) as u32,
                top: (iy - desk.top) as u32,
                front: 0,
                right: (ix - desk.left) as u32 + iw,
                bottom: (iy - desk.top) as u32 + ih,
                back: 1,
            };
            unsafe { ctx.CopySubresourceRegion(&stg, 0, 0, 0, 0, &tex, 0, Some(&bx)) };
            let _ = unsafe { dup.ReleaseFrame() };
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe { ctx.Map(&stg, 0, D3D11_MAP_READ, 0, Some(&mut m)) }.ok()?;
            let px = unsafe { read_fp16(&m, iw, ih) };
            unsafe { ctx.Unmap(&stg, 0) };
            break px;
        }
        let _ = unsafe { dup.ReleaseFrame() };
        if Instant::now() >= until {
            return None;
        }
    };
    let tm = ToneMap::new(hdr.white_nits, super::peak_of(&px));
    let mut rgba = vec![0u8; iw as usize * ih as usize * 4];
    tm.row(&px, &mut rgba, false);
    PixBuf::from_raw(iw, ih, rgba)
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

    /// The SDR white of every display, and each output's HDR state: what
    /// a live HDR test would need (prints; asserts only sane values).
    #[test]
    fn live_outputs_report_their_hdr_state() {
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
                if let Some(h) = hdr {
                    // An HDR display here: one FP16 frame, tone-mapped.
                    let r = d.DesktopCoordinates;
                    let shots = grab((r.left, r.top, 64, 64));
                    assert!(shots.iter().all(|(_, p)| p.dimensions() == (64, 64)), "{h:?}");
                }
            }
        }
    }
}
