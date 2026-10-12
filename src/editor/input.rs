//! The editor's input: keys, typed text, the wheel, presses, drags and
//! releases, and the toolbar's acts, all on [`Edit`] so the overlay and
//! the Edit window drive the same code. Points are image coordinates
//! (the overlay's client pixels are the capture's); the toolbar's points
//! are its own (the same in the overlay, the window's in the Edit window).
//! `mouse` is the pointer (Screen picks the monitor under it).

use super::*;

impl Edit {
    pub(super) fn handle_key(&mut self, vk: u32, repeat: bool, mods: Mods, mouse: Pt) {
        let left = vk == wind::key::LEFT;
        let right = vk == wind::key::RIGHT;
        let up = vk == wind::key::UP;
        let down = vk == wind::key::DOWN;
        let arrows = [left, right, up, down];
        let is_arrow = arrows.iter().any(|a| *a);
        let escape = vk == wind::key::ESCAPE;
        let enter = vk == wind::key::RETURN;
        let editing = vk == wind::key::BACK || vk == wind::key::DELETE;
        // Auto-repeat: selection nudges and text editing keys repeat,
        // everything else is edge-triggered.
        if repeat && !is_arrow && !editing {
            return;
        }

        // The UI size keys (Ctrl/Alt chords) work in every mode, the text
        // draft and the color picker included.
        if (mods.ctrl || mods.alt)
            && self.host.is_none()
            && (self.text.is_some() || self.pick.is_some() || self.obj.cedit.is_some())
            && let Some(a @ (Action::UiBigger | Action::UiSmaller | Action::UiReset)) = self.keys.resolve(vk, mods)
        {
            let now = Instant::now();
            match a {
                Action::UiBigger => self.set_ui_size(self.cfg.ui_scale.bigger(), now),
                Action::UiSmaller => self.set_ui_size(self.cfg.ui_scale.smaller(), now),
                _ => self.set_ui_size(UiSize::Auto, now),
            }
            return;
        }

        // An object drag: Esc puts everything back and ends it; any other
        // key waits for the release (no edit under the drag's feet).
        if self.obj_dragging() {
            if escape {
                self.cancel_obj_drag();
            }
            return;
        }

        // Typing: Ctrl+B / I / U style the text (before the other keys,
        // Ctrl+U included).
        if (self.text.is_some() || self.obj.cedit.is_some())
            && (mods.ctrl || mods.alt)
            && let Some(a) = self.keys.resolve_typing(vk, mods)
            && let Some(act) = act_of(a)
        {
            self.apply_act(act, mouse);
            return;
        }

        // Text editing mode consumes keys.
        if self.text.is_some() {
            if escape {
                self.text = None;
                return;
            }
            if enter && !mods.shift {
                self.commit_text();
                return;
            }
            let td = self.text.as_mut().expect("text draft");
            if enter {
                // Shift+Enter: a new line.
                let mut idx = td.caret.min(td.text.len());
                while idx > 0 && !td.text.is_char_boundary(idx) {
                    idx -= 1;
                }
                td.text.insert(idx, '\n');
                td.caret = idx + 1;
            } else if left {
                td.caret = prev_boundary(&td.text, td.caret);
            } else if right {
                td.caret = next_boundary(&td.text, td.caret);
            } else if vk == wind::key::HOME {
                td.caret = 0;
            } else if vk == wind::key::END {
                td.caret = td.text.len();
            } else if vk == wind::key::BACK && td.caret > 0 {
                let p = prev_boundary(&td.text, td.caret);
                td.text.replace_range(p..td.caret, "");
                td.caret = p;
            } else if vk == wind::key::DELETE {
                let n = next_boundary(&td.text, td.caret);
                td.text.replace_range(td.caret..n, "");
            }
            td.at = Instant::now();
            return;
        }

        // Typing in a callout consumes keys.
        if self.obj.cedit.is_some() {
            self.callout_key(vk);
            return;
        }

        // The color picker takes every key while it is open.
        if self.pick.is_some() {
            self.pick_key(vk, mods);
            return;
        }

        // Selected objects: their actions first (Del, Ctrl+D...), and
        // the arrows nudge them (Shift: 10 px).
        if !self.obj.sel.is_empty() {
            if let Some(a) = self.keys.resolve_objects(vk, mods)
                && let Some(act) = act_of(a)
            {
                self.apply_act(act, mouse);
                return;
            }
            if is_arrow && !mods.ctrl && !mods.alt {
                let k = if mods.shift { 10.0 } else { 1.0 };
                let (dx, dy) = if left {
                    (-k, 0.0)
                } else if right {
                    (k, 0.0)
                } else if up {
                    (0.0, -k)
                } else {
                    (0.0, k)
                };
                self.nudge(dx, dy);
                return;
            }
        }

        // Tab / Shift+Tab: into the part of the window under the pointer
        // and back out while snapping to windows (built in, like the arrows).
        if vk == wind::key::TAB && !mods.ctrl && !mods.alt && self.snap_rect().is_some() {
            if mods.shift {
                self.hover_win.outer();
            } else {
                self.hover_win.deeper();
            }
            if self.cfg.snap_tab_hint {
                // The hint has done its job: not shown again.
                self.cfg.snap_tab_hint = false;
                self.pending.set("snap_tab_hint".into(), Some("false".into()), Instant::now());
            }
            return;
        }

        // Arrow keys nudge / resize the selection (built in, not remappable).
        if is_arrow && self.sel.is_some() && !mods.ctrl && !mods.alt && self.host.is_none() {
            let step = self.shot.scale.round().max(1.0);
            let dir = if left {
                (-1.0, 0.0)
            } else if right {
                (1.0, 0.0)
            } else if up {
                (0.0, -1.0)
            } else {
                (0.0, 1.0)
            };
            let mut r = self.sel.unwrap();
            if mods.shift {
                if dir.0 < 0.0 {
                    r.x -= step;
                    r.w += step;
                } else if dir.0 > 0.0 {
                    r.w += step;
                }
                if dir.1 < 0.0 {
                    r.y -= step;
                    r.h += step;
                } else if dir.1 > 0.0 {
                    r.h += step;
                }
                r.clamp_to(self.shot.size.0 as f32, self.shot.size.1 as f32);
            } else {
                r.x += dir.0 * step;
                r.y += dir.1 * step;
                r.clamp_to(self.shot.size.0 as f32, self.shot.size.1 as f32);
            }
            self.sel = Some(r);
            return;
        }

        // Built in (not remappable): 1 / 2 switch Screenshot / Video; in
        // video mode F / W / R pick Screen / Window / Region.
        if self.host.is_none()
            && let Some(act) = video_key(self, vk, mods)
        {
            self.apply_act(act, mouse);
            return;
        }
        let Some(action) = self.keys.resolve(vk, mods) else { return };
        match action {
            // The Edit window zooms with these chords (its own keys, first).
            Action::UiBigger | Action::UiSmaller | Action::UiReset if self.host.is_some() => {}
            Action::UiBigger => self.set_ui_size(self.cfg.ui_scale.bigger(), Instant::now()),
            Action::UiSmaller => self.set_ui_size(self.cfg.ui_scale.smaller(), Instant::now()),
            Action::UiReset => self.set_ui_size(UiSize::Auto, Instant::now()),
            Action::Cancel => cancel_key(self),
            // In record mode (or with the Record popover open) Enter
            // records; never in a run with tasks (`gui --copy`: no Record).
            Action::Accept if (self.rec.mode || self.rec.open) && self.tasks.is_empty() => {
                self.apply_act(Act::RecStart, mouse)
            }
            Action::Accept => accept_key(self),
            // The Edit window: every object.
            Action::SelectAll if self.host.is_some() => {
                self.finish_callout_edit();
                self.obj.sel = (0..self.objects.len()).collect();
                if self.tool != Some(Tool::Select) {
                    self.tool = Some(Tool::Select);
                }
            }
            Action::SelectAll => {
                self.sel = Some(FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 });
                self.interact = Interact::None;
            }
            // The palette popover hangs off the toolbar, which needs a selection.
            Action::TogglePalette if self.sel.is_none() => {}
            Action::Record if self.sel.is_none() || !rec_ui::supported() || !self.tasks.is_empty() => {}
            a => {
                if let Some(act) = act_of(a) {
                    self.apply_act(act, mouse);
                }
            }
        }
    }

    pub(super) fn char_input(&mut self, c: u16) {
        {
            // UTF-16 in, one unit at a time: a pair's high half waits for
            // its low one (emoji and every other character past U+FFFF).
            let c = match c {
                0xD800..=0xDBFF => {
                    self.obj.high = Some(c);
                    return;
                }
                0xDC00..=0xDFFF => match self.obj.high.take() {
                    Some(h) => 0x10000 + (((h as u32) - 0xD800) << 10) + (c as u32 - 0xDC00),
                    None => return,
                },
                c => {
                    self.obj.high = None;
                    c as u32
                }
            };
            if self.pick.is_some() {
                if let Some(ch) = char::from_u32(c) {
                    self.pick_char(ch);
                }
                return;
            }
            if self.obj.cedit.is_some() {
                if let Some(ch) = char::from_u32(c).filter(|ch| !ch.is_control()) {
                    self.callout_char(ch);
                }
                return;
            }
            let Some(td) = self.text.as_mut() else { return };
            let Some(ch) = char::from_u32(c) else { return };
            if ch.is_control() {
                return;
            }
            let mut idx = td.caret.min(td.text.len());
            while idx > 0 && !td.text.is_char_boundary(idx) {
                idx -= 1;
            }
            td.text.insert(idx, ch);
            td.caret = idx + ch.len_utf8();
            td.at = Instant::now();
        }
    }

    pub(super) fn wheel(&mut self, delta: i32) {
        {
            if self.text.is_some() || self.obj_dragging() {
                return;
            }
            if delta != 0 && self.last_wheel.elapsed() > Duration::from_millis(160) {
                self.last_wheel = Instant::now();
                let dir = if delta > 0 { 1 } else { -1 };
                if !self.obj_act(Act::ObjSize(dir)) {
                    self.adjust_size(dir);
                }
            }
        }
    }

    /// A press at `p`: on the toolbar (and its popover) first, else on
    /// the capture.
    pub(super) fn pointer_down(&mut self, p: Pt) {
        if !self.bar_press(p) {
            self.canvas_press(p);
        }
    }

    /// A press on the toolbar or its popover at `p` (the toolbar's
    /// coordinates: the capture's in the overlay, the window's in the Edit
    /// window); false when it is not on them.
    pub(super) fn bar_press(&mut self, p: Pt) -> bool {
        {
            // The toolbar (and open palette) sits on top: it consumes the press.
            let on_bar = self
                .toolbar
                .as_ref()
                .filter(|tb| tb.contains(p))
                .map(|tb| (tb.act_at(p), tb.index_at(p)));
            let Some((act, idx)) = on_bar else { return false };
            {
                self.pressed = idx;
                if self.text.is_some() {
                    self.commit_text();
                }
                // A color or size for the bubble being typed in keeps it
                // open; anything else on the bar finishes it.
                if !matches!(act, Some(Act::Color(_) | Act::Palette | Act::Size(_) | Act::Fill | Act::Wheel)) {
                    self.finish_callout_edit();
                }
                if let Some(pk) = &self.pick {
                    let ring = pk.target == pick::Target::Ring;
                    let in_pop = self.toolbar.as_ref().and_then(|t| t.pop).is_some_and(|r| hit(r, p));
                    if !in_pop {
                        // The bar: the picker closes (keeping its color)
                        // and the button acts; its own button only closes it.
                        self.close_picker(true);
                        if act == Some(Act::Palette) || (ring && act == Some(Act::Record)) {
                            return true;
                        }
                    } else if let Some(Act::Pick(part)) = act {
                        self.pick_press(part, p);
                        return true;
                    } else if act != Some(Act::Eyedrop)
                        && let Some(pk) = self.pick.as_mut()
                    {
                        pk.eyedrop = false;
                    }
                }
                if let Some(act) = act {
                    self.apply_act(act, p);
                }
                true
            }
        }
    }

    /// A press on the capture at `p` (image coordinates).
    pub(super) fn canvas_press(&mut self, p: Pt) {
        {
            // The color picker: the eyedropper picks the pixel; any other
            // press outside closes it (keeping the color), nothing else.
            if let Some(pk) = &self.pick {
                if pk.eyedrop {
                    let c = self.sample(p);
                    if let Some(pk) = self.pick.as_mut() {
                        pk.set_color(c.with_alpha(pk.a));
                        pk.eyedrop = false;
                    }
                    self.loupe = None;
                    self.pick_changed();
                } else {
                    self.close_picker(true);
                }
                return;
            }
            // The text draft's mini-toolbar (its style) takes presses on
            // it; the font list open closes on a press anywhere else.
            if self.text.is_some()
                && let Some(act) = self.obj.mini.as_ref().filter(|m| m.contains(p)).map(|m| m.act_at(p))
            {
                if let Some(act) = act {
                    self.apply_act(act, p);
                }
                return;
            }
            if self.obj.font_menu {
                self.obj.font_menu = false;
                return;
            }
            // Text draft: a click outside commits it.
            if self.text.is_some() {
                let td = self.text.as_ref().expect("text draft");
                let r = text_box_rect(
                    self.shot.size.0 as f32,
                    td,
                    self.mem.size(Tool::Text),
                    self.text_font().as_ref(),
                    self.ui_scale(),
                );
                if !hit(r, p) {
                    let (pos, text) = (td.pos, td.text.clone());
                    self.text = None;
                    self.commit_text_str(pos, text);
                }
                return;
            }
            if !matches!(self.interact, Interact::None) {
                return; // already dragging
            }
            self.save_menu = false;
            // The mini-toolbar over the selected objects.
            if let Some(act) = self.obj.mini.as_ref().filter(|m| m.contains(p)).map(|m| m.act_at(p)) {
                if let Some(act) = act {
                    self.apply_act(act, p);
                }
                return;
            }
            // Typing in a callout: a click in it moves the caret, outside
            // finishes (and does nothing else).
            if self.obj.cedit.is_some() {
                if self.in_cedit(p) {
                    self.cedit_click(p);
                } else {
                    self.finish_callout_edit();
                }
                return;
            }
            let mods = drag_mods();
            // The selection's handles answer with any tool.
            if !self.obj.sel.is_empty()
                && let Some(h) = self.obj_handle_at(p)
            {
                self.handle_press(h);
                return;
            }
            // The select tool picks (or starts a marquee); Ctrl+click picks
            // with the others (the number tool keeps Ctrl for its tail).
            // With a drawing tool, Ctrl+press on an object not selected
            // still draws when it drags (Ctrl snaps lines): a click picks.
            self.obj.click_pick = None;
            if self.tool == Some(Tool::Select) {
                if self.select_press(p, mods, true) {
                    return;
                }
            } else if mods.ctrl
                && self.tool != Some(Tool::Counter)
                && let Some(i) = self.obj_at(p)
            {
                let draws = matches!(self.tool, Some(t) if t != Tool::Text && t != Tool::Callout);
                if !draws || self.obj.sel.contains(&i) {
                    self.select_press(p, mods, false);
                    return;
                }
                self.obj.click_pick = Some((i, p));
            }
            self.obj.sel.clear();
            // The callout tool on a bubble types in it.
            if self.tool == Some(Tool::Callout)
                && let Some(i) = self.obj_at(p).filter(|i| matches!(self.objects[*i], Obj::Callout { .. }))
            {
                self.begin_callout_edit(i, Some(p), false);
                return;
            }
            if let Some(tool) = self.tool {
                if tool == Tool::Text {
                    self.text = Some(TextDraft {
                        pos: p,
                        text: String::new(),
                        caret: 0,
                        at: Instant::now(),
                    });
                    return;
                }
                self.stroke_pts = vec![p];
                self.draft = self.make_draft(tool, p, p);
                self.interact = Interact::Drawing { start: p };
                return;
            }
            // The Edit window has no capture selection to move.
            if self.host.is_some() {
                return;
            }
            if let Some(sr) = self.sel {
                for (h, hp) in handle_points(sr) {
                    if (hp.x - p.x).hypot(hp.y - p.y) <= HANDLE_HIT * self.ui_scale() {
                        let aspect = if sr.h > 0.0 { sr.w / sr.h } else { 1.0 };
                        self.interact = Interact::Resize {
                            handle: h,
                            orig: sr,
                            aspect,
                        };
                        return;
                    }
                }
                if hit(sr, p) {
                    self.interact = Interact::MoveSel {
                        start: p,
                        orig: sr,
                    };
                    return;
                }
            }
            self.interact = Interact::NewSel {
                anchor: p,
                moved: false,
            };
        }
    }

    pub(super) fn pointer_move(&mut self, p: Pt, pb: Pt) {
        let _p = crate::perf::span(crate::perf::K::Ptr);
        {
            if matches!(self.interact, Interact::None) {
                return;
            }
            let mods = drag_mods();
            let img = (self.shot.size.0 as f32, self.shot.size.1 as f32);
            // A click on a window selects it: a press that moved less than
            // SNAP_CLICK_PX is still a click.
            let click_px = if self.snapping() { SNAP_CLICK_PX } else { CLICK_PX };
            match &mut self.interact {
                Interact::None => {}
                Interact::NewSel { anchor, moved } => {
                    let anchor = *anchor;
                    let dist = (p.x - anchor.x).hypot(p.y - anchor.y);
                    if dist >= click_px {
                        *moved = true;
                    }
                    if *moved {
                        // A pointer past the shot edge (captured drag) stops
                        // at the edge instead of pushing the rect back in.
                        let p = Pt::new(p.x.clamp(0.0, img.0), p.y.clamp(0.0, img.1));
                        let cur = if mods.shift {
                            square_within(anchor, constrain_square(anchor, p), img)
                        } else {
                            p
                        };
                        let mut r = FRect::from_pts(anchor, cur);
                        r.clamp_to(img.0, img.1);
                        self.sel = Some(r);
                    }
                }
                Interact::MoveSel { start, orig } => {
                    let (start, orig) = (*start, *orig);
                    let mut r = FRect {
                        x: orig.x + p.x - start.x,
                        y: orig.y + p.y - start.y,
                        w: orig.w,
                        h: orig.h,
                    };
                    r.clamp_to(img.0, img.1);
                    self.sel = Some(r);
                }
                Interact::Resize {
                    handle,
                    orig,
                    aspect,
                } => {
                    let (h, o, a) = (*handle, *orig, *aspect);
                    self.sel = Some(resize_rect(h, o, p, a, &mods, img));
                }
                Interact::Pick(part) => {
                    let part = *part;
                    self.pick_drag(part, pb);
                }
                Interact::Drawing { start } => {
                    let start = *start;
                    if self.obj.click_pick.is_some_and(|(_, q)| (p.x - q.x).hypot(p.y - q.y) >= CLICK_PX) {
                        self.obj.click_pick = None;
                    }
                    self.drag_draft(start, p, mods);
                }
                Interact::ObjMove { .. } | Interact::ObjResize { .. } | Interact::ObjPoint { .. } | Interact::Marquee { .. } => {
                    self.obj_drag(p, mods);
                }
            }
        }
    }

    pub(super) fn pointer_up(&mut self) {
        {
            self.pressed = None;
            end_interaction(self);
        }
    }

    pub(super) fn commit_text(&mut self) {
        let td = self.text.take().expect("text draft");
        self.commit_text_str(td.pos, td.text);
    }

    pub(super) fn commit_text_str(&mut self, pos: Pt, text: String) {
        let text = text.trim_end_matches('\n').to_string();
        if text.trim().is_empty() {
            return;
        }
        self.commit_object(Obj::Text {
            pos,
            text,
            color: self.mem.color(Tool::Text),
            size: self.mem.size(Tool::Text),
            fmt: self.mem.fmt(Tool::Text),
        });
    }

    pub(super) fn apply_act(&mut self, act: Act, mouse: Pt) {
        if act != Act::SaveMenu {
            self.save_menu = false;
        }
        match act {
            // Video mode draws nothing (the recording is live).
            Act::Tool(_) if self.rec.open => {}
            Act::Tool(t) => {
                self.finish_callout_edit();
                // The Edit window always has a tool: off again is Select.
                self.tool = if self.tool == Some(t) { self.host.as_ref().map(|_| Tool::Select) } else { Some(t) };
                // The selection stays for the select tool only.
                if self.tool != Some(Tool::Select) {
                    self.obj.sel.clear();
                }
                if t != Tool::Select {
                    self.last_tool = t;
                }
                self.draft = None;
                self.stroke_pts.clear();
            }
            Act::SaveMenu => {
                self.save_menu = !self.save_menu;
                self.palette_open = false;
            }
            Act::Undo | Act::Redo => {
                self.finish_callout_edit();
                if act == Act::Undo {
                    self.undo();
                } else {
                    self.redo();
                }
                // Indices may name other objects now.
                self.obj.sel.clear();
            }
            // With objects selected, size, fill and color are theirs.
            Act::Size(d) if !self.obj.sel.is_empty() => {
                self.obj_act(Act::ObjSize(d));
            }
            Act::Size(d) => self.adjust_size(d),
            Act::Fill if !self.obj.sel.is_empty() => {
                self.obj_act(Act::ObjFill);
            }
            Act::Fill => self.toggle_fill(Instant::now()),
            Act::Color(c) if !self.obj.sel.is_empty() => {
                self.color_selection(c);
                self.palette_open = false;
            }
            Act::Color(c) => {
                self.set_tool_color(c, Instant::now());
                self.palette_open = false;
            }
            Act::ObjColor | Act::ObjFill | Act::ObjSize(_) | Act::ObjStyle(_) | Act::ObjFront | Act::ObjBack | Act::ObjDup | Act::ObjDelete => {
                self.obj_act(act);
            }
            Act::ObjBold | Act::ObjItalic | Act::ObjUnderline | Act::ObjStrike | Act::ObjAlign(_) | Act::ObjFontMenu | Act::ObjFont(_) => {
                self.fmt_act(act);
            }
            // Pixelate and Invert have no color: no palette (Space included).
            Act::Palette if !self.palette_open && !tools::has_color(self.cur_tool()) && self.sel_color().is_none() => {}
            // Video mode has no palette (the panel replaces the toolbar).
            Act::Palette if self.rec.open => {}
            Act::Palette => self.palette_open = !self.palette_open,
            Act::Wheel if !tools::has_color(self.cur_tool()) && self.sel_color().is_none() => {}
            Act::Wheel => self.open_picker(pick::Target::Tool),
            Act::RingSize(d) => {
                let w = (self.rec.opts.cam_ring_w as i32 + d).clamp(0, config::RING_WIDTH_MAX as i32) as u32;
                self.rec.opts.cam_ring_w = w;
                self.remember_ring(Instant::now());
            }
            Act::RingPalette => {
                self.rec.ring_pal = !self.rec.ring_pal;
                self.rec.dev_open = None;
            }
            Act::RingColor(c) => {
                self.rec.opts.cam_ring = c;
                self.rec.ring_pal = false;
                self.remember_ring(Instant::now());
            }
            Act::RingWheel => {
                self.rec.ring_pal = false;
                self.open_picker(pick::Target::Ring);
            }
            // A press on the picker (the point matters): `pointer_down`.
            Act::Pick(_) => {}
            Act::PickColor(c) => {
                if let Some(pk) = self.pick.as_mut() {
                    pk.set_color(c);
                }
                self.pick_changed();
            }
            Act::Eyedrop => {
                if let Some(pk) = self.pick.as_mut() {
                    pk.eyedrop = !pk.eyedrop;
                }
                self.loupe = None;
            }
            // A run with tasks has no Record button.
            Act::Record | Act::RecStart | Act::Mode(_) if !self.tasks.is_empty() || self.host.is_some() => {}
            Act::RecSource(_) if self.host.is_some() => {}
            Act::Record => {
                let on = !self.rec.open;
                self.set_video(on);
            }
            Act::Mode(on) => self.set_video(on),
            Act::RecSource(src) => self.set_source(src, mouse),
            Act::RecCountdown(on) => self.rec.opts.countdown = on,
            Act::RecGifMax(n) => self.rec.opts.gif_max = n,
            Act::RecFormat(crate::rec::RecFormat::Mp4) if self.rec.mp4 != rec_ui::Mp4::Yes => {}
            Act::RecFormat(f) => {
                self.rec.opts.format = f;
                self.rec.wanted = f;
                self.rec.dev_open = None;
            }
            Act::RecFps(n) => self.rec.opts.set_fps(n),
            Act::RecSystem => self.rec.opts.system = !self.rec.opts.system,
            Act::RecMic => {
                self.rec.opts.mic = !self.rec.opts.mic;
                self.rec.dev_open = None;
                if self.rec.opts.mic {
                    self.rec.load_devs();
                }
            }
            // The bubble follows (`sync_bubble`).
            Act::RecCamera => {
                self.rec.load_cams();
                self.rec.opts.camera = !self.rec.opts.camera && self.rec.has_cams();
                self.rec.dev_open = None;
            }
            Act::RecShape(sh) => self.rec.opts.cam_shape = sh,
            Act::RecDevMenu(dev) => {
                self.rec.ring_pal = false;
                match dev {
                    Dev::Mic => self.rec.load_devs(),
                    Dev::Cam => self.rec.load_cams(),
                }
                self.rec.dev_open = if self.rec.dev_open == Some(dev) { None } else { Some(dev) };
            }
            Act::RecDev(dev, i) => self.rec.pick(dev, i),
            Act::RecStart => {
                // Nothing selected: the monitor under the pointer.
                if self.sel.is_none() {
                    let p = mouse;
                    self.sel = Some(pick_area(&self.shot.monitors, None, p));
                }
                self.rec.start = true;
                self.done = true;
            }
            Act::Copy => {
                self.tasks = vec![Task::Copy];
                self.done = true;
            }
            Act::CopyText => {
                self.tasks = vec![Task::Text];
                self.done = true;
            }
            Act::Save => {
                self.tasks = vec![Task::Save { path: None, ask: self.cfg.save_dialog }];
                self.done = true;
            }
            Act::SaveAs => {
                // Always ask, seeded with the auto-save path.
                self.tasks = vec![Task::Save { path: None, ask: true }];
                self.done = true;
            }
            Act::SaveDesktop => {
                self.tasks = vec![Task::SaveDesktop];
                self.done = true;
            }
            Act::Upload => {
                self.tasks = vec![Task::Upload];
                self.done = true;
            }
            Act::Pin => {
                self.tasks = vec![Task::Pin];
                self.done = true;
            }
            Act::Exit => {
                self.cancelled = true;
                self.done = true;
            }
            Act::Accept => self.done = true,
        }
    }
}
