//! The capture launcher (tray "Capture…", `rustshot launcher`): a small
//! window choosing what to capture (Region / Screen / Window), after how
//! long (0 / 3 / 5 / 10 s, or 0–60 s with the stepper) and what follows
//! (Edit / Copy / Save / Pin), remembered in the config
//! (`launcher_source`, `launcher_delay`, `launcher_after`).
//!
//! A delayed capture counts down in the daemon ([`Delay`]): the launcher
//! is gone, a small pill bottom-right says "Capturing in 3 s" with Cancel
//! (a notice, never in the capture), the tray's tip says it too and its
//! menu gains "Cancel capture". The tray's "Capture in 3 s" / "Capture
//! in 10 s" start a region capture the same way ([`Plan::region_in`]).
//! Without a daemon the launcher runs in its own process and the capture
//! (and its countdown) follows there ([`run_here`]).
#![cfg_attr(target_os = "macos", allow(dead_code))] // no launcher window there yet

use crate::config::Config;
use crate::editor::Pending;
use crate::export::Task;
use crate::hotkey::HotEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::chan::Sender;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(not(target_os = "macos"))]
mod window;

/// Default and fixed size of the window, logical px.
pub const W: u32 = 360;
pub const H: u32 = 260;
/// The longest delay the stepper allows.
pub const MAX_DELAY: u32 = 60;
/// The delay choices of the segmented control.
pub const DELAYS: [u32; 4] = [0, 3, 5, 10];

/// What to capture.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Source {
    #[default]
    Region,
    Screen,
    Window,
}

/// What follows the capture.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum After {
    /// The overlay's editor (annotate, then choose).
    #[default]
    Edit,
    Copy,
    Save,
    Pin,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Region, Source::Screen, Source::Window];

    pub fn id(self) -> &'static str {
        match self {
            Source::Region => "region",
            Source::Screen => "screen",
            Source::Window => "window",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Region => "Region",
            Source::Screen => "Screen",
            Source::Window => "Window",
        }
    }

    pub fn from_id(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|v| v.id() == s)
    }
}

impl After {
    pub const ALL: [After; 4] = [After::Edit, After::Copy, After::Save, After::Pin];

    pub fn id(self) -> &'static str {
        match self {
            After::Edit => "edit",
            After::Copy => "copy",
            After::Save => "save",
            After::Pin => "pin",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            After::Edit => "Edit",
            After::Copy => "Copy",
            After::Save => "Save",
            After::Pin => "Pin",
        }
    }

    pub fn from_id(s: &str) -> Option<After> {
        After::ALL.into_iter().find(|v| v.id() == s)
    }
}

/// One launch: what, after how many seconds, then what.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Plan {
    pub source: Source,
    pub delay: u32,
    pub after: After,
}

impl Plan {
    /// A region capture into the editor after `secs` (the tray's quick items).
    pub fn region_in(secs: u32) -> Plan {
        Plan { source: Source::Region, delay: secs.min(MAX_DELAY), after: After::Edit }
    }

    /// The launcher's last choices.
    pub fn from_config(cfg: &Config) -> Plan {
        Plan {
            source: Source::from_id(&cfg.launcher_source).unwrap_or_default(),
            delay: cfg.launcher_delay.min(MAX_DELAY),
            after: After::from_id(&cfg.launcher_after).unwrap_or_default(),
        }
    }

    /// The config edits that remember this plan (raw TOML values).
    pub fn config_edits(&self) -> Vec<(String, Option<String>)> {
        vec![
            ("launcher_source".into(), Some(crate::config::toml_str(self.source.id()))),
            ("launcher_delay".into(), Some(self.delay.min(MAX_DELAY).to_string())),
            ("launcher_after".into(), Some(crate::config::toml_str(self.after.id()))),
        ]
    }

    /// The capture to start once the delay is over.
    pub fn pending(&self) -> Pending {
        let tasks = match self.after {
            After::Edit => Vec::new(),
            After::Copy => vec![Task::Copy],
            After::Save => vec![Task::Save { path: None, ask: false }],
            After::Pin => vec![Task::Pin],
        };
        let accept = self.after != After::Edit;
        let mut p = match self.source {
            Source::Region => Pending::editor(),
            Source::Screen => Pending { select_all: true, ..Pending::editor() },
            Source::Window => Pending::window(),
        };
        p.tasks = tasks;
        p.accept_on_select = accept;
        p
    }
}

