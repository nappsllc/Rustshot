//! Live Wayland / portal / PipeWire tests: `#[ignore]`d, run in a Wayland
//! session with xdg-desktop-portal and PipeWire (the Docker harness and the
//! `linux-wayland` CI job: headless sway with xdg-desktop-portal-wlr and no
//! chooser, so `Start` shares the output without a dialog). Each prints
//! "SKIP:" and passes when there is no Wayland display, portal or
//! libpipewire ("SKIP: [wayland]", which fails a harness run in Wayland
//! mode). Camera tests (`live_cam_linux_`) skip without a camera.

use super::pipewire::{self, ScreenSource};
use super::portal::{self, Cast};
use crate::cam::CamSource;
use crate::rec::gif::GifEncoder;
use crate::rec::{FrameSource, Quality, RecFormat, RecSpec, Session};
use std::time::{Duration, Instant};

fn portal_ready() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        println!("SKIP: [wayland] no WAYLAND_DISPLAY");
        return false;
    }
    if !pipewire::available() {
        println!("SKIP: [wayland] libpipewire-0.3 cannot be loaded");
        return false;
    }
    if !portal::screencast_available() {
        println!("SKIP: [wayland] no ScreenCast portal on the session bus");
        return false;
    }
    true
}

#[test]
#[ignore = "needs a Wayland session with the ScreenCast portal and PipeWire"]
fn live_rec_wayland_portal_frames() {
    if !portal_ready() {
        return;
    }
    let cast = Cast::start((0, 0, 64, 64), &[]).expect("start a screen cast");
    let info = cast.info.clone();
    println!("WAYLAND cast node {} pos {:?} size {:?} token {}", info.stream.node, info.stream.pos, info.stream.size, info.restore_token.is_some());
    let mut src = ScreenSource::open(cast, |(w, h)| (0, 0, w, h)).expect("stream the cast");
    let (w, h) = src.size();
    assert!(w > 0 && h > 0);
    let end = Instant::now() + Duration::from_secs(3);
    let (mut frames, mut lit) = (0, false);
    while Instant::now() < end {
        if let Some(f) = src.next(Instant::now() + Duration::from_millis(100)) {
            assert_eq!((f.w, f.h), (w, h));
            assert_eq!(f.bgra.len(), (w * h * 4) as usize);
            frames += 1;
            lit |= f.bgra.as_chunks::<4>().0.iter().any(|p| p[0] | p[1] | p[2] != 0);
        }
    }
    println!("WAYLAND frames {frames} of {w}x{h}, non-black {lit}");
    assert!(frames > 0, "no picture from the screen cast");
}

