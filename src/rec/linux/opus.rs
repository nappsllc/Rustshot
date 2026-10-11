//! Opus for the MP4s recorded without ffmpeg: the system's libopus
//! (`libopus.so.0`, loaded with `dlopen`), 48 kHz stereo, 20 ms packets,
//! `OPUS_APPLICATION_AUDIO` at 128 kbit/s. The pre-skip written to `dOps`
//! is the encoder's look-ahead (312 samples for this setup). Without
//! libopus the recording has no sound, with a warning.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::rec::mp4::OPUS_FRAME;

/// The soname loaded.
pub const LIB: &str = "libopus.so.0";
/// Bit rate of the sound.
pub const BITRATE: i32 = 128_000;
/// libopus's look-ahead at 48 kHz (2.5 ms + 4 ms), the pre-skip when the
/// library does not say.
pub const PRE_SKIP: u16 = 312;
/// Interleaved stereo samples in one packet.
pub const PACKET_SAMPLES: usize = OPUS_FRAME as usize * 2;

/// Cuts contiguous interleaved stereo into whole packets' worth.
#[derive(Default)]
pub struct Packetizer {
    buf: Vec<f32>,
    /// Stereo frames taken in (the sound's real length).
    frames: u64,
}

impl Packetizer {
    pub fn push(&mut self, pcm: &[f32]) {
        let whole = &pcm[..pcm.len() / 2 * 2];
        self.buf.extend_from_slice(whole);
        self.frames += (whole.len() / 2) as u64;
    }

    /// The next full packet's samples, if there are enough.
    pub fn next(&mut self) -> Option<Vec<f32>> {
        (self.buf.len() >= PACKET_SAMPLES).then(|| self.buf.drain(..PACKET_SAMPLES).collect())
    }

    /// What is left, padded with silence to a packet; `None` when nothing is.
    pub fn rest(&mut self) -> Option<Vec<f32>> {
        if self.buf.is_empty() {
            return None;
        }
        let mut p = std::mem::take(&mut self.buf);
        p.resize(PACKET_SAMPLES, 0.0);
        Some(p)
    }

    /// The sound's length in samples (per channel).
    pub fn frames(&self) -> u64 {
        self.frames
    }
}

