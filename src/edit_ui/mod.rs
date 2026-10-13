//! The Edit window (boards 11: Edit, EditLight, EditStates): a capture
//! from the history opened again for editing, its objects editable as in
//! the overlay (the same editor, hosted: [`crate::editor::host::Canvas`]).
//! The overlay's toolbar is docked at the top (plus Save As and Save to
//! Desktop) with the zoom control on the right (−, the zoom, +, Fit, 1:1),
//! the objects list on the left (top = front-most; drag to reorder, Del
//! removes; collapses to a 44 px rail of icons), the picture centred on
//! the canvas at Fit, and a 28 px status bar ("1180 × 760 · 72% · Edited ·
//! unsaved changes", the selection's geometry, the pointer). Ctrl+wheel
//! zooms about the pointer, Space-drag and middle-drag pan, the wheel
//! scrolls (Shift: sideways, Alt: the size).
//!
//! Saving ([`Flow`]): Save asks "Overwrite the original" (the saved file;
//! the History entry stays) or "Save as new file" (`<name>-edited.png`),
//! with "Don't ask again" (`edit_save`); closing with unsaved changes asks
//! first (Discard / Cancel / Save as copy / Save). Either way the entry is
//! marked Edited with its new `annotations.json`; `original.png` is never
//! written, so a capture can always be edited again.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use crate::config::Config;
use crate::editor::host::{Canvas, Request};
use crate::history::{Entry, Store};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub mod draw;
pub mod image;
mod window;

/// Default and minimum size, logical px.
pub const W: u32 = 1200;
pub const H: u32 = 800;
pub const MIN: (u32, u32) = (960, 600);

// --- the save flow --------------------------------------------------------------

/// Where Save writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    /// Over the saved file (the History entry stays the same).
    Overwrite,
    /// Next to it as `<name>-edited.<ext>`.
    New,
}

impl Choice {
    /// `edit_save` in the config.
    pub fn from_config(v: &str) -> Option<Choice> {
        match v.trim() {
            "overwrite" => Some(Choice::Overwrite),
            "new" => Some(Choice::New),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Choice::Overwrite => "overwrite",
            Choice::New => "new",
        }
    }
}

/// A question of the save flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dialog {
    /// "Save changes to this capture?": Discard, Cancel, Save as copy, Save.
    Unsaved,
    /// "Save": Overwrite the original or Save as new file, Don't ask again;
    /// `close`: closing waits for it.
    Choose { choice: Choice, dont_ask: bool, close: bool },
}

/// A press in a question.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Btn {
    Discard,
    Cancel,
    SaveCopy,
    Save,
    Pick(Choice),
    DontAsk,
}

/// What the window must do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Do {
    /// Write the capture (`remember`: as the `edit_save` answer); then
    /// close when `close`.
    Write { choice: Choice, close: bool, remember: bool },
    Close,
}

/// The save flow's state machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub dialog: Option<Dialog>,
}

impl Flow {
    /// Save (the toolbar, Ctrl+S): the remembered answer at once, else
    /// the question.
    pub fn save(&mut self, remembered: Option<Choice>, close: bool) -> Vec<Do> {
        match remembered {
            Some(choice) => {
                self.dialog = None;
                vec![Do::Write { choice, close, remember: false }]
            }
            None => {
                self.dialog = Some(Dialog::Choose { choice: Choice::Overwrite, dont_ask: false, close });
                Vec::new()
            }
        }
    }

    /// Closing (the window's ×): with unsaved changes, the question.
    pub fn close(&mut self, unsaved: bool) -> Vec<Do> {
        if self.dialog.is_some() {
            return Vec::new();
        }
        if unsaved {
            self.dialog = Some(Dialog::Unsaved);
            Vec::new()
        } else {
            vec![Do::Close]
        }
    }

    pub fn press(&mut self, b: Btn, remembered: Option<Choice>) -> Vec<Do> {
        match (self.dialog, b) {
            (Some(Dialog::Unsaved), Btn::Discard) => {
                self.dialog = None;
                vec![Do::Close]
            }
            (Some(_), Btn::Cancel) => {
                self.dialog = None;
                Vec::new()
            }
            (Some(Dialog::Unsaved), Btn::SaveCopy) => {
                self.dialog = None;
                vec![Do::Write { choice: Choice::New, close: true, remember: false }]
            }
            (Some(Dialog::Unsaved), Btn::Save) => self.save(remembered, true),
            (Some(Dialog::Choose { close, dont_ask, .. }), Btn::Pick(choice)) => {
                self.dialog = Some(Dialog::Choose { choice, dont_ask, close });
                Vec::new()
            }
            (Some(Dialog::Choose { choice, close, dont_ask }), Btn::DontAsk) => {
                self.dialog = Some(Dialog::Choose { choice, dont_ask: !dont_ask, close });
                Vec::new()
            }
            (Some(Dialog::Choose { choice, close, dont_ask }), Btn::Save) => {
                self.dialog = None;
                vec![Do::Write { choice, close, remember: dont_ask }]
            }
            _ => Vec::new(),
        }
    }

