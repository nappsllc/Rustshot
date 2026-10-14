//! Toolbar model: four button groups, one- or two-row layout, placement
//! around the selection, the palette popover and hit-testing. Sizes are
//! logical px from the design spec; `Input::s` scales them to image pixels.

use super::pick::Part;
use super::Tool;
use crate::cam::Shape;
use crate::rec_ui::RecSource;
use crate::objects::{FRect, Pt};
use crate::rec::RecFormat;
use crate::uifb::C4;

pub const BTN: f32 = 32.0;
pub const PAD: f32 = 6.0;
pub const MARGIN: f32 = 12.0;
/// The Camera row's label while the cameras are being listed.
pub const LOOKING_CAMS: crate::i18n::Key = key!("rec.panel.looking_cams");
const IN_GAP: f32 = 2.0;
const GROUP_GAP: f32 = 6.0;
const SEP_LEN: f32 = 16.0;
/// The size readout: the number, then a preview drawn at the size.
const VALUE_W: f32 = 76.0;
/// The busy-mode OK button spans the output buttons it replaces (Copy,
/// Copy text, Save, Save As, Save to Desktop, Upload, Pin).
const ACCEPT_W: f32 = 5.0 * BTN + SAVE_MENU_W + 5.0 * IN_GAP;
/// The Save split button's chevron (its menu: Save As, Save to Desktop).
const SAVE_MENU_W: f32 = 16.0;
/// The save menu: its width and row height (logical px).
const MENU_W: f32 = 220.0;
const MENU_ROW: f32 = 30.0;
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

/// The tool buttons. Blur and Invert share a slot (the bar keeps its
/// width): a click picks Blur, a long press opens a menu with both
/// ([`SLOT_MENU`]), and the slot shows Invert while it is the tool (key I;
/// [`slot_act`], its tooltip too).
pub const TOOL_ORDER: [Tool; 14] = [
    Tool::Select,
    Tool::Path,
    Tool::Line,
    Tool::Arrow,
    Tool::Rect,
    Tool::Ellipse,
    Tool::Marker,
    Tool::Text,
    Tool::Callout,
    Tool::Counter,
    Tool::Pixelate,
    Tool::Blur,
    Tool::Spotlight,
    Tool::Magnify,
];

/// What a toolbar button shows and names with `tool` current: the Blur
/// slot is Invert while Invert is the tool (a press on it picks Blur).
pub fn slot_act(act: Act, tool: Option<Tool>) -> Act {
    match act {
        Act::Tool(Tool::Blur) if tool == Some(Tool::Invert) => Act::Tool(Tool::Invert),
        a => a,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Tool(Tool),
    Undo,
    Redo,
    Size(i32),
    Color(C4),
    Palette,
    /// The rectangle / ellipse: filled or outlined.
    Fill,
    Copy,
    /// Copy the selection's text (OCR).
    CopyText,
    /// The object mini-toolbar: the color (opens the palette), the fill,
    /// the size a step, the callout's shape, front / back, duplicate,
    /// delete.
    ObjColor,
    ObjFill,
    ObjSize(i32),
    ObjStyle(crate::objects::CalloutStyle),
    ObjFront,
    ObjBack,
    ObjDup,
    ObjDelete,
    /// The text style (the mini-toolbar over a text, a callout or the
    /// text draft; Ctrl+B / I / U while typing): bold, italic, underline,
    /// strikethrough on / off, the alignment, the font list open / closed,
    /// a font from it.
    ObjBold,
    ObjItalic,
    ObjUnderline,
    ObjStrike,
    ObjAlign(crate::text::Align),
    ObjFontMenu,
    ObjFont(crate::text::Family),
    Save,
    /// The Save As dialog (always asks where).
    SaveAs,
    /// Straight to the Desktop folder.
    SaveDesktop,
    /// Open the capture with an app the user picks (the Save menu's
    /// "Open with…").
    OpenWith,
    /// The system's share sheet (the Save menu's "Share…").
    Share,
    Upload,
    /// Pin the selection to the screen.
    Pin,
    Exit,
    Accept,
    /// The Record button: video mode on/off.
    Record,
    /// The mode switch above the selection: Screenshot (false) or Video
    /// (true); keys 1 and 2.
    Mode(bool),
    /// The panel's source: Screen, Window, Region (F / W / R).
    RecSource(RecSource),
    RecFormat(RecFormat),
    RecFps(u32),
    RecSystem,
    RecMic,
    RecCamera,
    /// Open/close a device list.
    RecDevMenu(Dev),
    /// Pick device `i` of the list (0 = the default device).
    RecDev(Dev, usize),
    RecShape(Shape),
    /// The camera's background: off or blurred.
    RecBg(crate::cam::bg::Bg),
    /// The countdown 0 / 3 s.
    RecCountdown(bool),
    /// A GIF's max length (seconds).
    RecGifMax(u32),
    /// "Start recording" (while the blur question is up: record with the
    /// camera fully blurred).
    RecStart,
    /// The blur question: turn the camera off and record.
    RecAskCamOff,
    /// The blur question: back to the panel.
    RecAskCancel,
    /// The palette's rainbow swatch: the color picker for the tool.
    Wheel,
    /// The camera ring's width, a step thinner or thicker.
    RingSize(i32),
    /// Open/close the ring's palette.
    RingPalette,
    /// A ring color (`None`: the theme's accent).
    RingColor(Option<C4>),
    /// The ring palette's rainbow swatch: the color picker for the ring.
    RingWheel,
    /// A press on a part of the color picker (the point says where).
    Pick(Part),
    /// A recent color in the picker.
    PickColor(C4),
    /// The picker's eyedropper on/off.
    Eyedrop,
    /// The Save split button's chevron: the menu with Save As and Save
    /// to Desktop.
    SaveMenu,
    /// The crop frame on / off (C): Enter crops, Esc leaves it.
    Crop,
    /// Quick Redact (Shift+R): blur the sensitive text OCR finds (in Text
    /// mode with words selected: those words).
    Redact,
    /// The freeform (lasso) selection on / off (Shift+F).
    Freeform,
    /// The Copy text menu: Text mode (select words), copy everything.
    TextSelect,
    CopyAll,
    /// Text mode: copy the words selected.
    TextCopy,
    /// The Copy text menu: a kind of sensitive text on / off.
    RedactKind(crate::ocr::redact::Kind),
    /// Tighten the selection to its content (the size label's menu).
    Trim,
    /// Read the QR codes in the selection (Q; the Copy text menu).
    ReadQr,
    /// Scrolling capture of the selection (the size label's menu, Shift+S).
    Scroll,
}

/// Which device dropdown of the Record popover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dev {
    Mic,
    Cam,
}