#[cfg(target_os = "linux")]
pub use imp::{Opus, OpusEnc};

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::rec::linux::dl::Lib;
    use anyhow::{anyhow, Result};
    use core::ffi::{c_char, c_int, c_void};
    use std::sync::Arc;

    const OPUS_APPLICATION_AUDIO: c_int = 2049;
    const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
    const OPUS_GET_LOOKAHEAD_REQUEST: c_int = 4027;
    /// The largest packet asked for (libopus recommends 4000 bytes).
    const MAX_PACKET: usize = 4000;

    type Create = unsafe extern "C" fn(i32, c_int, c_int, *mut c_int) -> *mut c_void;
    type Ctl = unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int;
    type EncodeFloat = unsafe extern "C" fn(*mut c_void, *const f32, c_int, *mut u8, i32) -> i32;
    type Destroy = unsafe extern "C" fn(*mut c_void);
    type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

    /// libopus, loaded.
    pub struct Opus {
        create: Create,
        ctl: Ctl,
        encode: EncodeFloat,
        destroy: Destroy,
        strerror: StrError,
        _lib: Lib,
    }

    impl Opus {
        /// Load the system's libopus ([`LIB`]).
        pub fn load() -> Result<Arc<Opus>> {
            let lib = Lib::open(LIB)?;
            // SAFETY: the types are libopus's documented prototypes.
            unsafe {
                Ok(Arc::new(Opus {
                    create: lib.sym("opus_encoder_create")?,
                    ctl: lib.sym("opus_encoder_ctl")?,
                    encode: lib.sym("opus_encode_float")?,
                    destroy: lib.sym("opus_encoder_destroy")?,
                    strerror: lib.sym("opus_strerror")?,
                    _lib: lib,
                }))
            }
        }

        fn error(&self, code: c_int) -> String {
            // SAFETY: opus_strerror returns a static string for any code.
            let p = unsafe { (self.strerror)(code) };
            if p.is_null() { format!("error {code}") } else { unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned() }
        }
    }

    /// One encoder (48 kHz stereo, [`BITRATE`]).
    pub struct OpusEnc {
        lib: Arc<Opus>,
        st: *mut c_void,
        pre_skip: u16,
        out: Vec<u8>,
    }

    // SAFETY: the encoder state is used by its one owner only.
    unsafe impl Send for OpusEnc {}

    impl OpusEnc {
        pub fn new(lib: Arc<Opus>) -> Result<OpusEnc> {
            let mut err: c_int = 0;
            // SAFETY: valid arguments; `err` receives the result code.
            let st = unsafe { (lib.create)(48_000, 2, OPUS_APPLICATION_AUDIO, &mut err) };
            if st.is_null() || err != 0 {
                return Err(anyhow!("opus_encoder_create: {}", lib.error(err)));
            }
            let mut enc = OpusEnc { lib, st, pre_skip: PRE_SKIP, out: vec![0; MAX_PACKET] };
            // SAFETY: documented requests with their argument types (opus_int32, opus_int32*).
            unsafe {
                let r = (enc.lib.ctl)(enc.st, OPUS_SET_BITRATE_REQUEST, BITRATE);
                if r != 0 {
                    eprintln!("rustshot: opus: cannot set the bit rate: {}", enc.lib.error(r));
                }
                let mut look: i32 = 0;
                if (enc.lib.ctl)(enc.st, OPUS_GET_LOOKAHEAD_REQUEST, &mut look as *mut i32) == 0 && (0..=u16::MAX as i32).contains(&look) {
                    enc.pre_skip = look as u16;
                }
            }
            Ok(enc)
        }

        /// The pre-skip for `dOps` and the edit list.
        pub fn pre_skip(&self) -> u16 {
            self.pre_skip
        }

        /// One packet from [`PACKET_SAMPLES`] interleaved samples.
        pub fn encode(&mut self, pcm: &[f32]) -> Result<Vec<u8>> {
            assert_eq!(pcm.len(), PACKET_SAMPLES);
            // SAFETY: `pcm` holds a whole 20 ms frame; `out` has MAX_PACKET bytes.
            let n = unsafe { (self.lib.encode)(self.st, pcm.as_ptr(), OPUS_FRAME as c_int, self.out.as_mut_ptr(), MAX_PACKET as i32) };
            if n < 0 {
                return Err(anyhow!("opus_encode_float: {}", self.lib.error(n)));
            }
            Ok(self.out[..n as usize].to_vec())
        }
    }

    impl Drop for OpusEnc {
        fn drop(&mut self) {
            // SAFETY: created by opus_encoder_create, destroyed once.
            unsafe { (self.lib.destroy)(self.st) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packetizer_cuts_whole_packets_and_pads_the_rest() {
        let mut p = Packetizer::default();
        p.push(&vec![0.5; PACKET_SAMPLES - 2]);
        assert!(p.next().is_none());
        p.push(&[0.25; 5]); // 2 frames and half of one (dropped)
        let a = p.next().unwrap();
        assert_eq!(a.len(), PACKET_SAMPLES);
        assert_eq!(a[PACKET_SAMPLES - 1], 0.25);
        assert!(p.next().is_none());
        p.push(&[0.75; 4]);
        let r = p.rest().unwrap();
        assert_eq!(r.len(), PACKET_SAMPLES);
        assert_eq!(&r[..6], &[0.25, 0.25, 0.75, 0.75, 0.75, 0.75]);
        assert!(r[6..].iter().all(|&s| s == 0.0));
        assert!(p.rest().is_none());
        assert_eq!(p.frames(), 959 + 2 + 2);
    }
}
