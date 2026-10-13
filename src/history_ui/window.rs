//! The History window's driver: input against the last layout, the
//! actions it asks for (carried out here or on a worker thread), and the
//! frames.

use super::draw::{self, Lay, MenuAct, Quick, Target};
use super::{Act, History, Menu, Msg};
use crate::objects::{FRect, Pt};
use crate::pixbuf::PixBuf;
use crate::ui::{FocusState, Input, Ui};
use crate::wind::{self, Cursor, Driver, Ev, Hwnd, key};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// What a press started.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// On a target: acts on release over the same one.
    On(Target),
    /// A marquee from this content point.
    Marquee(Pt),
    /// Dragging the thumbnail slider.
    Slider,
}

/// The pointer's state between frames.
#[derive(Default)]
pub struct Pointer {
    pub press: Option<(Press, Pt)>,
    /// The marquee's corners (content coordinates) and the selection it
    /// started from (Ctrl/Shift keep it).
    pub marquee: Option<(Pt, Pt, Vec<String>)>,
    /// The last click on an item: for double-clicks.
    pub last_click: Option<(String, Instant)>,
}

impl History {
    /// The marquee's corners (content coordinates).
    pub fn marquee(&self) -> Option<(Pt, Pt)> {
        self.ptr.marquee.as_ref().map(|(a, b, _)| (*a, *b))
    }
}

