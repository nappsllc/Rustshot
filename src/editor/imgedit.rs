//! The Edit window's image edits in the editor ([`crate::imgedit`]):
//! Rotate, Flip, Resize, Canvas and Reset are undo steps like any other.
//!
//! The editor works on the picture as it shows: turned and flipped. The
//! base is the original in the current orientation, the objects and the
//! crop are in its pixels; every undo step keeps its objects and crop in
//! the frame of its own orientation (`snip.edits_hist`), so undo and redo
//! restore them exactly and only turn the picture. A new orientation maps
//! the objects and the crop with the picture (the upright rule:
//! [`crate::imgedit`]). What is saved is mapped back into the original's
//! pixels ([`Edit::doc_objects`], [`Edit::doc_crop`]). The lasso of a
//! freeform capture is kept in the original's pixels and mapped from
//! there each time. Resize and Canvas apply on export only.

use super::*;
use crate::imgedit::{Edits, Orient};

impl Edit {
    /// Turn the picture (the base, the sizes, the selection, the lasso)
    /// to orientation `to`; the objects and the crop are the caller's.
    fn turn_picture(&mut self, to: Orient) {
        let from = self.snip.edits.orient;
        if from == to {
            return;
        }
        let rel = from.inverse().then(to);
        self.base = rel.image(&self.base);
        let (w, h) = self.base.dimensions();
        self.shot.size = (w, h);
        self.shot.monitors = vec![(0, 0, w, h)];
        self.sel = Some(FRect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 });
        self.composed = None;
        self.baked.clear();
        self.frame = PixBuf::default();
        self.scene = None;
        self.painted = None;
        self.area_drawn = None;
        self.dirty = true;
        let doc = self.snip.doc_size;
        self.snip.mask = self.snip.doc_mask.as_ref().map(|m| m.iter().map(|p| to.pt(*p, doc)).collect());
        self.snip.mask_sel = self.sel;
        self.snip.cov = None;
        self.snip.edits.orient = to;
    }

    /// Typing, Text mode, the crop frame, the object selection and a drag
    /// end before the picture changes under them.
    /// `commit`: typing becomes an object (an undo step); else (inside an
    /// undo or a redo, which must never push a step) it is dropped.
    fn settle_for_image_edit(&mut self, commit: bool) {
        if commit {
            if self.text.is_some() {
                self.commit_text();
            }
            self.finish_callout_edit();
        } else {
            self.text = None;
            self.obj.cedit = None;
        }
        self.leave_text_mode();
        self.snip.crop_mode = None;
        self.obj.sel.clear();
        self.interact = Interact::None;
        self.draft = None;
    }

    /// New image edits as one undo step: a new orientation turns the
    /// picture and maps the objects and the crop with it.
    pub(super) fn set_image_edits(&mut self, new: Edits) {
        if new == self.snip.edits {
            return;
        }
        self.settle_for_image_edit(true);
        let from = self.snip.edits.orient;
        if new.orient != from {
            let rel = from.inverse().then(new.orient);
            let size = self.shot.size;
            let font = self.font.as_ref();
            self.objects = self.objects.iter().map(|o| rel.map_obj(o, size, font)).collect();
            self.snip.crop = self.snip.crop.map(|c| rel.rect(c, size));
            self.turn_picture(new.orient);
        }
        self.snip.edits = new;
        self.push_history();
        self.dirty = true;
    }

    /// Undo or redo landed on step `hi`: its image edits (the objects and
    /// the crop restored are that step's, in its orientation already).
    pub(super) fn restore_edits(&mut self) {
        let Some(e) = self.snip.edits_hist.get(self.hi).copied() else { return };
        if e != self.snip.edits {
            self.settle_for_image_edit(false);
            self.turn_picture(e.orient);
            self.snip.edits = e;
        }
    }

    /// Open a document whose objects and crop are in the original's
    /// pixels with image edits `e`: the picture turned and everything
    /// mapped (the first undo step).
    pub(super) fn load_edits(&mut self, e: Edits) {
        self.snip.doc_size = self.shot.size;
        self.snip.doc_mask = self.snip.mask.clone();
        let o = e.orient;
        if !o.is_identity() {
            let size = self.shot.size;
            let font = self.font.as_ref();
            self.objects = self.objects.iter().map(|x| o.map_obj(x, size, font)).collect();
            self.snip.crop = self.snip.crop.map(|c| o.rect(c, size));
            self.turn_picture(o);
        }
        self.snip.edits = e;
        self.hist = vec![self.objects.clone()];
        self.hi = 0;
        self.sel_hist.clear();
        self.snip.crop_hist = vec![self.snip.crop];
        self.snip.edits_hist = vec![e];
        self.dirty = true;
    }

    /// The image edits now.
    pub(super) fn image_edits(&self) -> Edits {
        self.snip.edits
    }

    /// The objects in the original's pixels (what is saved).
    pub(super) fn doc_objects(&self) -> Vec<Obj> {
        let inv = self.snip.edits.orient.inverse();
        let font = self.font.as_ref();
        self.objects.iter().map(|o| inv.map_obj(o, self.shot.size, font)).collect()
    }

    /// The crop in the original's pixels (what is saved).
    pub(super) fn doc_crop(&self) -> Option<FRect> {
        let inv = self.snip.edits.orient.inverse();
        self.crop_of_doc().map(|c| inv.rect(c, self.shot.size)).and_then(|c| crate::objects::mask::crop_rect(c, self.snip.doc_size))
    }

    /// The size the crop leaves (the picture shown when there is none).
    pub(super) fn crop_size(&self) -> (u32, u32) {
        self.crop_of_doc().map_or(self.shot.size, |c| (c.w as u32, c.h as u32))
    }

    /// Rotate left / right (a quarter turn, the resize turning with it).
    pub(super) fn rotate_image(&mut self, cw: bool) {
        let e = self.snip.edits;
        self.set_image_edits(e.reoriented(e.orient.rotated(cw)));
    }

    /// Flip horizontal / vertical (as the picture shows now).
    pub(super) fn flip_image(&mut self, horizontal: bool) {
        let e = self.snip.edits;
        let o = if horizontal { e.orient.flipped_h() } else { e.orient.flipped_v() };
        self.set_image_edits(e.reoriented(o));
    }

    /// Reset image edits: the picture as captured (the crop and the
    /// objects stay).
    pub(super) fn reset_image(&mut self) {
        let e = self.snip.edits;
        if e.is_empty() {
            return;
        }
        self.set_image_edits(Edits::default());
        self.notice = Some(Toast::new("Image edits reset", ToastKind::Info));
    }
}
