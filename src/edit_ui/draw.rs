//! Laying out and painting the Edit window's own parts as on board 11:
//! the 44 px toolbar strip with the zoom control on its right, the
//! objects list (200 px; 44 px collapsed), the 28 px status bar, the save
//! flow's questions and the toast. The canvas and the docked toolbar's
//! buttons are the editor's ([`crate::editor::host::Canvas`]).

use super::{Btn, Choice, Dialog, Edit};
use crate::objects::{FRect, Obj};
use crate::ui::Ui;
use crate::ui::paint::{self, WTok};
use crate::uifb::C4;

pub const BAR_H: f32 = 44.0;
pub const STATUS_H: f32 = 28.0;
pub const RAIL_W: f32 = 200.0;
pub const RAIL_MIN: f32 = 44.0;
const RAIL_HEAD: f32 = 36.0;
const ROW_H: f32 = 32.0;
const NOTE_H: f32 = 52.0;
const DLG_W: f32 = 420.0;

/// The zoom control: −, the value, +, Fit, 1:1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomLay {
    pub r: FRect,
    pub minus: FRect,
    pub value: FRect,
    pub plus: FRect,
    pub fit: FRect,
    pub actual: FRect,
}

/// The window laid out (logical px).
#[derive(Clone, Debug, PartialEq)]
pub struct Lay {
    pub w: f32,
    pub h: f32,
    pub bar: FRect,
    pub rail: FRect,
    pub canvas: FRect,
    pub status: FRect,
    pub zoom: ZoomLay,
    /// The zoom control sits in the status bar (no room in the strip).
    pub zoom_in_status: bool,
    pub collapse: FRect,
    /// Rows of the objects list: their rect and object (z-index).
    pub rows: Vec<(FRect, usize)>,
}

/// What the pointer can be on (besides the canvas and its toolbar).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    ZoomOut,
    ZoomIn,
    Fit,
    Actual,
    /// The list's collapse (or, collapsed, expand) button.
    Collapse,
    Row(usize),
    Dialog(Btn),
}

fn zoom_lay(right: f32, cy: f32, h: f32) -> ZoomLay {
    let b = h - 4.0;
    let w = 2.0 + b + 2.0 + 44.0 + 2.0 + b + 2.0 + 5.0 + 2.0 + b + 2.0 + 34.0 + 2.0;
    let r = FRect { x: right - w, y: cy - h / 2.0, w, h };
    let y = cy - b / 2.0;
    let mut x = r.x + 2.0;
    let mut next = |bw: f32| {
        let q = FRect { x, y, w: bw, h: b };
        x += bw + 2.0;
        q
    };
    let minus = next(b);
    let value = next(44.0);
    let plus = next(b);
    x += 5.0;
    let fit = FRect { x, y, w: b, h: b };
    let actual = FRect { x: x + b + 2.0, y, w: 34.0, h: b };
    ZoomLay { r, minus, value, plus, fit, actual }
}

