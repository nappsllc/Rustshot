//! The overlay's selection helpers (Flameshot parity): the pixel
//! magnifier and crosshair while a region is picked, the right-click
//! color ring, accept on select, and the first-open hint card. Their
//! state lives in [`Assist`]; what a frame draws of them is resolved in
//! `prepare` ([`Edit::assist_scene`]) and drawn here.

use super::chrome::{self, HintPart, Ui};
use super::compose::PxRect;
use super::*;
use crate::theme::SHADOW;
use crate::uifb::Fb;

/// What "Accept on select" (Settings › Capture, `--accept-on-select`)
/// does with a region drawn (or a window clicked).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AcceptOn {
    Copy,
    Save,
}

impl AcceptOn {
    /// `accept_on_select` ("off", "copy", "save"); None for off.
    pub fn from_config(v: &str) -> Option<AcceptOn> {
        match v.trim().to_ascii_lowercase().as_str() {
            "copy" => Some(AcceptOn::Copy),
            "save" => Some(AcceptOn::Save),
            _ => None,
        }
    }

    /// The export it runs (Save as Ctrl+S does: asks when Settings say so).
    pub(super) fn tasks(self, cfg: &Config) -> Vec<Task> {
        match self {
            AcceptOn::Copy => vec![Task::Copy],
            AcceptOn::Save => vec![Task::Save { path: None, ask: cfg.save_dialog }],
        }
    }
}

/// Accept on select leaves regions smaller than this (logical px) alone.
pub const ACCEPT_MIN: f32 = 8.0;

/// Pixels across the magnifier (an odd count: the pointer's is the middle).
pub const MAG_N: usize = 17;
/// The magnifier's side, logical px (8 px per pixel plus a 2 px rim).
const MAG_SIZE: f32 = 140.0;
/// The rim around the magnified pixels, logical px.
const MAG_RIM: f32 = 2.0;
/// Pixel grid lines from this zoom on (physical px per pixel).
pub const MAG_GRID_AT: f32 = 6.0;
/// The magnifier's distance from the pointer, logical px.
const MAG_OFF: f32 = 24.0;
/// The caption under it ("x, y · #rrggbb"): gap and height.
const CAP_GAP: f32 = 6.0;
const CAP_H: f32 = 22.0;

/// The hint card stays this long (or until the pointer moves or a key
/// is pressed), then fades out.
pub const HINT_MS: u64 = 3000;
pub const HINT_FADE_MS: u64 = 150;
/// A move this far (physical px) from where the pointer first was
/// dismisses the hint (the window's own first move does not).
const HINT_MOVE_PX: f32 = 3.0;

/// The color ring: swatch centres this far out, swatch radius, the
/// centre button's radius and the panel's margin (logical px).
pub const RING_R: f32 = 70.0;
const RING_SW: f32 = 13.0;
const RING_C: f32 = 17.0;
const RING_PAD: f32 = 9.0;
/// The most colors the ring shows.
pub const RING_MAX: usize = 12;
/// The centre's dead zone (logical px): releasing there without having
/// left it keeps the ring open (click, then click a color).
const RING_DEAD: f32 = RING_C + 4.0;

/// What the ring has under a point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RingHit {
    Swatch(usize),
    /// The centre button: the color wheel ("Custom…").
    Center,
}

/// The ring while it is open.
#[derive(Clone, Debug)]
pub(super) struct Ring {
    /// Its centre (the press point, kept on the monitor).
    pub c: Pt,
    pub colors: Vec<C4>,
    /// The current color's swatch.
    pub cur: Option<usize>,
    pub hot: Option<RingHit>,
    /// The right button is still down (press, move, release); false: it
    /// was clicked and waits for a click on a color.
    pub held: bool,
    /// The pointer has left the dead zone around the press since.
    pub left: bool,
    /// Where the right button went down.
    pub press: Pt,
    /// The centre opens the color wheel (it needs the toolbar).
    pub custom: bool,
}

/// The helpers' state on [`Edit`].
pub(super) struct Assist {
    pub ring: Option<Ring>,
    /// The color picker opened from the ring with nothing selected: it
    /// stands alone here (no toolbar to hang off).
    pub pick_at: Option<Pt>,
    /// Each monitor (image coordinates) with its DPI scale.
    pub mon_scales: Vec<(capture::IRect, f32)>,
    /// The hint's phrases, made once (the keymap does not change).
    pub hint_parts: Option<(bool, Vec<HintPart>)>,
    /// Accept on select: what a new region runs.
    pub accept: Option<AcceptOn>,
    /// When the hint card goes (3 s after the overlay opened, or the
    /// first move or key before).
    pub hint_end: Instant,
    /// Where the pointer first was (moves are measured from there).
    pub hint_origin: Option<Pt>,
    /// Tests: the last color copied (the clipboard is not touched).
    #[cfg(test)]
    pub copied: Option<String>,
}

