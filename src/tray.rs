//! Tray menu model shared by the per-OS backends (Windows notification
//! area, the Linux StatusNotifierItem, the macOS status item).

use crate::hotkey::HotEvent;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::chan::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// Shown (first) only while a recording runs.
    StopRecording,
    /// Pause / resume the recording; payload = paused now.
    Pause(bool),
    /// Mute the microphone (when one is recorded); payload = muted now.
    MuteMic(bool),
    /// Hide the camera bubble (when there is one); payload = hidden now.
    HideCamera(bool),
    /// Hide the recording's control pill; payload = hidden now.
    HidePill(bool),
    /// Stop and throw the recording away.
    Discard,
    Capture,
    /// Capture the whole screen, a window; record (when recording works here).
    CaptureFull,
    CaptureWindow,
    Record,
    /// Close every pin (shown while there are any).
    ClosePins,
    /// The History window; payload = captures kept today ("4 today").
    History(usize),
    Settings,
    CheckUpdates,
    /// Start at login; payload = currently enabled.
    Autostart(bool),
    Quit,
    /// A line between groups.
    Separator,
    /// The capture launcher ("Capture…": source, delay, what follows).
    Launcher,
    /// A region capture after this many seconds ("Capture in 3 s").
    CaptureIn(u8),
    /// A delayed capture is counting down: call it off.
    CancelDelay,
    /// The "Recent uploads" submenu; its items ([`MenuItem::Upload`])
    /// follow it in the list.
    RecentUploads,
    /// Upload `i` of the recent ones (a click copies its link); `tag`
    /// changes with the link, so a changed list is a changed menu.
    Upload { i: u8, tag: u32 },
    /// "Delete upload ……" for upload `i` (its question first).
    DeleteUpload { i: u8, tag: u32 },
    /// The Make QR code window (with the launcher items; not on macOS).
    MakeQr,
}

impl MenuItem {
    /// The item's text ("History… · 4 today" counts today's captures).
    pub fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            MenuItem::History(0) => tr!("tray.history").into(),
            MenuItem::History(n) => tr!("tray.history_today", n).into(),
            MenuItem::CaptureIn(n) => tr!("tray.capture_in_n", n).into(),
            MenuItem::CancelDelay => match DELAY_LEFT.load(Ordering::SeqCst) {
                0 => tr!("tray.cancel_delay").into(),
                n => tr!("tray.cancel_delay_n", n).into(),
            },
            MenuItem::Upload { tag, .. } => upload_label(tag).into(),
            MenuItem::DeleteUpload { tag, .. } => tr!("tray.delete_upload", link = upload_label(tag)).into(),
            m => m.fixed_label().into(),
        }
    }

    fn fixed_label(self) -> &'static str {
        match self {
            MenuItem::CaptureFull => tr!("tray.capture_screen"),
            MenuItem::CaptureWindow => tr!("tray.capture_window"),
            MenuItem::Record => tr!("tray.record_video"),
            MenuItem::History(_) => tr!("tray.history"),
            MenuItem::StopRecording => tr!("tray.stop_recording"),
            MenuItem::Pause(false) => tr!("tray.pause_recording"),
            MenuItem::Pause(true) => tr!("tray.resume_recording"),
            MenuItem::MuteMic(_) => tr!("tray.mute_microphone"),
            MenuItem::HideCamera(_) => tr!("tray.hide_camera"),
            MenuItem::HidePill(_) => tr!("tray.hide_recording_controls"),
            MenuItem::Discard => tr!("tray.discard_recording"),
            MenuItem::Capture => tr!("tray.capture_region"),
            MenuItem::ClosePins => tr!("tray.close_all_pins"),
            MenuItem::Settings => tr!("tray.settings"),
            MenuItem::CheckUpdates => tr!("tray.check_for_updates"),
            MenuItem::Autostart(_) => tr!("tray.start_at_login"),
            MenuItem::Quit => tr!("tray.quit_rustshot"),
            MenuItem::Separator => "",
            MenuItem::Launcher => tr!("tray.capture"),
            MenuItem::CaptureIn(_) => tr!("tray.capture_in"),
            MenuItem::CancelDelay => tr!("tray.cancel_delayed_capture"),
            MenuItem::RecentUploads => tr!("tray.recent_uploads"),
            MenuItem::Upload { .. } | MenuItem::DeleteUpload { .. } => "",
            MenuItem::MakeQr => tr!("qr.make"),
        }
    }

    /// A check item's state (None: not a check item).
    pub fn checked(self) -> Option<bool> {
        match self {
            MenuItem::MuteMic(on) | MenuItem::HideCamera(on) | MenuItem::HidePill(on) | MenuItem::Autostart(on) => Some(on),
            _ => None,
        }
    }

    /// A stable id per kind (the payload does not change it): dbusmenu
    /// item ids, NSMenuItem tags. Separators are numbered by position
    /// from [`SEPARATOR_IDS`].
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn id(self) -> i32 {
        match self {
            MenuItem::StopRecording => 1,
            MenuItem::Pause(_) => 2,
            MenuItem::MuteMic(_) => 3,
            MenuItem::HideCamera(_) => 4,
            MenuItem::HidePill(_) => 5,
            MenuItem::Discard => 6,
            MenuItem::Capture => 10,
            MenuItem::Settings => 11,
            MenuItem::CheckUpdates => 12,
            MenuItem::Autostart(_) => 13,
            MenuItem::Quit => 14,
            MenuItem::ClosePins => 15,
            MenuItem::CaptureFull => 16,
            MenuItem::CaptureWindow => 17,
            MenuItem::Record => 18,
            MenuItem::History(_) => 19,
            MenuItem::Launcher => 20,
            MenuItem::CaptureIn(3) => 21,
            MenuItem::CaptureIn(10) => 22,
            MenuItem::CaptureIn(_) => 23,
            MenuItem::CancelDelay => 24,
            MenuItem::RecentUploads => 25,
            MenuItem::MakeQr => 26,
            MenuItem::Upload { i, .. } => 30 + 2 * i as i32,
            MenuItem::DeleteUpload { i, .. } => 31 + 2 * i as i32,
            MenuItem::Separator => SEPARATOR_IDS,
        }
    }
}

