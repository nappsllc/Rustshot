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
use std::sync::Mutex;
use std::sync::mpsc::Sender;

pub mod draw;
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

/// Where `choice` writes for an entry saved at `saved` (else where Ctrl+S
/// would save now).
pub fn target(choice: Choice, saved: Option<&Path>, cfg: &Config) -> PathBuf {
    let base = saved.map(Path::to_path_buf).unwrap_or_else(|| crate::export::auto_save_path(cfg, crate::export::local_ymdhms()));
    match choice {
        Choice::Overwrite => base,
        Choice::New => edited_path(&base),
    }
}

/// Save `img` (the capture with `objects`) for entry `id` to `path`; the
/// entry gets its objects, Edited and the file. Returns the file's size.
pub fn write(store: &Store, id: &str, img: &crate::pixbuf::PixBuf, objects: &[crate::objects::Obj], path: &Path, cfg: &Config) -> Result<u64, String> {
    let res = crate::export::run_export(img, (0, 0), &[crate::export::Task::Save { path: Some(path.to_path_buf()), ask: false }], cfg);
    if let Some(crate::export::Done::Failed(_, e)) = res.done.iter().find(|d| matches!(d, crate::export::Done::Failed(..))) {
        return Err(e.clone());
    }
    let saved = res.done.iter().find_map(|d| match d {
        crate::export::Done::Saved(p) | crate::export::Done::SavedDesktop(p) => Some(p.clone()),
        _ => None,
    });
    let saved = saved.ok_or_else(|| "not saved".to_string())?;
    let bytes = std::fs::metadata(&saved).map_or(0, |m| m.len());
    store.save_edit(id, objects, Some((saved, bytes))).map_err(|e| format!("{e:#}"))?;
    crate::history::changed();
    Ok(bytes)
}

// --- opening ------------------------------------------------------------------------

/// The open Edit windows: their entry and inbox.
static OPEN: Mutex<Vec<(String, Sender<Msg>)>> = Mutex::new(Vec::new());

enum Msg {
    Raise,
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
        if let Some(e) = Store::open_default().get(id) {
            crate::actions::open_path(&e.original());
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut open = lock(&OPEN);
        if open.iter().any(|(i, tx)| i == id && tx.send(Msg::Raise).is_ok()) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        open.push((id.to_string(), tx));
        drop(open);
        let id = id.to_string();
        std::thread::Builder::new().name("rustshot-edit".into()).spawn(move || run(&id, rx)).ok();
    }
}

/// Wait until every Edit window is closed (`rustshot history` before it
/// exits).
pub fn wait_all() {
    while !lock(&OPEN).is_empty() {
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
fn run(id: &str, rx: std::sync::mpsc::Receiver<Msg>) {
    let store = Store::open_default();
    let cfg = crate::config::load();
    let done = |id: &str| lock(&OPEN).retain(|(i, _)| i != id);
    let Some(e) = load(&store, id) else {
        crate::tray::notify("That capture is no longer in History");
        done(id);
        return;
    };
    let (img, objs) = match (store.original(&e), store.objects(&e)) {
        (Ok(i), Ok(o)) => (i, o),
        (Err(err), _) | (_, Err(err)) => {
            crate::tray::notify(&format!("Could not open the capture: {err:#}"));
            done(id);
            return;
        }
    };
    let th = crate::theme::resolve(&cfg);
    let canvas = Canvas::new(img, objs, cfg.clone(), th);
    let title = format!("Edit \u{2014} {}", crate::history::title(&e.meta));
    let mut w = window::Window::new(Edit::new(store, e, canvas, cfg), rx);
    let spec = crate::wind::WindowSpec { title, w: W, h: H, resizable: true, min: MIN };
    if let Err(err) = crate::wind::run_window(spec, &mut w) {
        eprintln!("rustshot: edit window: {err:#}");
    }
    w.finish();
    done(id);
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
}

impl Edit {
    pub fn new(store: Store, entry: Entry, canvas: Canvas, cfg: Config) -> Edit {
        Edit { store, entry, canvas, cfg, flow: Flow::default(), collapsed: false, rail_drag: None, toast: None, closing: false }
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
                let img = self.canvas.export();
                let objs = self.canvas.objects().to_vec();
                match write(&self.store, &self.entry.id, &img, &objs, &path, &self.cfg) {
                    Ok(_) => {
                        self.canvas.mark_saved();
                        if let Some(e) = self.store.get(&self.entry.id) {
                            self.entry = e;
                        }
                        let n = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        self.note(format!("Saved {n}"), crate::editor::style::ToastKind::Success);
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