/// Run one frame of input against `lay`: hover, presses, releases, the
/// wheel, keys. Returns the actions.
pub fn input(h: &mut History, lay: &Lay, targets: &[(FRect, Target)], inp: &Input, k: f32, now: Instant) -> Vec<Act> {
    let mut acts = Vec::new();
    let m = Pt::new(inp.mouse.x / k, inp.mouse.y / k);
    let mods = inp.mods;
    // The wheel scrolls (three rows a notch).
    if inp.wheel != 0 && !h.asking() && h.menu.is_none() {
        h.scroll = (h.scroll - inp.wheel as f32 / 120.0 * 72.0).clamp(0.0, lay.max_scroll());
    }
    if let Some(p) = inp.pressed {
        let p = Pt::new(p.x / k, p.y / k);
        let t = draw::hit(targets, p.x, p.y).unwrap_or(Target::Inert);
        // A press outside an open menu closes it (and does nothing else).
        if h.menu.is_some() && !matches!(t, Target::MenuItem(_)) {
            h.menu = None;
            if !matches!(t, Target::More | Target::Quick(_, Quick::More)) {
                return acts;
            }
        }
        let press = match t {
            Target::Slider => {
                set_slider(h, lay, p.x);
                Press::Slider
            }
            Target::Space if !h.asking() => {
                let c = Pt::new(p.x - lay.area.x, p.y - lay.area.y + h.scroll);
                let base = if mods.ctrl || mods.shift { h.sel.clone() } else { Vec::new() };
                h.sel = base.clone();
                h.ptr.marquee = Some((c, c, base));
                Press::Marquee(c)
            }
            t => Press::On(t),
        };
        h.ptr.press = Some((press, p));
    }
    // Drags.
    if inp.held
        && let Some((press, _)) = &h.ptr.press
    {
        match press {
            Press::Slider => set_slider(h, lay, m.x),
            Press::Marquee(a) => {
                let a = *a;
                let c = Pt::new(m.x - lay.area.x, (m.y - lay.area.y + h.scroll).clamp(0.0, lay.content_h));
                // Near the edges the area scrolls.
                if m.y > lay.area.y1() - 12.0 {
                    h.scroll = (h.scroll + 12.0).min(lay.max_scroll());
                } else if m.y < lay.area.y + 12.0 {
                    h.scroll = (h.scroll - 12.0).max(0.0);
                }
                marquee_select(h, lay, a, c);
            }
            Press::On(_) => {}
        }
    }
    if let Some(r) = inp.released {
        let r = Pt::new(r.x / k, r.y / k);
        let press = h.ptr.press.take();
        h.ptr.marquee = None;
        if let Some((Press::On(t), p0)) = press {
            let same = draw::hit(targets, r.x, r.y).as_ref() == Some(&t);
            if same {
                acts.extend(click(h, lay, t, mods, (r.x - p0.x).hypot(r.y - p0.y), now));
            }
        }
    }
    // Keys.
    let typing = h.search_focused;
    for ev in &inp.keys {
        let crate::ui::KeyIn::Key { vk, mods } = *ev else { continue };
        if h.asking() {
            match vk {
                key::ESCAPE => h.dialog_cancel(),
                key::RETURN => acts.extend(h.dialog_ok()),
                _ => {}
            }
            continue;
        }
        match vk {
            key::ESCAPE => {
                h.escape();
            }
            key::DELETE if !typing && !h.sel.is_empty() => {
                let ids = h.sel.clone();
                acts.extend(h.delete(ids));
            }
            key::RETURN if !typing => {
                if let Some(e) = h.single() {
                    acts.push(if e.meta.kind.is_recording() { Act::Open(e.id.clone()) } else { Act::Edit(e.id.clone()) });
                }
            }
            v if v == 'E' as u32 && !typing && !mods.ctrl && !mods.alt => {
                if let Some(e) = h.single().filter(|e| !e.meta.kind.is_recording()) {
                    acts.push(Act::Edit(e.id.clone()));
                }
            }
            v if v == 'A' as u32 && mods.ctrl && !typing => {
                h.sel = lay.items.iter().map(|i| i.id.clone()).collect();
            }
            v if v == 'C' as u32 && mods.ctrl && !typing && !h.sel.is_empty() => acts.push(Act::Copy(h.sel.clone())),
            v if v == 'S' as u32 && mods.ctrl && !typing => {
                if let Some(e) = h.single() {
                    acts.push(if mods.shift { Act::SaveAs(e.id.clone()) } else { Act::Save(e.id.clone()) });
                }
            }
            v if v == 'E' as u32 && mods.ctrl && mods.shift && !typing && crate::share::supported() => {
                if let Some(e) = h.single() {
                    acts.push(Act::Share(e.id.clone()));
                }
            }
            v if v == 'O' as u32 && mods.ctrl && !typing => {
                if let Some(e) = h.single() {
                    acts.push(Act::OpenWith(e.id.clone()));
                }
            }
            v if v == 'U' as u32 && mods.ctrl && !typing => {
                if let Some(e) = h.single() {
                    acts.push(Act::Upload(e.id.clone()));
                }
            }
            v if v == 'F' as u32 && mods.ctrl => h.focus_search = true,
            key::HOME if !typing => h.scroll = 0.0,
            key::END if !typing => h.scroll = lay.max_scroll(),
            key::PAGEDOWN => h.scroll = (h.scroll + lay.area.h * 0.9).min(lay.max_scroll()),
            key::PAGEUP => h.scroll = (h.scroll - lay.area.h * 0.9).max(0.0),
            _ => {}
        }
    }
    h.scroll = h.scroll.clamp(0.0, lay.max_scroll());
    acts
}

fn set_slider(h: &mut History, lay: &Lay, x: f32) {
    let Some(s) = lay.slider else { return };
    let f = ((x - (s.x + 22.0)) / 80.0).clamp(0.0, 1.0);
    let (lo, hi) = super::THUMB_RANGE;
    h.thumb = (lo + f * (hi - lo)).round();
}

/// The marquee from `a` to `c` (content coordinates): every item it
/// touches joins the selection it started from.
pub fn marquee_select(h: &mut History, lay: &Lay, a: Pt, c: Pt) {
    let Some((_, _, base)) = h.ptr.marquee.clone() else { return };
    h.ptr.marquee = Some((a, c, base.clone()));
    let r = FRect::from_pts(a, c);
    let mut sel = base;
    for it in &lay.items {
        let b = it.r;
        let touches = b.x < r.x1() && r.x < b.x1() && b.y < r.y1() && r.y < b.y1();
        if touches && !sel.contains(&it.id) {
            sel.push(it.id.clone());
        }
    }
    h.sel = sel;
}

