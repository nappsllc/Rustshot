//! Video mode's chrome (Video and Recording boards): the mode switch
//! (Screenshot | Video, keys 1 / 2) and the Close button above the
//! selection, and the setup panel, 380 px wide and centred under the
//! selection (below it when it fits, else above, else inside at the
//! bottom; sliding along a monitor edge): source (Screen / Window /
//! Region), the Camera, Microphone (with its level) and System audio rows,
//! the camera shape, the output format, the countdown (a GIF: its frame
//! rate and max length), the camera ring, and Start recording. Logical px
//! from the boards; `Input::s` scales them.

use super::toolbar::{Act, Dev, Input, Item, Kind, RecView, Toolbar, LIST_TEXT, LOOKING_CAMS};
use crate::objects::FRect;
use crate::rec::RecFormat;
use crate::rec_ui::RecSource;

pub const PANEL_W: f32 = 380.0;
const PAD: f32 = 10.0;
const GAP: f32 = 8.0;
/// A segmented control: its track (3 px padding) and segments.
const SEG_H: f32 = 34.0;
const SEG_IN: f32 = 3.0;
const SEG_GAP: f32 = 2.0;
const ROW_H: f32 = 40.0;
const ROW_NAME_X: f32 = 38.0;
const ROW_NAME_W: f32 = 92.0;
const TOGGLE_HIT: f32 = 44.0;
/// The 7-bar meter: 7 × 3 px bars, 2 px apart.
pub const METER_W: f32 = 7.0 * 3.0 + 6.0 * 2.0;
const CAPTION_H: f32 = 14.0;
const CAPTION_GAP: f32 = 5.0;
const SMALL_ROW_H: f32 = 32.0;
const START_H: f32 = 40.0;
const DEV_ITEM_H: f32 = 26.0;
/// At most this many devices are listed.
pub const DEV_MAX: usize = 8;
/// The mode switch and the Close button above the selection.
pub const MODE_H: f32 = 28.0;
const MODE_BTN: [f32; 2] = [100.0, 72.0];
const TOP_GAP: f32 = 8.0;
/// Between the panel and the selection, and the monitor margin.
const SEL_GAP: f32 = 8.0;
const MARGIN: f32 = 12.0;
const INSIDE_MIN_H: f32 = 120.0;
/// The grip hint under the camera bubble.
const GRIP_H: f32 = 22.0;
const GRIP_W: f32 = 156.0;
const GRIP_GAP: f32 = 12.0;
/// The ring row's controls (thinner, the readout, thicker, the swatch).
const RING_VALUE_W: f32 = 56.0;
const RING_BTN: f32 = 24.0;
const RING_PAL_COLS: usize = 6;
const DOT_BTN: f32 = 28.0;
const DOT_GAP: f32 = 4.0;

/// The mode switch's width (logical px).
pub fn mode_w() -> f32 {
    2.0 * SEG_IN + MODE_BTN[0] + SEG_GAP + MODE_BTN[1]
}

/// Top-left of a `bw`x`bh` panel centred under `sel` (`above`: it went
/// above the selection or inside its bottom edge): below when it fits,
/// else above, else inside at the bottom (a tall selection), else below
/// clamped; near a monitor edge it slides along it.
pub fn place_under(sel: FRect, area: FRect, (bw, bh): (f32, f32), s: f32) -> (f32, f32, bool) {
    let (m, g) = (MARGIN * s, SEL_GAP * s);
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
    (x, y, above)
}

/// The mode switch (and, in video mode, the Close button) above the
/// selection's top edge, 8 px over it; inside the top edge when there is
/// no room above or `avoid` (the toolbar or panel above) is in the way.
/// Returns the switch's rect; the items go to `items`.
pub fn place_top(sel: FRect, area: FRect, s: f32, video: bool, avoid: Option<FRect>, items: &mut Vec<Item>, extra: &mut Vec<FRect>) -> FRect {
    let (h, gap) = (MODE_H * s, TOP_GAP * s);
    let w = mode_w() * s;
    let above = sel.y - gap - h;
    let clash = |y: f32| avoid.is_some_and(|a| y < a.y1() && y + h > a.y && sel.x < a.x1() && sel.x1() > a.x);
    let y = if above >= area.y + 4.0 * s && !clash(above) { above } else { sel.y + gap };
    let x = sel.x.clamp(area.x, (area.x1() - w).max(area.x));
    let track = FRect { x, y, w, h };
    items.push(Item { kind: Kind::ModeTrack, r: track, disabled: false });
    let mut bx = x + SEG_IN * s;
    for (i, video_btn) in [false, true].into_iter().enumerate() {
        let bw = MODE_BTN[i] * s;
        let r = FRect { x: bx, y: y + SEG_IN * s, w: bw, h: h - 2.0 * SEG_IN * s };
        items.push(Item { kind: Kind::Mode { video: video_btn, on: video_btn == video }, r, disabled: false });
        bx += bw + SEG_GAP * s;
    }
    extra.push(track);
    if video {
        let cx = (sel.x1() - h).max(track.x1() + gap).min(area.x1() - h);
        let close = FRect { x: cx, y, w: h, h };
        items.push(Item { kind: Kind::Close, r: close, disabled: false });
        extra.push(close);
    }
    track
}

