//! The editor hosted in a normal window: the Edit window's canvas
//! (board 11). The same [`Edit`] as the overlay, over a whole capture
//! (its selection is the image, nothing dims), drives every tool, the
//! selection chrome, mini-toolbars, callout typing, multi-select and
//! snapping; the window shows it zoomed and panned ([`View`]).
//!
//! Coordinates: the editor works in image px, as in the overlay; its
//! chrome sizes there are the window's scale over the zoom
//! ([`Host::s`]), so handles, hit slack and the mini-toolbar come out at
//! their board sizes on screen at any zoom. The docked toolbar lives in
//! the window's physical px ([`Canvas::layout_bar`]): its presses go to
//! [`Edit::bar_press`] with window points, the canvas's to
//! [`Edit::canvas_press`] with image points.
//!
//! Painting ([`Canvas::paint`]): the capture with its objects baked in
//! (`composed`) comes from a box-filtered copy at the zoom below 100 %
//! (rebuilt only where a bake changed it), straight from `composed` at
//! 100 %, nearest-neighbour above; the draft and the drag's ghost are
//! drawn as objects in window px over it (cheap at any zoom: the bake
//! follows on release); the object chrome is drawn in window px from the
//! editor's image-space chrome, mapped through the view.


use super::compose::{CaretUi, ObjSelUi, PxRect};
use super::mini::MiniUi;
use super::*;
use super::chrome::Ui;
use crate::anim;
use crate::uifb::Fb;

/// The editor's hosting: chrome px per image px.
pub(super) struct Host {
    pub s: f32,
}

/// Where the capture sits: zoom (window px per image px) and the
/// picture's top-left in the window (physical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub z: f32,
    pub ox: f32,
    pub oy: f32,
}

impl View {
    pub fn to_img(self, p: Pt) -> Pt {
        Pt::new((p.x - self.ox) / self.z, (p.y - self.oy) / self.z)
    }

    pub fn to_win(self, p: Pt) -> Pt {
        Pt::new(self.ox + p.x * self.z, self.oy + p.y * self.z)
    }

    pub fn rect(&self, r: FRect) -> FRect {
        FRect { x: self.ox + r.x * self.z, y: self.oy + r.y * self.z, w: r.w * self.z, h: r.h * self.z }
    }

    /// The image rect on screen for a `w`×`h` capture.
    pub fn image(&self, size: (u32, u32)) -> FRect {
        let (w, h) = disp(size, self.z);
        FRect { x: self.ox, y: self.oy, w: w as f32, h: h as f32 }
    }
}

/// The picture's displayed size at zoom `z` (whole px).
fn disp(size: (u32, u32), z: f32) -> (u32, u32) {
    (((size.0 as f32 * z).round() as u32).max(1), ((size.1 as f32 * z).round() as u32).max(1))
}

/// What the Edit window should do, asked by a key or the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Save,
    SaveAs,
    SaveDesktop,
    OpenWith,
    Share,
    Copy,
    Upload,
    Pin,
    CopyText,
    /// Read the QR codes in the picture.
    ReadQr,
    Close,
}

/// The zoom steps (Ctrl+= / Ctrl+-, the zoom control's − and +).
pub const ZOOMS: [f32; 15] = [0.1, 0.125, 0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];
/// The margin round the picture at Fit (logical px): 1180 × 760 fits
/// the board's canvas at 72 %.
pub const FIT_MARGIN: f32 = 72.0;

/// The capture scaled below 100 %, box-filtered from `composed`.
struct Cache {
    z: f32,
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// The Edit window's canvas: the hosted editor and its view.
pub struct Canvas {
    e: Edit,
    pub view: View,
    /// The canvas, the toolbar strip and the window (physical px).
    area: FRect,
    bar: FRect,
    win: FRect,
    /// The window's scale (physical px per logical px).
    k: f32,
    /// Following the window: the zoom stays at Fit.
    fit: bool,
    cache: Option<Cache>,
    /// The pointer (window px).
    mouse: Pt,
    /// Panning (Space- or middle-drag): where it started and the view then.
    pan: Option<(Pt, View)>,
    /// The pan is the middle button's.
    middle: bool,
    /// Space is held: a press pans.
    pub space: bool,
    request: Option<Request>,
    /// The objects as last saved (unsaved changes: they differ).
    saved: Vec<Obj>,
    /// The objects left out of the bake (being dragged).
    bake_excl: Vec<usize>,
    /// A move's ghost and held objects, rendered once.
    sprites: Option<Sprites>,
    /// The crop and the image edits as last saved (the objects and the
    /// crop in the frame of those edits' orientation).
    saved_crop: Option<FRect>,
    saved_edits: crate::imgedit::Edits,
    /// The orientation the view and the lasso's coverage were made for.
    orient_seen: crate::imgedit::Orient,
    /// A freeform capture's lasso (the picture's px) and its coverage of
    /// the picture: outside it is hidden and exports transparent.
    mask: Option<(Vec<Pt>, Vec<u8>)>,
}

impl Canvas {
    /// The editor over `img` with `objects`, Select chosen, at Fit once a
    /// rect is set.
    #[cfg_attr(target_os = "macos", allow(dead_code))] // no Edit window there yet
    pub fn new(img: PixBuf, objects: Vec<Obj>, cfg: Config, th: Theme) -> Canvas {
        let size = img.dimensions();
        let shot = Shot { origin: (0, 0), size, scale: 1.0, image: PixBuf::default(), monitors: vec![(0, 0, size.0, size.1)] };
        let rec = RecPop::new(RecOpts::from_config(&cfg), false);
        let font = AnnotFont::load();
        let mut e = Edit::new(shot, img, cfg, th, font, Some(fonts::ui_font()), rec);
        e.sel = Some(FRect { x: 0.0, y: 0.0, w: size.0 as f32, h: size.1 as f32 });
        e.hist = vec![objects.clone()];
        e.dirty = !objects.is_empty();
        e.objects = objects.clone();
        e.tool = Some(Tool::Select);
        e.host = Some(Host { s: 1.0 });
        e.snip.doc_size = size;
        e.snip.edits_hist = vec![Default::default()];
        // The tools' sizes and colors are the overlay's (remembered alike).
        e.cfg_path = (!cfg!(test)).then(config::config_path);
        Canvas {
            e,
            view: View { z: 1.0, ox: 0.0, oy: 0.0 },
            area: FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            bar: FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            win: FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            k: 1.0,
            fit: true,
            cache: None,
            mouse: Pt::new(-1.0, -1.0),
            pan: None,
            middle: false,
            space: false,
            request: None,
            saved: objects,
            bake_excl: Vec::new(),
            sprites: None,
            saved_crop: None,
            saved_edits: Default::default(),
            orient_seen: Default::default(),
            mask: None,
        }
    }

    /// The document's image edits (after [`Self::set_doc`]): the picture
    /// turned as they say, the objects and the crop with it; as saved.
    #[cfg_attr(target_os = "macos", allow(dead_code))] // no Edit window there yet
    pub fn set_edits(&mut self, e: crate::imgedit::Edits) {
        self.e.load_edits(e);
        self.saved = self.e.objects.clone();
        self.saved_crop = self.e.snip.crop;
        self.saved_edits = e;
        self.after_input();
    }

    /// A text or a callout is being typed in (the Edit window's image keys
    /// wait: they would end the typing and turn the picture under it).
    pub fn typing(&self) -> bool {
        self.e.text.is_some() || self.e.obj.cedit.is_some()
    }

    /// The image edits now.
    pub fn edits(&self) -> crate::imgedit::Edits {
        self.e.image_edits()
    }

