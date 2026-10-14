//! Drawing round 2: what a drag draws (the drafts, with their Shift / Ctrl
//! constraints), the straight pencil and highlighter (Shift: a straight
//! stroke from the press point in 45-degree steps; let go of it and the
//! stroke is freehand again), the numbered tags (the counter tool) and the
//! rectangle / ellipse fill toggle.
//!
//! Numbered tags: a click places the next number, a drag moves the tag
//! until the release, Ctrl+drag leaves it at the press point and draws a
//! pointer tail to the pointer. The next number is one more than the tags
//! there are: undoing tag 3 makes the next one 3 again, redoing it brings
//! it back (and the next is 4).

use super::{constrain_square, marker_color, tools, App, Edit, Interact, State, Tool};
use crate::objects::{Obj, Pt};
use crate::wind::Mods;
use std::time::Instant;

/// Win32 virtual keys of the modifiers (the numbering `wind::key` uses).
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;

impl Edit {
    /// The draft of a drag of the current tool from `start` to the pointer
    /// `p` with `mods` held. Lines and arrows snap to 45-degree steps with
    /// Shift or Ctrl; shapes become squares and circles with Shift; the
    /// pencil and the marker record every point and draw straight while
    /// Shift is held; a tag follows the pointer (Ctrl: stays, with a tail).
    pub(super) fn drag_draft(&mut self, start: Pt, p: Pt, mods: Mods) {
        let tool = self.tool.unwrap_or(Tool::Line);
        let cur = match tool {
            Tool::Line | Tool::Arrow if mods.shift || mods.ctrl => self.snap_point(start, p, true),
            Tool::Rect | Tool::Ellipse | Tool::Pixelate | Tool::Blur | Tool::Invert if mods.shift => constrain_square(start, p),
            _ => p,
        };
        match tool {
            Tool::Ruler => {
                let b = if mods.shift && !mods.ctrl { super::ruler::snap15(start, p) } else { p };
                self.ruler = Some(super::ruler::Ruler { a: start, b, area: mods.ctrl, done: false });
            }
            Tool::Path | Tool::Marker => {
                if self.stroke_pts.last().is_none_or(|l| (p.x - l.x).hypot(p.y - l.y) >= 2.0) {
                    self.stroke_pts.push(p);
                }
                let (color, width) = (self.mem.color(tool), self.mem.size(tool));
                self.draft = Some(match (tool, mods.shift) {
                    (Tool::Path, true) => Obj::Path { pts: vec![start, self.snap_point(start, p, true)], color, width },
                    (Tool::Path, false) => Obj::Path { pts: self.stroke_pts.clone(), color, width },
                    (_, true) => Obj::Marker { a: start, b: self.snap_point(start, p, true), color: marker_color(color), width },
                    (_, false) => Obj::MarkerPath { pts: self.stroke_pts.clone(), color: marker_color(color), width },
                });
            }
            Tool::Counter => {
                if let Some(Obj::Counter { c, tail, .. }) = self.draft.as_mut() {
                    if mods.ctrl {
                        (*c, *tail) = (start, Some(p));
                    } else {
                        (*c, *tail) = (p, None);
                    }
                }
            }
            _ => self.draft = self.make_draft(tool, start, cur),
        }
    }

    /// The number the next tag gets: one more than the tags there are.
    pub(super) fn next_counter(&self) -> u32 {
        self.objects.iter().filter(|o| matches!(o, Obj::Counter { .. })).count() as u32 + 1
    }

    /// The fill toggle: the current shape tool draws filled or outlined
    /// (remembered); nothing for the other tools.
    pub(super) fn toggle_fill(&mut self, now: Instant) {
        let t = self.cur_tool();
        if !self.mem.toggle_fill(t) {
            return;
        }
        let Some(id) = tools::id(t) else { return };
        let on = self.mem.fill(t);
        self.cfg.tool_fills.insert(id.into(), on);
        self.pending.set(format!("tool_{id}_fill"), Some(on.to_string()), now);
    }
}

impl App {
    /// Shift or Ctrl went down or up (`vk`, `up`) while a stroke is being
    /// drawn: the draft is redrawn at the pointer with the new modifiers
    /// (the pencil and marker switch between freehand and straight, a tag
    /// between moving and its tail). True when the draft was redrawn.
    /// Windows reports modifier keys; elsewhere the next move does it.
    pub(super) fn modifier_key(&mut self, vk: u32, up: bool, mods: Mods) -> bool {
        if vk != VK_SHIFT && vk != VK_CONTROL {
            return false;
        }
        let State::Edit(edit) = &mut self.st else { return false };
        let mut m = mods;
        if vk == VK_SHIFT {
            m.shift = !up;
        } else {
            m.ctrl = !up;
        }
        let p = Pt::new(self.mouse.0 as f32, self.mouse.1 as f32);
        let Interact::Drawing { start } = edit.interact else {
            // Ctrl draws over an object: the pick-up outline goes (and
            // comes back on release).
            let h = edit.hover_obj(p, m);
            let changed = h != edit.obj.hover;
            edit.obj.hover = h;
            return changed;
        };
        edit.drag_draft(start, p, m);
        true
    }
}
