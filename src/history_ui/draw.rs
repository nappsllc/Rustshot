//! Laying out, hit-testing and painting the History window as on the
//! boards: the 48 px header, the scrolling grid or list (day headers 30
//! px, cards 16 apart at 24 px from the sides, rows 56 px), the 320 px
//! details panel, the 28 px footer, the action bar, menus and the
//! "Clear history?" dialog. Everything is laid out in logical px
//! ([`Lay`]); [`hit`] finds what the pointer is on; [`paint`] draws.

use super::{History, Menu, View};
use crate::history::{self, Entry, Filter, Kind};
use crate::objects::FRect;
use crate::ui::Ui;
use crate::ui::compact::CSS;
use crate::ui::paint::{self, WTok};
use crate::uifb::{Fb, C4};

pub const HDR_H: f32 = 48.0;
pub const FOOT_H: f32 = 28.0;
pub const PANEL_W: f32 = 320.0;
pub const DAY_H: f32 = 30.0;
pub const PAD_X: f32 = 24.0;
pub const GAP: f32 = 16.0;
pub const ROW_H: f32 = 56.0;
pub const META_H: f32 = 7.0 + 15.0 + 2.0 + 14.0;
pub const STRIP_H: f32 = 36.0;
const SEARCH_W: f32 = 280.0;
const SEG_H: f32 = 28.0;
const IB: f32 = 28.0;
const MENU_W: f32 = 210.0;
const MENU_ROW: f32 = 30.0;
const DLG_W: f32 = 400.0;

/// What a quick button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quick {
    Edit,
    Copy,
    Save,
    Upload,
    Pin,
    Open,
    Folder,
    More,
}

impl Quick {
    pub fn icon(self) -> &'static str {
        match self {
            Quick::Edit => "pencil",
            Quick::Copy => "copy",
            Quick::Save => "save",
            Quick::Upload => "cloud",
            Quick::Pin => "pin",
            Quick::Open => "play",
            Quick::Folder => "folder",
            Quick::More => "more",
        }
    }

    /// The tooltip: label and key.
    pub fn tip(self) -> (&'static str, &'static str) {
        match self {
            Quick::Edit => ("Edit", "E"),
            Quick::Copy => ("Copy", "Ctrl+C"),
            Quick::Save => ("Save", "Ctrl+S"),
            Quick::Upload => ("Upload", "Ctrl+U"),
            Quick::Pin => ("Pin to screen", "P"),
            Quick::Open => ("Open", "Enter"),
            Quick::Folder => ("Show in folder", ""),
            Quick::More => ("More", ""),
        }
    }
}

/// What the pointer can be on.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Search,
    SearchClear,
    Filter(Filter),
    View(View),
    Slider,
    PanelToggle,
    More,
    /// A card or row.
    Item(String),
    Quick(String, Quick),
    /// The details panel.
    PanelClose,
    PanelEdit,
    PanelOpen,
    PanelDelete,
    PanelLink,
    /// The action bar.
    BarCopy,
    BarSave,
    BarDelete,
    BarClear,
    /// A menu's row.
    MenuItem(MenuAct),
    /// The question's parts.
    ClearCheck,
    ClearCancel,
    ClearOk,
    /// The empty state's link, the off state's switch and its "Clear
    /// history…" (captures still kept).
    Settings,
    TurnOn,
    OffClear,
    /// The scrolling area's empty space (a marquee starts here).
    Space,
    /// Anywhere else (the panel's body, a dialog's body).
    Inert,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAct {
    Clear,
    OpenFolder,
    Settings,
    SaveAs,
    OpenWith,
    Share,
    ShowInFolder,
    CopyPath,
    CopyLink,
    DeleteUpload,
    Delete,
}

impl MenuAct {
    pub fn label(self) -> &'static str {
        match self {
            MenuAct::Clear => "Clear history\u{2026}",
            MenuAct::OpenFolder => "Open history folder",
            MenuAct::Settings => "Settings\u{2026}",
            MenuAct::SaveAs => "Save As\u{2026}",
            MenuAct::OpenWith => "Open with\u{2026}",
            MenuAct::Share => "Share\u{2026}",
            MenuAct::CopyLink => "Copy link",
            MenuAct::DeleteUpload => "Delete upload\u{2026}",
            MenuAct::ShowInFolder => "Show in folder",
            MenuAct::CopyPath => "Copy file path",
            MenuAct::Delete => "Delete",
        }
    }
}

/// One card or row, in content coordinates (y from the top of the
/// scrolling area, before the scroll).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemLay {
    pub id: String,
    pub idx: usize,
    /// The card (thumb and meta) or the row.
    pub r: FRect,
    /// The picture's box: the card's thumb, the row's 72 × 48.
    pub thumb: FRect,
    pub hl: Vec<std::ops::Range<usize>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupLay {
    pub y: f32,
    pub title: String,
    pub small: Option<String>,
}

/// The window laid out (logical px).
#[derive(Clone, Debug, PartialEq)]
pub struct Lay {
    pub w: f32,
    pub h: f32,
    pub hdr: FRect,
    /// The scrolling area.
    pub area: FRect,
    pub panel: Option<FRect>,
    pub foot: Option<FRect>,
    pub search: Option<FRect>,
    pub filters: Vec<(FRect, Filter)>,
    pub views: Vec<(FRect, View)>,
    pub slider: Option<FRect>,
    pub panel_btn: Option<FRect>,
    pub more: FRect,
    pub groups: Vec<GroupLay>,
    pub items: Vec<ItemLay>,
    pub content_h: f32,
    pub cols: usize,
    pub thumb_h: f32,
}

impl Lay {
    /// The most the area scrolls.
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.area.h).max(0.0)
    }

    /// Content rect `r` in window coordinates at `scroll`.
    pub fn on_screen(&self, r: FRect, scroll: f32) -> FRect {
        FRect { x: self.area.x + r.x, y: self.area.y + r.y - scroll, ..r }
    }
}

/// The filter labels.
pub const FILTERS: [(Filter, &str); 4] = [(Filter::All, "All"), (Filter::Shots, "Screenshots"), (Filter::Recordings, "Recordings"), (Filter::Uploaded, "Uploaded")];

fn seg_item_w(ui: &Ui, label: &str) -> f32 {
    (ui.text_w(11.5, label) + 18.0).ceil()
}

/// Lay the window out at `w` × `h` (logical px).
pub fn layout(ui: &Ui, h: &History, w: f32, hh: f32) -> Lay {
    let hdr = FRect { x: 0.0, y: 0.0, w, h: HDR_H };
    let on = h.on();
    let foot = on.then_some(FRect { x: 0.0, y: hh - FOOT_H, w, h: FOOT_H });
    let panel_w = if on && h.panel && h.single().is_some() && w >= super::PANEL_MIN_W { PANEL_W } else { 0.0 };
    let body_h = hh - HDR_H - foot.map_or(0.0, |f| f.h);
    let area = FRect { x: 0.0, y: HDR_H, w: w - panel_w, h: body_h.max(0.0) };
    let panel = (panel_w > 0.0).then_some(FRect { x: w - PANEL_W, y: HDR_H, w: PANEL_W, h: body_h.max(0.0) });
    let cy = HDR_H / 2.0;
    // Header, left to right: title, search, filter, view; right: slider,
    // the panel toggle, ⋯.
    let title_w = ui.text_w(14.0, "History") + 6.0;
    let mut x = 18.0 + title_w + 10.0;
    let more = FRect { x: w - 14.0 - IB, y: cy - IB / 2.0, w: IB, h: IB };
    let mut right = more.x - 10.0;
    let empty = h.entries.is_empty();
    let (mut search, mut filters, mut views, mut slider, mut panel_btn) = (None, Vec::new(), Vec::new(), None, None);
    if on {
        if !empty {
            let pb = FRect { x: right - IB, y: cy - IB / 2.0, w: IB, h: IB };
            panel_btn = Some(pb);
            right = pb.x - 10.0;
            if w >= super::PANEL_MIN_W {
                let s = FRect { x: right - 102.0, y: cy - 8.0, w: 102.0, h: 16.0 };
                slider = (h.view == View::Grid).then_some(s);
                right = s.x - 10.0;
            }
        }
        let segs: f32 = FILTERS.iter().map(|(_, l)| seg_item_w(ui, l)).sum::<f32>() + 4.0 + 6.0;
        let view_w = if empty { 0.0 } else { 2.0 * 32.0 + 2.0 + 6.0 + 10.0 };
        let room = right - x - 10.0 - segs - view_w;
        let sw = room.clamp(140.0, SEARCH_W);
        search = Some(FRect { x, y: cy - 14.0, w: sw, h: 28.0 });
        x += sw + 10.0;
        let mut sx = x + 3.0;
        for (f, l) in FILTERS {
            let iw = seg_item_w(ui, l);
            filters.push((FRect { x: sx, y: cy - 11.0, w: iw, h: 22.0 }, f));
            sx += iw + 2.0;
        }
        x = sx - 2.0 + 3.0 + 10.0;
        if !empty {
            let mut vx = x + 3.0;
            for v in [View::Grid, View::List] {
                views.push((FRect { x: vx, y: cy - 11.0, w: 32.0, h: 22.0 }, v));
                vx += 34.0;
            }
        }
    }
    // The content.
    let groups = h.groups();
    let mut gl = Vec::new();
    let mut items = Vec::new();
    let avail = (area.w - 2.0 * PAD_X).max(60.0);
    let cols = (((avail + GAP) / (h.thumb + GAP)).floor() as usize).max(1);
    let cw = ((avail - GAP * (cols - 1) as f32) / cols as f32).floor();
    let thumb_h = (h.thumb * 0.7).round();
    let mut y = 0.0;
    for g in &groups {
        gl.push(GroupLay { y, title: g.title.clone(), small: g.small.clone() });
        y += DAY_H;
        match h.view {
            View::Grid => {
                y += 4.0;
                let card_h = thumb_h + META_H;
                for (n, (idx, hl)) in g.items.iter().enumerate() {
                    let (c, r) = (n % cols, n / cols);
                    let cx = PAD_X + c as f32 * (cw + GAP);
                    let top = y + r as f32 * (card_h + GAP);
                    let card = FRect { x: cx, y: top, w: cw, h: card_h };
                    let thumb = FRect { x: cx, y: top, w: cw, h: thumb_h };
                    items.push(ItemLay { id: h.entries[*idx].id.clone(), idx: *idx, r: card, thumb, hl: hl.clone() });
                }
                let rows = g.items.len().div_ceil(cols);
                y += rows as f32 * card_h + rows.saturating_sub(1) as f32 * GAP + 18.0;
            }
            View::List => {
                for (idx, hl) in &g.items {
                    let r = FRect { x: 0.0, y, w: area.w, h: ROW_H };
                    let thumb = FRect { x: PAD_X, y: y + 4.0, w: 72.0, h: 48.0 };
                    items.push(ItemLay { id: h.entries[*idx].id.clone(), idx: *idx, r, thumb, hl: hl.clone() });
                    y += ROW_H;
                }
            }
        }
    }
    Lay {
        w,
        h: hh,
        hdr,
        area,
        panel,
        foot,
        search,
        filters,
        views,
        slider,
        panel_btn,
        more,
        groups: gl,
        items,
        content_h: y + 12.0,
        cols,
        thumb_h,
    }
}

