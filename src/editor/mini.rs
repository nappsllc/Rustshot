//! The object chrome (EditStates board): the selection's dashed accent
//! box, square handles (7 px, accent border, dark halo), round handles for
//! end points, tails and the loupe, snap guides, the marquee, the callout
//! caret, the ghost left at a drag's origin, and the 32 px mini-toolbar
//! 8 px above the selection (below it when there is no room): color,
//! fill, size, the text style (the font list, B / I / U / S, the text's
//! alignment), the callout's shape, front / back, duplicate, delete. The
//! text draft gets one too: its style only.

use super::chrome::{self, Ui};
use super::compose::{CaretUi, ObjSelUi};
use super::toolbar::{Act, Item, Kind};
use crate::objects::{CalloutStyle, FRect, Pt};
use crate::text::{Align, Family, TextFmt};
use crate::uifb::{Fb, C4};

const H: f32 = 32.0;
const BTN: f32 = 26.0;
const PAD: f32 = 3.0;
const GAP: f32 = 2.0;
const SEP_GAP: f32 = 2.0;
const SEP_LEN: f32 = 14.0;
const VALUE_W: f32 = 46.0;
/// The font box: the family's name and a chevron.
const FONT_W: f32 = 88.0;
/// The font list under (over) the bar: rows of 28 px in a 4 px padded
/// panel.
const MENU_W: f32 = 160.0;
const MENU_ROW: f32 = 28.0;
const MENU_PAD: f32 = 4.0;
const MENU_GAP: f32 = 4.0;
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
    /// The text style (a text or a callout alone, or the text draft).
    pub fmt: Option<TextFmt>,
    /// The alignment buttons (a text, not a callout).
    pub align: bool,
    /// The text draft's bar: its style only (no object buttons).
    pub draft: bool,
    /// The font list is open.
    pub font_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniUi {
    pub r: FRect,
    pub items: Vec<Item>,
    pub inp: MiniIn,
    pub hover: Option<usize>,
    /// The open font list's panel (its rows are items).
    pub pop: Option<FRect>,
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
        super::toolbar::hit(self.r, p) || self.pop.is_some_and(|r| super::toolbar::hit(r, p))
    }
}

/// Whether `act` shows as on for style `f`.
fn fmt_on(act: Act, f: Option<TextFmt>) -> bool {
    let Some(f) = f else { return false };
    match act {
        Act::ObjBold => f.bold,
        Act::ObjItalic => f.italic,
        Act::ObjUnderline => f.underline,
        Act::ObjStrike => f.strike,
        Act::ObjAlign(a) => f.align == a,
        _ => false,
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
    if inp.fmt.is_some() {
        let mut g = vec![(Kind::Btn(Act::ObjFontMenu), FONT_W)];
        g.extend([Act::ObjBold, Act::ObjItalic, Act::ObjUnderline, Act::ObjStrike].map(|a| (Kind::Btn(a), BTN)));
        groups.push(g);
        if inp.align {
            groups.push(Align::ALL.iter().map(|a| (Kind::Btn(Act::ObjAlign(*a)), BTN)).collect());
        }
    }
    if inp.style.is_some() && inp.count == 1 {
        groups.push(CalloutStyle::ALL.iter().map(|st| (Kind::Btn(Act::ObjStyle(*st)), BTN)).collect());
    }
    if !inp.draft {
        groups.push(vec![(Kind::Btn(Act::ObjFront), BTN), (Kind::Btn(Act::ObjBack), BTN)]);
        groups.push(vec![(Kind::Btn(Act::ObjDup), BTN), (Kind::Btn(Act::ObjDelete), BTN)]);
    }
    // Logical px from the bar's left, then placed and scaled. Wider than
    // the area (a long bar at a big UI size): the groups go on rows.
    let m = MARGIN * s;
    let room = ((area.w - 2.0 * m) / s).max(2.0 * BTN);
    let group_w = |g: &[(Kind, f32)]| g.iter().map(|e| e.1).sum::<f32>() + GAP * g.len().saturating_sub(1) as f32;
    let mut items = Vec::new();
    let (mut x, mut row, mut widest) = (PAD, 0usize, 0.0f32);
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            if x + 2.0 * SEP_GAP + 1.0 + group_w(g) + PAD > room {
                widest = widest.max(x);
                (x, row) = (PAD, row + 1);
            } else {
                x += SEP_GAP;
                let y = row as f32 * H + (H - SEP_LEN) / 2.0;
                items.push(Item { kind: Kind::SepV, r: FRect { x, y, w: 1.0, h: SEP_LEN }, disabled: false });
                x += 1.0 + SEP_GAP;
            }
        }
        for (j, (k, w)) in g.iter().enumerate() {
            if j > 0 {
                x += GAP;
            }
            items.push(Item { kind: *k, r: FRect { x, y: row as f32 * H + (H - BTN) / 2.0, w: *w, h: BTN }, disabled: false });
            x += w;
        }
    }
    let (bw, bh) = ((x.max(widest) + PAD) * s, (row + 1) as f32 * H * s);
    let bx = sel.x.clamp(area.x + m, (area.x1() - m - bw).max(area.x + m));
    let above = sel.y - ABOVE * s - bh;
    let by = if above >= area.y + m { above } else { (sel.y1() + ABOVE * s).min(area.y1() - m - bh) };
    for it in &mut items {
        it.r = FRect { x: bx + it.r.x * s, y: by + it.r.y * s, w: it.r.w * s, h: it.r.h * s };
    }
    let r = FRect { x: bx, y: by, w: bw, h: bh };
    // The font list: under the font box (over it when the bar is below
    // the selection and there is no room under), kept in the area.
    let pop = inp.font_open.then(|| items.iter().find(|i| i.kind == Kind::Btn(Act::ObjFontMenu)).map(|i| i.r)).flatten().map(|fb| {
        let (pw, ph) = (MENU_W * s, (Family::ALL.len() as f32 * MENU_ROW + 2.0 * MENU_PAD) * s);
        let px = fb.x.clamp(area.x + m, (area.x1() - m - pw).max(area.x + m));
        let down = r.y1() + MENU_GAP * s;
        let py = if down + ph <= area.y1() - m { down } else { r.y - MENU_GAP * s - ph };
        for (i, fam) in Family::ALL.iter().enumerate() {
            let row = FRect { x: px + MENU_PAD * s, y: py + (MENU_PAD + i as f32 * MENU_ROW) * s, w: pw - 2.0 * MENU_PAD * s, h: MENU_ROW * s };
            items.push(Item { kind: Kind::Btn(Act::ObjFont(*fam)), r: row, disabled: false });
        }
        FRect { x: px, y: py, w: pw, h: ph }
    });
    MiniUi { r, items, inp: inp.clone(), hover: None, pop }
}