/// The value shown after the Max length label.
fn max_len_text(n: u32) -> &'static str {
    match n {
        15 => "15 s \u{b7} then auto-stop",
        60 => "60 s \u{b7} then auto-stop",
        _ => "30 s \u{b7} then auto-stop",
    }
}

fn fps_label(n: u32) -> &'static str {
    match n {
        10 => "10",
        15 => "15",
        24 => "24",
        30 => "30",
        60 => "60",
        _ => "?",
    }
}

fn len_label(n: u32) -> &'static str {
    match n {
        15 => "15",
        60 => "60",
        _ => "30",
    }
}

/// One segmented control at (x, y), `w` wide: its track and segments
/// (`(act, label, icon, on, disabled)`), each as wide as the others.
fn seg(local: &mut Vec<Item>, x: f32, y: f32, w: f32, h: f32, inset: f32, segs: &[(Act, &'static str, &'static str, bool, bool)]) {
    local.push(Item { kind: Kind::SegTrack, r: FRect { x, y, w, h }, disabled: false });
    let n = segs.len() as f32;
    let sw = (w - 2.0 * inset - (n - 1.0) * SEG_GAP) / n;
    for (i, &(act, label, icon, on, disabled)) in segs.iter().enumerate() {
        let r = FRect { x: x + inset + i as f32 * (sw + SEG_GAP), y: y + inset, w: sw, h: h - 2.0 * inset };
        local.push(Item { kind: Kind::Seg { act, label, icon, on }, r, disabled });
    }
}

