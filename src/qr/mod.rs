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

/// A link's parts as the toast shows them: `head` is the scheme and host
/// (with its port) in full, the host in ASCII (punycode for an IDN);
/// `unicode` the head with the host as typed, when that differs; `rest`
/// the path, query and fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkParts<'a> {
    pub head: String,
    pub unicode: Option<String>,
    pub rest: &'a str,
}

/// `s` as a link "Open link" may offer: `http://` or `https://`, a host
/// (letters, digits, `-`, `.`, any non-ASCII letters; or an IPv4/IPv6
/// literal), an optional port, no user info (`user@`), no spaces or
/// control characters, at most 2048 bytes.
pub fn link_parts(s: &str) -> Option<LinkParts<'_>> {
    let t = s.trim();
    if t.len() > 2048 || t.chars().any(|c| c.is_whitespace() || c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c) || ('\u{2066}'..='\u{2069}').contains(&c)) {
        return None;
    }
    let (scheme, after) = t.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let (auth, rest) = after.split_at(end);
    if auth.is_empty() || auth.contains('@') {
        return None;
    }
    let (host, port) = if let Some(v6) = auth.strip_prefix('[') {
        let (h, p) = v6.split_once(']')?;
        if h.is_empty() || !h.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return None;
        }
        (format!("[{}]", h.to_ascii_lowercase()), p)
    } else {
        let (h, p) = match auth.rfind(':') {
            Some(i) => (&auth[..i], &auth[i..]),
            None => (auth, ""),
        };
        (h.to_string(), p)
    };
    if !port.is_empty() {
        let digits = port.strip_prefix(':')?;
        if digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.parse::<u32>().ok()? > 65535 {
            return None;
        }
    }
    let (ascii, unicode) = if host.starts_with('[') {
        (host.clone(), None)
    } else {
        if host.is_empty() || host.starts_with('.') || host.contains("..") {
            return None;
        }
        let mut labels = Vec::new();
        let mut idn = false;
        for label in host.trim_end_matches('.').split('.') {
            if label.is_empty() || label.len() > 63 {
                return None;
            }
            let lower = label.to_lowercase();
            if lower.is_ascii() {
                if !lower.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                    return None;
                }
                labels.push(lower);
            } else {
                if !lower.chars().all(|c| c.is_alphanumeric() || c == '-') {
                    return None;
                }
                idn = true;
                labels.push(format!("xn--{}", punycode(&lower)?));
            }
        }
        let a = labels.join(".");
        (a, idn.then(|| host.to_lowercase()))
    };
    let head = format!("{scheme}://{ascii}{port}");
    let unicode = unicode.map(|u| format!("{scheme}://{u}{port}"));
    Some(LinkParts { head, unicode, rest })
}

/// Whether `s` is a link "Open link" may offer ([`link_parts`]).
pub fn openable_link(s: &str) -> Option<&str> {
    link_parts(s).map(|_| s.trim())
}

/// RFC 3492 Punycode of one label (without the `xn--`).
pub fn punycode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const TMIN: u32 = 1;
    const TMAX: u32 = 26;
    let adapt = |mut delta: u32, num: u32, first: bool| {
        delta /= if first { 700 } else { 2 };
        delta += delta / num;
        let mut k = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k += BASE;
        }
        k + (BASE - TMIN + 1) * delta / (delta + 38)
    };
    let digit = |d: u32| (if d < 26 { b'a' + d as u8 } else { b'0' + (d - 26) as u8 }) as char;
    let cps: Vec<u32> = input.chars().map(|c| c as u32).collect();
    let mut out: String = input.chars().filter(|c| c.is_ascii()).collect();
    let b = out.len() as u32;
    let mut h = b;
    if b > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
    while (h as usize) < cps.len() {
        let m = *cps.iter().filter(|&&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(h + 1)?)?;
        n = m;
        for &c in &cps {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias { TMIN } else if k >= bias + TMAX { TMAX } else { k - bias };
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                out.push(digit(q));
                bias = adapt(delta, h + 1, h == b);
                delta = 0;
                h += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    Some(out)
}