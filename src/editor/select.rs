//! Editing committed objects in the overlay. The select tool (V) picks
//! the object under a press (Shift toggles it, a drag on empty space is a
//! marquee); with no tool and with the drawing tools a press on an object
//! picks it up as well (the move cursor and a faint outline show it on
//! hover; Ctrl held draws over it instead; the tools acting on what is
//! under them always draw, see `cursor`). A selection moves by dragging (snapping to other objects' edges
//! and the image centre within 6 px, Alt: free; a 40 % ghost stays at the
//! origin), resizes by its square handles, re-aims ends / tails / the
//! loupe by its round handles, nudges with the arrows (Shift: 10 px), and
//! takes Del, Ctrl+D, front / back, color and size from the keys, the
//! toolbar or its mini-toolbar. Every change is one undo step.
//!
//! Callouts: a drag (or a click) with the callout tool places a bubble
//! and starts typing in it: a caret, Enter for a new line, the text wraps
//! to the bubble's width and the bubble grows down; Esc or a click
//! outside finishes. A click on a selected callout (select tool) or on any
//! callout with the callout tool edits its text again.

use super::compose::{CaretUi, ObjSelUi};
use super::mini::{self, MiniIn, MiniUi};
use super::toolbar::Act;
use super::{tools, Edit, Handle, Interact, Tool, CLICK_PX, HANDLE_HIT};
use crate::objects::{callout, edit as oe, CalloutStyle, FRect, Obj, Pt};
use crate::wind::Mods;
use std::time::Instant;

/// Snap reach, logical px.
pub(super) const SNAP_PX: f32 = 6.0;
/// The ghost's opacity.
pub(super) const GHOST: f32 = 0.4;
/// Slack round an object a press still hits, logical px.
const HIT_PX: f32 = 8.0;
/// The snap guides fade out over this long after a drag.
const GUIDE_FADE_MS: f32 = 100.0;
/// Undo steps of the same kind (nudges, wheel ticks) this close coalesce.
const COALESCE: std::time::Duration = std::time::Duration::from_millis(700);
/// What a duplicate is offset by, logical px.
const DUP_PX: f32 = 12.0;
/// The dashed frame's distance outside an object's stroke, logical px.
const FRAME_PX: f32 = 4.0;

/// The object selection and the callout being typed in.
#[derive(Default)]
pub(super) struct ObjState {
    /// Selected objects (indices into `objects`, ascending).
    pub sel: Vec<usize>,
    pub cedit: Option<CalloutEdit>,
    /// Snap guides of the drag.
    pub guides: Vec<(Pt, Pt)>,
    /// The callout shape new bubbles get (the last one chosen).
    pub style: CalloutStyle,
    /// The mini-toolbar item and the handle under the pointer.
    pub mini_hover: Option<usize>,
    pub hot: Option<(bool, usize)>,
    /// The object a press would pick up (outlined faintly).
    pub hover: Option<usize>,
    /// The mini-toolbar as laid out by the last frame (hit tests).
    pub mini: Option<MiniUi>,
    /// The last coalescing undo step: its kind and when.
    pub coalesce: Option<(&'static str, Instant)>,
    /// The objects as they were when the color picker opened over a
    /// selection (Esc puts them back).
    pub pick_backup: Option<Vec<Obj>>,
    /// Snap guides fading out after a drag, and since when.
    pub fading: Option<(Vec<(Pt, Pt)>, Instant)>,
    /// A typed character's UTF-16 high half, waiting for its low one.
    pub high: Option<u16>,
    /// The mini-toolbar's font list is open.
    pub font_menu: bool,
    /// The selection the font list was opened over.
    pub font_menu_sel: Vec<usize>,
}

/// Typing in a callout.
pub(super) struct CalloutEdit {
    pub idx: usize,
    /// Byte index of the caret in the text.
    pub caret: usize,
    /// Last edit, for the caret blink.
    pub at: Instant,
    /// A bubble placed for this edit: removed again if left empty.
    pub fresh: bool,
}

/// The selection's square handles and its round ones.
pub(super) type Handles = (Vec<(Handle, Pt)>, Vec<(OHandle, Pt)>);

/// A handle of the selection.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum OHandle {
    Box(Handle),
    Point(usize, oe::Point),
}

fn corners(b: FRect) -> Vec<(Handle, Pt)> {
    vec![
        (Handle::NW, Pt::new(b.x, b.y)),
        (Handle::NE, Pt::new(b.x1(), b.y)),
        (Handle::SE, Pt::new(b.x1(), b.y1())),
        (Handle::SW, Pt::new(b.x, b.y1())),
    ]
}

/// The tool whose size range an object's size steps in.
fn tool_of(o: &Obj) -> Tool {
    match o {
        Obj::Path { .. } => Tool::Path,
        Obj::Line { .. } => Tool::Line,
        Obj::Arrow { .. } => Tool::Arrow,
        Obj::Rect { .. } | Obj::RectFill { .. } => Tool::Rect,
        Obj::Ellipse { .. } | Obj::EllipseFill { .. } => Tool::Ellipse,
        Obj::Marker { .. } | Obj::MarkerPath { .. } => Tool::Marker,
        Obj::Text { .. } => Tool::Text,
        Obj::Pixelate { .. } => Tool::Pixelate,
        Obj::Invert { .. } => Tool::Invert,
        Obj::Counter { .. } => Tool::Counter,
        Obj::Callout { .. } => Tool::Callout,
        Obj::Spotlight { .. } => Tool::Spotlight,
        Obj::Magnify { .. } => Tool::Magnify,
        Obj::Blur { .. } => Tool::Blur,
        Obj::Cursor { .. } => Tool::Select,
    }
}

