//! MP4 through one `ffmpeg` process (Linux, when ffmpeg is installed).
//!
//! Rustshot writes raw BGRA frames to ffmpeg's stdin at a constant frame
//! rate (`-f rawvideo -pix_fmt bgra -s WxH -r FPS -i -`; frames are
//! duplicated or dropped to keep the rate, see [`Cfr`]) and, with sound,
//! the session's mixed 48 kHz stereo f32 to a second pipe, ffmpeg's fd 3
//! (`-f f32le -ar 48000 -ac 2 -i pipe:3`; see [`super::child`]). ffmpeg
//! writes H.264 (libx264, else libopenh264, else h264_vaapi when an encode
//! probe works) and AAC, `-movflags +faststart`, to the `.part` file.
//!
//! The sound is captured in-app ([`super::parec`]) and mixed by the shared
//! [`crate::rec::Mixer`], as on Windows. Raw PCM has no timestamps, so the
//! pipe must carry contiguous sound from clock time 0: the mixer produces
//! that (silence included), [`PcmTimeline`] holds it to it, and the frames
//! are placed from clock time 0 too ([`Cfr`]), so the picture and the
//! sound line up by the session clock. Pauses need nothing here: the clock
//! leaves the paused time out of both.
//!
//! Frames and sound go to ffmpeg on two writer threads through bounded
//! queues, fed so that neither pipe can hold the other up for good (ffmpeg
//! 7 stops reading an input that runs ahead; see [`super::feed`]). Nothing
//! goes through a shell: every argument is its own argv entry.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::rec::{bitrate, mp4_fit, Quality};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// Absolute path of the ffmpeg to use instead of the one on `PATH`.
pub const ENV_OVERRIDE: &str = "RUSTSHOT_FFMPEG";

/// The VA-API render node tried for `h264_vaapi`.
pub const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

/// The stderr kept for error messages (the last bytes).
const TAIL_BYTES: usize = 4096;

/// The AAC bit rate of the recording's sound.
const AAC_RATE: &str = "160k";

/// Sound sample rate and channels on the pipe (the mixer's output).
const RATE: u32 = crate::rec::mix::RATE;

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

/// `name` in an absolute `PATH` entry (or the absolute override `over`).
pub fn find_tool(name: &str, over: Option<OsString>) -> Option<PathBuf> {
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

/// What `ffmpeg -encoders` said (and the VA-API probe).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caps {
    pub h264: Option<Codec>,
    pub aac: bool,
}

/// Names in a listing of `ffmpeg -hide_banner -encoders` whose flag column
/// (`flags` characters wide, after one leading space) starts with `kind`
/// ('V' video encoder, 'A' audio). The legend above the `---` line is
/// skipped.
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
/// libx264, else libopenh264, else h264_vaapi when `vaapi_works` (asked
/// only then: it runs an encode probe).
pub fn pick_h264(encoders: &str, vaapi_works: impl FnOnce() -> bool) -> Option<Codec> {
    let v = video_encoders(encoders);
    let has = |n: &str| v.iter().any(|e| e == n);
    if let Some(c) = [Codec::X264, Codec::OpenH264].into_iter().find(|c| has(c.name())) {
        return Some(c);
    }
    (has(Codec::Vaapi.name()) && vaapi_works()).then_some(Codec::Vaapi)
}

/// Whether `ffmpeg -hide_banner -encoders` lists the native AAC encoder.
pub fn has_aac(encoders: &str) -> bool {
    listed(encoders, 6, 'A').iter().any(|e| e == "aac")
}

/// The VA-API probe: one small frame encoded with h264_vaapi to nowhere.
pub fn vaapi_probe_args() -> Vec<OsString> {
    os(&[
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-vaapi_device",
        VAAPI_DEVICE,
        "-f",
        "lavfi",
        "-i",
        "color=black:s=64x64:d=0.1",
        "-vf",
        "format=nv12,hwupload",
        "-c:v",
        "h264_vaapi",
        "-frames:v",
        "1",
        "-f",
        "null",
        "-",
    ])
}

/// How long a capability probe (`-encoders`, the VA-API encode) may take
/// before it is stopped and taken as a failure.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Why [`run_bounded`] has no output.
#[derive(Debug, PartialEq, Eq)]
pub enum RunErr {
    /// It could not be started.
    Start(String),
    /// It ran longer than allowed and was stopped (our child, by its PID).
    Timeout,
}

/// Run `cmd` (stdin and stderr closed) for at most `limit`: whether it
/// succeeded, and its stdout.
pub fn run_bounded(mut cmd: Command, limit: Duration) -> Result<(bool, Vec<u8>), RunErr> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(target_os = "linux")]
    super::child::prepare(&mut cmd, None);
    let mut child = cmd.spawn().map_err(|e| RunErr::Start(e.to_string()))?;
    let reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = std::io::Read::read_to_end(&mut out, &mut b);
            b
        })
    });
    let end = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(10)),
            _ => break None,
        }
    };
    let Some(status) = status else {
        let _ = child.kill(); // our own child, by its PID
        let _ = child.wait();
        if let Some(r) = reader {
            let _ = r.join();
        }
        return Err(RunErr::Timeout);
    };
    let out = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    Ok((status.success(), out))
}

