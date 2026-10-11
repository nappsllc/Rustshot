//! Live recording on X11: `#[ignore]`d, run by CI's Linux job under Xvfb
//! with ffmpeg installed (names start `live_rec_linux_`), once with the
//! distribution's ffmpeg 6 and once with a static ffmpeg 7 build
//! (`RUSTSHOT_FFMPEG`). CI starts a PulseAudio daemon with a null sink (the
//! default) and a null source for the test with sound, which plays a tone
//! into the sink while it records; without a sound server it prints
//! "SKIP:" and passes.

use super::ffmpeg::{self, FfmpegEncoder};
use super::parec::ParecSource;
use super::xshm::XShmSource;
use crate::rec::gif::GifEncoder;
use crate::rec::{AudioSource, FrameSource, Quality, RecFormat, RecSpec, Session, VideoEncoder};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn out_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rustshot-live-rec-linux-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn spec(format: RecFormat, fps: u32, area: crate::capture::IRect, out: PathBuf) -> RecSpec {
    RecSpec { format, fps, quality: Quality::Medium, area, system_audio: false, mic: None, out }
}

/// Record `spec` for `secs` with the encoder `make` builds; the saved path.
fn record(spec: RecSpec, secs: f64, make: impl FnOnce(&Path, u32, u32) -> Box<dyn VideoEncoder>) -> PathBuf {
    let src = XShmSource::open(spec.area).expect("open XShm");
    let (w, h) = src.size();
    let enc = make(&spec.part_path(), w, h);
    let s = Session::start(spec, Box::new(src), Vec::new(), enc).unwrap();
    std::thread::sleep(Duration::from_secs_f64(secs));
    let st = s.stats();
    eprintln!("stats: {st:?}");
    let t = Instant::now();
    let p = s.stop().unwrap();
    eprintln!("stopped in {:?}: {}", t.elapsed(), p.display());
    p
}

fn ffprobe(path: &Path, entries: &str) -> String {
    let exe = ffmpeg::find_ffprobe().expect("ffprobe is installed next to ffmpeg");
    let o = Command::new(exe)
        .args(["-v", "error", "-show_entries", entries, "-of", "default=noprint_wrappers=1"])
        .arg(path)
        .output()
        .unwrap();
    assert!(o.status.success(), "ffprobe failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn value<'a>(probe: &'a str, key: &str) -> &'a str {
    probe.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).unwrap_or_else(|| panic!("{key} in {probe}"))
}

#[test]
#[ignore = "records the X screen (Xvfb in CI) with ffmpeg"]
fn live_rec_linux_xvfb_mp4_with_ffmpeg() {
    let exe = ffmpeg::find().expect("ffmpeg is installed");
    let caps = ffmpeg::caps(&exe);
    eprintln!("ffmpeg {}: {caps:?}", exe.display());
    let dir = out_dir("mp4");
    // An odd size: cropped to even by the video filter.
    let spec = spec(RecFormat::Mp4, 30, (100, 80, 641, 481), dir.join("screen.mp4"));
    let path = record(spec, 2.0, |p, w, h| {
        let e = FfmpegEncoder::create(p, w, h, 30, Quality::Medium, false).unwrap();
        eprintln!("encoder: {}", e.codec().name());
        Box::new(e)
    });
    let fmt = ffprobe(&path, "format=duration,format_name");
    let st = ffprobe(&path, "stream=codec_type,codec_name,width,height,nb_frames,avg_frame_rate");
    eprintln!("ffprobe:\n{fmt}{st}");
    let duration: f64 = value(&fmt, "duration").parse().unwrap();
    eprintln!("FFPROBE duration={duration:.3}s");
    let _ = std::fs::remove_dir_all(&dir);
    assert!((1.7..=2.3).contains(&duration), "duration {duration} s, want 2 ± 0.3");
    assert!(value(&fmt, "format_name").contains("mp4"), "{fmt}");
    assert_eq!(value(&st, "codec_type"), "video", "one stream, video, no sound");
    assert!(!st.contains("codec_type=audio"));
    assert_eq!(value(&st, "codec_name"), "h264");
    assert_eq!((value(&st, "width"), value(&st, "height")), ("640", "480"));
    assert_eq!(value(&st, "avg_frame_rate"), "30/1", "constant frame rate");
}

