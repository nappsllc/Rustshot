//! Snipping Tool parity in the editor (the overlay and the Edit window):
//!
//! - **Text mode** (Copy text, "Let me select"): the selection's words
//!   are recognized on a worker ([`crate::ocr::spawn_lines`]) and shown
//!   as regions over the picture (a light accent tint, the rest dimmed a
//!   little); a drag selects a range of words in reading order, Shift+click
//!   extends it, a double-click takes a word, a triple-click its line,
//!   Ctrl+A all; Ctrl+C or the floating Copy pill copies the selected
//!   words (line breaks kept), Copy all everything; Esc leaves.
//! - **Quick Redact** (Shift+R, the Copy text menu): the sensitive text
//!   ([`crate::ocr::redact`], the kinds in `redact_kinds`) becomes Blur
//!   objects over its words (padded 2 px), all in one undo step.
//! - **Freeform** (Shift+F, the lasso next to the mode switch): the next
//!   selection is a lasso; its bounding box is the selection, outside it
//!   the export is transparent (PNG) or white (JPG, BMP).
//! - **Crop** (C): a frame with handles inside the selection (the
//!   overlay) or the picture (the Edit window, where it is kept as the
//!   document's `crop`, non-destructive); Enter crops, Esc cancels.

use super::*;
use super::chrome::Ui;
use crate::objects::mask;
use crate::uifb::Fb;
use crate::ocr::textsel::{self, Pos};
use crate::ocr::{self, Line, OcrError, redact};

/// Quick Redact's blur strength (a light one: the words are small).
pub const REDACT_SIGMA: f32 = 8.0;
/// Around each matched word box.
pub const REDACT_PAD: f32 = 2.0;
/// Clicks closer than this in time and space count together (double,
/// triple click).
const MULTI_CLICK: Duration = Duration::from_millis(450);
const MULTI_PX: f32 = 4.0;
/// Text mode's dim over the recognized area and its tints.
const TEXT_DIM: u8 = 46;
const WORD_TINT: u8 = 40;
const HL_TINT: u8 = 110;
/// The crop frame: what lies outside it in its bounds is dimmed this much.
const CROP_DIM: u8 = 120;
/// Text mode's bar and pill (logical px).
const BAR_H: f32 = 32.0;
const BAR_PAD: f32 = 4.0;
const BAR_GAP: f32 = 8.0;
const BTN_PAD: f32 = 12.0;
const PILL_H: f32 = 26.0;

/// What a recognition is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Want {
    Text,
    Redact,
}

/// A recognition on its worker: where its words go.
pub(super) struct OcrRun {
    pub rx: Receiver<Result<Vec<Line>, OcrError>>,
    pub want: Want,
    /// The crop's top-left in the capture, and the area recognized.
    pub origin: Pt,
    pub area: FRect,
}

/// Text mode: the words (capture px, reading order) and the selection.
pub(super) struct TextMode {
    pub lines: Vec<Line>,
    pub area: FRect,
    /// (anchor, focus): the words a drag started at and reached.
    pub sel: Option<(Pos, Pos)>,
    /// The last press: when, where, and how many in a row.
    pub clicks: Option<(Instant, Pt, u8)>,
}

/// What a crop frame drag holds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Grab {
    Edge(Handle),
    Move,
    /// A new frame from this corner.
    New,
}

/// The crop frame while it is up.
pub(super) struct CropMode {
    pub r: FRect,
    /// What it may cover: the selection (overlay) or the picture.
    pub bounds: FRect,
    /// The drag: what, from where, the frame then.
    pub grab: Option<(Grab, Pt, FRect)>,
}

/// The lasso's coverage over its box (capture px), shared by the frames
/// that draw it (compared by pointer).
#[derive(Debug)]
pub struct MaskCov {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub a: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SharedCov(pub Arc<MaskCov>);

impl PartialEq for SharedCov {
    fn eq(&self, o: &SharedCov) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

/// The editor's Snipping Tool state.
#[derive(Default)]
pub(super) struct Snip {
    pub ocr: Option<OcrRun>,
    pub text: Option<TextMode>,
    pub crop_mode: Option<CropMode>,
    /// The Edit window's crop (non-destructive), and its undo history in
    /// step with the objects' (`Edit::hist`).
    pub crop: Option<FRect>,
    pub crop_hist: Vec<Option<FRect>>,
    /// The next selection is a lasso.
    pub freeform: bool,
    /// The lasso being drawn.
    pub lasso: Vec<Pt>,
    /// The selection's lasso and the selection it belongs to (another
    /// selection drops it).
    pub mask: Option<Vec<Pt>>,
    pub mask_sel: Option<FRect>,
    /// A move or resize of the selection: the lasso and selection it
    /// started from.
    pub mask_drag: Option<(FRect, Vec<Pt>)>,
    pub cov: Option<SharedCov>,
    /// The Copy text menu, and the button held since (a long press opens
    /// the menu, a click is Copy text).
    pub copy_menu: bool,
    pub copy_hold: Option<Instant>,
}

/// One of Text mode's buttons.
#[derive(Clone, Debug, PartialEq)]
pub struct BarBtn {
    pub act: Act,
    pub r: FRect,
    pub label: &'static str,
    pub icon: &'static str,
    pub on: bool,
}

/// Text mode's bar (Copy all, Redact) by the recognized area, and the
/// Copy pill by the words selected.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBar {
    pub r: FRect,
    pub btns: Vec<BarBtn>,
    pub pill: Option<BarBtn>,
    pub hover: Option<Act>,
}

impl TextBar {
    pub fn act_at(&self, p: Pt) -> Option<Act> {
        self.pill.iter().chain(&self.btns).find(|b| hit(b.r, p) && b.on).map(|b| b.act)
    }