/// A click (press and release on `t`).
fn click(h: &mut History, lay: &Lay, t: Target, mods: crate::wind::Mods, moved: f32, now: Instant) -> Vec<Act> {
    let order: Vec<String> = lay.items.iter().map(|i| i.id.clone()).collect();
    let one = |h: &History| h.single().map(|e| e.id.clone());
    match t {
        Target::Filter(f) if h.entries.iter().len() > 0 => {
            h.filter = f;
            h.scroll = 0.0;
        }
        Target::View(v) => {
            h.view = v;
            h.scroll = 0.0;
        }
        Target::SearchClear => {
            h.search.set("");
            h.scroll = 0.0;
        }
        Target::PanelToggle => h.panel = !h.panel,
        Target::PanelClose => h.panel = false,
        Target::More => h.menu = if h.menu == Some(Menu::More) { None } else { Some(Menu::More) },
        Target::Item(id) if moved < 4.0 => {
            let double = h.ptr.last_click.as_ref().is_some_and(|(l, at)| *l == id && now.duration_since(*at) < Duration::from_millis(450));
            if double && !mods.ctrl && !mods.shift {
                h.ptr.last_click = None;
                if let Some(e) = h.entry(&id) {
                    return vec![if e.meta.kind.is_recording() { Act::Open(id) } else { Act::Edit(id) }];
                }
                return Vec::new();
            }
            h.click(&id, mods.ctrl, mods.shift, &order);
            h.ptr.last_click = Some((id, now));
        }
        Target::Quick(id, q) => {
            return match q {
                Quick::Edit => vec![Act::Edit(id)],
                Quick::Open => vec![Act::Open(id)],
                Quick::Copy => vec![Act::Copy(vec![id])],
                Quick::Save => vec![Act::Save(id)],
                Quick::Upload => vec![Act::Upload(id)],
                Quick::Pin => vec![Act::Pin(id)],
                Quick::Folder => vec![Act::ShowInFolder(id)],
                Quick::More => {
                    let m = Some(Menu::Card(id));
                    h.menu = if h.menu == m { None } else { m };
                    Vec::new()
                }
            };
        }
        Target::PanelEdit => {
            if let Some(id) = one(h) {
                let rec = h.entry(&id).is_some_and(|e| e.meta.kind.is_recording());
                return vec![if rec { Act::Open(id) } else { Act::Edit(id) }];
            }
        }
        Target::PanelOpen => {
            if let Some(id) = one(h) {
                let rec = h.entry(&id).is_some_and(|e| e.meta.kind.is_recording());
                return vec![if rec { Act::ShowInFolder(id) } else { Act::Open(id) }];
            }
        }
        Target::PanelDelete => {
            if let Some(id) = one(h) {
                return h.delete(vec![id]);
            }
        }
        Target::PanelLink => {
            if let Some(id) = one(h) {
                return vec![Act::CopyLink(id)];
            }
        }
        Target::BarCopy => return vec![Act::Copy(h.sel.clone())],
        Target::BarSave => return vec![Act::SaveToFolder(h.sel.clone())],
        Target::BarDelete => {
            let ids = h.sel.clone();
            return h.delete(ids);
        }
        Target::BarClear => h.sel.clear(),
        Target::MenuItem(a) => {
            let card = match h.menu.take() {
                Some(Menu::Card(id)) => Some(id),
                _ => None,
            };
            return match (a, card) {
                (MenuAct::Clear, _) => {
                    h.clear = Some(false);
                    Vec::new()
                }
                (MenuAct::OpenFolder, _) => vec![Act::OpenFolder],
                (MenuAct::Settings, _) => vec![Act::Settings],
                (MenuAct::SaveAs, Some(id)) => vec![Act::SaveAs(id)],
                (MenuAct::OpenWith, Some(id)) => vec![Act::OpenWith(id)],
                (MenuAct::Share, Some(id)) => vec![Act::Share(id)],
                (MenuAct::CopyLink, Some(id)) => vec![Act::CopyLink(id)],
                (MenuAct::DeleteUpload, Some(id)) => {
                    h.ask_unupload = Some(id);
                    Vec::new()
                }
                (MenuAct::ShowInFolder, Some(id)) => vec![Act::ShowInFolder(id)],
                (MenuAct::CopyPath, Some(id)) => vec![Act::CopyPath(id)],
                (MenuAct::Delete, Some(id)) => vec![Act::Delete(vec![id])],
                _ => Vec::new(),
            };
        }
        Target::ClearCheck => h.clear = h.clear.map(|v| !v),
        Target::ClearCancel => h.dialog_cancel(),
        Target::ClearOk => return h.dialog_ok(),
        Target::Settings => return vec![Act::Settings],
        Target::TurnOn => return vec![Act::TurnOn],
        Target::OffClear => h.clear = Some(false),
        Target::Space | Target::Search | Target::Inert | Target::Slider | Target::Filter(_) | Target::Item(_) => {}
    }
    Vec::new()
}

