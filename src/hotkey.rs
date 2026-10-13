use crate::config::Config;
use crate::wind::key;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug, Clone)]
pub enum HotEvent {
    Capture,
    /// Rustshot was started again while this daemon runs (no command: the
    /// Start menu, the exe, `rustshot`): a capture, or Settings when the
    /// tray icon is hidden (or no capture can start now).
    Relaunch,
    /// `rustshot daemon --force` found this run: an on-demand run becomes
    /// the daemon (all its hotkeys, it stays); a stopping one stays.
    #[cfg_attr(not(windows), allow(dead_code))] // sent on Windows only
    BecomeDaemon,
    Quit,
    /// An update is about to install: reply on the sender as soon as no
    /// capture, editor or upload is running, and start no new capture from
    /// now on (until `RestartAborted` or the exit).
    RestartWhenIdle(Sender<()>),
    /// The install after `RestartWhenIdle` failed: captures work again.
    RestartAborted,
    /// An installed update is starting: quit once no capture is open.
    Restart,
    /// The Settings window saved `config.toml`: re-read it, re-register the
    /// global hotkeys and apply `check_updates`.
    ReloadConfig,
    /// The record hotkey: start a recording (area selection in record
    /// mode) when idle, stop the one in progress otherwise.
    Record,
    /// Capture every monitor at once and save it (no overlay).
    CaptureFull,
    /// The overlay picking a window: a click selects the outlined one.
    CaptureWindow,
    /// The last capture's region again, saved like it.
    RepeatLast,
    /// Pick an area and copy its text (OCR), no annotation step.
    CaptureText,
    /// Open the History window.
    History,
    /// A global hotkey was pressed: slot `i` (0 capture, 1 quit, 2 record,
    /// 3 full screen, 4 window, 5 repeat, 6 copy text, 7 History). The daemon
    /// runs [`slot_event`]`(i)`, or, with the overlay open, hands the chord
    /// to the overlay as a key (a chord bound there, e.g. Ctrl+Shift+S Save
    /// As, does its overlay job).
    Pressed(usize),
    /// Stop the recording in progress (the tray).
    StopRecording,
    /// The recording's pill, from the tray or a while-recording chord:
    /// pause/resume, mute the microphone, hide the camera bubble, hide
    /// the pill, discard.
    RecPause,
    RecMute,
    RecCamera,
    RecPill,
    RecDiscard,
    /// Close every pin (the tray's "Close all pins").
    ClosePins,
    /// The launcher (or the tray's "Capture in 3 s"): this capture,
    /// after its delay (counted down with a Cancel pill).
    Launch(crate::launcher::Plan),
    /// Call the delayed capture off (the tray's "Cancel capture").
    CancelDelay,
    /// The tray's "Delete … from Imgur…" (the upload's menu tag): ask first.
    UploadDelete(u32),
}

/// The chords registered only while a recording runs (Settings → Key
/// bindings → While recording): pause/resume, mute the microphone,
/// hide/show the pill. Stopping is the record hotkey (it toggles).
pub const REC_SLOTS: usize = 3;

/// The keymap action of each while-recording chord.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: not registered yet
pub const REC_ACTIONS: [crate::keymap::Action; REC_SLOTS] =
    [crate::keymap::Action::RecPause, crate::keymap::Action::RecMute, crate::keymap::Action::RecHidePill];

/// [`Failure::slot`] of the first while-recording chord (after the
/// config's [`SLOTS`]).
pub const REC_SLOT_BASE: usize = SLOTS;

/// The event of while-recording chord `i`.
pub fn rec_event(i: usize) -> HotEvent {
    match i {
        0 => HotEvent::RecPause,
        1 => HotEvent::RecMute,
        _ => HotEvent::RecPill,
    }
}

/// The while-recording chords `cfg` asks for (the `[shortcuts]` table;
/// "" = unbound), in [`REC_ACTIONS`] order.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn rec_wanted(cfg: &Config) -> [String; REC_SLOTS] {
    let km = crate::keymap::Keymap::from_config(&cfg.shortcuts).0;
    REC_ACTIONS.map(|a| km.chords(a).first().map(|c| c.display()).unwrap_or_default())
}

/// Registration specs of the while-recording chords: ids after the
/// config's, events of their own.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn rec_specs_for(s: &[String; REC_SLOTS]) -> [(i32, String, HotEvent); REC_SLOTS] {
    std::array::from_fn(|i| ((REC_SLOT_BASE + i) as i32 + 1, s[i].clone(), rec_event(i)))
}

/// The while-recording chords' registration: held only while a
/// recording runs. `W` is the registration (a worker thread; a fake in
/// tests): dropping it through `stop` unregisters the chords.
pub struct RecKeys<W> {
    held: Option<(W, [String; REC_SLOTS])>,
}

impl<W> Default for RecKeys<W> {
    fn default() -> Self {
        RecKeys { held: None }
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
impl<W> RecKeys<W> {
    /// Whether the chords are registered now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active(&self) -> bool {
        self.held.is_some()
    }

    /// A recording started (or its chords changed while it runs):
    /// register `want` with `start` (the registration and per chord whether
    /// it worked), after `stop`ping what was held. The failures, as
    /// [`Failure`]s with slots from [`REC_SLOT_BASE`].
    pub fn begin(
        &mut self,
        want: &[String; REC_SLOTS],
        start: impl FnOnce(&[String; REC_SLOTS]) -> (W, [Result<(), Cause>; REC_SLOTS]),
        stop: impl FnOnce(W),
    ) -> Vec<Failure> {
        if let Some((_, held)) = &self.held
            && held == want
        {
            return Vec::new();
        }
        self.end(stop);
        if want.iter().all(|s| s.trim().is_empty()) {
            return Vec::new(); // nothing to register
        }
        let (w, ok) = start(want);
        self.held = Some((w, want.clone()));
        (0..REC_SLOTS)
            .filter_map(|i| ok[i].err().map(|cause| Failure { slot: REC_SLOT_BASE + i, wanted: want[i].clone(), cause, ..Failure::default() }))
            .collect()
    }

    /// The recording is over: unregister the chords (nothing when none are held).
    pub fn end(&mut self, stop: impl FnOnce(W)) {
        if let Some((w, _)) = self.held.take() {
            stop(w);
        }
    }

    /// What is held (the chords asked for), while a recording runs.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn held(&self) -> Option<&[String; REC_SLOTS]> {
        self.held.as_ref().map(|(_, s)| s)
    }
}

/// Global hotkey slots: capture, quit, record, full screen, window,
/// repeat, copy text, History.
pub const SLOTS: usize = 8;