/// The quick buttons of a card (the strip at the thumb's bottom) or a
/// row (right-aligned), in window coordinates.
pub fn quick(lay: &Lay, it: &ItemLay, e: &Entry, scroll: f32, list: bool) -> Vec<(FRect, Quick)> {
    let rec = e.meta.kind.is_recording();
    let acts: &[Quick] = match (list, rec) {
        (false, false) => &[Quick::Edit, Quick::Copy, Quick::Save, Quick::Upload, Quick::Pin, Quick::More],
        (false, true) => &[Quick::Open, Quick::Copy, Quick::Folder, Quick::More],
        (true, false) => &[Quick::Edit, Quick::Copy, Quick::Save, Quick::More],
        (true, true) => &[Quick::Open, Quick::Copy, Quick::Folder, Quick::More],
    };
    if list {
        let r = lay.on_screen(it.r, scroll);
        let mut x = r.x1() - 14.0 - 26.0;
        let mut out = Vec::new();
        for q in acts.iter().rev() {
            out.push((FRect { x, y: r.y + (r.h - 26.0) / 2.0, w: 26.0, h: 26.0 }, *q));
            x -= 28.0;
        }
        out.reverse();
        return out;
    }
    let t = lay.on_screen(it.thumb, scroll);
    let y = t.y1() - STRIP_H + 5.0;
    let mut x = t.x + 6.0;
    let mut out = Vec::new();
    for q in acts {
        if *q == Quick::More {
            out.push((FRect { x: t.x1() - 6.0 - 26.0, y, w: 26.0, h: 26.0 }, *q));
            continue;
        }
        let w = if *q == Quick::Edit { 54.0 } else { 26.0 };
        out.push((FRect { x, y, w, h: 26.0 }, *q));
        x += w + 2.0;
    }
    // A narrow card keeps the buttons that fit.
    out.retain(|(r, q)| *q == Quick::More || r.x1() <= t.x1() - 6.0 - 28.0);
    out
}

/// The details panel's parts (window coordinates).
pub struct PanelLay {
    pub close: FRect,
    pub preview: FRect,
    pub rows_y: f32,
    pub edit: FRect,
    pub open: FRect,
    pub delete: FRect,
}

pub fn panel_lay(p: FRect) -> PanelLay {
    let by = p.y1() - 56.0;
    PanelLay {
        close: FRect { x: p.x1() - 10.0 - IB, y: p.y + 6.0, w: IB, h: IB },
        preview: FRect { x: p.x + 16.0, y: p.y + 40.0 + 14.0, w: p.w - 32.0, h: 160.0 },
        rows_y: p.y + 40.0 + 14.0 + 160.0 + 10.0,
        edit: FRect { x: p.x + 16.0, y: by + 14.0, w: 122.0, h: 28.0 },
        open: FRect { x: p.x + 16.0 + 122.0 + 8.0, y: by + 14.0, w: 122.0, h: 28.0 },
        delete: FRect { x: p.x1() - 16.0 - 36.0, y: by + 14.0, w: 36.0, h: 28.0 },
    }
}

/// The multi-select bar (window coordinates): the bar, Copy, Save to
/// folder…, Delete, ×.
pub fn bar_lay(ui: &Ui, lay: &Lay, n: usize) -> (FRect, [FRect; 4], String) {
    let label = format!("{n} selected");
    let bw = |l: &str| (ui.text_w(12.0, l) + 24.0 + 22.0).ceil();
    let (c, s, d) = (bw("Copy"), bw("Save to folder\u{2026}"), bw("Delete"));
    let lw = ui.text_w(12.5, &label).ceil() + 4.0;
    let w = 14.0 + lw + 8.0 + c + 8.0 + s + 8.0 + d + 8.0 + IB + 6.0;
    let r = FRect { x: lay.area.x + (lay.area.w - w) / 2.0, y: lay.area.y1() - 16.0 - 40.0, w, h: 40.0 };
    let mut x = r.x + 14.0 + lw + 8.0;
    let cy = r.y + 20.0;
    let mut b = |bw: f32| {
        let q = FRect { x, y: cy - 14.0, w: bw, h: 28.0 };
        x += bw + 8.0;
        q
    };
    let (rc, rs, rd, rx) = (b(c), b(s), b(d), b(IB));
    (r, [rc, rs, rd, rx], label)
}

/// A menu's rows (window coordinates).
pub fn menu_lay(lay: &Lay, h: &History, items: &[(FRect, Target)]) -> Option<(FRect, Vec<(FRect, MenuAct)>)> {
    let m = h.menu.as_ref()?;
    let (acts, anchor): (Vec<MenuAct>, FRect) = match m {
        Menu::More => (vec![MenuAct::Clear, MenuAct::OpenFolder, MenuAct::Settings], lay.more),
        Menu::Card(id) => {
            let a = items.iter().find(|(_, t)| *t == Target::Quick(id.clone(), Quick::More)).map(|x| x.0)?;
            let e = h.entry(id)?;
            (card_acts(h, e), a)
        }
    };
    let acts = &acts[..];
    let mh = 12.0 + acts.len() as f32 * (MENU_ROW + 1.0);
    let x = (anchor.x1() - MENU_W).clamp(4.0, (lay.w - MENU_W - 4.0).max(4.0));
    let below = anchor.y1() + 6.0;
    let y = if below + mh <= lay.h - 4.0 { below } else { (anchor.y - 6.0 - mh).max(4.0) };
    let r = FRect { x, y, w: MENU_W, h: mh };
    let rows = acts.iter().enumerate().map(|(i, a)| (FRect { x: x + 6.0, y: y + 6.0 + i as f32 * (MENU_ROW + 1.0), w: MENU_W - 12.0, h: MENU_ROW }, *a)).collect();
    Some((r, rows))
}

/// A card's ⋯ menu: Save As (screenshots), Open with, Share (Windows,
/// macOS), Show in folder, Copy file path, then Copy link and Delete
/// upload… when it was
/// uploaded (the latter when its deletehash was kept), Delete.
pub fn card_acts(h: &History, e: &Entry) -> Vec<MenuAct> {
    let mut v = Vec::new();
    if !e.meta.kind.is_recording() {
        v.push(MenuAct::SaveAs);
    }
    v.push(MenuAct::OpenWith);
    if crate::share::supported() {
        v.push(MenuAct::Share);
    }
    v.extend([MenuAct::ShowInFolder, MenuAct::CopyPath]);
    if e.meta.uploaded.is_some() {
        v.push(MenuAct::CopyLink);
        if h.can_unupload(&e.id) {
            v.push(MenuAct::DeleteUpload);
        }
    }
    v.push(MenuAct::Delete);
    v
}

