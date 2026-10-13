//! The History window (boards 10: History, HistoryLight, HistoryStates):
//! the captures the history keeps ([`crate::history`]), newest first, in
//! day groups with sticky headers ("Today · 4 captures", "Yesterday ·
//! Sun 11 Oct", "Fri 9 Oct"), as a grid of 200 × 140 cards or a list of
//! 56 px rows. The header searches window, app, file name and date
//! (matches highlighted, the groups collapse to "N results for "q"", Esc
//! clears), filters All / Screenshots / Recordings, switches the view,
//! sizes the thumbnails, toggles the details panel and has the ⋯ menu
//! (Clear history…, Open history folder, Settings…). A click selects
//! (Ctrl toggles, Shift extends, a drag on empty space is a marquee);
//! one selected capture opens its details on the right (collapsed below
//! 900 px), several get checkboxes and the bottom bar (Copy · Save to
//! folder… · Delete). A card's hover strip and a row's hover buttons
//! Edit, Copy, Save, Upload, Pin; double-click edits (a recording opens).
//! The empty state, "History is off" and the "Clear history?" question
//! are the board's other states.
//!
//! A normal resizable window (960 × 640, at least 720 × 480) on its own
//! thread, one at a time ([`show`] raises the open one). macOS opens no
//! windows off the main thread: there the history folder opens instead.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use crate::config::Config;
use crate::history::{self, Entry, Filter, Store};
use crate::pixbuf::PixBuf;
use crate::ui::TextState;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
#[cfg(not(target_os = "macos"))]
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;

pub mod draw;

/// Default and minimum size, logical px.
pub const W: u32 = 960;
pub const H: u32 = 640;
pub const MIN: (u32, u32) = (720, 480);
/// Below this width the details panel is not shown.
pub const PANEL_MIN_W: f32 = 900.0;
/// Thumbnail size (the slider): the card's minimum width, 140 high at 200.
pub const THUMB_DEFAULT: f32 = 200.0;
pub const THUMB_RANGE: (f32, f32) = (150.0, 300.0);

/// Grid or list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    #[default]
    Grid,
    List,
}

/// A menu open over the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Menu {
    /// The header's ⋯: Clear history…, Open history folder, Settings…
    More,
    /// A card's ⋯ (its id): Save As…, Show in folder, Copy file path, Delete.
    Card(String),
}

/// What the user asked for; the window carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Edit(String),
    /// Open the saved file (or the recording; else the original).
    Open(String),
    Copy(Vec<String>),
    Save(String),
    SaveAs(String),
    Upload(String),
    Pin(String),
    ShowInFolder(String),
    CopyPath(String),
    CopyLink(String),
    /// Open the capture with an app the user picks.
    OpenWith(String),
    /// Hand the capture (or the recording) to the share sheet.
    Share(String),
    /// Delete the upload from Imgur (after "Delete from Imgur?").
    DeleteUpload(String),
    Delete(Vec<String>),
    SaveToFolder(Vec<String>),
    /// "Clear history" (and the saved files with it).
    Clear(bool),
    OpenFolder,
    Settings,
    /// "Keep a history of captures" turned on (the off state's switch).
    TurnOn,
}

/// How many decoded thumbnails stay in memory (about 110 KB each).
pub const THUMBS_KEPT: usize = 200;

/// A decoded thumbnail (None: none or unreadable), when it was last
/// used, and its file's size and time (an edit writes a new one).
pub struct Thumb {
    pub pic: Option<PixBuf>,
    pub used: u64,
    pub stamp: Option<(u64, std::time::SystemTime)>,
}

/// `thumb.png`'s size and modification time.
fn thumb_stamp(e: &Entry) -> Option<(u64, std::time::SystemTime)> {
    std::fs::metadata(e.thumb()).ok().and_then(|m| Some((m.len(), m.modified().ok()?)))
}