// --- the countdown ---------------------------------------------------------------

/// A delayed capture waiting for its time.
#[derive(Clone, Debug, PartialEq)]
pub struct Countdown {
    pub plan: Plan,
    pub due: Instant,
    /// The seconds last shown (the pill and the tip change once a second).
    pub shown: Option<u64>,
}

/// What a countdown does at a tick.
#[derive(Clone, Debug, PartialEq)]
pub enum Tick {
    /// Still waiting; `Some(n)` when the whole seconds left changed to `n`.
    Wait(Option<u64>),
    /// Its time: capture.
    Fire(Plan),
}

impl Countdown {
    pub fn new(plan: Plan, now: Instant) -> Countdown {
        Countdown { plan, due: now + Duration::from_secs(plan.delay as u64), shown: None }
    }

    /// Whole seconds left at `now`, rounded up (3 for 2.1 s).
    pub fn left(&self, now: Instant) -> u64 {
        let d = self.due.saturating_duration_since(now);
        d.as_secs() + u64::from(d.subsec_nanos() > 0)
    }

    pub fn tick(&mut self, now: Instant) -> Tick {
        if now >= self.due {
            return Tick::Fire(self.plan);
        }
        let n = self.left(now);
        if self.shown == Some(n) {
            return Tick::Wait(None);
        }
        self.shown = Some(n);
        Tick::Wait(Some(n))
    }
}

/// The pill's text.
pub fn pill_text(left: u64) -> String {
    format!("Capturing in {left} s")
}

/// The pill: "Capturing in 3 s" with Cancel, bottom-right (it stays a
/// second past the next change: no fade between them).
pub fn pill(left: u64) -> crate::notice::Note {
    let mut n = crate::notice::Note::plain(pill_text(left), crate::editor::style::ToastKind::Info);
    n.actions = vec![crate::notice::NoteAct::CancelDelay];
    n.corner = true;
    n.ttl = Duration::from_millis(1500);
    n
}

/// Asked to call the countdown off (the pill's Cancel; any thread).
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// The daemon's delayed capture (at most one: a new one replaces it).
#[derive(Default)]
pub struct Delay {
    pub cd: Option<Countdown>,
}

/// What showing the countdown does (the pill, the tray); tests record it.
pub trait Show {
    /// The seconds left changed (`None`: the countdown is over).
    fn left(&mut self, n: Option<u64>);
}

/// The real one: the pill near the pointer's monitor, the tray's tip.
pub struct Screen {
    pub th: crate::theme::Theme,
    pub up: bool,
}

impl Show for Screen {
    fn left(&mut self, n: Option<u64>) {
        crate::tray::set_delay(n);
        match n {
            Some(n) if self.up => {
                if !crate::notice::replace(pill(n)) {
                    self.up = crate::notice::show(pill(n), pointer(), self.th);
                }
            }
            Some(n) => self.up = crate::notice::show(pill(n), pointer(), self.th),
            None => {
                if std::mem::take(&mut self.up) {
                    crate::notice::hide();
                }
            }
        }
    }
}

fn pointer() -> (i32, i32) {
    #[cfg(not(test))]
    return crate::capture::cursor_pos();
    #[cfg(test)]
    return (0, 0);
}

impl Delay {
    /// Start `plan` at `now` (replacing a countdown that runs). No delay:
    /// the capture to start now.
    pub fn start(&mut self, plan: Plan, now: Instant, show: &mut dyn Show) -> Option<Pending> {
        CANCEL.store(false, Ordering::SeqCst);
        if plan.delay == 0 {
            if self.cd.take().is_some() {
                show.left(None);
            }
            return Some(plan.pending());
        }
        self.cd = Some(Countdown::new(plan, now));
        self.poll(now, show)
    }

    /// Call the countdown off; whether one ran.
    pub fn cancel(&mut self, show: &mut dyn Show) -> bool {
        CANCEL.store(false, Ordering::SeqCst);
        let ran = self.cd.take().is_some();
        if ran {
            show.left(None);
        }
        ran
    }

    pub fn active(&self) -> bool {
        self.cd.is_some()
    }

