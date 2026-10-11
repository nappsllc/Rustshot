//! A bounded hand-off of timestamped sound packets from a capture thread
//! (WASAPI, ScreenCaptureKit, AVFoundation) to the mixer: at most `cap`
//! samples are held, the oldest packets go first, and a read hands out the
//! oldest packet and those that follow on from it (stopping at a
//! discontinuity, so the mixer places what comes after by its own time).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// One packet: when its first frame was captured, its samples, and
/// whether it follows on from the packet before it.
struct Chunk {
    at: Instant,
    data: Vec<f32>,
    cont: bool,
}

#[derive(Default)]
struct RingQ {
    chunks: VecDeque<Chunk>,
    len: usize,
}

/// Bounded hand-off from the capture thread.
pub(crate) struct Ring {
    q: Mutex<RingQ>,
    /// Most samples held (one second, set once the format is known).
    cap: AtomicUsize,
}

impl Ring {
    pub(crate) fn new(cap: usize) -> Ring {
        Ring { q: Mutex::new(RingQ::default()), cap: AtomicUsize::new(cap) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RingQ> {
        self.q.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue a packet captured at `at`; `cont` = it follows on from the
    /// previous one. Over the cap, the oldest packets go, and what is left
    /// no longer follows on from what was read before.
    pub(crate) fn push(&self, at: Instant, data: &[f32], cont: bool) {
        if data.is_empty() {
            return;
        }
        let mut q = self.lock();
        q.len += data.len();
        q.chunks.push_back(Chunk { at, data: data.to_vec(), cont });
        let cap = self.cap.load(Ordering::Relaxed);
        while q.len > cap
            && let Some(c) = q.chunks.pop_front()
        {
            q.len -= c.data.len();
            if let Some(f) = q.chunks.front_mut() {
                f.cont = false;
            }
        }
    }

    /// Append the oldest packet and those that follow on from it to `out`;
    /// returns the samples appended and when the first was captured.
    pub(crate) fn take(&self, out: &mut Vec<f32>) -> (usize, Option<Instant>) {
        let mut q = self.lock();
        let Some(first) = q.chunks.pop_front() else { return (0, None) };
        let at = first.at;
        let mut n = first.data.len();
        out.extend(first.data);
        while q.chunks.front().is_some_and(|c| c.cont) {
            let c = q.chunks.pop_front().expect("front exists");
            n += c.data.len();
            out.extend(c.data);
        }
        q.len -= n;
        (n, Some(at))
    }

    /// Hold at most `cap` samples from now on.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // the rate is known up front elsewhere
    pub(crate) fn set_cap(&self, cap: usize) {
        self.cap.store(cap, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn ring_keeps_the_newest_samples_and_reads_up_to_a_discontinuity() {
        let t = Instant::now();
        let ms = Duration::from_millis;
        let r = Ring::new(4);
        r.push(t, &[1.0, 2.0], true);
        r.push(t + ms(10), &[3.0, 4.0], true);
        r.push(t + ms(20), &[5.0, 6.0], true);
        // Over the cap: the oldest packet went; what is left is a new run.
        let mut out = Vec::new();
        assert_eq!(r.take(&mut out), (4, Some(t + ms(10))));
        assert_eq!(out, vec![3.0, 4.0, 5.0, 6.0]);
        assert_eq!(r.take(&mut out), (0, None));
        // A discontinuity the device reports splits the reads.
        r.push(t + ms(30), &[7.0], true);
        r.push(t + ms(45), &[8.0], false);
        r.push(t + ms(55), &[9.0], true);
        out.clear();
        assert_eq!(r.take(&mut out), (1, Some(t + ms(30))));
        assert_eq!(r.take(&mut out), (2, Some(t + ms(45))));
        assert_eq!(out, vec![7.0, 8.0, 9.0]);
        assert_eq!(r.lock().len, 0);
    }
}