/// `ffmpeg -hide_banner <what>`'s output; `None` when it did not run to
/// the end in time.
fn run_listing(exe: &Path, what: &str) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.args(["-hide_banner", what]);
    match run_bounded(cmd, PROBE_TIMEOUT) {
        Ok((_, out)) => Some(String::from_utf8_lossy(&out).into_owned()),
        Err(e) => {
            eprintln!("rustshot: cannot run {} {what}: {e:?}", exe.display());
            None
        }
    }
}

fn vaapi_works(exe: &Path) -> bool {
    if !Path::new(VAAPI_DEVICE).exists() {
        return false;
    }
    let mut cmd = Command::new(exe);
    cmd.args(vaapi_probe_args());
    let ok = matches!(run_bounded(cmd, PROBE_TIMEOUT), Ok((true, _)));
    if !ok {
        eprintln!("rustshot: h264_vaapi is listed but cannot encode here (or took over {} s); not used", PROBE_TIMEOUT.as_secs());
    }
    ok
}

/// What each ffmpeg can do: `None` while it is being asked.
type CapsCache = (Mutex<Vec<(PathBuf, Option<Caps>)>>, std::sync::Condvar);

fn cache() -> &'static CapsCache {
    static CACHE: OnceLock<CapsCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Ask `exe` what it can do (each probe at most [`PROBE_TIMEOUT`]); cached
/// per path, and only once at a time (a second caller waits for the
/// first). Blocks: call it off the UI thread ([`caps_now`] does not).
pub fn caps(exe: &Path) -> Caps {
    let (m, cv) = cache();
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        match g.iter().find(|(p, _)| p == exe) {
            Some((_, Some(c))) => return c.clone(),
            Some((_, None)) => g = cv.wait(g).unwrap_or_else(|e| e.into_inner()),
            None => break,
        }
    }
    g.push((exe.to_path_buf(), None));
    drop(g);
    let enc = run_listing(exe, "-encoders");
    let c = match &enc {
        Some(enc) => Caps { h264: pick_h264(enc, || vaapi_works(exe)), aac: has_aac(enc) },
        None => Caps { h264: None, aac: false },
    };
    // A listing that did not come in time counts as no encoder for the
    // rest of the run (the probe is not repeated every time the UI looks).
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|(p, _)| p != exe);
    g.push((exe.to_path_buf(), Some(c.clone())));
    drop(g);
    cv.notify_all();
    c
}

/// What `exe` can do if known; otherwise `None`, with the question asked
/// on a background thread (the UI shows "checking" meanwhile).
pub fn caps_now(exe: &Path) -> Option<Caps> {
    let (m, _) = cache();
    let g = m.lock().unwrap_or_else(|e| e.into_inner());
    match g.iter().find(|(p, _)| p == exe) {
        Some((_, c)) => return c.clone(),
        None => drop(g),
    }
    warm(exe);
    None
}

/// Ask `exe` what it can do on a background thread (at start, so the
/// Record popover knows by the time it opens).
pub fn warm(exe: &Path) {
    let exe = exe.to_path_buf();
    let _ = std::thread::Builder::new().name("ffmpeg-caps".into()).spawn(move || {
        caps(&exe);
    });
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// The ffmpeg run.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Input frame size.
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub quality: Quality,
    pub codec: Codec,
    /// Sound comes on fd 3.
    pub sound: bool,
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

fn os(v: &[&str]) -> Vec<OsString> {
    v.iter().map(OsString::from).collect()
}

const HEAD: [&str; 6] = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "warning", "-y"];

/// Frames ffmpeg's video input reads ahead: few, since each is a whole
/// raw frame (32 MB at 4K); the sound's packets are small (8 KB).
const VIDEO_QUEUE: &str = "16";
const SOUND_QUEUE: &str = "512";

impl Plan {
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
        let mut a = os(&HEAD);
        if self.codec == Codec::Vaapi {
            a.extend(os(&["-vaapi_device", VAAPI_DEVICE]));
        }
        let size = format!("{}x{}", self.w, self.h);
        let fps = self.fps.to_string();
        let probe = ["-probesize", "32", "-analyzeduration", "0"];
        a.extend(os(&["-thread_queue_size", VIDEO_QUEUE]));
        a.extend(os(&probe));
        a.extend(os(&["-f", "rawvideo", "-pix_fmt", "bgra", "-s", &size, "-r", &fps, "-i", "-"]));
        if self.sound {
            let rate = RATE.to_string();
            a.extend(os(&["-thread_queue_size", SOUND_QUEUE]));
            a.extend(os(&probe));
            a.extend(os(&["-f", "f32le", "-ar", &rate, "-ac", "2", "-i", "pipe:3"]));
        }
        a.extend(os(&["-map", "0:v"]));
        if self.sound {
            a.extend(os(&["-map", "1:a"]));
        }
        if let Some(vf) = self.video_filter() {
            a.extend(os(&["-vf", &vf]));
        }
        let q = crf(self.quality).to_string();
        match self.codec {
            Codec::X264 => a.extend(os(&["-c:v", "libx264", "-preset", "veryfast", "-crf", &q, "-pix_fmt", "yuv420p"])),
            Codec::OpenH264 => {
                let (ow, oh) = mp4_fit(self.w, self.h);
                let b = bitrate(ow, oh, self.fps, self.quality).to_string();
                a.extend(os(&["-c:v", "libopenh264", "-b:v", &b, "-pix_fmt", "yuv420p"]));
            }
            Codec::Vaapi => a.extend(os(&["-c:v", "h264_vaapi", "-qp", &q])),
        }
        if self.sound {
            a.extend(os(&["-c:a", "aac", "-b:a", AAC_RATE]));
        }
        a.extend(os(&["-movflags", "+faststart", "-f", "mp4"]));
        a.push(self.out.clone().into_os_string());
        a
    }
}