    pub fn contains(&self, p: Pt) -> bool {
        hit(self.r, p) || self.pill.as_ref().is_some_and(|b| hit(b.r, p))
    }
}

/// What a frame paints of the snip state.
#[derive(Clone, Debug, PartialEq)]
pub struct SnipUi {
    /// Text mode: the area recognized (dimmed a little), the words, the
    /// selected ones (one box per line), its bar.
    pub area: Option<FRect>,
    pub words: Vec<FRect>,
    pub hl: Vec<FRect>,
    pub bar: Option<TextBar>,
    /// The lasso being drawn (open), the selection's lasso (closed) and
    /// its coverage (dimmed outside, like outside the selection).
    pub lasso: Vec<Pt>,
    pub outline: Vec<Pt>,
    pub cov: Option<SharedCov>,
    pub dim_a: u8,
    /// The crop frame and its bounds.
    pub crop: Option<(FRect, FRect)>,
}

fn fr(b: textsel::Box4) -> FRect {
    FRect { x: b.x, y: b.y, w: b.w, h: b.h }
}

fn grow(r: FRect, g: f32) -> FRect {
    FRect { x: r.x - g, y: r.y - g, w: r.w + 2.0 * g, h: r.h + 2.0 * g }
}

/// The lines moved by `origin` (the crop's place in the capture).
pub fn offset_lines(lines: Vec<Line>, origin: Pt) -> Vec<Line> {
    lines
        .into_iter()
        .map(|mut l| {
            for w in &mut l.words {
                w.x += origin.x;
                w.y += origin.y;
            }
            l
        })
        .collect()
}

/// The blurs Quick Redact makes: per hit, its words' boxes on its line
/// joined and padded [`REDACT_PAD`], clamped to `bounds`.
pub fn redact_rects(lines: &[Line], hits: &[redact::Hit], bounds: FRect) -> Vec<FRect> {
    let mut out = Vec::new();
    for h in hits {
        let Some(l) = lines.get(h.line) else { continue };
        let boxes: Vec<textsel::Box4> = h.words.clone().filter_map(|wi| textsel::word_box(std::slice::from_ref(l), (0, wi))).collect();
        let Some(u) = boxes.into_iter().map(fr).reduce(toolbar::union) else { continue };
        let g = grow(u, REDACT_PAD);
        let x0 = g.x.max(bounds.x);
        let y0 = g.y.max(bounds.y);
        let x1 = g.x1().min(bounds.x1());
        let y1 = g.y1().min(bounds.y1());
        if x1 > x0 && y1 > y0 {
            out.push(FRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
        }
    }
    out
}

/// The words `a`..=`b` as hits (Redact selected words: one per line).
pub fn selection_hits(lines: &[Line], a: Pos, b: Pos) -> Vec<redact::Hit> {
    textsel::per_line(lines, a, b).into_iter().map(|(li, w0, w1)| redact::Hit { kind: redact::Kind::Url, line: li, words: w0..w1 + 1 }).collect()
}

/// "Redacted 3 items" / "Redacted 1 item" / "Nothing sensitive found".
pub fn redact_note(n: usize) -> (String, ToastKind) {
    match n {
        0 => ("Nothing sensitive found".into(), ToastKind::Info),
        1 => ("Redacted 1 item".into(), ToastKind::Success),
        n => (format!("Redacted {n} items"), ToastKind::Success),
    }
}

/// `pts` mapped from rect `a` to rect `b` (the selection moved or resized).
pub fn map_pts(pts: &[Pt], a: FRect, b: FRect) -> Vec<Pt> {
    let sx = if a.w > 0.0 { b.w / a.w } else { 1.0 };
    let sy = if a.h > 0.0 { b.h / a.h } else { 1.0 };
    pts.iter().map(|p| Pt::new(b.x + (p.x - a.x) * sx, b.y + (p.y - a.y) * sy)).collect()
}

/// A selection on whole pixels around the lasso, inside the capture.
pub fn lasso_sel(pts: &[Pt], img: (f32, f32)) -> FRect {
    let b = mask::bbox(pts);
    let x0 = b.x.floor().clamp(0.0, img.0);
    let y0 = b.y.floor().clamp(0.0, img.1);
    let x1 = b.x1().ceil().clamp(0.0, img.0);
    let y1 = b.y1().ceil().clamp(0.0, img.1);
    FRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// The crop frame `orig` dragged by `grab` from `start` to `p`, inside
/// `bounds` (on whole pixels).
pub fn crop_drag(grab: Grab, start: Pt, orig: FRect, p: Pt, bounds: FRect) -> FRect {
    let p = Pt::new(p.x.clamp(bounds.x, bounds.x1()), p.y.clamp(bounds.y, bounds.y1()));
    let r = match grab {
        Grab::Move => {
            let x = (orig.x + p.x - start.x).round().clamp(bounds.x, (bounds.x1() - orig.w).max(bounds.x));
            let y = (orig.y + p.y - start.y).round().clamp(bounds.y, (bounds.y1() - orig.h).max(bounds.y));
            FRect { x, y, ..orig }
        }
        Grab::New => FRect::from_pts(Pt::new(start.x.round(), start.y.round()), Pt::new(p.x.round(), p.y.round())),
        Grab::Edge(h) => {
            let (mut x0, mut y0, mut x1, mut y1) = (orig.x, orig.y, orig.x1(), orig.y1());
            let (px, py) = (p.x.round(), p.y.round());
            if matches!(h, Handle::W | Handle::NW | Handle::SW) {
                x0 = px;
            }
            if matches!(h, Handle::E | Handle::NE | Handle::SE) {
                x1 = px;
            }
            if matches!(h, Handle::N | Handle::NW | Handle::NE) {
                y0 = py;
            }
            if matches!(h, Handle::S | Handle::SW | Handle::SE) {
                y1 = py;
            }
            FRect { x: x0.min(x1), y: y0.min(y1), w: (x1 - x0).abs(), h: (y1 - y0).abs() }
        }
    };
    let x0 = r.x.max(bounds.x);
    let y0 = r.y.max(bounds.y);
    FRect { x: x0, y: y0, w: (r.x1().min(bounds.x1()) - x0).max(0.0), h: (r.y1().min(bounds.y1()) - y0).max(0.0) }
}

impl Edit {
    /// A recognition runs, or the Copy text button is held: the timer
    /// keeps ticking.
    pub(super) fn snip_busy(&self) -> bool {
        self.snip.ocr.is_some() || self.snip.copy_hold.is_some()
    }

    /// What Copy text and Redact read: the selection (the whole picture
    /// in the Edit window).
    fn ocr_area(&self) -> FRect {
        let full = FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 };
        let r = self.sel.unwrap_or(full);
        match self.snip.crop.filter(|_| self.host.is_some()) {
            Some(c) => mask::crop_rect(c, self.shot.size).unwrap_or(r),
            None => r,
        }
    }

    /// Recognize the area's words on a worker, for `want`.
    pub(super) fn start_ocr(&mut self, want: Want) {
        if self.snip.ocr.as_ref().is_some_and(|o| o.want == want) {
            return;
        }
        let area = self.ocr_area();
        if area.w < 1.0 || area.h < 1.0 {
            return;
        }
        if self.dirty {
            self.rebuild();
        }
        let img = self.export(area);
        let origin = Pt::new(area.x.floor().max(0.0), area.y.floor().max(0.0));
        self.snip.ocr = Some(OcrRun { rx: ocr::spawn_lines(img), want, origin, area });
        let msg = if want == Want::Text { "Recognizing text\u{2026}" } else { "Looking for sensitive text\u{2026}" };
        self.notice = Some(Toast::new(msg, ToastKind::Info));
    }

    /// A recognition finished: Text mode comes up, or Redact blurs.
    /// Returns whether something changed (a frame is due).
    pub(super) fn poll_snip(&mut self) -> bool {
        let Some(run) = &self.snip.ocr else { return false };
        let res = match run.rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(OcrError::Failed("the recognizer stopped".into())),
        };
        let run = self.snip.ocr.take().expect("a run");
        self.ocr_done(run.want, run.origin, run.area, res);
        true
    }

    /// What a recognition found, for `want`.
    pub(super) fn ocr_done(&mut self, want: Want, origin: Pt, area: FRect, res: Result<Vec<Line>, OcrError>) {
        let lines = match res {
            Ok(l) => offset_lines(ocr::ordered(l), origin),
            Err(e) => {
                let n = ocr::note(&Err(e), Ok(()));
                self.notice = Some(Toast::new(n.text, n.kind));
                return;
            }
        };
        match want {
            Want::Text if lines.is_empty() => self.notice = Some(Toast::new("No text found", ToastKind::Info)),
            Want::Text => {
                let n = lines.len();
                self.snip.text = Some(TextMode { lines, area, sel: None, clicks: None });
                let lines = if n == 1 { "1 line".to_string() } else { format!("{n} lines") };
                self.notice = Some(Toast::new(format!("Select text to copy \u{b7} {lines}"), ToastKind::Info));
            }
            Want::Redact => self.redact_lines(&lines, None, area),
        }
    }

    /// The kinds Quick Redact looks for (`redact_kinds`).
    pub(super) fn redact_kinds(&self) -> Vec<redact::Kind> {
        redact::parse_kinds(&self.cfg.redact_kinds)
    }

    /// Blur the sensitive words of `lines` (or the words `only`), one undo
    /// step, and say how many.
    pub(super) fn redact_lines(&mut self, lines: &[Line], only: Option<(Pos, Pos)>, area: FRect) {
        let hits = match only {
            Some((a, b)) => selection_hits(lines, a, b),
            None => redact::find(lines, &self.redact_kinds()),
        };
        let rects = redact_rects(lines, &hits, area);
        let n = rects.len();
        if n > 0 {
            self.finish_callout_edit();
            self.objects.extend(rects.into_iter().map(|r| Obj::Blur { r, sigma: REDACT_SIGMA }));
            self.push_history();
            self.dirty = true;
            self.obj.sel.clear();
        }
        let (msg, kind) = redact_note(n);
        self.notice = Some(Toast::new(msg, kind));
    }

    /// Quick Redact: in Text mode, the words selected (else every
    /// sensitive one it shows); otherwise recognize the area first.
    pub(super) fn redact(&mut self) {
        if let Some(tm) = self.snip.text.take() {
            let only = tm.sel;
            self.redact_lines(&tm.lines, only, tm.area);
            // The words stay; the selection is used up.
            self.snip.text = Some(TextMode { sel: None, ..tm });
            return;
        }
        self.start_ocr(Want::Redact);
    }

    /// A kind of sensitive text on / off (remembered).
    pub(super) fn toggle_redact_kind(&mut self, k: redact::Kind) {
        let mut kinds = self.redact_kinds();
        if let Some(i) = kinds.iter().position(|x| *x == k) {
            kinds.remove(i);
        } else {
            kinds.push(k);
        }
        let v = redact::kinds_text(&kinds);
        self.cfg.redact_kinds = v.clone();
        self.pending.set("redact_kinds".into(), Some(config::toml_str(&v)), Instant::now());
    }

    /// Copy text (the button, O): Text mode, or everything at once
    /// (`copy_text_mode = "all"`); in Text mode it leaves it.
    pub(super) fn copy_text(&mut self) {
        if self.snip.text.is_some() || self.snip.ocr.as_ref().is_some_and(|o| o.want == Want::Text) {
            self.leave_text_mode();
            return;
        }
        if self.cfg.copy_text_mode.trim() == "all" {
            self.copy_all();
        } else {
            self.start_ocr(Want::Text);
        }
    }

    /// Copy every word: Text mode's at once; otherwise the old Copy text
    /// (recognize and copy, the overlay closes).
    pub(super) fn copy_all(&mut self) {
        if let Some(tm) = &self.snip.text {
            let text = ocr::join_lines(tm.lines.clone());
            self.copy_and_say(text);
            return;
        }
        self.tasks = vec![Task::Text];
        self.done = true;
    }

    /// Copy the words selected in Text mode.
    pub(super) fn copy_selected(&mut self) -> bool {
        let Some(tm) = &self.snip.text else { return false };
        let Some((a, b)) = tm.sel else { return false };
        let text = textsel::text(&tm.lines, a, b);
        self.copy_and_say(text);
        true
    }

    fn copy_and_say(&mut self, text: String) {
        let res = Ok(text);
        let copied = match &res {
            Ok(t) if !t.trim().is_empty() => ocr::copy(t),
            _ => Ok(()),
        };
        let n = ocr::note(&res, copied);
        self.notice = Some(Toast::new(n.text, n.kind));
    }

    pub(super) fn leave_text_mode(&mut self) {
        self.snip.text = None;
        if self.snip.ocr.as_ref().is_some_and(|o| o.want == Want::Text) {
            self.snip.ocr = None;
            self.notice = None;
        }
        if matches!(self.interact, Interact::Snip) {
            self.interact = Interact::None;
        }
    }

    /// Text mode's bar and pill, laid out for this frame (capture px).
    pub(super) fn text_bar(&self, area: FRect, s: f32) -> Option<TextBar> {
        let tm = self.snip.text.as_ref()?;
        let ui = Ui { th: &self.th, s, font: self.ui_font };
        let sel = tm.sel;
        let btn_w = |label: &str| chrome::tw(&ui, 12.5, label) + 2.0 * BTN_PAD * s + 20.0 * s;
        let mut btns = Vec::new();
        let redact_label = if sel.is_some() { "Redact selected" } else { "Redact" };
        let defs = [(Act::CopyAll, "Copy all", "copy", true), (Act::Redact, redact_label, "redact", true)];
        let w: f32 = defs.iter().map(|d| btn_w(d.1)).sum::<f32>() + 2.0 * BAR_PAD * s + BAR_PAD * s;
        let h = BAR_H * s;
        let ta = tm.area;
        let x = (ta.x + ta.w / 2.0 - w / 2.0).clamp(area.x + 8.0 * s, (area.x1() - 8.0 * s - w).max(area.x + 8.0 * s));
        // Inside the area along its bottom (the toolbar is below it, the
        // mode switch above); a short area: above it.
        let g = BAR_GAP * s;
        let inside = ta.y1() - g - h;
        let y = if ta.h >= h + 2.0 * g + 24.0 * s || ta.y - g - h < area.y + 8.0 * s { inside.max(area.y + 8.0 * s) } else { ta.y - g - h };
        let mut bx = x + BAR_PAD * s;
        for (act, label, icon, on) in defs {
            let bw = btn_w(label);
            btns.push(BarBtn { act, r: FRect { x: bx, y: y + BAR_PAD * s, w: bw, h: h - 2.0 * BAR_PAD * s }, label, icon, on });
            bx += bw + BAR_PAD * s;
        }
        let pill = sel.and_then(|(a, b)| {
            let hl = textsel::highlight(&tm.lines, a, b);
            let last = hl.last().copied()?;
            let pw = chrome::tw(&ui, 12.5, "Copy") + 2.0 * BTN_PAD * s + 20.0 * s;
            let ph = PILL_H * s;
            let px = (last.x1() - pw).max(area.x + 4.0 * s).min(area.x1() - pw - 4.0 * s);
            let py = if last.y1() + 6.0 * s + ph <= area.y1() { last.y1() + 6.0 * s } else { last.y - 6.0 * s - ph };
            Some(BarBtn { act: Act::TextCopy, r: FRect { x: px, y: py, w: pw, h: ph }, label: "Copy", icon: "copy", on: true })
        });
        Some(TextBar { r: FRect { x, y, w, h }, btns, pill, hover: None })
    }

    /// The snip state as this frame paints it (capture px).
    pub(super) fn snip_scene(&mut self, area: FRect, s: f32, dim_a: u8, mouse: Pt) -> Option<SnipUi> {
        self.refresh_cov();
        let mut u = SnipUi { area: None, words: Vec::new(), hl: Vec::new(), bar: None, lasso: Vec::new(), outline: Vec::new(), cov: None, dim_a, crop: None };
        let mut any = false;
        if let Some(tm) = &self.snip.text {
            u.area = Some(tm.area);
            u.words = tm.lines.iter().flat_map(|l| l.words.iter()).map(|w| FRect { x: w.x, y: w.y, w: w.w, h: w.h }).collect();
            if let Some((a, b)) = tm.sel {
                u.hl = textsel::highlight(&tm.lines, a, b).into_iter().map(fr).collect();
            }
            u.bar = self.text_bar(area, s).map(|mut b| {
                b.hover = b.act_at(mouse);
                b
            });
            any = true;
        }
        if matches!(self.interact, Interact::NewSel { .. }) && self.snip.freeform && self.snip.lasso.len() > 1 {
            u.lasso = self.snip.lasso.clone();
            any = true;
        }
        if let Some(m) = self.live_mask() {
            u.outline = m.to_vec();
            // While the selection is dragged only the outline follows.
            if !matches!(self.interact, Interact::MoveSel { .. } | Interact::Resize { .. }) {
                u.cov = self.snip.cov.clone();
            }
            any = true;
        }
        if let Some(c) = &self.snip.crop_mode {
            u.crop = Some((c.r, c.bounds));
            any = true;
        }
        any.then_some(u)
    }

    // ---- freeform ----

    /// The selection's lasso, while it is that selection's.
    pub(super) fn live_mask(&self) -> Option<&[Pt]> {
        let m = self.snip.mask.as_deref()?;
        (self.snip.mask_sel.is_some() && self.snip.mask_sel == self.sel).then_some(m)
    }

    /// The lasso's coverage, made again when the lasso changed.
    fn refresh_cov(&mut self) {
        let Some(m) = self.live_mask().map(<[Pt]>::to_vec) else {
            self.snip.cov = None;
            return;
        };
        let b = mask::bbox(&m);
        let (x, y) = (b.x.floor() as i32, b.y.floor() as i32);
        let (w, h) = ((b.x1().ceil() as i32 - x).max(1) as u32, (b.y1().ceil() as i32 - y).max(1) as u32);
        if self.snip.cov.as_ref().is_some_and(|c| (c.0.x, c.0.y, c.0.w, c.0.h) == (x, y, w, h) && self.snip.mask_drag.is_none()) {
            return;
        }
        let a = mask::coverage(&m, w, h, Pt::new(x as f32, y as f32));
        self.snip.cov = Some(SharedCov(Arc::new(MaskCov { x, y, w, h, a })));
    }

    /// Freeform on / off (remembered); a selection there was goes (the
    /// next one is drawn in the new mode).
    pub(super) fn toggle_freeform(&mut self) {
        self.snip.freeform = !self.snip.freeform;
        let v = if self.snip.freeform { "freeform" } else { "rect" };
        self.cfg.selection_mode = v.into();
        self.pending.set("selection_mode".into(), Some(config::toml_str(v)), Instant::now());
        if self.host.is_none() && self.sel.is_some() && self.tasks.is_empty() {
            self.sel = None;
            self.snip.mask = None;
            self.snip.mask_sel = None;
            self.leave_text_mode();
            self.snip.crop_mode = None;
            self.obj.sel.clear();
        }
        let msg = if self.snip.freeform { "Freeform: drag a lasso to select" } else { "Rectangle selection" };
        self.notice = Some(Toast::new(msg, ToastKind::Info));
    }

    /// The lasso was let go: a selection around it (its box), the lasso
    /// kept for the export; too small a lasso selects nothing.
    pub(super) fn finish_lasso(&mut self) -> bool {
        let pts = mask::simplify(&std::mem::take(&mut self.snip.lasso));
        let img = (self.shot.size.0 as f32, self.shot.size.1 as f32);
        if !mask::is_shape(&pts) {
            self.sel = None;
            return false;
        }
        let r = lasso_sel(&pts, img);
        if r.w < 2.0 || r.h < 2.0 {
            self.sel = None;
            return false;
        }
        self.sel = Some(r);
        self.snip.mask = Some(pts);
        self.snip.mask_sel = Some(r);
        self.snip.cov = None;
        true
    }

    /// The selection moved or was resized: its lasso follows.
    pub(super) fn follow_mask(&mut self) {
        let orig = match &self.interact {
            Interact::MoveSel { orig, .. } | Interact::Resize { orig, .. } => *orig,
            _ => return,
        };
        if self.snip.mask_drag.is_none() {
            match (&self.snip.mask, self.snip.mask_sel) {
                (Some(m), Some(ms)) if ms == orig => self.snip.mask_drag = Some((orig, m.clone())),
                _ => return,
            }
        }
        if let (Some((a, base)), Some(b)) = (&self.snip.mask_drag, self.sel) {
            self.snip.mask = Some(map_pts(base, *a, b));
            self.snip.mask_sel = Some(b);
        }
    }

    /// The selection went from `a` to `b` (an arrow key): the lasso too.
    pub(super) fn fit_mask(&mut self, a: FRect, b: FRect) {
        if self.snip.mask_sel == Some(a)
            && let Some(m) = &self.snip.mask
        {
            self.snip.mask = Some(map_pts(m, a, b));
            self.snip.mask_sel = Some(b);
        }
    }

    // ---- the crop frame ----

    /// What the crop frame covers: the selection, or the picture.
    fn crop_bounds(&self) -> Option<FRect> {
        let full = FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 };
        if self.host.is_some() { Some(full) } else { self.sel }
    }

