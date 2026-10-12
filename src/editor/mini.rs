//! The object chrome (EditStates board): the selection's dashed accent
//! box, square handles (7 px, accent border, dark halo), round handles for
//! end points, tails and the loupe, snap guides, the marquee, the callout
//! caret, the ghost left at a drag's origin, and the 32 px mini-toolbar
//! 8 px above the selection (below it when there is no room): color,
//! fill, size, the callout's shape, front / back, duplicate, delete.

use super::chrome::{self, Ui};
use super::compose::{CaretUi, ObjSelUi};
use super::toolbar::{Act, Item, Kind};
use crate::objects::{CalloutStyle, FRect, Pt};
use crate::uifb::{Fb, C4};

const H: f32 = 32.0;
const BTN: f32 = 26.0;
const PAD: f32 = 3.0;
const GAP: f32 = 2.0;
const SEP_GAP: f32 = 2.0;
const SEP_LEN: f32 = 14.0;
const VALUE_W: f32 = 46.0;
/// Above the selection by this much.
const ABOVE: f32 = 8.0;
const MARGIN: f32 = 8.0;
/// Square handle: 7 px face, 1.5 px accent border, 1 px halo.
const SQ: f32 = 7.0;
const SQ_BORDER: f32 = 1.5;
/// Round handle: 8 px face.
const ROUND: f32 = 8.0;
/// Dashed box: 4 on, 3 off.
const DASH: (f32, f32) = (4.0, 3.0);

/// What the mini-toolbar shows for the selection.
#[derive(Clone, Debug, PartialEq)]
pub struct MiniIn {
    /// The color (`None`: no color, or a mix without one).
    pub color: Option<C4>,
    pub fill: Option<bool>,
    /// The size readout (`None`: no size) and whether it is a text size.
    pub size: Option<f32>,
    pub text: bool,
    pub style: Option<CalloutStyle>,
    /// More than one object.
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniUi {
    pub r: FRect,
    pub items: Vec<Item>,
    pub inp: MiniIn,
    pub hover: Option<usize>,
}

impl MiniUi {
    pub fn act_at(&self, p: Pt) -> Option<Act> {
        self.items.iter().find(|i| super::toolbar::hit(i.r, p) && !i.disabled).and_then(|i| match i.kind {
            Kind::Btn(a) => Some(a),
            _ => None,
        })
    }

    pub fn index_at(&self, p: Pt) -> Option<usize> {
        self.items.iter().position(|i| super::toolbar::hit(i.r, p) && matches!(i.kind, Kind::Btn(_)))
    }

