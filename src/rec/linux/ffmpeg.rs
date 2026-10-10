//! MP4 through `ffmpeg` subprocesses (Linux, when ffmpeg is installed).
//!
//! Rustshot writes raw BGRA frames to ffmpeg's stdin at a constant frame
//! rate (`-f rawvideo -pix_fmt bgra -s WxH -r FPS -i -`; frames are
//! duplicated or dropped to keep the rate, see [`Cfr`]) and ffmpeg writes
//! H.264 (libx264, else libopenh264, else h264_vaapi, whichever `ffmpeg
//! -encoders` lists), `-movflags +faststart`, to the `.part` file. Frames
//! go to ffmpeg on a writer thread through a bounded channel.
//!
//! The sound is recorded by ffmpeg too (`-f pulse -i <source>`), but by a
//! process of its own per source, never by the video's: a live input in
//! the video's process paces the picture by the sound (ffmpeg works on the
//! stream that is behind), so a source that is slow to open (a suspended
//! sink can take a second or two to wake) holds the picture back and
//! `-shortest` cuts it. Each sound process writes FLAC and logs the wall
//! clock time of its first sample (`start:` in its input dump); the time
//! of the stretch's first frame is known too. [`FfmpegEncoder::finish`]
//! then puts the sound where it belongs (`adelay` / `atrim`), mixes the
//! sources (`amix`), joins the stretches and muxes it with the picture
//! (`-c:v copy`, AAC). Sound is only heard while recording: a stretch has
//! no sound until its source is open.
//!
//! With sound, every stretch between pauses has its own processes (they
//! start at its first frame), as ffmpeg hears in real time and a pause
//! cannot skip time the way the session clock does. Without sound there is
//! one video process writing the `.part` directly, and pausing needs
//! nothing.
//!
//! Nothing goes through a shell: every argument is its own argv entry.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::rec::{bitrate, mp4_fit, Frame, Quality, VideoEncoder};
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

/// Absolute path of the ffmpeg to use instead of the one on `PATH`.
pub const ENV_OVERRIDE: &str = "RUSTSHOT_FFMPEG";

/// The VA-API render node tried for `h264_vaapi`.
pub const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

/// Frames waiting for the writer thread (bounded: a slow ffmpeg blocks the
/// encode thread, and the session's queue drops frames before it).
const PIPE_FRAMES: usize = 2;

/// How long finishing waits for ffmpeg to finish a file.
const FINISH_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a sound process gets to stop after `q`.
const AUDIO_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// The stderr kept for error messages (the last bytes).
const TAIL_BYTES: usize = 4096;

/// The AAC bit rate of the recording's sound.
const AAC_RATE: &str = "160k";

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

/// The picture's ffmpeg run.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Input frame size.
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub quality: Quality,
    pub codec: Codec,
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
        a.extend(os(&["-thread_queue_size", "64", "-f", "rawvideo", "-pix_fmt", "bgra", "-s", &size, "-r", &fps, "-i", "-"]));
        a.extend(os(&["-map", "0:v"]));
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
        a.extend(os(&["-movflags", "+faststart", "-f", "mp4"]));
        a.push(self.out.clone().into_os_string());
        a
    }
}

/// The command line of a sound process: PulseAudio source `src` to FLAC at
/// `out`. Logged at `info`, for the input's `start:` time; no `-nostdin`:
/// `q` on stdin ends it.
pub fn audio_args(src: &str, out: &Path) -> Vec<OsString> {
    let mut a = os(&["-hide_banner", "-nostats", "-loglevel", "info", "-y"]);
    a.extend(os(&["-thread_queue_size", "1024", "-probesize", "32", "-analyzeduration", "0", "-f", "pulse", "-i", src]));
    a.extend(os(&["-map", "0:a", "-c:a", "flac", "-f", "flac"]));
    a.push(out.as_os_str().to_owned());
    a
}

