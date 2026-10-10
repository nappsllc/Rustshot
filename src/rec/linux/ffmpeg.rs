//! MP4 through an `ffmpeg` subprocess (Linux, when ffmpeg is installed).
//!
//! Rustshot writes raw BGRA frames to ffmpeg's stdin at a constant frame
//! rate (frames are duplicated or dropped to keep it, see [`Cfr`]); ffmpeg
//! captures the sound itself (`-f pulse` inputs: the system monitor and the
//! microphone, mixed with `amix`) and writes H.264 (libx264, else
//! libopenh264, else h264_vaapi, whichever `ffmpeg -encoders` lists) + AAC
//! to the `.part` file, `-movflags +faststart`.
//!
//! Since ffmpeg hears the sound in real time, a pause cannot just skip
//! time the way the session clock does. With sound, every stretch between
//! pauses is its own ffmpeg run (a segment, started at its first frame);
//! [`VideoEncoder::finish`] joins them with the concat demuxer (`-c copy`).
//! Without sound there is one run and pausing needs nothing.
//!
//! Sound and picture are kept in step by the wall clock: with sound the
//! frames are stamped when ffmpeg reads them (`-use_wallclock_as_timestamps`)
//! and the pulse inputs stamp their samples the same way, `-copyts` keeps
//! both, and the output starts at the first frame (`-avoid_negative_ts
//! make_zero`). ffmpeg opens its inputs in order and waits on stdin for the
//! first frame before it opens the pulse ones, so a stretch has no sound
//! for the moment that takes (well under a second on a desktop; a
//! suspended sink can take longer to wake) but never sound from before it.
//!
//! Nothing goes through a shell: every argument is its own argv entry.
//! Frames go to ffmpeg on a writer thread through a bounded channel.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::rec::{bitrate, mp4_fit, Frame, Quality, VideoEncoder};
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Absolute path of the ffmpeg to use instead of the one on `PATH`.
pub const ENV_OVERRIDE: &str = "RUSTSHOT_FFMPEG";

/// The VA-API render node tried for `h264_vaapi`.
pub const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

/// Frames waiting for the writer thread (bounded: a slow ffmpeg blocks the
/// encode thread, and the session's queue drops frames before it).
const PIPE_FRAMES: usize = 2;

/// How long [`FfmpegEncoder::finish`] waits for ffmpeg to finish the file.
const FINISH_TIMEOUT: Duration = Duration::from_secs(60);
/// With live sound inputs ffmpeg stops at the end of the video
/// (`-shortest`); if it is still running this long after, it gets SIGINT
/// (which makes it finish the file).
const INT_AFTER: Duration = Duration::from_secs(5);

/// The stderr kept for error messages (the last bytes).
const TAIL_BYTES: usize = 4096;

/// The ffmpeg to run: [`ENV_OVERRIDE`] when it names an absolute,
/// executable file, else the first `ffmpeg` in an absolute `PATH` entry
/// (relative and empty entries, which mean the current directory, are
/// skipped). Always an absolute path.
pub fn find() -> Option<PathBuf> {
    find_tool("ffmpeg", std::env::var_os(ENV_OVERRIDE))
}

/// `ffprobe`, next to the ffmpeg in use or on `PATH` (tests).
pub fn find_ffprobe() -> Option<PathBuf> {
    let beside = find().and_then(|f| Some(f.parent()?.join("ffprobe"))).filter(|p| is_exec(p));
    beside.or_else(|| find_tool("ffprobe", None))
}

fn find_tool(name: &str, over: Option<OsString>) -> Option<PathBuf> {
    find_with(name, over.as_deref(), std::env::var_os("PATH").as_deref(), is_exec)
}

/// [`find`] with the override, `PATH` and the executable check given.
/// An override that is set but not an absolute executable is reported and
/// `PATH` is searched instead.
pub fn find_with(name: &str, over: Option<&OsStr>, path: Option<&OsStr>, ok: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Some(o) = over.filter(|o| !o.is_empty()) {
        let p = Path::new(o);
        if p.is_absolute() && ok(p) {
            return Some(p.to_path_buf());
        }
        eprintln!("rustshot: {ENV_OVERRIDE}={} is not an absolute path to an executable; using PATH", p.display());
    }
    std::env::split_paths(path?)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| ok(p))
}

fn is_exec(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        m.is_file()
    }
}

// ---------------------------------------------------------------------------
// What this ffmpeg can do
// ---------------------------------------------------------------------------

/// The H.264 encoders used, in order of preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    X264,
    OpenH264,
    Vaapi,
}

impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Codec::X264 => "libx264",
            Codec::OpenH264 => "libopenh264",
            Codec::Vaapi => "h264_vaapi",
        }
    }
}

/// What `ffmpeg -encoders` / `-devices` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caps {
    pub h264: Option<Codec>,
    pub aac: bool,
    /// The PulseAudio input device (`-f pulse`) is built in.
    pub pulse: bool,
}