/// Lay the window out at `w` × `h`; `bar_w`: the docked toolbar's buttons.
pub fn layout(e: &Edit, w: f32, h: f32, bar_w: f32) -> Lay {
    let bar = FRect { x: 0.0, y: 0.0, w, h: BAR_H };
    let status = FRect { x: 0.0, y: h - STATUS_H, w, h: STATUS_H };
    let rw = if e.collapsed { RAIL_MIN } else { RAIL_W };
    let body_h = (h - BAR_H - STATUS_H).max(0.0);
    let rail = FRect { x: 0.0, y: BAR_H, w: rw, h: body_h };
    let canvas = FRect { x: rw, y: BAR_H, w: (w - rw).max(1.0), h: body_h.max(1.0) };
    let in_bar = zoom_lay(w - 8.0, BAR_H / 2.0, 32.0);
    let zoom_in_status = in_bar.r.x < bar_w + 8.0;
    let zoom = if zoom_in_status { zoom_lay(w - 8.0, status.y + STATUS_H / 2.0, 24.0) } else { in_bar };
    let collapse = if e.collapsed {
        FRect { x: (RAIL_MIN - 32.0) / 2.0, y: BAR_H + 6.0, w: 32.0, h: 32.0 }
    } else {
        FRect { x: RAIL_W - 6.0 - 24.0, y: BAR_H + (RAIL_HEAD - 24.0) / 2.0, w: 24.0, h: 24.0 }
    };
    // Top of the list is front-most: the last object first.
    let n = e.canvas.objects().len();
    let mut rows = Vec::new();
    if e.collapsed {
        let mut y = BAR_H + 6.0 + 32.0 + 9.0;
        for i in (0..n).rev() {
            rows.push((FRect { x: (RAIL_MIN - 32.0) / 2.0, y, w: 32.0, h: 32.0 }, i));
            y += 34.0;
        }
    } else {
        let mut y = BAR_H + RAIL_HEAD + 4.0;
        for i in (0..n).rev() {
            rows.push((FRect { x: 6.0, y: y + 2.0, w: RAIL_W - 12.0, h: ROW_H }, i));
            y += ROW_H + 2.0;
        }
    }
    let note_top = rail.y1() - NOTE_H;
    rows.retain(|(r, _)| r.y1() <= note_top);
    Lay { w, h, bar, rail, canvas, status, zoom, zoom_in_status, collapse, rows }
}

fn inside(r: FRect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x1() && y < r.y1()
}

/// What window point (`x`, `y`) (logical px) is on.
pub fn hit(ui: &Ui, e: &Edit, lay: &Lay, x: f32, y: f32) -> Option<Hit> {
    if e.flow.dialog.is_some() {
        return dialog_lay(ui, e, lay).1.into_iter().find(|(r, _)| inside(*r, x, y)).map(|(_, b)| Hit::Dialog(b));
    }
    let z = &lay.zoom;
    for (r, h) in [(z.minus, Hit::ZoomOut), (z.plus, Hit::ZoomIn), (z.fit, Hit::Fit), (z.actual, Hit::Actual), (lay.collapse, Hit::Collapse)] {
        if inside(r, x, y) {
            return Some(h);
        }
    }
    lay.rows.iter().find(|(r, _)| inside(*r, x, y)).map(|(_, i)| Hit::Row(*i))
}

/// An object's name in the list: "Callout "Move this…"", "Number 2", "Arrow".
pub fn obj_name(o: &Obj) -> String {
    let quote = |t: &str| {
        let t = t.trim().replace('\n', " ");
        let short: String = t.chars().take(12).collect();
        if t.chars().count() > 12 { format!("\"{short}\u{2026}\"") } else { format!("\"{short}\"") }
    };
    match o {
        Obj::Counter { n, .. } => format!("Number {n}"),
        Obj::Text { text, .. } if !text.trim().is_empty() => format!("Text {}", quote(text)),
        Obj::Callout { text, .. } if !text.trim().is_empty() => format!("Callout {}", quote(text)),
        o => crate::history::kind_name(o).to_string(),
    }
}

/// An object's icon (its tool's).
pub fn obj_icon(o: &Obj) -> &'static str {
    match o {
        Obj::Path { .. } => "pencil",
        Obj::Line { .. } => "line",
        Obj::Arrow { .. } => "arrow",
        Obj::Rect { .. } | Obj::RectFill { .. } => "rect",
        Obj::Ellipse { .. } | Obj::EllipseFill { .. } => "circle",
        Obj::Marker { .. } | Obj::MarkerPath { .. } => "marker",
        Obj::Text { .. } => "text",
        Obj::Pixelate { .. } => "pixel",
        Obj::Invert { .. } => "invert",
        Obj::Counter { .. } => "counter",
        Obj::Callout { .. } => "callout",
        Obj::Spotlight { .. } => "spotlight",
        Obj::Magnify { .. } => "magnify",
        Obj::Blur { .. } => "blur",
    }
}

