//! The Record panel's microphone level: the chosen microphone is opened
//! only while the panel is open with the microphone on (like the camera:
//! released when it closes), read on its own thread, and its peak over
//! the last 50 ms published for the panel's 7-bar meter (polled at 20 Hz).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
#[cfg(not(test))]
use std::time::{Duration, Instant};

/// The meter's bars (logical px heights, fixed; 3 px wide, 2 px apart).
pub const BARS: [f32; 7] = [5.0, 9.0, 12.0, 8.0, 4.0, 6.0, 6.0];
/// A peak at or above this is clipping: the top two bars turn amber.
pub const CLIP: f32 = 0.98;
/// How often the panel looks at the level.
pub const POLL_MS: u64 = 50;

/// How many bars a `peak` (0..=1) lights, left to right (-60 dBFS and
/// below: none; 0 dBFS: all), and whether it clips.
pub fn bars(peak: f32) -> (u8, bool) {
    let peak = peak.clamp(0.0, 1.0);
    if peak <= 0.001 {
        return (0, false);
    }
    let db = 20.0 * peak.log10();
    let k = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    ((k * BARS.len() as f32).round() as u8, peak >= CLIP)
}

/// An open microphone being metered (see the module docs).
pub struct MicMeter {
    dev: String,
    level: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
}

impl MicMeter {
    /// Start metering microphone `dev` ("" = the default). Tests open
    /// nothing (the level is set by hand).
    pub fn open(dev: &str) -> MicMeter {
        let m = MicMeter { dev: dev.to_string(), level: Arc::default(), stop: Arc::default() };
        #[cfg(not(test))]
        {
            let (id, level, stop) = (dev.to_string(), m.level.clone(), m.stop.clone());
            let spawned = std::thread::Builder::new().name("mic-meter".into()).spawn(move || run(&id, &level, &stop));
            if let Err(e) = spawned {
                eprintln!("rustshot: cannot meter the microphone: {e}");
            }
        }
        m
    }

    /// The microphone it meters.
    pub fn device(&self) -> &str {
        &self.dev
    }

    /// The latest peak, 0..=1.
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Tests: what the microphone "hears".
    #[cfg(test)]
    pub fn set_peak(&self, v: f32) {
        self.level.store(v.to_bits(), Ordering::Relaxed);
    }
}

impl Drop for MicMeter {
    /// The thread lets go of the microphone within a read.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Open the microphone `id` on this platform.
#[cfg(not(test))]
fn open_mic(id: &str) -> anyhow::Result<Box<dyn crate::rec::AudioSource>> {
    #[cfg(windows)]
    {
        Ok(Box::new(crate::rec::win::WasapiCapture::open(Some(id))?))
    }
    #[cfg(target_os = "linux")]
    {
        use crate::rec::linux::pulse_list;
        let all = pulse_list::list();
        let name = if all.is_none() && crate::rec::linux::pulse::available() {
            Ok(if id.is_empty() { pulse_list::DEFAULT_SOURCE.into() } else { id.to_string() })
        } else {
            pulse_list::mic_source(all.as_deref(), id)
        };
        let name = name.map_err(|e| anyhow::anyhow!(e))?;
        Ok(Box::new(crate::rec::linux::pulse::Sound::open(&name)?))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(crate::rec::macos::mic::open(id)?))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = id;
        anyhow::bail!("no microphone here")
    }
}

/// Read the microphone until told to stop, publishing the peak of every
/// [`POLL_MS`] window.
#[cfg(not(test))]
fn run(id: &str, level: &AtomicU32, stop: &AtomicBool) {
    #[cfg(windows)]
    crate::rec::win::com_init();
    let mut src = match open_mic(id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("rustshot: microphone level: {e:#}");
            return;
        }
    };
    let mut buf = Vec::new();
    let (mut peak, mut since) = (0.0f32, Instant::now());
    while !stop.load(Ordering::SeqCst) {
        buf.clear();
        if src.read(&mut buf) > 0 {
            peak = peak.max(super::peak_of(&buf));
        }
        if since.elapsed() >= Duration::from_millis(POLL_MS) {
            level.store(peak.to_bits(), Ordering::Relaxed);
            (peak, since) = (0.0, Instant::now());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bars fill left to right with the level; the top two go amber when
    /// it clips.
    #[test]
    fn bars_follow_the_level() {
        assert_eq!(bars(0.0), (0, false));
        assert_eq!(bars(0.0005), (0, false));
        assert_eq!(bars(0.001_1).0, 0, "-59 dB rounds to none");
        assert_eq!(bars(0.1).0, 5, "-20 dB");
        assert_eq!(bars(0.5), (6, false), "-6 dB");
        assert_eq!(bars(0.8), (7, false));
        assert_eq!(bars(1.0), (7, true));
        let mut last = 0;
        for i in 1..=100 {
            let (n, _) = bars(i as f32 / 100.0);
            assert!(n >= last, "monotonic");
            last = n;
        }
        let m = MicMeter::open("{mic-1}");
        assert_eq!((m.device(), m.peak()), ("{mic-1}", 0.0));
        m.set_peak(0.25);
        assert_eq!(m.peak(), 0.25);
    }
}