/// The first separator's id (the next ones count up from it).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const SEPARATOR_IDS: i32 = 100;

/// The items of `menu` with their ids (separators numbered in order).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn with_ids(menu: &[MenuItem]) -> Vec<(i32, MenuItem)> {
    let mut sep = SEPARATOR_IDS;
    menu.iter()
        .map(|m| {
            if *m == MenuItem::Separator {
                sep += 1;
                (sep - 1, *m)
            } else {
                (m.id(), *m)
            }
        })
        .collect()
}

/// What the tray menu shows of a running recording (the pill's state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecMenu {
    pub paused: bool,
    /// The microphone's mute state when one is recorded.
    pub mic: Option<bool>,
    /// Whether the camera bubble is hidden, when there is one.
    pub cam: Option<bool>,
    /// Whether the control pill is hidden.
    pub pill_hidden: bool,
}

/// Menu contents; `autostart` = None hides "Start at login" (managed
/// installs). While a recording runs (`rec`) its items come first: Stop,
/// Pause/Resume, Mute microphone (when recorded), Hide camera (with a
/// bubble), Hide recording controls, Discard.
#[cfg_attr(not(test), allow(dead_code))] // the tray and dbusmenu tests
pub fn menu(autostart: Option<bool>, rec: Option<RecMenu>) -> Vec<MenuItem> {
    menu_with(autostart, rec, false)
}

/// [`menu`]; with `pins` (a pin is up) "Close all pins" follows Capture.
pub fn menu_with(autostart: Option<bool>, rec: Option<RecMenu>, pins: bool) -> Vec<MenuItem> {
    menu_full(autostart, rec, pins, None, false)
}

/// The menu: the capture actions (Record when `record`), "Close all pins"
/// with `pins`, then History (`history`: today's count; `None`: History
/// is off), then Settings, updates, Start at login, Quit.
pub fn menu_full(autostart: Option<bool>, rec: Option<RecMenu>, pins: bool, history: Option<usize>, record: bool) -> Vec<MenuItem> {
    let mut v = menu_old(autostart, rec, pins);
    let at = v.iter().position(|m| *m == MenuItem::Capture).map_or(0, |i| i + 1);
    let mut caps = vec![MenuItem::CaptureFull, MenuItem::CaptureWindow];
    if record && rec.is_none() {
        caps.push(MenuItem::Record);
    }
    v.splice(at..at, caps);
    let set = v.iter().position(|m| *m == MenuItem::Settings).unwrap_or(v.len());
    if let Some(n) = history {
        v.insert(set, MenuItem::History(n));
    }
    v
}

fn menu_old(autostart: Option<bool>, rec: Option<RecMenu>, pins: bool) -> Vec<MenuItem> {
    let mut v = Vec::new();
    if let Some(r) = rec {
        v.extend([MenuItem::StopRecording, MenuItem::Pause(r.paused)]);
        if let Some(m) = r.mic {
            v.push(MenuItem::MuteMic(m));
        }
        if let Some(c) = r.cam {
            v.push(MenuItem::HideCamera(c));
        }
        v.extend([MenuItem::HidePill(r.pill_hidden), MenuItem::Discard, MenuItem::Separator]);
    }
    v.push(MenuItem::Capture);
    if pins {
        v.push(MenuItem::ClosePins);
    }
    v.extend([MenuItem::Settings, MenuItem::CheckUpdates]);
    if let Some(on) = autostart {
        v.push(MenuItem::Autostart(on));
    }
    v.extend([MenuItem::Separator, MenuItem::Quit]);
    v
}

/// What [`menu_more`] adds to [`menu_full`]: the launcher items, a
/// countdown's Cancel, the recent uploads (their tags).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    /// "Capture…", "Capture in 3 s", "Capture in 10 s" (not while recording).
    pub launcher: bool,
    /// A delayed capture is counting down.
    pub delay: bool,
    /// One tag per recent upload (newest first).
    pub uploads: Vec<u32>,
    /// Which of them can be deleted (an S3 object, or an Imgur upload whose key was kept).
    pub deletable: Vec<bool>,
}

/// [`menu_full`] with `x`: a countdown's Cancel first, the launcher items
/// after the capture actions, "Recent uploads" (its items following it)
/// before Settings.
pub fn menu_more(autostart: Option<bool>, rec: Option<RecMenu>, pins: bool, history: Option<usize>, record: bool, x: &Extras) -> Vec<MenuItem> {
    let mut v = menu_full(autostart, rec, pins, history, record);
    if x.launcher && rec.is_none() {
        let last = [MenuItem::Record, MenuItem::CaptureWindow].iter().find_map(|l| v.iter().position(|m| m == l)).map_or(0, |i| i + 1);
        v.splice(last..last, [MenuItem::Launcher, MenuItem::CaptureIn(3), MenuItem::CaptureIn(10)]);
        if cfg!(not(target_os = "macos")) {
            let at = last + 3;
            v.insert(at, MenuItem::MakeQr);
        }
    }
    if !x.uploads.is_empty() {
        let at = v.iter().position(|m| *m == MenuItem::Settings).unwrap_or(v.len());
        let mut items = vec![MenuItem::RecentUploads];
        for (i, t) in x.uploads.iter().take(32).enumerate() {
            items.push(MenuItem::Upload { i: i as u8, tag: *t });
            if x.deletable.get(i).copied().unwrap_or(false) {
                items.push(MenuItem::DeleteUpload { i: i as u8, tag: *t });
            }
        }
        v.splice(at..at, items);
    }
    if x.delay {
        v.splice(0..0, [MenuItem::CancelDelay, MenuItem::Separator]);
    }
    v
}