    /// Esc: Cancel.
    pub fn escape(&mut self) -> Vec<Do> {
        self.press(Btn::Cancel, None)
    }

    /// Enter: the default button (Save).
    pub fn enter(&mut self, remembered: Option<Choice>) -> Vec<Do> {
        self.press(Btn::Save, remembered)
    }
}

// --- writing --------------------------------------------------------------------

/// `<stem>-edited.<ext>` next to `p` (`-edited_1`... when taken).
pub fn edited_path(p: &Path) -> PathBuf {
    let stem = p.file_stem().map_or_else(|| "capture".into(), |s| s.to_string_lossy().into_owned());
    let name = match p.extension() {
        Some(e) => format!("{stem}-edited.{}", e.to_string_lossy()),
        None => format!("{stem}-edited"),
    };
    crate::export::unique_path(&p.with_file_name(name))
}

/// Where `choice` writes for an entry saved at `saved`: over that file,
/// or next to it as `-edited`; never saved, a new file where Ctrl+S
/// would save now.
pub fn target(choice: Choice, saved: Option<&Path>, cfg: &Config) -> PathBuf {
    match (saved, choice) {
        (Some(p), Choice::Overwrite) => p.to_path_buf(),
        (Some(p), Choice::New) => edited_path(p),
        (None, _) => crate::export::unique_path(&crate::export::auto_save_path(cfg, crate::export::local_ymdhms())),
    }
}

/// A save of the Edit window: the file and whether the History entry
/// learnt of it (false: the entry was deleted meanwhile).
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub path: PathBuf,
    pub bytes: u64,
    pub recorded: bool,
}

impl Saved {
    /// The toast: "Saved 2026-10-05_14-32.png", and when the entry is
    /// gone, "(History entry gone)" after it.
    pub fn note(&self) -> String {
        let n = self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if self.recorded { format!("Saved {n}") } else { format!("Saved {n} (History entry gone)") }
    }
}

/// Save `img` (the capture with `objects`) for entry `id` to `path`,
/// replacing the file there (Overwrite) all at once; then the entry gets
/// its objects, Edited and the file. A file written for an entry deleted
/// meanwhile is still a save.
pub fn write(store: &Store, id: &str, img: &crate::pixbuf::PixBuf, doc: &crate::objects::Doc, path: &Path, cfg: &Config) -> Result<Saved, String> {
    crate::export::save_image(img, path, cfg.jpeg_quality).map_err(|e| format!("{e:#}"))?;
    let bytes = std::fs::metadata(path).map_or(0, |m| m.len());
    let recorded = record_save(store, id, doc, path, bytes);
    Ok(Saved { path: path.to_path_buf(), bytes, recorded })
}

/// Entry `id` was saved to `path`: its document (objects, crop and image
/// edits, in the original's pixels), Edited and the file. False when the
/// entry is gone.
pub fn record_save(store: &Store, id: &str, doc: &crate::objects::Doc, path: &Path, bytes: u64) -> bool {
    match store.save_edit_doc(id, &doc.objects, doc.crop, &doc.edits, Some((path.to_path_buf(), bytes))) {
        Ok(_) => {
            crate::history::changed();
            true
        }
        Err(e) => {
            eprintln!("rustshot: history: {e:#}");
            false
        }
    }
}

// --- opening ------------------------------------------------------------------------

/// An open Edit window: its entry, its inbox, whether it has unsaved edits.
struct Slot {
    id: String,
    tx: Sender<Msg>,
    unsaved: Arc<AtomicBool>,
}

/// The open Edit windows (the daemon asks them before it quits).
pub struct Windows {
    list: Mutex<Vec<Slot>>,
    /// A quit was cancelled in one of their questions.
    cancelled: AtomicBool,
}

impl Default for Windows {
    fn default() -> Self {
        Self::new()
    }
}

impl Windows {
    pub const fn new() -> Windows {
        Windows { list: Mutex::new(Vec::new()), cancelled: AtomicBool::new(false) }
    }

    /// How many are open.
    pub fn count(&self) -> usize {
        lock(&self.list).len()
    }