    /// What Save writes: the objects and the crop in the original's
    /// pixels, and the image edits.
    pub fn doc(&self) -> crate::objects::Doc {
        crate::objects::Doc { objects: self.e.doc_objects(), crop: self.e.doc_crop(), edits: self.e.image_edits() }
    }

    /// The picture's size before the resize and the canvas: the crop, turned.
    pub fn source_size(&self) -> (u32, u32) {
        self.e.crop_size()
    }

    /// Rotate left / right, Flip horizontal / vertical, Reset image edits
    /// (each one undo step).
    pub fn rotate(&mut self, cw: bool) {
        self.e.rotate_image(cw);
        self.after_input();
    }

    pub fn flip(&mut self, horizontal: bool) {
        self.e.flip_image(horizontal);
        self.after_input();
    }

    pub fn reset_image(&mut self) {
        self.e.reset_image();
        self.after_input();
    }

    /// Resize… and Canvas… answered (one undo step each; `None` removes it).
    pub fn set_resize(&mut self, r: Option<crate::imgedit::Resize>) {
        let e = self.e.image_edits();
        self.e.set_image_edits(crate::imgedit::Edits { resize: r, ..e });
        self.after_input();
    }

    /// Effects… answered (one undo step).
    pub fn set_effects(&mut self, fx: crate::imgedit::effects::Effects) {
        let e = self.e.image_edits();
        self.e.set_image_edits(crate::imgedit::Edits { effects: fx, ..e });
        self.after_input();
    }

    pub fn set_canvas(&mut self, c: Option<crate::imgedit::Pad>) {
        let e = self.e.image_edits();
        self.e.set_image_edits(crate::imgedit::Edits { canvas: c.filter(|c| !c.is_noop()), ..e });
        self.after_input();
    }

    /// The picture turned (Rotate, Flip, an undo of them): the lasso's
    /// coverage and the view follow.
    fn sync_orient(&mut self) {
        let o = self.e.snip.edits.orient;
        if o == self.orient_seen {
            return;
        }
        self.orient_seen = o;
        let size = self.size();
        self.mask = self.e.snip.mask.clone().filter(|m| crate::objects::mask::is_shape(m)).map(|m| {
            let cov = crate::objects::mask::coverage(&m, size.0, size.1, Pt::new(0.0, 0.0));
            (m, cov)
        });
        self.cache = None;
        self.sprites = None;
        self.bake_excl.clear();
        if self.fit {
            self.zoom_fit();
        } else {
            self.clamp_view();
        }
    }

    /// The document's crop and the capture's lasso (as saved: no unsaved
    /// change).
    #[cfg_attr(target_os = "macos", allow(dead_code))] // no Edit window there yet
    pub fn set_doc(&mut self, crop: Option<FRect>, mask: Option<Vec<Pt>>) {
        let size = self.size();
        let crop = crop.and_then(|c| crate::objects::mask::crop_rect(c, size));
        self.e.snip.crop = crop;
        self.e.snip.crop_hist = vec![crop];
        self.saved_crop = crop;
        // The editor knows the lasso too: Text mode and Redact read only
        // inside it.
        self.e.snip.mask = mask.clone().filter(|m| crate::objects::mask::is_shape(m));
        self.e.snip.mask_sel = self.e.sel;
        self.e.snip.doc_size = size;
        self.e.snip.doc_mask = self.e.snip.mask.clone();
        self.mask = mask.filter(|m| crate::objects::mask::is_shape(m)).map(|m| {
            let cov = crate::objects::mask::coverage(&m, size.0, size.1, Pt::new(0.0, 0.0));
            (m, cov)
        });
    }

    /// The crop (non-destructive: `Save` writes it with the objects).
    pub fn crop(&self) -> Option<FRect> {
        self.e.crop_of_doc()
    }

    /// A recognition finished (Text mode, Quick Redact): true when a
    /// frame is due.
    pub fn poll(&mut self) -> bool {
        let c = self.e.poll_snip();
        // A Save or Copy that waited for Redact goes now.
        self.after_input();
        c
    }

    /// The editor's notice (Text mode, Redact, Crop), for the window's toast.
    pub fn take_notice(&mut self) -> Option<(String, ToastKind)> {
        self.e.notice.take().map(|t| (t.text, t.kind))
    }

    pub fn size(&self) -> (u32, u32) {
        self.e.shot.size
    }

    pub fn objects(&self) -> &[Obj] {
        &self.e.objects
    }

    /// The capture with its objects and its image edits (what Save writes).
    pub fn export(&mut self) -> PixBuf {
        let e = self.e.image_edits();
        crate::imgedit::finish(self.export_picture(), &e)
    }

    /// [`Self::export`] before the resize and the canvas (what Copy text reads).
    pub fn export_picture(&mut self) -> PixBuf {
        if self.e.dirty || self.e.spots_stale() {
            let d = self.e.rebuild();
            self.cache_damage(d);
        }
        // A crop or a lasso: baked again over the picture with what they
        // leave out blanked (a magnifier inside never shows it), then cut.
        let crop = self.crop();
        // (The lasso as the editor has it now, in the picture's current turn.)
        let mask = self.e.snip.mask.as_deref().filter(|m| crate::objects::mask::is_shape(m));
        if crop.is_some() || mask.is_some() {
            return crate::history::render(&self.e.base, &self.e.objects, crop, mask);
        }
        self.e.composed().clone()
    }

    /// The objects differ from the last save, or text is being typed.
    pub fn unsaved(&self) -> bool {
        // (Same edits: the same orientation, so the same frame.)
        self.e.snip.edits != self.saved_edits
            || self.e.objects != self.saved
            || self.e.snip.crop != self.saved_crop
            || self.e.text.as_ref().is_some_and(|t| !t.text.trim().is_empty())
    }

    /// `objects` were saved (what [`Self::objects`] was when the save began).
    pub fn mark_saved_as(&mut self, objects: Vec<Obj>) {
        self.saved = objects;
        self.saved_crop = self.e.snip.crop;
        self.saved_edits = self.e.snip.edits;
    }

    /// Text being typed (a text box, a callout's text) becomes an object.
    pub fn commit_drafts(&mut self) {
        if self.e.text.is_some() {
            self.e.commit_text();
        }
        self.e.finish_callout_edit();
        self.after_input();
    }

    pub fn theme(&self) -> &Theme {
        &self.e.th
    }

    pub fn keymap(&self) -> &Keymap {
        &self.e.keys
    }

    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    // --- the view ---------------------------------------------------------------

    /// The canvas at `area` and the toolbar strip at `bar` in a window
    /// `win` (physical px) at scale `k`.
    pub fn set_rects(&mut self, area: FRect, bar: FRect, win: FRect, k: f32) {
        let changed = area != self.area || k != self.k;
        self.area = area;
        self.bar = bar;
        self.win = win;
        self.k = k;
        if changed {
            if self.fit {
                self.zoom_fit();
            } else {
                self.clamp_view();
            }
        }
        self.sync_scale();
    }

    pub fn is_fit(&self) -> bool {
        self.fit
    }

    pub fn zoom(&self) -> f32 {
        self.view.z
    }

    /// The zoom at which the capture fits the canvas (never above 100 %).
    pub fn fit_zoom(&self) -> f32 {
        let (iw, ih) = (self.size().0.max(1) as f32, self.size().1.max(1) as f32);
        let m = FIT_MARGIN * self.k;
        let z = ((self.area.w - 2.0 * m) / iw).min((self.area.h - 2.0 * m) / ih);
        z.clamp(0.02, 1.0)
    }

    /// Zoom to Fit (and follow the window).
    pub fn zoom_fit(&mut self) {
        let z = self.fit_zoom();
        self.fit = true;
        self.set_zoom_centred(z);
    }

