//! HDR screenshots: an HDR monitor's desktop read as scRGB (linear BT.709,
//! 1.0 = 80 nits, FP16) and tone-mapped to sRGB 8-bit.
//!
//! The mapping, per pixel: scale by the SDR white level (Settings ›
//! Display › "SDR content brightness", `DisplayConfigGetDeviceInfo`
//! `DISPLAYCONFIG_SDR_WHITE_LEVEL`), so SDR white is 1.0; everything up
//! to that white is kept as it is (SDR content, and SDR white itself,
//! come out exactly as on an SDR display: 255 is 255, on every monitor of
//! a span, whatever else is in the picture); luminance above it is
//! clamped to white, and a color that still runs past 1.0 in a channel is
//! desaturated toward its luminance (its hue is kept); then the sRGB OETF.
//! Channels below 0 (wide-gamut colors outside BT.709) are clamped to 0,
//! which shifts very saturated P3 / BT.2020 colors a little.
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

#![cfg_attr(not(windows), allow(dead_code))] // Linux, macOS: the setting only

use std::sync::atomic::{AtomicBool, Ordering};

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

/// Every half's f32 (65536 entries, built once).
fn half_lut() -> &'static [f32] {
    static LUT: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| (0..=u16::MAX).map(f16_to_f32).collect())
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

/// One display's tone mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneMap {
    /// scRGB value of SDR white (white nits / 80).
    pub white: f32,
}

/// Pixels below which [`ToneMap::rows`] stays on the calling thread.
const PAR_MIN: usize = 1 << 18;

impl ToneMap {
    /// For SDR white at `white_nits` (80 when it is not a usable number).
    pub fn new(white_nits: f32) -> ToneMap {
        let white = (if white_nits.is_finite() && white_nits > 0.0 { white_nits } else { DEFAULT_WHITE_NITS } / SCRGB_NITS).max(0.1);
        ToneMap { white }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    /// The luminance curve (SDR-white units in, 0..1 out): the identity
    /// up to SDR white, white above it.
    pub fn curve(&self, y: f32) -> f32 {
        y.min(1.0)
    }

    /// Linear 0..1 BT.709 of scRGB `c`.
    pub fn linear(&self, c: [f32; 3]) -> [f32; 3] {
        let v = c.map(|x| if x.is_finite() { x.max(0.0) / self.white } else { 0.0 });
        let y = luminance(v);
        if y <= 0.0 {
            return [0.0; 3];
        }
        let mut o = if y > 1.0 { v.map(|x| x / y) } else { v };
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
        let (lut, half) = (oetf_lut(), half_lut());
        let k = (LUT_N - 1) as f32 / self.white;
        let top = (LUT_N - 1) as f32;
        for (s, d) in src.as_chunks::<4>().0.iter().zip(dst.as_chunks_mut::<4>().0) {
            let (r, g, b) = (half[s[0] as usize] * k, half[s[1] as usize] * k, half[s[2] as usize] * k);
            // Most pixels are SDR (every channel within 0..white): straight
            // to the OETF table; the rest (highlights, negatives, NaN) take
            // the full mapping.
            let q = if (0.0..=top).contains(&r) && (0.0..=top).contains(&g) && (0.0..=top).contains(&b) {
                [lut[(r + 0.5) as usize], lut[(g + 0.5) as usize], lut[(b + 0.5) as usize]]
            } else {
                self.linear([half[s[0] as usize], half[s[1] as usize], half[s[2] as usize]]).map(|x| lut[(x * top + 0.5) as usize])
            };
            *d = if bgra { [q[2], q[1], q[0], 255] } else { [q[0], q[1], q[2], 255] };
        }
    }

    /// [`ToneMap::row`] over many (source, destination) rows, split
    /// across up to four threads when there are enough pixels.
    pub fn rows(&self, rows: Vec<(&[u16], &mut [u8])>, bgra: bool) {
        let px: usize = rows.iter().map(|r| r.0.len() / 4).sum();
        let n = std::thread::available_parallelism().map_or(1, |n| n.get()).min(4);
        if n < 2 || px < PAR_MIN || rows.len() < 2 * n {
            for (s, d) in rows {
                self.row(s, d, bgra);
            }
            return;
        }
        let mut rows = rows;
        let per = rows.len().div_ceil(n);
        std::thread::scope(|sc| {
            for part in rows.chunks_mut(per) {
                sc.spawn(move || {
                    for (s, d) in part.iter_mut() {
                        self.row(s, d, bgra);
                    }
                });
            }
        });
    }
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
        let t = ToneMap::new(80.0);
        assert_eq!(t.srgb8([0.0, 0.0, 0.0]), [0, 0, 0]);
        assert_eq!(t.srgb8([1.0, 1.0, 1.0]), [255, 255, 255]);
        assert_eq!(t.srgb8([0.5, 0.5, 0.5]), [188, 188, 188]);
        assert_eq!(t.srgb8([1.0, 0.0, 0.0]), [255, 0, 0]);
        // SDR white at 240 nits (scRGB 3.0): 3.0 is white, 1.5 is linear
        // 0.5 (sRGB 0.7354 = 188).
        let t = ToneMap::new(240.0);
        assert_eq!(t.white, 3.0);
        assert_eq!(t.srgb8([3.0, 3.0, 3.0]), [255, 255, 255]);
        assert_eq!(t.srgb8([1.5, 1.5, 1.5]), [188, 188, 188]);
        assert_eq!(t.srgb8([0.6, 1.2, 2.4]), [124, 170, 231]);
        // Negative (outside BT.709) channels and NaN are black there.
        assert_eq!(t.srgb8([-0.5, 3.0, f32::NAN]), [0, 255, 0]);
        assert_eq!(ToneMap::new(f32::NAN).white, 1.0);
    }

