//! Live: the recording frame (border, control bar, countdown) is on
//! screen but never in the recording. `#[ignore]`d, run by CI's desktop
//! job (`live_rec_`).

use super::*;
use crate::rec::win::tests::{out_dir, record, skip_without_mf, spec, TestWindow};
use crate::rec::{FrameSource, Quality};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Pump this thread's messages for `ms` (the windows live here).
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut m = MSG::default();
        while Instant::now() < end {
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn visible(l: &Layer) -> bool {
    unsafe { IsWindowVisible(l.hwnd).as_bool() }
}

/// The frame drawn over a white window inside the recorded area: the
/// recording shows only the window (`WDA_EXCLUDEFROMCAPTURE`).
#[test]
#[ignore = "records the real desktop"]
fn live_rec_border_and_bar_are_not_recorded() {
    crate::capture::enable_dpi_awareness();
    let win = TestWindow::open(300, 300, 360, 240, 1_000_000);
    win.set_colour(3); // white
    let area = win.rect();
    // The border hugs a smaller rect, so its strips lie inside the area;
    // with no room outside, the bar and the countdown go inside too.
    let inner = (area.0 + 40, area.1 + 40, area.2 - 80, area.3 - 80);
    let border = Border::show(inner, &[(area, 4)]).unwrap();
    let geom = Geom { area, monitors: vec![(area, 1.0)], scale: 1.0, mult: 1.0 };
    let mut bar = BarWin::new(0).unwrap();
    let v = BarView { elapsed: Duration::from_secs(5), paused: false, mic: Some(false), cam: None, system: None, stopping: false, compact: false, asking: false };
    bar.show(&geom, &crate::theme::DARK, &v);
    let mut count = CountWin { layer: Layer::new(true).unwrap(), drawn: None };
    count.show(&geom, &crate::theme::DARK, Some((3, 1.0)));
    pump(300);
    assert!(border._strips.iter().all(visible) && visible(&bar.layer) && visible(&count.layer), "the frame is up");
    let (bx, by) = bar.spot.map(|s| (s.pos.0 - bar::MARGIN as i32, s.pos.1 - bar::MARGIN as i32)).unwrap();
    eprintln!("area {area:?}, bar window at {:?}", (bx, by));

    // GIF: exact colours. No red (border, dot, stop) and almost no dark
    // pixels (the bar's surface; the pointer may be over the area).
    let dir = out_dir();
    let s = spec(RecFormat::Gif, 10, area, dir.join("frame.gif"));
    let rec = std::thread::spawn(move || record(s, vec![], 1.0, |p, w, h| Box::new(GifEncoder::create(p, w, h, 10).unwrap())));
    pump(1600);
    let (path, kind) = rec.join().unwrap();
    let d = crate::rec::gif::tests::decode(&std::fs::read(&path).unwrap());
    eprintln!("source {kind:?}, {} GIF frames", d.frames.len());
    assert!(!d.frames.is_empty(), "a static picture is one frame");
    for (n, f) in d.frames.iter().enumerate() {
        let red = f.canvas.iter().filter(|p| p[0] > 180 && p[1] < 120 && p[2] < 120).count();
        let dark = f.canvas.iter().filter(|p| p[0] < 80 && p[1] < 80 && p[2] < 80).count();
        assert_eq!(red, 0, "frame {n}: border red in the recording");
        assert!(dark < 700, "frame {n}: {dark} dark pixels (the bar?)");
    }

    // MP4: luma on a border strip, the bar and the countdown is white's.
    if !skip_without_mf(false) {
        let s = spec(RecFormat::Mp4, 30, area, dir.join("frame.mp4"));
        let rec = std::thread::spawn(move || {
            record(s, vec![], 1.0, |p, w, h| Box::new(MfEncoder::create(p, w, h, 30, Quality::Medium, false).unwrap()))
        });
        pump(1600);
        let (path, _) = rec.join().unwrap();
        let l = bar::layout(&Ui { th: &crate::theme::DARK, s: 1.0, font: Some(crate::fonts::ui_font()) }, &v);
        let bar_mid = ((bx - area.0) as u32 + l.panel.x as u32 + 20, (by - area.1) as u32 + l.panel.y as u32 + l.panel.h as u32 / 2);
        let pts = [(40 + 100, 41), (41, 40 + 60), (area.2 / 2, area.3 / 2), bar_mid];
        let p = mf::probe_at(&path, &pts).unwrap();
        eprintln!("MP4 {} frames; luma at {pts:?}: {:?}", p.frames, p.luma_at.last());
        assert!(p.frames >= 15);
        for row in &p.luma_at {
            assert!(row.iter().all(|&y| y > 200), "not white: {row:?} at {pts:?}");
        }
    }
    drop((border, bar, count));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Clicks on the bar queue its buttons and nudge the owner.
#[test]
#[ignore = "opens windows on the real desktop"]
fn live_rec_bar_clicks_queue() {
    crate::capture::enable_dpi_awareness();
    let geom = Geom { area: (200, 200, 400, 300), monitors: vec![((0, 0, 1280, 720), 1.0)], scale: 1.0, mult: 1.0 };
    let mut bar = BarWin::new(0).unwrap();
    let v = BarView { elapsed: Duration::ZERO, paused: false, mic: None, cam: None, system: None, stopping: false, compact: false, asking: false };
    bar.show(&geom, &crate::theme::LIGHT, &v);
    pump(100);
    let l = BAR.with(|b| b.borrow().layout.clone()).unwrap();
    let stop = l.buttons.iter().find(|b| b.0 == BarAct::Stop).unwrap().1;
    let lp = LPARAM((((stop.y as i32 + 5) << 16) | (stop.x as i32 + 5)) as isize);
    unsafe {
        let _ = PostMessageW(Some(bar.layer.hwnd), WM_LBUTTONDOWN, WPARAM(1), lp);
        let _ = PostMessageW(Some(bar.layer.hwnd), WM_LBUTTONUP, WPARAM(0), lp);
    }
    pump(100);
    assert_eq!(BarWin::take_clicks(), [BarAct::Stop]);
    assert!(BarWin::take_clicks().is_empty());
}

/// By hand: print what a recording holds. `RUSTSHOT_PROBE=<file>` (MP4 or
/// GIF); luma (MP4) or RGB (GIF) 1 px inside each edge's middle and at
/// the centre, for the last frame.
#[test]
#[ignore = "inspects RUSTSHOT_PROBE"]
fn probe_recording_file() {
    let Some(p) = std::env::var_os("RUSTSHOT_PROBE") else { return };
    let p = std::path::PathBuf::from(p);
    if p.extension().is_some_and(|e| e == "gif") {
        let d = crate::rec::gif::tests::decode(&std::fs::read(&p).unwrap());
        let f = d.frames.last().unwrap();
        let at = |x: usize, y: usize| f.canvas[y * d.w + x];
        let (w, h) = (d.w, d.h);
        println!("GIF {w}x{h}, {} frames", d.frames.len());
        println!("top {:?} bottom {:?} left {:?} right {:?} centre {:?}", at(w / 2, 0), at(w / 2, h - 1), at(0, h / 2), at(w - 1, h / 2), at(w / 2, h / 2));
    } else {
        let q = mf::probe(&p).unwrap();
        let (w, h) = q.size;
        let pts = [(w / 2, 0), (w / 2, h - 1), (0, h / 2), (w - 1, h / 2), (w / 2, h / 2)];
        let q = mf::probe_at(&p, &pts).unwrap();
        println!("MP4 {w}x{h}, {} frames, {:?}, audio {}", q.frames, q.duration, q.has_audio);
        println!("luma top/bottom/left/right/centre of the last frame: {:?}", q.luma_at.last());
    }
}

// ---- the worker, with injected parts (no screen, no devices) ----------------

/// A 4x4 grey frame every call.
struct Still;

impl FrameSource for Still {
    fn next(&mut self, _deadline: Instant) -> Option<crate::rec::Frame> {
        std::thread::sleep(Duration::from_millis(5));
        Some(crate::rec::Frame { w: 4, h: 4, bgra: vec![128; 64], ts: Duration::MAX })
    }
    fn size(&self) -> (u32, u32) {
        (4, 4)
    }
}

/// Writes a byte per frame to `.part`; fails pushing frame `fail_at`;
/// `finished` is set by `finish`.
struct Enc {
    file: std::fs::File,
    n: usize,
    fail_at: Option<usize>,
    finished: Arc<AtomicBool>,
}

impl VideoEncoder for Enc {
    fn push_video(&mut self, _f: &crate::rec::Frame) -> Result<()> {
        use std::io::Write;
        if Some(self.n) == self.fail_at {
            bail!("disk full");
        }
        self.n += 1;
        self.file.write_all(&[1])?;
        Ok(())
    }
    fn push_audio(&mut self, _pcm: &[f32], _ts: Duration) -> Result<()> {
        Ok(())
    }
    fn finish(self: Box<Self>) -> Result<()> {
        self.finished.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn audio(&self) -> bool {
        false
    }
}

/// An opener for the fake parts (`warnings` as if a device failed).
fn fake_parts(fail_at: Option<usize>, finished: Arc<AtomicBool>, warnings: Vec<String>) -> Opener {
    Box::new(move |s: &RecSpec| {
        std::fs::create_dir_all(s.out.parent().unwrap())?;
        let file = std::fs::File::create(s.part_path())?;
        let enc = Enc { file, n: 0, fail_at, finished };
        Ok(Parts { src: Box::new(Still), audio: Vec::new(), enc: Box::new(enc), warnings, late: Vec::new() })
    })
}

fn worker_dir(tag: &str) -> std::path::PathBuf {
    let d = out_dir().join(format!("worker-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn wait_end(w: &Worker, secs: u64) -> Option<End> {
    w.rx.recv_timeout(Duration::from_secs(secs)).ok()
}

/// The encoder fails mid-recording: the worker stops by itself and the
/// recording ends as failed (nobody pressed Stop).
#[test]
fn worker_encoder_failure_ends_failed() {
    let dir = worker_dir("fail");
    let s = spec(RecFormat::Gif, 10, (0, 0, 4, 4), dir.join("a.gif"));
    let w = Worker::spawn(s, fake_parts(Some(2), Arc::new(AtomicBool::new(false)), vec![])).unwrap();
    w.tx.send(Cmd::Go).unwrap();
    let end = wait_end(&w, 10).expect("ended by itself");
    match &end {
        End::Failed(e) => assert!(e.contains("disk full"), "{e}"),
        e => panic!("not failed: {e:?}"),
    }
    drop(w);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Saved: the file is in place and what was left out is reported.
#[test]
fn worker_saves_with_warnings() {
    let dir = worker_dir("warn");
    let s = spec(RecFormat::Gif, 10, (0, 0, 4, 4), dir.join("a.gif"));
    let warn = vec!["microphone not recorded: no such device".to_string()];
    let w = Worker::spawn(s, fake_parts(None, Arc::new(AtomicBool::new(false)), warn.clone())).unwrap();
    w.tx.send(Cmd::Go).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    w.tx.send(Cmd::Stop).unwrap();
    let end = wait_end(&w, 10).expect("saved");
    assert_eq!(end, End::Saved { path: dir.join("a.gif"), warnings: warn });
    assert!(dir.join("a.gif").is_file() && !dir.join("a.gif.part").exists());
    drop(w);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sound source that stopped while recording is a warning of the saved
/// file, asked once it is saved.
#[test]
fn worker_adds_late_warnings_to_the_saved_file() {
    let dir = worker_dir("late");
    let s = spec(RecFormat::Gif, 10, (0, 0, 4, 4), dir.join("a.gif"));
    let inner = fake_parts(None, Arc::new(AtomicBool::new(false)), vec!["first".into()]);
    let open: Opener = Box::new(move |s: &RecSpec| {
        let mut p = inner(s)?;
        p.late.push(Box::new(|| Some("microphone stopped during the recording: gone".into())));
        p.late.push(Box::new(|| None));
        Ok(p)
    });
    let w = Worker::spawn(s, open).unwrap();
    w.tx.send(Cmd::Go).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    w.tx.send(Cmd::Stop).unwrap();
    let end = wait_end(&w, 10).expect("saved");
    let warnings = vec!["first".to_string(), "microphone stopped during the recording: gone".into()];
    assert_eq!(end, End::Saved { path: dir.join("a.gif"), warnings });
    drop(w);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Dropping the worker mid-recording (the daemon quits) finishes the file.
#[test]
fn worker_drop_finishes_the_file() {
    let dir = worker_dir("drop");
    let s = spec(RecFormat::Gif, 10, (0, 0, 4, 4), dir.join("a.gif"));
    let finished = Arc::new(AtomicBool::new(false));
    let w = Worker::spawn(s, fake_parts(None, finished.clone(), vec![])).unwrap();
    w.tx.send(Cmd::Go).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    drop(w);
    assert!(finished.load(std::sync::atomic::Ordering::SeqCst), "finished");
    assert!(dir.join("a.gif").is_file(), "moved into place");
    assert!(!std::fs::read(dir.join("a.gif")).unwrap().is_empty(), "frames written");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Parts that cannot be opened: failed, and no `.part` is left behind.
#[test]
fn worker_open_failure_removes_the_part() {
    let dir = worker_dir("open");
    let s = spec(RecFormat::Gif, 10, (0, 0, 4, 4), dir.join("a.gif"));
    let open: Opener = Box::new(|s: &RecSpec| {
        std::fs::create_dir_all(s.out.parent().unwrap())?;
        std::fs::write(s.part_path(), b"x")?;
        bail!("no H.264 encoder is installed")
    });
    let w = Worker::spawn(s, open).unwrap();
    assert_eq!(wait_end(&w, 10), Some(End::Failed("no H.264 encoder is installed".into())));
    assert!(!dir.join("a.gif.part").exists());
    drop(w);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A recording that fails to start (its parts cannot be opened) takes its
/// windows (border, bar, countdown) down when the failure arrives.
#[test]
#[ignore = "opens windows on the real desktop"]
fn live_rec_windows_close_when_the_start_fails() {
    crate::capture::enable_dpi_awareness();
    let geom = Geom { area: (200, 200, 400, 300), monitors: vec![((0, 0, 1280, 720), 1.0)], scale: 1.0, mult: 1.0 };
    let dir = worker_dir("live-fail");
    let s = spec(RecFormat::Gif, 10, geom.area, dir.join("a.gif"));
    let open: Opener = Box::new(|_: &RecSpec| bail!("no capture"));
    let ui = Recorder::start_with(s, geom, crate::theme::DARK, HWND::default(), open).unwrap();
    let hwnds: Vec<HWND> = (ui.border.as_ref().unwrap()._strips.iter().map(|l| l.hwnd))
        .chain([ui.bar.as_ref().unwrap().layer.hwnd, ui.count.as_ref().unwrap().layer.hwnd])
        .collect();
    assert_eq!(hwnds.len(), 6);
    let mut run = super::super::Run::fake(true, false, Instant::now());
    run.m = super::super::Machine::new();
    run.ui = Some(ui);
    let t = Instant::now();
    run.m.start(true, t);
    let mut end = None;
    while end.is_none() && t.elapsed() < Duration::from_secs(5) {
        pump(20);
        end = run.tick(Instant::now());
    }
    assert_eq!(end, Some(End::Failed("no capture".into())));
    assert!(hwnds.iter().all(|h| !unsafe { IsWindow(Some(*h)) }.as_bool()), "every window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The camera fails mid-recording: the bubble hides and the saved file
/// says "camera stopped: <reason>".
#[test]
#[ignore = "opens windows on the real desktop"]
fn live_rec_camera_failure_is_said_with_the_file() {
    crate::capture::enable_dpi_awareness();
    let geom = Geom { area: (200, 200, 400, 300), monitors: vec![((0, 0, 1280, 720), 1.0)], scale: 1.0, mult: 1.0 };
    let dir = worker_dir("live-cam");
    let s = spec(RecFormat::Gif, 10, geom.area, dir.join("a.gif"));
    let mut ui = Recorder::start_with(s, geom, crate::theme::DARK, HWND::default(), fake_parts(None, Arc::new(AtomicBool::new(false)), vec![])).unwrap();
    let look = crate::cam::Look::new(crate::cam::Shape::Circle, crate::cam::CamSize::S);
    let open: crate::cam::Opener = Box::new(|| bail!("The camera was disconnected or taken by another app"));
    let bubble = crate::cam::Bubble::show_with(look, "", (200, 200, 400, 300), 1.0, &crate::theme::DARK, HWND::default(), open).unwrap();
    let bh = bubble.hwnd();
    ui.bubble = Some(bubble);
    ui.send(Effect::Go);
    pump(300);
    assert!(!unsafe { IsWindowVisible(bh) }.as_bool(), "the bubble hid");
    ui.send(Effect::Stop);
    let t = Instant::now();
    let mut end = None;
    while end.is_none() && t.elapsed() < Duration::from_secs(10) {
        pump(20);
        end = ui.poll_end();
    }
    ui.close();
    assert_eq!(
        end,
        Some(End::Saved { path: dir.join("a.gif"), warnings: vec!["camera stopped: The camera was disconnected or taken by another app".into()] })
    );
    let _ = std::fs::remove_dir_all(&dir);
}