    /// 100 %, centred on the canvas's centre point of the capture.
    pub fn zoom_actual(&mut self) {
        let c = Pt::new(self.area.x + self.area.w / 2.0, self.area.y + self.area.h / 2.0);
        self.zoom_at(1.0, c);
    }

    /// One step in or out (`dir` > 0: in) about the canvas's centre.
    pub fn zoom_step(&mut self, dir: i32) {
        let z = self.view.z;
        let next = if dir > 0 {
            ZOOMS.iter().copied().find(|s| *s > z * 1.001).unwrap_or(*ZOOMS.last().unwrap())
        } else {
            ZOOMS.iter().rev().copied().find(|s| *s < z * 0.999).unwrap_or(ZOOMS[0])
        };
        let c = Pt::new(self.area.x + self.area.w / 2.0, self.area.y + self.area.h / 2.0);
        self.zoom_at(next, c);
    }

    /// Zoom by `factor` keeping the capture's point under window point
    /// `at` there (Ctrl+wheel).
    pub fn zoom_by(&mut self, factor: f32, at: Pt) {
        let z = (self.view.z * factor).clamp(ZOOMS[0], *ZOOMS.last().unwrap());
        self.zoom_at(z, at);
    }

    fn zoom_at(&mut self, z: f32, at: Pt) {
        let ip = self.view.to_img(at);
        self.fit = false;
        self.view.z = z;
        self.view.ox = (at.x - ip.x * z).round();
        self.view.oy = (at.y - ip.y * z).round();
        self.clamp_view();
        self.sync_scale();
    }

    fn set_zoom_centred(&mut self, z: f32) {
        let (w, h) = disp(self.size(), z);
        self.view = View { z, ox: (self.area.x + (self.area.w - w as f32) / 2.0).round(), oy: (self.area.y + (self.area.h - h as f32) / 2.0).round() };
        self.sync_scale();
    }

    /// Keep the picture on the canvas: centred along an axis it fits in,
    /// else never leaving a gap past its edges.
    fn clamp_view(&mut self) {
        let (w, h) = disp(self.size(), self.view.z);
        let (w, h) = (w as f32, h as f32);
        let a = self.area;
        self.view.ox = if w <= a.w { (a.x + (a.w - w) / 2.0).round() } else { self.view.ox.clamp(a.x1() - w, a.x).round() };
        self.view.oy = if h <= a.h { (a.y + (a.h - h) / 2.0).round() } else { self.view.oy.clamp(a.y1() - h, a.y).round() };
    }

    /// Pan by (`dx`, `dy`) window px (the wheel, a drag).
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.fit = false;
        self.view.ox += dx;
        self.view.oy += dy;
        self.clamp_view();
    }

    /// The editor's chrome scale follows the zoom.
    fn sync_scale(&mut self) {
        if let Some(h) = self.e.host.as_mut() {
            h.s = self.k / self.view.z.max(1e-3);
        }
    }

    /// The visible part of the capture (image px), where the
    /// mini-toolbar and the loupe stay.
    fn visible_img(&self) -> FRect {
        let a = self.view.to_img(Pt::new(self.area.x, self.area.y));
        let b = self.view.to_img(Pt::new(self.area.x1(), self.area.y1()));
        FRect::from_pts(a, b)
    }

    // --- input ------------------------------------------------------------------------

    /// Whether `p` (window px) is on the canvas.
    pub fn on_canvas(&self, p: Pt) -> bool {
        hit(self.area, p)
    }

    /// Whether `p` is on the toolbar or its open popover.
    pub fn on_bar(&self, p: Pt) -> bool {
        self.e.toolbar.as_ref().is_some_and(|t| t.contains(p))
    }

    /// A press at window point `p`: the toolbar's, the canvas's (Space
    /// held: a pan). Returns whether it was taken.
    pub fn press(&mut self, p: Pt) -> bool {
        self.mouse = p;
        if self.e.bar_press(p) {
            self.after_input();
            return true;
        }
        // An open popover or picker closes on a press outside it.
        if !self.on_canvas(p) {
            if self.e.palette_open || self.e.save_menu || self.e.slot_menu || self.e.pick.is_some() {
                self.e.palette_open = false;
                self.e.save_menu = false;
                self.e.slot_menu = false;
                self.e.close_picker(true);
                return true;
            }
            return false;
        }
        if self.space {
            self.pan = Some((p, self.view));
            return true;
        }
        self.e.save_menu = false;
        self.e.slot_menu = false;
        self.e.canvas_press(self.view.to_img(p));
        self.after_input();
        true
    }

    /// The middle button: a pan.
    pub fn middle_press(&mut self, p: Pt) {
        if self.pan.is_none() && self.on_canvas(p) {
            self.pan = Some((p, self.view));
            self.middle = true;
        }
    }

    /// The middle button went up: the end of its pan only (a left drag
    /// meanwhile goes on).
    pub fn middle_release(&mut self, p: Pt) {
        if std::mem::take(&mut self.middle) {
            self.mouse = p;
            self.pan = None;
        }
    }

    /// The pointer moved to window point `p` (a button held or not).
    /// Returns whether something changed (a repaint is due).
    pub fn moved(&mut self, p: Pt, held: bool) -> bool {
        self.mouse = p;
        if let Some((start, v)) = self.pan {
            self.fit = false;
            self.view = View { ox: v.ox + p.x - start.x, oy: v.oy + p.y - start.y, ..v };
            self.clamp_view();
            return true;
        }
        let ip = self.view.to_img(p);
        if held && !matches!(self.e.interact, Interact::None) {
            let _p = crate::perf::span(crate::perf::K::Ptr);
            self.e.pointer_move(ip, p);
            return true;
        }
        self.e.track_hover_at(ip, p) | self.e.track_loupe(ip)
    }

    /// The button went up at `p`.
    pub fn release(&mut self, p: Pt) {
        self.mouse = p;
        if !self.middle && self.pan.take().is_some() {
            return;
        }
        self.e.pointer_up();
        self.after_input();
    }

    /// A key (`up`: released). Returns whether the editor took it (the
    /// window's own keys go first).
    pub fn key(&mut self, vk: u32, up: bool, repeat: bool, mods: Mods) {
        if vk == wind::key::SPACE && self.e.text.is_none() && self.e.obj.cedit.is_none() && self.e.pick.is_none() {
            // Space pans while held (no palette toggle in the Edit window).
            self.space = !up;
            return;
        }
        let ip = self.view.to_img(self.mouse);
        if (vk == 0x10 || vk == 0x11)
            && let Interact::Drawing { start } = self.e.interact
        {
            let mut m = mods;
            if vk == 0x10 {
                m.shift = !up;
            } else {
                m.ctrl = !up;
            }
            self.e.drag_draft(start, ip, m);
        }
        if up {
            return;
        }
        self.e.handle_key(vk, repeat, mods, ip);
        self.after_input();
    }

    pub fn char_input(&mut self, c: u16) {
        self.e.char_input(c);
        self.after_input();
    }

    /// The wheel (no Ctrl): the size of the tool or the selected objects.
    pub fn wheel_size(&mut self, delta: i32) {
        self.e.wheel(delta);
    }

