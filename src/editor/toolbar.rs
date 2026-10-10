//! Toolbar model: four button groups, one- or two-row layout, placement
//! around the selection, the palette popover and hit-testing. Sizes are
//! logical px from the design spec; `Input::s` scales them to image pixels.

use super::Tool;
use crate::objects::{FRect, Pt};
use crate::rec::RecFormat;
use crate::uifb::C4;

pub const BTN: f32 = 32.0;
pub const PAD: f32 = 6.0;
pub const MARGIN: f32 = 12.0;
const IN_GAP: f32 = 2.0;
const GROUP_GAP: f32 = 6.0;
const SEP_LEN: f32 = 16.0;
const VALUE_W: f32 = 56.0;
/// The busy-mode OK button spans the three output buttons it replaces.
const ACCEPT_W: f32 = 3.0 * BTN + 2.0 * IN_GAP;
const ROW_GAP: f32 = 4.0;
const SEL_GAP: f32 = 8.0;
const NARROW: f32 = 260.0;
const WRAP_SLACK: f32 = 48.0;
const INSIDE_MIN_H: f32 = 120.0;
const DOT_BTN: f32 = 28.0;
const DOT_GAP: f32 = 4.0;
const POP_GAP: f32 = 6.0;
const LABEL_H: f32 = 22.0;
const LABEL_GAP: f32 = 8.0;
const LABEL_MIN_ROOM: f32 = 30.0;