/// `b` resized by dragging `h` to `p` (`aspect`: keep the box's shape).
fn resize_box(h: Handle, b: FRect, p: Pt, aspect: bool) -> FRect {
    let (mut x0, mut y0, mut x1, mut y1) = (b.x, b.y, b.x1(), b.y1());
    match h {
        Handle::NW => (x0, y0) = (p.x, p.y),
        Handle::N => y0 = p.y,
        Handle::NE => (x1, y0) = (p.x, p.y),
        Handle::E => x1 = p.x,
        Handle::SE => (x1, y1) = (p.x, p.y),
        Handle::S => y1 = p.y,
        Handle::SW => (x0, y1) = (p.x, p.y),
        Handle::W => x0 = p.x,
    }
    let corner = matches!(h, Handle::NW | Handle::NE | Handle::SE | Handle::SW);
    if aspect && corner && b.w > 0.0 && b.h > 0.0 {
        // The larger relative change wins; the opposite corner stays.
        let k = ((x1 - x0).abs() / b.w).max((y1 - y0).abs() / b.h);
        let (w, hh) = (b.w * k, b.h * k);
        let (ax, ay) = match h {
            Handle::NW => (b.x1(), b.y1()),
            Handle::NE => (b.x, b.y1()),
            Handle::SE => (b.x, b.y),
            _ => (b.x1(), b.y),
        };
        let sx = if matches!(h, Handle::NW | Handle::SW) { -1.0 } else { 1.0 };
        let sy = if matches!(h, Handle::NW | Handle::NE) { -1.0 } else { 1.0 };
        (x0, x1) = (ax, ax + sx * w);
        (y0, y1) = (ay, ay + sy * hh);
    }
    let r = FRect::from_pts(Pt::new(x0, y0), Pt::new(x1, y1));
    FRect { w: r.w.max(2.0), h: r.h.max(2.0), ..r }
}

impl Edit {
    fn hit_slack(&self) -> f32 {
        HIT_PX * self.ui_scale()
    }

    /// The topmost object under `p`.
    pub(super) fn obj_at(&self, p: Pt) -> Option<usize> {
        let tol = self.hit_slack();
        (0..self.objects.len()).rev().find(|&i| oe::hit(&self.objects[i], p, tol, self.font.as_ref()))
    }

    /// The selection's box: the union of the selected objects' boxes.
    pub(super) fn osel_box(&self) -> Option<FRect> {
        let font = self.font.as_ref();
        oe::union(self.obj.sel.iter().filter_map(|&i| self.objects.get(i)).filter_map(|o| oe::sel_box(o, font)))
    }

    /// The dashed frame drawn round `o`: its box out by half its stroke
    /// and 4 px, so it clears the stroke (the handles sit on it).
    fn frame_of(&self, o: &Obj) -> Option<FRect> {
        let b = oe::sel_box(o, self.font.as_ref())?;
        let stroke = match o {
            Obj::Text { .. } | Obj::Counter { .. } | Obj::Callout { .. } | Obj::Pixelate { .. } | Obj::Blur { .. } | Obj::Magnify { .. } => 0.0,
            o => oe::size(o).unwrap_or(0.0),
        };
        let m = stroke / 2.0 + FRAME_PX * self.ui_scale();
        Some(FRect { x: b.x - m, y: b.y - m, w: b.w + 2.0 * m, h: b.h + 2.0 * m })
    }

    /// The selection's frame: the union of the selected objects' frames.
    pub(super) fn osel_frame(&self) -> Option<FRect> {
        oe::union(self.obj.sel.iter().filter_map(|&i| self.objects.get(i)).filter_map(|o| self.frame_of(o)))
    }

    /// Push an undo step when the objects differ from the last one.
    /// While a callout is being typed in, its edit is the step (taken when
    /// it finishes): nothing is pushed now.
    pub(super) fn commit_edits(&mut self) {
        self.obj.coalesce = None;
        if self.obj.cedit.is_none() && self.hist.get(self.hi) != Some(&self.objects) {
            self.push_history();
        }
    }

    /// [`Edit::commit_edits`] where a quick run of the same `kind` (arrow
    /// repeats, wheel ticks) is one undo step.
    pub(super) fn commit_coalesced(&mut self, kind: &'static str) {
        let now = Instant::now();
        let joins = self.obj.coalesce.is_some_and(|(k, at)| k == kind && now.duration_since(at) < COALESCE)
            && self.hi + 1 == self.hist.len()
            && self.hi > 0;
        if joins && self.obj.cedit.is_none() {
            self.hist[self.hi] = self.objects.clone();
        } else {
            self.commit_edits();
        }
        self.obj.coalesce = Some((kind, now));
    }

    /// Esc or a key during an object drag: everything goes back to where
    /// the drag started, and it ends.
    pub(super) fn cancel_obj_drag(&mut self) {
        match std::mem::replace(&mut self.interact, Interact::None) {
            Interact::ObjMove { orig, idx, .. } | Interact::ObjResize { orig, idx, .. } => {
                for (k, i) in idx.into_iter().enumerate() {
                    if let (Some(o), Some(slot)) = (orig.get(k), self.objects.get_mut(i)) {
                        *slot = o.clone();
                    }
                }
            }
            Interact::ObjPoint { idx, orig, .. } => {
                if let Some(slot) = self.objects.get_mut(idx) {
                    *slot = orig;
                }
            }
            Interact::Marquee { base, .. } => self.obj.sel = base,
            other => self.interact = other,
        }
        self.obj.guides.clear();
        self.dirty = true;
    }

    /// The snap guides this frame and their opacity: the drag's, or the
    /// last drag's fading out over 100 ms.
    pub(super) fn guides_now(&mut self, now: Instant) -> (Vec<(Pt, Pt)>, f32) {
        if !self.obj.guides.is_empty() {
            self.obj.fading = None;
            return (self.obj.guides.clone(), 1.0);
        }
        let Some((g, at)) = &self.obj.fading else { return (Vec::new(), 0.0) };
        let t = now.saturating_duration_since(*at).as_secs_f32() * 1000.0;
        if t >= GUIDE_FADE_MS {
            self.obj.fading = None;
            return (Vec::new(), 0.0);
        }
        // Quantised so a frame and its repeat compare equal.
        (g.clone(), ((1.0 - t / GUIDE_FADE_MS) * 16.0).ceil() / 16.0)
    }