/// A 16-bit stereo WAV of a `hz` sine at half scale, `secs` long.
fn tone_wav(path: &Path, hz: f64, secs: f64) {
    let rate = 48_000u32;
    let n = (f64::from(rate) * secs) as u32;
    let mut b = Vec::with_capacity(44 + n as usize * 4);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + n * 4).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(n * 4).to_le_bytes());
    for i in 0..n {
        let v = ((f64::from(i) * hz * std::f64::consts::TAU / f64::from(rate)).sin() * 16_000.0) as i16;
        b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

/// `mean_volume` (dB) of `path`'s sound, from ffmpeg's volumedetect.
fn mean_volume(path: &Path) -> f64 {
    let exe = ffmpeg::find().unwrap();
    let o = Command::new(exe)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-map", "0:a", "-af", "volumedetect", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&o.stderr);
    log.lines()
        .find_map(|l| l.split("mean_volume: ").nth(1)?.split_whitespace().next()?.parse().ok())
        .unwrap_or_else(|| panic!("no mean_volume in:\n{log}"))
}

/// System sound (a tone played into the null sink) and microphone (the
/// null source), read with parec, mixed by the session and fed to one
/// ffmpeg; paused once: 1.5 s + 1.5 s recorded.
#[test]
#[ignore = "records the X screen (Xvfb in CI) and PulseAudio with parec and ffmpeg"]
fn live_rec_linux_xvfb_mp4_with_sound_and_pause() {
    use super::pulse_list;
    let all = pulse_list::list();
    eprintln!("pulse sources: {all:?}");
    let system = match pulse_list::system_source(all.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP: no sound to record ({e})");
            return;
        }
    };
    let mic = pulse_list::mic_source(all.as_deref(), "").ok();
    eprintln!("system {system}, mic {mic:?}");
    let dir = out_dir("sound");
    let wav = std::env::temp_dir().join(format!("rustshot-tone-{}.wav", std::process::id()));
    tone_wav(&wav, 440.0, 20.0);
    let paplay = ffmpeg::find_tool("paplay", None).expect("paplay (pulseaudio-utils)");
    let mut player = Command::new(paplay).arg(&wav).stdin(Stdio::null()).spawn().expect("play the tone");
    std::thread::sleep(Duration::from_millis(500)); // the sink wakes up
    let mut audio: Vec<Box<dyn AudioSource>> = vec![Box::new(ParecSource::open(&system).expect("parec the monitor"))];
    if let Some(m) = &mic {
        audio.push(Box::new(ParecSource::open(m).expect("parec the mic")));
    }
    let spec = spec(RecFormat::Mp4, 30, (0, 0, 320, 240), dir.join("sound.mp4"));
    let src = XShmSource::open(spec.area).expect("open XShm");
    let (w, h) = src.size();
    let enc = FfmpegEncoder::create(&spec.part_path(), w, h, 30, Quality::Low, true).unwrap();
    let s = Session::start(spec, Box::new(src), audio, Box::new(enc)).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    s.pause();
    std::thread::sleep(Duration::from_millis(1000));
    s.resume();
    std::thread::sleep(Duration::from_millis(1500));
    let t = Instant::now();
    let path = s.stop().unwrap();
    eprintln!("stopped in {:?}", t.elapsed());
    let _ = player.kill(); // our own child, by its PID
    let _ = player.wait();
    let _ = std::fs::remove_file(&wav);
    let fmt = ffprobe(&path, "format=duration");
    let st = ffprobe(&path, "stream=codec_type,codec_name,duration");
    eprintln!("ffprobe:\n{fmt}{st}");
    let duration: f64 = value(&fmt, "duration").parse().unwrap();
    // Per stream: codec_type comes before its duration.
    let mut durs = std::collections::HashMap::new();
    let mut kind = String::new();
    for l in st.lines() {
        if let Some(k) = l.strip_prefix("codec_type=") {
            kind = k.to_string();
        } else if let Some(d) = l.strip_prefix("duration=") {
            durs.insert(kind.clone(), d.parse::<f64>().unwrap_or(f64::NAN));
        }
    }
    let vol = mean_volume(&path);
    let (v, a) = (durs.get("video").copied().unwrap_or(f64::NAN), durs.get("audio").copied().unwrap_or(f64::NAN));
    eprintln!("FFPROBE sound+pause duration={duration:.3}s video={v:.3}s audio={a:.3}s mean_volume={vol:.1}dB");
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(leftovers, [std::ffi::OsString::from("sound.mp4")], "nothing else is written");
    assert!((2.7..=3.3).contains(&duration), "duration {duration} s, want 3 ± 0.3 (the pause is cut)");
    assert!(st.contains("codec_type=video") && st.contains("codec_name=h264"), "{st}");
    assert!(st.contains("codec_type=audio") && st.contains("codec_name=aac"), "{st}");
    assert!((v - a).abs() <= 0.1, "video {v} s and audio {a} s differ by more than 0.1 s");
    assert!(vol > -60.0, "the tone is in the recording (mean_volume {vol} dB)");
}