    /// Crop on / off: the frame comes up at the crop (the Edit window) or
    /// the selection.
    pub(super) fn toggle_crop(&mut self) {
        if self.snip.crop_mode.take().is_some() {
            return;
        }
        let Some(bounds) = self.crop_bounds() else { return };
        self.leave_text_mode();
        self.finish_callout_edit();
        self.obj.sel.clear();
        let r = self.snip.crop.filter(|_| self.host.is_some()).unwrap_or(bounds);
        self.snip.crop_mode = Some(CropMode { r, bounds, grab: None });
        self.notice = Some(Toast::new("Drag the frame \u{b7} Enter crops \u{b7} Esc cancels", ToastKind::Info));
    }

    /// Enter: crop to the frame (the overlay: the selection becomes it;
    /// the Edit window: the document's crop, one undo step).
    pub(super) fn apply_crop(&mut self) {
        let Some(c) = self.snip.crop_mode.take() else { return };
        let r = FRect { x: c.r.x.round(), y: c.r.y.round(), w: c.r.w.round(), h: c.r.h.round() };
        if r.w < 1.0 || r.h < 1.0 {
            return;
        }
        if self.host.is_some() {
            let full = FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 };
            let new = (r != full).then_some(r);
            if new != self.snip.crop {
                self.snip.crop = new;
                self.push_history();
                self.dirty = true;
            }
        } else if self.sel != Some(r) {
            // A crop of a lasso keeps the lasso (clipped by the export).
            let lasso = self.live_mask().map(<[Pt]>::to_vec);
            self.sel = Some(r);
            if let Some(m) = lasso {
                self.snip.mask = Some(m);
                self.snip.mask_sel = Some(r);
            }
            self.rec.source = rec_ui::RecSource::Region;
        }
    }

    /// A press while the crop frame is up (always taken).
    pub(super) fn crop_press(&mut self, p: Pt) {
        let s = self.ui_scale();
        let Some(c) = self.snip.crop_mode.as_mut() else { return };
        let grab = handle_points(c.r)
            .into_iter()
            .find(|(_, q)| (q.x - p.x).hypot(q.y - p.y) <= HANDLE_HIT * s)
            .map(|(h, _)| Grab::Edge(h))
            .or_else(|| hit(c.r, p).then_some(Grab::Move))
            .or_else(|| hit(c.bounds, p).then_some(Grab::New));
        if let Some(g) = grab {
            c.grab = Some((g, p, c.r));
            self.interact = Interact::Snip;
        }
    }

    // ---- Text mode's input ----

    /// A press in Text mode: its bar, else a word (a selection starts;
    /// Shift extends, two clicks a word, three a line); elsewhere in the
    /// area it clears the selection. False: outside (not taken).
    pub(super) fn text_press(&mut self, p: Pt, mods: Mods) -> bool {
        let area = self.area_drawn.unwrap_or(FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 });
        let s = self.ui_scale();
        if let Some(bar) = self.text_bar(area, s)
            && bar.contains(p)
        {
            if let Some(act) = bar.act_at(p) {
                self.apply_act(act, p);
            }
            return true;
        }
        let now = Instant::now();
        let Some(tm) = self.snip.text.as_mut() else { return false };
        let slack = 4.0 * s;
        if !textsel::over_text(&tm.lines, (p.x, p.y), slack) {
            if hit(tm.area, p) {
                tm.sel = None;
                return true;
            }
            return false;
        }
        let Some(at) = textsel::nearest(&tm.lines, (p.x, p.y)) else { return true };
        let n = match tm.clicks {
            Some((t, q, n)) if now.saturating_duration_since(t) < MULTI_CLICK && (q.x - p.x).hypot(q.y - p.y) <= MULTI_PX * s => (n % 3) + 1,
            _ => 1,
        };
        tm.clicks = Some((now, p, n));
        match n {
            2 => tm.sel = Some((at, at)),
            3 => tm.sel = textsel::line_span(&tm.lines, at.0),
            _ if mods.shift && tm.sel.is_some() => {
                let (a, _) = tm.sel.expect("a selection");
                tm.sel = Some((a, at));
                self.interact = Interact::Snip;
            }
            _ => {
                tm.sel = Some((at, at));
                self.interact = Interact::Snip;
            }
        }
        true
    }

    /// A drag of the crop frame or of a text selection.
    pub(super) fn snip_drag(&mut self, p: Pt) {
        if let Some(c) = self.snip.crop_mode.as_mut()
            && let Some((g, start, orig)) = c.grab
        {
            c.r = crop_drag(g, start, orig, p, c.bounds);
            return;
        }
        if let Some(tm) = self.snip.text.as_mut()
            && let Some((a, _)) = tm.sel
            && let Some(at) = textsel::nearest(&tm.lines, (p.x, p.y))
        {
            tm.sel = Some((a, at));
        }
    }

    /// The drag ended.
    pub(super) fn snip_release(&mut self) {
        if let Some(c) = self.snip.crop_mode.as_mut()
            && let Some((g, _, orig)) = c.grab.take()
            && (c.r.w < 2.0 || c.r.h < 2.0)
        {
            // A click (or a sliver): the frame as it was (a new frame: none).
            c.r = if g == Grab::New { c.bounds } else { orig };
        }
    }

    /// Keys while Text mode or the crop frame is up; true: taken.
    pub(super) fn snip_key(&mut self, vk: u32, mods: Mods) -> bool {
        let escape = vk == wind::key::ESCAPE;
        let enter = vk == wind::key::RETURN;
        if self.snip.crop_mode.is_some() {
            if escape {
                self.snip.crop_mode = None;
                return true;
            }
            if enter {
                self.apply_crop();
                return true;
            }
            return false;
        }
        if self.snip.ocr.is_some() && escape {
            self.snip.ocr = None;
            self.notice = None;
            return true;
        }
        if self.snip.copy_menu && escape {
            self.snip.copy_menu = false;
            return true;
        }
        if self.snip.text.is_none() {
            return false;
        }
        if escape {
            self.leave_text_mode();
            return true;
        }
        let plain_ctrl = mods.ctrl && !mods.alt && !mods.shift;
        if (plain_ctrl && vk == 'C' as u32) || enter {
            if !self.copy_selected() && enter {
                self.copy_all();
            }
            return true;
        }
        if plain_ctrl && vk == 'A' as u32 {
            if let Some(tm) = self.snip.text.as_mut() {
                tm.sel = textsel::all(&tm.lines);
            }
            return true;
        }
        false
    }

    /// The pointer over Text mode's words (the I-beam).
    pub(super) fn over_words(&self, p: Pt) -> bool {
        self.snip.text.as_ref().is_some_and(|tm| textsel::over_text(&tm.lines, (p.x, p.y), 4.0 * self.ui_scale()))
    }

    /// The Copy text button held long enough: its menu opens.
    pub(super) fn copy_long_press(&mut self, now: Instant) {
        if self.snip.copy_hold.is_some_and(|t| now.saturating_duration_since(t) >= Self::SLOT_HOLD) {
            self.snip.copy_hold = None;
            self.snip.copy_menu = true;
            self.palette_open = false;
            self.save_menu = false;
            self.slot_menu = false;
        }
    }

    /// The Copy text menu as the toolbar lays it out.
    pub(super) fn copy_menu_view(&self) -> Option<toolbar::CopyMenu> {
        self.snip.copy_menu.then(|| toolbar::CopyMenu {
            keys: toolbar::COPY_MENU.map(|(a, _)| chrome::act_tip(a, &self.keys).1.join("+")),
            kinds: self.redact_kinds(),
        })
    }

    /// The cursor the crop frame or Text mode wants at `p`.
    pub(super) fn snip_cursor(&self, p: Pt) -> Option<Cursor> {
        if let Some(c) = &self.snip.crop_mode {
            let s = self.ui_scale();
            if let Some((h, _)) = handle_points(c.r).into_iter().find(|(_, q)| (q.x - p.x).hypot(q.y - p.y) <= HANDLE_HIT * s) {
                return Some(handle_cursor(h));
            }
            return Some(if hit(c.r, p) { Cursor::Move } else { Cursor::Cross });
        }
        let tm = self.snip.text.as_ref()?;
        let area = self.area_drawn.unwrap_or(tm.area);
        if self.text_bar(area, self.ui_scale()).is_some_and(|b| b.contains(p)) {
            return Some(Cursor::Arrow);
        }
        if self.over_words(p) {
            return Some(Cursor::IBeam);
        }
        hit(tm.area, p).then_some(Cursor::Arrow)
    }

    /// The export of selection `sel` with the lasso applied (outside it
    /// transparent); `img`'s top-left is the selection's, floored.
    pub(super) fn masked(&self, mut img: PixBuf, sel: FRect) -> PixBuf {
        if let Some(m) = self.live_mask() {
            mask::apply(&mut img, m, Pt::new(sel.x.floor().max(0.0), sel.y.floor().max(0.0)));
        }
        img
    }

    /// The lasso in the export's pixels (the history keeps it).
    pub(super) fn mask_in(&self, sel: FRect) -> Option<Vec<Pt>> {
        self.live_mask().map(|m| mask::moved(m, -sel.x.floor().max(0.0), -sel.y.floor().max(0.0)))
    }

    /// Edit window: the crop applies to what it exports.
    pub(super) fn crop_of_doc(&self) -> Option<FRect> {
        self.snip.crop.and_then(|c| mask::crop_rect(c, self.shot.size))
    }
}

