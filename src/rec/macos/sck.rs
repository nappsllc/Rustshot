//! ScreenCaptureKit: one `SCStream` of the display the recorded area is
//! on, cropped to the area (`sourceRect`, points) and scaled to the frame
//! size (pixels), BGRA, at most the recording's frame rate, with the
//! cursor. Our own windows are filtered out (the whole application except
//! the camera bubble; see [`super::plan_filter`]). On macOS 13+ the same
//! stream carries the system sound (48 kHz stereo, our own sound left
//! out), read through [`FeedSource`].
//!
//! The stream sends a frame when the screen changes; [`SckSource::next`]
//! hands out the newest one (again, while nothing changes). When the
//! stream stops under us (the display went away, permission revoked) the
//! recording ends with the reason ([`FrameSource::ended`]).

use super::media::{self, FeedSource, Media, PcmFeed, SampleSink};
use super::{CMTime, FilterPlan, RectF};
use crate::capture::{CGPoint, CGRect, CGSize, IRect};
use crate::macos_objc::{self as objc, Id, Queue};
use crate::rec::{Frame, FrameSource};
use crate::wind::{msg0, msg1, msg2, msg3, msg4, objc_cls, objc_sel};
use anyhow::{anyhow, bail, Context, Result};
use core::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long ScreenCaptureKit gets to answer (shareable content, start, stop).
const ANSWER: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Latest {
    px: Vec<u8>,
    spare: Vec<u8>,
    have: bool,
}

struct Shared {
    size: (u32, u32),
    latest: Mutex<Latest>,
    cv: Condvar,
    ended: Mutex<Option<String>>,
    audio: Option<Arc<PcmFeed>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// A sample buffer from the stream (on its queue).
    fn sample(&self, m: &Media, sbuf: *mut c_void, ty: isize) {
        match ty {
            0 => {
                let mut buf = std::mem::take(&mut lock(&self.latest).spare);
                // Idle and blank frames carry no image: the last one stays.
                let got = unsafe { media::copy_pixels(m, sbuf, &mut buf) };
                let mut l = lock(&self.latest);
                if got == Some(self.size) {
                    l.spare = std::mem::replace(&mut l.px, buf);
                    l.have = true;
                    self.cv.notify_all();
                } else {
                    l.spare = buf;
                }
            }
            1 => {
                if let Some(a) = &self.audio {
                    unsafe { a.push(m, sbuf) };
                }
            }
            _ => {}
        }
    }
}

/// The screen stream (and with it the system sound).
pub struct SckSource {
    shared: Arc<Shared>,
    stream: Id,
    video_q: Queue,
    audio_q: Option<Queue>,
    _sink: SampleSink,
}

/// A stream that was opened: the frames, and the system sound when it
/// was asked for (or why it is not recorded).
pub struct Opened {
    pub video: SckSource,
    pub audio: Option<Result<FeedSource, String>>,
}

/// Whether the stream can carry system sound (macOS 13+).
fn sound_capable() -> bool {
    unsafe { objc::has_method(objc_cls(c"SCStreamConfiguration"), c"setCapturesAudio:") }
}

/// The shareable content now (displays, windows, applications), retained.
fn shareable() -> Result<Id> {
    let (block, rx) = objc::completion(2, |a| {
        if a[0] == 0 { Err(unsafe { objc::error_text(a[1] as *mut c_void) }) } else { Ok(Id::retain(a[0] as *mut c_void)) }
    });
    unsafe {
        let _: () = msg1(objc_cls(c"SCShareableContent"), objc_sel(c"getShareableContentWithCompletionHandler:"), block.as_ptr());
    }
    match rx.recv_timeout(ANSWER) {
        Ok(Ok(Some(c))) => Ok(c),
        Ok(Ok(None)) => bail!("no shareable content"),
        // Most often: Screen Recording permission is off.
        Ok(Err(e)) => bail!("{}: {e}", super::SCREEN_DENIED),
        Err(_) => bail!("ScreenCaptureKit did not answer"),
    }
}

unsafe fn pid_of(app: *mut c_void) -> i32 {
    if app.is_null() { -1 } else { unsafe { msg0(app, objc_sel(c"processID")) } }
}

