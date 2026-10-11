//! The microphone through an `AVCaptureSession`: the device's sound
//! converted by AVFoundation to 48 kHz stereo f32 (`audioSettings`),
//! timed by the session's clock, read as a [`FeedSource`]. Microphone
//! access is asked for on first use (`NSMicrophoneUsageDescription`).

use super::media::{self, Capture, FeedSource, PcmFeed, SampleSink};
use crate::macos_objc::{self as objc};
use crate::wind::{msg1, objc_cls, objc_sel};
use anyhow::{anyhow, Result};

/// The microphones as (unique id, name).
pub fn list() -> Vec<(String, String)> {
    match media::media() {
        Ok(m) => media::devices(m.k.AVMediaTypeAudio),
        Err(_) => Vec::new(),
    }
}

/// Open microphone `id` ("" = the default one).
pub fn open(id: &str) -> Result<FeedSource> {
    let m = media::media()?;
    media::av_access(m.k.AVMediaTypeAudio, super::MIC_DENIED)?;
    let _pool = objc::Pool::new();
    unsafe {
        let dev = media::device(m.k.AVMediaTypeAudio, id)
            .ok_or_else(|| if id.is_empty() { anyhow!("no microphone") } else { anyhow!("microphone {id} not found") })?;
        let out = objc::new_obj(objc_cls(c"AVCaptureAudioDataOutput")).ok_or_else(|| anyhow!("create the sound output"))?;
        let k = &m.k;
        let settings = objc::dict(&[
            (k.AVFormatIDKey, objc::num_i64(i64::from(super::LPCM))),
            (k.AVSampleRateKey, objc::num_f64(f64::from(super::RATE))),
            (k.AVNumberOfChannelsKey, objc::num_i64(2)),
            (k.AVLinearPCMBitDepthKey, objc::num_i64(32)),
            (k.AVLinearPCMIsFloatKey, objc::num_bool(true)),
            (k.AVLinearPCMIsNonInterleaved, objc::num_bool(false)),
            (k.AVLinearPCMIsBigEndianKey, objc::num_bool(false)),
        ]);
        let _: () = msg1(out.ptr(), objc_sel(c"setAudioSettings:"), settings);
        // Packets that come before the session's clock is known (it is
        // read once the session runs) are dropped, not timed in host time.
        let feed = PcmFeed::awaiting_clock();
        let f = feed.clone();
        let sink = SampleSink::new(move |sbuf, _| f.push(m, sbuf), None)?;
        let cap = Capture::start(dev, out, sink, core::ptr::null_mut())?;
        feed.set_clock(cap.clock());
        Ok(FeedSource { feed, _keep: Some(Box::new(cap)) })
    }
}