/// Global hotkeys backed by Win32 `RegisterHotKey` (replaces the
/// `global-hotkey` crate). Registration and the message loop run on a
/// dedicated thread so we own the queue instead of relying on any toolkit.
pub struct Hotkeys {
    rx: Receiver<HotEvent>,
    tx: Sender<HotEvent>,
    /// The registration thread (stopped and replaced by `reload`).
    #[cfg(not(target_os = "macos"))]
    worker: Option<imp::Worker>,
    /// The startup registration's result, not read yet.
    #[cfg(not(target_os = "macos"))]
    started: Option<Receiver<Registered>>,
    /// The specs registered now (per slot); "" where none is (or
    /// where it is not known, so the next reload tries again).
    specs: [String; SLOTS],
    /// Failures already told to the user (once per run each).
    #[cfg(not(target_os = "macos"))]
    told: Vec<Failure>,
    /// The startup registration's failures, not taken yet ([`Hotkeys::take_news`]).
    #[cfg(not(target_os = "macos"))]
    news: Vec<Failure>,
    /// The while-recording chords, registered only while a recording runs.
    #[cfg(not(target_os = "macos"))]
    rec: RecKeys<imp::Worker>,
}

/// Why a hotkey did not register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
pub enum Cause {
    /// Not a chord Rustshot can register (or a key this keyboard lacks).
    Invalid,
    /// No X display to grab keys on (Linux).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    NoDisplay,
    /// Another app holds it.
    InUse,
    /// The registration thread did not answer in time.
    #[default]
    Unknown,
}

/// Per slot: registered (or nothing to register), or why not.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
pub type Registered = [Result<(), Cause>; SLOTS];

/// The hotkeys that are not registered as saved, for the Settings window
/// (published by the daemon after every reload).
static OUTSTANDING: std::sync::Mutex<Vec<Failure>> = std::sync::Mutex::new(Vec::new());

/// The hotkeys the daemon could not register as saved (none in a process
/// without one).
pub fn outstanding() -> Vec<Failure> {
    OUTSTANDING.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
fn publish(f: &[Failure]) {
    *OUTSTANDING.lock().unwrap_or_else(|e| e.into_inner()) = f.to_vec();
}

/// How long `reload` waits for a registration thread's result.
#[cfg(not(target_os = "macos"))]
const RESULT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// The event of each slot.
pub fn slot_event(i: usize) -> HotEvent {
    match i {
        0 => HotEvent::Capture,
        1 => HotEvent::Quit,
        2 => HotEvent::Record,
        3 => HotEvent::CaptureFull,
        4 => HotEvent::CaptureWindow,
        5 => HotEvent::RepeatLast,
        6 => HotEvent::CaptureText,
        7 => HotEvent::History,
        i => rec_event(i - REC_SLOT_BASE),
    }
}

/// Registration specs per slot: ids 1..=SLOTS.
fn specs_for(s: &[String; SLOTS]) -> [(i32, String, HotEvent); SLOTS] {
    std::array::from_fn(|i| (i as i32 + 1, s[i].clone(), HotEvent::Pressed(i)))
}

/// The chord each slot wants (trimmed; "" = none), in slot order.
pub fn wanted(cfg: &Config) -> [String; SLOTS] {
    [
        &cfg.capture_hotkey,
        &cfg.quit_hotkey,
        &cfg.record_hotkey,
        &cfg.capture_full_hotkey,
        &cfg.capture_window_hotkey,
        &cfg.repeat_hotkey,
        &cfg.capture_text_hotkey,
        &cfg.history_hotkey,
    ]
    .map(|s| s.trim().to_string())
}

/// What each slot is called (as Settings › Key bindings lists it).
pub const SLOT_NAMES: [&str; SLOTS] = [
    "Capture region",
    "Quit Rustshot",
    "Record video",
    "Capture full screen",
    "Capture window",
    "Repeat last capture",
    "Copy text from area",
    "Open History",
];

/// What each while-recording chord is called (Settings › Key bindings).
pub const REC_NAMES: [&str; REC_SLOTS] = ["Pause / resume recording", "Mute microphone", "Hide / show recording controls"];

/// The name of slot `slot` (a config slot or a while-recording one).
pub fn slot_name(slot: usize) -> &'static str {
    match slot {
        s if s < SLOTS => SLOT_NAMES[s],
        s => REC_NAMES.get(s - REC_SLOT_BASE).copied().unwrap_or("hotkey"),
    }
}

/// The current default chord of each slot ("" = none).
pub fn defaults() -> [String; SLOTS] {
    wanted(&Config::default())
}

/// The keys a slot still on its default falls back to, in order, when
/// another app holds the default (from the Win+Shift keys free on a clean
/// Windows: E Z Q G J Y, minus our own defaults where they would clash;
/// a key another slot wants or holds is skipped).
pub fn alternates(slot: usize) -> &'static [&'static str] {
    match slot {
        0 => &["Shift+Meta+Q", "Shift+Meta+J", "Shift+Meta+Y"],
        2 => &["Shift+Meta+J", "Shift+Meta+Y", "Shift+Meta+Q"],
        3 => &["Shift+Meta+Y", "Shift+Meta+J"],
        4 => &["Shift+Meta+J", "Shift+Meta+Y", "Shift+Meta+Q"],
        6 => &["Shift+Meta+Q", "Shift+Meta+Y", "Shift+Meta+J"],
        7 => &["Shift+Meta+J", "Shift+Meta+Y", "Shift+Meta+Q"],
        _ => &[],
    }
}

/// The keys probed for a "… is free" suggestion when a hotkey stays
/// unregistered (the first that registers now and no slot wants).
pub const FREE_POOL: [&str; 6] = ["Shift+Meta+Y", "Shift+Meta+J", "Shift+Meta+Q", "Shift+Meta+E", "Shift+Meta+Z", "Shift+Meta+G"];

/// A hotkey `reload` could not register.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Failure {
    /// The slot: 0 = capture, 1 = quit, 2 = record, 3 =
    /// full screen, 4 = window, 5 = repeat, 6 = copy text, 7 = History.
    pub slot: usize,
    pub wanted: String,
    pub cause: Cause,
    /// The previous chord, registered again in its place.
    pub kept: Option<String>,
    /// The slot is on its default and another app holds it: this
    /// alternate ([`alternates`]) is registered instead (the config is not
    /// changed, so the default comes back once it is free).
    pub alt: Option<String>,
    /// Nothing is registered for the slot: a key that registers now
    /// (probed), suggested in the message.
    pub free: Option<String>,
}

impl Failure {
    /// "Win+Shift+F is in use by another app — Capture full screen is on
    /// Win+Shift+Y instead.", "Win+Shift+F is used by another app. Pick
    /// another key: …", "Couldn't register Ctrl+Alt+F9 — it's in use by
    /// another app; kept Shift+Win+X."
    pub fn message(&self) -> String {
        let chord = chord_label(&self.wanted);
        let name = slot_name(self.slot);
        if let Some(a) = &self.alt {
            return format!("{chord} is in use by another app \u{2014} {name} is on {} instead.", chord_label(a));
        }
        let pick = || {
            let mut s = format!("Pick another key: click Change shortcuts\u{2026}, click the {name} key and press a new one");
            if let Some(f) = &self.free {
                s.push_str(&format!(" ({} is free)", chord_label(f)));
            }
            s.push('.');
            s
        };
        let why = match self.cause {
            Cause::Unknown => return format!("Couldn't confirm {chord} was registered."),
            Cause::InUse if self.kept.is_none() => return format!("{chord} is used by another app. {}", pick()),
            Cause::Invalid if self.kept.is_none() => {
                return format!("Couldn't register {chord} \u{2014} it's not a hotkey Rustshot can register. {}", pick());
            }
            Cause::Invalid => "it's not a hotkey Rustshot can register",
            Cause::NoDisplay => "no X display is available",
            Cause::InUse => "it's in use by another app",
        };
        let mut s = format!("Couldn't register {chord} — {why}");
        if let Some(k) = &self.kept {
            s.push_str(&format!("; kept {}", chord_label(k)));
        }
        s.push('.');
        s
    }