/// The window's state.
pub struct History {
    pub store: Store,
    pub entries: Vec<Entry>,
    /// `history::generation` when listed.
    pub listed: u64,
    pub search: TextState,
    pub filter: Filter,
    pub view: View,
    /// Selected ids, and the one Shift extends from.
    pub sel: Vec<String>,
    pub anchor: Option<String>,
    /// The details panel is wanted (it shows for one selected capture).
    pub panel: bool,
    pub thumb: f32,
    pub scroll: f32,
    /// The "Clear history?" question: its "Also delete saved files".
    pub clear: Option<bool>,
    pub menu: Option<Menu>,
    pub cfg: Config,
    /// Today (local), for the group labels.
    pub today: (i32, u32, u32),
    /// Bytes the history takes now.
    pub usage: u64,
    /// A short message at the bottom (and when).
    pub toast: Option<(String, crate::editor::style::ToastKind, std::time::Instant)>,
    /// Decoded thumbnails by id (the last used [`THUMBS_KEPT`]).
    pub thumbs: HashMap<String, Thumb>,
    /// Counts [`History::thumb_of`] calls (the cache's "last used").
    pub thumb_clock: u64,
    /// "Delete N captures?" (more than one selected) asks first.
    pub ask_delete: Option<Vec<String>>,
    /// "Delete from Imgur?" (the entry whose upload goes).
    pub ask_unupload: Option<String>,
    /// The pointer between frames (press, marquee, double-click).
    pub ptr: window::Pointer,
    /// The search field has the keyboard (keys are its); focus it next frame.
    pub search_focused: bool,
    pub focus_search: bool,
    /// The pointer is over the search field (an I-beam).
    pub cursor_text: bool,
    /// What the pointer was on in the last frame.
    pub hover: Option<draw::Target>,
}

impl History {
    pub fn new(store: Store, cfg: Config, today: (i32, u32, u32)) -> History {
        let mut h = History {
            store,
            entries: Vec::new(),
            listed: 0,
            search: TextState::new(""),
            filter: Filter::All,
            view: View::Grid,
            sel: Vec::new(),
            anchor: None,
            panel: true,
            thumb: THUMB_DEFAULT,
            scroll: 0.0,
            clear: None,
            menu: None,
            cfg,
            today,
            usage: 0,
            toast: None,
            thumbs: HashMap::new(),
            thumb_clock: 0,
            ask_delete: None,
            ask_unupload: None,
            ptr: Default::default(),
            search_focused: false,
            focus_search: false,
            cursor_text: false,
            hover: None,
        };
        h.reload();
        h
    }

    /// List the store again (selection kept where its entries still are).
    pub fn reload(&mut self) {
        self.listed = history::generation();
        let entries = self.store.list();
        self.take(entries);
    }

    /// Show `entries` (a listing of the store): the selection stays where
    /// its entries still are; thumbnails of gone or changed ones go.
    pub fn take(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.usage = self.entries.iter().map(|e| e.size).sum();
        let ids: std::collections::HashSet<&str> = self.entries.iter().map(|e| e.id.as_str()).collect();
        self.sel.retain(|s| ids.contains(s.as_str()));
        let changed: Vec<String> = self
            .thumbs
            .iter()
            .filter(|(k, c)| !ids.contains(k.as_str()) || self.entry(k).is_none_or(|e| thumb_stamp(e) != c.stamp))
            .map(|(k, _)| k.clone())
            .collect();
        for k in changed {
            self.thumbs.remove(&k);
        }
    }