/// Frames of a moving bar, delivered on time except once: the capture
/// stalls for `stall` after `at` (a hung X server, a busy machine).
struct StallingFrames {
    w: u32,
    h: u32,
    start: Instant,
    at: Duration,
    stall: Duration,
    stalled: bool,
    n: u32,
}

impl FrameSource for StallingFrames {
    fn next(&mut self, deadline: Instant) -> Option<crate::rec::Frame> {
        if !self.stalled && self.start.elapsed() >= self.at {
            self.stalled = true;
            eprintln!("stalling the capture for {:?}", self.stall);
            std::thread::sleep(self.stall);
        } else {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
        self.n += 1;
        let mut bgra = vec![40u8; (self.w * self.h * 4) as usize];
        let x = (self.n * 4) % self.w;
        for y in 0..self.h {
            let i = ((y * self.w + x) * 4) as usize;
            bgra[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        Some(crate::rec::Frame { w: self.w, h: self.h, bgra, ts: Duration::ZERO })
    }

    fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }
}

/// A 440 Hz tone that arrives in step with the wall clock, like a device.
struct LiveTone {
    start: Instant,
    sent: u64,
}

impl AudioSource for LiveTone {
    fn rate(&self) -> u32 {
        48_000
    }

    fn channels(&self) -> u16 {
        2
    }

    fn read(&mut self, out: &mut Vec<f32>) -> usize {
        let due = (self.start.elapsed().as_secs_f64() * 48_000.0) as u64;
        let n = due.saturating_sub(self.sent);
        for i in self.sent..self.sent + n {
            let v = ((i as f64) * 440.0 * std::f64::consts::TAU / 48_000.0).sin() as f32 * 0.4;
            out.extend_from_slice(&[v, v]);
        }
        self.sent += n;
        (n * 2) as usize
    }
}

/// ffmpeg 7's scheduler stops reading an input that runs more than 100 ms
/// ahead of the other: a capture that stalls for 5 s mid-recording must
/// neither deadlock the two pipes nor leave the sound and picture with
/// different lengths (the last frame is held while no new one comes).
#[test]
#[ignore = "records synthetic frames and sound with ffmpeg"]
fn live_rec_linux_ffmpeg_capture_stall_keeps_av_in_step() {
    let exe = ffmpeg::find().expect("ffmpeg is installed");
    let version = Command::new(&exe).arg("-version").output().map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string());
    eprintln!("ffmpeg {}: {}", exe.display(), version.unwrap_or_default());
    let dir = out_dir("stall");
    let spec = spec(RecFormat::Mp4, 30, (0, 0, 320, 240), dir.join("stall.mp4"));
    let enc = FfmpegEncoder::create(&spec.part_path(), 320, 240, 30, Quality::Low, true).unwrap();
    eprintln!("encoder: {}", enc.codec().name());
    let now = Instant::now();
    let src = StallingFrames { w: 320, h: 240, start: now, at: Duration::from_millis(1500), stall: Duration::from_secs(5), stalled: false, n: 0 };
    let audio: Vec<Box<dyn AudioSource>> = vec![Box::new(LiveTone { start: now, sent: 0 })];
    let s = Session::start(spec, Box::new(src), audio, Box::new(enc)).unwrap();
    std::thread::sleep(Duration::from_millis(8000)); // 1.5 s, the 5 s stall, 1.5 s
    let st = s.stats();
    eprintln!("stats: {st:?}");
    assert!(!st.failed, "the encoder failed while recording");
    let t = Instant::now();
    // Stop must come back (a deadlock would hang here): bounded by a thread.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(s.stop().map_err(|e| format!("{e:#}")));
    });
    let path = rx.recv_timeout(Duration::from_secs(60)).expect("stop came back within 60 s").unwrap();
    let stop_secs = t.elapsed().as_secs_f64();
    let fmt = ffprobe(&path, "format=duration");
    let st = ffprobe(&path, "stream=codec_type,duration");
    eprintln!("ffprobe:\n{fmt}{st}");
    let duration: f64 = value(&fmt, "duration").parse().unwrap();
    let mut durs = std::collections::HashMap::new();
    let mut kind = String::new();
    for l in st.lines() {
        if let Some(k) = l.strip_prefix("codec_type=") {
            kind = k.to_string();
        } else if let Some(d) = l.strip_prefix("duration=") {
            durs.insert(kind.clone(), d.parse::<f64>().unwrap_or(f64::NAN));
        }
    }
    let (v, a) = (durs.get("video").copied().unwrap_or(f64::NAN), durs.get("audio").copied().unwrap_or(f64::NAN));
    let vol = mean_volume(&path);
    eprintln!("FFPROBE stall duration={duration:.3}s video={v:.3}s audio={a:.3}s stop={stop_secs:.2}s mean_volume={vol:.1}dB");
    let _ = std::fs::remove_dir_all(&dir);
    assert!((7.4..=8.6).contains(&duration), "duration {duration} s, want 8 ± 0.6 (the stall is recorded as a still)");
    assert!((v - a).abs() <= 0.1, "video {v} s and audio {a} s differ by more than 0.1 s");
    assert!(vol > -60.0, "the tone is in the recording (mean_volume {vol} dB)");
}