    /// Nothing is registered for the slot and the user has to pick
    /// another key (another app holds it, or it is not a valid hotkey).
    pub fn needs_action(&self) -> bool {
        matches!(self.cause, Cause::InUse | Cause::Invalid) && self.kept.is_none() && self.alt.is_none()
    }

    /// The id of this slot's key field in Settings › Key bindings.
    pub fn field_id(&self) -> String {
        match self.slot {
            s if s < SLOTS => format!("global{s}"),
            s => REC_ACTIONS.get(s - REC_SLOT_BASE).map_or_else(String::new, |a| format!("key_{}", a.id())),
        }
    }

    /// The same slot, chord and cause (what was kept does not matter).
    #[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
    fn same(&self, o: &Failure) -> bool {
        self.slot == o.slot && self.cause == o.cause && same_chord(&self.wanted, &o.wanted)
    }
}

/// The failures in `now` not told yet (each slot, chord and cause once per
/// run); they are added to `told`.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
pub fn untold(told: &mut Vec<Failure>, now: &[Failure]) -> Vec<Failure> {
    let new: Vec<Failure> = now.iter().filter(|f| !told.iter().any(|t| t.same(f))).cloned().collect();
    told.extend(new.iter().cloned());
    new
}

/// `spec` as the Settings window shows it (Win/Super/Cmd for Meta); left
/// as written when it does not parse.
pub fn chord_label(spec: &str) -> String {
    crate::keymap::Chord::parse(spec).map_or_else(|| spec.trim().to_string(), |c| c.label())
}

/// Move from the registered `old` specs to `want`. `register` replaces
/// every registration with the specs it is given and says which worked.
/// A slot that failed for a known cause gets its old chord back (unless a
/// working slot took that chord); one whose result is unknown (no answer)
/// is left empty, with no fallback. Afterwards a slot holds its spec only
/// where the registration is confirmed, so the next reload retries the
/// others. Returns the specs registered now and the failures.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
pub fn plan_reload(
    old: &[String; SLOTS],
    want: &[String; SLOTS],
    mut register: impl FnMut(&[String; SLOTS]) -> Registered,
) -> ([String; SLOTS], Vec<Failure>) {
    if want == old {
        return (old.clone(), Vec::new());
    }
    let first = register(want);
    settle(old, want, first, register)
}

/// The rest of [`plan_reload`] once `want` was registered with result
/// `first` (also the daemon's startup registration, with no `old`):
/// slots on their default that another app holds fall back
/// ([`fall_back`]), other known failures get their `old` chord back, and
/// a slot left with nothing because another app holds its key gets a
/// free key probed for the message ([`probe_free`]).
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
pub fn settle(
    old: &[String; SLOTS],
    want: &[String; SLOTS],
    first: Registered,
    mut register: impl FnMut(&[String; SLOTS]) -> Registered,
) -> ([String; SLOTS], Vec<Failure>) {
    let known = |r: &Result<(), Cause>| matches!(r, Err(c) if *c != Cause::Unknown);
    let (mut tried, mut last, alt) = fall_back(want, first, &mut register);
    if last.iter().any(known) {
        let retry: [String; SLOTS] = std::array::from_fn(|i| {
            if !known(&last[i]) {
                tried[i].clone()
            } else if (0..SLOTS).any(|j| last[j].is_ok() && j != i && !tried[j].is_empty() && same_chord(&tried[j], &old[i])) {
                String::new() // the other slot holds that chord now
            } else {
                old[i].clone()
            }
        });
        if retry != tried {
            last = register(&retry);
            tried = retry;
        }
    }
    let now = held(&tried, last);
    let mut failures: Vec<Failure> = (0..SLOTS)
        .filter_map(|i| {
            // A known cause from the first try, else what the last one says.
            let cause = match (first[i], last[i]) {
                (Err(c), _) if c != Cause::Unknown => c,
                (_, Err(c)) => c,
                _ => return None,
            };
            let other = (!now[i].is_empty() && now[i] != want[i]).then(|| now[i].clone());
            let fell = other.is_some() && alt[i].as_deref() == other.as_deref();
            Some(Failure {
                slot: i,
                wanted: want[i].clone(),
                cause,
                kept: if fell { None } else { other.clone() },
                alt: if fell { other } else { None },
                free: None,
            })
        })
        .collect();
    let need: Vec<usize> = failures.iter().filter(|f| f.cause == Cause::InUse && now[f.slot].is_empty()).map(|f| f.slot).collect();
    let free = probe_free(want, &now, &need, &mut register);
    for f in &mut failures {
        f.free = free[f.slot].clone();
    }
    (now, failures)
}

/// Each slot still on its default ([`defaults`]) that another app holds
/// tries its [`alternates`] in turn (skipping a key another slot wants or
/// tries), every round one registration of all slots, until each such
/// slot registered or ran out. Returns what was tried last, its result,
/// and per slot the alternate it registered on.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
fn fall_back(
    want: &[String; SLOTS],
    first: Registered,
    register: &mut impl FnMut(&[String; SLOTS]) -> Registered,
) -> ([String; SLOTS], Registered, [Option<String>; SLOTS]) {
    let def = defaults();
    let (mut tried, mut ok) = (want.clone(), first);
    let mut next = [0usize; SLOTS];
    loop {
        let mut moved = false;
        for i in 0..SLOTS {
            if ok[i] != Err(Cause::InUse) || def[i].is_empty() || !same_chord(&want[i], &def[i]) {
                continue;
            }
            let alts = alternates(i);
            let mut pick = None;
            while let Some(c) = alts.get(next[i]) {
                next[i] += 1;
                if !(0..SLOTS).any(|j| j != i && (same_chord(&want[j], c) || same_chord(&tried[j], c))) {
                    pick = Some(*c);
                    break;
                }
            }
            match pick {
                Some(c) => {
                    tried[i] = c.to_string();
                    moved = true;
                }
                None => tried[i] = want[i].clone(), // ran out: the default, not registered
            }
        }
        if !moved {
            break;
        }
        ok = register(&tried);
    }
    let alt = std::array::from_fn(|i| (ok[i].is_ok() && tried[i] != want[i]).then(|| tried[i].clone()));
    (tried, ok, alt)
}

