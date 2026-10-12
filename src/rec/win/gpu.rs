//! BGRA → NV12 on the GPU for the Desktop Duplication source: the D3D11
//! video processor scales (when the area is larger than the MP4 limit) and
//! converts (BT.709, limited range) in one blit.
//!
//! The desktop image of the area lives in a texture (`canvas`, no pointer)
//! that the duplication copies its new frames into. A frame is made by
//! copying it to `work`, drawing the pointer there, blitting `work` to an
//! NV12 texture of the encoder's size and reading that back through a
//! staging texture (one map per frame). The pointer is composited on the
//! CPU by the same code as the CPU path: the few pixels under it are read
//! back, drawn on and written back to `work`.

use crate::capture::IRect;
use anyhow::{bail, Context, Result};
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT,
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};

/// Draws the pointer on the tight BGRA pixels of a box (virtual-screen px).
pub(super) type DrawPointer<'a> = &'a dyn Fn(&mut [u8], IRect);

pub(super) struct Gpu {
    pub device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    vctx: ID3D11VideoContext,
    vp: ID3D11VideoProcessor,
    /// The desktop image of the area (no pointer).
    pub canvas: ID3D11Texture2D,
    /// `canvas` plus the pointer: the video processor's input.
    work: ID3D11Texture2D,
    input: ID3D11VideoProcessorInputView,
    output: ID3D11VideoProcessorOutputView,
    nv12: ID3D11Texture2D,
    readback: ID3D11Texture2D,
    /// Staging texture for the pixels under the pointer, and its size.
    patch: Option<(ID3D11Texture2D, u32, u32)>,
    patch_px: Vec<u8>,
    area: IRect,
    pub out: (u32, u32),
}

fn texture(device: &ID3D11Device, w: u32, h: u32, format: DXGI_FORMAT, usage: D3D11_USAGE, bind: u32, cpu: u32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex)) }.with_context(|| format!("create a {w}x{h} texture"))?;
    tex.context("no texture")
}

/// What part of a `w`×`h` area becomes the `ow`×`oh` picture: all of it,
/// or (sizes rounded down to even, no scaling) the top-left `ow`×`oh`, as
/// the CPU conversion does.
fn source_rect(w: u32, h: u32, ow: u32, oh: u32) -> RECT {
    let crop = ow <= w && oh <= h && w - ow <= 1 && h - oh <= 1;
    let (sw, sh) = if crop { (ow, oh) } else { (w, h) };
    RECT { left: 0, top: 0, right: sw as i32, bottom: sh as i32 }
}