// ---------------------------------------------------------------------------
// Constant frame rate, contiguous sound
// ---------------------------------------------------------------------------

/// Keeps a constant frame rate from timestamped frames: each frame is
/// written as many times as it takes to reach its slot (`round(t * fps)`,
/// `t` the session clock, which the sound starts at too): 0 (dropped: the
/// output is ahead), 1, or more (duplicated over a gap; the first frame
/// covers the time from 0 to it).
#[derive(Clone, Debug)]
pub struct Cfr {
    fps: u32,
    sent: u64,
}

/// The most frames one frame is written (two seconds' worth). After a
/// longer gap (a stall) the rest is made up by the frames that follow,
/// two seconds per frame, so the picture keeps time with the sound
/// without one frame holding the encoder for minutes.
const MAX_REPEATS_SECS: u64 = 2;

impl Cfr {
    pub fn new(fps: u32) -> Cfr {
        Cfr { fps: fps.max(1), sent: 0 }
    }

    /// Frames written so far.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// The length of what was written.
    pub fn length(&self) -> Duration {
        Duration::from_secs_f64(self.sent as f64 / f64::from(self.fps))
    }

    fn slot(&self, ts: Duration) -> u64 {
        (ts.as_secs_f64() * f64::from(self.fps) + 0.5).floor() as u64
    }

    /// How many times to write the frame stamped `ts`.
    pub fn repeats(&mut self, ts: Duration) -> u64 {
        let slot = self.slot(ts);
        if self.sent > slot {
            return 0;
        }
        let n = (slot - self.sent + 1).min(MAX_REPEATS_SECS * u64::from(self.fps));
        self.sent += n;
        n
    }

    /// `n` of the frames counted were not written after all (no room): the
    /// frames that follow fill their slots.
    pub fn unsend(&mut self, n: u64) {
        self.sent = self.sent.saturating_sub(n);
    }

    /// How many more times to write the last frame so the picture lasts
    /// `end` (to the frame; the end of the recording).
    pub fn hold_to(&mut self, end: Duration) -> u64 {
        let n = self.slot(end).saturating_sub(self.sent);
        self.sent += n;
        n
    }
}

/// Holds the sound pipe to contiguous PCM from time 0 (raw PCM carries no
/// timestamps): a push that starts later than where the last one ended
/// gets silence before it, one that overlaps loses its head. The session
/// delivers contiguous sound, so neither should happen.
#[derive(Clone, Debug, Default)]
pub struct PcmTimeline {
    /// Stereo frames written.
    next: u64,
}

fn frames_at(t: Duration) -> u64 {
    ((t.as_nanos() * u128::from(RATE) + 500_000_000) / 1_000_000_000) as u64
}

impl PcmTimeline {
    /// For `pcm` (interleaved stereo) stamped `ts`: the silent frames to
    /// write first, and the samples of it to write.
    pub fn place<'a>(&mut self, pcm: &'a [f32], ts: Duration) -> (u64, &'a [f32]) {
        let at = frames_at(ts);
        let whole = &pcm[..pcm.len() / 2 * 2];
        let (gap, pcm) = if at >= self.next {
            (at - self.next, whole)
        } else {
            let skip = ((self.next - at) as usize * 2).min(whole.len());
            (0, &whole[skip..])
        };
        self.next += gap + (pcm.len() / 2) as u64;
        (gap, pcm)
    }

    /// The silent frames that make the sound last `end`.
    pub fn pad_to(&mut self, end: Duration) -> u64 {
        let n = frames_at(end).saturating_sub(self.next);
        self.next += n;
        n
    }

    /// The length of what was written.
    pub fn length(&self) -> Duration {
        Duration::from_nanos((u128::from(self.next) * 1_000_000_000 / u128::from(RATE)) as u64)
    }
}

