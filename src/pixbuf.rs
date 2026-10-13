//! Minimal RGBA8 image buffer with PNG codec, replacing the `image` crate.

use anyhow::{Context, Result};

/// `Default` is the empty 0x0 buffer (a placeholder after `mem::take`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PixBuf {
    w: u32,
    h: u32,
    d: Vec<u8>,
}

impl PixBuf {
    pub fn new(w: u32, h: u32) -> Self {
        PixBuf {
            w,
            h,
            d: vec![0; (w as usize) * (h as usize) * 4],
        }
    }

    #[cfg(test)]
    pub fn from_pixel(w: u32, h: u32, px: [u8; 4]) -> Self {
        let mut p = Self::new(w, h);
        for c in p.d.as_chunks_mut::<4>().0 {
            c.copy_from_slice(&px);
        }
        p
    }

    pub fn from_raw(w: u32, h: u32, d: Vec<u8>) -> Option<Self> {
        if w == 0 || h == 0 || d.len() != (w as usize) * (h as usize) * 4 {
            return None;
        }
        Some(PixBuf { w, h, d })
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    pub fn as_raw(&self) -> &Vec<u8> {
        &self.d
    }

    pub fn as_raw_mut(&mut self) -> &mut Vec<u8> {
        &mut self.d
    }

    #[cfg(test)]
    pub fn get_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.w as usize + x as usize) * 4;
        [self.d[i], self.d[i + 1], self.d[i + 2], self.d[i + 3]]
    }

    /// Copy a rect out of the buffer (bounds are the caller's contract).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> PixBuf {
        debug_assert!(x + w <= self.w && y + h <= self.h);
        let mut out = PixBuf::new(w, h);
        let stride = w as usize * 4;
        for row in 0..h as usize {
            let s = ((y as usize + row) * self.w as usize + x as usize) * 4;
            let d = row * stride;
            out.d[d..d + stride].copy_from_slice(&self.d[s..s + stride]);
        }
        out
    }

    /// The buffer as a PNG file (an empty buffer is an error).
    pub fn to_png(&self) -> Result<Vec<u8>> {
        Ok(crate::pngfile::encode(self.w, self.h, crate::pngfile::Color::Rgba, &self.d)?)
    }

    /// Decode a PNG (any color type and depth) into RGBA.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // the Screenshot portal's file
    pub fn from_png(bytes: &[u8]) -> Result<PixBuf> {
        let im = crate::pngfile::decode(bytes)?;
        PixBuf::from_raw(im.w, im.h, im.rgba).context("png: bad size")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trips() {
        let mut p = PixBuf::from_pixel(3, 2, [10, 20, 30, 255]);
        p.as_raw_mut()[4..8].copy_from_slice(&[200, 100, 50, 128]);
        let back = PixBuf::from_png(&p.to_png().unwrap()).unwrap();
        assert_eq!(back, p);
        assert!(PixBuf::from_png(b"not a png").is_err());
    }

    #[test]
    fn rgb_png_decodes_opaque() {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 1); // the png crate (a dev-dependency)
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&[1, 2, 3, 4, 5, 6]).unwrap();
        }
        let p = PixBuf::from_png(&out).unwrap();
        assert_eq!(p.as_raw(), &vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
