use crate::anim::Tween;
use crate::capture::{self, Shot};
use crate::config::{self, Config};
use crate::export::{self, Task};
use crate::fonts;
use crate::hotkey::{HotEvent, Hotkeys};
use crate::keymap::{Action, Keymap};
use crate::objects::{FRect, Obj, Pt};
use crate::pixbuf::PixBuf;
use crate::theme::{self, Theme};
use crate::raster::Order;
use crate::rec_ui::{self, RecOpts};
use crate::uifb::C4;
use crate::uisize::UiSize;
use compose::PixBufBackdrop;
use crate::wind::{self, Cursor, Driver, Ev, Hwnd, Mods};
use crate::fonts::UiFont;
use crate::text::AnnotFont;
use chrome::ToastKind;
use toolbar::{Act, Dev, Toolbar};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod toolbar;
mod panel;
mod chrome;
mod compose;
mod pick;
mod tools;
mod draw;
mod mini;
mod select;
mod input;
mod assist;
pub use assist::AcceptOn;
// The editor in a normal window: the Edit window's canvas.
pub mod host;

/// Chrome styling shared with the window control kit (`crate::ui`).
pub(crate) mod style {
    pub(crate) use super::chrome::{chord_caps, fill, ring, round, surface, Ui, ALL_LAYERS};
    pub(crate) use super::chrome::{toast, toast_size, ToastKind};
    pub(crate) use super::toolbar::BTN;
}
#[cfg(windows)]
mod gdi;
/// Linux/macOS: no GDI backend (`Edit::gdi` is always `None`).
#[cfg(not(windows))]
mod gdi {
    pub struct GdiScreen;
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    OneShot,
    Daemon,
}

/// Everything needed to start one capture.
pub struct Pending {
    pub tasks: Vec<Task>,
    pub screen: Option<u32>,
    /// Initial selection in global (virtual-screen physical) coords.
    pub region: Option<(i32, i32, u32, u32)>,
    pub accept_on_select: bool,
    pub filename: Option<String>,
    /// Started by the record hotkey: the Record popover opens with the
    /// selection and Enter starts recording.
    pub record: bool,
    /// Select the whole capture (the full-screen hotkey; with
    /// `accept_on_select` it is saved at once).
    pub select_all: bool,
    /// Snap to windows for this capture even when `snap_windows` is off
    /// (the capture-window hotkey).
    pub pick_window: bool,
    /// Accept on select for this capture (`--accept-on-select`), over
    /// the setting.
    pub accept_new: Option<AcceptOn>,
}

impl Pending {
    pub fn editor() -> Self {
        Self {
            tasks: Vec::new(),
            screen: None,
            region: None,
            accept_on_select: false,
            filename: None,
            record: false,
            select_all: false,
            pick_window: false,
            accept_new: None,
        }
    }

    /// The full-screen hotkey: everything, saved straight away.
    pub fn full() -> Self {
        Self { select_all: true, accept_on_select: true, tasks: vec![Task::Save { path: None, ask: false }], ..Self::editor() }
    }

    /// The capture-window hotkey: the overlay outlines windows, a click
    /// selects one.
    pub fn window() -> Self {
        Self { pick_window: true, ..Self::editor() }
    }

    /// Repeat last capture: `region` (global physical px) again on
    /// `screen`, saved straight away.
    pub fn repeat(region: (i32, i32, u32, u32), screen: Option<u32>) -> Self {
        Self { region: Some(region), screen, accept_on_select: true, tasks: vec![Task::Save { path: None, ask: false }], ..Self::editor() }
    }

    /// The copy-text hotkey: pick an area, then its text is recognized
    /// and copied (no annotation step).
    pub fn text() -> Self {
        Self { accept_on_select: true, tasks: vec![Task::Text], ..Self::editor() }
    }

    /// Area selection in record mode (the record hotkey).
    pub fn record() -> Self {
        Self { record: true, ..Self::editor() }
    }
}

pub type UploadSlot = Arc<Mutex<Option<Receiver<Result<String, String>>>>>;

/// `base` with `objs` baked in over the whole image (the spotlights dim
/// all of it), exactly as the overlay exports a selection of that size:
/// History's thumbnails and the Edit window's saves.
pub fn bake_objects(base: &PixBuf, objs: &[Obj], font: Option<&AnnotFont>) -> PixBuf {
    let mut out = base.clone();
    if objs.is_empty() || base.width() == 0 || base.height() == 0 {
        return out;
    }
    let size = base.dimensions();
    let img = compose::PxRect::image(size);
    let iw = base.width() as usize;
    let bg = |r: compose::PxRect, out: &mut [u8]| {
        let n = r.w() as usize * 4;
        for (i, y) in (r.y0..r.y1).enumerate() {
            out[i * n..][..n].copy_from_slice(&base.as_raw()[(y as usize * iw + r.x0 as usize) * 4..][..n]);
        }
    };
    let area = compose::spot_area(None, size);
    let bake = compose::Bake { objs, font, pass: crate::objects::SpotPass::of(objs, area), img, bg: &bg };
    bake.window(out.as_raw_mut(), img, Order::Rgba);
    out
}

/// A transient bottom-centre notice.
struct Toast {
    text: String,
    kind: ToastKind,
    at: Instant,
    /// Visible time before the fade-out starts.
    ttl: f32,
}

impl Toast {
    fn new(text: impl Into<String>, kind: ToastKind) -> Self {
        let ttl = if kind == ToastKind::Error { 4000.0 } else { 1600.0 };
        Toast { text: text.into(), kind, at: Instant::now(), ttl }
    }

    fn ttl_ms(&self) -> f32 {
        self.ttl
    }

    fn age_ms(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.at).as_secs_f32() * 1000.0
    }

    const FADE_IN: f32 = 120.0;
    const FADE_OUT: f32 = 100.0;

    fn expired(&self, now: Instant) -> bool {
        self.age_ms(now) > self.ttl_ms() + Self::FADE_OUT
    }

    /// Fade + rise in over 120 ms, fade out over 100 ms after the TTL.
    fn opacity(&self, now: Instant) -> f32 {
        let a = self.age_ms(now);
        let fade_in = crate::anim::ease_out(a / Self::FADE_IN);
        let fade_out = (1.0 - (a - self.ttl_ms()) / Self::FADE_OUT).clamp(0.0, 1.0);
        fade_in.min(fade_out)
    }

    /// Needs the fast tick (fading in, or close enough to fading out).
    fn animating(&self, now: Instant) -> bool {
        let a = self.age_ms(now);
        a < Self::FADE_IN || a > self.ttl_ms() - 160.0
    }
}

/// Overlay animation state (spec "Motion").
struct Motion {
    dim: Tween,
    bar: Tween,
    pop: Tween,
    hint: Tween,
    hover: Tween,
}

impl Motion {
    fn new(now: Instant) -> Self {
        let mut dim = Tween::new(0.0, now);
        dim.set(1.0, 120, now);
        let mut hint = Tween::new(0.0, now);
        hint.set(1.0, 120, now);
        Motion {
            dim,
            bar: Tween::new(0.0, now),
            pop: Tween::new(0.0, now),
            hint,
            hover: Tween::new(1.0, now),
        }
    }

    fn active(&self, now: Instant) -> bool {
        [&self.dim, &self.bar, &self.pop, &self.hint, &self.hover].iter().any(|t| t.active(now))
    }
}

/// Hotkeys that did not register: on stderr, and in the Settings window
/// when it is open, else a tray balloon.
fn tell_hotkey_failures(failed: &[crate::hotkey::Failure]) {
    if failed.is_empty() {
        return;
    }
    let text = failed.iter().map(|f| f.message()).collect::<Vec<_>>().join("\n");
    eprintln!("rustshot: {text}");
    if !crate::settings_ui::notify(&text) {
        crate::tray::notify(&text);
    }
}