// --- painting --------------------------------------------------------------------

fn fill(ui: &mut Ui, r: FRect, rad: f32, c: C4) {
    let q = paint::px(r, ui.k);
    paint::fill(&mut ui.fb, q, rad * ui.k, c);
}

fn ring(ui: &mut Ui, r: FRect, rad: f32, c: C4) {
    let (q, k) = (paint::px(r, ui.k), ui.k);
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

fn text(ui: &mut Ui, size: f32, s: &str, x: f32, cy: f32, c: C4, bold: bool) -> f32 {
    let k = ui.k;
    ui.text_at(size, s, (x * k).round(), cy * k, c, bold);
    ui.text_w(size, s)
}

fn caps(ui: &mut Ui, size: f32, s: &str, x: f32, cy: f32, c: C4) -> f32 {
    let mut cx = x;
    for ch in s.to_uppercase().chars() {
        let t = ch.to_string();
        text(ui, size, &t, cx, cy, c, true);
        cx += ui.text_w(size, &t) + size * 0.07;
    }
    cx - x
}

fn btn_bg(ui: &mut Ui, r: FRect, t: &WTok, on: bool, hov: bool, rad: f32) {
    if on {
        fill(ui, r, rad, t.acbg);
        ring(ui, r, rad, t.acrg);
    } else if hov {
        fill(ui, r, rad, t.hv);
    }
}

/// The strip under the docked toolbar (its buttons are the editor's).
pub fn paint_strip(ui: &mut Ui, lay: &Lay, t: &WTok) {
    fill(ui, lay.bar, 0.0, t.win);
    line_h(ui, 0.0, BAR_H - 1.0, lay.w, t.sep);
}

pub fn paint_zoom(ui: &mut Ui, e: &Edit, lay: &Lay, t: &WTok, hover: Option<Hit>) {
    let z = &lay.zoom;
    fill(ui, z.r, 8.0, t.sf2);
    ring(ui, z.r, 8.0, t.bd);
    let isz = if lay.zoom_in_status { 14.0 } else { 16.0 };
    for (r, h, name) in [(z.minus, Hit::ZoomOut, "minus"), (z.plus, Hit::ZoomIn, "plus")] {
        let hov = hover == Some(h);
        btn_bg(ui, r, t, false, hov, 6.0);
        icon(ui, name, r.x + (r.w - isz) / 2.0, r.y + (r.h - isz) / 2.0, isz, if hov { t.ich } else { t.ic });
    }
    let v = format!("{}%", (e.canvas.zoom() * 100.0).round() as i32);
    let vw = ui.text_w(12.0, &v);
    text(ui, 12.0, &v, z.value.x + (z.value.w - vw) / 2.0, z.value.y + z.value.h / 2.0, t.tx, true);
    line_v(ui, z.plus.x1() + 4.0, z.r.y + (z.r.h - 16.0) / 2.0, 16.0, t.sep);
    let fit_on = e.canvas.is_fit();
    let hov = hover == Some(Hit::Fit);
    btn_bg(ui, z.fit, t, fit_on, hov, 6.0);
    icon(ui, "fit", z.fit.x + (z.fit.w - isz) / 2.0, z.fit.y + (z.fit.h - isz) / 2.0, isz, if fit_on { t.acfg } else if hov { t.ich } else { t.ic });
    let one = (e.canvas.zoom() - 1.0).abs() < 1e-3 && !fit_on;
    let hov = hover == Some(Hit::Actual);
    btn_bg(ui, z.actual, t, one, hov, 6.0);
    let lw = ui.text_w(11.5, "1:1");
    text(ui, 11.5, "1:1", z.actual.x + (z.actual.w - lw) / 2.0, z.actual.y + z.actual.h / 2.0, if one { t.acfg } else if hov { t.ich } else { t.ic }, true);
}

pub fn paint_rail(ui: &mut Ui, e: &Edit, lay: &Lay, t: &WTok, hover: Option<Hit>) {
    let r = lay.rail;
    fill(ui, r, 0.0, t.rail);
    line_v(ui, r.x1() - 1.0, r.y, r.h, t.sep);
    let objs = e.canvas.objects();
    let sel = e.canvas.selected();
    if e.collapsed {
        let b = lay.collapse;
        let hov = hover == Some(Hit::Collapse);
        btn_bg(ui, b, t, false, hov, 7.0);
        icon(ui, "layers", b.x + 8.0, b.y + 8.0, 16.0, if hov { t.ich } else { t.ic });
        line_h(ui, 10.0, b.y1() + 4.0, RAIL_MIN - 20.0, t.sep);
        for (rr, i) in &lay.rows {
            let on = sel.contains(i);
            let hov = hover == Some(Hit::Row(*i));
            btn_bg(ui, *rr, t, on, hov, 7.0);
            let c = if on { t.acfg } else if hov { t.ich } else { t.ic };
            icon(ui, obj_icon(&objs[*i]), rr.x + 8.0, rr.y + 8.0, 16.0, c);
        }
        return;
    }
    let cy = r.y + RAIL_HEAD / 2.0;
    caps(ui, 10.5, &format!("Objects \u{b7} {}", objs.len()), r.x + 12.0, cy, t.tx3);
    let b = lay.collapse;
    let hov = hover == Some(Hit::Collapse);
    btn_bg(ui, b, t, false, hov, 6.0);
    icon(ui, "collapse", b.x + 5.0, b.y + 5.0, 14.0, if hov { t.ich } else { t.ic });
    line_h(ui, r.x, r.y + RAIL_HEAD - 1.0, r.w - 1.0, t.sep);
    let drag = e.rail_drag;
    for (rr, i) in &lay.rows {
        let on = sel.contains(i);
        let hov = hover == Some(Hit::Row(*i));
        let dragged = drag.is_some_and(|d| d.obj == *i && d.to.is_some());
        if on || dragged {
            fill(ui, *rr, 7.0, t.acbg);
        } else if hov {
            fill(ui, *rr, 7.0, t.hv);
        }
        let ry = rr.y + rr.h / 2.0;
        icon(ui, "grip", rr.x + 4.0, ry - 7.0, 14.0, t.tx3);
        let o = &objs[*i];
        let c = crate::objects::edit::color(o).map_or(t.tx3, |c| c.with_alpha(255));
        let k = ui.k;
        ui.fb.fill_circle((rr.x + 26.0) * k, ry * k, 5.0 * k, c);
        let name = ui.fit_text(12.0, &obj_name(o), rr.w - 40.0);
        text(ui, 12.0, &name, rr.x + 38.0, ry, if on { t.acfg } else { t.tx }, true);
    }
    // Where a dragged row would land.
    if let Some(d) = drag
        && let Some(to) = d.to
        && let Some((rr, _)) = lay.rows.iter().find(|(_, i)| *i == to)
    {
        let from_above = lay.rows.iter().position(|(_, i)| *i == d.obj) < lay.rows.iter().position(|(_, i)| *i == to);
        let y = if from_above { rr.y1() + 1.0 } else { rr.y - 1.0 };
        fill(ui, FRect { x: rr.x, y: y - 1.0, w: rr.w, h: 2.0 }, 1.0, t.ac);
    }
    // The note at the bottom.
    let ny = r.y1() - NOTE_H;
    line_h(ui, r.x, ny, r.w - 1.0, t.sep);
    let note = "Top of list is front-most. Drag to reorder \u{b7} Del removes.";
    let lines = crate::ui::controls::wrap(note, r.w - 24.0, |s| ui.text_w(11.0, s));
    for (n, l) in lines.iter().enumerate() {
        text(ui, 11.0, l, r.x + 12.0, ny + 10.0 + 8.0 + n as f32 * 15.0, t.tx3, false);
    }
}

pub fn paint_status(ui: &mut Ui, e: &Edit, lay: &Lay, t: &WTok) {
    let s = lay.status;
    fill(ui, s, 0.0, t.win);
    line_h(ui, 0.0, s.y, s.w, t.sep);
    let cy = s.y + s.h / 2.0;
    let (iw, ih) = e.canvas.size();
    let mut x = 14.0;
    x += text(ui, 11.0, &format!("{iw} \u{d7} {ih}"), x, cy, t.tx, false) + 14.0;
    x += text(ui, 11.0, "\u{b7}", x, cy, t.tx3, false) + 14.0;
    x += text(ui, 11.0, &format!("{}%", (e.canvas.zoom() * 100.0).round() as i32), x, cy, t.tx2, false) + 14.0;
    let unsaved = e.canvas.unsaved();
    if unsaved || e.entry.meta.edited {
        x += text(ui, 11.0, "\u{b7}", x, cy, t.tx3, false) + 14.0;
        if unsaved {
            let k = ui.k;
            ui.fb.fill_circle((x + 3.5) * k, cy * k, 3.5 * k, C4::rgb(0xFF, 0xB4, 0x54));
            x += 13.0;
            text(ui, 11.0, "Edited \u{b7} unsaved changes", x, cy, t.tx2, false);
        } else {
            text(ui, 11.0, "Edited", x, cy, t.tx2, false);
        }
    }
    // Right: the selection's geometry and the pointer.
    let mut right = if lay.zoom_in_status { lay.zoom.r.x - 14.0 } else { s.w - 14.0 };
    let pointer = e.canvas.mouse_img().map(|p| format!("x {}, y {}", p.x.floor() as i32, p.y.floor() as i32));
    if let Some(pt) = &pointer {
        let w = ui.text_w(11.0, pt);
        text(ui, 11.0, pt, right - w, cy, t.tx2, false);
        right -= w + 14.0;
    }
    if let Some((b, o)) = e.canvas.selection_box() {
        let n = e.canvas.selected().len();
        let what = if n > 1 { format!("{n} objects") } else { crate::history::kind_name(o).to_string() };
        let g = format!("{what} \u{b7} {} \u{d7} {} at {}, {}", b.w.round() as i32, b.h.round() as i32, b.x.round() as i32, b.y.round() as i32);
        if pointer.is_some() {
            let dw = ui.text_w(11.0, "\u{b7}");
            text(ui, 11.0, "\u{b7}", right - dw, cy, t.tx3, false);
            right -= dw + 14.0;
        }
        let w = ui.text_w(11.0, &g);
        text(ui, 11.0, &g, right - w, cy, t.tx2, false);
    }
}

/// The question's rect and its buttons (logical px).
pub fn dialog_lay(ui: &Ui, e: &Edit, lay: &Lay) -> (FRect, Vec<(FRect, Btn)>) {
    let Some(d) = e.flow.dialog else { return (FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, Vec::new()) };
    let bw = |l: &str| (ui.text_w(12.0, l) + 24.0).ceil();
    let mut bs = Vec::new();
    let h = match d {
        Dialog::Unsaved => {
            let lines = crate::ui::controls::wrap(&unsaved_text(e), DLG_W - 40.0, |s| ui.text_w(12.0, s)).len().max(1) as f32;
            20.0 + 18.0 + 10.0 + lines * 18.0 + 16.0 + 28.0 + 20.0
        }
        Dialog::Choose { .. } => 20.0 + 18.0 + 10.0 + 2.0 * 50.0 + 10.0 + 10.0 + 18.0 + 16.0 + 28.0 + 20.0,
    };
    let r = FRect { x: ((lay.w - DLG_W) / 2.0).round(), y: ((lay.h - h) / 2.0).round(), w: DLG_W, h };
    let by = r.y1() - 20.0 - 28.0;
    let mut x = r.x1() - 20.0;
    let mut right = |l: &str, b: Btn, bs: &mut Vec<(FRect, Btn)>| {
        let w = bw(l);
        x -= w;
        bs.push((FRect { x, y: by, w, h: 28.0 }, b));
        x -= 8.0;
    };
    match d {
        Dialog::Unsaved => {
            right("Save", Btn::Save, &mut bs);
            right("Save as copy", Btn::SaveCopy, &mut bs);
            right("Cancel", Btn::Cancel, &mut bs);
            bs.push((FRect { x: r.x + 20.0, y: by, w: bw("Discard"), h: 28.0 }, Btn::Discard));
        }
        Dialog::Choose { .. } => {
            right("Save", Btn::Save, &mut bs);
            right("Cancel", Btn::Cancel, &mut bs);
            let oy = r.y + 20.0 + 18.0 + 10.0;
            bs.push((FRect { x: r.x + 20.0, y: oy, w: DLG_W - 40.0, h: 50.0 }, Btn::Pick(Choice::Overwrite)));
            bs.push((FRect { x: r.x + 20.0, y: oy + 60.0, w: DLG_W - 40.0, h: 50.0 }, Btn::Pick(Choice::New)));
            let w = ui.text_w(12.0, "Don't ask again") + 24.0;
            bs.push((FRect { x: r.x + 20.0, y: oy + 120.0, w, h: 18.0 }, Btn::DontAsk));
        }
    }
    (r, bs)
}

fn unsaved_text(e: &Edit) -> String {
    format!("You edited {}. Unsaved annotations will be lost if you discard.", e.file_name())
}

fn button(ui: &mut Ui, r: FRect, label: &str, style: u8, hov: bool, focus: bool, t: &WTok) {
    let (bg, fg, rg) = match style {
        1 => (if hov { t.acbg_hover } else { t.acbg }, t.acfg, t.acrg),
        2 => (t.erbg, t.er, t.errg),
        _ => (t.hv, t.tx, t.bd),
    };
    fill(ui, r, 7.0, bg);
    if hov && style != 1 {
        fill(ui, r, 7.0, t.hv);
    }
    ring(ui, r, 7.0, rg);
    if focus {
        let k = ui.k;
        let q = paint::px(r, k);
        paint::outer_ring(&mut ui.fb, q, 7.0 * k, 0.0, 2.0 * k, t.ac);
    }
    let lw = ui.text_w(12.0, label);
    text(ui, 12.0, label, r.x + (r.w - lw) / 2.0, r.y + r.h / 2.0, fg, false);
}

pub fn paint_dialog(ui: &mut Ui, e: &Edit, lay: &Lay, t: &WTok, hover: Option<Hit>) {
    let Some(d) = e.flow.dialog else { return };
    fill(ui, FRect { x: 0.0, y: 0.0, w: lay.w, h: lay.h }, 0.0, t.scrim);
    let (r, bs) = dialog_lay(ui, e, lay);
    let k = ui.k;
    let q = paint::px(r, k);
    paint::shadow(&mut ui.fb, t, q, 12.0 * k, k, &[0, 1, 2, 3]);
    fill(ui, r, 12.0, t.win);
    ring(ui, r, 12.0, t.bd);
    let hov = |b: Btn| hover == Some(Hit::Dialog(b));
    match d {
        Dialog::Unsaved => {
            text(ui, 14.0, "Save changes to this capture?", r.x + 20.0, r.y + 29.0, t.tx, true);
            let lines = crate::ui::controls::wrap(&unsaved_text(e), DLG_W - 40.0, |s| ui.text_w(12.0, s));
            for (n, l) in lines.iter().enumerate() {
                text(ui, 12.0, l, r.x + 20.0, r.y + 48.0 + 9.0 + n as f32 * 18.0, t.tx2, false);
            }
            for (br, b) in &bs {
                let (label, style) = match b {
                    Btn::Discard => ("Discard", 2),
                    Btn::Cancel => ("Cancel", 0),
                    Btn::SaveCopy => ("Save as copy", 0),
                    _ => ("Save", 1),
                };
                button(ui, *br, label, style, hov(*b), *b == Btn::Save, t);
            }
        }
        Dialog::Choose { choice, dont_ask, .. } => {
            text(ui, 14.0, "Save", r.x + 20.0, r.y + 29.0, t.tx, true);
            let (over, new) = e.choice_files();
            for (br, b) in &bs {
                match b {
                    Btn::Pick(c) => {
                        let on = *c == choice;
                        fill(ui, *br, 8.0, if on { t.acbg } else { t.sf2 });
                        ring(ui, *br, 8.0, if on { t.acrg } else { t.bd });
                        let (cx, cy) = (br.x + 12.0 + 8.0, br.y + br.h / 2.0);
                        let k = ui.k;
                        if on {
                            ui.fb.fill_circle(cx * k, cy * k, 8.0 * k, t.ac);
                            ui.fb.fill_circle(cx * k, cy * k, 4.0 * k, t.win);
                            ui.fb.fill_circle(cx * k, cy * k, 2.5 * k, t.ac);
                        } else {
                            ui.fb.stroke_circle(cx * k, cy * k, 7.25 * k, 1.5 * k, t.cb);
                        }
                        let (label, small) = match c {
                            Choice::Overwrite => ("Overwrite the original", format!("{over} \u{b7} keeps the History entry")),
                            Choice::New => ("Save as new file", new.clone()),
                        };
                        text(ui, 12.5, label, br.x + 38.0, br.y + 17.0, t.tx, true);
                        let small = ui.fit_text(11.0, &small, br.w - 50.0);
                        text(ui, 11.0, &small, br.x + 38.0, br.y + 34.0, t.tx2, false);
                    }
                    Btn::DontAsk => {
                        let bx = FRect { x: br.x, y: br.y + 1.0, w: 16.0, h: 16.0 };
                        if dont_ask {
                            fill(ui, bx, 4.0, t.ac);
                            icon(ui, "check", bx.x + 2.0, bx.y + 2.0, 12.0, C4::rgb(0x0E, 0x10, 0x20));
                        } else {
                            let qb = paint::px(bx, k);
                            ui.fb.stroke_rounded(FRect { x: qb.x + 0.75 * k, y: qb.y + 0.75 * k, w: qb.w - 1.5 * k, h: qb.h - 1.5 * k }, 3.25 * k, 1.5 * k, t.cb);
                        }
                        text(ui, 12.0, "Don't ask again", br.x + 24.0, br.y + 9.0, t.tx2, false);
                    }
                    Btn::Cancel => button(ui, *br, "Cancel", 0, hov(*b), false, t),
                    _ => button(ui, *br, "Save", 1, hov(*b), false, t),
                }
            }
        }
    }
}

/// The toast at the canvas's bottom.
pub fn paint_toast(ui: &mut Ui, e: &Edit, lay: &Lay, now: std::time::Instant) {
    let Some((msg, kind, at)) = &e.toast else { return };
    let age = now.saturating_duration_since(*at).as_secs_f32() * 1000.0;
    let k = crate::anim::ease_out(age / 120.0).min((1.0 - (age - 2200.0) / 150.0).clamp(0.0, 1.0));
    if k <= 0.0 || msg.is_empty() {
        return;
    }
    let cu = crate::editor::style::Ui { th: ui.theme, s: ui.k, font: Some(&crate::fonts::UI) };
    let a = paint::px(lay.canvas, ui.k);
    crate::editor::style::toast(&mut ui.fb, &cu, msg, *kind, a, k);
}
