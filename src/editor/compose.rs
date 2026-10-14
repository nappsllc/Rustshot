//! Rect-based composition of the overlay. `Edit::prepare` resolves one
//! frame's state (layout, tweens, toast, caret) into a [`Scene`];
//! `Edit::compose_rect` renders any rect of that scene, byte-exact with a
//! whole-image compose, from a [`Backdrop`] (the capture, plain or dimmed)
//! plus the draft object and the chrome drawn through offset surfaces.
//! `dirty_rects` diffs two scenes into the rects whose pixels may differ.

use super::assist::{self, CrossUi, HintUi, MagUi, RingUi};
use super::chrome::{self, BarState, ToastKind, Ui};
use super::toolbar::{self, Act, Toolbar};
use super::{animating, caret_on, palette_colors, pick_area, text_box_rect, tools, Dimmer, Edit, Interact, Toast, Tool};
use crate::anim;
use crate::objects::{CellGrid, FRect, Obj, Pt};
use crate::pixbuf::PixBuf;
use crate::raster::{Order, Surf};
use crate::theme::Theme;
use crate::uifb::{Fb, C4};
use std::sync::Arc;
use std::time::Instant;

/// Pixel rect in image coordinates, end-exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PxRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl PxRect {
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        PxRect { x0, y0, x1, y1 }
    }

    /// The whole `w` x `h` image.
    pub fn image(size: (u32, u32)) -> Self {
        PxRect::new(0, 0, size.0 as i32, size.1 as i32)
    }

    /// Smallest pixel rect containing `r` (floor / ceil).
    pub fn outer(r: FRect) -> Self {
        PxRect::new(r.x.floor() as i32, r.y.floor() as i32, r.x1().ceil() as i32, r.y1().ceil() as i32)
    }

    pub fn w(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    pub fn h(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn intersect(&self, o: &PxRect) -> PxRect {
        PxRect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }

    pub fn intersects(&self, o: &PxRect) -> bool {
        !self.intersect(o).is_empty()
    }

    pub fn contains(&self, o: &PxRect) -> bool {
        o.is_empty() || (self.x0 <= o.x0 && self.y0 <= o.y0 && o.x1 <= self.x1 && o.y1 <= self.y1)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn contains_px(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// `self` minus `o` as up to four disjoint rects.
    pub fn minus(&self, o: &PxRect) -> Vec<PxRect> {
        self.minus_iter(o).collect()
    }

    /// [`PxRect::minus`] without allocating.
    pub fn minus_iter(&self, o: &PxRect) -> impl Iterator<Item = PxRect> + use<> {
        let i = self.intersect(o);
        let parts = if i.is_empty() {
            [*self, PxRect::default(), PxRect::default(), PxRect::default()]
        } else {
            [
                PxRect::new(self.x0, self.y0, self.x1, i.y0),
                PxRect::new(self.x0, i.y1, self.x1, self.y1),
                PxRect::new(self.x0, i.y0, i.x0, i.y1),
                PxRect::new(i.x1, i.y0, self.x1, i.y1),
            ]
        };
        parts.into_iter().filter(|r| !r.is_empty())
    }

    /// Smallest rect containing both (an empty one contributes nothing).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn union(&self, o: &PxRect) -> PxRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        PxRect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }
}

/// What a backdrop rect shows: the capture as is, or blended toward the
/// theme dim color by `alpha`, `times` over (two only where the dim
/// rects of a fractional selection overlap).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Plain,
    Dimmed { alpha: u8, times: u8 },
}

/// The pixels under the chrome: the capture (with committed objects).
pub trait Backdrop {
    /// Write rect `r` (inside the image) into `out`: row `i` starts at byte
    /// `i * stride * 4`, `r.w()` pixels in `order`.
    fn fill(&self, r: PxRect, src: Source, out: &mut [u8], stride: usize, order: Order);
}

/// In-memory backdrop (Linux/macOS, tests): `composed` plus the dim color.
pub struct PixBufBackdrop<'a> {
    pub img: &'a PixBuf,
    /// Theme dim color; its alpha is ignored (`Source` carries it).
    pub dim: C4,
}

impl Backdrop for PixBufBackdrop<'_> {
    fn fill(&self, r: PxRect, src: Source, out: &mut [u8], stride: usize, order: Order) {
        let iw = self.img.width() as usize;
        let w = r.w() as usize * 4;
        let raw = self.img.as_raw();
        let dimmer = match src {
            Source::Plain => None,
            Source::Dimmed { alpha, times } => Some((Dimmer::new(self.dim.with_alpha(alpha)), times as i32)),
        };
        for (i, y) in (r.y0..r.y1).enumerate() {
            let s = &raw[(y as usize * iw + r.x0 as usize) * 4..][..w];
            let d = &mut out[i * stride * 4..][..w];
            match &dimmer {
                None => d.copy_from_slice(s),
                Some((dm, times)) => dm.run(s, d, *times),
            }
            if order == Order::Bgra {
                for px in d.as_chunks_mut::<4>().0 {
                    px.swap(0, 2);
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct SelUi {
    pub sr: FRect,
    pub hot: Option<usize>,
    /// Settings › Capture "Add a border": its preview around the
    /// selection (width in image px, color); not part of the capture.
    pub border: Option<(f32, C4)>,
}

/// Window snapping: the window (or part) under the pointer, outlined with
/// its size badge (and the Tab hint).
#[derive(Clone, PartialEq, Debug)]
pub struct SnapUi {
    pub r: FRect,
    pub hint: bool,
    pub area: FRect,
}

#[derive(Clone, PartialEq, Debug)]
pub struct LabelUi {
    pub sr: FRect,
    pub show_pos: bool,
    pub area: FRect,
    pub avoid: Option<FRect>,
    /// The mode switch the label follows.
    pub lead: Option<FRect>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct TextUi {
    pub boxr: FRect,
    pub pos: Pt,
    pub text: String,
    pub px: f32,
    pub color: C4,
    /// Byte index of the caret while the blink phase shows it.
    pub caret: Option<usize>,
    /// The text tool's style.
    pub fmt: crate::text::TextFmt,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ToastUi {
    pub text: String,
    pub kind: ToastKind,
    pub area: FRect,
    pub k: f32,
}

#[derive(Clone, PartialEq)]
pub struct BarUi {
    /// The toolbar as drawn (scaled during the fade-in).
    pub tb: Toolbar,
    pub k: f32,
    pub hover: Option<usize>,
    pub hover_k: f32,
    pub pressed: Option<usize>,
    pub tool: Option<super::Tool>,
    pub color: C4,
    pub value: String,
    /// The tool and size the readout previews.
    pub ptool: Tool,
    pub size: f32,
    /// The fill toggle is on (the current tool draws filled).
    pub fill: bool,
    pub pop_k: f32,
    pub pick: Option<super::pick::Picker>,
}

/// The eyedropper's loupe: where, and the pixels it magnifies.
#[derive(Clone, PartialEq, Debug)]
pub struct LoupeUi {
    pub r: FRect,
    pub px: Vec<C4>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct TipUi {
    pub anchor: FRect,
    pub act: Act,
    pub label: &'static str,
    /// Key caps of the first chord bound to `act`.
    pub keys: Vec<String>,
    pub area: FRect,
    /// Below the anchor (the Camera row's: never over the rows above).
    pub below: bool,
    pub k: f32,
}

impl TipUi {
    fn at(&self) -> chrome::TipAt {
        chrome::TipAt { anchor: self.anchor, area: self.area, below: self.below }
    }
}

/// The selected objects' chrome: dashed boxes (one per object, or the
/// group's), a dashed line along a selected line / arrow, square and round
/// handles.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ObjSelUi {
    pub boxes: Vec<FRect>,
    pub line: Option<(Pt, Pt)>,
    pub square: Vec<Pt>,
    pub round: Vec<Pt>,
    /// The handle under the pointer: (round?, index).
    pub hot: Option<(bool, usize)>,
}

/// The callout caret: its rect and color.
#[derive(Clone, PartialEq, Debug)]
pub struct CaretUi {
    pub r: FRect,
    pub color: C4,
}

/// Chrome elements in paint order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum El {
    Draft,
    Ghost,
    /// Text mode, the lasso, the crop frame (`snip`).
    Snip,
    /// The magnifier's crosshair (under the selection chrome and the bar).
    Cross,
    Sel,
    ObjSel,
    Guides,
    Marquee,
    Caret,
    Snap,
    Label,
    Text,
    Toast,
    Bar,
    Mini,
    Tip,
    Hint,
    Mag,
    Loupe,
    Ring,
}

const N_EL: usize = 20;
const ELS: [El; N_EL] = [
    El::Draft,
    El::Ghost,
    El::Snip,
    El::Cross,
    El::Sel,
    El::ObjSel,
    El::Guides,
    El::Marquee,
    El::Caret,
    El::Snap,
    El::Label,
    El::Text,
    El::Toast,
    El::Bar,
    El::Mini,
    El::Tip,
    El::Hint,
    El::Mag,
    El::Loupe,
    El::Ring,
];

/// Everything one frame paints, resolved at one instant. Two scenes
/// compare element by element (`dirty_rects`); a clone is the "painted
/// state" a presenter keeps.
#[derive(Clone, PartialEq)]
pub struct Scene {
    pub size: (u32, u32),
    pub s: f32,
    pub th: Theme,
    /// Dim alpha this frame (0 while the fade-in starts).
    pub dim_alpha: u8,
    /// The selection as the backdrop sees it (unrounded); while snapping
    /// to windows, the window outlined.
    pub sel: Option<FRect>,
    pub objects: Arc<Vec<Obj>>,
    pub draft: Option<Obj>,
    pub sel_ui: Option<SelUi>,
    pub snap: Option<SnapUi>,
    pub label: Option<LabelUi>,
    pub text: Option<TextUi>,
    pub toast: Option<ToastUi>,
    pub bar: Option<BarUi>,
    pub tip: Option<TipUi>,
    /// The first-open hint card.
    pub hint: Option<HintUi>,
    pub loupe: Option<LoupeUi>,
    /// The magnifier and its crosshair while a region is picked.
    pub cross: Option<CrossUi>,
    pub mag: Option<MagUi>,
    /// The right-click color ring.
    pub ring: Option<RingUi>,
    /// The drag's origin at 40 % (the objects as they were).
    pub ghost: Vec<Obj>,
    pub osel: Option<ObjSelUi>,
    /// Snap guides: 1 px accent lines, at this opacity (fading out for
    /// 100 ms after a drag).
    pub guides: Vec<(Pt, Pt)>,
    pub guides_k: f32,
    pub marquee: Option<FRect>,
    pub caret: Option<CaretUi>,
    pub mini: Option<super::mini::MiniUi>,
    pub snip: Option<super::snip::SnipUi>,
    /// Per element (index = `El as usize`): conservative pixel bounds.
    bounds: [Vec<PxRect>; N_EL],
}

impl Scene {
    pub(super) fn rects(&self, e: El) -> &[PxRect] {
        &self.bounds[e as usize]
    }

    fn same(&self, o: &Scene, e: El) -> bool {
        match e {
            El::Draft => self.draft == o.draft,
            El::Ghost => self.ghost == o.ghost,
            El::Snip => self.snip == o.snip,
            El::ObjSel => self.osel == o.osel,
            El::Guides => self.guides == o.guides && self.guides_k == o.guides_k,
            El::Marquee => self.marquee == o.marquee,
            El::Caret => self.caret == o.caret,
            El::Mini => self.mini == o.mini,
            El::Sel => self.sel_ui == o.sel_ui,
            El::Snap => self.snap == o.snap,
            El::Label => self.label == o.label,
            El::Text => self.text == o.text,
            El::Toast => self.toast == o.toast,
            El::Bar => self.bar == o.bar,
            El::Tip => self.tip == o.tip,
            El::Hint => self.hint == o.hint,
            El::Loupe => self.loupe == o.loupe,
            El::Cross => self.cross == o.cross,
            El::Mag => self.mag == o.mag,
            El::Ring => self.ring == o.ring,
        }
    }

    /// Every chrome (and draft) bound, clipped to the image.
    pub fn chrome_rects(&self) -> Vec<PxRect> {
        let mut v = Vec::new();
        self.chrome_rects_into(&mut v);
        v
    }

    /// [`Scene::chrome_rects`] appended to `out`.
    pub fn chrome_rects_into(&self, out: &mut Vec<PxRect>) {
        let img = PxRect::image(self.size);
        out.extend(self.bounds.iter().flatten().map(|r| r.intersect(&img)).filter(|r| !r.is_empty()));
    }

    /// The pixelate draft's cell grid, if the draft is a pixelate.
    pub fn draft_grid(&self) -> Option<CellGrid> {
        self.draft.as_ref().and_then(Obj::pixel_grid)
    }
}

/// Dim spans around the selection, snapped outward exactly like the
/// former whole-image pass (fractional edges overlap and dim twice).
/// Up to four; the count is returned with them.
fn dim_spans(sel: Option<FRect>, size: (u32, u32)) -> ([PxRect; 4], usize) {
    let (ww, wh) = (size.0 as f32, size.1 as f32);
    let (bw, bh) = (size.0 as i32, size.1 as i32);
    let none = (0.0, 0.0, 0.0, 0.0);
    let rects: [(f32, f32, f32, f32); 4] = match sel {
        None => [(0.0, 0.0, ww, wh), none, none, none],
        Some(sr) => [
            (0.0, 0.0, ww, sr.y),
            (0.0, sr.y1(), ww, wh - sr.y1()),
            (0.0, sr.y, sr.x, sr.h),
            (sr.x1(), sr.y, ww - sr.x1(), sr.h),
        ],
    };
    let mut out = [PxRect::default(); 4];
    let mut n = 0;
    for (x, y, w, h) in rects {
        if !(w > 0.0 && h > 0.0) {
            continue;
        }
        let x0 = x.floor().max(0.0) as i32;
        let y0 = y.floor().max(0.0) as i32;
        let x1 = ((x + w).ceil().min(bw as f32) as i32).min(bw);
        let y1 = ((y + h).ceil().min(bh as f32) as i32).min(bh);
        let r = PxRect::new(x0, y0, x1, y1);
        if !r.is_empty() {
            out[n] = r;
            n += 1;
        }
    }
    (out, n)
}

/// Backdrop of `r` into `out` (stride `r.w()`): plain inside the
/// selection, dimmed outside. Split into bands of rows crossed by the same
/// spans, so a backdrop gets a handful of rect fills, not one per row.
pub(super) fn compose_bg(sc: &Scene, r: PxRect, bd: &dyn Backdrop, out: &mut [u8], order: Order) {
    backdrop_rect(sc.sel, sc.size, sc.dim_alpha, r, bd, out, order);
}

/// `compose_bg` for a selection, image size and dim alpha.
pub(super) fn backdrop_rect(
    sel: Option<FRect>,
    size: (u32, u32),
    alpha: u8,
    r: PxRect,
    bd: &dyn Backdrop,
    out: &mut [u8],
    order: Order,
) {
    let stride = r.w() as usize;
    let mut runs = Vec::new();
    backdrop_runs_into(sel, size, alpha, r, &mut runs);
    for (q, src) in runs {
        let off = ((q.y0 - r.y0) as usize * stride + (q.x0 - r.x0) as usize) * 4;
        bd.fill(q, src, &mut out[off..], stride, order);
    }
}

/// Rect `r` split into disjoint runs that each show one [`Source`]: bands
/// of rows crossed by the same dim spans, cut where the dim depth changes.
/// A handful of runs per rect (the GDI overlay blits each one), appended
/// to `runs` (no other allocation).
pub(super) fn backdrop_runs_into(
    sel: Option<FRect>,
    size: (u32, u32),
    alpha: u8,
    r: PxRect,
    runs: &mut Vec<(PxRect, Source)>,
) {
    if r.is_empty() {
        return;
    }
    let (all, n) = if alpha == 0 { ([PxRect::default(); 4], 0) } else { dim_spans(sel, size) };
    let mut spans = [PxRect::default(); 4];
    let mut ns = 0;
    for s in all[..n].iter().filter(|s| s.intersects(&r)) {
        spans[ns] = *s;
        ns += 1;
    }
    let spans = &spans[..ns];
    if spans.is_empty() {
        runs.push((r, Source::Plain));
        return;
    }
    let mut ys = [0i32; 10];
    ys[0] = r.y0;
    ys[1] = r.y1;
    let mut ny = 2;
    for s in spans {
        for y in [s.y0, s.y1] {
            if y > r.y0 && y < r.y1 {
                ys[ny] = y;
                ny += 1;
            }
        }
    }
    let ys = &mut ys[..ny];
    crate::sort::sort(ys);
    let mut nu = 0;
    for i in 0..ys.len() {
        if i == 0 || ys[i] != ys[nu - 1] {
            ys[nu] = ys[i];
            nu += 1;
        }
    }
    for band in ys[..nu].windows(2) {
        let (ya, yb) = (band[0], band[1]);
        let mut edges = [(0i32, 0i32); 9];
        let mut ne = 0;
        for s in spans.iter().filter(|s| s.y0 <= ya && ya < s.y1) {
            edges[ne] = (s.x0.clamp(r.x0, r.x1), 1);
            edges[ne + 1] = (s.x1.clamp(r.x0, r.x1), -1);
            ne += 2;
        }
        edges[ne] = (r.x1, 0);
        ne += 1;
        let edges = &mut edges[..ne];
        crate::sort::sort(edges);
        let (mut x, mut depth) = (r.x0, 0i32);
        for &(ex, de) in edges.iter() {
            if ex > x {
                let src = if depth == 0 {
                    Source::Plain
                } else {
                    Source::Dimmed { alpha, times: depth as u8 }
                };
                runs.push((PxRect::new(x, ya, ex, yb), src));
                x = ex;
            }
            depth += de;
        }
    }
}

/// Coalesce rects: two merge when their bounding box is no larger than
/// the two areas summed (so merging never paints more). Order-insensitive
/// enough for invalidation; the result covers every input rect.
pub fn merge_rects(mut v: Vec<PxRect>) -> Vec<PxRect> {
    v.retain(|r| !r.is_empty());
    let area = |r: &PxRect| r.w() as i64 * r.h() as i64;
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < v.len() {
            let mut j = i + 1;
            while j < v.len() {
                let (a, b) = (v[i], v[j]);
                let u = PxRect::new(a.x0.min(b.x0), a.y0.min(b.y0), a.x1.max(b.x1), a.y1.max(b.y1));
                if b.contains(&a) || a.contains(&b) || area(&u) <= area(&a) + area(&b) {
                    v[i] = u;
                    v.swap_remove(j);
                    changed = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }
    v
}

fn px_bounds(r: FRect) -> PxRect {
    PxRect::outer(r)
}

/// `r` grown to whole cells of `g` where it cuts the pixelated area.
fn cell_align(r: PxRect, g: &CellGrid, size: (u32, u32)) -> PxRect {
    let (x0, y0, x1, y1) = g.align((r.x0, r.y0, r.x1, r.y1), size.0 as i32, size.1 as i32);
    PxRect::new(x0, y0, x1, y1)
}

impl Edit {
    /// Resolve this frame: toolbar layout, tweens, area, toast, caret,
    /// draft; the result is `self.scene`. Also records what is drawn for
    /// `stale()` and rebuilds `composed` when objects changed.
    pub(super) fn prepare(&mut self, app_notice: Option<&Toast>, mouse: (i32, i32), now: Instant) {
        self.slot_long_press(now);
        self.copy_long_press(now);
        if self.dirty || self.spots_stale() {
            let _p = crate::perf::span(crate::perf::K::Bake);
            self.rebuild();
        }
        let (ww, _) = (self.shot.size.0 as f32, self.shot.size.1 as f32);
        let area = pick_area(&self.shot.monitors, self.sel, Pt::new(mouse.0 as f32, mouse.1 as f32));
        self.area_drawn = Some(area);
        let s = self.ui_scale();
        let colors = palette_colors(&self.cfg);
        let rp = &self.rec;
        let names = rp.dev_open.map(|d| (d, rp.dev_names(d)));
        let mic_label = rp.mic_label();
        let cam_label = rp.cam_label();
        let rv = toolbar::RecView {
            format: rp.opts.format,
            mp4: rp.mp4,
            fps: rp.opts.fps(),
            fps_choices: rp.opts.fps_choices(),
            system: rp.opts.system,
            mic: rp.opts.mic,
            camera: rp.opts.camera,
            cams: rp.has_cams(),
            cams_looking: rp.cams_looking(),
            shape: rp.opts.cam_shape,
            bg: rp.opts.cam_bg,
            blur: crate::cam::bg::available(),
            source: rp.source,
            countdown: rp.opts.countdown,
            gif_max: rp.opts.gif_max,
            meter: rp.meter.as_ref().map(|_| rp.bars),
            mic_label: &mic_label,
            cam_label: &cam_label,
            devs: names.as_ref().map(|(d, n)| (*d, n.as_slice())),
            dev_sel: rp.dev_open.map_or(0, |d| rp.dev_sel(d)),
            ring: rp.opts.cam_ring,
            ring_w: rp.opts.cam_ring_w,
            ring_pal: rp.ring_pal,
            ring_colors: &colors,
            accent: self.th.accent,
        };
        let cur = self.cur_tool();
        let recents = self.recents();
        let pv = self.pick.as_ref().map(|p| toolbar::PickView {
            ring: p.target == super::pick::Target::Ring,
            alpha: p.has_alpha(),
            recents: &recents,
        });
        let busy = !self.tasks.is_empty();
        let save_keys: Vec<String> = toolbar::save_menu().iter().map(|&(a, _)| chrome::act_tip(a, &self.keys).1.join("+")).collect();
        let slot_keys = toolbar::SLOT_MENU.map(|(a, _)| chrome::act_tip(a, &self.keys).1.join("+"));
        let copy_menu = self.copy_menu_view();
        if self.pick.is_none() {
            self.assist.pick_at = None;
        }
        // Hidden while a new selection is drawn (it fades in on release):
        // following the drag it would be redrawn on every move.
        let bar_sel = self.sel.filter(|_| !matches!(self.interact, Interact::NewSel { .. }));
        let standalone = match (self.sel, self.assist.pick_at, pv.as_ref()) {
            (None, Some(at), Some(pv)) => Some(toolbar::layout_picker(pv, at, area, s)),
            _ => None,
        };
        self.toolbar = bar_sel.map(|sel| {
            toolbar::layout(&toolbar::Input {
                sel,
                area,
                s,
                busy,
                can_undo: self.hi > 0,
                can_redo: self.hi + 1 < self.hist.len(),
                palette: self.palette_open.then_some(colors.as_slice()),
                record: crate::rec_ui::supported() && !busy,
                rec: (rp.open && crate::rec_ui::supported() && !busy).then_some(&rv),
                grip: rp.grip.filter(|_| rp.open),
                // Not while the selection is dragged (it would repaint every move).
                top: !matches!(self.interact, Interact::MoveSel { .. } | Interact::Resize { .. } | Interact::NewSel { .. }),
                sizes: tools::has_size(cur),
                colors: tools::has_color(cur),
                fills: tools::has_fill(cur),
                pick: pv.as_ref(),
                save_menu: self.save_menu.then_some(save_keys.as_slice()),
                slot_menu: self.slot_menu.then_some(&slot_keys),
                copy_menu: copy_menu.as_ref(),
                freeform: (!busy && self.host.is_none()).then_some(self.snip.freeform),
            })
        }).or(standalone);

        let interacting = !matches!(self.interact, Interact::None);
        if self.toolbar.is_some() {
            self.mo.bar.set(1.0, 120, now);
        } else {
            self.mo.bar.snap(0.0);
        }
        if self.palette_open || self.pick.is_some() || self.save_menu || self.slot_menu || self.snip.copy_menu || self.size_menu {
            self.mo.pop.set(1.0, 100, now);
        } else {
            self.mo.pop.snap(0.0);
        }
        let hint_on = self.sel.is_none() && !interacting && self.assist.hint_live(now);
        self.mo.hint.set(if hint_on { 1.0 } else { 0.0 }, if hint_on { 120 } else { assist::HINT_FADE_MS }, now);

        let dim_a = self.th.dim_alpha(self.cfg.contrast_opacity) as f32 * self.mo.dim.value(now);
        let objects = match &self.scene {
            Some(sc) if *sc.objects == self.objects => sc.objects.clone(),
            _ => Arc::new(self.objects.clone()),
        };

        let border = crate::export::border(&self.cfg, self.shot.scale).map(|(b, c)| (b as f32, C4::rgb(c[0], c[1], c[2])));
        // The crop frame has its own handles (not two sets at once).
        let sel_ui = self
            .sel
            .filter(|_| self.snip.crop_mode.is_none())
            .map(|sr| SelUi { sr, hot: self.hot_handle, border: border.filter(|_| self.host.is_none() && !self.rec.open) });
        let snap_r = self.snap_rect();
        let snap = snap_r.map(|r| SnapUi { r, hint: self.cfg.snap_tab_hint && self.hover_win.has_parts(), area });
        let label = self.sel.map(|sr| {
            let avoid = self
                .toolbar
                .as_ref()
                .filter(|t| t.above && t.bar.y1() <= sr.y)
                .map(|t| t.pop.map_or(t.bar, |p| toolbar::union(t.bar, p)));
            let lead = self.toolbar.as_ref().and_then(|t| t.mode);
            // Following the mode switch it keeps off the Close button.
            let close = self.toolbar.as_ref().and_then(|t| t.items.iter().find(|i| i.kind == toolbar::Kind::Close)).map(|i| i.r);
            let avoid = if lead.is_some() { close } else { avoid };
            LabelUi { sr, show_pos: !interacting, area, avoid, lead }
        });
        // The label's menu (Trim to content) hangs off it, part of the
        // toolbar's popover.
        self.label_at = label.as_ref().map(|l| {
            let ui = Ui { th: &self.th, s, font: self.ui_font };
            chrome::size_label_rect(&ui, l.sr, l.show_pos, l.area, (l.avoid, l.lead))
        });
        // A menu that cannot open (video mode, another popover) is not
        // left open to swallow the next press.
        if self.size_menu {
            let keys: Vec<String> = toolbar::size_menu().iter().map(|(a, _)| chrome::act_tip(*a, &self.keys).1.join("+")).collect();
            self.size_menu = self.host.is_none()
                && !self.rec.open
                && match (self.toolbar.as_mut(), self.label_at) {
                    (Some(tb), Some(at)) => toolbar::add_size_menu(tb, at, &keys, area, s),
                    _ => false,
                };
        }
        let tfont = self.text_font();
        let text = self.text.as_ref().map(|td| TextUi {
            boxr: text_box_rect(ww, td, self.mem.size(Tool::Text), tfont.as_ref(), s),
            pos: td.pos,
            text: td.text.clone(),
            px: self.mem.size(Tool::Text),
            color: self.mem.color(Tool::Text),
            caret: caret_on(td).then_some(td.caret),
            fmt: self.mem.fmt(Tool::Text),
        });
        let toast = self.notice.as_ref().or(app_notice).map(|t| ToastUi {
            text: t.text.clone(),
            kind: t.kind,
            area,
            k: t.opacity(now),
        });
        // Video mode with a selection: the estimate ("1080p · 30 fps · est.
        // 2.1 MB / min") while no other toast is up.
        let toast = toast.or_else(|| {
            let sel = self.sel.filter(|_| self.rec.open && crate::rec_ui::supported() && self.tasks.is_empty())?;
            let q = crate::rec::Quality::from_config(&self.cfg.rec_quality);
            let text = self.rec.opts.estimate((sel.w.round().max(2.0) as u32, sel.h.round().max(2.0) as u32), q);
            // Over the panel it goes to the top of the monitor instead.
            let ui = Ui { th: &self.th, s, font: self.ui_font };
            let r = chrome::toast_panel(&ui, &text, area, 1.0);
            let clash = self.toolbar.as_ref().is_some_and(|t| r.x < t.bar.x1() && t.bar.x < r.x1() && r.y < t.bar.y1() && t.bar.y < r.y1());
            let area = if clash { FRect { h: (12.0 + 36.0 + 28.0) * s, ..area } } else { area };
            Some(ToastUi { text, kind: ToastKind::Info, area, k: 1.0 })
        });
        let mut tip = None;
        let looking = self.rec.cams_looking();
        let bar = self.toolbar.as_ref().map(|tb| {
            let kb = self.mo.bar.value(now);
            let value = self.size_label();
            let shown_ms = now.saturating_duration_since(self.hover_at).as_secs_f32() * 1000.0 - 400.0;
            if !interacting
                && shown_ms >= 0.0
                && let Some(it) = self.hover.and_then(|i| tb.items.get(i))
            {
                let k = anim::ease_out(shown_ms / 80.0);
                // A control of a popover below the bar: its tip goes below
                // it (above, it would cover the bar).
                let c = Pt::new(it.r.x + it.r.w / 2.0, it.r.y + it.r.h / 2.0);
                let pop_below = tb.pop.is_some_and(|p| toolbar::hit(p, c) && p.y >= tb.bar.y1() - 0.5);
                match it.kind {
                    toolbar::Kind::Btn(act) | toolbar::Kind::Rainbow(act) | toolbar::Kind::Swatch { act: act @ toolbar::Act::RingPalette, .. } => {
                        let (label, keys) = chrome::act_tip(toolbar::slot_act(act, self.tool), &self.keys);
                        tip = Some(TipUi { anchor: it.r, act, label, keys, area, below: pop_below, k });
                    }
                    // Why the Camera row does nothing: below its switch,
                    // clear of the Microphone row above.
                    toolbar::Kind::Switch { act: act @ toolbar::Act::RecCamera, .. } if it.disabled && !looking => {
                        let sw = 40.0 * s;
                        let anchor = FRect { x: it.r.x1() - sw, y: it.r.y, w: sw, h: it.r.h };
                        tip = Some(TipUi { anchor, act, label: crate::cam::NO_CAMERA, keys: Vec::new(), area, below: true, k });
                    }
                    // Why MP4 is off (Linux without ffmpeg, or still asking it).
                    toolbar::Kind::Seg { act: act @ toolbar::Act::RecFormat(crate::rec::RecFormat::Mp4), .. } if it.disabled => {
                        let label = if self.rec.mp4 == crate::rec_ui::Mp4::Checking { crate::rec_ui::CHECKING_MP4 } else { crate::rec_ui::NO_MP4 };
                        tip = Some(TipUi { anchor: it.r, act, label, keys: Vec::new(), area, below: pop_below, k });
                    }
                    _ => {}
                }
            }
            BarUi {
                tb: tb.scaled(0.96 + 0.04 * kb),
                k: kb,
                hover: self.hover,
                hover_k: self.mo.hover.value(now),
                pressed: self.pressed,
                tool: self.tool,
                color: self.color(),
                value,
                ptool: cur,
                size: self.mem.size(cur),
                fill: self.mem.fill(cur),
                pop_k: self.mo.pop.value(now),
                pick: self.pick.clone(),
            }
        });
        let kh = self.mo.hint.value(now);
        let hint = (kh > 0.01).then(|| {
            let snap = self.snap.is_some() && self.tool.is_none();
            let parts = match &self.assist.hint_parts {
                Some((sn, p)) if *sn == snap => p.clone(),
                _ => {
                    let p = assist::hint_parts(&self.keys, snap);
                    self.assist.hint_parts = Some((snap, p.clone()));
                    p
                }
            };
            // Freeform: the lasso is what a drag draws.
            let mut parts = parts;
            if self.snip.freeform
                && let Some(first) = parts.first_mut()
            {
                *first = chrome::HintPart::T(if snap { "Click a window or drag a lasso" } else { "Drag a lasso to select" }.into());
            }
            HintUi { area, k: kh, parts }
        });
        let label_r: Vec<FRect> = {
            let ui = Ui { th: &self.th, s, font: self.ui_font };
            label.iter().map(|l| chrome::size_label_bounds(&ui, l.sr, l.show_pos, l.area, (l.avoid, l.lead))).collect()
        };
        let (cross, mag, ring) = self.assist_scene(Pt::new(mouse.0 as f32, mouse.1 as f32), s, &label_r);
        // The eyedropper's loupe follows the pointer, off the toolbar.
        let on_chrome = |p: Pt| self.toolbar.as_ref().is_some_and(|t| t.contains(p));
        let loupe = match (self.pick.as_ref().filter(|p| p.eyedrop), self.loupe) {
            (Some(_), Some(p)) if !on_chrome(p) => {
                let ui = Ui { th: &self.th, s, font: self.ui_font };
                // The monitor under the pointer (the eyedropper can pick on
                // any monitor, not only the selection's), clear of the bar.
                let mon = super::monitor_at(&self.shot.monitors, p).unwrap_or(area);
                let avoid: Vec<FRect> = self.toolbar.iter().flat_map(|t| std::iter::once(t.bar).chain(t.pop)).collect();
                Some(LoupeUi { r: chrome::loupe_rect(&ui, p, mon, &avoid), px: self.loupe_pixels(p) })
            }
            _ => None,
        };

        let (ghost, osel, marquee, caret, mini) = self.obj_scene(area, s);
        let snip = self.snip_scene(area, s, dim_a.round() as u8, Pt::new(mouse.0 as f32, mouse.1 as f32));
        let (guides, guides_k) = self.guides_now(now);
        // The save menu open: no tip over it from its own chevron.
        if self.save_menu {
            tip = tip.filter(|t| t.act != Act::SaveMenu);
        }
        // A tip for the mini-toolbar's buttons too.
        if tip.is_none()
            && !interacting
            && let Some(m) = &mini
            && let Some(it) = m.hover.and_then(|i| m.items.get(i))
            && let toolbar::Kind::Btn(act) = it.kind
        {
            let shown_ms = now.saturating_duration_since(self.hover_at).as_secs_f32() * 1000.0 - 400.0;
            if shown_ms >= 0.0 {
                let (label, keys) = chrome::act_tip(act, &self.keys);
                tip = Some(TipUi { anchor: it.r, act, label, keys, area, below: false, k: anim::ease_out(shown_ms / 80.0) });
            }
        }
        let mut sc = Scene {
            ghost,
            osel,
            guides,
            guides_k,
            marquee,
            caret,
            mini,
            snip,
            size: self.shot.size,
            s,
            th: self.th,
            dim_alpha: dim_a.round() as u8,
            sel: self.sel.or(snap_r),
            objects,
            draft: self.draft.clone(),
            sel_ui,
            snap,
            label,
            text,
            toast,
            bar,
            tip,
            hint,
            loupe,
            cross,
            mag,
            ring,
            bounds: Default::default(),
        };
        sc.bounds = ELS.map(|e| self.el_bounds(&sc, e));
        if !matches!(sc.draft, Some(Obj::Pixelate { .. } | Obj::Blur { .. })) && self.draft_buf.borrow().capacity() > 0 {
            // The drag ended: release the pixelate scratch.
            *self.draft_buf.borrow_mut() = Vec::new();
        }
        self.caret_drawn = self.text.as_ref().map(caret_on).or_else(|| self.cedit_caret_on());
        self.toast_drawn = self.notice.as_ref().or(app_notice).map(|t| t.at);
        self.anim_drawn = animating(self, app_notice, now);
        self.scene = Some(sc);
    }

    fn ui<'a>(&'a self, sc: &'a Scene) -> Ui<'a> {
        Ui { th: &sc.th, s: sc.s, font: self.ui_font }
    }

    fn el_bounds(&self, sc: &Scene, e: El) -> Vec<PxRect> {
        let ui = self.ui(sc);
        let one = |r: FRect| vec![px_bounds(r)];
        match e {
            El::Draft => sc.draft.as_ref().and_then(|d| d.bounds(self.font.as_ref())).map_or(Vec::new(), one),
            El::Ghost => sc.ghost.iter().filter_map(|o| o.bounds(self.font.as_ref())).map(px_bounds).collect(),
            El::Snip => sc.snip.as_ref().map_or(Vec::new(), |u| super::snip::bounds(&ui, u).into_iter().map(px_bounds).collect()),
            El::ObjSel => sc.osel.as_ref().map_or(Vec::new(), |u| super::mini::osel_bounds(&ui, u).into_iter().map(px_bounds).collect()),
            El::Guides => super::mini::guides_bounds(&ui, &sc.guides).into_iter().map(px_bounds).collect(),
            El::Marquee => sc.marquee.map_or(Vec::new(), |r| one(super::mini::marquee_bounds(r))),
            El::Caret => sc.caret.as_ref().map_or(Vec::new(), |c| one(super::mini::caret_bounds(c))),
            El::Mini => sc.mini.as_ref().map_or(Vec::new(), |m| one(super::mini::bounds(&ui, m))),
            El::Sel => sc
                .sel_ui
                .as_ref()
                .map_or(Vec::new(), |u| {
                    let mut v = chrome::selection_bounds(&ui, u.sr);
                    if let Some((b, _)) = u.border {
                        v.extend(chrome::border_preview_bounds(&ui, u.sr, b));
                    }
                    v.into_iter().map(px_bounds).collect()
                }),
            El::Snap => sc.snap.as_ref().map_or(Vec::new(), |u| {
                let mut v: Vec<PxRect> = chrome::outline_bounds(&ui, u.r).into_iter().map(px_bounds).collect();
                v.push(px_bounds(chrome::snap_label_bounds(&ui, u.r, u.hint, u.area)));
                v
            }),
            El::Label => sc
                .label
                .as_ref()
                .map_or(Vec::new(), |l| one(chrome::size_label_bounds(&ui, l.sr, l.show_pos, l.area, (l.avoid, l.lead)))),
            El::Text => sc.text.as_ref().map_or(Vec::new(), |t| {
                let lw = sc.s.round().max(1.0);
                let m = lw / 2.0 + 2.0;
                let b = FRect { x: t.boxr.x - m, y: t.boxr.y - m, w: t.boxr.w + 2.0 * m, h: t.boxr.h + 2.0 * m };
                let (tw, th) = self.font.as_ref().map_or((0.0, 0.0), |f| {
                    let (w, h, _) = f.styled(t.fmt).measure(&t.text, t.px);
                    (w, h)
                });
                let g = t.px + 2.0;
                let txt = FRect { x: t.pos.x - g, y: t.pos.y - g, w: tw + 2.0 * g + 2.0 * sc.s, h: th.max(t.px * 1.15) + 2.0 * g };
                vec![px_bounds(b), px_bounds(txt)]
            }),
            El::Toast => sc.toast.as_ref().map_or(Vec::new(), |t| one(chrome::toast_bounds(&ui, &t.text, t.area, t.k))),
            El::Bar => sc.bar.as_ref().map_or(Vec::new(), |b| chrome::toolbar_bounds(&ui, &b.tb).into_iter().map(px_bounds).collect()),
            El::Tip => sc.tip.as_ref().map_or(Vec::new(), |t| {
                one(chrome::tooltip_bounds(&ui, t.at(), t.label, &t.keys))
            }),
            El::Hint => sc.hint.as_ref().map_or(Vec::new(), |h| one(chrome::hint_bounds(&ui, h.area, &h.parts))),
            El::Loupe => sc.loupe.as_ref().map_or(Vec::new(), |l| one(chrome::loupe_bounds(&ui, l.r))),
            El::Cross => sc.cross.as_ref().map_or(Vec::new(), |c| assist::cross_bounds(c, sc.size)),
            El::Mag => sc.mag.as_ref().map_or(Vec::new(), |m| assist::mag_bounds(&ui, m).into_iter().map(px_bounds).collect()),
            El::Ring => sc.ring.as_ref().map_or(Vec::new(), |r| one(assist::ring_bounds(&ui, r))),
        }
    }

    /// Conservative bounds of every chrome element and the draft object
    /// in the prepared scene.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn chrome_rects(&self) -> Vec<PxRect> {
        self.scene.as_ref().map_or(Vec::new(), Scene::chrome_rects)
    }

    /// Render rect `r` of the prepared scene into `out` (`r.w()` x `r.h()`
    /// pixels in `order`): exactly the pixels a whole-image compose has in
    /// `r`. `r` is clipped to the image; `out` must hold the clipped rect.
    pub fn compose_rect(&self, r: PxRect, bd: &dyn Backdrop, out: &mut [u8], order: Order) {
        let Some(sc) = &self.scene else { return };
        let img = PxRect::image(sc.size);
        let r = r.intersect(&img);
        if r.is_empty() {
            return;
        }
        let _p = crate::perf::span(crate::perf::K::Compose);
        let (w, h) = (r.w() as usize, r.h() as usize);
        let out = &mut out[..w * h * 4];
        let hits = |e: El| sc.rects(e).iter().any(|b| b.intersects(&r));
        // Pixelate averages whole cells: where `r` cuts through cells,
        // render over `r` grown to whole cells (a reused scratch buffer,
        // about the size of `r`), then copy `r` out.
        let grown = match &sc.draft {
            Some(d) if hits(El::Draft) && d.reads_far() => {
                d.bounds(self.font.as_ref()).map(|b| px_bounds(b).intersect(&img).union(&r)).filter(|e| *e != r)
            }
            Some(d) if hits(El::Draft) => d.pixel_grid().map(|g| cell_align(r, &g, sc.size)).filter(|e| *e != r),
            _ => None,
        };
        if let (Some(e), Some(d)) = (grown, &sc.draft) {
            let (ew, eh) = (e.w() as usize, e.h() as usize);
            let mut tmp = self.draft_buf.borrow_mut();
            // Every byte is written by `compose_bg`: no clearing.
            tmp.resize(ew * eh * 4, 0);
            let tmp = &mut tmp[..ew * eh * 4];
            compose_bg(sc, e, bd, tmp, order);
            d.render_into(&mut Surf::with_origin(tmp, ew as u32, eh as u32, e.x0, e.y0, order), self.font.as_ref());
            let n = w * 4;
            for y in r.y0..r.y1 {
                let si = ((y - e.y0) as usize * ew + (r.x0 - e.x0) as usize) * 4;
                out[(y - r.y0) as usize * n..][..n].copy_from_slice(&tmp[si..si + n]);
            }
        } else {
            let p = crate::perf::span(crate::perf::K::Bg);
            compose_bg(sc, r, bd, out, order);
            drop(p);
            if let Some(d) = &sc.draft
                && hits(El::Draft)
            {
                d.render_into(&mut Surf::with_origin(out, w as u32, h as u32, r.x0, r.y0, order), self.font.as_ref());
            }
        }

        if hits(El::Ghost) {
            let mut sf = Surf::with_origin(out, w as u32, h as u32, r.x0, r.y0, order);
            for g in &sc.ghost {
                g.render_into(&mut sf, self.font.as_ref());
            }
        }
        let mut f = Fb::with_origin(out, w, r.x0, r.y0, order);
        let ui = self.ui(sc);
        if let Some(u) = &sc.snip
            && hits(El::Snip)
        {
            super::snip::draw(&mut f, &ui, u);
        }
        if let Some(c) = &sc.cross
            && hits(El::Cross)
        {
            assist::cross(&mut f, &ui, c, sc.size);
        }
        // A spotlight being drawn: its rect (the dim comes on release).
        if let Some(Obj::Spotlight { r: sr }) = &sc.draft
            && hits(El::Draft)
        {
            super::mini::marquee(&mut f, &ui, *sr);
        }
        if let Some(u) = &sc.sel_ui
            && hits(El::Sel)
        {
            let _p = crate::perf::span(crate::perf::K::Sel);
            if let Some((b, c)) = u.border {
                chrome::border_preview(&mut f, &ui, u.sr, b, c);
            }
            chrome::selection(&mut f, &ui, u.sr, u.hot, 1.0);
        }
        if let Some(u) = &sc.osel
            && hits(El::ObjSel)
        {
            super::mini::osel(&mut f, &ui, u);
        }
        if hits(El::Guides) {
            super::mini::guides(&mut f, &ui, &sc.guides, sc.guides_k);
        }
        if let Some(m) = sc.marquee
            && hits(El::Marquee)
        {
            super::mini::marquee(&mut f, &ui, m);
        }
        if let Some(c) = &sc.caret
            && hits(El::Caret)
        {
            super::mini::caret(&mut f, c);
        }
        if let Some(u) = &sc.snap
            && hits(El::Snap)
        {
            let _p = crate::perf::span(crate::perf::K::Sel);
            chrome::outline(&mut f, &ui, u.r, 1.0);
            chrome::snap_label(&mut f, &ui, u.r, u.hint, u.area, 1.0);
        }
        if let Some(l) = &sc.label
            && hits(El::Label)
        {
            let _p = crate::perf::span(crate::perf::K::Label);
            chrome::size_label(&mut f, &ui, l.sr, l.show_pos, l.area, (l.avoid, l.lead), 1.0);
        }
        if let Some(t) = &sc.text
            && hits(El::Text)
        {
            let s = sc.s;
            f.stroke_dashed_rect(t.boxr, 4.0 * s, 3.0 * s, s.round().max(1.0), sc.th.accent);
            if let Some(font) = self.font.as_ref() {
                // Drawn exactly as the committed text object renders.
                let font = font.styled(t.fmt);
                font.render(&mut f.surf(), &t.text, t.px, t.pos, t.color);
                if let Some(caret) = t.caret {
                    let at = font.caret_at(&t.text, caret, t.px);
                    let cx = t.pos.x + at.x;
                    f.fill_rect(
                        cx.round() as i32,
                        (t.pos.y + at.y).round() as i32,
                        (2.0 * s).round() as i32,
                        (t.px * 1.15).round() as i32,
                        t.color,
                    );
                }
            }
        }
        if let Some(t) = &sc.toast
            && hits(El::Toast)
        {
            chrome::toast(&mut f, &ui, &t.text, t.kind, t.area, t.k);
        }
        if let Some(b) = &sc.bar
            && hits(El::Bar)
        {
            let _p = crate::perf::span(crate::perf::K::Bar);
            let st = BarState {
                hover: b.hover,
                hover_k: b.hover_k,
                pressed: b.pressed,
                tool: b.tool,
                color: b.color,
                value: &b.value,
                ptool: b.ptool,
                size: b.size,
                fill: b.fill,
                pop_k: b.pop_k,
                pick: b.pick.as_ref(),
            };
            chrome::toolbar(&mut f, &ui, &b.tb, &st, b.k);
        }
        if let Some(m) = &sc.mini
            && hits(El::Mini)
        {
            super::mini::draw(&mut f, &ui, m);
        }
        if let Some(t) = &sc.tip
            && hits(El::Tip)
        {
            chrome::tooltip(&mut f, &ui, t.at(), t.label, &t.keys, t.k);
        }
        if let Some(h) = &sc.hint
            && hits(El::Hint)
        {
            chrome::hint(&mut f, &ui, h.area, h.k, &h.parts);
        }
        if let Some(m) = &sc.mag
            && hits(El::Mag)
        {
            assist::mag(&mut f, &ui, m);
        }
        if let Some(l) = &sc.loupe
            && hits(El::Loupe)
        {
            chrome::loupe(&mut f, &ui, l.r, &l.px);
        }
        if let Some(rg) = &sc.ring
            && hits(El::Ring)
        {
            assist::ring(&mut f, &ui, rg);
        }
    }

    /// Rects whose pixels may differ between the painted scene `prev` and
    /// the prepared one (see [`dirty_rects`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn dirty_rects(&self, prev: Option<&Scene>) -> Vec<PxRect> {
        match &self.scene {
            Some(now) => dirty_rects(prev, now, self.font.as_ref()),
            None => Vec::new(),
        }
    }
}

/// Pixels two pixelate drafts render identically: the whole cells inside
/// both, when the grid origin and cell size, the selection and the
/// objects (so the backdrop under them) are unchanged. A drag that moves
/// one corner repaints only the cells along the moving edges.
fn stable_draft(prev: &Scene, now: &Scene) -> Option<PxRect> {
    let (a, b) = (prev.draft_grid()?, now.draft_grid()?);
    if prev.sel != now.sel || !Arc::ptr_eq(&prev.objects, &now.objects) {
        return None;
    }
    let (x0, y0, x1, y1) = a.stable_with(&b, now.size.0 as i32, now.size.1 as i32)?;
    Some(PxRect::new(x0, y0, x1, y1)).filter(|r| !r.is_empty())
}

/// The selection chrome that may differ between `prev` and `now` (same
/// hot handle), appended to `out`; `false` = not comparable, repaint all.
/// A border strip whose line stays put changes only near its ends, where
/// the crossing edges, corners and handles are: its middle is kept (a
/// resize repaints the moving edges and the strips' ends, not the whole
/// outline). Every stroke pixel lies in its own strip and every handle in
/// its box, so a pixel changed by a moved edge or handle is in that edge's
/// or handle's rects, which are repainted whole.
fn sel_dirty(prev: &Scene, now: &Scene, out: &mut Vec<PxRect>) -> bool {
    let (Some(a), Some(b)) = (&prev.sel_ui, &now.sel_ui) else { return false };
    // Four strips (top, bottom, left, right), then the eight handles.
    let (p, n) = (prev.rects(El::Sel), now.rects(El::Sel));
    if a.hot != b.hot || p.len() != 12 || n.len() != 12 {
        return false;
    }
    for (i, (&r, &s)) in p.iter().zip(n).enumerate() {
        if i >= 4 {
            if r != s {
                out.extend([r, s]);
            }
            continue;
        }
        // The crossing edges lie in the first and last strip-width pixels.
        let keep = if i < 2 && (r.y0, r.y1) == (s.y0, s.y1) {
            PxRect::new(r.x0.max(s.x0) + r.h(), r.y0, r.x1.min(s.x1) - r.h(), r.y1)
        } else if i >= 2 && (r.x0, r.x1) == (s.x0, s.x1) {
            PxRect::new(r.x0, r.y0.max(s.y0) + r.w(), r.x1, r.y1.min(s.y1) - r.w())
        } else {
            PxRect::default()
        };
        out.extend(r.minus_iter(&keep).chain(s.minus_iter(&keep)));
    }
    true
}

/// Rects (clipped to the image, possibly overlapping) covering every pixel
/// that may differ between scenes `prev` and `now`: old and new bounds of
/// each changed element, the symmetric difference of the selections, and
/// the bounds of objects past the first changed one. The whole image when
/// there is no previous scene, or size, scale, theme or dim alpha changed,
/// or a selection edge is fractional.
pub fn dirty_rects(prev: Option<&Scene>, now: &Scene, font: Option<&crate::text::AnnotFont>) -> Vec<PxRect> {
    let img = PxRect::image(now.size);
    let whole = vec![img];
    let Some(prev) = prev else { return whole };
    if prev.size != now.size || prev.s != now.s || prev.th != now.th || prev.dim_alpha != now.dim_alpha {
        return whole;
    }
    let mut out = Vec::new();
    if prev.sel != now.sel && now.dim_alpha != 0 {
        let integral = |r: &Option<FRect>| {
            r.is_none_or(|r| [r.x, r.y, r.w, r.h].iter().all(|v| v.fract() == 0.0))
        };
        if !integral(&prev.sel) || !integral(&now.sel) {
            return whole;
        }
        let inside = |r: &Option<FRect>| r.map_or(PxRect::default(), PxRect::outer).intersect(&img);
        let (a, b) = (inside(&prev.sel), inside(&now.sel));
        out.extend(a.minus(&b));
        out.extend(b.minus(&a));
    }
    for e in ELS {
        if prev.same(now, e) {
            continue;
        }
        if e == El::Sel && sel_dirty(prev, now, &mut out) {
            continue;
        }
        if e == El::Cross
            && let (Some(a), Some(b)) = (&prev.cross, &now.cross)
        {
            // One band per line from the old to the new position (a move
            // is a few pixels): two rects, not four, so a drag stays
            // under the damage rect cap.
            out.extend(assist::cross_dirty(a, b, now.size));
            continue;
        }
        if e == El::Draft
            && let Some(keep) = stable_draft(prev, now)
        {
            out.extend(prev.rects(e).iter().chain(now.rects(e)).flat_map(|r| r.minus_iter(&keep)));
            continue;
        }
        out.extend_from_slice(prev.rects(e));
        out.extend_from_slice(now.rects(e));
    }
    // The objects, and the spotlights' dim (which follows the selection).
    if !Arc::ptr_eq(&prev.objects, &now.objects) || (prev.sel != now.sel && now.objects.iter().any(Obj::is_spot)) {
        let (pa, na) = (spot_area(prev.sel, prev.size), spot_area(now.sel, now.size));
        match object_damage(&prev.objects, &now.objects, pa, na, font, now.size) {
            Some(v) => out.extend(v),
            None => return whole,
        }
    }
    out.into_iter().map(|r| r.intersect(&img)).filter(|r| !r.is_empty()).collect()
}

/// The area a bake's spotlight pass dims: the capture's selection (on
/// whole pixels, as the export crops it), else the whole image.
pub fn spot_area(sel: Option<FRect>, size: (u32, u32)) -> crate::objects::Px {
    let img = PxRect::image(size);
    let r = sel.map_or(img, PxRect::outer).intersect(&img);
    (r.x0, r.y0, r.x1, r.y1)
}

fn px_of(p: crate::objects::Px) -> PxRect {
    PxRect::new(p.0, p.1, p.2, p.3)
}

/// The pixels two spotlight passes dim differently.
fn pass_damage(a: Option<&crate::objects::SpotPass>, b: Option<&crate::objects::SpotPass>, out: &mut Vec<PxRect>) {
    match (a, b) {
        (None, None) => {}
        (Some(p), None) | (None, Some(p)) => out.push(px_of(p.area)),
        (Some(p), Some(q)) if p == q => {}
        (Some(p), Some(q)) if p.at != q.at || p.spots.len() != q.spots.len() => {
            out.push(px_of(p.area));
            out.push(px_of(q.area));
        }
        (Some(p), Some(q)) => {
            // Only where an area or a spot was or is now, not both.
            let (ap, aq) = (px_of(p.area), px_of(q.area));
            if ap != aq {
                out.extend(ap.minus_iter(&aq));
                out.extend(aq.minus_iter(&ap));
            }
            for (s, t) in p.spots.iter().zip(&q.spots).filter(|(s, t)| s != t) {
                let (s, t) = (px_of(*s), px_of(*t));
                out.extend(s.minus_iter(&t).map(|r| r.intersect(&ap.union(&aq))));
                out.extend(t.minus_iter(&s).map(|r| r.intersect(&ap.union(&aq))));
            }
        }
    }
}

/// Pixel windows covering every pixel whose baked value differs between
/// baking `prev` over spotlight area `pa` and `now` over `na`, each safe to
/// re-bake on its own: cell-aligned to every pixelate it cuts. Changed
/// objects' old and new bounds, a spotlight pass that changed, then what
/// later objects read from them (cells of a pixelate, a loupe's view);
/// the objects below the first change are untouched. `None` when the
/// windows do not settle (re-bake everything).
pub fn object_damage(
    prev: &[Obj],
    now: &[Obj],
    pa: crate::objects::Px,
    na: crate::objects::Px,
    font: Option<&crate::text::AnnotFont>,
    size: (u32, u32),
) -> Option<Vec<PxRect>> {
    let img = PxRect::image(size);
    let pb = |o: &Obj| o.bounds(font).map(|b| px_bounds(b).intersect(&img)).filter(|r| !r.is_empty());
    let common = prev.iter().zip(now).take_while(|(a, b)| a == b).count();
    let mut d = Vec::new();
    let add = |o: &Obj, d: &mut Vec<PxRect>| {
        if !o.is_spot() {
            d.extend(pb(o));
        }
    };
    if prev.len() == now.len() {
        for (a, b) in prev[common..].iter().zip(&now[common..]) {
            if a != b {
                add(a, &mut d);
                add(b, &mut d);
            }
        }
    } else {
        for o in prev[common..].iter().chain(&now[common..]) {
            add(o, &mut d);
        }
    }
    let (pp, np) = (crate::objects::SpotPass::of(prev, pa), crate::objects::SpotPass::of(now, na));
    pass_damage(pp.as_ref(), np.as_ref(), &mut d);
    d.retain(|r| !r.intersect(&img).is_empty());
    if d.is_empty() {
        return Some(d);
    }
    let common = if pp != np { 0 } else { common };
    // What the later objects read from the changed pixels.
    for o in now[common.min(now.len())..].iter().filter(|o| !o.is_spot()) {
        let Some(b) = pb(o) else { continue };
        let hits: Vec<PxRect> = d.iter().map(|r| r.intersect(&b)).filter(|r| !r.is_empty()).collect();
        if hits.is_empty() {
            continue;
        }
        if let Some((s, l)) = o.loupe_parts() {
            // A loupe shows its source: changed there, changed here.
            if d.iter().any(|r| r.intersects(&px_bounds(s))) {
                d.push(px_bounds(l).intersect(&img));
            }
        } else if let Some(g) = o.pixel_grid() {
            d.extend(hits.into_iter().map(|h| cell_align(h, &g, size)));
        }
    }
    // Windows that re-bake exactly on their own.
    let mut d = merge_rects(d);
    for _ in 0..8 {
        let mut grew = false;
        for w in d.iter_mut() {
            for o in now.iter().filter(|o| !o.is_spot()) {
                let Some(b) = pb(o) else { continue };
                if !w.intersects(&b) {
                    continue;
                }
                // (A loupe reads its source beside the window: `render_window`.)
                let n = if let Some(g) = o.pixel_grid() {
                    cell_align(*w, &g, size)
                } else {
                    *w
                };
                if n != *w {
                    *w = n;
                    grew = true;
                }
            }
        }
        d = merge_rects(d);
        if !grew {
            return Some(d);
        }
    }
    None
}

/// Re-bake windows `rects` of `out` (the capture `base` with `objs` baked
/// in, the spotlight pass over `area`): each from the base, every object
/// over it rendered into it.
pub fn rebake(out: &mut PixBuf, base: &PixBuf, objs: &[Obj], area: crate::objects::Px, font: Option<&crate::text::AnnotFont>, rects: &[PxRect]) {
    let iw = base.width() as usize;
    let img = PxRect::image(base.dimensions());
    let bg = |r: PxRect, out: &mut [u8]| {
        let n = r.w() as usize * 4;
        for (i, y) in (r.y0..r.y1).enumerate() {
            out[i * n..][..n].copy_from_slice(&base.as_raw()[(y as usize * iw + r.x0 as usize) * 4..][..n]);
        }
    };
    let bake = Bake { objs, font, pass: crate::objects::SpotPass::of(objs, area), img, bg: &bg };
    let mut tmp = Vec::new();
    for w in rects.iter().map(|w| w.intersect(&img)).filter(|w| !w.is_empty()) {
        let (ww, wh) = (w.w() as usize, w.h() as usize);
        tmp.clear();
        tmp.resize(ww * wh * 4, 0);
        bg(w, &mut tmp);
        bake.window(&mut tmp, w, Order::Rgba);
        let raw = out.as_raw_mut();
        for (i, y) in (w.y0..w.y1).enumerate() {
            raw[(y as usize * iw + w.x0 as usize) * 4..][..ww * 4].copy_from_slice(&tmp[i * ww * 4..][..ww * 4]);
        }
    }
}

/// What a bake renders: the objects, their spotlight pass, the image
/// rect, and where the background of any rect comes from (`bg`: the
/// capture's pixels of a rect inside the image).
pub struct Bake<'a> {
    pub objs: &'a [Obj],
    pub font: Option<&'a crate::text::AnnotFont>,
    pub pass: Option<crate::objects::SpotPass>,
    pub img: PxRect,
    pub bg: &'a dyn Fn(PxRect, &mut [u8]),
}

impl Bake<'_> {
    /// Every object over window `w` rendered into `buf` (its pixels, in
    /// `order`, already the background), the spotlight pass after the
    /// topmost spotlight.
    pub fn window(&self, buf: &mut [u8], w: PxRect, order: Order) {
        self.window_upto(buf, w, order, self.objs.len());
    }

    /// [`Bake::window`] with the objects below `upto` only. A loupe whose
    /// source lies outside the window reads it from a window of its own,
    /// rendered with the objects below the loupe.
    fn window_upto(&self, buf: &mut [u8], w: PxRect, order: Order, upto: usize) {
        let font = self.font;
        let mut sf = Surf::with_origin(buf, w.w() as u32, w.h() as u32, w.x0, w.y0, order);
        for (j, o) in self.objs[..upto].iter().enumerate() {
            if let Some(p) = self.pass.as_ref().filter(|p| p.at == j) {
                p.apply(&mut sf);
                continue;
            }
            if o.is_spot() || !o.bounds(font).is_some_and(|b| px_bounds(b).intersects(&w)) {
                continue;
            }
            if let Some((s, l)) = o.loupe_parts() {
                let mut s = px_bounds(s).intersect(&self.img);
                if px_bounds(l).intersects(&w) && !s.is_empty() && !w.contains(&s) {
                    // Whole cells of the pixelates below (it renders them).
                    let size = (self.img.x1 as u32, self.img.y1 as u32);
                    for _ in 0..4 {
                        let n = self.objs[..j].iter().filter_map(Obj::pixel_grid).fold(s, |s, g| cell_align(s, &g, size));
                        if n == s {
                            break;
                        }
                        s = n;
                    }
                    let mut sb = vec![0u8; s.w() as usize * s.h() as usize * 4];
                    (self.bg)(s, &mut sb);
                    self.window_upto(&mut sb, s, order, j);
                    let src = Surf::with_origin(&mut sb, s.w() as u32, s.h() as u32, s.x0, s.y0, order);
                    o.render_with_source(&mut sf, &src, font);
                    continue;
                }
            }
            o.render_into(&mut sf, font);
        }
    }
}