pub fn run(
    cfg: Config,
    kind: RunKind,
    pending: Option<Pending>,
    exit_code: Arc<AtomicI32>,
    upload_slot: UploadSlot,
    instance: Option<crate::instance::Guard>,
) -> i32 {
    // Settings, the update dialog and the notice follow the UI size too.
    publish_ui_size(cfg.ui_scale);
    if cfg.snap_windows && !cfg!(test) {
        crate::snap::warm_up();
    }
    let hot = match kind {
        RunKind::Daemon => Some(Hotkeys::new(&cfg)),
        RunKind::OneShot => None,
    };
    // A later launch of the daemon asks this one to capture; the guard lives until we return.
    if let (Some(g), Some(h)) = (&instance, &hot) {
        g.listen(h.sender());
        crate::tray::spawn(h.sender(), cfg.show_tray);
    }
    if let Some(h) = &hot {
        // The update worker asks us to go idle (`RestartWhenIdle`), then to
        // quit once the new version starts (`Restart`).
        crate::update_ui::set_quit_sender(h.sender());
        // A save in the Settings window makes us reload the config.
        crate::settings_ui::set_daemon(h.sender());
        // The launcher's Capture comes here.
        crate::launcher::set_daemon(h.sender());
    }
    let updates = match kind {
        RunKind::Daemon => crate::update::spawn_checker(cfg.check_updates),
        RunKind::OneShot => None,
    };
    let font = AnnotFont::load();
    let ui_font = Some(fonts::ui_font());
    let mut app = App {
        cfg,
        kind,
        hot,
        pending,
        st: State::Hidden,
        exit_code,
        upload_slot,
        ocr: Vec::new(),
        updates,
        update_pending: None,
        restart_pending: false,
        updating: false,
        idle_reply: None,
        edits: crate::edit_ui::windows(),
        quit_pending: false,
        font,
        ui_font,
        notice: None,
        hwnd: Hwnd::default(),
        mouse: (0, 0),
        focus_tries: 0,
        rec: None,
        rec_opts: None,
        last_region: None,
        rec_area: None,
        wait_pins: false,
        // Tests never write the user's history.
        history_root: (!cfg!(test)).then(crate::history::find_root).flatten(),
        last_entry: None,
        upload_entry: None,
        rec_poster: None,
        delay: Default::default(),
        delay_show: crate::launcher::Screen { th: theme::DARK, up: false },
        #[cfg(test)]
        rec_spec: None,
        #[cfg(test)]
        rec_cam: None,
        #[cfg(test)]
        rec_bubble: None,
    };
    app.keep_pruned();
    // `rustshot launcher` without a daemon: its countdown, then the capture.
    if app.one_shot()
        && app.pending.is_none()
        && let Some(plan) = crate::launcher::take_start()
    {
        app.delay_show.th = theme::resolve(&app.cfg);
        app.pending = app.delay.start(plan, Instant::now(), &mut app.delay_show);
    }
    if wind::run(&mut app) != 0 {
        return 1;
    }
    drop(instance);
    app.exit_code.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// App state machine
// ---------------------------------------------------------------------------

enum State {
    /// Waiting for a hotkey (daemon) or about to close (one-shot).
    Hidden,
    /// Interactive editor is up.
    Edit(Box<Edit>),
    /// Window is hidden; running export tasks (may show a file dialog).
    Finish(Box<FinishJob>),
}

/// Text recognition running on its worker (Copy text): where its
/// notice goes and in which theme.
struct OcrJob {
    rx: Receiver<Result<String, crate::ocr::OcrError>>,
    at: (i32, i32),
    th: Theme,
}

/// Work to do after the overlay has been hidden again.
struct FinishJob {
    img: PixBuf,
    sel_global: (i32, i32),
    tasks: Vec<Task>,
    cfg: Config,
    /// What the history keeps of it (history on).
    hist: Option<Box<HistShot>>,
}

/// A capture for the history: the plain selection, its objects in its
/// coordinates, and where it came from.
struct HistShot {
    original: PixBuf,
    objects: Vec<Obj>,
    title: String,
    app: String,
    monitor: String,
    scale: f32,
}

/// Whether an export with `tasks` is a capture the history keeps: a
/// picture that was copied, saved, uploaded or pinned (not Copy text or
/// the CLI's raw output alone).
fn history_keeps(tasks: &[Task]) -> bool {
    tasks.iter().any(|t| matches!(t, Task::Copy | Task::Save { .. } | Task::SaveDesktop | Task::Upload | Task::Pin))
}

struct App {
    cfg: Config,
    kind: RunKind,
    hot: Option<Hotkeys>,
    pending: Option<Pending>,
    st: State,
    exit_code: Arc<AtomicI32>,
    upload_slot: UploadSlot,
    /// Copy text: the recognitions in flight (each delivers its result).
    ocr: Vec<OcrJob>,
    /// Results from the background update checker (daemon only).
    updates: Option<Receiver<crate::update::Release>>,
    /// A newer release found; shown in the update dialog once no capture is up.
    update_pending: Option<crate::update::Release>,
    /// An installed update asked us to quit (`HotEvent::Restart`); done
    /// once no capture is open.
    restart_pending: bool,
    /// An update is installing (`HotEvent::RestartWhenIdle`): no new capture.
    updating: bool,
    /// Told (once) as soon as we are idle; the worker installs only then.
    idle_reply: Option<std::sync::mpsc::Sender<()>>,
    /// This process's Edit windows: quitting asks those with unsaved
    /// edits, an update's restart waits for them.
    edits: &'static crate::edit_ui::Windows,
    /// Quit was asked while Edit windows ask "Save changes?".
    quit_pending: bool,
    font: Option<AnnotFont>,
    ui_font: Option<&'static UiFont>,
    notice: Option<Toast>,
    hwnd: Hwnd,
    /// Last known mouse position (client == image coordinates).
    mouse: (i32, i32),
    focus_tries: u8,
    /// The recording in progress (the overlay is hidden meanwhile).
    rec: Option<rec_ui::Run>,
    /// The Record panel's last choices (until the config is reloaded).
    rec_opts: Option<RecOpts>,
    /// The last exported capture's region (global physical px), for
    /// Repeat last capture.
    last_region: Option<(i32, i32, u32, u32)>,
    /// The recorded area (physical px): the toast after it goes there.
    rec_area: Option<capture::IRect>,
    /// A one-shot run that pinned: it closes once its pins are gone.
    wait_pins: bool,
    /// The history folder (`None`: not kept; tests).
    history_root: Option<std::path::PathBuf>,
    /// The last capture's history entry, and the one whose upload runs.
    last_entry: Option<String>,
    upload_entry: Option<String>,
    /// The running recording's poster and source, for its entry.
    rec_poster: Option<Box<HistShot>>,
    /// A delayed capture counting down (the launcher, the tray), and
    /// how it is shown (the pill, the tray's tip).
    delay: crate::launcher::Delay,
    delay_show: crate::launcher::Screen,
    /// Tests: the spec the last recording started with.
    #[cfg(test)]
    rec_spec: Option<crate::rec::RecSpec>,
    /// Tests: the camera bubble the last recording was handed.
    #[cfg(test)]
    rec_cam: Option<(String, crate::cam::Look, capture::IRect, f32)>,
    /// Tests: the last recording got a bubble (Some), and it showed a
    /// picture (true) when it started.
    #[cfg(test)]
    rec_bubble: Option<bool>,
}

impl App {
    fn one_shot(&self) -> bool {
        self.kind == RunKind::OneShot
    }

    fn request_exit(&mut self, code: i32) {
        self.save_before_exit();
        self.exit_code.store(code, Ordering::SeqCst);
        wind::close(self.hwnd);
    }

    /// Quitting under an open capture: an open picker keeps its color (as
    /// closing the overlay does), then the remembered tool settings are
    /// written before we go.
    fn save_before_exit(&mut self) {
        if let State::Edit(e) = &mut self.st {
            e.close_picker(true);
            e.flush_pending(true);
        }
    }

    /// Run hotkey/upload polling and advance the state machine until stable.
    fn pump(&mut self) {
        while let Some(ev) = self.hot.as_mut().and_then(|h| h.poll()) {
            if !self.on_hot(ev) {
                return;
            }
        }
        self.poll_upload();
        self.poll_ocr();
        self.poll_delay();
        if let Some(rx) = &self.updates {
            while let Ok(r) = rx.try_recv() {
                // macOS has no dialog (windows need the main thread): the
                // release is announced on stderr and in the next capture.
                #[cfg(target_os = "macos")]
                eprintln!("Rustshot {} is available — run `rustshot update`", r.version);
                self.update_pending = Some(r);
            }
        }
        // Never over a capture in progress: the dialog would take its focus.
        #[cfg(not(target_os = "macos"))]
        if matches!(self.st, State::Hidden)
            && self.pending.is_none()
            && let Some(r) = self.update_pending.take()
        {
            crate::update_ui::offer(crate::update_ui::DialogState::Available(r));
        }
        if self.notice.as_ref().is_some_and(|t| t.expired(Instant::now())) {
            self.notice = None;
        }
        self.tick_recording();
        if let State::Edit(e) = &mut self.st
            && e.pending.is_due(Instant::now())
        {
            e.flush_pending(false);
        }
        if let State::Edit(e) = &mut self.st
            && (e.rec.poll_cams() | e.rec.poll_mp4())
        {
            self.repaint(); // the Camera row or the MP4 choice is ready
        }
        self.sync_bubble();
        self.sync_meter();

        for _ in 0..16 {
            match std::mem::replace(&mut self.st, State::Hidden) {
                State::Hidden => {
                    if let Some(pending) = self.pending.take() {
                        self.begin_capture(pending);
                        continue;
                    }
                    break;
                }
                State::Finish(job) => self.run_finish(*job),
                State::Edit(edit) => {
                    let mut edit = *edit;
                    // Windows only delivers keys to the foreground window;
                    // retry focus for a few events after the overlay shows.
                    if self.focus_tries < 5 {
                        self.focus_tries += 1;
                        capture::focus_our_window();
                    }
                    let had = edit.notice.is_some();
                    if edit.notice.as_ref().is_some_and(|t| t.expired(Instant::now())) {
                        edit.notice = None;
                    }
                    let cleared = had && edit.notice.is_none();
                    self.st = State::Edit(Box::new(edit));
                    if cleared {
                        self.repaint();
                    }
                    break;
                }
            }
        }
        // A one-shot run with pins: done once the last one is closed.
        if self.wait_pins && crate::pin::count() == 0 {
            self.wait_pins = false;
            wind::close(self.hwnd);
        }
        // Quitting once the Edit windows have answered.
        self.quit_gate();
        // An update waits for us to go idle before it installs.
        self.idle_gate();
        // An update is restarting us: quit as soon as no capture is open
        // (never under the user's open capture or export).
        if self.restart_due() {
            self.request_exit(0);
        }
    }

    /// Handle one daemon event; false once we are quitting.
    fn on_hot(&mut self, ev: HotEvent) -> bool {
        match ev {
            HotEvent::Quit => {
                // Edit windows with unsaved edits ask first.
                if self.edits.ask_quit() > 0 {
                    self.quit_pending = true;
                    return true;
                }
                self.request_exit(0);
                return false;
            }
            HotEvent::Restart => self.restart_pending = true,
            HotEvent::RestartWhenIdle(reply) => {
                self.updating = true;
                self.idle_reply = Some(reply);
            }
            HotEvent::RestartAborted => {
                self.updating = false;
                self.idle_reply = None;
            }
            // A file that stopped parsing is not taken as "all defaults".
            HotEvent::ReloadConfig => match crate::config::read_at(&crate::config::config_path()) {
                Ok(cfg) => self.reload_config(cfg.unwrap_or_default()),
                Err(e) => eprintln!("rustshot: config.toml not reloaded: {e}"),
            },
            // The record hotkey stops a recording; so do the tray and the bar.
            HotEvent::Record | HotEvent::StopRecording if self.rec.is_some() => {
                if let Some(r) = self.rec.as_mut() {
                    r.stop(Instant::now());
                }
            }
            HotEvent::StopRecording => {}
            HotEvent::ClosePins => crate::pin::close_all(),
            HotEvent::CancelDelay => {
                self.delay.cancel(&mut self.delay_show);
            }
            HotEvent::Launch(_) if self.rec.is_some() => eprintln!("rustshot: a recording is running; capture ignored"),
            HotEvent::Launch(_) if self.updating || self.restart_pending => eprintln!("rustshot: an update is installing; capture ignored"),
            HotEvent::Launch(plan) => {
                self.delay_show.th = theme::resolve(&self.cfg);
                if let Some(p) = self.delay.start(plan, Instant::now(), &mut self.delay_show) {
                    if matches!(self.st, State::Hidden) && self.pending.is_none() {
                        self.pending = Some(p);
                    } else {
                        self.say_capture_open();
                    }
                }
            }
            HotEvent::UploadDelete(tag) => match crate::uploads::ask_delete(tag) {
                Some(n) => {
                    #[cfg(not(test))]
                    let at = capture::cursor_pos();
                    #[cfg(test)]
                    let at = (0, 0);
                    if !crate::notice::show(n, at, theme::resolve(&self.cfg)) {
                        crate::tray::notify("Delete it from the History window (no notice window here)");
                    }
                }
                None => crate::tray::notify("That upload is no longer listed"),
            },
            // History opens over anything (a recording, another window).
            HotEvent::History => crate::history_ui::show(),
            // The pill's buttons from the tray or a while-recording chord.
            HotEvent::RecPause | HotEvent::RecMute | HotEvent::RecCamera | HotEvent::RecPill | HotEvent::RecDiscard => {
                let now = Instant::now();
                if let Some(r) = self.rec.as_mut() {
                    match ev {
                        HotEvent::RecPause => r.toggle_pause(now),
                        HotEvent::RecMute => r.toggle_mute(),
                        HotEvent::RecCamera => r.toggle_camera(),
                        HotEvent::RecPill => r.toggle_pill(now),
                        _ => r.discard(now),
                    }
                }
                self.publish_rec_menu();
            }
            // A global hotkey: inside the overlay its chord is the
            // overlay's key (Ctrl+Shift+S, if bound globally: Save As there), else its action.
            // Quit always quits.
            HotEvent::Pressed(i) if i != 1 && i != 7 && matches!(self.st, State::Edit(_)) => self.forward_hotkey(i),
            HotEvent::Pressed(i) => return self.on_hot(crate::hotkey::slot_event(i)),
            // A capture would show the overlay in the recording.
            HotEvent::Capture | HotEvent::Record | HotEvent::CaptureFull | HotEvent::CaptureWindow | HotEvent::RepeatLast | HotEvent::CaptureText
                if self.rec.is_some() =>
            {
                eprintln!("rustshot: a recording is running; capture ignored");
            }
            // No new capture while an update installs or restarts us.
            HotEvent::Capture | HotEvent::Record | HotEvent::CaptureFull | HotEvent::CaptureWindow | HotEvent::RepeatLast | HotEvent::CaptureText
                if self.updating || self.restart_pending =>
            {
                eprintln!("rustshot: an update is installing; capture ignored");
            }
            _ if !matches!(self.st, State::Hidden) => {} // a capture is open
            HotEvent::Capture => self.pending = Some(Pending::editor()),
            HotEvent::CaptureFull => self.pending = Some(Pending::full()),
            HotEvent::CaptureWindow => self.pending = Some(Pending::window()),
            HotEvent::RepeatLast => self.pending = Some(self.repeat_pending()),
            HotEvent::CaptureText => self.pending = Some(Pending::text()),
            HotEvent::Record if !rec_ui::supported() => eprintln!("rustshot: recording is not supported on this platform yet"),
            HotEvent::Record => self.pending = Some(Pending::record()),
        }
        true
    }

    /// The delayed capture's countdown: its capture once it is due (not
    /// over an open one, a recording or an update); a one-shot run that
    /// was called off exits (2, as a cancelled capture).
    fn poll_delay(&mut self) {
        if !self.delay.active() {
            return;
        }
        let busy = !matches!(self.st, State::Hidden) || self.pending.is_some() || self.rec.is_some() || self.updating || self.restart_pending;
        let now = Instant::now();
        match self.delay.poll(now, &mut self.delay_show) {
            Some(p) if !busy => self.pending = Some(p),
            Some(_) => self.say_capture_open(),
            None if self.one_shot() && !self.delay.active() && self.pending.is_none() && matches!(self.st, State::Hidden) => self.request_exit(2),
            None => {}
        }
    }

    /// A launch found a capture open (or a recording, an update): say so
    /// over the capture, else in a balloon.
    fn say_capture_open(&mut self) {
        let text = "Capture already open";
        eprintln!("rustshot: {text}; the launch was dropped");
        if let State::Edit(e) = &mut self.st {
            e.notice = Some(Toast::new(text.to_string(), ToastKind::Info));
            self.repaint();
        } else {
            crate::tray::notify(text);
        }
    }

    /// A global hotkey pressed while the overlay is open: its chord goes
    /// to the overlay as a key press (the OS took it from the window).
    fn forward_hotkey(&mut self, slot: usize) {
        let spec = crate::hotkey::wanted(&self.cfg)[slot].clone();
        let Some(c) = crate::keymap::Chord::parse(&spec) else { return };
        if c.meta {
            return; // the overlay has no Meta chords
        }
        self.key(c.vk, false, Mods { ctrl: c.ctrl, shift: c.shift, alt: c.alt });
        self.repaint();
    }

    /// Repeat last capture: the last region again (on its monitor when the
    /// monitors do not share one scale), or a plain capture when there was
    /// none yet.
    fn repeat_pending(&self) -> Pending {
        let Some(r) = self.last_region else { return Pending::editor() };
        #[cfg(not(test))]
        let screen = capture::monitors().ok().and_then(|m| crate::actions::monitor_for_region(&m, r));
        #[cfg(test)]
        let screen = None;
        Pending::repeat(r, screen)
    }

    /// The camera bubble follows the Record popover: up while it is open
    /// with the camera on and a selection (placed by the selection), else
    /// closed. When the camera cannot be used, a toast says why and the
    /// camera is switched off.
    fn sync_bubble(&mut self) {
        let nudge = self.hwnd;
        let State::Edit(e) = &mut self.st else { return };
        let want = rec_ui::supported() && e.rec.open && e.rec.opts.camera && e.rec.has_cams() && e.sel.is_some() && e.tasks.is_empty();
        if !want {
            e.rec.cam.close();
            e.rec.grip = None;
            return;
        }
        // The wheel over the bubble resized it: the panel follows.
        if let Some(z) = crate::cam::take_resized() {
            e.rec.opts.cam_size = z;
        }
        #[cfg(not(test))]
        let mons = e.rec.cam.mons.get_or_insert_with(|| capture::monitors().unwrap_or_default()).clone();
        #[cfg(test)]
        let mons = Vec::new();
        let geom = e.rec_geom(&mons);
        let mut want = e.cam_want(&geom);
        want.avoid = e.toolbar_screen_rects();
        if let Some(err) = e.rec.cam.sync(&want, nudge) {
            e.rec.opts.camera = false;
            e.notice = Some(Toast::new(err, ToastKind::Error));
        }
        // The grip hint goes under the bubble (not while it is dragged).
        let grip = e.rec.cam.shape_rect().map(|(x, y, d)| {
            let (ox, oy) = e.shot.origin;
            FRect { x: (x - ox) as f32, y: (y - oy) as f32, w: d as f32, h: d as f32 }
        });
        if grip != e.rec.grip {
            e.rec.grip = grip;
            self.repaint();
        }
    }

    /// The microphone's level for the panel: metered while video mode is up
    /// with the microphone on (MP4), released otherwise; looked at 20
    /// times a second, a frame only when the bars change.
    fn sync_meter(&mut self) {
        let State::Edit(e) = &mut self.st else { return };
        let o = &e.rec.opts;
        let want = rec_ui::supported() && e.rec.open && e.tasks.is_empty() && o.mic && o.format == crate::rec::RecFormat::Mp4;
        if !want {
            e.rec.meter = None;
            e.rec.bars = (0, false);
            return;
        }
        if e.rec.meter.as_ref().is_none_or(|m| m.device() != o.mic_dev) {
            e.rec.meter = Some(rec_ui::meter::MicMeter::open(&o.mic_dev));
        }
        let now = Instant::now();
        if now.saturating_duration_since(e.rec.meter_at) < Duration::from_millis(rec_ui::meter::POLL_MS) {
            return;
        }
        e.rec.meter_at = now;
        let bars = rec_ui::meter::bars(e.rec.meter.as_ref().map_or(0.0, |m| m.peak()));
        if bars != e.rec.bars {
            e.rec.bars = bars;
            self.repaint();
        }
    }

    /// Advance the recording; once it is over, say how it went.
    fn tick_recording(&mut self) {
        let Some(run) = self.rec.as_mut() else { return };
        // Wayland: the desktop put the bubble or the bar in a bad place.
        if let Some(hint) = run.take_hint() {
            eprintln!("rustshot: {hint}");
            if !self.one_shot() {
                crate::tray::notify(hint);
            }
        }
        let Some(run) = self.rec.as_mut() else { return };
        let now = Instant::now();
        let ended = run.tick(now);
        // "No mic input": the microphone has been silent for 5 s.
        if let Some(n) = run.take_notice()
            && !self.one_shot()
        {
            let at = self.rec_area.map_or((0, 0), |a| (a.0 + a.2 as i32 / 2, a.1 + a.3 as i32 / 2));
            if !crate::notice::show(crate::notice::Note::plain(n, ToastKind::Error), at, theme::resolve(&self.cfg)) {
                crate::tray::notify(n);
            }
        }
        let Some(run) = self.rec.as_ref() else { return };
        let length = run.length(now);
        // The tray's tip says the time recorded (once a second).
        if !self.one_shot() && ended.is_none() {
            crate::tray::set_elapsed(length);
            // ...and its menu what the pill shows (paused, muted, ...).
            self.publish_rec_menu();
        }
        let Some(end) = ended else { return };
        self.rec = None;
        self.recording_over(end, length);
    }

    /// Say how a recording ended: on stdout/stderr, and in a tray balloon
    /// for the daemon. A one-shot run (no tray, no hotkeys: the bar's Stop
    /// ends it) exits like a capture: 0 saved, 1 failed, 2 cancelled.
    /// A saved recording is said in the toast with its buttons (Copy link
    /// or Copy, Open, Show in folder; a tray balloon where there is no toast
    /// window); `length` is the time recorded.
    fn recording_over(&mut self, end: rec_ui::End, length: Duration) {
        let daemon = !self.one_shot();
        if daemon {
            crate::tray::set_recording(false);
            self.rec_hotkeys(false);
        }
        let poster = self.rec_poster.take();
        let code = match end {
            rec_ui::End::Saved { path, warnings } => {
                println!("saved: {}", path.display());
                let mut entry = None;
                if let (Some(root), Some(p)) = (self.history_root.clone(), poster) {
                    let gif = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"));
                    let a = self.rec_area.unwrap_or((0, 0, 0, 0));
                    let meta = crate::history::Meta {
                        captured: export::local_ymdhms(),
                        kind: if gif { crate::history::Kind::Gif } else { crate::history::Kind::Mp4 },
                        w: a.2,
                        h: a.3,
                        title: p.title,
                        app: p.app,
                        monitor: p.monitor,
                        scale: p.scale,
                        duration_ms: length.as_millis() as u64,
                        file: Some(path.clone()),
                        ..Default::default()
                    };
                    entry = crate::history::record_recording(&self.cfg, &root, Some(p.original), meta);
                }
                for w in &warnings {
                    eprintln!("warning: {w}");
                }
                if daemon {
                    let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
                    let mut note = crate::notice::recording_saved(&path, length, bytes, &warnings, crate::notice::upload_configured(&self.cfg));
                    // Copy link files its record with the entry.
                    note.entry = entry;
                    let at = self.rec_area.map_or((0, 0), |a| (a.0 + a.2 as i32 / 2, a.1 + a.3 as i32 / 2));
                    if !crate::notice::show(note, at, theme::resolve(&self.cfg)) {
                        crate::tray::notify_saved(&path, &warnings);
                    }
                }
                0
            }
            rec_ui::End::Failed(e) => {
                eprintln!("error: recording failed: {e}");
                if daemon {
                    crate::tray::notify(&format!("Recording failed: {e}"));
                }
                1
            }
            rec_ui::End::Cancelled => 2,
        };
        if self.one_shot() {
            self.request_exit(code);
        }
    }

    /// Start recording `area` (physical px) with `opts`: the overlay is
    /// already hidden.
    /// `bubble`: the camera bubble (the popover's, or opened for the
    /// recording), up until the recording is over.
    fn start_recording(&mut self, cfg: &Config, opts: &RecOpts, geom: rec_ui::Geom, th: Theme, bubble: Option<crate::cam::Bubble>) {
        let spec = opts.spec(cfg, geom.area, export::local_ymdhms());
        self.rec_area = Some(geom.area);
        // The panel's Countdown (0 / 3s); a GIF stops by itself at its max length.
        let countdown = opts.countdown;
        #[cfg(test)]
        {
            #[cfg(windows)]
            {
                self.rec_bubble = bubble.as_ref().map(|b| b.has_picture());
            }
            #[cfg(not(windows))]
            {
                self.rec_bubble = bubble.as_ref().map(|_| false);
            }
            let _ = (th, bubble);
            self.rec_spec = Some(spec);
            self.recording_started(Ok(rec_ui::Run::fake(countdown, opts.mic, Instant::now()).with_max(opts.max_len())));
        }
        #[cfg(not(test))]
        {
            let run = rec_ui::Run::start(spec, geom, th, countdown, self.hwnd, bubble).map(|r| r.with_max(opts.max_len()));
            self.recording_started(run);
        }
    }

    /// The recording is up, or it could not start (said like a failure).
    fn recording_started(&mut self, run: anyhow::Result<rec_ui::Run>) {
        match run {
            Ok(run) => {
                self.rec = Some(run);
                if !self.one_shot() {
                    crate::tray::set_recording(true);
                    self.publish_rec_menu();
                    self.rec_hotkeys(true);
                }
            }
            Err(e) if e.is::<rec_ui::NotStarted>() => self.recording_over(rec_ui::End::Cancelled, Duration::ZERO),
            Err(e) => self.recording_over(rec_ui::End::Failed(format!("{e:#}")), Duration::ZERO),
        }
    }

    /// The tray menu follows the recording's pill (daemon only).
    fn publish_rec_menu(&self) {
        if let Some(r) = &self.rec
            && !self.one_shot()
        {
            crate::tray::set_rec_menu(r.menu_state());
        }
    }

    /// A recording started (`on`) or ended: its global chords (pause,
    /// mute, hide the pill) are registered only meanwhile. A chord that
    /// does not register is told like any other hotkey.
    fn rec_hotkeys(&mut self, on: bool) {
        if let Some(h) = self.hot.as_mut() {
            let failed = h.set_recording(&self.cfg, on);
            tell_hotkey_failures(&failed);
        }
    }

    /// Take `cfg` (Settings saved it): the next capture uses it, the global
    /// hotkeys are re-registered and the update checker follows
    /// `check_updates`. An open capture keeps the config it started with.
    fn reload_config(&mut self, cfg: Config) {
        if let Some(h) = self.hot.as_mut() {
            let mut failed = h.reload(&cfg);
            // While recording, its chords follow the new key bindings.
            if self.rec.is_some() {
                failed.extend(h.set_recording(&cfg, true));
            }
            tell_hotkey_failures(&failed);
        }
        if cfg.check_updates != self.cfg.check_updates && self.kind == RunKind::Daemon {
            // A new checker generation: the old one stops before its next
            // check (and never takes the day's stamp).
            self.updates = crate::update::spawn_checker(cfg.check_updates);
        }
        // The Record popover starts from the (new) defaults again.
        self.rec_opts = None;
        if self.kind == RunKind::Daemon && cfg.show_tray != self.cfg.show_tray {
            crate::tray::set_visible(cfg.show_tray);
        }
        publish_ui_size(cfg.ui_scale);
        self.cfg = cfg;
        // New limits apply at once.
        self.keep_pruned();
    }

    /// The daemon keeps the history within its limits (on or off; at
    /// start, hourly, when they change).
    fn keep_pruned(&self) {
        if self.kind == RunKind::Daemon
            && let Some(root) = &self.history_root
        {
            crate::history::keep_pruned(root, &self.cfg);
        }
    }

    /// No capture, editor, recording or export is open or about to open,
    /// and no upload is in flight.
    fn idle(&self) -> bool {
        matches!(self.st, State::Hidden)
            && self.pending.is_none()
            && self.rec.is_none()
            && self.upload_slot.lock().unwrap_or_else(|e| e.into_inner()).is_none()
            && self.ocr.is_empty()
            && self.edits.unsaved() == 0
    }

    /// A quit asked while Edit windows asked about their unsaved edits:
    /// done once none is left, given up when one was cancelled.
    fn quit_gate(&mut self) {
        if !self.quit_pending {
            return;
        }
        match crate::edit_ui::quit_gate(self.edits.unsaved(), self.edits.take_cancelled()) {
            crate::edit_ui::QuitGate::Now => {
                self.quit_pending = false;
                self.request_exit(0);
            }
            crate::edit_ui::QuitGate::Abort => self.quit_pending = false,
            crate::edit_ui::QuitGate::Wait => {}
        }
    }

    /// Answer a waiting update worker once we are idle. Captures stay
    /// refused afterwards; if the worker is gone it will not install, so
    /// they are allowed again.
    fn idle_gate(&mut self) {
        if self.idle_reply.is_some() && self.idle() {
            let reply = self.idle_reply.take().expect("checked");
            if reply.send(()).is_err() {
                self.updating = false;
            }
        }
    }

    /// A pending restart may happen now: we are idle. Clears the request
    /// when it says yes.
    fn restart_due(&mut self) -> bool {
        let due = self.restart_pending && self.idle();
        if due {
            self.restart_pending = false;
        }
        due
    }

    /// Timer tick: poll, then report whether the screen needs a new frame
    /// (an animation runs, the caret blinked, a toast came or went, the
    /// state switched). Idle ticks cost no frame.
    fn tick(&mut self) -> bool {
        let sig = |a: &App| std::mem::discriminant(&a.st);
        let before = sig(self);
        self.pump();
        let changed = sig(self) != before;
        // The countdown's arc drains smoothly: fast ticks while it is up.
        if let Some(r) = &self.rec {
            wind::set_fast_timer(self.hwnd, r.counting());
        }
        // macOS: the pins' input is handled on these ticks.
        crate::pin::tick();
        // macOS: the notice fades on these ticks (fast while it is up).
        if let Some(fast) = crate::notice::tick()
            && self.rec.is_none()
            && !matches!(self.st, State::Edit(_))
        {
            wind::set_fast_timer(self.hwnd, fast);
        }
        let State::Edit(edit) = &self.st else { return changed };
        let repaint = changed || stale(edit, self.notice.as_ref(), Instant::now());
        // The microphone's level is looked at 20 times a second.
        if !repaint && edit.rec.meter.is_none() {
            // No frame runs to re-evaluate the cadence: drop to idle.
            wind::set_fast_timer(self.hwnd, false);
        }
        repaint
    }

    /// Take the edit state out of `self`, hand it to `f`, then finish or
    /// put it back.
    fn edit_tx<F: FnOnce(&mut Self, &mut Edit)>(&mut self, f: F) {
        if !matches!(self.st, State::Edit(_)) {
            return;
        }
        let State::Edit(edit) = std::mem::replace(&mut self.st, State::Hidden) else {
            unreachable!("guarded by matches! above");
        };
        let mut edit = *edit;
        f(self, &mut edit);
        if edit.done {
            let cancelled = edit.cancelled;
            self.finish(&mut edit, cancelled);
        } else {
            self.st = State::Edit(Box::new(edit));
        }
    }

    fn begin_capture(&mut self, mut pending: Pending) {
        // The last capture's notice never sits over the new one.
        crate::notice::hide();
        let mut cfg = self.cfg.clone();
        if let Some(f) = pending.filename.take() {
            cfg.filename_pattern = f;
        }
        let screen = pending.screen.take();
        let region = pending.region.take();
        let tasks = std::mem::take(&mut pending.tasks);
        let accept_on_select = pending.accept_on_select;
        let accept_new = pending.accept_new;
        if pending.pick_window {
            cfg.snap_windows = true;
        }
        let rec_opts = self.rec_opts.clone().unwrap_or_else(|| RecOpts::from_config(&cfg));
        let rec = RecPop::new(rec_opts, pending.record && rec_ui::supported());

        let th = theme::resolve(&cfg);
        let (shot, gdi) = match grab(&cfg, &th, screen) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: capture failed: {e:#}");
                if self.one_shot() {
                    self.exit_code.store(1, Ordering::SeqCst);
                    wind::close(self.hwnd);
                }
                return;
            }
        };
        let initial_sel = region.map(|(x, y, w, h)| {
            let (ix, iy) = shot.global_to_image((x, y));
            let mut r = FRect {
                x: ix as f32,
                y: iy as f32,
                w: w as f32,
                h: h as f32,
            };
            r.clamp_to(shot.size.0 as f32, shot.size.1 as f32);
            r
        });
        // The capture is held once, as `base`; `shot` keeps only geometry.
        let mut shot = shot;
        let base = std::mem::take(&mut shot.image);
        let initial_sel = if pending.select_all {
            Some(FRect { x: 0.0, y: 0.0, w: shot.size.0 as f32, h: shot.size.1 as f32 })
        } else {
            initial_sel
        };
        let sel = initial_sel.filter(|r| !r.is_trivial());
        let accept_now = accept_on_select && sel.is_some();
        let mut edit = Edit::new(shot, base, cfg, th, self.font.clone(), self.ui_font, rec);
        edit.gdi = gdi;
        edit.sel = sel;
        edit.tasks = tasks;
        edit.accept_on_select = accept_on_select;
        if accept_new.is_some() {
            edit.assist.accept = accept_new;
        }
        // Tests never write the user's config.
        edit.cfg_path = (!cfg!(test)).then(config::config_path);
        // The window in front before the overlay is: the history's source.
        if !cfg!(test) && self.history_root.is_some() {
            edit.source = crate::history::source::foreground();
        }
        #[cfg(target_os = "macos")]
        if let Some(r) = self.update_pending.take() {
            let mut t = Toast::new(format!("Rustshot {} is available — run `rustshot update`", r.version), ToastKind::Info);
            t.ttl = 6000.0;
            edit.notice = Some(t);
        }
        // The pointer where it is (the magnifier shows there at once),
        // and each monitor's scale (the magnifier's size on mixed DPI).
        #[cfg(not(test))]
        {
            self.mouse = edit.shot.global_to_image(capture::cursor_pos());
            let o = edit.shot.origin;
            edit.assist.mon_scales = capture::monitors()
                .unwrap_or_default()
                .iter()
                .map(|m| ((m.x - o.0, m.y - o.1, m.w, m.h), m.scale))
                .collect();
        }
        if accept_now {
            if edit.tasks.is_empty()
                && let Some(m) = edit.assist.accept
            {
                edit.tasks = m.tasks(&edit.cfg);
            }
            edit.done = true;
            edit.cancelled = false;
        } else if edit.sel.is_none() {
            // The windows on screen now (the overlay is not up yet), and
            // the one under the pointer.
            edit.snap = take_snap(&edit.cfg, &edit.shot);
            if edit.snap.is_some() {
                let (x, y) = self.mouse;
                edit.track_snap(Pt::new(x as f32, y as f32));
            }
        }
        // Place the overlay at the exact physical rect of the shot (the
        // window is DPI-aware and borderless, so outer == inner).
        wind::show_at(
            self.hwnd,
            edit.shot.origin.0,
            edit.shot.origin.1,
            edit.shot.size.0 as i32,
            edit.shot.size.1 as i32,
        );
        self.focus_tries = 0;
        // The hint's 3 s count from now, the overlay on screen.
        edit.assist.restart_hint(edit.cfg.overlay_hints, Instant::now());
        if edit.done {
            let cancelled = edit.cancelled;
            self.finish(&mut edit, cancelled);
        } else {
            self.st = State::Edit(Box::new(edit));
        }
    }

    /// Start the finish sequence: crop now, hide the window, export next
    /// (so a native save dialog is never covered by the overlay).
    fn finish(&mut self, edit: &mut Edit, cancelled: bool) {
        wind::set_fast_timer(self.hwnd, false);
        // An open picker keeps its color; what the tools remember is
        // written now and taken by the next capture.
        edit.close_picker(true);
        edit.flush_pending(true);
        // Only what changed in this capture (a reset in Settings meanwhile
        // holds for the rest).
        for k in edit.pending.touched() {
            config::copy_key(&edit.cfg, &mut self.cfg, k);
        }
        self.rec_opts = Some(edit.rec.opts.clone());
        if edit.rec.start && !cancelled {
            // Recording: the overlay goes away, the area is recorded live.
            #[cfg(not(test))]
            let mons = capture::monitors().unwrap_or_default();
            #[cfg(test)]
            let mons = Vec::new();
            let geom = edit.rec_geom(&mons);
            // The history's poster: the area as it looks now.
            self.rec_poster = (self.history_root.is_some() && edit.cfg.history).then(|| {
                let full = FRect { x: 0.0, y: 0.0, w: edit.shot.size.0 as f32, h: edit.shot.size.1 as f32 };
                let poster = crate::scale::fit(&edit.export_plain(edit.sel.unwrap_or(full)), 400, 280);
                let a = geom.area;
                let name = if cfg!(test) { String::new() } else { crate::history::source::monitor_name(a.0 + a.2 as i32 / 2, a.1 + a.3 as i32 / 2) };
                Box::new(HistShot {
                    original: poster,
                    objects: Vec::new(),
                    title: edit.source.0.clone(),
                    app: edit.source.1.clone(),
                    monitor: crate::history::source::monitor_text(&name, edit.shot.scale),
                    scale: edit.shot.scale,
                })
            });
            wind::hide(self.hwnd);
            self.st = State::Hidden;
            let (cfg, opts, th) = (edit.cfg.clone(), edit.rec.opts.clone(), edit.th);
            #[cfg(test)]
            {
                self.rec_cam = edit.rec.cam.shown.clone();
            }
            let mut bubble = edit.rec.cam.take();
            // Started without the popover (Enter in record mode): the
            // camera opens now. Without a countdown the recording waits
            // (up to a second) for its first picture, so it does not
            // begin on the placeholder.
            if bubble.is_none() && opts.camera && rec_ui::supported() {
                edit.rec.wait_cams(CAM_LIST_WAIT);
                if edit.rec.has_cams() {
                    let want = edit.cam_want(&geom);
                    let wait = (!opts.countdown).then_some(FIRST_PICTURE_WAIT);
                    bubble = edit.rec.cam.open_now(&want, self.hwnd, wait);
                }
            }
            self.start_recording(&cfg, &opts, geom, th, bubble);
            return;
        }
        if cancelled {
            if self.one_shot() {
                self.exit_code.store(2, Ordering::SeqCst);
                wind::close(self.hwnd);
            } else {
                wind::hide(self.hwnd);
                self.st = State::Hidden;
            }
            return;
        }
        if edit.dirty {
            edit.rebuild();
        }
        let sel = edit.sel.unwrap_or(FRect {
            x: 0.0,
            y: 0.0,
            w: edit.shot.size.0 as f32,
            h: edit.shot.size.1 as f32,
        });
        #[cfg(windows)]
        crate::memlog("overlay");
        let img = edit.export(sel);
        let g = (
            (sel.x.round() as i32) + edit.shot.origin.0,
            (sel.y.round() as i32) + edit.shot.origin.1,
        );
        wind::hide(self.hwnd);
        self.last_region = Some((g.0, g.1, img.width(), img.height()));
        let mut tasks = std::mem::take(&mut edit.tasks);
        if self.kind == RunKind::Daemon {
            // A capture from the overlay or a hotkey (never `gui --path`).
            tasks = crate::actions::after_capture(tasks, &edit.cfg);
        }
        // Copying text is not a picture: no shutter.
        if edit.cfg.shutter_sound && !tasks.is_empty() && tasks != [Task::Text] {
            crate::actions::shutter();
        }
        // History: the capture as it was, its objects, where it came from.
        let hist = (self.history_root.is_some() && edit.cfg.history && history_keeps(&tasks)).then(|| {
            let (original, objects) = edit.history_shot(sel);
            let c = (g.0 + img.width() as i32 / 2, g.1 + img.height() as i32 / 2);
            let name = if cfg!(test) { String::new() } else { crate::history::source::monitor_name(c.0, c.1) };
            Box::new(HistShot {
                original,
                objects,
                title: edit.source.0.clone(),
                app: edit.source.1.clone(),
                monitor: crate::history::source::monitor_text(&name, edit.shot.scale),
                scale: edit.shot.scale,
            })
        });
        self.st = State::Finish(Box::new(FinishJob {
            img,
            sel_global: g,
            tasks,
            cfg: edit.cfg.clone(),
            hist,
        }));
    }

    /// Keep the exported capture in the history (its id), saying where it
    /// was saved.
    fn keep_history(&mut self, h: HistShot, res: &export::ExportResult, cfg: &Config) -> Option<String> {
        let root = self.history_root.clone()?;
        let saved = res.done.iter().rev().find_map(|d| match d {
            export::Done::Saved(p) | export::Done::SavedDesktop(p) => Some(p.clone()),
            _ => None,
        });
        let meta = crate::history::Meta {
            captured: export::local_ymdhms(),
            title: h.title,
            app: h.app,
            monitor: h.monitor,
            scale: h.scale,
            saved,
            ..Default::default()
        };
        crate::history::record(cfg, &root, crate::history::ShotRec { original: h.original, objects: h.objects, meta })
    }

    fn run_finish(&mut self, job: FinishJob) {
        let res = export::run_export(&job.img, job.sel_global, &job.tasks, &job.cfg);
        for m in &res.messages {
            println!("{m}");
        }
        if res.error {
            self.exit_code.store(1, Ordering::SeqCst);
        }
        let at = (job.sel_global.0 + job.img.width() as i32 / 2, job.sel_global.1 + job.img.height() as i32 / 2);
        let th = theme::resolve(&job.cfg);
        let entry = job.hist.and_then(|h| self.keep_history(*h, &res, &job.cfg));
        // Its link goes to the entry once the upload is done.
        self.upload_entry = entry.clone().filter(|_| res.upload.is_some());
        self.last_entry = entry.clone();
        let img = Arc::new(job.img);
        // Pin to screen: the image where it was captured.
        if job.tasks.contains(&Task::Pin) && !crate::pin::open(img.clone(), job.sel_global, th) {
            eprintln!("rustshot: pin to screen is not available here");
            if self.kind == RunKind::Daemon {
                crate::tray::notify("Pin to screen is not available here");
            }
        }
        // The overlay is gone: a notice says what happened (the daemon's
        // only window left; a one-shot run says it on stdout). A copied or
        // saved screenshot gets the thumbnail card instead (when on).
        if self.kind == RunKind::Daemon {
            let up = res.upload.is_some();
            let note = job
                .cfg
                .show_thumbnail
                .then(|| crate::notice::thumbnail(&res.done, up, img.clone(), job.sel_global, entry.clone()))
                .flatten()
                .or_else(|| crate::notice::compose(&res.done, up).map(|n| crate::notice::with_edit(n, entry.clone())));
            if let Some(note) = note {
                crate::notice::show(note, at, th);
            }
        }
        *self.upload_slot.lock().unwrap() = res.upload;
        if job.tasks.contains(&Task::Text) {
            match self.kind {
                // Off the UI thread; `poll_ocr` copies and says so.
                RunKind::Daemon => self.ocr.push(OcrJob { rx: crate::ocr::spawn(Arc::unwrap_or_clone(img)), at, th }),
                // The overlay is gone and nothing else runs: wait here.
                RunKind::OneShot => {
                    let res = crate::ocr::recognize(&img);
                    let note = crate::ocr::finish(&res);
                    println!("{}", note.text);
                    if note.kind == ToastKind::Error {
                        self.exit_code.store(1, Ordering::SeqCst);
                    }
                }
            }
        }
        wind::hide(self.hwnd);
        match self.kind {
            RunKind::Daemon => self.st = State::Hidden,
            // A one-shot run lives on while its pins are up.
            RunKind::OneShot if crate::pin::count() > 0 => {
                self.st = State::Hidden;
                self.wait_pins = true;
            }
            RunKind::OneShot => wind::close(self.hwnd),
        }
    }

    /// Copy text finished: copy what was found and say so (the notice,
    /// or a tray balloon where there is none).
    /// Every job in flight delivers its own result, in the order they end.
    fn poll_ocr(&mut self) {
        let mut done = Vec::new();
        self.ocr.retain(|job| {
            let res = match job.rx.try_recv() {
                Ok(res) => res,
                Err(std::sync::mpsc::TryRecvError::Empty) => return true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(crate::ocr::OcrError::Failed("the recognizer stopped".into())),
            };
            done.push((res, job.at, job.th));
            false
        });
        for (res, at, th) in done {
            if let Err(e) = &res {
                eprintln!("rustshot: text recognition: {e:?}");
            }
            let note = crate::ocr::finish(&res);
            let text = note.text.clone();
            if matches!(self.st, State::Edit(_)) {
                // A newer capture is up: its own toast.
                self.notice = Some(Toast::new(text, note.kind));
            } else if !crate::notice::show(note, at, th) {
                crate::tray::notify(&text);
            }
        }
    }

    /// An upload finished: copy its link and say so. A capture's overlay
    /// is hidden once it is exported, so the daemon says it in the
    /// notice still showing "Uploading…", or in a tray balloon once that
    /// is gone; only over a newer capture is it the overlay's toast.
    fn poll_upload(&mut self) {
        let slot = self.upload_slot.clone();
        let mut guard = slot.lock().unwrap();
        let Some(rx) = guard.take() else { return };
        let res = match rx.try_recv() {
            Ok(res) => res,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                *guard = Some(rx);
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
        };
        let (text, kind, copied) = match &res {
            Ok(url) => {
                println!("uploaded: {url}");
                // Its record: the entry's meta, or uploads.json.
                let id = self.upload_entry.take();
                let entry = self.history_root.as_deref().zip(id.as_deref());
                if !cfg!(test) {
                    crate::uploads::finished(url, entry);
                }
                let copied = self.cfg.copy_url_after_upload
                    && export::copy_text_to_clipboard(url)
                        .map_err(|e| eprintln!("warning: could not copy URL: {e:#}"))
                        .is_ok();
                let head = if copied { "Uploaded \u{2014} link copied" } else { "Uploaded" };
                (format!("{head}\n{url}"), ToastKind::Success, copied)
            }
            Err(e) => {
                eprintln!("error: upload failed: {e}");
                (format!("Upload failed: {e}"), ToastKind::Error, false)
            }
        };
        if matches!(self.st, State::Edit(_)) || self.one_shot() {
            self.notice = Some(Toast::new(text.replace('\n', " "), kind));
        } else if !crate::notice::replace(crate::notice::upload_note(&res, copied)) {
            crate::tray::notify(&text);
        }
    }

    // --- input ------------------------------------------------------------

    fn key(&mut self, vk: u32, repeat: bool, mods: Mods) {
        self.edit_tx(|this, edit| this.handle_key(edit, vk, repeat, mods));
    }

    /// The pointer as a point.
    fn mouse_pt(&self) -> Pt {
        Pt::new(self.mouse.0 as f32, self.mouse.1 as f32)
    }

    fn handle_key(&mut self, edit: &mut Edit, vk: u32, repeat: bool, mods: Mods) {
        let m = self.mouse_pt();
        edit.handle_key(vk, repeat, mods, m);
    }

    fn char_input(&mut self, c: u16) {
        self.edit_tx(|_this, edit| edit.char_input(c));
    }

    fn wheel(&mut self, delta: i32) {
        self.edit_tx(|_this, edit| edit.wheel(delta));
    }

    fn pointer_down(&mut self, x: i32, y: i32) {
        let p = Pt::new(x as f32, y as f32);
        self.edit_tx(|_this, edit| edit.pointer_down(p));
    }

    fn pointer_move(&mut self, x: i32, y: i32) {
        let _p = crate::perf::span(crate::perf::K::Ptr);
        let p = Pt::new(x as f32, y as f32);
        self.edit_tx(|_this, edit| edit.pointer_move(p, p));
    }

    fn pointer_up(&mut self) {
        self.edit_tx(|_this, edit| edit.pointer_up());
    }

    #[cfg(test)]
    fn apply_act(&mut self, edit: &mut Edit, act: Act) {
        let m = self.mouse_pt();
        edit.apply_act(act, m);
    }
}

