//! Feeding ffmpeg's two pipes (frames on stdin, sound on fd 3) from the
//! one encode thread without ever blocking it for good.
//!
//! ffmpeg 7's scheduler stops reading an input whose stream is more than
//! 100 ms ahead of the stream that trails (measured on the encoded
//! packets). Fed from one thread with blocking writes, two pipes can then
//! deadlock: the thread waits on the pipe ffmpeg stopped reading, so the
//! other one, which ffmpeg waits for, never gets more. So:
//!
//! - **The picture never trails the sound** by more than one encode tick:
//!   on every tick with sound the last frame is repeated up to where the
//!   sound is ([`Cfr::hold_to`]), so a capture stall does not leave the
//!   video behind.
//! - **Frames never wait when there is sound**: a frame that does not fit
//!   in the video queue is not written now ([`Cfr::unsend`]); the frames
//!   after it fill its slots. So a video input held back because it runs
//!   ahead (an encoder without delay: openh264, VA-API) does not stop the
//!   sound, which is what lets it go again.
//! - **The sound has room**: ten seconds ([`SOUND_BYTES`]) wait for ffmpeg
//!   (x264 holds the first packets back for a while on many-core machines,
//!   and the sound is held back meanwhile).
//! - **Every wait has a limit** ([`SEND_TIMEOUT`]): a pipe that takes
//!   nothing for that long is reported, and the encoder stops ffmpeg.
//!
//! Each pipe is a [`Bounded`] queue drained by its writer thread
//! ([`pump`]).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use super::ffmpeg::{pcm_bytes, Cfr, PcmTimeline};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Raw frames waiting for the video writer (repeats share one buffer).
pub const VIDEO_FRAMES: usize = 2;
/// Sound waiting for the sound writer: 10 s of 48 kHz stereo f32 (3.84 MB).
pub const SOUND_BYTES: usize = 10 * crate::rec::mix::RATE as usize * 2 * 4;
/// How long one write may wait for ffmpeg to take something.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Which pipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pipe {
    Frames,
    Sound,
}

impl Pipe {
    pub fn name(self) -> &'static str {
        match self {
            Pipe::Frames => "frames",
            Pipe::Sound => "sound",
        }
    }
}

/// Why a write did not go through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stuck {
    /// The pipe took nothing for [`SEND_TIMEOUT`] (ffmpeg hung).
    Timeout(Pipe),
    /// Its writer stopped (ffmpeg went away; the writer says why).
    Closed(Pipe),
}

/// Why [`Bounded::send`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendErr {
    Timeout,
    Full,
    Closed,
}

/// What [`Bounded::recv_timeout`] got.
#[derive(Debug, PartialEq, Eq)]
pub enum Recv<T> {
    Item(T),
    Empty,
    Closed,
}

/// A queue bounded by the weight of what waits in it (frames, bytes), with
/// sends that give up after a while and either side able to close it.
pub struct Bounded<T> {
    inner: Mutex<Inner<T>>,
    cv: Condvar,
}

struct Inner<T> {
    q: VecDeque<(T, usize)>,
    used: usize,
    cap: usize,
    /// No more sends (the encoder is done).
    tx_closed: bool,
    /// No more receives (the writer stopped).
    rx_closed: bool,
}

impl<T> Bounded<T> {
    pub fn new(cap: usize) -> Arc<Bounded<T>> {
        let inner = Inner { q: VecDeque::new(), used: 0, cap: cap.max(1), tx_closed: false, rx_closed: false };
        Arc::new(Bounded { inner: Mutex::new(inner), cv: Condvar::new() })
    }