    /// The thumbnail of `id`, decoded once (the [`THUMBS_KEPT`] last used
    /// stay decoded).
    pub fn thumb_of(&mut self, id: &str) -> Option<&PixBuf> {
        self.thumb_clock += 1;
        let now = self.thumb_clock;
        if !self.thumbs.contains_key(id) {
            let e = self.entries.iter().find(|e| e.id == id);
            let stamp = e.and_then(thumb_stamp);
            let pic = e.and_then(|e| std::fs::read(e.thumb()).ok()).and_then(|b| PixBuf::from_png(&b).ok());
            if self.thumbs.len() >= THUMBS_KEPT
                && let Some(old) = self.thumbs.iter().min_by_key(|(_, c)| c.used).map(|(k, _)| k.clone())
            {
                self.thumbs.remove(&old);
            }
            self.thumbs.insert(id.to_string(), Thumb { pic, used: now, stamp });
        }
        let c = self.thumbs.get_mut(id)?;
        c.used = now;
        c.pic.as_ref()
    }

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The history is on.
    pub fn on(&self) -> bool {
        self.cfg.history
    }

    pub fn query(&self) -> &str {
        self.search.text.trim()
    }

    /// The entries shown (filter and search) with their highlights, in order.
    pub fn shown(&self) -> Vec<(usize, Vec<std::ops::Range<usize>>)> {
        let q = self.query();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.filter.keeps_meta(&e.meta))
            .filter_map(|(i, e)| history::search(&e.meta, q, self.today).map(|h| (i, h)))
            .collect()
    }

    /// The shown entries in groups: (heading, its smaller part, entries).
    /// A search is one group, "N results for "q"".
    pub fn groups(&self) -> Vec<Group> {
        let shown = self.shown();
        if !self.query().is_empty() {
            let items = shown;
            return vec![Group { title: history::results_header(items.len(), self.query()), small: None, items }];
        }
        let mut out: Vec<Group> = Vec::new();
        let mut day = None;
        for (i, h) in shown {
            let d = self.entries[i].day();
            if day != Some(d) {
                day = Some(d);
                let n = self.entries.iter().filter(|e| e.day() == d && self.filter.keeps_meta(&e.meta)).count();
                let (title, small) = history::day_header(d, self.today, n);
                out.push(Group { title, small, items: Vec::new() });
            }
            out.last_mut().expect("a group").items.push((i, h));
        }
        out
    }

    /// The one selected entry (the details panel's).
    pub fn single(&self) -> Option<&Entry> {
        match &self.sel[..] {
            [id] => self.entry(id),
            _ => None,
        }
    }

    /// A click on `id`: select it alone, Ctrl toggles it, Shift selects
    /// from the anchor to it (in `order`, the shown ids).
    pub fn click(&mut self, id: &str, ctrl: bool, shift: bool, order: &[String]) {
        if shift && let Some(a) = self.anchor.clone() {
            let (Some(i), Some(j)) = (order.iter().position(|o| *o == a), order.iter().position(|o| o == id)) else {
                self.sel = vec![id.to_string()];
                return;
            };
            let (lo, hi) = (i.min(j), i.max(j));
            let range: Vec<String> = order[lo..=hi].to_vec();
            if ctrl {
                for r in range {
                    if !self.sel.contains(&r) {
                        self.sel.push(r);
                    }
                }
            } else {
                self.sel = range;
            }
        } else if ctrl {
            if let Some(p) = self.sel.iter().position(|s| s == id) {
                self.sel.remove(p);
            } else {
                self.sel.push(id.to_string());
            }
            self.anchor = Some(id.to_string());
        } else {
            self.sel = vec![id.to_string()];
            self.anchor = Some(id.to_string());
            self.panel = true;
        }
    }

    /// Esc: closes a menu or the question, else clears the search, else
    /// the selection. Returns whether it did something.
    pub fn escape(&mut self) -> bool {
        if self.menu.take().is_some() || self.clear.take().is_some() || self.ask_delete.take().is_some() || self.ask_unupload.take().is_some() {
            return true;
        }
        if !self.search.text.is_empty() {
            self.search.set("");
            self.scroll = 0.0;
            return true;
        }
        if !self.sel.is_empty() {
            self.sel.clear();
            return true;
        }
        false
    }

    /// "Keeping 30 days", "1.2 GB of 2 GB", "112 captures".
    pub fn footer(&self) -> (String, String, String, f32) {
        let max = self.cfg.history_max_mb.max(1) as u64 * 1_000_000;
        let days = self.cfg.history_days;
        let n = self.entries.len();
        (
            format!("Keeping {days} day{}", if days == 1 { "" } else { "s" }),
            format!("{} of {}", history::size_text(self.usage), history::size_text(max).replace(".0 GB", " GB")),
            format!("{n} capture{}", if n == 1 { "" } else { "s" }),
            (self.usage as f32 / max as f32).clamp(0.0, 1.0),
        )
    }

    /// The question's text: "Removes all 112 captures from History. This
    /// can't be undone." and "Also delete saved files (1.2 GB in
    /// ~/Pictures/rustshot)".
    pub fn clear_texts(&self) -> (String, String) {
        let n = self.entries.len();
        let (_, bytes, dir) = history::saved_files(&self.entries);
        let where_ = dir.map(|d| format!(" in {}", history::tilde(&d))).unwrap_or_default();
        (
            format!("Removes all {n} capture{} from History. This can't be undone.", if n == 1 { "" } else { "s" }),
            format!("Also delete saved files ({}{where_}) and forget upload links", history::size_text(bytes)),
        )
    }

    pub fn note(&mut self, text: impl Into<String>, kind: crate::editor::style::ToastKind) {
        self.toast = Some((text.into(), kind, std::time::Instant::now()));
    }

    /// A question is open ("Clear history?", "Delete N captures?").
    pub fn asking(&self) -> bool {
        self.clear.is_some() || self.ask_delete.is_some() || self.ask_unupload.is_some()
    }

    /// Whether entry `id`'s upload can be deleted from Imgur (its
    /// deletehash was kept).
    pub fn can_unupload(&self, id: &str) -> bool {
        self.entry(id).is_some_and(|e| e.meta.uploaded.is_some() && e.meta.upload.as_ref().is_some_and(|u| !u.deletehash.is_empty()))
    }

    /// Delete `ids`: one at once, several after "Delete N captures?".
    pub fn delete(&mut self, ids: Vec<String>) -> Vec<Act> {
        if ids.len() > 1 {
            self.ask_delete = Some(ids);
            Vec::new()
        } else {
            vec![Act::Delete(ids)]
        }
    }

    /// The question's title, text, checkbox (Clear only) and button.
    pub fn dialog_texts(&self) -> (String, String, Option<String>, &'static str) {
        if let Some(id) = &self.ask_unupload {
            let link = self.entry(id).and_then(|e| e.meta.uploaded.clone()).unwrap_or_default();
            return (
                "Delete from Imgur?".into(),
                format!("{} stops working for everyone. The capture stays in History. This can't be undone.", crate::uploads::short_link(&link)),
                None,
                "Delete from Imgur",
            );
        }
        if let Some(ids) = &self.ask_delete {
            let n = ids.len();
            return (
                format!("Delete {n} captures?"),
                format!("Removes {n} captures from History. Saved files are kept. This can't be undone."),
                None,
                "Delete",
            );
        }
        let (para, check) = self.clear_texts();
        ("Clear history?".into(), para, Some(check), "Clear history")
    }

    /// The question's button (or Enter).
    pub fn dialog_ok(&mut self) -> Vec<Act> {
        if let Some(id) = self.ask_unupload.take() {
            return vec![Act::DeleteUpload(id)];
        }
        if let Some(ids) = self.ask_delete.take() {
            return vec![Act::Delete(ids)];
        }
        match self.clear.take() {
            Some(files) => vec![Act::Clear(files)],
            None => Vec::new(),
        }
    }

    /// Cancel (or Esc).
    pub fn dialog_cancel(&mut self) {
        self.clear = None;
        self.ask_delete = None;
        self.ask_unupload = None;
    }
}