    /// Advance to `now`: the capture once it is due (the pill gone first).
    pub fn poll(&mut self, now: Instant, show: &mut dyn Show) -> Option<Pending> {
        if CANCEL.load(Ordering::SeqCst) {
            self.cancel(show);
            return None;
        }
        let cd = self.cd.as_mut()?;
        match cd.tick(now) {
            Tick::Wait(Some(n)) => {
                show.left(Some(n));
                None
            }
            Tick::Wait(None) => None,
            Tick::Fire(plan) => {
                self.cd = None;
                show.left(None);
                Some(plan.pending())
            }
        }
    }
}

// --- the window ------------------------------------------------------------------

/// Where the window's Capture goes (the daemon, set when it starts).
static DAEMON: Mutex<Option<Sender<HotEvent>>> = Mutex::new(None);
/// The plan a window without a daemon chose (`run_here` captures it).
static CHOSEN: Mutex<Option<Plan>> = Mutex::new(None);
/// The open window's "come to the front".
static OPEN: Mutex<Option<Sender<()>>> = Mutex::new(None);
/// A plan for the one-shot editor to count down first (`run_here`).
static START: Mutex<Option<Plan>> = Mutex::new(None);

/// Whether the window is open in this process (an on-demand run waits
/// for it before it quits).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn is_open() -> bool {
    lock(&OPEN).is_some()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_daemon(tx: Sender<HotEvent>) {
    *lock(&DAEMON) = Some(tx);
}

/// The one-shot editor's countdown, if `run_here` set one.
pub fn take_start() -> Option<Plan> {
    lock(&START).take()
}

/// The window chose `plan`: remember it and capture (the daemon counts
/// down; without one, `run_here` does).
#[cfg(test)]
fn launch(plan: Plan) {
    LAUNCHED.with(|l| l.borrow_mut().push(plan));
}

#[cfg(not(test))]
fn launch(plan: Plan) {
    let path = crate::config::config_path();
    crate::config::queue_key_edits(&path, plan.config_edits(), true);
    let sent = lock(&DAEMON).as_ref().is_some_and(|tx| tx.send(HotEvent::Launch(plan)).is_ok());
    if !sent {
        *lock(&CHOSEN) = Some(plan);
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: the plans launched.
    pub static LAUNCHED: std::cell::RefCell<Vec<Plan>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// How long a closed launcher is given to leave the screen (the DWM's
/// close fade, a compositor's unmap) before its capture starts.
pub const SETTLE: Duration = Duration::from_millis(250);

/// The window is gone: after [`SETTLE`] (`settle`), its plan (if one was
/// chosen) is launched, so the launcher is never in the capture.
pub fn after_close(chosen: Option<Plan>, settle: impl FnOnce()) {
    if let Some(p) = chosen {
        settle();
        launch(p);
    }
}

/// Open the launcher (or raise the open one), on its own thread. macOS
/// has no windows off the main thread: the last choices start at once.
pub fn show() {
    #[cfg(target_os = "macos")]
    {
        let plan = Plan::from_config(&crate::config::load());
        if let Some(tx) = lock(&DAEMON).as_ref() {
            let _ = tx.send(HotEvent::Launch(plan));
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(rx) = claim() {
        std::thread::Builder::new().name("rustshot-launcher".into()).spawn(move || window::run(rx)).ok();
    }
}

#[cfg(not(target_os = "macos"))]
fn claim() -> Option<crate::chan::Receiver<()>> {
    let mut open = lock(&OPEN);
    if let Some(tx) = open.as_ref()
        && tx.send(()).is_ok()
    {
        return None;
    }
    let (tx, rx) = crate::chan::channel();
    *open = Some(tx);
    Some(rx)
}

#[cfg(not(target_os = "macos"))]
fn release() {
    *lock(&OPEN) = None;
}

/// `rustshot launcher` with no daemon: the window here, then the capture
/// it chose (counted down by the one-shot editor). None: closed.
pub fn run_here() -> Option<Plan> {
    #[cfg(target_os = "macos")]
    let plan = Some(Plan::from_config(&crate::config::load()));
    #[cfg(not(target_os = "macos"))]
    let plan = {
        let rx = claim()?;
        window::run(rx);
        lock(&CHOSEN).take()
    };
    if let Some(p) = plan
        && p.delay > 0
    {
        *lock(&START) = Some(p);
    }
    plan
}

#[cfg(test)]
mod tests;