    /// How many have unsaved edits.
    pub fn unsaved(&self) -> usize {
        lock(&self.list).iter().filter(|s| s.unsaved.load(Ordering::SeqCst)).count()
    }

    /// Quitting: each window with unsaved edits asks "Save changes?".
    /// Returns how many ask (none: quit now).
    pub fn ask_quit(&self) -> usize {
        self.cancelled.store(false, Ordering::SeqCst);
        lock(&self.list).iter().filter(|s| s.unsaved.load(Ordering::SeqCst) && s.tx.send(Msg::Quit).is_ok()).count()
    }

    /// One of them answered a quit's question with Cancel (once).
    pub fn take_cancelled(&self) -> bool {
        self.cancelled.swap(false, Ordering::SeqCst)
    }

    fn cancel_quit(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// A window for tests: its "unsaved edits" flag.
    #[cfg(test)]
    pub fn add_for_tests(&self, id: &str, unsaved: bool) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(unsaved));
        let (tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(rx);
        lock(&self.list).push(Slot { id: id.into(), tx, unsaved: flag.clone() });
        flag
    }

    #[cfg(test)]
    pub fn remove_for_tests(&self, id: &str) {
        lock(&self.list).retain(|s| s.id != id);
    }

    #[cfg(test)]
    pub fn cancel_for_tests(&self) {
        self.cancel_quit();
    }
}

/// What the daemon does about a quit while Edit windows ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitGate {
    /// Nothing unsaved is left: quit.
    Now,
    /// Some still ask.
    Wait,
    /// Cancel was pressed: stay.
    Abort,
}

/// The quit's state: `unsaved` windows left, `cancelled` in one.
pub fn quit_gate(unsaved: usize, cancelled: bool) -> QuitGate {
    if cancelled {
        QuitGate::Abort
    } else if unsaved == 0 {
        QuitGate::Now
    } else {
        QuitGate::Wait
    }
}

static OPEN: Windows = Windows::new();

/// This process's Edit windows.
pub fn windows() -> &'static Windows {
    &OPEN
}