#[test]
#[ignore = "needs a Wayland session with the ScreenCast portal and PipeWire"]
fn live_rec_wayland_portal_gif_of_an_area() {
    if !portal_ready() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("rustshot-live-wayland-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let area = (10, 20, 320, 200);
    let spec = RecSpec {
        format: RecFormat::Gif,
        fps: 10,
        quality: Quality::Medium,
        area,
        system_audio: false,
        mic: None,
        out: dir.join("area.gif"),
    };
    let cast = Cast::start(area, &[]).expect("start a screen cast");
    let info = cast.info.stream.clone();
    let mut fits = false;
    let src = ScreenSource::open(cast, |frame| {
        let (c, ok) = portal::crop_for(area, portal::stream_rect(&info, frame, area, &[]), frame);
        fits = ok;
        c
    })
    .expect("stream the cast");
    let (w, h) = src.size();
    println!("WAYLAND crop {w}x{h} (fits {fits})");
    if fits {
        assert!(w >= 319 && h >= 199, "cropped to the area (scaled): {w}x{h}");
    }
    let enc = Box::new(GifEncoder::create(&spec.part_path(), w, h, 10).unwrap());
    let s = Session::start(spec, Box::new(src), Vec::new(), enc).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let st = s.stats();
    let path = s.stop().unwrap();
    let b = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    println!("WAYLAND gif {} bytes, {} frames", b.len(), st.frames);
    assert!(b.starts_with(b"GIF89a"));
    assert!(st.frames >= 10, "{} frames in 2 s at 10 fps", st.frames);
}

#[test]
#[ignore = "needs a Wayland session with the ScreenCast portal (restore tokens)"]
fn live_rec_wayland_portal_restore_token_is_kept() {
    if !portal_ready() {
        return;
    }
    // A state dir of its own: the user's stored token is not touched.
    let state = std::env::temp_dir().join(format!("rustshot-live-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    std::fs::create_dir_all(&state).unwrap();
    let old = std::env::var_os("XDG_STATE_HOME");
    // SAFETY: the live tests run one at a time (`--test-threads=1`).
    unsafe { std::env::set_var("XDG_STATE_HOME", &state) };
    struct Restore(Option<std::ffi::OsString>, std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: as above.
            unsafe {
                match &self.0 {
                    Some(v) => std::env::set_var("XDG_STATE_HOME", v),
                    None => std::env::remove_var("XDG_STATE_HOME"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    let _restore = Restore(old, state.clone());
    let first = Cast::start((0, 0, 64, 64), &[]).expect("first cast");
    let had = first.info.restore_token.clone();
    drop(first);
    let stored = portal::load_token();
    assert_eq!(stored, had, "the token Start returned is stored");
    if let Some(p) = portal::token_path(std::env::var("XDG_STATE_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())
        && had.is_some()
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{} is private", p.display());
    }
    if let Some(p) = portal::token_path(std::env::var("XDG_STATE_HOME").ok().as_deref(), None) {
        assert!(p.starts_with(&state), "{} is in the test's state dir", p.display());
    }
    let second = Cast::start((0, 0, 64, 64), &[]).expect("second cast (restored)");
    println!("WAYLAND restore token first {} second {}", had.is_some(), second.info.restore_token.is_some());
}

/// The share stops under a running recording (the portal session closes):
/// the recording ends as a failure saying so, and the file is kept and
/// plays.
#[test]
#[ignore = "needs a Wayland session with the ScreenCast portal and PipeWire"]
fn live_rec_wayland_share_stopped_ends_the_recording() {
    if !portal_ready() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("rustshot-live-wayland-stop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let area = (0, 0, 320, 200);
    let spec = RecSpec {
        format: RecFormat::Gif,
        fps: 10,
        quality: Quality::Medium,
        area,
        system_audio: false,
        mic: None,
        out: dir.join("stopped.gif"),
    };
    let cast = Cast::start(area, &[]).expect("start a screen cast");
    let info = cast.info.stream.clone();
    let src = ScreenSource::open(cast, |frame| portal::crop_for(area, portal::stream_rect(&info, frame, area, &[]), frame).0)
        .expect("stream the cast");
    let stop = src.stop_handle();
    let (w, h) = src.size();
    let enc = Box::new(GifEncoder::create(&spec.part_path(), w, h, 10).unwrap());
    let s = Session::start(spec, Box::new(src), Vec::new(), enc).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let t = Instant::now();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    while !s.stats().failed && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let noticed = t.elapsed();
    let st = s.stats();
    assert!(st.failed, "the recording did not notice the share stopping within 10 s");
    let e = format!("{:#}", s.stop().unwrap_err());
    println!("WAYLAND share stopped: noticed in {} ms, {} frames: {e}", noticed.as_millis(), st.frames);
    assert!(e.contains(portal::SHARING_STOPPED.get()), "{e}");
    assert!(e.contains("saved"), "the file is kept: {e}");
    let b = std::fs::read(dir.join("stopped.gif")).expect("the kept file");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(b.starts_with(b"GIF89a") && b.ends_with(&[0x3b]), "a whole GIF ({} bytes)", b.len());
    assert!(st.frames >= 5, "{} frames before the stop", st.frames);
}

#[test]
#[ignore = "needs a V4L2 camera"]
fn live_cam_linux_v4l2_reads_pictures() {
    let cams = super::v4l2::list();
    let Some(c) = cams.first() else {
        println!("SKIP: no V4L2 camera");
        return;
    };
    let mut cam = match super::v4l2::V4l2Cam::open(&c.id) {
        Ok(cam) => cam,
        Err(e) => {
            println!("SKIP: {} cannot be opened: {e:#}", c.id);
            return;
        }
    };
    let mut f = crate::cam::CamFrame::default();
    let end = Instant::now() + Duration::from_secs(5);
    let mut got = 0;
    while Instant::now() < end && got < 5 {
        if cam.read(&mut f).unwrap() {
            got += 1;
        }
    }
    println!("CAM {} {}x{}: {got} pictures", c.name, f.w, f.h);
    assert!(got > 0);
    assert_eq!(f.bgra.len(), (f.w * f.h * 4) as usize);
}

#[test]
#[ignore = "needs the camera portal and a camera"]
fn live_cam_linux_portal_reads_pictures() {
    let present = crate::dbus::Conn::session().ok().and_then(|mut c| portal::camera_present(&mut c).ok());
    if present != Some(true) {
        println!("SKIP: no camera behind the camera portal ({present:?})");
        return;
    }
    let mut cam = pipewire::PwCam::open(None).expect("open the portal camera");
    let mut f = crate::cam::CamFrame::default();
    let end = Instant::now() + Duration::from_secs(5);
    let mut got = 0;
    while Instant::now() < end && got < 5 {
        if cam.read(&mut f).unwrap() {
            got += 1;
        }
    }
    println!("CAM portal {}x{}: {got} pictures", f.w, f.h);
    assert!(got > 0);
}