/// The menu's items grouped for backends with submenus: each top item's
/// index with its submenu's ("Recent uploads": the uploads after it).
pub fn nest(menu: &[MenuItem]) -> Vec<(usize, Vec<usize>)> {
    let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
    for (i, m) in menu.iter().enumerate() {
        match out.last_mut() {
            Some((p, kids)) if matches!(m, MenuItem::Upload { .. } | MenuItem::DeleteUpload { .. }) && menu[*p] == MenuItem::RecentUploads => {
                kids.push(i)
            }
            _ => out.push((i, Vec::new())),
        }
    }
    out
}

/// Current menu for this install.
pub fn current_menu() -> Vec<MenuItem> {
    let auto = crate::update::managed_install().is_none().then(crate::autostart::is_enabled);
    let cfg = crate::config::load();
    let today = {
        let t = crate::export::local_ymdhms();
        (t.0, t.1, t.2)
    };
    let history = cfg.history.then(|| crate::history::today_count(&crate::history::default_root(), today));
    let recent = crate::uploads::recent();
    let x = Extras {
        launcher: true,
        delay: DELAYING.load(Ordering::SeqCst),
        uploads: recent.iter().map(|r| tag_of(&r.link)).collect(),
        deletable: recent.iter().map(|r| r.info.deletable()).collect(),
    };
    *RECENT.lock().unwrap_or_else(|e| e.into_inner()) = recent.into_iter().map(|r| r.link).collect();
    menu_more(auto, rec_menu(), crate::pin::count() > 0, history, crate::rec_ui::supported(), &x)
}

/// The links "Recent uploads" lists (as the menu was last built).
static RECENT: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A short hash of `link` (a [`MenuItem::Upload`]'s tag).
pub fn tag_of(link: &str) -> u32 {
    link.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// The listed link with menu tag `tag` (never another one: a menu built
/// from an older list finds nothing).
fn recent_link(tag: u32) -> Option<String> {
    RECENT.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|l| tag_of(l) == tag).cloned()
}

/// An upload's menu text: "i.imgur.com/aB3x.png".
fn upload_label(tag: u32) -> String {
    recent_link(tag).map_or_else(|| tr!("tray.gone").into(), |l| crate::uploads::short_link(&l))
}

/// An upload record was filed or forgotten: "Recent uploads" follows.
pub fn uploads_changed() {
    bump();
    #[cfg(all(target_os = "macos", not(test)))]
    crate::tray_macos::refresh();
}

/// Seconds left of a delayed capture (0: none counting).
static DELAY_LEFT: AtomicU64 = AtomicU64::new(0);
/// A delayed capture is set up (Cancel is offered, even in its last second).
static DELAYING: AtomicBool = AtomicBool::new(false);

/// A delayed capture's countdown: `Some(seconds left)`, or `None` once it
/// ended. The menu gains "Cancel capture", the tip says the seconds.
pub fn set_delay(left: Option<u64>) {
    let was = DELAYING.swap(left.is_some(), Ordering::SeqCst);
    let n = left.unwrap_or(0);
    if DELAY_LEFT.swap(n, Ordering::SeqCst) == n && was == left.is_some() {
        return;
    }
    bump();
    #[cfg(all(windows, not(test)))]
    crate::tray_win::refresh_tip();
    #[cfg(all(target_os = "macos", not(test)))]
    {
        crate::tray_macos::refresh_tip();
        crate::tray_macos::refresh();
    }
}

/// The daemon event of a menu item (None: the item does its work here,
/// see [`activate`]).
pub fn item_event(item: MenuItem) -> Option<HotEvent> {
    Some(match item {
        MenuItem::StopRecording => HotEvent::StopRecording,
        MenuItem::Pause(_) => HotEvent::RecPause,
        MenuItem::MuteMic(_) => HotEvent::RecMute,
        MenuItem::HideCamera(_) => HotEvent::RecCamera,
        MenuItem::HidePill(_) => HotEvent::RecPill,
        MenuItem::Discard => HotEvent::RecDiscard,
        MenuItem::Capture => HotEvent::Capture,
        MenuItem::CaptureFull => HotEvent::CaptureFull,
        MenuItem::CaptureWindow => HotEvent::CaptureWindow,
        MenuItem::Record => HotEvent::Record,
        MenuItem::History(_) => HotEvent::History,
        MenuItem::ClosePins => HotEvent::ClosePins,
        MenuItem::Quit => HotEvent::Quit,
        MenuItem::CaptureIn(n) => HotEvent::Launch(crate::launcher::Plan::region_in(n as u32)),
        MenuItem::CancelDelay => HotEvent::CancelDelay,
        MenuItem::DeleteUpload { tag, .. } => HotEvent::UploadDelete(tag),
        MenuItem::Settings
        | MenuItem::CheckUpdates
        | MenuItem::Autostart(_)
        | MenuItem::Separator
        | MenuItem::Launcher
        | MenuItem::MakeQr
        | MenuItem::RecentUploads
        | MenuItem::Upload { .. } => return None,
    })
}

/// Carry out `item`: its event goes to `tx`; Settings, updates and Start
/// at login run here. `Err`: what to tell the user.
pub fn activate(item: MenuItem, tx: &Sender<HotEvent>) -> Result<(), String> {
    if let Some(ev) = item_event(item) {
        let _ = tx.send(ev);
        return Ok(());
    }
    match item {
        MenuItem::Settings => crate::settings_ui::show(),
        MenuItem::Launcher => crate::launcher::show(),
        MenuItem::MakeQr => crate::qr_make::show(),
        MenuItem::Upload { tag, .. } => {
            let link = recent_link(tag).ok_or(tr!("editor.upload_gone"))?;
            crate::export::copy_text_to_clipboard(&link).map_err(|e| tr!("tray.copy_link_failed", e = format!("{e:#}")))?;
            notify(&tr!("notice.upload_done", head = tr!("history.link_copied"), link = crate::uploads::short_link(&link)));
        }
        MenuItem::CheckUpdates => crate::actions::check_updates(),
        MenuItem::Autostart(_) => {
            crate::actions::toggle_autostart().map_err(|e| tr!("tray.autostart_failed", e))?;
            bump();
        }
        _ => {}
    }
    Ok(())
}

/// What a click on the icon itself does: stop a recording, else capture
/// (macOS: a click opens the menu).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn click_event() -> HotEvent {
    if is_recording() { HotEvent::StopRecording } else { HotEvent::Capture }
}