enum Msg {
    Raise,
    /// The daemon is quitting: ask about unsaved edits, then close.
    Quit,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Open History entry `id` in an Edit window (its own thread), or raise
/// the one it is open in. macOS: no windows off the main thread yet (the
/// original opens instead).
pub fn open(id: &str) {
    #[cfg(target_os = "macos")]
    {
        // The picture with its annotations (never the bare original).
        let store = Store::open_default();
        if let Some(e) = store.get(id)
            && let Ok(f) = crate::history_ui::file_of(&store, &e)
        {
            crate::actions::open_path(&f);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut open = lock(&OPEN.list);
        if open.iter().any(|s| s.id == id && s.tx.send(Msg::Raise).is_ok()) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let unsaved = Arc::new(AtomicBool::new(false));
        open.push(Slot { id: id.to_string(), tx, unsaved: unsaved.clone() });
        drop(open);
        let id = id.to_string();
        std::thread::Builder::new().name("rustshot-edit".into()).spawn(move || run(&id, rx, unsaved)).ok();
    }
}

/// Wait until every Edit window is closed (`rustshot history` before it
/// exits).
pub fn wait_all() {
    while OPEN.count() > 0 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// The entry, waiting (a little) for the history thread to finish it: a
/// thumbnail's Edit can come before the files are written.
fn load(store: &Store, id: &str) -> Option<Entry> {
    store.get(id).or_else(|| {
        crate::history::flush(std::time::Duration::from_secs(10));
        store.get(id)
    })
}

#[cfg(not(target_os = "macos"))]
fn run(id: &str, rx: std::sync::mpsc::Receiver<Msg>, unsaved: Arc<AtomicBool>) {
    let store = Store::open_default();
    let cfg = crate::config::load();
    let done = |id: &str| lock(&OPEN.list).retain(|s| s.id != id);
    let Some(e) = load(&store, id) else {
        crate::tray::notify("That capture is no longer in History");
        done(id);
        return;
    };
    if crate::history::mask_unreadable(&e.meta) {
        crate::tray::notify("Could not open the capture: its lasso cannot be read");
        done(id);
        return;
    }
    let (img, doc) = match (store.original(&e), store.doc(&e)) {
        (Ok(i), Ok(d)) => (i, d),
        (Err(err), _) | (_, Err(err)) => {
            crate::tray::notify(&format!("Could not open the capture: {err:#}"));
            done(id);
            return;
        }
    };
    let th = crate::theme::resolve(&cfg);
    if let Some(c) = doc.crop
        && crate::objects::mask::crop_rect(c, img.dimensions()).is_none()
    {
        crate::tray::notify("Could not open the capture: its crop lies outside the picture");
        done(id);
        return;
    }
    let mut canvas = Canvas::new(img, doc.objects, cfg.clone(), th);
    // Its crop, a freeform capture's lasso and its image edits.
    canvas.set_doc(doc.crop, e.meta.mask.clone());
    canvas.set_edits(doc.edits);
    let title = format!("Edit \u{2014} {}", crate::history::title(&e.meta));
    // Pruning leaves the entry alone while it is open.
    let held = crate::history::hold_open(&e.id, &e.dir);
    let mut ed = Edit::new(store, e, canvas, cfg);
    ed.held = Some(held);
    let mut w = window::Window::new(ed, rx);
    w.unsaved = unsaved;
    let spec = crate::wind::WindowSpec { title, w: W, h: H, resizable: true, min: MIN, controls: crate::wind::Controls::MinMaxClose };
    if let Err(err) = crate::wind::run_window(spec, &mut w) {
        eprintln!("rustshot: edit window: {err:#}");
    }
    w.finish();
    // Out of the list before the window goes: an Edit pressed from now
    // on opens a new one; one pressed just before is opened here.
    done(id);
    let late = w.rx.try_iter().any(|m| matches!(m, Msg::Raise));
    drop(w);
    if late {
        open(id);
    }
}

// --- the window's state ---------------------------------------------------------

/// A press on the objects list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailDrag {
    /// The object (z-index) pressed and where (window px).
    pub obj: usize,
    pub y0: f32,
    /// Dragged far enough to reorder: the row it would drop at.
    pub to: Option<usize>,
}

/// The Edit window's state (everything but the window).
pub struct Edit {
    pub store: Store,
    pub entry: Entry,
    pub canvas: Canvas,
    pub cfg: Config,
    pub flow: Flow,
    /// The objects list is collapsed to its 44 px rail.
    pub collapsed: bool,
    pub rail_drag: Option<RailDrag>,
    /// A message at the bottom and since when.
    pub toast: Option<(String, crate::editor::style::ToastKind, std::time::Instant)>,
    /// The window should close (after a save or Discard).
    pub closing: bool,
    /// The daemon is quitting: the question asked is the quit's (Cancel
    /// stays, and the daemon too).
    pub quitting: bool,
    /// The entry is held open (never pruned meanwhile).
    pub held: Option<crate::history::OpenGuard>,
    /// The Image menu is open (from the status bar's size chip).
    pub img_menu: bool,
    /// Resize… or Canvas… is open, with the picture it works on (before
    /// the resize and the canvas) and that picture fitted for the preview
    /// (and its scale).
    pub img_dialog: Option<image::ImgDialog>,
    pub dlg_pic: Option<(crate::pixbuf::PixBuf, (crate::pixbuf::PixBuf, f32))>,
}

impl Edit {
    pub fn new(store: Store, entry: Entry, canvas: Canvas, cfg: Config) -> Edit {
        Edit {
            store,
            entry,
            canvas,
            cfg,
            flow: Flow::default(),
            collapsed: false,
            rail_drag: None,
            toast: None,
            closing: false,
            quitting: false,
            held: None,
            img_menu: false,
            img_dialog: None,
            dlg_pic: None,
        }
    }

    /// An Image menu row (or its key).
    pub fn image_act(&mut self, a: image::ImgAct) {
        use image::{CanvasForm, ImgAct, ImgDialog, ResizeForm};
        self.img_menu = false;
        if self.flow.dialog.is_some() {
            return;
        }
        match a {
            ImgAct::RotateLeft => self.canvas.rotate(false),
            ImgAct::RotateRight => self.canvas.rotate(true),
            ImgAct::FlipH => self.canvas.flip(true),
            ImgAct::FlipV => self.canvas.flip(false),
            ImgAct::Reset => self.canvas.reset_image(),
            ImgAct::Resize | ImgAct::Canvas => {
                self.canvas.commit_drafts();
                let pic = self.canvas.export_picture();
                let (w, h) = pic.dimensions();
                let fit = crate::scale::fit_size(w, h, 400, 240);
                let k = fit.0 as f32 / w.max(1) as f32;
                let small = crate::imgedit::resample::resize(&pic, fit.0, fit.1, crate::imgedit::Filter::Area);
                let e = self.canvas.edits();
                self.img_dialog = Some(if a == ImgAct::Resize {
                    ImgDialog::Resize(ResizeForm::new(self.canvas.source_size(), e.resize))
                } else {
                    ImgDialog::Canvas(CanvasForm::new(e.canvas))
                });
                self.dlg_pic = Some((pic, (small, k)));
            }
        }
    }

    /// The open image dialog was answered.
    pub fn image_answer(&mut self, out: image::DlgOut) {
        use image::{DlgOut, ImgDialog};
        let Some(d) = self.img_dialog.as_ref() else { return };
        match (d, out) {
            (_, DlgOut::Cancel) => {}
            (ImgDialog::Resize(f), DlgOut::Apply) => match f.result() {
                Some(r) => self.canvas.set_resize(r),
                None => return,
            },
            (ImgDialog::Resize(_), DlgOut::Remove) => self.canvas.set_resize(None),
            (ImgDialog::Canvas(f), DlgOut::Apply) => match f.result() {
                Some(p) => self.canvas.set_canvas(Some(p)),
                None => return,
            },
            (ImgDialog::Canvas(_), DlgOut::Remove) => self.canvas.set_canvas(None),
        }
        self.img_dialog = None;
        self.dlg_pic = None;
    }

    /// The daemon is quitting: ask about unsaved edits (else close).
    pub fn quit(&mut self) {
        self.quitting = true;
        let unsaved = self.canvas.unsaved();
        for d in self.flow.close(unsaved) {
            self.perform(d);
        }
    }

    /// After an answer: a quit's question closed without closing the
    /// window (Cancel, or the save failed) cancels the quit.
    pub fn settle_quit(&mut self) {
        if self.quitting && self.flow.dialog.is_none() && !self.closing {
            self.quitting = false;
            OPEN.cancel_quit();
        }
    }

    /// A save finished: the objects saved, the entry as it is now.
    pub fn saved(&mut self, objects: Vec<crate::objects::Obj>, s: &Saved) {
        self.canvas.mark_saved_as(objects);
        if let Some(e) = self.store.get(&self.entry.id) {
            self.entry = e;
        } else {
            self.entry.meta.saved = Some(s.path.clone());
        }
    }

    pub fn remembered(&self) -> Option<Choice> {
        Choice::from_config(&self.cfg.edit_save)
    }

    pub fn note(&mut self, text: impl Into<String>, kind: crate::editor::style::ToastKind) {
        self.toast = Some((text.into(), kind, std::time::Instant::now()));
    }

    /// The file name the questions talk about ("2026-10-05_14-32.png").
    pub fn file_name(&self) -> String {
        self.entry
            .meta
            .saved
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| crate::history::title(&self.entry.meta))
    }

    /// The two answers' files as the Save question shows them.
    pub fn choice_files(&self) -> (String, String) {
        let name = |c: Choice| {
            target(c, self.entry.meta.saved.as_deref(), &self.cfg).file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        };
        (name(Choice::Overwrite), name(Choice::New))
    }

    /// Carry out `d`.
    pub fn perform(&mut self, d: Do) {
        match d {
            Do::Close => self.closing = true,
            Do::Write { choice, close, remember } => {
                if remember {
                    self.cfg.edit_save = choice.id().into();
                    let path = crate::config::config_path();
                    if !cfg!(test) {
                        let _ = crate::config::edit_top_keys_at(&path, &[("edit_save".into(), Some(crate::config::toml_str(choice.id())))]);
                        crate::settings_ui::tell_daemon_reload();
                    }
                }
                let path = target(choice, self.entry.meta.saved.as_deref(), &self.cfg);
                // Text being typed is part of what is saved.
                self.canvas.commit_drafts();
                // Save at (unless resized on purpose) and the border.
                let resized = self.canvas.edits().resize.is_some();
                let img = crate::export::for_output(self.canvas.export(), &self.cfg, self.entry.meta.scale, resized);
                let objs = self.canvas.objects().to_vec();
                let doc = self.canvas.doc();
                match write(&self.store, &self.entry.id, &img, &doc, &path, &self.cfg) {
                    Ok(s) => {
                        self.saved(objs, &s);
                        let kind = if s.recorded { crate::editor::style::ToastKind::Success } else { crate::editor::style::ToastKind::Info };
                        self.note(s.note(), kind);
                        if close {
                            self.closing = true;
                        }
                    }
                    Err(e) => self.note(format!("Save failed: {e}"), crate::editor::style::ToastKind::Error),
                }
            }
        }
    }

    /// The editor asked for something.
    pub fn request(&mut self, r: Request) -> Vec<Do> {
        match r {
            Request::Save => {
                let rem = self.remembered();
                self.flow.save(rem, false)
            }
            Request::Close => self.flow.close(self.canvas.unsaved()),
            _ => Vec::new(), // the window runs the exports (threads)
        }
    }
}

#[cfg(test)]
mod tests;
