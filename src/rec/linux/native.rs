//! MP4 without ffmpeg: OpenH264 pictures and Opus sound in Rustshot's own
//! muxer ([`crate::rec::mp4`]), written to the `.part` as they come.
//!
//! Frames keep their session timestamps (the MP4 has variable frame
//! timing; a frame stamped no later than the one before it is left out),
//! except that the first picture starts at 0, as the sound does, so both
//! line up by the session clock. The sound is the mixer's contiguous
//! stream from 0, held to it by [`PcmTimeline`] and cut into 20 ms Opus
//! packets. At the end the last picture lasts until the sound ends (or
//! one frame, if that is later).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::rec::mp4::VIDEO_TIMESCALE;
use std::time::Duration;

/// `t` in 90 kHz ticks.
pub fn ticks(t: Duration) -> u64 {
    (t.as_nanos() * u128::from(VIDEO_TIMESCALE) / 1_000_000_000) as u64
}

/// When the last picture (starting at `last`, 90 kHz) ends: one frame
/// after it, or when the sound ends (`audio_samples` at 48 kHz) if later.
pub fn video_end(last: u64, fps: u32, audio_samples: u64) -> u64 {
    let frame = u64::from(VIDEO_TIMESCALE) / u64::from(fps.max(1));
    let audio = audio_samples * u64::from(VIDEO_TIMESCALE) / u64::from(crate::rec::mp4::AUDIO_TIMESCALE);
    (last + frame).max(audio)
}

/// Which MP4 encoder Linux records with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Ffmpeg,
    OpenH264,
    /// Neither is there: ask the user (the first-use dialog).
    Ask,
}

/// The encoder for `rec_linux_encoder` (`auto`, `ffmpeg`, `openh264`):
/// the preferred one when it is there, else the other, else ask. `auto`
/// prefers ffmpeg. `ffmpeg`: ffmpeg is installed and encodes H.264;
/// `openh264`: the pinned library was downloaded.
pub fn pick(pref: &str, ffmpeg: bool, openh264: bool) -> Pick {
    let order = if pref.eq_ignore_ascii_case("openh264") { [Pick::OpenH264, Pick::Ffmpeg] } else { [Pick::Ffmpeg, Pick::OpenH264] };
    order.into_iter().find(|p| if *p == Pick::Ffmpeg { ffmpeg } else { openh264 }).unwrap_or(Pick::Ask)
}