/// The "Clear history?" / "Delete N captures?" dialog: its rect, the
/// checkbox row (Clear only), Cancel, the button.
pub fn dialog_lay(ui: &Ui, lay: &Lay, h: &History) -> (FRect, FRect, FRect, FRect) {
    let (_, p, check, okl) = h.dialog_texts();
    let lines = crate::ui::controls::wrap(&p, DLG_W - 40.0, |s| ui.text_w(12.0, s)).len().max(1) as f32;
    let check_h = if check.is_some() { 18.0 + 10.0 } else { 0.0 };
    let dh = 20.0 + 18.0 + 10.0 + lines * 18.0 + 10.0 + check_h + 6.0 + 28.0 + 20.0;
    let r = FRect { x: ((lay.w - DLG_W) / 2.0).round(), y: ((lay.h - dh) / 2.0).round(), w: DLG_W, h: dh };
    let cb = FRect { x: r.x + 20.0, y: r.y + 20.0 + 18.0 + 10.0 + lines * 18.0 + 10.0, w: DLG_W - 40.0, h: 18.0 };
    let by = r.y1() - 20.0 - 28.0;
    let cw = (ui.text_w(12.0, okl) + 24.0).ceil();
    let ok = FRect { x: r.x1() - 20.0 - cw, y: by, w: cw, h: 28.0 };
    let kw = (ui.text_w(12.0, "Cancel") + 24.0).ceil();
    let cancel = FRect { x: ok.x - 8.0 - kw, y: by, w: kw, h: 28.0 };
    (r, cb, cancel, ok)
}

/// The empty state's "History settings" link and the off state's switch.
pub fn empty_lay(ui: &Ui, lay: &Lay, h: &History) -> FRect {
    let a = lay.area;
    let cy = a.y + a.h / 2.0;
    if !h.on() {
        let tw = ui.text_w(12.5, "Keep a history of captures");
        let w = 32.0 + 10.0 + tw;
        return FRect { x: a.x + (a.w - w) / 2.0, y: cy + 30.0, w, h: 18.0 };
    }
    let lw = ui.text_w(12.0, "History settings");
    FRect { x: a.x + (a.w - lw) / 2.0, y: cy + 46.0, w: lw, h: 18.0 }
}

/// The off state's caption.
const OFF_CAPTION: &str = "Captures aren't kept after you close them. Saved and uploaded files are not affected.";

/// The off state's "N captures still kept · Clear history…" while the
/// history still holds captures (they expire as usual): the text's left
/// end, its line's center y, and the link's rect.
pub fn off_kept_lay(ui: &Ui, lay: &Lay, h: &History) -> Option<(String, f32, f32, FRect)> {
    let n = h.entries.len();
    if h.on() || n == 0 {
        return None;
    }
    let a = lay.area;
    let lines = crate::ui::controls::wrap(OFF_CAPTION, a.w - 48.0, |s| ui.text_w(12.0, s)).len() as f32;
    let cy = empty_lay(ui, lay, h).y1() + 18.0 + lines * 18.0 + 12.0;
    let pre = format!("{n} capture{} still kept \u{b7} ", if n == 1 { "" } else { "s" });
    let link = "Clear history\u{2026}";
    let (pw, lw) = (ui.text_w(12.0, &pre), ui.text_w(12.0, link));
    let x = a.x + (a.w - pw - lw) / 2.0;
    Some((pre, x, cy, FRect { x: x + pw, y: cy - 9.0, w: lw, h: 18.0 }))
}

fn inside(r: FRect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x1() && y < r.y1()
}

/// Every target, topmost last (window coordinates).
pub fn targets(ui: &Ui, lay: &Lay, h: &History, mouse: (f32, f32)) -> Vec<(FRect, Target)> {
    let mut t = Vec::new();
    if let Some(s) = lay.search {
        t.push((s, Target::Search));
        if !h.search.text.is_empty() {
            t.push((FRect { x: s.x1() - 26.0, y: s.y + 4.0, w: 20.0, h: 20.0 }, Target::SearchClear));
        }
    }
    for (r, f) in &lay.filters {
        t.push((*r, Target::Filter(*f)));
    }
    for (r, v) in &lay.views {
        t.push((*r, Target::View(*v)));
    }
    if let Some(s) = lay.slider {
        t.push((s, Target::Slider));
    }
    if let Some(p) = lay.panel_btn {
        t.push((p, Target::PanelToggle));
    }
    t.push((lay.more, Target::More));
    if h.on() && h.entries.is_empty() {
        t.push((empty_lay(ui, lay, h), Target::Settings));
    } else if !h.on() {
        t.push((empty_lay(ui, lay, h), Target::TurnOn));
        if let Some((_, _, _, link)) = off_kept_lay(ui, lay, h) {
            t.push((link, Target::OffClear));
        }
    } else {
        t.push((lay.area, Target::Space));
        let list = h.view == View::List;
        for it in &lay.items {
            let r = lay.on_screen(it.r, h.scroll);
            if r.y1() < lay.area.y || r.y > lay.area.y1() {
                continue;
            }
            t.push((clip_y(r, lay.area), Target::Item(it.id.clone())));
            let hovered = inside(r, mouse.0, mouse.1) && inside(lay.area, mouse.0, mouse.1);
            let menu_open = matches!(&h.menu, Some(Menu::Card(id)) if *id == it.id);
            if (hovered || menu_open)
                && h.sel.len() <= 1
                && let Some(e) = h.entries.get(it.idx)
            {
                for (q, a) in quick(lay, it, e, h.scroll, list) {
                    if q.y >= lay.area.y && q.y1() <= lay.area.y1() {
                        t.push((q, Target::Quick(it.id.clone(), a)));
                    }
                }
            }
        }
        if let Some(p) = lay.panel {
            t.push((p, Target::Inert));
            let pl = panel_lay(p);
            t.push((pl.close, Target::PanelClose));
            t.push((pl.edit, Target::PanelEdit));
            t.push((pl.open, Target::PanelOpen));
            t.push((pl.delete, Target::PanelDelete));
            if let Some(e) = h.single()
                && e.meta.uploaded.is_some()
            {
                // The "Uploaded" row's copy button.
                t.push((FRect { x: p.x + 16.0 + 98.0, y: pl.rows_y + 5.0 * 24.0, w: p.w - 32.0 - 98.0, h: 24.0 }, Target::PanelLink));
            }
        }
        if h.sel.len() > 1 {
            let (r, b, _) = bar_lay(ui, lay, h.sel.len());
            t.push((r, Target::Inert));
            t.push((b[0], Target::BarCopy));
            t.push((b[1], Target::BarSave));
            t.push((b[2], Target::BarDelete));
            t.push((b[3], Target::BarClear));
        }
    }
    let items = t.clone();
    if let Some((r, rows)) = menu_lay(lay, h, &items) {
        t.push((r, Target::Inert));
        t.extend(rows.into_iter().map(|(r, a)| (r, Target::MenuItem(a))));
    }
    if h.asking() {
        let (r, cb, cancel, ok) = dialog_lay(ui, lay, h);
        t.push((FRect { x: 0.0, y: 0.0, w: lay.w, h: lay.h }, Target::Inert));
        t.push((r, Target::Inert));
        if h.clear.is_some() {
            t.push((cb, Target::ClearCheck));
        }
        t.push((cancel, Target::ClearCancel));
        t.push((ok, Target::ClearOk));
    }
    t
}

fn clip_y(r: FRect, a: FRect) -> FRect {
    let y0 = r.y.max(a.y);
    let y1 = r.y1().min(a.y1());
    FRect { y: y0, h: (y1 - y0).max(0.0), ..r }
}

/// The topmost target under `p` (window coordinates).
pub fn hit(t: &[(FRect, Target)], x: f32, y: f32) -> Option<Target> {
    t.iter().rev().find(|(r, _)| inside(*r, x, y)).map(|(_, t)| t.clone())
}

// --- painting ----------------------------------------------------------------------

fn p(ui: &Ui, r: FRect) -> FRect {
    paint::px(r, ui.k)
}

fn fill(ui: &mut Ui, r: FRect, rad: f32, c: C4) {
    let q = p(ui, r);
    paint::fill(&mut ui.fb, q, rad * ui.k, c);
}

fn ring(ui: &mut Ui, r: FRect, rad: f32, c: C4) {
    let (q, k) = (p(ui, r), ui.k);
    paint::ring(&mut ui.fb, q, rad * k, k, c);
}

fn line_h(ui: &mut Ui, x: f32, y: f32, w: f32, c: C4) {
    let k = ui.k;
    ui.fb.fill_rect((x * k).round() as i32, (y * k).round() as i32, (w * k).round() as i32, k.round().max(1.0) as i32, c);
}

fn line_v(ui: &mut Ui, x: f32, y: f32, h: f32, c: C4) {
    let k = ui.k;
    ui.fb.fill_rect((x * k).round() as i32, (y * k).round() as i32, k.round().max(1.0) as i32, (h * k).round() as i32, c);
}