impl Gpu {
    /// A converter of `area` (its size) to `out` NV12 on `device`.
    pub fn new(device: &ID3D11Device, ctx: &ID3D11DeviceContext, area: IRect, out: (u32, u32)) -> Result<Gpu> {
        let (aw, ah) = (area.2, area.3);
        let (ow, oh) = out;
        if ow < 2 || oh < 2 || !ow.is_multiple_of(2) || !oh.is_multiple_of(2) {
            bail!("NV12 size must be even");
        }
        let vdev: ID3D11VideoDevice = device.cast().context("no video device (D3D11 video support)")?;
        let vctx: ID3D11VideoContext = ctx.cast().context("no video context")?;
        let rate = DXGI_RATIONAL { Numerator: 60, Denominator: 1 };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: aw,
            InputHeight: ah,
            OutputFrameRate: rate,
            OutputWidth: ow,
            OutputHeight: oh,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_QUALITY,
        };
        unsafe {
            let en = vdev.CreateVideoProcessorEnumerator(&desc).context("create a video processor enumerator")?;
            let fin = en.CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM)?;
            let fout = en.CheckVideoProcessorFormat(DXGI_FORMAT_NV12)?;
            if fin & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32 == 0 {
                bail!("the video processor takes no BGRA input");
            }
            if fout & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 == 0 {
                bail!("the video processor gives no NV12 output");
            }
            let vp = vdev.CreateVideoProcessor(&en, 0).context("create a video processor")?;
            let rt = D3D11_BIND_RENDER_TARGET.0 as u32;
            let canvas = texture(device, aw, ah, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_DEFAULT, 0, 0)?;
            let work = texture(device, aw, ah, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_DEFAULT, rt, 0)?;
            let nv12 = texture(device, ow, oh, DXGI_FORMAT_NV12, D3D11_USAGE_DEFAULT, rt, 0)?;
            let read = D3D11_CPU_ACCESS_READ.0 as u32;
            let readback = texture(device, ow, oh, DXGI_FORMAT_NV12, D3D11_USAGE_STAGING, 0, read)?;
            let ivd = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
            };
            let mut input = None;
            vdev.CreateVideoProcessorInputView(&work, &en, &ivd, Some(&mut input)).context("create the input view")?;
            let ovd = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            };
            let mut output = None;
            vdev.CreateVideoProcessorOutputView(&nv12, &en, &ovd, Some(&mut output)).context("create the output view")?;
            // Full-range RGB in, BT.709 limited-range YCbCr out.
            if let Ok(v1) = vctx.cast::<ID3D11VideoContext1>() {
                v1.VideoProcessorSetStreamColorSpace1(&vp, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
                v1.VideoProcessorSetOutputColorSpace1(&vp, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);
            } else {
                // Bits: Usage, RGB_Range (0: 0-255), YCbCr_Matrix (1: BT.709),
                // YCbCr_xvYCC, Nominal_Range (2 bits; 1: 16-235).
                let rgb = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 };
                let yuv = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: (1 << 2) | (1 << 4) };
                vctx.VideoProcessorSetStreamColorSpace(&vp, 0, &rgb);
                vctx.VideoProcessorSetOutputColorSpace(&vp, &yuv);
            }
            vctx.VideoProcessorSetStreamFrameFormat(&vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            // No driver "enhancements" (denoise, edge, colour): screen content.
            vctx.VideoProcessorSetStreamAutoProcessingMode(&vp, 0, false);
            let src = source_rect(aw, ah, ow, oh);
            let dst = RECT { left: 0, top: 0, right: ow as i32, bottom: oh as i32 };
            vctx.VideoProcessorSetStreamSourceRect(&vp, 0, true, Some(&src));
            vctx.VideoProcessorSetStreamDestRect(&vp, 0, true, Some(&dst));
            vctx.VideoProcessorSetOutputTargetRect(&vp, true, Some(&dst));
            Ok(Gpu {
                device: device.clone(),
                ctx: ctx.clone(),
                vctx,
                vp,
                canvas,
                work,
                input: input.context("no input view")?,
                output: output.context("no output view")?,
                nv12,
                readback,
                patch: None,
                patch_px: Vec::new(),
                area,
                out,
            })
        }
    }

    /// Write `px` (tight BGRA rows) to `r` (virtual-screen px, inside the
    /// area) of the canvas.
    pub fn upload(&self, r: IRect, px: &[u8]) {
        if r.2 == 0 || r.3 == 0 || px.len() < r.2 as usize * r.3 as usize * 4 {
            return;
        }
        let (x, y) = ((r.0 - self.area.0) as u32, (r.1 - self.area.1) as u32);
        let bx = D3D11_BOX { left: x, top: y, front: 0, right: x + r.2, bottom: y + r.3, back: 1 };
        unsafe { self.ctx.UpdateSubresource(&self.canvas, 0, Some(&bx), px.as_ptr().cast(), r.2 * 4, 0) };
    }

    /// The canvas as NV12 of the output size, with `pointer` (a box in
    /// virtual-screen px inside the area, and how to draw on the tight
    /// BGRA pixels of that box) composited first.
    pub fn convert(&mut self, pointer: Option<(IRect, DrawPointer)>) -> Result<Vec<u8>> {
        unsafe {
            self.ctx.CopyResource(&self.work, &self.canvas);
            if let Some((r, draw)) = pointer
                && r.2 > 0
                && r.3 > 0
            {
                self.draw_pointer(r, draw)?;
            }
            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(Some(self.input.clone())),
                ..Default::default()
            };
            let mut streams = [stream];
            let r = self.vctx.VideoProcessorBlt(&self.vp, &self.output, 0, &streams);
            ManuallyDrop::drop(&mut streams[0].pInputSurface);
            r.context("video processor blit")?;
            self.ctx.CopyResource(&self.readback, &self.nv12);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&self.readback, 0, D3D11_MAP_READ, 0, Some(&mut m)).context("map the NV12 picture")?;
            let (ow, oh) = (self.out.0 as usize, self.out.1 as usize);
            let pitch = m.RowPitch as usize;
            let mut out = Vec::with_capacity(ow * oh * 3 / 2);
            let base = m.pData as *const u8;
            // Y rows, then the U/V plane, which follows at `pitch * height`.
            for row in 0..oh + oh / 2 {
                out.extend_from_slice(std::slice::from_raw_parts(base.add(row * pitch), ow));
            }
            self.ctx.Unmap(&self.readback, 0);
            Ok(out)
        }
    }

    /// Read `r` of `work` back, `draw` on it, write it back.
    unsafe fn draw_pointer(&mut self, r: IRect, draw: DrawPointer) -> Result<()> {
        unsafe {
            let (w, h) = (r.2, r.3);
            if self.patch.as_ref().is_none_or(|p| p.1 < w || p.2 < h) {
                let (pw, ph) = (w.next_power_of_two().max(64), h.next_power_of_two().max(64));
                let cpu = (D3D11_CPU_ACCESS_READ.0 | D3D11_CPU_ACCESS_WRITE.0) as u32;
                let t = texture(&self.device, pw, ph, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_STAGING, 0, cpu)?;
                self.patch = Some((t, pw, ph));
            }
            let Some((patch, _, _)) = &self.patch else { return Ok(()) };
            let (x, y) = ((r.0 - self.area.0) as u32, (r.1 - self.area.1) as u32);
            let bx = D3D11_BOX { left: x, top: y, front: 0, right: x + w, bottom: y + h, back: 1 };
            self.ctx.CopySubresourceRegion(patch, 0, 0, 0, 0, &self.work, 0, Some(&bx));
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(patch, 0, D3D11_MAP_READ, 0, Some(&mut m)).context("map the pointer patch")?;
            let row = w as usize * 4;
            self.patch_px.clear();
            for k in 0..h as usize {
                let src = std::slice::from_raw_parts((m.pData as *const u8).add(k * m.RowPitch as usize), row);
                self.patch_px.extend_from_slice(src);
            }
            self.ctx.Unmap(patch, 0);
            draw(&mut self.patch_px, r);
            self.ctx.UpdateSubresource(&self.work, 0, Some(&bx), self.patch_px.as_ptr().cast(), w * 4, 0);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_rect_crops_odd_sizes_and_scales_otherwise() {
        let r = |x: RECT| (x.right, x.bottom);
        assert_eq!(r(source_rect(321, 241, 320, 240)), (320, 240));
        assert_eq!(r(source_rect(320, 240, 320, 240)), (320, 240));
        assert_eq!(r(source_rect(5120, 1440, 4096, 1152)), (5120, 1440));
        assert_eq!(r(source_rect(5121, 1440, 4096, 1152)), (5121, 1440));
    }
}