/// The wall-clock time (seconds since the epoch) of the first sample, from
/// the `Duration: N/A, start: 1791669391.947440, bitrate: …` line of a
/// sound process's input dump.
pub fn parse_start(log: &str) -> Option<f64> {
    log.lines()
        .filter(|l| l.trim_start().starts_with("Duration:"))
        .find_map(|l| l.split("start: ").nth(1)?.split(',').next()?.trim().parse().ok())
}

/// A finished stretch: its picture and length, the wall-clock time of its
/// first frame, and per sound source the FLAC file and the wall-clock time
/// of its first sample (`None`: the source never opened).
#[derive(Clone, Debug, PartialEq)]
pub struct Stretch {
    pub video: PathBuf,
    pub len: Duration,
    pub origin: f64,
    pub sound: Vec<(PathBuf, Option<f64>)>,
}

/// The filter graph that lays each stretch's sound under its picture:
/// every source delayed (`adelay`) or trimmed (`atrim`) to its first
/// frame and cut or padded to its length, sources mixed (`amix`), silence
/// for a stretch without sound, stretches joined (`concat`) into `[a]`.
/// Inputs: 0 is the picture, then every present source in order.
pub fn mux_filter(stretches: &[Stretch]) -> String {
    let mut chains = Vec::new();
    let mut k = 1;
    for (i, st) in stretches.iter().enumerate() {
        let len = st.len.as_secs_f64();
        let mut parts = Vec::new();
        for (j, (_, start)) in st.sound.iter().enumerate() {
            let Some(start) = start else { continue };
            let off = start - st.origin;
            let place = if off >= 0.0 {
                let ms = (off * 1000.0).round() as u64;
                format!("adelay={ms}|{ms}")
            } else {
                format!("atrim=start={:.6},asetpts=PTS-STARTPTS", -off)
            };
            let label = format!("s{i}m{j}");
            chains.push(format!(
                "[{k}:a]aformat=sample_rates=48000:channel_layouts=stereo,{place},apad,atrim=duration={len:.6}[{label}]"
            ));
            parts.push(label);
            k += 1;
        }
        let out = format!("s{i}");
        match parts.len() {
            0 => chains.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={len:.6}[{out}]")),
            1 => chains.push(format!("[{}]anull[{out}]", parts[0])),
            n => {
                let ins: String = parts.iter().map(|p| format!("[{p}]")).collect();
                chains.push(format!("{ins}amix=inputs={n}:duration=longest:dropout_transition=0[{out}]"));
            }
        }
    }
    let ins: String = (0..stretches.len()).map(|i| format!("[s{i}]")).collect();
    chains.push(format!("{ins}concat=n={}:v=0:a=1[a]", stretches.len()));
    chains.join(";")
}

/// The command line that muxes the joined pictures (`vlist`, a concat
/// list) with the stretches' sound into `out`.
pub fn mux_args(vlist: &Path, stretches: &[Stretch], out: &Path) -> Vec<OsString> {
    let mut a = os(&HEAD);
    a.extend(os(&["-f", "concat", "-safe", "0", "-i"]));
    a.push(vlist.as_os_str().to_owned());
    for st in stretches {
        for (p, start) in &st.sound {
            if start.is_some() {
                a.push("-i".into());
                a.push(p.as_os_str().to_owned());
            }
        }
    }
    a.extend(os(&["-filter_complex", &mux_filter(stretches), "-map", "0:v", "-map", "[a]"]));
    a.extend(os(&["-c:v", "copy", "-c:a", "aac", "-b:a", AAC_RATE, "-movflags", "+faststart", "-f", "mp4"]));
    a.push(out.as_os_str().to_owned());
    a
}

