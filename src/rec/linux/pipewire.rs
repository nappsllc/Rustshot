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
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "C" fn on_state(data: *mut c_void, _old: c_int, state: c_int, error: *const c_char) {
    // SAFETY: `data` is the stream's `Shared`, alive while the listener is.
    let sh = unsafe { &*(data as *const Shared) };
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
    if id != pod::PARAM_FORMAT || param.is_null() {
        return;
    }
    // SAFETY: as in `on_state`; a pod is its 8-byte header plus `size` bytes.
    let sh = unsafe { &*(data as *const Shared) };
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
        lock(&self.shared.error).clone()
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

/// Frames of a portal screen cast: the area cropped from the shared
/// monitor's pictures. Holds the cast (its session) while it lives.
pub struct ScreenSource {
    stream: Stream,
    slot: Arc<Slot<Vec<u8>>>,
    size: (u32, u32),
    seen: u64,
    last: Option<Frame>,
    _cast: super::portal::Cast,
}

/// How long the compositor may take to agree a format.
const FORMAT_WAIT: Duration = Duration::from_secs(10);

impl ScreenSource {
    /// Stream `cast` and crop what `crop` says from each picture (given the
    /// agreed picture size: the crop rect and whether the area fit).
    pub fn open(mut cast: super::portal::Cast, crop: impl FnOnce((u32, u32)) -> IRect) -> Result<ScreenSource> {
        let fd = cast.fd.take().ok_or_else(|| anyhow!("the screen cast has no PipeWire socket"))?;
        let rect = Arc::new(Mutex::new(None::<(IRect, (u32, u32))>));
        let slot = Arc::new(Slot::new());
        let (r2, s2) = (rect.clone(), slot.clone());
        let mut buf: Vec<Vec<u8>> = Vec::new();
        let mut last_pts = None;
        let on_frame: OnFrame = Box::new(move |raw: &Raw| {
            let Some((c, (ow, oh))) = *lock(&r2) else { return };
            // A picture older than one already delivered is dropped.
            if let (Some(p), Some(l)) = (raw.pts, last_pts)
                && p < l
            {
                return;
            }
            last_pts = raw.pts.or(last_pts);
            let Some(&(data, stride)) = raw.planes.first() else { return };
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
        *lock(&rect) = Some((c, size));
        eprintln!("rustshot: PipeWire {}x{} (format {}), recording {:?}", fmt.w, fmt.h, fmt.format, c);
        Ok(ScreenSource { stream, slot, size, seen: 0, last: None, _cast: cast })
    }
}

impl FrameSource for ScreenSource {
    fn next(&mut self, deadline: Instant) -> Option<Frame> {
        if let Some((bgra, n)) = self.slot.take_after(self.seen, deadline) {
            self.seen = n;
            let f = Frame { w: self.size.0, h: self.size.1, bgra, ts: Duration::ZERO };
            self.last = Some(f.clone());
            return Some(f);
        }
        if let Some(e) = self.stream.error()
            && self.last.is_none()
        {
            eprintln!("rustshot: screen cast: {e}");
        }
        // The compositor sends pictures only when the screen changes.
        self.last.clone()
    }

    fn size(&self) -> (u32, u32) {
        self.size
    }
}

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

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
    /// Ask the camera portal for the camera (a dialog the first time) and
    /// stream the default one at about 1280×720.
    pub fn open() -> Result<PwCam> {
        let mut conn = crate::dbus::Conn::session()?;
        let fd = super::portal::access_camera(&mut conn).map_err(|e| {
            if e.is::<super::portal::Cancelled>() { anyhow!("camera access was not allowed") } else { e }
        })?;
        let slot = Arc::new(Slot::new());
        let s2 = slot.clone();
        let mut spare = crate::cam::CamFrame::default();
        let on_frame: OnFrame = Box::new(move |raw: &Raw| {
            let (w, h) = (raw.fmt.w, raw.fmt.h);
            let Some(&(y, stride)) = raw.planes.first() else { return };
            let cs = crate::cam::Matrix::Bt601;
            let mut f = std::mem::take(&mut spare);
            match raw.fmt.format {
                pod::VF_YUY2 => {
                    let stride = if stride == 0 { w as usize * 2 } else { stride };
                    crate::cam::yuy2_to_bgra(y, stride, w, h, cs, &mut f.bgra);
                    (f.w, f.h) = (w & !1, h);
                }
                pod::VF_NV12 => {
                    let stride = if stride == 0 { w as usize } else { stride };
                    let (yp, uv) = match raw.planes.get(1) {
                        Some(&(uv, _)) => (y, uv),
                        None if y.len() > stride * h as usize => y.split_at(stride * h as usize),
                        None => return,
                    };
                    crate::cam::nv12_to_bgra(yp, uv, stride, w, h, cs, &mut f.bgra);
                    (f.w, f.h) = (w & !1, h & !1);
                }
                _ => return,
            }
            if f.bgra.len() == (f.w * f.h * 4) as usize && f.w > 0 {
                s2.put(f);
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
}