    fn lock(&self) -> MutexGuard<'_, Inner<T>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `item` of `weight`, waiting up to `wait` for room (`None`: no
    /// waiting at all, [`SendErr::Full`]). An item heavier than the whole
    /// queue goes in once the queue is empty.
    pub fn send(&self, item: T, weight: usize, wait: Option<Duration>) -> Result<(), SendErr> {
        let end = wait.map(|w| Instant::now() + w);
        let mut g = self.lock();
        loop {
            if g.rx_closed || g.tx_closed {
                return Err(SendErr::Closed);
            }
            if g.q.is_empty() || g.used + weight <= g.cap {
                g.used += weight;
                g.q.push_back((item, weight));
                drop(g);
                self.cv.notify_all();
                return Ok(());
            }
            let Some(end) = end else { return Err(SendErr::Full) };
            let now = Instant::now();
            if now >= end {
                return Err(SendErr::Timeout);
            }
            g = self.cv.wait_timeout(g, end - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The oldest item, waiting up to `wait`; [`Recv::Closed`] once the
    /// sender closed and everything was taken.
    pub fn recv_timeout(&self, wait: Duration) -> Recv<T> {
        let end = Instant::now() + wait;
        let mut g = self.lock();
        loop {
            if let Some((item, w)) = g.q.pop_front() {
                g.used -= w;
                drop(g);
                self.cv.notify_all();
                return Recv::Item(item);
            }
            if g.tx_closed || g.rx_closed {
                return Recv::Closed;
            }
            let now = Instant::now();
            if now >= end {
                return Recv::Empty;
            }
            g = self.cv.wait_timeout(g, end - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The oldest item, waiting for one; `None` once closed and drained.
    pub fn recv(&self) -> Option<T> {
        loop {
            match self.recv_timeout(Duration::from_secs(3600)) {
                Recv::Item(t) => return Some(t),
                Recv::Closed => return None,
                Recv::Empty => {}
            }
        }
    }

    /// The sender is done: the receiver takes what is left, then ends.
    pub fn close_tx(&self) {
        self.lock().tx_closed = true;
        self.cv.notify_all();
    }

    /// The receiver stopped: what waits is dropped, sends fail.
    pub fn close_rx(&self) {
        let mut g = self.lock();
        g.rx_closed = true;
        g.q.clear();
        g.used = 0;
        drop(g);
        self.cv.notify_all();
    }

    /// The weight waiting.
    pub fn used(&self) -> usize {
        self.lock().used
    }
}

/// Closes the receiving side when the writer ends, however it ends.
struct RxGuard<'a, T>(&'a Bounded<T>);

impl<T> Drop for RxGuard<'_, T> {
    fn drop(&mut self) {
        self.0.close_rx();
    }
}

/// A writer thread's loop: every item of `q` to `w`, counting the bytes in
/// `written`, then a flush; on an error (a broken pipe) the queue is
/// closed so the encoder hears of it at its next send.
pub fn pump<T: AsRef<[u8]>>(q: &Bounded<T>, mut w: impl std::io::Write, written: &AtomicU64) -> std::io::Result<()> {
    let _rx = RxGuard(q);
    while let Some(b) = q.recv() {
        w.write_all(b.as_ref())?;
        written.fetch_add(b.as_ref().len() as u64, Ordering::Relaxed);
    }
    w.flush()
}

/// Logs a recurring message at most once per `every`, with how many were
/// left out since.
#[derive(Debug)]
pub struct RateLog {
    every: Duration,
    last: Option<Instant>,
    skipped: u64,
}

impl RateLog {
    pub fn new(every: Duration) -> RateLog {
        RateLog { every, last: None, skipped: 0 }
    }

    /// Whether to log now; `Some(n)` with the number left out before it.
    pub fn allow(&mut self, now: Instant) -> Option<u64> {
        if self.last.is_some_and(|t| now.duration_since(t) < self.every) {
            self.skipped += 1;
            return None;
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.skipped))
    }
}

/// Puts frames and sound on the two queues: the constant frame rate and
/// contiguous sound of [`Cfr`] / [`PcmTimeline`], with the rules in the
/// module docs.
pub struct Feeder {
    pub video: Arc<Bounded<Arc<[u8]>>>,
    pub sound: Option<Arc<Bounded<Vec<u8>>>>,
    cfr: Cfr,
    pcm: PcmTimeline,
    /// The last frame written (repeated to keep up with the sound).
    last: Option<Arc<[u8]>>,
    timeout: Duration,
    late: RateLog,
}

impl Feeder {
    /// Queues for `fps` frames and, with `sound`, the sound pipe; writes
    /// wait up to `timeout`.
    pub fn new(fps: u32, sound: bool, timeout: Duration) -> Feeder {
        Feeder {
            video: Bounded::new(VIDEO_FRAMES),
            sound: sound.then(|| Bounded::new(SOUND_BYTES)),
            cfr: Cfr::new(fps),
            pcm: PcmTimeline::default(),
            last: None,
            timeout,
            late: RateLog::new(Duration::from_secs(1)),
        }
    }

    /// Frames written.
    pub fn frames(&self) -> u64 {
        self.cfr.sent()
    }

    /// The lengths written: picture, sound.
    pub fn lengths(&self) -> (Duration, Duration) {
        (self.cfr.length(), self.pcm.length())
    }

    /// Write `buf` `n` times; without waiting (with sound) only what fits,
    /// the rest given back to the frame rate. Returns how many went.
    fn put_frames(&mut self, buf: &Arc<[u8]>, n: u64, wait: bool) -> Result<u64, Stuck> {
        let wait = wait.then_some(self.timeout);
        for i in 0..n {
            match self.video.send(buf.clone(), 1, wait) {
                Ok(()) => {}
                Err(SendErr::Full) => {
                    self.cfr.unsend(n - i);
                    return Ok(i);
                }
                Err(SendErr::Timeout) => {
                    self.cfr.unsend(n - i);
                    return Err(Stuck::Timeout(Pipe::Frames));
                }
                Err(SendErr::Closed) => {
                    self.cfr.unsend(n - i);
                    return Err(Stuck::Closed(Pipe::Frames));
                }
            }
        }
        Ok(n)
    }

    fn put_pcm(&mut self, pcm: &[f32]) -> Result<(), Stuck> {
        if pcm.is_empty() {
            return Ok(());
        }
        let Some(q) = &self.sound else { return Ok(()) };
        let b = pcm_bytes(pcm);
        let n = b.len();
        q.send(b, n, Some(self.timeout)).map_err(|e| match e {
            SendErr::Closed => Stuck::Closed(Pipe::Sound),
            SendErr::Timeout | SendErr::Full => Stuck::Timeout(Pipe::Sound),
        })
    }

    fn put_silence(&mut self, frames: u64) -> Result<(), Stuck> {
        let mut left = frames as usize;
        while left > 0 {
            let k = left.min(crate::rec::mix::RATE as usize / 10);
            self.put_pcm(&vec![0.0; k * 2])?;
            left -= k;
        }
        Ok(())
    }

    /// The frame stamped `ts` (`make` copies it, only when it is written).
    pub fn frame(&mut self, ts: Duration, make: impl FnOnce() -> Arc<[u8]>) -> Result<(), Stuck> {
        let n = self.cfr.repeats(ts);
        if n == 0 {
            return Ok(());
        }
        let buf = make();
        self.last = Some(buf.clone());
        let wait = self.sound.is_none();
        self.put_frames(&buf, n, wait).map(|_| ())
    }

    /// Sound stamped `ts` (the session's mixed, contiguous sound); then
    /// the picture is held up to it.
    pub fn audio(&mut self, pcm: &[f32], ts: Duration) -> Result<(), Stuck> {
        let (gap, pcm) = self.pcm.place(pcm, ts);
        if gap > 0
            && let Some(skipped) = self.late.allow(Instant::now())
        {
            let more = if skipped > 0 { format!(" ({skipped} more since the last report)") } else { String::new() };
            eprintln!("rustshot: sound {gap} frames late at {ts:?}; filled with silence{more}");
        }
        self.put_silence(gap)?;
        self.put_pcm(pcm)?;
        self.hold_picture()
    }

    /// Repeat the last frame up to the sound's end (not waiting).
    fn hold_picture(&mut self) -> Result<(), Stuck> {
        let Some(last) = self.last.clone() else { return Ok(()) };
        let n = self.cfr.hold_to(self.pcm.length());
        self.put_frames(&last, n, false).map(|_| ())
    }

    /// At the end: the picture held to the sound's end and the sound
    /// padded to the picture's, each write waiting at most the timeout.
    pub fn even_out(&mut self) -> Result<(), Stuck> {
        if self.sound.is_none() {
            return Ok(());
        }
        if let Some(last) = self.last.clone() {
            let n = self.cfr.hold_to(self.pcm.length());
            self.put_frames(&last, n, true)?;
        }
        let pad = self.pcm.pad_to(self.cfr.length());
        self.put_silence(pad)
    }

    /// No more writes: the writers finish what waits, then close the pipes.
    pub fn close(&self) {
        self.video.close_tx();
        if let Some(s) = &self.sound {
            s.close_tx();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn bounded_send_waits_for_room_then_times_out() {
        let q = Bounded::<u8>::new(2);
        assert_eq!(q.send(1, 1, None), Ok(()));
        assert_eq!(q.send(2, 1, None), Ok(()));
        assert_eq!(q.send(3, 1, None), Err(SendErr::Full));
        let t = Instant::now();
        assert_eq!(q.send(3, 1, Some(ms(100))), Err(SendErr::Timeout));
        assert!(t.elapsed() >= ms(100) && t.elapsed() < ms(2000), "{:?}", t.elapsed());
        // Room made by a receiver lets a waiting send through.
        let q2 = q.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(ms(50));
            q2.recv()
        });
        assert_eq!(q.send(3, 1, Some(ms(5000))), Ok(()));
        assert_eq!(h.join().unwrap(), Some(1));
        // A heavy item goes in alone.
        let big = Bounded::<u8>::new(10);
        assert_eq!(big.send(1, 50, None), Ok(()));
        assert_eq!(big.send(2, 1, None), Err(SendErr::Full));
        assert_eq!(big.used(), 50);
    }

    #[test]
    fn bounded_close_ends_both_sides() {
        let q = Bounded::<u8>::new(4);
        q.send(1, 1, None).unwrap();
        q.close_tx();
        assert_eq!(q.recv(), Some(1), "what waits is still taken");
        assert_eq!(q.recv(), None);
        assert_eq!(q.send(2, 1, None), Err(SendErr::Closed));
        let q = Bounded::<u8>::new(1);
        q.send(1, 1, None).unwrap();
        // A sender waiting for room hears at once that the writer stopped.
        let q2 = q.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(ms(50));
            q2.close_rx();
        });
        let t = Instant::now();
        assert_eq!(q.send(2, 1, Some(ms(5000))), Err(SendErr::Closed));
        assert!(t.elapsed() < ms(2000));
        h.join().unwrap();
        assert_eq!(q.used(), 0, "dropped");
    }

    #[test]
    fn pump_writes_in_order_and_closes_on_a_broken_pipe() {
        let q = Bounded::<Vec<u8>>::new(1 << 20);
        q.send(pcm_bytes(&[1.0, -0.5]), 8, None).unwrap();
        q.send(pcm_bytes(&[0.25]), 4, None).unwrap();
        q.close_tx();
        let mut out = Vec::new();
        let n = AtomicU64::new(0);
        pump(&q, &mut out, &n).unwrap();
        let want: Vec<u8> = [1.0f32, -0.5, 0.25].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(out, want);
        assert_eq!(&out[..4], &[0, 0, 0x80, 0x3F], "little-endian 1.0");
        assert_eq!(n.load(Ordering::Relaxed), 12);

        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let q = Bounded::<Vec<u8>>::new(16);
        q.send(vec![1, 2, 3, 4], 4, None).unwrap();
        assert_eq!(pump(&q, Broken, &n).unwrap_err().kind(), std::io::ErrorKind::BrokenPipe);
        assert_eq!(q.send(vec![5], 1, Some(ms(5000))), Err(SendErr::Closed), "the encoder hears of it");
    }

    #[test]
    fn rate_log_reports_once_a_second_with_the_count_left_out() {
        let t0 = Instant::now();
        let mut r = RateLog::new(Duration::from_secs(1));
        assert_eq!(r.allow(t0), Some(0));
        for k in 1..=9 {
            assert_eq!(r.allow(t0 + ms(k * 100 - 1)), None);
        }
        assert_eq!(r.allow(t0 + ms(1000)), Some(9));
        assert_eq!(r.allow(t0 + ms(1500)), None);
        assert_eq!(r.allow(t0 + ms(2500)), Some(1));
    }

    /// A stand-in for ffmpeg 7 reading the two pipes: an input is read only
    /// while its stream is less than 100 ms ahead of the other one, counted
    /// on the encoded packets; the video encoder holds `delay` frames back
    /// before its first packet (x264's look-ahead and threads). Returns the
    /// picture and sound read (seconds) once both pipes closed, or `None`
    /// when `stop` is set first.
    fn fake_ffmpeg7(
        video: Arc<Bounded<Arc<[u8]>>>,
        sound: Arc<Bounded<Vec<u8>>>,
        fps: u32,
        delay: u64,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<Option<(f64, f64)>> {
        std::thread::spawn(move || {
            let tol = 0.1;
            let (mut frames, mut bytes) = (0u64, 0u64);
            let (mut v_done, mut a_done) = (false, false);
            let bps = f64::from(crate::rec::mix::RATE) * 8.0;
            while !(v_done && a_done) {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                // Encoded positions (None before the first packet).
                let v_out = (frames > delay).then(|| (frames - delay) as f64 / f64::from(fps));
                let a_out = (bytes > 0).then(|| bytes as f64 / bps);
                let ahead = |mine: Option<f64>, other: Option<f64>, other_done: bool| match (mine, other) {
                    _ if other_done => false,
                    (Some(m), Some(o)) => m - o >= tol,
                    (Some(_), None) => true, // the other has no packet yet
                    (None, _) => false,
                };
                let mut idle = true;
                if !v_done && !ahead(v_out, a_out, a_done) {
                    match video.recv_timeout(ms(1)) {
                        Recv::Item(_) => {
                            frames += 1;
                            idle = false;
                        }
                        Recv::Closed => v_done = true,
                        Recv::Empty => {}
                    }
                }
                if !a_done && !ahead(a_out, v_out, v_done) {
                    match sound.recv_timeout(ms(1)) {
                        Recv::Item(b) => {
                            bytes += b.len() as u64;
                            idle = false;
                        }
                        Recv::Closed => a_done = true,
                        Recv::Empty => {}
                    }
                }
                if idle {
                    std::thread::yield_now();
                }
            }
            Some((frames as f64 / f64::from(fps), bytes as f64 / bps))
        })
    }

    /// The session's encode thread, sped up: a tick every 10 ms of clock
    /// time, a frame each 1/fps of it except while the capture `stalls`
    /// (clock times), the sound mixed `lag` behind the clock; with `drain`
    /// the video queue is emptied after each tick (an ffmpeg that always
    /// takes frames) and the picture checked to be at most a tick (and a
    /// frame) behind the sound. Returns the first stuck write.
    fn drive(f: &mut Feeder, fps: u32, secs: f64, lag: Duration, stall: Option<(f64, f64)>, drain: bool) -> Result<(), Stuck> {
        let tick = ms(10);
        let mut t = Duration::ZERO;
        let mut next_frame = Duration::ZERO;
        let mut sound_at = Duration::ZERO;
        let frame: Arc<[u8]> = Arc::from(vec![0u8; 16]);
        while t.as_secs_f64() < secs {
            let stalled = stall.is_some_and(|(a, b)| (a..b).contains(&t.as_secs_f64()));
            if t >= next_frame && !stalled {
                f.frame(t, || frame.clone())?;
                next_frame = t + Duration::from_secs_f64(1.0 / f64::from(fps));
            }
            let upto = t.saturating_sub(lag);
            if upto > sound_at {
                let n = ((upto - sound_at).as_nanos() * 48_000 / 1_000_000_000) as usize;
                f.audio(&vec![0.1; n * 2], sound_at)?;
                sound_at += Duration::from_nanos((n as u128 * 1_000_000_000 / 48_000) as u64);
            }
            if drain {
                let (v, a) = f.lengths();
                if f.frames() > 0 {
                    assert!(v + tick + Duration::from_secs_f64(1.0 / f64::from(fps)) >= a, "picture {v:?} behind sound {a:?} at {t:?}");
                }
                while let Recv::Item(_) = f.video.recv_timeout(Duration::ZERO) {}
            } else {
                std::thread::sleep(Duration::from_micros(500));
            }
            t += tick;
        }
        f.audio(&[], t)?;
        f.even_out()
    }

    fn run_fake(fps: u32, delay: u64, lag: Duration, stall: Option<(f64, f64)>) -> (f64, f64) {
        let mut f = Feeder::new(fps, true, Duration::from_secs(5));
        let stop = Arc::new(AtomicBool::new(false));
        let h = fake_ffmpeg7(f.video.clone(), f.sound.clone().unwrap(), fps, delay, stop.clone());
        let r = drive(&mut f, fps, 8.0, lag, stall, false);
        f.close();
        if r.is_err() {
            stop.store(true, Ordering::Relaxed);
        }
        let got = h.join().unwrap();
        r.unwrap();
        let (v, a) = got.unwrap();
        let (fv, fa) = f.lengths();
        assert!((v - fv.as_secs_f64()).abs() < 1e-6 && (a - fa.as_secs_f64()).abs() < 1e-6, "all of it was read");
        (v, a)
    }

    #[test]
    fn fake_ffmpeg7_capture_stall_of_five_seconds_finishes_with_matching_lengths() {
        // x264-like delay, the sound 100 ms behind the clock.
        let (v, a) = run_fake(30, 10, ms(100), Some((1.5, 6.5)));
        assert!((v - a).abs() <= 0.05, "video {v} s, audio {a} s");
        assert!((7.8..=8.2).contains(&v), "{v}");
    }

    #[test]
    fn fake_ffmpeg7_long_encoder_delay_holds_the_sound_back_without_blocking() {
        // A many-core x264: 3.5 s before its first packet.
        let (v, a) = run_fake(30, 105, ms(100), None);
        assert!((v - a).abs() <= 0.05, "video {v} s, audio {a} s");
    }

    #[test]
    fn fake_ffmpeg7_video_ahead_of_the_sound_drops_frames_instead_of_blocking() {
        // No encoder delay (openh264, VA-API): the picture runs 100+ ms
        // ahead of the sound and ffmpeg holds it back.
        let (v, a) = run_fake(60, 0, ms(150), Some((2.0, 2.5)));
        assert!((v - a).abs() <= 0.05, "video {v} s, audio {a} s");
    }

    #[test]
    fn a_pipe_nobody_reads_times_out_instead_of_blocking() {
        let mut f = Feeder::new(30, true, ms(200));
        let t = Instant::now();
        // Nothing reads the sound: 10 s fit, then one write waits 200 ms.
        let e = drive(&mut f, 30, 30.0, ms(100), None, false).unwrap_err();
        assert_eq!(e, Stuck::Timeout(Pipe::Sound));
        assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
        // Video only: a frame write waits, then gives up.
        let mut f = Feeder::new(30, false, ms(200));
        let frame: Arc<[u8]> = Arc::from(vec![0u8; 4]);
        let t = Instant::now();
        let mut r = Ok(());
        for i in 0..10u64 {
            r = f.frame(ms(i * 34), || frame.clone());
            if r.is_err() {
                break;
            }
        }
        assert_eq!(r, Err(Stuck::Timeout(Pipe::Frames)));
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(f.frames(), VIDEO_FRAMES as u64, "what did not go is not counted");
        // A writer that went away: reported at once.
        f.video.close_rx();
        assert_eq!(f.frame(ms(1000), || frame.clone()), Err(Stuck::Closed(Pipe::Frames)));
    }

    #[test]
    fn picture_keeps_up_with_the_sound_through_a_capture_stall() {
        // Without a reader of the sound, but every frame taken: the last
        // frame is repeated while no new one comes (checked in `drive`).
        let mut f = Feeder::new(30, true, Duration::from_secs(5));
        let sound = f.sound.clone().unwrap();
        let h = std::thread::spawn(move || while sound.recv().is_some() {});
        drive(&mut f, 30, 6.0, ms(100), Some((1.0, 5.0)), true).unwrap();
        f.close();
        h.join().unwrap();
        let (v, a) = f.lengths();
        assert!(v.abs_diff(a) <= ms(34), "{v:?} vs {a:?}");
    }

    #[test]
    fn frames_that_do_not_fit_are_made_up_by_the_next_ones() {
        let mut f = Feeder::new(10, true, ms(200));
        let frame: Arc<[u8]> = Arc::from(vec![0u8; 4]);
        f.frame(ms(0), || frame.clone()).unwrap();
        f.frame(ms(100), || frame.clone()).unwrap();
        // The queue is full: this one is not written now...
        f.frame(ms(200), || frame.clone()).unwrap();
        assert_eq!(f.frames(), 2);
        // ...the next one, once there is room, fills both slots.
        f.video.recv();
        f.video.recv();
        f.frame(ms(300), || frame.clone()).unwrap();
        assert_eq!(f.frames(), 4);
    }
}