    /// What a finished capture step asks the window to do.
    fn after_input(&mut self) {
        self.sync_orient();
        if !self.e.done {
            return;
        }
        self.e.done = false;
        let tasks = std::mem::take(&mut self.e.tasks);
        self.request = if std::mem::take(&mut self.e.cancelled) {
            Some(Request::Close)
        } else {
            tasks.first().map(|t| match t {
                Task::Save { ask: true, .. } => Request::SaveAs,
                Task::Save { .. } => Request::Save,
                Task::SaveDesktop => Request::SaveDesktop,
                Task::OpenWith => Request::OpenWith,
                Task::Share => Request::Share,
                Task::Upload => Request::Upload,
                Task::Pin => Request::Pin,
                Task::Text => Request::CopyText,
                Task::Qr => Request::ReadQr,
                _ => Request::Copy,
            })
        };
    }

    /// The cursor at window point `p`.
    pub fn cursor(&self, p: Pt) -> Cursor {
        if self.pan.is_some() || self.space {
            return Cursor::Move;
        }
        if !self.on_canvas(p) || self.on_bar(p) {
            return Cursor::Arrow;
        }
        self.e.cursor_at(self.view.to_img(p), super::drag_mods())
    }

    /// The pointer in image px (the status bar).
    pub fn mouse_img(&self) -> Option<Pt> {
        let p = self.view.to_img(self.mouse);
        let (w, h) = self.size();
        (self.on_canvas(self.mouse) && p.x >= 0.0 && p.y >= 0.0 && p.x < w as f32 && p.y < h as f32).then_some(p)
    }

    // --- the objects list ----------------------------------------------------------

    pub fn selected(&self) -> &[usize] {
        &self.e.obj.sel
    }

    /// Select object `i` (`add`: Ctrl, toggled), as a click with Select.
    pub fn select(&mut self, i: usize, add: bool) {
        if i >= self.e.objects.len() {
            return;
        }
        self.e.finish_callout_edit();
        if self.e.tool != Some(Tool::Select) {
            self.e.tool = Some(Tool::Select);
        }
        if add {
            if let Some(p) = self.e.obj.sel.iter().position(|s| *s == i) {
                self.e.obj.sel.remove(p);
            } else {
                self.e.obj.sel.push(i);
                crate::sort::sort(&mut self.e.obj.sel);
            }
        } else {
            self.e.obj.sel = vec![i];
        }
    }

    /// Move object `from` to z-index `to` (one undo step).
    pub fn reorder(&mut self, from: usize, to: usize) {
        let n = self.e.objects.len();
        if from >= n || to >= n || from == to {
            return;
        }
        self.e.finish_callout_edit();
        let o = self.e.objects.remove(from);
        self.e.objects.insert(to, o);
        self.e.obj.sel = vec![to];
        self.e.push_history();
        self.e.dirty = true;
    }

    /// The selection's box (image px), and the first selected object.
    pub fn selection_box(&self) -> Option<(FRect, &Obj)> {
        let i = *self.e.obj.sel.first()?;
        Some((self.e.osel_box()?, self.e.objects.get(i)?))
    }

    /// Something animates (the caret blinks, guides fade, a tooltip is
    /// due, a popover fades): the window keeps its fast timer.
    pub fn animating(&self, now: Instant) -> bool {
        super::animating(&self.e, None, now) || self.e.text.is_some() || self.e.obj.cedit.is_some() || self.e.snip_busy()
    }

    /// Write the tools' remembered sizes and colors (`wait`: now).
    pub fn flush(&mut self, wait: bool) {
        if wait || self.e.pending.is_due(Instant::now()) {
            self.e.close_picker(true);
            self.e.flush_pending(wait);
        }
    }

    // --- painting -------------------------------------------------------------------