/// A day group (or the search's results).
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub title: String,
    pub small: Option<String>,
    pub items: Vec<(usize, Vec<std::ops::Range<usize>>)>,
}

// --- carrying out ---------------------------------------------------------------

/// Edit capture `id` (the Edit window).
pub fn open_edit(id: &str) {
    crate::edit_ui::open(id);
}

/// The picture of `e` as it looks now: its original with its objects.
/// Fails closed: when `annotations.json` is there but cannot be read (a
/// broken file, or one from a newer build), it is an error, never the
/// original without the annotations (they may hide something).
pub fn picture(store: &Store, e: &Entry) -> anyhow::Result<PixBuf> {
    let objs = store.objects(e).map_err(|err| anyhow::anyhow!("its annotations cannot be read, so it is not shown without them ({err:#})"))?;
    let orig = store.original(e)?;
    if objs.is_empty() {
        return Ok(orig);
    }
    let font = crate::text::AnnotFont::load();
    Ok(crate::editor::bake_objects(&orig, &objs, font.as_ref()))
}

/// The file a capture opens as: the saved one (or the recording) when it
/// is still there; else the original when it has no annotations, or the
/// picture with them written next to it (`picture.png`, again each time).
/// An error (not the original) when its annotations cannot be read.
pub fn file_of(store: &Store, e: &Entry) -> anyhow::Result<PathBuf> {
    if let Some(p) = e.meta.saved.iter().chain(e.meta.file.iter()).find(|p| p.is_file()) {
        return Ok(p.clone());
    }
    if !e.annotations().exists() {
        return Ok(e.original());
    }
    let img = picture(store, e)?;
    let to = e.annotations().with_file_name("picture.png");
    crate::export::write_atomic(&to, &img.to_png()?)?;
    Ok(to)
}