impl App {
    /// Ask for a repaint of what changed (GDI: the dirty rects; software:
    /// the whole window).
    fn repaint(&mut self) {
        let h = self.hwnd;
        wind::request(h, self);
    }
}

impl Driver for App {
    fn on_create(&mut self, hwnd: Hwnd) {
        self.hwnd = hwnd;
        self.pump();
    }

    fn on_event(&mut self, ev: Ev) -> bool {
        match ev {
            Ev::Move { x, y } => {
                self.mouse = (x, y);
                let dragging =
                    matches!(&self.st, State::Edit(e) if !matches!(e.interact, Interact::None));
                if dragging {
                    self.pointer_move(x, y);
                    self.repaint();
                } else if let State::Edit(e) = &mut self.st {
                    let p = Pt::new(x as f32, y as f32);
                    // Chrome follows the monitor under the pointer when
                    // nothing is selected (hint, toast placement).
                    let area_moved =
                        e.area_drawn.is_some_and(|a| a != pick_area(&e.shot.monitors, e.sel, p));
                    if e.track_hover(p) | e.track_loupe(p) | e.track_snap(p) | e.track_assist(p) || area_moved {
                        self.repaint();
                    }
                }
            }
            Ev::Down { x, y } => {
                self.mouse = (x, y);
                self.pointer_down(x, y);
            }
            Ev::Right { down, x, y } => {
                self.mouse = (x, y);
                let p = Pt::new(x as f32, y as f32);
                self.edit_tx(|_this, edit| if down { edit.right_down(p) } else { edit.right_up(p) });
            }
            Ev::Up { x, y } => {
                self.mouse = (x, y);
                self.pointer_up();
            }
            Ev::Wheel { delta, x, y } => {
                self.mouse = (x, y);
                self.wheel(delta);
            }
            Ev::Key { vk, up, repeat, mods } => {
                // Shift / Ctrl pressed or let go mid-stroke redraw the draft.
                if self.modifier_key(vk, up, mods) {
                    self.repaint();
                }
                if !up {
                    self.key(vk, repeat, mods);
                }
            }
            Ev::Char(c) => self.char_input(c),
            Ev::Timer => return self.tick(),
            // Decorated-window events (`run_window`); the overlay never gets them.
            Ev::Close | Ev::Resize(..) | Ev::Focus(_) => return false,
            // The middle button does nothing in the overlay.
            Ev::Middle { .. } => return false,
        }
        self.pump();
        // Only `Ev::Timer` consults this value (returned from `tick`);
        // moves call `invalidate` themselves, only when something changed.
        true
    }

    fn frame(&mut self) -> Option<&mut PixBuf> {
        let State::Edit(edit) = &mut self.st else { return None };
        let edit: &mut Edit = edit;
        if edit.gdi.is_some() {
            return None; // painted by `paint`
        }
        let now = Instant::now();
        edit.prepare(self.notice.as_ref(), self.mouse, now);
        // Borrowed out of `edit` while composing, put back below;
        // allocated once per capture.
        let mut img = std::mem::take(&mut edit.frame);
        if img.dimensions() != edit.base.dimensions() {
            img = PixBuf::new(edit.base.width(), edit.base.height());
        }
        // The whole image as one rect: same code path as partial repaints.
        let bd = PixBufBackdrop { img: edit.composed(), dim: edit.th.dim };
        edit.compose_rect(compose::PxRect::image(edit.shot.size), &bd, img.as_raw_mut(), Order::Rgba);
        wind::set_fast_timer(self.hwnd, animating(edit, self.notice.as_ref(), now) || edit.rec.meter.is_some());
        edit.frame = img;
        Some(&mut edit.frame)
    }

    fn damage(&mut self) -> Option<Vec<[i32; 4]>> {
        let State::Edit(edit) = &mut self.st else { return None };
        let edit: &mut Edit = edit;
        edit.gdi.as_ref()?;
        let _p = crate::perf::span(crate::perf::K::Damage);
        let now = Instant::now();
        edit.prepare(self.notice.as_ref(), self.mouse, now);
        wind::set_fast_timer(self.hwnd, animating(edit, self.notice.as_ref(), now) || edit.rec.meter.is_some());
        let rects = compose::merge_rects(edit.dirty_rects(edit.painted.as_ref()));
        // The target state: every pixel that differs from it is invalid now.
        edit.painted = edit.scene.clone();
        crate::perf::damage(rects.len(), rects.iter().map(|r| r.w() as u64 * r.h() as u64).sum());
        if rects.len() > MAX_DAMAGE_RECTS {
            return None;
        }
        Some(rects.into_iter().map(|r| [r.x0, r.y0, r.x1, r.y1]).collect())
    }

    #[cfg(windows)]
    fn paint(&mut self, hdc: windows::Win32::Graphics::Gdi::HDC, rects: &[[i32; 4]]) -> bool {
        let State::Edit(edit) = &mut self.st else { return false };
        let edit: &mut Edit = edit;
        if edit.gdi.is_none() {
            return false;
        }
        if edit.scene.is_none() {
            // A system paint before any damage (first show).
            let now = Instant::now();
            edit.prepare(self.notice.as_ref(), self.mouse, now);
            wind::set_fast_timer(self.hwnd, animating(edit, self.notice.as_ref(), now) || edit.rec.meter.is_some());
            edit.painted = edit.scene.clone();
        }
        let rects: Vec<compose::PxRect> =
            rects.iter().map(|r| compose::PxRect::new(r[0], r[1], r[2], r[3])).collect();
        edit.paint_gdi(hdc, &rects);
        true
    }

    fn cursor(&self) -> Cursor {
        let State::Edit(edit) = &self.st else { return Cursor::Arrow };
        if edit.assist.ring.is_some() {
            return Cursor::Arrow;
        }
        if edit.pick.as_ref().is_some_and(|p| p.eyedrop) {
            return Cursor::Cross;
        }
        if edit.text.is_some() {
            return Cursor::IBeam;
        }
        let p = Pt::new(self.mouse.0 as f32, self.mouse.1 as f32);
        if edit.in_cedit(p) {
            return Cursor::IBeam;
        }
        if !edit.obj.sel.is_empty()
            && let Some(h) = edit.obj_handle_at(p)
        {
            return match h {
                select::OHandle::Box(h) => handle_cursor(h),
                select::OHandle::Point(..) => Cursor::Move,
            };
        }
        if edit.tool == Some(Tool::Select) {
            return if edit.obj_at(p).is_some() { Cursor::Move } else { Cursor::Arrow };
        }
        if edit.tool.is_some() {
            return Cursor::Cross;
        }
        if let Some(sr) = edit.sel {
            if let Some((h, _)) = handle_points(sr)
                .into_iter()
                .find(|(_, q)| (q.x - p.x).hypot(q.y - p.y) <= HANDLE_HIT * edit.ui_scale())
            {
                return handle_cursor(h);
            }
            if hit(sr, p) {
                return Cursor::Move;
            }
        }
        Cursor::Cross
    }
}

// ---------------------------------------------------------------------------
// The editor
// ---------------------------------------------------------------------------

/// Capture for the editor with the renderer from `cfg` (resolved once per
/// capture): Windows `gdi` keeps the pixels in GDI bitmaps and leaves
/// `shot.image` empty; `software` (and Linux/macOS) reads them into memory.
fn grab(cfg: &Config, th: &Theme, screen: Option<u32>) -> anyhow::Result<(Shot, Option<gdi::GdiScreen>)> {
    // Tests never capture the screen: they hand `begin_capture` a shot.
    #[cfg(test)]
    {
        let _ = (cfg, th, screen);
        FAKE_GRAB
            .with(|g| g.borrow_mut().take())
            .unwrap_or_else(|| Err(anyhow::anyhow!("no test shot queued")))
            .map(|s| (s, None))
    }
    #[cfg(not(test))]
    grab_screen(cfg, th, screen)
}

#[cfg_attr(test, allow(dead_code))]
fn grab_screen(cfg: &Config, th: &Theme, screen: Option<u32>) -> anyhow::Result<(Shot, Option<gdi::GdiScreen>)> {
    #[cfg(windows)]
    if cfg.use_gdi() {
        let shot = capture::plan_edit(screen, cfg.capture_active_monitor)?;
        let dim = th.dim.with_alpha(th.dim_alpha(cfg.contrast_opacity));
        let scr = gdi::GdiScreen::capture(shot.origin, shot.size, dim)?;
        return Ok((shot, Some(scr)));
    }
    let _ = th;
    Ok((capture::grab_edit(screen, cfg.capture_active_monitor)?, None))
}

#[cfg(test)]
thread_local! {
    /// Tests: what the next `grab` returns (none queued: an error).
    static FAKE_GRAB: std::cell::RefCell<Option<anyhow::Result<Shot>>> = const { std::cell::RefCell::new(None) };
    /// Tests: the windows the next capture snaps to (none queued: none).
    static FAKE_SNAP: std::cell::RefCell<Option<crate::snap::Snapshot>> = const { std::cell::RefCell::new(None) };
    /// Tests: the modifiers a drag sees (the real keyboard is not read).
    static FAKE_MODS: std::cell::Cell<Option<Mods>> =
        const { std::cell::Cell::new(Some(Mods { shift: false, ctrl: false, alt: false })) };
}

/// The windows for window snapping (`snap_windows`), listed now.
fn take_snap(cfg: &Config, shot: &Shot) -> Option<crate::snap::Snapshot> {
    if !cfg.snap_windows {
        return None;
    }
    take_snap_any(shot)
}

/// The windows on screen now, snapping on or off (video mode's Window
/// source picks one like window snapping does).
fn take_snap_any(shot: &Shot) -> Option<crate::snap::Snapshot> {
    // Tests never list the real windows: they queue a snapshot.
    #[cfg(test)]
    {
        let _ = shot;
        FAKE_SNAP.with(|s| s.borrow_mut().take())
    }
    #[cfg(not(test))]
    crate::snap::take(shot.origin, shot.size)
}

/// Modifiers held during a drag (Shift/Ctrl constraints).
fn drag_mods() -> Mods {
    #[cfg(test)]
    if let Some(m) = FAKE_MODS.with(|m| m.get()) {
        return m;
    }
    Mods::current()
}

/// The built-in key `vk` of video mode: 1 / 2 (Screenshot / Video) while
/// recording works here, and F / W / R (the source) in video mode.
fn video_key(edit: &Edit, vk: u32, mods: Mods) -> Option<Act> {
    if mods.ctrl || mods.alt || mods.shift || !rec_ui::supported() || !edit.tasks.is_empty() {
        return None;
    }
    let video = edit.rec.open;
    match char::from_u32(vk)? {
        '1' => Some(Act::Mode(false)),
        '2' => Some(Act::Mode(true)),
        'F' if video => Some(Act::RecSource(rec_ui::RecSource::Screen)),
        'W' if video => Some(Act::RecSource(rec_ui::RecSource::Window)),
        'R' if video => Some(Act::RecSource(rec_ui::RecSource::Region)),
        _ => None,
    }
}

/// Esc: close a device list or the ring's palette, drop the current
/// tool / draft / palette, or cancel the capture (video mode's Close).
fn cancel_key(edit: &mut Edit) {
    if edit.save_menu {
        edit.save_menu = false;
        return;
    }
    if edit.obj.cedit.is_some() {
        edit.finish_callout_edit();
        return;
    }
    if !edit.obj.sel.is_empty() {
        edit.obj.sel.clear();
        return;
    }
    if edit.rec.dev_open.is_some() {
        edit.rec.dev_open = None;
        return;
    }
    if edit.rec.ring_pal {
        edit.rec.ring_pal = false;
        return;
    }
    if matches!(edit.interact, Interact::None)
        && edit.draft.is_none()
        && !edit.palette_open
        && edit.tool.is_none()
    {
        edit.cancelled = true;
        edit.done = true;
    } else {
        edit.interact = Interact::None;
        edit.draft = None;
        edit.stroke_pts.clear();
        edit.palette_open = false;
        // The Edit window never closes on Esc: back to Select.
        edit.tool = edit.host.as_ref().map(|_| Tool::Select);
    }
}

/// Enter: select everything if nothing is selected, then run the given
/// tasks (or save). The Edit window saves with its own Save.
fn accept_key(edit: &mut Edit) {
    if edit.host.is_some() {
        edit.finish_callout_edit();
        return;
    }
    if edit.sel.is_none() {
        edit.sel = Some(FRect { x: 0.0, y: 0.0, w: edit.shot.size.0 as f32, h: edit.shot.size.1 as f32 });
        if edit.accept_on_select {
            edit.done = true;
            edit.cancelled = false;
            return;
        }
    }
    if edit.tasks.is_empty() {
        edit.tasks = vec![Task::Save { path: None, ask: edit.cfg.save_dialog }];
    }
    edit.done = true;
    edit.cancelled = false;
}

/// The toolbar act a keymap action triggers (`None`: handled in
/// `handle_key` itself).
fn act_of(a: Action) -> Option<Act> {
    Some(match a {
        Action::ToolSelect => Act::Tool(Tool::Select),
        Action::ToolPencil => Act::Tool(Tool::Path),
        Action::ToolLine => Act::Tool(Tool::Line),
        Action::ToolArrow => Act::Tool(Tool::Arrow),
        Action::ToolRectangle => Act::Tool(Tool::Rect),
        Action::ToolCircle => Act::Tool(Tool::Ellipse),
        Action::ToolMarker => Act::Tool(Tool::Marker),
        Action::ToolText => Act::Tool(Tool::Text),
        Action::ToolCounter => Act::Tool(Tool::Counter),
        Action::ToolPixelate => Act::Tool(Tool::Pixelate),
        Action::ToolInvert => Act::Tool(Tool::Invert),
        Action::ToolCallout => Act::Tool(Tool::Callout),
        Action::ToolSpotlight => Act::Tool(Tool::Spotlight),
        Action::ToolMagnify => Act::Tool(Tool::Magnify),
        Action::DeleteObject => Act::ObjDelete,
        Action::Duplicate => Act::ObjDup,
        Action::BringFront => Act::ObjFront,
        Action::SendBack => Act::ObjBack,
        Action::SizeDown => Act::Size(-1),
        Action::SizeUp => Act::Size(1),
        Action::Copy => Act::Copy,
        Action::CopyText => Act::CopyText,
        Action::Save => Act::Save,
        Action::SaveAs => Act::SaveAs,
        Action::SaveDesktop => Act::SaveDesktop,
        Action::OpenWith => Act::OpenWith,
        Action::Upload => Act::Upload,
        Action::Pin => Act::Pin,
        Action::Undo => Act::Undo,
        Action::Redo => Act::Redo,
        Action::TogglePalette => Act::Palette,
        Action::Record => Act::Record,
        Action::SelectAll | Action::Accept | Action::Cancel | Action::CopyColor => return None,
        Action::UiBigger | Action::UiSmaller | Action::UiReset => return None,
        Action::RecPause | Action::RecMute | Action::RecHidePill => return None,
        Action::ZoomIn | Action::ZoomOut | Action::ZoomFit | Action::ZoomActual => return None,
    })
}