impl Assist {
    pub fn new(cfg: &Config, now: Instant) -> Assist {
        Assist {
            ring: None,
            pick_at: None,
            mon_scales: Vec::new(),
            hint_parts: None,
            accept: AcceptOn::from_config(&cfg.accept_on_select),
            hint_end: now + Duration::from_millis(if cfg.overlay_hints { HINT_MS } else { 0 }),
            hint_origin: None,
            #[cfg(test)]
            copied: None,
        }
    }

    /// The hint's time starts again (the overlay just came up).
    pub fn restart_hint(&mut self, on: bool, now: Instant) {
        self.hint_end = now + Duration::from_millis(if on { HINT_MS } else { 0 });
    }

    /// The hint is still up at `now` (fading out after it).
    pub fn hint_live(&self, now: Instant) -> bool {
        now < self.hint_end
    }

    /// The hint goes now (the first move or key).
    pub fn dismiss_hint(&mut self, now: Instant) {
        self.hint_end = self.hint_end.min(now);
    }

    /// The hint's end is near: fast ticks, so it fades out on time.
    pub fn hint_due(&self, now: Instant) -> bool {
        self.hint_live(now) && self.hint_end.saturating_duration_since(now) <= Duration::from_millis(160)
    }
}

/// The crosshair: 1 px lines through the pointer's pixel across the
/// whole overlay (accent at 60 %), left out under the toolbar (`skip`:
/// its bounds), so a drag never redraws the bar for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CrossUi {
    pub x: i32,
    pub y: i32,
    pub lw: i32,
    pub skip: Vec<PxRect>,
}

impl CrossUi {
    /// The two lines (horizontal, vertical; the vertical without the
    /// crossing), each minus `skip`.
    fn parts(&self, size: (u32, u32)) -> Vec<PxRect> {
        let (w, h) = (size.0 as i32, size.1 as i32);
        let lines = [
            PxRect::new(0, self.y, w, self.y + self.lw),
            PxRect::new(self.x, 0, self.x + self.lw, self.y),
            PxRect::new(self.x, self.y + self.lw, self.x + self.lw, h),
        ];
        cut(lines.into_iter().filter(|r| !r.is_empty()).collect(), &self.skip)
    }
}

/// `v` minus every rect of `skip`.
fn cut(mut v: Vec<PxRect>, skip: &[PxRect]) -> Vec<PxRect> {
    for k in skip {
        v = v.iter().flat_map(|r| r.minus_iter(k)).collect();
    }
    v
}