    /// Lay out the docked toolbar (and its popovers) for this frame.
    fn layout_bar(&mut self, now: Instant) {
        let e = &mut self.e;
        e.slot_long_press(now);
        let colors = palette_colors(&e.cfg);
        let cur = e.cur_tool();
        let recents = e.recents();
        let pv = e.pick.as_ref().map(|p| toolbar::PickView { ring: false, alpha: p.has_alpha(), recents: &recents });
        let save_keys: Vec<String> = toolbar::save_menu().iter().map(|&(a, _)| chrome::act_tip(a, &e.keys).1.join("+")).collect();
        let slot_keys = toolbar::SLOT_MENU.map(|(a, _)| chrome::act_tip(a, &e.keys).1.join("+"));
        let inp = toolbar::Input {
            sel: e.sel.unwrap_or(FRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }),
            area: self.win,
            s: self.k,
            busy: false,
            can_undo: e.hi > 0,
            can_redo: e.hi + 1 < e.hist.len(),
            palette: e.palette_open.then_some(colors.as_slice()),
            record: false,
            rec: None,
            grip: None,
            top: false,
            sizes: tools::has_size(cur),
            colors: tools::has_color(cur) || e.sel_color().is_some(),
            fills: tools::has_fill(cur),
            pick: pv.as_ref(),
            save_menu: e.save_menu.then_some(save_keys.as_slice()),
            slot_menu: e.slot_menu.then_some(&slot_keys),
            copy_menu: None,
            freeform: None,
        };
        e.toolbar = Some(toolbar::layout_docked(&inp, self.bar));
        e.mo.bar.snap(1.0);
        if e.palette_open || e.pick.is_some() || e.save_menu || e.slot_menu {
            e.mo.pop.set(1.0, 100, now);
        } else {
            e.mo.pop.snap(0.0);
        }
    }

    /// The docked toolbar's buttons' width (logical px) — where the
    /// window's zoom control can go.
    pub fn bar_width(&self) -> f32 {
        let cur = self.e.cur_tool();
        let inp = toolbar::Input {
            sel: FRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            area: self.win,
            s: self.k,
            busy: false,
            can_undo: true,
            can_redo: true,
            palette: None,
            record: false,
            rec: None,
            grip: None,
            top: false,
            sizes: tools::has_size(cur),
            colors: true,
            fills: true,
            pick: None,
            save_menu: None,
            slot_menu: None,
            copy_menu: None,
            freeform: None,
        };
        toolbar::docked_width(&inp)
    }

    /// The objects a drag is moving, resizing or re-aiming (not the
    /// spotlights: their dim must follow the drag, baked).
    fn dragged(&self) -> Vec<usize> {
        let idx: &[usize] = match &self.e.interact {
            Interact::ObjMove { moved: true, idx, .. } | Interact::ObjResize { idx, .. } => idx,
            Interact::ObjPoint { idx, .. } => std::slice::from_ref(idx),
            _ => &[],
        };
        idx.iter().copied().filter(|&i| self.e.objects.get(i).is_some_and(|o| !o.is_spot())).collect()
    }

    /// Bring the scaled copy in line with `composed` where a bake changed it.
    fn cache_damage(&mut self, d: Option<Vec<PxRect>>) {
        let _p = crate::perf::span(crate::perf::K::Blit);
        let Some(c) = self.cache.as_mut() else { return };
        let src = self.e.composed.as_ref().unwrap_or(&self.e.base);
        let (iw, ih) = (src.width() as usize, src.height() as usize);
        let rects = match d {
            Some(r) => r,
            None => vec![PxRect::image((iw as u32, ih as u32))],
        };
        let (cw, ch) = (c.w as usize, c.h as usize);
        for r in rects {
            let r = r.intersect(&PxRect::image((iw as u32, ih as u32)));
            if r.is_empty() {
                continue;
            }
            // The destination pixels whose spans meet `r` (one more each side).
            let x0 = (r.x0 as usize * cw / iw).saturating_sub(1);
            let y0 = (r.y0 as usize * ch / ih).saturating_sub(1);
            let x1 = ((r.x1 as usize * cw).div_ceil(iw) + 1).min(cw);
            let y1 = ((r.y1 as usize * ch).div_ceil(ih) + 1).min(ch);
            crate::scale::box_rect(src.as_raw(), iw, ih, &mut c.px, cw, ch, (x0, y0, x1, y1));
        }
    }

    /// The scaled copy for the current zoom (below 100 %), made when the
    /// zoom changed.
    fn ensure_cache(&mut self) {
        let z = self.view.z;
        if z >= 0.999 {
            self.cache = None;
            return;
        }
        if self.cache.as_ref().is_some_and(|c| c.z == z) {
            return;
        }
        let _p = crate::perf::span(crate::perf::K::Bake);
        let src = self.e.composed.as_ref().unwrap_or(&self.e.base);
        let (w, h) = disp(src.dimensions(), z);
        let mut px = vec![0u8; w as usize * h as usize * 4];
        crate::scale::box_rect(src.as_raw(), src.width() as usize, src.height() as usize, &mut px, w as usize, h as usize, (0, 0, w as usize, h as usize));
        self.cache = Some(Cache { z, w, h, px });
    }

    /// Paint the canvas into `f` (the window's frame, physical px): the
    /// canvas color `cv`, the picture, the drafts and the object chrome;
    /// then the docked toolbar (its strip is the window's).
    pub fn paint(&mut self, f: &mut Fb, cv: C4, edge: C4, now: Instant) {
        self.layout_bar(now);
        // Bake what changed, then the scaled copy follows. The objects a
        // drag moves stay out of the bake meanwhile (drawn over it in
        // window px, baked again on release): a move costs no re-bake.
        let drag = self.dragged();
        if self.e.dirty || self.e.spots_stale() || drag != self.bake_excl {
            let _p = crate::perf::span(crate::perf::K::Bake);
            let d = if drag.is_empty() {
                self.e.rebuild()
            } else {
                let keep: Vec<Obj> = self.e.objects.iter().enumerate().filter(|(i, _)| !drag.contains(i)).map(|(_, o)| o.clone()).collect();
                let all = std::mem::replace(&mut self.e.objects, keep);
                let d = self.e.rebuild();
                self.e.objects = all;
                d
            };
            self.bake_excl = drag;
            if self.cache.as_ref().is_some_and(|c| c.z == self.view.z) {
                self.cache_damage(d);
            } else {
                self.cache = None;
            }
        }
        self.ensure_cache();
        let a = self.area;
        let img = self.view.image(self.size());
        // The canvas color round the picture (not under it).
        let pic = PxRect::new(img.x as i32, img.y as i32, img.x1() as i32, img.y1() as i32);
        for r in PxRect::new(a.x as i32, a.y as i32, a.x1() as i32, a.y1() as i32).minus_iter(&pic) {
            f.fill_rect(r.x0, r.y0, r.w(), r.h(), cv);
        }
        // The picture's edge (dark: 1 px border; light: a soft shadow).
        let k = self.k;
        if edge.a > 0 {
            // A ring just outside the picture (never under it: a blend
            // over the whole picture would cost more than the picture).
            let lw = k.round().max(1.0) as i32;
            let (x0, y0, x1, y1) = (img.x as i32 - lw, img.y as i32 - lw, img.x1() as i32 + lw, img.y1() as i32 + lw);
            let outer = PxRect::new(x0, y0, x1, y1).intersect(&PxRect::new(a.x as i32, a.y as i32, a.x1() as i32, a.y1() as i32));
            for r in outer.minus_iter(&pic) {
                f.fill_rect(r.x0, r.y0, r.w(), r.h(), edge);
            }
        }
        self.blit_picture(f);
        self.paint_drafts(f);
        self.paint_hidden(f, cv);
        self.paint_chrome(f, now);
    }

    /// The picture (with its objects) into the canvas.
    fn blit_picture(&self, f: &mut Fb) {
        let _p = crate::perf::span(crate::perf::K::Bg);
        let a = self.area;
        let v = self.view;
        let img = v.image(self.size());
        let x0 = img.x.max(a.x).max(0.0) as i32;
        let y0 = img.y.max(a.y).max(0.0) as i32;
        let x1 = (img.x1().min(a.x1()) as i32).min(f.stride as i32);
        let y1 = (img.y1().min(a.y1()) as i32).min(f.height());
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let stride = f.stride * 4;
        let n = (x1 - x0) as usize;
        let (ox, oy) = (v.ox as i32, v.oy as i32);
        if let Some(c) = &self.cache {
            for y in y0..y1 {
                let sy = (y - oy) as usize;
                let s = (sy * c.w as usize + (x0 - ox) as usize) * 4;
                let d = y as usize * stride + x0 as usize * 4;
                f.d[d..d + n * 4].copy_from_slice(&c.px[s..s + n * 4]);
            }
            return;
        }
        let src = self.e.composed();
        let (iw, ih) = (src.width() as usize, src.height() as usize);
        let raw = src.as_raw();
        if (v.z - 1.0).abs() < 1e-4 {
            for y in y0..y1 {
                let sy = ((y - oy) as usize).min(ih - 1);
                let s = (sy * iw + ((x0 - ox) as usize).min(iw - 1)) * 4;
                let len = (n * 4).min(raw.len() - s);
                let d = y as usize * stride + x0 as usize * 4;
                f.d[d..d + len].copy_from_slice(&raw[s..s + len]);
            }
            return;
        }
        // Above 100 %: nearest neighbour, a row reused while it repeats.
        let xs: Vec<usize> = (x0..x1).map(|x| (((x as f32 + 0.5 - v.ox) / v.z) as usize).min(iw - 1)).collect();
        let mut last: Option<(usize, usize)> = None;
        for y in y0..y1 {
            let sy = (((y as f32 + 0.5 - v.oy) / v.z) as usize).min(ih - 1);
            let d = y as usize * stride + x0 as usize * 4;
            if let Some((ly, ld)) = last
                && ly == sy
            {
                f.d.copy_within(ld..ld + n * 4, d);
                continue;
            }
            let row = &raw[sy * iw * 4..(sy + 1) * iw * 4];
            let out = &mut f.d[d..d + n * 4];
            for (o, &sx) in out.as_chunks_mut::<4>().0.iter_mut().zip(&xs) {
                o.copy_from_slice(&row[sx * 4..sx * 4 + 4]);
            }
            last = Some((sy, d));
        }
    }

    /// The draft and the drag's ghost, as objects in window px (they may
    /// reach past the canvas: the window paints its rail, toolbar strip
    /// and status bar after).
    fn paint_drafts(&mut self, f: &mut Fb) {
        let v = self.view;
        let moving = match &self.e.interact {
            Interact::ObjMove { moved: true, orig, idx, .. } => Some((orig.clone(), idx.clone())),
            _ => None,
        };
        if moving.is_none() && self.e.draft.is_none() && self.bake_excl.is_empty() {
            self.sprites = None;
            return;
        }
        let _p = crate::perf::span(crate::perf::K::Compose);
        // A move only translates: the ghost and the moved objects are
        // rendered once into sprites, then blitted at the offset.
        let mut drawn = false;
        if let Some((orig, idx)) = &moving
            && orig.iter().all(|o| !reads_below(o))
            && self.bake_excl == *idx
        {
            let key = (v.z, v.ox, v.oy, idx.clone());
            if self.sprites.as_ref().is_none_or(|s| s.key != key) {
                let font = self.e.font.as_ref();
                let clip = FRect { x: self.area.x - self.area.w, y: self.area.y - self.area.h, w: 3.0 * self.area.w, h: 3.0 * self.area.h };
                let ghost: Vec<Obj> = orig.iter().filter_map(|o| crate::objects::edit::faded(o, select::GHOST)).map(|o| view_obj(&o, &v)).collect();
                let held: Vec<Obj> = orig.iter().map(|o| view_obj(o, &v)).collect();
                let at = idx.first().and_then(|_| crate::objects::edit::sel_box(&orig[0], font)).unwrap_or(FRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
                self.sprites = Some(Sprites { key, ghost: Sprite::of(&ghost, font, clip), held: Sprite::of(&held, font, clip), at });
            }
            if let Some(s) = &self.sprites {
                let now = idx.first().and_then(|&i| self.e.objects.get(i)).and_then(|o| crate::objects::edit::sel_box(o, self.e.font.as_ref())).unwrap_or(s.at);
                let (dx, dy) = (((now.x - s.at.x) * v.z).round() as i32, ((now.y - s.at.y) * v.z).round() as i32);
                if let Some(g) = &s.ghost {
                    g.blit(f, 0, 0);
                }
                if let Some(h) = &s.held {
                    h.blit(f, dx, dy);
                }
                drawn = true;
            }
        } else {
            self.sprites = None;
        }
        let e = &self.e;
        let font = e.font.as_ref();
        let src = e.composed.as_ref().unwrap_or(&e.base);
        let area = self.area;
        let mut sf = f.surf();
        // A blur is computed on the image (as export does), then scaled.
        let draw = |sf: &mut crate::raster::Surf, o: &Obj| match o {
            Obj::Blur { r, sigma } => paint_blur(sf, src, *r, *sigma, &v, area),
            o => view_obj(o, &v).render_into(sf, font),
        };
        if !drawn {
            if let Some((orig, _)) = &moving {
                for g in orig.iter().filter_map(|o| crate::objects::edit::faded(o, select::GHOST)) {
                    draw(&mut sf, &g);
                }
            }
            // What the drag holds (left out of the bake).
            for &i in &self.bake_excl {
                if let Some(o) = e.objects.get(i) {
                    draw(&mut sf, o);
                }
            }
        }
        match &e.draft {
            Some(Obj::Spotlight { .. }) | None => {}
            Some(d) => draw(&mut sf, d),
        }
    }

    /// What the crop and the lasso leave out, in the canvas color (the
    /// crop frame up: all of the picture shows, the frame dims the rest).
    fn paint_hidden(&self, f: &mut Fb, cv: C4) {
        if self.e.snip.crop_mode.is_some() {
            return;
        }
        let v = self.view;
        let a = self.area;
        let img = v.image(self.size());
        let vis = PxRect::new(img.x.max(a.x) as i32, img.y.max(a.y) as i32, img.x1().min(a.x1()) as i32, img.y1().min(a.y1()) as i32);
        if vis.is_empty() {
            return;
        }
        if let Some((_, cov)) = &self.mask {
            let (iw, ih) = (self.size().0 as usize, self.size().1 as usize);
            for y in vis.y0..vis.y1 {
                let sy = (((y as f32 + 0.5 - v.oy) / v.z) as usize).min(ih - 1);
                let row = &cov[sy * iw..(sy + 1) * iw];
                for x in vis.x0..vis.x1 {
                    let sx = (((x as f32 + 0.5 - v.ox) / v.z) as usize).min(iw - 1);
                    let c = row[sx];
                    if c < 255 {
                        f.blend_px(x, y, cv.with_alpha(255 - c));
                    }
                }
            }
        }
        if let Some(c) = self.crop() {
            let w = v.rect(c);
            let keep = PxRect::new(w.x.round() as i32, w.y.round() as i32, w.x1().round() as i32, w.y1().round() as i32);
            for r in vis.minus_iter(&keep) {
                f.fill_rect(r.x0, r.y0, r.w(), r.h(), cv);
            }
        }
    }

    /// The object chrome (selection, handles, guides, marquee, caret,
    /// mini-toolbar and its tips, the text draft, the loupe) mapped to the
    /// window.
    fn paint_chrome(&mut self, f: &mut Fb, now: Instant) {
        let _p = crate::perf::span(crate::perf::K::Sel);
        let s_img = self.e.ui_scale();
        let area_img = self.visible_img();
        let (_ghost, osel, marquee, caret, mini) = self.e.obj_scene(area_img, s_img);
        let (guides, gk) = self.e.guides_now(now);
        let v = self.view;
        let mouse_img = v.to_img(self.mouse);
        let snip = self.e.snip_scene(area_img, s_img, 0, mouse_img);
        let a = self.area;
        let k = self.k;
        let e = &self.e;
        let ui = Ui { th: &e.th, s: k, font: e.ui_font };
        // Text mode, the crop frame: drawn in window px.
        if let Some(u) = snip {
            super::snip::draw(f, &ui, &super::snip::map_ui(&u, &|r| v.rect(r), &|p| v.to_win(p)));
        }
        if let Some(Obj::Spotlight { r }) = &e.draft {
            super::mini::marquee(f, &ui, v.rect(*r));
        }
        if let Some(u) = &osel {
            super::mini::osel(f, &ui, &map_osel(u, &v));
        }
        if !guides.is_empty() {
            let g: Vec<(Pt, Pt)> = guides.iter().map(|(p, q)| (v.to_win(*p), v.to_win(*q))).collect();
            super::mini::guides(f, &ui, &g, gk);
        }
        if let Some(m) = marquee {
            super::mini::marquee(f, &ui, v.rect(m));
        }
        if let Some(c) = &caret {
            super::mini::caret(f, &CaretUi { r: v.rect(c.r), color: c.color });
        }
        // The text tool's draft: its dashed box and the text as it will be.
        if let Some(td) = &e.text {
            let tfont = e.text_font();
            let boxr = text_box_rect(e.shot.size.0 as f32, td, e.mem.size(Tool::Text), tfont.as_ref(), s_img);
            let (px, color) = (e.mem.size(Tool::Text) * v.z, e.mem.color(Tool::Text));
            f.stroke_dashed_rect(v.rect(boxr), 4.0 * k, 3.0 * k, k.round().max(1.0), e.th.accent);
            if let Some(font) = tfont.as_ref() {
                let pos = v.to_win(td.pos);
                font.render(&mut f.surf(), &td.text, px, pos, color);
                if caret_on(td) {
                    let at = font.caret_at(&td.text, td.caret, px);
                    let (cx, cy) = (pos.x + at.x, pos.y + at.y);
                    f.fill_rect(cx.round() as i32, cy.round() as i32, (2.0 * k).round() as i32, (px * 1.15).round() as i32, color);
                }
            }
        }
        let mini = mini.map(|m| map_mini(&m, &v));
        if let Some(m) = &mini {
            super::mini::draw(f, &ui, m);
        }
        // The eyedropper's loupe.
        if e.pick.as_ref().is_some_and(|p| p.eyedrop)
            && let Some(p) = e.loupe
        {
            let r = chrome::loupe_rect(&ui, v.to_win(p), a, &[]);
            chrome::loupe(f, &ui, r, &e.loupe_pixels(p));
        }
        // A tip for the mini-toolbar's buttons.
        let interacting = !matches!(e.interact, Interact::None);
        let shown_ms = now.saturating_duration_since(e.hover_at).as_secs_f32() * 1000.0 - 400.0;
        if !interacting
            && shown_ms >= 0.0
            && let Some(m) = &mini
            && let Some(it) = m.hover.and_then(|i| m.items.get(i))
            && let toolbar::Kind::Btn(act) = it.kind
        {
            let (label, keys) = chrome::act_tip(act, &e.keys);
            let at = chrome::TipAt { anchor: it.r, area: a, below: false };
            chrome::tooltip(f, &ui, at, label, &keys, anim::ease_out(shown_ms / 80.0));
        }
    }

    /// The docked toolbar with its popover and tooltip (after the window
    /// drew its strip).
    pub fn paint_bar(&self, f: &mut Fb, now: Instant) {
        let _p = crate::perf::span(crate::perf::K::Bar);
        let e = &self.e;
        let Some(tb) = &e.toolbar else { return };
        let ui = Ui { th: &e.th, s: self.k, font: e.ui_font };
        let value = e.size_label();
        let cur = e.cur_tool();
        let st = chrome::BarState {
            hover: e.hover,
            hover_k: e.mo.hover.value(now),
            pressed: e.pressed,
            tool: e.tool,
            color: e.sel_color().unwrap_or_else(|| e.color()),
            value: &value,
            ptool: cur,
            size: e.mem.size(cur),
            fill: e.mem.fill(cur),
            pop_k: e.mo.pop.value(now),
            pick: e.pick.as_ref(),
        };
        chrome::toolbar(f, &ui, tb, &st, 1.0);
        let interacting = !matches!(e.interact, Interact::None);
        let shown_ms = now.saturating_duration_since(e.hover_at).as_secs_f32() * 1000.0 - 400.0;
        if !interacting
            && shown_ms >= 0.0
            && let Some(it) = e.hover.and_then(|i| tb.items.get(i))
            && let toolbar::Kind::Btn(act) | toolbar::Kind::Rainbow(act) = it.kind
        {
            let (label, keys) = chrome::act_tip(toolbar::slot_act(act, e.tool), &e.keys);
            let in_pop = tb.pop.is_some_and(|p| toolbar::hit(p, Pt::new(it.r.x + 1.0, it.r.y + 1.0)));
            let at = chrome::TipAt { anchor: it.r, area: self.win, below: !in_pop };
            chrome::tooltip(f, &ui, at, label, &keys, anim::ease_out(shown_ms / 80.0));
        }
    }
}