/// `pcm` as f32le bytes (what `-f f32le` reads).
pub fn pcm_bytes(pcm: &[f32]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

// ---------------------------------------------------------------------------
// Finishing
// ---------------------------------------------------------------------------

/// Whether the MP4 at `path` has a complete `moov` box at its top level
/// with a complete `mvhd` (movie header) in it: without them no player can
/// open the file.
pub fn has_moov(path: &Path) -> bool {
    std::fs::File::open(path).is_ok_and(|f| {
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        moov_in(std::io::BufReader::new(f), len)
    })
}

/// [`has_moov`] over a reader of `len` bytes.
pub fn moov_in(mut r: impl std::io::Read + std::io::Seek, len: u64) -> bool {
    let Some((start, end)) = find_box(&mut r, 0, len, b"moov") else { return false };
    find_box(&mut r, start, end, b"mvhd").is_some()
}

/// The first complete box of `kind` among the boxes from `from` to `to`
/// (one level): where its body starts and where it ends. `None` when there
/// is none, or a box before it is cut short.
fn find_box(r: &mut (impl std::io::Read + std::io::Seek), from: u64, to: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    use std::io::SeekFrom;
    let mut at = from;
    while at + 8 <= to {
        let mut h = [0u8; 16];
        r.seek(SeekFrom::Start(at)).ok()?;
        r.read_exact(&mut h[..8]).ok()?;
        let size32 = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
        let (size, head) = match size32 {
            0 => (to - at, 8), // to the end
            1 => {
                r.read_exact(&mut h[8..16]).ok()?;
                (u64::from_be_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]), 16)
            }
            n => (u64::from(n), 8),
        };
        if size < head || at + size > to {
            return None; // cut short
        }
        if &h[4..8] == kind {
            return Some((at + head, at + size));
        }
        at += size;
    }
    None
}

/// Waits for ffmpeg to finish a file while the file keeps changing: no
/// fixed time limit, only a limit on how long nothing happens.
#[derive(Debug)]
pub struct Progress<S> {
    last: Option<S>,
    since: Instant,
    limit: Duration,
}

/// What [`Progress`] watches while ffmpeg finishes: the output's length
/// and modification time, and the bytes the writers have handed it.
pub type FileState = (u64, Option<SystemTime>, u64);

/// How long the output may stay unchanged before ffmpeg is taken for hung.
pub const STALL: Duration = Duration::from_secs(30);

impl<S: PartialEq> Progress<S> {
    pub fn new(now: Instant, limit: Duration) -> Progress<S> {
        Progress { last: None, since: now, limit }
    }

    /// The state at `now` (e.g. [`FileState`]): whether it stopped changing
    /// for longer than the limit.
    pub fn stalled(&mut self, now: Instant, state: S) -> bool {
        if self.last.as_ref() != Some(&state) {
            self.last = Some(state);
            self.since = now;
            return false;
        }
        now.duration_since(self.since) > self.limit
    }
}

/// The end of a stream, bounded.
#[derive(Default)]
pub(crate) struct Tail(Vec<u8>);

impl Tail {
    pub(crate) fn push(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
        if self.0.len() > TAIL_BYTES {
            let cut = self.0.len() - TAIL_BYTES;
            self.0.drain(..cut);
        }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0).trim().to_string()
    }
}