fn icon(ui: &mut Ui, name: &str, x: f32, y: f32, size: f32, c: C4) {
    let k = ui.k;
    crate::icon_path::draw_icon(&mut ui.fb, name, (x * k).round(), (y * k).round(), size * k, c);
}

/// Text at logical (x, cy): returns its width (logical px).
fn text(ui: &mut Ui, size: f32, s: &str, x: f32, cy: f32, c: C4, bold: bool) -> f32 {
    let k = ui.k;
    ui.text_at(size, s, (x * k).round(), cy * k, c, bold);
    ui.text_w(size, s)
}

fn caps(ui: &mut Ui, size: f32, s: &str, x: f32, cy: f32, c: C4) -> f32 {
    // Uppercase, tracked .07em.
    let mut cx = x;
    let up = s.to_uppercase();
    for ch in up.chars() {
        let t = ch.to_string();
        text(ui, size, &t, cx, cy, c, true);
        cx += ui.text_w(size, &t) + size * 0.07;
    }
    cx - x
}

/// The window, everything; `hover`: the target under the pointer.
pub fn paint(ui: &mut Ui, h: &mut History, lay: &Lay, hover: Option<&Target>, now: std::time::Instant) {
    let t = WTok::of(ui.theme);
    fill(ui, FRect { x: 0.0, y: 0.0, w: lay.w, h: lay.h }, 0.0, t.win);
    paint_body(ui, h, lay, &t, hover);
    if let Some(p) = lay.panel {
        paint_panel(ui, h, p, &t, hover);
    }
    paint_header(ui, h, lay, &t, hover);
    if let Some(f) = lay.foot {
        paint_footer(ui, h, f, &t);
    }
    if h.sel.len() > 1 && h.on() {
        paint_bar(ui, h, lay, &t, hover);
    }
    // The tooltip over a quick button, over everything else.
    if let Some(Target::Quick(id, q)) = hover
        && let Some(it) = lay.items.iter().find(|i| i.id == *id)
        && let Some(e) = h.entries.get(it.idx)
        && let Some((r, _)) = quick(lay, it, e, h.scroll, h.view == View::List).into_iter().find(|(_, a)| a == q)
    {
        tooltip(ui, r, *q, &t, lay);
    }
    if let Some((msg, kind, at)) = &h.toast {
        let age = now.saturating_duration_since(*at).as_secs_f32() * 1000.0;
        let k = (crate::anim::ease_out(age / 120.0)).min((1.0 - (age - 2200.0) / 150.0).clamp(0.0, 1.0));
        if k > 0.0 && !msg.is_empty() {
            let cu = crate::editor::style::Ui { th: ui.theme, s: ui.k, font: Some(&crate::fonts::UI) };
            let a = p(ui, FRect { x: lay.area.x, y: lay.area.y, w: lay.area.w, h: lay.area.h - if h.sel.len() > 1 { 56.0 } else { 0.0 } });
            crate::editor::style::toast(&mut ui.fb, &cu, msg, *kind, a, k);
        }
    }
    paint_menu(ui, h, lay, &t, hover);
    if h.asking() {
        paint_dialog(ui, h, lay, &t, hover);
    }
}

fn btn_bg(ui: &mut Ui, r: FRect, t: &WTok, on: bool, hov: bool, rad: f32) {
    if on {
        fill(ui, r, rad, t.acbg);
        ring(ui, r, rad, t.acrg);
    } else if hov {
        fill(ui, r, rad, t.hv);
    }
}

fn paint_header(ui: &mut Ui, h: &mut History, lay: &Lay, t: &WTok, hover: Option<&Target>) {
    let cy = HDR_H / 2.0;
    fill(ui, lay.hdr, 0.0, t.win);
    line_h(ui, 0.0, HDR_H - 1.0, lay.w, t.sep);
    text(ui, 14.0, "History", 18.0, cy, t.tx, true);
    let dim = h.on() && h.entries.is_empty();
    let k = if dim { 0.5 } else { 1.0 };
    if let Some(s) = lay.search {
        let focused = ui.focus.is_focused("history.search");
        // The field: the kit's text field over the board's frame.
        fill(ui, s, 7.0, t.sf2.fade(k));
        ring(ui, s, 7.0, t.bd.fade(k));
        if focused {
            let q = p(ui, FRect { x: s.x - 2.0, y: s.y - 2.0, w: s.w + 4.0, h: s.h + 4.0 });
            let lw = (2.0 * ui.k).round();
            ui.fb.stroke_rounded(FRect { x: q.x + lw / 2.0, y: q.y + lw / 2.0, w: q.w - lw, h: q.h - lw }, 9.0 * ui.k - lw / 2.0, lw, t.ac);
        }
        icon(ui, "search", s.x + 10.0, cy - 8.0, 16.0, t.ic.fade(k));
        let tf = FRect { x: s.x + 24.0, y: s.y, w: s.w - 24.0 - if h.search.text.is_empty() { 4.0 } else { 26.0 }, h: s.h };
        let (rad, ctl, px) = (ui.radius, ui.ctl_h, ui.text_px);
        ui.radius = 7.0;
        ui.ctl_h = 28.0;
        ui.text_px = 12.0 * CSS;
        ui.frameless = true;
        let en = ui.enabled;
        ui.enabled = !dim;
        let changed = ui.area(tf, |ui| ui.text_field("history.search", &mut h.search));
        ui.enabled = en;
        ui.radius = rad;
        ui.ctl_h = ctl;
        ui.text_px = px;
        ui.frameless = false;
        if changed {
            h.scroll = 0.0;
        }
        if h.search.text.is_empty() {
            text(ui, 12.0, "Search by window or date\u{2026}", s.x + 34.0, cy, t.tx3.fade(k), false);
        } else {
            let hov = hover == Some(&Target::SearchClear);
            icon(ui, "x", s.x1() - 23.0, cy - 7.0, 14.0, if hov { t.ich } else { t.ic });
        }
    }
    if let (Some(first), Some(last)) = (lay.filters.first(), lay.filters.last()) {
        let b = FRect { x: first.0.x - 3.0, y: cy - SEG_H / 2.0, w: last.0.x1() - first.0.x + 6.0, h: SEG_H };
        fill(ui, b, 8.0, t.sf2.fade(k));
        ring(ui, b, 8.0, t.bd.fade(k));
        for (i, (r, f)) in lay.filters.iter().enumerate() {
            let on = h.filter == *f;
            let hov = hover == Some(&Target::Filter(*f));
            btn_bg(ui, *r, t, on, hov && !dim, 5.0);
            let c = if on { t.acfg } else if hov { t.ich } else { t.ic };
            let l = FILTERS[i].1;
            let tw = ui.text_w(11.5, l);
            text(ui, 11.5, l, r.x + (r.w - tw) / 2.0, cy, c.fade(k), false);
        }
    }
    if let (Some(first), Some(last)) = (lay.views.first(), lay.views.last()) {
        let b = FRect { x: first.0.x - 3.0, y: cy - SEG_H / 2.0, w: last.0.x1() - first.0.x + 6.0, h: SEG_H };
        fill(ui, b, 8.0, t.sf2);
        ring(ui, b, 8.0, t.bd);
        for (r, v) in &lay.views {
            let on = h.view == *v;
            let hov = hover == Some(&Target::View(*v));
            btn_bg(ui, *r, t, on, hov, 5.0);
            let c = if on { t.acfg } else if hov { t.ich } else { t.ic };
            icon(ui, if *v == View::Grid { "grid" } else { "list" }, r.x + 9.0, cy - 7.0, 14.0, c);
        }
    }
    if let Some(s) = lay.slider {
        icon(ui, "grid", s.x, cy - 7.0, 14.0, t.ic);
        let tr = FRect { x: s.x + 22.0, y: cy - 1.5, w: 80.0, h: 3.0 };
        let track = if t.dark { C4::new(255, 255, 255, 36) } else { C4::new(0, 0, 0, 30) };
        fill(ui, tr, 1.5, track);
        let f = ((h.thumb - super::THUMB_RANGE.0) / (super::THUMB_RANGE.1 - super::THUMB_RANGE.0)).clamp(0.0, 1.0);
        fill(ui, FRect { w: tr.w * f, ..tr }, 1.5, t.ac);
        let kx = tr.x + tr.w * f;
        let k2 = ui.k;
        ui.fb.fill_circle(kx * k2, cy * k2 + k2, 7.0 * k2, C4::new(0, 0, 0, 90));
        ui.fb.fill_circle(kx * k2, cy * k2, 6.5 * k2, C4::rgb(255, 255, 255));
    }
    if let Some(pb) = lay.panel_btn {
        let hov = hover == Some(&Target::PanelToggle);
        let on = lay.panel.is_some();
        btn_bg(ui, pb, t, false, hov, 7.0);
        icon(ui, "panel", pb.x + 6.0, pb.y + 6.0, 16.0, if on || hov { t.ich } else { t.ic });
    }
    let hov = hover == Some(&Target::More) || h.menu == Some(Menu::More);
    btn_bg(ui, lay.more, t, false, hov, 7.0);
    icon(ui, "more", lay.more.x + 6.0, lay.more.y + 6.0, 16.0, if hov { t.ich } else { t.ic });
}