    /// The guides are fading (frames are due).
    pub(super) fn guides_fading(&self, now: Instant) -> bool {
        self.obj.fading.as_ref().is_some_and(|(_, at)| now.saturating_duration_since(*at).as_secs_f32() * 1000.0 < GUIDE_FADE_MS + 20.0)
    }

    /// An object drag is on (keys that edit objects wait for its end).
    pub(super) fn obj_dragging(&self) -> bool {
        matches!(self.interact, Interact::ObjMove { .. } | Interact::ObjResize { .. } | Interact::ObjPoint { .. } | Interact::Marquee { .. })
    }

    /// How near a handle a press must be: the usual reach, less on a
    /// small selection so its middle still moves it.
    fn handle_reach(&self) -> f32 {
        let full = HANDLE_HIT * self.ui_scale();
        let small = match self.obj.sel[..] {
            [i] => match self.objects.get(i) {
                Some(Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. }) => (a.x - b.x).hypot(a.y - b.y),
                Some(Obj::Magnify { radius, zoom, .. }) => 2.0 * radius / zoom.max(1.0),
                _ => self.osel_frame().map_or(f32::MAX, |f| f.w.min(f.h)),
            },
            _ => self.osel_frame().map_or(f32::MAX, |f| f.w.min(f.h)),
        };
        (small / 3.0).clamp(4.0 * self.ui_scale(), full)
    }

    /// The selection's square and round handles (none while typing in a
    /// callout or dragging the selection).
    pub(super) fn obj_handles(&self) -> Handles {
        if self.obj.sel.is_empty() || self.obj.cedit.is_some() || matches!(self.interact, Interact::ObjMove { moved: true, .. }) {
            return (Vec::new(), Vec::new());
        }
        let font = self.font.as_ref();
        if let [i] = self.obj.sel[..] {
            let Some(o) = self.objects.get(i) else { return (Vec::new(), Vec::new()) };
            let Some(b) = self.frame_of(o) else { return (Vec::new(), Vec::new()) };
            let _ = font;
            let sq = match oe::boxes(o) {
                oe::Boxes::None => Vec::new(),
                oe::Boxes::Corners => corners(b),
                oe::Boxes::All => super::handle_points(b).to_vec(),
            };
            let round = oe::points(o).into_iter().map(|(w, p)| (OHandle::Point(i, w), p)).collect();
            return (sq, round);
        }
        (self.osel_frame().map(corners).unwrap_or_default(), Vec::new())
    }

    /// The handle under `p` (round ones first: a tail by a corner).
    pub(super) fn obj_handle_at(&self, p: Pt) -> Option<OHandle> {
        let reach = self.handle_reach();
        let near = |q: &Pt| (q.x - p.x).hypot(q.y - p.y) <= reach;
        let (sq, round) = self.obj_handles();
        round.iter().find(|(_, q)| near(q)).map(|(h, _)| *h).or_else(|| sq.iter().find(|(_, q)| near(q)).map(|(h, _)| OHandle::Box(*h)))
    }

    /// The hot handle under `p`, for the chrome: (round?, index).
    pub(super) fn obj_hot_at(&self, p: Pt) -> Option<(bool, usize)> {
        if self.obj.sel.is_empty() {
            return None;
        }
        let reach = self.handle_reach();
        let near = |q: &Pt| (q.x - p.x).hypot(q.y - p.y) <= reach;
        let (sq, round) = self.obj_handles();
        if let Some(i) = round.iter().position(|(_, q)| near(q)) {
            return Some((true, i));
        }
        sq.iter().position(|(_, q)| near(q)).map(|i| (false, i))
    }

    /// A press on a handle of the selection: resize, or move the point.
    pub(super) fn handle_press(&mut self, h: OHandle) {
        self.interact = match h {
            OHandle::Box(handle) => {
                let orig = self.obj.sel.iter().filter_map(|&i| self.objects.get(i).cloned()).collect();
                let (Some(bx), Some(fr)) = (self.osel_box(), self.osel_frame()) else { return };
                Interact::ObjResize { handle, orig, bx, fr, idx: self.obj.sel.clone(), n: self.objects.len() }
            }
            OHandle::Point(idx, which) => {
                let Some(orig) = self.objects.get(idx).cloned() else { return };
                Interact::ObjPoint { idx, which, orig, n: self.objects.len() }
            }
        };
    }

    /// A press with the select tool (or Ctrl with another): the object
    /// under `p` is selected (Shift: toggled) and the selection starts
    /// moving; on empty space a marquee starts (`marquee`: the select
    /// tool). True when the press was taken.
    pub(super) fn select_press(&mut self, p: Pt, mods: Mods, marquee: bool) -> bool {
        if let Some(i) = self.obj_at(p) {
            let was = self.obj.sel.contains(&i);
            if mods.shift {
                if was {
                    self.obj.sel.retain(|&k| k != i);
                } else {
                    self.obj.sel.push(i);
                    crate::sort::sort(&mut self.obj.sel);
                }
                return true;
            }
            if !was {
                self.obj.sel = vec![i];
            }
            let orig = self.obj.sel.iter().filter_map(|&k| self.objects.get(k).cloned()).collect();
            let bx = self.osel_box().unwrap_or(FRect { x: p.x, y: p.y, w: 0.0, h: 0.0 });
            let (idx, n) = (self.obj.sel.clone(), self.objects.len());
            self.interact = Interact::ObjMove { start: p, orig, bx, moved: false, hit: i, was_sel: was, idx, n };
            return true;
        }
        if marquee {
            let base = if mods.shift { self.obj.sel.clone() } else { Vec::new() };
            self.obj.sel = base.clone();
            self.interact = Interact::Marquee { anchor: p, cur: p, base };
            return true;
        }
        false
    }

    /// Where the snapping image is: the capture's selection, else the shot.
    fn snap_image(&self) -> FRect {
        self.sel.unwrap_or(FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 })
    }

    /// The extra shift that snaps box `b` (already moved) to the other
    /// objects' edges or the image centre, and its guides.
    pub(super) fn snap_box(&self, b: FRect) -> (f32, f32, Vec<(Pt, Pt)>) {
        let reach = SNAP_PX * self.ui_scale();
        let font = self.font.as_ref();
        let others: Vec<FRect> = (0..self.objects.len())
            .filter(|i| !self.obj.sel.contains(i))
            .filter_map(|i| oe::sel_box(&self.objects[i], font))
            .collect();
        let img = self.snap_image();
        // (target value, the box it belongs to; `None`: the image centre)
        let best = |tests: [f32; 3], cands: &mut dyn Iterator<Item = (f32, Option<FRect>)>| {
            let mut best: Option<(f32, f32, Option<FRect>)> = None;
            for (c, ob) in cands {
                for t in tests {
                    let d = c - t;
                    if d.abs() <= reach && best.is_none_or(|(bd, _, _)| d.abs() < bd.abs()) {
                        best = Some((d, c, ob));
                    }
                }
            }
            best
        };
        let mut xs = others.iter().flat_map(|o| [(o.x, Some(*o)), (o.x1(), Some(*o))]).chain([(img.x + img.w / 2.0, None)]);
        let mut ys = others.iter().flat_map(|o| [(o.y, Some(*o)), (o.y1(), Some(*o))]).chain([(img.y + img.h / 2.0, None)]);
        let bx = best([b.x, b.x1(), b.x + b.w / 2.0], &mut xs);
        let by = best([b.y, b.y1(), b.y + b.h / 2.0], &mut ys);
        let (sx, sy) = (bx.map_or(0.0, |v| v.0), by.map_or(0.0, |v| v.0));
        let m = FRect { x: b.x + sx, y: b.y + sy, ..b };
        let mut g = Vec::new();
        if let Some((_, c, ob)) = bx {
            let (y0, y1) = match ob {
                Some(o) => (m.y.min(o.y), m.y1().max(o.y1())),
                None => (img.y, img.y1()),
            };
            g.push((Pt::new(c, y0), Pt::new(c, y1)));
        }
        if let Some((_, c, ob)) = by {
            let (x0, x1) = match ob {
                Some(o) => (m.x.min(o.x), m.x1().max(o.x1())),
                None => (img.x, img.x1()),
            };
            g.push((Pt::new(x0, c), Pt::new(x1, c)));
        }
        (sx, sy, g)
    }

    /// A callout keeps its text inside: at least the height it needs.
    /// The annotation font in callout `idx`'s text style (the plain font
    /// for another object).
    pub(super) fn callout_font(&self, idx: usize) -> Option<crate::text::AnnotFont> {
        let f = self.font.as_ref()?;
        Some(match self.objects.get(idx).and_then(oe::fmt) {
            Some(fmt) => f.styled(fmt),
            None => f.clone(),
        })
    }

    pub(super) fn refit_callout(&mut self, idx: usize) {
        self.fit_callout(idx, false, true);
    }

    /// Fit callout `idx`'s height to its text: grow only, or (`shrink`, a
    /// text size change) also shrink. The bubble grows and shrinks away
    /// from its tail (`away`; else down, as while a handle drags it), so
    /// the tail stays as drawn.
    pub(super) fn fit_callout(&mut self, idx: usize, shrink: bool, away: bool) {
        let font = self.callout_font(idx);
        if let Some(Obj::Callout { r, tail, text, size, style, .. }) = self.objects.get_mut(idx) {
            let need = callout::needed_h(font.as_ref(), *r, text, *size, *style);
            let h = if shrink { need } else { need.max(r.h) };
            if h != r.h {
                if away && callout::tail_below(*r, *tail) {
                    r.y -= h - r.h;
                }
                r.h = h;
            }
        }
        self.dirty = true;
    }

    /// A pointer move during an object interaction.
    pub(super) fn obj_drag(&mut self, p: Pt, mods: Mods) {
        // The list changed under the drag (it should not: keys wait for
        // the release): the drag ends where it is.
        let stale = match &self.interact {
            Interact::ObjMove { n, .. } | Interact::ObjResize { n, .. } | Interact::ObjPoint { n, .. } => *n != self.objects.len(),
            _ => false,
        };
        if stale {
            self.interact = Interact::None;
            self.obj.guides.clear();
            return;
        }
        let mut it = std::mem::replace(&mut self.interact, Interact::None);
        match &mut it {
            Interact::ObjMove { start, orig, bx, moved, idx, .. } => {
                if !*moved && (p.x - start.x).hypot(p.y - start.y) < CLICK_PX {
                    self.interact = it;
                    return;
                }
                *moved = true;
                let (mut dx, mut dy) = (p.x - start.x, p.y - start.y);
                self.obj.guides.clear();
                if !mods.alt {
                    let (sx, sy, g) = self.snap_box(FRect { x: bx.x + dx, y: bx.y + dy, ..*bx });
                    dx += sx;
                    dy += sy;
                    self.obj.guides = g;
                }
                for (&i, o0) in idx.iter().zip(orig.iter()) {
                    let mut o = o0.clone();
                    oe::translate(&mut o, dx, dy);
                    if let Some(slot) = self.objects.get_mut(i) {
                        *slot = o;
                    }
                }
                self.dirty = true;
            }
            Interact::ObjResize { handle, orig, bx, fr, idx, .. } => {
                // Ctrl or Shift: keep the shape (Ctrl as for the capture's
                // selection); text, tags and groups always do.
                let keep = idx.len() > 1 || orig.first().is_some_and(|o| oe::boxes(o) == oe::Boxes::Corners) || mods.shift || mods.ctrl;
                // The frame follows the pointer; the box keeps its margins in it.
                let nf = resize_box(*handle, *fr, p, keep);
                let (l, t) = (bx.x - fr.x, bx.y - fr.y);
                let (r, b) = (fr.x1() - bx.x1(), fr.y1() - bx.y1());
                let nb = FRect { x: nf.x + l, y: nf.y + t, w: (nf.w - l - r).max(1.0), h: (nf.h - t - b).max(1.0) };
                for (&i, o0) in idx.iter().zip(orig.iter()) {
                    let mut o = o0.clone();
                    oe::map_box(&mut o, *bx, nb);
                    let callout = matches!(o, Obj::Callout { .. });
                    if let Some(slot) = self.objects.get_mut(i) {
                        *slot = o;
                    }
                    if callout {
                        self.fit_callout(i, false, false);
                    }
                }
                self.dirty = true;
            }
            Interact::ObjPoint { idx, which, orig, .. } => {
                let mut o = orig.clone();
                let mut q = p;
                // Shift: a line's end in 45-degree steps from the other.
                if mods.shift
                    && let Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } = &o
                {
                    let other = if *which == oe::Point::A { *b } else { *a };
                    q = self.snap_point(other, p, true);
                }
                oe::set_point(&mut o, *which, q);
                if let Some(slot) = self.objects.get_mut(*idx) {
                    *slot = o;
                }
                self.dirty = true;
            }
            Interact::Marquee { anchor, cur, base } => {
                *cur = p;
                let r = FRect::from_pts(*anchor, p);
                let font = self.font.as_ref();
                let mut sel = base.clone();
                for (i, o) in self.objects.iter().enumerate() {
                    let hit = oe::sel_box(o, font).is_some_and(|b| b.x <= r.x1() && r.x <= b.x1() && b.y <= r.y1() && r.y <= b.y1());
                    if hit && !sel.contains(&i) {
                        sel.push(i);
                    }
                }
                crate::sort::sort(&mut sel);
                self.obj.sel = sel;
            }
            _ => {}
        }
        self.interact = it;
    }

    /// The end of an object interaction (`it`, taken out of `interact`).
    pub(super) fn obj_release(&mut self, it: Interact) {
        // The guides fade out over 100 ms.
        let g = std::mem::take(&mut self.obj.guides);
        if !g.is_empty() {
            self.obj.fading = Some((g, Instant::now()));
        }
        match it {
            Interact::ObjMove { moved: true, .. } | Interact::ObjResize { .. } | Interact::ObjPoint { .. } => self.commit_edits(),
            Interact::ObjMove { moved: false, hit, was_sel, start, .. } => {
                if was_sel && self.obj.sel.len() > 1 {
                    // A click in a group picks that one.
                    self.obj.sel = vec![hit];
                } else if (was_sel && self.tool == Some(Tool::Select) || self.tool == Some(Tool::Callout))
                    && matches!(self.objects.get(hit), Some(Obj::Callout { .. }))
                {
                    // A click on a bubble (the callout tool: any) types in it.
                    self.begin_callout_edit(hit, Some(start), false);
                }
            }
            _ => {}
        }
    }

    /// Delete, duplicate, restack, restyle or resize the selection (the
    /// mini-toolbar's and the keys' acts); true when handled.
    pub(super) fn obj_act(&mut self, act: Act) -> bool {
        if self.obj.sel.is_empty() {
            return false;
        }
        if let Act::ObjStyle(st) = act {
            self.obj.style = st;
            self.cfg.callout_style = st.id().into();
            self.pending.set("tool_callout_style".into(), Some(crate::config::toml_str(st.id())), Instant::now());
        }
        // Color, fill, size and shape restyle a bubble being typed in
        // (it stays open, a new one included); the others finish it first,
        // keeping a new bubble even when it is still empty.
        let restyle = matches!(act, Act::ObjStyle(_) | Act::ObjSize(_) | Act::ObjFill | Act::ObjColor);
        if !restyle {
            if act != Act::ObjDelete
                && let Some(ce) = self.obj.cedit.as_mut()
            {
                ce.fresh = false;
            }
            self.finish_callout_edit();
        }
        if self.obj.sel.is_empty() {
            return true;
        }
        self.obj.sel.retain(|&i| i < self.objects.len());
        let sel = self.obj.sel.clone();
        match act {
            Act::ObjDelete => {
                for &i in sel.iter().rev() {
                    self.objects.remove(i);
                }
                self.obj.sel.clear();
            }
            Act::ObjDup => {
                let d = DUP_PX * self.ui_scale();
                let n = self.objects.len();
                for &i in &sel {
                    let mut o = self.objects[i].clone();
                    oe::translate(&mut o, d, d);
                    self.objects.push(o);
                }
                self.obj.sel = (n..n + sel.len()).collect();
            }
            Act::ObjFront | Act::ObjBack => {
                let (picked, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.objects).into_iter().enumerate().partition(|(i, _)| sel.contains(i));
                let (picked, rest): (Vec<Obj>, Vec<Obj>) = (picked.into_iter().map(|p| p.1).collect(), rest.into_iter().map(|p| p.1).collect());
                let k = picked.len();
                if act == Act::ObjFront {
                    let n = rest.len();
                    self.objects = rest.into_iter().chain(picked).collect();
                    self.obj.sel = (n..n + k).collect();
                } else {
                    self.objects = picked.into_iter().chain(rest).collect();
                    self.obj.sel = (0..k).collect();
                }
            }
            Act::ObjFill => {
                let on = !sel.iter().filter_map(|&i| oe::fill(&self.objects[i])).next().unwrap_or(false);
                for &i in &sel {
                    oe::set_fill(&mut self.objects[i], on);
                }
            }
            Act::ObjStyle(st) => {
                for &i in &sel {
                    if let Obj::Callout { style, .. } = &mut self.objects[i] {
                        *style = st;
                    }
                    self.refit_callout(i);
                }
            }
            Act::ObjSize(d) => {
                for &i in &sel {
                    let o = &mut self.objects[i];
                    let Some(v) = oe::size(o) else { continue };
                    let (lo, hi) = tools::range(tool_of(o));
                    let step = if matches!(o, Obj::Magnify { .. }) { 5.0 } else { 1.0 };
                    oe::set_size(o, (v + d as f32 * step).clamp(lo, hi));
                    if matches!(o, Obj::Callout { .. }) {
                        self.fit_callout(i, true, true);
                    }
                }
                let v = sel.first().and_then(|&i| oe::size(&self.objects[i]));
                if let Some(v) = v {
                    self.notice = Some(super::Toast::new(tr!("editor.size", n = v as i32), super::ToastKind::Info));
                }
            }
            Act::ObjColor => {
                self.palette_open = true;
                return true;
            }
            _ => return false,
        }
        if matches!(act, Act::ObjSize(_) | Act::ObjFill) {
            self.remember_sel();
        }
        self.dirty = true;
        if matches!(act, Act::ObjSize(_)) {
            self.commit_coalesced("size");
        } else {
            self.commit_edits();
        }
        true
    }

    /// A text style act (B / I / U / S, the alignment, the font list, a
    /// font): on the text draft and the text tool, else on the selected
    /// texts and callouts (the callout being typed in included), and the
    /// tools remember it. A toggle follows the first one (all on, or all
    /// off). False when there is nothing to style.
    pub(super) fn fmt_act(&mut self, act: Act) -> bool {
        use crate::text::TextFmt;
        if act == Act::ObjFontMenu {
            self.obj.font_menu = !self.obj.font_menu;
            self.obj.font_menu_sel = self.obj.sel.clone();
            return true;
        }
        let set = |f: TextFmt, first: TextFmt| -> TextFmt {
            match act {
                Act::ObjBold => TextFmt { bold: !first.bold, ..f },
                Act::ObjItalic => TextFmt { italic: !first.italic, ..f },
                Act::ObjUnderline => TextFmt { underline: !first.underline, ..f },
                Act::ObjStrike => TextFmt { strike: !first.strike, ..f },
                Act::ObjAlign(a) => TextFmt { align: a, ..f },
                Act::ObjFont(fam) => TextFmt { family: fam, ..f },
                _ => f,
            }
        };
        if !matches!(act, Act::ObjBold | Act::ObjItalic | Act::ObjUnderline | Act::ObjStrike | Act::ObjAlign(_) | Act::ObjFont(_)) {
            return false;
        }
        if matches!(act, Act::ObjFont(_)) {
            self.obj.font_menu = false;
        }
        let now = Instant::now();
        let sel: Vec<usize> = self.obj.sel.iter().copied().filter(|&i| self.objects.get(i).and_then(oe::fmt).is_some()).collect();
        if self.text.is_some() || sel.is_empty() {
            // The text draft, or the text / callout tool with nothing
            // selected: the tool's own style.
            let t = match self.tool {
                Some(Tool::Callout) if self.text.is_none() => Tool::Callout,
                Some(Tool::Text) | Some(Tool::Callout) => Tool::Text,
                _ if self.text.is_some() => Tool::Text,
                _ => return false,
            };
            let f = self.mem.fmt(t);
            if self.mem.set_fmt(t, set(f, f)) {
                self.remember_fmt(t, now);
            }
            if let Some(td) = self.text.as_mut() {
                td.at = now;
            }
            return true;
        }
        let first = oe::fmt(&self.objects[sel[0]]).unwrap_or_default();
        for &i in &sel {
            let o = &mut self.objects[i];
            let f = oe::fmt(o).unwrap_or_default();
            oe::set_fmt(o, set(f, first));
            if matches!(o, Obj::Callout { .. }) {
                self.fit_callout(i, true, true);
            }
        }
        for &i in &sel {
            let (t, f) = (tool_of(&self.objects[i]), oe::fmt(&self.objects[i]).unwrap_or_default());
            if self.mem.set_fmt(t, f) {
                self.remember_fmt(t, now);
            }
        }
        if let Some(ce) = self.obj.cedit.as_mut() {
            ce.at = now;
        }
        self.dirty = true;
        self.commit_edits();
        true
    }

    /// Queue `t`'s text style for the config (`tool_<id>_font`, its style
    /// words, the text's alignment).
    fn remember_fmt(&mut self, t: Tool, now: Instant) {
        use crate::config::{fmt_key_name, toml_str, FmtKey};
        let Some(id) = tools::id(t).filter(|_| tools::has_fmt(t)) else { return };
        let f = self.mem.fmt(t);
        let words = f.style_id();
        self.cfg.tool_fonts.insert(id.into(), f.family.id().into());
        self.cfg.tool_text_styles.insert(id.into(), words.clone());
        self.pending.set(fmt_key_name(id, FmtKey::Font), Some(toml_str(f.family.id())), now);
        self.pending.set(fmt_key_name(id, FmtKey::Style), Some(toml_str(&words)), now);
        if t == Tool::Text {
            self.cfg.tool_aligns.insert(id.into(), f.align.id().into());
            self.pending.set(fmt_key_name(id, FmtKey::Align), Some(toml_str(f.align.id())), now);
        }
    }

    /// The selection's size, color and fill become what its tools draw
    /// next (remembered, as when set with the tool itself).
    fn remember_sel(&mut self) {
        let now = Instant::now();
        for &i in &self.obj.sel.clone() {
            let Some(o) = self.objects.get(i) else { continue };
            let t = tool_of(o);
            let (size, color, fill) = (oe::size(o), oe::color(o), oe::fill(o));
            if let Some(v) = size.filter(|_| tools::has_size(t)) {
                self.mem.set_size(t, v);
                self.remember_size(t, now);
            }
            if let Some(c) = color.filter(|_| tools::has_color(t))
                && self.mem.set_color(t, c)
            {
                self.remember_color(t, now);
            }
            if let Some(on) = fill
                && self.mem.set_fill(t, on)
                && let Some(id) = tools::id(t)
            {
                self.cfg.tool_fills.insert(id.into(), on);
                self.pending.set(format!("tool_{id}_fill"), Some(on.to_string()), now);
            }
        }
    }

    /// A color from the palette or the picker for the selection.
    pub(super) fn color_selection(&mut self, c: crate::uifb::C4) -> bool {
        if self.obj.sel.is_empty() {
            return false;
        }
        for &i in &self.obj.sel.clone() {
            if let Some(o) = self.objects.get_mut(i) {
                oe::set_color(o, c);
            }
        }
        self.remember_sel();
        self.dirty = true;
        self.commit_edits();
        true
    }

    /// The color picker over a selection: its color shows on the
    /// objects as it changes (no undo step yet).
    pub(super) fn preview_color(&mut self, c: crate::uifb::C4) {
        if self.obj.pick_backup.is_none() {
            self.obj.pick_backup = Some(self.objects.clone());
        }
        for &i in &self.obj.sel.clone() {
            if let Some(o) = self.objects.get_mut(i) {
                oe::set_color(o, c);
            }
        }
        self.dirty = true;
    }

    /// The picker closed over a selection: `keep` its color (one undo
    /// step) or put the objects back.
    pub(super) fn end_preview_color(&mut self, keep: bool, c: crate::uifb::C4) {
        let backup = self.obj.pick_backup.take();
        if keep {
            self.color_selection(c);
        } else if let Some(b) = backup {
            self.objects = b;
            self.dirty = true;
        }
    }

    /// The selection's color: the first selected object with one.
    pub(super) fn sel_color(&self) -> Option<crate::uifb::C4> {
        self.obj.sel.iter().filter_map(|&i| self.objects.get(i)).find_map(oe::color)
    }

    /// Arrow keys: the selection moves 1 px (Shift: 10) times the scale.
    pub(super) fn nudge(&mut self, dx: f32, dy: f32) {
        let k = self.shot.scale.round().max(1.0);
        for &i in &self.obj.sel.clone() {
            if let Some(o) = self.objects.get_mut(i) {
                oe::translate(o, dx * k, dy * k);
            }
        }
        self.dirty = true;
        self.commit_coalesced("nudge");
    }

    /// Start typing in callout `idx`: the caret at `at` (a click), else at
    /// the end. `fresh`: the bubble was just placed.
    pub(super) fn begin_callout_edit(&mut self, idx: usize, at: Option<Pt>, fresh: bool) {
        let font = self.callout_font(idx);
        let Some(Obj::Callout { r, text, size, style, .. }) = self.objects.get(idx) else { return };
        let caret = match (at, font.as_ref()) {
            (Some(p), Some(f)) => callout::caret_near(f, &callout::layout(f, *r, text, *size, *style), text, p, *size),
            _ => text.len(),
        };
        self.obj.sel = vec![idx];
        self.obj.cedit = Some(CalloutEdit { idx, caret, at: Instant::now(), fresh });
        self.tool = self.tool.filter(|t| *t == Tool::Select || *t == Tool::Callout);
    }

    /// Done typing: a bubble placed for this edit and left empty goes;
    /// otherwise the text is one undo step.
    pub(super) fn finish_callout_edit(&mut self) {
        let Some(ce) = self.obj.cedit.take() else { return };
        let empty = matches!(self.objects.get(ce.idx), Some(Obj::Callout { text, .. }) if text.trim().is_empty());
        if ce.fresh && empty {
            self.objects.remove(ce.idx);
            self.obj.sel.clear();
            self.dirty = true;
            return;
        }
        self.commit_edits();
    }

    /// Place callout `o` (a finished drag of the callout tool) and type in it.
    pub(super) fn place_callout(&mut self, o: Obj) {
        self.objects.push(o);
        let i = self.objects.len() - 1;
        self.refit_callout(i);
        self.begin_callout_edit(i, None, true);
    }

    /// The caret's text and position while typing.
    fn cedit_text(&mut self) -> Option<(&mut String, &mut usize)> {
        let ce = self.obj.cedit.as_mut()?;
        ce.at = Instant::now();
        match self.objects.get_mut(ce.idx) {
            Some(Obj::Callout { text, .. }) => Some((text, &mut ce.caret)),
            _ => None,
        }
    }

    /// A typed character into the callout.
    pub(super) fn callout_char(&mut self, ch: char) {
        let Some((text, caret)) = self.cedit_text() else { return };
        let mut i = (*caret).min(text.len());
        while i > 0 && !text.is_char_boundary(i) {
            i -= 1;
        }
        text.insert(i, ch);
        *caret = i + ch.len_utf8();
        let idx = self.obj.cedit.as_ref().map(|c| c.idx).unwrap_or(0);
        self.refit_callout(idx);
    }

    /// A key while typing in a callout (all keys are taken).
    pub(super) fn callout_key(&mut self, vk: u32) {
        use crate::wind::key;
        if vk == key::ESCAPE {
            self.finish_callout_edit();
            return;
        }
        if vk == key::RETURN {
            self.callout_char('\n');
            return;
        }
        let font = self.obj.cedit.as_ref().and_then(|ce| self.callout_font(ce.idx));
        let Some(ce) = self.obj.cedit.as_ref() else { return };
        let idx = ce.idx;
        let lay = match (&font, self.objects.get(idx)) {
            (Some(f), Some(Obj::Callout { r, text, size, style, .. })) => Some((callout::layout(f, *r, text, *size, *style), *size)),
            _ => None,
        };
        let Some((text, caret)) = self.cedit_text() else { return };
        let c = (*caret).min(text.len());
        if vk == key::LEFT {
            *caret = super::prev_boundary(text, c);
        } else if vk == key::RIGHT {
            *caret = super::next_boundary(text, c);
        } else if vk == key::HOME {
            *caret = 0;
        } else if vk == key::END {
            *caret = text.len();
        } else if vk == key::BACK && c > 0 {
            let p = super::prev_boundary(text, c);
            text.replace_range(p..c, "");
            *caret = p;
        } else if vk == key::DELETE && c < text.len() {
            let n = super::next_boundary(text, c);
            text.replace_range(c..n, "");
        } else if (vk == key::UP || vk == key::DOWN)
            && let (Some(f), Some((lay, px))) = (&font, &lay)
        {
            let at = callout::caret_at(f, lay, text, c, *px);
            let dy = if vk == key::UP { -lay.line_h } else { lay.line_h };
            *caret = callout::caret_near(f, lay, text, Pt::new(at.x, at.y + dy + lay.line_h / 2.0), *px);
        }
        self.refit_callout(idx);
    }

    /// The callout under `p` is being typed in.
    pub(super) fn in_cedit(&self, p: Pt) -> bool {
        self.obj.cedit.as_ref().is_some_and(|c| match self.objects.get(c.idx) {
            Some(Obj::Callout { r, .. }) => super::hit(*r, p),
            _ => false,
        })
    }

    /// Move the caret to a click in the callout being typed in.
    pub(super) fn cedit_click(&mut self, p: Pt) {
        let font = self.obj.cedit.as_ref().and_then(|ce| self.callout_font(ce.idx));
        let Some(ce) = self.obj.cedit.as_mut() else { return };
        if let (Some(f), Some(Obj::Callout { r, text, size, style, .. })) = (&font, self.objects.get(ce.idx)) {
            ce.caret = callout::caret_near(f, &callout::layout(f, *r, text, *size, *style), text, p, *size);
            ce.at = Instant::now();
        }
    }

    /// The caret blink phase while typing in a callout.
    pub(super) fn cedit_caret_on(&self) -> Option<bool> {
        self.obj.cedit.as_ref().map(|c| super::blink_on(c.at))
    }

    /// What the frame shows of the objects: the ghost, the selection
    /// chrome, guides, marquee, caret and mini-toolbar.
    #[allow(clippy::type_complexity)]
    pub(super) fn obj_scene(&mut self, area: FRect, s: f32) -> (Vec<Obj>, Option<ObjSelUi>, Option<FRect>, Option<CaretUi>, Option<MiniUi>) {
        let ghost = match &self.interact {
            Interact::ObjMove { moved: true, orig, .. } => orig.iter().filter_map(|o| oe::faded(o, GHOST)).collect(),
            _ => Vec::new(),
        };
        let marquee = match &self.interact {
            Interact::Marquee { anchor, cur, .. } => Some(FRect::from_pts(*anchor, *cur)),
            _ => None,
        };
        let dragging = matches!(self.interact, Interact::ObjMove { moved: true, .. });
        let osel = (!self.obj.sel.is_empty() && !dragging).then(|| {
            let (sq, round) = self.obj_handles();
            let mut u = ObjSelUi { square: sq.iter().map(|h| h.1).collect(), round: round.iter().map(|h| h.1).collect(), hot: self.obj.hot, ..Default::default() };
            match self.obj.sel[..] {
                [i] => match &self.objects[i] {
                    Obj::Line { a, b, .. } | Obj::Arrow { a, b, .. } | Obj::Marker { a, b, .. } => u.line = Some((*a, *b)),
                    o => u.boxes.extend(self.frame_of(o)),
                },
                _ => u.boxes.extend(self.osel_frame()),
            }
            u
        });
        let caret = self.obj.cedit.as_ref().filter(|c| super::blink_on(c.at)).and_then(|ce| {
            let f = &self.callout_font(ce.idx)?;
            let Some(Obj::Callout { r, text, size, style, color, fill, .. }) = self.objects.get(ce.idx) else { return None };
            let lay = callout::layout(f, *r, text, *size, *style);
            let at = callout::caret_at(f, &lay, text, ce.caret, *size);
            let ink = if *fill { crate::objects::counter_ink(*color) } else { *color };
            Some(CaretUi { r: FRect { x: at.x, y: at.y, w: (2.0 * s).round().max(1.0), h: lay.line_h.round() }, color: ink.with_alpha(255) })
        });
        let idle = matches!(self.interact, Interact::None);
        // The object a press would pick up: a faint outline.
        let hover = self.obj.hover.filter(|i| idle && !self.obj.sel.contains(i)).and_then(|i| self.objects.get(i));
        let osel = match hover {
            Some(o) => {
                // A frame for lines too (a line along one is hidden by it).
                let mut u = osel.unwrap_or_default();
                u.hover.extend(self.frame_of(o));
                Some(u)
            }
            None => osel,
        };
        // The font list belongs to what it was opened over: another
        // selection (a click on the next text) closes it.
        if self.obj.font_menu && self.obj.font_menu_sel != self.obj.sel {
            self.obj.font_menu = false;
        }
        let font_open = self.obj.font_menu;
        let mini = (idle && !self.obj.sel.is_empty()).then(|| self.osel_frame()).flatten().map(|b| {
            let sel = &self.obj.sel;
            let one = (sel.len() == 1).then(|| &self.objects[sel[0]]);
            let inp = MiniIn {
                color: self.sel_color(),
                fill: one.and_then(oe::fill),
                size: one.and_then(oe::size),
                text: one.is_some_and(|o| matches!(o, Obj::Text { .. } | Obj::Callout { .. })),
                style: one.and_then(|o| match o {
                    Obj::Callout { style, .. } => Some(*style),
                    _ => None,
                }),
                count: sel.len(),
                fmt: one.and_then(oe::fmt),
                align: one.is_some_and(|o| matches!(o, Obj::Text { .. })),
                draft: false,
                font_open,
            };
            let mut m = mini::layout(&inp, b, area, s);
            m.hover = self.obj.mini_hover.filter(|i| *i < m.items.len());
            m
        });
        // The text draft's bar: its style, over its box.
        let mini = mini.or_else(|| {
            let td = self.text.as_ref()?;
            let tf = self.text_font();
            let b = super::text_box_rect(self.shot.size.0 as f32, td, self.mem.size(Tool::Text), tf.as_ref(), s);
            let inp = MiniIn {
                color: None,
                fill: None,
                size: None,
                text: true,
                style: None,
                count: 1,
                fmt: Some(self.mem.fmt(Tool::Text)),
                align: true,
                draft: true,
                font_open,
            };
            let mut m = mini::layout(&inp, b, area, s);
            m.hover = self.obj.mini_hover.filter(|i| *i < m.items.len());
            Some(m)
        });
        if mini.as_ref().is_none_or(|m| m.inp.fmt.is_none()) {
            self.obj.font_menu = false;
        }
        self.obj.mini = mini.clone();
        (ghost, osel, marquee, caret, mini)
    }
}
