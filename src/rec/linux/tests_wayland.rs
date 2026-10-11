//! Live Wayland / portal / PipeWire tests: `#[ignore]`d, run in a Wayland
//! session with xdg-desktop-portal and PipeWire (the Docker harness and the
//! `linux-wayland` CI job: headless sway with xdg-desktop-portal-wlr and no
//! chooser, so `Start` shares the output without a dialog). Each prints
//! "SKIP:" and passes when there is no Wayland display, portal or
//! libpipewire. Camera tests (`live_cam_linux_`) skip without a camera.

use super::pipewire::{self, ScreenSource};
use super::portal::{self, Cast};
use crate::cam::CamSource;
use crate::rec::gif::GifEncoder;
use crate::rec::{FrameSource, Quality, RecFormat, RecSpec, Session};
use std::time::{Duration, Instant};

fn portal_ready() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        println!("SKIP: no WAYLAND_DISPLAY");
        return false;
    }
    if !pipewire::available() {
        println!("SKIP: libpipewire-0.3 cannot be loaded");
        return false;
    }
    if !portal::screencast_available() {
        println!("SKIP: no ScreenCast portal on the session bus");
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
    let second = Cast::start((0, 0, 64, 64), &[]).expect("second cast (restored)");
    println!("WAYLAND restore token first {} second {}", had.is_some(), second.info.restore_token.is_some());
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
    let mut cam = pipewire::PwCam::open().expect("open the portal camera");
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
