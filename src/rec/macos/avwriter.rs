//! MP4 through `AVAssetWriter`: H.264 (bit rate by quality, like the other
//! platforms; the system picks the hardware encoder) from BGRA pixel
//! buffers (the encoder converts), and AAC 48 kHz stereo at 160 kbit/s
//! from the mixer's f32 PCM. Real-time inputs: a frame that comes while
//! the video input is busy is dropped; sound is queued (bounded,
//! [`SoundQueue`]) and handed over as the audio input takes it, so the
//! encode thread never waits for the writer.
//! Presentation times come from the session clock (video in µs, made
//! strictly increasing; sound by sample index from 0).

use super::media::{self, CFRelease, Media};
use super::{Asbd, CMTime, SoundQueue};
use crate::macos_objc::{self as objc, Id};
use crate::rec::{Frame, Quality, VideoEncoder};
use crate::wind::{msg0, msg1, msg2, msg3, objc_cls, objc_sel};
use anyhow::{anyhow, bail, Result};
use core::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long finishing the file may take.
const FINISH: Duration = Duration::from_secs(60);
/// The most sound queued for a busy audio input (2 s).
const SOUND_QUEUE: usize = super::RATE as usize * 2;
/// How long finishing waits for the audio input to take what is queued.
const SOUND_DRAIN: Duration = Duration::from_secs(1);

/// `AVAssetWriterStatusCompleted`.
const COMPLETED: isize = 2;

pub struct AvWriterEncoder {
    m: &'static Media,
    writer: Id,
    video: Id,
    adaptor: Id,
    audio: Option<Id>,
    /// Sound the audio input was not ready for yet.
    sound: SoundQueue,
    /// The LPCM format of the sound handed to the writer.
    fmt: *mut c_void,
    size: (u32, u32),
    last: Option<i64>,
    frames: u64,
    dropped: u64,
    failed: Option<String>,
    done: bool,
    part: PathBuf,
}

// Used from one thread at a time (created on the worker, then the encode thread).
unsafe impl Send for AvWriterEncoder {}