    pub fn contains(&self, p: Pt) -> bool {
        super::toolbar::hit(self.r, p)
    }
}

/// Lay the mini-toolbar out over `sel` (the selection's box) in `area`.
pub fn layout(inp: &MiniIn, sel: FRect, area: FRect, s: f32) -> MiniUi {
    let mut groups: Vec<Vec<(Kind, f32)>> = Vec::new();
    let mut g = Vec::new();
    if inp.color.is_some() {
        g.push((Kind::Btn(Act::ObjColor), BTN));
    }
    if inp.fill.is_some() && inp.count == 1 {
        g.push((Kind::Btn(Act::ObjFill), BTN));
    }
    if !g.is_empty() {
        groups.push(g);
    }
    if inp.size.is_some() && inp.count == 1 {
        groups.push(vec![(Kind::Btn(Act::ObjSize(-1)), BTN), (Kind::Value, VALUE_W), (Kind::Btn(Act::ObjSize(1)), BTN)]);
    }
    if inp.style.is_some() && inp.count == 1 {
        groups.push(CalloutStyle::ALL.iter().map(|st| (Kind::Btn(Act::ObjStyle(*st)), BTN)).collect());
    }
    groups.push(vec![(Kind::Btn(Act::ObjFront), BTN), (Kind::Btn(Act::ObjBack), BTN)]);
    groups.push(vec![(Kind::Btn(Act::ObjDup), BTN), (Kind::Btn(Act::ObjDelete), BTN)]);
    // Logical px from the bar's left, then placed and scaled.
    let mut items = Vec::new();
    let mut x = PAD;
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            x += SEP_GAP;
            items.push(Item { kind: Kind::SepV, r: FRect { x, y: (H - SEP_LEN) / 2.0, w: 1.0, h: SEP_LEN }, disabled: false });
            x += 1.0 + SEP_GAP;
        }
        for (j, (k, w)) in g.iter().enumerate() {
            if j > 0 {
                x += GAP;
            }
            items.push(Item { kind: *k, r: FRect { x, y: (H - BTN) / 2.0, w: *w, h: BTN }, disabled: false });
            x += w;
        }
    }
    let (bw, bh) = ((x + PAD) * s, H * s);
    let m = MARGIN * s;
    let bx = sel.x.clamp(area.x + m, (area.x1() - m - bw).max(area.x + m));
    let above = sel.y - ABOVE * s - bh;
    let by = if above >= area.y + m { above } else { (sel.y1() + ABOVE * s).min(area.y1() - m - bh) };
    for it in &mut items {
        it.r = FRect { x: bx + it.r.x * s, y: by + it.r.y * s, w: it.r.w * s, h: it.r.h * s };
    }
    MiniUi { r: FRect { x: bx, y: by, w: bw, h: bh }, items, inp: inp.clone(), hover: None }
}

pub fn bounds(ui: &Ui, m: &MiniUi) -> FRect {
    chrome::surface_bounds(ui, m.r)
}

