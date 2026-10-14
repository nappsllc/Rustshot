//! Scrolling capture (the overlay's "Scrolling capture", Shift+S by
//! default, in the selection size label's menu): Rustshot scrolls the
//! window under the selected region at a steady pace, grabs the region
//! after each step and stitches the frames into one tall picture
//! ([`stitch`]), which then opens in the Edit window.
//!
//! The run ([`run`]) is platform-neutral: a [`Wheel`] scrolls the target
//! window (Windows: `WM_MOUSEWHEEL` posted to the window under the
//! region, UI Automation's ScrollPattern when the wheel does nothing;
//! X11: XTest wheel buttons 4/5, only while the pointer is over the
//! target; macOS: a CGEvent scroll, compile-only) and a grab function
//! captures the region. It stops at the end of the content (two steps that
//! move nothing), at [`stitch::MAX_H`], when the frames no longer match,
//! or on Stop / Esc ([`Ctl`]). The progress pill (`pill`) shows the
//! height so far with a Stop button, outside the region.
//!
//! Only the window under the region is ever scrolled: the target is found
//! once, when the run starts, and every scroll goes to it (or, on X11,
//! is sent only while the window under the pointer is still it).
#![cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]

pub mod stitch;
#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "macos")]
mod macos;
pub mod pill;

use crate::capture::IRect;
use crate::pixbuf::PixBuf;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stitch::{Img, Step, Stitcher};

/// What the pill and the overlay say (one place, for the translations).
pub mod text {
    /// The size label menu's row and the Settings key row.
    pub const ACTION: &str = "Scrolling capture";
    /// The pill while it runs: "Scrolling… 4 820 px".
    pub const RUNNING: &str = "Scrolling\u{2026}";
    pub const STOP: &str = "Stop";
    /// Why it ended, said in a toast when that is not the end of the page.
    pub const LOST: &str = "Scrolling capture stopped: the next part did not match";
    pub const CAP: &str = "Scrolling capture stopped at 20 000 px";
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub const NO_TARGET: &str = "Nothing to scroll under the selection";
    pub const UNSUPPORTED: &str = "Scrolling capture is not available here";
}

/// Scrolls the target window.
pub trait Wheel: Send {
    /// Scroll down by `notches` wheel notches (or line steps).
    fn scroll(&mut self, notches: u32) -> Result<()>;
    /// The scrolls did nothing: try another way (Windows: UI Automation
    /// after the wheel). True when there is one to try.
    fn fallback(&mut self) -> bool {
        false
    }
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// The end of the content.
    Bottom,
    /// [`stitch::MAX_H`] reached.
    Cap,
    /// Stop or Esc.
    Stopped,
    /// The frames stopped matching (kept up to there).
    Lost,
}

impl End {
    /// What to tell the user, when it is worth a toast.
    pub fn note(&self) -> Option<&'static str> {
        match self {
            End::Lost => Some(text::LOST),
            End::Cap => Some(text::CAP),
            End::Bottom | End::Stopped => None,
        }
    }
}

/// The run's state, shared with the pill.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// Stitched height so far (px; the pill shows it).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub height: u32,
    pub frames: u32,
}

/// Stop asked (the pill's Stop, Esc), shared with the run.
#[derive(Clone, Default)]
pub struct Ctl {
    stop: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}

impl Ctl {
    #[cfg_attr(target_os = "macos", allow(dead_code))] // the pill's Stop
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, p: Progress) {
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) = p;
    }
}

/// Pacing of a run.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    /// Wheel notches of the first step (adapted after: a step should
    /// move about half the region).
    pub notches: u32,
    /// After a scroll, grab every `poll` until two grabs agree (smooth
    /// scrolling has settled), for at most `settle`.
    pub poll: Duration,
    pub settle: Duration,
    /// At least this between steps.
    pub min_step: Duration,
}

impl Default for Pace {
    fn default() -> Pace {
        Pace { notches: 3, poll: Duration::from_millis(40), settle: Duration::from_millis(700), min_step: Duration::from_millis(120) }
    }
}

/// The most notches one step takes.
const MAX_NOTCHES: u32 = 24;

/// The next step's notches: aim for a step of about half the frame `h`.
pub fn adapt(notches: u32, dy: u32, h: u32) -> u32 {
    let n = notches.max(1);
    if dy == 0 {
        return (n * 2).min(MAX_NOTCHES);
    }
    // Pixels a notch moved; as many notches as half the frame takes.
    let per = (dy as f32 / n as f32).max(1.0);
    let want = ((h as f32 * 0.5) / per).floor() as u32;
    want.clamp(1, MAX_NOTCHES)
}

