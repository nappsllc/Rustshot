//! PipeWire video streams through `libpipewire-0.3.so.0`, loaded with
//! `dlopen` (nothing is linked: without the library the portal paths say
//! so and the rest of Rustshot is unaffected).
//!
//! One [`Stream`] per screen cast or camera: a `pw_thread_loop`, a context
//! connected to the portal's socket (`pw_context_connect_fd`), and an input
//! stream offering raw video formats ([`pod::enum_format`]: BGRx/RGBx for
//! screens, YUY2/NV12 for cameras; shared memory only, no DMA-BUF). Once a
//! format is negotiated the stream asks for mappable buffers and a header
//! meta (timestamps); each buffer is handed to the stream's `on_frame` on
//! PipeWire's thread, which converts it and keeps only the newest picture
//! ([`ScreenSource`] for recordings, [`PwCam`] for the camera bubble).
//! Nothing queues up: a slow reader just sees fewer pictures.

use super::pipewire_pod as pod;
use super::dl::Lib;
use crate::capture::IRect;
use crate::rec::{Frame, FrameSource};
use anyhow::{anyhow, bail, Result};
use core::ffi::{c_char, c_int, c_void};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[repr(C)]
struct SpaHook {
    _p: [*mut c_void; 6],
}

type Destroy = unsafe extern "C" fn(*mut c_void);
type StateChanged = unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char);
type ParamChanged = unsafe extern "C" fn(*mut c_void, u32, *const u8);
type Process = unsafe extern "C" fn(*mut c_void);

/// `struct pw_stream_events`, version 0 (up to `drained`).
#[repr(C)]
struct StreamEvents {
    version: u32,
    destroy: Option<Destroy>,
    state_changed: Option<StateChanged>,
    control_info: Option<unsafe extern "C" fn()>,
    io_changed: Option<unsafe extern "C" fn()>,
    param_changed: Option<ParamChanged>,
    add_buffer: Option<unsafe extern "C" fn()>,
    remove_buffer: Option<unsafe extern "C" fn()>,
    process: Option<Process>,
    drained: Option<unsafe extern "C" fn()>,
}

#[repr(C)]
struct PwBuffer {
    buffer: *mut SpaBuffer,
    user_data: *mut c_void,
    size: u64,
    requested: u64,
}

#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut SpaMeta,
    datas: *mut SpaData,
}

#[repr(C)]
struct SpaMeta {
    ty: u32,
    size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct SpaData {
    ty: u32,
    flags: u32,
    fd: i64,
    mapoffset: u32,
    maxsize: u32,
    data: *mut c_void,
    chunk: *mut SpaChunk,
}

#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

#[repr(C)]
struct SpaMetaHeader {
    flags: u32,
    offset: u32,
    pts: i64,
    dts_offset: i64,
    seq: u64,
}

const DIRECTION_INPUT: c_int = 0;
const FLAG_AUTOCONNECT: c_int = 1 << 0;
const FLAG_MAP_BUFFERS: c_int = 1 << 2;
pub const ID_ANY: u32 = 0xFFFF_FFFF;
const STATE_ERROR: c_int = -1;
const STATE_UNCONNECTED: c_int = 0;
const STATE_STREAMING: c_int = 3;
const CHUNK_CORRUPTED: i32 = 1;
const HEADER_CORRUPTED: u32 = 1 << 1;

/// The functions taken from libpipewire.
struct Pw {
    _lib: Lib,
    thread_loop_new: unsafe extern "C" fn(*const c_char, *const c_void) -> *mut c_void,
    thread_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    thread_loop_start: unsafe extern "C" fn(*mut c_void) -> c_int,
    thread_loop_stop: unsafe extern "C" fn(*mut c_void),
    thread_loop_lock: unsafe extern "C" fn(*mut c_void),
    thread_loop_unlock: unsafe extern "C" fn(*mut c_void),
    thread_loop_destroy: unsafe extern "C" fn(*mut c_void),
    context_new: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    context_destroy: unsafe extern "C" fn(*mut c_void),
    context_connect_fd: unsafe extern "C" fn(*mut c_void, c_int, *mut c_void, usize) -> *mut c_void,
    core_disconnect: unsafe extern "C" fn(*mut c_void) -> c_int,
    properties_new_string: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    stream_new: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> *mut c_void,
    stream_add_listener: unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const StreamEvents, *mut c_void),
    stream_connect: unsafe extern "C" fn(*mut c_void, c_int, u32, c_int, *const *const u8, u32) -> c_int,
    stream_disconnect: unsafe extern "C" fn(*mut c_void) -> c_int,
    stream_destroy: unsafe extern "C" fn(*mut c_void),
    stream_dequeue_buffer: unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer,
    stream_queue_buffer: unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> c_int,
    stream_update_params: unsafe extern "C" fn(*mut c_void, *const *const u8, u32) -> c_int,
}

// SAFETY: plain function pointers into a library that stays loaded.
unsafe impl Send for Pw {}
unsafe impl Sync for Pw {}

