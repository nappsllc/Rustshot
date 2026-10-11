//! PulseAudio sound read in-app through the simple API
//! (`libpulse-simple.so.0`, loaded with `dlopen`; works on PipeWire's
//! pulse server too): one record stream per source, 48 kHz stereo
//! float32le with a 20 ms fragment, read on a thread into the same
//! bounded ring as [`super::parec`] ([`PcmRing`]). After each read the
//! stream's latency (`pa_simple_get_latency`) is taken as how long ago
//! the newest sample was captured, which [`AudioSource::read_timed`]
//! reports. Preferred over parec when the library is there (Flatpak and
//! Snap have it but not parec).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

/// The soname loaded.
pub const LIB: &str = "libpulse-simple.so.0";
/// Bytes read at a time: 10 ms of 48 kHz stereo f32.
pub const READ_BYTES: usize = 480 * 2 * 4;
/// The fragment asked of the server: 20 ms.
pub const FRAGMENT_BYTES: u32 = 960 * 2 * 4;

#[cfg(target_os = "linux")]
pub use imp::{available, Sound};

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::rec::linux::dl::Lib;
    use crate::rec::linux::parec::{Ended, PcmRing, CHANNELS, RATE};
    use crate::rec::AudioSource;
    use anyhow::{anyhow, Result};
    use core::ffi::{c_char, c_int, c_void};
    use std::ffi::{CStr, CString};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    #[repr(C)]
    struct SampleSpec {
        format: c_int,
        rate: u32,
        channels: u8,
    }

    #[repr(C)]
    struct BufferAttr {
        maxlength: u32,
        tlength: u32,
        prebuf: u32,
        minreq: u32,
        fragsize: u32,
    }

    const PA_STREAM_RECORD: c_int = 2;
    const PA_SAMPLE_FLOAT32LE: c_int = 5;

    type New = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        c_int,
        *const c_char,
        *const c_char,
        *const SampleSpec,
        *const c_void,
        *const BufferAttr,
        *mut c_int,
    ) -> *mut c_void;
    type Read = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut c_int) -> c_int;
    type Latency = unsafe extern "C" fn(*mut c_void, *mut c_int) -> u64;
    type Free = unsafe extern "C" fn(*mut c_void);
    type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

    struct Api {
        new: New,
        read: Read,
        latency: Latency,
        free: Free,
        strerror: StrError,
        _lib: Lib,
    }

    impl Api {
        fn error(&self, e: c_int) -> String {
            // SAFETY: pa_strerror returns a static string (or null).
            let p = unsafe { (self.strerror)(e) };
            if p.is_null() { format!("error {e}") } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
        }
    }

    /// libpulse-simple, loaded once (`None` when it is not installed).
    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| {
            let lib = Lib::open(LIB).ok()?;
            // SAFETY: the prototypes of <pulse/simple.h> and <pulse/error.h>
            // (pa_strerror comes from libpulse, which the simple library loads).
            unsafe {
                Some(Api {
                    new: lib.sym("pa_simple_new").ok()?,
                    read: lib.sym("pa_simple_read").ok()?,
                    latency: lib.sym("pa_simple_get_latency").ok()?,
                    free: lib.sym("pa_simple_free").ok()?,
                    strerror: lib.sym("pa_strerror").ok()?,
                    _lib: lib,
                })
            }
        })
        .as_ref()
    }

    /// Whether libpulse-simple can be loaded.
    pub fn available() -> bool {
        api().is_some()
    }

    struct Stream(*mut c_void);
    // SAFETY: the stream is used by the reader thread only, then freed
    // after it has been joined.
    unsafe impl Send for Stream {}

    /// One PulseAudio source (see the module docs).
    pub struct PulseSource {
        ring: Arc<Mutex<PcmRing>>,
        stop: Arc<AtomicBool>,
        reader: Option<JoinHandle<()>>,
        ended: Ended,
        name: String,
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl PulseSource {
        /// Start recording `device` (a pulse source name, e.g.
        /// `@DEFAULT_MONITOR@`); the server's error when it cannot.
        pub fn open(device: &str) -> Result<PulseSource> {
            let api = api().ok_or_else(|| anyhow!("{LIB} is not installed"))?;
            let dev = CString::new(device).map_err(|_| anyhow!("bad source name"))?;
            let spec = SampleSpec { format: PA_SAMPLE_FLOAT32LE, rate: RATE, channels: CHANNELS as u8 };
            let attr = BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: FRAGMENT_BYTES };
            let mut err: c_int = 0;
            // SAFETY: valid C strings and structures that outlive the call.
            let s = unsafe {
                (api.new)(
                    std::ptr::null(),
                    c"Rustshot".as_ptr(),
                    PA_STREAM_RECORD,
                    dev.as_ptr(),
                    c"Recording".as_ptr(),
                    &spec,
                    std::ptr::null(),
                    &attr,
                    &mut err,
                )
            };
            if s.is_null() {
                return Err(anyhow!("cannot record {device}: {}", api.error(err)));
            }
            let stream = Stream(s);
            let ring = Arc::new(Mutex::new(PcmRing::new(RATE, CHANNELS, Duration::from_millis(20))));
            let stop = Arc::new(AtomicBool::new(false));
            let ended = Ended::default();
            let reader = {
                let (ring, stop, ended, name) = (ring.clone(), stop.clone(), ended.clone(), device.to_string());
                std::thread::Builder::new().name("pulse-reader".into()).spawn(move || {
                    let stream = stream;
                    let mut buf = vec![0u8; READ_BYTES];
                    while !stop.load(Ordering::Relaxed) {
                        let mut e: c_int = 0;
                        // SAFETY: `buf` has READ_BYTES bytes; the stream is live.
                        if unsafe { (api.read)(stream.0, buf.as_mut_ptr().cast(), buf.len(), &mut e) } < 0 {
                            let msg = format!("PulseAudio source {name} stopped: {}", api.error(e));
                            eprintln!("rustshot: {msg}; the rest of the recording has no sound from it");
                            *lock(&ended) = Some(msg);
                            break;
                        }
                        let at = Instant::now();
                        let mut e: c_int = 0;
                        // SAFETY: a live stream.
                        let lat = unsafe { (api.latency)(stream.0, &mut e) };
                        let mut r = lock(&ring);
                        if e == 0 && lat < 2_000_000 {
                            r.set_latency(Duration::from_micros(lat));
                        }
                        r.push(&buf, at);
                    }
                    // SAFETY: created by pa_simple_new, freed once, here.
                    unsafe { (api.free)(stream.0) };
                })
            };
            let reader = match reader {
                Ok(r) => r,
                Err(e) => return Err(anyhow!("start the PulseAudio reader thread: {e}")),
            };
            Ok(PulseSource { ring, stop, reader: Some(reader), ended, name: device.into() })
        }

        /// Set to why the source stopped, if it stops while recording.
        pub fn ended(&self) -> Ended {
            self.ended.clone()
        }

        pub fn name(&self) -> &str {
            &self.name
        }
    }

    impl AudioSource for PulseSource {
        fn rate(&self) -> u32 {
            RATE
        }

        fn channels(&self) -> u16 {
            CHANNELS
        }

        fn read(&mut self, out: &mut Vec<f32>) -> usize {
            self.read_timed(out).0
        }

        fn read_timed(&mut self, out: &mut Vec<f32>) -> (usize, Option<Instant>) {
            lock(&self.ring).take(out)
        }
    }

    /// A pulse source as recorded: through libpulse-simple when it is
    /// installed, else through parec.
    pub enum Sound {
        Simple(PulseSource),
        Parec(crate::rec::linux::parec::ParecSource),
    }

    impl Sound {
        pub fn open(device: &str) -> Result<Sound> {
            if available() {
                PulseSource::open(device).map(Sound::Simple)
            } else {
                crate::rec::linux::parec::ParecSource::open(device).map(Sound::Parec)
            }
        }

        /// Set to why the source stopped, if it stops while recording.
        pub fn ended(&self) -> Ended {
            match self {
                Sound::Simple(s) => s.ended(),
                Sound::Parec(s) => s.ended(),
            }
        }

        /// Which reader it uses (for the log).
        pub fn via(&self) -> &'static str {
            match self {
                Sound::Simple(_) => "libpulse-simple",
                Sound::Parec(_) => "parec",
            }
        }
    }

    impl AudioSource for Sound {
        fn rate(&self) -> u32 {
            RATE
        }

        fn channels(&self) -> u16 {
            CHANNELS
        }

        fn read(&mut self, out: &mut Vec<f32>) -> usize {
            self.read_timed(out).0
        }

        fn read_timed(&mut self, out: &mut Vec<f32>) -> (usize, Option<Instant>) {
            match self {
                Sound::Simple(s) => s.read_timed(out),
                Sound::Parec(s) => s.read_timed(out),
            }
        }
    }

    impl Drop for PulseSource {
        fn drop(&mut self) {
            // A read returns within a fragment (10-20 ms). Should the
            // server stall it, the thread is left to end (and free the
            // stream) on its own rather than hold up the recording's end.
            self.stop.store(true, Ordering::Relaxed);
            if let Some(r) = self.reader.take() {
                let end = Instant::now() + Duration::from_secs(1);
                while !r.is_finished() && Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if r.is_finished() {
                    let _ = r.join();
                } else {
                    eprintln!("rustshot: PulseAudio source {} did not stop in time; left to end", self.name);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_ten_ms_of_stereo_f32() {
        assert_eq!(READ_BYTES, 48_000 / 100 * 2 * 4);
        assert_eq!(FRAGMENT_BYTES as usize, 2 * READ_BYTES);
    }
}