/// One frame: layout, input, paint. Returns the actions and whether the
/// kit wants another frame.
pub fn frame(canvas: &mut PixBuf, th: &crate::theme::Theme, inp: &Input, focus: &mut FocusState, h: &mut History, k: f32, now: Instant) -> (Vec<Act>, bool) {
    let (w, hh) = canvas.dimensions();
    let (lw, lh) = (w as f32 / k, hh as f32 / k);
    let area = FRect { x: 0.0, y: 0.0, w: lw, h: lh };
    let mut clip = crate::ui::SystemClipboard;
    let fb = crate::uifb::Fb::new(canvas.as_raw_mut(), w as usize);
    let mut ui = Ui::new(fb, th, inp, focus, &mut clip, k, area);
    if h.focus_search {
        h.focus_search = false;
        ui.focus.focus("history.search");
    }
    let lay = draw::layout(&ui, h, lw, lh);
    h.scroll = h.scroll.clamp(0.0, lay.max_scroll());
    let m = (inp.mouse.x / k, inp.mouse.y / k);
    let targets = draw::targets(&ui, &lay, h, m);
    h.search_focused = ui.focus.is_focused("history.search");
    // Esc in a question answers it only (it is not the window's Esc too).
    let asked = h.asking();
    let acts = input(h, &lay, &targets, inp, k, now);
    // Input may have changed what is shown: lay out again for the paint.
    let lay = draw::layout(&ui, h, lw, lh);
    h.scroll = h.scroll.clamp(0.0, lay.max_scroll());
    let targets = draw::targets(&ui, &lay, h, m);
    let hover = if inp.held && !matches!(h.ptr.press, Some((Press::On(_), _))) { None } else { draw::hit(&targets, m.0, m.1) };
    let hover = match (&h.ptr.press, hover) {
        // While a press is held only its target is hot.
        (Some((Press::On(t), _)), Some(hv)) if inp.held && *t != hv => None,
        (_, hv) => hv,
    };
    h.cursor_text = matches!(hover, Some(Target::Search));
    draw::paint(&mut ui, h, &lay, hover.as_ref(), now);
    h.hover = hover;
    let out = ui.finish();
    let mut acts = acts;
    if out.escape && !asked && !h.escape() {
        // Esc with nothing to clear does nothing (the window stays).
    }
    let _ = &mut acts;
    (acts, out.redraw)
}

/// The DELETE request (tests never send one).
fn delete_http(host: &str, path: &str, headers: &[(&str, &str)]) -> Result<(u32, String), String> {
    #[cfg(test)]
    {
        let _ = (host, path, headers);
        Err("no network in tests".into())
    }
    #[cfg(not(test))]
    crate::export::http_delete(host, path, headers)
}

/// Save to folder…: the ids and the folder picked (None: cancelled).
type Picked = (Vec<String>, Option<std::path::PathBuf>);

/// A listing of the store and the generation it was made at.
type Listing = (u64, Vec<crate::history::Entry>);