/// The magnifier: the loupe square `r` (round unless `square`), its
/// caption panel `cap`, the pixels around the pointer, the caption text.
#[derive(Clone, PartialEq, Debug)]
pub struct MagUi {
    pub r: FRect,
    pub cap: FRect,
    pub px: Vec<C4>,
    pub text: String,
    pub square: bool,
    /// Physical px per magnified pixel, and the rim.
    pub cell: f32,
    pub rim: f32,
    /// Its UI scale: the monitor's under the pointer.
    pub s: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct RingUi {
    pub c: Pt,
    pub colors: Vec<C4>,
    pub cur: Option<usize>,
    pub hot: Option<RingHit>,
    pub custom: bool,
}

/// The hint card: monitor, opacity, its words and keys.
#[derive(Clone, PartialEq, Debug)]
pub struct HintUi {
    pub area: FRect,
    pub k: f32,
    pub parts: Vec<HintPart>,
}

// ---- geometry (pure; tested) ----

/// Physical px per magnified pixel and the rim at scale `s`.
pub fn mag_geom(s: f32) -> (f32, f32) {
    let rim = (MAG_RIM * s).round().max(1.0);
    let cell = ((MAG_SIZE - 2.0 * MAG_RIM) / MAG_N as f32 * s).round().max(1.0);
    (cell, rim)
}

/// Where the magnifier goes for pointer `p` on monitor `area`: below
/// right of the pointer, flipped left / up to stay on the monitor and
/// off `avoid` (the toolbar); its caption under it. Never over the
/// pointer while the monitor has room on some side.
pub fn mag_rect(p: Pt, area: FRect, s: f32, cap_w: f32, avoid: &[FRect]) -> (FRect, FRect) {
    let (cell, rim) = mag_geom(s);
    let d = cell * MAG_N as f32 + 2.0 * rim;
    let (gap, ch) = ((CAP_GAP * s).round(), (CAP_H * s).round());
    let (w, h) = (d.max(cap_w), d + gap + ch);
    let off = MAG_OFF * s;
    let (right, left) = (p.x + off, p.x - off - w);
    let (below, above) = (p.y + off, p.y - off - h);
    let fits = |x: f32, y: f32| x >= area.x && x + w <= area.x1() && y >= area.y && y + h <= area.y1();
    let clear = |x: f32, y: f32| avoid.iter().all(|a| x + w <= a.x || a.x1() <= x || y + h <= a.y || a.y1() <= y);
    let cands = [(right, below), (left, below), (right, above), (left, above)];
    let (x, y) = cands
        .into_iter()
        .find(|&(x, y)| fits(x, y) && clear(x, y))
        .or_else(|| cands.into_iter().find(|&(x, y)| fits(x, y)))
        .unwrap_or_else(|| {
            let x = if right + w > area.x1() { left } else { right };
            let y = if below + h > area.y1() { above } else { below };
            (x.clamp(area.x, (area.x1() - w).max(area.x)), y.clamp(area.y, (area.y1() - h).max(area.y)))
        });
    let (x, y) = (x.round(), y.round());
    let r = FRect { x: x + ((w - d) / 2.0).round(), y, w: d, h: d };
    let cap = FRect { x: x + ((w - cap_w) / 2.0).round(), y: y + d + gap, w: cap_w.round(), h: ch };
    (r, cap)
}

/// The caption: the pixel's position (image coordinates, as the size
/// label counts) and its color.
pub fn mag_text(x: i32, y: i32, c: C4) -> String {
    format!("{x}, {y} \u{b7} {}", hex(c))
}

/// `#rrggbb` (lowercase).
pub fn hex(c: C4) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// The centre of swatch `i` of `n` (clockwise from the top).
pub fn ring_swatch(c: Pt, i: usize, n: usize, s: f32) -> Pt {
    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / n.max(1) as f32;
    Pt::new(c.x + RING_R * s * a.cos(), c.y + RING_R * s * a.sin())
}

/// What the ring centred on `c` with `n` swatches has under `p`: the
/// centre within the dead zone; past it the swatch in that direction
/// (`sector`: a press-drag-release gesture) or only a swatch hit itself
/// (clicks).
pub fn ring_hit(c: Pt, p: Pt, n: usize, s: f32, sector: bool) -> Option<RingHit> {
    let (dx, dy) = (p.x - c.x, p.y - c.y);
    if dx.hypot(dy) <= RING_DEAD * s {
        return Some(RingHit::Center);
    }
    if n == 0 {
        return None;
    }
    if sector {
        let step = std::f32::consts::TAU / n as f32;
        let a = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
        let i = ((a / step).round() as i32).rem_euclid(n as i32) as usize;
        return Some(RingHit::Swatch(i));
    }
    (0..n)
        .find(|&i| {
            let q = ring_swatch(c, i, n, s);
            (p.x - q.x).hypot(p.y - q.y) <= (RING_SW + 4.0) * s
        })
        .map(RingHit::Swatch)
}

/// The ring's outer radius, logical px.
fn ring_outer() -> f32 {
    RING_R + RING_SW + RING_PAD
}

/// The ring's centre for a press at `p`: kept whole on monitor `area`.
pub fn ring_center(p: Pt, area: FRect, s: f32) -> Pt {
    let r = ring_outer() * s + 4.0;
    let cx = if area.w > 2.0 * r { p.x.clamp(area.x + r, area.x1() - r) } else { area.x + area.w / 2.0 };
    let cy = if area.h > 2.0 * r { p.y.clamp(area.y + r, area.y1() - r) } else { area.y + area.h / 2.0 };
    Pt::new(cx, cy)
}

/// The hint's phrases with the keymap's keys: "Drag to select · Ctrl+C
/// copy · Ctrl+S save · Esc cancel · right-click colors" ("Click a window
/// or drag to select" while snapping). An unbound action is left out.
pub fn hint_parts(keys: &Keymap, snap: bool) -> Vec<HintPart> {
    let dot = || HintPart::T(chrome::HINT_DOT.into());
    let first = if snap { tr!("editor.hint.snap_select") } else { tr!("editor.hint.select") };
    let mut v = vec![HintPart::T(first.into())];
    for (a, word) in [(Action::Copy, tr!("editor.hint.copy")), (Action::Save, tr!("editor.hint.save")), (Action::Cancel, tr!("editor.hint.cancel"))] {
        if let Some(c) = keys.chords(a).first() {
            v.extend([dot(), HintPart::K(chrome::chord_caps(c).join("+")), HintPart::T(word.into())]);
        }
    }
    v.extend([dot(), HintPart::T(tr!("editor.hint.colors").into())]);
    v
}

// ---- the editor's side ----

impl Edit {
    /// The magnifier shows: picking a region (nothing selected yet and no
    /// tool, or drawing / resizing the selection), not while annotating,
    /// typing, picking a color or in video mode.
    pub(super) fn mag_on(&self) -> bool {
        self.cfg.magnifier
            && self.host.is_none()
            && self.pick.is_none()
            && self.text.is_none()
            && self.obj.cedit.is_none()
            && self.assist.ring.is_none()
            && !self.rec.open
            && match self.interact {
                Interact::None => (self.sel.is_none() && self.tool.is_none()) || self.tool == Some(Tool::Ruler),
                Interact::NewSel { .. } | Interact::Resize { .. } => true,
                Interact::Drawing { .. } => self.tool == Some(Tool::Ruler),
                _ => false,
            }
    }