/// A concat-demuxer list of `files` (`file '<path>'` lines; a `'` in a
/// path is written `'\''`).
pub fn concat_list(files: &[&Path]) -> String {
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

    /// The length of what was written.
    pub fn length(&self) -> Duration {
        Duration::from_secs_f64(self.sent as f64 / f64::from(self.fps))
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
// Processes
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

fn tail_text(tail: &Mutex<Tail>) -> String {
    let t = tail.lock().unwrap_or_else(|e| e.into_inner()).text();
    if t.is_empty() { "no output".into() } else { t }
}

/// RUSTSHOT_FFMPEG_LOG set: ffmpeg's command lines and logs are printed.
fn verbose() -> bool {
    std::env::var_os("RUSTSHOT_FFMPEG_LOG").is_some()
}

/// Start `exe args` with stdin piped and stderr read into a tail (and,
/// with `head`, its first lines kept whole: the input dump).
/// A started ffmpeg: the child, its stdin, its stderr tail and reader.
type Spawned = (Child, ChildStdin, Arc<Mutex<Tail>>, Option<JoinHandle<()>>);

fn spawn(exe: &Path, args: &[OsString], head: Option<Arc<Mutex<String>>>) -> Result<Spawned> {
    if verbose() {
        eprintln!("rustshot: {} {}", exe.display(), args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    }
    let mut child = Command::new(exe)
        .args(args)
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
                    if let Some(h) = &head {
                        let mut h = h.lock().unwrap_or_else(|e| e.into_inner());
                        if h.len() < 8192 {
                            h.push_str(&String::from_utf8_lossy(&buf[..n]));
                        }
                    }
                }
            })
            .ok()
    });
    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            bail!("ffmpeg has no stdin");
        }
    };
    Ok((child, stdin, tail, reader))
}