fn paint_footer(ui: &mut Ui, h: &History, f: FRect, t: &WTok) {
    fill(ui, f, 0.0, t.win);
    line_h(ui, 0.0, f.y, f.w, t.sep);
    let cy = f.y + f.h / 2.0;
    let (keep, used, count, frac) = h.footer();
    let mut x = 18.0;
    x += text(ui, 11.0, &keep, x, cy, t.tx2, false) + 10.0;
    if h.entries.is_empty() {
        // The empty state's footer: "Keeping 30 days · 0 B of 2 GB".
        x += text(ui, 11.0, "\u{b7}", x, cy, t.tx3, false) + 10.0;
        text(ui, 11.0, &used, x, cy, t.tx2, false);
        return;
    }
    x += text(ui, 11.0, "\u{b7}", x, cy, t.tx3, false) + 10.0;
    let bar = FRect { x, y: cy - 1.5, w: 120.0, h: 3.0 };
    let track = if t.dark { C4::new(255, 255, 255, 31) } else { C4::new(0, 0, 0, 26) };
    fill(ui, bar, 1.5, track);
    fill(ui, FRect { w: (bar.w * frac).max(if frac > 0.0 { 2.0 } else { 0.0 }), ..bar }, 1.5, t.ac);
    x += 120.0 + 10.0;
    text(ui, 11.0, &used, x, cy, t.tx2, false);
    let cw = ui.text_w(11.0, &count);
    text(ui, 11.0, &count, f.w - 18.0 - cw, cy, t.tx2, false);
}

fn paint_body(ui: &mut Ui, h: &mut History, lay: &Lay, t: &WTok, hover: Option<&Target>) {
    let a = lay.area;
    if !h.on() {
        paint_off(ui, h, lay, t, hover);
        return;
    }
    if h.entries.is_empty() {
        paint_empty(ui, h, lay, t, hover);
        return;
    }
    let scroll = h.scroll;
    let list = h.view == View::List;
    let multi = h.sel.len() > 1;
    let items = lay.items.clone();
    let hov_id = match hover {
        Some(Target::Item(id) | Target::Quick(id, _)) => Some(id.clone()),
        _ => None,
    };
    let menu_id = match &h.menu {
        Some(Menu::Card(id)) => Some(id.clone()),
        _ => None,
    };
    for it in &items {
        let r = lay.on_screen(it.r, scroll);
        if r.y1() < a.y || r.y > a.y1() {
            continue;
        }
        let Some(e) = h.entries.get(it.idx).cloned() else { continue };
        let selected = h.sel.contains(&it.id);
        let hovered = hov_id.as_deref() == Some(it.id.as_str()) || menu_id.as_deref() == Some(it.id.as_str());
        {
            if list {
                paint_row(ui, h, lay, it, &e, t, selected, hovered, multi, hover);
            } else {
                paint_card(ui, h, lay, it, &e, t, selected, hovered, multi, hover);
            }
        }
    }
    // Group headers, and the sticky one at the top.
    for (i, g) in lay.groups.iter().enumerate() {
        let next = lay.groups.get(i + 1).map_or(f32::MAX, |n| n.y);
        let y = a.y + g.y - scroll;
        let sticky = g.y < scroll && next > scroll;
        let y = if sticky { a.y.max((a.y + next - scroll - DAY_H).min(a.y)) } else { y };
        if y + DAY_H < a.y || y > a.y1() {
            continue;
        }
        let gr = FRect { x: a.x, y, w: a.w, h: DAY_H };
        {
            fill(ui, gr, 0.0, t.win);
            let cy = y + DAY_H / 2.0;
            let w = caps(ui, 10.5, &g.title, PAD_X, cy, t.tx3);
            if let Some(s) = g.small.as_ref().filter(|_| !list) {
                text(ui, 11.0, s, PAD_X + w + 8.0, cy, t.tx3, false);
            }
        }
    }
    if lay.items.is_empty() {
        let msg = if h.query().is_empty() { "Nothing here yet" } else { "No captures match" };
        let tw = ui.text_w(12.0, msg);
        text(ui, 12.0, msg, a.x + (a.w - tw) / 2.0, a.y + 80.0, t.tx3, false);
    }
    // The search's note under the results.
    if !h.query().is_empty() && list {
        let y = a.y + lay.content_h - 12.0 - scroll + 14.0;
        if y < a.y1() - 10.0 {
            let note = "Searches window title, app name, file name and date.";
            {
                text(ui, 11.5, note, PAD_X, y + 8.0, t.tx3, false);
            }
        }
    }
    // The marquee.
    if let Some((a0, a1)) = h.marquee() {
        let r = FRect::from_pts(a0, a1);
        let r = FRect { x: r.x, y: r.y - scroll + a.y, ..r };
        {
            fill(ui, r, 0.0, t.ac.with_alpha(26));
            ring(ui, r, 0.0, t.ac);
        }
    }
}


/// The kind's badge at (x, y); its width.
fn kind_badge(ui: &mut Ui, e: &Entry, x: f32, y: f32, t: &WTok) -> f32 {
    let gif = e.meta.kind == Kind::Gif;
    let w = if gif { ui.text_w(10.5, "GIF") + 12.0 } else { 26.0 };
    let b = FRect { x, y, w, h: 20.0 };
    fill(ui, b, 5.0, t.badge);
    if gif {
        text(ui, 10.5, "GIF", x + 6.0, y + 10.0, C4::rgb(0xEC, 0xEE, 0xF3), true);
    } else {
        let name = if e.meta.kind == Kind::Mp4 { "video" } else { "camera" };
        icon(ui, name, x + 6.0, y + 3.0, 14.0, C4::rgb(0xEC, 0xEE, 0xF3));
    }
    w
}

/// "Uploaded": a cloud badge beside the kind's.
fn upload_badge(ui: &mut Ui, x: f32, y: f32, t: &WTok) {
    let b = FRect { x, y, w: 26.0, h: 20.0 };
    fill(ui, b, 5.0, t.badge);
    icon(ui, "cloud", x + 6.0, y + 3.0, 14.0, C4::rgb(0xEC, 0xEE, 0xF3));
}

/// The picture of `id` in `box_` (contained, over the checkerboard).
fn picture(ui: &mut Ui, h: &mut History, id: &str, box_: FRect, rad: f32, cell: f32, t: &WTok) {
    let q = p(ui, box_);
    let k = ui.k;
    paint::checker(&mut ui.fb, q, (cell * k).round(), t.chk1, t.chk2);
    if let Some(img) = h.thumb_of(id) {
        let d = paint::contain(img.dimensions(), q, 4.0);
        let img = img.clone();
        paint::blit(&mut ui.fb, &img, d, q, rad * k);
    }
    // The corners of the checkerboard: round them off.
    round_corners(ui, q, rad * k, t.win);
}