/// The window driver.
pub struct Window {
    hwnd: Hwnd,
    h: History,
    input: Input,
    focus: FocusState,
    canvas: PixBuf,
    out: PixBuf,
    pub(super) rx: Receiver<Msg>,
    /// Worker results.
    done: (Sender<super::Done>, Receiver<super::Done>),
    /// The folder picker's answer for Save to folder… (the ids).
    picked: (Sender<Picked>, Receiver<Picked>),
    busy: u32,
    checked: Instant,
    /// A listing runs on a worker; its result (the generation then).
    scanning: bool,
    scanned: (Sender<Listing>, Receiver<Listing>),
}

impl Window {
    pub fn new(h: History, rx: Receiver<Msg>) -> Window {
        Window {
            hwnd: Hwnd::default(),
            h,
            input: Input::default(),
            focus: FocusState::default(),
            canvas: PixBuf::default(),
            out: PixBuf::default(),
            rx,
            done: std::sync::mpsc::channel(),
            picked: std::sync::mpsc::channel(),
            busy: 0,
            checked: Instant::now(),
            scanning: false,
            scanned: std::sync::mpsc::channel(),
        }
    }

    fn k(&self) -> f32 {
        wind::scale(self.hwnd)
    }

    fn render(&mut self) {
        for _ in 0..4 {
            if self.canvas.width() == 0 {
                return;
            }
            let th = crate::theme::resolve(&self.h.cfg);
            let k = self.k();
            let (acts, redraw) = frame(&mut self.canvas, &th, &self.input, &mut self.focus, &mut self.h, k, Instant::now());
            self.input.end_frame();
            let any = !acts.is_empty();
            for a in acts {
                self.perform(a);
            }
            if !(any || redraw) {
                return;
            }
        }
    }

    fn spawn(&mut self, f: impl FnOnce() -> super::Done + Send + 'static) {
        let tx = self.done.0.clone();
        self.busy += 1;
        crate::busy::spawn(move || {
            let _ = tx.send(f());
        });
    }