/// A blur drawn in the window while drafted or dragged: computed on the
/// image `src` (the capture as baked below it) over its whole rect at its
/// real strength, exactly as the export bakes it, then drawn scaled into
/// the window (nearest pixel) inside `clip`. Not from the pixels in view:
/// the same at any zoom and while panning.
fn paint_blur(sf: &mut crate::raster::Surf, src: &crate::pixbuf::PixBuf, r: FRect, sigma: f32, v: &View, clip: FRect) {
    let (iw, ih) = (src.width() as i32, src.height() as i32);
    let (x0, y0, x1, y1) = crate::objects::blur::px_rect(r);
    let (x1, y1) = (x1.min(iw), y1.min(ih));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let raw = src.as_raw();
    let mut block = Vec::with_capacity(w * h * 4);
    for y in y0..y1 {
        let i = (y as usize * iw as usize + x0 as usize) * 4;
        block.extend_from_slice(&raw[i..i + w * 4]);
    }
    crate::objects::blur::blur_block(&mut block, w, h, sigma, &mut Vec::new(), &mut Vec::new());
    let wr = v.rect(FRect { x: x0 as f32, y: y0 as f32, w: w as f32, h: h as f32 });
    let (wx0, wy0) = (wr.x.max(clip.x).floor() as i32, wr.y.max(clip.y).floor() as i32);
    let (wx1, wy1) = (wr.x1().min(clip.x1()).ceil() as i32, wr.y1().min(clip.y1()).ceil() as i32);
    let ch = sf.order().rgb_idx();
    for wy in wy0..wy1 {
        for wx in wx0..wx1 {
            let p = v.to_img(Pt::new(wx as f32 + 0.5, wy as f32 + 0.5));
            let (bx, by) = (p.x.floor() as i32 - x0, p.y.floor() as i32 - y0);
            if bx < 0 || by < 0 || bx >= w as i32 || by >= h as i32 {
                continue;
            }
            let q = &block[(by as usize * w + bx as usize) * 4..][..4];
            let mut px = [0, 0, 0, q[3]];
            for (c, &k) in ch.iter().enumerate() {
                px[k] = q[c];
            }
            sf.mix_raw(wx, wy, px, 1.0);
        }
    }
}

