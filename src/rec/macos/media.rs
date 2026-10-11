//! CoreMedia, CoreVideo, AVFoundation and ScreenCaptureKit, loaded with
//! `dlopen` on first use ([`media`], [`sck`]), and the sample-buffer
//! plumbing shared by the screen stream, the microphone and the camera:
//! a delegate object that hands sample buffers to a Rust closure
//! ([`SampleSink`]), BGRA pixels out of a buffer ([`copy_pixels`]) and
//! timed stereo PCM into a [`Ring`] ([`PcmFeed`], read as an
//! [`AudioSource`] through [`FeedSource`]).

use super::{Asbd, AudioBufferList, CMTime, Pcm};
use crate::macos_objc::{self as objc, Id};
use crate::rec::ring::Ring;
use crate::rec::AudioSource;
use crate::wind::{msg0, msg1, msg2, objc_cls, objc_sel};
use anyhow::{anyhow, bail, Result};
use core::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Loaded functions and constants, by name.
macro_rules! table {
    ($(#[$m:meta])* $name:ident { $($f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[allow(non_snake_case)]
        pub struct $name { $(pub $f: $t,)* }

        impl $name {
            /// Every symbol from `lib`; `None` when one is missing.
            unsafe fn load(lib: *mut c_void) -> Option<$name> {
                Some($name { $($f: {
                    let name = CStr::from_bytes_with_nul(concat!(stringify!($f), "\0").as_bytes()).ok()?;
                    let p = objc::sym(lib, name);
                    if p.is_null() {
                        return None;
                    }
                    unsafe { std::mem::transmute::<*mut c_void, $t>(p) }
                },)* })
            }
        }
    };
}

type Cf = *mut c_void;

table!(
    /// CoreMedia.
    Cm {
        CMSampleBufferGetImageBuffer: unsafe extern "C" fn(Cf) -> Cf,
        CMSampleBufferGetPresentationTimeStamp: unsafe extern "C" fn(Cf) -> CMTime,
        CMSampleBufferGetFormatDescription: unsafe extern "C" fn(Cf) -> Cf,
        CMSampleBufferGetNumSamples: unsafe extern "C" fn(Cf) -> isize,
        CMSampleBufferGetSampleAttachmentsArray: unsafe extern "C" fn(Cf, u8) -> Cf,
        CMAudioFormatDescriptionGetStreamBasicDescription: unsafe extern "C" fn(Cf) -> *const Asbd,
        CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer:
            unsafe extern "C" fn(Cf, *mut usize, *mut AudioBufferList, usize, Cf, Cf, u32, *mut Cf) -> i32,
        CMClockGetHostTimeClock: unsafe extern "C" fn() -> Cf,
        CMClockGetTime: unsafe extern "C" fn(Cf) -> CMTime,
        CMSyncConvertTime: unsafe extern "C" fn(CMTime, Cf, Cf) -> CMTime,
        CMAudioFormatDescriptionCreate:
            unsafe extern "C" fn(Cf, *const Asbd, usize, *const c_void, usize, *const c_void, Cf, *mut Cf) -> i32,
        CMBlockBufferCreateWithMemoryBlock:
            unsafe extern "C" fn(Cf, *mut c_void, usize, Cf, *const c_void, usize, usize, u32, *mut Cf) -> i32,
        CMBlockBufferReplaceDataBytes: unsafe extern "C" fn(*const c_void, Cf, usize, usize) -> i32,
        CMAudioSampleBufferCreateReadyWithPacketDescriptions:
            unsafe extern "C" fn(Cf, Cf, Cf, isize, CMTime, *const c_void, *mut Cf) -> i32,
    }
);

table!(
    /// CoreVideo.
    Cv {
        CVPixelBufferLockBaseAddress: unsafe extern "C" fn(Cf, u64) -> i32,
        CVPixelBufferUnlockBaseAddress: unsafe extern "C" fn(Cf, u64) -> i32,
        CVPixelBufferGetBaseAddress: unsafe extern "C" fn(Cf) -> *mut u8,
        CVPixelBufferGetBytesPerRow: unsafe extern "C" fn(Cf) -> usize,
        CVPixelBufferGetWidth: unsafe extern "C" fn(Cf) -> usize,
        CVPixelBufferGetHeight: unsafe extern "C" fn(Cf) -> usize,
        CVPixelBufferGetDataSize: unsafe extern "C" fn(Cf) -> usize,
        CVPixelBufferPoolCreatePixelBuffer: unsafe extern "C" fn(Cf, Cf, *mut Cf) -> i32,
    }
);

/// The AVFoundation / CoreVideo string constants used (NSString / CFString).
#[allow(non_snake_case)]
pub struct Keys {
    pub AVMediaTypeVideo: Cf,
    pub AVMediaTypeAudio: Cf,
    pub AVFileTypeMPEG4: Cf,
    pub AVVideoCodecKey: Cf,
    pub AVVideoCodecTypeH264: Cf,
    pub AVVideoWidthKey: Cf,
    pub AVVideoHeightKey: Cf,
    pub AVVideoCompressionPropertiesKey: Cf,
    pub AVVideoAverageBitRateKey: Cf,
    pub AVVideoMaxKeyFrameIntervalKey: Cf,
    pub AVVideoExpectedSourceFrameRateKey: Cf,
    pub AVVideoProfileLevelKey: Cf,
    pub AVVideoProfileLevelH264HighAutoLevel: Cf,
    pub AVFormatIDKey: Cf,
    pub AVSampleRateKey: Cf,
    pub AVNumberOfChannelsKey: Cf,
    pub AVEncoderBitRateKey: Cf,
    pub AVChannelLayoutKey: Cf,
    pub AVLinearPCMBitDepthKey: Cf,
    pub AVLinearPCMIsFloatKey: Cf,
    pub AVLinearPCMIsNonInterleaved: Cf,
    pub AVLinearPCMIsBigEndianKey: Cf,
    pub AVCaptureSessionPreset1280x720: Cf,
    pub kCVPixelBufferPixelFormatTypeKey: Cf,
    pub kCVPixelBufferWidthKey: Cf,
    pub kCVPixelBufferHeightKey: Cf,
}

/// Everything loaded.
pub struct Media {
    pub cm: Cm,
    pub cv: Cv,
    pub k: Keys,
}

// Function pointers and immortal constant objects.
unsafe impl Send for Media {}
unsafe impl Sync for Media {}

const FRAMEWORKS: &str = "/System/Library/Frameworks/";

fn framework(name: &str) -> *mut c_void {
    let path = format!("{FRAMEWORKS}{name}.framework/{name}\0");
    CStr::from_bytes_with_nul(path.as_bytes()).map_or(core::ptr::null_mut(), objc::load)
}

unsafe fn load_media() -> Option<Media> {
    unsafe {
        let (av, cm, cv) = (framework("AVFoundation"), framework("CoreMedia"), framework("CoreVideo"));
        if av.is_null() || cm.is_null() || cv.is_null() {
            return None;
        }
        let c = |lib, n: &CStr| objc::const_obj(lib, n);
        let k = Keys {
            AVMediaTypeVideo: c(av, c"AVMediaTypeVideo"),
            AVMediaTypeAudio: c(av, c"AVMediaTypeAudio"),
            AVFileTypeMPEG4: c(av, c"AVFileTypeMPEG4"),
            AVVideoCodecKey: c(av, c"AVVideoCodecKey"),
            // 10.13+; the older constant has the same value.
            AVVideoCodecTypeH264: {
                let v = c(av, c"AVVideoCodecTypeH264");
                if v.is_null() { c(av, c"AVVideoCodecH264") } else { v }
            },
            AVVideoWidthKey: c(av, c"AVVideoWidthKey"),
            AVVideoHeightKey: c(av, c"AVVideoHeightKey"),
            AVVideoCompressionPropertiesKey: c(av, c"AVVideoCompressionPropertiesKey"),
            AVVideoAverageBitRateKey: c(av, c"AVVideoAverageBitRateKey"),
            AVVideoMaxKeyFrameIntervalKey: c(av, c"AVVideoMaxKeyFrameIntervalKey"),
            AVVideoExpectedSourceFrameRateKey: c(av, c"AVVideoExpectedSourceFrameRateKey"),
            AVVideoProfileLevelKey: c(av, c"AVVideoProfileLevelKey"),
            AVVideoProfileLevelH264HighAutoLevel: c(av, c"AVVideoProfileLevelH264HighAutoLevel"),
            AVFormatIDKey: c(av, c"AVFormatIDKey"),
            AVSampleRateKey: c(av, c"AVSampleRateKey"),
            AVNumberOfChannelsKey: c(av, c"AVNumberOfChannelsKey"),
            AVEncoderBitRateKey: c(av, c"AVEncoderBitRateKey"),
            AVChannelLayoutKey: c(av, c"AVChannelLayoutKey"),
            AVLinearPCMBitDepthKey: c(av, c"AVLinearPCMBitDepthKey"),
            AVLinearPCMIsFloatKey: c(av, c"AVLinearPCMIsFloatKey"),
            AVLinearPCMIsNonInterleaved: c(av, c"AVLinearPCMIsNonInterleaved"),
            AVLinearPCMIsBigEndianKey: c(av, c"AVLinearPCMIsBigEndianKey"),
            AVCaptureSessionPreset1280x720: c(av, c"AVCaptureSessionPreset1280x720"),
            kCVPixelBufferPixelFormatTypeKey: c(cv, c"kCVPixelBufferPixelFormatTypeKey"),
            kCVPixelBufferWidthKey: c(cv, c"kCVPixelBufferWidthKey"),
            kCVPixelBufferHeightKey: c(cv, c"kCVPixelBufferHeightKey"),
        };
        // The keys every path needs.
        if k.AVMediaTypeVideo.is_null() || k.AVMediaTypeAudio.is_null() || k.kCVPixelBufferPixelFormatTypeKey.is_null() {
            return None;
        }
        Some(Media { cm: Cm::load(cm)?, cv: Cv::load(cv)?, k })
    }
}

/// CoreMedia, CoreVideo and AVFoundation (loaded once).
pub fn media() -> Result<&'static Media> {
    static M: OnceLock<Option<Media>> = OnceLock::new();
    M.get_or_init(|| unsafe { load_media() }).as_ref().ok_or_else(|| anyhow!("AVFoundation could not be loaded"))
}