/// What a worker reports back: a message, and whether the list changed.
pub struct Done {
    pub text: String,
    pub kind: crate::editor::style::ToastKind,
    pub changed: bool,
}

fn ok(text: impl Into<String>) -> Done {
    Done { text: text.into(), kind: crate::editor::style::ToastKind::Success, changed: false }
}

fn fail(text: impl Into<String>) -> Done {
    Done { text: text.into(), kind: crate::editor::style::ToastKind::Error, changed: false }
}

pub(super) fn ok_done() -> Done {
    ok("")
}

pub(super) fn fail_done(text: impl Into<String>) -> Done {
    fail(text)
}

/// Copy, Save, Save As, Upload, Pin of entry `id` (on a worker thread:
/// encoding and uploading take a while). Uploads wait for their link.
pub fn run_export(store: &Store, id: &str, task: crate::export::Task, cfg: &Config) -> Done {
    use crate::export::Task;
    let Some(e) = store.get(id) else { return fail("That capture is gone") };
    if e.meta.kind.is_recording() {
        let Some(f) = e.meta.file.clone().filter(|f| f.is_file()) else { return fail("The recording's file is gone") };
        return match task {
            Task::Copy => match crate::notice::copy_file(&f) {
                Ok(()) => ok("Copied to clipboard"),
                Err(err) => fail(format!("Copy failed: {err:#}")),
            },
            _ => {
                crate::notice::show_in_folder(&f);
                ok("")
            }
        };
    }
    let img = match picture(store, &e) {
        Ok(i) => i,
        Err(err) => return fail(format!("Could not read the capture: {err:#}")),
    };
    if task == Task::Pin {
        let th = crate::theme::resolve(cfg);
        #[cfg(not(test))]
        let at = crate::capture::cursor_pos();
        #[cfg(test)]
        let at = (0, 0);
        return if crate::pin::open(std::sync::Arc::new(img), at, th) { ok("") } else { fail("Pin to screen is not available here") };
    }
    let res = crate::export::run_export(&img, (0, 0), std::slice::from_ref(&task), cfg);
    let mut changed = false;
    // The file it was saved to becomes the entry's.
    for d in &res.done {
        if let crate::export::Done::Saved(p) | crate::export::Done::SavedDesktop(p) = d {
            let (p, bytes) = (p.clone(), std::fs::metadata(p).map_or(0, |m| m.len()));
            changed |= store.update(id, |m| {
                m.format = p.extension().map_or(m.format.clone(), |x| x.to_string_lossy().to_uppercase());
                m.saved = Some(p);
                m.bytes = bytes;
            }).is_ok();
        }
    }
    if let Some(rx) = res.upload {
        return match rx.recv() {
            Ok(Ok(url)) => {
                let copied = cfg.copy_url_after_upload && crate::export::copy_text_to_clipboard(&url).is_ok();
                let _ = crate::uploads::finished_in(store, id, &url);
                Done { changed: true, ..ok(if copied { "Uploaded \u{2014} link copied" } else { "Uploaded" }) }
            }
            Ok(Err(err)) => fail(format!("Upload failed: {err}")),
            Err(_) => fail("Upload failed"),
        };
    }
    match crate::notice::compose(&res.done, false) {
        Some(n) => Done { text: n.text, kind: n.kind, changed },
        None => Done { text: String::new(), ..ok("") },
    }
}