/// An object that reads the pixels below it (it cannot be drawn apart).
fn reads_below(o: &Obj) -> bool {
    matches!(o, Obj::Pixelate { .. } | Obj::Blur { .. } | Obj::Invert { .. } | Obj::Magnify { .. } | Obj::Spotlight { .. })
}

/// Objects rendered once over transparency (premultiplied RGBA: the
/// rasterizer's blend over a zero pixel leaves color × alpha), blitted
/// at an offset frame after frame.
struct Sprite {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    px: Vec<u8>,
}

/// A move's sprites: the 40 % ghost and the held objects, for one view
/// (and the held objects' box at the start).
struct Sprites {
    key: (f32, f32, f32, Vec<usize>),
    ghost: Option<Sprite>,
    held: Option<Sprite>,
    at: FRect,
}

impl Sprite {
    /// `objs` (window px) rendered inside `clip`.
    fn of(objs: &[Obj], font: Option<&AnnotFont>, clip: FRect) -> Option<Sprite> {
        let b = objs.iter().filter_map(|o| o.bounds(font)).reduce(|a, b| {
            let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
            FRect { x: x0, y: y0, w: a.x1().max(b.x1()) - x0, h: a.y1().max(b.y1()) - y0 }
        })?;
        let r = PxRect::outer(b).intersect(&PxRect::outer(clip));
        if r.is_empty() {
            return None;
        }
        let (w, h) = (r.w() as usize, r.h() as usize);
        let mut px = vec![0u8; w * h * 4];
        {
            let mut sf = crate::raster::Surf::with_origin(&mut px, w as u32, h as u32, r.x0, r.y0, Order::Rgba);
            for o in objs {
                o.render_into(&mut sf, font);
            }
        }
        Some(Sprite { x: r.x0, y: r.y0, w, h, px })
    }

    /// Premultiplied over at offset (`dx`, `dy`), clipped to `f`.
    fn blit(&self, f: &mut Fb, dx: i32, dy: i32) {
        let (fw, fh) = (f.stride as i32, f.height());
        let (x0, y0) = (self.x + dx, self.y + dy);
        let (sx0, sy0) = ((-x0).max(0) as usize, (-y0).max(0) as usize);
        let (cx1, cy1) = ((x0 + self.w as i32).min(fw), (y0 + self.h as i32).min(fh));
        if cx1 <= x0.max(0) || cy1 <= y0.max(0) {
            return;
        }
        let n = (cx1 - x0.max(0)) as usize;
        for row in sy0..(cy1 - y0) as usize {
            let s = &self.px[(row * self.w + sx0) * 4..][..n * 4];
            let d0 = ((y0 + row as i32) as usize * f.stride + x0.max(0) as usize) * 4;
            let d = &mut f.d[d0..d0 + n * 4];
            for (o, p) in d.as_chunks_mut::<4>().0.iter_mut().zip(s.as_chunks::<4>().0) {
                let a = p[3] as u32;
                if a == 0 {
                    continue;
                }
                if a == 255 {
                    *o = [p[0], p[1], p[2], 255];
                    continue;
                }
                let ia = 255 - a;
                for c in 0..3 {
                    o[c] = (p[c] as u32 + (o[c] as u32 * ia + 127) / 255).min(255) as u8;
                }
                o[3] = 255;
            }
        }
    }
}