    /// The UI scale of the monitor under `p` (`mon_scales`, filled when
    /// the capture starts), None when unknown or one scale fits all.
    fn scale_at(&self, p: Pt) -> Option<f32> {
        let (_, sc) = self.assist.mon_scales.iter().find(|(m, _)| toolbar::hit(FRect { x: m.0 as f32, y: m.1 as f32, w: m.2 as f32, h: m.3 as f32 }, p))?;
        Some(self.cfg.ui_scale.apply(*sc))
    }

    /// The pointer's pixel, if it is on the capture.
    fn mouse_px(&self, m: Pt) -> Option<(i32, i32)> {
        let (x, y) = (m.x.floor() as i32, m.y.floor() as i32);
        (x >= 0 && y >= 0 && x < self.shot.size.0 as i32 && y < self.shot.size.1 as i32).then_some((x, y))
    }

    /// `n`² pixels of the capture (with the objects) around pixel `c`,
    /// row by row; black off the image.
    pub(super) fn pixels_around(&self, c: (i32, i32), n: usize) -> Vec<C4> {
        let n = n as i32;
        let (x0, y0) = (c.0 - n / 2, c.1 - n / 2);
        let r = PxRect::new(x0, y0, x0 + n, y0 + n);
        let q = r.intersect(&PxRect::image(self.shot.size));
        let mut out = vec![C4::rgb(0, 0, 0); (n * n) as usize];
        let buf = self.backdrop_pixels(q);
        for y in q.y0..q.y1 {
            for x in q.x0..q.x1 {
                let i = (((y - q.y0) * q.w() + (x - q.x0)) * 4) as usize;
                out[((y - y0) * n + (x - x0)) as usize] = C4::rgb(buf[i], buf[i + 1], buf[i + 2]);
            }
        }
        out
    }

    /// The crosshair, the magnifier and the color ring of this frame.
    pub(super) fn assist_scene(&self, mouse: Pt, s: f32, avoid_more: &[FRect]) -> (Option<CrossUi>, Option<MagUi>, Option<RingUi>) {
        let ring = self.assist.ring.as_ref().map(|r| RingUi { c: r.c, colors: r.colors.clone(), cur: r.cur, hot: r.hot, custom: r.custom });
        let Some((x, y)) = self.mouse_px(mouse).filter(|_| self.mag_on()) else { return (None, None, ring) };
        let lw = s.round().max(1.0) as i32;
        let ui = Ui { th: &self.th, s, font: self.ui_font };
        let skip = self.toolbar.as_ref().map_or(Vec::new(), |t| chrome::toolbar_bounds(&ui, t).into_iter().map(PxRect::outer).collect());
        let cross = CrossUi { x: x - (lw - 1) / 2, y: y - (lw - 1) / 2, lw, skip };
        let px = self.pixels_around((x, y), MAG_N);
        let c = px[MAG_N * MAG_N / 2];
        let text = mag_text(x, y, c);
        // The loupe at the scale of the monitor under the pointer (mixed DPI).
        let ms = self.scale_at(mouse).unwrap_or(s);
        let mui = Ui { th: &self.th, s: ms, font: self.ui_font };
        let cap_w = cap_width(&mui, &text);
        let area = monitor_at(&self.shot.monitors, mouse).unwrap_or(FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 });
        let mut avoid: Vec<FRect> = self.toolbar.iter().flat_map(|t| std::iter::once(t.bar).chain(t.pop)).collect();
        avoid.extend_from_slice(avoid_more);
        let (r, cap) = mag_rect(mouse, area, ms, cap_w, &avoid);
        let (cell, rim) = mag_geom(ms);
        let mag = MagUi { r, cap, px, text, square: self.cfg.magnifier_square, cell, rim, s: ms };
        (Some(cross), Some(mag), ring)
    }

    /// The pointer moved (no drag): the magnifier, the ring's highlight
    /// and the hint follow; true when that must be drawn.
    pub(super) fn track_assist(&mut self, p: Pt) -> bool {
        let now = Instant::now();
        let mut changed = false;
        match self.assist.hint_origin {
            None => self.assist.hint_origin = Some(p),
            Some(o) if self.assist.hint_live(now) && (p.x - o.x).hypot(p.y - o.y) >= HINT_MOVE_PX => {
                self.assist.dismiss_hint(now);
                changed = true;
            }
            Some(_) => {}
        }
        let s = self.ui_scale();
        if let Some(r) = self.assist.ring.as_mut() {
            // The dead zone is around the press (the ring itself may sit
            // off it, kept on the monitor).
            if (p.x - r.press.x).hypot(p.y - r.press.y) > RING_DEAD * s {
                r.left = true;
            }
            let hot = if r.held && !r.left { None } else { ring_hit(r.c, p, r.colors.len(), s, r.held) };
            if hot != r.hot {
                r.hot = hot;
                changed = true;
            }
        }
        let drawn = self.scene.as_ref().is_some_and(|sc| sc.mag.is_some());
        changed || drawn || self.mag_on()
    }

