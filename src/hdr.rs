//! HDR screenshots: an HDR monitor's desktop read as scRGB (linear BT.709,
//! 1.0 = 80 nits, FP16) and tone-mapped to sRGB 8-bit.
//!
//! The mapping, per pixel: scale by the SDR white level (Settings ›
//! Display › "SDR content brightness", `DisplayConfigGetDeviceInfo`
//! `DISPLAYCONFIG_SDR_WHITE_LEVEL`), so SDR white is 1.0; then, only when
//! the picture holds highlights above that white, compress luminance above
//! [`KNEE`] with an extended Reinhard curve that takes the brightest value
//! ([`ToneMap::peak`]) to 1.0 (a picture with none is left as it is, SDR
//! white staying 255); scale the color by the new luminance over the old,
//! desaturate what still runs past 1.0 toward its luminance (the hue is
//! kept), and encode with the sRGB OETF.
//!
//! The math is here and tested everywhere; reading the monitor (DXGI
//! Desktop Duplication in FP16, `IDXGIOutput5::DuplicateOutput1`) is in
//! `hdr_win.rs` for screenshots and in `rec/win/dxgi.rs` for recordings.
//! Linux and macOS: nothing to do (the system hands over SDR pixels).
//!
//! Settings › Capture "HDR screenshots" (`hdr_screenshots`): "tonemap"
//! (the default) or "keep" (the pixels as GDI and the legacy duplication
//! hand them over). [`configure`] sets it for the process at every
//! capture; recordings follow the last capture's setting.

use std::sync::atomic::{AtomicBool, Ordering};

/// Luminance (in SDR-white units) below which the curve is the identity.
pub const KNEE: f32 = 0.9;

/// scRGB's 1.0, in nits.
pub const SCRGB_NITS: f32 = 80.0;

/// The SDR white level Windows uses when it cannot say (nits).
pub const DEFAULT_WHITE_NITS: f32 = 80.0;

static TONEMAP: AtomicBool = AtomicBool::new(true);

/// Take Settings' choice (`hdr_screenshots`) for this process.
pub fn configure(cfg: &crate::config::Config) {
    TONEMAP.store(cfg.hdr_screenshots.trim() != "keep", Ordering::Relaxed);
}

/// HDR monitors are read in FP16 and tone-mapped (else as captured).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn enabled() -> bool {
    TONEMAP.load(Ordering::Relaxed)
}

/// IEEE half to f32 (every value: subnormals, infinities, NaN).
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1F) as u32;
    let man = (h & 0x3FF) as u32;
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, m) => {
            // Subnormal: normalise it.
            let mut e = 127 - 15 + 1;
            let mut m = m;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3FF) << 13)
        }
        (0x1F, m) => sign | 0x7F80_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

/// BT.709 luminance of linear RGB.
pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The sRGB OETF of linear `v` (clamped to 0..1).
pub fn srgb_oetf(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Entries of the OETF table (over 0..1).
const LUT_N: usize = 16384;

fn oetf_lut() -> &'static [u8; LUT_N] {
    static LUT: std::sync::OnceLock<Box<[u8; LUT_N]>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = Box::new([0u8; LUT_N]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = (srgb_oetf(i as f32 / (LUT_N - 1) as f32) * 255.0).round() as u8;
        }
        t
    })
}

/// One picture's (or recording's) tone mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneMap {
    /// scRGB value of SDR white (white nits / 80).
    pub white: f32,
    /// The brightest luminance to keep apart, in SDR-white units (1.0:
    /// no highlights, the curve is the identity).
    pub peak: f32,
}

impl ToneMap {
    /// For SDR white at `white_nits` and the brightest scRGB luminance
    /// `peak` (of the picture, or the display's maximum for a recording).
    pub fn new(white_nits: f32, peak: f32) -> ToneMap {
        let white = (if white_nits.is_finite() && white_nits > 0.0 { white_nits } else { DEFAULT_WHITE_NITS } / SCRGB_NITS).max(0.1);
        let peak = if peak.is_finite() { (peak / white).max(1.0) } else { 1.0 };
        ToneMap { white, peak }
    }