pub const TOOL_ORDER: [Tool; 9] = [
    Tool::Path,
    Tool::Line,
    Tool::Arrow,
    Tool::Rect,
    Tool::Ellipse,
    Tool::Marker,
    Tool::Text,
    Tool::Pixelate,
    Tool::Invert,
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Tool(Tool),
    Undo,
    Redo,
    Size(i32),
    Color(C4),
    Palette,
    Copy,
    Save,
    /// Keyboard only (not on the toolbar): Save As dialog.
    SaveAs,
    Upload,
    Exit,
    Accept,
    /// The Record button: opens/closes the Record popover.
    Record,
    RecFormat(RecFormat),
    RecFps(u32),
    RecSystem,
    RecMic,
    RecCamera,
    /// Open/close the microphone list.
    RecDevMenu,
    /// Pick microphone `i` of the list (0 = the default device).
    RecDev(usize),
    /// "Start recording".
    RecStart,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Btn(Act),
    /// The stroke-size readout ("3 line").
    Value,
    SepV,
    SepH,
    /// A palette color in the popover.
    Dot(C4),
    /// Record popover: a segmented control's track.
    SegTrack,
    /// A segment (`on` = chosen).
    Seg { act: Act, label: &'static str, on: bool },
    /// Muted text.
    Label(&'static str),
    /// A row with a label and an on/off switch (the whole row toggles).
    Switch { act: Act, label: &'static str, on: bool },
    /// The microphone dropdown box (shows `Toolbar::texts[0]`).
    DevBox { open: bool },
    /// The open microphone list's panel.
    DevList,
    /// Microphone `i` of the list (`Toolbar::texts[i + 1]`).
    DevItem { i: usize, on: bool },
    /// The popover's primary button.
    Primary { act: Act, label: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub r: FRect,
    pub disabled: bool,
}

pub struct Input<'a> {
    pub sel: FRect,
    /// Work area (the active monitor) in image px.
    pub area: FRect,
    pub s: f32,
    /// Export tasks were given up front: Copy/Save/Upload collapse into OK.
    pub busy: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Palette colors while the popover is open.
    pub palette: Option<&'a [C4]>,
    /// Show the Record button (recording works on this platform).
    pub record: bool,
    /// The Record popover while it is open (instead of the palette).
    pub rec: Option<&'a RecView<'a>>,
}

/// What the Record popover shows.
#[derive(Clone, Debug, PartialEq)]
pub struct RecView<'a> {
    pub format: RecFormat,
    pub fps: u32,
    pub fps_choices: [u32; 2],
    pub system: bool,
    pub mic: bool,
    pub camera: bool,
    /// The chosen microphone's name ("Default microphone" for the default).
    pub mic_label: &'a str,
    /// The microphone list while it is open: the default first.
    pub devs: Option<&'a [String]>,
    /// Index into `devs` of the chosen one.
    pub dev_sel: usize,
}

#[derive(Clone, PartialEq)]
pub struct Toolbar {
    pub bar: FRect,
    pub pop: Option<FRect>,
    pub items: Vec<Item>,
    pub wrapped: bool,
    /// The bar's anchor edge is its bottom (placed above / inside).
    pub above: bool,
    /// Texts of the Record popover's microphone box and list.
    pub texts: Vec<String>,
}

type Group = Vec<(Kind, bool, f32)>;

fn groups(inp: &Input) -> [Group; 4] {
    let tools = TOOL_ORDER.iter().map(|t| (Kind::Btn(Act::Tool(*t)), false, BTN)).collect();
    let history = vec![
        (Kind::Btn(Act::Undo), !inp.can_undo, BTN),
        (Kind::Btn(Act::Redo), !inp.can_redo, BTN),
    ];
    let size = vec![
        (Kind::Btn(Act::Size(-1)), false, BTN),
        (Kind::Value, false, VALUE_W),
        (Kind::Btn(Act::Size(1)), false, BTN),
        (Kind::Btn(Act::Palette), false, BTN),
    ];
    let mut out: Group = if inp.busy {
        vec![(Kind::Btn(Act::Accept), false, ACCEPT_W)]
    } else {
        let mut v = Vec::new();
        if inp.record {
            v.push((Kind::Btn(Act::Record), false, BTN));
        }
        v.extend([
            (Kind::Btn(Act::Copy), false, BTN),
            (Kind::Btn(Act::Save), false, BTN),
            (Kind::Btn(Act::Upload), false, BTN),
        ]);
        v
    };
    out.push((Kind::Btn(Act::Exit), false, BTN));
    [tools, history, size, out]
}

fn group_w(g: &Group) -> f32 {
    g.iter().map(|e| e.2).sum::<f32>() + IN_GAP * g.len().saturating_sub(1) as f32
}

fn row_w(gs: &[Group]) -> f32 {
    gs.iter().map(group_w).sum::<f32>() + gs.len().saturating_sub(1) as f32 * (2.0 * GROUP_GAP + 1.0)
}

/// Lay `gs` out left to right from (x0, y) with separators between groups.
fn place_row(gs: &[Group], x0: f32, y: f32, items: &mut Vec<Item>) {
    let mut x = x0;
    for (gi, g) in gs.iter().enumerate() {
        if gi > 0 {
            x += GROUP_GAP;
            let r = FRect { x, y: y + (BTN - SEP_LEN) / 2.0, w: 1.0, h: SEP_LEN };
            items.push(Item { kind: Kind::SepV, r, disabled: false });
            x += 1.0 + GROUP_GAP;
        }
        for (j, (kind, disabled, w)) in g.iter().enumerate() {
            if j > 0 {
                x += IN_GAP;
            }
            items.push(Item { kind: *kind, r: FRect { x, y, w: *w, h: BTN }, disabled: *disabled });
            x += w;
        }
    }
}

pub fn layout(inp: &Input) -> Toolbar {
    let s = inp.s;
    let gs = groups(inp);
    let area = inp.area;
    let m = MARGIN * s;
    let one_row = row_w(&gs) + 2.0 * PAD;
    let room = (inp.sel.w.max(NARROW * s) + WRAP_SLACK * s).min(area.w - 2.0 * m);
    let wrapped = one_row * s > room;

    let mut items = Vec::new();
    let (w, h) = if !wrapped {
        place_row(&gs, PAD, PAD, &mut items);
        (one_row, BTN + 2.0 * PAD)
    } else {
        let (w1, w2) = (row_w(&gs[..2]), row_w(&gs[2..]));
        let inner = w1.max(w2);
        place_row(&gs[..2], PAD, PAD, &mut items);
        let rule_y = PAD + BTN + ROW_GAP;
        let rule = FRect { x: PAD + 4.0, y: rule_y, w: inner - 8.0, h: 1.0 };
        items.push(Item { kind: Kind::SepH, r: rule, disabled: false });
        place_row(&gs[2..], PAD + inner - w2, rule_y + 1.0 + ROW_GAP, &mut items);
        (inner + 2.0 * PAD, 2.0 * BTN + 2.0 * ROW_GAP + 1.0 + 2.0 * PAD)
    };

    let (bw, bh) = (w * s, h * s);
    let sel = inp.sel;
    let g = SEL_GAP * s;
    let narrow = sel.w < NARROW * s;
    let x = if narrow { sel.x + sel.w / 2.0 - bw / 2.0 } else { sel.x1() - bw };
    let x = x.clamp(area.x + m, (area.x1() - m - bw).max(area.x + m));
    let (y, above) = if sel.y1() + g + bh <= area.y1() - m {
        (sel.y1() + g, false)
    } else if sel.y - g - bh >= area.y + m {
        (sel.y - g - bh, true)
    } else if sel.h > INSIDE_MIN_H * s {
        ((sel.y1() - g - bh).clamp(area.y + m, (area.y1() - m - bh).max(area.y + m)), true)
    } else {
        ((sel.y1() + g).clamp(area.y + m, (area.y1() - m - bh).max(area.y + m)), false)
    };
    for it in &mut items {
        it.r = FRect { x: x + it.r.x * s, y: y + it.r.y * s, w: it.r.w * s, h: it.r.h * s };
    }
    let bar = FRect { x, y, w: bw, h: bh };
    let mut texts = Vec::new();
    let pop = match (inp.rec, inp.palette) {
        (Some(rv), _) => Some(place_rec(rv, bar, above, &mut items, &mut texts, inp)),
        (None, Some(colors)) => Some(place_palette(colors, bar, above, &mut items, inp)),
        (None, None) => None,
    };
    Toolbar { bar, pop, items, wrapped, above, texts }
}

/// Top-left of a `pw`x`ph` popover 6 px from the bar on the side away from
/// the selection (below a bar below it, above a bar above/inside it), or
/// the other side when it does not fit there; right-aligned to `anchor`.
/// When it fits on neither side it goes where there is more room,
/// clamped to the area.
fn place_pop(pw: f32, ph: f32, anchor: FRect, bar: FRect, above: bool, inp: &Input) -> (f32, f32) {
    let s = inp.s;
    let m = MARGIN * s;
    let px = (anchor.x1() - pw).clamp(inp.area.x + m, (inp.area.x1() - m - pw).max(inp.area.x + m));
    let up = bar.y - POP_GAP * s - ph;
    let down = bar.y1() + POP_GAP * s;
    let up_ok = up >= inp.area.y + m;
    let down_ok = down + ph <= inp.area.y1() - m;
    let py = match (up_ok, down_ok) {
        (true, true) => {
            if above {
                up
            } else {
                down
            }
        }
        (true, false) => up,
        (false, true) => down,
        (false, false) => {
            let lo = inp.area.y + m;
            let hi = (inp.area.y1() - m - ph).max(lo);
            if bar.y - inp.area.y > inp.area.y1() - bar.y1() { up.clamp(lo, hi) } else { down.clamp(lo, hi) }
        }
    };
    (px, py)
}

const REC_W: f32 = 256.0;
const REC_PAD: f32 = 12.0;
const SEG_H: f32 = 28.0;
const SWITCH_H: f32 = 32.0;
const DEV_H: f32 = 28.0;
const DEV_ITEM_H: f32 = 26.0;
/// At most this many microphones are listed.
const DEV_MAX: usize = 8;
const START_H: f32 = 32.0;

/// Segment labels for the frame rates on offer.
fn fps_label(n: u32) -> &'static str {
    match n {
        10 => "10 fps",
        15 => "15 fps",
        30 => "30 fps",
        60 => "60 fps",
        _ => "fps",
    }
}

/// The Record popover: format, frame rate, sound, microphone, camera and
/// Start recording, in a column `REC_W` wide; placed like the palette.
fn place_rec(rv: &RecView, bar: FRect, above: bool, items: &mut Vec<Item>, texts: &mut Vec<String>, inp: &Input) -> FRect {
    let s = inp.s;
    let mut local: Vec<Item> = Vec::new();
    let mut push = |kind: Kind, r: FRect, disabled: bool| local.push(Item { kind, r, disabled });
    let inner = REC_W - 2.0 * REC_PAD;
    let x0 = REC_PAD;
    let mut y = REC_PAD;
    // Format: MP4 | GIF across the width.
    push(Kind::SegTrack, FRect { x: x0, y, w: inner, h: SEG_H }, false);
    let half = (inner - 4.0) / 2.0;
    for (i, (f, label)) in [(RecFormat::Mp4, "MP4"), (RecFormat::Gif, "GIF")].into_iter().enumerate() {
        let r = FRect { x: x0 + 2.0 + i as f32 * half, y: y + 2.0, w: half, h: SEG_H - 4.0 };
        push(Kind::Seg { act: Act::RecFormat(f), label, on: rv.format == f }, r, false);
    }
    y += SEG_H + 10.0;
    // Frame rate: label left, two segments right.
    push(Kind::Label("Frame rate"), FRect { x: x0, y, w: inner / 2.0, h: SEG_H }, false);
    let fw = 128.0;
    let fx = x0 + inner - fw;
    push(Kind::SegTrack, FRect { x: fx, y, w: fw, h: SEG_H }, false);
    let fh = (fw - 4.0) / 2.0;
    for (i, n) in rv.fps_choices.into_iter().enumerate() {
        let r = FRect { x: fx + 2.0 + i as f32 * fh, y: y + 2.0, w: fh, h: SEG_H - 4.0 };
        push(Kind::Seg { act: Act::RecFps(n), label: fps_label(n), on: rv.fps == n }, r, false);
    }
    y += SEG_H + 8.0;
    push(Kind::SepH, FRect { x: x0, y, w: inner, h: 1.0 }, false);
    y += 1.0 + 4.0;
    let audio = rv.format == RecFormat::Mp4;
    let row = |y: f32| FRect { x: x0 - 4.0, y, w: inner + 8.0, h: SWITCH_H };
    push(Kind::Switch { act: Act::RecSystem, label: "System sound", on: audio && rv.system }, row(y), !audio);
    y += SWITCH_H;
    push(Kind::Switch { act: Act::RecMic, label: "Microphone", on: audio && rv.mic }, row(y), !audio);
    y += SWITCH_H;
    texts.push(rv.mic_label.to_string());
    let mut list = None;
    if audio && rv.mic {
        y += 2.0;
        let open = rv.devs.is_some();
        push(Kind::DevBox { open }, FRect { x: x0, y, w: inner, h: DEV_H }, false);
        y += DEV_H;
        if let Some(devs) = rv.devs {
            let n = devs.len().min(DEV_MAX);
            let lr = FRect { x: x0, y: y + 4.0, w: inner, h: n as f32 * DEV_ITEM_H + 8.0 };
            list = Some((lr, n));
            texts.extend(devs.iter().take(n).cloned());
        }
        y += 6.0;
    }
    // Camera: wired up with the camera bubble.
    push(Kind::Switch { act: Act::RecCamera, label: "Camera", on: rv.camera }, row(y), true);
    y += SWITCH_H + 8.0;
    push(Kind::Primary { act: Act::RecStart, label: "Start recording" }, FRect { x: x0, y, w: inner, h: START_H }, false);
    y += START_H + REC_PAD;
    // The open list floats over the rows below the box: drawn (and hit) last.
    let mut h = y;
    if let Some((lr, n)) = list {
        h = h.max(lr.y1() + REC_PAD);
        push(Kind::DevList, lr, false);
        for i in 0..n {
            let r = FRect { x: lr.x + 4.0, y: lr.y + 4.0 + i as f32 * DEV_ITEM_H, w: lr.w - 8.0, h: DEV_ITEM_H };
            push(Kind::DevItem { i, on: i == rv.dev_sel }, r, false);
        }
    }
    let (pw, ph) = (REC_W * s, h * s);
    let anchor = items.iter().find(|it| it.kind == Kind::Btn(Act::Record)).map(|it| it.r).unwrap_or(bar);
    let (px, py) = place_pop(pw, ph, anchor, bar, above, inp);
    for it in local {
        let r = FRect { x: px + it.r.x * s, y: py + it.r.y * s, w: it.r.w * s, h: it.r.h * s };
        items.push(Item { r, ..it });
    }
    FRect { x: px, y: py, w: pw, h: ph }
}

/// Popover 6 px from the bar on the side away from the selection (below a
/// bar that sits below it, above a bar that sits above/inside it), falling
/// back to the other side when it does not fit; right-aligned to the swatch
/// button, one row of dots.
fn place_palette(colors: &[C4], bar: FRect, above: bool, items: &mut Vec<Item>, inp: &Input) -> FRect {
    let s = inp.s;
    let m = MARGIN * s;
    let n = colors.len().max(1) as f32;
    let pw = (n * DOT_BTN + (n - 1.0) * DOT_GAP + 2.0 * PAD) * s;
    let ph = (DOT_BTN + 2.0 * PAD) * s;
    let anchor = items
        .iter()
        .find(|it| it.kind == Kind::Btn(Act::Palette))
        .map(|it| it.r)
        .unwrap_or(bar);
    let px = (anchor.x1() - pw).clamp(inp.area.x + m, (inp.area.x1() - m - pw).max(inp.area.x + m));
    let up = bar.y - POP_GAP * s - ph;
    let down = bar.y1() + POP_GAP * s;
    let py = if above {
        if up < inp.area.y + m { down } else { up }
    } else if down + ph > inp.area.y1() - m {
        up
    } else {
        down
    };
    for (i, c) in colors.iter().enumerate() {
        let r = FRect {
            x: px + (PAD + i as f32 * (DOT_BTN + DOT_GAP)) * s,
            y: py + PAD * s,
            w: DOT_BTN * s,
            h: DOT_BTN * s,
        };
        items.push(Item { kind: Kind::Dot(*c), r, disabled: false });
    }
    FRect { x: px, y: py, w: pw, h: ph }
}

impl Toolbar {
    /// Index of the item under `p` (popover dots win over the bar).
    pub fn index_at(&self, p: Pt) -> Option<usize> {
        self.items.iter().rposition(|it| hit(it.r, p))
    }

    pub fn act_at(&self, p: Pt) -> Option<Act> {
        let it = &self.items[self.index_at(p)?];
        if it.disabled {
            return None;
        }
        match it.kind {
            Kind::Btn(a) => Some(a),
            Kind::Dot(c) => Some(Act::Color(c)),
            Kind::Seg { act, .. } | Kind::Switch { act, .. } | Kind::Primary { act, .. } => Some(act),
            Kind::DevBox { .. } => Some(Act::RecDevMenu),
            Kind::DevItem { i, .. } => Some(Act::RecDev(i)),
            Kind::Value | Kind::SepV | Kind::SepH | Kind::SegTrack | Kind::Label(_) | Kind::DevList => None,
        }
    }

    /// True over the bar or the open popover: the press is consumed.
    pub fn contains(&self, p: Pt) -> bool {
        hit(self.bar, p) || self.pop.is_some_and(|r| hit(r, p))
    }

    /// A copy scaled by `z` about the anchor edge (scale-in animation).
    pub fn scaled(&self, z: f32) -> Toolbar {
        let ax = self.bar.x + self.bar.w / 2.0;
        let ay = if self.above { self.bar.y1() } else { self.bar.y };
        let f = |r: FRect| FRect { x: ax + (r.x - ax) * z, y: ay + (r.y - ay) * z, w: r.w * z, h: r.h * z };
        Toolbar {
            bar: f(self.bar),
            pop: self.pop.map(f),
            items: self.items.iter().map(|it| Item { r: f(it.r), ..*it }).collect(),
            wrapped: self.wrapped,
            above: self.above,
            texts: self.texts.clone(),
        }
    }
}

/// Size label: 8 above the selection's top-left, or 8 inside it when there
/// is less than 30 px of the work area above or the placement above would
/// overlap `avoid` (the toolbar and popover when they sit above).
pub fn label_rect(sel: FRect, w: f32, s: f32, area: FRect, avoid: Option<FRect>) -> FRect {
    let h = LABEL_H * s;
    let inside = (sel.x + LABEL_GAP * s, sel.y + LABEL_GAP * s);
    let (mut x, mut y) = if sel.y - area.y < LABEL_MIN_ROOM * s {
        inside
    } else {
        (sel.x, sel.y - LABEL_GAP * s - h)
    };
    x = x.clamp(area.x, (area.x1() - w).max(area.x));
    if let Some(a) = avoid {
        let hits = x < a.x1() && x + w > a.x && y < a.y1() && y + h > a.y;
        if hits {
            (x, y) = inside;
            x = x.clamp(area.x, (area.x1() - w).max(area.x));
        }
    }
    FRect { x, y, w, h }
}

pub(super) fn union(a: FRect, b: FRect) -> FRect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    FRect { x, y, w: a.x1().max(b.x1()) - x, h: a.y1().max(b.y1()) - y }
}