/// For each slot in `need` (nothing registered: another app holds its
/// key): a key of [`FREE_POOL`] that registers now and that no slot wants
/// or holds. Probed by registering `now` with the candidates in those
/// slots (a round per candidate), then `now` again.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
fn probe_free(
    want: &[String; SLOTS],
    now: &[String; SLOTS],
    need: &[usize],
    register: &mut impl FnMut(&[String; SLOTS]) -> Registered,
) -> [Option<String>; SLOTS] {
    let mut free: [Option<String>; SLOTS] = Default::default();
    let mut next = [0usize; SLOTS];
    let mut pending: Vec<usize> = need.to_vec();
    let mut probed = false;
    while !pending.is_empty() {
        let mut probe = now.clone();
        let mut trying = Vec::new();
        for &i in &pending {
            while let Some(c) = FREE_POOL.get(next[i]) {
                next[i] += 1;
                let busy = (0..SLOTS).any(|j| same_chord(&want[j], c) || same_chord(&now[j], c))
                    || free.iter().flatten().any(|f| same_chord(f, c))
                    || trying.iter().any(|&t: &usize| same_chord(&probe[t], c));
                if !busy {
                    probe[i] = c.to_string();
                    trying.push(i);
                    break;
                }
            }
        }
        if trying.is_empty() {
            break;
        }
        let ok = register(&probe);
        probed = true;
        for &i in &trying {
            if ok[i].is_ok() {
                free[i] = Some(probe[i].clone());
            }
        }
        pending.retain(|&i| free[i].is_none() && next[i] < FREE_POOL.len());
    }
    if probed {
        let _ = register(now); // back to what is held
    }
    free
}

/// The specs of `tried` that registered.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
fn held(tried: &[String; SLOTS], ok: Registered) -> [String; SLOTS] {
    std::array::from_fn(|i| if ok[i].is_ok() { tried[i].clone() } else { String::new() })
}

/// What a registration thread that did not answer means: every slot with
/// a chord is unknown.
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
fn unanswered<const N: usize>(specs: &[String; N]) -> [Result<(), Cause>; N] {
    std::array::from_fn(|i| if specs[i].trim().is_empty() { Ok(()) } else { Err(Cause::Unknown) })
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS: hotkeys apply after a restart
fn same_chord(a: &str, b: &str) -> bool {
    match (parse_hotkey(a), parse_hotkey(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim().eq_ignore_ascii_case(b.trim()),
    }
}

impl Hotkeys {
    pub fn new(cfg: &Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let now = wanted(cfg);
        let specs = specs_for(&now);
        #[cfg(not(target_os = "macos"))]
        let (worker, started) = {
            let (done, started) = mpsc::channel();
            (Some(imp::Worker::start(specs, tx.clone(), done)), Some(started))
        };
        #[cfg(target_os = "macos")]
        {
            let hot_tx = tx.clone();
            std::thread::spawn(move || imp::hotkey_thread(specs, hot_tx));
        }
        Self {
            rx,
            tx,
            #[cfg(not(target_os = "macos"))]
            worker,
            #[cfg(not(target_os = "macos"))]
            started,
            specs: now,
            #[cfg(not(target_os = "macos"))]
            told: Vec::new(),
            #[cfg(not(target_os = "macos"))]
            news: Vec::new(),
            #[cfg(not(target_os = "macos"))]
            rec: RecKeys::default(),
        }
    }

    /// A recording started (`on`) or ended: register the while-recording
    /// chords of `cfg` (pause, mute, hide pill) or release them. Also
    /// after a config reload while recording (changed chords register
    /// again). Returns the failures not told to the user yet in this run.
    /// macOS: not supported yet (its hotkeys register once, at start).
    pub fn set_recording(&mut self, cfg: &Config, on: bool) -> Vec<Failure> {
        #[cfg(not(target_os = "macos"))]
        {
            if !on {
                self.rec.end(imp::Worker::stop);
                return Vec::new();
            }
            let tx = self.tx.clone();
            let failures = self.rec.begin(
                &rec_wanted(cfg),
                |want| {
                    let (done, result) = mpsc::channel();
                    let w = imp::Worker::start(rec_specs_for(want), tx, done);
                    let ok = result.recv_timeout(RESULT_WAIT).unwrap_or_else(|_| unanswered(want));
                    (w, ok)
                },
                imp::Worker::stop,
            );
            untold(&mut self.told, &failures)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = (cfg, on);
            Vec::new()
        }
    }

    /// Take the startup registration's result once it is there (never
    /// waits): what did not register is left empty, so the first reload
    /// tries it again, and shows in the Settings window meanwhile.
    #[cfg(not(target_os = "macos"))]
    fn check_started(&mut self) {
        let Some(started) = &self.started else { return };
        let ok = match started.try_recv() {
            Ok(ok) => ok,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => unanswered(&self.specs),
        };
        self.started = None;
        // A default another app holds falls back; the rest is told.
        let want = self.specs.clone();
        let none: [String; SLOTS] = Default::default();
        let (now, failed) = if ok.contains(&Err(Cause::InUse)) {
            settle(&none, &want, ok, |s| self.restart(s))
        } else {
            let failed = (0..SLOTS)
                .filter_map(|i| ok[i].err().map(|cause| Failure { slot: i, wanted: want[i].clone(), cause, ..Failure::default() }))
                .collect();
            (held(&want, ok), failed)
        };
        self.specs = now;
        publish(&failed);
        let new = untold(&mut self.told, &failed);
        self.news.extend(new);
    }

    /// The startup registration's failures not told yet (once each); the
    /// daemon tells them like a reload's.
    pub fn take_news(&mut self) -> Vec<Failure> {
        #[cfg(not(target_os = "macos"))]
        return std::mem::take(&mut self.news);
        #[cfg(target_os = "macos")]
        Vec::new()
    }

    /// Register `cfg`'s hotkeys instead of the current ones (no-op when
    /// they are unchanged). The old registrations are released first, so
    /// a chord can move between capture and quit. A chord that cannot be
    /// registered keeps the previous one ([`plan_reload`]). Returns the
    /// failures not told to the user yet in this run ([`untold`]); every
    /// current one is published for the Settings window ([`outstanding`]).
    pub fn reload(&mut self, cfg: &Config) -> Vec<Failure> {
        let want = wanted(cfg);
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(started) = self.started.take() {
                let ok = started.recv_timeout(RESULT_WAIT).unwrap_or_else(|_| unanswered(&self.specs));
                self.specs = held(&self.specs, ok);
            }
            let old = self.specs.clone();
            let (now, failures) = plan_reload(&old, &want, |s| self.restart(s));
            publish(&failures);
            self.specs = now;
            untold(&mut self.told, &failures)
        }
        #[cfg(target_os = "macos")]
        {
            if want != self.specs {
                eprintln!("rustshot: new global hotkeys take effect after a restart");
            }
            Vec::new()
        }
    }

    /// Replace the registration thread with one for `specs`; its result.
    #[cfg(not(target_os = "macos"))]
    fn restart(&mut self, specs: &[String; SLOTS]) -> Registered {
        if let Some(w) = self.worker.take() {
            w.stop();
        }
        let (done, result) = mpsc::channel();
        self.worker = Some(imp::Worker::start(specs_for(specs), self.tx.clone(), done));
        // No answer in time: unknown (left empty, so the next reload retries).
        result.recv_timeout(RESULT_WAIT).unwrap_or_else(|_| unanswered(specs))
    }

    /// Another producer of events (single-instance listener, tray).
    pub fn sender(&self) -> Sender<HotEvent> {
        self.tx.clone()
    }

    /// The next queued event, if any.
    pub fn poll(&mut self) -> Option<HotEvent> {
        #[cfg(not(target_os = "macos"))]
        self.check_started();
        self.rx.try_recv().ok()
    }
}

#[cfg(windows)]
#[path = "hotkey_win.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "hotkey_linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "hotkey_macos.rs"]
mod imp;

/// Parse things like `Shift+Meta+X`, `Ctrl+Alt+Shift+Q`, `PrintScreen`.
/// Returns `(modifier flags, virtual-key code)` for `RegisterHotKey`.
pub fn parse_hotkey(spec: &str) -> Option<(u32, u32)> {
    // Win32 modifier flags: MOD_ALT=1, MOD_CONTROL=2, MOD_SHIFT=4, MOD_WIN=8.
    let mut mods = 0u32;
    let mut vk: Option<u32> = None;
    for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => {
                mods |= 0x0002;
                continue;
            }
            "shift" => {
                mods |= 0x0004;
                continue;
            }
            "alt" => {
                mods |= 0x0001;
                continue;
            }
            "meta" | "win" | "super" | "cmd" => {
                mods |= 0x0008;
                continue;
            }
            _ => {}
        }
        if vk.is_some() {
            return None; // two non-modifier keys ("A+B")
        }
        vk = Some(key_vk(&lower)?);
    }
    Some((mods, vk?))
}