/// Names in a listing of `ffmpeg -hide_banner -encoders` or `-devices`
/// whose flag column (`flags` characters wide, after one leading space)
/// starts with `kind` ('V' video encoder, 'A' audio, 'D' demuxing device).
/// The legend above the `---` line is skipped.
fn listed(out: &str, flags: usize, kind: char) -> Vec<String> {
    let mut names = Vec::new();
    let mut body = !out.lines().any(|l| l.trim_start().starts_with("---"));
    for line in out.lines() {
        if line.trim_start().starts_with("---") {
            body = true;
            continue;
        }
        if !body {
            continue;
        }
        let Some(l) = line.strip_prefix(' ') else { continue };
        let Some(f) = l.get(..flags) else { continue };
        if !f.starts_with(kind) {
            continue;
        }
        if let Some(name) = l[flags..].split_whitespace().next()
            && name != "="
        {
            names.push(name.to_string());
        }
    }
    names
}

/// The video encoders in `ffmpeg -hide_banner -encoders` output.
pub fn video_encoders(out: &str) -> Vec<String> {
    listed(out, 6, 'V')
}

/// The H.264 encoder to use from `ffmpeg -hide_banner -encoders` output:
/// libx264, else libopenh264, else h264_vaapi (when `vaapi` says the
/// render node exists).
pub fn pick_h264(encoders: &str, vaapi: bool) -> Option<Codec> {
    let v = video_encoders(encoders);
    let has = |n: &str| v.iter().any(|e| e == n);
    [Codec::X264, Codec::OpenH264, Codec::Vaapi]
        .into_iter()
        .find(|c| has(c.name()) && (*c != Codec::Vaapi || vaapi))
}

/// Whether `ffmpeg -hide_banner -encoders` lists the native AAC encoder.
pub fn has_aac(encoders: &str) -> bool {
    listed(encoders, 6, 'A').iter().any(|e| e == "aac")
}

/// Whether `ffmpeg -hide_banner -devices` lists `pulse` as an input.
pub fn has_pulse(devices: &str) -> bool {
    listed(devices, 2, 'D').iter().any(|d| d == "pulse")
}

fn run_listing(exe: &Path, what: &str) -> String {
    match Command::new(exe).args(["-hide_banner", what]).stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(e) => {
            eprintln!("rustshot: cannot run {} {what}: {e}", exe.display());
            String::new()
        }
    }
}

/// Ask `exe` what it can do (runs it twice; cached per path).
pub fn caps(exe: &Path) -> Caps {
    static CACHE: OnceLock<Mutex<Vec<(PathBuf, Caps)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, c)) = cache.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(p, _)| p == exe) {
        return c.clone();
    }
    let enc = run_listing(exe, "-encoders");
    let dev = run_listing(exe, "-devices");
    let c = Caps { h264: pick_h264(&enc, Path::new(VAAPI_DEVICE).exists()), aac: has_aac(&enc), pulse: has_pulse(&dev) };
    cache.lock().unwrap_or_else(|e| e.into_inner()).push((exe.to_path_buf(), c.clone()));
    c
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// One ffmpeg run.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Input frame size.
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub quality: Quality,
    pub codec: Codec,
    /// PulseAudio source of the system sound (a sink's monitor).
    pub system: Option<String>,
    /// PulseAudio source of the microphone.
    pub mic: Option<String>,
    pub out: PathBuf,
}

/// `-crf` (libx264) / `-qp` (VA-API) for a quality.
pub fn crf(q: Quality) -> u32 {
    match q {
        Quality::Low => 28,
        Quality::Medium => 23,
        Quality::High => 18,
    }
}

impl Plan {
    /// The sound inputs, in input order (input 0 is the video).
    pub fn audio_inputs(&self) -> Vec<&str> {
        [self.system.as_deref(), self.mic.as_deref()].into_iter().flatten().collect()
    }

    /// The video filter: scale to the MP4 limit / crop odd sizes to even,
    /// and upload for VA-API; `None` when nothing is needed.
    fn video_filter(&self) -> Option<String> {
        let (ow, oh) = mp4_fit(self.w, self.h);
        let mut f = Vec::new();
        if (ow, oh) != (self.w, self.h) {
            if self.w - ow <= 1 && self.h - oh <= 1 {
                f.push(format!("crop={ow}:{oh}:0:0"));
            } else {
                f.push(format!("scale={ow}:{oh}:flags=area"));
            }
        }
        if self.codec == Codec::Vaapi {
            f.push("format=nv12".into());
            f.push("hwupload".into());
        }
        (!f.is_empty()).then(|| f.join(","))
    }

