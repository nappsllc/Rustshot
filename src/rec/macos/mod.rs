//! macOS recording back end (compile-checked here; run on a Mac by hand,
//! see the task report's checklist):
//!
//! - frames and system sound from ScreenCaptureKit (`sck.rs`, macOS 12.3+;
//!   sound 13+): one `SCStream` of the display the area is on, with the
//!   area as its source rect, our own windows (border, bar, countdown)
//!   filtered out but the camera bubble kept;
//! - the microphone through an `AVCaptureSession` (`mic.rs`);
//! - MP4 through `AVAssetWriter`, H.264 + AAC (`avwriter.rs`); GIF through
//!   the shared encoder;
//! - the CoreMedia / CoreVideo / AVFoundation calls and sample-buffer
//!   plumbing (`media.rs`).
//!
//! ScreenCaptureKit, AVFoundation, CoreMedia and CoreVideo are loaded with
//! `dlopen` when first used (the app starts on macOS 11, and a screenshot
//! never pays for them): without ScreenCaptureKit (before 12.3) the Record
//! button is hidden ([`available`]).
//!
//! This file holds the platform-neutral parts (tested everywhere): CMTime
//! and audio format layouts, host time → `Instant`, PCM conversion, the
//! display and source rect of an area, which windows the filter keeps out,
//! presentation times.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

#[cfg(target_os = "macos")]
pub mod avwriter;
#[cfg(target_os = "macos")]
pub mod media;
#[cfg(target_os = "macos")]
pub mod mic;
#[cfg(target_os = "macos")]
pub mod sck;

use crate::capture::IRect;
use std::time::{Duration, Instant};

/// Shown when Screen Recording permission is off.
pub const SCREEN_DENIED: &str = "Allow Rustshot in System Settings › Privacy & Security › Screen Recording, then try again";
/// Shown when microphone access is off.
pub const MIC_DENIED: &str = "Allow Rustshot in System Settings › Privacy & Security › Microphone, then try again";
/// Shown when camera access is off.
pub const CAMERA_DENIED: &str = "Allow Rustshot in System Settings › Privacy & Security › Camera, then try again";

/// `'BGRA'` (`kCVPixelFormatType_32BGRA`).
pub const BGRA: u32 = fourcc(b"BGRA");
/// `kAudioFormatLinearPCM`.
pub const LPCM: u32 = fourcc(b"lpcm");
/// `kAudioFormatMPEG4AAC`.
pub const AAC: u32 = fourcc(b"aac ");
/// AAC bit rate (stereo).
pub const AAC_BITRATE: i64 = 160_000;
/// Sound rate asked of every source and written to the file.
pub const RATE: u32 = 48_000;
/// Video presentation times are in microseconds.
pub const VIDEO_TIMESCALE: i32 = 1_000_000;

/// A four-character code as macOS packs it (big-endian).
pub const fn fourcc(b: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*b)
}

/// `CMTime`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

impl CMTime {
    /// `kCMTimeFlags_Valid`.
    pub const VALID: u32 = 1;
    /// `kCMTimeFlags_HasBeenRounded` (does not make a time unusable).
    pub const ROUNDED: u32 = 1 << 1;
    /// `kCMTimeFlags_ImpliedValueFlagsMask`: positive / negative infinity,
    /// indefinite.
    pub const IMPLIED: u32 = (1 << 2) | (1 << 3) | (1 << 4);

    /// `CMTimeMake(value, timescale)`.
    pub fn new(value: i64, timescale: i32) -> CMTime {
        CMTime { value, timescale, flags: CMTime::VALID, epoch: 0 }
    }

    /// In seconds (`None` when invalid, indefinite or infinite).
    pub fn seconds(self) -> Option<f64> {
        // Valid and none of positive/negative infinity, indefinite; the
        // has-been-rounded flag does not matter.
        (self.flags & (CMTime::VALID | CMTime::IMPLIED) == CMTime::VALID && self.timescale > 0).then(|| self.value as f64 / f64::from(self.timescale))
    }
}

/// The instant of host time `pts` (seconds, `CMClockGetHostTimeClock`)
/// given that host time is `host_now` at `now`. Times in the future are
/// now.
pub fn instant_at(pts: f64, host_now: f64, now: Instant) -> Instant {
    let ago = host_now - pts;
    if !ago.is_finite() || ago <= 0.0 {
        return now;
    }
    now.checked_sub(Duration::from_secs_f64(ago)).unwrap_or(now)
}

/// `AudioStreamBasicDescription`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Asbd {
    pub sample_rate: f64,
    pub format_id: u32,
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
    pub reserved: u32,
}