impl AvWriterEncoder {
    /// Write `part` (an MP4 whatever its extension) of `w`×`h` frames at
    /// `fps`, with sound when `audio`.
    pub fn create(part: &Path, w: u32, h: u32, fps: u32, q: Quality, audio: bool) -> Result<AvWriterEncoder> {
        let m = media::media()?;
        let _pool = objc::Pool::new();
        // The writer refuses a file that exists.
        let _ = std::fs::remove_file(part);
        unsafe {
            let k = &m.k;
            let url: *mut c_void =
                msg1(objc_cls(c"NSURL"), objc_sel(c"fileURLWithPath:"), crate::wind::ns_string(&part.to_string_lossy()));
            let mut err: *mut c_void = core::ptr::null_mut();
            let wr: *mut c_void = msg0(objc_cls(c"AVAssetWriter"), objc_sel(c"alloc"));
            let wr: *mut c_void = msg3(wr, objc_sel(c"initWithURL:fileType:error:"), url, k.AVFileTypeMPEG4, core::ptr::addr_of_mut!(err));
            let writer = Id::owned(wr).ok_or_else(|| anyhow!("create the MP4 writer: {}", objc::error_text(err)))?;

            let fps = fps.max(1);
            let props = objc::dict(&[
                (k.AVVideoAverageBitRateKey, objc::num_i64(i64::from(crate::rec::bitrate(w, h, fps, q)))),
                (k.AVVideoMaxKeyFrameIntervalKey, objc::num_i64(i64::from(fps * 2))),
                (k.AVVideoExpectedSourceFrameRateKey, objc::num_i64(i64::from(fps))),
                (k.AVVideoProfileLevelKey, k.AVVideoProfileLevelH264HighAutoLevel),
            ]);
            let vset = objc::dict(&[
                (k.AVVideoCodecKey, k.AVVideoCodecTypeH264),
                (k.AVVideoWidthKey, objc::num_i64(i64::from(w))),
                (k.AVVideoHeightKey, objc::num_i64(i64::from(h))),
                (k.AVVideoCompressionPropertiesKey, props),
            ]);
            let video = input(k.AVMediaTypeVideo, vset)?;
            let pix = objc::dict(&[
                (k.kCVPixelBufferPixelFormatTypeKey, objc::num_i64(i64::from(super::BGRA))),
                (k.kCVPixelBufferWidthKey, objc::num_i64(i64::from(w))),
                (k.kCVPixelBufferHeightKey, objc::num_i64(i64::from(h))),
            ]);
            let ad: *mut c_void = msg2(
                objc_cls(c"AVAssetWriterInputPixelBufferAdaptor"),
                objc_sel(c"assetWriterInputPixelBufferAdaptorWithAssetWriterInput:sourcePixelBufferAttributes:"),
                video.ptr(),
                pix,
            );
            let adaptor = Id::retain(ad).ok_or_else(|| anyhow!("create the pixel buffer adaptor"))?;
            add(&writer, &video, "video")?;

            let mut fmt: *mut c_void = core::ptr::null_mut();
            let audio = if audio {
                let layout = super::stereo_layout();
                let layout: *mut c_void =
                    msg2(objc_cls(c"NSData"), objc_sel(c"dataWithBytes:length:"), layout.as_ptr(), layout.len());
                let aset = objc::dict(&[
                    (k.AVFormatIDKey, objc::num_i64(i64::from(super::AAC))),
                    (k.AVSampleRateKey, objc::num_f64(f64::from(super::RATE))),
                    (k.AVNumberOfChannelsKey, objc::num_i64(2)),
                    (k.AVEncoderBitRateKey, objc::num_i64(super::AAC_BITRATE)),
                    (k.AVChannelLayoutKey, layout),
                ]);
                let a = input(k.AVMediaTypeAudio, aset)?;
                add(&writer, &a, "audio")?;
                let asbd = Asbd::f32_stereo(super::RATE);
                let st = (m.cm.CMAudioFormatDescriptionCreate)(
                    core::ptr::null_mut(),
                    &asbd,
                    0,
                    core::ptr::null(),
                    0,
                    core::ptr::null(),
                    core::ptr::null_mut(),
                    &mut fmt,
                );
                if st != 0 || fmt.is_null() {
                    bail!("create the sound format ({st})");
                }
                Some(a)
            } else {
                None
            };
            let enc = AvWriterEncoder {
                m,
                writer,
                video,
                adaptor,
                audio,
                sound: SoundQueue::new(SOUND_QUEUE),
                fmt,
                size: (w, h),
                last: None,
                frames: 0,
                dropped: 0,
                failed: None,
                done: false,
                part: part.to_path_buf(),
            };
            let ok: i8 = msg0(enc.writer.ptr(), objc_sel(c"startWriting"));
            if ok == 0 {
                bail!("start the MP4 writer: {}", enc.writer_error());
            }
            let _: () = msg1(enc.writer.ptr(), objc_sel(c"startSessionAtSourceTime:"), CMTime::new(0, 1));
            Ok(enc)
        }
    }

    unsafe fn writer_error(&self) -> String {
        unsafe { objc::error_text(msg0(self.writer.ptr(), objc_sel(c"error"))) }
    }

    fn fail(&mut self, what: &str) -> anyhow::Error {
        let e = format!("{what}: {}", unsafe { self.writer_error() });
        self.failed = Some(e.clone());
        anyhow!(e)
    }

    fn ready(input: &Id) -> bool {
        let r: i8 = unsafe { msg0(input.ptr(), objc_sel(c"isReadyForMoreMediaData")) };
        r != 0
    }

    /// Hand queued sound to the audio input while it takes more; returns
    /// at once when it is busy (what is left goes later).
    fn feed_sound(&mut self) -> Result<()> {
        while self.audio.as_ref().is_some_and(Self::ready) {
            let Some((pcm, ts)) = self.sound.pop() else { break };
            unsafe { self.append_sound(&pcm, ts)? };
        }
        Ok(())
    }

    /// One buffer of sound into the (ready) audio input.
    unsafe fn append_sound(&mut self, pcm: &[f32], ts: Duration) -> Result<()> {
        let Some(audio) = &self.audio else { return Ok(()) };
        let frames = pcm.len() / 2;
        unsafe {
            let cm = &self.m.cm;
            let bytes = frames * 8;
            let mut block: *mut c_void = core::ptr::null_mut();
            let st = (cm.CMBlockBufferCreateWithMemoryBlock)(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                bytes,
                core::ptr::null_mut(),
                core::ptr::null(),
                0,
                bytes,
                1, // kCMBlockBufferAssureMemoryNowFlag
                &mut block,
            );
            if st != 0 || block.is_null() {
                bail!("no buffer for the sound ({st})");
            }
            (cm.CMBlockBufferReplaceDataBytes)(pcm.as_ptr().cast(), block, 0, bytes);
            let mut sbuf: *mut c_void = core::ptr::null_mut();
            let st = (cm.CMAudioSampleBufferCreateReadyWithPacketDescriptions)(
                core::ptr::null_mut(),
                block,
                self.fmt,
                frames as isize,
                CMTime::new(super::audio_pts(ts), super::RATE as i32),
                core::ptr::null(),
                &mut sbuf,
            );
            CFRelease(block);
            if st != 0 || sbuf.is_null() {
                bail!("no sample buffer for the sound ({st})");
            }
            let ok: i8 = msg1(audio.ptr(), objc_sel(c"appendSampleBuffer:"), sbuf);
            CFRelease(sbuf);
            if ok == 0 {
                return Err(self.fail("write the sound"));
            }
        }
        Ok(())
    }
}