/// Video mode: the top row and the setup panel for `rv`.
pub fn layout(inp: &Input, rv: &RecView) -> Toolbar {
    let s = inp.s;
    let mut local: Vec<Item> = Vec::new();
    let mut texts = vec![rv.mic_label.to_string(), rv.cam_label.to_string()];
    let inner = PANEL_W - 2.0 * PAD;
    let x0 = PAD;
    let mut y = PAD;
    // Source: Screen | Window | Region (F / W / R).
    let src = |v: RecSource| v == rv.source;
    seg(&mut local, x0, y, inner, SEG_H, SEG_IN, &[
        (Act::RecSource(RecSource::Screen), "Screen", "monitor", src(RecSource::Screen), false),
        (Act::RecSource(RecSource::Window), "Window", "window", src(RecSource::Window), false),
        (Act::RecSource(RecSource::Region), "Region", "region", src(RecSource::Region), false),
    ]);
    y += SEG_H + GAP;
    let mp4 = rv.format == RecFormat::Mp4;
    let row_r = |y: f32| FRect { x: x0, y, w: inner, h: ROW_H };
    let mut list = None;
    let ring_c = rv.ring.unwrap_or(rv.accent);
    let mut row = |local: &mut Vec<Item>, y: f32, act: Act, icon: &'static str, label: &'static str, value: Option<&'static str>, dev: Option<Dev>, on: bool, disabled: bool, meter: bool| {
        let r = row_r(y);
        local.push(Item { kind: Kind::Row { act, icon, label, value, dim: !on }, r, disabled });
        let tx = r.x1() - 6.0 - 32.0;
        let mut vx1 = tx - 10.0;
        // The camera on: its ring's swatch before the toggle (the ring's
        // palette and width).
        if act == Act::RecCamera && on {
            let sr = FRect { x: vx1 - RING_BTN, y: r.y + (ROW_H - RING_BTN) / 2.0, w: RING_BTN, h: RING_BTN };
            local.push(Item { kind: Kind::Swatch { act: Act::RingPalette, c: ring_c, on: rv.ring_pal }, r: sr, disabled: false });
            vx1 = sr.x - 8.0;
        }
        if meter {
            let mr = FRect { x: vx1 - METER_W, y: r.y + (ROW_H - 12.0) / 2.0, w: METER_W, h: 12.0 };
            let (lit, clip) = rv.meter.unwrap_or((0, false));
            local.push(Item { kind: Kind::Meter { lit, clip }, r: mr, disabled: false });
            vx1 = mr.x - 10.0;
        }
        if let Some(dev) = dev.filter(|_| !disabled) {
            let open = rv.devs.is_some_and(|d| d.0 == dev);
            let vx = r.x + ROW_NAME_X + ROW_NAME_W + 10.0;
            local.push(Item { kind: Kind::DevBox { dev, open }, r: FRect { x: vx, y: r.y + 6.0, w: (vx1 - vx).max(20.0), h: ROW_H - 12.0 }, disabled: false });
            if open {
                list = Some((dev, r));
            }
        }
        let hit_r = FRect { x: r.x1() - TOGGLE_HIT, y: r.y, w: TOGGLE_HIT, h: ROW_H };
        local.push(Item { kind: Kind::Switch { act, label, on }, r: hit_r, disabled });
    };
    // Camera: the device, on/off.
    let cam_on = rv.camera && rv.cams;
    let cam_label = if rv.cams_looking { LOOKING_CAMS } else { "Camera" };
    let cam_value = if rv.cams_looking { Some("") } else if rv.cams { None } else { Some(crate::cam::NO_CAMERA) };
    row(&mut local, y, Act::RecCamera, "video", cam_label, cam_value, rv.cams.then_some(Dev::Cam), cam_on, !rv.cams, false);
    let cam_row = row_r(y);
    y += ROW_H + GAP;
    // Microphone (MP4 only): the device and its level, on/off.
    let mic_on = mp4 && rv.mic;
    let (mic_icon, mic_value) = if mp4 { ("mic", None) } else { ("mic-off", Some(crate::rec_ui::NOT_IN_GIF)) };
    row(&mut local, y, Act::RecMic, mic_icon, "Microphone", mic_value, mp4.then_some(Dev::Mic), mic_on, !mp4, mic_on && rv.meter.is_some());
    y += ROW_H + GAP;
    // System audio (MP4 only).
    let sys_on = mp4 && rv.system;
    let sys_value = if !mp4 { crate::rec_ui::NOT_IN_GIF } else if sys_on { "On" } else { "Off" };
    row(&mut local, y, Act::RecSystem, "volume", "System audio", Some(sys_value), None, sys_on, !mp4, false);
    y += ROW_H + GAP;
    local.push(Item { kind: Kind::SepH, r: FRect { x: x0 + 4.0, y, w: inner - 8.0, h: 1.0 }, disabled: false });
    y += 1.0 + GAP;
    // Camera shape | Output | Countdown (a GIF: Frame rate).
    let third = if mp4 { 92.0 } else { 112.0 };
    let col = (inner - third - 2.0 * GAP) / 2.0;
    let cols = [(x0, col), (x0 + col + GAP, col), (x0 + 2.0 * (col + GAP), third)];
    let captions = ["Camera shape", "Output", if mp4 { "Countdown" } else { "Frame rate" }];
    for ((cx, cw), cap) in cols.iter().zip(captions) {
        local.push(Item { kind: Kind::Caption(cap), r: FRect { x: cx + 2.0, y, w: *cw, h: CAPTION_H }, disabled: false });
    }
    let sy = y + CAPTION_H + CAPTION_GAP;
    let round = rv.shape.is_round();
    seg(&mut local, cols[0].0, sy, cols[0].1, SEG_H, SEG_IN, &[
        (Act::RecShape(crate::cam::Shape::Circle), "", "round", round, false),
        (Act::RecShape(crate::cam::Shape::Rounded), "", "square", !round, false),
    ]);
    let checking = rv.mp4 == crate::rec_ui::Mp4::Checking;
    let mp4_off = rv.mp4 != crate::rec_ui::Mp4::Yes;
    seg(&mut local, cols[1].0, sy, cols[1].1, SEG_H, SEG_IN, &[
        (Act::RecFormat(RecFormat::Mp4), if checking { "checking\u{2026}" } else { "MP4" }, "film", mp4, mp4_off),
        (Act::RecFormat(RecFormat::Gif), "GIF", "gif", !mp4, false),
    ]);
    if mp4 {
        seg(&mut local, cols[2].0, sy, cols[2].1, SEG_H, SEG_IN, &[
            (Act::RecCountdown(false), "0", "", !rv.countdown, false),
            (Act::RecCountdown(true), "3s", "", rv.countdown, false),
        ]);
    } else {
        let fs: Vec<(Act, &'static str, &'static str, bool, bool)> =
            rv.fps_choices.iter().map(|&n| (Act::RecFps(n), fps_label(n), "", rv.fps == n, false)).collect();
        seg(&mut local, cols[2].0, sy, cols[2].1, SEG_H, SEG_IN, &fs);
    }
    y = sy + SEG_H + GAP;
    // A GIF: Max length 15 / 30 / 60 s, then it stops by itself.
    if !mp4 {
        let r = FRect { x: x0, y, w: inner, h: SMALL_ROW_H };
        local.push(Item { kind: Kind::Row { act: Act::RecGifMax(rv.gif_max), icon: "", label: "Max length", value: Some(max_len_text(rv.gif_max)), dim: false }, r, disabled: true });
        let sw = 96.0;
        let segs: Vec<(Act, &'static str, &'static str, bool, bool)> =
            crate::rec_ui::GIF_MAX.iter().map(|&n| (Act::RecGifMax(n), len_label(n), "", rv.gif_max == n, false)).collect();
        seg(&mut local, r.x1() - 6.0 - sw, y + (SMALL_ROW_H - 26.0) / 2.0, sw, 26.0, 2.0, &segs);
        y += SMALL_ROW_H + GAP;
    }
    // The camera ring's palette, below the camera row: thinner, the
    // width, thicker; the accent, the palette's colours, the rainbow.
    let ring_pal = (cam_on && rv.ring_pal).then(|| {
        let n = rv.ring_colors.len() + 2;
        let rows = n.div_ceil(RING_PAL_COLS);
        let pw = RING_PAL_COLS as f32 * DOT_BTN + (RING_PAL_COLS - 1) as f32 * DOT_GAP + 12.0;
        let ph = RING_BTN + 8.0 + rows as f32 * DOT_BTN + (rows - 1) as f32 * DOT_GAP + 12.0;
        FRect { x: cam_row.x1() - 6.0 - pw, y: cam_row.y1() + 2.0, w: pw, h: ph }
    });
    local.push(Item { kind: Kind::Primary { act: Act::RecStart, label: "Start recording" }, r: FRect { x: x0, y, w: inner, h: START_H }, disabled: false });
    y += START_H + PAD;
    let mut h = y;
    // The open list floats over the rows below its row: drawn (and hit) last.
    if let Some((dev, rr)) = list
        && let Some((_, names)) = rv.devs
    {
        let n = names.len().min(DEV_MAX);
        let lr = FRect { x: rr.x + ROW_NAME_X, y: rr.y1() + 2.0, w: rr.w - ROW_NAME_X, h: n as f32 * DEV_ITEM_H + 8.0 };
        texts.extend(names.iter().take(n).cloned());
        h = h.max(lr.y1() + PAD);
        local.push(Item { kind: Kind::DevList, r: lr, disabled: false });
        for i in 0..n {
            let r = FRect { x: lr.x + 4.0, y: lr.y + 4.0 + i as f32 * DEV_ITEM_H, w: lr.w - 8.0, h: DEV_ITEM_H };
            local.push(Item { kind: Kind::DevItem { dev, i, on: i == rv.dev_sel }, r, disabled: false });
        }
    }
    if let Some(pr) = ring_pal {
        let cur = ring_c;
        h = h.max(pr.y1() + PAD);
        local.push(Item { kind: Kind::DevList, r: pr, disabled: false });
        let by = pr.y + 6.0;
        let mx = pr.x + 6.0;
        let vx = mx + RING_BTN + 2.0;
        let px = vx + RING_VALUE_W + 2.0;
        local.push(Item { kind: Kind::Btn(Act::RingSize(-1)), r: FRect { x: mx, y: by, w: RING_BTN, h: RING_BTN }, disabled: rv.ring_w == 0 });
        local.push(Item { kind: Kind::RingValue { w: rv.ring_w, c: ring_c }, r: FRect { x: vx, y: by, w: RING_VALUE_W, h: RING_BTN }, disabled: false });
        local.push(Item { kind: Kind::Btn(Act::RingSize(1)), r: FRect { x: px, y: by, w: RING_BTN, h: RING_BTN }, disabled: rv.ring_w >= crate::config::RING_WIDTH_MAX });
        let top = by + RING_BTN + 8.0;
        let at = |i: usize| FRect {
            x: pr.x + 6.0 + (i % RING_PAL_COLS) as f32 * (DOT_BTN + DOT_GAP),
            y: top + (i / RING_PAL_COLS) as f32 * (DOT_BTN + DOT_GAP),
            w: DOT_BTN,
            h: DOT_BTN,
        };
        local.push(Item { kind: Kind::Swatch { act: Act::RingColor(None), c: rv.accent, on: rv.ring.is_none() }, r: at(0), disabled: false });
        for (i, c) in rv.ring_colors.iter().enumerate() {
            let on = rv.ring.is_some() && *c == cur;
            local.push(Item { kind: Kind::Swatch { act: Act::RingColor(Some(c.with_alpha(255))), c: c.with_alpha(255), on }, r: at(i + 1), disabled: false });
        }
        local.push(Item { kind: Kind::Rainbow(Act::RingWheel), r: at(rv.ring_colors.len() + 1), disabled: false });
    }
    let (pw, ph) = (PANEL_W * s, h * s);
    let (px, py, above) = place_under(inp.sel, inp.area, (pw, ph), s);
    let mut items: Vec<Item> = Vec::new();
    let mut extra = Vec::new();
    let bar = FRect { x: px, y: py, w: pw, h: ph };
    let avoid = above.then_some(bar).filter(|b| b.y1() <= inp.sel.y + 0.5);
    let mode = inp.top.then(|| place_top(inp.sel, inp.area, s, true, avoid, &mut items, &mut extra));
    for it in local {
        let r = FRect { x: px + it.r.x * s, y: py + it.r.y * s, w: it.r.w * s, h: it.r.h * s };
        items.push(Item { r, ..it });
    }
    // The grip hint under the bubble (it is shown, not dragged).
    if let Some(b) = inp.grip {
        let (gw, gh) = (GRIP_W * s, GRIP_H * s);
        let r = FRect { x: b.x + (b.w - gw) / 2.0, y: b.y1() + GRIP_GAP * s, w: gw, h: gh };
        items.push(Item { kind: Kind::Grip, r, disabled: false });
        extra.push(r);
    }
    // The camera ring's colour picker, by the panel.
    let pop = inp.pick.map(|pv| super::toolbar::place_picker(pv, bar, above, &mut items, inp));
    debug_assert!(texts.len() >= LIST_TEXT);
    Toolbar { bar, pop, items, wrapped: false, above, texts, extra, mode, video: true, docked: false }
}