    /// A key or a press: the hint goes.
    pub(super) fn assist_input(&mut self) {
        self.assist.dismiss_hint(Instant::now());
    }

    /// The right button went down at `p`: the color ring opens there (or,
    /// open after a click, the press picks).
    pub(super) fn right_down(&mut self, p: Pt) {
        self.assist_input();
        if self.assist.ring.as_ref().is_some_and(|r| !r.held) {
            self.ring_click(p);
            return;
        }
        if self.assist.ring.is_some()
            || self.host.is_some()
            || self.text.is_some()
            || self.pick.is_some()
            || self.obj.cedit.is_some()
            || self.rec.open
            || !matches!(self.interact, Interact::None)
        {
            return;
        }
        // The select tool on an object: what a right press did before.
        if self.tool == Some(Tool::Select) && self.obj_at(p).is_some() {
            return;
        }
        // Pixelate and Invert have no color (as the palette).
        if !tools::has_color(self.cur_tool()) && self.sel_color().is_none() {
            return;
        }
        let colors: Vec<C4> = palette_colors(&self.cfg).into_iter().take(RING_MAX).collect();
        if colors.is_empty() {
            return;
        }
        let now = self.sel_color().unwrap_or(self.color());
        let cur = colors.iter().position(|c| (c.r, c.g, c.b, c.a) == (now.r, now.g, now.b, now.a));
        let s = self.ui_scale();
        let area = monitor_at(&self.shot.monitors, p).unwrap_or(FRect { x: 0.0, y: 0.0, w: self.shot.size.0 as f32, h: self.shot.size.1 as f32 });
        let c = ring_center(p, area, s);
        self.palette_open = false;
        self.save_menu = false;
        self.assist.ring = Some(Ring { c, press: p, colors, cur, hot: None, held: true, left: false, custom: true });
    }

    /// The right button went up: on a color it is picked; in the dead
    /// zone, never left, the ring stays (click mode); else it closes.
    pub(super) fn right_up(&mut self, p: Pt) {
        let s = self.ui_scale();
        let Some(r) = self.assist.ring.as_mut().filter(|r| r.held) else { return };
        if (p.x - r.press.x).hypot(p.y - r.press.y) > RING_DEAD * s {
            r.left = true;
        }
        // Never moved off the press: a click, the ring waits for another.
        let hit = if r.left { ring_hit(r.c, p, r.colors.len(), s, true) } else { None };
        match hit {
            Some(RingHit::Swatch(i)) => {
                let c = r.colors[i];
                self.assist.ring = None;
                self.apply_act(Act::Color(c), p);
            }
            _ if !r.left => {
                r.held = false;
                r.hot = ring_hit(r.c, p, r.colors.len(), s, false);
            }
            _ => self.assist.ring = None,
        }
    }

    /// A press while the ring waits for a click: a color picks it, the
    /// centre opens the color wheel, anywhere else closes the ring.
    pub(super) fn ring_click(&mut self, p: Pt) {
        let s = self.ui_scale();
        let Some(r) = self.assist.ring.take() else { return };
        match ring_hit(r.c, p, r.colors.len(), s, false) {
            Some(RingHit::Swatch(i)) => self.apply_act(Act::Color(r.colors[i]), p),
            Some(RingHit::Center) => {
                // Nothing selected (no toolbar): the picker stands alone
                // at the ring.
                self.assist.pick_at = self.toolbar.is_none().then_some(r.c);
                self.apply_act(Act::Wheel, p);
                if self.pick.is_none() {
                    self.assist.pick_at = None;
                }
            }
            None => {}
        }
    }

    /// `H`: the hex color under the pointer to the clipboard, while the
    /// magnifier shows.
    pub(super) fn copy_color(&mut self, mouse: Pt) {
        let Some(px) = self.mouse_px(mouse).filter(|_| self.mag_on()) else { return };
        let c = self.pixels_around(px, 1)[0];
        let text = hex(c);
        #[cfg(test)]
        {
            self.assist.copied = Some(text.clone());
        }
        #[cfg(not(test))]
        if let Err(e) = export::copy_text_to_clipboard(&text) {
            self.notice = Some(Toast::new(tr!("editor.copy_failed", e), ToastKind::Error));
            return;
        }
        self.notice = Some(Toast::new(tr!("editor.copied", text), ToastKind::Success));
    }

    /// Accept on select after a new region (or window click): run the
    /// tasks (the CLI's, else the setting's) and close, unless Alt is
    /// held (adjust first; Shift is the square), the region is under
    /// 8 x 8 logical px (a slip), in record or video mode, or in the Edit
    /// window.
    pub(super) fn accept_new_region(&mut self) -> bool {
        let Some(m) = self.assist.accept else { return false };
        if drag_mods().alt || self.rec.mode || self.rec.open || self.host.is_some() {
            return false;
        }
        let min = ACCEPT_MIN * self.ui_scale();
        if self.sel.is_none_or(|r| r.w < min || r.h < min) {
            return false;
        }
        if self.tasks.is_empty() {
            self.tasks = m.tasks(&self.cfg);
        }
        self.done = true;
        self.cancelled = false;
        true
    }
}