static RECORDING: AtomicBool = AtomicBool::new(false);
static REC_MENU: Mutex<RecMenu> = Mutex::new(RecMenu { paused: false, mic: None, cam: None, pill_hidden: false });

/// Whether a recording runs (the tray menu and icon follow it).
pub fn is_recording() -> bool {
    RECORDING.load(Ordering::SeqCst)
}

/// The recording's menu state, while one runs.
pub fn rec_menu() -> Option<RecMenu> {
    is_recording().then(|| *REC_MENU.lock().unwrap_or_else(|e| e.into_inner()))
}

/// A recording started or ended: the menu gets (or loses) its recording
/// items and the tray icon its red dot.
pub fn set_recording(on: bool) {
    RECORDING.store(on, Ordering::SeqCst);
    ELAPSED.store(if on { 0 } else { NO_TIME }, Ordering::SeqCst);
    *REC_MENU.lock().unwrap_or_else(|e| e.into_inner()) = RecMenu::default();
    bump();
    // Tests never post to a tray window (it could be the user's daemon's).
    #[cfg(all(windows, not(test)))]
    crate::tray_win::refresh_state();
    #[cfg(all(target_os = "macos", not(test)))]
    crate::tray_macos::refresh();
}

/// The recording's pill state changed (paused, muted, camera, pill
/// hidden): the menu follows. Returns whether it changed.
pub fn set_rec_menu(m: RecMenu) -> bool {
    let mut cur = REC_MENU.lock().unwrap_or_else(|e| e.into_inner());
    if *cur == m {
        return false;
    }
    *cur = m;
    drop(cur);
    bump();
    true
}

/// Pins came or went: the menu gains or loses "Close all pins" (Windows
/// builds it when it opens).
pub fn pins_changed() {
    bump();
    #[cfg(all(target_os = "macos", not(test)))]
    crate::tray_macos::refresh();
}

/// Bumped whenever what the tray shows changes (the Linux tray polls it).
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn bump() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// The tray state's generation: a change means the menu, icon or tip may
/// have changed.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

const NO_TIME: u64 = u64::MAX;
/// The whole seconds recorded, for the tip; [`NO_TIME`] when not recording.
static ELAPSED: AtomicU64 = AtomicU64::new(NO_TIME);

/// The time recorded so far (paused time excluded): the tray's tip
/// follows it, redrawn when the whole seconds change. Returns whether
/// they did.
pub fn set_elapsed(d: std::time::Duration) -> bool {
    if !is_recording() {
        return false;
    }
    let s = d.as_secs();
    let changed = ELAPSED.swap(s, Ordering::SeqCst) != s;
    if changed {
        bump();
    }
    #[cfg(all(windows, not(test)))]
    if changed {
        crate::tray_win::refresh_tip();
    }
    #[cfg(all(target_os = "macos", not(test)))]
    if changed {
        crate::tray_macos::refresh_tip();
    }
    changed
}

/// The tray icon's tooltip: "Rustshot", or "Rustshot — recording 1:23"
/// while a recording runs.
pub fn tip_text() -> String {
    match DELAY_LEFT.load(Ordering::SeqCst) {
        0 => tip_for(is_recording(), ELAPSED.load(Ordering::SeqCst)),
        n => delay_tip(n),
    }
}

/// The tip while a delayed capture counts down.
fn delay_tip(n: u64) -> String {
    tr!("tray.tip.delay", n)
}

fn tip_for(recording: bool, secs: u64) -> String {
    match (recording, secs) {
        (false, _) => "Rustshot".into(),
        (true, NO_TIME) => tr!("tray.tip.recording").into(),
        (true, s) => tr!("tray.tip.recording_time", time = crate::rec_ui::fmt_time(std::time::Duration::from_secs(s))),
    }
}

/// "Saved <path>" (with what was left out, e.g. "microphone not
/// recorded: <reason>"): a tray balloon whose click opens the folder
/// (Windows); stdout elsewhere.
pub fn notify_saved(path: &std::path::Path, warnings: &[String]) {
    #[cfg(windows)]
    crate::tray_win::notify_saved(path, &saved_text(path, warnings));
    #[cfg(not(windows))]
    println!("{}", saved_text(path, warnings));
}

/// "Saved <path> — microphone not recorded: <reason>".
pub fn saved_text(path: &std::path::Path, warnings: &[String]) -> String {
    let mut t = tr!("notice.saved", name = path.display());
    for w in warnings {
        t.push_str(" — ");
        t.push_str(w);
    }
    t
}

/// Start the tray for the running daemon. `tx` receives Capture/Quit;
/// `visible` (`show_tray`) false: no icon until [`set_visible`].
pub fn spawn(tx: Sender<HotEvent>, visible: bool) {
    VISIBLE.store(visible, Ordering::SeqCst);
    #[cfg(windows)]
    crate::tray_win::spawn(tx);
    #[cfg(all(target_os = "linux", not(test)))]
    crate::tray_linux::spawn(tx);
    #[cfg(all(target_os = "macos", not(test)))]
    crate::tray_macos::spawn(tx);
    #[cfg(all(not(windows), any(test, not(any(target_os = "linux", target_os = "macos")))))]
    let _ = tx;
}