#[test]
#[ignore = "records the X screen (Xvfb in CI)"]
fn live_rec_linux_xvfb_gif() {
    let dir = out_dir("gif");
    let spec = spec(RecFormat::Gif, 15, (0, 0, 320, 200), dir.join("screen.gif"));
    let path = record(spec, 1.0, |p, w, h| Box::new(GifEncoder::create(p, w, h, 15).unwrap()));
    let b = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(b.starts_with(b"GIF89a"), "a GIF");
    assert!(b.len() > 100, "{} bytes", b.len());
}

#[test]
#[ignore = "reads the X screen (Xvfb in CI)"]
fn live_rec_linux_xshm_frames() {
    let mut src = XShmSource::open((-50, -50, 300, 200)).expect("open XShm");
    assert_eq!(src.size(), (250, 150), "clipped to the screen");
    let f = src.next(Instant::now() + Duration::from_millis(100)).expect("a frame");
    assert_eq!((f.w, f.h, f.bgra.len()), (250, 150, 250 * 150 * 4));
    assert!(f.bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "opaque");
}

/// A 440 Hz tone at 0.4 for `n` stereo frames from frame `from`.
fn tone(from: u64, n: u64) -> Vec<f32> {
    (from..from + n)
        .flat_map(|i| {
            let v = ((i as f64) * 440.0 * std::f64::consts::TAU / 48_000.0).sin() as f32 * 0.4;
            [v, v]
        })
        .collect()
}

