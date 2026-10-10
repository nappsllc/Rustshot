//! Live: the recording frame (border, control bar, countdown) is on
//! screen but never in the recording. `#[ignore]`d, run by CI's desktop
//! job (`live_rec_`).

use super::*;
use crate::rec::win::tests::{out_dir, record, skip_without_mf, spec, TestWindow};
use crate::rec::Quality;
use std::time::Instant;

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
    let border = Border::show(inner, 4, area).unwrap();
    let geom = Geom { area, monitors: vec![area], scale: 1.0 };
    let mut bar = BarWin::new(0).unwrap();
    let v = BarView { elapsed: Duration::from_secs(5), paused: false, mic: Some(false), stopping: false };
    bar.show(&geom, &crate::theme::DARK, &v);
    let mut count = CountWin { layer: Layer::new(true).unwrap(), drawn: None };
    count.show(&geom, &crate::theme::DARK, Some(3));
    pump(300);
    assert!(border._strips.iter().all(visible) && visible(&bar.layer) && visible(&count.layer), "the frame is up");
    let (bx, by) = bar.pos.unwrap();
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
    let geom = Geom { area: (200, 200, 400, 300), monitors: vec![(0, 0, 1280, 720)], scale: 1.0 };
    let mut bar = BarWin::new(0).unwrap();
    let v = BarView { elapsed: Duration::ZERO, paused: false, mic: None, stopping: false };
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
