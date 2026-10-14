//! What is under the pointer, for its cursor, the hover outline and a
//! press: the chrome first (toolbar, its popovers, the mini-toolbar, the
//! color ring, the size label's menu button, the notice and the hint
//! card: the arrow),
//! then typing and Text mode, the selected objects' handles, an object a
//! press would pick up (the move cursor), then the tool's own cursor and
//! the capture selection's handles.
//!
//! Objects are picked up with the select tool, with no tool and with every
//! drawing tool that makes objects of its own; Ctrl held draws over them
//! instead. The tools that act on what is under them (Pixelate, Blur,
//! Invert, Spotlight, Ruler, Magnify) always draw.

use super::*;

/// What the pointer is over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Under {
    /// A cursor, nothing to pick up.
    Cursor(Cursor),
    /// Object `i`: a press picks it up (a drag moves it, a click selects).
    Grab(usize),
}

/// Tool `t` acts on what is under it: it draws over objects (no pick-up).
pub(super) fn acts_beneath(t: Tool) -> bool {
    matches!(t, Tool::Pixelate | Tool::Blur | Tool::Invert | Tool::Spotlight | Tool::Ruler | Tool::Magnify)
}

impl Edit {
    /// The object a press at `p` picks up instead of drawing (the topmost
    /// one hit), see the module docs.
    pub(super) fn grab_at(&self, p: Pt, mods: Mods) -> Option<usize> {
        match self.tool {
            Some(Tool::Select) => {}
            Some(t) if acts_beneath(t) => return None,
            _ if mods.ctrl => return None,
            _ => {}
        }
        self.obj_at(p)
    }

    /// The overlay's chrome under `p` (the toolbar and its popovers, the
    /// mini-toolbar, the size label, the notice, the hint card, a tip).
    pub(super) fn on_chrome(&self, p: Pt) -> bool {
        if self.obj.mini.as_ref().is_some_and(|m| m.contains(p)) {
            return true;
        }
        if self.host.is_some() {
            return false;
        }
        if self.toolbar.as_ref().is_some_and(|t| t.contains(p)) {
            return true;
        }
        // The size label is a button (its menu) with no tool or the
        // select tool; with a drawing tool a press on it draws.
        if self.on_label(p) {
            return true;
        }
        let Some(sc) = &self.scene else { return false };
        let px = |r: &compose::PxRect| p.x >= r.x0 as f32 && p.x < r.x1 as f32 && p.y >= r.y0 as f32 && p.y < r.y1 as f32;
        [compose::El::Toast, compose::El::Hint, compose::El::Tip].iter().any(|&e| sc.rects(e).iter().any(px))
    }

    /// The cursor of an interaction in progress (`None`: none).
    fn drag_cursor(&self, p: Pt) -> Option<Cursor> {
        Some(match &self.interact {
            Interact::None => return None,
            Interact::MoveSel { .. } | Interact::ObjMove { .. } | Interact::ObjPoint { .. } => Cursor::Move,
            Interact::Resize { handle, .. } | Interact::ObjResize { handle, .. } => handle_cursor(*handle),
            Interact::Pick(_) => Cursor::Arrow,
            Interact::Marquee { .. } => Cursor::Arrow,
            Interact::Snip => return self.snip_cursor(p),
            Interact::Drawing { .. } | Interact::NewSel { .. } => Cursor::Cross,
        })
    }

    /// What is under `p` (image coordinates) with modifiers `mods`.
    pub(super) fn under(&self, p: Pt, mods: Mods) -> Under {
        use Under::Cursor as C;
        if let Some(c) = self.drag_cursor(p) {
            return C(c);
        }
        // The color ring takes the pointer while it is open.
        if self.assist.ring.is_some() {
            return C(Cursor::Arrow);
        }
        if self.on_chrome(p) {
            return C(Cursor::Arrow);
        }
        if self.pick.as_ref().is_some_and(|k| k.eyedrop) {
            return C(Cursor::Cross);
        }
        if self.text.is_some() || self.in_cedit(p) {
            return C(Cursor::IBeam);
        }
        if let Some(c) = self.snip_cursor(p) {
            return C(c);
        }
        if !self.obj.sel.is_empty()
            && let Some(h) = self.obj_handle_at(p)
        {
            return C(match h {
                select::OHandle::Box(h) => handle_cursor(h),
                select::OHandle::Point(..) => Cursor::Move,
            });
        }
        // No tool: the capture selection's handles come before the
        // objects (they sit on its edge).
        let sel_handle = |e: &Edit| {
            let sr = e.sel.filter(|_| e.host.is_none() && e.tool.is_none())?;
            handle_points(sr).into_iter().find(|(_, q)| (q.x - p.x).hypot(q.y - p.y) <= HANDLE_HIT * e.ui_scale()).map(|(h, _)| h)
        };
        if let Some(h) = sel_handle(self) {
            return C(handle_cursor(h));
        }
        if let Some(i) = self.grab_at(p, mods) {
            return Under::Grab(i);
        }
        match self.tool {
            Some(Tool::Select) => C(Cursor::Arrow),
            Some(_) => C(Cursor::Cross),
            None if self.sel.is_some_and(|sr| hit(sr, p)) && self.host.is_none() => C(Cursor::Move),
            None => C(Cursor::Cross),
        }
    }

    /// The pointer's cursor at `p`.
    pub(super) fn cursor_at(&self, p: Pt, mods: Mods) -> Cursor {
        match self.under(p, mods) {
            Under::Cursor(c) => c,
            Under::Grab(_) => Cursor::Move,
        }
    }

    /// The object outlined under the pointer at `p`: one a press would
    /// pick up, not selected already, while nothing is dragged.
    pub(super) fn hover_obj(&self, p: Pt, mods: Mods) -> Option<usize> {
        match self.under(p, mods) {
            Under::Grab(i) if !self.obj.sel.contains(&i) => Some(i),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod cursor_tests;