pub fn hit(r: FRect, p: Pt) -> bool {
    p.x >= r.x && p.x < r.x1() && p.y >= r.y && p.y < r.y1()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: FRect = FRect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };

    fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
        FRect { x, y, w, h }
    }

    fn inp<'a>(sel: FRect) -> Input<'a> {
        Input { sel, area: AREA, s: 1.0, busy: false, can_undo: true, can_redo: true, palette: None, record: false, rec: None }
    }

    fn btn(tb: &Toolbar, a: Act) -> FRect {
        tb.items.iter().find(|i| i.kind == Kind::Btn(a)).expect("button").r
    }

    #[test]
    fn single_row_metrics() {
        let tb = layout(&inp(r(100.0, 100.0, 800.0, 300.0)));
        assert!(!tb.wrapped);
        assert_eq!((tb.bar.w, tb.bar.h), (713.0, 44.0));
    }

    #[test]
    fn busy_swaps_outputs_for_ok_and_keeps_width() {
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.busy = true;
        let tb = layout(&i);
        assert_eq!(tb.bar.w, 713.0);
        assert!(tb.items.iter().any(|it| it.kind == Kind::Btn(Act::Accept)));
        assert!(!tb.items.iter().any(|it| it.kind == Kind::Btn(Act::Copy)));
    }

    #[test]
    fn wraps_on_narrow_selection() {
        let tb = layout(&inp(r(100.0, 100.0, 440.0, 170.0)));
        assert!(tb.wrapped);
        assert_eq!((tb.bar.w, tb.bar.h), (395.0, 85.0));
        assert!(tb.items.iter().any(|i| i.kind == Kind::SepH));
        assert!(btn(&tb, Act::Exit).y > btn(&tb, Act::Undo).y, "actions on row 2");
        assert!((btn(&tb, Act::Exit).x1() - (tb.bar.x1() - PAD)).abs() < 1e-3, "row 2 right-aligned");
    }

    #[test]
    fn below_and_right_aligned() {
        let tb = layout(&inp(r(100.0, 100.0, 800.0, 300.0)));
        assert_eq!(tb.bar.y, 408.0);
        assert_eq!(tb.bar.x1(), 900.0);
        assert!(!tb.above);
    }

    #[test]
    fn flips_above_near_bottom() {
        let tb = layout(&inp(r(100.0, 700.0, 800.0, 340.0)));
        assert_eq!(tb.bar.y, 648.0);
        assert!(tb.above);
    }

    #[test]
    fn goes_inside_fullscreen_selection() {
        let tb = layout(&inp(r(0.0, 0.0, 1920.0, 1080.0)));
        assert_eq!(tb.bar.y1(), 1068.0, "clamped to the 12 px margin");
        assert_eq!(tb.bar.x1(), 1908.0, "clamped to the 12 px margin");
    }

    #[test]
    fn clamps_to_left_margin() {
        let tb = layout(&inp(r(0.0, 100.0, 300.0, 200.0)));
        assert_eq!(tb.bar.x, 12.0);
    }

    #[test]
    fn tiny_selection_centred_and_not_covered() {
        let sel = r(900.0, 500.0, 40.0, 20.0);
        let tb = layout(&inp(sel));
        assert!((tb.bar.x + tb.bar.w / 2.0 - 920.0).abs() < 1e-3);
        assert!(tb.bar.y >= sel.y1());
    }

    #[test]
    fn scales_with_dpi() {
        let mut i = inp(r(100.0, 100.0, 1600.0, 300.0));
        i.s = 2.0;
        let tb = layout(&i);
        assert!(!tb.wrapped);
        assert_eq!(tb.bar.w, 1426.0);
        assert_eq!(btn(&tb, Act::Undo).w, 64.0);
    }

    #[test]
    fn disabled_buttons_do_not_act_but_consume() {
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.can_undo = false;
        let tb = layout(&i);
        let u = btn(&tb, Act::Undo);
        let p = Pt::new(u.x + 5.0, u.y + 5.0);
        assert_eq!(tb.act_at(p), None);
        assert!(tb.contains(p));
        let c = btn(&tb, Act::Copy);
        assert_eq!(tb.act_at(Pt::new(c.x + 5.0, c.y + 5.0)), Some(Act::Copy));
    }

    #[test]
    fn palette_pops_below_bar_right_aligned() {
        let colors = [C4::rgb(1, 2, 3); 10];
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.palette = Some(&colors);
        let tb = layout(&i);
        assert!(!tb.above);
        let pop = tb.pop.expect("popover");
        assert_eq!((pop.w, pop.h), (328.0, 40.0));
        assert_eq!(pop.y, tb.bar.y1() + 6.0);
        assert_eq!(pop.x1(), btn(&tb, Act::Palette).x1());
        let dots: Vec<&Item> = tb.items.iter().filter(|it| matches!(it.kind, Kind::Dot(_))).collect();
        assert_eq!(dots.len(), 10);
        let d = dots[0].r;
        assert_eq!(tb.act_at(Pt::new(d.x + 3.0, d.y + 3.0)), Some(Act::Color(C4::rgb(1, 2, 3))));
    }

    #[test]
    fn palette_above_bar_when_bar_flips_above() {
        let colors = [C4::rgb(1, 2, 3); 10];
        let mut i = inp(r(100.0, 700.0, 800.0, 340.0));
        i.palette = Some(&colors);
        let tb = layout(&i);
        assert!(tb.above);
        assert_eq!(tb.pop.unwrap().y1(), tb.bar.y - 6.0);
    }

    #[test]
    fn palette_above_bar_when_no_room_below_it() {
        let colors = [C4::rgb(1, 2, 3); 10];
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.area = r(0.0, 0.0, 1920.0, 464.0); // bar fits (ends at 452) but the popover does not
        i.palette = Some(&colors);
        let tb = layout(&i);
        assert!(!tb.above);
        assert_eq!(tb.pop.unwrap().y1(), tb.bar.y - 6.0);
    }

    #[test]
    fn palette_drops_below_when_no_room_above() {
        let colors = [C4::rgb(1, 2, 3); 10];
        let mut i = inp(r(100.0, 10.0, 800.0, 200.0));
        i.area = r(0.0, 0.0, 1920.0, 68.0);
        i.palette = Some(&colors);
        let tb = layout(&i);
        assert!(tb.above);
        assert_eq!(tb.pop.unwrap().y, tb.bar.y1() + 6.0);
    }

    #[test]
    fn label_above_or_inside() {
        let l = label_rect(r(240.0, 140.0, 960.0, 540.0), 120.0, 1.0, AREA, None);
        assert_eq!((l.x, l.y, l.h), (240.0, 110.0, 22.0));
        let l = label_rect(r(300.0, 0.0, 664.0, 110.0), 120.0, 1.0, AREA, None);
        assert_eq!((l.x, l.y), (308.0, 8.0));
    }

    #[test]
    fn area_offset_clamps_inside_monitor() {
        let area = r(1920.0, 0.0, 1280.0, 720.0);
        let mut i = inp(r(1920.0, 600.0, 1280.0, 120.0));
        i.area = area;
        let tb = layout(&i);
        assert!(tb.above);
        assert!(tb.bar.x >= 1932.0, "{:?}", tb.bar);
        assert!(tb.bar.x1() <= 3188.0, "{:?}", tb.bar);
    }

    #[test]
    fn label_inside_when_no_room_in_area() {
        let area = r(0.0, 200.0, 1920.0, 880.0);
        let l = label_rect(r(300.0, 210.0, 664.0, 300.0), 120.0, 1.0, area, None);
        assert_eq!((l.x, l.y), (308.0, 218.0));
    }

    #[test]
    fn label_moves_inside_when_bar_above_covers_it() {
        let sel = r(900.0, 950.0, 200.0, 100.0);
        let tb = layout(&inp(sel));
        assert!(tb.above && tb.bar.y1() <= sel.y);
        let l = label_rect(sel, 120.0, 1.0, AREA, Some(tb.bar));
        assert_eq!((l.x, l.y), (908.0, 958.0));
        let l = label_rect(sel, 120.0, 1.0, AREA, None);
        assert_eq!((l.x, l.y), (900.0, 920.0));
    }

    #[test]
    fn scaled_shrinks_toward_anchor_edge() {
        let tb = layout(&inp(r(100.0, 100.0, 800.0, 300.0)));
        let z = tb.scaled(0.5);
        assert_eq!(z.bar.y, tb.bar.y);
        assert!((z.bar.w - tb.bar.w * 0.5).abs() < 1e-3);
    }

    fn rec_view(format: RecFormat, mic: bool, devs: Option<&[String]>) -> RecView<'_> {
        RecView {
            format,
            fps: if format == RecFormat::Gif { 15 } else { 30 },
            fps_choices: if format == RecFormat::Gif { [10, 15] } else { [30, 60] },
            system: true,
            mic,
            camera: false,
            mic_label: "Default microphone",
            devs,
            dev_sel: 0,
        }
    }

    fn kind_at(tb: &Toolbar, f: impl Fn(&Kind) -> bool) -> Item {
        *tb.items.iter().find(|i| f(&i.kind)).expect("item")
    }

    fn centre(r: FRect) -> Pt {
        Pt::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn record_button_joins_the_outputs() {
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.record = true;
        let tb = layout(&i);
        assert_eq!(tb.bar.w, 713.0 + BTN + IN_GAP);
        let rec = btn(&tb, Act::Record);
        assert_eq!(rec.x1() + IN_GAP, btn(&tb, Act::Copy).x, "first of the output group");
        assert_eq!(tb.act_at(centre(rec)), Some(Act::Record));
        // The busy OK mode has no Record button.
        i.busy = true;
        assert!(!layout(&i).items.iter().any(|it| it.kind == Kind::Btn(Act::Record)));
    }

    #[test]
    fn record_popover_layout_and_hits() {
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.record = true;
        let rv = rec_view(RecFormat::Mp4, false, None);
        i.rec = Some(&rv);
        let tb = layout(&i);
        let pop = tb.pop.expect("popover");
        assert_eq!(pop.w, REC_W);
        assert_eq!(pop.y, tb.bar.y1() + POP_GAP, "below a bar below the selection");
        assert_eq!(pop.x1(), btn(&tb, Act::Record).x1(), "right-aligned to the Record button");
        assert!(tb.items.iter().all(|it| !matches!(it.kind, Kind::Dot(_))), "no palette");
        // Every popover item lies inside it.
        let n = tb.items.iter().position(|it| it.kind == Kind::SegTrack).unwrap();
        for it in &tb.items[n..] {
            assert!(it.r.x >= pop.x && it.r.x1() <= pop.x1() + 0.01 && it.r.y >= pop.y && it.r.y1() <= pop.y1() + 0.01, "{it:?}");
        }
        let seg = |a: Act| kind_at(&tb, |k| matches!(k, Kind::Seg { act, .. } if *act == a));
        assert!(matches!(seg(Act::RecFormat(RecFormat::Mp4)).kind, Kind::Seg { on: true, .. }));
        assert!(matches!(seg(Act::RecFormat(RecFormat::Gif)).kind, Kind::Seg { on: false, .. }));
        assert_eq!(tb.act_at(centre(seg(Act::RecFormat(RecFormat::Gif)).r)), Some(Act::RecFormat(RecFormat::Gif)));
        assert_eq!(tb.act_at(centre(seg(Act::RecFps(60)).r)), Some(Act::RecFps(60)));
        let sw = |a: Act| kind_at(&tb, |k| matches!(k, Kind::Switch { act, .. } if *act == a));
        // A whole switch row toggles: its left end too.
        let sys = sw(Act::RecSystem);
        assert_eq!(tb.act_at(Pt::new(sys.r.x + 6.0, sys.r.y + 10.0)), Some(Act::RecSystem));
        assert_eq!(tb.act_at(centre(sw(Act::RecMic).r)), Some(Act::RecMic));
        // The camera is wired up later: shown, not clickable.
        let cam = sw(Act::RecCamera);
        assert!(cam.disabled);
        assert_eq!(tb.act_at(centre(cam.r)), None);
        assert!(tb.contains(centre(cam.r)), "but the press is consumed");
        let start = kind_at(&tb, |k| matches!(k, Kind::Primary { .. }));
        assert_eq!(tb.act_at(centre(start.r)), Some(Act::RecStart));
        assert!(start.r.y1() < pop.y1() && start.r.y > cam.r.y1());
        // No microphone box while the microphone is off.
        assert!(!tb.items.iter().any(|it| matches!(it.kind, Kind::DevBox { .. })));
        // Labels hit nothing.
        let label = kind_at(&tb, |k| matches!(k, Kind::Label(_)));
        assert_eq!(tb.act_at(Pt::new(label.r.x + 4.0, label.r.y + 4.0)), None);
    }

    #[test]
    fn record_popover_gif_disables_sound() {
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.record = true;
        let rv = rec_view(RecFormat::Gif, true, None);
        i.rec = Some(&rv);
        let tb = layout(&i);
        let sys = kind_at(&tb, |k| matches!(k, Kind::Switch { act: Act::RecSystem, .. }));
        assert!(sys.disabled && matches!(sys.kind, Kind::Switch { on: false, .. }));
        assert!(!tb.items.iter().any(|it| matches!(it.kind, Kind::DevBox { .. })), "no microphone for a GIF");
        let fps: Vec<Act> = tb.items.iter().filter_map(|it| match it.kind {
            Kind::Seg { act: a @ Act::RecFps(_), .. } => Some(a),
            _ => None,
        }).collect();
        assert_eq!(fps, [Act::RecFps(10), Act::RecFps(15)]);
    }

    #[test]
    fn record_popover_microphone_list() {
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.record = true;
        let closed = rec_view(RecFormat::Mp4, true, None);
        i.rec = Some(&closed);
        let short = layout(&i);
        let bx = kind_at(&short, |k| matches!(k, Kind::DevBox { open: false }));
        assert_eq!(short.act_at(centre(bx.r)), Some(Act::RecDevMenu));
        assert_eq!(short.texts, ["Default microphone"]);
        let devs: Vec<String> = ["Default microphone", "USB mic", "Headset"].iter().map(|s| s.to_string()).collect();
        let mut open = rec_view(RecFormat::Mp4, true, Some(&devs));
        open.dev_sel = 2;
        i.rec = Some(&open);
        let tb = layout(&i);
        assert_eq!(tb.texts.len(), 4);
        let items: Vec<Item> = tb.items.iter().filter(|it| matches!(it.kind, Kind::DevItem { .. })).copied().collect();
        assert_eq!(items.len(), 3);
        assert!(matches!(items[2].kind, Kind::DevItem { i: 2, on: true }));
        // The list floats over the rows below the box and wins their hits.
        let cam = kind_at(&tb, |k| matches!(k, Kind::Switch { act: Act::RecCamera, .. }));
        assert!(items[1].r.y < cam.r.y1() && items[2].r.y1() > cam.r.y);
        assert_eq!(tb.act_at(centre(items[1].r)), Some(Act::RecDev(1)));
        let pop = tb.pop.unwrap();
        assert!(items[2].r.y1() <= pop.y1(), "the popover grows to hold the list");
        // Many devices: the list is capped.
        let many: Vec<String> = (0..20).map(|n| format!("Mic {n}")).collect();
        let lots = rec_view(RecFormat::Mp4, true, Some(&many));
        i.rec = Some(&lots);
        let tb = layout(&i);
        assert_eq!(tb.items.iter().filter(|it| matches!(it.kind, Kind::DevItem { .. })).count(), DEV_MAX);
    }

    #[test]
    fn record_popover_flips_above_and_clamps() {
        let mut i = inp(r(100.0, 600.0, 800.0, 300.0));
        i.record = true;
        let rv = rec_view(RecFormat::Mp4, false, None);
        i.rec = Some(&rv);
        let tb = layout(&i);
        let pop = tb.pop.unwrap();
        assert!(pop.y1() <= tb.bar.y - POP_GAP + 0.01 || pop.y >= tb.bar.y1(), "{pop:?} {:?}", tb.bar);
        assert!(pop.y >= MARGIN && pop.y1() <= AREA.h - MARGIN, "{pop:?}");
        // A short area fits it on neither side: clamped inside.
        i.area = r(0.0, 0.0, 1920.0, 360.0);
        i.sel = r(100.0, 100.0, 800.0, 150.0);
        let tb = layout(&i);
        let pop = tb.pop.unwrap();
        assert!(pop.y >= MARGIN - 0.01, "{pop:?}");
        // At 2x every size doubles.
        let mut j = inp(r(100.0, 100.0, 1600.0, 300.0));
        j.area = r(0.0, 0.0, 3840.0, 2160.0);
        j.s = 2.0;
        j.record = true;
        j.rec = Some(&rv);
        assert_eq!(layout(&j).pop.unwrap().w, 2.0 * REC_W);
    }
}