fn cap_width(ui: &Ui, text: &str) -> f32 {
    ui.px(8.0 + 10.0 + 6.0 + 8.0) + chrome::tw(ui, 12.0, text)
}

// ---- drawing ----

/// Coverage of pixel centre (`px`, `py`) by the rounded rect `r` (radius `rad`).
fn rr_cov(px: f32, py: f32, r: FRect, rad: f32) -> f32 {
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0);
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (qx, qy) = ((px - cx).abs() - (r.w / 2.0 - rad), (py - cy).abs() - (r.h / 2.0 - rad));
    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad;
    (0.5 - d).clamp(0.0, 1.0)
}

/// A grid line over `c`: a little darker on light pixels, lighter on dark.
fn grid_line(c: C4) -> C4 {
    let l = (c.r as u32 * 3 + c.g as u32 * 6 + c.b as u32) / 10;
    let t = if l > 110 { 0u32 } else { 255 };
    let m = |v: u8| ((v as u32 * 200 + t * 55) / 255) as u8;
    C4::rgb(m(c.r), m(c.g), m(c.b))
}

pub fn cross(f: &mut Fb, ui: &Ui, c: &CrossUi, size: (u32, u32)) {
    let col = ui.th.accent.with_alpha(153);
    for r in c.parts(size) {
        f.fill_rect(r.x0, r.y0, r.w(), r.h(), col);
    }
}

pub fn cross_bounds(c: &CrossUi, size: (u32, u32)) -> Vec<PxRect> {
    c.parts(size)
}

/// A disc or a rounded rect, row by row: the shape's partly covered
/// pixels go to `edge(f, x, y, cov)` one by one, its fully covered runs
/// to `span(f, y, x0, x1)` (most of it: cheap fills, no per-pixel
/// distance). Only rows and columns inside `f`'s window are visited.
fn shape_rows(
    f: &mut Fb,
    r: FRect,
    round: bool,
    rad: f32,
    mut edge: impl FnMut(&mut Fb, i32, i32, f32),
    mut span: impl FnMut(&mut Fb, i32, i32, i32),
) {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let rr = r.w / 2.0;
    let cov = |px: f32, py: f32| if round { (rr + 0.5 - (px - cx).hypot(py - cy)).clamp(0.0, 1.0) } else { rr_cov(px, py, r, rad) };
    let (bx0, by0, bx1, by1) = f.bounds();
    let y0 = ((r.y - 1.0).floor() as i32).max(by0);
    let y1 = ((r.y1() + 1.0).ceil() as i32).min(by1);
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let (xl, xr) = if round {
            let dy = py - cy;
            let o = (rr + 0.5) * (rr + 0.5) - dy * dy;
            if o <= 0.0 {
                continue;
            }
            let h = o.sqrt();
            (cx - h, cx + h)
        } else {
            (r.x - 1.0, r.x1() + 1.0)
        };
        let mut x = (xl.floor() as i32).max(bx0);
        let mut xe = (xr.ceil() as i32).min(bx1);
        while x < xe {
            let c = cov(x as f32 + 0.5, py);
            if c >= 1.0 {
                break;
            }
            if c > 0.0 {
                edge(f, x, y, c);
            }
            x += 1;
        }
        while xe > x {
            let c = cov(xe as f32 - 0.5, py);
            if c >= 1.0 {
                break;
            }
            if c > 0.0 {
                edge(f, xe - 1, y, c);
            }
            xe -= 1;
        }
        if x < xe {
            span(f, y, x, xe);
        }
    }
}

/// An anti-aliased disc of radius `rad` (fast [`Fb::fill_circle`]),
/// leaving out the pixels nearer than `hole` to its centre (something
/// opaque covers them next: the magnified pixels).
fn disc(f: &mut Fb, cx: f32, cy: f32, rad: f32, hole: f32, c: C4) {
    let r = FRect { x: cx - rad, y: cy - rad, w: 2.0 * rad, h: 2.0 * rad };
    shape_rows(f, r, true, 0.0, |f, x, y, k| f.blend_px(x, y, c.fade(k)), |f, y, x0, x1| {
        let dy = y as f32 + 0.5 - cy;
        let o = hole * hole - dy * dy;
        if hole <= 0.0 || o <= 0.0 {
            f.fill_rect(x0, y, x1 - x0, 1, c);
            return;
        }
        let h = o.sqrt();
        // Pixel centres strictly inside the hole are skipped.
        let (a, b) = (((cx - h - 0.5).floor() as i32 + 1).clamp(x0, x1), ((cx + h - 0.5).ceil() as i32).clamp(x0, x1));
        f.fill_rect(x0, y, a - x0, 1, c);
        f.fill_rect(b, y, x1 - b, 1, c);
    });
}