/// libpipewire, loaded and initialised once (never unloaded: its threads
/// may still be winding down when a stream is dropped).
fn pw() -> Result<&'static Pw> {
    static PW: OnceLock<std::result::Result<Pw, String>> = OnceLock::new();
    let r = PW.get_or_init(|| load().map_err(|e| format!("{e:#}")));
    r.as_ref().map_err(|e| anyhow!("{e} (install PipeWire: libpipewire-0.3)"))
}

fn load() -> Result<Pw> {
    let lib = Lib::open("libpipewire-0.3.so.0")?;
    // SAFETY: the types are libpipewire-0.3's.
    unsafe {
        let init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) = lib.sym("pw_init")?;
        let pw = Pw {
            thread_loop_new: lib.sym("pw_thread_loop_new")?,
            thread_loop_get_loop: lib.sym("pw_thread_loop_get_loop")?,
            thread_loop_start: lib.sym("pw_thread_loop_start")?,
            thread_loop_stop: lib.sym("pw_thread_loop_stop")?,
            thread_loop_lock: lib.sym("pw_thread_loop_lock")?,
            thread_loop_unlock: lib.sym("pw_thread_loop_unlock")?,
            thread_loop_destroy: lib.sym("pw_thread_loop_destroy")?,
            context_new: lib.sym("pw_context_new")?,
            context_destroy: lib.sym("pw_context_destroy")?,
            context_connect_fd: lib.sym("pw_context_connect_fd")?,
            core_disconnect: lib.sym("pw_core_disconnect")?,
            properties_new_string: lib.sym("pw_properties_new_string")?,
            stream_new: lib.sym("pw_stream_new")?,
            stream_add_listener: lib.sym("pw_stream_add_listener")?,
            stream_connect: lib.sym("pw_stream_connect")?,
            stream_disconnect: lib.sym("pw_stream_disconnect")?,
            stream_destroy: lib.sym("pw_stream_destroy")?,
            stream_dequeue_buffer: lib.sym("pw_stream_dequeue_buffer")?,
            stream_queue_buffer: lib.sym("pw_stream_queue_buffer")?,
            stream_update_params: lib.sym("pw_stream_update_params")?,
            _lib: lib,
        };
        init(core::ptr::null_mut(), core::ptr::null_mut());
        Ok(pw)
    }
}

/// Whether libpipewire can be loaded.
pub fn available() -> bool {
    pw().is_ok()
}

/// One delivered picture, borrowed for the `on_frame` call.
pub struct Raw<'a> {
    pub fmt: pod::VideoFmt,
    /// The planes: (bytes from the picture's start, row stride).
    pub planes: Vec<(&'a [u8], usize)>,
    /// The header meta's presentation time (ns), when there is one.
    pub pts: Option<i64>,
}

type OnFrame = Box<dyn FnMut(&Raw) + Send>;