/// `kAudioFormatFlagIsFloat`.
pub const FLAG_FLOAT: u32 = 1 << 0;
/// `kAudioFormatFlagIsBigEndian`.
pub const FLAG_BIG_ENDIAN: u32 = 1 << 1;
/// `kAudioFormatFlagIsPacked`.
pub const FLAG_PACKED: u32 = 1 << 3;
/// `kAudioFormatFlagIsNonInterleaved`.
pub const FLAG_NON_INTERLEAVED: u32 = 1 << 5;

impl Asbd {
    /// Interleaved stereo f32 at `rate`: what the writer is given.
    pub fn f32_stereo(rate: u32) -> Asbd {
        Asbd {
            sample_rate: f64::from(rate),
            format_id: LPCM,
            format_flags: FLAG_FLOAT | FLAG_PACKED,
            bytes_per_packet: 8,
            frames_per_packet: 1,
            bytes_per_frame: 8,
            channels_per_frame: 2,
            bits_per_channel: 32,
            reserved: 0,
        }
    }
}

/// `AudioBuffer`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AudioBuffer {
    pub channels: u32,
    pub bytes: u32,
    pub data: *mut core::ffi::c_void,
}

/// `AudioBufferList` with room for [`AudioBufferList::MAX`] buffers.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AudioBufferList {
    pub count: u32,
    pub buffers: [AudioBuffer; 8],
}

impl AudioBufferList {
    pub const MAX: usize = 8;

    pub fn empty() -> AudioBufferList {
        AudioBufferList {
            count: 0,
            buffers: [AudioBuffer { channels: 0, bytes: 0, data: core::ptr::null_mut() }; AudioBufferList::MAX],
        }
    }
}

/// `AudioChannelLayout` for stereo (`kAudioChannelLayoutTag_Stereo`,
/// no channel descriptions): the bytes of `AVChannelLayoutKey`.
pub fn stereo_layout() -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&((101u32 << 16) | 2).to_ne_bytes());
    b
}

/// Linear PCM as a capture delivers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub float: bool,
    /// Bytes per sample: 2 or 4.
    pub bytes: usize,
    pub big: bool,
    /// One buffer per channel.
    pub planar: bool,
    pub channels: usize,
}

/// The layout of `a`, when it is linear PCM this converts (f32, s16, s32).
pub fn pcm_of(a: &Asbd) -> Result<Pcm, String> {
    if a.format_id != LPCM {
        return Err(format!("not linear PCM (format {:#x})", a.format_id));
    }
    let float = a.format_flags & FLAG_FLOAT != 0;
    let bytes = match (float, a.bits_per_channel) {
        (true, 32) => 4,
        (false, 16) => 2,
        (false, 32) => 4,
        (f, b) => return Err(format!("unsupported PCM ({b}-bit {})", if f { "float" } else { "integer" })),
    };
    if a.channels_per_frame == 0 || a.sample_rate <= 0.0 {
        return Err("PCM without channels or rate".into());
    }
    Ok(Pcm {
        float,
        bytes,
        big: a.format_flags & FLAG_BIG_ENDIAN != 0,
        planar: a.format_flags & FLAG_NON_INTERLEAVED != 0,
        channels: a.channels_per_frame as usize,
    })
}

fn sample(p: &Pcm, b: &[u8]) -> f32 {
    match (p.float, p.bytes, p.big) {
        (true, _, false) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (true, _, true) => f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        (false, 2, false) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
        (false, 2, true) => f32::from(i16::from_be_bytes([b[0], b[1]])) / 32768.0,
        (false, _, false) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        (false, _, true) => i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
    }
}

/// Append `bufs` (laid out as `p`: one interleaved buffer, or one per
/// channel) to `out` as interleaved stereo f32: mono is doubled, channels
/// past the second are left out. Returns the frames appended.
pub fn to_stereo(p: &Pcm, bufs: &[&[u8]], out: &mut Vec<f32>) -> usize {
    let Some(first) = bufs.first() else { return 0 };
    let (b, ch) = (p.bytes, p.channels);
    if p.planar {
        if bufs.len() < ch.min(2) {
            return 0;
        }
        let n = bufs.iter().take(ch.min(2)).map(|x| x.len() / b).min().unwrap_or(0);
        let r = if ch >= 2 { bufs[1] } else { first };
        for i in 0..n {
            out.extend([sample(p, &first[i * b..]), sample(p, &r[i * b..])]);
        }
        n
    } else {
        let n = first.len() / (b * ch);
        for i in 0..n {
            let f = &first[i * b * ch..];
            let l = sample(p, f);
            let r = if ch >= 2 { sample(p, &f[b..]) } else { l };
            out.extend([l, r]);
        }
        n
    }
}

/// Whether a packet starting at `start` (seconds) follows on from one
/// that ended at `prev_end` (within 2 ms).
pub fn follows(prev_end: Option<f64>, start: f64) -> bool {
    prev_end.is_some_and(|e| (start - e).abs() < 0.002)
}