    /// The luminance curve (SDR-white units in, 0..1 out).
    pub fn curve(&self, y: f32) -> f32 {
        if y <= KNEE || self.peak <= 1.0 {
            return y.min(1.0);
        }
        let x = (y - KNEE) / (1.0 - KNEE);
        let xw = (self.peak - KNEE) / (1.0 - KNEE);
        let f = x * (1.0 + x / (xw * xw)) / (1.0 + x);
        (KNEE + (1.0 - KNEE) * f).min(1.0)
    }

    /// Linear 0..1 BT.709 of scRGB `c`.
    pub fn linear(&self, c: [f32; 3]) -> [f32; 3] {
        let v = c.map(|x| if x.is_finite() { x.max(0.0) / self.white } else { 0.0 });
        let y = luminance(v);
        if y <= 0.0 {
            return [0.0; 3];
        }
        let k = self.curve(y) / y;
        let mut o = v.map(|x| x * k);
        let m = o[0].max(o[1]).max(o[2]);
        if m > 1.0 {
            // Past white in a channel: toward gray at the same luminance.
            let yo = luminance(o).min(1.0);
            let t = if m > yo { (1.0 - yo) / (m - yo) } else { 0.0 };
            o = o.map(|x| yo + (x - yo) * t);
        }
        o.map(|x| x.clamp(0.0, 1.0))
    }

    /// sRGB 8-bit of scRGB `c` (exact OETF; tests and single pixels).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn srgb8(&self, c: [f32; 3]) -> [u8; 3] {
        self.linear(c).map(|x| (srgb_oetf(x) * 255.0).round() as u8)
    }

    /// Tone-map a row of `R16G16B16A16_FLOAT` pixels into 8-bit pixels,
    /// RGBA (`bgra` false) or BGRA; alpha 255.
    pub fn row(&self, src: &[u16], dst: &mut [u8], bgra: bool) {
        let lut = oetf_lut();
        for (s, d) in src.as_chunks::<4>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
            let l = self.linear([f16_to_f32(s[0]), f16_to_f32(s[1]), f16_to_f32(s[2])]);
            let q = l.map(|x| lut[(x * (LUT_N - 1) as f32 + 0.5) as usize]);
            *d = if bgra { [q[2], q[1], q[0], 255] } else { [q[0], q[1], q[2], 255] };
        }
    }
}

/// The brightest scRGB luminance among `R16G16B16A16_FLOAT` pixels.
pub fn peak_of(src: &[u16]) -> f32 {
    src.as_chunks::<4>()
        .0
        .iter()
        .map(|s| luminance([f16_to_f32(s[0]), f16_to_f32(s[1]), f16_to_f32(s[2])]))
        .filter(|y| y.is_finite())
        .fold(0.0, f32::max)
}

#[cfg(windows)]
#[path = "hdr_win.rs"]
pub mod win;

#[cfg(test)]
mod tests {
    use super::*;

    /// f32 to the nearest half (finite, normal range): for building test
    /// pixels.
    fn h(v: f32) -> u16 {
        if v == 0.0 {
            return 0;
        }
        let b = v.to_bits();
        let sign = ((b >> 16) & 0x8000) as u16;
        let e = ((b >> 23) & 0xFF) as i32 - 127 + 15;
        let m = ((b >> 13) & 0x3FF) as u16 + (((b >> 12) & 1) as u16);
        sign | ((e as u16) << 10).wrapping_add(m)
    }

