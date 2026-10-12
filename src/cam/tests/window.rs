//! The bubble window with a fake camera (no camera needed).

use super::super::bubble::{Bubble, Opener};
use super::super::*;
use crate::theme::DARK;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Pictures of a solid colour, about 100 a second; says when it is dropped
/// (the camera released).
struct Fake {
    dropped: Arc<AtomicBool>,
}

impl CamSource for Fake {
    fn size(&self) -> (u32, u32) {
        (64, 48)
    }

    fn read(&mut self, into: &mut CamFrame) -> anyhow::Result<bool> {
        std::thread::sleep(Duration::from_millis(10));
        (into.w, into.h) = (64, 48);
        into.bgra.clear();
        into.bgra.extend_from_slice(&[40u8, 160, 90, 255].repeat(64 * 48));
        Ok(true)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// Dispatch this thread's messages for `ms`.
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.unwrap();
    r
}

const M: Look = Look { shape: Shape::Circle, size: CamSize::M, mirror: true, ring: None, ring_w: 3 };

#[test]
fn bubble_shows_pictures_follows_the_area_and_releases_the_camera() {
    let dropped = Arc::new(AtomicBool::new(false));
    let d2 = dropped.clone();
    let open: Opener = Box::new(move || Ok(Box::new(Fake { dropped: d2 }) as Box<dyn CamSource>));
    let _g = super::REMEMBERED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    remember(Rel::default());
    let area = (100, 100, 800, 600);
    let b = Bubble::show_with(M, "", area, 1.0, &DARK, HWND::default(), open);
    let mut b = b.unwrap();
    let h = b.hwnd();
    // Waited for, not a fixed pump: under a loaded test run the camera
    // thread may take far longer than a few hundred ms to deliver.
    assert!(b.wait_picture(Duration::from_secs(10)), "the camera's pictures arrive");
    unsafe {
        assert!(IsWindowVisible(h).as_bool());
        // In the recording: not excluded from capture.
        let mut aff = 0u32;
        GetWindowDisplayAffinity(h, &mut aff).unwrap();
        assert_eq!(aff, WDA_NONE.0, "the bubble is captured");
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        for f in [WS_EX_LAYERED, WS_EX_TOPMOST, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW] {
            assert_ne!(ex & f.0, 0, "{f:?}");
        }
    }
    // Bottom left of the area, inset; the window is the shadow's pad larger.
    let (d, pad) = (160, pad(1.0) as i32);
    assert_eq!(b.pos(), Some((124, 700 - 24 - d)));
    let r = rect(h);
    assert_eq!((r.left, r.top, r.right - r.left), (124 - pad, 700 - 24 - d - pad, d + 2 * pad));
    // The selection moves: the bubble keeps its corner.
    b.set_anchor((0, 0, 1000, 700), 1.0);
    assert_eq!(b.pos(), Some((24, 700 - 24 - d)));
    assert_eq!(rect(h).left, 24 - pad);
    // Larger: still at its corner, the window grows.
    b.set_look(Look { size: CamSize::L, ..M }, &DARK);
    assert_eq!(b.pos(), Some((24, 700 - 24 - 200)));
    let r = rect(h);
    assert_eq!(r.right - r.left, 200 + 2 * pad);
    // A picture of the old size is dropped; one of the new size comes.
    assert!(b.wait_picture(Duration::from_secs(10)), "pictures at the new size");
    assert!(b.error().is_none());
    // Closed: the window is gone and the camera released.
    drop(b);
    assert!(dropped.load(Ordering::SeqCst), "camera released on close");
    assert!(!unsafe { IsWindow(Some(h)) }.as_bool());
    // A new bubble can be shown on the thread.
    let open: Opener = Box::new(|| Ok(Box::new(Fake { dropped: Arc::new(AtomicBool::new(false)) }) as Box<dyn CamSource>));
    let again = Bubble::show_with(M, "", area, 1.0, &DARK, HWND::default(), open).unwrap();
    let open: Opener = Box::new(|| anyhow::bail!("unused"));
    assert!(Bubble::show_with(M, "", area, 1.0, &DARK, HWND::default(), open).is_err(), "one at a time");
    drop(again);
}

/// The bubble with the real camera for two seconds (on screen!). Skips
/// without a camera.
#[test]
#[ignore = "needs a camera and shows a window; run with --ignored"]
fn live_cam_bubble_shows_the_camera() {
    let _g = super::LIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cams = list();
    if cams.is_empty() {
        eprintln!("SKIP: no camera on this machine");
        return;
    }
    let mut b = Bubble::show(M, "", (200, 200, 900, 600), 1.0, &DARK, HWND::default()).unwrap();
    pump(1000);
    if let Some(e) = b.error() {
        eprintln!("SKIP: {e}");
        return;
    }
    assert!(b.has_picture(), "pictures within a second");
    b.set_look(Look { shape: Shape::Rounded, size: CamSize::S, mirror: false, ..M }, &DARK);
    pump(1000);
    assert!(b.has_picture() && b.error().is_none());
    let t = Instant::now();
    drop(b);
    eprintln!("closed in {:?}", t.elapsed());
}

#[test]
fn bubble_says_why_the_camera_failed() {
    let open: Opener = Box::new(|| anyhow::bail!(ACCESS_OFF));
    let b = Bubble::show_with(M, "", (0, 0, 800, 600), 1.0, &DARK, HWND::default(), open).unwrap();
    let h = b.hwnd();
    pump(300);
    assert_eq!(b.error().as_deref(), Some(ACCESS_OFF));
    assert!(!unsafe { IsWindowVisible(h) }.as_bool(), "hidden once the camera failed");
    drop(b);
    assert!(!unsafe { IsWindow(Some(h)) }.as_bool());
}

/// Start without a countdown waits for the first picture; the bubble
/// keeps off the toolbar and comes back when it is gone; closing stops
/// the camera thread posting to the window.
#[test]
fn bubble_waits_for_a_picture_and_keeps_off_the_toolbar() {
    let _g = super::REMEMBERED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    remember(Rel::default());
    let dropped = Arc::new(AtomicBool::new(false));
    let d2 = dropped.clone();
    let open: Opener = Box::new(move || Ok(Box::new(Fake { dropped: d2 }) as Box<dyn CamSource>));
    let area = (100, 100, 800, 600);
    let mut b = Bubble::show_with(M, "", area, 1.0, &DARK, HWND::default(), open).unwrap();
    assert!(b.wait_picture(Duration::from_secs(1)), "a picture within a second");
    assert!(b.has_picture());
    let home = b.pos().unwrap();
    assert_eq!(home, (124, 700 - 24 - 160));
    // A popover down the left side: the bubble moves to the bottom right.
    let screen = (0, 0, 1920, 1080);
    b.set_avoid(&[(110, 100, 320, 600)], screen);
    assert_eq!(b.pos(), Some((900 - 24 - 160, 700 - 24 - 160)));
    assert_eq!(rect(b.hwnd()).left, 900 - 24 - 160 - pad(1.0) as i32);
    // The toolbar is gone: back where it was left.
    b.clear_avoid();
    assert_eq!(b.pos(), Some(home));
    drop(b);
    assert!(dropped.load(Ordering::SeqCst));
    // No picture when the camera fails.
    let open: Opener = Box::new(|| anyhow::bail!("broken"));
    let b = Bubble::show_with(M, "", area, 1.0, &DARK, HWND::default(), open).unwrap();
    assert!(!b.wait_picture(Duration::from_secs(1)));
    assert_eq!(b.error().as_deref(), Some("broken"));
}