    /// The command line (without the program).
    pub fn args(&self) -> Vec<OsString> {
        let mut a: Vec<OsString> = Vec::new();
        let mut push = |s: &str| a.push(s.into());
        let audio = self.audio_inputs();
        let sound = !audio.is_empty();
        for s in ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "warning", "-y"] {
            push(s);
        }
        if sound {
            push("-copyts");
        }
        if self.codec == Codec::Vaapi {
            push("-vaapi_device");
            push(VAAPI_DEVICE);
        }
        let size = format!("{}x{}", self.w, self.h);
        let fps = self.fps.to_string();
        if sound {
            // Wall-clock stamps (an input `-r` would replace them with a
            // count); the frame rate comes back on the output.
            for s in [
                "-thread_queue_size", "64", "-use_wallclock_as_timestamps", "1", "-probesize", "32", "-analyzeduration", "0",
                "-f", "rawvideo", "-pix_fmt", "bgra", "-s", &size, "-framerate", &fps, "-i", "-",
            ] {
                push(s);
            }
        } else {
            for s in ["-thread_queue_size", "64", "-f", "rawvideo", "-pix_fmt", "bgra", "-s", &size, "-r", &fps, "-i", "-"] {
                push(s);
            }
        }
        for src in &audio {
            for s in ["-thread_queue_size", "1024", "-probesize", "32", "-analyzeduration", "0", "-f", "pulse", "-i", src] {
                push(s);
            }
        }
        match audio.len() {
            0 => {
                for s in ["-map", "0:v"] {
                    push(s);
                }
            }
            1 => {
                for s in ["-map", "0:v", "-map", "1:a"] {
                    push(s);
                }
            }
            _ => {
                for s in ["-filter_complex", "[1:a][2:a]amix=inputs=2:duration=first:dropout_transition=0[a]", "-map", "0:v", "-map", "[a]"] {
                    push(s);
                }
            }
        }
        if sound {
            for s in ["-vsync", "cfr", "-r", &fps, "-avoid_negative_ts", "make_zero"] {
                push(s);
            }
        }
        if let Some(vf) = self.video_filter() {
            push("-vf");
            push(&vf);
        }
        let q = crf(self.quality).to_string();
        match self.codec {
            Codec::X264 => {
                for s in ["-c:v", "libx264", "-preset", "veryfast", "-crf", &q, "-pix_fmt", "yuv420p"] {
                    push(s);
                }
            }
            Codec::OpenH264 => {
                let (ow, oh) = mp4_fit(self.w, self.h);
                let b = bitrate(ow, oh, self.fps, self.quality).to_string();
                for s in ["-c:v", "libopenh264", "-b:v", &b, "-pix_fmt", "yuv420p"] {
                    push(s);
                }
            }
            Codec::Vaapi => {
                for s in ["-c:v", "h264_vaapi", "-qp", &q] {
                    push(s);
                }
            }
        }
        if !audio.is_empty() {
            for s in ["-c:a", "aac", "-b:a", "160k", "-shortest"] {
                push(s);
            }
        }
        for s in ["-movflags", "+faststart", "-f", "mp4"] {
            push(s);
        }
        a.push(self.out.clone().into_os_string());
        a
    }
}

/// The command line that joins `list` (a concat-demuxer file) into `out`.
pub fn concat_args(list: &Path, out: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "warning", "-y", "-f", "concat", "-safe", "0", "-i"]
        .iter()
        .map(OsString::from)
        .collect();
    a.push(list.as_os_str().to_owned());
    for s in ["-c", "copy", "-movflags", "+faststart", "-f", "mp4"] {
        a.push(s.into());
    }
    a.push(out.as_os_str().to_owned());
    a
}

/// A concat-demuxer list of `files` (`file '<path>'` lines; a `'` in a
/// path is written `'\''`).
pub fn concat_list(files: &[PathBuf]) -> String {
    files.iter().map(|f| format!("file '{}'\n", f.to_string_lossy().replace('\'', "'\\''"))).collect()
}

// ---------------------------------------------------------------------------
// Constant frame rate
// ---------------------------------------------------------------------------

/// Keeps a constant frame rate from timestamped frames: each frame is
/// written as many times as it takes to reach its slot (`round(t * fps)`,
/// `t` from the first frame): 0 (dropped: the output is ahead), 1, or more
/// (duplicated over a gap).
#[derive(Clone, Debug)]
pub struct Cfr {
    fps: u32,
    origin: Option<Duration>,
    sent: u64,
}

/// The most frames one frame is repeated (a minute: a gap that long is a
/// stall; the time is kept anyway, just not beyond this).
const MAX_REPEATS_SECS: u64 = 60;

impl Cfr {
    pub fn new(fps: u32) -> Cfr {
        Cfr { fps: fps.max(1), origin: None, sent: 0 }
    }

    /// Frames written so far.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// How many times to write the frame stamped `ts`.
    pub fn repeats(&mut self, ts: Duration) -> u64 {
        let origin = *self.origin.get_or_insert(ts);
        let t = ts.saturating_sub(origin).as_secs_f64();
        let slot = (t * f64::from(self.fps) + 0.5).floor() as u64;
        if self.sent > slot {
            return 0;
        }
        let n = (slot - self.sent + 1).min(MAX_REPEATS_SECS * u64::from(self.fps));
        self.sent += n;
        n
    }
}

// ---------------------------------------------------------------------------
// The encoder
// ---------------------------------------------------------------------------