/// Scroll and grab until the end; the stitched picture and why it ended.
/// `grab` captures the region; `esc` says whether Esc is held.
pub fn run(
    grab: &mut dyn FnMut() -> Result<PixBuf>,
    wheel: &mut dyn Wheel,
    ctl: &Ctl,
    pace: Pace,
    esc: &dyn Fn() -> bool,
) -> Result<(PixBuf, End)> {
    let first = grab()?;
    let h = first.height();
    let mut st = Stitcher::new(Img::of(&first));
    let mut last = first;
    let mut notches = pace.notches.max(1);
    let mut stills = 0;
    let mut tried_fallback = false;
    let mut frames = 1;
    let stop = || ctl.stopped() || esc();
    let end = loop {
        if stop() {
            break End::Stopped;
        }
        let t0 = Instant::now();
        wheel.scroll(notches)?;
        // Wait for the picture to settle (smooth scrolling, repaints).
        let deadline = t0 + pace.settle;
        let mut cur = grab()?;
        loop {
            if stop() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(pace.poll);
            let next = grab()?;
            let settled = next.as_raw() == cur.as_raw() && next.as_raw() != last.as_raw();
            cur = next;
            if settled {
                break;
            }
        }
        if stop() {
            break End::Stopped;
        }
        frames += 1;
        match st.push(Img::of(&cur)) {
            Step::Moved(dy) => {
                stills = 0;
                notches = adapt(notches, dy, h);
                last = cur;
            }
            Step::Still => {
                stills += 1;
                // Nothing moved at all yet: maybe the wheel does not reach
                // it; another way, once.
                if st.last_dy().is_none() && !tried_fallback {
                    tried_fallback = true;
                    if wheel.fallback() {
                        stills = 0;
                        continue;
                    }
                }
                if stills >= 2 || st.full() {
                    break if st.full() { End::Cap } else { End::Bottom };
                }
            }
            Step::Lost => break End::Lost,
        }
        ctl.set(Progress { height: st.height(), frames });
        if st.full() {
            break End::Cap;
        }
        let spent = t0.elapsed();
        if spent < pace.min_step {
            std::thread::sleep(pace.min_step - spent);
        }
    };
    ctl.set(Progress { height: st.height(), frames });
    Ok((st.finish(), end))
}

/// Grab `area` (physical virtual-screen px) as RGBA.
pub fn grab_area(area: IRect) -> Result<PixBuf> {
    crate::capture::gdi_capture(area.0, area.1, area.2, area.3)
}

/// Whether scrolling capture works here (Windows; X11 with XTest; not on
/// Wayland, where no app may scroll another's window).
pub fn supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        x11::available()
    }
    #[cfg(target_os = "macos")]
    {
        macos::available()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        false
    }
}

/// The wheel for the window under `area`'s centre (found now: the overlay
/// must be hidden already).
#[cfg_attr(test, allow(dead_code))] // live only (tests drive `run`)
pub fn wheel_for(area: IRect) -> Result<Box<dyn Wheel>> {
    let at = (area.0 + area.2 as i32 / 2, area.1 + area.3 as i32 / 2);
    #[cfg(windows)]
    {
        Ok(Box::new(win::WinWheel::at(at)?))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(x11::XWheel::at(at)?))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacWheel::at(at)?))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = at;
        anyhow::bail!(text::UNSUPPORTED)
    }
}

/// Whether Esc is held now (read from the keyboard state; no hook).
#[cfg_attr(test, allow(dead_code))]
pub fn esc_down() -> bool {
    #[cfg(windows)]
    {
        win::esc_down()
    }
    #[cfg(target_os = "linux")]
    {
        x11::esc_down()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// A run on its own thread: the overlay's daemon ticks it.
pub struct Job {
    pub ctl: Ctl,
    pub area: IRect,
    rx: crate::chan::Receiver<Result<(PixBuf, End), String>>,
    pill: Option<pill::Pill>,
}

impl Job {
    /// Start scrolling the window under `area`: the wheel is found now, the
    /// run goes on a worker thread, the pill shows beside the area.
    #[cfg_attr(test, allow(dead_code))] // the app starts it outside tests
    pub fn start(area: IRect, th: crate::theme::Theme) -> Result<Job> {
        let wheel = wheel_for(area)?;
        let ctl = Ctl::default();
        let (tx, rx) = crate::chan::channel();
        let c2 = ctl.clone();
        std::thread::Builder::new().name("scroll-capture".into()).spawn(move || {
            #[cfg(windows)]
            crate::rec::win::com_init();
            let mut wheel = wheel;
            let mut grab = || grab_area(area);
            let r = run(&mut grab, wheel.as_mut(), &c2, Pace::default(), &esc_down).map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
        })?;
        let pill = pill::Pill::show(area, th, &ctl).map_err(|e| eprintln!("rustshot: scrolling capture pill: {e:#}")).ok();
        Ok(Job { ctl, area, rx, pill })
    }

    /// Advance the pill; the result once the run is over (the pill gone).
    pub fn tick(&mut self) -> Option<Result<(PixBuf, End), String>> {
        if let Some(p) = &mut self.pill {
            p.tick(&self.ctl);
        }
        match self.rx.try_recv() {
            Ok(r) => {
                self.pill = None;
                Some(r)
            }
            Err(crate::chan::TryRecvError::Empty) => None,
            Err(crate::chan::TryRecvError::Disconnected) => {
                self.pill = None;
                Some(Err("the scrolling capture stopped".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests;