/// A writer input of `media_type` with `settings`, for real-time data.
unsafe fn input(media_type: *mut c_void, settings: *mut c_void) -> Result<Id> {
    unsafe {
        let i: *mut c_void = msg2(
            objc_cls(c"AVAssetWriterInput"),
            objc_sel(c"assetWriterInputWithMediaType:outputSettings:"),
            media_type,
            settings,
        );
        let i = Id::retain(i).ok_or_else(|| anyhow!("create a writer input"))?;
        let _: () = msg1(i.ptr(), objc_sel(c"setExpectsMediaDataInRealTime:"), 1i8);
        Ok(i)
    }
}

unsafe fn add(writer: &Id, input: &Id, what: &str) -> Result<()> {
    unsafe {
        let ok: i8 = msg1(writer.ptr(), objc_sel(c"canAddInput:"), input.ptr());
        if ok == 0 {
            bail!("the MP4 writer takes no {what} input with these settings");
        }
        let _: () = msg1(writer.ptr(), objc_sel(c"addInput:"), input.ptr());
        Ok(())
    }
}

impl VideoEncoder for AvWriterEncoder {
    fn push_video(&mut self, f: &Frame) -> Result<()> {
        if let Some(e) = &self.failed {
            bail!("{e}");
        }
        let _pool = objc::Pool::new();
        self.feed_sound()?;
        if !Self::ready(&self.video) {
            self.dropped += 1;
            return Ok(());
        }
        unsafe {
            let pool: *mut c_void = msg0(self.adaptor.ptr(), objc_sel(c"pixelBufferPool"));
            if pool.is_null() {
                return Err(self.fail("the MP4 writer stopped"));
            }
            let mut pb: *mut c_void = core::ptr::null_mut();
            let st = (self.m.cv.CVPixelBufferPoolCreatePixelBuffer)(core::ptr::null_mut(), pool, &mut pb);
            if st != 0 || pb.is_null() {
                bail!("no pixel buffer for the frame ({st})");
            }
            let cv = &self.m.cv;
            if (cv.CVPixelBufferLockBaseAddress)(pb, 0) == 0 {
                let (w, h) = ((cv.CVPixelBufferGetWidth)(pb), (cv.CVPixelBufferGetHeight)(pb));
                let stride = (cv.CVPixelBufferGetBytesPerRow)(pb);
                let base = (cv.CVPixelBufferGetBaseAddress)(pb);
                if !base.is_null() {
                    let len = (cv.CVPixelBufferGetDataSize)(pb).max(stride * h);
                    let dst = std::slice::from_raw_parts_mut(base, len);
                    super::fill_rows(dst, stride, (w, h), &f.bgra, (f.w as usize, f.h as usize));
                }
                (cv.CVPixelBufferUnlockBaseAddress)(pb, 0);
            }
            let pts = super::video_pts(f.ts, self.last);
            let ok: i8 = msg2(
                self.adaptor.ptr(),
                objc_sel(c"appendPixelBuffer:withPresentationTime:"),
                pb,
                CMTime::new(pts, super::VIDEO_TIMESCALE),
            );
            CFRelease(pb);
            if ok == 0 {
                return Err(self.fail("write a frame"));
            }
            self.last = Some(pts);
            self.frames += 1;
        }
        Ok(())
    }

    fn push_audio(&mut self, pcm: &[f32], ts: Duration) -> Result<()> {
        if self.audio.is_none() {
            return Ok(());
        }
        if let Some(e) = &self.failed {
            bail!("{e}");
        }
        if pcm.len() < 2 {
            return Ok(());
        }
        let _pool = objc::Pool::new();
        self.sound.push(pcm, ts);
        self.feed_sound()
    }