/// The end of a stream, bounded.
#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    fn push(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
        if self.0.len() > TAIL_BYTES {
            let cut = self.0.len() - TAIL_BYTES;
            self.0.drain(..cut);
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0).trim().to_string()
    }
}

/// A running ffmpeg: the child, its frame writer and its stderr reader.
struct Run {
    child: Child,
    tx: Option<SyncSender<Arc<Vec<u8>>>>,
    writer: Option<JoinHandle<std::io::Result<()>>>,
    reader: Option<JoinHandle<()>>,
    tail: Arc<Mutex<Tail>>,
    cfr: Cfr,
    out: PathBuf,
    /// Live sound inputs: ffmpeg gets SIGINT if it lingers at the end.
    live: bool,
}

fn stderr_tail(tail: &Mutex<Tail>) -> String {
    let t = tail.lock().unwrap_or_else(|e| e.into_inner()).text();
    if t.is_empty() { "no output".into() } else { t }
}

impl Run {
    fn spawn(exe: &Path, plan: &Plan) -> Result<Run> {
        let (args, fps, out, live) = (plan.args(), plan.fps, plan.out.clone(), !plan.audio_inputs().is_empty());
        let mut child = Command::new(exe)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("start {}", exe.display()))?;
        let tail = Arc::new(Mutex::new(Tail::default()));
        let reader = child.stderr.take().and_then(|mut err| {
            let tail = tail.clone();
            std::thread::Builder::new()
                .name("ffmpeg-stderr".into())
                .spawn(move || {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = err.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        tail.lock().unwrap_or_else(|e| e.into_inner()).push(&buf[..n]);
                    }
                })
                .ok()
        });
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("ffmpeg has no stdin"))?;
        let (tx, rx) = sync_channel::<Arc<Vec<u8>>>(PIPE_FRAMES);
        let writer = std::thread::Builder::new().name("ffmpeg-writer".into()).spawn(move || {
            let mut stdin = stdin;
            for f in rx {
                stdin.write_all(&f)?;
            }
            stdin.flush()
            // Dropping stdin closes the pipe: ffmpeg sees the end.
        });
        let writer = match writer {
            Ok(w) => w,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow::Error::new(e).context("start the ffmpeg writer thread"));
            }
        };
        Ok(Run { child, tx: Some(tx), writer: Some(writer), reader, tail, cfr: Cfr::new(fps), out, live })
    }

    /// Write `f` as many times as the frame rate wants.
    fn push(&mut self, f: &Frame) -> Result<()> {
        let n = self.cfr.repeats(f.ts);
        if n == 0 {
            return Ok(());
        }
        let buf = Arc::new(f.bgra[..(f.w * f.h * 4) as usize].to_vec());
        let tx = self.tx.as_ref().ok_or_else(|| anyhow!("ffmpeg input closed"))?;
        for _ in 0..n {
            if tx.send(buf.clone()).is_err() {
                // The writer stopped: ffmpeg is gone or stopped reading.
                let why = match self.writer.take().map(|w| w.join()) {
                    Some(Ok(Err(e))) => format!("{e}"),
                    _ => "stopped".into(),
                };
                let _ = self.child.wait_timeout(Duration::from_secs(2), false);
                bail!("ffmpeg stopped taking frames ({why}): {}", stderr_tail(&self.tail));
            }
        }
        Ok(())
    }

    /// Close the input and wait for ffmpeg to finish the file. `None`: no
    /// frame was written (a stretch paused before its first frame); ffmpeg
    /// is stopped and its file removed.
    fn finish(mut self) -> Result<Option<PathBuf>> {
        if self.cfr.sent() == 0 {
            drop(self.tx.take());
            if let Some(w) = self.writer.take() {
                let _ = w.join();
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_file(&self.out);
            return Ok(None);
        }
        drop(self.tx.take());
        let wrote = self.writer.take().map(|w| w.join());
        let status = self.child.wait_timeout(FINISH_TIMEOUT, self.live);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        let status = status?;
        if !status.success() {
            bail!("ffmpeg failed ({status}): {}", stderr_tail(&self.tail));
        }
        if let Some(Ok(Err(e))) = wrote {
            bail!("writing frames to ffmpeg failed: {e}: {}", stderr_tail(&self.tail));
        }
        Ok(Some(self.out.clone()))
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // Not finished (an error, or dropped): stop ffmpeg.
        drop(self.tx.take());
        if self.writer.is_some() || self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

trait WaitTimeout {
    fn wait_timeout(&mut self, t: Duration, int: bool) -> Result<std::process::ExitStatus>;
}

impl WaitTimeout for Child {
    /// Wait up to `t` (with `int`, SIGINT after [`INT_AFTER`]); killed
    /// after that.
    fn wait_timeout(&mut self, t: Duration, int: bool) -> Result<std::process::ExitStatus> {
        let start = Instant::now();
        let mut interrupted = !int;
        loop {
            if let Some(s) = self.try_wait()? {
                return Ok(s);
            }
            if !interrupted && start.elapsed() >= INT_AFTER.min(t) {
                interrupted = true;
                interrupt(self);
            }
            if start.elapsed() >= t {
                let _ = self.kill();
                let _ = self.wait();
                bail!("ffmpeg did not finish within {} s", t.as_secs());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// SIGINT to a child we started (ffmpeg then finishes the file).
fn interrupt(c: &Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        const SIGINT: i32 = 2;
        if let Ok(pid) = i32::try_from(c.id()) {
            unsafe {
                kill(pid, SIGINT);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = c;
}

/// MP4 through ffmpeg; see the module docs.
pub struct FfmpegEncoder {
    exe: PathBuf,
    plan: Plan,
    /// The file the session renames (`.part`).
    part: PathBuf,
    /// The ffmpeg writing now (started at the first frame of a segment).
    run: Option<Run>,
    /// Finished segments (with sound, one per stretch between pauses).
    done: Vec<PathBuf>,
    /// After a pause: frames stamped up to here belong to the stretch
    /// before it, which is finished.
    skip_until: Option<Duration>,
    /// A segment failed; `finish` reports it.
    failed: Option<String>,
}

impl FfmpegEncoder {
    /// An encoder for `w`×`h` frames at `fps`, writing `path` (the `.part`
    /// file). `system_monitor` / `mic`: PulseAudio sources ffmpeg records
    /// (they must exist: ask [`super::pulse_list`]). ffmpeg starts at the
    /// first frame, so nothing is heard before the recording begins.
    pub fn create(
        path: &Path,
        w: u32,
        h: u32,
        fps: u32,
        quality: Quality,
        system_monitor: Option<String>,
        mic: Option<String>,
    ) -> Result<FfmpegEncoder> {
        if w == 0 || h == 0 {
            bail!("empty recording area");
        }
        let exe = find().ok_or_else(|| anyhow!("ffmpeg is not installed"))?;
        let caps = caps(&exe);
        let codec = caps.h264.ok_or_else(|| anyhow!("{} has no H.264 encoder (libx264, libopenh264 or h264_vaapi)", exe.display()))?;
        let sound = system_monitor.is_some() || mic.is_some();
        if sound && !caps.pulse {
            bail!("{} cannot record PulseAudio sound", exe.display());
        }
        if sound && !caps.aac {
            bail!("{} has no AAC encoder", exe.display());
        }
        let plan = Plan { w, h, fps: fps.clamp(1, 60), quality, codec, system: system_monitor, mic, out: path.to_path_buf() };
        Ok(FfmpegEncoder { exe, plan, part: path.to_path_buf(), run: None, done: Vec::new(), skip_until: None, failed: None })
    }

    /// The H.264 encoder ffmpeg uses.
    pub fn codec(&self) -> Codec {
        self.plan.codec
    }

    fn sound(&self) -> bool {
        !self.plan.audio_inputs().is_empty()
    }

    /// Where segment `i` is written (with sound) or the `.part` (without).
    fn segment_path(&self, i: usize) -> PathBuf {
        if !self.sound() {
            return self.part.clone();
        }
        let mut s = self.part.as_os_str().to_os_string();
        s.push(format!(".seg{i}.mp4"));
        PathBuf::from(s)
    }

    fn end_segment(&mut self) -> Result<()> {
        if let Some(r) = self.run.take() {
            match r.finish() {
                Ok(p) => self.done.extend(p),
                Err(e) => {
                    self.failed = Some(format!("{e:#}"));
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn cleanup_segments(&self) {
        if self.sound() {
            for p in &self.done {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

impl VideoEncoder for FfmpegEncoder {
    fn push_video(&mut self, f: &Frame) -> Result<()> {
        if let Some(e) = &self.failed {
            bail!("{e}");
        }
        if let Some(t) = self.skip_until {
            if f.ts <= t {
                return Ok(());
            }
            self.skip_until = None;
        }
        if f.w != self.plan.w || f.h != self.plan.h || f.bgra.len() < (f.w * f.h * 4) as usize {
            bail!("frame size changed to {}x{}", f.w, f.h);
        }
        if self.run.is_none() {
            let mut plan = self.plan.clone();
            plan.out = self.segment_path(self.done.len());
            let run = Run::spawn(&self.exe, &plan);
            self.run = Some(run.inspect_err(|e| self.failed = Some(format!("{e:#}")))?);
        }
        let r = self.run.as_mut().map_or(Ok(()), |r| r.push(f));
        if let Err(e) = &r {
            self.failed = Some(format!("{e:#}"));
        }
        r
    }

    fn push_audio(&mut self, _pcm: &[f32], _ts: Duration) -> Result<()> {
        Ok(()) // never called: `audio()` is false (ffmpeg records the sound)
    }

    fn pause(&mut self, at: Duration) -> Result<()> {
        if !self.sound() {
            return Ok(()); // the clock leaves the paused time out
        }
        // What ffmpeg hears from now on is not part of this stretch: end
        // it; the next one starts at the first frame after the pause.
        self.skip_until = Some(at);
        self.end_segment()
    }

    fn finish(mut self: Box<Self>) -> Result<()> {
        if let Some(e) = self.failed.take() {
            // Keep a finished file when there is one, as the contract asks.
            let _ = self.end_segment();
            self.cleanup_segments();
            bail!("{e}");
        }
        if let Err(e) = self.end_segment() {
            self.cleanup_segments();
            return Err(e);
        }
        if !self.sound() {
            return if self.done.is_empty() { Err(anyhow!("no frames were recorded")) } else { Ok(()) };
        }
        match self.done.len() {
            0 => bail!("no frames were recorded"),
            1 => {
                std::fs::rename(&self.done[0], &self.part).context("move the recording into place")?;
                Ok(())
            }
            _ => {
                let mut list = self.part.as_os_str().to_os_string();
                list.push(".concat.txt");
                let list = PathBuf::from(list);
                let r = (|| -> Result<()> {
                    std::fs::write(&list, concat_list(&self.done)).context("write the segment list")?;
                    let out = Command::new(&self.exe)
                        .args(concat_args(&list, &self.part))
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .output()
                        .context("start ffmpeg to join the segments")?;
                    if !out.status.success() {
                        let mut t = Tail::default();
                        t.push(&out.stderr);
                        bail!("joining the segments failed ({}): {}", out.status, t.text());
                    }
                    Ok(())
                })();
                let _ = std::fs::remove_file(&list);
                self.cleanup_segments();
                r
            }
        }
    }

    fn audio(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(codec: Codec, system: bool, mic: bool) -> Plan {
        Plan {
            w: 1280,
            h: 720,
            fps: 30,
            quality: Quality::Medium,
            codec,
            system: system.then(|| "@DEFAULT_MONITOR@".to_string()),
            mic: mic.then(|| "alsa_input.usb-mic".to_string()),
            out: PathBuf::from("/tmp/rec dir/it's.mp4.part"),
        }
    }

    fn joined(p: &Plan) -> String {
        p.args().iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")
    }

    const HEAD: &str = "-hide_banner -nostdin -nostats -loglevel warning -y";
    const HEAD_SOUND: &str = "-hide_banner -nostdin -nostats -loglevel warning -y -copyts";
    const VIDEO_IN: &str = "-thread_queue_size 64 -f rawvideo -pix_fmt bgra -s 1280x720 -r 30 -i -";
    const VIDEO_IN_SOUND: &str = "-thread_queue_size 64 -use_wallclock_as_timestamps 1 -probesize 32 -analyzeduration 0 \
                                  -f rawvideo -pix_fmt bgra -s 1280x720 -framerate 30 -i -";
    const PULSE: &str = "-thread_queue_size 1024 -probesize 32 -analyzeduration 0 -f pulse -i";
    const OUT_SOUND: &str = "-vsync cfr -r 30 -avoid_negative_ts make_zero";
    const X264: &str = "-c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p";
    const TAIL: &str = "-movflags +faststart -f mp4 /tmp/rec dir/it's.mp4.part";

    #[test]
    fn args_video_only() {
        assert_eq!(joined(&plan(Codec::X264, false, false)), format!("{HEAD} {VIDEO_IN} -map 0:v {X264} {TAIL}"));
    }

    #[test]
    fn args_system_sound_only() {
        let p = plan(Codec::X264, true, false);
        assert_eq!(
            joined(&p),
            format!(
                "{HEAD_SOUND} {VIDEO_IN_SOUND} {PULSE} @DEFAULT_MONITOR@ -map 0:v -map 1:a {OUT_SOUND} {X264} \
                 -c:a aac -b:a 160k -shortest {TAIL}"
            )
        );
    }

    #[test]
    fn args_mic_only() {
        let p = plan(Codec::X264, false, true);
        assert_eq!(
            joined(&p),
            format!(
                "{HEAD_SOUND} {VIDEO_IN_SOUND} {PULSE} alsa_input.usb-mic -map 0:v -map 1:a {OUT_SOUND} {X264} \
                 -c:a aac -b:a 160k -shortest {TAIL}"
            )
        );
    }

    #[test]
    fn args_both_sounds_are_mixed() {
        let p = plan(Codec::X264, true, true);
        assert_eq!(
            joined(&p),
            format!(
                "{HEAD_SOUND} {VIDEO_IN_SOUND} {PULSE} @DEFAULT_MONITOR@ {PULSE} alsa_input.usb-mic \
                 -filter_complex [1:a][2:a]amix=inputs=2:duration=first:dropout_transition=0[a] -map 0:v -map [a] {OUT_SOUND} {X264} \
                 -c:a aac -b:a 160k -shortest {TAIL}"
            )
        );
    }

    #[test]
    fn args_each_argument_is_its_own_entry() {
        // Paths and device names with spaces and quotes stay one argv entry.
        let mut p = plan(Codec::X264, true, true);
        p.mic = Some("weird name; rm -rf ~".into());
        let a = p.args();
        assert!(a.iter().any(|x| x == "weird name; rm -rf ~"));
        assert_eq!(a.last().unwrap(), &OsString::from("/tmp/rec dir/it's.mp4.part"));
    }

    /// Picture and sound share the wall clock: the frames' stamps are
    /// kept (no input `-r`, which would replace them), the output gets the
    /// frame rate back and starts at the first frame; the video input is
    /// first, so ffmpeg opens the pulse inputs only once frames come.
    #[test]
    fn args_with_sound_keep_wall_clock_stamps() {
        let a: Vec<String> = plan(Codec::X264, true, true).args().iter().map(|s| s.to_string_lossy().into()).collect();
        let at = |x: &str| a.iter().position(|y| y == x).unwrap_or_else(|| panic!("{x} in {a:?}"));
        let first_i = at("-i");
        assert_eq!(a[first_i + 1], "-", "the video is input 0");
        assert!(at("-use_wallclock_as_timestamps") < first_i && at("-framerate") < first_i);
        assert!(!a[..first_i].contains(&"-r".to_string()), "no input -r");
        assert!(at("-copyts") < first_i);
        assert!(at("-vsync") > at("-map"));
        // Without sound: the spec's plain input, counted at -r.
        let v: Vec<String> = plan(Codec::X264, false, false).args().iter().map(|s| s.to_string_lossy().into()).collect();
        assert!(!v.iter().any(|x| x == "-copyts" || x == "-use_wallclock_as_timestamps" || x == "-vsync"), "{v:?}");
    }

    #[test]
    fn args_quality_sets_crf() {
        for (q, n) in [(Quality::Low, "28"), (Quality::Medium, "23"), (Quality::High, "18")] {
            let mut p = plan(Codec::X264, false, false);
            p.quality = q;
            let a = p.args();
            let i = a.iter().position(|x| x == "-crf").unwrap();
            assert_eq!(a[i + 1], OsString::from(n), "{q:?}");
        }
    }

    #[test]
    fn args_openh264_uses_a_bit_rate() {
        let s = joined(&plan(Codec::OpenH264, false, false));
        let b = bitrate(1280, 720, 30, Quality::Medium);
        assert!(s.contains(&format!("-c:v libopenh264 -b:v {b} -pix_fmt yuv420p")), "{s}");
        assert!(!s.contains("-crf"), "{s}");
    }

    #[test]
    fn args_vaapi_uploads_frames() {
        let s = joined(&plan(Codec::Vaapi, true, false));
        assert!(s.starts_with(&format!("{HEAD_SOUND} -vaapi_device /dev/dri/renderD128 -thread_queue_size 64")), "{s}");
        assert!(s.contains("-vf format=nv12,hwupload -c:v h264_vaapi -qp 23 -c:a aac"), "{s}");
        assert!(!s.contains("yuv420p"), "{s}");
    }

    #[test]
    fn args_odd_sizes_crop_and_big_ones_scale() {
        let mut p = plan(Codec::X264, false, false);
        (p.w, p.h) = (641, 481);
        assert!(joined(&p).contains("-s 641x481 -r 30 -i - -map 0:v -vf crop=640:480:0:0 -c:v"));
        (p.w, p.h) = (7680, 4320);
        assert!(joined(&p).contains("-vf scale=4096:2304:flags=area -c:v"));
        (p.w, p.h) = (1920, 1080);
        assert!(!joined(&p).contains("-vf"));
        p.codec = Codec::Vaapi;
        (p.w, p.h) = (641, 480);
        assert!(joined(&p).contains("-vf crop=640:480:0:0,format=nv12,hwupload"));
    }

    #[test]
    fn concat_list_quotes_paths() {
        let l = concat_list(&[PathBuf::from("/a/b.seg0.mp4"), PathBuf::from("/a/it's.seg1.mp4")]);
        assert_eq!(l, "file '/a/b.seg0.mp4'\nfile '/a/it'\\''s.seg1.mp4'\n");
        let a: Vec<String> = concat_args(Path::new("/a/l.txt"), Path::new("/a/o.part")).iter().map(|s| s.to_string_lossy().into()).collect();
        assert_eq!(
            a.join(" "),
            "-hide_banner -nostdin -nostats -loglevel warning -y -f concat -safe 0 -i /a/l.txt -c copy -movflags +faststart -f mp4 /a/o.part"
        );
    }

    const ENCODERS: &str = "Encoders:
 V..... = Video
 A..... = Audio
 S..... = Subtitle
 .F.... = Frame-level multithreading
 ------
 V....D a64multi             Multicolor charset for Commodore 64 (codec a64_multi)
 V....D libopenh264          OpenH264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V....D h264_vaapi           H.264/AVC (VAAPI) (codec h264)
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 A....D aac                  AAC (Advanced Audio Coding)
 A....D libopus              libopus Opus (codec opus)
";

    #[test]
    fn encoder_detection_prefers_x264_then_openh264_then_vaapi() {
        assert_eq!(pick_h264(ENCODERS, true), Some(Codec::X264));
        let no_x264 = ENCODERS.replace(" V....D libx264 ", " V....D libx265 ");
        assert_eq!(pick_h264(&no_x264, true), Some(Codec::OpenH264));
        let only_vaapi = no_x264.replace("libopenh264", "libvpx");
        assert_eq!(pick_h264(&only_vaapi, true), Some(Codec::Vaapi));
        assert_eq!(pick_h264(&only_vaapi, false), None, "no render node");
        assert_eq!(pick_h264("", true), None);
        assert!(has_aac(ENCODERS));
        assert!(!has_aac(&ENCODERS.replace(" aac ", " ac3 ")));
        // The legend's "V..... = Video" is not an encoder.
        assert!(!video_encoders(ENCODERS).iter().any(|e| e == "="));
        assert_eq!(video_encoders(ENCODERS).len(), 4);
        // An audio encoder named like a video one is not taken.
        assert_eq!(pick_h264(" ------\n A....D libx264  fake\n", true), None);
    }

    #[test]
    fn pulse_device_detection() {
        let devices = "Devices:
 D. = Demuxing supported
 .E = Muxing supported
 ---
 DE alsa            ALSA audio output
 D  lavfi           Libavfilter virtual input device
 DE pulse           Pulse audio output
 D  x11grab         X11 screen capture, using XCB
";
        assert!(has_pulse(devices));
        assert!(!has_pulse(&devices.replace("DE pulse", " E pulse")), "output only");
        assert!(!has_pulse(&devices.replace("pulse", "oss")));
    }

    #[test]
    fn find_takes_an_absolute_override_then_absolute_path_entries() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let (a, b) = if cfg!(windows) { ("C:\\bin", "C:\\usr\\bin") } else { ("/bin", "/usr/bin") };
        let path = OsString::from(format!("relative{sep}{sep}{a}{sep}{b}"));
        let ok_b = |p: &Path| p.starts_with(b);
        // Relative and empty entries are never searched.
        let found = find_with("ffmpeg", None, Some(&path), |p| p.starts_with("relative") || ok_b(p) || p == Path::new("ffmpeg"));
        assert_eq!(found, Some(Path::new(b).join("ffmpeg")));
        let any = |p: &Path| p.is_absolute();
        assert_eq!(find_with("ffmpeg", None, Some(&path), any), Some(Path::new(a).join("ffmpeg")));
        // The override wins when absolute and executable.
        let over = Path::new(b).join("my-ffmpeg");
        assert_eq!(find_with("ffmpeg", Some(over.as_os_str()), Some(&path), any), Some(over.clone()));
        // A relative override is ignored (PATH is searched).
        assert_eq!(find_with("ffmpeg", Some(OsStr::new("bin/ffmpeg")), Some(&path), any), Some(Path::new(a).join("ffmpeg")));
        assert_eq!(find_with("ffmpeg", None, None, any), None);
        assert_eq!(find_with("ffmpeg", None, Some(&path), |_| false), None);
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn cfr_on_time_frames_are_written_once() {
        let mut c = Cfr::new(30);
        let n: Vec<u64> = (0..30).map(|i| c.repeats(ms(1000 * i / 30))).collect();
        assert!(n.iter().all(|&k| k == 1), "{n:?}");
        assert_eq!(c.sent(), 30);
    }

    #[test]
    fn cfr_starts_at_the_first_frame_and_tolerates_jitter() {
        let mut c = Cfr::new(30);
        assert_eq!(c.repeats(ms(5000)), 1, "origin = first frame");
        assert_eq!(c.repeats(ms(5000 + 33 + 12)), 1, "late by a third");
        assert_eq!(c.repeats(ms(5000 + 66 - 12)), 1, "early by a third");
        assert_eq!(c.sent(), 3);
    }

    #[test]
    fn cfr_duplicates_over_gaps_and_drops_when_ahead() {
        let mut c = Cfr::new(10);
        assert_eq!(c.repeats(ms(0)), 1);
        // 450 ms later: slots 1..=5 (4 behind + this one).
        assert_eq!(c.repeats(ms(450)), 4 + 1);
        assert_eq!(c.sent(), 6);
        // Two frames for one slot: the second is dropped.
        assert_eq!(c.repeats(ms(600)), 1);
        assert_eq!(c.repeats(ms(620)), 0);
        // Same timestamp again (the frame in flight at a pause).
        assert_eq!(c.repeats(ms(620)), 0);
        assert_eq!(c.repeats(ms(700)), 1);
        assert_eq!(c.sent(), 8);
        // The duration follows the last timestamp: 0.7 s → 8 frames at 10 fps.
    }

    #[test]
    fn cfr_bounds_one_frames_repeats() {
        let mut c = Cfr::new(30);
        c.repeats(ms(0));
        assert_eq!(c.repeats(Duration::from_secs(3600)), MAX_REPEATS_SECS * 30);
    }

    #[test]
    fn tail_keeps_the_end() {
        let mut t = Tail::default();
        t.push(&[b'a'; TAIL_BYTES]);
        t.push(b"the error\n");
        assert_eq!(t.0.len(), TAIL_BYTES);
        assert!(t.text().ends_with("the error"));
    }
}