#[cfg(target_os = "linux")]
pub use imp::FfmpegEncoder;

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::rec::linux::child;
    use crate::rec::linux::feed::{self, Feeder, Pipe, Stuck};
    use crate::rec::{Frame, Salvaged, VideoEncoder};
    use anyhow::{anyhow, bail, Context, Result};
    use std::io::{Read, Write};
    use std::process::{Child, ExitStatus};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;

    fn tail_text(tail: &Mutex<Tail>) -> String {
        let t = tail.lock().unwrap_or_else(|e| e.into_inner()).text();
        if t.is_empty() { "no output".into() } else { t }
    }

    /// RUSTSHOT_FFMPEG_LOG set: ffmpeg's command line and log are printed.
    fn verbose() -> bool {
        std::env::var_os("RUSTSHOT_FFMPEG_LOG").is_some()
    }

    type Writer = Option<JoinHandle<std::io::Result<()>>>;

    fn writer<T: AsRef<[u8]> + Send + 'static>(
        name: &str,
        q: Arc<feed::Bounded<T>>,
        w: impl Write + Send + 'static,
        written: Arc<AtomicU64>,
    ) -> Result<Writer> {
        let h = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || feed::pump(&q, w, &written))
            .context("start an ffmpeg writer thread")?;
        Ok(Some(h))
    }

    /// MP4 through ffmpeg; see the module docs.
    pub struct FfmpegEncoder {
        plan: Plan,
        child: Child,
        feed: Feeder,
        video_w: Writer,
        sound_w: Writer,
        /// Bytes the writers handed ffmpeg (progress while finishing).
        written: Arc<AtomicU64>,
        reader: Option<JoinHandle<()>>,
        tail: Arc<Mutex<Tail>>,
        /// A write failed; `finish` reports it.
        failed: Option<String>,
        finished: bool,
    }

    impl FfmpegEncoder {
        /// An encoder for `w`×`h` frames at `fps`, writing `path` (the
        /// `.part` file); with `sound` it takes the session's mixed sound
        /// ([`VideoEncoder::audio`]). ffmpeg starts now, on the calling
        /// thread, which must outlive it (see [`child`]): it waits for the
        /// first frame.
        pub fn create(path: &Path, w: u32, h: u32, fps: u32, quality: Quality, sound: bool) -> Result<FfmpegEncoder> {
            if w == 0 || h == 0 {
                bail!("empty recording area");
            }
            let exe = find().ok_or_else(|| anyhow!("ffmpeg is not installed"))?;
            let caps = caps(&exe);
            let codec = caps.h264.ok_or_else(|| anyhow!("{} has no usable H.264 encoder (libx264, libopenh264 or h264_vaapi)", exe.display()))?;
            if sound && !caps.aac {
                bail!("{} has no AAC encoder", exe.display());
            }
            let fps = fps.clamp(1, 60);
            let plan = Plan { w, h, fps, quality, codec, sound, out: path.to_path_buf() };
            let args = plan.args();
            if verbose() {
                eprintln!("rustshot: {} {}", exe.display(), args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
            }
            let pipe = if sound { Some(child::pipe().context("make the sound pipe")?) } else { None };
            let mut cmd = Command::new(&exe);
            cmd.args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
            child::prepare(&mut cmd, pipe.as_ref().map(|p| &p.0));
            let mut child = cmd.spawn().with_context(|| format!("start {}", exe.display()))?;
            // ffmpeg has its copy of the read end; ours goes, so a dead
            // ffmpeg is a broken pipe rather than a stuck write.
            let sound_fd = pipe.map(|(r, w)| {
                drop(r);
                w
            });
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
            let stdin = child.stdin.take();
            let mut enc = FfmpegEncoder {
                plan,
                child,
                feed: Feeder::new(fps, sound, feed::SEND_TIMEOUT),
                video_w: None,
                sound_w: None,
                written: Arc::new(AtomicU64::new(0)),
                reader,
                tail,
                failed: None,
                finished: false,
            };
            let stdin = stdin.ok_or_else(|| anyhow!("ffmpeg has no stdin"))?;
            enc.video_w = writer("ffmpeg-video", enc.feed.video.clone(), stdin, enc.written.clone())?;
            if let (Some(fd), Some(q)) = (sound_fd, enc.feed.sound.clone()) {
                enc.sound_w = writer("ffmpeg-sound", q, std::fs::File::from(fd), enc.written.clone())?;
            }
            Ok(enc)
        }

        /// The H.264 encoder ffmpeg uses.
        pub fn codec(&self) -> Codec {
            self.plan.codec
        }

        /// Stop ffmpeg (our child, by its PID).
        fn kill(&mut self) {
            let _ = self.child.kill();
        }

        /// A write that did not go through: why, with ffmpeg's last words.
        /// A pipe that took nothing for the timeout means ffmpeg hung: it
        /// is stopped.
        fn stuck(&mut self, s: Stuck) -> anyhow::Error {
            let e = match s {
                Stuck::Timeout(p) => {
                    self.kill();
                    anyhow!("ffmpeg took no {} for {} s and was stopped: {}", p.name(), feed::SEND_TIMEOUT.as_secs(), tail_text(&self.tail))
                }
                Stuck::Closed(p) => {
                    let w = if p == Pipe::Frames { self.video_w.take() } else { self.sound_w.take() };
                    let why = match w.map(|w| w.join()) {
                        Some(Ok(Err(e))) => format!("{e}"),
                        _ => "stopped".into(),
                    };
                    std::thread::sleep(Duration::from_millis(200)); // its last words
                    anyhow!("ffmpeg stopped taking {} ({why}): {}", p.name(), tail_text(&self.tail))
                }
            };
            self.failed = Some(format!("{e:#}"));
            e
        }

        /// Even out the ends (unless a write failed already), close both
        /// pipes and wait while ffmpeg keeps going: the writers handing it
        /// what is left and ffmpeg writing the file. Nothing changing for
        /// [`STALL`] (writers stuck or ffmpeg hung) stops it.
        fn close_and_wait(&mut self, failed: bool) -> Result<ExitStatus> {
            let even = if failed { Ok(()) } else { self.feed.even_out().map_err(|s| self.stuck(s)) };
            self.feed.close();
            let mut p = Progress::new(Instant::now(), STALL);
            let mut killed = false;
            let status = loop {
                let writing = [&self.video_w, &self.sound_w].iter().any(|w| w.as_ref().is_some_and(|h| !h.is_finished()));
                if !writing && let Some(s) = self.child.try_wait()? {
                    break s;
                }
                let (len, modified) = std::fs::metadata(&self.plan.out).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
                let state: FileState = (len, modified, self.written.load(Ordering::Relaxed));
                if !killed && p.stalled(Instant::now(), state) {
                    self.kill(); // the writers then fail on the broken pipes
                    killed = true;
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let wrote: Vec<_> = [self.video_w.take(), self.sound_w.take()].into_iter().flatten().map(|w| w.join()).collect();
            if let Some(r) = self.reader.take() {
                let _ = r.join();
            }
            if verbose() {
                eprintln!("rustshot: ffmpeg ended ({status}):\n{}", tail_text(&self.tail));
            }
            even?;
            if killed {
                bail!("ffmpeg made no progress for {} s and was stopped: {}", STALL.as_secs(), tail_text(&self.tail));
            }
            if !status.success() {
                bail!("ffmpeg failed ({status}): {}", tail_text(&self.tail));
            }
            for w in wrote {
                if let Ok(Err(e)) = w {
                    bail!("writing to ffmpeg failed: {e}: {}", tail_text(&self.tail));
                }
            }
            Ok(status)
        }
    }

    impl VideoEncoder for FfmpegEncoder {
        fn push_video(&mut self, f: &Frame) -> Result<()> {
            if let Some(e) = &self.failed {
                bail!("{e}");
            }
            if f.w != self.plan.w || f.h != self.plan.h || f.bgra.len() < (f.w * f.h * 4) as usize {
                bail!("frame size changed to {}x{}", f.w, f.h);
            }
            let n = (f.w * f.h * 4) as usize;
            self.feed.frame(f.ts, || Arc::from(&f.bgra[..n])).map_err(|s| self.stuck(s))
        }

        fn push_audio(&mut self, pcm: &[f32], ts: Duration) -> Result<()> {
            if let Some(e) = &self.failed {
                bail!("{e}");
            }
            self.feed.audio(pcm, ts).map_err(|s| self.stuck(s))
        }

        /// `Ok` once ffmpeg wrote the whole file. When it fails but the
        /// file still has its index (`moov` with `mvhd`), the error is
        /// [`Salvaged`] and the session keeps the file; otherwise it
        /// removes it.
        fn finish(mut self: Box<Self>) -> Result<()> {
            self.finished = true;
            let failed = self.failed.take();
            let r = self.close_and_wait(failed.is_some());
            let e = match (failed, r) {
                (None, Ok(_)) if self.feed.frames() > 0 => return Ok(()),
                (None, Ok(_)) => anyhow!("no frames were recorded"),
                (Some(f), _) => anyhow!("{f}"),
                (None, Err(e)) => e,
            };
            if self.feed.frames() > 0 && has_moov(&self.plan.out) {
                return Err(anyhow::Error::new(Salvaged(format!("{e:#}"))));
            }
            Err(e)
        }

        fn audio(&self) -> bool {
            self.plan.sound
        }
    }

    impl Drop for FfmpegEncoder {
        /// Dropped without finishing (cancelled, or the session failed to
        /// start): ffmpeg is stopped (our child, by its PID).
        fn drop(&mut self) {
            self.feed.close();
            if !self.finished || self.child.try_wait().ok().flatten().is_none() {
                self.kill();
                let _ = self.child.wait();
            }
            for w in [self.video_w.take(), self.sound_w.take()].into_iter().flatten() {
                let _ = w.join();
            }
            if let Some(r) = self.reader.take() {
                let _ = r.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(codec: Codec) -> Plan {
        Plan { w: 1280, h: 720, fps: 30, quality: Quality::Medium, codec, sound: false, out: PathBuf::from("/tmp/rec dir/it's.mp4.part") }
    }

    fn joined(a: &[OsString]) -> String {
        a.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")
    }

    const HEAD_S: &str = "-hide_banner -nostdin -nostats -loglevel warning -y";
    const VIDEO_IN: &str = "-thread_queue_size 16 -probesize 32 -analyzeduration 0 -f rawvideo -pix_fmt bgra -s 1280x720 -r 30 -i -";
    const SOUND_IN: &str = "-thread_queue_size 512 -probesize 32 -analyzeduration 0 -f f32le -ar 48000 -ac 2 -i pipe:3";
    const X264: &str = "-c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p";
    const TAIL: &str = "-movflags +faststart -f mp4 /tmp/rec dir/it's.mp4.part";

    #[test]
    fn video_args() {
        assert_eq!(joined(&plan(Codec::X264).args()), format!("{HEAD_S} {VIDEO_IN} -map 0:v {X264} {TAIL}"));
    }

    #[test]
    fn sound_args_add_the_pcm_pipe_and_aac() {
        let mut p = plan(Codec::X264);
        p.sound = true;
        let a = p.args();
        assert_eq!(joined(&a), format!("{HEAD_S} {VIDEO_IN} {SOUND_IN} -map 0:v -map 1:a {X264} -c:a aac -b:a 160k {TAIL}"));
        // One process, no filters mixing sound, no -shortest.
        assert!(!a.iter().any(|x| x == "-shortest" || x == "-filter_complex" || x == "pulse"));
        // The output path stays one argv entry.
        assert_eq!(a.last().unwrap(), &OsString::from("/tmp/rec dir/it's.mp4.part"));
    }

    #[test]
    fn video_args_quality_sets_crf() {
        for (q, n) in [(Quality::Low, "28"), (Quality::Medium, "23"), (Quality::High, "18")] {
            let mut p = plan(Codec::X264);
            p.quality = q;
            let a = p.args();
            let i = a.iter().position(|x| x == "-crf").unwrap();
            assert_eq!(a[i + 1], OsString::from(n), "{q:?}");
        }
    }

    #[test]
    fn video_args_openh264_uses_a_bit_rate() {
        let s = joined(&plan(Codec::OpenH264).args());
        let b = bitrate(1280, 720, 30, Quality::Medium);
        assert!(s.contains(&format!("-c:v libopenh264 -b:v {b} -pix_fmt yuv420p")), "{s}");
        assert!(!s.contains("-crf"), "{s}");
    }

    #[test]
    fn video_args_vaapi_uploads_frames() {
        let s = joined(&plan(Codec::Vaapi).args());
        assert!(s.starts_with(&format!("{HEAD_S} -vaapi_device /dev/dri/renderD128 -thread_queue_size 16")), "{s}");
        assert!(s.contains("-vf format=nv12,hwupload -c:v h264_vaapi -qp 23 -movflags"), "{s}");
        assert!(!s.contains("yuv420p"), "{s}");
    }

    #[test]
    fn video_args_odd_sizes_crop_and_big_ones_scale() {
        let mut p = plan(Codec::X264);
        (p.w, p.h) = (641, 481);
        assert!(joined(&p.args()).contains("-s 641x481 -r 30 -i - -map 0:v -vf crop=640:480:0:0 -c:v"));
        (p.w, p.h) = (7680, 4320);
        assert!(joined(&p.args()).contains("-vf scale=4096:2304:flags=area -c:v"));
        (p.w, p.h) = (1920, 1080);
        assert!(!joined(&p.args()).contains("-vf"));
        p.codec = Codec::Vaapi;
        (p.w, p.h) = (641, 480);
        assert!(joined(&p.args()).contains("-vf crop=640:480:0:0,format=nv12,hwupload"));
    }

    #[test]
    fn vaapi_probe_encodes_one_frame_to_nowhere() {
        let s = joined(&vaapi_probe_args());
        assert!(s.contains("-vaapi_device /dev/dri/renderD128 -f lavfi -i color=black:s=64x64:d=0.1"), "{s}");
        assert!(s.ends_with("-vf format=nv12,hwupload -c:v h264_vaapi -frames:v 1 -f null -"), "{s}");
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
    fn encoder_detection_prefers_x264_then_openh264_then_a_working_vaapi() {
        let never = || -> bool { panic!("no probe needed") };
        assert_eq!(pick_h264(ENCODERS, never), Some(Codec::X264));
        let no_x264 = ENCODERS.replace(" V....D libx264 ", " V....D libx265 ");
        assert_eq!(pick_h264(&no_x264, never), Some(Codec::OpenH264));
        let only_vaapi = no_x264.replace("libopenh264", "libvpx");
        assert_eq!(pick_h264(&only_vaapi, || true), Some(Codec::Vaapi));
        assert_eq!(pick_h264(&only_vaapi, || false), None, "the probe failed: nothing usable");
        assert_eq!(pick_h264("", || true), None);
        assert!(has_aac(ENCODERS));
        assert!(!has_aac(&ENCODERS.replace(" aac ", " ac3 ")));
        // The legend's "V..... = Video" is not an encoder.
        assert!(!video_encoders(ENCODERS).iter().any(|e| e == "="));
        assert_eq!(video_encoders(ENCODERS).len(), 4);
        // An audio encoder named like a video one is not taken.
        assert_eq!(pick_h264(" ------\n A....D libx264  fake\n", || true), None);
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
        assert_eq!(c.length(), Duration::from_secs(1));
    }

    #[test]
    fn cfr_starts_at_clock_zero_and_tolerates_jitter() {
        let mut c = Cfr::new(30);
        // The first frame came 70 ms in: it covers slots 0..=2 (the sound
        // starts at 0 too).
        assert_eq!(c.repeats(ms(70)), 3);
        assert_eq!(c.repeats(ms(100 + 12)), 1, "late by a third");
        assert_eq!(c.repeats(ms(133 - 12)), 1, "early by a third");
        assert_eq!(c.sent(), 5);
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
        // Held to the end of the sound at 1 s: slots 8 and 9.
        assert_eq!(c.hold_to(ms(1000)), 2);
        assert_eq!(c.length(), Duration::from_secs(1));
        assert_eq!(c.hold_to(ms(900)), 0, "never shortened");
    }

    #[test]
    fn cfr_catch_up_after_a_stall_is_bounded_per_frame_and_keeps_time() {
        let mut c = Cfr::new(30);
        c.repeats(ms(0));
        // An hour-long stall: at most two seconds of copies per frame...
        assert_eq!(c.repeats(Duration::from_secs(3600)), MAX_REPEATS_SECS * 30);
        assert_eq!(c.repeats(Duration::from_secs(3600) + ms(33)), MAX_REPEATS_SECS * 30);
        // ...and the frames that follow make up the rest.
        let mut t = Duration::from_secs(3600) + ms(33);
        loop {
            t += ms(33);
            let k = c.repeats(t);
            assert!(k <= MAX_REPEATS_SECS * 30);
            if k <= 1 {
                break;
            }
        }
        assert!(c.length().abs_diff(t) < ms(100), "{:?} vs {t:?}", c.length());
    }

    #[test]
    fn pcm_timeline_keeps_the_pipe_contiguous_from_zero() {
        let mut p = PcmTimeline::default();
        let tone = vec![0.5f32; 960]; // 10 ms
        // Contiguous pushes from 0 pass through untouched.
        assert_eq!(p.place(&tone, ms(0)), (0, &tone[..]));
        assert_eq!(p.place(&tone, ms(10)), (0, &tone[..]));
        assert_eq!(p.length(), ms(20));
        // A push 5 ms late gets 240 silent frames first.
        assert_eq!(p.place(&tone, ms(25)).0, 240);
        assert_eq!(p.length(), ms(35));
        // An overlapping one loses its first 5 ms.
        let (gap, rest) = p.place(&tone, ms(30));
        assert_eq!((gap, rest.len()), (0, 480));
        assert_eq!(p.length(), ms(40));
        // A push that is all in the past writes nothing.
        assert_eq!(p.place(&tone, ms(0)), (0, &[][..]));
        // Padded to the picture's end.
        assert_eq!(p.pad_to(ms(50)), 480);
        assert_eq!(p.pad_to(ms(10)), 0);
        assert_eq!(p.length(), ms(50));
        // A first push that starts late (the mixer always starts at 0) is
        // put in its place.
        let mut q = PcmTimeline::default();
        assert_eq!(q.place(&tone, ms(100)).0, 4800);
    }

    fn atom(kind: &[u8; 4], body: usize) -> Vec<u8> {
        let mut v = ((body + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.resize(body + 8, 0);
        v
    }

    /// A box holding `children`.
    fn parent(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = children.concat();
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(&body);
        v
    }

    #[test]
    fn moov_check_finds_a_complete_index() {
        let check = |b: &[u8]| moov_in(std::io::Cursor::new(b), b.len() as u64);
        let ftyp = atom(b"ftyp", 16);
        let mdat = atom(b"mdat", 1000);
        let moov = parent(b"moov", &[atom(b"mvhd", 100), atom(b"trak", 60)]);
        // faststart (moov first) and moov at the end.
        assert!(check(&[ftyp.clone(), moov.clone(), mdat.clone()].concat()));
        assert!(check(&[ftyp.clone(), mdat.clone(), moov.clone()].concat()));
        // The movie header may come after the tracks.
        let later = parent(b"moov", &[atom(b"trak", 60), atom(b"mvhd", 100)]);
        assert!(check(&[ftyp.clone(), later].concat()));
        // Killed while writing: no moov, or a moov cut short.
        assert!(!check(&[ftyp.clone(), mdat.clone()].concat()));
        let cut = [ftyp.clone(), mdat.clone(), moov.clone()].concat();
        assert!(!check(&cut[..cut.len() - 10]));
        // A moov without its movie header, or with one cut short inside it.
        assert!(!check(&[ftyp.clone(), atom(b"moov", 200)].concat()));
        assert!(!check(&[ftyp.clone(), parent(b"moov", &[atom(b"trak", 60)])].concat()));
        let mut short = parent(b"moov", &[atom(b"mvhd", 100)]);
        short.truncate(short.len() - 1);
        let n = (short.len() as u32).to_be_bytes();
        short[..4].copy_from_slice(&n); // the moov's own size fits, the mvhd's does not
        assert!(!check(&[ftyp.clone(), short].concat()));
        // An mdat still open (size 0: to the end) and a 64-bit size.
        let mut open = ftyp.clone();
        open.extend_from_slice(&[0, 0, 0, 0, b'm', b'd', b'a', b't', 1, 2, 3]);
        assert!(!check(&open));
        let mut big = ftyp.clone();
        big.extend_from_slice(&[0, 0, 0, 1, b'm', b'd', b'a', b't']);
        big.extend_from_slice(&24u64.to_be_bytes());
        big.extend_from_slice(&[0; 8]);
        big.extend_from_slice(&moov);
        assert!(check(&big));
        assert!(!check(b""));
    }

    #[test]
    fn progress_waits_while_the_file_changes() {
        let t0 = Instant::now();
        let s = |t: u64| t0 + Duration::from_secs(t);
        let mut p = Progress::new(t0, Duration::from_secs(30));
        let st = |len: u64, fed: u64| -> FileState { (len, None, fed) };
        assert!(!p.stalled(s(0), st(0, 0)));
        // Growing: never stalled, however long it takes.
        for k in 1..100 {
            assert!(!p.stalled(s(k * 20), st(k, 0)));
        }
        // The writers still handing ffmpeg sound counts as progress too.
        assert!(!p.stalled(s(1990), st(99, 1)));
        // Then unchanged: stalled only after the limit.
        assert!(!p.stalled(s(2000), st(99, 1)));
        assert!(!p.stalled(s(2020), st(99, 1)));
        assert!(p.stalled(s(2021), st(99, 1)));
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_hangs_is_stopped_after_its_time() {
        let t = Instant::now();
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30");
        assert_eq!(run_bounded(cmd, Duration::from_millis(300)), Err(RunErr::Timeout));
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        let mut cmd = Command::new("/bin/echo");
        cmd.arg("V....D libx264");
        let (ok, out) = run_bounded(cmd, Duration::from_secs(5)).unwrap();
        assert!(ok);
        assert_eq!(String::from_utf8_lossy(&out).trim(), "V....D libx264");
        assert!(matches!(run_bounded(Command::new("/nonexistent/ffmpeg"), Duration::from_secs(1)), Err(RunErr::Start(_))));
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