/// A `w`×`h` frame with a white bar at column `n * 8`.
fn bar_frame(w: u32, h: u32, n: u32, ts: Duration) -> crate::rec::Frame {
    let mut bgra = vec![60u8; (w * h * 4) as usize];
    let x = (n * 8) % w;
    for y in 0..h {
        for dx in 0..4 {
            let i = ((y * w + (x + dx) % w) * 4) as usize;
            bgra[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    crate::rec::Frame { w, h, bgra, ts }
}

/// Per-stream durations from `ffprobe -show_entries stream=codec_type,duration`.
fn stream_durations(st: &str) -> std::collections::HashMap<String, f64> {
    let mut durs = std::collections::HashMap::new();
    let mut kind = String::new();
    for l in st.lines() {
        if let Some(k) = l.strip_prefix("codec_type=") {
            kind = k.to_string();
        } else if let Some(d) = l.strip_prefix("duration=") {
            durs.insert(kind.clone(), d.parse::<f64>().unwrap_or(f64::NAN));
        }
    }
    durs
}

/// Decode every frame of `path` with ffmpeg: its error output (empty
/// when the file decodes cleanly).
fn decode_errors(path: &Path) -> String {
    let exe = ffmpeg::find().expect("ffmpeg is installed (to check the file)");
    let o = Command::new(exe).args(["-v", "error", "-i"]).arg(path).args(["-f", "null", "-"]).stdin(Stdio::null()).output().unwrap();
    assert!(o.status.success(), "ffmpeg could not decode {}: {}", path.display(), String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stderr).trim().to_string()
}

/// The library and libopus, downloading OpenH264 first if needed (CI sets
/// `XDG_DATA_HOME` to a temporary folder).
fn openh264_and_opus() -> (std::sync::Arc<super::openh264::OpenH264>, std::sync::Arc<super::opus::Opus>) {
    use super::openh264;
    if openh264::installed().is_none() {
        let t = Instant::now();
        let p = openh264::download(&|_, _| true).expect("download OpenH264 from Cisco");
        eprintln!("downloaded {} in {:?}", p.display(), t.elapsed());
    }
    let lib = openh264::OpenH264::load().expect("load OpenH264");
    let opus = super::opus::Opus::load().expect("load libopus");
    (lib, opus)
}

/// The real path without ffmpeg: the pinned OpenH264 from Cisco, the
/// system's libopus and the built-in muxer, 1 s of synthetic frames and a
/// tone; ffprobe must see H.264 and Opus of 1 s each, and ffmpeg must
/// decode it without an error.
#[test]
#[ignore = "downloads OpenH264 from Cisco; needs libopus and ffmpeg (to check)"]
fn live_oh264_one_second_of_frames_and_a_tone() {
    let (lib, opus) = openh264_and_opus();
    let dir = out_dir("oh264");
    let path = dir.join("oh264.mp4");
    let mut enc = super::native::NativeEncoder::create(&path, 641, 361, 30, Quality::Medium, lib, Some(opus)).unwrap();
    assert!(enc.audio());
    let mut sent = 0u64;
    for n in 0..30u32 {
        let ts = Duration::from_secs_f64(f64::from(n) / 30.0);
        enc.push_video(&bar_frame(641, 361, n, ts)).unwrap();
        // The sound in 10 ms pieces up to this frame's end.
        let until = (f64::from(n + 1) / 30.0 * 48_000.0) as u64;
        while sent < until {
            let k = 480.min(until - sent);
            enc.push_audio(&tone(sent, k), Duration::from_secs_f64(sent as f64 / 48_000.0)).unwrap();
            sent += k;
        }
    }
    Box::new(enc).finish().unwrap();
    assert!(ffmpeg::has_moov(&path));
    let fmt = ffprobe(&path, "format=duration,format_name");
    let st = ffprobe(&path, "stream=codec_type,codec_name,width,height,duration,nb_frames");
    eprintln!("ffprobe:\n{fmt}{st}");
    let durs = stream_durations(&st);
    let (v, a) = (durs.get("video").copied().unwrap_or(f64::NAN), durs.get("audio").copied().unwrap_or(f64::NAN));
    let errors = decode_errors(&path);
    let vol = mean_volume(&path);
    eprintln!("OH264 one second: video={v:.3}s audio={a:.3}s mean_volume={vol:.1}dB size={}", std::fs::metadata(&path).unwrap().len());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(st.contains("codec_name=h264") && st.contains("codec_name=opus"), "{st}");
    assert_eq!((value(&st, "width"), value(&st, "height")), ("640", "360"), "odd sizes cropped to even");
    assert!((v - 1.0).abs() <= 0.1, "video {v} s, want 1 ± 0.1");
    assert!((a - 1.0).abs() <= 0.1, "audio {a} s, want 1 ± 0.1");
    assert!(errors.is_empty(), "decode errors: {errors}");
    assert!(vol > -30.0, "the tone is in the recording (mean_volume {vol} dB)");
}

/// The same encoder under a session: frames paced by the capture loop
/// and a live tone mixed by the session, 2 s with a pause; the picture and
/// the sound must come out the same length.
#[test]
#[ignore = "downloads OpenH264 from Cisco; needs libopus and ffmpeg (to check)"]
fn live_oh264_session_keeps_picture_and_sound_in_step() {
    let (lib, opus) = openh264_and_opus();
    let dir = out_dir("oh264-session");
    let spec = spec(RecFormat::Mp4, 30, (0, 0, 320, 240), dir.join("session.mp4"));
    let enc = super::native::NativeEncoder::create(&spec.part_path(), 320, 240, 30, Quality::Low, lib, Some(opus)).unwrap();
    let now = Instant::now();
    let src = StallingFrames { w: 320, h: 240, start: now, at: Duration::from_secs(3600), stall: Duration::ZERO, stalled: false, n: 0 };
    let audio: Vec<Box<dyn AudioSource>> = vec![Box::new(LiveTone { start: now, sent: 0 })];
    let s = Session::start(spec, Box::new(src), audio, Box::new(enc)).unwrap();
    std::thread::sleep(Duration::from_millis(1000));
    s.pause();
    std::thread::sleep(Duration::from_millis(500));
    s.resume();
    std::thread::sleep(Duration::from_millis(1000));
    let path = s.stop().unwrap();
    let fmt = ffprobe(&path, "format=duration");
    let st = ffprobe(&path, "stream=codec_type,codec_name,duration");
    eprintln!("ffprobe:\n{fmt}{st}");
    let durs = stream_durations(&st);
    let (v, a) = (durs.get("video").copied().unwrap_or(f64::NAN), durs.get("audio").copied().unwrap_or(f64::NAN));
    let errors = decode_errors(&path);
    eprintln!("OH264 session: video={v:.3}s audio={a:.3}s");
    let _ = std::fs::remove_dir_all(&dir);
    assert!((1.7..=2.3).contains(&v), "video {v} s, want 2 ± 0.3 (the pause is cut)");
    assert!((v - a).abs() <= 0.1, "video {v} s and audio {a} s differ by more than 0.1 s");
    assert!(errors.is_empty(), "decode errors: {errors}");
}

/// Every pinned OpenH264 (x86_64 and aarch64) as Cisco serves it today:
/// the download's and the unpacked library's SHA-256 match the pins.
#[test]
#[ignore = "downloads every pinned OpenH264 from Cisco"]
fn live_oh264_pins_match_ciscos_files() {
    use super::openh264::{unpack_bzip2, verify_and_unpack};
    let dir = out_dir("pins");
    for pin in crate::rec::openh264_pins::PINS {
        let bz2 = dir.join(format!("{}.download", pin.file));
        crate::export::download_http_exact(&pin.url(), &bz2, pin.bz2_size, &|_, _| true).expect("download");
        let got = crate::sha256::hex(&crate::sha256::file_digest(&bz2).unwrap());
        eprintln!("OH264 pin {}: sha256 {got} (pinned {})", pin.file, pin.bz2_sha256);
        verify_and_unpack(&bz2, &pin, &dir.join(pin.so), unpack_bzip2).expect("matches the pin");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// libpulse-simple reads the null sink's monitor while a tone plays into
/// it: sound comes, with capture times close to now.
#[test]
#[ignore = "records PulseAudio through libpulse-simple"]
fn live_rec_linux_pulse_simple_hears_the_tone() {
    use super::pulse::{self, Sound};
    assert!(pulse::available(), "libpulse-simple.so.0 is installed");
    let all = super::pulse_list::list();
    let Ok(system) = super::pulse_list::system_source(all.as_deref()) else {
        eprintln!("SKIP: no sound server");
        return;
    };
    let wav = std::env::temp_dir().join(format!("rustshot-tone-simple-{}.wav", std::process::id()));
    tone_wav(&wav, 440.0, 5.0);
    let paplay = ffmpeg::find_tool("paplay", None).expect("paplay (pulseaudio-utils)");
    let mut player = Command::new(paplay).arg(&wav).stdin(Stdio::null()).spawn().expect("play the tone");
    std::thread::sleep(Duration::from_millis(300));
    let mut s = Sound::open(&system).expect("open the monitor");
    assert_eq!(s.via(), "libpulse-simple");
    let mut got = Vec::new();
    let mut stamps = Vec::new();
    let end = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < end {
        let (n, at) = s.read_timed(&mut got);
        if n > 0 {
            stamps.push((Instant::now(), at));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(s);
    let _ = player.kill(); // our own child, by its PID
    let _ = player.wait();
    let _ = std::fs::remove_file(&wav);
    let rms = (got.iter().map(|v| v * v).sum::<f32>() / got.len().max(1) as f32).sqrt();
    eprintln!("PULSE simple: {} samples, rms {rms:.3}, {} reads", got.len(), stamps.len());
    assert!(got.len() > 48_000, "about 1.5 s of stereo sound, got {}", got.len());
    assert!(rms > 0.05, "the tone is heard (rms {rms})");
    for (now, at) in stamps {
        let at = at.expect("a capture time");
        assert!(at <= now && now - at < Duration::from_millis(500), "capture time {:?} before the read", now - at);
    }
}