/// The content filter: the display, our windows out but `keep`.
unsafe fn filter(content: *mut c_void, display: *mut c_void, keep: &[u32]) -> Result<Id> {
    unsafe {
        let apps = objc::items(msg0(content, objc_sel(c"applications")));
        let wins = objc::items(msg0(content, objc_sel(c"windows")));
        let app_pids: Vec<i32> = apps.iter().map(|&a| pid_of(a)).collect();
        let win_ids: Vec<(u32, i32)> = wins
            .iter()
            .map(|&w| (msg0::<u32>(w, objc_sel(c"windowID")), pid_of(msg0(w, objc_sel(c"owningApplication")))))
            .collect();
        let me = std::process::id() as i32;
        let f: *mut c_void = msg0(objc_cls(c"SCContentFilter"), objc_sel(c"alloc"));
        let f: *mut c_void = match super::plan_filter(&app_pids, &win_ids, me, keep) {
            FilterPlan::App { app, except } => {
                let except: Vec<_> = except.iter().map(|&i| wins[i]).collect();
                msg3(
                    f,
                    objc_sel(c"initWithDisplay:excludingApplications:exceptingWindows:"),
                    display,
                    objc::array(&[apps[app]]),
                    objc::array(&except),
                )
            }
            FilterPlan::Windows(out) => {
                let out: Vec<_> = out.iter().map(|&i| wins[i]).collect();
                msg2(f, objc_sel(c"initWithDisplay:excludingWindows:"), display, objc::array(&out))
            }
        };
        Id::owned(f).ok_or_else(|| anyhow!("create the content filter"))
    }
}