/// Between PipeWire's thread and the stream's owner.
struct Shared {
    stream: Mutex<usize>,
    fmt: Mutex<Option<pod::VideoFmt>>,
    error: Mutex<Option<String>>,
    streaming: Mutex<bool>,
    changed: Condvar,
    on_frame: Mutex<OnFrame>,
    /// Buffers PipeWire handed over, and those of them with no usable
    /// picture (empty or corrupted): diagnostics for a cast that shows nothing.
    buffers: AtomicU64,
    unusable: AtomicU64,
    /// When the agreed format was taken away (`param_changed` with no
    /// Format: the peer node unlinked, e.g. the portal closed the session)
    /// and none came back since.
    lost: Mutex<Option<Instant>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "C" fn on_state(data: *mut c_void, old: c_int, state: c_int, error: *const c_char) {
    // SAFETY: `data` is the stream's `Shared`, alive while the listener is.
    let sh = unsafe { &*(data as *const Shared) };
    if matches!(state, STATE_ERROR | STATE_UNCONNECTED) {
        eprintln!("rustshot: PipeWire stream state {old} -> {state}");
    }
    match state {
        STATE_ERROR => {
            let msg = if error.is_null() { "the stream failed".to_string() } else { unsafe { std::ffi::CStr::from_ptr(error) }.to_string_lossy().into_owned() };
            *lock(&sh.error) = Some(msg);
        }
        STATE_UNCONNECTED => {
            let had = lock(&sh.fmt).is_some();
            if had {
                *lock(&sh.error) = Some("the stream ended".into());
            }
        }
        _ => {}
    }
    *lock(&sh.streaming) = state == STATE_STREAMING;
    sh.changed.notify_all();
}

unsafe extern "C" fn on_param(data: *mut c_void, id: u32, param: *const u8) {
    if id != pod::PARAM_FORMAT {
        return;
    }
    // SAFETY: as in `on_state`; a pod is its 8-byte header plus `size` bytes.
    let sh = unsafe { &*(data as *const Shared) };
    if param.is_null() {
        // The format is gone: the node we were linked to went away (or a
        // renegotiation, which brings a new one at once).
        let mut lost = lock(&sh.lost);
        if lock(&sh.fmt).is_some() && lost.is_none() {
            eprintln!("rustshot: PipeWire: the stream lost its format");
            *lost = Some(Instant::now());
            drop(lost);
            sh.changed.notify_all();
        }
        return;
    }
    *lock(&sh.lost) = None;
    let p = unsafe {
        let size = (param as *const u32).read_unaligned() as usize;
        core::slice::from_raw_parts(param, 8 + size)
    };
    let Some(f) = pod::parse_format(p) else { return };
    *lock(&sh.fmt) = Some(f);
    sh.changed.notify_all();
    let Ok(pw) = pw() else { return };
    let bufs = pod::aligned(&pod::buffers_param());
    let meta = pod::aligned(&pod::meta_header_param());
    let params = [bufs.as_ptr() as *const u8, meta.as_ptr() as *const u8];
    let s = *lock(&sh.stream) as *mut c_void;
    // SAFETY: on the loop's thread, in its callback; the stream is alive.
    unsafe {
        (pw.stream_update_params)(s, params.as_ptr(), params.len() as u32);
    }
}

unsafe extern "C" fn on_process(data: *mut c_void) {
    // SAFETY: as in `on_state`.
    let sh = unsafe { &*(data as *const Shared) };
    let Ok(pw) = pw() else { return };
    let s = *lock(&sh.stream) as *mut c_void;
    let Some(fmt) = *lock(&sh.fmt) else { return };
    // Take the newest buffer, giving back any older ones.
    let mut b: *mut PwBuffer = core::ptr::null_mut();
    loop {
        // SAFETY: the stream is alive; buffers go back once read.
        let n = unsafe { (pw.stream_dequeue_buffer)(s) };
        if n.is_null() {
            break;
        }
        if !b.is_null() {
            unsafe { (pw.stream_queue_buffer)(s, b) };
        }
        b = n;
    }
    if b.is_null() {
        return;
    }
    sh.buffers.fetch_add(1, Ordering::Relaxed);
    // SAFETY: a dequeued buffer's spa_buffer, metas, datas and chunks are
    // valid until it is queued again; mapped data (MAP_BUFFERS) is `maxsize`.
    unsafe {
        let sb = &*(*b).buffer;
        let mut pts = None;
        let mut corrupted = false;
        for i in 0..sb.n_metas as usize {
            let m = &*sb.metas.add(i);
            if m.ty == pod::META_HEADER && m.size as usize >= size_of::<SpaMetaHeader>() && !m.data.is_null() {
                let h = &*(m.data as *const SpaMetaHeader);
                pts = Some(h.pts);
                corrupted |= h.flags & HEADER_CORRUPTED != 0;
            }
        }
        let mut planes = Vec::new();
        for i in 0..sb.n_datas as usize {
            let d = &*sb.datas.add(i);
            if d.data.is_null() || d.chunk.is_null() {
                break;
            }
            let c = &*d.chunk;
            if c.flags & CHUNK_CORRUPTED != 0 {
                corrupted = true;
            }
            let max = d.maxsize as usize;
            let off = (c.offset as usize).min(max);
            let len = (c.size as usize).min(max - off);
            if len == 0 {
                break; // a cursor-only or empty update: no picture
            }
            planes.push((core::slice::from_raw_parts((d.data as *const u8).add(off), len), c.stride.max(0) as usize));
        }
        if !corrupted && !planes.is_empty() {
            (lock(&sh.on_frame))(&Raw { fmt, planes, pts });
        } else {
            sh.unusable.fetch_add(1, Ordering::Relaxed);
        }
        (pw.stream_queue_buffer)(s, b);
    }
}

static EVENTS: StreamEvents = StreamEvents {
    version: 0,
    destroy: None,
    state_changed: Some(on_state),
    control_info: None,
    io_changed: None,
    param_changed: Some(on_param),
    add_buffer: None,
    remove_buffer: None,
    process: Some(on_process),
    drained: None,
};

/// What a stream asks for.
pub struct Want {
    /// The node to connect to ([`ID_ANY`]: the default, for cameras).
    pub node: u32,
    /// Stream properties (`key=value ...`).
    pub props: &'static str,
    pub formats: &'static [u32],
    pub size: (u32, u32),
    pub fps: u32,
}

/// A connected PipeWire input stream.
pub struct Stream {
    pw: &'static Pw,
    tl: *mut c_void,
    ctx: *mut c_void,
    core: *mut c_void,
    stream: *mut c_void,
    _hook: Box<SpaHook>,
    shared: Arc<Shared>,
}

// SAFETY: the PipeWire objects are only touched under the thread loop's lock.
unsafe impl Send for Stream {}

impl Stream {
    /// Connect to the PipeWire socket `fd` (taken) and stream `want`;
    /// `on_frame` gets every picture on PipeWire's thread.
    pub fn open(fd: OwnedFd, want: &Want, on_frame: OnFrame) -> Result<Stream> {
        let pw = pw()?;
        let shared = Arc::new(Shared {
            stream: Mutex::new(0),
            fmt: Mutex::new(None),
            error: Mutex::new(None),
            streaming: Mutex::new(false),
            changed: Condvar::new(),
            on_frame: Mutex::new(on_frame),
            buffers: AtomicU64::new(0),
            unusable: AtomicU64::new(0),
            lost: Mutex::new(None),
        });
        // SAFETY: libpipewire's documented call order; every object made
        // here is destroyed by `Drop` (or below when a step fails).
        unsafe {
            let tl = (pw.thread_loop_new)(c"rustshot-pw".as_ptr(), core::ptr::null());
            if tl.is_null() {
                bail!("pw_thread_loop_new failed");
            }
            let ctx = (pw.context_new)((pw.thread_loop_get_loop)(tl), core::ptr::null_mut(), 0);
            if ctx.is_null() {
                (pw.thread_loop_destroy)(tl);
                bail!("pw_context_new failed");
            }
            let mut s = Stream {
                pw,
                tl,
                ctx,
                core: core::ptr::null_mut(),
                stream: core::ptr::null_mut(),
                _hook: Box::new(SpaHook { _p: [core::ptr::null_mut(); 6] }),
                shared,
            };
            if (pw.thread_loop_start)(tl) < 0 {
                bail!("pw_thread_loop_start failed");
            }
            (pw.thread_loop_lock)(tl);
            let r = s.connect(fd, want);
            (pw.thread_loop_unlock)(tl);
            r?;
            Ok(s)
        }
    }