/// The panel's items (not the top row or grip) lie inside it (tests).
#[cfg(test)]
pub fn inside_panel(tb: &Toolbar) -> bool {
    let first = tb.items.iter().position(|i| i.kind == Kind::SegTrack).unwrap_or(0);
    let p = tb.bar;
    tb.items[first..]
        .iter()
        .filter(|i| !matches!(i.kind, Kind::Grip | Kind::DevList | Kind::DevItem { .. }))
        .all(|it| super::toolbar::hit(p, crate::objects::Pt::new(it.r.x + 0.01, it.r.y + 0.01)) && it.r.x1() <= p.x1() + 0.01 && it.r.y1() <= p.y1() + 0.01)
}

#[cfg(test)]
mod tests {
    use super::super::toolbar::{layout, Input};
    use super::*;
    use crate::objects::Pt;
    use crate::uifb::C4;

    const AREA: FRect = FRect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 };
    const RING: [C4; 4] = [C4::rgb(240, 68, 56); 4];

    fn r(x: f32, y: f32, w: f32, h: f32) -> FRect {
        FRect { x, y, w, h }
    }

    fn rv(format: RecFormat) -> RecView<'static> {
        RecView {
            format,
            mp4: crate::rec_ui::Mp4::Yes,
            fps: if format == RecFormat::Gif { 15 } else { 30 },
            fps_choices: if format == RecFormat::Gif { &crate::rec_ui::GIF_FPS } else { &[30, 60] },
            system: true,
            mic: true,
            camera: false,
            cams: true,
            cams_looking: false,
            shape: crate::cam::Shape::Circle,
            mic_label: "Default microphone",
            cam_label: "Default camera",
            devs: None,
            dev_sel: 0,
            ring: None,
            ring_w: 2,
            ring_pal: false,
            ring_colors: &RING,
            accent: C4::rgb(0x8b, 0x93, 0xff),
            source: RecSource::Region,
            countdown: true,
            gif_max: 30,
            meter: Some((4, false)),
        }
    }

    fn input<'a>(sel: FRect, v: &'a RecView<'a>) -> Input<'a> {
        Input {
            sel,
            area: AREA,
            s: 1.0,
            busy: false,
            can_undo: false,
            can_redo: false,
            palette: None,
            record: true,
            rec: Some(v),
            grip: None,
            top: true,
            sizes: true,
            colors: true,
            fills: true,
            pick: None,
            save_menu: None,
        }
    }

    fn panel<'a>(sel: FRect, v: &'a RecView<'a>) -> Toolbar {
        layout(&input(sel, v))
    }

    fn find(tb: &Toolbar, f: impl Fn(&Kind) -> bool) -> Item {
        *tb.items.iter().find(|i| f(&i.kind)).expect("item")
    }

    fn centre(r: FRect) -> Pt {
        Pt::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    /// The board's selection: the panel 380 wide, centred under it, 8 px
    /// below; the mode switch above its top-left, Close at its top right.
    #[test]
    fn centred_below_the_selection() {
        let v = rv(RecFormat::Mp4);
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        assert!(tb.video && !tb.above);
        assert_eq!((tb.bar.w, tb.bar.x + tb.bar.w / 2.0, tb.bar.y), (PANEL_W, 720.0, 538.0));
        assert!(inside_panel(&tb), "every panel item lies inside it");
        let mode = tb.mode.unwrap();
        assert_eq!((mode.x, mode.y1(), mode.h), (240.0, 102.0, MODE_H));
        let close = find(&tb, |k| *k == Kind::Close).r;
        assert_eq!((close.x1(), close.y, close.w), (1200.0, mode.y, MODE_H));
        assert!(tb.contains(centre(mode)) && tb.contains(centre(close)), "they take presses");
        assert_eq!(tb.act_at(centre(close)), Some(Act::Exit));
        let video = find(&tb, |k| matches!(k, Kind::Mode { video: true, .. }));
        assert!(matches!(video.kind, Kind::Mode { on: true, .. }));
        assert_eq!(tb.act_at(centre(video.r)), Some(Act::Mode(true)));
        let shot = find(&tb, |k| matches!(k, Kind::Mode { video: false, .. }));
        assert_eq!(tb.act_at(centre(shot.r)), Some(Act::Mode(false)));
        // While the selection is dragged: no top row (nothing to repaint).
        let mut i = input(r(240.0, 110.0, 960.0, 420.0), &v);
        i.top = false;
        let tb = layout(&i);
        assert!(tb.mode.is_none() && !tb.items.iter().any(|it| matches!(it.kind, Kind::ModeTrack | Kind::Close)) && tb.extra.is_empty());
        i.rec = None;
        assert!(layout(&i).mode.is_none(), "the toolbar's switch too");
    }

    /// No room below: above (centred); none above either: inside the
    /// selection's bottom edge; a narrow selection at an edge: the panel
    /// slides along it, never off the monitor.
    #[test]
    fn above_inside_and_sliding() {
        let v = rv(RecFormat::Mp4);
        let tb = panel(r(240.0, 520.0, 960.0, 340.0), &v);
        assert!(tb.above && tb.bar.y1() == 512.0 && tb.bar.x + tb.bar.w / 2.0 == 720.0, "{:?}", tb.bar);
        let m = tb.mode.unwrap();
        assert!(m.y >= 520.0, "the switch moves inside, clear of the panel: {m:?}");
        let tb = panel(r(0.0, 0.0, 1440.0, 900.0), &v);
        assert!(tb.above && tb.bar.y1() == 888.0, "inside, kept 12 px off the bottom: {:?}", tb.bar);
        assert_eq!(tb.bar.x + tb.bar.w / 2.0, 720.0);
        assert_eq!(tb.mode.unwrap().y, 8.0, "no room above: inside the top edge");
        let tb = panel(r(1300.0, 100.0, 120.0, 100.0), &v);
        assert_eq!(tb.bar.x1(), 1428.0, "against the right margin");
        let tb = panel(r(10.0, 100.0, 120.0, 100.0), &v);
        assert_eq!(tb.bar.x, 12.0, "against the left margin");
        // A selection narrower than the panel, mid-screen: still centred.
        let tb = panel(r(600.0, 100.0, 100.0, 80.0), &v);
        assert_eq!(tb.bar.x + tb.bar.w / 2.0, 650.0);
        // At 2x everything doubles.
        let mut i = input(r(480.0, 220.0, 1920.0, 840.0), &v);
        i.s = 2.0;
        i.area = r(0.0, 0.0, 2880.0, 1800.0);
        let tb2 = layout(&i);
        let tb1 = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        assert_eq!((tb2.bar.w, tb2.bar.h), (2.0 * tb1.bar.w, 2.0 * tb1.bar.h));
    }

    /// The rows and their parts: the device opens its list, the switch
    /// toggles, the meter sits before the switch; the list floats below
    /// its row, capped, and wins the hits there.
    #[test]
    fn rows_switches_and_lists() {
        let mut v = rv(RecFormat::Mp4);
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        let mic_box = find(&tb, |k| matches!(k, Kind::DevBox { dev: Dev::Mic, .. }));
        assert_eq!(tb.act_at(centre(mic_box.r)), Some(Act::RecDevMenu(Dev::Mic)));
        for act in [Act::RecCamera, Act::RecMic, Act::RecSystem] {
            let sw = find(&tb, |k| matches!(k, Kind::Switch { act: a, .. } if *a == act));
            assert_eq!(tb.act_at(centre(sw.r)), Some(act), "{act:?}");
        }
        let meter = find(&tb, |k| matches!(k, Kind::Meter { .. }));
        assert_eq!(meter.kind, Kind::Meter { lit: 4, clip: false });
        assert_eq!((meter.r.w, meter.r.h), (METER_W, 12.0), "7 bars × 3 px, 2 px apart");
        let sw = find(&tb, |k| matches!(k, Kind::Switch { act: Act::RecMic, .. }));
        assert!(meter.r.x1() <= sw.r.x1() - 32.0 - 6.0, "before the toggle");
        let start = find(&tb, |k| matches!(k, Kind::Primary { .. }));
        assert_eq!(tb.act_at(centre(start.r)), Some(Act::RecStart));
        assert_eq!(start.r.h, 40.0);
        // No meter while the microphone is off.
        v.mic = false;
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        assert!(!tb.items.iter().any(|i| matches!(i.kind, Kind::Meter { .. })));
        // The list.
        v.mic = true;
        let names: Vec<String> = (0..20).map(|n| format!("Mic {n}")).collect();
        v.devs = Some((Dev::Mic, &names));
        v.dev_sel = 2;
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        let items: Vec<Item> = tb.items.iter().filter(|i| matches!(i.kind, Kind::DevItem { .. })).copied().collect();
        assert_eq!(items.len(), DEV_MAX, "capped");
        assert!(matches!(items[2].kind, Kind::DevItem { dev: Dev::Mic, i: 2, on: true }));
        let sys = find(&tb, |k| matches!(k, Kind::Row { act: Act::RecSystem, .. }));
        assert!(items[1].r.y < sys.r.y1(), "it floats over the rows below");
        assert_eq!(tb.act_at(centre(items[1].r)), Some(Act::RecDev(Dev::Mic, 1)));
        assert_eq!(&tb.texts[LIST_TEXT..], &names[..DEV_MAX]);
        assert!(items.last().unwrap().r.y1() <= tb.bar.y1(), "the panel grows to hold it");
    }

    /// GIF: the sound rows disabled ("Not recorded in GIF"), Frame rate
    /// 10 / 15 / 24 instead of the countdown, Max length 15 / 30 / 60.
    #[test]
    fn gif_rows_and_choices() {
        let v = rv(RecFormat::Gif);
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        assert!(inside_panel(&tb));
        for act in [Act::RecMic, Act::RecSystem] {
            let row = find(&tb, |k| matches!(k, Kind::Row { act: a, .. } if *a == act));
            assert!(row.disabled && matches!(row.kind, Kind::Row { value: Some(crate::rec_ui::NOT_IN_GIF), dim: true, .. }));
            assert_eq!(tb.act_at(centre(row.r)), None);
        }
        assert!(!tb.items.iter().any(|i| matches!(i.kind, Kind::Meter { .. } | Kind::DevBox { dev: Dev::Mic, .. })));
        let segs = |f: fn(&Act) -> bool| -> Vec<(Act, bool)> {
            tb.items
                .iter()
                .filter_map(|i| match i.kind {
                    Kind::Seg { act, on, .. } if f(&act) => Some((act, on)),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(segs(|a| matches!(a, Act::RecFps(_))), [(Act::RecFps(10), false), (Act::RecFps(15), true), (Act::RecFps(24), false)]);
        assert_eq!(segs(|a| matches!(a, Act::RecGifMax(_))), [(Act::RecGifMax(15), false), (Act::RecGifMax(30), true), (Act::RecGifMax(60), false)]);
        assert!(segs(|a| matches!(a, Act::RecCountdown(_))).is_empty());
        let mp4 = panel(r(240.0, 110.0, 960.0, 420.0), &rv(RecFormat::Mp4));
        let n = mp4.items.iter().filter(|i| matches!(i.kind, Kind::Seg { act: Act::RecCountdown(_), .. })).count();
        assert_eq!(n, 2, "MP4: Countdown 0 / 3s");
        assert!(tb.bar.h > mp4.bar.h, "the max length row");
    }

    /// The source segments mark the source; the shape is round or square
    /// (a sharp square from an old config shows as square).
    #[test]
    fn source_and_shape_segments() {
        let mut v = rv(RecFormat::Mp4);
        v.source = RecSource::Window;
        v.shape = crate::cam::Shape::Square;
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        let on = |act: Act| matches!(find(&tb, |k| matches!(k, Kind::Seg { act: a, .. } if *a == act)).kind, Kind::Seg { on: true, .. });
        assert!(on(Act::RecSource(RecSource::Window)) && !on(Act::RecSource(RecSource::Region)));
        assert!(on(Act::RecShape(crate::cam::Shape::Rounded)) && !on(Act::RecShape(crate::cam::Shape::Circle)));
        let first = tb.items.iter().position(|i| i.kind == Kind::SegTrack).unwrap();
        assert!(matches!(tb.items[first + 1].kind, Kind::Seg { label: "Screen", icon: "monitor", .. }), "Screen first");
        // The camera on: the ring's swatch in its row; its palette (the
        // width, the colours) below the row, the panel no taller.
        let h = tb.bar.h;
        v.camera = true;
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        assert_eq!(tb.bar.h, h, "the board's height");
        let sw = find(&tb, |k| matches!(k, Kind::Swatch { act: Act::RingPalette, .. }));
        let cam = find(&tb, |k| matches!(k, Kind::Row { act: Act::RecCamera, .. }));
        assert!(sw.r.y >= cam.r.y && sw.r.y1() <= cam.r.y1() && tb.act_at(centre(sw.r)) == Some(Act::RingPalette));
        assert!(!tb.items.iter().any(|i| i.kind == Kind::Btn(Act::RingSize(1))), "closed");
        v.ring_pal = true;
        let tb = panel(r(240.0, 110.0, 960.0, 420.0), &v);
        let plus = find(&tb, |k| *k == Kind::Btn(Act::RingSize(1)));
        assert!(plus.r.y > cam.r.y1() && tb.act_at(centre(plus.r)) == Some(Act::RingSize(1)));
        let acc = find(&tb, |k| matches!(k, Kind::Swatch { act: Act::RingColor(None), .. }));
        assert!(acc.r.y > plus.r.y1() && tb.act_at(centre(acc.r)) == Some(Act::RingColor(None)));
        assert!(inside_panel(&tb));
    }

    /// The grip hint: centred under the bubble, 12 px below it.
    #[test]
    fn grip_under_the_bubble() {
        let v = rv(RecFormat::Mp4);
        let mut i = input(r(240.0, 110.0, 960.0, 420.0), &v);
        i.grip = Some(r(264.0, 346.0, 160.0, 160.0));
        let tb = layout(&i);
        let g = find(&tb, |k| *k == Kind::Grip).r;
        assert_eq!((g.x + g.w / 2.0, g.y, g.h), (344.0, 518.0, GRIP_H));
        assert!(tb.extra.contains(&g));
        i.grip = None;
        assert!(!layout(&i).items.iter().any(|it| it.kind == Kind::Grip));
    }
}