/// A circle outline `w` wide just inside radius `rad`, visiting only
/// the pixels near it.
fn circle_line(f: &mut Fb, cx: f32, cy: f32, rad: f32, w: f32, c: C4) {
    let (bx0, by0, bx1, by1) = f.bounds();
    let ri = rad - w;
    let y0 = ((cy - rad - 1.0).floor() as i32).max(by0);
    let y1 = ((cy + rad + 1.0).ceil() as i32).min(by1);
    for y in y0..y1 {
        let dy = y as f32 + 0.5 - cy;
        let o = (rad + 1.0) * (rad + 1.0) - dy * dy;
        if o <= 0.0 {
            continue;
        }
        let ho = o.sqrt();
        let hi = ((ri - 1.0).max(0.0).powi(2) - dy * dy).max(0.0).sqrt();
        for (a, b) in [(cx - ho, cx - hi), (cx + hi, cx + ho)] {
            for x in ((a.floor() as i32).max(bx0))..((b.ceil() as i32).min(bx1)) {
                let d = (x as f32 + 0.5 - cx).hypot(dy);
                let k = (rad + 0.5 - d).clamp(0.0, 1.0).min((d - ri + 0.5).clamp(0.0, 1.0));
                if k > 0.0 {
                    f.blend_px(x, y, c.fade(k));
                }
            }
        }
    }
}

/// What a crosshair move repaints: the band between the old and new
/// horizontal lines and the one between the vertical lines (each line
/// alone when they are far apart).
pub fn cross_dirty(a: &CrossUi, b: &CrossUi, size: (u32, u32)) -> Vec<PxRect> {
    // Left out only where both frames leave the line out.
    let both: Vec<PxRect> = a.skip.iter().flat_map(|p| b.skip.iter().map(move |q| p.intersect(q))).filter(|r| !r.is_empty()).collect();
    cut(cross_bands(a, b, size), &both)
}

fn cross_bands(a: &CrossUi, b: &CrossUi, size: (u32, u32)) -> Vec<PxRect> {
    let (w, h) = (size.0 as i32, size.1 as i32);
    let mut v = Vec::with_capacity(4);
    if (a.y - b.y).abs() <= 64 {
        v.push(PxRect::new(0, a.y.min(b.y), w, (a.y + a.lw).max(b.y + b.lw)));
    } else {
        v.extend([PxRect::new(0, a.y, w, a.y + a.lw), PxRect::new(0, b.y, w, b.y + b.lw)]);
    }
    if (a.x - b.x).abs() <= 64 {
        v.push(PxRect::new(a.x.min(b.x), 0, (a.x + a.lw).max(b.x + b.lw), h));
    } else {
        v.extend([PxRect::new(a.x, 0, a.x + a.lw, h), PxRect::new(b.x, 0, b.x + b.lw, h)]);
    }
    v
}

pub fn mag(f: &mut Fb, ui: &Ui, m: &MagUi) {
    let ui = &Ui { s: m.s, ..*ui };
    let r = m.r;
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let rad = ui.px(12.0);
    // Inside this the magnified pixels are opaque (fully covered).
    let hole = m.cell * MAG_N as f32 / 2.0 - 1.0;
    if m.square {
        chrome::surface(f, ui, r, 12.0, &chrome::SMALL_LAYERS, 1.0);
    } else {
        for &i in chrome::SMALL_LAYERS.iter().rev() {
            let (dy, sp) = (ui.px(SHADOW[i].0), ui.px(SHADOW[i].1));
            disc(f, cx, cy + dy, r.w / 2.0 + sp, hole - dy, ui.th.shadow[i]);
        }
        disc(f, cx, cy, r.w / 2.0, hole, ui.th.surface);
    }
    // The pixels, clipped to the inner shape: whole runs of a cell's
    // color, its first column and row in the grid line's.
    let g = m.cell * MAG_N as f32;
    let (gx, gy) = (r.x + m.rim, r.y + m.rim);
    let inner = FRect { x: gx, y: gy, w: g, h: g };
    let grid = m.cell >= MAG_GRID_AT;
    let n = MAG_N as i32;
    let cell = m.cell.max(1.0) as i32;
    let (gx0, gy0) = (gx.round() as i32, gy.round() as i32);
    let at = |x: i32, y: i32| -> C4 {
        let (ix, iy) = (((x - gx0) / cell).clamp(0, n - 1), ((y - gy0) / cell).clamp(0, n - 1));
        let c = m.px[(iy * n + ix) as usize].with_alpha(255);
        let line = grid && ((ix > 0 && (x - gx0) % cell == 0) || (iy > 0 && (y - gy0) % cell == 0));
        if line { grid_line(c) } else { c }
    };
    shape_rows(
        f,
        inner,
        !m.square,
        (rad - m.rim).max(0.0),
        |f, x, y, k| f.blend_px(x, y, at(x, y).fade(k)),
        |f, y, x0, x1| {
            let iy = ((y - gy0) / cell).clamp(0, n - 1);
            let row_line = grid && iy > 0 && (y - gy0) % cell == 0;
            let mut x = x0;
            while x < x1 {
                let ix = ((x - gx0) / cell).clamp(0, n - 1);
                let end = if ix == n - 1 { x1 } else { (gx0 + (ix + 1) * cell).min(x1) };
                let c = m.px[(iy * n + ix) as usize].with_alpha(255);
                let lc = grid_line(c);
                if row_line {
                    f.fill_rect(x, y, end - x, 1, lc);
                } else if grid && ix > 0 && (x - gx0) % cell == 0 {
                    f.fill_rect(x, y, 1, 1, lc);
                    f.fill_rect(x + 1, y, end - x - 1, 1, c);
                } else {
                    f.fill_rect(x, y, end - x, 1, c);
                }
                x = end.max(x + 1);
            }
        },
    );
    // The pointer's pixel, outlined dark then light.
    let mid = (MAG_N / 2) as f32;
    let lw = ui.line_px();
    let (x, y) = (gx + mid * m.cell, gy + mid * m.cell);
    f.stroke_rect(x - lw / 2.0, y - lw / 2.0, m.cell + lw, m.cell + lw, lw, C4::rgb(0, 0, 0));
    f.stroke_rect(x - lw * 1.5, y - lw * 1.5, m.cell + 3.0 * lw, m.cell + 3.0 * lw, lw, C4::rgb(255, 255, 255));
    if !m.square {
        circle_line(f, cx, cy, r.w / 2.0, lw, ui.th.border);
    }
    // The caption: a dot of the color, position and hex.
    let cap = m.cap;
    chrome::surface(f, ui, cap, 6.0, &chrome::SMALL_LAYERS, 1.0);
    let c = m.px[MAG_N * MAG_N / 2];
    let ccy = cap.y + cap.h / 2.0;
    let dr = ui.px(5.0);
    chrome::swatch_r(f, ui, cap.x + ui.px(8.0) + dr, ccy, dr, c.with_alpha(255), 1.0);
    chrome::text(f, ui, 12.0, &m.text, cap.x + ui.px(8.0 + 10.0 + 6.0), ccy, ui.th.text);
}