    /// Under the loop lock.
    unsafe fn connect(&mut self, fd: OwnedFd, want: &Want) -> Result<()> {
        let pw = self.pw;
        unsafe {
            self.core = (pw.context_connect_fd)(self.ctx, fd.into_raw_fd(), core::ptr::null_mut(), 0);
            if self.core.is_null() {
                bail!("cannot connect to PipeWire: {}", std::io::Error::last_os_error());
            }
            let props = std::ffi::CString::new(want.props).expect("no NUL");
            self.stream = (pw.stream_new)(self.core, c"rustshot".as_ptr(), (pw.properties_new_string)(props.as_ptr()));
            if self.stream.is_null() {
                bail!("pw_stream_new failed");
            }
            *lock(&self.shared.stream) = self.stream as usize;
            (pw.stream_add_listener)(self.stream, &mut *self._hook, &EVENTS, Arc::as_ptr(&self.shared) as *mut c_void);
            let fmt = pod::aligned(&pod::enum_format(want.formats, want.size, want.fps));
            let params = [fmt.as_ptr() as *const u8];
            let r = (pw.stream_connect)(self.stream, DIRECTION_INPUT, want.node, FLAG_AUTOCONNECT | FLAG_MAP_BUFFERS, params.as_ptr(), 1);
            if r < 0 {
                bail!("pw_stream_connect failed ({r})");
            }
        }
        Ok(())
    }