impl Dev {
    /// Index in `Toolbar::texts` of the chosen device's name.
    pub fn text(self) -> usize {
        match self {
            Dev::Mic => 0,
            Dev::Cam => 1,
        }
    }
}

/// `Toolbar::texts` index of the open list's first entry.
pub const LIST_TEXT: usize = 2;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Btn(Act),
    /// The stroke-size readout ("3 line").
    Value,
    SepV,
    SepH,
    /// A palette color in the popover.
    Dot(C4),
    /// The Record panel: a segmented control's track.
    SegTrack,
    /// A segment (`on` = chosen), with an icon and/or a label.
    Seg { act: Act, label: &'static str, icon: &'static str, on: bool },
    /// Muted text.
    Label(&'static str),
    /// A column's caption over a segmented control (11 px, muted).
    Caption(&'static str),
    /// A panel row: icon, name, a value (else its device box), `dim`
    /// while off; a press on it acts `act` (unless disabled).
    Row { act: Act, icon: &'static str, label: &'static str, value: Option<&'static str>, dim: bool },
    /// A row's on/off toggle at its right end (`label` names the row).
    Switch { act: Act, label: &'static str, on: bool },
    /// A row's device: its name and a chevron (shows
    /// `Toolbar::texts[dev.text()]`); opens the list.
    DevBox { dev: Dev, open: bool },
    /// The microphone's 7-bar level meter.
    Meter { lit: u8, clip: bool },
    /// The mode switch's track and its two modes.
    ModeTrack,
    Mode { video: bool, on: bool },
    /// Video mode's Close button above the selection.
    Close,
    /// "Drag · scroll to resize" under the camera bubble.
    Grip,
    /// The open device list's panel.
    DevList,
    /// Device `i` of the open list (`Toolbar::texts[LIST_TEXT + i]`).
    DevItem { dev: Dev, i: usize, on: bool },
    /// The popover's primary button.
    Primary { act: Act, label: &'static str },
    /// A color swatch that acts (`on`: the chosen one).
    Swatch { act: Act, c: C4, on: bool },
    /// The rainbow swatch at the end of a palette: the color picker.
    Rainbow(Act),
    /// The camera ring's width readout and preview (logical px, color).
    RingValue { w: u32, c: C4 },
    /// The picker's hue ring (the wheel's bounding square).
    PickHue,
    /// The saturation/value square inside the ring.
    PickSv,
    /// The live swatch of the color picked.
    PickSwatch,
    /// The hex field.
    PickHex,
    /// The opacity slider's track.
    PickAlpha,
    /// Recent color `i`.
    Recent { i: usize, c: C4 },
    /// A row of the save menu: its act, label, and its keys
    /// (`Toolbar::texts[key]`).
    MenuItem { act: Act, label: &'static str, key: usize },
    /// A checkbox row of a menu (the Copy text menu's kinds).
    Check { act: Act, label: &'static str, on: bool },
    /// A small square button with its own surface (the freeform toggle
    /// next to the mode switch): accent while `on`.
    Toggle { act: Act, icon: &'static str, on: bool },
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
    /// Video mode: the setup panel (instead of the toolbar).
    pub rec: Option<&'a RecView<'a>>,
    /// The camera bubble on screen (image px): the grip hint goes under it.
    pub grip: Option<FRect>,
    /// The mode switch (and Close) above the selection; not while the
    /// selection is dragged (it would repaint on every move).
    pub top: bool,
    /// The current tool has a size (else `– N +` is disabled) and a
    /// color (else the palette button is).
    pub sizes: bool,
    pub colors: bool,
    /// The current tool can be filled (else the fill toggle is disabled).
    pub fills: bool,
    /// The color picker while it is open (over the others).
    pub pick: Option<&'a PickView<'a>>,
    /// The save menu while it is open: the keys of its rows
    /// ([`save_menu`]).
    pub save_menu: Option<&'a [String]>,
    /// The Blur / Invert menu while it is open: their keys.
    pub slot_menu: Option<&'a [String; 2]>,
    /// The Copy text menu while it is open.
    pub copy_menu: Option<&'a CopyMenu>,
    /// The freeform toggle by the mode switch (`Some(on)`: shown).
    pub freeform: Option<bool>,
}

/// The Copy text menu (a long press on Copy text): Select text, Copy all
/// text, Redact sensitive text (their keys), then the kinds to redact.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyMenu {
    pub keys: [String; 4],
    pub kinds: Vec<crate::ocr::redact::Kind>,
}

/// The Copy text menu's rows.
pub const COPY_MENU: [(Act, crate::i18n::Key); 4] = [
    (Act::TextSelect, key!("tip.select_text")),
    (Act::CopyAll, key!("tip.copy_all_text")),
    (Act::ReadQr, key!("tip.read_qr")),
    (Act::Redact, key!("editor.menu.redact_sensitive")),
];

/// The Copy text menu under (or over) its button: the three rows, a
/// separator, the kinds' checkboxes.
fn place_copy_menu(cm: &CopyMenu, bar: FRect, above: bool, items: &mut Vec<Item>, inp: &Input) -> FRect {
    use crate::ocr::redact::Kind as K;
    let s = inp.s;
    let rows = COPY_MENU.len() as f32 + K::ALL.len() as f32;
    let (pw, ph) = (MENU_W * s, (rows * MENU_ROW + 2.0 * PAD + 2.0 * ROW_GAP + 1.0) * s);
    let anchor = items.iter().find(|it| it.kind == Kind::Btn(Act::CopyText)).map_or(bar, |it| it.r);
    let (px, py) = place_pop(pw, ph, anchor, bar, above, inp);
    let row = |y: f32| FRect { x: px + PAD * s, y: py + y * s, w: pw - 2.0 * PAD * s, h: MENU_ROW * s };
    let mut y = PAD;
    for (i, (act, label)) in COPY_MENU.into_iter().enumerate() {
        items.push(Item { kind: Kind::MenuItem { act, label: label.get(), key: i }, r: row(y), disabled: false });
        y += MENU_ROW;
    }
    y += ROW_GAP;
    items.push(Item { kind: Kind::SepH, r: FRect { x: px + 2.0 * PAD * s, y: py + y * s, w: pw - 4.0 * PAD * s, h: s.max(1.0) }, disabled: false });
    y += 1.0 + ROW_GAP;
    for k in K::ALL {
        items.push(Item { kind: Kind::Check { act: Act::RedactKind(k), label: k.label(), on: cm.kinds.contains(&k) }, r: row(y), disabled: false });
        y += MENU_ROW;
    }
    FRect { x: px, y: py, w: pw, h: ph }
}

/// The freeform toggle's side (logical px): the mode switch's height.
const TOGGLE: f32 = 28.0;

/// What shapes the color picker popover.
#[derive(Clone, Debug, PartialEq)]
pub struct PickView<'a> {
    /// Over the Record button (the camera ring), else the palette button.
    pub ring: bool,
    /// With the opacity row.
    pub alpha: bool,
    pub recents: &'a [C4],
}

/// What the Record panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct RecView<'a> {
    pub format: RecFormat,
    /// MP4 can be recorded (Linux: ffmpeg is installed and encodes
    /// H.264); else its segment is disabled (`rec_ui::NO_MP4`), and while
    /// ffmpeg is being asked it says "checking…" (disabled too).
    pub mp4: crate::rec_ui::Mp4,
    pub fps: u32,
    pub fps_choices: &'static [u32],
    pub system: bool,
    pub mic: bool,
    pub camera: bool,
    /// There is a camera (else the Camera row is disabled, "No camera found").
    pub cams: bool,
    /// The cameras are still being listed (the row says so, disabled).
    pub cams_looking: bool,
    pub shape: Shape,
    /// The camera's background; whether a blur can be chosen; if not,
    /// the row's note (the model downloading, off, damaged; a failed
    /// download, when a press of the row retries).
    pub bg: crate::cam::bg::Bg,
    pub blur: bool,
    pub blur_note: Option<&'static str>,
    pub blur_retry: bool,
    /// Start was pressed with a blur chosen that cannot be had: the
    /// question's two lines (in place of Start recording).
    pub blur_ask: Option<(&'static str, &'static str)>,
    /// The chosen microphone's name ("Default microphone" for the default).
    pub mic_label: &'a str,
    /// The chosen camera's name ("Default camera" for the first).
    pub cam_label: &'a str,
    /// The open device list: whose, and its names (the default first).
    pub devs: Option<(Dev, &'a [String])>,
    /// Index into the open list of the chosen one.
    pub dev_sel: usize,
    /// The camera ring: its color (`None` = accent), width (logical px),
    /// and whether its palette is open (with these colors).
    pub ring: Option<C4>,
    pub ring_w: u32,
    pub ring_pal: bool,
    pub ring_colors: &'a [C4],
    pub accent: C4,
    pub source: RecSource,
    /// 3-2-1 before the first frame (MP4's Countdown segment).
    pub countdown: bool,
    /// A GIF's max length (seconds).
    pub gif_max: u32,
    /// The microphone's level as bars lit and clipping, while metered.
    pub meter: Option<(u8, bool)>,
}

#[derive(Clone, PartialEq)]
pub struct Toolbar {
    pub bar: FRect,
    pub pop: Option<FRect>,
    pub items: Vec<Item>,
    pub wrapped: bool,
    /// The bar's anchor edge is its bottom (placed above / inside).
    pub above: bool,
    /// Texts of the Record panel: the microphone box, the camera box,
    /// then the open list ([`LIST_TEXT`]).
    pub texts: Vec<String>,
    /// Surfaces outside the bar and popover (the mode switch, Close, the
    /// grip hint): they take presses too.
    pub extra: Vec<FRect>,
    /// The mode switch above the selection (the size label follows it).
    pub mode: Option<FRect>,
    /// Video mode: `bar` is the setup panel.
    pub video: bool,
    /// Docked in the Edit window: no surface of its own (the window draws
    /// the strip).
    pub docked: bool,
}

type Group = Vec<(Kind, bool, f32)>;

fn groups(inp: &Input) -> [Group; 4] {
    let tools = TOOL_ORDER.iter().map(|t| (Kind::Btn(Act::Tool(*t)), false, BTN)).collect();
    let history = vec![
        (Kind::Btn(Act::Undo), !inp.can_undo, BTN),
        (Kind::Btn(Act::Redo), !inp.can_redo, BTN),
    ];
    let size = vec![
        (Kind::Btn(Act::Size(-1)), !inp.sizes, BTN),
        (Kind::Value, !inp.sizes, VALUE_W),
        (Kind::Btn(Act::Size(1)), !inp.sizes, BTN),
        (Kind::Btn(Act::Fill), !inp.fills, BTN),
        (Kind::Btn(Act::Palette), !inp.colors, BTN),
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
            (Kind::Btn(Act::CopyText), false, BTN),
            (Kind::Btn(Act::Save), false, BTN),
            (Kind::Btn(Act::SaveMenu), false, SAVE_MENU_W),
            (Kind::Btn(Act::Upload), false, BTN),
            (Kind::Btn(Act::Pin), false, BTN),
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

/// A right-to-left language runs the toolbar right to left: the items
/// laid out so far mirrored within `0..w` (logical px), so the first
/// group (the tools) starts at the right.
fn mirror_row(items: &mut [Item], w: f32) {
    if crate::i18n::is_rtl() {
        for it in items {
            it.r.x = w - it.r.x - it.r.w;
        }
    }
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
    if let Some(rv) = inp.rec {
        return super::panel::layout(inp, rv);
    }
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
    mirror_row(&mut items, w);

    let (bw, bh) = (w * s, h * s);
    let sel = inp.sel;
    let g = SEL_GAP * s;
    // Centred on the selection; near a monitor edge it slides along it
    // (never over to the other side).
    let x = (sel.x + sel.w / 2.0 - bw / 2.0).clamp(area.x + m, (area.x1() - m - bw).max(area.x + m));
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
    let pop = match (inp.pick, inp.palette, inp.save_menu) {
        (Some(pv), _, _) => Some(place_picker(pv, bar, above, &mut items, inp)),
        (None, Some(colors), _) => Some(place_palette(colors, bar, above, &mut items, inp)),
        (None, None, Some(keys)) => {
            texts.extend(keys.iter().cloned());
            Some(place_save_menu(bar, above, &mut items, inp))
        }
        (None, None, None) => match (inp.slot_menu, inp.copy_menu) {
            (Some(keys), _) => {
                texts.extend(keys.iter().cloned());
                Some(place_slot_menu(bar, above, &mut items, inp))
            }
            (None, Some(cm)) => {
                texts.extend(cm.keys.iter().cloned());
                Some(place_copy_menu(cm, bar, above, &mut items, inp))
            }
            (None, None) => None,
        },
    };
    // The mode switch above the selection (recording works here).
    let mut extra = Vec::new();
    let mut mode = (inp.record && inp.top).then(|| {
        let up = above.then(|| pop.map_or(bar, |p| union(bar, p))).filter(|b| b.y1() <= sel.y + 0.5);
        super::panel::place_top(sel, area, s, false, up, &mut items, &mut extra)
    });
    // The freeform toggle: right of the mode switch (else above the
    // selection's left end); the size label follows it.
    if let (Some(on), true) = (inp.freeform, inp.top) {
        let h = TOGGLE * s;
        let r = match mode {
            Some(m) => FRect { x: m.x1() + 6.0 * s, y: m.y + (m.h - h) / 2.0, w: h, h },
            None => {
                let up = sel.y - 8.0 * s - h;
                let y = if up >= area.y + 4.0 * s { up } else { sel.y + 8.0 * s };
                FRect { x: sel.x.clamp(area.x, (area.x1() - h).max(area.x)), y, w: h, h }
            }
        };
        items.push(Item { kind: Kind::Toggle { act: Act::Freeform, icon: "lasso", on }, r, disabled: false });
        extra.push(r);
        mode = Some(mode.map_or(r, |m| union(m, r)));
    }
    Toolbar { bar, pop, items, wrapped, above, texts, extra, mode, video: false, docked: false }
}

/// The Edit window's groups: the overlay's tools and Crop, history and
/// size groups, then Save, Save As, Save to Desktop, Copy, Upload (no
/// Record, Copy text, Pin or Close: the window has its own).
fn docked_groups(inp: &Input) -> [Group; 4] {
    let [mut tools, history, size, _] = groups(inp);
    tools.push((Kind::Btn(Act::Crop), false, BTN));
    let btn = |a: Act| (Kind::Btn(a), false, BTN);
    let out: Group = [Act::Save, Act::SaveAs, Act::SaveDesktop, Act::Copy, Act::Upload].into_iter().map(btn).collect();
    let mut gs = [tools, history, size, out];
    // A window too narrow for them all (the Edit window at its smallest):
    // Save As and Save to Desktop go into Save's menu (its chevron),
    // as the overlay's.
    if row_w(&gs) + 2.0 * DOCK_PAD > inp.area.w / inp.s.max(0.01) + 0.5 {
        gs[3] = vec![btn(Act::Save), (Kind::Btn(Act::SaveMenu), false, SAVE_MENU_W), btn(Act::Copy), btn(Act::Upload)];
    }
    gs
}

/// The width the docked toolbar's buttons take (logical px).
pub fn docked_width(inp: &Input) -> f32 {
    row_w(&docked_groups(inp)) + 2.0 * DOCK_PAD
}

/// The docked toolbar's side padding (logical px).
const DOCK_PAD: f32 = 8.0;

/// The toolbar docked in the Edit window's top strip `bar` (the window's
/// physical px, `inp.s` its scale): one row from the left, centred
/// vertically; the palette, color picker and save menu open below it,
/// kept inside `inp.area` (the window).
pub fn layout_docked(inp: &Input, bar: FRect) -> Toolbar {
    let s = inp.s;
    let gs = docked_groups(inp);
    let mut items = Vec::new();
    place_row(&gs, 0.0, 0.0, &mut items);
    let row = row_w(&gs);
    mirror_row(&mut items, row);
    // Right to left: from the strip's right end.
    let x0 = if crate::i18n::is_rtl() { bar.x1() - (DOCK_PAD + row) * s } else { bar.x + DOCK_PAD * s };
    let (x, y) = (x0, bar.y + ((bar.h - BTN * s) / 2.0).round());
    for it in &mut items {
        it.r = FRect { x: (x + it.r.x * s).round(), y: y + it.r.y * s, w: (it.r.w * s).round(), h: (it.r.h * s).round() };
    }
    let mut texts = Vec::new();
    let pop = match (inp.pick, inp.palette, inp.save_menu) {
        (Some(pv), _, _) => Some(place_picker(pv, bar, false, &mut items, inp)),
        (None, Some(colors), _) => Some(place_palette(colors, bar, false, &mut items, inp)),
        (None, None, Some(keys)) => {
            texts.extend(keys.iter().cloned());
            Some(place_save_menu(bar, false, &mut items, inp))
        }
        (None, None, None) => match (inp.slot_menu, inp.copy_menu) {
            (Some(keys), _) => {
                texts.extend(keys.iter().cloned());
                Some(place_slot_menu(bar, false, &mut items, inp))
            }
            (None, Some(cm)) => {
                texts.extend(cm.keys.iter().cloned());
                Some(place_copy_menu(cm, bar, false, &mut items, inp))
            }
            (None, None) => None,
        },
    };
    Toolbar { bar, pop, items, wrapped: false, above: false, texts, extra: Vec::new(), mode: None, video: false, docked: true }
}

/// The Blur / Invert button's menu (a long press on it): its two tools.
pub const SLOT_MENU: [(Act, crate::i18n::Key); 2] = [(Act::Tool(Tool::Blur), key!("tip.blur")), (Act::Tool(Tool::Invert), key!("tip.invert"))];

/// The Blur / Invert menu under (or over) its button, as the save menu.
fn place_slot_menu(bar: FRect, above: bool, items: &mut Vec<Item>, inp: &Input) -> FRect {
    let s = inp.s;
    let (pw, ph) = (SLOT_MENU_W * s, (SLOT_MENU.len() as f32 * MENU_ROW + 2.0 * PAD) * s);
    let anchor = items.iter().find(|it| it.kind == Kind::Btn(Act::Tool(Tool::Blur))).map_or(bar, |it| it.r);
    let (px, py) = place_pop(pw, ph, anchor, bar, above, inp);
    for (i, (act, label)) in SLOT_MENU.into_iter().enumerate() {
        let r = FRect { x: px + PAD * s, y: py + (PAD + i as f32 * MENU_ROW) * s, w: pw - 2.0 * PAD * s, h: MENU_ROW * s };
        items.push(Item { kind: Kind::MenuItem { act, label: label.get(), key: i }, r, disabled: false });
    }
    FRect { x: px, y: py, w: pw, h: ph }
}

/// The Blur / Invert menu's width (logical px).
const SLOT_MENU_W: f32 = 160.0;

/// The save menu's rows (Share last, where there is a share sheet: see
/// [`save_menu`]).
pub const SAVE_MENU: [(Act, crate::i18n::Key); 4] =
    [(Act::SaveAs, key!("tip.save_as")), (Act::SaveDesktop, key!("tip.save_to_desktop")), (Act::OpenWith, key!("tip.open_with")), (Act::Share, key!("tip.share"))];

/// The save menu's rows here: no Share on Linux (no standard sheet).
pub fn save_menu() -> &'static [(Act, crate::i18n::Key)] {
    if crate::share::supported() { &SAVE_MENU } else { &SAVE_MENU[..3] }
}

/// The save menu under (or over) the Save button: Save As, Save to
/// Desktop, Open with, Share.
fn place_save_menu(bar: FRect, above: bool, items: &mut Vec<Item>, inp: &Input) -> FRect {
    let s = inp.s;
    let rows = save_menu();
    let (pw, ph) = (MENU_W * s, (rows.len() as f32 * MENU_ROW + 2.0 * PAD) * s);
    let anchor = items.iter().find(|it| it.kind == Kind::Btn(Act::Save)).map_or(bar, |it| it.r);
    let (px, py) = place_pop(pw, ph, anchor, bar, above, inp);
    for (i, &(act, label)) in rows.iter().enumerate() {
        let r = FRect { x: px + PAD * s, y: py + (PAD + i as f32 * MENU_ROW) * s, w: pw - 2.0 * PAD * s, h: MENU_ROW * s };
        items.push(Item { kind: Kind::MenuItem { act, label: label.get(), key: i }, r, disabled: false });
    }
    FRect { x: px, y: py, w: pw, h: ph }
}

/// The selection size label's menu (a click on the label): Scrolling
/// capture (where the window under the selection can be scrolled) and
/// Trim to content.
pub const SIZE_MENU: [(Act, crate::i18n::Key); 2] = [(Act::Scroll, key!("tip.scroll_capture")), (Act::Trim, key!("tip.trim_to_content"))];

/// The size label menu's rows here.
pub fn size_menu() -> &'static [(Act, crate::i18n::Key)] {
    if crate::scroll::supported() { &SIZE_MENU } else { &SIZE_MENU[1..] }
}
/// Its width (logical px).
const SIZE_MENU_W: f32 = 200.0;

/// Open the size label's menu on `tb`: under the label at `label` (above
/// it when it does not fit), kept on the monitor `area`; `keys` are its
/// rows' keys (in [`size_menu`]'s order). Not over another popover or the
/// Record panel (false then).
pub fn add_size_menu(tb: &mut Toolbar, label: FRect, keys: &[String], area: FRect, s: f32) -> bool {
    if tb.pop.is_some() || tb.video || tb.docked {
        return false;
    }
    let m = MARGIN * s;
    let (pw, ph) = (SIZE_MENU_W * s, (size_menu().len() as f32 * MENU_ROW + 2.0 * PAD) * s);
    let px = label.x.clamp(area.x + m, (area.x1() - m - pw).max(area.x + m));
    let below = label.y1() + POP_GAP * s;
    let py = if below + ph <= area.y1() - m { below } else { (label.y - POP_GAP * s - ph).max(area.y + m) };
    for (i, &(act, text)) in size_menu().iter().enumerate() {
        let r = FRect { x: px + PAD * s, y: py + (PAD + i as f32 * MENU_ROW) * s, w: pw - 2.0 * PAD * s, h: MENU_ROW * s };
        tb.items.push(Item { kind: Kind::MenuItem { act, label: text.get(), key: tb.texts.len() + i }, r, disabled: false });
    }
    tb.texts.extend(keys.iter().cloned());
    tb.pop = Some(FRect { x: px, y: py, w: pw, h: ph });
    true
}

/// Top-left of a `pw`x`ph` popover 6 px from the bar on the side away from
/// the selection (below a bar below it, above a bar above/inside it), or
/// the other side when it does not fit there; centred over `anchor` (kept
/// on the monitor).
/// When it fits on neither side it goes where there is more room,
/// clamped to the area.
fn place_pop(pw: f32, ph: f32, anchor: FRect, bar: FRect, above: bool, inp: &Input) -> (f32, f32) {
    let s = inp.s;
    let m = MARGIN * s;
    let px = (anchor.x + anchor.w / 2.0 - pw / 2.0).clamp(inp.area.x + m, (inp.area.x1() - m - pw).max(inp.area.x + m));
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

/// Popover 6 px from the bar on the side away from the selection (below a
/// bar that sits below it, above a bar that sits above/inside it), falling
/// back to the other side when it does not fit; centred over the swatch
/// button, one row of dots and the rainbow swatch (the color picker).
fn place_palette(colors: &[C4], bar: FRect, above: bool, items: &mut Vec<Item>, inp: &Input) -> FRect {
    let s = inp.s;
    let m = MARGIN * s;
    let n = colors.len() as f32 + 1.0;
    let pw = (n * DOT_BTN + (n - 1.0) * DOT_GAP + 2.0 * PAD) * s;
    let ph = (DOT_BTN + 2.0 * PAD) * s;
    let anchor = items
        .iter()
        .find(|it| it.kind == Kind::Btn(Act::Palette))
        .map(|it| it.r)
        .unwrap_or(bar);
    let px = (anchor.x + anchor.w / 2.0 - pw / 2.0).clamp(inp.area.x + m, (inp.area.x1() - m - pw).max(inp.area.x + m));
    let up = bar.y - POP_GAP * s - ph;
    let down = bar.y1() + POP_GAP * s;
    let py = if above {
        if up < inp.area.y + m { down } else { up }
    } else if down + ph > inp.area.y1() - m {
        up
    } else {
        down
    };
    let at = |i: usize| FRect {
        x: px + (PAD + i as f32 * (DOT_BTN + DOT_GAP)) * s,
        y: py + PAD * s,
        w: DOT_BTN * s,
        h: DOT_BTN * s,
    };
    for (i, c) in colors.iter().enumerate() {
        items.push(Item { kind: Kind::Dot(*c), r: at(i), disabled: false });
    }
    items.push(Item { kind: Kind::Rainbow(Act::Wheel), r: at(colors.len()), disabled: false });
    FRect { x: px, y: py, w: pw, h: ph }
}

/// The color picker's width and padding (logical px).
pub const PICK_W: f32 = 232.0;
const PICK_PAD: f32 = 12.0;
/// The hue ring's outer diameter and thickness.
pub const WHEEL: f32 = 168.0;
pub const WHEEL_T: f32 = 16.0;
/// The saturation/value square's side (inside the ring, clear of it).
pub const SV_SIDE: f32 = 86.0;
const PICK_ROW: f32 = 32.0;
const ALPHA_H: f32 = 24.0;
const RECENT_DOT: f32 = 22.0;
const RECENT_GAP: f32 = 4.0;

/// The color picker: the hue ring with the saturation/value square inside,
/// then the live swatch, the hex field and the eyedropper, the opacity
/// slider (tool colors) and the recent colors; centred over the palette
/// button (or the Record button, for the camera ring), placed like the
/// Record popover.
pub(super) fn place_picker(pv: &PickView, bar: FRect, above: bool, items: &mut Vec<Item>, inp: &Input) -> FRect {
    let s = inp.s;
    let mut local: Vec<Item> = Vec::new();
    let mut push = |kind: Kind, r: FRect| local.push(Item { kind, r, disabled: false });
    let x0 = PICK_PAD;
    let inner = PICK_W - 2.0 * PICK_PAD;
    let mut y = PICK_PAD;
    let wx = (PICK_W - WHEEL) / 2.0;
    push(Kind::PickHue, FRect { x: wx, y, w: WHEEL, h: WHEEL });
    let c = (WHEEL - SV_SIDE) / 2.0;
    push(Kind::PickSv, FRect { x: wx + c, y: y + c, w: SV_SIDE, h: SV_SIDE });
    y += WHEEL + 12.0;
    push(Kind::PickSwatch, FRect { x: x0, y, w: PICK_ROW, h: PICK_ROW });
    let hx = x0 + PICK_ROW + 8.0;
    let hw = inner - 2.0 * PICK_ROW - 16.0;
    push(Kind::PickHex, FRect { x: hx, y, w: hw, h: PICK_ROW });
    push(Kind::Btn(Act::Eyedrop), FRect { x: x0 + inner - PICK_ROW, y, w: PICK_ROW, h: PICK_ROW });
    y += PICK_ROW + 12.0;
    if pv.alpha {
        push(Kind::Label(tr!("editor.palette.opacity")), FRect { x: x0, y, w: 60.0, h: ALPHA_H });
        // The percentage sits right of the track.
        push(Kind::PickAlpha, FRect { x: x0 + 64.0, y, w: inner - 64.0 - 40.0, h: ALPHA_H });
        y += ALPHA_H + 10.0;
    }
    push(Kind::Label(if pv.recents.is_empty() { tr!("editor.palette.no_recent") } else { tr!("editor.palette.recent") }), FRect { x: x0, y, w: inner, h: 16.0 });
    y += 16.0 + 6.0;
    if !pv.recents.is_empty() {
        for (i, c) in pv.recents.iter().take(crate::config::RECENT_MAX).enumerate() {
            let r = FRect { x: x0 + i as f32 * (RECENT_DOT + RECENT_GAP), y, w: RECENT_DOT, h: RECENT_DOT };
            push(Kind::Recent { i, c: *c }, r);
        }
        y += RECENT_DOT;
    } else {
        y -= 6.0;
    }
    y += PICK_PAD;
    let (pw, ph) = (PICK_W * s, y * s);
    let at = if pv.ring { Act::Record } else { Act::Palette };
    let anchor = items.iter().find(|it| it.kind == Kind::Btn(at)).map(|it| it.r).unwrap_or(bar);
    let (px, py) = place_pop(pw, ph, anchor, bar, above, inp);
    for it in local {
        let r = FRect { x: px + it.r.x * s, y: py + it.r.y * s, w: it.r.w * s, h: it.r.h * s };
        items.push(Item { r, ..it });
    }
    FRect { x: px, y: py, w: pw, h: ph }
}

/// The color picker alone below `at` (opened from the color ring with
/// nothing selected): no bar of its own (`docked`: no surface drawn).
pub fn layout_picker(pv: &PickView, at: Pt, area: FRect, s: f32) -> Toolbar {
    let bar = FRect { x: at.x, y: at.y, w: 0.0, h: 0.0 };
    let inp = Input {
        sel: bar,
        area,
        s,
        busy: false,
        can_undo: false,
        can_redo: false,
        palette: None,
        record: false,
        rec: None,
        grip: None,
        top: false,
        sizes: false,
        colors: true,
        fills: false,
        pick: Some(pv),
        save_menu: None,
        slot_menu: None,
        copy_menu: None,
        freeform: None,
    };
    let mut items = Vec::new();
    let pop = place_picker(pv, bar, false, &mut items, &inp);
    Toolbar { bar, pop: Some(pop), items, wrapped: false, above: false, texts: Vec::new(), extra: Vec::new(), mode: None, video: false, docked: true }
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
            Kind::Seg { act, .. } | Kind::Switch { act, .. } | Kind::Primary { act, .. } | Kind::Swatch { act, .. } => Some(act),
            Kind::Row { act, .. } => Some(act),
            Kind::Mode { video, .. } => Some(Act::Mode(video)),
            Kind::Close => Some(Act::Exit),
            Kind::Rainbow(act) => Some(act),
            Kind::DevBox { dev, .. } => Some(Act::RecDevMenu(dev)),
            Kind::DevItem { dev, i, .. } => Some(Act::RecDev(dev, i)),
            Kind::PickHue => Some(Act::Pick(Part::Hue)),
            Kind::PickSv => Some(Act::Pick(Part::Sv)),
            Kind::PickHex => Some(Act::Pick(Part::Hex)),
            Kind::PickAlpha => Some(Act::Pick(Part::Alpha)),
            Kind::Recent { c, .. } => Some(Act::PickColor(c)),
            Kind::MenuItem { act, .. } | Kind::Check { act, .. } | Kind::Toggle { act, .. } => Some(act),
            Kind::Value
            | Kind::SepV
            | Kind::SepH
            | Kind::SegTrack
            | Kind::Label(_)
            | Kind::Caption(_)
            | Kind::Meter { .. }
            | Kind::ModeTrack
            | Kind::Grip
            | Kind::DevList
            | Kind::RingValue { .. }
            | Kind::PickSwatch => None,
        }
    }

    /// True over the bar or the open popover: the press is consumed.
    pub fn contains(&self, p: Pt) -> bool {
        hit(self.bar, p) || self.pop.is_some_and(|r| hit(r, p)) || self.extra.iter().any(|r| hit(*r, p))
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
            extra: self.extra.iter().map(|r| f(*r)).collect(),
            mode: self.mode.map(f),
            video: self.video,
            docked: self.docked,
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
        Input {
            sel,
            area: AREA,
            s: 1.0,
            busy: false,
            can_undo: true,
            can_redo: true,
            palette: None,
            record: false,
            rec: None,
            grip: None,
            top: true,
            sizes: true,
            colors: true,
            fills: true,
            pick: None,
            save_menu: None,
            slot_menu: None,
            copy_menu: None,
            freeform: None,
        }
    }

    fn btn(tb: &Toolbar, a: Act) -> FRect {
        tb.items.iter().find(|i| i.kind == Kind::Btn(a)).expect("button").r
    }

    /// The bar's horizontal centre.
    fn cx(r: FRect) -> f32 {
        r.x + r.w / 2.0
    }

    #[test]
    fn single_row_metrics() {
        let tb = layout(&inp(r(100.0, 100.0, 1040.0, 300.0)));
        assert!(!tb.wrapped);
        assert_eq!((tb.bar.w, tb.bar.h), (1023.0, 44.0));
        // 960 wide (+ the slack) is too narrow for the one row now.
        assert!(layout(&inp(r(100.0, 100.0, 960.0, 300.0))).wrapped);
    }

    /// Invert has no size, Pixelate and Invert no color: those controls
    /// are there but disabled (they consume the press).
    #[test]
    fn size_and_color_controls_follow_the_tool() {
        let mut i = inp(r(100.0, 100.0, 1040.0, 300.0));
        i.sizes = false;
        i.colors = false;
        let tb = layout(&i);
        for a in [Act::Size(-1), Act::Size(1), Act::Palette] {
            let b = btn(&tb, a);
            assert_eq!(tb.act_at(Pt::new(b.x + 5.0, b.y + 5.0)), None, "{a:?}");
            assert!(tb.contains(Pt::new(b.x + 5.0, b.y + 5.0)));
        }
        assert!(tb.items.iter().any(|it| it.kind == Kind::Value && it.disabled));
        assert_eq!(tb.bar.w, 1023.0, "same width");
    }

    #[test]
    fn busy_swaps_outputs_for_ok_and_keeps_width() {
        let mut i = inp(r(100.0, 100.0, 1040.0, 300.0));
        i.busy = true;
        let tb = layout(&i);
        assert_eq!(tb.bar.w, 1023.0);
        assert!(tb.items.iter().any(|it| it.kind == Kind::Btn(Act::Accept)));
        assert!(!tb.items.iter().any(|it| it.kind == Kind::Btn(Act::Copy)));
    }

    #[test]
    fn wraps_on_narrow_selection() {
        let tb = layout(&inp(r(100.0, 100.0, 440.0, 170.0)));
        assert!(tb.wrapped);
        assert_eq!((tb.bar.w, tb.bar.h), (565.0, 85.0));
        assert!(tb.items.iter().any(|i| i.kind == Kind::SepH));
        assert!(btn(&tb, Act::Exit).y > btn(&tb, Act::Undo).y, "actions on row 2");
        assert!((btn(&tb, Act::Exit).x1() - (tb.bar.x1() - PAD)).abs() < 1e-3, "row 2 right-aligned");
    }

    #[test]
    fn below_and_centred() {
        let tb = layout(&inp(r(100.0, 100.0, 800.0, 300.0)));
        assert_eq!(tb.bar.y, 408.0);
        assert_eq!(cx(tb.bar), 500.0);
        assert!(!tb.above);
    }

    /// Near a monitor edge the bar slides along it, as close to centred as
    /// it can be (never over to the other side of the selection).
    #[test]
    fn slides_along_the_monitor_edges() {
        let tb = layout(&inp(r(1600.0, 100.0, 300.0, 300.0)));
        assert_eq!(tb.bar.x1(), 1908.0, "against the right margin");
        assert!(tb.bar.x < 1600.0);
        let tb = layout(&inp(r(20.0, 100.0, 300.0, 300.0)));
        assert_eq!(tb.bar.x, 12.0, "against the left margin");
        // A little off centre: only as far as it must.
        let tb = layout(&inp(r(300.0, 100.0, 900.0, 300.0)));
        assert_eq!(cx(tb.bar), 750.0, "room for it: centred");
        let tb = layout(&inp(r(300.0, 100.0, 300.0, 300.0)));
        assert!(tb.wrapped && cx(tb.bar) == 450.0, "wrapped, centred");
        // Selections narrower than the bar: centred all the same.
        for x in [600.0, 900.0, 1100.0] {
            let tb = layout(&inp(r(x, 300.0, 120.0, 90.0)));
            assert_eq!(cx(tb.bar), x + 60.0, "{x}");
        }
        // On a monitor that does not start at 0.
        let mut i = inp(r(1920.0 + 1200.0, 100.0, 60.0, 60.0));
        i.area = r(1920.0, 0.0, 1280.0, 720.0);
        let tb = layout(&i);
        assert!(tb.bar.x1() <= 1920.0 + 1280.0 - 12.0 + 0.01 && tb.bar.x >= 1932.0, "{:?}", tb.bar);
    }

    /// No room below: above, centred; no room above either: inside the
    /// selection's bottom edge.
    #[test]
    fn above_then_inside_stay_centred() {
        let tb = layout(&inp(r(400.0, 800.0, 600.0, 250.0)));
        assert!(tb.above && tb.bar.y1() == 792.0 && cx(tb.bar) == 700.0, "{:?}", tb.bar);
        let tb = layout(&inp(r(400.0, 20.0, 600.0, 1050.0)));
        assert!(tb.above, "anchored at its bottom");
        assert_eq!(tb.bar.y1(), 1062.0, "inside, 8 above the bottom edge");
        assert_eq!(cx(tb.bar), 700.0);
    }

    #[test]
    fn flips_above_near_bottom() {
        let tb = layout(&inp(r(100.0, 700.0, 1040.0, 340.0)));
        assert_eq!(tb.bar.y, 648.0);
        assert!(tb.above);
    }

    #[test]
    fn goes_inside_fullscreen_selection() {
        let tb = layout(&inp(r(0.0, 0.0, 1920.0, 1080.0)));
        assert_eq!(tb.bar.y1(), 1068.0, "clamped to the 12 px margin");
        assert_eq!(cx(tb.bar), 960.0, "centred");
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
        let mut i = inp(r(100.0, 100.0, 2110.0, 300.0));
        i.area = r(0.0, 0.0, 3840.0, 2160.0);
        i.s = 2.0;
        let tb = layout(&i);
        assert!(!tb.wrapped);
        assert_eq!(tb.bar.w, 2046.0);
        assert_eq!(btn(&tb, Act::Undo).w, 64.0);
    }

    /// Copy text sits right after Copy and acts like the other outputs.
    #[test]
    fn copy_text_follows_copy() {
        let tb = layout(&inp(r(100.0, 100.0, 1040.0, 300.0)));
        let (c, t) = (btn(&tb, Act::Copy), btn(&tb, Act::CopyText));
        assert_eq!(c.x1() + IN_GAP, t.x);
        assert_eq!(t.x1() + IN_GAP, btn(&tb, Act::Save).x);
        assert_eq!(tb.act_at(centre(t)), Some(Act::CopyText));
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
    fn palette_pops_below_bar_centred_with_the_rainbow_last() {
        let colors = [C4::rgb(1, 2, 3); 10];
        let mut i = inp(r(100.0, 100.0, 800.0, 300.0));
        i.palette = Some(&colors);
        let tb = layout(&i);
        assert!(!tb.above);
        let pop = tb.pop.expect("popover");
        assert_eq!((pop.w, pop.h), (360.0, 40.0));
        assert_eq!(pop.y, tb.bar.y1() + 6.0);
        assert_eq!(cx(pop), cx(btn(&tb, Act::Palette)));
        let dots: Vec<&Item> = tb.items.iter().filter(|it| matches!(it.kind, Kind::Dot(_))).collect();
        assert_eq!(dots.len(), 10);
        let d = dots[0].r;
        assert_eq!(tb.act_at(Pt::new(d.x + 3.0, d.y + 3.0)), Some(Act::Color(C4::rgb(1, 2, 3))));
        let rb = kind_at(&tb, |k| matches!(k, Kind::Rainbow(_)));
        assert!(rb.r.x > dots[9].r.x1() && rb.r.x1() <= pop.x1() - PAD + 0.01, "at the far right");
        assert_eq!(tb.act_at(centre(rb.r)), Some(Act::Wheel));
        // At the right edge of the monitor it stays on it.
        let mut i = inp(r(1700.0, 100.0, 200.0, 300.0));
        i.palette = Some(&colors);
        let tb = layout(&i);
        let pop = tb.pop.unwrap();
        assert!(pop.x1() <= 1908.0 && cx(pop) == cx(btn(&tb, Act::Palette)), "{pop:?}");
        // Over a palette button near the edge it is clamped to the margin.
        let mut i = inp(r(1700.0, 100.0, 200.0, 300.0));
        i.area = r(0.0, 0.0, 1650.0, 1080.0);
        i.sel = r(1400.0, 100.0, 200.0, 300.0);
        i.palette = Some(&colors);
        let tb = layout(&i);
        let pop = tb.pop.unwrap();
        assert!(pop.x1() <= 1638.0 + 0.01 && pop.x >= 12.0, "{pop:?}");
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
        let mut i = inp(r(100.0, 100.0, 1040.0, 300.0));
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

    fn centre(r: FRect) -> Pt {
        Pt::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    fn kind_at(tb: &Toolbar, f: impl Fn(&Kind) -> bool) -> Item {
        *tb.items.iter().find(|i| f(&i.kind)).expect("item")
    }

    #[test]
    fn record_button_joins_the_outputs() {
        let mut i = inp(r(100.0, 100.0, 1090.0, 300.0));
        i.record = true;
        let tb = layout(&i);
        assert_eq!(tb.bar.w, 1023.0 + BTN + IN_GAP);
        let rec = btn(&tb, Act::Record);
        assert_eq!(rec.x1() + IN_GAP, btn(&tb, Act::Copy).x, "first of the output group");
        assert_eq!(tb.act_at(centre(rec)), Some(Act::Record));
        // The busy OK mode has no Record button.
        i.busy = true;
        assert!(!layout(&i).items.iter().any(|it| it.kind == Kind::Btn(Act::Record)));
    }

    /// Pin to screen ends the output group (after Upload, before Close);
    /// the busy OK mode has none.
    #[test]
    fn pin_button_follows_upload() {
        let mut i = inp(r(100.0, 100.0, 900.0, 300.0));
        let tb = layout(&i);
        let pin = btn(&tb, Act::Pin);
        assert_eq!(btn(&tb, Act::Upload).x1() + IN_GAP, pin.x);
        assert!(btn(&tb, Act::Exit).x > pin.x1());
        assert_eq!(tb.act_at(centre(pin)), Some(Act::Pin));
        i.busy = true;
        assert!(!layout(&i).items.iter().any(|it| it.kind == Kind::Btn(Act::Pin)));
    }

    /// A right-to-left language runs the toolbar right to left: the same
    /// buttons in the same bar, mirrored within it (the tools start at the
    /// right), in the overlay and docked in the Edit window.
    #[test]
    fn right_to_left_runs_the_toolbar_backwards() {
        let sel = r(600.0, 300.0, 700.0, 400.0);
        let btns = |t: &Toolbar| t.items.iter().filter(|i| matches!(i.kind, Kind::Btn(_))).map(|i| (i.kind, i.r)).collect::<Vec<_>>();
        let en = layout(&inp(sel));
        let rtl = crate::i18n::with_lang("qps-rtl", || layout(&inp(sel)));
        assert_eq!(en.bar, rtl.bar);
        let (a, b) = (btns(&en), btns(&rtl));
        assert_eq!(a.len(), b.len());
        for ((ka, ra), (kb, rb)) in a.iter().zip(&b) {
            assert_eq!(ka, kb);
            assert!(((ra.x - en.bar.x) - (en.bar.x1() - rb.x1())).abs() < 0.01, "{ka:?}: {ra:?} vs {rb:?}");
        }
        assert!(b[0].1.x > b[b.len() - 1].1.x, "the first tool is rightmost");
        let strip = r(0.0, 0.0, 1200.0, 44.0);
        let d_en = layout_docked(&inp(sel), strip);
        let d_rtl = crate::i18n::with_lang("qps-rtl", || layout_docked(&inp(sel), strip));
        let (a, b) = (btns(&d_en), btns(&d_rtl));
        assert!(((a[0].1.x - strip.x) - (strip.x1() - b[0].1.x1())).abs() < 1.0, "docked: from the right end");
        assert!(b.iter().all(|(_, r)| r.x >= strip.x && r.x1() <= strip.x1()));
    }

    /// Every toolbar button's tooltip fits the monitor in the long
    /// pseudo-languages (accented and right to left), on a small screen.
    #[test]
    fn tooltips_fit_in_long_languages() {
        let small = FRect { x: 0.0, y: 0.0, w: 1280.0, h: 720.0 };
        let km = crate::keymap::Keymap::defaults();
        for lang in crate::ui::preview::langs() {
            crate::i18n::with_lang(lang, || {
                for s in [1.0f32, 1.5] {
                    let mut i = inp(r(100.0, 100.0, 1080.0, 500.0));
                    i.area = small;
                    i.s = s;
                    let tb = layout(&i);
                    let ui = crate::editor::chrome::Ui { th: &crate::theme::DARK, s, font: Some(crate::fonts::ui_font()) };
                    for it in &tb.items {
                        let Kind::Btn(a) = it.kind else { continue };
                        let (label, keys) = crate::editor::chrome::act_tip(a, &km);
                        let at = crate::editor::chrome::TipAt { anchor: it.r, area: small, below: false };
                        let t = crate::editor::chrome::tooltip_rect(&ui, at, label, &keys);
                        assert!(t.x >= small.x && t.x1() <= small.x1() + 0.5 && t.y >= small.y && t.y1() <= small.y1(), "{lang} {a:?} at {s}: {t:?}");
                    }
                }
            });
        }
    }
}
