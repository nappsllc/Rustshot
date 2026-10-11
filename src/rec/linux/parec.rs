//! PulseAudio (and PipeWire's pulse server) sound read in-app, through
//! `parec` until the libpulse-simple source lands: one `parec --raw
//! --format=float32le --rate=48000 --channels=2 --latency-msec=20 -d
//! <source>` per source, its stdout read on a thread into a bounded ring
//! ([`PcmRing`]) that [`ParecSource`] hands to the session's mixer, with
//! the capture time of what it hands out estimated from when it was read
//! (see [`PcmRing::take`]).
//!
//! parec is found like ffmpeg (absolute `PATH` entries only), started
//! without a shell and dies with the thread that started it (see
//! [`super::child`]); dropping the source kills it (by its PID).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use std::collections::VecDeque;
use std::ffi::OsString;
use std::time::{Duration, Instant};

/// What parec delivers: 48 kHz stereo f32.
pub const RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
/// The latency asked of parec (its buffer: how long a sample can wait in
/// the server before we read it).
pub const LATENCY_MS: u64 = 20;
/// At most this much sound waits in the ring (seconds); older samples are
/// dropped when the mixer falls behind.
const RING_SECS: usize = 2;

/// parec's command line (without the program) for `source`.
pub fn args(source: &str) -> Vec<OsString> {
    let mut a: Vec<OsString> =
        ["--raw", "--format=float32le", "--rate=48000", "--channels=2", &format!("--latency-msec={LATENCY_MS}"), "-d"]
            .iter()
            .map(OsString::from)
            .collect();
    a.push(source.into());
    a
}

/// Raw f32le bytes from a pipe, as whole frames with their read time.
pub struct PcmRing {
    samples: VecDeque<f32>,
    /// Bytes of a sample split across reads.
    carry: Vec<u8>,
    /// When the newest samples were read.
    newest: Option<Instant>,
    cap: usize,
    channels: usize,
    rate: u32,
    latency: Duration,
}

impl PcmRing {
    pub fn new(rate: u32, channels: u16, latency: Duration) -> PcmRing {
        let channels = usize::from(channels.max(1));
        let cap = rate as usize * channels * RING_SECS;
        PcmRing { samples: VecDeque::new(), carry: Vec::new(), newest: None, cap, channels, rate: rate.max(1), latency }
    }

    /// Bytes read from the pipe at `at`. A sample split across reads is
    /// completed by the next one; the oldest samples go when it is full.
    pub fn push(&mut self, bytes: &[u8], at: Instant) {
        let mut b = bytes;
        if !self.carry.is_empty() {
            let need = 4 - self.carry.len();
            let k = need.min(b.len());
            self.carry.extend_from_slice(&b[..k]);
            b = &b[k..];
            if self.carry.len() == 4 {
                let s = f32::from_le_bytes([self.carry[0], self.carry[1], self.carry[2], self.carry[3]]);
                self.samples.push_back(s);
                self.carry.clear();
            }
        }
        let (whole, rest) = b.as_chunks::<4>();
        self.samples.extend(whole.iter().map(|c| f32::from_le_bytes(*c)));
        self.carry.extend_from_slice(rest);
        if self.samples.len() > self.cap {
            // Drop whole frames, so channels stay in place.
            let over = self.samples.len() - self.cap;
            let cut = over.div_ceil(self.channels) * self.channels;
            self.samples.drain(..cut.min(self.samples.len()));
        }
        if !bytes.is_empty() {
            self.newest = Some(at);
        }
    }

    /// How long samples now wait in the server before they are read (a
    /// source that can ask, like libpulse-simple, updates it every read).
    pub fn set_latency(&mut self, latency: Duration) {
        self.latency = latency;
    }

    /// Append every whole frame waiting to `out`; how many samples, and
    /// when the first of them was captured: the newest was read at the
    /// last push, about one parec latency after it was captured, and the
    /// ones before it came at the sample rate. A frame split across reads
    /// waits for the rest.
    pub fn take(&mut self, out: &mut Vec<f32>) -> (usize, Option<Instant>) {
        let frames = self.samples.len() / self.channels;
        let n = frames * self.channels;
        if n == 0 {
            return (0, None);
        }
        let span = Duration::from_nanos((frames as u128 * 1_000_000_000 / u128::from(self.rate)) as u64);
        out.extend(self.samples.drain(..n));
        let at = self.newest.and_then(|t| t.checked_sub(self.latency + span));
        (n, at)
    }
}