/// Wait up to `t` for `c`; killed after that.
fn wait_timeout(c: &mut Child, t: Duration) -> Result<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(s) = c.try_wait()? {
            return Ok(s);
        }
        if start.elapsed() >= t {
            let _ = c.kill();
            let _ = c.wait();
            bail!("ffmpeg did not finish within {} s", t.as_secs());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The picture's ffmpeg: the child, its frame writer and its stderr reader.
struct VideoRun {
    child: Child,
    tx: Option<SyncSender<Arc<Vec<u8>>>>,
    writer: Option<JoinHandle<std::io::Result<()>>>,
    reader: Option<JoinHandle<()>>,
    tail: Arc<Mutex<Tail>>,
    cfr: Cfr,
    out: PathBuf,
    /// The last frame written (held up to the end of a stretch).
    last: Option<Arc<Vec<u8>>>,
}

impl VideoRun {
    fn start(exe: &Path, plan: &Plan) -> Result<VideoRun> {
        let (mut child, stdin, tail, reader) = spawn(exe, &plan.args(), None)?;
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
        Ok(VideoRun {
            child,
            tx: Some(tx),
            writer: Some(writer),
            reader,
            tail,
            cfr: Cfr::new(plan.fps),
            out: plan.out.clone(),
            last: None,
        })
    }

    /// Write `f` as many times as the frame rate wants.
    fn push(&mut self, f: &Frame) -> Result<()> {
        let n = self.cfr.repeats(f.ts);
        if n == 0 {
            return Ok(());
        }
        let buf = Arc::new(f.bgra[..(f.w * f.h * 4) as usize].to_vec());
        self.last = Some(buf.clone());
        self.write(buf, n)
    }

    /// Hold the last frame up to clock time `ts` (the end of a stretch:
    /// frames dropped while ffmpeg was slow to take them leave the picture
    /// still rather than the stretch short).
    fn hold_to(&mut self, ts: Duration) -> Result<()> {
        let Some(buf) = self.last.clone() else { return Ok(()) };
        let n = self.cfr.repeats(ts);
        self.write(buf, n)
    }

    fn write(&mut self, buf: Arc<Vec<u8>>, n: u64) -> Result<()> {
        let tx = self.tx.as_ref().ok_or_else(|| anyhow!("ffmpeg input closed"))?;
        for _ in 0..n {
            if tx.send(buf.clone()).is_err() {
                // The writer stopped: ffmpeg is gone or stopped reading.
                let why = match self.writer.take().map(|w| w.join()) {
                    Some(Ok(Err(e))) => format!("{e}"),
                    _ => "stopped".into(),
                };
                let _ = wait_timeout(&mut self.child, Duration::from_secs(2));
                bail!("ffmpeg stopped taking frames ({why}): {}", tail_text(&self.tail));
            }
        }
        Ok(())
    }

    /// Close the input and wait for ffmpeg to finish the file: its path and
    /// length, or `None` when no frame was written (a stretch paused before
    /// its first frame; ffmpeg is stopped and its file removed).
    fn finish(mut self) -> Result<Option<(PathBuf, Duration)>> {
        drop(self.tx.take());
        let wrote = self.writer.take().map(|w| w.join());
        if self.cfr.sent() == 0 {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_file(&self.out);
            return Ok(None);
        }
        let status = wait_timeout(&mut self.child, FINISH_TIMEOUT);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        let status = status?;
        if verbose() {
            eprintln!("rustshot: ffmpeg (picture) ended ({status}):\n{}", tail_text(&self.tail));
        }
        if !status.success() {
            bail!("ffmpeg failed ({status}): {}", tail_text(&self.tail));
        }
        if let Some(Ok(Err(e))) = wrote {
            bail!("writing frames to ffmpeg failed: {e}: {}", tail_text(&self.tail));
        }
        Ok(Some((self.out.clone(), self.cfr.length())))
    }
}

impl Drop for VideoRun {
    fn drop(&mut self) {
        // Not finished (an error, or dropped): stop ffmpeg.
        drop(self.tx.take());
        if self.writer.is_some() || self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// A sound process: one PulseAudio source to FLAC.
struct AudioRun {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: Option<JoinHandle<()>>,
    tail: Arc<Mutex<Tail>>,
    head: Arc<Mutex<String>>,
    out: PathBuf,
}

impl AudioRun {
    fn start(exe: &Path, src: &str, out: PathBuf) -> Result<AudioRun> {
        let head = Arc::new(Mutex::new(String::new()));
        let (child, stdin, tail, reader) = spawn(exe, &audio_args(src, &out), Some(head.clone()))?;
        Ok(AudioRun { child, stdin: Some(stdin), reader, tail, head, out })
    }

    /// Stop it (`q`) and wait: the file and the wall-clock time of its
    /// first sample (`None` when the source never opened; it is said).
    fn finish(mut self) -> (PathBuf, Option<f64>) {
        if let Some(mut s) = self.stdin.take() {
            let _ = s.write_all(b"q");
        }
        let status = wait_timeout(&mut self.child, AUDIO_STOP_TIMEOUT);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        let start = parse_start(&self.head.lock().unwrap_or_else(|e| e.into_inner()));
        let ok = matches!(&status, Ok(s) if s.success());
        if verbose() || !ok || start.is_none() {
            eprintln!("rustshot: ffmpeg (sound) ended ({status:?}), start {start:?}:\n{}", tail_text(&self.tail));
        }
        let have = ok && start.is_some() && std::fs::metadata(&self.out).is_ok_and(|m| m.len() > 0);
        (self.out.clone(), start.filter(|_| have))
    }
}

impl Drop for AudioRun {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn wall_now() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

// ---------------------------------------------------------------------------
// The encoder
// ---------------------------------------------------------------------------

/// The running stretch (with sound).
struct Current {
    video: VideoRun,
    origin: f64,
    sound: Vec<AudioRun>,
}

/// MP4 through ffmpeg; see the module docs.
pub struct FfmpegEncoder {
    exe: PathBuf,
    plan: Plan,
    /// PulseAudio sources ffmpeg records (system monitor, microphone).
    sources: Vec<String>,
    /// The file the session renames (`.part`).
    part: PathBuf,
    /// Without sound: the one video run.
    run: Option<VideoRun>,
    /// With sound: the stretch being recorded, and the finished ones.
    cur: Option<Current>,
    done: Vec<Stretch>,
    /// After a pause: frames stamped up to here belong to the stretch
    /// before it, which is finished.
    skip_until: Option<Duration>,
    /// A run failed; `finish` reports it.
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
        let sources: Vec<String> = [system_monitor, mic].into_iter().flatten().collect();
        if !sources.is_empty() && !caps.pulse {
            bail!("{} cannot record PulseAudio sound", exe.display());
        }
        if !sources.is_empty() && !caps.aac {
            bail!("{} has no AAC encoder", exe.display());
        }
        let plan = Plan { w, h, fps: fps.clamp(1, 60), quality, codec, out: path.to_path_buf() };
        Ok(FfmpegEncoder {
            exe,
            plan,
            sources,
            part: path.to_path_buf(),
            run: None,
            cur: None,
            done: Vec::new(),
            skip_until: None,
            failed: None,
        })
    }

    /// The H.264 encoder ffmpeg uses.
    pub fn codec(&self) -> Codec {
        self.plan.codec
    }

    fn sound(&self) -> bool {
        !self.sources.is_empty()
    }

    /// `<part>.<suffix>`.
    fn beside(&self, suffix: &str) -> PathBuf {
        let mut s = self.part.as_os_str().to_os_string();
        s.push(suffix);
        PathBuf::from(s)
    }

    /// Start a stretch with sound: the picture and one process per source.
    fn start_stretch(&mut self) -> Result<()> {
        let i = self.done.len();
        let mut plan = self.plan.clone();
        plan.out = self.beside(&format!(".s{i}.mp4"));
        let video = VideoRun::start(&self.exe, &plan)?;
        let origin = wall_now();
        let mut sound = Vec::new();
        for (j, src) in self.sources.iter().enumerate() {
            match AudioRun::start(&self.exe, src, self.beside(&format!(".s{i}.a{j}.flac"))) {
                Ok(a) => sound.push(a),
                Err(e) => eprintln!("rustshot: {src} not recorded: {e:#}"),
            }
        }
        self.cur = Some(Current { video, origin, sound });
        Ok(())
    }

    /// Finish the running stretch (with sound).
    fn end_stretch(&mut self) -> Result<()> {
        let Some(c) = self.cur.take() else { return Ok(()) };
        let video = c.video.finish();
        let sound: Vec<(PathBuf, Option<f64>)> = c.sound.into_iter().map(AudioRun::finish).collect();
        match video {
            Ok(Some((video, len))) => {
                self.done.push(Stretch { video, len, origin: c.origin, sound });
                Ok(())
            }
            Ok(None) => {
                for (p, _) in sound {
                    let _ = std::fs::remove_file(p);
                }
                Ok(())
            }
            Err(e) => {
                for (p, _) in sound {
                    let _ = std::fs::remove_file(p);
                }
                self.failed = Some(format!("{e:#}"));
                Err(e)
            }
        }
    }

    fn cleanup(&mut self) {
        for st in self.done.drain(..) {
            let _ = std::fs::remove_file(&st.video);
            for (p, _) in st.sound {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    /// Join the stretches and their sound into the `.part`.
    fn mux(&self) -> Result<()> {
        let vlist = self.beside(".concat.txt");
        let videos: Vec<&Path> = self.done.iter().map(|s| s.video.as_path()).collect();
        let r = (|| -> Result<()> {
            std::fs::write(&vlist, concat_list(&videos)).context("write the stretch list")?;
            let args = mux_args(&vlist, &self.done, &self.part);
            if verbose() {
                eprintln!("rustshot: {} {}", self.exe.display(), args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
            }
            let out = Command::new(&self.exe)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .context("start ffmpeg to put the sound in")?;
            if !out.status.success() {
                let mut t = Tail::default();
                t.push(&out.stderr);
                bail!("putting the sound in failed ({}): {}", out.status, t.text());
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(&vlist);
        r
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
        let r = (|| -> Result<()> {
            if !self.sound() {
                if self.run.is_none() {
                    self.run = Some(VideoRun::start(&self.exe, &self.plan)?);
                }
                return self.run.as_mut().map_or(Ok(()), |r| r.push(f));
            }
            if self.cur.is_none() {
                self.start_stretch()?;
            }
            self.cur.as_mut().map_or(Ok(()), |c| c.video.push(f))
        })();
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
        // What ffmpeg hears from now on is not part of this stretch: end it
        // (its picture lasting up to the pause); the next one starts at the
        // first frame after the pause.
        self.skip_until = Some(at);
        if let Some(c) = self.cur.as_mut()
            && let Err(e) = c.video.hold_to(at)
        {
            self.failed = Some(format!("{e:#}"));
            return Err(e);
        }
        self.end_stretch()
    }

    /// `Ok` once a playable file is written, even after a failure part-way
    /// (the session reports that one): with sound, the stretches finished
    /// before it are kept.
    fn finish(mut self: Box<Self>) -> Result<()> {
        if !self.sound() {
            let failed = self.failed.take();
            return match self.run.take().map(VideoRun::finish) {
                Some(Ok(Some(_))) => Ok(()),
                Some(Err(e)) => Err(e),
                Some(Ok(None)) | None => Err(anyhow!("{}", failed.unwrap_or_else(|| "no frames were recorded".into()))),
            };
        }
        let ended = self.end_stretch();
        let failed = self.failed.take();
        let r = if self.done.is_empty() {
            Err(match (failed, ended) {
                (Some(e), _) => anyhow!("{e}"),
                (None, Err(e)) => e,
                (None, Ok(())) => anyhow!("no frames were recorded"),
            })
        } else {
            self.mux()
        };
        self.cleanup();
        r
    }

    fn audio(&self) -> bool {
        false
    }
}

impl Drop for FfmpegEncoder {
    /// Dropped without finishing (cancelled): the runs stop, their files go.
    fn drop(&mut self) {
        if let Some(c) = self.cur.take() {
            let video = c.video.out.clone();
            drop(c.video);
            let _ = std::fs::remove_file(video);
            for a in c.sound {
                let out = a.out.clone();
                drop(a);
                let _ = std::fs::remove_file(out);
            }
        }
        self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(codec: Codec) -> Plan {
        Plan { w: 1280, h: 720, fps: 30, quality: Quality::Medium, codec, out: PathBuf::from("/tmp/rec dir/it's.mp4.part") }
    }

    fn joined(a: &[OsString]) -> String {
        a.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")
    }

    const HEAD_S: &str = "-hide_banner -nostdin -nostats -loglevel warning -y";
    const VIDEO_IN: &str = "-thread_queue_size 64 -f rawvideo -pix_fmt bgra -s 1280x720 -r 30 -i -";
    const X264: &str = "-c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p";
    const TAIL: &str = "-movflags +faststart -f mp4 /tmp/rec dir/it's.mp4.part";

    #[test]
    fn video_args() {
        assert_eq!(joined(&plan(Codec::X264).args()), format!("{HEAD_S} {VIDEO_IN} -map 0:v {X264} {TAIL}"));
    }

    #[test]
    fn sound_args_per_source() {
        for src in ["@DEFAULT_MONITOR@", "alsa_input.usb-mic", "weird name; rm -rf ~"] {
            let a = audio_args(src, Path::new("/tmp/a dir/x.s0.a1.flac"));
            assert_eq!(
                joined(&a),
                format!(
                    "-hide_banner -nostats -loglevel info -y -thread_queue_size 1024 -probesize 32 -analyzeduration 0 \
                     -f pulse -i {src} -map 0:a -c:a flac -f flac /tmp/a dir/x.s0.a1.flac"
                )
            );
            // Device names and paths stay one argv entry each; no -nostdin (q stops it).
            assert!(a.iter().any(|x| x == src));
            assert!(!a.iter().any(|x| x == "-nostdin"));
        }
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
        assert!(s.starts_with(&format!("{HEAD_S} -vaapi_device /dev/dri/renderD128 -thread_queue_size 64")), "{s}");
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
    fn sound_start_from_the_input_dump() {
        let log = "Input #0, pulse, from '@DEFAULT_MONITOR@':\n  Duration: N/A, start: 1791669391.947440, bitrate: 1536 kb/s\n  \
                   Stream #0:0: Audio: pcm_s16le, 48000 Hz, 2 channels, s16, 1536 kb/s\n";
        assert_eq!(parse_start(log), Some(1_791_669_391.947_44));
        assert_eq!(parse_start("[pulse @ 0x5] pa_stream_get_latency() failed\n"), None);
        assert_eq!(parse_start(""), None);
    }

    fn st(len_ms: u64, origin: f64, sound: &[Option<f64>]) -> Stretch {
        Stretch {
            video: PathBuf::from("v.mp4"),
            len: Duration::from_millis(len_ms),
            origin,
            sound: sound.iter().enumerate().map(|(j, s)| (PathBuf::from(format!("a{j}.flac")), *s)).collect(),
        }
    }

    const FMT: &str = "aformat=sample_rates=48000:channel_layouts=stereo";

    #[test]
    fn mux_one_stretch_one_source_late() {
        // The source opened 250 ms after the first frame: delayed by that.
        let f = mux_filter(&[st(2000, 100.0, &[Some(100.25)])]);
        assert_eq!(f, format!("[1:a]{FMT},adelay=250|250,apad,atrim=duration=2.000000[s0m0];[s0m0]anull[s0];[s0]concat=n=1:v=0:a=1[a]"));
    }

    #[test]
    fn mux_early_sound_is_trimmed() {
        let f = mux_filter(&[st(1500, 100.0, &[Some(99.5)])]);
        assert!(f.starts_with(&format!("[1:a]{FMT},atrim=start=0.500000,asetpts=PTS-STARTPTS,apad,atrim=duration=1.500000[s0m0]")), "{f}");
    }

    #[test]
    fn mux_both_sources_are_mixed_and_stretches_joined() {
        let f = mux_filter(&[st(1500, 10.0, &[Some(11.3), Some(12.0)]), st(1000, 20.0, &[Some(20.1), None])]);
        let want = [
            format!("[1:a]{FMT},adelay=1300|1300,apad,atrim=duration=1.500000[s0m0]"),
            format!("[2:a]{FMT},adelay=2000|2000,apad,atrim=duration=1.500000[s0m1]"),
            "[s0m0][s0m1]amix=inputs=2:duration=longest:dropout_transition=0[s0]".to_string(),
            format!("[3:a]{FMT},adelay=100|100,apad,atrim=duration=1.000000[s1m0]"),
            "[s1m0]anull[s1]".to_string(),
            "[s0][s1]concat=n=2:v=0:a=1[a]".to_string(),
        ];
        assert_eq!(f, want.join(";"));
    }

    #[test]
    fn mux_a_stretch_without_sound_is_silent() {
        let f = mux_filter(&[st(500, 1.0, &[None, None]), st(700, 2.0, &[Some(2.0)])]);
        assert!(f.starts_with("anullsrc=r=48000:cl=stereo,atrim=duration=0.500000[s0];[1:a]"), "{f}");
        assert!(f.ends_with("[s0][s1]concat=n=2:v=0:a=1[a]"), "{f}");
    }

    #[test]
    fn mux_args_take_the_present_sources() {
        let sts = [st(1500, 10.0, &[Some(11.3), None]), st(1000, 20.0, &[Some(20.1), Some(20.2)])];
        let a = mux_args(Path::new("/t/list.txt"), &sts, Path::new("/t/out.mp4.part"));
        let s = joined(&a);
        assert!(s.starts_with(&format!("{HEAD_S} -f concat -safe 0 -i /t/list.txt -i a0.flac -i a0.flac -i a1.flac -filter_complex ")), "{s}");
        assert!(s.ends_with("-map 0:v -map [a] -c:v copy -c:a aac -b:a 160k -movflags +faststart -f mp4 /t/out.mp4.part"), "{s}");
        // The filter graph is one argv entry.
        assert!(a.iter().any(|x| x.to_string_lossy().starts_with("[1:a]aformat")));
    }

    #[test]
    fn concat_list_quotes_paths() {
        let l = concat_list(&[Path::new("/a/b.s0.mp4"), Path::new("/a/it's.s1.mp4")]);
        assert_eq!(l, "file '/a/b.s0.mp4'\nfile '/a/it'\\''s.s1.mp4'\n");
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
        assert_eq!(c.length(), Duration::from_secs(1));
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
        // Held to the end of a stretch (the pause at 1 s): slots 8..=10.
        assert_eq!(c.repeats(ms(1000)), 3);
        assert_eq!(c.length(), Duration::from_millis(1100));
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