    fn perform(&mut self, a: Act) {
        let store = self.h.store.clone();
        let cfg = self.h.cfg.clone();
        match a {
            Act::Edit(id) => super::open_edit(&id),
            Act::Open(id) => {
                if let Some(e) = self.h.entry(&id).cloned() {
                    self.spawn(move || match super::file_of(&store, &e) {
                        Ok(f) => {
                            crate::actions::open_path(&f);
                            super::Done { text: String::new(), ..super::ok_done() }
                        }
                        Err(err) => super::fail_done(format!("Could not open the capture: {err:#}")),
                    });
                }
            }
            Act::ShowInFolder(id) => {
                if let Some(e) = self.h.entry(&id) {
                    match super::file_of(&store, e) {
                        Ok(f) => crate::notice::show_in_folder(&f),
                        Err(err) => self.h.note(format!("Could not show the capture: {err:#}"), crate::editor::style::ToastKind::Error),
                    }
                }
            }
            Act::CopyPath(id) => {
                if let Some(e) = self.h.entry(&id) {
                    let f = match super::file_of(&store, e) {
                        Ok(f) => f,
                        Err(err) => {
                            self.h.note(format!("Copy failed: {err:#}"), crate::editor::style::ToastKind::Error);
                            return;
                        }
                    };
                    match crate::export::copy_text_to_clipboard(&f.display().to_string()) {
                        Ok(()) => self.h.note("File path copied", crate::editor::style::ToastKind::Success),
                        Err(e) => self.h.note(format!("Copy failed: {e:#}"), crate::editor::style::ToastKind::Error),
                    }
                }
            }
            Act::CopyLink(id) => {
                if let Some(u) = self.h.entry(&id).and_then(|e| e.meta.uploaded.clone()) {
                    match crate::export::copy_text_to_clipboard(&u) {
                        Ok(()) => self.h.note("Link copied", crate::editor::style::ToastKind::Success),
                        Err(e) => self.h.note(format!("Copy failed: {e:#}"), crate::editor::style::ToastKind::Error),
                    }
                }
            }
            Act::Copy(ids) => {
                // Several pictures cannot share the clipboard: the first.
                if let Some(id) = ids.first().cloned() {
                    self.spawn(move || super::run_export(&store, &id, crate::export::Task::Copy, &cfg));
                }
            }
            Act::Save(id) => self.spawn(move || super::run_export(&store, &id, crate::export::Task::Save { path: None, ask: false }, &cfg)),
            Act::SaveAs(id) => self.spawn(move || super::run_export(&store, &id, crate::export::Task::Save { path: None, ask: true }, &cfg)),
            Act::Upload(id) => {
                self.h.note("Uploading\u{2026}", crate::editor::style::ToastKind::Info);
                self.spawn(move || super::run_export(&store, &id, crate::export::Task::Upload, &cfg))
            }
            Act::Pin(id) => self.spawn(move || super::run_export(&store, &id, crate::export::Task::Pin, &cfg)),
            Act::OpenWith(id) => {
                // The chooser is owned by this window.
                #[allow(clippy::unnecessary_cast)] // a pointer, a u64 or a usize
                let owner = Some(self.hwnd.0 as usize);
                self.spawn(move || super::open_with(&store, &id, &cfg, owner))
            }
            Act::Share(id) => {
                // The sheet comes up over this window.
                #[allow(clippy::unnecessary_cast)] // a pointer, a u64 or a usize
                let owner = Some(self.hwnd.0 as usize);
                self.spawn(move || super::share(&store, &id, &cfg, owner))
            }
            Act::DeleteUpload(id) => {
                self.h.note("Deleting upload\u{2026}", crate::editor::style::ToastKind::Info);
                self.spawn(move || {
                    let del = |h: &str, p: &str, hd: &[(&str, &str)]| delete_http(h, p, hd);
                    let s3 = crate::uploads::s3_delete_with(&cfg);
                    super::delete_upload(&store, &id, &cfg.upload_client_id, &del, &s3)
                })
            }
            Act::Delete(ids) => {
                let n = ids.iter().filter(|id| self.h.store.delete(id, false).is_ok()).count();
                crate::history::changed();
                self.h.sel.retain(|s| !ids.contains(s));
                self.h.reload();
                self.h.note(format!("Deleted {n} capture{}", if n == 1 { "" } else { "s" }), crate::editor::style::ToastKind::Info);
            }
            Act::SaveToFolder(ids) => {
                let tx = self.picked.0.clone();
                let start = crate::export::default_save_dir(&cfg);
                #[allow(clippy::unnecessary_cast)] // a pointer, a u64 or a usize
                let owner = self.hwnd.0 as usize;
                std::thread::spawn(move || {
                    #[cfg(windows)]
                    let h = (owner != 0).then_some(windows::Win32::Foundation::HWND(owner as *mut core::ffi::c_void));
                    #[cfg(not(windows))]
                    let h = (owner != 0).then_some(Hwnd(owner as _));
                    let _ = tx.send((ids, crate::ui::folder_dialog::pick_folder(h, &start)));
                });
            }
            Act::Clear(files) => {
                let n = self.h.store.clear(files);
                if files {
                    // uploads.json too (uploads made with History off).
                    crate::uploads::forget_book();
                }
                crate::history::changed();
                self.h.sel.clear();
                self.h.reload();
                self.h.note(format!("Cleared {n} capture{}", if n == 1 { "" } else { "s" }), crate::editor::style::ToastKind::Info);
            }
            Act::OpenFolder => {
                let root = self.h.store.root.clone();
                let _ = std::fs::create_dir_all(&root);
                std::thread::spawn(move || crate::actions::open_path(&root));
            }
            Act::Settings => crate::settings_ui::show_at(crate::settings_ui::HISTORY),
            Act::TurnOn => {
                let path = crate::config::config_path();
                match crate::config::edit_top_keys_at(&path, &[("history".into(), Some("true".into()))]) {
                    Ok(()) => {
                        self.h.cfg.history = true;
                        crate::settings_ui::tell_daemon_reload();
                        self.h.reload();
                    }
                    Err(e) => self.h.note(format!("Could not turn History on: {e}"), crate::editor::style::ToastKind::Error),
                }
            }
        }
    }