    /// Every SDR gray, shown on an HDR monitor (linear x SDR white in
    /// scRGB), comes back as the 8-bit value an SDR monitor shows: no
    /// seam where a span crosses from an SDR to an HDR monitor, and the
    /// same with or without highlights next to it in the row.
    #[test]
    fn sdr_values_round_trip_with_and_without_highlights() {
        let lin = |v: u8| {
            let c = v as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        for nits in [80.0f32, 200.0, 240.0, 480.0] {
            let t = ToneMap::new(nits);
            let mut plain = Vec::new();
            let mut lit = Vec::new();
            for v in 0..=255u8 {
                let x = h(lin(v) * t.white);
                plain.extend([x, x, x, h(1.0)]);
                lit.extend([x, x, x, h(1.0)]);
                lit.extend([h(40.0), h(40.0), h(40.0), h(1.0)]); // a 3200-nit highlight
            }
            let mut a = vec![0u8; plain.len()];
            let mut b = vec![0u8; lit.len()];
            t.row(&plain, &mut a, false);
            t.row(&lit, &mut b, false);
            for v in 0..=255usize {
                let got = a[v * 4];
                assert!(got.abs_diff(v as u8) <= 1, "{nits} nits: {v} -> {got}");
                assert_eq!(b[v * 8..v * 8 + 4], a[v * 4..v * 4 + 4], "the highlight changes nothing next to it");
                assert_eq!(b[v * 8 + 4..v * 8 + 8], [255, 255, 255, 255], "the highlight is white");
            }
            assert_eq!(a[255 * 4..], [255, 255, 255, 255], "SDR white is 255 at {nits} nits");
        }
    }

    #[test]
    fn highlights_go_to_white_keeping_their_hue() {
        // White 200 nits (2.5), highlights to 1000 nits (12.5 = 5x white).
        let t = ToneMap::new(200.0);
        assert_eq!(t.curve(0.5), 0.5, "below white: as is");
        assert_eq!(t.curve(1.0), 1.0, "SDR white stays white");
        assert_eq!(t.curve(5.0), 1.0, "above white: white");
        assert_eq!(t.srgb8([2.5, 2.5, 2.5]), [255, 255, 255]);
        assert_eq!(t.srgb8([12.5, 12.5, 12.5]), [255, 255, 255]);
        assert_eq!(t.srgb8([1.25, 1.25, 1.25]), [188, 188, 188], "mid tones as in SDR");
        // A bright red (past white in red, under it in luminance) keeps
        // its hue: red most, green and blue equal; past white in
        // luminance too it is white.
        let [r, g, b] = t.srgb8([6.0, 0.5, 0.5]);
        assert_eq!(t.srgb8([12.0, 1.0, 1.0]), [255, 255, 255]);
        assert!(r == 255 && g == b && g < r, "{r} {g} {b}");
        // A saturated red under white luminance but past 1.0 in red: red
        // stays the strongest and the luminance is kept.
        let o = t.linear([5.0, 0.0, 0.0]);
        assert!(o[0] == 1.0 && o[1] > 0.0 && (o[1] - o[2]).abs() < 1e-6, "{o:?}");
        assert!((luminance(o) - luminance([2.0, 0.0, 0.0])).abs() < 1e-4, "{o:?}");
    }

    #[test]
    fn rows_match_the_exact_mapping_within_a_level() {
        let t = ToneMap::new(160.0);
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
        assert_eq!(half_lut()[0x3C00], 1.0);
        assert!(half_lut().iter().enumerate().all(|(i, &v)| v.to_bits() == f16_to_f32(i as u16).to_bits()));
    }

    /// The threaded rows equal the single-thread ones.
    #[test]
    fn threaded_rows_match() {
        let t = ToneMap::new(240.0);
        let (w, hgt) = (1024usize, 300usize);
        let src: Vec<u16> = (0..w * hgt).flat_map(|i| {
            let v = (i % 997) as f32 * 0.01;
            [h(v), h(v * 0.7), h(v * 0.3), h(1.0)]
        }).collect();
        let mut one = vec![0u8; w * hgt * 4];
        for (s, d) in src.chunks(w * 4).zip(one.chunks_mut(w * 4)) {
            t.row(s, d, true);
        }
        let mut par = vec![0u8; w * hgt * 4];
        t.rows(src.chunks(w * 4).zip(par.chunks_mut(w * 4)).collect(), true);
        assert!(one == par);
    }

    /// Release-build timing of a full 4K FP16 frame (`cargo test --release
    /// bench_tonemap_4k -- --ignored --nocapture`).
    #[test]
    #[ignore = "measurement"]
    fn bench_tonemap_4k() {
        let t = ToneMap::new(240.0);
        let (w, hgt) = (3840usize, 2160usize);
        let mut dst = vec![0u8; w * hgt * 4];
        // A desktop: SDR up to 10 % past white (about 9 % highlights);
        // then a frame of nothing but highlights (the worst case).
        for (what, over) in [("desktop", 1.1f32), ("all highlights", 8.0)] {
        let src: Vec<u16> = (0..w * hgt).flat_map(|i| {
            let v = if over > 2.0 { 3.5 + (i % 4093) as f32 * 0.005 } else { (i % 4093) as f32 / 4093.0 * t.white * over };
            [h(v), h(v * 0.8), h(v * 0.5), h(1.0)]
        }).collect();
        for (label, par) in [("1 thread", false), ("threads", true)] {
            let t0 = std::time::Instant::now();
            let n = 5;
            for _ in 0..n {
                if par {
                    t.rows(src.chunks(w * 4).zip(dst.chunks_mut(w * 4)).collect(), true);
                } else {
                    for (s, d) in src.chunks(w * 4).zip(dst.chunks_mut(w * 4)) {
                        t.row(s, d, true);
                    }
                }
            }
            eprintln!("4K FP16 tone map, {what}, {label}: {:.1} ms a frame", t0.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
        }
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