static VISIBLE: AtomicBool = AtomicBool::new(true);

/// Whether the icon is wanted (`show_tray`).
pub fn is_visible() -> bool {
    VISIBLE.load(Ordering::SeqCst)
}

/// Show or hide the icon (Settings changed `show_tray`); the daemon,
/// its hotkeys and a second launch work either way.
pub fn set_visible(on: bool) {
    VISIBLE.store(on, Ordering::SeqCst);
    bump();
    #[cfg(all(windows, not(test)))]
    crate::tray_win::refresh_visible();
    #[cfg(all(target_os = "macos", not(test)))]
    crate::tray_macos::refresh();
}

/// Tell the user `text` outside any window: a tray balloon on Windows,
/// stderr elsewhere (for now).
pub fn notify(text: &str) {
    // Tests never pop balloons on the user's daemon: they read `take_notes`.
    #[cfg(test)]
    {
        NOTES.with(|n| n.borrow_mut().push(text.to_string()));
    }
    // No icon, no balloon: stderr instead.
    #[cfg(all(windows, not(test)))]
    if is_visible() {
        crate::tray_win::notify(text);
    } else {
        eprintln!("rustshot: {text}");
    }
    #[cfg(not(windows))]
    eprintln!("rustshot: {text}");
}

/// With the tray icon off nothing runs in the background: Windows, not
/// the Store (MSIX) build, where Win+Shift+S / Print Screen can start
/// Rustshot on demand. The Store build (and Linux, macOS) keep the daemon
/// running without an icon, its hotkeys working.
pub fn hidden_means_no_daemon() -> bool {
    cfg!(windows) && crate::screenclip_win::available()
}

/// [`notify`] about the hotkeys: a click on the balloon opens Settings ›
/// Key bindings (Windows).
pub fn notify_keys(text: &str) {
    notify(text);
    #[cfg(all(windows, not(test)))]
    if is_visible() {
        crate::tray_win::click_opens_keys();
    }
}