/// Read `r` (parec's stdout) into `ring` until it ends.
pub fn pump(mut r: impl std::io::Read, ring: &std::sync::Mutex<PcmRing>) {
    let mut buf = [0u8; 4096];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => ring.lock().unwrap_or_else(|e| e.into_inner()).push(&buf[..n], Instant::now()),
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::{Ended, ParecSource};

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::rec::linux::{child, ffmpeg};
    use crate::rec::AudioSource;
    use anyhow::{anyhow, bail, Context, Result};
    use std::io::Read;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;

    /// How long opening waits for parec to either deliver or fail.
    const OPEN_WAIT: Duration = Duration::from_millis(500);
    /// How often a running source checks that parec is still there.
    const CHECK_EVERY: Duration = Duration::from_millis(500);

    /// Why a source stopped mid-recording ("parec -d X ended (...): ...").
    pub type Ended = Arc<Mutex<Option<String>>>;

    /// One PulseAudio source through parec (see the module docs).
    pub struct ParecSource {
        child: Child,
        source: String,
        ring: Arc<Mutex<PcmRing>>,
        reader: Option<JoinHandle<()>>,
        /// parec's messages, drained on a thread (an unread pipe would
        /// stop parec once full; a closed one would kill it).
        errs: Arc<Mutex<ffmpeg::Tail>>,
        err_reader: Option<JoinHandle<()>>,
        checked: Instant,
        ended: Ended,
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl ParecSource {
        /// Start reading `source` (a pulse source name, e.g.
        /// `@DEFAULT_MONITOR@`). Waits up to half a second: an error
        /// (no such source, no server) is returned with parec's message.
        pub fn open(source: &str) -> Result<ParecSource> {
            let exe = ffmpeg::find_tool("parec", None).ok_or_else(|| anyhow!("parec is not installed (pulseaudio-utils)"))?;
            let mut cmd = Command::new(&exe);
            cmd.args(args(source)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
            child::prepare(&mut cmd, None);
            let mut child = cmd.spawn().with_context(|| format!("start {}", exe.display()))?;
            let ring = Arc::new(Mutex::new(PcmRing::new(RATE, CHANNELS, Duration::from_millis(LATENCY_MS))));
            let errs = Arc::new(Mutex::new(ffmpeg::Tail::default()));
            let err_reader = child.stderr.take().and_then(|mut e| {
                let errs = errs.clone();
                std::thread::Builder::new()
                    .name("parec-stderr".into())
                    .spawn(move || {
                        let mut buf = [0u8; 512];
                        while let Ok(n) = e.read(&mut buf) {
                            if n == 0 {
                                break;
                            }
                            lock(&errs).push(&buf[..n]);
                        }
                    })
                    .ok()
            });
            let reader = child.stdout.take().and_then(|out| {
                let ring = ring.clone();
                std::thread::Builder::new().name("parec-reader".into()).spawn(move || pump(out, &ring)).ok()
            });
            let mut src = ParecSource {
                child,
                source: source.to_string(),
                ring,
                reader,
                errs,
                err_reader,
                checked: Instant::now(),
                ended: Ended::default(),
            };
            if src.reader.is_none() {
                bail!("start the parec reader thread");
            }
            let end = Instant::now() + OPEN_WAIT;
            while Instant::now() < end {
                if let Some(st) = src.child.try_wait()? {
                    if let Some(r) = src.err_reader.take() {
                        let _ = r.join(); // its last words
                    }
                    bail!("parec -d {source} ended ({st}): {}", lock(&src.errs).text());
                }
                if lock(&src.ring).newest.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(src)
        }

        /// Set to why parec stopped, if it stops while recording.
        pub fn ended(&self) -> Ended {
            self.ended.clone()
        }

        /// Now and then: whether parec is still running; once it is not,
        /// why is logged and kept for the recording's warnings.
        fn check(&mut self) {
            let now = Instant::now();
            if now.duration_since(self.checked) < CHECK_EVERY || lock(&self.ended).is_some() {
                return;
            }
            self.checked = now;
            let st = match self.child.try_wait() {
                Ok(Some(st)) => st.to_string(),
                Ok(None) => return,
                Err(e) => e.to_string(),
            };
            std::thread::sleep(Duration::from_millis(50)); // its last words
            let why = lock(&self.errs).text();
            let msg = format!("parec -d {} ended ({st}){}{why}", self.source, if why.is_empty() { "" } else { ": " });
            eprintln!("rustshot: {msg}; the rest of the recording has no sound from it");
            *lock(&self.ended) = Some(msg);
        }
    }

    impl AudioSource for ParecSource {
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
            self.check();
            lock(&self.ring).take(out)
        }
    }

    impl Drop for ParecSource {
        fn drop(&mut self) {
            // Our own child, by its PID.
            let _ = self.child.kill();
            let _ = self.child.wait();
            for r in [self.reader.take(), self.err_reader.take()].into_iter().flatten() {
                let _ = r.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn parec_args_are_argv_entries() {
        let a = args("weird name; rm -rf ~");
        let s: Vec<String> = a.iter().map(|x| x.to_string_lossy().into_owned()).collect();
        assert_eq!(s, ["--raw", "--format=float32le", "--rate=48000", "--channels=2", "--latency-msec=20", "-d", "weird name; rm -rf ~"]);
    }

    #[test]
    fn ring_parses_split_samples_and_hands_out_whole_frames() {
        let t0 = Instant::now();
        let mut r = PcmRing::new(48_000, 2, Duration::ZERO);
        let b = bytes(&[0.5, -0.25, 1.0, 0.125]);
        // Split inside the second sample and inside the second frame.
        r.push(&b[..6], t0);
        let mut out = Vec::new();
        assert_eq!(r.take(&mut out).0, 0, "half a frame waits");
        r.push(&b[6..10], t0);
        assert_eq!(r.take(&mut out).0, 2);
        assert_eq!(out, [0.5, -0.25]);
        r.push(&b[10..], t0);
        assert_eq!(r.take(&mut out).0, 2);
        assert_eq!(out, [0.5, -0.25, 1.0, 0.125]);
        assert_eq!(r.take(&mut out), (0, None), "empty");
    }

    #[test]
    fn ring_stamps_the_first_sample_from_the_read_time() {
        let t0 = Instant::now() + Duration::from_secs(10);
        let mut r = PcmRing::new(48_000, 2, Duration::from_millis(20));
        // 10 ms read at t0, 10 ms more read at t0 + 10 ms.
        r.push(&bytes(&vec![0.1; 960]), t0);
        r.push(&bytes(&vec![0.1; 960]), t0 + Duration::from_millis(10));
        let mut out = Vec::new();
        let (n, at) = r.take(&mut out);
        assert_eq!(n, 1920);
        // The newest sample came at t0 + 10 ms, captured 20 ms before; the
        // 20 ms of sound started 20 ms before that.
        let want = t0 + Duration::from_millis(10) - Duration::from_millis(40);
        let at = at.unwrap();
        assert!(at.max(want) - at.min(want) < Duration::from_micros(1), "{at:?} vs {want:?}");
    }

    #[test]
    fn ring_is_bounded_and_keeps_channels_in_place() {
        let t0 = Instant::now();
        let mut r = PcmRing::new(100, 2, Duration::ZERO);
        // 3 s of (L = 1, R = -1) at 100 Hz into a 2 s ring.
        let lr: Vec<f32> = (0..300).flat_map(|_| [1.0, -1.0]).collect();
        r.push(&bytes(&lr), t0);
        r.push(&bytes(&[1.0]), t0); // half a frame on top
        let mut out = Vec::new();
        let (n, _) = r.take(&mut out);
        assert!(n <= 400, "{n}");
        assert!(out.as_chunks::<2>().0.iter().all(|f| *f == [1.0, -1.0]), "frames stay whole");
    }

    /// A pipe that hands out 7 bytes at a time (splitting samples).
    struct Trickle(Vec<u8>, usize);

    impl std::io::Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = 7.min(buf.len()).min(self.0.len() - self.1);
            buf[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
            self.1 += n;
            Ok(n)
        }
    }

    #[test]
    fn pump_reads_a_fake_pipe_to_the_end() {
        let ring = std::sync::Mutex::new(PcmRing::new(48_000, 2, Duration::ZERO));
        let want: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        pump(Trickle(bytes(&want), 0), &ring);
        let mut out = Vec::new();
        let (n, at) = ring.lock().unwrap().take(&mut out);
        assert_eq!(n, 1000);
        assert_eq!(out, want, "every sample, in order, across split reads");
        assert!(at.is_some());
    }
}