    #[test]
    fn halves_decode() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xC000), -2.0);
        assert_eq!(f16_to_f32(0x7BFF), 65504.0);
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24), "smallest subnormal");
        assert_eq!(f16_to_f32(0x03FF), 1023.0 * 2f32.powi(-24), "largest subnormal");
        assert_eq!(f16_to_f32(0x3555), 0.333_251_95);
        assert!(f16_to_f32(0x7C00).is_infinite() && f16_to_f32(0x7E00).is_nan());
        assert_eq!(f16_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
        for v in [0.5f32, 1.0, 2.5, 12.5, 0.25] {
            assert_eq!(f16_to_f32(h(v)), v);
        }
    }

    #[test]
    fn sdr_content_maps_exactly_at_the_white_level() {
        // SDR white at 80 nits: scRGB 1.0 is white, 0.5 is sRGB 0.7354.
        let t = ToneMap::new(80.0, 1.0);
        assert_eq!(t.srgb8([0.0, 0.0, 0.0]), [0, 0, 0]);
        assert_eq!(t.srgb8([1.0, 1.0, 1.0]), [255, 255, 255]);
        assert_eq!(t.srgb8([0.5, 0.5, 0.5]), [188, 188, 188]);
        assert_eq!(t.srgb8([1.0, 0.0, 0.0]), [255, 0, 0]);
        // SDR white at 240 nits (scRGB 3.0): 3.0 is white, 1.5 is linear
        // 0.5 (sRGB 0.7354 = 188), and the picture's own peak at white
        // changes nothing.
        let t = ToneMap::new(240.0, 3.0);
        assert_eq!(t.white, 3.0);
        assert_eq!(t.srgb8([3.0, 3.0, 3.0]), [255, 255, 255]);
        assert_eq!(t.srgb8([1.5, 1.5, 1.5]), [188, 188, 188]);
        assert_eq!(t.srgb8([0.6, 1.2, 2.4]), [124, 170, 231]);
        // Negative (outside BT.709) channels and NaN are black there.
        assert_eq!(t.srgb8([-0.5, 3.0, f32::NAN]), [0, 255, 0]);
    }

    #[test]
    fn highlights_compress_to_white_above_the_knee() {
        // White 200 nits (2.5), highlights to 1000 nits (12.5 = 5x white).
        let t = ToneMap::new(200.0, 12.5);
        assert_eq!(t.peak, 5.0);
        assert_eq!(t.curve(0.5), 0.5, "below the knee: as is");
        assert_eq!(t.curve(KNEE), KNEE);
        assert!((t.curve(5.0) - 1.0).abs() < 1e-6, "the peak is white");
        assert_eq!(t.curve(9.0), 1.0, "past the peak: white");
        let w = t.curve(1.0);
        assert!(w > KNEE && w < 0.96, "SDR white just under 1: {w}");
        // Monotonic, continuous at the knee.
        let mut last = 0.0;
        for i in 0..=600 {
            let y = i as f32 / 100.0;
            let c = t.curve(y);
            assert!(c >= last - 1e-6, "{y}: {c} < {last}");
            last = c;
        }
        assert!((t.curve(KNEE + 1e-4) - KNEE).abs() < 1e-3);
        assert_eq!(t.srgb8([12.5, 12.5, 12.5]), [255, 255, 255]);
        assert_eq!(t.srgb8([0.5, 0.5, 0.5]), [124, 124, 124], "dark tones as in SDR");
        // A bright red keeps its hue: red most, green and blue equal.
        let [r, g, b] = t.srgb8([12.0, 1.0, 1.0]);
        assert!(r == 255 && g == b && g < r, "{r} {g} {b}");
    }

    #[test]
    fn rows_match_the_exact_mapping_within_a_level() {
        let t = ToneMap::new(160.0, 8.0);
        let vals: Vec<f32> = (0..200).map(|i| i as f32 * 0.05).collect();
        let src: Vec<u16> = vals.iter().flat_map(|&v| [h(v), h(v * 0.5), h(v * 0.25), h(1.0)]).collect();
        let mut rgba = vec![0u8; vals.len() * 4];
        let mut bgra = vec![0u8; vals.len() * 4];
        t.row(&src, &mut rgba, false);
        t.row(&src, &mut bgra, true);
        for (i, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let c = [f16_to_f32(src[i * 4]), f16_to_f32(src[i * 4 + 1]), f16_to_f32(src[i * 4 + 2])];
            let want = t.srgb8(c);
            for k in 0..3 {
                assert!(px[k].abs_diff(want[k]) <= 1, "{i}: {px:?} vs {want:?}");
            }
            assert_eq!(px[3], 255);
            assert_eq!(bgra[i * 4..i * 4 + 4], [px[2], px[1], px[0], 255]);
        }
        assert!((peak_of(&src) - luminance([9.95, 4.975, 2.4875])).abs() < 0.02);
    }

    #[test]
    fn the_setting_switches_the_process() {
        let c = crate::config::Config { hdr_screenshots: "keep".into(), ..Default::default() };
        configure(&c);
        assert!(!enabled());
        configure(&crate::config::Config::default());
        assert!(enabled());
    }
}