// ---- painting ----

/// `u` mapped into the Edit window (`r`: a rect, `p`: a point); the
/// lasso's coverage stays out (the window hides outside it itself).
pub fn map_ui(u: &SnipUi, r: &dyn Fn(FRect) -> FRect, p: &dyn Fn(Pt) -> Pt) -> SnipUi {
    let btn = |b: &BarBtn| BarBtn { r: r(b.r), ..b.clone() };
    SnipUi {
        area: u.area.map(r),
        words: u.words.iter().map(|w| r(*w)).collect(),
        hl: u.hl.iter().map(|w| r(*w)).collect(),
        bar: u.bar.as_ref().map(|b| TextBar { r: r(b.r), btns: b.btns.iter().map(btn).collect(), pill: b.pill.as_ref().map(btn), hover: b.hover }),
        lasso: u.lasso.iter().map(|q| p(*q)).collect(),
        outline: u.outline.iter().map(|q| p(*q)).collect(),
        cov: None,
        dim_a: u.dim_a,
        crop: u.crop.map(|(a, b)| (r(a), r(b))),
    }
}

/// Bounds of what `u` paints.
pub fn bounds(ui: &Ui, u: &SnipUi) -> Vec<FRect> {
    let mut v = Vec::new();
    if let Some(a) = u.area {
        v.push(grow(a, 2.0));
    }
    if let Some(b) = &u.bar {
        // The surfaces' shadows reach past them.
        v.push(chrome::surface_bounds(ui, b.r));
        if let Some(p) = &b.pill {
            v.push(chrome::surface_bounds(ui, p.r));
        }
    }
    let lw = ui.px(3.0) + 2.0;
    if u.lasso.len() > 1 {
        v.push(grow(mask::bbox(&u.lasso), lw));
    }
    if !u.outline.is_empty() {
        v.push(grow(mask::bbox(&u.outline), lw));
    }
    if let Some((r, b)) = u.crop {
        v.push(grow(b, 1.0));
        v.extend(chrome::selection_bounds(ui, r));
    }
    v
}