/// `o` in window px through `v`: points and rects mapped, strokes and
/// sizes scaled.
pub fn view_obj(o: &Obj, v: &View) -> Obj {
    let p = |q: &Pt| v.to_win(*q);
    let r = |q: &FRect| v.rect(*q);
    let z = v.z;
    match o {
        Obj::Line { a, b, color, width } => Obj::Line { a: p(a), b: p(b), color: *color, width: width * z },
        Obj::Arrow { a, b, color, width } => Obj::Arrow { a: p(a), b: p(b), color: *color, width: width * z },
        Obj::Marker { a, b, color, width } => Obj::Marker { a: p(a), b: p(b), color: *color, width: width * z },
        Obj::Rect { r: q, color, width } => Obj::Rect { r: r(q), color: *color, width: width * z },
        Obj::RectFill { r: q, color, width } => Obj::RectFill { r: r(q), color: *color, width: width * z },
        Obj::Ellipse { r: q, color, width } => Obj::Ellipse { r: r(q), color: *color, width: width * z },
        Obj::EllipseFill { r: q, color, width } => Obj::EllipseFill { r: r(q), color: *color, width: width * z },
        Obj::Path { pts, color, width } => Obj::Path { pts: pts.iter().map(p).collect(), color: *color, width: width * z },
        Obj::MarkerPath { pts, color, width } => Obj::MarkerPath { pts: pts.iter().map(p).collect(), color: *color, width: width * z },
        Obj::Text { pos, text, color, size, fmt } => Obj::Text { pos: p(pos), text: text.clone(), color: *color, size: size * z, fmt: *fmt },
        Obj::Pixelate { r: q, cell } => Obj::Pixelate { r: r(q), cell: (cell * z).max(1.0) },
        Obj::Blur { r: q, sigma } => Obj::Blur { r: r(q), sigma: sigma * z },
        Obj::Invert { r: q } => Obj::Invert { r: r(q) },
        Obj::Spotlight { r: q } => Obj::Spotlight { r: r(q) },
        Obj::Counter { c, n, color, d, tail } => Obj::Counter { c: p(c), n: *n, color: *color, d: d * z, tail: tail.as_ref().map(p) },
        Obj::Callout { r: q, tail, text, color, size, style, fill, fmt } => {
            Obj::Callout { r: r(q), tail: p(tail), text: text.clone(), color: *color, size: size * z, style: *style, fill: *fill, fmt: *fmt }
        }
        Obj::Magnify { src, dst, radius, zoom, color, width } => {
            Obj::Magnify { src: p(src), dst: p(dst), radius: radius * z, zoom: *zoom, color: *color, width: width * z }
        }
        // The pointer's picture at the zoom (it is small: resampled each time).
        Obj::Cursor { at, img } => {
            let (w, h) = (((img.width() as f32 * z).round() as u32).max(1), ((img.height() as f32 * z).round() as u32).max(1));
            let img = if (w, h) == img.dimensions() { img.clone() } else { std::sync::Arc::new(crate::imgedit::resample::resize(img, w, h, crate::imgedit::Filter::Area)) };
            Obj::Cursor { at: p(at), img }
        }
    }
}

fn map_osel(u: &ObjSelUi, v: &View) -> ObjSelUi {
    ObjSelUi {
        boxes: u.boxes.iter().map(|b| v.rect(*b)).collect(),
        line: u.line.map(|(a, b)| (v.to_win(a), v.to_win(b))),
        square: u.square.iter().map(|p| v.to_win(*p)).collect(),
        round: u.round.iter().map(|p| v.to_win(*p)).collect(),
        hot: u.hot,
        hover: u.hover.iter().map(|b| v.rect(*b)).collect(),
        hover_line: u.hover_line.map(|(a, b)| (v.to_win(a), v.to_win(b))),
    }
}

fn map_mini(m: &MiniUi, v: &View) -> MiniUi {
    let r = |q: FRect| {
        let w = v.rect(q);
        FRect { x: w.x.round(), y: w.y.round(), w: w.w.round(), h: w.h.round() }
    };
    MiniUi { r: r(m.r), items: m.items.iter().map(|it| toolbar::Item { r: r(it.r), ..*it }).collect(), inp: m.inp.clone(), hover: m.hover, pop: m.pop.map(r) }
}

/// Helpers for the Edit window's tests.
#[cfg(test)]
impl Canvas {
    /// The objects, to change behind the editor's back (then baked).
    pub fn edit_mut_for_tests(&mut self) -> &mut Vec<Obj> {
        self.e.dirty = true;
        &mut self.e.objects
    }

    /// The docked toolbar's button with icon `icon` (window px).
    pub fn button(&self, icon: &str) -> Option<FRect> {
        let t = self.e.toolbar.as_ref()?;
        t.items.iter().find(|i| matches!(i.kind, toolbar::Kind::Btn(a) if chrome::act_icon(a) == icon)).map(|i| i.r)
    }

    /// Open Save's menu (as its chevron does).
    pub fn open_save_menu_for_tests(&mut self) {
        self.e.save_menu = true;
    }

    /// The rows of the docked toolbar's open menu (window px).
    pub fn bar_menu_rows(&self) -> Vec<FRect> {
        let Some(t) = self.e.toolbar.as_ref() else { return Vec::new() };
        t.items.iter().filter(|i| matches!(i.kind, toolbar::Kind::MenuItem { .. })).map(|i| i.r).collect()
    }

    /// The docked toolbar's buttons: (icon, rect in window px).
    pub fn bar_buttons(&self) -> Vec<(&'static str, FRect)> {
        let Some(t) = self.e.toolbar.as_ref() else { return Vec::new() };
        (t.items.iter())
            .filter_map(|i| match i.kind {
                toolbar::Kind::Btn(a) => Some((chrome::act_icon(a), i.r)),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod blur_view_tests {
    use super::*;

    /// A drafted blur in the window is the image's blur scaled: at 2x
    /// every window pixel is its image pixel of the export's bake, and a
    /// part in view draws what the whole does there.
    #[test]
    fn a_blur_in_the_window_is_the_exports() {
        let (w, h) = (120u32, 90u32);
        let mut src = crate::pixbuf::PixBuf::new(w, h);
        for (i, p) in src.as_raw_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *p = [(i * 37 % 251) as u8, (i * 11 % 241) as u8, (i % 7 * 30) as u8, 255];
        }
        let r = FRect { x: 10.0, y: 8.0, w: 80.0, h: 60.0 };
        let mut baked = src.clone();
        Obj::Blur { r, sigma: 30.0 }.render(&mut baked, None);
        let v = View { z: 2.0, ox: -20.0, oy: -10.0 };
        let (ww, wh) = (200u32, 150u32);
        let paint = |clip: FRect| {
            let mut d = vec![0u8; (ww * wh * 4) as usize];
            paint_blur(&mut crate::raster::Surf::with_origin(&mut d, ww, wh, 0, 0, crate::raster::Order::Bgra), &src, r, 30.0, &v, clip);
            d
        };
        let full = paint(FRect { x: 0.0, y: 0.0, w: ww as f32, h: wh as f32 });
        let part = paint(FRect { x: 60.0, y: 40.0, w: 50.0, h: 50.0 });
        let mut n = 0;
        for y in 0..wh as i32 {
            for x in 0..ww as i32 {
                let p = v.to_img(Pt::new(x as f32 + 0.5, y as f32 + 0.5));
                let (ix, iy) = (p.x.floor() as i32, p.y.floor() as i32);
                let i = ((y * ww as i32 + x) * 4) as usize;
                if (10..90).contains(&ix) && (8..68).contains(&iy) {
                    let want = baked.get_pixel(ix as u32, iy as u32);
                    assert_eq!([full[i + 2], full[i + 1], full[i], full[i + 3]], want, "({x}, {y})");
                    n += 1;
                }
                if (60..110).contains(&x) && (40..90).contains(&y) {
                    assert_eq!(part[i..i + 4], full[i..i + 4], "({x}, {y}) in view");
                }
            }
        }
        assert!(n > 1000);
    }
}