#[cfg(target_os = "linux")]
pub use imp::NativeEncoder;

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::rec::linux::ffmpeg::PcmTimeline;
    use crate::rec::linux::openh264::{H264Enc, OpenH264};
    use crate::rec::linux::opus::{Opus, OpusEnc, Packetizer};
    use crate::rec::mp4::{access_unit, AudioParams, Finished, Mp4Writer, VideoParams};
    use crate::rec::{bitrate, mp4_fit, yuv, Frame, Quality, Salvaged, VideoEncoder};
    use anyhow::{anyhow, Context, Result};
    use std::path::Path;
    use std::sync::Arc;

    /// See the module docs.
    pub struct NativeEncoder {
        mp4: Mp4Writer<std::fs::File>,
        h264: H264Enc,
        opus: Option<OpusEnc>,
        pcm: Packetizer,
        timeline: PcmTimeline,
        fps: u32,
        out: (u32, u32),
        i420: Vec<u8>,
        nv12: Vec<u8>,
        scratch: Vec<u8>,
        /// The newest frame's session time, and its picture's MP4 time.
        last: Option<(Duration, u64)>,
        silence: Vec<f32>,
    }

    impl NativeEncoder {
        /// Write `path` for `w`×`h` frames at `fps`; with `opus` the
        /// session's sound is encoded too.
        pub fn create(path: &Path, w: u32, h: u32, fps: u32, quality: Quality, lib: Arc<OpenH264>, opus: Option<Arc<Opus>>) -> Result<NativeEncoder> {
            let (ow, oh) = mp4_fit(w, h);
            let h264 = H264Enc::new(lib, ow, oh, fps, bitrate(ow, oh, fps, quality))?;
            let opus = match opus.map(OpusEnc::new).transpose() {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("rustshot: {e:#}; recording without sound");
                    None
                }
            };
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .with_context(|| format!("create {}", path.display()))?;
            let audio = opus.as_ref().map(|o| AudioParams { pre_skip: o.pre_skip() });
            let mp4 = Mp4Writer::new(file, Some(VideoParams { w: ow, h: oh }), audio).context("write the MP4 header")?;
            Ok(NativeEncoder {
                mp4,
                h264,
                opus,
                pcm: Packetizer::default(),
                timeline: PcmTimeline::default(),
                fps: fps.max(1),
                out: (ow, oh),
                i420: Vec::new(),
                nv12: Vec::new(),
                scratch: Vec::new(),
                last: None,
                silence: Vec::new(),
            })
        }

        fn packets(&mut self) -> Result<()> {
            while let Some(p) = self.pcm.next() {
                self.write_packet(&p)?;
            }
            Ok(())
        }

        fn write_packet(&mut self, p: &[f32]) -> Result<()> {
            let Some(o) = self.opus.as_mut() else { return Ok(()) };
            let pk = o.encode(p)?;
            self.mp4.write_audio(&pk).context("write the recording")
        }
    }

    impl VideoEncoder for NativeEncoder {
        fn push_video(&mut self, f: &Frame) -> Result<()> {
            if self.last.is_some_and(|(t, _)| f.ts <= t) {
                return Ok(());
            }
            let (ow, oh) = self.out;
            yuv::convert_i420_into(&f.bgra, f.w, f.h, ow, oh, &mut self.i420, &mut self.nv12, &mut self.scratch);
            let time = if self.last.is_none() { 0 } else { ticks(f.ts) };
            let ms = f.ts.as_millis() as i64;
            self.last = Some((f.ts, time));
            let Some(bytes) = self.h264.encode(&self.i420, ms)? else { return Ok(()) };
            let au = access_unit(&bytes);
            if let (Some(s), Some(p)) = (&au.sps, &au.pps) {
                self.mp4.set_avc_config(s, p);
            }
            self.mp4.write_video(&au.avcc, time, au.idr).context("write the recording")
        }

        fn push_audio(&mut self, pcm: &[f32], ts: Duration) -> Result<()> {
            if self.opus.is_none() {
                return Ok(());
            }
            let (gap, pcm) = self.timeline.place(pcm, ts);
            if gap > 0 {
                self.silence.clear();
                self.silence.resize(gap as usize * 2, 0.0);
                let s = std::mem::take(&mut self.silence);
                self.pcm.push(&s);
                self.silence = s;
            }
            self.pcm.push(pcm);
            self.packets()
        }

        fn finish(mut self: Box<Self>) -> Result<()> {
            let mut err = None;
            if self.opus.is_some() {
                if let Err(e) = self.packets() {
                    err = Some(e);
                } else if let Some(rest) = self.pcm.rest()
                    && let Err(e) = self.write_packet(&rest)
                {
                    err = Some(e);
                }
                let n = self.pcm.frames();
                self.mp4.set_audio_length(n);
            }
            let audio = if self.opus.is_some() { self.pcm.frames() } else { 0 };
            let last = self.last.map_or(0, |(_, t)| t);
            let end = video_end(last, self.fps, audio);
            let s = *self;
            let NativeEncoder { mp4, h264, opus, .. } = s;
            drop((h264, opus));
            match mp4.finish(end) {
                Ok((f, Finished::Done)) => {
                    f.sync_all().context("write the recording")?;
                    match err {
                        Some(e) => Err(anyhow::Error::new(Salvaged(format!("{e:#}")))),
                        None => Ok(()),
                    }
                }
                Ok((f, Finished::Partly(why))) => {
                    let _ = f.sync_all();
                    Err(anyhow::Error::new(Salvaged(why)))
                }
                Err(e) => Err(anyhow!("finish the MP4: {e}")),
            }
        }

        fn audio(&self) -> bool {
            self.opus.is_some()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_prefers_ffmpeg_then_openh264_then_asks() {
        assert_eq!(pick("auto", true, true), Pick::Ffmpeg);
        assert_eq!(pick("auto", false, true), Pick::OpenH264);
        assert_eq!(pick("auto", true, false), Pick::Ffmpeg);
        assert_eq!(pick("auto", false, false), Pick::Ask);
        assert_eq!(pick("ffmpeg", true, true), Pick::Ffmpeg);
        assert_eq!(pick("ffmpeg", false, true), Pick::OpenH264, "chose ffmpeg but has not installed it yet");
        assert_eq!(pick("ffmpeg", false, false), Pick::Ask);
        assert_eq!(pick("openh264", true, true), Pick::OpenH264);
        assert_eq!(pick("openh264", true, false), Pick::Ffmpeg);
        assert_eq!(pick("openh264", false, false), Pick::Ask);
    }

    #[test]
    fn times_and_the_end_of_the_picture() {
        assert_eq!(ticks(Duration::from_millis(1500)), 135_000);
        assert_eq!(ticks(Duration::from_nanos(11_111)), 0);
        assert_eq!(video_end(90_000, 30, 0), 93_000, "one frame");
        assert_eq!(video_end(90_000, 30, 48_000 * 2), 180_000, "to the end of the sound");
    }
}