/// Key name (lowercase) → virtual-key code; the one key table shared by
/// global hotkeys and the editor keymap.
fn key_vk(lower: &str) -> Option<u32> {
    if lower.len() == 1 {
        let c = lower.chars().next().unwrap();
        if c.is_ascii_lowercase() {
            // VK_A..VK_Z = 0x41..0x5A.
            return Some(0x41 + (c as u32 - 'a' as u32));
        }
        if c.is_ascii_digit() {
            // VK_0..VK_9 = 0x30..0x39.
            return Some(0x30 + (c as u32 - '0' as u32));
        }
    }
    if let Some(rest) = lower.strip_prefix('f')
        && let Ok(n) = rest.parse::<u32>()
        && (1..=12).contains(&n)
    {
        return Some(0x70 + n - 1); // VK_F1..VK_F12
    }
    let vk = match lower {
        "space" => key::SPACE,
        "enter" | "return" => key::RETURN,
        "esc" | "escape" => key::ESCAPE,
        "tab" => key::TAB,
        "backspace" => key::BACK,
        "delete" | "del" => key::DELETE,
        "insert" => key::INSERT,
        "home" => key::HOME,
        "end" => key::END,
        "pageup" => key::PAGEUP,
        "pagedown" => key::PAGEDOWN,
        "up" => key::UP,
        "down" => key::DOWN,
        "left" => key::LEFT,
        "right" => key::RIGHT,
        "printscreen" | "prtsc" | "print" => key::PRINTSCREEN,
        "=" | "equals" | "plus" => key::EQUALS,
        "-" | "minus" => key::MINUS,
        "[" | "bracketleft" => key::LBRACKET,
        "]" | "bracketright" => key::RBRACKET,
        "numpadadd" => key::ADD,
        "numpadsubtract" => key::SUBTRACT,
        _ => {
            let n = lower.strip_prefix("numpad")?.parse::<u32>().ok().filter(|n| *n <= 9)?;
            key::NUMPAD0 + n
        }
    };
    Some(vk)
}

