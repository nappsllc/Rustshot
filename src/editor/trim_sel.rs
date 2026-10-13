//! Trim to content in the overlay: the selection tightened to what is in
//! it ([`crate::trim::content`] on the capture under it, objects left
//! out), from the size label's menu, its key (Shift+A) or, with Settings ›
//! Capture "Trim margins automatically" (`trim_margins`), every new region
//! or window (not in video mode). A window snap whose own edges are not
//! uniform (a title bar and frame) trims its client area instead, when that
//! has margins; otherwise the window stays whole.
//! Each trim is an undo step that puts the selection back.

use super::*;
use crate::snap::R;

/// The toast when a trim finds nothing to cut.
pub(super) const NOTHING: crate::i18n::Key = key!("editor.nothing_to_trim");

impl Edit {
    /// The selection tightened to its content; `None` when it stays.
    pub(super) fn trimmed_sel(&self) -> Option<FRect> {
        let sel = self.sel?;
        // Whole pixels, as the export crops it.
        let (x0, y0) = (sel.x.floor().max(0.0), sel.y.floor().max(0.0));
        let img = self.export_plain(sel);
        let at = |(x, y, w, h): crate::trim::Rect, ox: u32, oy: u32| FRect {
            x: x0 + (ox + x) as f32,
            y: y0 + (oy + y) as f32,
            w: w as f32,
            h: h as f32,
        };
        if let Some(r) = crate::trim::content(&img) {
            return Some(at(r, 0, 0));
        }
        // A window snap (the selection is a window's rect): its client
        // area, when that has margins to trim (else the window stays
        // whole, title bar and frame included).
        let (iw, ih) = img.dimensions();
        let me = R::new(x0 as i32, y0 as i32, x0 as i32 + iw as i32, y0 as i32 + ih as i32);
        let c = self.snap.as_ref()?.client_of(me)?.intersect(&me);
        if c.is_empty() || c == me {
            return None;
        }
        let (cx, cy) = ((c.x0 - me.x0) as u32, (c.y0 - me.y0) as u32);
        let sub = img.crop(cx, cy, c.w() as u32, c.h() as u32);
        crate::trim::content(&sub).map(|r| at(r, cx, cy))
    }

    /// Trim the selection (an undo step); false (and a toast when `say`)
    /// when there was nothing to trim.
    pub(super) fn trim_selection(&mut self, say: bool) -> bool {
        self.size_menu = false;
        let Some(new) = self.trimmed_sel() else {
            if say && self.sel.is_some() {
                self.notice = Some(Toast::new(NOTHING.to_string(), ToastKind::Info));
            }
            return false;
        };
        let old = self.sel;
        self.sel = Some(new);
        // A lasso stays (it is in the capture's pixels; the export cuts it).
        self.rebind_mask(old);
        // A window's area cut down is a region now.
        self.rec.source = rec_ui::RecSource::Region;
        self.obj.coalesce = None;
        self.push_history();
        if let Some(last) = self.sel_hist.last_mut() {
            *last = Some((old, new));
        }
        true
    }

    /// Over the selection's size label where a press opens its menu: in
    /// the overlay, not in video mode, with no drawing tool (or the
    /// select tool); with one, a press there draws as anywhere else.
    pub(super) fn on_label(&self, p: Pt) -> bool {
        self.host.is_none()
            && self.sel.is_some()
            && !self.rec.open
            && matches!(self.tool, None | Some(Tool::Select))
            && self.label_at.is_some_and(|r| hit(r, p))
    }
}