/// A rectangle in points (Quartz: top-left origin, y down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectF {
    fn of(r: IRect) -> RectF {
        RectF { x: f64::from(r.0), y: f64::from(r.1), w: f64::from(r.2), h: f64::from(r.3) }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    fn overlap(&self, o: &RectF) -> Option<RectF> {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = ((self.x + self.w).min(o.x + o.w), (self.y + self.h).min(o.y + o.h));
        (x1 > x0 && y1 > y0).then_some(RectF { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }
}

/// The display to record `area` (points) from: the one with its centre,
/// else the one it overlaps most.
pub fn pick_display(area: IRect, displays: &[RectF]) -> Option<usize> {
    let a = RectF::of(area);
    let (cx, cy) = (a.x + a.w / 2.0, a.y + a.h / 2.0);
    if let Some(i) = displays.iter().position(|d| d.contains(cx, cy)) {
        return Some(i);
    }
    let size = |i: &usize| displays[*i].overlap(&a).map_or(0.0, |o| o.w * o.h);
    (0..displays.len()).filter(|i| size(i) > 0.0).max_by(|i, j| size(i).total_cmp(&size(j)))
}

/// What the stream captures: `src` in the display's own points (its
/// top-left is 0, 0) and the frame size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamGeom {
    pub src: RectF,
    pub w: u32,
    pub h: u32,
}

/// The part of `area` (global points) on `display` as the stream's
/// source rect, and its size in pixels at `px_per_pt`: for MP4 scaled to
/// fit 4096×2304 with even sides ([`crate::rec::mp4_fit`]), for GIF as
/// it is. `None` when the area is not on the display.
pub fn stream_geom(area: IRect, display: RectF, px_per_pt: f64, mp4: bool) -> Option<StreamGeom> {
    let o = RectF::of(area).overlap(&display)?;
    let src = RectF { x: o.x - display.x, y: o.y - display.y, w: o.w, h: o.h };
    let s = if px_per_pt.is_finite() && px_per_pt > 0.0 { px_per_pt } else { 1.0 };
    let (w, h) = (((o.w * s).round() as u32).max(1), ((o.h * s).round() as u32).max(1));
    let (w, h) = if mp4 { crate::rec::mp4_fit(w, h) } else { (w, h) };
    Some(StreamGeom { src, w, h })
}

/// How the stream keeps our windows out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterPlan {
    /// Exclude our application (`applications[app]`), except these windows
    /// (indices into the window list): everything of ours, even windows
    /// that come up later, is left out but the camera bubble.
    App { app: usize, except: Vec<usize> },
    /// Our application is not listed: exclude these windows (ours now,
    /// except the bubble).
    Windows(Vec<usize>),
}

/// The filter for a recording by process `me`: `apps` are the shareable
/// applications' process ids, `windows` the shareable windows' (id,
/// owner's process id), `keep` our windows that are recorded (the bubble).
pub fn plan_filter(apps: &[i32], windows: &[(u32, i32)], me: i32, keep: &[u32]) -> FilterPlan {
    let ours = |w: &(u32, i32)| w.1 == me;
    match apps.iter().position(|p| *p == me) {
        Some(app) => FilterPlan::App {
            app,
            except: (0..windows.len()).filter(|&i| ours(&windows[i]) && keep.contains(&windows[i].0)).collect(),
        },
        None => FilterPlan::Windows((0..windows.len()).filter(|&i| ours(&windows[i]) && !keep.contains(&windows[i].0)).collect()),
    }
}

/// `SCFrameStatusComplete`: a frame with new content. The others (idle,
/// blank, suspended, started, stopped) carry no new picture.
pub const FRAME_COMPLETE: i64 = 0;

/// Whether a screen sample buffer is a new picture, by its
/// `SCStreamFrameInfoStatus` attachment (`None` when it has none: then
/// whether it has an image decides).
pub fn frame_is_new(status: Option<i64>) -> bool {
    status.is_none_or(|s| s == FRAME_COMPLETE)
}

/// Sound waiting for the writer's audio input, at most `cap` frames: when
/// more comes, the oldest goes (and is counted). The encode thread never
/// waits for the writer; what is queued goes in as the input takes it.
#[derive(Debug, Default)]
pub struct SoundQueue {
    q: std::collections::VecDeque<(Vec<f32>, Duration)>,
    frames: usize,
    cap: usize,
    /// Stereo frames dropped because the writer fell behind.
    pub dropped: u64,
}

impl SoundQueue {
    pub fn new(cap: usize) -> SoundQueue {
        SoundQueue { cap, ..SoundQueue::default() }
    }