/// The keymap action whose chord a toolbar button's tooltip shows.
fn action_of(a: Act) -> Option<Action> {
    match a {
        Act::Exit => Some(Action::Cancel),
        Act::Accept => Some(Action::Accept),
        _ => Action::ALL.into_iter().find(|x| act_of(*x) == Some(a)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    /// Pick, move and resize objects (V).
    Select,
    Path,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Marker,
    Text,
    /// Numbered tags.
    Counter,
    Pixelate,
    Invert,
    /// Speech bubbles with wrapped text (K).
    Callout,
    /// Dims everything else in the selection (S).
    Spotlight,
    /// A 2x loupe with a connector to its source (Z).
    Magnify,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

enum Interact {
    None,
    NewSel {
        anchor: Pt,
        moved: bool,
    },
    MoveSel {
        start: Pt,
        orig: FRect,
    },
    Resize {
        handle: Handle,
        orig: FRect,
        aspect: f32,
    },
    Drawing {
        start: Pt,
    },
    /// Dragging in the color picker (the hue ring, the square, opacity).
    Pick(pick::Part),
    /// Moving the selected objects (`orig`: as they were, in selection
    /// order; `bx`: their box); `hit` was pressed, `was_sel` already
    /// selected (a click on it then picks it alone, or edits a callout).
    /// `idx`: the objects moved (`orig` in the same order), `n`: how
    /// many objects there were (a change ends the drag).
    ObjMove {
        start: Pt,
        orig: Vec<Obj>,
        bx: FRect,
        moved: bool,
        hit: usize,
        was_sel: bool,
        idx: Vec<usize>,
        n: usize,
    },
    /// Resizing the selected objects by a handle on their frame `fr`
    /// (`bx`: their box inside it).
    ObjResize {
        handle: Handle,
        orig: Vec<Obj>,
        bx: FRect,
        fr: FRect,
        idx: Vec<usize>,
        n: usize,
    },
    /// Dragging a round handle (an end, a tail, the loupe or its source).
    ObjPoint {
        idx: usize,
        which: crate::objects::edit::Point,
        orig: Obj,
        n: usize,
    },
    /// A marquee from `anchor` (`base`: kept selected, Shift).
    Marquee {
        anchor: Pt,
        cur: Pt,
        base: Vec<usize>,
    },
}

/// The Record popover in the overlay.
struct RecPop {
    opts: RecOpts,
    open: bool,
    /// Record mode (the record hotkey): Enter records.
    mode: bool,
    /// Microphones as (id, name), listed once per capture when needed.
    devs: Option<Vec<(String, String)>>,
    /// Cameras, listed once per capture (on a worker: [`RecPop::load_cams`])
    /// when the popover opens or record mode begins.
    cams: Option<Vec<crate::cam::CamDevice>>,
    /// The camera list on its way.
    cams_rx: Option<std::sync::mpsc::Receiver<Vec<crate::cam::CamDevice>>>,
    /// The open device list.
    dev_open: Option<Dev>,
    /// The camera ring's palette is open.
    ring_pal: bool,
    /// "Start recording" pressed: `finish` records instead of exporting.
    start: bool,
    /// The camera bubble while the popover shows the camera.
    cam: CamSlot,
    /// MP4 can be recorded (Linux: ffmpeg found and able, asked in the
    /// background); else the popover records GIF and MP4 is disabled,
    /// saying "checking…" until ffmpeg answers.
    mp4: rec_ui::Mp4,
    /// The format chosen (the config's, or a click): taken again once MP4
    /// turns out to be there.
    wanted: crate::rec::RecFormat,
    /// What is recorded: the screen, a window or the region.
    source: rec_ui::RecSource,
    /// The microphone metered while the panel shows it on (released
    /// otherwise), the bars it lit when last looked at, and when.
    meter: Option<rec_ui::meter::MicMeter>,
    bars: (u8, bool),
    meter_at: Instant,
    /// The camera bubble's shape (image px): the grip hint goes under it.
    grip: Option<FRect>,
}

/// How long Start waits for the camera list (it is usually there long
/// before).
const CAM_LIST_WAIT: Duration = Duration::from_secs(2);
/// How long a recording started with no countdown waits for the camera's
/// first picture.
const FIRST_PICTURE_WAIT: Duration = Duration::from_secs(1);

impl RecPop {
    fn new(mut opts: RecOpts, mode: bool) -> RecPop {
        let mp4 = rec_ui::mp4_status();
        let wanted = opts.format;
        if mp4 != rec_ui::Mp4::Yes {
            opts.format = crate::rec::RecFormat::Gif;
        }
        let mut p = RecPop {
            mp4,
            wanted,
            opts,
            open: mode,
            mode,
            devs: None,
            cams: None,
            cams_rx: None,
            dev_open: None,
            ring_pal: false,
            start: false,
            cam: CamSlot::default(),
            source: rec_ui::RecSource::Region,
            meter: None,
            bars: (0, false),
            meter_at: Instant::now(),
            grip: None,
        };
        if mode {
            p.load_cams();
        }
        p
    }

    fn load_devs(&mut self) {
        if self.devs.is_none() {
            self.devs = Some(rec_ui::mics());
        }
    }

    /// Start listing the cameras (once per capture), off the UI thread.
    fn load_cams(&mut self) {
        if self.cams.is_none() && self.cams_rx.is_none() {
            self.cams_rx = Some(rec_ui::cameras_async());
            self.poll_cams();
        }
    }

    /// While ffmpeg is being asked: its answer, if it came; true when the
    /// popover changed (MP4 known; the chosen format is taken again).
    fn poll_mp4(&mut self) -> bool {
        if self.mp4 != rec_ui::Mp4::Checking {
            return false;
        }
        self.mp4 = rec_ui::mp4_status();
        match self.mp4 {
            rec_ui::Mp4::Checking => false,
            rec_ui::Mp4::Yes => {
                self.opts.format = self.wanted;
                true
            }
            rec_ui::Mp4::No => true,
        }
    }

    /// Take the camera list if it has come; true when it just did.
    fn poll_cams(&mut self) -> bool {
        let Some(rx) = &self.cams_rx else { return false };
        let list = match rx.try_recv() {
            Ok(l) => l,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        self.cams = Some(list);
        self.cams_rx = None;
        true
    }

    /// Wait up to `t` for the camera list (Start needs it now).
    fn wait_cams(&mut self, t: Duration) {
        self.load_cams();
        if let Some(rx) = self.cams_rx.take() {
            self.cams = Some(rx.recv_timeout(t).unwrap_or_default());
        }
    }

    /// The cameras are still being listed.
    fn cams_looking(&self) -> bool {
        self.cams_rx.is_some()
    }

    /// There is a camera (listed when the popover opened).
    fn has_cams(&self) -> bool {
        self.cams.as_ref().is_some_and(|c| !c.is_empty())
    }

    /// The open list's names: the default first, then every device.
    fn dev_names(&self, dev: Dev) -> Vec<String> {
        match dev {
            Dev::Mic => {
                let mut v = vec![DEFAULT_MIC.to_string()];
                v.extend(self.devs.iter().flatten().map(|d| d.1.clone()));
                v
            }
            Dev::Cam => {
                let mut v = vec![DEFAULT_CAM.to_string()];
                v.extend(self.cams.iter().flatten().map(|d| d.name.clone()));
                v
            }
        }
    }

    /// Index of the chosen device in [`RecPop::dev_names`].
    fn dev_sel(&self, dev: Dev) -> usize {
        match dev {
            Dev::Mic if !self.opts.mic_dev.is_empty() => {
                self.devs.iter().flatten().position(|d| d.0 == self.opts.mic_dev).map_or(0, |i| i + 1)
            }
            Dev::Cam if !self.opts.cam_dev.is_empty() => {
                self.cams.iter().flatten().position(|d| d.id == self.opts.cam_dev).map_or(0, |i| i + 1)
            }
            _ => 0,
        }
    }

    /// Pick entry `i` of `dev`'s list (0 = the default).
    fn pick(&mut self, dev: Dev, i: usize) {
        match dev {
            Dev::Mic => {
                self.opts.mic_dev = if i == 0 {
                    String::new()
                } else {
                    self.devs.as_ref().and_then(|d| d.get(i - 1)).map(|d| d.0.clone()).unwrap_or_default()
                };
            }
            Dev::Cam => {
                self.opts.cam_dev = if i == 0 {
                    String::new()
                } else {
                    self.cams.as_ref().and_then(|d| d.get(i - 1)).map(|d| d.id.clone()).unwrap_or_default()
                };
            }
        }
        self.dev_open = None;
    }

    fn mic_label(&self) -> String {
        if self.opts.mic_dev.is_empty() {
            return DEFAULT_MIC.into();
        }
        let named = self.devs.iter().flatten().find(|d| d.0 == self.opts.mic_dev).map(|d| d.1.clone());
        named.unwrap_or_else(|| "Microphone".into())
    }

    fn cam_label(&self) -> String {
        if self.opts.cam_dev.is_empty() {
            return DEFAULT_CAM.into();
        }
        let named = self.cams.iter().flatten().find(|d| d.id == self.opts.cam_dev).map(|d| d.name.clone());
        named.unwrap_or_else(|| "Camera".into())
    }
}

/// What the camera bubble should show, and where.
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
struct CamWant {
    dev: String,
    look: crate::cam::Look,
    /// The area (physical px) and its UI scale.
    area: capture::IRect,
    scale: f32,
    th: Theme,
    /// Rects to keep off (the toolbar and popover, physical px) and the
    /// monitor it may go to around a small area.
    avoid: Vec<capture::IRect>,
    screen: capture::IRect,
}

/// Tests: opens a fake camera for a device id.
#[cfg(all(windows, test))]
type FakeCam = Box<dyn Fn(&str) -> crate::cam::Opener>;

/// The camera bubble of the Record popover: shown while the popover is
/// open with the camera on (and a selection), following the selection;
/// handed to the recording on Start, closed otherwise (dropped with the
/// overlay on every other way out).
#[derive(Default)]
struct CamSlot {
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    bubble: Option<crate::cam::Bubble>,
    /// Tests: the camera the bubble opens (no bubble without one).
    #[cfg(all(windows, test))]
    fake: Option<FakeCam>,
    /// Tests: how many times a camera was opened.
    #[cfg(all(windows, test))]
    opened: usize,
    /// What is shown: camera, look, area (physical px), UI scale.
    shown: Option<(String, crate::cam::Look, capture::IRect, f32)>,
    /// The system's monitors (their UI scales), read once per capture.
    #[cfg_attr(test, allow(dead_code))]
    mons: Option<Vec<capture::MonInfo>>,
}

impl CamSlot {
    fn close(&mut self) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        {
            self.bubble = None;
        }
        self.shown = None;
    }

    /// A bubble can be made (tests: only with a fake camera; none on
    /// Linux, whose test runs have no display).
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn can_open(&self) -> bool {
        #[cfg(all(windows, test))]
        {
            self.fake.is_some()
        }
        #[cfg(all(any(target_os = "linux", target_os = "macos"), test))]
        {
            false
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    /// Open a bubble for `w`.
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn open(&mut self, w: &CamWant, nudge: Hwnd) -> anyhow::Result<crate::cam::Bubble> {
        #[cfg(all(windows, test))]
        {
            let open = self.fake.as_ref().map(|f| f(&w.dev)).ok_or_else(|| anyhow::anyhow!("no fake camera"))?;
            self.opened += 1;
            crate::cam::Bubble::show_with(w.look, &w.dev, w.area, w.scale, &w.th, nudge, open)
        }
        #[cfg(not(all(windows, test)))]
        {
            crate::cam::Bubble::show(w.look, &w.dev, w.area, w.scale, &w.th, nudge)
        }
    }

    /// Show `w`; the camera's error when it could not be opened or
    /// stopped (the bubble is closed then).
    #[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(unused_variables))]
    fn sync(&mut self, w: &CamWant, nudge: Hwnd) -> Option<String> {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if self.can_open() {
            if self.bubble.as_ref().is_some_and(|b| b.device() != w.dev) {
                self.bubble = None; // another camera: reopen
            }
            if let Some(err) = self.bubble.as_ref().and_then(|b| b.error()) {
                self.close();
                return Some(err);
            }
            if self.bubble.is_none() {
                match self.open(w, nudge) {
                    Ok(b) => self.bubble = Some(b),
                    Err(e) => {
                        self.close();
                        return Some(format!("{e:#}"));
                    }
                }
            }
            if let Some(b) = &mut self.bubble {
                b.set_look(w.look, &w.th);
                b.set_anchor(w.area, w.scale);
                b.set_avoid(&w.avoid, w.screen);
            }
        }
        self.shown = Some((w.dev.clone(), w.look, w.area, w.scale));
        None
    }

    /// A bubble for a recording started without the popover's: opened
    /// now; with `wait`, shown once its first picture is in (or `wait`
    /// is over). None when the camera cannot be opened (said on stderr).
    #[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(unused_variables))]
    fn open_now(&mut self, w: &CamWant, nudge: Hwnd, wait: Option<Duration>) -> Option<crate::cam::Bubble> {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if self.can_open() {
            match self.open(w, nudge) {
                Ok(b) => {
                    if let Some(t) = wait
                        && !b.wait_picture(t)
                    {
                        eprintln!("rustshot: camera: no picture within {} ms; recording anyway", t.as_millis());
                    }
                    return Some(b);
                }
                Err(e) => eprintln!("warning: camera not shown: {e:#}"),
            }
        }
        None
    }

    /// The bubble's shape on screen (physical top-left and side) while it
    /// is shown and not dragged.
    fn shape_rect(&self) -> Option<(i32, i32, u32)> {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        {
            let b = self.bubble.as_ref()?;
            if b.dragging() || b.error().is_some() {
                return None;
            }
            let (x, y) = b.pos()?;
            let scale = self.shown.as_ref().map_or(1.0, |s| s.3);
            Some((x, y, crate::cam::side(b.look()?.size, scale)))
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }

    /// The bubble, for the recording (closed here); it keeps off nothing
    /// now (the toolbar is gone).
    fn take(&mut self) -> Option<crate::cam::Bubble> {
        self.shown = None;
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        {
            let mut b = self.bubble.take();
            if let Some(b) = &mut b {
                b.clear_avoid();
            }
            b
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }
}

const DEFAULT_MIC: &str = "Default microphone";
const DEFAULT_CAM: &str = "Default camera";

struct TextDraft {
    pos: Pt,
    text: String,
    /// Byte index of the caret inside `text`.
    caret: usize,
    /// Last edit, for the caret blink phase.
    at: Instant,
}

struct Edit {
    mo: Motion,
    shot: Shot,
    /// The capture (moved out of `shot.image`, which stays empty).
    base: PixBuf,
    /// `base` with the committed objects baked in; `None` while there are
    /// none (identical to `base`, so no second full-size buffer). Read it
    /// through `composed()`.
    composed: Option<PixBuf>,
    /// Display buffer `frame()` composes into; reused across frames,
    /// allocated on the first frame of a capture.
    frame: PixBuf,
    /// The last prepared frame state (`prepare`), drawn by `compose_rect`.
    scene: Option<compose::Scene>,
    /// GDI renderer: the scene the window shows once the pending
    /// invalidations are painted (what `dirty_rects` diffs against).
    painted: Option<compose::Scene>,
    /// GDI renderer (Windows, `renderer = "gdi"`): the capture as GDI
    /// bitmaps; `base` then stays empty and `composed` unused.
    gdi: Option<gdi::GdiScreen>,
    /// Scratch for a pixelate draft rendered over whole cells
    /// (`compose_rect`); about one band, released when the draft ends.
    draft_buf: std::cell::RefCell<Vec<u8>>,
    /// Caret blink phase in the last frame (None: no text draft).
    caret_drawn: Option<bool>,
    /// `at` of the toast drawn in the last frame (None: no toast).
    toast_drawn: Option<Instant>,
    /// The last frame was drawn mid-animation: one more frame, at the
    /// animation's end values, is owed (else the fade-in's last frame, a
    /// pixel or two short of its place, could stay on screen).
    anim_drawn: bool,
    /// Monitor area chrome was placed in by the last frame.
    area_drawn: Option<FRect>,
    objects: Vec<Obj>,
    hist: Vec<Vec<Obj>>,
    hi: usize,
    sel: Option<FRect>,
    tool: Option<Tool>,
    draft: Option<Obj>,
    stroke_pts: Vec<Pt>,
    interact: Interact,
    /// Every tool's size and color (remembered across captures).
    mem: tools::ToolMem,
    /// The tool whose size and color the toolbar shows while none is
    /// chosen: the last one chosen.
    last_tool: Tool,
    /// Remembered settings waiting to be written to the config.
    pending: tools::Pending,
    /// The config file they go to (`None`: not written; tests).
    cfg_path: Option<std::path::PathBuf>,
    /// The color picker while it is open.
    pick: Option<pick::Picker>,
    /// The pointer while the eyedropper is on (the loupe follows it).
    loupe: Option<Pt>,
    text: Option<TextDraft>,
    tasks: Vec<Task>,
    accept_on_select: bool,
    cfg: Config,
    /// Editor shortcuts (`cfg.shortcuts` over the defaults).
    keys: Keymap,
    toolbar: Option<Toolbar>,
    palette_open: bool,
    /// The Save split button's menu is open.
    save_menu: bool,
    rec: RecPop,
    done: bool,
    cancelled: bool,
    dirty: bool,
    notice: Option<Toast>,
    last_wheel: Instant,
    font: Option<AnnotFont>,
    th: Theme,
    ui_font: Option<&'static UiFont>,
    /// Toolbar item under the pointer and since when (tooltip delay).
    hover: Option<usize>,
    hover_at: Instant,
    pressed: Option<usize>,
    /// Selection handle under the pointer (ring grows to 3 px).
    hot_handle: Option<usize>,
    /// Window snapping: the windows when the capture started (`None`:
    /// off, or windows cannot be listed here).
    snap: Option<crate::snap::Snapshot>,
    /// The window (or part, after Tab) under the pointer.
    hover_win: crate::snap::Hover,
    /// The object selection and the callout being typed in.
    obj: select::ObjState,
    /// The objects `composed` holds (software renderer): rebuilt in place
    /// where they changed.
    baked: Vec<Obj>,
    /// The spotlight area `composed` was baked with.
    baked_area: crate::objects::Px,
    /// The window in front when the capture started: (title, app).
    source: (String, String),
    /// Hosted in the Edit window ([`host::Canvas`]); `None`: the overlay.
    host: Option<host::Host>,
    /// The magnifier, color ring, accept on select and hint card.
    assist: assist::Assist,
}

/// More merged dirty rects than this invalidate the whole window (a
/// selection drag with the magnifier and its crosshair takes up to 16).
const MAX_DAMAGE_RECTS: usize = 24;

const HANDLE_HIT: f32 = 10.0; // handle hit radius, logical px (20 px target)
const CLICK_PX: f32 = 2.5; // movement below this counts as a click
/// While snapping to windows, a press moving less than this is a click
/// (selects the window); from it on, a drag draws a rectangle.
const SNAP_CLICK_PX: f32 = 4.0;

impl Edit {
    /// A fresh editor over `base` (the capture; `shot` its geometry, its
    /// image already moved out): nothing selected, drawn or open.
    #[allow(clippy::too_many_arguments)]
    fn new(shot: Shot, base: PixBuf, cfg: Config, th: Theme, font: Option<AnnotFont>, ui_font: Option<&'static UiFont>, rec: RecPop) -> Edit {
        let style = crate::objects::CalloutStyle::from_id(&cfg.callout_style).unwrap_or_default();
        let assist = assist::Assist::new(&cfg, Instant::now());
        Edit {
            mo: Motion::new(Instant::now()),
            shot,
            base,
            composed: None,
            frame: PixBuf::default(),
            scene: None,
            painted: None,
            gdi: None,
            draft_buf: Default::default(),
            caret_drawn: None,
            toast_drawn: None,
            anim_drawn: false,
            area_drawn: None,
            objects: Vec::new(),
            hist: vec![Vec::new()],
            hi: 0,
            sel: None,
            tool: None,
            draft: None,
            stroke_pts: Vec::new(),
            interact: Interact::None,
            mem: tools::ToolMem::from_cfg(&cfg),
            last_tool: Tool::Path,
            pending: tools::Pending::default(),
            cfg_path: None,
            pick: None,
            loupe: None,
            text: None,
            tasks: Vec::new(),
            accept_on_select: false,
            keys: Keymap::from_config(&cfg.shortcuts).0,
            cfg,
            toolbar: None,
            palette_open: false,
            save_menu: false,
            rec,
            done: false,
            cancelled: false,
            dirty: false,
            notice: None,
            last_wheel: Instant::now(),
            font,
            th,
            ui_font,
            hover: None,
            hover_at: Instant::now(),
            pressed: None,
            hot_handle: None,
            snap: None,
            hover_win: Default::default(),
            obj: select::ObjState { style, ..Default::default() },
            baked: Vec::new(),
            baked_area: (0, 0, 0, 0),
            source: Default::default(),
            host: None,
            assist,
        }
    }

    /// The selection crop (`sel` in image coords) to export, objects
    /// baked in.
    fn export(&self, sel: FRect) -> PixBuf {
        #[cfg(windows)]
        if self.gdi.is_some() {
            return self.export_gdi(sel, false);
        }
        crop_to_image(self.composed(), sel)
    }

    /// [`Edit::export`] without the objects: the capture as it was (the
    /// history's `original.png`).
    fn export_plain(&self, sel: FRect) -> PixBuf {
        #[cfg(windows)]
        if self.gdi.is_some() {
            return self.export_gdi(sel, true);
        }
        crop_to_image(&self.base, sel)
    }

    /// What the history keeps of export `sel`: the plain crop and the
    /// objects moved into its coordinates (the crop's top-left is the
    /// selection's, floored, as `crop_to_image` takes it).
    fn history_shot(&self, sel: FRect) -> (PixBuf, Vec<Obj>) {
        let (x0, y0) = (sel.x.floor().max(0.0), sel.y.floor().max(0.0));
        let objs = self
            .objects
            .iter()
            .cloned()
            .map(|mut o| {
                crate::objects::edit::translate(&mut o, -x0, -y0);
                o
            })
            .collect();
        (self.export_plain(sel), objs)
    }

    /// The area the spotlights dim: the selection, on whole pixels.
    fn spot_area(&self) -> crate::objects::Px {
        compose::spot_area(self.sel, self.shot.size)
    }

    /// The spotlights' area moved since the bake (the selection changed
    /// under them): the bake is behind.
    fn spots_stale(&self) -> bool {
        self.baked_area != self.spot_area() && self.objects.iter().any(Obj::is_spot)
    }

    /// The capture with committed objects baked in.
    fn composed(&self) -> &PixBuf {
        self.composed.as_ref().unwrap_or(&self.base)
    }

    /// Bake the objects into `composed` where they changed; returns the
    /// rects re-baked (`None`: maybe everywhere).
    fn rebuild(&mut self) -> Option<Vec<compose::PxRect>> {
        let area = self.spot_area();
        #[cfg(windows)]
        if let Some(scr) = self.gdi.as_mut() {
            // The annotation layer: objects baked over the plain capture,
            // only where they are.
            scr.set_objects(&self.objects, area, self.font.as_ref());
            self.baked_area = area;
            self.dirty = false;
            return None;
        }
        let mut out = None;
        if self.objects.is_empty() {
            // Nothing to bake (e.g. undid the last object): drop the copy.
            self.composed = None;
            self.baked.clear();
        } else {
            let size = self.shot.size;
            let font = self.font.as_ref();
            let damage = self
                .composed
                .as_ref()
                .and_then(|_| compose::object_damage(&self.baked, &self.objects, self.baked_area, area, font, size));
            let base = &self.base;
            // The first spotlight, on top: its dim over what is there.
            let first_spot = self.objects.len() == self.baked.len() + 1
                && self.objects.starts_with(&self.baked)
                && self.objects.last().is_some_and(Obj::is_spot)
                && !self.baked.iter().any(Obj::is_spot);
            match (self.composed.as_mut(), damage) {
                (Some(composed), Some(_)) if first_spot => {
                    // The pass dims the whole area.
                    out = None;
                    if let Some(p) = crate::objects::SpotPass::of(&self.objects, area) {
                        p.apply(&mut crate::raster::Surf::from_buf(composed));
                    }
                }
                // Re-bake only where the objects (or the spotlights' area) changed.
                (Some(composed), Some(rects)) => {
                    compose::rebake(composed, base, &self.objects, area, font, &rects);
                    out = Some(rects);
                }
                _ => {
                    // Re-bake in place; allocate only when it was `None`.
                    let composed = self.composed.get_or_insert_with(|| base.clone());
                    composed.as_raw_mut().copy_from_slice(base.as_raw());
                    let img = compose::PxRect::image(size);
                    let iw = base.width() as usize;
                    let bg = |r: compose::PxRect, out: &mut [u8]| {
                        let n = r.w() as usize * 4;
                        for (i, y) in (r.y0..r.y1).enumerate() {
                            out[i * n..][..n].copy_from_slice(&base.as_raw()[(y as usize * iw + r.x0 as usize) * 4..][..n]);
                        }
                    };
                    let bake = compose::Bake { objs: &self.objects, font, pass: crate::objects::SpotPass::of(&self.objects, area), img, bg: &bg };
                    bake.window(composed.as_raw_mut(), img, Order::Rgba);
                }
            }
            self.baked.clone_from(&self.objects);
        }
        self.baked_area = area;
        self.dirty = false;
        out
    }

    fn push_history(&mut self) {
        self.hist.truncate(self.hi + 1);
        self.hist.push(self.objects.clone());
        self.hi = self.hist.len() - 1;
        let limit = self.cfg.undo_limit.max(1);
        if self.hist.len() > limit {
            let excess = self.hist.len() - limit;
            self.hist.drain(0..excess);
            self.hi -= excess;
        }
    }

    fn undo(&mut self) {
        if self.hi > 0 {
            self.hi -= 1;
            self.objects = self.hist[self.hi].clone();
            self.dirty = true;
        }
    }

    fn redo(&mut self) {
        if self.hi + 1 < self.hist.len() {
            self.hi += 1;
            self.objects = self.hist[self.hi].clone();
            self.dirty = true;
        }
    }

    fn commit_object(&mut self, obj: Obj) {
        self.objects.push(obj);
        self.push_history();
        self.dirty = true;
    }

    /// The tool whose size and color apply: the chosen one, else the
    /// last one chosen.
    fn cur_tool(&self) -> Tool {
        self.tool.unwrap_or(self.last_tool)
    }

    /// The current tool's color.
    fn color(&self) -> C4 {
        self.mem.color(self.cur_tool())
    }

    /// Step the current tool's size (a toast says the new one); it is
    /// remembered.
    fn adjust_size(&mut self, dir: i32) {
        let t = self.cur_tool();
        if !tools::has_size(t) {
            return;
        }
        let v = self.mem.step(t, dir).unwrap_or(self.mem.size(t));
        self.notice = Some(Toast::new(format!("Size {}", v as i32), ToastKind::Info));
        self.remember_size(t, Instant::now());
    }

    /// Queue `t`'s size for the config.
    fn remember_size(&mut self, t: Tool, now: Instant) {
        let Some(id) = tools::id(t) else { return };
        let v = self.mem.size(t);
        self.cfg.tool_sizes.insert(id.into(), v);
        self.pending.set(format!("tool_{id}_size"), Some(config::toml_num(v)), now);
    }

    /// Set the current tool's color (remembered).
    fn set_tool_color(&mut self, c: C4, now: Instant) {
        let t = self.cur_tool();
        if !tools::has_color(t) {
            return;
        }
        self.mem.set_color(t, c);
        self.remember_color(t, now);
    }

    /// Queue `t`'s color for the config.
    fn remember_color(&mut self, t: Tool, now: Instant) {
        let Some(id) = tools::id(t).filter(|_| tools::has_color(t)) else { return };
        let v = tools::hex(self.mem.color(t));
        self.pending.set(format!("tool_{id}_color"), Some(config::toml_str(&v)), now);
        self.cfg.tool_colors.insert(id.into(), v);
    }

    fn make_draft(&self, tool: Tool, a: Pt, b: Pt) -> Option<Obj> {
        let c = self.mem.color(tool);
        let w = self.mem.size(tool);
        Some(match tool {
            Tool::Path => Obj::Path {
                pts: vec![a, b],
                color: c,
                width: w,
            },
            Tool::Line => Obj::Line { a, b, color: c, width: w },
            Tool::Arrow => Obj::Arrow { a, b, color: c, width: w },
            Tool::Rect if self.mem.fill(tool) => Obj::RectFill { r: FRect::from_pts(a, b), color: c, width: w },
            Tool::Rect => Obj::Rect {
                r: FRect::from_pts(a, b),
                color: c,
                width: w,
            },
            Tool::Ellipse if self.mem.fill(tool) => Obj::EllipseFill { r: FRect::from_pts(a, b), color: c, width: w },
            Tool::Ellipse => Obj::Ellipse {
                r: FRect::from_pts(a, b),
                color: c,
                width: w,
            },
            Tool::Counter => Obj::Counter { c: b, n: self.next_counter(), color: c, d: w, tail: None },
            Tool::Marker => Obj::Marker {
                a,
                b,
                color: marker_color(c),
                width: w,
            },
            Tool::Pixelate => Obj::Pixelate {
                r: FRect::from_pts(a, b),
                cell: w,
            },
            Tool::Invert => Obj::Invert {
                r: FRect::from_pts(a, b),
            },
            Tool::Spotlight => Obj::Spotlight { r: FRect::from_pts(a, b) },
            Tool::Callout => {
                let fill = self.mem.fill(tool);
                let (style, text) = (self.obj.style, String::new());
                let mut r = FRect::from_pts(a, b);
                if r.w < crate::objects::callout::MIN_W && r.h < crate::objects::callout::MIN_W {
                    // A click: the default bubble at the press point.
                    r = FRect { x: a.x, y: a.y, w: crate::objects::callout::DEFAULT_W, h: 0.0 };
                }
                r.h = r.h.max(crate::objects::callout::needed_h(self.font.as_ref(), r, &text, w, style));
                let tail = Pt::new(r.x + r.w * 0.25, r.y1() + (r.h * 0.5).clamp(24.0, 60.0));
                Obj::Callout { r, tail, text, color: c, size: w, style, fill }
            }
            Tool::Magnify => {
                // A click: the loupe up and to the right of its source.
                let far = (b.x - a.x).hypot(b.y - a.y) >= w * 0.5;
                let dst = if far { b } else { Pt::new(a.x + w * 1.8, a.y - w * 1.8) };
                Obj::Magnify { src: a, dst, radius: w, zoom: 2.0, color: c, width: 3.0 }
            }
            Tool::Text | Tool::Select => None?,
        })
    }

    fn draft_is_valid(&self) -> bool {
        match &self.draft {
            None => false,
            Some(Obj::Line { a, b, .. })
            | Some(Obj::Arrow { a, b, .. })
            | Some(Obj::Marker { a, b, .. }) => (a.x - b.x).hypot(a.y - b.y) >= 2.0,
            Some(Obj::Rect { r, .. })
            | Some(Obj::Ellipse { r, .. })
            | Some(Obj::RectFill { r, .. })
            | Some(Obj::EllipseFill { r, .. })
            | Some(Obj::Pixelate { r, .. })
            | Some(Obj::Spotlight { r })
            | Some(Obj::Invert { r }) => !r.is_trivial(),
            Some(Obj::Callout { .. }) | Some(Obj::Magnify { .. }) => true,
            Some(Obj::Path { pts, .. }) => pts.len() >= 2,
            // A highlighter stroke reaches 2 px from where it began.
            Some(Obj::MarkerPath { pts, .. }) => pts.iter().any(|p| (p.x - pts[0].x).hypot(p.y - pts[0].y) >= 2.0),
            Some(Obj::Text { .. }) | Some(Obj::Counter { .. }) => true,
        }
    }

    fn snap_point(&self, start: Pt, cur: Pt, shift_axis: bool) -> Pt {
        if !shift_axis {
            return cur;
        }
        let dx = cur.x - start.x;
        let dy = cur.y - start.y;
        let len = dx.hypot(dy);
        if len < 1.0 {
            return cur;
        }
        let deg = dy.atan2(dx).to_degrees();
        let snapped = ((deg / 45.0).round() * 45.0).to_radians();
        Pt::new(start.x + len * snapped.cos(), start.y + len * snapped.sin())
    }

    /// Toolbar readout: the current tool's size ("" for Invert, which
    /// has none).
    fn size_label(&self) -> String {
        let t = self.cur_tool();
        if tools::has_size(t) { (self.mem.size(t) as i32).to_string() } else { String::new() }
    }

    /// The overlay's UI scale: the shot's OS scale times the UI size. In
    /// the Edit window: the window's scale over the zoom (chrome sizes in
    /// image px that come out at the window's size on screen).
    fn ui_scale(&self) -> f32 {
        if let Some(h) = &self.host {
            return h.s;
        }
        self.cfg.ui_scale.apply(self.shot.scale)
    }

    /// Ctrl+= / Ctrl+- / Ctrl+0: the UI size steps (or goes back to
    /// Auto) live: the next frame lays out again at the new scale and,
    /// the scale being part of the scene, repaints in full. Saved like the
    /// tools' sizes (debounced, flushed on close); a toast says the size.
    fn set_ui_size(&mut self, size: UiSize, now: Instant) {
        if size != self.cfg.ui_scale {
            self.cfg.ui_scale = size;
            self.pending.set("ui_scale".into(), Some(size.to_toml()), now);
            publish_ui_size(size);
            // Said once per change, and never over an error still showing.
            if self.notice.as_ref().is_none_or(|t| t.kind != ToastKind::Error) {
                self.notice = Some(Toast::new(size.describe(), ToastKind::Info));
            }
        }
    }

    /// The selection as a recording area (physical virtual-screen px,
    /// at least 2x2) with every monitor and its UI scale (from `mons`, the
    /// system's monitors; the shot's scale where none matches).
    fn rec_geom(&self, mons: &[capture::MonInfo]) -> rec_ui::Geom {
        let (ox, oy) = self.shot.origin;
        let full = FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 };
        let mut s = self.sel.unwrap_or(full);
        s.clamp_to(full.w, full.h);
        let (x, y) = (s.x.round() as i32, s.y.round() as i32);
        let (w, h) = ((s.x1().round() as i32 - x).max(2), (s.y1().round() as i32 - y).max(2));
        rec_ui::Geom {
            area: (x + ox, y + oy, w as u32, h as u32),
            monitors: (self.shot.monitors.iter())
                .map(|m| {
                    let r = (m.0 + ox, m.1 + oy, m.2, m.3);
                    let (cx, cy) = (r.0 + r.2 as i32 / 2, r.1 + r.3 as i32 / 2);
                    let s = mons.iter().find(|i| i.contains(cx, cy)).map_or(self.ui_scale(), |i| self.cfg.ui_scale.apply(i.scale));
                    (r, s)
                })
                .collect(),
            scale: self.ui_scale(),
            mult: self.cfg.ui_scale.mult(),
        }
    }

    /// What the camera bubble shows for a recording of `geom`'s area:
    /// the popover's camera and look, kept on the area's monitor.
    fn cam_want(&self, geom: &rec_ui::Geom) -> CamWant {
        let a = geom.area;
        let (cx, cy) = (a.0 + a.2 as i32 / 2, a.1 + a.3 as i32 / 2);
        let screen = geom.monitors.iter().map(|m| m.0).find(|m| rec_ui::contains(*m, cx, cy)).unwrap_or(a);
        CamWant {
            dev: self.rec.opts.cam_dev.clone(),
            look: self.rec.opts.cam_look(),
            area: a,
            scale: geom.area_scale(),
            th: self.th,
            avoid: Vec::new(),
            screen,
        }
    }

    /// The toolbar and its popover on screen (physical px), for the
    /// camera bubble to keep off.
    fn toolbar_screen_rects(&self) -> Vec<capture::IRect> {
        let (ox, oy) = self.shot.origin;
        let to = |r: FRect| {
            let (x, y) = (r.x.floor() as i32, r.y.floor() as i32);
            (x + ox, y + oy, (r.x1().ceil() as i32 - x).max(0) as u32, (r.y1().ceil() as i32 - y).max(0) as u32)
        };
        // The bar (video mode: the panel), its popover, the mode switch and
        // Close (not the grip hint: it follows the bubble).
        self.toolbar.as_ref().map_or(Vec::new(), |t| {
            let top = t.items.iter().filter(|i| matches!(i.kind, toolbar::Kind::ModeTrack | toolbar::Kind::Close)).map(|i| i.r);
            std::iter::once(t.bar).chain(t.pop).chain(top).map(to).collect()
        })
    }

    /// Write the remembered settings waiting (`wait`: before returning).
    fn flush_pending(&mut self, wait: bool) {
        if self.pending.is_empty() {
            return;
        }
        let edits = self.pending.take();
        if let Some(path) = &self.cfg_path {
            config::queue_key_edits(path, edits, wait);
        }
    }

    /// Video mode on (the setup panel instead of the toolbar; no tool, no
    /// palette or picker) or off (back to the toolbar).
    fn set_video(&mut self, on: bool) {
        if on && !rec_ui::supported() {
            return;
        }
        self.rec.open = on;
        self.rec.dev_open = None;
        self.rec.ring_pal = false;
        self.palette_open = false;
        if on {
            self.close_picker(true);
            self.tool = None;
            self.draft = None;
            self.text = None;
            if self.rec.opts.mic && !self.rec.opts.mic_dev.is_empty() {
                self.rec.load_devs(); // the chosen microphone's name
            }
            self.rec.load_cams(); // whether there is one
        }
    }

    /// The panel's source: Screen selects the monitor (the selection's, or
    /// the one under `pointer`); Window drops the selection and picks a
    /// window like window snapping (a click takes it); Region keeps the
    /// selection to drag a new one.
    fn set_source(&mut self, src: rec_ui::RecSource, pointer: Pt) {
        self.rec.source = src;
        self.rec.dev_open = None;
        self.interact = Interact::None;
        match src {
            rec_ui::RecSource::Screen => self.sel = Some(pick_area(&self.shot.monitors, self.sel, pointer)),
            rec_ui::RecSource::Window => {
                self.sel = None;
                if self.snap.is_none() {
                    self.snap = take_snap_any(&self.shot);
                }
                self.track_snap(pointer);
            }
            rec_ui::RecSource::Region => {}
        }
    }

    /// Queue the camera ring's color and width for the config.
    fn remember_ring(&mut self, now: Instant) {
        let c = self.rec.opts.cam_ring.map(|c| tools::hex(c.with_alpha(255))).unwrap_or_default();
        let w = self.rec.opts.cam_ring_w;
        self.pending.set("rec_camera_ring_color".into(), Some(config::toml_str(&c)), now);
        self.pending.set("rec_camera_ring_width".into(), Some(w.to_string()), now);
        self.cfg.rec_camera_ring_color = c;
        self.cfg.rec_camera_ring_width = w;
    }

    /// Window snapping is live: windows were listed and nothing is
    /// selected, drawn or typed yet (a press that has not moved is still
    /// a click on the window).
    fn snapping(&self) -> bool {
        self.snap.is_some()
            && self.sel.is_none()
            && self.tool.is_none()
            && self.text.is_none()
            && self.pick.is_none()
            && matches!(self.interact, Interact::None | Interact::NewSel { moved: false, .. })
    }

    /// The window (or part) a click selects now, when snapping.
    fn snap_rect(&self) -> Option<FRect> {
        if !self.snapping() {
            return None;
        }
        let r = self.hover_win.rect()?;
        Some(FRect { x: r.x0 as f32, y: r.y0 as f32, w: r.w() as f32, h: r.h() as f32 })
    }

    /// Follow the window under `p` (over none: the monitor there, the
    /// "whole screen"); true when the outline changed.
    fn track_snap(&mut self, p: Pt) -> bool {
        let Some(s) = &self.snap else { return false };
        let _p = crate::perf::span(crate::perf::K::Snap);
        let mut chain = s.chain(p.x.floor() as i32, p.y.floor() as i32);
        if chain.is_empty()
            && let Some(m) = monitor_at(&self.shot.monitors, p)
        {
            chain.push(crate::snap::R::new(m.x as i32, m.y as i32, m.x1() as i32, m.y1() as i32));
        }
        let h = self.hover_win.moved(chain);
        let changed = h.rect() != self.hover_win.rect() || h.has_parts() != self.hover_win.has_parts();
        self.hover_win = h;
        changed && self.snapping()
    }

    /// Track the hovered toolbar item / handle; true when either changed.
    fn track_hover(&mut self, p: Pt) -> bool {
        self.track_hover_at(p, p)
    }

    /// [`Edit::track_hover`] with the pointer at `p` on the capture and
    /// at `pb` in the toolbar's coordinates.
    fn track_hover_at(&mut self, p: Pt, pb: Pt) -> bool {
        // The color ring is over everything: no toolbar hover (nor tooltip) under it.
        let hover = self.toolbar.as_ref().filter(|_| self.assist.ring.is_none()).and_then(|tb| tb.index_at(pb));
        let reach = HANDLE_HIT * self.ui_scale();
        let hot = self.sel.and_then(|sr| {
            handle_points(sr).iter().position(|(_, q)| (q.x - p.x).hypot(q.y - p.y) <= reach)
        });
        let mini = self.obj.mini.as_ref().and_then(|m| m.index_at(p));
        let ohot = self.obj_hot_at(p);
        let changed = hover != self.hover || hot != self.hot_handle || mini != self.obj.mini_hover || ohot != self.obj.hot;
        self.obj.mini_hover = mini;
        self.obj.hot = ohot;
        if hover != self.hover {
            self.hover = hover;
            self.hover_at = Instant::now();
            let now = Instant::now();
            self.mo.hover = Tween::new(0.0, now);
            self.mo.hover.set(1.0, 60, now);
        }
        self.hot_handle = hot;
        changed
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Make `s` the process-wide UI size (the windows without a config of
/// their own read it).
fn publish_ui_size(s: UiSize) {
    crate::uisize::set(s);
}

fn hit(r: FRect, p: Pt) -> bool {
    p.x >= r.x && p.x < r.x1() && p.y >= r.y && p.y < r.y1()
}

fn end_interaction(edit: &mut Edit) {
    match std::mem::replace(&mut edit.interact, Interact::None) {
        Interact::Drawing { .. } if edit.obj.click_pick.is_some() => {
            // A Ctrl+click on an object: it is selected, nothing drawn.
            let (i, _) = edit.obj.click_pick.take().expect("click pick");
            edit.draft = None;
            edit.stroke_pts.clear();
            edit.obj.sel = vec![i];
        }
        Interact::Drawing { .. } => {
            if edit.draft_is_valid() {
                let d = edit.draft.take().expect("draft exists");
                if matches!(d, Obj::Callout { .. }) {
                    edit.place_callout(d);
                } else {
                    edit.commit_object(d);
                }
            }
            edit.draft = None;
            edit.stroke_pts.clear();
        }
        it @ (Interact::ObjMove { .. } | Interact::ObjResize { .. } | Interact::ObjPoint { .. } | Interact::Marquee { .. }) => {
            edit.obj_release(it);
        }
        Interact::NewSel { moved, .. } => {
            // A window clicked (or a region drawn): accept on select may
            // take it; a plain click that selects everything does not.
            let mut window = moved;
            if !moved {
                // A plain click selects the window under the pointer
                // (window snapping), else everything (Flameshot-ish).
                let all = FRect { x: 0.0, y: 0.0, w: edit.shot.size.0 as f32, h: edit.shot.size.1 as f32 };
                let snapped = edit.snap_rect();
                window = snapped.is_some();
                edit.sel = Some(snapped.unwrap_or(all));
            } else {
                // A dragged area is a region, whatever the Record panel's
                // source was (Window, Screen).
                edit.rec.source = rec_ui::RecSource::Region;
            }
            if edit.accept_on_select {
                edit.done = true;
                edit.cancelled = false;
            } else if window {
                edit.accept_new_region();
            }
        }
        // A window's or a monitor's area moved or resized: a region now.
        Interact::MoveSel { orig, .. } | Interact::Resize { orig, .. } => {
            if edit.sel != Some(orig) {
                edit.rec.source = rec_ui::RecSource::Region;
            }
        }
        Interact::Pick(_) | Interact::None => {}
    }
}

fn handle_points(sr: FRect) -> [(Handle, Pt); 8] {
    let (l, r) = (sr.x, sr.x1());
    let (t, b) = (sr.y, sr.y1());
    let (cx, cy) = (sr.x + sr.w / 2.0, sr.y + sr.h / 2.0);
    [
        (Handle::NW, Pt::new(l, t)),
        (Handle::N, Pt::new(cx, t)),
        (Handle::NE, Pt::new(r, t)),
        (Handle::E, Pt::new(r, cy)),
        (Handle::SE, Pt::new(r, b)),
        (Handle::S, Pt::new(cx, b)),
        (Handle::SW, Pt::new(l, b)),
        (Handle::W, Pt::new(l, cy)),
    ]
}

fn handle_cursor(h: Handle) -> Cursor {
    match h {
        Handle::NW | Handle::SE => Cursor::SizeNWSE,
        Handle::NE | Handle::SW => Cursor::SizeNESW,
        Handle::N | Handle::S => Cursor::SizeNS,
        Handle::E | Handle::W => Cursor::SizeWE,
    }
}

/// Photoshop-style Shift constraint: force the drag extent into a square,
/// keeping the larger dragged dimension and the drag quadrant.
fn constrain_square(start: Pt, cur: Pt) -> Pt {
    let dx = cur.x - start.x;
    let dy = cur.y - start.y;
    let side = dx.abs().max(dy.abs());
    Pt::new(
        start.x + side.copysign(dx),
        start.y + side.copysign(dy),
    )
}

/// Shrink a Shift-constrained square so it fits inside the shot without
/// moving `anchor`: the side is limited by the room from the anchor to the
/// shot edge in the drag direction on both axes.
fn square_within(anchor: Pt, cur: Pt, img: (f32, f32)) -> Pt {
    let dx = cur.x - anchor.x;
    let dy = cur.y - anchor.y;
    let room_x = if dx < 0.0 { anchor.x } else { img.0 - anchor.x };
    let room_y = if dy < 0.0 { anchor.y } else { img.1 - anchor.y };
    let side = dx.abs().min(room_x.max(0.0)).min(room_y.max(0.0));
    Pt::new(anchor.x + side.copysign(dx), anchor.y + side.copysign(dy))
}

/// Compute the selection rect while resizing with a handle.
fn resize_rect(
    h: Handle,
    orig: FRect,
    cur: Pt,
    aspect: f32,
    mods: &Mods,
    img: (f32, f32),
) -> FRect {
    let cur = Pt {
        x: cur.x.round(),
        y: cur.y.round(),
    };
    let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.x1(), orig.y1());
    let left = matches!(h, Handle::W | Handle::NW | Handle::SW);
    let right = matches!(h, Handle::E | Handle::NE | Handle::SE);
    let top = matches!(h, Handle::N | Handle::NW | Handle::NE);
    let bot = matches!(h, Handle::S | Handle::SW | Handle::SE);

    if left {
        x0 = cur.x;
    }
    if right {
        x1 = cur.x;
    }
    if top {
        y0 = cur.y;
    }
    if bot {
        y1 = cur.y;
    }
    if mods.shift {
        if left {
            x1 = orig.x1() + (orig.x - x0);
        }
        if right {
            x0 = orig.x - (x1 - orig.x1());
        }
        if top {
            y1 = orig.y1() + (orig.y - y0);
        }
        if bot {
            y0 = orig.y - (y1 - orig.y1());
        }
    }
    if mods.ctrl && aspect > 0.0 {
        let w = (x1 - x0).abs();
        let hh = (y1 - y0).abs();
        if w / aspect <= hh {
            let nh = w / aspect;
            let cy = (y0 + y1) / 2.0;
            y0 = cy - nh / 2.0;
            y1 = cy + nh / 2.0;
        } else {
            let nw = hh * aspect;
            let cx = (x0 + x1) / 2.0;
            x0 = cx - nw / 2.0;
            x1 = cx + nw / 2.0;
        }
    }
    let mut r = FRect {
        x: x0.min(x1),
        y: y0.min(y1),
        w: (x1 - x0).abs(),
        h: (y1 - y0).abs(),
    };
    r.clamp_to(img.0, img.1);
    r
}

/// Dashed box around the text draft: 4 px padding, at least 24×28; the text
/// itself is drawn at `td.pos`, exactly where the committed object renders.
fn text_box_rect(win_w: f32, td: &TextDraft, font_px: f32, font: Option<&AnnotFont>, s: f32) -> FRect {
    let pad = 4.0 * s;
    let tw = font.map(|f| f.line_width(&td.text, font_px)).unwrap_or(0.0);
    let w = (tw + 2.0 * pad + 2.0 * s).max(24.0 * s);
    let h = (font_px * 1.15 + 2.0 * pad).max(28.0 * s);
    let x = (td.pos.x - pad).min((win_w - w).max(0.0));
    FRect { x, y: td.pos.y - pad, w, h }
}

/// The marker's translucency on top of the color's own opacity.
fn marker_color(c: C4) -> C4 {
    c.with_alpha(((c.a as u32 * 90 + 127) / 255) as u8)
}

fn palette_colors(cfg: &Config) -> Vec<C4> {
    cfg.user_colors
        .iter()
        .filter_map(|c| config::parse_color(c).map(|(r, g, b, a)| C4::new(r, g, b, a)))
        .collect()
}

fn prev_boundary(s: &str, i: usize) -> usize {
    s[..i]
        .char_indices()
        .next_back()
        .map(|(k, _)| k)
        .unwrap_or(0)
}

fn next_boundary(s: &str, i: usize) -> usize {
    s[i..].char_indices().nth(1).map(|(k, _)| i + k).unwrap_or(s.len())
}

/// Something on screen moves by itself: a tween, the pending tooltip,
/// a toast fading in or out; or the camera list is on its way (polled
/// on the timer).
fn animating(edit: &Edit, app_notice: Option<&Toast>, now: Instant) -> bool {
    let tip_pending = edit.hover.is_some() && now.saturating_duration_since(edit.hover_at) < Duration::from_millis(500);
    let toast_moving = edit.notice.as_ref().or(app_notice).is_some_and(|t| t.animating(now));
    // The Camera row says "Looking for cameras…" until the list comes.
    edit.mo.active(now) || tip_pending || toast_moving || edit.rec.cams_looking() || edit.guides_fading(now) || edit.assist.hint_due(now)
}

/// The last frame no longer matches: something animates (or did when it
/// was drawn: its end state is not on screen yet), the caret blink phase
/// flipped, or the toast that would be drawn now differs from the one
/// drawn (set, cleared or replaced between frames).
fn stale(edit: &Edit, app_notice: Option<&Toast>, now: Instant) -> bool {
    let toast_now = edit.notice.as_ref().or(app_notice).map(|t| t.at);
    edit.anim_drawn
        || animating(edit, app_notice, now)
        || edit.text.as_ref().map(caret_on).or_else(|| edit.cedit_caret_on()) != edit.caret_drawn
        || toast_now != edit.toast_drawn
}

/// Caret blink phase: shown 530 ms, hidden 530 ms.
fn caret_on(td: &TextDraft) -> bool {
    blink_on(td.at)
}

/// The caret's blink phase for an edit at `at`: on, off every 530 ms.
fn blink_on(at: Instant) -> bool {
    (at.elapsed().as_millis() / 530).is_multiple_of(2)
}

/// Blends toward a dim color by alpha `a`, two RGBA pixels per `u64`
/// (SWAR, 16-bit lanes): per channel `(v * (255 - a) + c * a + 127) / 255`.
/// For integer inputs that is exactly the rounding of the former per-pixel
/// float blend (`n / 255` is never within float error of a half), so the
/// output is bit-identical. Alpha bytes pass through. Built per alpha (the
/// fade-in changes it every frame), which costs nothing.
#[derive(Clone, Copy)]
struct Dimmer {
    k: u64,
    add_rb: u64,
    add_ga: u64,
}

const LANES: u64 = 0x00FF_00FF_00FF_00FF;
const ALPHA: u64 = 0xFF00_0000_FF00_0000;

impl Dimmer {
    fn new(c: C4) -> Self {
        let a = c.a as u64;
        let lane = |lo: u8, hi: u8| {
            let pair = (lo as u64 * a + 127) | ((hi as u64 * a + 127) << 16);
            pair | (pair << 32)
        };
        // Lanes (little-endian bytes R G B A): R|B and G|A of two pixels.
        Dimmer { k: 255 - a, add_rb: lane(c.r, c.b), add_ga: lane(c.g, 0) }
    }

    /// Exact `x / 255` per 16-bit lane for lane values below 65535
    /// (ours stay below 65153).
    #[inline(always)]
    fn div255(x: u64) -> u64 {
        ((x + ((x >> 8) & LANES) + 0x0001_0001_0001_0001) >> 8) & LANES
    }

    #[inline(always)]
    fn pair(&self, v: u64) -> u64 {
        let rb = Self::div255((v & LANES) * self.k + self.add_rb);
        let ga = Self::div255(((v >> 8) & LANES) * self.k + self.add_ga);
        ((rb | (ga << 8)) & !ALPHA) | (v & ALPHA)
    }

    /// `buf` blended in place, `times` over.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn apply(&self, buf: &mut [u8], times: i32) {
        let (b8, bt) = buf.as_chunks_mut::<8>();
        for o in b8 {
            let mut v = u64::from_le_bytes(*o);
            for _ in 0..times {
                v = self.pair(v);
            }
            *o = v.to_le_bytes();
        }
        if let Some(o) = bt.first_chunk_mut::<4>() {
            let mut v = u32::from_le_bytes(*o) as u64;
            for _ in 0..times {
                v = self.pair(v);
            }
            *o = (v as u32).to_le_bytes();
        }
    }

    /// `dst = src` blended `times` over (overlapping dim rects blend once
    /// per rect; only fractional selections overlap).
    fn run(&self, src: &[u8], dst: &mut [u8], times: i32) {
        let (d8, dt) = dst.as_chunks_mut::<8>();
        let (s8, st) = src.as_chunks::<8>();
        if times == 1 {
            for (o, i) in d8.iter_mut().zip(s8) {
                *o = self.pair(u64::from_le_bytes(*i)).to_le_bytes();
            }
        } else {
            for (o, i) in d8.iter_mut().zip(s8) {
                let mut v = u64::from_le_bytes(*i);
                for _ in 0..times {
                    v = self.pair(v);
                }
                *o = v.to_le_bytes();
            }
        }
        if let (Some(o), Some(i)) = (dt.first_chunk_mut::<4>(), st.first_chunk::<4>()) {
            let mut v = u32::from_le_bytes(*i) as u64;
            for _ in 0..times {
                v = self.pair(v);
            }
            *o = (v as u32).to_le_bytes();
        }
    }
}

fn x1_clamp(x: f32, w: f32, bw: i32) -> i32 {
    (x + w).ceil().min(bw as f32) as i32
}

fn y1_clamp(y: f32, h: f32, bh: i32) -> i32 {
    (y + h).ceil().min(bh as f32) as i32
}

/// Crop a rect from an unpremultiplied RGBA image (clamped to bounds).
fn crop_to_image(img: &PixBuf, r: FRect) -> PixBuf {
    let pw = img.width() as i32;
    let ph = img.height() as i32;
    let x0 = r.x.floor().max(0.0) as i32;
    let y0 = r.y.floor().max(0.0) as i32;
    let x1 = x1_clamp(r.x, r.w, pw);
    let y1 = y1_clamp(r.y, r.h, ph);
    let w = (x1 - x0).max(1) as u32;
    let h = (y1 - y0).max(1) as u32;
    let data = img.as_raw();
    let mut out = PixBuf::new(w, h);
    let dst = out.as_raw_mut();
    for y in 0..h {
        for x in 0..w {
            let sx = (x0.max(0) as u32 + x).min(img.width() - 1);
            let sy = (y0.max(0) as u32 + y).min(img.height() - 1);
            let si = ((sy * img.width() + sx) as usize) * 4;
            let di = ((y * w + x) as usize) * 4;
            dst[di..di + 4].copy_from_slice(&data[si..si + 4]);
        }
    }
    out
}

/// The monitor chrome is placed on: largest overlap with `sel` (ties go to
/// the first), else the one under `pointer`, else the first.
fn pick_area(monitors: &[capture::IRect], sel: Option<FRect>, pointer: Pt) -> FRect {
    let rect = |m: &capture::IRect| FRect { x: m.0 as f32, y: m.1 as f32, w: m.2 as f32, h: m.3 as f32 };
    let first = monitors.first().map(rect).unwrap_or(FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
    if let Some(sel) = sel {
        let overlap = |r: FRect| {
            let w = (sel.x1().min(r.x1()) - sel.x.max(r.x)).max(0.0);
            let h = (sel.y1().min(r.y1()) - sel.y.max(r.y)).max(0.0);
            w * h
        };
        let mut best: Option<(f32, FRect)> = None;
        for r in monitors.iter().map(rect) {
            let o = overlap(r);
            if o > 0.0 && best.is_none_or(|(bo, _)| o > bo) {
                best = Some((o, r));
            }
        }
        if let Some((_, r)) = best {
            return r;
        }
    }
    monitors.iter().map(rect).find(|r| toolbar::hit(*r, pointer)).unwrap_or(first)
}

/// The monitor (image coordinates) under `p`, or the nearest one when `p`
/// is in a gap between monitors; `None` without monitors.
fn monitor_at(monitors: &[capture::IRect], p: Pt) -> Option<FRect> {
    let rect = |m: &capture::IRect| FRect { x: m.0 as f32, y: m.1 as f32, w: m.2 as f32, h: m.3 as f32 };
    let dist = |r: &FRect| {
        let dx = (r.x - p.x).max(p.x - r.x1()).max(0.0);
        let dy = (r.y - p.y).max(p.y - r.y1()).max(0.0);
        dx * dx + dy * dy
    };
    monitors.iter().map(rect).find(|r| toolbar::hit(*r, p)).or_else(|| {
        monitors.iter().map(rect).min_by(|a, b| dist(a).total_cmp(&dist(b)))
    })
}

#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod tools_tests;
#[cfg(test)]
mod tools2_tests;
#[cfg(test)]
mod uisize_tests;
#[cfg(test)]
mod snap_tests;
#[cfg(test)]
mod objects_tests;
#[cfg(test)]
mod ocr_tests;
#[cfg(test)]
mod pin_tests;
#[cfg(test)]
mod history_tests;

#[cfg(test)]
mod tests {
    use super::*;

    // ---- developer preview (renders overlay frames to PNG; no display) ----

    /// 2400x900 desktop: a shorter, offset left monitor and a taller right one.
    pub(super) fn two_monitor_shot() -> Shot {
        let mut shot = synthetic_shot();
        let (w, h) = (2400u32, 900u32);
        let mut img = PixBuf::from_pixel(w, h, [30, 34, 44, 255]);
        let src = shot.image.clone();
        for y in 0..h {
            for x in 0..w {
                let i = (y as usize * w as usize + x as usize) * 4;
                let (sx, sy) = (x % src.width(), y.min(src.height() - 1));
                let j = (sy as usize * src.width() as usize + sx as usize) * 4;
                img.as_raw_mut()[i..i + 4].copy_from_slice(&src.as_raw()[j..j + 4]);
            }
        }
        shot.size = (w, h);
        shot.image = img;
        shot.monitors = vec![(0, 200, 1200, 700), (1200, 0, 1200, 900)];
        shot
    }

    pub(super) fn synthetic_shot() -> Shot {
        let (w, h) = (1440u32, 900u32);
        let mut img = PixBuf::from_pixel(w, h, [0, 0, 0, 255]);
        let mut put = |x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 3]| {
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let i = (y as usize * w as usize + x as usize) * 4;
                    img.as_raw_mut()[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        };
        for y in 0..h {
            let t = y as f32 / h as f32;
            put(0, y, w, y + 1, [(20.0 + 20.0 * t) as u8, (40.0 + 40.0 * t) as u8, (90.0 + 60.0 * t) as u8]);
        }
        put(120, 90, 1320, 810, [245, 246, 248]); // window
        put(120, 90, 1320, 130, [225, 228, 233]); // title bar
        put(120, 130, 340, 810, [236, 239, 243]); // sidebar
        for i in 0..8 {
            put(140, 160 + i * 40, 320, 182 + i * 40, [200, 206, 216]);
        }
        for i in 0..6 {
            put(370, 160 + i * 60, 1290 - i * 90, 172 + i * 60, [180, 188, 200]);
        }
        put(370, 540, 700, 760, [92, 140, 230]);
        put(740, 540, 1060, 760, [240, 170, 70]);
        put(1100, 540, 1290, 760, [110, 190, 140]);
        Shot {
            origin: (0, 0),
            size: (w, h),
            scale: 1.0,
            image: img,
            monitors: vec![(0, 0, w, h)],
        }
    }

    pub(super) fn preview_app(th: Theme, sel: Option<FRect>) -> App {
        preview_app_with(th, sel, synthetic_shot())
    }

    /// An update's restart never quits under an open capture: it waits
    /// until the editor is hidden again.
    #[test]
    fn restart_waits_for_the_open_capture() {
        let mut app = preview_app(theme::DARK, None);
        assert!(!app.restart_due(), "nothing asked");
        app.restart_pending = true;
        assert!(!app.restart_due(), "capture open");
        assert!(app.restart_pending);
        app.st = State::Hidden;
        app.pending = Some(Pending::editor());
        assert!(!app.restart_due(), "capture about to open");
        app.pending = None;
        assert!(app.restart_due(), "idle: quit now");
        assert!(!app.restart_pending && !app.restart_due(), "once");
    }

    /// An Edit window with unsaved edits holds an update's restart (and
    /// its install) until it is saved or closed; one without does not.
    #[test]
    fn restart_waits_for_unsaved_edit_windows() {
        let mut app = preview_app(theme::DARK, None);
        app.st = State::Hidden;
        let clean = app.edits.add_for_tests("2026-10-05/143200", false);
        let dirty = app.edits.add_for_tests("2026-10-05/143300", true);
        app.restart_pending = true;
        assert!(!app.restart_due(), "unsaved edits");
        let (tx, rx) = std::sync::mpsc::channel();
        app.idle_reply = Some(tx);
        app.idle_gate();
        assert!(rx.try_recv().is_err(), "no install meanwhile");
        dirty.store(false, Ordering::SeqCst); // saved
        app.idle_gate();
        assert_eq!(rx.try_recv(), Ok(()));
        assert!(app.restart_due(), "saved: restart");
        drop(clean);
    }

    /// Quit asks the Edit windows with unsaved edits and waits for their
    /// answers: done once none is left, given up on Cancel; with none
    /// unsaved it quits at once.
    #[test]
    fn quit_asks_unsaved_edit_windows_first() {
        use crate::edit_ui::{QuitGate, quit_gate};
        assert_eq!(quit_gate(0, false), QuitGate::Now);
        assert_eq!(quit_gate(2, false), QuitGate::Wait);
        assert_eq!(quit_gate(1, true), QuitGate::Abort);
        assert_eq!(quit_gate(0, true), QuitGate::Abort);

        let mut app = preview_app(theme::DARK, None);
        app.st = State::Hidden;
        let a = app.edits.add_for_tests("2026-10-05/143200", true);
        let _b = app.edits.add_for_tests("2026-10-05/143300", false);
        assert!(app.on_hot(HotEvent::Quit), "still running: the window asks");
        assert!(app.quit_pending);
        app.quit_gate();
        assert!(app.quit_pending, "waiting for the answer");
        // Cancel in the window's question: the quit is off.
        app.edits.cancel_for_tests();
        app.quit_gate();
        assert!(!app.quit_pending);
        // Again; this time the window is saved (or closed).
        assert!(app.on_hot(HotEvent::Quit));
        a.store(false, Ordering::SeqCst);
        app.edits.remove_for_tests("2026-10-05/143200");
        app.quit_gate();
        assert!(!app.quit_pending, "quit");
        // Nothing unsaved: quit at once.
        let mut app = preview_app(theme::DARK, None);
        app.st = State::Hidden;
        let _c = app.edits.add_for_tests("2026-10-05/143400", false);
        assert!(!app.on_hot(HotEvent::Quit));
    }

    /// An upload still in flight keeps the daemon from being idle.
    pub(super) fn upload_in_flight(app: &App) -> std::sync::mpsc::Sender<Result<String, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        *app.upload_slot.lock().unwrap() = Some(rx);
        tx
    }

    /// A Settings save: the next capture uses the new config (the open one
    /// keeps its own) and the update checker follows `check_updates`.
    #[test]
    fn reload_takes_the_new_config() {
        let mut app = preview_app(theme::DARK, None);
        app.kind = RunKind::Daemon;
        let (_tx, rx) = std::sync::mpsc::channel();
        app.updates = Some(rx);
        let cfg = Config { save_format: "jpg".into(), check_updates: false, ..Config::default() };
        app.reload_config(cfg.clone());
        assert_eq!(app.cfg, cfg);
        assert!(app.updates.is_none(), "checker stopped");
        let State::Edit(edit) = &app.st else { panic!("capture still open") };
        assert_eq!(edit.cfg.save_format, "png");
    }

    /// The update worker installs only once no capture, editor or upload
    /// runs; from its request on no new capture starts.
    #[test]
    fn idle_gate_waits_for_capture_and_upload() {
        let mut app = preview_app(theme::DARK, None); // a capture is open
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(app.on_hot(HotEvent::RestartWhenIdle(tx)));
        app.idle_gate();
        assert!(rx.try_recv().is_err(), "capture open: no reply");
        app.st = State::Hidden;
        let up = upload_in_flight(&app);
        app.idle_gate();
        assert!(rx.try_recv().is_err(), "upload in flight: no reply");
        up.send(Err("offline".into())).unwrap();
        app.poll_upload();
        app.idle_gate();
        assert_eq!(rx.try_recv(), Ok(()), "idle: reply");
        app.idle_gate();
        assert!(rx.try_recv().is_err(), "reply once");
        // From now on until the exit a capture is refused.
        assert!(app.on_hot(HotEvent::Capture));
        assert!(app.pending.is_none(), "capture refused after the gate");
        // The install failed: captures work again.
        assert!(app.on_hot(HotEvent::RestartAborted));
        assert!(app.on_hot(HotEvent::Capture));
        assert!(app.pending.is_some());
    }

    /// A capture asked while the worker waits is refused too (it would
    /// keep the daemon busy); a reply nobody reads lifts the gate.
    #[test]
    fn idle_gate_refuses_captures_while_waiting() {
        let mut app = preview_app(theme::DARK, None);
        app.st = State::Hidden;
        let (tx, rx) = std::sync::mpsc::channel();
        app.pending = Some(Pending::editor()); // a capture about to open
        app.on_hot(HotEvent::RestartWhenIdle(tx));
        app.idle_gate();
        assert!(rx.try_recv().is_err(), "capture about to open: no reply");
        app.pending = None;
        app.on_hot(HotEvent::Capture);
        assert!(app.pending.is_none(), "refused while waiting");
        drop(rx); // the worker gave up
        app.idle_gate();
        assert!(app.idle_reply.is_none() && !app.updating);
        app.on_hot(HotEvent::Capture);
        assert!(app.pending.is_some());
    }

    /// `Restart` sets the flag; a capture is ignored while it is pending.
    #[test]
    fn restart_event_sets_the_flag_and_refuses_captures() {
        let mut app = preview_app(theme::DARK, None);
        app.st = State::Hidden;
        assert!(app.on_hot(HotEvent::Restart));
        assert!(app.restart_pending);
        assert!(app.on_hot(HotEvent::Capture));
        assert!(app.pending.is_none());
        let _up = upload_in_flight(&app);
        assert!(!app.restart_due(), "upload in flight");
    }

    pub(super) fn preview_app_with(th: Theme, sel: Option<FRect>, shot: Shot) -> App {
        let theme_name = if th == theme::DARK { "dark" } else { "light" };
        let cfg = Config { theme: theme_name.into(), ..Config::default() };
        let font = AnnotFont::load();
        let ui_font = Some(fonts::ui_font());
        let mut shot = shot;
        let base = std::mem::take(&mut shot.image);
        let mut edit = Edit::new(shot, base, cfg.clone(), th, font.clone(), ui_font, RecPop::new(RecOpts::from_config(&cfg), false));
        edit.sel = sel;
        App {
            cfg,
            kind: RunKind::OneShot,
            hot: None,
            pending: None,
            st: State::Edit(Box::new(edit)),
            exit_code: Arc::new(AtomicI32::new(0)),
            upload_slot: Arc::new(Mutex::new(None)),
            ocr: Vec::new(),
            updates: None,
            update_pending: None,
            restart_pending: false,
            updating: false,
            idle_reply: None,
            edits: Box::leak(Box::new(crate::edit_ui::Windows::new())),
            quit_pending: false,
            font,
            ui_font,
            notice: None,
            hwnd: Hwnd::default(),
            mouse: (0, 0),
            focus_tries: 0,
            rec: None,
            rec_opts: None,
            last_region: None,
            rec_area: None,
            wait_pins: false,
            history_root: None,
            last_entry: None,
            upload_entry: None,
            rec_poster: None,
            delay: Default::default(),
            delay_show: crate::launcher::Screen { th: theme::DARK, up: false },
            rec_spec: None,
            rec_cam: None,
            rec_bubble: None,
        }
    }

    pub(super) fn edit_of(app: &mut App) -> &mut Edit {
        match &mut app.st {
            State::Edit(e) => e,
            _ => unreachable!(),
        }
    }

    /// Press `vk` without finishing the capture (no export runs).
    fn press(app: &mut App, vk: u32, mods: Mods) -> Edit {
        let State::Edit(e) = std::mem::replace(&mut app.st, State::Hidden) else { unreachable!() };
        let mut e = *e;
        app.handle_key(&mut e, vk, false, mods);
        e
    }

    const CTRL: Mods = Mods { ctrl: true, shift: false, alt: false };
    const CTRL_SHIFT: Mods = Mods { ctrl: true, shift: true, alt: false };

    /// A frame drawn mid-animation owes one more at the end values: the
    /// fade-in's last frame (the toolbar a pixel short of its place) never
    /// stays on screen once the animation is over and the ticks go idle.
    #[test]
    fn the_last_animation_frame_is_drawn() {
        let sel = FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 };
        let mut app = preview_app(theme::DARK, Some(sel));
        let t0 = Instant::now();
        let e = edit_of(&mut app);
        e.mo = Motion::new(t0);
        e.hover = None;
        let ms = |n| t0 + Duration::from_millis(n);
        e.prepare(None, (0, 0), ms(60));
        assert!(animating(e, None, ms(60)), "the dim fades in over 120 ms");
        assert!(!animating(e, None, ms(500)));
        assert!(stale(e, None, ms(500)), "the frame at 60 ms is not the end state");
        e.prepare(None, (0, 0), ms(500));
        assert!(!stale(e, None, ms(600)), "the end state is on screen: idle");
    }

    #[test]
    fn keymap_drives_handle_key() {
        let sel = FRect { x: 10.0, y: 10.0, w: 50.0, h: 40.0 };
        let mut app = preview_app(theme::DARK, Some(sel));
        let e = press(&mut app, 'S' as u32, CTRL_SHIFT);
        assert!(e.done && matches!(e.tasks[..], [Task::Save { path: None, ask: true }]), "Save As asks");
        let mut app = preview_app(theme::DARK, Some(sel));
        let e = press(&mut app, 'S' as u32, CTRL);
        assert!(e.done && matches!(e.tasks[..], [Task::Save { path: None, ask: false }]));

        let mut app = preview_app(theme::DARK, Some(sel));
        let e = press(&mut app, 'A' as u32, CTRL);
        assert_eq!(e.sel, Some(FRect { x: 0.0, y: 0.0, w: e.shot.size.0 as f32, h: e.shot.size.1 as f32 }));
        assert!(!e.done);

        let mut app = preview_app(theme::DARK, Some(sel));
        assert_eq!(press(&mut app, 'L' as u32, Mods::default()).tool, Some(Tool::Line));
        let mut app = preview_app(theme::DARK, Some(sel));
        assert_eq!(press(&mut app, 'L' as u32, Mods { shift: true, ..Mods::default() }).tool, None);
        let mut app = preview_app(theme::DARK, Some(sel));
        assert!(press(&mut app, wind::key::SPACE, Mods::default()).palette_open);

        // Remapped: Pencil on K, Cancel unbound from Esc.
        let mut app = preview_app(theme::DARK, Some(sel));
        let km = [("tool_pencil", "K"), ("cancel", "")].map(|(a, b)| (a.to_string(), b.to_string()));
        edit_of(&mut app).keys = Keymap::from_config(&km.into_iter().collect()).0;
        let mut e = press(&mut app, 'K' as u32, Mods::default());
        assert_eq!(e.tool, Some(Tool::Path));
        app.handle_key(&mut e, 'P' as u32, false, Mods::default());
        assert_eq!(e.tool, Some(Tool::Path), "P no longer bound");
        app.handle_key(&mut e, wind::key::ESCAPE, false, Mods::default());
        assert!(e.tool.is_some() && !e.done, "Esc unbound");
    }

    /// Selection with the Arrow tool, a Rect and an Arrow object inside it.
    pub(super) fn annotated(th: Theme, sel: FRect) -> App {
        let mut app = preview_app(th, Some(sel));
        let e = edit_of(&mut app);
        e.tool = Some(Tool::Arrow);
        let red = C4::rgb(240, 68, 56);
        e.objects.push(Obj::Rect {
            r: FRect { x: sel.x + 40.0, y: sel.y + 40.0, w: sel.w * 0.35, h: sel.h * 0.3 },
            color: red,
            width: 3.0,
        });
        e.objects.push(Obj::Arrow {
            a: Pt::new(sel.x + sel.w * 0.8, sel.y + sel.h * 0.8),
            b: Pt::new(sel.x + sel.w * 0.5, sel.y + sel.h * 0.5),
            color: red,
            width: 4.0,
        });
        e.dirty = true;
        app
    }

    fn one_rect(sel: FRect) -> Obj {
        Obj::Rect {
            r: FRect { x: sel.x + 40.0, y: sel.y + 40.0, w: sel.w * 0.35, h: sel.h * 0.3 },
            color: C4::rgb(240, 68, 56),
            width: 3.0,
        }
    }

    /// A fresh edit has no `composed` copy; frame and export match what a
    /// materialised copy of `base` would give.
    #[test]
    fn fresh_edit_has_no_composed_and_output_matches_base() {
        let sel = FRect { x: 100.0, y: 80.0, w: 300.0, h: 200.0 };
        let mut app = preview_app(theme::DARK, Some(sel));
        let e = edit_of(&mut app);
        assert!(e.composed.is_none());
        assert!(std::ptr::eq(e.composed(), &e.base));
        let want_crop = crop_to_image(&e.base.clone(), sel);
        assert!(crop_to_image(e.composed(), sel) == want_crop);
        let got = app.frame().expect("frame").clone();
        // Same edit with an explicit identical copy of base.
        let mut app2 = preview_app(theme::DARK, Some(sel));
        let e2 = edit_of(&mut app2);
        e2.composed = Some(e2.base.clone());
        let want = app2.frame().expect("frame").clone();
        // Animation state starts at the same instant-ish; compare the
        // captured area inside the selection, which no tween touches.
        let (x0, y0) = (sel.x as u32 + 8, sel.y as u32 + 8);
        let w = sel.w as u32 - 16;
        let h = sel.h as u32 - 16;
        assert!(got.crop(x0, y0, w, h) == want.crop(x0, y0, w, h));
        assert!(edit_of(&mut app).composed.is_none(), "frame() must not allocate it");
    }

    /// Committing an object allocates `composed`; undoing back to zero
    /// objects drops it; redo brings it back.
    #[test]
    fn composed_allocated_on_commit_and_dropped_on_undo() {
        let sel = FRect { x: 100.0, y: 80.0, w: 300.0, h: 200.0 };
        let mut app = preview_app(theme::DARK, Some(sel));
        let e = edit_of(&mut app);
        e.commit_object(one_rect(sel));
        e.rebuild();
        assert!(e.composed.is_some());
        assert!(e.composed() != &e.base, "object is baked in");
        e.undo();
        e.rebuild();
        assert!(e.composed.is_none());
        assert!(e.composed() == &e.base);
        e.redo();
        e.rebuild();
        assert!(e.composed.is_some() && e.composed() != &e.base);
    }

    /// The Windows/Linux presenters convert the frame in place: no
    /// staging thread-local remains (structural check on the source).
    #[test]
    fn presenters_have_no_staging_buffer() {
        for src in [include_str!("../wind_win.rs"), include_str!("../wind_linux.rs")] {
            let prod = src.split("#[cfg(test)]").next().unwrap();
            assert!(!prod.contains("static BGRA") && !prod.contains("static BGRX"), "staging buffer is back");
        }
    }

    /// The frame with every tween settled (and a toast past its fade-in).
    pub(super) fn settled_frame(app: &mut App) -> PixBuf {
        if let State::Edit(e) = &mut app.st {
            for t in [&mut e.mo.dim, &mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint, &mut e.mo.hover] {
                t.snap(t.target());
            }
            if let Some(n) = e.notice.as_mut() {
                n.at = Instant::now() - Duration::from_millis(500);
            }
        }
        app.frame(); // settle: tweens that start in frame() (bar/pop/hint)
        if let State::Edit(e) = &mut app.st {
            for t in [&mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint] {
                t.snap(t.target());
            }
        }
        app.frame().expect("frame").clone()
    }

    #[test]
    #[ignore = "writes preview PNGs; set RUSTSHOT_PREVIEW_DIR"]
    fn render_preview_pngs() {
        let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let save = |app: &mut App, name: &str| {
            if let State::Edit(e) = &mut app.st {
                for t in [&mut e.mo.dim, &mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint, &mut e.mo.hover] {
                    t.snap(t.target());
                }
                if let Some(n) = e.notice.as_mut() {
                    n.at = Instant::now() - Duration::from_millis(500);
                }
            }
            app.frame(); // settle: tweens that start in frame() (bar/pop/hint)
            if let State::Edit(e) = &mut app.st {
                for t in [&mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint] {
                    t.snap(t.target());
                }
            }
            let png = app.frame().expect("frame").to_png().unwrap();
            std::fs::write(dir.join(name), png).unwrap();
        };
        let big = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };

        let mut app = annotated(theme::DARK, big);
        app.frame();
        let e = edit_of(&mut app);
        let copy = e
            .toolbar
            .as_ref()
            .and_then(|t| t.items.iter().position(|i| i.kind == toolbar::Kind::Btn(Act::Copy)))
            .expect("copy button");
        e.hover = Some(copy);
        e.hover_at = Instant::now() - Duration::from_secs(2);
        e.notice = Some(Toast::new("Size 4", ToastKind::Info));
        save(&mut app, "dark-toolbar.png");

        let e = edit_of(&mut app);
        e.palette_open = true;
        e.hover = None;
        save(&mut app, "dark-palette.png");

        let mut app = annotated(theme::DARK, FRect { x: 500.0, y: 300.0, w: 380.0, h: 200.0 });
        save(&mut app, "dark-narrow.png");

        // Tooltips with word key caps (Ctrl Shift Z): the widest ones, and
        // at the screen edge on a toolbar pushed against it.
        let btn = |app: &mut App, act: Act| {
            let e = edit_of(app);
            e.toolbar.as_ref().and_then(|t| t.items.iter().position(|i| i.kind == toolbar::Kind::Btn(act))).expect("button")
        };
        for (name, th, sel, act) in [
            ("dark-tooltip-redo.png", theme::DARK, big, Act::Redo),
            ("light-tooltip-save.png", theme::LIGHT, big, Act::Save),
            ("dark-tooltip-edge.png", theme::DARK, FRect { x: 1036.0, y: 4.0, w: 400.0, h: 200.0 }, Act::Exit),
            ("light-tooltip-edge-left.png", theme::LIGHT, FRect { x: 4.0, y: 700.0, w: 300.0, h: 196.0 }, Act::Undo),
        ] {
            let mut app = annotated(th, sel);
            app.frame();
            let i = btn(&mut app, act);
            let e = edit_of(&mut app);
            e.hover = Some(i);
            e.hover_at = Instant::now() - Duration::from_secs(2);
            save(&mut app, name);
        }

        let mut app = annotated(theme::LIGHT, big);
        let e = edit_of(&mut app);
        e.hover = Some(copy);
        e.hover_at = Instant::now() - Duration::from_secs(2);
        e.notice = Some(Toast::new("Size 4", ToastKind::Info));
        save(&mut app, "light-toolbar.png");

        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        e.tool = Some(Tool::Text);
        let text = "Check this".to_string();
        e.text = Some(TextDraft { pos: Pt::new(400.0, 300.0), caret: text.len(), text, at: Instant::now() });
        save(&mut app, "dark-text.png");

        let mut app = annotated(theme::LIGHT, big);
        let e = edit_of(&mut app);
        e.objects.push(Obj::Text {
            pos: Pt::new(300.0, 420.0),
            text: "Annotation text\nПривет, мир · مرحبا · 你好".into(),
            color: C4::rgb(240, 68, 56),
            size: 28.0,
        });
        e.dirty = true;
        save(&mut app, "light-text-object.png");

        let mut app = preview_app(theme::DARK, None);
        save(&mut app, "dark-hint.png");

        let sel = FRect { x: 100.0, y: 700.0, w: 800.0, h: 180.0 };
        let mut app = preview_app_with(theme::DARK, Some(sel), two_monitor_shot());
        edit_of(&mut app).notice = Some(Toast::new("Copied", ToastKind::Success));
        save(&mut app, "dark-two-monitors.png");

        // Video mode (the Video and Recording boards): the setup panel
        // centred under a 960x420 selection, MP4 with the microphone's
        // level, GIF, the lists, without a camera, above and inside, the
        // grip hint under a bubble (drawn in), and the screenshot mode's
        // switch.
        let vsel = FRect { x: 240.0, y: 110.0, w: 960.0, h: 420.0 };
        let video = |th: Theme, sel: FRect| {
            let mut app = preview_app(th, Some(sel));
            let e = edit_of(&mut app);
            e.set_video(true);
            e.rec.opts.mic = true;
            e.rec.opts.camera = true;
            e.rec.meter = Some(rec_ui::meter::MicMeter::open(""));
            e.rec.bars = (5, false);
            app
        };
        // The bubble the overlay does not draw (its own window), drawn into
        // the preview where it would be.
        let with_bubble = |png: &mut PixBuf, th: &Theme, look: crate::cam::Look, at: (i32, i32)| {
            let t = crate::cam::template(look, 1.0, th);
            let mut px = Vec::new();
            crate::cam::compose(&t, None, false, &mut px);
            for y in 0..t.w as i32 {
                for x in 0..t.w as i32 {
                    let (dx, dy) = (at.0 - t.pad as i32 + x, at.1 - t.pad as i32 + y);
                    if dx < 0 || dy < 0 || dx >= png.width() as i32 || dy >= png.height() as i32 {
                        continue;
                    }
                    let s = &px[((y as u32 * t.w + x as u32) * 4) as usize..][..4];
                    let a = s[3] as u32;
                    let d = png.get_pixel(dx as u32, dy as u32);
                    let ch = |c: u8, b: u8| (c as u32 + b as u32 * (255 - a) / 255).min(255) as u8;
                    let i = ((dy as u32 * png.width() + dx as u32) * 4) as usize;
                    png.as_raw_mut()[i..i + 4].copy_from_slice(&[ch(s[2], d[0]), ch(s[1], d[1]), ch(s[0], d[2]), 255]);
                }
            }
        };
        for (tn, th) in [("dark", theme::DARK), ("light", theme::LIGHT)] {
            let mut app = video(th, vsel);
            // The bubble at the bottom-left corner (24 px in), its grip.
            let look = edit_of(&mut app).rec.opts.cam_look();
            let d = crate::cam::side(look.size, 1.0) as i32;
            let at = crate::cam::place(crate::cam::remembered(), (240, 110, 960, 420), d, 1.0);
            edit_of(&mut app).rec.grip = Some(FRect { x: at.0 as f32, y: at.1 as f32, w: d as f32, h: d as f32 });
            app.frame();
            let e = edit_of(&mut app);
            let tb = e.toolbar.clone().expect("panel");
            e.hover = tb.items.iter().position(|i| matches!(i.kind, toolbar::Kind::Primary { .. }));
            save(&mut app, &format!("{tn}-video-panel.png"));
            let path = dir.join(format!("{tn}-video-panel.png"));
            let mut png = PixBuf::from_png(&std::fs::read(&path).unwrap()).unwrap();
            with_bubble(&mut png, &th, look, at);
            std::fs::write(&path, png.to_png().unwrap()).unwrap();
            // Clipping: the top bars amber; the square bubble.
            let mut app = video(th, vsel);
            let e = edit_of(&mut app);
            e.rec.bars = (7, true);
            e.rec.opts.cam_shape = crate::cam::Shape::Rounded;
            e.rec.source = rec_ui::RecSource::Screen;
            save(&mut app, &format!("{tn}-video-clipping-square.png"));
            // GIF: the sound rows off, frame rate and max length.
            let mut app = video(th, vsel);
            let e = edit_of(&mut app);
            e.rec.opts.format = crate::rec::RecFormat::Gif;
            e.rec.meter = None;
            save(&mut app, &format!("{tn}-video-gif.png"));
            // The microphone list.
            let mut app = video(th, vsel);
            let e = edit_of(&mut app);
            e.rec.load_devs();
            e.rec.opts.mic_dev = "{mic-1}".into();
            e.rec.dev_open = Some(Dev::Mic);
            app.frame();
            let e = edit_of(&mut app);
            let tb = e.toolbar.clone().unwrap();
            e.hover = tb.items.iter().position(|i| matches!(i.kind, toolbar::Kind::DevItem { i: 2, .. }));
            save(&mut app, &format!("{tn}-video-mic-list.png"));
            // The camera ring's row and palette.
            let mut app = video(th, vsel);
            let e = edit_of(&mut app);
            e.rec.load_cams();
            e.rec.ring_pal = true;
            save(&mut app, &format!("{tn}-video-ring-palette.png"));
            // No room below: above the selection; none above either: inside.
            save(&mut video(th, FRect { x: 240.0, y: 520.0, w: 960.0, h: 340.0 }), &format!("{tn}-video-above.png"));
            save(&mut video(th, FRect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 }), &format!("{tn}-video-inside.png"));
            // At the right edge: it slides along it.
            save(&mut video(th, FRect { x: 1200.0, y: 200.0, w: 220.0, h: 200.0 }), &format!("{tn}-video-edge.png"));
            // Screenshot mode: the mode switch before the size label.
            let mut app = annotated(th, vsel);
            app.frame();
            save(&mut app, &format!("{tn}-screenshot-mode-switch.png"));
            // No camera on this machine: the row says so (and its tooltip).
            rec_ui::FAKE_CAMS.with(|c| c.borrow_mut().clear());
            let mut app = video(th, vsel);
            edit_of(&mut app).rec.load_cams();
            app.frame();
            let e = edit_of(&mut app);
            let tb = e.toolbar.clone().expect("toolbar");
            e.hover = tb.items.iter().position(|i| matches!(i.kind, toolbar::Kind::Switch { act: Act::RecCamera, .. }));
            e.hover_at = Instant::now() - Duration::from_secs(2);
            save(&mut app, &format!("{tn}-video-no-camera.png"));
            rec_ui::FAKE_CAMS.with(|c| *c.borrow_mut() = rec_ui::fake_cams());
            // The cameras still being listed.
            let mut app = video(th, vsel);
            let (_tx, rx) = std::sync::mpsc::channel();
            edit_of(&mut app).rec.cams_rx = Some(rx);
            save(&mut app, &format!("{tn}-video-looking-for-cameras.png"));
            // Linux without ffmpeg: GIF only, MP4 disabled with its tooltip.
            rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::No));
            let mut app = video(th, vsel);
            let e = edit_of(&mut app);
            e.rec = RecPop::new(e.rec.opts.clone(), false);
            e.set_video(true);
            app.frame();
            let e = edit_of(&mut app);
            let tb = e.toolbar.clone().expect("toolbar");
            e.hover = tb.items.iter().position(|i| matches!(i.kind, toolbar::Kind::Seg { act: Act::RecFormat(crate::rec::RecFormat::Mp4), .. }));
            e.hover_at = Instant::now() - Duration::from_secs(2);
            save(&mut app, &format!("{tn}-video-no-ffmpeg.png"));
            rec_ui::FAKE_MP4.with(|c| c.set(rec_ui::Mp4::Yes));
        }
    }

    fn two_mons() -> Vec<capture::IRect> {
        vec![(0, 0, 1000, 800), (1000, 0, 1000, 800)]
    }

    #[test]
    fn pick_area_follows_selection_overlap() {
        let m = two_mons();
        let sel = FRect { x: 900.0, y: 100.0, w: 400.0, h: 100.0 };
        let a = pick_area(&m, Some(sel), Pt::new(0.0, 0.0));
        assert_eq!((a.x, a.w), (1000.0, 1000.0));
        let split = FRect { x: 800.0, y: 100.0, w: 400.0, h: 100.0 };
        let a = pick_area(&m, Some(split), Pt::new(1500.0, 10.0));
        assert_eq!(a.x, 0.0, "tie goes to the first monitor");
    }

    #[test]
    fn pick_area_falls_back_to_pointer_then_first() {
        let m = vec![(0, 0, 1000, 800), (1100, 0, 1000, 800)];
        let a = pick_area(&m, None, Pt::new(1500.0, 10.0));
        assert_eq!(a.x, 1100.0);
        let a = pick_area(&m, None, Pt::new(1050.0, 10.0));
        assert_eq!(a.x, 0.0, "pointer in a gap");
    }

    #[test]
    fn toast_fades_in_and_out() {
        let t = Toast::new("x", ToastKind::Info);
        let at = t.at;
        let ms = |n: u64| at + Duration::from_millis(n);
        assert!(t.opacity(ms(0)) < 0.01);
        assert_eq!(t.opacity(ms(120)), 1.0);
        assert!((t.opacity(ms(1650)) - 0.5).abs() < 0.02);
        assert!(!t.expired(ms(1650)));
        assert!(t.expired(ms(1701)));
        assert!(t.animating(ms(10)) && !t.animating(ms(800)) && t.animating(ms(1500)));
    }

    #[test]
    fn square_constraint_keeps_quadrant() {
        let s = Pt::new(10.0, 10.0);
        let r = constrain_square(s, Pt::new(130.0, 50.0));
        assert_eq!((r.x, r.y), (130.0, 130.0));
        let r = constrain_square(s, Pt::new(-40.0, 5.0));
        assert_eq!((r.x, r.y), (-40.0, -40.0));
    }

    #[test]
    fn resize_from_se_corner() {
        let orig = FRect {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 50.0,
        };
        let mods = Mods::default();
        let r = resize_rect(Handle::SE, orig, Pt::new(210.0, 160.0), 2.0, &mods, (1000.0, 1000.0));
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 10.0);
        assert_eq!(r.w, 200.0);
        assert_eq!(r.h, 150.0);
    }

    #[test]
    fn resize_aspect_lock() {
        let orig = FRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let mods = Mods {
            ctrl: true,
            ..Default::default()
        };
        let r = resize_rect(Handle::SE, orig, Pt::new(200.0, 150.0), 2.0, &mods, (1000.0, 1000.0));
        // width 200, height forced to 100 (aspect 2), centered on y.
        assert_eq!(r.w, 200.0);
        assert!((r.h - 100.0).abs() < 0.01, "{}", r.h);
    }

    #[test]
    fn crop_roundtrip_preserves_opaque() {
        let img = PixBuf::from_pixel(4, 4, [10, 20, 30, 255]);
        let back = crop_to_image(
            &img,
            FRect {
                x: 0.0,
                y: 0.0,
                w: 4.0,
                h: 4.0,
            },
        );
        assert_eq!(back.get_pixel(0, 0), [10, 20, 30, 255]);
    }

    #[test]
    fn crop_clamps_to_bounds() {
        let pm = PixBuf::new(10, 10);
        let img = crop_to_image(
            &pm,
            FRect {
                x: -5.0,
                y: -5.0,
                w: 100.0,
                h: 100.0,
            },
        );
        assert_eq!(img.dimensions(), (10, 10));
    }

    #[test]
    fn caret_boundaries_stay_on_chars() {
        let s = "héllo";
        assert_eq!(next_boundary(s, 0), 1);
        assert_eq!(next_boundary(s, 1), 3);
        assert_eq!(prev_boundary(s, 3), 1);
        assert_eq!(prev_boundary(s, 0), 0);
        assert_eq!(next_boundary(s, s.len()), s.len());
    }

    #[test]
    fn text_box_pads_around_the_draft() {
        let td = TextDraft { pos: Pt::new(100.0, 50.0), text: String::new(), caret: 0, at: Instant::now() };
        let r = text_box_rect(1000.0, &td, 16.0, None, 1.0);
        assert_eq!((r.x, r.y), (96.0, 46.0));
        assert_eq!((r.w, r.h), (24.0, 28.0));
        let r = text_box_rect(110.0, &td, 16.0, None, 1.0);
        assert_eq!(r.x, 86.0, "kept on screen");
    }

    #[test]
    fn settled_editor_is_not_stale() {
        let mut app = preview_app(theme::DARK, Some(FRect { x: 10.0, y: 10.0, w: 200.0, h: 100.0 }));
        let e = edit_of(&mut app);
        for t in [&mut e.mo.dim, &mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint, &mut e.mo.hover] {
            t.snap(t.target());
        }
        // Taken after setup: a slow setup under a loaded test run must not
        // make `now` predate the editor's own timestamps.
        let now = Instant::now();
        assert!(!stale(e, None, now), "idle tick: no frame");
        e.hover = Some(0);
        e.hover_at = now;
        assert!(stale(e, None, now), "tooltip pending");
        e.hover_at = now - Duration::from_secs(1);
        assert!(!stale(e, None, now), "tooltip settled");
        let mut toast = Toast::new("x", ToastKind::Info);
        assert!(stale(e, Some(&toast), now), "toast fading in");
        toast.at = now - Duration::from_millis(500);
        e.toast_drawn = Some(toast.at);
        assert!(!stale(e, Some(&toast), now), "toast steady");
        e.toast_drawn = None;
        e.text = Some(TextDraft { pos: Pt::new(0.0, 0.0), text: String::new(), caret: 0, at: now });
        assert!(stale(e, None, now), "caret never drawn");
        e.caret_drawn = Some(true);
        assert!(!stale(e, None, now), "same blink phase");
        e.text.as_mut().unwrap().at = now - Duration::from_millis(600);
        assert!(stale(e, None, now), "blink phase flipped");
    }

    #[test]
    fn toast_set_or_cleared_between_frames_is_stale() {
        let now = Instant::now();
        let mut app = preview_app(theme::DARK, Some(FRect { x: 10.0, y: 10.0, w: 200.0, h: 100.0 }));
        let e = edit_of(&mut app);
        for t in [&mut e.mo.dim, &mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint, &mut e.mo.hover] {
            t.snap(t.target());
        }
        assert!(!stale(e, None, now), "settled");
        let mut toast = Toast::new("Uploaded", ToastKind::Success);
        toast.at = now - Duration::from_millis(500);
        assert!(stale(e, Some(&toast), now), "set between frames, age > 120 ms");
        e.toast_drawn = Some(toast.at);
        assert!(!stale(e, Some(&toast), now), "drawn");
        assert!(stale(e, None, now), "cleared between frames");
        e.toast_drawn = None;
        e.notice = Some(toast);
        assert!(stale(e, None, now), "edit toast set between frames");
        let mut newer = Toast::new("Failed", ToastKind::Error);
        newer.at = now - Duration::from_millis(300);
        e.toast_drawn = Some(newer.at);
        assert!(stale(e, None, now), "replaced by another toast");
    }

    /// The former per-pixel float dim, kept as the reference.
    fn dim_rect_ref(img: &mut PixBuf, x: f32, y: f32, w: f32, h: f32, c: C4) {
        if w <= 0.0 || h <= 0.0 || c.a == 0 {
            return;
        }
        let (bw, bh) = (img.width() as i32, img.height() as i32);
        let x0 = x.floor().max(0.0) as i32;
        let y0 = y.floor().max(0.0) as i32;
        let (x1, y1) = (x1_clamp(x, w, bw), y1_clamp(y, h, bh));
        let a = c.a as f32 / 255.0;
        let k = 1.0 - a;
        let data = img.as_raw_mut();
        for y in y0.max(0)..y1.min(bh) {
            for x in x0.max(0)..x1.min(bw) {
                let i = ((y * bw + x) as usize) * 4;
                for (ch, sv) in [c.r, c.g, c.b].iter().enumerate() {
                    data[i + ch] = (data[i + ch] as f32 * k + *sv as f32 * a).round() as u8;
                }
            }
        }
    }

    #[test]
    fn dimmer_matches_float_blend_exhaustively() {
        for a in 0..=255u8 {
            for cv in 0..=255u8 {
                let c = C4::new(cv, 255 - cv, cv / 2, a);
                let d = Dimmer::new(c);
                let (af, k) = (a as f32 / 255.0, 1.0 - a as f32 / 255.0);
                for v in 0..=255u8 {
                    let px = [v, v, v, 200, 255 - v, v, 255 - v, 9];
                    let mut out = [0u8; 8];
                    d.run(&px, &mut out, 1);
                    for (j, &sv) in [c.r, c.g, c.b, 0, c.r, c.g, c.b, 0].iter().enumerate() {
                        let want = if j % 4 == 3 { px[j] } else { (px[j] as f32 * k + sv as f32 * af).round() as u8 };
                        assert_eq!(out[j], want, "a {a} c {c:?} v {v} byte {j}");
                    }
                }
            }
        }
    }

    #[test]
    fn backdrop_matches_four_dim_rects() {
        use compose::{backdrop_rect, PxRect};
        let src = synthetic_shot().image.crop(100, 80, 301, 211);
        let (ww, wh) = (src.width() as f32, src.height() as f32);
        let size = src.dimensions();
        let c = C4::new(8, 10, 14, 141);
        let bd = PixBufBackdrop { img: &src, dim: c };
        let whole = PxRect::image(size);
        for sr in [
            FRect { x: 40.0, y: 30.0, w: 120.0, h: 90.0 },
            FRect { x: 40.4, y: 30.6, w: 120.3, h: 90.2 },
            FRect { x: -5.0, y: 0.0, w: 400.0, h: 0.5 },
            FRect { x: 300.5, y: 210.5, w: 3.0, h: 3.0 },
        ] {
            let rects = [
                (0.0, 0.0, ww, sr.y),
                (0.0, sr.y1(), ww, wh - sr.y1()),
                (0.0, sr.y, sr.x, sr.h),
                (sr.x1(), sr.y, ww - sr.x1(), sr.h),
            ];
            let mut want = src.clone();
            for r in rects {
                dim_rect_ref(&mut want, r.0, r.1, r.2, r.3, c);
            }
            let mut got = PixBuf::from_pixel(src.width(), src.height(), [1, 2, 3, 4]);
            backdrop_rect(Some(sr), size, c.a, whole, &bd, got.as_raw_mut(), Order::Rgba);
            assert!(got == want, "{sr:?}");
        }
        let mut got = PixBuf::new(src.width(), src.height());
        backdrop_rect(None, size, 0, whole, &bd, got.as_raw_mut(), Order::Rgba);
        assert!(got == src, "alpha 0 copies");
    }

    // ---- rect composition: tilings and dirty rects ----

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    pub(super) fn snap_all(app: &mut App) {
        let e = edit_of(app);
        for t in [&mut e.mo.dim, &mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint, &mut e.mo.hover] {
            t.snap(t.target());
        }
    }

    /// Settle every tween (some start in `frame()`), then paint.
    pub(super) fn settled(app: &mut App) -> PixBuf {
        snap_all(app);
        app.frame();
        snap_all(app);
        app.frame().expect("frame").clone()
    }

    /// A tween stuck at about a third of the way to `to`.
    fn mid(to: f32) -> Tween {
        let now = Instant::now();
        let mut t = Tween::new(0.0, now);
        t.set(to, 1_000_000, now - Duration::from_secs(60));
        t
    }

    /// Compose the prepared scene through random tiles (both orders) and
    /// compare with the whole-image frame `want`.
    pub(super) fn assert_tiles_match(app: &mut App, want: &PixBuf, name: &str, seed: u64) {
        use compose::PxRect;
        let e = edit_of(app);
        let bd = PixBufBackdrop { img: e.composed(), dim: e.th.dim };
        let (w, h) = (e.shot.size.0 as i32, e.shot.size.1 as i32);
        let mut rng = Rng(seed | 1);
        let mut n = 0;
        let mut y = 0;
        while y < h {
            let cap = if rng.below(4) == 0 { 7 } else { 150 };
            let th = 1 + rng.below(cap) as i32;
            let mut x = 0;
            while x < w {
                let cap = if rng.below(4) == 0 { 9 } else { 260 };
                let tw = 1 + rng.below(cap) as i32;
                let r = PxRect::new(x, y, (x + tw).min(w), (y + th).min(h));
                let order = if n % 2 == 0 { Order::Bgra } else { Order::Rgba };
                n += 1;
                let mut out = vec![7u8; (r.w() * r.h() * 4) as usize];
                e.compose_rect(r, &bd, &mut out, order);
                for ty in 0..r.h() {
                    for tx in 0..r.w() {
                        let i = ((ty * r.w() + tx) * 4) as usize;
                        let mut got = [out[i], out[i + 1], out[i + 2], out[i + 3]];
                        if order == Order::Bgra {
                            got.swap(0, 2);
                        }
                        let exp = want.get_pixel((r.x0 + tx) as u32, (r.y0 + ty) as u32);
                        assert_eq!(got, exp, "{name}: pixel ({}, {}) in tile {r:?} {order:?}", r.x0 + tx, r.y0 + ty);
                    }
                }
                x += tw;
            }
            y += th;
        }
    }

    fn tiling_states() -> Vec<(&'static str, App)> {
        let big = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
        let mut v = Vec::new();
        v.push(("A hint", preview_app(theme::DARK, None)));
        v.push(("B toolbar", annotated(theme::DARK, big)));
        let mut app = annotated(theme::DARK, big);
        edit_of(&mut app).interact = Interact::NewSel { anchor: Pt::new(big.x, big.y), moved: true };
        v.push(("C dragging", app));
        let mut app = annotated(theme::DARK, big);
        edit_of(&mut app).palette_open = true;
        v.push(("D palette", app));
        let mut app = annotated(theme::LIGHT, big);
        app.frame();
        let e = edit_of(&mut app);
        let copy = e.toolbar.as_ref().unwrap().items.iter().position(|i| i.kind == toolbar::Kind::Btn(Act::Copy));
        e.hover = copy;
        e.hover_at = Instant::now() - Duration::from_secs(2);
        let mut toast = Toast::new("Size 4", ToastKind::Info);
        toast.at = Instant::now() - Duration::from_millis(500);
        e.notice = Some(toast);
        v.push(("light tooltip toast", app));
        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        e.tool = Some(Tool::Text);
        let text = "Check this".to_string();
        e.text = Some(TextDraft { pos: Pt::new(400.0, 300.0), caret: 5, text, at: Instant::now() });
        v.push(("text", app));
        let sel = FRect { x: 100.0, y: 700.0, w: 800.0, h: 180.0 };
        let mut app = preview_app_with(theme::DARK, Some(sel), two_monitor_shot());
        let mut toast = Toast::new("Uploaded https://example.com/a-rather-long-url-that-runs-on", ToastKind::Success);
        toast.at = Instant::now() - Duration::from_millis(500);
        app.notice = Some(toast);
        v.push(("two monitors", app));
        v.push(("narrow", annotated(theme::DARK, FRect { x: 500.0, y: 300.0, w: 380.0, h: 200.0 })));
        v.push(("fractional", annotated(theme::DARK, FRect { x: 240.4, y: 140.6, w: 960.3, h: 540.2 })));
        // Drafts straddling the selection edge; pixelate needs its whole rect.
        for (name, d) in [
            ("draft pixelate", Obj::Pixelate { r: FRect { x: 180.0, y: 100.0, w: 300.5, h: 210.0 }, cell: 13.0 }),
            ("draft invert", Obj::Invert { r: FRect { x: 1100.0, y: 600.0, w: 200.0, h: 150.0 } }),
            ("draft arrow", Obj::Arrow { a: Pt::new(150.0, 120.0), b: Pt::new(700.0, 420.0), color: C4::rgb(20, 200, 90), width: 9.0 }),
            ("draft arrow translucent", Obj::Arrow { a: Pt::new(150.0, 620.0), b: Pt::new(700.0, 420.0), color: C4::new(20, 200, 90, 128), width: 12.0 }),
            ("draft marker", Obj::Marker { a: Pt::new(200.0, 600.0), b: Pt::new(900.0, 640.0), color: C4::rgb(250, 220, 0).with_alpha(90), width: 20.0 }),
            ("draft marker path", Obj::MarkerPath { pts: vec![Pt::new(200.0, 600.0), Pt::new(420.0, 660.0), Pt::new(600.0, 590.0), Pt::new(900.0, 640.0)], color: C4::rgb(250, 220, 0).with_alpha(90), width: 20.0 }),
            ("draft rect fill translucent", Obj::RectFill { r: FRect { x: 180.0, y: 300.0, w: 420.5, h: 160.0 }, color: C4::new(59, 130, 246, 110), width: 4.0 }),
            ("draft ellipse fill", Obj::EllipseFill { r: FRect { x: 1000.0, y: 500.0, w: 300.0, h: 180.0 }, color: C4::rgb(240, 68, 56), width: 3.0 }),
            ("draft counter tail", Obj::Counter { c: Pt::new(260.5, 160.0), n: 7, color: C4::new(240, 68, 56, 200), d: 40.0, tail: Some(Pt::new(120.0, 90.0)) }),
        ] {
            let mut app = annotated(theme::DARK, big);
            let e = edit_of(&mut app);
            e.interact = Interact::Drawing { start: Pt::new(0.0, 0.0) };
            e.draft = Some(d);
            v.push((name, app));
        }
        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        e.objects.push(Obj::Pixelate { r: FRect { x: 200.0, y: 400.0, w: 333.0, h: 120.0 }, cell: 11.0 });
        e.hover = Some(3);
        e.pressed = Some(4);
        e.hot_handle = Some(2);
        v.push(("objects hover pressed", app));
        // The color picker (recents, a translucent color, a focus), the
        // eyedropper's loupe, the camera ring's palette.
        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        e.cfg.recent_colors = vec!["#3b82f6cc".into(), "#2dc06f".into()];
        e.open_picker(pick::Target::Tool);
        let p = e.pick.as_mut().unwrap();
        p.set_color(C4::new(139, 92, 246, 150));
        p.focus = Some(pick::Part::Alpha);
        v.push(("picker", app));
        let mut app = annotated(theme::LIGHT, big);
        let e = edit_of(&mut app);
        e.open_picker(pick::Target::Tool);
        e.pick.as_mut().unwrap().eyedrop = true;
        e.loupe = Some(Pt::new(690.0, 548.0));
        v.push(("loupe", app));
        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        e.rec.open = true;
        e.rec.load_cams();
        e.rec.opts.camera = true;
        e.rec.ring_pal = true;
        v.push(("ring palette", app));
        // Video mode: the panel (the meter clipping, the grip hint, a
        // hovered row), GIF in light, the microphone list, the panel above
        // a low selection (the estimate toast moved off it).
        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        e.set_video(true);
        e.rec.opts.mic = true;
        e.rec.opts.camera = true;
        e.rec.meter = Some(rec_ui::meter::MicMeter::open(""));
        e.rec.bars = (7, true);
        e.rec.grip = Some(FRect { x: 264.0, y: 496.0, w: 160.0, h: 160.0 });
        e.hover = Some(12);
        e.hover_at = Instant::now() - Duration::from_secs(2);
        v.push(("video panel", app));
        let mut app = annotated(theme::LIGHT, big);
        let e = edit_of(&mut app);
        e.set_video(true);
        e.rec.opts.format = crate::rec::RecFormat::Gif;
        v.push(("video gif", app));
        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        e.set_video(true);
        e.rec.opts.mic = true;
        e.rec.load_devs();
        e.rec.dev_open = Some(Dev::Mic);
        v.push(("video mic list", app));
        let mut app = annotated(theme::LIGHT, FRect { x: 240.0, y: 600.0, w: 960.0, h: 260.0 });
        edit_of(&mut app).set_video(true);
        v.push(("video above", app));
        // Numbered tags (one across the selection edge), a filled shape,
        // the number tool's bar and the fill toggle on.
        let mut app = annotated(theme::DARK, big);
        let e = edit_of(&mut app);
        for (i, (x, y)) in [(300.0, 200.0), (245.0, 500.0), (900.0, 650.0)].into_iter().enumerate() {
            e.objects.push(Obj::Counter { c: Pt::new(x, y), n: i as u32 + 1, color: C4::rgb(59, 130, 246), d: 36.0, tail: None });
        }
        e.objects.push(Obj::Counter { c: Pt::new(700.0, 300.0), n: 12, color: C4::rgb(250, 220, 0), d: 30.0, tail: Some(Pt::new(780.0, 380.0)) });
        e.objects.push(Obj::RectFill { r: FRect { x: 500.0, y: 450.0, w: 300.0, h: 80.0 }, color: C4::new(250, 220, 0, 120), width: 3.0 });
        e.tool = Some(Tool::Counter);
        v.push(("counters", app));
        let mut app = annotated(theme::LIGHT, big);
        let e = edit_of(&mut app);
        e.tool = Some(Tool::Ellipse);
        e.mem.toggle_fill(Tool::Ellipse);
        v.push(("fill on", app));
        v
    }

    #[test]
    fn random_tilings_match_whole_frame() {
        for (i, (name, mut app)) in tiling_states().into_iter().enumerate() {
            let want = settled(&mut app);
            assert_tiles_match(&mut app, &want, name, 0x9E37_79B9 + i as u64);
        }
    }

    /// Mid-animation: dim, toolbar scale/fade, palette, hint, hover and
    /// toast at fractional opacities.
    #[test]
    fn random_tilings_match_mid_animation() {
        let big = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
        let mut app = annotated(theme::DARK, big);
        app.frame();
        let e = edit_of(&mut app);
        e.palette_open = true;
        e.hover = Some(1);
        (e.mo.dim, e.mo.bar, e.mo.pop, e.mo.hover) = (mid(1.0), mid(1.0), mid(1.0), mid(1.0));
        let mut toast = Toast::new("Halfway", ToastKind::Error);
        toast.at = Instant::now() - Duration::from_millis(30);
        e.notice = Some(toast);
        let want = app.frame().expect("frame").clone();
        assert_tiles_match(&mut app, &want, "mid toolbar", 77);

        let mut app = preview_app(theme::LIGHT, None);
        app.frame();
        let e = edit_of(&mut app);
        (e.mo.dim, e.mo.hint) = (mid(1.0), mid(1.0));
        let want = app.frame().expect("frame").clone();
        assert_tiles_match(&mut app, &want, "mid hint", 78);
    }

    /// Paint, apply `change`, paint again: every pixel that differs lies in
    /// `dirty_rects(before, after)`. Returns the dirty area fraction.
    pub(super) fn assert_dirty_covers(app: &mut App, name: &str, change: impl FnOnce(&mut App)) -> f64 {
        let before = app.frame().expect("frame").clone();
        let prev = edit_of(app).scene.clone().expect("scene");
        change(app);
        let after = app.frame().expect("frame").clone();
        let e = edit_of(app);
        let rects = e.dirty_rects(Some(&prev));
        let (w, h) = before.dimensions();
        let (a, b) = (before.as_raw(), after.as_raw());
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = (y as usize * w as usize + x as usize) * 4;
                if a[i..i + 4] != b[i..i + 4] {
                    assert!(
                        rects.iter().any(|r| r.contains_px(x, y)),
                        "{name}: pixel ({x}, {y}) changed outside {rects:?}"
                    );
                }
            }
        }
        let area: i64 = rects.iter().map(|r| r.w() as i64 * r.h() as i64).sum();
        area as f64 / (w as f64 * h as f64)
    }

    /// A pixelate drag repaints only the cells along the moving edges, and
    /// still covers every changed pixel (grow, shrink, fractional edges,
    /// past the image edge, the anchor moving, cell size changing).
    #[test]
    fn dirty_rects_cover_pixelate_drag() {
        let big = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
        let mut app = annotated(theme::DARK, big);
        settled(&mut app);
        let set = |app: &mut App, r: FRect, cell: f32| {
            let e = edit_of(app);
            e.interact = Interact::Drawing { start: Pt::new(r.x, r.y) };
            e.draft = Some(Obj::Pixelate { r, cell });
        };
        let r0 = FRect { x: 180.0, y: 100.0, w: 700.0, h: 500.0 };
        assert_dirty_covers(&mut app, "start", |a| set(a, r0, 12.0));
        let f = assert_dirty_covers(&mut app, "grow", |a| set(a, FRect { w: 706.0, h: 503.0, ..r0 }, 12.0));
        assert!(f < 0.1, "grow repaints the edges only: {f}");
        let f = assert_dirty_covers(&mut app, "shrink", |a| set(a, FRect { w: 650.5, h: 470.25, ..r0 }, 12.0));
        assert!(f < 0.1, "shrink repaints the edges only: {f}");
        assert_dirty_covers(&mut app, "fractional", |a| set(a, FRect { x: 180.4, y: 100.6, w: 651.0, h: 471.0 }, 12.0));
        assert_dirty_covers(&mut app, "past the edge", |a| set(a, FRect { x: 180.0, y: 100.0, w: 1400.0, h: 900.0 }, 12.0));
        assert_dirty_covers(&mut app, "back", |a| set(a, FRect { w: 1300.0, h: 820.0, ..r0 }, 12.0));
        assert_dirty_covers(&mut app, "anchor moves", |a| set(a, FRect { x: 150.0, y: 90.0, w: 730.0, h: 510.0 }, 12.0));
        assert_dirty_covers(&mut app, "cell changes", |a| set(a, FRect { x: 150.0, y: 90.0, w: 730.0, h: 510.0 }, 13.0));
        let mut neg = annotated(theme::DARK, big);
        settled(&mut neg);
        assert_dirty_covers(&mut neg, "offscreen start", |a| set(a, FRect { x: -30.5, y: -20.0, w: 300.0, h: 200.0 }, 9.0));
        assert_dirty_covers(&mut neg, "offscreen grow", |a| set(a, FRect { x: -30.5, y: -20.0, w: 340.0, h: 230.0 }, 9.0));
    }

    /// Selection resizes repaint the moving edges and the ends of the
    /// strips that stay, not the whole outline, and still cover every
    /// changed pixel (each handle, both directions, tiny selections, the
    /// image edge, a hot handle).
    #[test]
    fn dirty_rects_cover_selection_resizes() {
        let big = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
        let mut app = annotated(theme::DARK, big);
        settled(&mut app);
        let set = |r: FRect| {
            move |app: &mut App| {
                let e = edit_of(app);
                e.interact = Interact::Resize { handle: Handle::SE, orig: big, aspect: 1.0 };
                e.sel = Some(r);
            }
        };
        assert_dirty_covers(&mut app, "start", set(big));
        let se = FRect { w: big.w + 3.0, h: big.h + 1.0, ..big };
        assert_dirty_covers(&mut app, "se", set(se));
        // The middle of the top and left edges stays.
        let rects = {
            let prev = edit_of(&mut app).scene.clone().unwrap();
            set(FRect { w: se.w + 3.0, h: se.h + 1.0, ..se })(&mut app);
            let _ = app.frame();
            edit_of(&mut app).dirty_rects(Some(&prev))
        };
        for (x, y) in [(big.x + big.w * 0.3, big.y), (big.x, big.y + big.h * 0.3)] {
            assert!(!rects.iter().any(|r| r.contains_px(x as i32, y as i32)), "({x}, {y}) repainted: {rects:?}");
        }
        let steps = [
            ("nw", FRect { x: big.x - 4.0, y: big.y - 2.0, w: big.w + 4.0, h: big.h + 2.0 }),
            ("nw back", FRect { x: big.x + 5.0, y: big.y + 3.0, w: big.w - 5.0, h: big.h - 3.0 }),
            ("e", FRect { x: big.x + 5.0, y: big.y + 3.0, w: big.w + 40.0, h: big.h - 3.0 }),
            ("s", FRect { x: big.x + 5.0, y: big.y + 3.0, w: big.w + 40.0, h: big.h - 60.0 }),
            ("w", FRect { x: big.x - 30.0, y: big.y + 3.0, w: big.w + 75.0, h: big.h - 60.0 }),
            ("n", FRect { x: big.x - 30.0, y: big.y - 50.0, w: big.w + 75.0, h: big.h - 7.0 }),
            ("thin", FRect { x: 400.0, y: 300.0, w: 3.0, h: 200.0 }),
            ("tiny", FRect { x: 400.0, y: 300.0, w: 3.0, h: 2.0 }),
            ("tiny grows", FRect { x: 400.0, y: 300.0, w: 9.0, h: 7.0 }),
            ("flat", FRect { x: 400.0, y: 300.0, w: 300.0, h: 4.0 }),
            ("to the edge", FRect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 }),
            ("off the edge", FRect { x: 0.0, y: 0.0, w: 1200.0, h: 900.0 }),
            ("back", se),
        ];
        for (name, r) in steps {
            assert_dirty_covers(&mut app, name, set(r));
        }
        assert_dirty_covers(&mut app, "hot", |app| edit_of(app).hot_handle = Some(4));
        assert_dirty_covers(&mut app, "hot resize", set(FRect { w: se.w + 7.0, ..se }));
        assert_dirty_covers(&mut app, "cold resize", |app| {
            set(FRect { w: se.w + 9.0, ..se })(app);
            edit_of(app).hot_handle = None;
        });
    }

    #[test]
    fn dirty_rects_cover_every_changed_pixel() {
        let big = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
        let mut app = annotated(theme::DARK, big);
        settled(&mut app);
        let small = |f: f64, name: &str| assert!(f < 0.5, "{name}: dirty area {f}");

        let f = assert_dirty_covers(&mut app, "drag", |app| {
            let e = edit_of(app);
            e.interact = Interact::MoveSel { start: Pt::new(0.0, 0.0), orig: big };
            e.sel = Some(FRect { x: big.x + 13.0, y: big.y + 7.0, ..big });
        });
        small(f, "drag");
        let f = assert_dirty_covers(&mut app, "resize", |app| {
            let e = edit_of(app);
            e.sel = Some(FRect { x: big.x + 13.0, y: big.y + 7.0, w: big.w + 20.0, h: big.h - 5.0 });
        });
        small(f, "resize");
        assert_dirty_covers(&mut app, "drop", |app| {
            let e = edit_of(app);
            e.interact = Interact::None;
            e.sel = Some(big);
        });
        snap_all(&mut app);
        let f = assert_dirty_covers(&mut app, "hover", |app| {
            let e = edit_of(app);
            e.hover = Some(2);
            e.hover_at = Instant::now();
            e.mo.hover = mid(1.0);
        });
        small(f, "hover");
        let f = assert_dirty_covers(&mut app, "hover settles", snap_all);
        small(f, "hover settles");
        let f = assert_dirty_covers(&mut app, "tooltip appears", |app| {
            edit_of(app).hover_at = Instant::now() - Duration::from_secs(2);
        });
        small(f, "tooltip");
        let f = assert_dirty_covers(&mut app, "hover moves", |app| edit_of(app).hover = Some(5));
        small(f, "hover moves");
        assert_dirty_covers(&mut app, "hover leaves", |app| edit_of(app).hover = None);
        assert_dirty_covers(&mut app, "hot handle", |app| edit_of(app).hot_handle = Some(4));
        assert_dirty_covers(&mut app, "pressed", |app| edit_of(app).pressed = Some(6));
        let f = assert_dirty_covers(&mut app, "palette opens", |app| edit_of(app).palette_open = true);
        small(f, "palette");
        assert_dirty_covers(&mut app, "palette fades in", snap_all);
        assert_dirty_covers(&mut app, "palette closes", |app| edit_of(app).palette_open = false);
        let f = assert_dirty_covers(&mut app, "toast set", |app| {
            edit_of(app).notice = Some(Toast::new("Size 5", ToastKind::Info));
        });
        small(f, "toast");
        assert_dirty_covers(&mut app, "toast steady", |app| {
            edit_of(app).notice.as_mut().unwrap().at = Instant::now() - Duration::from_millis(500);
        });
        assert_dirty_covers(&mut app, "toast fade frame", |app| {
            edit_of(app).notice.as_mut().unwrap().at = Instant::now() - Duration::from_millis(1650);
        });
        assert_dirty_covers(&mut app, "toast cleared", |app| edit_of(app).notice = None);
        assert_dirty_covers(&mut app, "app toast", |app| {
            let mut t = Toast::new("Uploaded https://example.com/x", ToastKind::Success);
            t.at = Instant::now() - Duration::from_millis(500);
            app.notice = Some(t);
        });
        assert_dirty_covers(&mut app, "app toast cleared", |app| app.notice = None);
        let f = assert_dirty_covers(&mut app, "tool switch", |app| edit_of(app).tool = Some(Tool::Rect));
        small(f, "tool");
        let f = assert_dirty_covers(&mut app, "draft", |app| {
            let e = edit_of(app);
            e.interact = Interact::Drawing { start: Pt::new(300.0, 200.0) };
            e.draft = e.make_draft(Tool::Rect, Pt::new(300.0, 200.0), Pt::new(420.0, 260.0));
        });
        small(f, "draft");
        assert_dirty_covers(&mut app, "draft grows", |app| {
            let e = edit_of(app);
            e.draft = e.make_draft(Tool::Rect, Pt::new(300.0, 200.0), Pt::new(520.0, 330.0));
        });
        let f = assert_dirty_covers(&mut app, "commit", |app| end_interaction(edit_of(app)));
        small(f, "commit");
        assert_dirty_covers(&mut app, "pixelate commit", |app| {
            edit_of(app).commit_object(Obj::Pixelate { r: FRect { x: 150.0, y: 100.0, w: 260.0, h: 200.0 }, cell: 12.0 });
        });
        let f = assert_dirty_covers(&mut app, "undo", |app| edit_of(app).undo());
        small(f, "undo");
        assert_dirty_covers(&mut app, "undo again", |app| edit_of(app).undo());
        assert_dirty_covers(&mut app, "redo", |app| edit_of(app).redo());
        assert_dirty_covers(&mut app, "text draft", |app| {
            let e = edit_of(app);
            e.tool = Some(Tool::Text);
            e.text = Some(TextDraft { pos: Pt::new(500.0, 400.0), text: "Hi".into(), caret: 2, at: Instant::now() });
        });
        let f = assert_dirty_covers(&mut app, "caret blink", |app| {
            edit_of(app).text.as_mut().unwrap().at = Instant::now() - Duration::from_millis(600);
        });
        small(f, "caret");
        assert_dirty_covers(&mut app, "typing", |app| {
            let td = edit_of(app).text.as_mut().unwrap();
            td.text.push_str(" there");
            td.caret = td.text.len();
            td.at = Instant::now();
        });
        assert_dirty_covers(&mut app, "fractional sel", |app| {
            edit_of(app).sel = Some(FRect { x: 240.5, y: 140.0, w: 960.0, h: 540.0 });
        });

        // No selection yet: hint, then the first selection appears.
        let mut app = preview_app(theme::DARK, None);
        settled(&mut app);
        assert_dirty_covers(&mut app, "first selection", |app| {
            let e = edit_of(app);
            e.interact = Interact::NewSel { anchor: Pt::new(100.0, 100.0), moved: true };
            e.sel = Some(FRect { x: 100.0, y: 100.0, w: 50.0, h: 40.0 });
        });
        assert_dirty_covers(&mut app, "hint fades", snap_all);
        let f = assert_dirty_covers(&mut app, "sel grows", |app| {
            edit_of(app).sel = Some(FRect { x: 100.0, y: 100.0, w: 90.0, h: 70.0 });
        });
        small(f, "sel grows");
        assert_dirty_covers(&mut app, "dim fade", |app| edit_of(app).mo.dim = mid(1.0));
    }

    /// Video mode: on and off, a source, the meter's bars, a row hovered,
    /// the format, the list, the grip hint: every changed pixel lies in
    /// the dirty rects, and a meter tick repaints only a little.
    #[test]
    fn dirty_rects_cover_video_mode() {
        let big = FRect { x: 240.0, y: 140.0, w: 960.0, h: 540.0 };
        let mut app = annotated(theme::DARK, big);
        settled(&mut app);
        assert_dirty_covers(&mut app, "video on", |a| edit_of(a).set_video(true));
        snap_all(&mut app);
        assert_dirty_covers(&mut app, "settled", snap_all);
        assert_dirty_covers(&mut app, "mic on", |a| {
            let e = edit_of(a);
            e.rec.opts.mic = true;
            e.rec.meter = Some(rec_ui::meter::MicMeter::open(""));
        });
        let f = assert_dirty_covers(&mut app, "meter tick", |a| edit_of(a).rec.bars = (5, false));
        assert!(f < 0.25, "a meter tick repaints the panel, not the screen: {f}");
        assert_dirty_covers(&mut app, "clipping", |a| edit_of(a).rec.bars = (7, true));
        assert_dirty_covers(&mut app, "row hover", |a| {
            let e = edit_of(a);
            e.hover = Some(14);
            e.hover_at = Instant::now() - Duration::from_secs(2);
        });
        assert_dirty_covers(&mut app, "source", |a| edit_of(a).rec.source = rec_ui::RecSource::Screen);
        assert_dirty_covers(&mut app, "gif", |a| edit_of(a).rec.opts.format = crate::rec::RecFormat::Gif);
        assert_dirty_covers(&mut app, "list", |a| {
            let e = edit_of(a);
            e.rec.opts.format = crate::rec::RecFormat::Mp4;
            e.rec.load_devs();
            e.rec.dev_open = Some(Dev::Mic);
        });
        assert_dirty_covers(&mut app, "grip", |a| edit_of(a).rec.grip = Some(FRect { x: 264.0, y: 496.0, w: 160.0, h: 160.0 }));
        assert_dirty_covers(&mut app, "grip moves", |a| edit_of(a).rec.grip = Some(FRect { x: 1016.0, y: 496.0, w: 160.0, h: 160.0 }));
        assert_dirty_covers(&mut app, "selection moves", |a| edit_of(a).sel = Some(FRect { x: 250.0, y: 150.0, ..big }));
        assert_dirty_covers(&mut app, "video off", |a| edit_of(a).set_video(false));
    }

    /// The estimate toast never sits on the panel: it moves to the top of
    /// the monitor when the panel is at the bottom.
    /// Video mode needs recording (`rec_ui::supported`: not on macOS in tests).
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn estimate_toast_keeps_off_the_panel() {
        for sel in [FRect { x: 240.0, y: 110.0, w: 960.0, h: 420.0 }, FRect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 }, FRect { x: 300.0, y: 200.0, w: 400.0, h: 200.0 }] {
            let mut app = preview_app(theme::DARK, Some(sel));
            edit_of(&mut app).set_video(true);
            settled(&mut app);
            let e = edit_of(&mut app);
            let sc = e.scene.as_ref().unwrap();
            let t = sc.toast.as_ref().expect("the estimate");
            let ui = chrome::Ui { th: &sc.th, s: sc.s, font: e.ui_font };
            let r = chrome::toast_panel(&ui, &t.text, t.area, 1.0);
            let p = e.toolbar.as_ref().unwrap().bar;
            assert!(!(r.x < p.x1() && p.x < r.x1() && r.y < p.y1() && p.y < r.y1()), "{sel:?}: toast {r:?} on the panel {p:?}");
        }
    }

    /// Every pixel the chrome and draft change over the bare backdrop lies
    /// inside `chrome_rects`.
    #[test]
    fn chrome_rects_bound_the_chrome() {
        for (name, mut app) in tiling_states() {
            let want = settled(&mut app);
            let e = edit_of(&mut app);
            let sc = e.scene.as_ref().unwrap();
            let size = sc.size;
            let mut bg = PixBuf::new(size.0, size.1);
            let bd = PixBufBackdrop { img: e.composed(), dim: e.th.dim };
            compose::backdrop_rect(sc.sel, size, sc.dim_alpha, compose::PxRect::image(size), &bd, bg.as_raw_mut(), Order::Rgba);
            let rects = e.chrome_rects();
            for y in 0..size.1 {
                for x in 0..size.0 {
                    if bg.get_pixel(x, y) != want.get_pixel(x, y) {
                        assert!(rects.iter().any(|r| r.contains_px(x as i32, y as i32)), "{name}: ({x}, {y})");
                    }
                }
            }
        }
    }

    /// Release-mode frame cost at 5120x1440 (states A-D of the perf report).
    /// `cargo test --release perf_frame_bench -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark; run in release"]
    fn perf_frame_bench() {
        let (w, h) = (5120u32, 1440u32);
        let src = synthetic_shot();
        let mut img = PixBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = (y as usize * w as usize + x as usize) * 4;
                let j = ((y % src.size.1) as usize * src.size.0 as usize + (x % src.size.0) as usize) * 4;
                img.as_raw_mut()[i..i + 4].copy_from_slice(&src.image.as_raw()[j..j + 4]);
            }
        }
        let shot = Shot { origin: (0, 0), size: (w, h), scale: 1.0, image: img, monitors: vec![(0, 0, w, h)] };
        let sel = FRect { x: 1000.0, y: 200.0, w: 2000.0, h: 900.0 };
        let median = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        for (name, s, drag, pal) in [
            ("A no sel, hint", None, false, false),
            ("B sel 2000x900 + toolbar", Some(sel), false, false),
            ("C dragging NewSel", Some(sel), true, false),
            ("D palette open", Some(sel), false, true),
        ] {
            let mut app = preview_app_with(theme::DARK, s, Shot { image: shot.image.clone(), monitors: shot.monitors.clone(), ..shot });
            {
                let e = edit_of(&mut app);
                e.palette_open = pal;
                if drag {
                    e.interact = Interact::NewSel { anchor: Pt::new(sel.x, sel.y), moved: true };
                }
            }
            for _ in 0..2 {
                let _ = app.frame();
                let e = edit_of(&mut app);
                for t in [&mut e.mo.dim, &mut e.mo.bar, &mut e.mo.pop, &mut e.mo.hint, &mut e.mo.hover] {
                    t.snap(t.target());
                }
            }
            let (mut tf, mut tp) = (Vec::new(), Vec::new());
            for _ in 0..30 {
                let t0 = Instant::now();
                let fb = app.frame().expect("frame");
                let t1 = Instant::now();
                bench_present_prep(fb);
                let t2 = Instant::now();
                std::hint::black_box(&*fb);
                tf.push((t1 - t0).as_secs_f64() * 1000.0);
                tp.push((t2 - t1).as_secs_f64() * 1000.0);
            }
            let (f, p) = (median(tf), median(tp));
            println!("BENCH {name}: frame {f:.2} ms, present prep {p:.2} ms, sum {:.2} ms", f + p);
        }
    }

    /// What `wind_win::present` does before StretchDIBits.
    fn bench_present_prep(fb: &mut PixBuf) {
        #[cfg(windows)]
        crate::wind::swap_rb(fb.as_raw_mut());
        #[cfg(not(windows))]
        let _ = fb;
    }

    #[cfg(windows)]
    #[path = "../gdi_tests.rs"]
    mod gdi_path;
}