pub fn bounds(ui: &Ui, m: &MiniUi) -> FRect {
    let b = chrome::surface_bounds(ui, m.r);
    match m.pop {
        Some(p) => {
            let q = chrome::surface_bounds(ui, p);
            let (x0, y0) = (b.x.min(q.x), b.y.min(q.y));
            FRect { x: x0, y: y0, w: b.x1().max(q.x1()) - x0, h: b.y1().max(q.y1()) - y0 }
        }
        None => b,
    }
}

/// Draw the mini-toolbar.
pub fn draw(f: &mut Fb, ui: &Ui, m: &MiniUi) {
    let th = ui.th;
    chrome::surface(f, ui, m.r, 9.0, &[0, 1, 2], 1.0);
    if let Some(p) = m.pop {
        chrome::panel(f, ui, chrome::round(p), 8.0, &chrome::SMALL_LAYERS, th.surface.with_alpha(255), th.border, 1.0);
    }
    for (i, it) in m.items.iter().enumerate() {
        let r = chrome::round(it.r);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let hovered = m.hover == Some(i);
        match it.kind {
            Kind::Btn(Act::ObjFontMenu) => {
                // The family's name and a chevron, as the panel's device box.
                let fam = m.inp.fmt.unwrap_or_default().family;
                if hovered || m.inp.font_open {
                    chrome::fill(f, r, ui.px(6.0), th.bg_hover);
                }
                let inner = FRect { x: r.x + ui.px(8.0), w: r.w - ui.px(12.0), ..r };
                chrome::dev_box(f, ui, inner, fam.label(), true, 1.0);
                continue;
            }
            Kind::Btn(Act::ObjFont(fam)) => {
                let on = m.inp.fmt.is_some_and(|x| x.family == fam);
                chrome::dev_item(f, ui, r, fam.label(), on, hovered, 1.0);
                continue;
            }
            _ => {}
        }
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
                    a => fmt_on(a, m.inp.fmt),
                };
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

/// A square handle at `p` and scale `s`: its halo, accent and white face
/// rects. Whole pixels first (the line, the border, the face), and every
/// layer offset from one integer origin, so the border is equally wide on
/// all four sides at any scale and position (rounding each layer's edges
/// on its own left it 1 px on one side and 2 on the other).
pub(crate) fn square_handle(p: Pt, s: f32, hot: bool) -> [FRect; 3] {
    let lw = s.round().max(1.0);
    let bd = (SQ_BORDER * s).round().max(1.0);
    let face = ((if hot { SQ + 1.0 } else { SQ }) * s).round().max(1.0);
    let outer = face + 2.0 * bd;
    let o = FRect { x: (p.x - outer / 2.0).round(), y: (p.y - outer / 2.0).round(), w: outer, h: outer };
    let g = |r: FRect, d: f32| FRect { x: r.x - d, y: r.y - d, w: r.w + 2.0 * d, h: r.h + 2.0 * d };
    [g(o, lw), o, g(o, -bd)]
}

/// The selection chrome of the objects.
/// The hover outline's opacity (the selection's is full).
const HOVER_K: f32 = 0.55;

pub fn osel(f: &mut Fb, ui: &Ui, u: &ObjSelUi) {
    let th = ui.th;
    let s = ui.s;
    let lw = s.round().max(1.0);
    for b in &u.hover {
        let r = FRect { x: b.x.round() - lw / 2.0, y: b.y.round() - lw / 2.0, w: b.w.round() + lw, h: b.h.round() + lw };
        f.stroke_dashed_rect(r, DASH.0 * s, DASH.1 * s, lw, th.accent.fade(HOVER_K));
    }
    if let Some((a, b)) = u.hover_line {
        dashed_line(f, a, b, s, th.accent.fade(0.7 * HOVER_K));
    }
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
        let [h, o, w] = square_handle(*p, s, hot);
        chrome::fill(f, h, ui.px(3.0), halo);
        chrome::fill(f, o, ui.px(2.5), th.accent);
        chrome::fill(f, w, ui.px(1.5), C4::rgb(255, 255, 255));
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
    for b in u.boxes.iter().chain(&u.hover) {
        let (x0, y0, x1, y1) = (b.x.round() - m, b.y.round() - m, b.x1().round() + m, b.y1().round() + m);
        let t = 2.0 * m;
        v.push(FRect { x: x0, y: y0, w: x1 - x0, h: t });
        v.push(FRect { x: x0, y: y1 - t, w: x1 - x0, h: t });
        v.push(FRect { x: x0, y: y0, w: t, h: y1 - y0 });
        v.push(FRect { x: x1 - t, y: y0, w: t, h: y1 - y0 });
    }
    for (a, b) in u.line.into_iter().chain(u.hover_line) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A callout's bar (the longest) at 200 % on a 1000 px area: on
    /// rows, inside the area, every button inside the bar; at 100 % one row.
    #[test]
    fn a_long_bar_wraps_to_fit_the_screen() {
        let inp = MiniIn {
            color: Some(C4::rgb(240, 68, 56)),
            fill: Some(true),
            size: Some(17.0),
            text: true,
            style: Some(CalloutStyle::Rounded),
            count: 1,
            fmt: Some(TextFmt::default()),
            align: false,
            draft: false,
            font_open: true,
        };
        let area = FRect { x: 0.0, y: 0.0, w: 1000.0, h: 800.0 };
        let sel = FRect { x: 900.0, y: 300.0, w: 200.0, h: 80.0 };
        let one = layout(&inp, sel, area, 1.0);
        assert_eq!(one.r.h, H, "one row at 100 %");
        let m = layout(&inp, sel, area, 2.0);
        assert!(m.r.h >= 2.0 * H * 2.0 && m.r.x >= area.x && m.r.x1() <= area.x1(), "{:?}", m.r);
        for it in m.items.iter().filter(|i| !matches!(i.kind, Kind::Btn(Act::ObjFont(_)))) {
            assert!(it.r.x >= m.r.x && it.r.x1() <= m.r.x1() + 0.5 && it.r.y >= m.r.y && it.r.y1() <= m.r.y1() + 0.5, "{it:?} outside {:?}", m.r);
        }
        let p = m.pop.expect("the font list");
        assert!(p.x >= area.x && p.x1() <= area.x1());
    }
}

#[cfg(test)]
mod handle_tests {
    use super::*;

    /// Every layer's edge is a whole pixel and each ring is equally wide on
    /// all four sides, at every scale and sub-pixel position.
    #[test]
    fn square_handles_are_even_all_round() {
        for s in [1.0f32, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            for k in 0..8 {
                let p = Pt::new(100.0 + k as f32 / 8.0, 57.0 + k as f32 / 5.0);
                for hot in [false, true] {
                    let [h, o, w] = square_handle(p, s, hot);
                    for r in [h, o, w] {
                        for v in [r.x, r.y, r.w, r.h] {
                            assert_eq!(v, v.round(), "whole pixels at s={s} p={p:?}: {r:?}");
                        }
                        assert_eq!(r.w, r.h, "square at s={s}");
                    }
                    let ring = |a: FRect, b: FRect| [b.x - a.x, b.y - a.y, a.x1() - b.x1(), a.y1() - b.y1()];
                    for (outer, inner) in [(h, o), (o, w)] {
                        let d = ring(outer, inner);
                        assert!(d.iter().all(|x| *x == d[0] && *x >= 1.0), "even ring at s={s} p={p:?} hot={hot}: {d:?}");
                    }
                }
            }
        }
    }
}