    /// Queue interleaved stereo `pcm` starting at `ts`.
    pub fn push(&mut self, pcm: &[f32], ts: Duration) {
        if pcm.len() < 2 {
            return;
        }
        self.frames += pcm.len() / 2;
        self.q.push_back((pcm.to_vec(), ts));
        while self.frames > self.cap && self.q.len() > 1 {
            if let Some((old, _)) = self.q.pop_front() {
                self.frames -= old.len() / 2;
                self.dropped += (old.len() / 2) as u64;
            }
        }
    }

    /// The oldest queued sound.
    pub fn pop(&mut self) -> Option<(Vec<f32>, Duration)> {
        let v = self.q.pop_front()?;
        self.frames -= v.0.len() / 2;
        Some(v)
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// Stereo frames queued.
    pub fn frames(&self) -> usize {
        self.frames
    }
}

/// The presentation time of a frame stamped `ts`, in
/// [`VIDEO_TIMESCALE`]: strictly after `last` (a frame stamped at a pause
/// repeats the time; the writer refuses equal or earlier times).
pub fn video_pts(ts: Duration, last: Option<i64>) -> i64 {
    let v = i64::try_from(ts.as_micros()).unwrap_or(i64::MAX);
    match last {
        Some(l) if v <= l => l + 1,
        _ => v,
    }
}

/// The sample index (48 kHz) of sound starting at `ts`.
pub fn audio_pts(ts: Duration) -> i64 {
    ((ts.as_nanos() * u128::from(RATE) + 500_000_000) / 1_000_000_000) as i64
}

/// Copy `h` rows of `w` BGRA pixels from `src` (rows `stride` bytes apart)
/// into `out` (tight rows, replaced). False when `src` is too short.
pub fn copy_rows(src: &[u8], stride: usize, w: usize, h: usize, out: &mut Vec<u8>) -> bool {
    let row = w * 4;
    if h == 0 || stride < row || src.len() < stride * (h - 1) + row {
        return false;
    }
    out.clear();
    out.reserve(row * h);
    for y in 0..h {
        out.extend_from_slice(&src[y * stride..][..row]);
    }
    true
}

/// Write the tight BGRA picture `src` (`sw`×`sh`) into `dst` (`dw`×`dh`,
/// rows `stride` bytes apart): the overlap is copied, the rest is black.
pub fn fill_rows(dst: &mut [u8], stride: usize, (dw, dh): (usize, usize), src: &[u8], (sw, sh): (usize, usize)) {
    let (w, h) = (dw.min(sw), dh.min(sh));
    for y in 0..dh {
        let Some(d) = dst.get_mut(y * stride..y * stride + dw * 4) else { return };
        if y < h && src.len() >= (y * sw + w) * 4 {
            d[..w * 4].copy_from_slice(&src[y * sw * 4..][..w * 4]);
            d[w * 4..].fill(0);
        } else {
            d.fill(0);
        }
        // Opaque black where there is no picture.
        for p in d.as_chunks_mut::<4>().0.iter_mut().skip(if y < h { w } else { 0 }) {
            p[3] = 255;
        }
    }
}

// ---------------------------------------------------------------------------
// macOS: availability, permission, displays
// ---------------------------------------------------------------------------

/// Whether ScreenCaptureKit can be used (macOS 12.3+): the Record button
/// shows only then. Loads it the first time.
pub fn available() -> bool {
    #[cfg(target_os = "macos")]
    {
        media::sck().is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "macos")]
mod mac {
    use crate::capture::{CGPoint, CGRect};
    use anyhow::{bail, Result};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
        fn CGDisplayBounds(id: u32) -> CGRect;
        fn CGGetDisplaysWithPoint(p: CGPoint, max: u32, ids: *mut u32, count: *mut u32) -> i32;
    }

    /// Screen Recording permission: asked for (the system's prompt, the
    /// first time) when it is not given; the error says where to turn it
    /// on. A permission given while running counts after a restart.
    pub fn screen_access() -> Result<()> {
        unsafe {
            if CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() {
                return Ok(());
            }
        }
        bail!(super::SCREEN_DENIED)
    }

    /// Display `id` in global points.
    pub fn display_rect(id: u32) -> super::RectF {
        let b = unsafe { CGDisplayBounds(id) };
        super::RectF { x: b.origin.x, y: b.origin.y, w: b.size.width, h: b.size.height }
    }

    /// Pixels per point of display `id` (2 on Retina; 1 when unknown).
    pub fn display_scale(id: u32) -> f64 {
        crate::capture::display_scale(id)
    }

    /// Pixels per point of the display at (x, y) global points.
    pub fn backing_scale_at(x: f64, y: f64) -> f64 {
        let (mut id, mut n) = (0u32, 0u32);
        let ok = unsafe { CGGetDisplaysWithPoint(CGPoint { x, y }, 1, &mut id, &mut n) } == 0;
        if ok && n > 0 { display_scale(id) } else { 1.0 }
    }
}

#[cfg(test)]
mod tests;