/// Paint `u` (capture px) into `f`.
pub fn draw(f: &mut Fb, ui: &Ui, u: &SnipUi) {
    let th = ui.th;
    // Outside the lasso inside its box: dimmed like outside the selection.
    if let Some(c) = &u.cov
        && u.dim_a > 0
    {
        let c = &c.0;
        let (bx0, by0, bx1, by1) = f.bounds();
        let (x0, y0) = (c.x.max(bx0), c.y.max(by0));
        let (x1, y1) = ((c.x + c.w as i32).min(bx1), (c.y + c.h as i32).min(by1));
        for y in y0..y1 {
            let row = ((y - c.y) as usize) * c.w as usize;
            for x in x0..x1 {
                let cov = c.a[row + (x - c.x) as usize];
                if cov < 255 {
                    let a = (u.dim_a as u32 * (255 - cov as u32) / 255) as u8;
                    f.blend_px(x, y, th.dim.with_alpha(a));
                }
            }
        }
    }
    let lw = ui.px(1.5);
    if u.lasso.len() > 1 {
        let mut sf = f.surf();
        sf.stroke_polyline(&u.lasso, lw + ui.px(2.0), th.sel_outer, crate::raster::Blend::Normal);
        sf.stroke_polyline(&u.lasso, lw, th.accent, crate::raster::Blend::Normal);
    }
    if u.outline.len() > 2 {
        let mut closed = u.outline.clone();
        closed.push(u.outline[0]);
        let mut sf = f.surf();
        sf.stroke_polyline(&closed, lw + ui.px(2.0), th.sel_outer, crate::raster::Blend::Normal);
        sf.stroke_polyline(&closed, lw, th.accent, crate::raster::Blend::Normal);
    }
    if let Some(a) = u.area {
        let r = chrome::round(a);
        f.fill_rect(r.x as i32, r.y as i32, r.w as i32, r.h as i32, th.dim.with_alpha(TEXT_DIM));
        for w in &u.words {
            let r = chrome::round(grow(*w, ui.px(1.0)));
            chrome::fill(f, r, ui.px(3.0), th.accent.with_alpha(WORD_TINT));
        }
        for h in &u.hl {
            let r = chrome::round(grow(*h, ui.px(2.0)));
            chrome::fill(f, r, ui.px(3.0), th.accent.with_alpha(HL_TINT));
        }
    }
    if let Some(b) = &u.bar {
        chrome::surface(f, ui, b.r, 10.0, &chrome::ALL_LAYERS, 1.0);
        for btn in &b.btns {
            draw_btn(f, ui, btn, b.hover == Some(btn.act));
        }
        if let Some(p) = &b.pill {
            chrome::surface(f, ui, p.r, 8.0, &chrome::SMALL_LAYERS, 1.0);
            draw_btn(f, ui, p, b.hover == Some(p.act));
        }
    }
    if let Some((r, bounds)) = u.crop {
        let dim = th.dim.with_alpha(CROP_DIM);
        let o = chrome::round(bounds);
        let i = chrome::round(r);
        let bands = [
            (o.x, o.y, o.w, i.y - o.y),
            (o.x, i.y1(), o.w, o.y1() - i.y1()),
            (o.x, i.y, i.x - o.x, i.h),
            (i.x1(), i.y, o.x1() - i.x1(), i.h),
        ];
        for (x, y, w, h) in bands {
            f.fill_rect(x as i32, y as i32, w.ceil() as i32, h.ceil() as i32, dim);
        }
        // Thirds, then the frame and its handles.
        let line = th.sel_inner.with_alpha(110);
        for k in [1.0f32, 2.0] {
            let x = (i.x + i.w * k / 3.0).round() as i32;
            let y = (i.y + i.h * k / 3.0).round() as i32;
            f.fill_rect(x, i.y as i32, 1, i.h as i32, line);
            f.fill_rect(i.x as i32, y, i.w as i32, 1, line);
        }
        chrome::selection(f, ui, r, None, 1.0);
    }
}