    /// Wait up to `timeout` for the format to be agreed.
    pub fn wait_format(&self, timeout: Duration) -> Result<pod::VideoFmt> {
        let end = Instant::now() + timeout;
        let mut f = lock(&self.shared.fmt);
        loop {
            if let Some(e) = lock(&self.shared.error).clone() {
                bail!("PipeWire: {e}");
            }
            if let Some(v) = *f {
                return Ok(v);
            }
            let now = Instant::now();
            if now >= end {
                bail!("PipeWire: no picture format agreed within {} s", timeout.as_secs());
            }
            f = self.shared.changed.wait_timeout(f, end - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The stream failed or ended.
    pub fn error(&self) -> Option<String> {
        if let Some(e) = lock(&self.shared.error).clone() {
            return Some(e);
        }
        // Unlinked (the format taken away) and not back after a while: the
        // source node is gone (xdg-desktop-portal-wlr leaves the stream
        // paused, not unconnected, when its session closes).
        let lost = *lock(&self.shared.lost);
        (lost.is_some_and(|t| t.elapsed() >= RELINK_WAIT) && !*lock(&self.shared.streaming)).then(|| "the stream ended".into())
    }

    /// (buffers received, of which without a usable picture).
    pub fn buffer_counts(&self) -> (u64, u64) {
        (self.shared.buffers.load(Ordering::Relaxed), self.shared.unusable.load(Ordering::Relaxed))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let pw = self.pw;
        // SAFETY: tearing down what `open` made, the stream and core under
        // the loop lock, then the loop and context once it stopped.
        unsafe {
            (pw.thread_loop_lock)(self.tl);
            if !self.stream.is_null() {
                (pw.stream_disconnect)(self.stream);
                (pw.stream_destroy)(self.stream);
            }
            if !self.core.is_null() {
                (pw.core_disconnect)(self.core);
            }
            (pw.thread_loop_unlock)(self.tl);
            (pw.thread_loop_stop)(self.tl);
            (pw.context_destroy)(self.ctx);
            (pw.thread_loop_destroy)(self.tl);
        }
    }
}

/// The newest picture and how many have come.
struct Slot<T> {
    item: Mutex<(Option<T>, u64)>,
    cv: Condvar,
}

impl<T> Slot<T> {
    fn new() -> Slot<T> {
        Slot { item: Mutex::new((None, 0)), cv: Condvar::new() }
    }

    fn put(&self, v: T) {
        let mut g = lock(&self.item);
        g.0 = Some(v);
        g.1 += 1;
        drop(g);
        self.cv.notify_all();
    }

    /// Put `v` only when nothing has come yet (a stand-in never replaces a
    /// real picture); whether it was put.
    fn put_first(&self, v: T) -> bool {
        let mut g = lock(&self.item);
        if g.1 != 0 {
            return false;
        }
        g.0 = Some(v);
        g.1 = 1;
        drop(g);
        self.cv.notify_all();
        true
    }

    /// Whether any picture has come, waiting until `deadline` for the first.
    fn wait_first(&self, deadline: Instant) -> bool {
        let mut g = lock(&self.item);
        loop {
            if g.1 > 0 {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            g = self.cv.wait_timeout(g, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The picture after number `seen`, waiting until `deadline`.
    fn take_after(&self, seen: u64, deadline: Instant) -> Option<(T, u64)> {
        let mut g = lock(&self.item);
        loop {
            if g.1 != seen
                && let Some(v) = g.0.take()
            {
                return Some((v, g.1));
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            g = self.cv.wait_timeout(g, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

/// The newest screen picture, repeated while nothing new comes: the
/// compositor sends pictures only when the screen changes (a still screen
/// sends nothing new, or nothing at all), and a recording needs one per tick (the
/// DXGI path's WAIT_TIMEOUT case on Windows).
struct Latest {
    slot: Arc<Slot<Vec<u8>>>,
    size: (u32, u32),
    seen: u64,
    last: Option<Frame>,
}

impl Latest {
    /// A new picture when one comes before `deadline`, else the last one
    /// again (`None` only before the first picture).
    fn next(&mut self, deadline: Instant) -> Option<Frame> {
        if let Some((bgra, n)) = self.slot.take_after(self.seen, deadline) {
            self.seen = n;
            let f = Frame { w: self.size.0, h: self.size.1, bgra, ts: Duration::ZERO };
            self.last = Some(f.clone());
            return Some(f);
        }
        self.last.clone()
    }
}

/// A raw picture: its format, bytes and row stride.
type RawCopy = (pod::VideoFmt, Vec<u8>, usize);

/// Where the crop stands: unknown until the format is agreed (meanwhile
/// the newest raw picture is kept: a still screen may never send another),
/// then the rect and the output size.
enum Crop {
    Waiting(Option<RawCopy>),
    Ready(IRect, (u32, u32)),
}

/// Frames of a portal screen cast: the area cropped from the shared
/// monitor's pictures. Holds the cast (its session) while it lives.
pub struct ScreenSource {
    stream: Stream,
    latest: Latest,
    cast: super::portal::Cast,
    /// Why the share stopped under the recording, once it did.
    ended: Option<String>,
    /// Set to close the portal session from the next `next` (tests: the
    /// desktop's "stop sharing" under a running recording).
    #[cfg(test)]
    stop_req: Arc<std::sync::atomic::AtomicBool>,
}

impl ScreenSource {
    /// See `stop_req`.
    #[cfg(test)]
    pub fn stop_handle(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.stop_req.clone()
    }
}

/// How long a stream may be without a format (unlinked) before it counts
/// as ended.
const RELINK_WAIT: Duration = Duration::from_secs(2);
/// How long the compositor may take to agree a format.
const FORMAT_WAIT: Duration = Duration::from_secs(10);
/// How long the compositor may take to send its first picture.
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(5);
/// How long to wait for it before seeding the source with a screenshot.
const SEED_AFTER: Duration = Duration::from_millis(1500);
/// How long the seed screenshot may take (the Screenshot portal); without
/// it the source waits for the cast's first picture as before.
const SEED_WAIT: Duration = Duration::from_secs(3);
/// The error when it sends none.
pub const NO_PICTURE: &str = "The screen share sent no picture";

impl ScreenSource {
    /// Stream `cast` and crop what `crop` says from each picture (given the
    /// agreed picture size: the crop rect and whether the area fit). Fails
    /// unless a first picture comes within [`FIRST_FRAME_WAIT`].
    pub fn open(mut cast: super::portal::Cast, crop: impl FnOnce((u32, u32)) -> IRect) -> Result<ScreenSource> {
        let fd = cast.fd.take().ok_or_else(|| anyhow!("the screen cast has no PipeWire socket"))?;
        let state = Arc::new(Mutex::new(Crop::Waiting(None)));
        let slot = Arc::new(Slot::new());
        let (r2, s2) = (state.clone(), slot.clone());
        let mut buf: Vec<Vec<u8>> = Vec::new();
        let mut last_pts = None;
        let on_frame: OnFrame = Box::new(move |raw: &Raw| {
            // A picture older than one already delivered is dropped.
            if let (Some(p), Some(l)) = (raw.pts, last_pts)
                && p < l
            {
                return;
            }
            last_pts = raw.pts.or(last_pts);
            let Some(&(data, stride)) = raw.planes.first() else { return };
            let mut st = lock(&r2);
            let (c, (ow, oh)) = match &mut *st {
                Crop::Ready(c, o) => (*c, *o),
                Crop::Waiting(p) => {
                    *p = Some((raw.fmt, data.to_vec(), stride));
                    return;
                }
            };
            drop(st);
            let mut out = buf.pop().unwrap_or_default();
            if pod::convert_screen(raw.fmt.format, data, stride, raw.fmt.w, raw.fmt.h, c, ow, oh, &mut out) {
                s2.put(out);
            } else {
                buf.push(out);
            }
        });
        let want = Want {
            node: cast.info.stream.node,
            props: "media.type=Video media.category=Capture media.role=Screen",
            formats: &[pod::VF_BGRX, pod::VF_RGBX, pod::VF_BGRA, pod::VF_RGBA],
            size: (1920, 1080),
            fps: 60,
        };
        let stream = Stream::open(fd, &want, on_frame)?;
        let fmt = stream.wait_format(FORMAT_WAIT)?;
        let c = crop((fmt.w, fmt.h));
        let size = (c.2.max(1), c.3.max(1));
        {
            // A picture that came before the crop was known is not lost.
            let mut st = lock(&state);
            if let Crop::Waiting(Some((f, data, stride))) = core::mem::replace(&mut *st, Crop::Ready(c, size)) {
                let mut out = Vec::new();
                if pod::convert_screen(f.format, &data, stride, f.w, f.h, c, size.0, size.1, &mut out) {
                    slot.put(out);
                }
            }
        }
        eprintln!("rustshot: PipeWire {}x{} (format {}), recording {:?}", fmt.w, fmt.h, fmt.format, c);
        let end = Instant::now() + FIRST_FRAME_WAIT;
        // Some compositors (xdg-desktop-portal-wlr) send a picture only
        // once something on the screen changes: a still screen sends none.
        // Like the Windows DXGI path seeds a still display with a GDI grab,
        // the first picture is then a screenshot (Screenshot portal), which
        // the source repeats until the cast sends one.
        if !slot.wait_first(Instant::now() + SEED_AFTER) {
            let s = &cast.info.stream;
            let mon = s.pos.zip(s.size);
            match super::portal::grab_screen(SEED_WAIT) {
                Ok(img) => match seed_picture(&img, mon, (fmt.w, fmt.h), c) {
                    Some(px) => {
                        if slot.put_first(px) {
                            eprintln!("rustshot: the screen cast sent nothing yet; starting from a screenshot");
                        }
                    }
                    None => eprintln!("rustshot: the screenshot does not fit the shared screen"),
                },
                Err(e) => eprintln!("rustshot: no screenshot to start the screen cast from: {e:#}"),
            }
        }
        if !slot.wait_first(end) {
            let (n, bad) = stream.buffer_counts();
            let why = stream.error().map(|e| format!("; {e}")).unwrap_or_default();
            bail!("{NO_PICTURE} within {} s ({n} buffers from PipeWire, {bad} without a picture{why})", FIRST_FRAME_WAIT.as_secs());
        }
        Ok(ScreenSource {
            stream,
            latest: Latest { slot, size, seen: 0, last: None },
            cast,
            ended: None,
            #[cfg(test)]
            stop_req: Arc::default(),
        })
    }
}

/// The part `c` of the shared screen (whose pictures are `frame` px; at
/// `mon` = (position, size) in the compositor's layout, when known) cut
/// from a screenshot `img` of the whole desktop, as BGRA of `c`'s size.
fn seed_picture(img: &crate::pixbuf::PixBuf, mon: Option<((i32, i32), (i32, i32))>, frame: (u32, u32), c: IRect) -> Option<Vec<u8>> {
    let (iw, ih) = img.dimensions();
    let region = match mon {
        _ if (iw, ih) == frame => (0, 0, iw, ih),
        Some(((x, y), (w, h))) if x >= 0 && y >= 0 && w > 0 && h > 0 && (x + w) as u32 <= iw && (y + h) as u32 <= ih => {
            (x as u32, y as u32, w as u32, h as u32)
        }
        _ => (0, 0, iw, ih),
    };
    let sub = img.crop(region.0, region.1, region.2, region.3);
    let mut px = super::portal::crop_screen(&sub, frame, c)?.as_raw().clone();
    for p in px.as_chunks_mut::<4>().0 {
        p.swap(0, 2); // RGBA -> BGRA
    }
    Some(px)
}

impl FrameSource for ScreenSource {
    /// The newest picture (the last one again on a still screen), unless
    /// the share stopped: the stream failed or ended, or the portal closed
    /// the session (looked at on every call), which ends the recording
    /// ([`FrameSource::ended`]).
    fn next(&mut self, deadline: Instant) -> Option<Frame> {
        #[cfg(test)]
        if self.stop_req.swap(false, Ordering::SeqCst) {
            self.cast.close_now();
        }
        if self.ended.is_none() {
            if let Some(e) = self.stream.error() {
                eprintln!("rustshot: screen cast: {e}");
                self.ended = Some(format!("{} (PipeWire: {e})", super::portal::SHARING_STOPPED));
            } else if self.cast.closed() {
                self.ended = Some(super::portal::SHARING_STOPPED.into());
            }
        }
        if self.ended.is_some() {
            return None;
        }
        self.latest.next(deadline)
    }

    fn size(&self) -> (u32, u32) {
        self.latest.size
    }

    fn ended(&self) -> Option<String> {
        self.ended.clone()
    }
}

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

/// One camera buffer (`planes` of a `fmt` picture) as BGRA into `f`;
/// false when it holds no whole picture (too short, another format): the
/// frame is skipped rather than shown black.
fn cam_picture(fmt: pod::VideoFmt, planes: &[(&[u8], usize)], f: &mut crate::cam::CamFrame) -> bool {
    let (w, h) = (fmt.w, fmt.h);
    let Some(&(y, stride)) = planes.first() else { return false };
    let cs = crate::cam::Matrix::Bt601;
    let ok = match fmt.format {
        pod::VF_YUY2 => {
            let stride = if stride == 0 { w as usize * 2 } else { stride };
            (f.w, f.h) = (w & !1, h);
            crate::cam::yuy2_to_bgra(y, stride, w, h, cs, &mut f.bgra)
        }
        pod::VF_NV12 => {
            let stride = if stride == 0 { w as usize } else { stride };
            let (yp, uv) = match planes.get(1) {
                Some(&(uv, _)) => (y, uv),
                None if y.len() > stride * h as usize => y.split_at(stride * h as usize),
                None => return false,
            };
            (f.w, f.h) = (w & !1, h & !1);
            crate::cam::nv12_to_bgra(yp, uv, stride, w, h, cs, &mut f.bgra)
        }
        _ => None,
    };
    ok.is_some() && f.w > 0 && f.bgra.len() == (f.w * f.h * 4) as usize
}

/// A camera through the camera portal's PipeWire remote.
pub struct PwCam {
    stream: Stream,
    slot: Arc<Slot<crate::cam::CamFrame>>,
    size: (u32, u32),
    seen: u64,
    /// The bus connection the access was granted on.
    _conn: crate::dbus::Conn,
}

impl PwCam {
    /// Ask the camera portal for the camera (a dialog the first time; `cancel`
    /// calls it off) and stream the default one at about 1280×720.
    pub fn open(cancel: Option<&super::portal::CancelFlag>) -> Result<PwCam> {
        let mut conn = crate::dbus::Conn::session()?;
        let fd = super::portal::access_camera(&mut conn, cancel).map_err(|e| {
            if e.is::<super::portal::Cancelled>() { anyhow!("camera access was not allowed") } else { e }
        })?;
        let slot = Arc::new(Slot::new());
        let s2 = slot.clone();
        let mut spare = crate::cam::CamFrame::default();
        let on_frame: OnFrame = Box::new(move |raw: &Raw| {
            let mut f = std::mem::take(&mut spare);
            if cam_picture(raw.fmt, &raw.planes, &mut f) {
                s2.put(f);
            } else {
                spare = f; // a short or odd buffer: skipped, not shown black
            }
        });
        let want = Want {
            node: ID_ANY,
            props: "media.type=Video media.category=Capture media.role=Camera",
            formats: &[pod::VF_YUY2, pod::VF_NV12],
            size: crate::cam::PREF,
            fps: 30,
        };
        let stream = Stream::open(fd, &want, on_frame)?;
        let fmt = stream.wait_format(FORMAT_WAIT)?;
        if !matches!(fmt.format, pod::VF_YUY2 | pod::VF_NV12) {
            bail!("{}", crate::cam::NO_FORMAT);
        }
        eprintln!("rustshot: camera through the portal, {}x{} (format {})", fmt.w, fmt.h, fmt.format);
        Ok(PwCam { stream, slot, size: (fmt.w, fmt.h), seen: 0, _conn: conn })
    }
}

impl crate::cam::CamSource for PwCam {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn read(&mut self, into: &mut crate::cam::CamFrame) -> Result<bool> {
        if let Some(e) = self.stream.error() {
            bail!("the camera stopped: {e}");
        }
        match self.slot.take_after(self.seen, Instant::now() + Duration::from_millis(200)) {
            Some((f, n)) => {
                self.seen = n;
                *into = f;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_structs_have_their_c_sizes() {
        assert_eq!(size_of::<SpaHook>(), 48);
        assert_eq!(size_of::<StreamEvents>(), 80);
        assert_eq!(size_of::<SpaData>(), 40);
        assert_eq!(size_of::<SpaChunk>(), 16);
        assert_eq!(size_of::<SpaMetaHeader>(), pod::META_HEADER_SIZE as usize);
        assert_eq!(size_of::<SpaBuffer>(), 24);
    }

    #[test]
    fn the_slot_keeps_only_the_newest() {
        let s = Slot::new();
        s.put(1);
        s.put(2);
        let soon = Instant::now() + Duration::from_millis(10);
        assert_eq!(s.take_after(0, soon), Some((2, 2)));
        assert_eq!(s.take_after(2, soon), None, "nothing new");
    }

    #[test]
    fn the_first_picture_wait_times_out() {
        let s: Slot<u8> = Slot::new();
        let t = Instant::now();
        assert!(!s.wait_first(t + Duration::from_millis(30)));
        assert!(t.elapsed() >= Duration::from_millis(30));
        s.put(7);
        assert!(s.wait_first(Instant::now()), "one has come");
    }

    #[test]
    fn a_stand_in_never_replaces_a_real_picture() {
        let s = Slot::new();
        assert!(s.put_first(1));
        assert_eq!(s.take_after(0, Instant::now()), Some((1, 1)));
        let s = Slot::new();
        s.put(2);
        assert!(!s.put_first(1));
        assert_eq!(s.take_after(0, Instant::now()), Some((2, 1)));
    }

    #[test]
    fn the_seed_is_the_area_of_the_shared_screen() {
        use crate::pixbuf::PixBuf;
        // A 4x2 desktop of two 2x2 monitors; pixel (x, y) = [x, y, 7, 255].
        let mut d = Vec::new();
        for y in 0..2u8 {
            for x in 0..4u8 {
                d.extend([x, y, 7, 255]);
            }
        }
        let img = PixBuf::from_raw(4, 2, d).unwrap();
        // The right monitor, its pictures 2x2; the area is its (1, 0) 1x2.
        let px = seed_picture(&img, Some(((2, 0), (2, 2))), (2, 2), (1, 0, 1, 2)).unwrap();
        assert_eq!(px, [7, 0, 3, 255, 7, 1, 3, 255], "BGRA of x 3, y 0..2");
        // The same size as the pictures: cut as is.
        let px = seed_picture(&img, None, (4, 2), (0, 1, 1, 1)).unwrap();
        assert_eq!(px, [7, 1, 0, 255]);
    }

    /// A camera buffer shorter than its picture is skipped (no black frame).
    #[test]
    fn short_camera_buffers_are_skipped() {
        let fmt = pod::VideoFmt { format: pod::VF_YUY2, w: 4, h: 2 };
        let mut f = crate::cam::CamFrame::default();
        let whole = [16u8, 128, 16, 128].repeat(4);
        assert!(cam_picture(fmt, &[(&whole, 8)], &mut f));
        assert_eq!((f.w, f.h, f.bgra.len()), (4, 2, 32));
        assert!(!cam_picture(fmt, &[(&whole[..10], 8)], &mut f), "half a picture");
        let nv = pod::VideoFmt { format: pod::VF_NV12, w: 4, h: 2 };
        assert!(!cam_picture(nv, &[(&[16u8; 8], 4)], &mut f), "no chroma");
        assert!(cam_picture(nv, &[(&[16u8; 8], 4), (&[128u8; 4], 4)], &mut f));
        assert!(!cam_picture(nv, &[(&[16u8; 8], 4), (&[128u8; 2], 4)], &mut f), "short chroma");
        assert!(!cam_picture(pod::VideoFmt { format: pod::VF_BGRX, w: 4, h: 2 }, &[(&whole, 8)], &mut f));
        assert!(!cam_picture(fmt, &[], &mut f));
    }

    /// A still screen: one picture, then nothing; every tick still gets it.
    #[test]
    fn a_still_screen_repeats_its_last_picture() {
        let slot = Arc::new(Slot::new());
        let mut l = Latest { slot: slot.clone(), size: (1, 1), seen: 0, last: None };
        let soon = || Instant::now() + Duration::from_millis(5);
        assert!(l.next(soon()).is_none(), "nothing before the first picture");
        slot.put(vec![1, 2, 3, 255]);
        for _ in 0..5 {
            let f = l.next(soon()).expect("the picture, again");
            assert_eq!(f.bgra, [1, 2, 3, 255]);
            assert_eq!((f.w, f.h), (1, 1));
        }
        slot.put(vec![9, 9, 9, 255]);
        assert_eq!(l.next(soon()).unwrap().bgra, [9, 9, 9, 255], "a new picture wins");
        assert_eq!(l.next(soon()).unwrap().bgra, [9, 9, 9, 255]);
    }
}