/// ScreenCaptureKit's image, when it loads and has `SCStream` (12.3+).
pub fn sck() -> Option<usize> {
    static S: OnceLock<Option<usize>> = OnceLock::new();
    *S.get_or_init(|| {
        let lib = framework("ScreenCaptureKit");
        (!lib.is_null() && !objc_cls(c"SCStream").is_null() && !objc_cls(c"SCShareableContent").is_null())
            .then_some(lib as usize)
    })
}

// `CFRelease` (CoreFoundation is linked already).
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFRelease(cf: *const c_void);
}

/// The instant a sample buffer's presentation time stands for; `clock`
/// is the clock it is in (null = the host clock).
pub unsafe fn instant_of(m: &Media, pts: CMTime, clock: Cf) -> Option<(Instant, f64)> {
    unsafe {
        let now = Instant::now();
        let host = (m.cm.CMClockGetHostTimeClock)();
        let pts = if clock.is_null() || clock == host { pts } else { (m.cm.CMSyncConvertTime)(pts, clock, host) };
        let at = pts.seconds()?;
        let host_now = (m.cm.CMClockGetTime)(host).seconds()?;
        Some((super::instant_at(at, host_now, now), at))
    }
}

/// The BGRA pixels of a sample buffer's image (tight rows) into `out`;
/// its size, or `None` when it has no image (an idle frame) or another
/// layout.
pub unsafe fn copy_pixels(m: &Media, sbuf: Cf, out: &mut Vec<u8>) -> Option<(u32, u32)> {
    unsafe {
        let pb = (m.cm.CMSampleBufferGetImageBuffer)(sbuf);
        if pb.is_null() {
            return None;
        }
        copy_image(m, pb, out)
    }
}