    fn finish(mut self: Box<Self>) -> Result<()> {
        let _pool = objc::Pool::new();
        self.done = true;
        // The sound still queued: the input gets a moment to take it
        // (once, at the end; not on the way).
        let end = Instant::now() + SOUND_DRAIN;
        while self.failed.is_none() && !self.sound.is_empty() && Instant::now() < end {
            let before = self.sound.frames();
            if self.feed_sound().is_err() {
                break;
            }
            if self.sound.frames() == before {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if self.dropped > 0 {
            eprintln!("rustshot: {} frames dropped (the encoder was busy)", self.dropped);
        }
        let lost = self.sound.dropped + self.sound.frames() as u64;
        if lost > 0 {
            eprintln!("rustshot: {:.2} s of sound dropped (the writer was busy)", lost as f64 / f64::from(super::RATE));
        }
        if let Some(e) = self.failed.take() {
            unsafe {
                let _: () = msg0(self.writer.ptr(), objc_sel(c"cancelWriting"));
            }
            bail!("{e}");
        }
        if self.frames == 0 {
            unsafe {
                let _: () = msg0(self.writer.ptr(), objc_sel(c"cancelWriting"));
            }
            bail!("no frame was recorded");
        }
        unsafe {
            let _: () = msg0(self.video.ptr(), objc_sel(c"markAsFinished"));
            if let Some(a) = &self.audio {
                let _: () = msg0(a.ptr(), objc_sel(c"markAsFinished"));
            }
            let (block, rx) = objc::completion(0, |_| ());
            let _: () = msg1(self.writer.ptr(), objc_sel(c"finishWritingWithCompletionHandler:"), block.as_ptr());
            if rx.recv_timeout(FINISH).is_err() {
                bail!("the MP4 writer did not finish {} in time", self.part.display());
            }
            let status: isize = msg0(self.writer.ptr(), objc_sel(c"status"));
            if status != COMPLETED {
                bail!("finish the MP4: {}", self.writer_error());
            }
        }
        Ok(())
    }

    fn audio(&self) -> bool {
        self.audio.is_some()
    }
}

impl Drop for AvWriterEncoder {
    fn drop(&mut self) {
        unsafe {
            if !self.done {
                let _pool = objc::Pool::new();
                let _: () = msg0(self.writer.ptr(), objc_sel(c"cancelWriting"));
            }
            if !self.fmt.is_null() {
                CFRelease(self.fmt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A future Mac: record 2 s through ScreenCaptureKit into this
    /// encoder and check the file's duration with AVAsset. Screen only:
    /// an unbundled test binary has no Info.plist, and TCC kills a process
    /// that opens the microphone or the camera without
    /// `NSMicrophoneUsageDescription` / `NSCameraUsageDescription` — try
    /// those from the bundled Rustshot.app (packaging/macos/bundle.sh).
    #[test]
    #[ignore = "needs a Mac with Screen Recording permission"]
    fn live_rec_macos_records_two_seconds() {
        use crate::rec::{FrameSource, RecFormat, RecSpec, Session};
        let dir = std::env::temp_dir().join(format!("rustshot-live-mac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("live.mp4");
        let mons = crate::capture::monitors().unwrap();
        let m = mons.iter().find(|m| m.primary).unwrap_or(&mons[0]);
        let area = (m.x + 40, m.y + 40, 640, 360);
        let spec = RecSpec {
            format: RecFormat::Mp4,
            fps: 30,
            quality: Quality::Medium,
            area,
            system_audio: false,
            mic: None,
            out: out.clone(),
        };
        let opened = super::super::sck::open(crate::capture::to_points_i(area), 30, true, false, &[]).unwrap();
        let (w, h) = opened.video.size();
        let enc = AvWriterEncoder::create(&spec.part_path(), w, h, 30, Quality::Medium, false).unwrap();
        let s = Session::start(spec, Box::new(opened.video), Vec::new(), Box::new(enc)).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let saved = s.stop().unwrap();
        // AVAsset's duration (seconds).
        let secs = unsafe {
            let _pool = objc::Pool::new();
            let url: *mut c_void =
                msg1(objc_cls(c"NSURL"), objc_sel(c"fileURLWithPath:"), crate::wind::ns_string(&saved.to_string_lossy()));
            let asset: *mut c_void = msg1(objc_cls(c"AVURLAsset"), objc_sel(c"assetWithURL:"), url);
            let d: CMTime = objc::msg_big0(asset, objc_sel(c"duration"));
            d.seconds().unwrap()
        };
        let _ = std::fs::remove_dir_all(&dir);
        assert!((1.5..3.0).contains(&secs), "duration {secs}");
    }
}