/// "Open with…" for entry `id`: its saved file (or the recording) when
/// that is still what it looks like, else the picture as a temporary file.
pub fn open_with(store: &Store, id: &str, cfg: &Config, owner: Option<usize>) -> Done {
    match file_to_hand(store, id, cfg, "Open with") {
        Ok(f) => crate::open_with::open_now(&f, owner).map_or_else(fail, |()| ok("")),
        Err(d) => d,
    }
}

/// "Share…" for entry `id`: the same file as Open with (its saved file
/// or the recording, else a temporary PNG), to the share sheet over
/// window `owner`.
pub fn share(store: &Store, id: &str, cfg: &Config, owner: Option<usize>) -> Done {
    match file_to_hand(store, id, cfg, "Share") {
        Ok(f) => crate::share::share_now(&f, owner).map_or_else(fail, |()| ok("")),
        Err(d) => d,
    }
}

/// The file Open with / Share hands over for entry `id`: its saved file
/// (or the recording) when that is still what it looks like, else the
/// picture as a temporary file. `Err`: the failure to show.
fn file_to_hand(store: &Store, id: &str, cfg: &Config, what: &str) -> Result<std::path::PathBuf, Done> {
    let Some(e) = store.get(id) else { return Err(fail("That capture is gone")) };
    let as_is = e.meta.kind.is_recording() || (!e.meta.edited && e.meta.saved.as_ref().is_some_and(|p| p.is_file()));
    if as_is {
        let f = file_of(store, &e).map_err(|err| fail(format!("Could not read the capture: {err:#}")))?;
        if !f.is_file() {
            return Err(fail("The file is gone"));
        }
        return Ok(f);
    }
    let img = picture(store, &e).map_err(|err| fail(format!("Could not read the capture: {err:#}")))?;
    crate::open_with::file_for(&img, None, cfg).map_err(|err| fail(format!("{what} failed: {err:#}")))
}

/// Delete entry `id`'s upload from Imgur over `http` (a fake in tests),
/// then forget its link and record.
pub fn delete_upload(store: &Store, id: &str, client_id: &str, http: crate::uploads::DeleteFn) -> Done {
    let Some(e) = store.get(id) else { return fail("That capture is gone") };
    let (Some(link), Some(info)) = (e.meta.uploaded.clone(), e.meta.upload.clone()) else { return fail("This capture has no upload to delete") };
    let rec = crate::uploads::Record { link, info, entry: Some(id.to_string()) };
    let (text, kind) = crate::uploads::delete_record(store, &rec, client_id, http);
    Done { text, kind, changed: true }
}