/// [`copy_pixels`] of a `CVPixelBuffer`.
pub unsafe fn copy_image(m: &Media, pb: Cf, out: &mut Vec<u8>) -> Option<(u32, u32)> {
    unsafe {
        const READ_ONLY: u64 = 1;
        if (m.cv.CVPixelBufferLockBaseAddress)(pb, READ_ONLY) != 0 {
            return None;
        }
        let (w, h) = ((m.cv.CVPixelBufferGetWidth)(pb), (m.cv.CVPixelBufferGetHeight)(pb));
        let (base, stride) = ((m.cv.CVPixelBufferGetBaseAddress)(pb), (m.cv.CVPixelBufferGetBytesPerRow)(pb));
        let size = (m.cv.CVPixelBufferGetDataSize)(pb).max(stride * h);
        let ok = !base.is_null() && w > 0 && h > 0 && super::copy_rows(std::slice::from_raw_parts(base, size), stride, w, h, out);
        (m.cv.CVPixelBufferUnlockBaseAddress)(pb, READ_ONLY);
        ok.then_some((w as u32, h as u32))
    }
}

/// A sample buffer's sound as interleaved stereo f32 appended to `out`:
/// (frames, rate), or why it cannot be read.
pub unsafe fn stereo_of(m: &Media, sbuf: Cf, out: &mut Vec<f32>) -> Result<(usize, f64), String> {
    unsafe {
        let fd = (m.cm.CMSampleBufferGetFormatDescription)(sbuf);
        let a = if fd.is_null() { core::ptr::null() } else { (m.cm.CMAudioFormatDescriptionGetStreamBasicDescription)(fd) };
        if a.is_null() {
            return Err("sound without a format".into());
        }
        let asbd = *a;
        let pcm: Pcm = super::pcm_of(&asbd)?;
        let mut abl = AudioBufferList::empty();
        let mut block: Cf = core::ptr::null_mut();
        let st = (m.cm.CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer)(
            sbuf,
            core::ptr::null_mut(),
            &mut abl,
            std::mem::size_of::<AudioBufferList>(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            1, // kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment
            &mut block,
        );
        if st != 0 {
            return Err(format!("sound buffer unreadable ({st})"));
        }
        let n = (abl.count as usize).min(AudioBufferList::MAX);
        let bufs: Vec<&[u8]> = abl.buffers[..n]
            .iter()
            .filter(|b| !b.data.is_null())
            .map(|b| std::slice::from_raw_parts(b.data as *const u8, b.bytes as usize))
            .collect();
        let frames = super::to_stereo(&pcm, &bufs, out);
        if !block.is_null() {
            CFRelease(block);
        }
        Ok((frames, asbd.sample_rate))
    }
}

/// Timed stereo sound from a capture callback into a [`Ring`].
pub struct PcmFeed {
    ring: Ring,
    rate: AtomicU32,
    /// Where the last packet ended (seconds of its clock).
    end: Mutex<Option<f64>>,
    /// The clock the timestamps are in (null = host time).
    clock: Mutex<usize>,
    /// The clock is known: packets before that are dropped (their times
    /// would be read in the wrong clock).
    ready: AtomicBool,
    /// The first problem with the sound (some of it was dropped), said
    /// once with the saved file ([`PcmFeed::problem`]).
    problem: Mutex<Option<String>>,
    scratch: Mutex<Vec<f32>>,
}

impl PcmFeed {
    /// A feed timed in host time (ScreenCaptureKit).
    pub fn new() -> Arc<PcmFeed> {
        PcmFeed::make(true)
    }

    /// A feed timed by a clock given later with [`PcmFeed::set_clock`]
    /// (an `AVCaptureSession`'s, known once it runs); nothing is taken
    /// before.
    pub fn awaiting_clock() -> Arc<PcmFeed> {
        PcmFeed::make(false)
    }

    fn make(ready: bool) -> Arc<PcmFeed> {
        Arc::new(PcmFeed {
            ring: Ring::new(super::RATE as usize * 2),
            rate: AtomicU32::new(super::RATE),
            end: Mutex::new(None),
            clock: Mutex::new(0),
            ready: AtomicBool::new(ready),
            problem: Mutex::new(None),
            scratch: Mutex::new(Vec::new()),
        })
    }

    /// Timestamps are in `clock` (a `CMClock`, kept alive by its owner;
    /// null = host time). Packets are taken from now on.
    pub fn set_clock(&self, clock: Cf) {
        *self.clock.lock().unwrap_or_else(|e| e.into_inner()) = clock as usize;
        self.ready.store(true, Ordering::Release);
    }

    /// The first problem that dropped sound, if any (a recording warning).
    pub fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Note a problem once (logged and kept for [`PcmFeed::problem`]).
    fn warn(&self, e: String) {
        let mut p = self.problem.lock().unwrap_or_else(|e| e.into_inner());
        if p.is_none() {
            eprintln!("rustshot: sound dropped: {e}");
            *p = Some(e);
        }
    }

    /// Queue a sample buffer's sound (on the capture queue).
    pub unsafe fn push(&self, m: &Media, sbuf: Cf) {
        if !self.ready.load(Ordering::Acquire) {
            return;
        }
        let mut data = self.scratch.lock().unwrap_or_else(|e| e.into_inner());
        data.clear();
        let (frames, rate) = match unsafe { stereo_of(m, sbuf, &mut data) } {
            Ok(v) => v,
            Err(e) => return self.warn(e),
        };
        if frames == 0 || rate <= 0.0 {
            return;
        }
        let clock = *self.clock.lock().unwrap_or_else(|e| e.into_inner()) as Cf;
        let pts = unsafe { (m.cm.CMSampleBufferGetPresentationTimeStamp)(sbuf) };
        let Some((at, start)) = (unsafe { instant_of(m, pts, clock) }) else {
            return self.warn(format!("sound without a usable time (CMTime flags {:#x})", pts.flags));
        };
        let r = rate.round() as u32;
        if self.rate.swap(r, Ordering::Relaxed) != r {
            self.ring.set_cap(r as usize * 2);
        }
        let mut end = self.end.lock().unwrap_or_else(|e| e.into_inner());
        let cont = super::follows(*end, start);
        *end = Some(start + frames as f64 / rate);
        self.ring.push(at, &data, cont);
    }
}

/// A [`PcmFeed`] read by the mixer (stereo).
pub struct FeedSource {
    pub feed: Arc<PcmFeed>,
    /// What keeps the capture running (dropped with the source).
    pub _keep: Option<Box<dyn Send>>,
}

impl AudioSource for FeedSource {
    fn rate(&self) -> u32 {
        self.feed.rate.load(Ordering::Relaxed)
    }

    fn channels(&self) -> u16 {
        2
    }

    fn read(&mut self, out: &mut Vec<f32>) -> usize {
        self.read_timed(out).0
    }

    fn read_timed(&mut self, out: &mut Vec<f32>) -> (usize, Option<Instant>) {
        self.feed.ring.take(out)
    }
}

/// What a [`SampleSink`] calls: (sample buffer, output type: 0 screen,
/// 1 sound for ScreenCaptureKit; -1 for an AVCapture output).
type OnSample = dyn Fn(Cf, isize) + Send + Sync;
/// Called when a stream stopped with an error.
pub type OnStop = dyn Fn(String) + Send + Sync;

struct SinkCtx {
    sample: Box<OnSample>,
    stop: Option<Box<OnStop>>,
}

/// A delegate object for `SCStream` (output and delegate) and the AVCapture
/// data outputs, calling Rust closures. Drop it only once no callback can
/// come any more (the stream or session stopped and its queue drained).
pub struct SampleSink {
    obj: Id,
    ctx: *const SinkCtx,
}

unsafe impl Send for SampleSink {}
unsafe impl Sync for SampleSink {}

unsafe fn sink_of<'a>(this: *mut c_void) -> Option<&'a SinkCtx> {
    unsafe {
        let p = objc::ctx(this) as *const SinkCtx;
        (!p.is_null()).then(|| &*p)
    }
}

/// `-stream:didOutputSampleBuffer:ofType:`.
unsafe extern "C" fn sck_output(this: *mut c_void, _: usize, _stream: *mut c_void, sbuf: Cf, ty: isize) {
    let _pool = objc::Pool::new();
    if let Some(s) = unsafe { sink_of(this) } {
        (s.sample)(sbuf, ty);
    }
}

/// `-stream:didStopWithError:`.
unsafe extern "C" fn sck_stopped(this: *mut c_void, _: usize, _stream: *mut c_void, err: *mut c_void) {
    let _pool = objc::Pool::new();
    if let Some(s) = unsafe { sink_of(this) }
        && let Some(f) = &s.stop
    {
        f(unsafe { objc::error_text(err) });
    }
}

/// `-captureOutput:didOutputSampleBuffer:fromConnection:`.
unsafe extern "C" fn av_output(this: *mut c_void, _: usize, _out: *mut c_void, sbuf: Cf, _conn: *mut c_void) {
    let _pool = objc::Pool::new();
    if let Some(s) = unsafe { sink_of(this) } {
        (s.sample)(sbuf, -1);
    }
}

impl SampleSink {
    pub fn new(sample: impl Fn(Cf, isize) + Send + Sync + 'static, stop: Option<Box<OnStop>>) -> Result<SampleSink> {
        let sck_out: unsafe extern "C" fn(*mut c_void, usize, *mut c_void, Cf, isize) = sck_output;
        let sck_stop: unsafe extern "C" fn(*mut c_void, usize, *mut c_void, *mut c_void) = sck_stopped;
        let av_out: unsafe extern "C" fn(*mut c_void, usize, *mut c_void, Cf, *mut c_void) = av_output;
        let cls = objc::make_class(
            c"RustshotSampleSink",
            &[
                c"SCStreamOutput",
                c"SCStreamDelegate",
                c"AVCaptureAudioDataOutputSampleBufferDelegate",
                c"AVCaptureVideoDataOutputSampleBufferDelegate",
            ],
            &[
                (c"stream:didOutputSampleBuffer:ofType:", sck_out as *const c_void, c"v@:@^vq"),
                (c"stream:didStopWithError:", sck_stop as *const c_void, c"v@:@@"),
                (c"captureOutput:didOutputSampleBuffer:fromConnection:", av_out as *const c_void, c"v@:@^v@"),
            ],
        );
        let obj = unsafe { objc::new_obj(cls) }.ok_or_else(|| anyhow!("create the capture delegate"))?;
        let ctx = Arc::into_raw(Arc::new(SinkCtx { sample: Box::new(sample), stop }));
        unsafe { objc::set_ctx(obj.ptr(), ctx.cast()) };
        Ok(SampleSink { obj, ctx })
    }

    pub fn ptr(&self) -> *mut c_void {
        self.obj.ptr()
    }
}

impl Drop for SampleSink {
    fn drop(&mut self) {
        unsafe {
            objc::set_ctx(self.obj.ptr(), core::ptr::null());
            drop(Arc::from_raw(self.ctx));
        }
    }
}

/// Camera or microphone access (`media_type`: AVMediaTypeVideo / Audio):
/// asked for when not decided yet (the system's prompt; waits for the
/// answer), `denied` when it is off.
pub fn av_access(media_type: Cf, denied: &str) -> Result<()> {
    let _pool = objc::Pool::new();
    let dev = objc_cls(c"AVCaptureDevice");
    if dev.is_null() {
        bail!("AVFoundation is not available");
    }
    let status: isize = unsafe { msg1(dev, objc_sel(c"authorizationStatusForMediaType:"), media_type) };
    match status {
        3 => Ok(()), // authorized
        0 => {
            let (block, rx) = objc::completion(1, |a| a[0] as u8 != 0);
            let _: () = unsafe { msg2(dev, objc_sel(c"requestAccessForMediaType:completionHandler:"), media_type, block.as_ptr()) };
            match rx.recv_timeout(Duration::from_secs(120)) {
                Ok(true) => Ok(()),
                _ => bail!("{denied}"),
            }
        }
        _ => bail!("{denied}"),
    }
}

/// (uniqueID, localizedName) of the capture devices of `media_type`.
pub fn devices(media_type: Cf) -> Vec<(String, String)> {
    let _pool = objc::Pool::new();
    let dev = objc_cls(c"AVCaptureDevice");
    if dev.is_null() {
        return Vec::new();
    }
    unsafe {
        let arr: *mut c_void = msg1(dev, objc_sel(c"devicesWithMediaType:"), media_type);
        objc::items(arr)
            .into_iter()
            .map(|d| {
                let id: *mut c_void = msg0(d, objc_sel(c"uniqueID"));
                let name: *mut c_void = msg0(d, objc_sel(c"localizedName"));
                (objc::string_of(id), objc::string_of(name))
            })
            .filter(|(id, _)| !id.is_empty())
            .collect()
    }
}

/// The capture device `id` of `media_type` ("" = the default one).
pub unsafe fn device(media_type: Cf, id: &str) -> Option<*mut c_void> {
    unsafe {
        let dev = objc_cls(c"AVCaptureDevice");
        let d: *mut c_void = if id.is_empty() {
            msg1(dev, objc_sel(c"defaultDeviceWithMediaType:"), media_type)
        } else {
            msg1(dev, objc_sel(c"deviceWithUniqueID:"), crate::wind::ns_string(id))
        };
        (!d.is_null()).then_some(d)
    }
}

/// A running `AVCaptureSession` of one device into one data output whose
/// sample buffers go to a [`SampleSink`]; stopped on drop.
pub struct Capture {
    session: Id,
    output: Id,
    queue: objc::Queue,
    _sink: SampleSink,
}

impl Capture {
    /// Capture `device` into `output` (an AVCapture*DataOutput, set up by
    /// the caller), calling `sink` on a queue of its own.
    pub unsafe fn start(device: *mut c_void, output: Id, sink: SampleSink, preset: Cf) -> Result<Capture> {
        unsafe {
            let mut err: *mut c_void = core::ptr::null_mut();
            let input: *mut c_void =
                msg2(objc_cls(c"AVCaptureDeviceInput"), objc_sel(c"deviceInputWithDevice:error:"), device, core::ptr::addr_of_mut!(err));
            if input.is_null() {
                bail!("{}", objc::error_text(err));
            }
            let session = objc::new_obj(objc_cls(c"AVCaptureSession")).ok_or_else(|| anyhow!("create a capture session"))?;
            let s = session.ptr();
            let _: () = msg0(s, objc_sel(c"beginConfiguration"));
            if !preset.is_null() {
                let ok: i8 = msg1(s, objc_sel(c"canSetSessionPreset:"), preset);
                if ok != 0 {
                    let _: () = msg1(s, objc_sel(c"setSessionPreset:"), preset);
                }
            }
            let ok_in: i8 = msg1(s, objc_sel(c"canAddInput:"), input);
            let ok_out: i8 = msg1(s, objc_sel(c"canAddOutput:"), output.ptr());
            if ok_in == 0 || ok_out == 0 {
                let _: () = msg0(s, objc_sel(c"commitConfiguration"));
                bail!("the device cannot be captured");
            }
            let _: () = msg1(s, objc_sel(c"addInput:"), input);
            let _: () = msg1(s, objc_sel(c"addOutput:"), output.ptr());
            let queue = objc::Queue::serial(c"rustshot.capture");
            let _: () = msg2(output.ptr(), objc_sel(c"setSampleBufferDelegate:queue:"), sink.ptr(), queue.ptr());
            let _: () = msg0(s, objc_sel(c"commitConfiguration"));
            let _: () = msg0(s, objc_sel(c"startRunning"));
            let running: i8 = msg0(s, objc_sel(c"isRunning"));
            let cap = Capture { session, output, queue, _sink: sink };
            if running == 0 {
                bail!("the device did not start");
            }
            Ok(cap)
        }
    }

    /// The session's clock (its sample times are in it); null if none.
    pub fn clock(&self) -> Cf {
        unsafe {
            let s = self.session.ptr();
            let sel = if objc::has_method(objc_cls(c"AVCaptureSession"), c"synchronizationClock") {
                objc_sel(c"synchronizationClock")
            } else {
                objc_sel(c"masterClock")
            };
            msg0(s, sel)
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _pool = objc::Pool::new();
        unsafe {
            let _: () = msg0(self.session.ptr(), objc_sel(c"stopRunning"));
            let none: *mut c_void = core::ptr::null_mut();
            let _: () = msg2(self.output.ptr(), objc_sel(c"setSampleBufferDelegate:queue:"), none, none);
        }
        // A callback in progress finishes before the sink goes.
        self.queue.drain();
    }
}
