//! Live recording on X11: `#[ignore]`d, run by CI's Linux job under Xvfb
//! with ffmpeg installed (names start `live_rec_linux_`). CI starts a
//! PulseAudio daemon with a null sink and a null source for the test with
//! sound; without a sound server it prints "SKIP:" and passes.

use super::ffmpeg::{self, FfmpegEncoder};
use super::xshm::XShmSource;
use crate::rec::gif::GifEncoder;
use crate::rec::{FrameSource, Quality, RecFormat, RecSpec, Session, VideoEncoder};
use std::path::{Path, PathBuf};
use std::process::Command;
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
        let e = FfmpegEncoder::create(p, w, h, 30, Quality::Medium, None, None).unwrap();
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

/// System sound and microphone (mixed) through ffmpeg's pulse inputs, with
/// a pause: two segments joined into one file without the paused time.
#[test]
#[ignore = "records the X screen (Xvfb in CI) and PulseAudio with ffmpeg"]
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
    let spec = spec(RecFormat::Mp4, 30, (0, 0, 320, 240), dir.join("sound.mp4"));
    let src = XShmSource::open(spec.area).expect("open XShm");
    let (w, h) = src.size();
    let enc = FfmpegEncoder::create(&spec.part_path(), w, h, 30, Quality::Low, Some(system), mic).unwrap();
    let s = Session::start(spec, Box::new(src), Vec::new(), Box::new(enc)).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    s.pause();
    std::thread::sleep(Duration::from_millis(1000));
    s.resume();
    std::thread::sleep(Duration::from_millis(1500));
    let t = Instant::now();
    let path = s.stop().unwrap();
    eprintln!("stopped in {:?}", t.elapsed());
    let fmt = ffprobe(&path, "format=duration");
    let st = ffprobe(&path, "stream=codec_type,codec_name,start_time,duration");
    eprintln!("ffprobe:\n{fmt}{st}");
    let duration: f64 = value(&fmt, "duration").parse().unwrap();
    eprintln!("FFPROBE sound+pause duration={duration:.3}s");
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(leftovers, [std::ffi::OsString::from("sound.mp4")], "segments and lists are cleaned up");
    assert!((2.7..=3.3).contains(&duration), "duration {duration} s, want 3 ± 0.3 (the pause is cut)");
    assert!(st.contains("codec_type=video") && st.contains("codec_name=h264"), "{st}");
    assert!(st.contains("codec_type=audio") && st.contains("codec_name=aac"), "{st}");
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