fn draw_btn(f: &mut Fb, ui: &Ui, b: &BarBtn, hovered: bool) {
    let th = ui.th;
    let r = chrome::round(b.r);
    if hovered {
        chrome::fill(f, r, ui.px(7.0), th.bg_hover);
    }
    let fg = if b.on { if hovered { th.icon_hover } else { th.text } } else { th.text_muted };
    let isz = ui.px(16.0);
    let cy = r.y + r.h / 2.0;
    let x = r.x + ui.px(BTN_PAD);
    crate::icon_path::draw_icon(f, b.icon, x, cy - isz / 2.0, isz, fg);
    chrome::text(f, ui, 12.5, b.label, x + isz + ui.px(4.0), cy, fg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::Word;

    fn line(y: f32, words: &[&str]) -> Line {
        Line { words: words.iter().enumerate().map(|(i, t)| Word { text: (*t).into(), x: 10.0 + i as f32 * 80.0, y, w: 70.0, h: 14.0 }).collect() }
    }

    #[test]
    fn redact_boxes_join_a_hits_words_padded() {
        let lines = vec![line(20.0, &["Card", "4111", "1111", "1111", "1111"]), line(50.0, &["mail", "a@b.com"])];
        let hits = redact::find(&lines, &redact::Kind::ALL);
        let bounds = FRect { x: 0.0, y: 0.0, w: 1000.0, h: 1000.0 };
        let r = redact_rects(&lines, &hits, bounds);
        assert_eq!(r, vec![FRect { x: 88.0, y: 18.0, w: 314.0, h: 18.0 }, FRect { x: 88.0, y: 48.0, w: 74.0, h: 18.0 }]);
        // Clamped to the area.
        let r = redact_rects(&lines, &hits, FRect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 });
        assert_eq!(r, vec![FRect { x: 88.0, y: 18.0, w: 112.0, h: 18.0 }]);
        assert_eq!(redact_note(0).0, "Nothing sensitive found");
        assert_eq!(redact_note(1).0, "Redacted 1 item");
        assert_eq!(redact_note(3).0, "Redacted 3 items");
        // Selected words: one blur per line.
        let s = selection_hits(&lines, (0, 3), (1, 0));
        assert_eq!(redact_rects(&lines, &s, bounds).len(), 2);
    }

    #[test]
    fn a_lasso_maps_with_its_selection() {
        let pts = vec![Pt::new(10.0, 10.0), Pt::new(30.0, 10.0), Pt::new(20.0, 30.0)];
        let a = FRect { x: 10.0, y: 10.0, w: 20.0, h: 20.0 };
        let b = FRect { x: 110.0, y: 20.0, w: 40.0, h: 20.0 };
        assert_eq!(map_pts(&pts, a, b), vec![Pt::new(110.0, 20.0), Pt::new(150.0, 20.0), Pt::new(130.0, 40.0)]);
        assert_eq!(lasso_sel(&[Pt::new(-3.0, 4.5), Pt::new(20.2, 9.0)], (15.0, 100.0)), FRect { x: 0.0, y: 4.0, w: 15.0, h: 5.0 });
    }

    #[test]
    fn the_crop_frame_stays_in_its_bounds() {
        let b = FRect { x: 100.0, y: 100.0, w: 400.0, h: 300.0 };
        let r = FRect { x: 150.0, y: 150.0, w: 100.0, h: 80.0 };
        assert_eq!(crop_drag(Grab::Move, Pt::new(200.0, 200.0), r, Pt::new(1000.0, 200.0), b), FRect { x: 400.0, ..r });
        assert_eq!(crop_drag(Grab::Edge(Handle::SE), Pt::new(250.0, 230.0), r, Pt::new(260.4, 999.0), b), FRect { x: 150.0, y: 150.0, w: 110.0, h: 250.0 });
        assert_eq!(crop_drag(Grab::Edge(Handle::W), Pt::new(150.0, 190.0), r, Pt::new(0.0, 190.0), b), FRect { x: 100.0, y: 150.0, w: 150.0, h: 80.0 });
        // Past the opposite edge it flips.
        assert_eq!(crop_drag(Grab::Edge(Handle::E), Pt::new(250.0, 190.0), r, Pt::new(120.0, 190.0), b), FRect { x: 120.0, y: 150.0, w: 30.0, h: 80.0 });
        assert_eq!(crop_drag(Grab::New, Pt::new(300.0, 300.0), r, Pt::new(200.0, 250.0), b), FRect { x: 200.0, y: 250.0, w: 100.0, h: 50.0 });
    }
}