/// Paint `bg` over the outside of `r`'s rounded corners.
fn round_corners(ui: &mut Ui, r: FRect, rad: f32, bg: C4) {
    if rad <= 0.5 {
        return;
    }
    let n = rad.ceil() as i32;
    for (cx, cy) in [(r.x, r.y), (r.x1() - n as f32, r.y), (r.x, r.y1() - n as f32), (r.x1() - n as f32, r.y1() - n as f32)] {
        for dy in 0..n {
            for dx in 0..n {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let ix = if px < r.x + rad { r.x + rad } else if px > r.x1() - rad { r.x1() - rad } else { px };
                let iy = if py < r.y + rad { r.y + rad } else if py > r.y1() - rad { r.y1() - rad } else { py };
                let d = (px - ix).hypot(py - iy);
                let out = (d - rad + 0.5).clamp(0.0, 1.0);
                if out > 0.0 {
                    ui.fb.blend_px(x, y, bg.with_alpha((bg.a as f32 * out) as u8));
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_card(ui: &mut Ui, h: &mut History, lay: &Lay, it: &ItemLay, e: &Entry, t: &WTok, selected: bool, hovered: bool, multi: bool, hover: Option<&Target>) {
    let scroll = h.scroll;
    let th = lay.on_screen(it.thumb, scroll);
    let k = ui.k;
    if hovered && !multi {
        let q = p(ui, th);
        paint::shadow(&mut ui.fb, t, q, 10.0 * k, k, &[0, 1, 2]);
    }
    picture(ui, h, &it.id, th, 10.0, 12.0, t);
    ring(ui, th, 10.0, t.bd);
    if selected {
        let q = p(ui, th);
        paint::outer_ring(&mut ui.fb, q, 10.0 * k, 2.0 * k, 2.0 * k, t.ac);
    }
    if multi {
        let cb = FRect { x: th.x + 8.0, y: th.y + 8.0, w: 20.0, h: 20.0 };
        checkbox(ui, cb, selected, t);
    } else {
        let bw = kind_badge(ui, e, th.x + 8.0, th.y + 8.0, t);
        if e.meta.uploaded.is_some() {
            upload_badge(ui, th.x + 8.0 + bw + 4.0, th.y + 8.0, t);
        }
    }
    if e.meta.edited {
        let tw = ui.text_w(10.5, "Edited");
        let b = FRect { x: th.x1() - 8.0 - tw - 14.0, y: th.y + 8.0, w: tw + 14.0, h: 20.0 };
        fill(ui, b, 5.0, t.acbg);
        ring(ui, b, 5.0, t.acrg);
        text(ui, 10.5, "Edited", b.x + 7.0, b.y + 10.0, t.acfg, true);
    }
    if e.meta.kind.is_recording() && (multi || !hovered) {
        let d = history::duration_text(e.meta.duration_ms);
        let tw = ui.text_w(10.5, &d);
        let b = FRect { x: th.x1() - 8.0 - tw - 12.0, y: th.y1() - 28.0, w: tw + 12.0, h: 20.0 };
        fill(ui, b, 5.0, t.badge);
        text(ui, 10.5, &d, b.x + 6.0, b.y + 10.0, C4::rgb(0xEC, 0xEE, 0xF3), true);
    }
    if hovered && !multi {
        // The quick-action strip.
        let strip = FRect { x: th.x, y: th.y1() - STRIP_H, w: th.w, h: STRIP_H };
        let q = p(ui, strip);

        strip_fill(ui, q, 10.0 * k, t.strip);
        for (r, a) in quick(lay, it, e, scroll, false) {
            let hov = hover == Some(&Target::Quick(it.id.clone(), a));
            if a == Quick::Edit {
                fill(ui, r, 6.0, if hov { t.acbg_hover } else { t.acbg });
                ring(ui, r, 6.0, t.acrg);
                icon(ui, "pencil", r.x + 7.0, r.y + 6.0, 14.0, t.acfg);
                text(ui, 11.5, "Edit", r.x + 26.0, r.y + 13.0, t.acfg, true);
            } else {
                if hov {
                    fill(ui, r, 6.0, C4::new(255, 255, 255, 30));
                }
                icon(ui, a.icon(), r.x + 6.0, r.y + 6.0, 14.0, t.strip_fg);
            }
        }
    }
    // The name and the meta line.
    let title = history::title(&e.meta);
    let my = th.y1() + 7.0;
    let name = ui.fit_text(12.0, &title, th.w);
    highlighted(ui, &name, &it.hl, th.x, my + 7.5, 12.0, t);
    let mut m = format!("{} \u{d7} {} \u{b7} {}", e.meta.w, e.meta.h, history::time_text(&e.meta.captured, h.today, !h.query().is_empty()));
    if e.meta.kind.is_recording() {
        m.push_str(&format!(" \u{b7} {}", e.meta.kind.label()));
    }
    let m = ui.fit_text(11.0, &m, th.w);
    text(ui, 11.0, &m, th.x, my + 15.0 + 2.0 + 7.0, t.tx2, false);
}

/// Fill `r` (physical px) with its bottom corners rounded by `rad` (the
/// strip at a card's bottom follows the thumb's corners).
fn strip_fill(ui: &mut Ui, r: FRect, rad: f32, c: C4) {
    let top = (r.h - rad).max(0.0).floor();
    ui.fb.fill_rect(r.x as i32, r.y as i32, r.w as i32, top as i32, c);
    let low = FRect { y: r.y + top, h: r.h - top, ..r };
    let full = FRect { y: r.y1() - 2.0 * rad, h: 2.0 * rad, ..r };
    crate::ui::clipped(&mut ui.fb, low, |fb| fb.fill_rounded(full.x as i32, full.y as i32, full.w as i32, full.h as i32, rad, c));
}

fn tooltip(ui: &mut Ui, anchor: FRect, q: Quick, t: &WTok, lay: &Lay) {
    let (label, key) = q.tip();
    let lw = ui.text_w(11.5, label);
    let kw = if key.is_empty() { 0.0 } else { ui.text_w(10.0, key) + 8.0 + 6.0 };
    let w = 8.0 + lw + kw + 8.0;
    let r = FRect { x: (anchor.x + anchor.w / 2.0 - w / 2.0).clamp(4.0, lay.w - w - 4.0), y: anchor.y - 30.0 - 6.0, w, h: 24.0 };
    fill(ui, r, 6.0, t.tip_bg);
    ring(ui, r, 6.0, C4::new(255, 255, 255, 20));
    text(ui, 11.5, label, r.x + 8.0, r.y + 12.0, t.tip_fg, false);
    if !key.is_empty() {
        let kr = FRect { x: r.x + 8.0 + lw + 6.0, y: r.y + 4.0, w: ui.text_w(10.0, key) + 8.0, h: 16.0 };
        fill(ui, kr, 3.0, C4::new(255, 255, 255, 36));
        text(ui, 10.0, key, kr.x + 4.0, kr.y + 8.0, C4::rgb(0xC4, 0xC7, 0xD0), true);
    }
}

fn checkbox(ui: &mut Ui, r: FRect, on: bool, t: &WTok) {
    if on {
        fill(ui, r, 6.0, t.ac);
        icon(ui, "check", r.x + 3.0, r.y + 3.0, 14.0, C4::rgb(0x0E, 0x10, 0x20));
    } else {
        fill(ui, r, 6.0, C4::new(14, 15, 18, 153));
        let q = p(ui, r);
        let k = ui.k;
        ui.fb.stroke_rounded(FRect { x: q.x + 0.75 * k, y: q.y + 0.75 * k, w: q.w - 1.5 * k, h: q.h - 1.5 * k }, 5.25 * k, 1.5 * k, C4::new(255, 255, 255, 178));
    }
}

/// `s` with the byte ranges `hl` on the mark color.
fn highlighted(ui: &mut Ui, s: &str, hl: &[std::ops::Range<usize>], x: f32, cy: f32, size: f32, t: &WTok) {
    for r in hl {
        if r.end > s.len() || !s.is_char_boundary(r.start) || !s.is_char_boundary(r.end) {
            continue;
        }
        let x0 = x + ui.text_w(size, &s[..r.start]);
        let w = ui.text_w(size, &s[r.start..r.end]);
        fill(ui, FRect { x: x0 - 1.0, y: cy - 8.0, w: w + 2.0, h: 16.0 }, 3.0, t.mark);
    }
    text(ui, size, s, x, cy, t.tx, false);
}

#[allow(clippy::too_many_arguments)]
fn paint_row(ui: &mut Ui, h: &mut History, lay: &Lay, it: &ItemLay, e: &Entry, t: &WTok, selected: bool, hovered: bool, multi: bool, hover: Option<&Target>) {
    let r = lay.on_screen(it.r, h.scroll);
    if selected {
        fill(ui, r, 0.0, t.acbg);
    } else if hovered {
        fill(ui, r, 0.0, t.hv);
    }
    line_h(ui, r.x, r.y1() - 1.0, r.w, t.sep);
    let cy = r.y + r.h / 2.0;
    let mut x = PAD_X;
    if multi {
        checkbox(ui, FRect { x, y: cy - 10.0, w: 20.0, h: 20.0 }, selected, t);
        x += 20.0 + 14.0;
    }
    let th = FRect { x, y: cy - 24.0, w: 72.0, h: 48.0 };
    picture(ui, h, &it.id, th, 6.0, 8.0, t);
    ring(ui, th, 6.0, t.bd);
    x += 72.0 + 14.0;
    // Columns from the right: actions 118, time, dimensions 90, type 70.
    let search = !h.query().is_empty();
    let time_w = if search { 110.0 } else { 44.0 };
    let act_x = r.x1() - 14.0 - 118.0;
    let time_x = act_x - 14.0 - time_w;
    let dim_x = time_x - 14.0 - 90.0;
    let ty_x = dim_x - 14.0 - 92.0;
    let title = history::title(&e.meta);
    let edited_w = if e.meta.edited { ui.text_w(10.5, "Edited") + 6.0 } else { 0.0 };
    let name = ui.fit_text(12.5, &title, (ty_x - 14.0 - x - edited_w).max(20.0));
    highlighted(ui, &name, &it.hl, x, cy, 12.5, t);
    if e.meta.edited {
        let nw = ui.text_w(12.5, &name);
        text(ui, 10.5, "Edited", x + nw + 6.0, cy, t.acfg, true);
    }
    let rec = e.meta.kind.is_recording();
    icon(ui, if rec { "video" } else { "camera" }, ty_x, cy - 7.0, 14.0, t.tx2);
    if e.meta.uploaded.is_some() {
        // Uploaded: the cloud before the type.
        icon(ui, "cloud", ty_x - 19.0, cy - 7.0, 14.0, t.tx2);
    }
    let ty = if rec { format!("{} \u{b7} {}", e.meta.kind.label(), history::duration_text(e.meta.duration_ms)) } else { e.meta.format.clone() };
    let ty = ui.fit_text(11.5, &ty, 92.0 - 19.0);
    text(ui, 11.5, &ty, ty_x + 19.0, cy, t.tx2, false);
    let d = format!("{} \u{d7} {}", e.meta.w, e.meta.h);
    let dw = ui.text_w(11.5, &d);
    text(ui, 11.5, &d, dim_x + 90.0 - dw, cy, t.tx2, false);
    let tm = history::time_text(&e.meta.captured, h.today, search);
    let tw = ui.text_w(11.5, &tm);
    text(ui, 11.5, &tm, time_x + time_w - tw, cy, t.tx2, false);
    if hovered && !multi {
        for (q, a) in quick(lay, it, e, h.scroll, true) {
            let hov = hover == Some(&Target::Quick(it.id.clone(), a));
            if hov {
                fill(ui, q, 6.0, t.hv);
            }
            icon(ui, a.icon(), q.x + 6.0, q.y + 6.0, 14.0, if hov { t.ich } else { t.ic });
        }
    }
}

fn paint_panel(ui: &mut Ui, h: &mut History, pr: FRect, t: &WTok, hover: Option<&Target>) {
    fill(ui, pr, 0.0, t.rail);
    line_v(ui, pr.x, pr.y, pr.h, t.sep);
    let Some(e) = h.single().cloned() else { return };
    let pl = panel_lay(pr);
    text(ui, 12.5, "Details", pr.x + 16.0, pr.y + 20.0, t.tx, true);
    let hov = hover == Some(&Target::PanelClose);
    btn_bg(ui, pl.close, t, false, hov, 7.0);
    icon(ui, "x", pl.close.x + 6.0, pl.close.y + 6.0, 16.0, if hov { t.ich } else { t.ic });
    line_h(ui, pr.x, pr.y + 40.0, pr.w, t.sep);
    picture(ui, h, &e.id, pl.preview, 8.0, 12.0, t);
    ring(ui, pl.preview, 8.0, t.bd);
    // Rows: key 88 wide, then the value.
    let today = h.today;
    let size = format!("{} \u{d7} {} \u{b7} {} \u{b7} {}", e.meta.w, e.meta.h, history::size_text(e.meta.bytes), if e.meta.format.is_empty() { e.meta.kind.label().to_string() } else { e.meta.format.clone() });
    let src = {
        let s = history::title(&e.meta);
        if e.meta.app.is_empty() && e.meta.title.is_empty() { "\u{2014}".to_string() } else { s }
    };
    let saved = e.meta.saved.as_ref().or(e.meta.file.as_ref()).map_or("\u{2014}".to_string(), |p| history::tilde(p));
    let mut rows: Vec<(&str, String)> = vec![
        ("Captured", history::captured_text(&e.meta.captured, today)),
        ("Size", size),
        ("Monitor", if e.meta.monitor.is_empty() { "\u{2014}".into() } else { e.meta.monitor.clone() }),
        ("Source", src),
        ("Saved to", saved),
    ];
    if e.meta.kind.is_recording() {
        rows.insert(2, ("Length", history::duration_text(e.meta.duration_ms)));
    }
    let mut y = pl.rows_y;
    for (k, v) in &rows {
        text(ui, 11.5, k, pr.x + 16.0, y + 12.0, t.tx3, false);
        let v = ui.fit_text(11.5, v, pr.w - 32.0 - 98.0);
        text(ui, 11.5, &v, pr.x + 16.0 + 98.0, y + 12.0, t.tx, false);
        y += 24.0;
    }
    if let Some(u) = &e.meta.uploaded {
        text(ui, 11.5, "Uploaded", pr.x + 16.0, y + 12.0, t.tx3, false);
        let shown = u.trim_start_matches("https://").trim_start_matches("http://");
        let v = ui.fit_text(11.5, shown, pr.w - 32.0 - 98.0 - 28.0);
        let vw = text(ui, 11.5, &v, pr.x + 16.0 + 98.0, y + 12.0, t.acfg, false);
        let b = FRect { x: pr.x + 16.0 + 98.0 + vw + 6.0, y: y + 1.0, w: 22.0, h: 22.0 };
        let hov = hover == Some(&Target::PanelLink);
        btn_bg(ui, b, t, false, hov, 5.0);
        icon(ui, "copy", b.x + 4.0, b.y + 4.0, 14.0, if hov { t.ich } else { t.ic });
        y += 24.0;
    }
    if !e.meta.kind.is_recording() {
        y += 12.0;
        let n = e.meta.objects.len();
        let head = format!("Annotations \u{b7} {n} object{}", if n == 1 { "" } else { "s" });
        caps(ui, 10.5, &head, pr.x + 16.0, y + 6.0, t.tx3);
        y += 16.0;
        let mut x = pr.x + 16.0;
        for c in history::object_chips(&e.meta.objects) {
            let w = ui.text_w(11.0, &c) + 16.0;
            if x + w > pr.x1() - 16.0 {
                x = pr.x + 16.0;
                y += 28.0;
            }
            let r = FRect { x, y, w, h: 22.0 };
            fill(ui, r, 6.0, t.sf2);
            ring(ui, r, 6.0, t.bd);
            text(ui, 11.0, &c, x + 8.0, y + 11.0, t.tx, false);
            x += w + 6.0;
        }
    }
    // The buttons.
    line_h(ui, pr.x, pl.edit.y - 14.0, pr.w, t.sep);
    let rec = e.meta.kind.is_recording();
    let (hov_e, hov_o, hov_d) = (hover == Some(&Target::PanelEdit), hover == Some(&Target::PanelOpen), hover == Some(&Target::PanelDelete));
    button(ui, pl.edit, if rec { "Open" } else { "Edit" }, Some(if rec { "play" } else { "pencil" }), Style::Pri, hov_e, t);
    button(ui, pl.open, if rec { "Show in folder" } else { "Open file" }, Some("folder"), Style::Plain, hov_o, t);
    button(ui, pl.delete, "", Some("trash"), Style::Danger, hov_d, t);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    Plain,
    Pri,
    Danger,
    Solid,
}

/// The boards' 28 px button: centred icon and label.
pub fn button(ui: &mut Ui, r: FRect, label: &str, icn: Option<&str>, st: Style, hov: bool, t: &WTok) {
    let (bg, fg, rg) = match st {
        Style::Plain => (t.hv, t.tx, t.bd),
        Style::Pri => (if hov { t.acbg_hover } else { t.acbg }, t.acfg, t.acrg),
        Style::Danger => (t.erbg, t.er, t.errg),
        Style::Solid => (t.solid, C4::rgb(255, 255, 255), C4::new(0, 0, 0, 0)),
    };
    fill(ui, r, 7.0, bg);
    if hov && st != Style::Pri {
        fill(ui, r, 7.0, if st == Style::Solid { C4::new(255, 255, 255, 28) } else { t.hv });
    }
    if rg.a > 0 {
        ring(ui, r, 7.0, rg);
    }
    let lw = if label.is_empty() { 0.0 } else { ui.text_w(12.0, label) };
    let cw = lw + if icn.is_some() { if label.is_empty() { 14.0 } else { 20.0 } } else { 0.0 };
    let mut x = r.x + (r.w - cw) / 2.0;
    let cy = r.y + r.h / 2.0;
    if let Some(i) = icn {
        icon(ui, i, x, cy - 7.0, 14.0, fg);
        x += 20.0;
    }
    if !label.is_empty() {
        text(ui, 12.0, label, x, cy, fg, false);
    }
}

fn paint_bar(ui: &mut Ui, h: &History, lay: &Lay, t: &WTok, hover: Option<&Target>) {
    let (r, b, label) = bar_lay(ui, lay, h.sel.len());
    let k = ui.k;
    let q = p(ui, r);
    paint::shadow(&mut ui.fb, t, q, 12.0 * k, k, &[0, 1, 2]);
    fill(ui, r, 12.0, t.pop);
    ring(ui, r, 12.0, t.bd);
    text(ui, 12.5, &label, r.x + 14.0, r.y + 20.0, t.tx, true);
    button(ui, b[0], "Copy", Some("copy"), Style::Plain, hover == Some(&Target::BarCopy), t);
    button(ui, b[1], "Save to folder\u{2026}", Some("folder"), Style::Plain, hover == Some(&Target::BarSave), t);
    button(ui, b[2], "Delete", Some("trash"), Style::Danger, hover == Some(&Target::BarDelete), t);
    let hov = hover == Some(&Target::BarClear);
    btn_bg(ui, b[3], t, false, hov, 7.0);
    icon(ui, "x", b[3].x + 6.0, b[3].y + 6.0, 16.0, if hov { t.ich } else { t.ic });
}

fn paint_menu(ui: &mut Ui, h: &History, lay: &Lay, t: &WTok, hover: Option<&Target>) {
    let items = targets(ui, lay, h, (-1.0, -1.0));
    // The card's ⋯ is a target only while hovered: find it again.
    let anchor_items: Vec<(FRect, Target)> = match &h.menu {
        Some(Menu::Card(id)) => {
            let it = lay.items.iter().find(|i| i.id == *id);
            match (it, h.entry(id)) {
                (Some(it), Some(e)) => quick(lay, it, e, h.scroll, h.view == View::List).into_iter().map(|(r, q)| (r, Target::Quick(id.clone(), q))).collect(),
                _ => Vec::new(),
            }
        }
        _ => items,
    };
    let Some((r, rows)) = menu_lay(lay, h, &anchor_items) else { return };
    let k = ui.k;
    let q = p(ui, r);
    paint::shadow(&mut ui.fb, t, q, 10.0 * k, k, &[0, 1, 2, 3]);
    fill(ui, r, 10.0, t.pop);
    ring(ui, r, 10.0, t.bd);
    for (rr, a) in rows {
        let hov = hover == Some(&Target::MenuItem(a));
        let danger = matches!(a, MenuAct::Delete | MenuAct::Clear | MenuAct::DeleteUpload);
        if hov {
            fill(ui, rr, 6.0, if danger { t.erbg } else { t.acbg });
        }
        let c = if hov && danger { t.er } else if hov { t.acfg } else { t.tx };
        text(ui, 12.5, a.label(), rr.x + 10.0, rr.y + rr.h / 2.0, c, false);
    }
}

fn paint_dialog(ui: &mut Ui, h: &History, lay: &Lay, t: &WTok, hover: Option<&Target>) {
    fill(ui, FRect { x: 0.0, y: 0.0, w: lay.w, h: lay.h }, 0.0, t.scrim);
    let (r, cb, cancel, ok) = dialog_lay(ui, lay, h);
    let k = ui.k;
    let q = p(ui, r);
    paint::shadow(&mut ui.fb, t, q, 12.0 * k, k, &[0, 1, 2, 3]);
    fill(ui, r, 12.0, t.win);
    ring(ui, r, 12.0, t.bd);
    let (title, para, check, okl) = h.dialog_texts();
    text(ui, 14.0, &title, r.x + 20.0, r.y + 20.0 + 9.0, t.tx, true);
    let lines = crate::ui::controls::wrap(&para, DLG_W - 40.0, |s| ui.text_w(12.0, s));
    let mut y = r.y + 20.0 + 18.0 + 10.0;
    for l in &lines {
        text(ui, 12.0, l, r.x + 20.0, y + 9.0, t.tx2, false);
        y += 18.0;
    }
    if let Some(check) = check {
        let on = h.clear == Some(true);
        let bx = FRect { x: cb.x, y: cb.y + 1.0, w: 16.0, h: 16.0 };
        if on {
            fill(ui, bx, 4.0, t.ac);
            icon(ui, "check", bx.x + 2.0, bx.y + 2.0, 12.0, C4::rgb(0x0E, 0x10, 0x20));
        } else {
            let qb = p(ui, bx);
            ui.fb.stroke_rounded(FRect { x: qb.x + 0.75 * k, y: qb.y + 0.75 * k, w: qb.w - 1.5 * k, h: qb.h - 1.5 * k }, 3.25 * k, 1.5 * k, t.cb);
        }
        let check = ui.fit_text(12.0, &check, cb.w - 24.0);
        text(ui, 12.0, &check, cb.x + 24.0, cb.y + 9.0, t.tx, false);
    }
    button(ui, cancel, "Cancel", None, Style::Plain, hover == Some(&Target::ClearCancel), t);
    let style = if h.ask_delete.is_some() || h.ask_unupload.is_some() { Style::Danger } else { Style::Solid };
    button(ui, ok, okl, None, style, hover == Some(&Target::ClearOk), t);
}

fn key_caps(ui: &mut Ui, keys: &[String], x: f32, cy: f32, t: &WTok) -> f32 {
    let mut cx = x;
    for kname in keys {
        let w = (ui.text_w(10.5, kname) + 10.0).max(18.0);
        let r = FRect { x: cx, y: cy - 9.0, w, h: 18.0 };
        fill(ui, r, 4.0, t.key_bg);
        text(ui, 10.5, kname, cx + (w - ui.text_w(10.5, kname)) / 2.0, cy, t.key_fg, true);
        cx += w + 4.0;
    }
    cx - x
}

/// The capture hotkey's caps in the board's order: Win, Ctrl, Alt, Shift, key.
pub fn hotkey_caps(spec: &str) -> Vec<String> {
    let Some(c) = crate::keymap::Chord::parse(spec) else { return Vec::new() };
    let mut v = Vec::new();
    if c.meta {
        v.push(crate::keymap::META_NAME.to_string());
    }
    for (on, n) in [(c.ctrl, "Ctrl"), (c.alt, "Alt"), (c.shift, "Shift")] {
        if on {
            v.push(n.to_string());
        }
    }
    v.push(c.key());
    v
}

fn paint_empty(ui: &mut Ui, h: &History, lay: &Lay, t: &WTok, hover: Option<&Target>) {
    let a = lay.area;
    let cx = a.x + a.w / 2.0;
    let cy = a.y + a.h / 2.0;
    icon(ui, "history", cx - 28.0, cy - 80.0, 56.0, t.tx3);
    let tt = "No captures yet";
    text(ui, 14.0, tt, cx - ui.text_w(14.0, tt) / 2.0, cy - 8.0, t.tx, true);
    let caps_ = hotkey_caps(&h.cfg.capture_hotkey);
    let (pre, post) = ("Press", "to take your first screenshot");
    let capw: f32 = caps_.iter().map(|k| (ui.text_w(10.5, k) + 10.0).max(18.0) + 4.0).sum::<f32>();
    let total = ui.text_w(12.0, pre) + 5.0 + capw + 1.0 + ui.text_w(12.0, post);
    let mut x = cx - total / 2.0;
    let ly = cy + 18.0;
    if caps_.is_empty() {
        let s = "Pick a capture region from the tray to take your first screenshot";
        text(ui, 12.0, s, cx - ui.text_w(12.0, s) / 2.0, ly, t.tx2, false);
    } else {
        x += text(ui, 12.0, pre, x, ly, t.tx2, false) + 5.0;
        x += key_caps(ui, &caps_, x, ly, t) + 1.0;
        text(ui, 12.0, post, x, ly, t.tx2, false);
    }
    let link = empty_lay(ui, lay, h);
    let hov = hover == Some(&Target::Settings);
    let w = text(ui, 12.0, "History settings", link.x, link.y + 9.0, t.acfg, true);
    if hov {
        line_h(ui, link.x, link.y + 16.0, w, t.acfg);
    }
}

fn paint_off(ui: &mut Ui, h: &History, lay: &Lay, t: &WTok, hover: Option<&Target>) {
    let a = lay.area;
    let cx = a.x + a.w / 2.0;
    let cy = a.y + a.h / 2.0;
    icon(ui, "history-off", cx - 28.0, cy - 70.0, 56.0, t.tx3);
    let tt = "History is off";
    text(ui, 14.0, tt, cx - ui.text_w(14.0, tt) / 2.0, cy + 2.0, t.tx, true);
    let sw = empty_lay(ui, lay, h);
    let tr = FRect { x: sw.x, y: sw.y, w: 32.0, h: 18.0 };
    let hov = hover == Some(&Target::TurnOn);
    let track = if t.dark { C4::new(255, 255, 255, if hov { 56 } else { 36 }) } else { C4::new(0, 0, 0, if hov { 56 } else { 40 }) };
    fill(ui, tr, 9.0, track);
    let k = ui.k;
    ui.fb.fill_circle((tr.x + 9.0) * k, (tr.y + 9.0) * k + 0.5 * k, 7.5 * k, C4::new(0, 0, 0, 80));
    ui.fb.fill_circle((tr.x + 9.0) * k, (tr.y + 9.0) * k, 7.0 * k, C4::rgb(255, 255, 255));
    text(ui, 12.5, "Keep a history of captures", tr.x + 42.0, tr.y + 9.0, t.tx, false);
    let lines = crate::ui::controls::wrap(OFF_CAPTION, a.w - 48.0, |s| ui.text_w(12.0, s));
    let mut y = sw.y1() + 18.0;
    for l in lines {
        text(ui, 12.0, &l, cx - ui.text_w(12.0, &l) / 2.0, y, t.tx3, false);
        y += 18.0;
    }
    if let Some((pre, x, cy, link)) = off_kept_lay(ui, lay, h) {
        text(ui, 12.0, &pre, x, cy, t.tx2, false);
        let w = text(ui, 12.0, "Clear history\u{2026}", link.x, cy, t.acfg, true);
        if hover == Some(&Target::OffClear) {
            line_h(ui, link.x, cy + 7.0, w, t.acfg);
        }
    }
}

/// Unused helper kept for the Fb type in signatures.
#[allow(dead_code)]
fn _fb(_: &Fb) {}