/// Display name of a virtual key in the chord syntax `parse_hotkey`
/// accepts ("S", "5", "F5", "Del", "Space", ...).
pub fn key_name(vk: u32) -> Option<String> {
    if (0x41..=0x5A).contains(&vk) || (0x30..=0x39).contains(&vk) {
        return Some(char::from_u32(vk)?.to_string());
    }
    if (0x70..=0x7B).contains(&vk) {
        return Some(format!("F{}", vk - 0x70 + 1));
    }
    let name = match vk {
        key::SPACE => "Space",
        key::RETURN => "Enter",
        key::ESCAPE => "Esc",
        key::TAB => "Tab",
        key::BACK => "Backspace",
        key::DELETE => "Del",
        key::INSERT => "Insert",
        key::HOME => "Home",
        key::END => "End",
        key::PAGEUP => "PageUp",
        key::PAGEDOWN => "PageDown",
        key::UP => "Up",
        key::DOWN => "Down",
        key::LEFT => "Left",
        key::RIGHT => "Right",
        key::PRINTSCREEN => "PrintScreen",
        key::EQUALS => "=",
        key::MINUS => "-",
        key::LBRACKET => "[",
        key::RBRACKET => "]",
        key::ADD => "NumpadAdd",
        key::SUBTRACT => "NumpadSubtract",
        k if (key::NUMPAD0..=key::NUMPAD0 + 9).contains(&k) => return Some(format!("Numpad{}", k - key::NUMPAD0)),
        _ => return None,
    };
    Some(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_specs() {
        assert_eq!(parse_hotkey("Shift+Meta+X"), Some((0x0008 | 0x0004, 0x58)));
        assert_eq!(
            parse_hotkey("Ctrl+Alt+Shift+Q"),
            Some((0x0002 | 0x0001 | 0x0004, 0x51))
        );
        assert_eq!(parse_hotkey("PrintScreen"), Some((0, 0x2C)));
        assert_eq!(parse_hotkey("F5"), Some((0, 0x74)));
        assert_eq!(parse_hotkey("bogus+key"), None);
        assert_eq!(parse_hotkey(""), None);
        assert_eq!(parse_hotkey("A+B"), None);
        assert_eq!(parse_hotkey("Shift"), None);
    }

    fn s2(a: &str, b: &str) -> [String; SLOTS] {
        std::array::from_fn(|i| match i {
            0 => a.to_string(),
            1 => b.to_string(),
            _ => String::new(),
        })
    }

    /// `r` for the first two slots, Ok for the rest.
    fn r2(a: Result<(), Cause>, b: Result<(), Cause>) -> Registered {
        std::array::from_fn(|i| match i {
            0 => a,
            1 => b,
            _ => Ok(()),
        })
    }

    /// A fake registrar: chords in `taken` belong to another app.
    fn registrar<'a>(taken: &'a [&'a str], calls: &'a mut Vec<[String; SLOTS]>) -> impl FnMut(&[String; SLOTS]) -> Registered + 'a {
        move |s: &[String; SLOTS]| {
            calls.push(s.clone());
            std::array::from_fn(|i| {
                if s[i].is_empty() || !taken.iter().any(|t| same_chord(t, &s[i])) { Ok(()) } else { Err(Cause::InUse) }
            })
        }
    }

    fn fail(slot: usize, wanted: &str, cause: Cause, kept: Option<&str>) -> Failure {
        Failure { slot, wanted: wanted.into(), cause, kept: kept.map(Into::into), ..Failure::default() }
    }

    #[test]
    fn reload_plan_unchanged_registers_nothing() {
        let mut calls = Vec::new();
        let old = s2("Shift+Meta+X", "Ctrl+Alt+Shift+Q");
        let (now, f) = plan_reload(&old, &old.clone(), registrar(&[], &mut calls));
        assert_eq!((now, f), (old, vec![]));
        assert!(calls.is_empty());
    }

    #[test]
    fn reload_plan_success_and_swap() {
        let mut calls = Vec::new();
        let old = s2("Shift+Meta+X", "Ctrl+Alt+Shift+Q");
        // Capture and quit trade places: one registration, both work.
        let want = s2("Ctrl+Alt+Shift+Q", "Shift+Meta+X");
        let (now, f) = plan_reload(&old, &want, registrar(&[], &mut calls));
        assert_eq!((now, f), (want.clone(), vec![]));
        assert_eq!(calls, [want]);
    }

    #[test]
    fn reload_plan_failure_keeps_the_old_chord() {
        let mut calls = Vec::new();
        let old = s2("Shift+Meta+X", "Ctrl+Alt+Shift+Q");
        let want = s2("Ctrl+Alt+F9", "Ctrl+Alt+Shift+Q");
        let (now, f) = plan_reload(&old, &want, registrar(&["ctrl+alt+f9"], &mut calls));
        assert_eq!(now, old, "the old capture chord is back");
        assert_eq!(calls, [want.clone(), old.clone()]);
        assert_eq!(f, [fail(0, "Ctrl+Alt+F9", Cause::InUse, Some("Shift+Meta+X"))]);
        #[cfg(windows)]
        assert_eq!(f[0].message(), "Couldn't register Ctrl+Alt+F9 — it's in use by another app; kept Shift+Win+X.");
        // Saving the same settings again retries (the specs differ).
        let mut calls = Vec::new();
        let (again, _) = plan_reload(&now, &want, registrar(&[], &mut calls));
        assert_eq!((again, calls.len()), (want, 1));
    }

    #[test]
    fn reload_plan_without_a_fallback() {
        // No previous chord, or the previous one is gone too: the slot is
        // left empty (so a later save retries) and nothing is "kept".
        let mut calls = Vec::new();
        let old = s2("", "Ctrl+Alt+Shift+Q");
        let want = s2("Ctrl+Alt+F9", "Ctrl+Alt+Shift+Q");
        let (now, f) = plan_reload(&old, &want, registrar(&["Ctrl+Alt+F9"], &mut calls));
        assert_eq!(now, s2("", "Ctrl+Alt+Shift+Q"));
        assert_eq!(f[0].kept, None);
        assert_eq!(f[0].free.as_deref(), Some("Shift+Meta+Y"), "a free key is probed");
        assert_eq!(
            f[0].message(),
            format!("Ctrl+Alt+F9 is used by another app. Pick another key: click Change shortcuts\u{2026}, click the Capture region key and press a new one ({} is free).", chord_label("Shift+Meta+Y"))
        );
        let mut calls = Vec::new();
        let old = s2("Shift+Meta+X", "Ctrl+Alt+Shift+Q");
        let (now, f) = plan_reload(&old, &want, registrar(&["Ctrl+Alt+F9", "Meta+Shift+X"], &mut calls));
        assert_eq!(now, s2("", "Ctrl+Alt+Shift+Q"));
        assert_eq!(f[0].kept, None);
        // The quit slot took the old capture chord: capture is not given
        // it back (it would knock quit out).
        let mut calls = Vec::new();
        let want = s2("Ctrl+Alt+F9", "Shift+Meta+X");
        let (now, f) = plan_reload(&old, &want, registrar(&["Ctrl+Alt+F9"], &mut calls));
        assert_eq!(now, s2("", "Shift+Meta+X"));
        assert_eq!(calls[..2], [want.clone(), s2("", "Shift+Meta+X")]);
        // Then the probe for a free key, and what is held again.
        assert_eq!(calls.last(), Some(&now));
        assert_eq!((f.len(), f[0].kept.clone()), (1, None));
    }

    /// No answer from the registration thread: the result is unknown, the
    /// slots are left empty (the next reload retries), nothing falls back,
    /// and the user is told it is not confirmed.
    #[test]
    fn reload_plan_timeout_is_unknown() {
        let old = s2("Shift+Meta+X", "Ctrl+Alt+Shift+Q");
        let want = s2("Ctrl+Alt+F9", "");
        let mut calls = 0;
        let (now, f) = plan_reload(&old, &want, |s| {
            calls += 1;
            unanswered(s)
        });
        assert_eq!((now.clone(), calls), (s2("", ""), 1), "no fallback registration");
        assert_eq!(f, [fail(0, "Ctrl+Alt+F9", Cause::Unknown, None)]);
        assert_eq!(f[0].message(), "Couldn't confirm Ctrl+Alt+F9 was registered.");
        // The next reload (an unrelated save) tries again, and it works.
        let mut calls = Vec::new();
        let (again, f) = plan_reload(&now, &want, registrar(&[], &mut calls));
        assert_eq!((again, f, calls.len()), (want.clone(), vec![], 1));
        // One slot fails for a known cause, the retry gets no answer: both
        // are left empty; the known cause is the one reported.
        let want = s2("Ctrl+Alt+F9", "Ctrl+Alt+F10");
        let mut n = 0;
        let (now, f) = plan_reload(&old, &want, |s| {
            n += 1;
            if n == 1 { r2(Err(Cause::InUse), Ok(())) } else { unanswered(s) }
        });
        assert_eq!(now, s2("", ""));
        assert_eq!(f, [fail(0, "Ctrl+Alt+F9", Cause::InUse, None), fail(1, "Ctrl+Alt+F10", Cause::Unknown, None)]);
    }

    #[test]
    fn failure_causes_have_their_own_words() {
        let m = |c| fail(1, "Ctrl+Alt+F9", c, Some("Ctrl+Alt+Shift+Q")).message();
        assert_eq!(m(Cause::InUse), "Couldn't register Ctrl+Alt+F9 — it's in use by another app; kept Ctrl+Alt+Shift+Q.");
        assert_eq!(m(Cause::Invalid), "Couldn't register Ctrl+Alt+F9 — it's not a hotkey Rustshot can register; kept Ctrl+Alt+Shift+Q.");
        assert_eq!(m(Cause::NoDisplay), "Couldn't register Ctrl+Alt+F9 — no X display is available; kept Ctrl+Alt+Shift+Q.");
        assert_eq!(m(Cause::Unknown), "Couldn't confirm Ctrl+Alt+F9 was registered.");
        // An invalid chord is reported as such, left as written.
        let (_, f) = plan_reload(&s2("", ""), &s2("Hyper+X", ""), |s| {
            r2(if s[0].is_empty() { Ok(()) } else { Err(Cause::Invalid) }, Ok(()))
        });
        assert_eq!(
            f[0].message(),
            "Couldn't register Hyper+X \u{2014} it's not a hotkey Rustshot can register. Pick another key: click Change shortcuts\u{2026}, click the Capture region key and press a new one."
        );
    }

    /// A failure is told once per run: an unrelated save that hits the
    /// same (slot, chord, cause) again stays quiet; another cause, chord
    /// or slot is new.
    #[test]
    fn failures_are_told_once() {
        let mut told = Vec::new();
        let a = fail(0, "Ctrl+Alt+F9", Cause::InUse, Some("Shift+Meta+X"));
        assert_eq!(untold(&mut told, std::slice::from_ref(&a)), std::slice::from_ref(&a));
        assert_eq!(untold(&mut told, std::slice::from_ref(&a)), []);
        // Spelled differently, kept something else: still the same failure.
        let a2 = fail(0, "ctrl+alt+f9", Cause::InUse, None);
        assert_eq!(untold(&mut told, &[a2]), []);
        let b = fail(0, "Ctrl+Alt+F9", Cause::Unknown, None);
        let c = fail(1, "Ctrl+Alt+F9", Cause::InUse, None);
        let d = fail(0, "Ctrl+Alt+F10", Cause::InUse, None);
        assert_eq!(untold(&mut told, &[a, b.clone(), c.clone(), d.clone()]), [b, c, d]);
    }

    /// The record hotkey is the third slot: it registers like the others
    /// and a failure names it.
    #[test]
    fn record_hotkey_is_a_slot() {
        let cfg = Config { record_hotkey: " Ctrl+Alt+R ".into(), ..Config::default() };
        let w = wanted(&cfg);
        assert_eq!(w[2], "Ctrl+Alt+R");
        let specs = specs_for(&w);
        assert_eq!(specs[2].0, 3);
        assert!(matches!(specs[2].2, HotEvent::Pressed(2)) && matches!(slot_event(2), HotEvent::Record));
        let mut calls = Vec::new();
        let old = s2("Shift+Meta+X", "");
        let want = w.clone();
        let (now, f) = plan_reload(&old, &want, registrar(&["Ctrl+Alt+R"], &mut calls));
        assert_eq!(now[2], "");
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].slot, f[0].cause), (2, Cause::InUse));
    }

    /// The redesign's global actions are slots 3-5 with their own events
    /// and config keys; a failure on one names it.
    #[test]
    fn new_global_actions_are_slots() {
        let cfg = Config { repeat_hotkey: "Ctrl+Alt+P".into(), ..Config::default() };
        let w = wanted(&cfg);
        assert_eq!(w, ["Shift+Meta+X", "Ctrl+Alt+Shift+Q", "Shift+Meta+G", "Shift+Meta+F", "Shift+Meta+E", "Ctrl+Alt+P", "Shift+Meta+Z", "Shift+Meta+H"]);
        let specs = specs_for(&w);
        for (i, (id, _, ev)) in specs.iter().enumerate() {
            assert!(*id == i as i32 + 1 && matches!(ev, HotEvent::Pressed(j) if *j == i));
        }
        assert!(matches!(slot_event(3), HotEvent::CaptureFull));
        assert!(matches!(slot_event(4), HotEvent::CaptureWindow));
        assert!(matches!(slot_event(5), HotEvent::RepeatLast));
        assert!(matches!(slot_event(6), HotEvent::CaptureText));
        assert!(matches!(slot_event(7), HotEvent::History));
        let mut calls = Vec::new();
        let (now, f) = plan_reload(&s2("", ""), &w, registrar(&["Shift+Meta+E"], &mut calls));
        assert_eq!(now[4], "Shift+Meta+J", "the default fell back");
        assert_eq!((f.len(), f[0].slot, f[0].cause, f[0].alt.as_deref()), (1, 4, Cause::InUse, Some("Shift+Meta+J")));
    }

    /// The while-recording chords come from the keymap (`[shortcuts]`),
    /// with ids after the config's slots and their own events.
    #[test]
    fn rec_chords_are_the_keymap_rows() {
        let cfg = Config::default();
        assert_eq!(rec_wanted(&cfg), ["Ctrl+Alt+Shift+P", "Ctrl+Alt+Shift+M", "Ctrl+Alt+Shift+H"]);
        let mut cfg = Config::default();
        cfg.shortcuts.insert("rec_pause".into(), "Ctrl+Alt+P".into());
        cfg.shortcuts.insert("rec_mute".into(), "".into());
        assert_eq!(rec_wanted(&cfg), ["Ctrl+Alt+P", "", "Ctrl+Alt+Shift+H"]);
        let specs = rec_specs_for(&rec_wanted(&cfg));
        assert_eq!(specs.clone().map(|s| s.0), [9, 10, 11]);
        assert!(matches!(specs[0].2, HotEvent::RecPause));
        assert!(matches!(specs[1].2, HotEvent::RecMute));
        assert!(matches!(specs[2].2, HotEvent::RecPill));
        for s in specs {
            assert!(s.1.is_empty() || parse_hotkey(&s.1).is_some(), "{s:?}");
        }
    }

    /// A fake registration: which chords it holds, shared with the test.
    struct FakeReg(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

    /// Registered only while recording: begin registers (once, even when
    /// asked again), end unregisters, a changed chord while recording
    /// registers again, and a refused chord is a failure with its slot.
    #[test]
    fn rec_chords_live_only_while_recording() {
        let live = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut rk: RecKeys<FakeReg> = RecKeys::default();
        let starts = std::cell::Cell::new(0);
        let start = |want: &[String; REC_SLOTS], taken: &[&str]| {
            starts.set(starts.get() + 1);
            *live.borrow_mut() = want.iter().filter(|s| !s.is_empty() && !taken.contains(&s.as_str())).cloned().collect();
            let ok = std::array::from_fn(|i| if taken.contains(&want[i].as_str()) { Err(Cause::InUse) } else { Ok(()) });
            (FakeReg(live.clone()), ok)
        };
        let stop = |w: FakeReg| w.0.borrow_mut().clear();
        // Before any recording: nothing held; ending is a no-op.
        assert!(!rk.active());
        rk.end(stop);
        let want = rec_wanted(&Config::default());
        let f = rk.begin(&want, |w| start(w, &[]), stop);
        assert!(f.is_empty() && rk.active());
        assert_eq!(live.borrow().len(), 3);
        // Asked again (a reload with the same chords): no new registration.
        let f = rk.begin(&want, |w| start(w, &[]), stop);
        assert!(f.is_empty());
        assert_eq!(starts.get(), 1);
        // The recording ends: all released.
        rk.end(stop);
        assert!(!rk.active() && live.borrow().is_empty());
        rk.end(stop);
        // One chord is taken by another app: a failure for its slot, the others work.
        let f = rk.begin(&want, |w| start(w, &["Ctrl+Alt+Shift+M"]), stop);
        assert_eq!(f, [Failure { slot: REC_SLOT_BASE + 1, wanted: "Ctrl+Alt+Shift+M".into(), cause: Cause::InUse, ..Failure::default() }]);
        assert_eq!(
            f[0].message(),
            "Ctrl+Alt+Shift+M is used by another app. Pick another key: click Change shortcuts\u{2026}, click the Mute microphone key and press a new one."
        );
        assert_eq!(f[0].field_id(), "key_rec_mute");
        assert_eq!(*live.borrow(), ["Ctrl+Alt+Shift+P", "Ctrl+Alt+Shift+H"]);
        // New chords while recording: the old ones go, the new ones come.
        let mut want2 = want.clone();
        want2[0] = "Ctrl+Alt+P".into();
        let f = rk.begin(&want2, |w| start(w, &[]), stop);
        assert!(f.is_empty());
        assert_eq!(rk.held(), Some(&want2));
        assert!(live.borrow().contains(&"Ctrl+Alt+P".to_string()));
        // All unbound: nothing registered at all.
        rk.end(stop);
        let none: [String; REC_SLOTS] = Default::default();
        let before = starts.get();
        assert!(rk.begin(&none, |w| start(w, &[]), stop).is_empty());
        assert!(!rk.active());
        assert_eq!(starts.get(), before);
    }

    /// The full-screen default held by another app moves to its first
    /// free alternate (Y, then J), says so, and the slot reports the
    /// fallback (not a kept chord); with every alternate taken it stays
    /// unregistered and a probed free key is suggested.
    #[test]
    fn default_falls_back_to_a_free_alternate() {
        let want = defaults();
        let mut calls = Vec::new();
        let (now, f) = plan_reload(&Default::default(), &want, registrar(&["Shift+Meta+F"], &mut calls));
        assert_eq!(now[3], "Shift+Meta+Y");
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].slot, f[0].alt.as_deref(), f[0].kept.as_deref()), (3, Some("Shift+Meta+Y"), None));
        assert!(!f[0].needs_action());
        assert_eq!(
            f[0].message(),
            format!("{} is in use by another app \u{2014} Capture full screen is on {} instead.", chord_label("Shift+Meta+F"), chord_label("Shift+Meta+Y"))
        );
        #[cfg(windows)]
        assert_eq!(f[0].message(), "Shift+Win+F is in use by another app \u{2014} Capture full screen is on Shift+Win+Y instead.");
        // Y taken too: J.
        let mut calls = Vec::new();
        let (now, f) = plan_reload(&Default::default(), &want, registrar(&["Shift+Meta+F", "Meta+Shift+Y"], &mut calls));
        assert_eq!((now[3].as_str(), f[0].alt.as_deref()), ("Shift+Meta+J", Some("Shift+Meta+J")));
        // All taken: nothing registered, a key that registers now is suggested
        // (Y and J are taken, E Z G are our own defaults: Q).
        let mut calls = Vec::new();
        let (now, f) = plan_reload(&Default::default(), &want, registrar(&["Shift+Meta+F", "Shift+Meta+Y", "Shift+Meta+J"], &mut calls));
        assert_eq!(now[3], "");
        assert_eq!((f[0].alt.clone(), f[0].kept.clone(), f[0].free.as_deref()), (None, None, Some("Shift+Meta+Q")));
        assert!(f[0].needs_action());
        assert_eq!(f[0].field_id(), "global3");
        #[cfg(windows)]
        assert_eq!(
            f[0].message(),
            "Shift+Win+F is used by another app. Pick another key: click Change shortcuts\u{2026}, click the Capture full screen key and press a new one (Shift+Win+Q is free)."
        );
        // The probe left only what is held registered.
        assert_eq!(calls.last(), Some(&now));
    }

    /// An alternate another slot wants (or tries) is skipped; a key the
    /// user chose (not the default) never falls back.
    #[test]
    fn fallback_skips_our_own_keys_and_user_choices() {
        let mut want = defaults();
        want[0] = "Shift+Meta+Y".into(); // the user put Capture region on Y
        let mut calls = Vec::new();
        let (now, f) = plan_reload(&Default::default(), &want, registrar(&["Shift+Meta+F"], &mut calls));
        assert_eq!((now[0].as_str(), now[3].as_str()), ("Shift+Meta+Y", "Shift+Meta+J"));
        assert_eq!(f[0].alt.as_deref(), Some("Shift+Meta+J"));
        // Two defaults held: each gets its own alternate, never the same.
        let want = defaults();
        let mut calls = Vec::new();
        let (now, f) = plan_reload(&Default::default(), &want, registrar(&["Shift+Meta+F", "Shift+Meta+G"], &mut calls));
        assert_eq!((now[2].as_str(), now[3].as_str()), ("Shift+Meta+J", "Shift+Meta+Y"));
        assert_eq!(f.len(), 2);
        // A key the user chose: no fallback, a free key suggested.
        let mut want = defaults();
        want[3] = "Ctrl+Alt+F".into();
        let mut calls = Vec::new();
        let (now, f) = plan_reload(&Default::default(), &want, registrar(&["Ctrl+Alt+F"], &mut calls));
        assert_eq!(now[3], "");
        assert_eq!((f[0].alt.clone(), f[0].free.as_deref()), (None, Some("Shift+Meta+Y")));
        // Saved spelled differently, still the default: falls back.
        let mut want = defaults();
        want[3] = "Meta+Shift+F".into();
        let mut calls = Vec::new();
        let (now, _) = plan_reload(&Default::default(), &want, registrar(&["Shift+Meta+F"], &mut calls));
        assert_eq!(now[3], "Shift+Meta+Y");
        // The other app let go: the next reload gets the default back.
        let mut calls = Vec::new();
        let (again, f) = plan_reload(&now, &defaults(), registrar(&[], &mut calls));
        assert_eq!((again, f), (defaults(), vec![]));
    }

    /// The daemon's startup registration settles the same way (no old
    /// chords to keep).
    #[test]
    fn startup_settles_with_fallbacks() {
        let want = defaults();
        let first: Registered = std::array::from_fn(|i| if i == 3 { Err(Cause::InUse) } else { Ok(()) });
        let mut calls = Vec::new();
        let (now, f) = settle(&Default::default(), &want, first, registrar(&["Shift+Meta+F"], &mut calls));
        assert_eq!(now[3], "Shift+Meta+Y");
        assert_eq!(f[0].alt.as_deref(), Some("Shift+Meta+Y"));
        assert_eq!(calls.len(), 1, "one registration with the alternate");
    }

    /// The slot names follow Settings › Key bindings.
    #[test]
    fn slot_names_are_the_settings_rows() {
        for (label, slot) in crate::settings_ui::GLOBALS {
            assert_eq!(slot_name(slot), label);
        }
        for (i, (label, a)) in crate::settings_ui::draw::REC_ROWS.iter().enumerate() {
            assert_eq!((slot_name(REC_SLOT_BASE + i), REC_ACTIONS[i]), (*label, *a));
        }
        // Every alternate is a Win+Shift chord no default has.
        let def = defaults();
        for i in 0..SLOTS {
            for a in alternates(i) {
                let c = crate::keymap::Chord::parse(a).unwrap();
                assert!(c.meta && c.shift && !c.ctrl && !c.alt, "{a}");
                assert!(!def.iter().any(|d| same_chord(d, a)), "{a} is a default");
            }
        }
    }

    #[test]
    fn key_names_round_trip() {
        for vk in (0x30..=0x39).chain(0x41..=0x5A).chain(0x70..=0x7B) {
            let n = key_name(vk).unwrap();
            assert_eq!(key_vk(&n.to_ascii_lowercase()), Some(vk), "{n}");
        }
        for vk in [key::SPACE, key::RETURN, key::ESCAPE, key::DELETE, key::PRINTSCREEN, key::LEFT, key::EQUALS, key::MINUS, key::LBRACKET, key::RBRACKET, key::ADD, key::SUBTRACT, key::NUMPAD0, key::NUMPAD0 + 9] {
            let n = key_name(vk).unwrap();
            assert_eq!(key_vk(&n.to_ascii_lowercase()), Some(vk), "{n}");
        }
    }
}