pub fn mag_bounds(ui: &Ui, m: &MagUi) -> Vec<FRect> {
    let ui = &Ui { s: m.s, ..*ui };
    let cap = m.cap;
    // The text may run past a panel narrower than it.
    let t = FRect { x: cap.x, y: cap.y, w: cap.w + chrome::tw(ui, 12.0, &m.text) + ui.px(8.0), h: cap.h };
    vec![chrome::surface_bounds(ui, m.r), toolbar::union(chrome::surface_bounds(ui, cap), t)]
}

pub fn ring(f: &mut Fb, ui: &Ui, r: &RingUi) {
    let (cx, cy) = (r.c.x, r.c.y);
    let ro = ui.px(ring_outer());
    for &i in chrome::SMALL_LAYERS.iter().rev() {
        let (dy, sp) = (ui.px(SHADOW[i].0), ui.px(SHADOW[i].1));
        disc(f, cx, cy + dy, ro + sp, 0.0, ui.th.shadow[i]);
    }
    let lw = ui.line_px();
    disc(f, cx, cy, ro, 0.0, ui.th.surface);
    circle_line(f, cx, cy, ro, lw, ui.th.border);
    let n = r.colors.len();
    for (i, c) in r.colors.iter().enumerate() {
        let p = ring_swatch(r.c, i, n, ui.s);
        let hot = r.hot == Some(RingHit::Swatch(i));
        let rad = ui.px(if hot { RING_SW + 2.0 } else { RING_SW });
        if hot || r.cur == Some(i) {
            let ring_c = if hot { ui.th.accent } else { ui.th.text };
            f.stroke_circle(p.x, p.y, rad + ui.px(3.0), ui.px(2.0), ring_c);
        }
        chrome::swatch_r(f, ui, p.x, p.y, rad, *c, 1.0);
    }
    // The centre: the color wheel (dimmed without a toolbar to hang it on).
    let k = if r.custom { 1.0 } else { 0.4 };
    let hot = r.hot == Some(RingHit::Center) && r.custom;
    let bg = if hot { ui.th.bg_pressed } else { ui.th.bg_hover };
    f.fill_circle(cx, cy, ui.px(RING_C), bg);
    chrome::rainbow(f, ui, cx, cy, ui.px(9.0), k);
    if hot {
        let label = tr!("editor.palette.custom");
        let w = chrome::tw(ui, 11.0, label);
        chrome::text(f, ui, 11.0, label, cx - w / 2.0, cy + ui.px(RING_C + 11.0), ui.th.text_muted);
    }
}

pub fn ring_bounds(ui: &Ui, r: &RingUi) -> FRect {
    let ro = ui.px(ring_outer());
    chrome::surface_bounds(ui, FRect { x: r.c.x - ro, y: r.c.y - ro, w: 2.0 * ro, h: 2.0 * ro })
}

#[cfg(test)]
#[path = "assist_tests.rs"]
mod assist_tests;