#[cfg(test)]
thread_local! {
    static NOTES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Tests: the balloons this thread asked for since the last call.
#[cfg(test)]
pub fn take_notes() -> Vec<String> {
    NOTES.with(|n| std::mem::take(&mut *n.borrow_mut()))
}

/// Tray glyph on a 16-unit grid: four rounded corner brackets (stroked) and a centre dot.
const GLYPH_PATH: &str = "M1.5 6V3.5a2 2 0 0 1 2-2H6 M10 1.5h2.5a2 2 0 0 1 2 2V6 M14.5 10v2.5a2 2 0 0 1-2 2H10 M6 14.5H3.5a2 2 0 0 1-2-2V10";
const GLYPH_STROKE: f32 = 1.6;
const GLYPH_DOT_R: f32 = 2.4;

/// The recording dot's color.
pub const REC_RED: (u8, u8, u8) = (0xF0, 0x3E, 0x3E);
const REC_DOT_R: f32 = 3.6;

/// The app icon's tile: its gradient's top-left and bottom-right colors
/// (packaging/icons/make-icon.ps1) and corner radius (of the side).
const TILE_TOP: (u8, u8, u8) = (79, 142, 247);
const TILE_BOTTOM: (u8, u8, u8) = (37, 78, 196);
const TILE_RADIUS: f32 = 56.0 / 256.0;
/// The glyph's box, of the tile's side: the app icon's brackets span 55%
/// of it, the glyph's 91% of its box.
const TILE_GLYPH: f32 = 0.6;

/// The tray icon as straight-alpha RGBA, `size`x`size`: the app icon (the
/// blue tile with the white glyph); with `recording` the centre dot is a
/// larger red one. Each shape's coverage is rasterised white-on-black into
/// an opaque scratch buffer (the rasterizer assumes an opaque destination).
pub fn tray_icon_rgba(size: u32, recording: bool) -> Vec<u8> {
    use crate::uifb::{C4, Fb};
    let n = size as usize;
    let white = C4::rgb(255, 255, 255);
    let mask = |draw: &dyn Fn(&mut Fb)| {
        let mut buf = [0u8, 0, 0, 255].repeat(n * n);
        draw(&mut Fb::new(&mut buf, n));
        buf.as_chunks::<4>().0.iter().map(|p| p[0] as f32 / 255.0).collect::<Vec<f32>>()
    };
    let side = size as f32;
    let tile = mask(&|fb| fb.fill_rounded(0, 0, n as i32, n as i32, side * TILE_RADIUS, white));
    let g = side * TILE_GLYPH;
    let o = (side - g) / 2.0;
    let k = g / 16.0;
    let brackets = mask(&|fb| draw_brackets(fb, o, o, g, white));
    let r = if recording { REC_DOT_R } else { GLYPH_DOT_R };
    let dot = mask(&|fb| fb.fill_circle(o + 8.0 * k, o + 8.0 * k, r * k, white));
    let dot_rgb = if recording { REC_RED } else { (255, 255, 255) };
    let mix = |a: u8, b: u8, t: f32| a as f32 + (b as f32 - a as f32) * t;
    let mut out = Vec::with_capacity(n * n * 4);
    for i in 0..n * n {
        let (x, y) = (i % n, i / n);
        let t = (x + y) as f32 / (2 * n.max(2) - 2) as f32;
        let base = [mix(TILE_TOP.0, TILE_BOTTOM.0, t), mix(TILE_TOP.1, TILE_BOTTOM.1, t), mix(TILE_TOP.2, TILE_BOTTOM.2, t)];
        let (gb, gd) = (brackets[i], dot[i]);
        let px = [(base[0], dot_rgb.0), (base[1], dot_rgb.1), (base[2], dot_rgb.2)]
            .map(|(b, d)| ((b + (255.0 - b) * gb) * (1.0 - gd) + d as f32 * gd).round() as u8);
        out.extend_from_slice(&[px[0], px[1], px[2], (tile[i] * 255.0).round() as u8]);
    }
    out
}

/// The app icon ([`tray_icon_rgba`]) blended into `fb` with its top-left
/// at physical px (x, y), `size` px square: the Settings rail and About,
/// the update dialog.
pub fn draw_app_icon(fb: &mut crate::uifb::Fb, x: i32, y: i32, size: u32) {
    let n = size as usize;
    for (i, p) in tray_icon_rgba(size, false).as_chunks::<4>().0.iter().enumerate() {
        if p[3] != 0 {
            fb.blend_px(x + (i % n) as i32, y + (i / n) as i32, crate::uifb::C4::new(p[0], p[1], p[2], p[3]));
        }
    }
}

/// The glyph's corner brackets alone (no centre dot), `size`x`size` at (x, y).
fn draw_brackets(fb: &mut crate::uifb::Fb, x: f32, y: f32, size: f32, c: crate::uifb::C4) {
    use crate::raster::Blend;
    let k = size / 16.0;
    if let Some(lines) = crate::icon_path::parse_path(GLYPH_PATH) {
        let mut surf = fb.surf();
        for mut line in lines {
            for p in &mut line {
                p.x = x + p.x * k;
                p.y = y + p.y * k;
            }
            surf.stroke_polyline(&line, GLYPH_STROKE * k, c, Blend::Normal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tip says the time recorded while a recording runs.
    #[test]
    fn tip_says_the_recorded_time() {
        assert_eq!(tip_for(false, 42), "Rustshot");
        assert_eq!(tip_for(true, NO_TIME), "Rustshot — recording");
        assert_eq!(tip_for(true, 0), "Rustshot — recording 0:00");
        assert_eq!(tip_for(true, 83), "Rustshot — recording 1:23");
        assert_eq!(tip_for(true, 3723), "Rustshot — recording 1:02:03");
    }

    #[test]
    fn saved_text_says_what_was_left_out() {
        let p = std::path::Path::new("C:/shots/a.mp4");
        assert_eq!(saved_text(p, &[]), format!("Saved {}", p.display()));
        let w = ["microphone not recorded: no device".to_string(), "system sound not recorded: busy".to_string()];
        assert_eq!(
            saved_text(p, &w),
            format!("Saved {} — microphone not recorded: no device — system sound not recorded: busy", p.display())
        );
    }

    #[test]
    fn menu_composition() {
        assert_eq!(
            menu(Some(true), None),
            vec![
                MenuItem::Capture,
                MenuItem::CaptureFull,
                MenuItem::CaptureWindow,
                MenuItem::Settings,
                MenuItem::CheckUpdates,
                MenuItem::Autostart(true),
                MenuItem::Separator,
                MenuItem::Quit
            ]
        );
        assert_eq!(menu(None, None).len(), 7);
        // The full menu: Record with the capture actions, History before Settings.
        let f = menu_full(None, None, false, Some(4), true);
        assert_eq!(f[..6], [MenuItem::Capture, MenuItem::CaptureFull, MenuItem::CaptureWindow, MenuItem::Record, MenuItem::History(4), MenuItem::Settings]);
        assert!(!menu_full(None, None, false, None, true).iter().any(|m| matches!(m, MenuItem::History(_))), "History off");
        assert!(!menu_full(None, Some(RecMenu::default()), false, Some(0), true).contains(&MenuItem::Record), "not while recording");
        assert!(!menu(None, None).iter().any(|m| matches!(m, MenuItem::Autostart(_))));
        assert_eq!(*menu(None, None).last().unwrap(), MenuItem::Quit);
        assert!(!menu(Some(false), None).contains(&MenuItem::StopRecording));
        assert!(!menu(Some(true), None).contains(&MenuItem::ClosePins), "no pins: no Close all pins");
    }

    /// With pins up, "Close all pins" follows Capture (recording or not)
    /// and tells the daemon to close them.
    #[test]
    fn menu_with_pins() {
        let m = menu_with(None, None, true);
        assert_eq!(m[..5], [MenuItem::Capture, MenuItem::CaptureFull, MenuItem::CaptureWindow, MenuItem::ClosePins, MenuItem::Settings]);
        let r = menu_with(None, Some(RecMenu::default()), true);
        let i = r.iter().position(|m| *m == MenuItem::Capture).unwrap();
        assert_eq!(r[i + 3], MenuItem::ClosePins);
        assert_eq!(MenuItem::ClosePins.label(), "Close all pins");
        assert_eq!(MenuItem::ClosePins.checked(), None);
        assert_eq!(format!("{:?}", item_event(MenuItem::ClosePins)), "Some(ClosePins)");
        let ids = with_ids(&menu_with(Some(true), Some(RecMenu { mic: Some(false), cam: Some(true), ..RecMenu::default() }), true));
        let mut seen: Vec<i32> = ids.iter().map(|(i, _)| *i).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len(), "ids stay unique");
    }

    /// While recording, the recording's items come first: what the pill
    /// can do, with the pill's state.
    #[test]
    fn menu_while_recording() {
        let r = RecMenu::default();
        let m = menu(None, Some(r));
        assert_eq!(
            m[..5],
            [MenuItem::StopRecording, MenuItem::Pause(false), MenuItem::HidePill(false), MenuItem::Discard, MenuItem::Separator]
        );
        assert_eq!(&m[5..], &menu(None, None)[..]);
        // A microphone and a camera bubble add their items; the labels and
        // checks follow the state.
        let r = RecMenu { paused: true, mic: Some(true), cam: Some(false), pill_hidden: true };
        let m = menu(Some(false), Some(r));
        assert_eq!(
            m[..7],
            [
                MenuItem::StopRecording,
                MenuItem::Pause(true),
                MenuItem::MuteMic(true),
                MenuItem::HideCamera(false),
                MenuItem::HidePill(true),
                MenuItem::Discard,
                MenuItem::Separator
            ]
        );
        assert_eq!(MenuItem::Pause(true).label(), "Resume recording");
        assert_eq!(MenuItem::Pause(false).label(), "Pause recording");
        assert_eq!(MenuItem::MuteMic(true).checked(), Some(true));
        assert_eq!(MenuItem::HideCamera(false).checked(), Some(false));
        assert_eq!(MenuItem::HidePill(true).checked(), Some(true));
        assert_eq!(MenuItem::StopRecording.checked(), None);
        // Each recording item sends its event to the daemon.
        let ev = |i| format!("{:?}", item_event(i));
        assert_eq!(ev(MenuItem::StopRecording), "Some(StopRecording)");
        assert_eq!(ev(MenuItem::Pause(true)), "Some(RecPause)");
        assert_eq!(ev(MenuItem::MuteMic(false)), "Some(RecMute)");
        assert_eq!(ev(MenuItem::HideCamera(false)), "Some(RecCamera)");
        assert_eq!(ev(MenuItem::HidePill(false)), "Some(RecPill)");
        assert_eq!(ev(MenuItem::Discard), "Some(RecDiscard)");
        assert_eq!(ev(MenuItem::Settings), "None");
    }

    /// Ids are stable per kind (the payload does not change them) and
    /// unique within a menu; separators count up from SEPARATOR_IDS.
    #[test]
    fn menu_ids() {
        assert_eq!(MenuItem::Pause(true).id(), MenuItem::Pause(false).id());
        let r = RecMenu { paused: false, mic: Some(false), cam: Some(false), pill_hidden: false };
        let ids = with_ids(&menu(Some(true), Some(r)));
        let mut seen: Vec<i32> = ids.iter().map(|(i, _)| *i).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len());
        let seps: Vec<i32> = ids.iter().filter(|(_, m)| *m == MenuItem::Separator).map(|(i, _)| *i).collect();
        assert_eq!(seps, [SEPARATOR_IDS, SEPARATOR_IDS + 1]);
    }

    /// The launcher's items follow the capture actions; "Recent uploads"
    /// (its uploads after it) comes before Settings; a countdown puts
    /// Cancel first. Ids stay unique; submenus nest.
    #[test]
    fn menu_with_launcher_uploads_and_a_countdown() {
        let x = Extras { launcher: true, delay: false, uploads: vec![tag_of("a"), tag_of("b")], deletable: vec![false, true] };
        let m = menu_more(Some(true), None, false, Some(2), true, &x);
        // Make QR code… after the launcher's items (not on macOS).
        assert_eq!(m.iter().position(|i| *i == MenuItem::MakeQr), cfg!(not(target_os = "macos")).then_some(7));
        let m: Vec<MenuItem> = m.into_iter().filter(|i| *i != MenuItem::MakeQr).collect();
        assert_eq!(
            m[..11],
            [
                MenuItem::Capture,
                MenuItem::CaptureFull,
                MenuItem::CaptureWindow,
                MenuItem::Record,
                MenuItem::Launcher,
                MenuItem::CaptureIn(3),
                MenuItem::CaptureIn(10),
                MenuItem::History(2),
                MenuItem::RecentUploads,
                MenuItem::Upload { i: 0, tag: tag_of("a") },
                MenuItem::Upload { i: 1, tag: tag_of("b") },
            ]
        );
        assert_eq!(m[11], MenuItem::DeleteUpload { i: 1, tag: tag_of("b") }, "only the one whose key was kept");
        assert_eq!(m[12], MenuItem::Settings);
        // Without Record (not supported here), after Capture window.
        let m2 = menu_more(None, None, false, None, false, &x);
        assert_eq!(m2[3], MenuItem::Launcher);
        // Nothing new without extras: the old menu.
        assert_eq!(menu_more(None, None, true, Some(0), true, &Extras::default()), menu_full(None, None, true, Some(0), true));
        // Not while recording; Cancel first while counting down.
        let r = menu_more(None, Some(RecMenu::default()), false, None, true, &x);
        assert!(!r.contains(&MenuItem::Launcher));
        let d = menu_more(None, None, false, None, true, &Extras { delay: true, ..x.clone() });
        assert_eq!(d[..2], [MenuItem::CancelDelay, MenuItem::Separator]);
        // Ids unique; the submenu nests its uploads.
        let ids = with_ids(&d);
        let mut seen: Vec<i32> = ids.iter().map(|(i, _)| *i).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len());
        let tree = nest(&d);
        let ru = d.iter().position(|i| *i == MenuItem::RecentUploads).unwrap();
        let (_, kids) = tree.iter().find(|(p, _)| *p == ru).unwrap();
        assert_eq!(kids, &vec![ru + 1, ru + 2, ru + 3]);
        assert_eq!(tree.len(), d.len() - 3, "the uploads are not top items");
        // A changed list is a changed menu (the Linux tray re-sends it).
        let other = menu_more(None, None, false, None, true, &Extras { uploads: vec![tag_of("c"), tag_of("b")], ..x.clone() });
        assert_ne!(other, menu_more(None, None, false, None, true, &x));
    }

    #[test]
    fn launcher_items_and_their_events() {
        assert_eq!(MenuItem::Launcher.label(), "Capture\u{2026}");
        assert_eq!(MenuItem::CaptureIn(3).label(), "Capture in 3 s");
        assert_eq!(MenuItem::CaptureIn(10).label(), "Capture in 10 s");
        assert_eq!(MenuItem::RecentUploads.label(), "Recent uploads");
        assert_ne!(MenuItem::CaptureIn(3).id(), MenuItem::CaptureIn(10).id());
        assert_eq!(MenuItem::Upload { i: 2, tag: 1 }.id(), MenuItem::Upload { i: 2, tag: 9 }.id());
        let ev = |i| format!("{:?}", item_event(i));
        assert_eq!(ev(MenuItem::CaptureIn(3)), format!("Some(Launch({:?}))", crate::launcher::Plan::region_in(3)));
        assert_eq!(ev(MenuItem::CancelDelay), "Some(CancelDelay)");
        // Done here, not by the daemon: the window, copying a link.
        for i in [MenuItem::Launcher, MenuItem::RecentUploads, MenuItem::Upload { i: 0, tag: 0 }] {
            assert_eq!(ev(i), "None");
        }
        assert_eq!(delay_tip(3), "Rustshot \u{2014} capturing in 3 s");
        // An upload's text is its link, as the menu was built.
        let l = "https://i.imgur.com/aB3x.png";
        *RECENT.lock().unwrap() = vec!["https://i.imgur.com/zz.png".into(), l.into()];
        assert_eq!(MenuItem::Upload { i: 0, tag: tag_of(l) }.label(), "i.imgur.com/aB3x.png", "by its tag, not its place");
        assert_eq!(MenuItem::DeleteUpload { i: 0, tag: tag_of(l) }.label(), "Delete upload i.imgur.com/aB3x.png\u{2026}");
        assert_eq!(MenuItem::Upload { i: 4, tag: 7 }.label(), "(gone)");
        assert_eq!(format!("{:?}", item_event(MenuItem::DeleteUpload { i: 0, tag: 5 })), "Some(UploadDelete(5))");
    }

    #[test]
    fn recording_icon_has_a_red_centre() {
        for size in [16u32, 32] {
            let plain = tray_icon_rgba(size, false);
            let rec = tray_icon_rgba(size, true);
            let c = size / 2;
            let i = ((c * size + c) * 4) as usize;
            assert_eq!(&plain[i..i + 4], &[255, 255, 255, 255]);
            assert_eq!(&rec[i..i + 4], &[REC_RED.0, REC_RED.1, REC_RED.2, 255]);
            // The red dot is larger than the plain one: white or red, not the tile's blue.
            let off = ((c * size + c + (size as f32 * 0.11) as u32) * 4) as usize;
            assert!(rec[off] > plain[off], "dot radius {:?} {:?}", &rec[off..off + 4], &plain[off..off + 4]);
        }
    }

    #[test]
    fn labels() {
        assert_eq!(MenuItem::StopRecording.label(), "Stop recording");
        assert_eq!(MenuItem::Capture.label(), "Capture region");
        assert_eq!(MenuItem::History(0).label(), "History\u{2026}");
        assert_eq!(MenuItem::History(4).label(), "History\u{2026} \u{b7} 4 today");
        assert_eq!(MenuItem::History(4).id(), MenuItem::History(0).id());
        assert_eq!(format!("{:?}", item_event(MenuItem::History(2))), "Some(History)");
        assert_eq!(MenuItem::Settings.label(), "Settings…");
        assert_eq!(MenuItem::CheckUpdates.label(), "Check for updates");
        assert_eq!(MenuItem::Autostart(false).label(), "Start at login");
        assert_eq!(MenuItem::Quit.label(), "Quit Rustshot");
    }


    #[test]
    fn icon_is_the_blue_tile_with_the_white_glyph() {
        for size in [16u32, 20, 24, 32] {
            let b = tray_icon_rgba(size, false);
            assert_eq!(b.len(), (size * size * 4) as usize);
            let px = |x: u32, y: u32| &b[((y * size + x) * 4) as usize..][..4];
            // The tile is opaque blue just inside its edge; its corners are rounded off.
            let m = size / 2;
            for (x, y) in [(m, 1), (1, m), (m, size - 2), (size - 2, m)] {
                let p = px(x, y);
                assert!(p[3] == 255 && p[2] > 180 && p[0] < 100, "edge {x},{y}: {p:?}");
            }
            for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
                assert!(px(x, y)[3] < 128, "corner {x},{y}");
            }
            // The gradient runs from the lighter top-left to the darker bottom-right.
            assert!(px(2, 2)[2] > px(size - 3, size - 3)[2]);
            // A bracket's corner is white-ish.
            let o = size as f32 * (1.0 - TILE_GLYPH) / 2.0;
            let k = size as f32 * TILE_GLYPH / 16.0;
            let (x, y) = ((o + 2.1 * k) as u32, (o + 2.1 * k) as u32);
            assert!(px(x, y)[0] > 140, "bracket {x},{y}: {:?}", px(x, y));
        }
    }

    /// Writes enlarged previews for eyeballing; run with --ignored.
    #[test]
    #[ignore = "writes preview PNGs to the scratchpad"]
    fn write_glyph_previews() {
        let dir = std::path::Path::new(
            "C:/Users/plato/AppData/Local/Temp/claude/E--Personal-rustshot/388c4929-22ab-4934-9dbf-1b81d7818b10/scratchpad/glyph",
        );
        std::fs::create_dir_all(dir).unwrap();
        for size in [16u32, 20, 24, 32] {
            for (name, bg, rec) in [("dark", 0x20u8, false), ("light", 0xF3, false), ("dark-rec", 0x20, true), ("light-rec", 0xF3, true)] {
                let g = tray_icon_rgba(size, rec);
                let scale = 8u32;
                let w = size * scale;
                let mut img = Vec::with_capacity((w * w * 3) as usize);
                for y in 0..w {
                    for x in 0..w {
                        let i = (((y / scale) * size + x / scale) * 4) as usize;
                        let a = g[i + 3] as u32;
                        for ch in 0..3 {
                            let v = (g[i + ch] as u32 * a + bg as u32 * (255 - a) + 127) / 255;
                            img.push(v as u8);
                        }
                    }
                }
                let png = crate::pngfile::encode(w, w, crate::pngfile::Color::Rgb, &img).unwrap();
                std::fs::write(dir.join(format!("glyph_{size}_{name}.png")), png).unwrap();
            }
        }
    }
}
