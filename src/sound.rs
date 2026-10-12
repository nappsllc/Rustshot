//! The shutter sound (`shutter_sound`): a short click synthesised once at
//! run time into a tiny in-memory WAV (16-bit mono, 22.05 kHz, ~70 ms,
//! about 3 KB), played without blocking. Windows: `PlaySoundW` from
//! memory. Linux: the PulseAudio simple API (`libpulse-simple.so.0`,
//! loaded with `dlopen` as the recorder does; silently nothing without
//! it). macOS: the system "Tink" sound through `NSSound`.

/// Sample rate of the click.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS plays a system sound
pub const RATE: u32 = 22_050;

/// The click's samples: two decaying noise bursts (the shutter opening
/// and closing), 70 ms in all. Deterministic (a fixed LCG), so the WAV is
/// the same every run.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn click_samples() -> Vec<i16> {
    let n = (RATE as usize * 70) / 1000;
    let mut seed: u32 = 0x1234_5678;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = ((seed >> 16) as f32 / 32_768.0) - 1.0;
        let t = i as f32 / RATE as f32;
        // Burst 1 at 0 ms, burst 2 at 38 ms; each decays in ~6 ms.
        let env = |t0: f32| if t >= t0 { (-(t - t0) / 0.006).exp() } else { 0.0 };
        let a = env(0.0) * 0.55 + env(0.038) * 0.4;
        out.push((noise * a * i16::MAX as f32) as i16);
    }
    out
}

/// `samples` as a RIFF/WAVE file (PCM, mono, 16-bit, [`RATE`]).
#[cfg_attr(not(windows), allow(dead_code))] // Linux streams the samples
pub fn wav(samples: &[i16]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// Play the shutter click, never blocking the caller.
pub fn shutter() {
    #[cfg(test)]
    {
        PLAYED.with(|p| p.set(p.get() + 1));
    }
    #[cfg(all(windows, not(test)))]
    win::play();
    #[cfg(all(target_os = "linux", not(test)))]
    std::thread::spawn(linux::play);
    #[cfg(all(target_os = "macos", not(test)))]
    mac::play();
}

#[cfg(test)]
thread_local! {
    static PLAYED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Tests: how many clicks this thread asked for since the last call.
#[cfg(test)]
pub fn take_played() -> u32 {
    PLAYED.with(|p| p.replace(0))
}

#[cfg(all(windows, not(test)))]
mod win {
    use std::sync::OnceLock;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    use windows::core::PCWSTR;

    /// The WAV lives for the whole run: `SND_ASYNC` reads it after return.
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();

    pub fn play() {
        let w = WAV.get_or_init(|| super::wav(&super::click_samples()));
        // SAFETY: SND_MEMORY takes the WAV image's address in place of a
        // name; the buffer is static.
        unsafe {
            let _ = PlaySoundW(PCWSTR(w.as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
mod linux {
    use crate::rec::linux::dl::Lib;
    use core::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    struct SampleSpec {
        format: c_int,
        rate: u32,
        channels: u8,
    }
    const PA_STREAM_PLAYBACK: c_int = 1;
    const PA_SAMPLE_S16LE: c_int = 3;

    type New = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        c_int,
        *const c_char,
        *const c_char,
        *const SampleSpec,
        *const c_void,
        *const c_void,
        *mut c_int,
    ) -> *mut c_void;
    type Write = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int;
    type Drain = unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int;
    type Free = unsafe extern "C" fn(*mut c_void);

    /// One playback stream for the click (on its own thread).
    pub fn play() {
        let Ok(lib) = Lib::open("libpulse-simple.so.0") else { return };
        // SAFETY: the prototypes of <pulse/simple.h>.
        let Some((new, write, drain, free)) = (unsafe {
            (|| Some((lib.sym::<New>("pa_simple_new").ok()?, lib.sym::<Write>("pa_simple_write").ok()?, lib.sym::<Drain>("pa_simple_drain").ok()?, lib.sym::<Free>("pa_simple_free").ok()?)))()
        }) else {
            return;
        };
        let pcm = super::click_samples();
        let spec = SampleSpec { format: PA_SAMPLE_S16LE, rate: super::RATE, channels: 1 };
        let mut err: c_int = 0;
        // SAFETY: valid C strings and spec; the stream is freed below.
        unsafe {
            let s = new(std::ptr::null(), c"Rustshot".as_ptr(), PA_STREAM_PLAYBACK, std::ptr::null(), c"shutter".as_ptr(), &spec, std::ptr::null(), std::ptr::null(), &mut err);
            if s.is_null() {
                return;
            }
            if write(s, pcm.as_ptr().cast(), pcm.len() * 2, &mut err) == 0 {
                let _ = drain(s, &mut err);
            }
            free(s);
        }
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod mac {
    use crate::wind::{msg0, msg1, ns_string, objc_cls, objc_sel};
    use std::ffi::c_void;

    /// `[[NSSound soundNamed:@"Tink"] play]` (the system sound; plays
    /// asynchronously).
    pub fn play() {
        unsafe {
            let cls = objc_cls(c"NSSound");
            if cls.is_null() {
                return;
            }
            let snd: *mut c_void = msg1(cls, objc_sel(c"soundNamed:"), ns_string("Tink"));
            if !snd.is_null() {
                let _: i8 = msg0(snd, objc_sel(c"play"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_is_a_small_valid_wav() {
        let s = click_samples();
        assert_eq!(s.len(), RATE as usize * 70 / 1000);
        assert!(s.iter().any(|v| v.unsigned_abs() > 8000), "audible");
        assert!(s.last().unwrap().unsigned_abs() < 2000, "decays to quiet");
        let w = wav(&s);
        assert!(w.len() <= 8 * 1024, "{} bytes", w.len());
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize, s.len() * 2);
        assert_eq!(click_samples(), s, "deterministic");
    }
}
