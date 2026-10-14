//! QR codes, hand-rolled (ISO/IEC 18004): an encoder for the Make QR code
//! window (byte mode, level M, versions 1–40, the mask of least penalty)
//! and a decoder for Read QR code (any number of codes in a picture,
//! rotated or perspective-skewed; numeric, alphanumeric, byte and ECI
//! segments; Kanji segments are skipped).

// macOS has no Make QR code window (yet): the encoder is the tests' there.
#![cfg_attr(target_os = "macos", allow(dead_code))]

pub mod detect;
pub mod encode;
pub mod gf;
pub mod matrix;
pub mod tables;

#[cfg(test)]
mod tests;

pub use encode::{QrCode, encode_text};
pub use tables::Ecl;

use crate::pixbuf::PixBuf;

/// The texts of every QR code in `img`, in reading order.
pub fn read(img: &PixBuf) -> Vec<String> {
    let (w, h) = img.dimensions();
    detect::decode_rgba(w as usize, h as usize, img.as_raw()).into_iter().map(|f| f.content.text).collect()
}

/// A code as a picture: `scale` px per module, `quiet` light modules
/// around it, black on white.
pub fn render(code: &QrCode, scale: u32, quiet: u32) -> PixBuf {
    let n = code.size() as u32;
    let side = (n + 2 * quiet) * scale.max(1);
    let mut img = PixBuf::from_raw(side, side, vec![255u8; (side as usize).pow(2) * 4]).unwrap_or_default();
    let s = scale.max(1) as usize;
    let side = side as usize;
    let d = img.as_raw_mut();
    for y in 0..code.size() {
        for x in 0..code.size() {
            if !code.get(x, y) {
                continue;
            }
            let (px, py) = ((x + quiet as usize) * s, (y + quiet as usize) * s);
            for yy in py..py + s {
                for p in d[(yy * side + px) * 4..(yy * side + px + s) * 4].as_chunks_mut::<4>().0 {
                    *p = [0, 0, 0, 255];
                }
            }
        }
    }
    img
}

/// Whether `s` is a link "Open link" may offer: http or https only, no
/// spaces or control characters.
pub fn openable_link(s: &str) -> Option<&str> {
    let t = s.trim();
    let lower = t.get(..8).unwrap_or(t).to_ascii_lowercase();
    let ok_scheme = lower.starts_with("https://") || lower.starts_with("http://");
    let host_ok = t.split_once("://").is_some_and(|(_, rest)| !rest.is_empty() && !rest.starts_with('/'));
    if ok_scheme && host_ok && t.len() <= 2048 && !t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Some(t)
    } else {
        None
    }
}