    /// Timer: worker results, a changed history, the toast's fade.
    fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Raise => {
                    #[cfg(windows)]
                    wind::raise(self.hwnd);
                }
                Msg::Clear => {
                    if !self.h.entries.is_empty() {
                        self.h.clear = Some(false);
                        changed = true;
                    }
                }
            }
        }
        while let Ok(d) = self.done.1.try_recv() {
            self.busy = self.busy.saturating_sub(1);
            if !d.text.is_empty() {
                self.h.note(d.text, d.kind);
            }
            if d.changed {
                crate::history::changed();
            }
            changed = true;
        }
        while let Ok((ids, dir)) = self.picked.1.try_recv() {
            if let Some(dir) = dir {
                let store = self.h.store.clone();
                self.spawn(move || super::save_to_folder(&store, &ids, &dir));
            }
        }
        // The store is listed again on a worker (thousands of entries on a
        // slow disk take a while): when it changed here, and every 5 s
        // (other processes).
        if !self.scanning && (crate::history::generation() != self.h.listed || self.checked.elapsed() > Duration::from_secs(5)) {
            self.checked = Instant::now();
            self.scanning = true;
            let (store, tx, generation) = (self.h.store.clone(), self.scanned.0.clone(), crate::history::generation());
            std::thread::spawn(move || {
                let _ = tx.send((generation, store.list()));
            });
        }
        while let Ok((generation, entries)) = self.scanned.1.try_recv() {
            self.scanning = false;
            self.h.listed = generation;
            let t = crate::export::local_ymdhms();
            let today = (t.0, t.1, t.2);
            if entries != self.h.entries || today != self.h.today {
                self.h.today = today;
                self.h.take(entries);
                changed = true;
            }
        }
        let toast_live = self.h.toast.as_ref().is_some_and(|(_, _, at)| at.elapsed() < Duration::from_millis(2500));
        if self.h.toast.is_some() && !toast_live {
            self.h.toast = None;
            changed = true;
        }
        wind::set_fast_timer(self.hwnd, toast_live);
        changed || toast_live
    }
}

impl Driver for Window {
    fn on_create(&mut self, hwnd: Hwnd) {
        self.hwnd = hwnd;
    }

    fn on_event(&mut self, ev: Ev) -> bool {
        match ev {
            Ev::Resize(w, h) => {
                self.canvas = PixBuf::new(w.max(1), h.max(1));
                self.render();
            }
            Ev::Close => wind::close(self.hwnd),
            Ev::Timer => {
                if self.tick() {
                    self.render();
                    return true;
                }
                return false;
            }
            Ev::Focus(true) => {
                // Settings may have changed meanwhile.
                self.h.cfg = crate::config::load();
                self.input.feed(&ev);
                self.render();
            }
            Ev::Move { .. } => {
                self.input.feed(&ev);
                self.render();
                wind::invalidate(self.hwnd);
            }
            _ => {
                self.input.feed(&ev);
                self.render();
            }
        }
        true
    }

    fn frame(&mut self) -> Option<&mut PixBuf> {
        if self.canvas.width() == 0 {
            return None;
        }
        if self.out.dimensions() != self.canvas.dimensions() {
            self.out = PixBuf::new(self.canvas.width(), self.canvas.height());
        }
        self.out.as_raw_mut().copy_from_slice(self.canvas.as_raw());
        Some(&mut self.out)
    }

    fn cursor(&self) -> Cursor {
        if self.h.cursor_text { Cursor::IBeam } else { Cursor::Arrow }
    }

    fn frame_colors(&self) -> Option<wind::FrameColors> {
        Some(crate::ui::paint::WTok::of(&crate::theme::resolve(&self.h.cfg)).frame())
    }
}