/// Open a stream of `area` (global points) at `fps`; frames sized for MP4
/// (`mp4`, scaled to fit) or GIF (full size); with the system sound when
/// `sound`; our windows left out but those in `keep` (the camera bubble).
pub fn open(area: IRect, fps: u32, mp4: bool, sound: bool, keep: &[u32]) -> Result<Opened> {
    let m = media::media()?;
    if media::sck().is_none() {
        bail!("recording needs macOS 12.3 or later (ScreenCaptureKit)");
    }
    super::screen_access()?;
    let _pool = objc::Pool::new();
    let content = shareable()?;
    unsafe {
        let displays = objc::items(msg0(content.ptr(), objc_sel(c"displays")));
        let ids: Vec<u32> = displays.iter().map(|&d| msg0(d, objc_sel(c"displayID"))).collect();
        let rects: Vec<RectF> = ids.iter().map(|&id| super::display_rect(id)).collect();
        let i = super::pick_display(area, &rects).ok_or_else(|| anyhow!("the area is on no display"))?;
        let g = super::stream_geom(area, rects[i], super::display_scale(ids[i]), mp4)
            .ok_or_else(|| anyhow!("the area is on no display"))?;
        let flt = filter(content.ptr(), displays[i], keep)?;

        let cfg = objc::new_obj(objc_cls(c"SCStreamConfiguration")).ok_or_else(|| anyhow!("create the stream configuration"))?;
        let c = cfg.ptr();
        let _: () = msg1(c, objc_sel(c"setWidth:"), g.w as usize);
        let _: () = msg1(c, objc_sel(c"setHeight:"), g.h as usize);
        let src = CGRect { origin: CGPoint { x: g.src.x, y: g.src.y }, size: CGSize { width: g.src.w, height: g.src.h } };
        let _: () = msg1(c, objc_sel(c"setSourceRect:"), src);
        let _: () = msg1(c, objc_sel(c"setPixelFormat:"), super::BGRA);
        let _: () = msg1(c, objc_sel(c"setMinimumFrameInterval:"), CMTime::new(1, fps.max(1) as i32));
        let _: () = msg1(c, objc_sel(c"setShowsCursor:"), 1i8);
        let _: () = msg1(c, objc_sel(c"setQueueDepth:"), 5isize);
        let mut audio = None;
        if sound && sound_capable() {
            let _: () = msg1(c, objc_sel(c"setCapturesAudio:"), 1i8);
            let _: () = msg1(c, objc_sel(c"setSampleRate:"), super::RATE as isize);
            let _: () = msg1(c, objc_sel(c"setChannelCount:"), 2isize);
            let _: () = msg1(c, objc_sel(c"setExcludesCurrentProcessAudio:"), 1i8);
            audio = Some(PcmFeed::new());
        }
        let shared = Arc::new(Shared {
            size: (g.w, g.h),
            latest: Mutex::new(Latest::default()),
            cv: Condvar::new(),
            ended: Mutex::new(None),
            audio: audio.clone(),
        });
        let (s1, s2) = (shared.clone(), shared.clone());
        let sink = SampleSink::new(
            move |sbuf, ty| s1.sample(m, sbuf, ty),
            Some(Box::new(move |e: String| {
                *lock(&s2.ended) = Some(format!("the screen recording stopped: {e}"));
                s2.cv.notify_all();
            }) as Box<media::OnStop>),
        )?;
        let st: *mut c_void = msg0(objc_cls(c"SCStream"), objc_sel(c"alloc"));
        let st: *mut c_void = msg3(st, objc_sel(c"initWithFilter:configuration:delegate:"), flt.ptr(), c, sink.ptr());
        let stream = Id::owned(st).ok_or_else(|| anyhow!("create the screen stream"))?;
        let video_q = Queue::serial(c"rustshot.sck.video");
        let mut err: *mut c_void = core::ptr::null_mut();
        let ok: i8 = msg4(
            stream.ptr(),
            objc_sel(c"addStreamOutput:type:sampleHandlerQueue:error:"),
            sink.ptr(),
            0isize, // SCStreamOutputTypeScreen
            video_q.ptr(),
            core::ptr::addr_of_mut!(err),
        );
        if ok == 0 {
            bail!("add the screen output: {}", objc::error_text(err));
        }
        let mut audio_q = None;
        let mut sound_res: Option<Result<FeedSource, String>> = None;
        if sound {
            sound_res = Some(Err("system sound needs macOS 13 or later".into()));
        }
        if let Some(feed) = &audio {
            let q = Queue::serial(c"rustshot.sck.audio");
            let mut err: *mut c_void = core::ptr::null_mut();
            let ok: i8 = msg4(
                stream.ptr(),
                objc_sel(c"addStreamOutput:type:sampleHandlerQueue:error:"),
                sink.ptr(),
                1isize, // SCStreamOutputTypeAudio
                q.ptr(),
                core::ptr::addr_of_mut!(err),
            );
            sound_res = Some(if ok != 0 {
                audio_q = Some(q);
                Ok(FeedSource { feed: feed.clone(), _keep: None })
            } else {
                Err(objc::error_text(err))
            });
        }
        let src = SckSource { shared, stream, video_q, audio_q, _sink: sink };
        let (block, rx) = objc::completion(1, |a| (a[0] != 0).then(|| objc::error_text(a[0] as *mut c_void)));
        let _: () = msg1(src.stream.ptr(), objc_sel(c"startCaptureWithCompletionHandler:"), block.as_ptr());
        match rx.recv_timeout(ANSWER) {
            Ok(None) => {}
            Ok(Some(e)) => return Err(anyhow!("{e}")).context("start the screen stream"),
            Err(_) => bail!("the screen stream did not start"),
        }
        eprintln!(
            "rustshot: recording {}x{} from display {} ({}x{} pt at {}, {})",
            g.w, g.h, ids[i], g.src.w, g.src.h, g.src.x, g.src.y
        );
        Ok(Opened { video: src, audio: sound_res })
    }
}

impl FrameSource for SckSource {
    fn next(&mut self, deadline: Instant) -> Option<Frame> {
        let mut l = lock(&self.shared.latest);
        while !l.have {
            if lock(&self.shared.ended).is_some() {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            l = self.shared.cv.wait_timeout(l, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        let (w, h) = self.shared.size;
        Some(Frame { w, h, bgra: l.px.clone(), ts: Duration::ZERO })
    }

    fn size(&self) -> (u32, u32) {
        self.shared.size
    }

    fn ended(&self) -> Option<String> {
        lock(&self.shared.ended).clone()
    }
}

impl Drop for SckSource {
    fn drop(&mut self) {
        let _pool = objc::Pool::new();
        let (block, rx) = objc::completion(1, |_| ());
        unsafe {
            let _: () = msg1(self.stream.ptr(), objc_sel(c"stopCaptureWithCompletionHandler:"), block.as_ptr());
        }
        let _ = rx.recv_timeout(ANSWER);
        // Callbacks in progress finish before the sink goes.
        self.video_q.drain();
        if let Some(q) = &self.audio_q {
            q.drain();
        }
    }
}