/// Save every capture of `ids` as a PNG into `dir` (named by when they
/// were taken).
pub fn save_to_folder(store: &Store, ids: &[String], dir: &std::path::Path) -> Done {
    let mut n = 0;
    let mut why = None;
    for id in ids {
        let Some(e) = store.get(id) else { continue };
        let t = e.meta.captured;
        if e.meta.kind.is_recording() {
            if let Some(f) = e.meta.file.as_ref().filter(|f| f.is_file()) {
                let to = crate::export::unique_path(&dir.join(f.file_name().unwrap_or_default()));
                if std::fs::copy(f, to).is_ok() {
                    n += 1;
                }
            }
            continue;
        }
        let name = format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.png", t.0, t.1, t.2, t.3, t.4, t.5);
        let to = crate::export::unique_path(&dir.join(name));
        let img = match picture(store, &e) {
            Ok(img) => img,
            Err(err) => {
                // Not the original without its annotations.
                why.get_or_insert(format!("{err:#}"));
                continue;
            }
        };
        if let Ok(png) = img.to_png()
            && std::fs::write(&to, png).is_ok()
        {
            n += 1;
        }
    }
    match (n == ids.len(), why) {
        (true, _) => ok(format!("Saved {n} to {}", dir.display())),
        (false, Some(w)) => fail(format!("Saved {n} of {}: a capture could not be read: {w}", ids.len())),
        (false, None) => fail(format!("Saved {n} of {}", ids.len())),
    }
}

// --- the window -----------------------------------------------------------------

enum Msg {
    Raise,
    /// Ask "Clear history?" (Settings › Capture › Clear history…).
    Clear,
}

static OPEN: Mutex<Option<Sender<Msg>>> = Mutex::new(None);
/// The window [`show_clear`] opens starts with the question.
static START_CLEAR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `rustshot history`: the window on this thread; returns once it and
/// the Edit windows it opened are closed.
pub fn run_here() {
    #[cfg(target_os = "macos")]
    show();
    #[cfg(not(target_os = "macos"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let mut open = lock(&OPEN);
            if let Some(t) = open.as_ref()
                && t.send(Msg::Raise).is_ok()
            {
                return;
            }
            *open = Some(tx);
        }
        run(rx);
        crate::edit_ui::wait_all();
    }
}

/// Open the History window (or raise it) asking "Clear history?".
pub fn show_clear() {
    #[cfg(not(target_os = "macos"))]
    {
        let open = lock(&OPEN);
        if let Some(tx) = open.as_ref()
            && tx.send(Msg::Clear).is_ok()
        {
            let _ = tx.send(Msg::Raise);
            return;
        }
        START_CLEAR.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    show();
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Open the History window on its own thread, or raise the open one.
pub fn show() {
    #[cfg(target_os = "macos")]
    {
        let root = history::default_root();
        let _ = std::fs::create_dir_all(&root);
        crate::actions::open_path(&root);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut open = lock(&OPEN);
        if let Some(tx) = open.as_ref()
            && tx.send(Msg::Raise).is_ok()
        {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        *open = Some(tx);
        drop(open);
        std::thread::Builder::new().name("rustshot-history-ui".into()).spawn(move || run(rx)).ok();
    }
}

#[cfg(not(target_os = "macos"))]
fn run(rx: Receiver<Msg>) {
    let cfg = crate::config::load();
    let t = crate::export::local_ymdhms();
    let mut h = History::new(Store::open_default(), cfg, (t.0, t.1, t.2));
    if START_CLEAR.swap(false, std::sync::atomic::Ordering::SeqCst) && !h.entries.is_empty() {
        h.clear = Some(false);
    }
    let mut w = window::Window::new(h, rx);
    let spec = crate::wind::WindowSpec { title: "Rustshot History".into(), w: W, h: H, resizable: true, min: MIN };
    if let Err(e) = crate::wind::run_window(spec, &mut w) {
        eprintln!("rustshot: history window: {e:#}");
    }
    let mut open = lock(&OPEN);
    let late = w.rx.try_iter().any(|m| matches!(m, Msg::Raise));
    *open = None;
    drop(open);
    if late {
        show();
    }
}

mod window;

#[cfg(test)]
mod tests;