/// Draw the mini-toolbar.
pub fn draw(f: &mut Fb, ui: &Ui, m: &MiniUi) {
    let th = ui.th;
    chrome::surface(f, ui, m.r, 9.0, &[0, 1, 2], 1.0);
    for (i, it) in m.items.iter().enumerate() {
        let r = chrome::round(it.r);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        match it.kind {
            Kind::SepV => chrome::fill(f, r, 0.0, th.separator),
            Kind::Value => {
                let v = m.inp.size.unwrap_or(0.0).round() as i32;
                let num = v.to_string();
                if m.inp.text {
                    let (nw, uw) = (chrome::tw(ui, 12.0, &num), chrome::tw(ui, 10.5, "px"));
                    let x0 = cx - (nw + ui.px(3.0) + uw) / 2.0;
                    chrome::text(f, ui, 12.0, &num, x0, cy, th.text);
                    chrome::text(f, ui, 10.5, "px", x0 + nw + ui.px(3.0), cy, th.text_muted);
                } else {
                    // A 22 px line at the width (capped), then the number.
                    let lw = ui.px((v as f32).clamp(1.0, 6.0)).round().max(1.0);
                    let (lx, nw) = (ui.px(22.0), chrome::tw(ui, 12.0, &num));
                    let x0 = cx - (lx + ui.px(3.0) + nw) / 2.0;
                    let c = m.inp.color.unwrap_or(th.text);
                    f.fill_rounded(x0.round() as i32, (cy - lw / 2.0).round() as i32, lx.round() as i32, lw as i32, lw / 2.0, c.with_alpha(255));
                    chrome::text(f, ui, 12.0, &num, x0 + lx + ui.px(3.0), cy, th.text);
                }
            }
            Kind::Btn(act) => {
                let on = match act {
                    Act::ObjFill => m.inp.fill == Some(true),
                    Act::ObjStyle(st) => m.inp.style == Some(st),
                    _ => false,
                };
                let hovered = m.hover == Some(i);
                let rad = ui.px(6.0);
                if on {
                    chrome::fill(f, r, rad, if hovered { th.accent_bg_hover } else { th.accent_bg });
                    chrome::ring(f, ui, r, rad, th.accent_ring);
                } else if hovered {
                    chrome::fill(f, r, rad, if act == Act::ObjDelete { th.danger_bg } else { th.bg_hover });
                }
                let fg = if on {
                    th.accent_fg
                } else if hovered && act == Act::ObjDelete {
                    th.danger_fg
                } else if hovered {
                    th.icon_hover
                } else {
                    th.icon
                };
                match act {
                    Act::ObjColor => chrome::swatch_r(f, ui, cx, cy, ui.px(8.0), m.inp.color.unwrap_or(th.text), 1.0),
                    _ => {
                        let sz = ui.px(16.0);
                        crate::icon_path::draw_icon(f, chrome::act_icon(act), cx - sz / 2.0, cy - sz / 2.0, sz, fg);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A dashed line (`DASH` at scale `s`), 1 px.
fn dashed_line(f: &mut Fb, a: Pt, b: Pt, s: f32, c: C4) {
    let len = (b.x - a.x).hypot(b.y - a.y);
    if len < 0.5 {
        return;
    }
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    let (on, off) = (DASH.0 * s, DASH.1 * s);
    let w = s.round().max(1.0);
    let mut t = 0.0;
    let mut sf = f.surf();
    while t < len {
        let e = (t + on).min(len);
        let p0 = Pt::new(a.x + ux * t, a.y + uy * t);
        let p1 = Pt::new(a.x + ux * e, a.y + uy * e);
        sf.stroke_polyline(&[p0, p1], w, c, crate::raster::Blend::Normal);
        t = e + off;
    }
}

/// The selection chrome of the objects.
pub fn osel(f: &mut Fb, ui: &Ui, u: &ObjSelUi) {
    let th = ui.th;
    let s = ui.s;
    let lw = s.round().max(1.0);
    for b in &u.boxes {
        // On the pixel grid, half a line out (the board's x.5 rect).
        let r = FRect { x: b.x.round() - lw / 2.0, y: b.y.round() - lw / 2.0, w: b.w.round() + lw, h: b.h.round() + lw };
        f.stroke_dashed_rect(r, DASH.0 * s, DASH.1 * s, lw, th.accent);
    }
    if let Some((a, b)) = u.line {
        dashed_line(f, a, b, s, th.accent.fade(0.7));
    }
    let halo = C4::new(0, 0, 0, 115);
    for (i, p) in u.square.iter().enumerate() {
        let hot = u.hot == Some((false, i));
        let face = ui.px(if hot { SQ + 1.0 } else { SQ });
        let bd = ui.px(SQ_BORDER);
        let (x, y) = (p.x.round(), p.y.round());
        let outer = face + 2.0 * bd;
        let o = FRect { x: x - outer / 2.0, y: y - outer / 2.0, w: outer, h: outer };
        let g = |r: FRect, d: f32| FRect { x: r.x - d, y: r.y - d, w: r.w + 2.0 * d, h: r.h + 2.0 * d };
        chrome::fill(f, chrome::round(g(o, lw)), ui.px(3.0), halo);
        chrome::fill(f, chrome::round(o), ui.px(2.5), th.accent);
        chrome::fill(f, chrome::round(g(o, -bd)), ui.px(1.5), C4::rgb(255, 255, 255));
    }
    for (i, p) in u.round.iter().enumerate() {
        let hot = u.hot == Some((true, i));
        let r = ui.px(if hot { ROUND / 2.0 + 0.5 } else { ROUND / 2.0 });
        let bd = ui.px(SQ_BORDER);
        f.fill_circle(p.x, p.y, r + bd + lw, halo);
        f.fill_circle(p.x, p.y, r + bd, th.accent);
        f.fill_circle(p.x, p.y, r, C4::rgb(255, 255, 255));
    }
}

/// Every pixel `osel` touches, as rects (the boxes' four edges apart, so
/// a drag repaints the outline, not the area inside it).
pub fn osel_bounds(ui: &Ui, u: &ObjSelUi) -> Vec<FRect> {
    let s = ui.s;
    let lw = s.round().max(1.0);
    let m = lw + 2.0;
    let mut v = Vec::new();
    for b in &u.boxes {
        let (x0, y0, x1, y1) = (b.x.round() - m, b.y.round() - m, b.x1().round() + m, b.y1().round() + m);
        let t = 2.0 * m;
        v.push(FRect { x: x0, y: y0, w: x1 - x0, h: t });
        v.push(FRect { x: x0, y: y1 - t, w: x1 - x0, h: t });
        v.push(FRect { x: x0, y: y0, w: t, h: y1 - y0 });
        v.push(FRect { x: x1 - t, y: y0, w: t, h: y1 - y0 });
    }
    if let Some((a, b)) = u.line {
        let (x0, y0) = (a.x.min(b.x) - m, a.y.min(b.y) - m);
        v.push(FRect { x: x0, y: y0, w: (a.x - b.x).abs() + 2.0 * m, h: (a.y - b.y).abs() + 2.0 * m });
    }
    let hs = ui.px(SQ + 1.0 + 2.0 * SQ_BORDER) / 2.0 + lw + 2.0;
    for p in u.square.iter().chain(&u.round) {
        v.push(FRect { x: p.x.round() - hs, y: p.y.round() - hs, w: 2.0 * hs, h: 2.0 * hs });
    }
    v
}

/// Snap guides: 1 px accent lines.
pub fn guides(f: &mut Fb, ui: &Ui, g: &[(Pt, Pt)], k: f32) {
    let lw = ui.line_px();
    for (a, b) in g {
        let r = guide_rect(*a, *b, lw);
        f.fill_rect(r.x as i32, r.y as i32, r.w as i32, r.h as i32, ui.th.accent.fade(k));
    }
}

fn guide_rect(a: Pt, b: Pt, lw: f32) -> FRect {
    if (a.x - b.x).abs() < 0.5 {
        let x = a.x.round();
        FRect { x, y: a.y.min(b.y).round(), w: lw, h: (a.y - b.y).abs().round().max(1.0) }
    } else {
        let y = a.y.round();
        FRect { x: a.x.min(b.x).round(), y, w: (a.x - b.x).abs().round().max(1.0), h: lw }
    }
}

pub fn guides_bounds(ui: &Ui, g: &[(Pt, Pt)]) -> Vec<FRect> {
    let lw = ui.line_px();
    g.iter().map(|(a, b)| guide_rect(*a, *b, lw)).map(|r| FRect { x: r.x - 1.0, y: r.y - 1.0, w: r.w + 2.0, h: r.h + 2.0 }).collect()
}

/// The marquee: accent at 10 %, a 1 px dashed accent border.
pub fn marquee(f: &mut Fb, ui: &Ui, r: FRect) {
    let r = chrome::round(r);
    f.fill_rect(r.x as i32, r.y as i32, r.w as i32, r.h as i32, ui.th.accent.with_alpha(26));
    let lw = ui.line_px();
    f.stroke_dashed_rect(FRect { x: r.x + lw / 2.0, y: r.y + lw / 2.0, w: r.w - lw, h: r.h - lw }, DASH.0 * ui.s, DASH.1 * ui.s, lw, ui.th.accent);
}

pub fn marquee_bounds(r: FRect) -> FRect {
    let r = chrome::round(r);
    FRect { x: r.x - 2.0, y: r.y - 2.0, w: r.w + 4.0, h: r.h + 4.0 }
}

pub fn caret(f: &mut Fb, c: &CaretUi) {
    let r = chrome::round(c.r);
    f.fill_rect(r.x as i32, r.y as i32, r.w.max(1.0) as i32, r.h as i32, c.color);
}

pub fn caret_bounds(c: &CaretUi) -> FRect {
    let r = chrome::round(c.r);
    FRect { x: r.x - 1.0, y: r.y - 1.0, w: r.w + 2.0, h: r.h + 2.0 }
}
