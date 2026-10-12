//! The recording control pill (Recording board): the timer (a red dot,
//! amber and still while paused), then Pause and Stop, the microphone,
//! camera and system-sound toggles, and Discard; the compact pill is the
//! timer and Stop only. Logical px, scaled by `s`; drawn into a
//! transparent layer with `MARGIN` around the panel for its shadow.

use super::{AMBER, ON_REC, REC};
use crate::editor::style::{fill, ring, round, surface, Ui, BTN};
use crate::icon_path::draw_icon;
use crate::objects::{FRect, Pt};
use crate::uifb::{text_height, text_width, Fb, C4};
use std::time::Duration;

/// Room around the panel for the shadow (logical px).
pub const MARGIN: f32 = 22.0;
const PAD: f32 = 6.0;
/// Between the groups and separators (`.tb gap: 6px`), and inside a group.
const GAP: f32 = 6.0;
const IN_GAP: f32 = 2.0;
const TIMER_L: f32 = 8.0;
const TIMER_R: f32 = 10.0;
/// The dot's radius and its pulsing halo.
const DOT: f32 = 5.0;
const HALO: f32 = 3.0;
const DOT_GAP: f32 = 8.0;
const TEXT: f32 = 13.0;
/// The Stop button: its square, the gap to the word, its padding.
const STOP_SQ: f32 = 10.0;
const STOP_L: f32 = 10.0;
const STOP_R: f32 = 12.0;
/// Disabled buttons (`.bt.d`).
const DIM: f32 = 0.34;
/// Shadow layers (`theme::SHADOW` indices) of the light theme's bar.
const LIGHT_LAYERS: [usize; 2] = [0, 1];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarAct {
    Pause,
    Stop,
    /// The microphone on/off.
    Mute,
    /// The camera bubble hidden/shown.
    Camera,
    /// The system sound on/off.
    System,
    /// Stop and throw the recording away.
    Discard,
    /// The timer: the compact pill and back (H).
    Compact,
}

/// What the bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct BarView {
    pub elapsed: Duration,
    pub paused: bool,
    /// The microphone's mute state when one is recorded.
    pub mic: Option<bool>,
    /// Whether the camera bubble is hidden, when there is one.
    pub cam: Option<bool>,
    /// The system sound's mute state when it is recorded.
    pub system: Option<bool>,
    /// Stop pressed; finishing the file.
    pub stopping: bool,
    /// The compact pill (timer and Stop only): a region narrower than
    /// 520 px, the user's choice (H), or on X11 a bar that has to sit
    /// inside the recorded area (where it is recorded too).
    pub compact: bool,
    /// The screen is being chosen (Wayland: the portal's dialog is open):
    /// [`ASKING`] instead of the timer, and only Stop (which calls it off).
    pub asking: bool,
}

/// What the bar says while the screen is being chosen.
pub const ASKING: &str = "Asking…";

/// The bar in its window (physical px; the panel sits `MARGIN` inside).
#[derive(Clone, Debug, PartialEq)]
pub struct BarLayout {
    pub size: (u32, u32),
    pub panel: FRect,
    pub timer: FRect,
    /// The separators between the groups.
    pub seps: Vec<FRect>,
    pub buttons: Vec<(BarAct, FRect)>,
    /// Buttons shown but not usable (no microphone, camera or system sound).
    pub disabled: Vec<BarAct>,
}

fn tw(ui: &Ui, size: f32, s: &str) -> f32 {
    ui.font.map_or(s.chars().count() as f32 * size * 0.6 * ui.s, |f| text_width(f, size * ui.s, s))
}

fn text(f: &mut Fb, ui: &Ui, size: f32, s: &str, x: f32, cy: f32, c: C4) {
    if let Some(font) = ui.font {
        let px = size * ui.s;
        f.draw_text(font, px, s, x, cy - text_height(font, px) / 2.0, c);
    }
}

/// The timer's text.
fn timer_text(v: &BarView) -> String {
    if v.asking { ASKING.into() } else { super::fmt_time(v.elapsed) }
}

/// The Stop button's width (logical px).
fn stop_w(ui: &Ui) -> f32 {
    STOP_L + STOP_SQ + DOT_GAP + (tw(ui, 12.0, "Stop") / ui.s).ceil() + STOP_R
}

/// Lay the bar out for `v` at scale `ui.s` (the timer is as wide as
/// "00:00", so the bar does not jitter as the seconds tick).
pub fn layout(ui: &Ui, v: &BarView) -> BarLayout {
    let s = ui.s;
    let tt = timer_text(v);
    let digits = tw(ui, TEXT, "00:00").max(tw(ui, TEXT, &tt)) / s;
    let timer_w = TIMER_L + 2.0 * DOT + DOT_GAP + digits.ceil() + TIMER_R;
    let groups: Vec<Vec<BarAct>> = if v.compact || v.asking {
        vec![vec![BarAct::Stop]]
    } else {
        vec![vec![BarAct::Pause, BarAct::Stop], vec![BarAct::Mute, BarAct::Camera, BarAct::System], vec![BarAct::Discard]]
    };
    let mut disabled = Vec::new();
    if v.mic.is_none() {
        disabled.push(BarAct::Mute);
    }
    if v.cam.is_none() {
        disabled.push(BarAct::Camera);
    }
    if v.system.is_none() {
        disabled.push(BarAct::System);
    }
    let m = MARGIN;
    let px = |v: f32| v * s;
    let (y, h) = (m + PAD, BTN);
    let timer = FRect { x: px(m + PAD), y: px(y), w: px(timer_w), h: px(h) };
    let mut x = m + PAD + timer_w;
    let mut seps = Vec::new();
    let mut buttons = Vec::new();
    let sw = stop_w(ui);
    for g in &groups {
        x += GAP;
        seps.push(FRect { x: px(x), y: px(y + (h - 16.0) / 2.0), w: px(1.0).round().max(1.0), h: px(16.0) });
        x += 1.0 + GAP;
        for (i, a) in g.iter().enumerate() {
            if i > 0 {
                x += IN_GAP;
            }
            let w = if *a == BarAct::Stop { sw } else { BTN };
            buttons.push((*a, FRect { x: px(x), y: px(y), w: px(w), h: px(h) }));
            x += w;
        }
    }
    let w = x + PAD - m;
    let ph = h + 2.0 * PAD;
    let panel = FRect { x: px(m), y: px(m), w: px(w), h: px(ph) };
    let size = ((px(w + 2.0 * m)).ceil() as u32, (px(ph + 2.0 * m)).ceil() as u32);
    disabled.retain(|d| buttons.iter().any(|b| b.0 == *d));
    BarLayout { size, panel, timer, seps, buttons, disabled }
}

/// What is under `p` (window px): a usable button, or the timer (the
/// compact pill and back).
pub fn hit(l: &BarLayout, p: Pt) -> Option<BarAct> {
    let inside = |r: &FRect| p.x >= r.x && p.x < r.x1() && p.y >= r.y && p.y < r.y1();
    if let Some((a, _)) = l.buttons.iter().find(|(_, r)| inside(r)) {
        return (!l.disabled.contains(a)).then_some(*a);
    }
    inside(&l.timer).then_some(BarAct::Compact)
}

fn icon_of(a: BarAct, v: &BarView) -> &'static str {
    match a {
        BarAct::Pause => "pause",
        BarAct::Mute if v.mic == Some(true) => "mic-off",
        BarAct::Mute => "mic",
        BarAct::Camera if v.cam == Some(true) => "video-off",
        BarAct::Camera => "video",
        BarAct::System => "volume",
        BarAct::Discard => "x",
        BarAct::Stop | BarAct::Compact => "",
    }
}

fn mix(a: C4, b: C4, t: f32) -> C4 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    C4::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), a.a)
}

/// Draw the bar (window px, origin at the window's top-left).
pub fn draw(f: &mut Fb, ui: &Ui, l: &BarLayout, v: &BarView, hover: Option<BarAct>, pressed: Option<BarAct>) {
    let th = ui.th;
    let s = ui.s;
    // Over the desktop a light theme's full shadow looks heavy: the two
    // tight layers only (1 px and 3 px down).
    let layers: &[usize] = if th.dark { &crate::editor::style::ALL_LAYERS } else { &LIGHT_LAYERS };
    surface(f, ui, l.panel, 12.0, layers, 1.0);
    // Timer: the red dot (pulsing: its halo on every other second), amber
    // and still while paused, grey while asking or stopping.
    let cy = l.timer.y + l.timer.h / 2.0;
    let dx = l.timer.x + (TIMER_L + DOT) * s;
    let live = !(v.paused || v.stopping || v.asking);
    if live && v.elapsed.as_secs().is_multiple_of(2) {
        f.fill_circle(dx, cy, (DOT + HALO) * s, REC.fade(0.22));
    }
    let dot = if v.paused {
        AMBER
    } else if live {
        REC
    } else {
        th.text_muted
    };
    f.fill_circle(dx, cy, DOT * s, dot);
    text(f, ui, TEXT, &timer_text(v), dx + (DOT + DOT_GAP) * s, cy, th.text);
    for sep in &l.seps {
        let r = round(*sep);
        f.fill_rect(r.x as i32, r.y as i32, r.w as i32, r.h as i32, th.separator);
    }
    let k_all = if v.stopping { 0.4 } else { 1.0 };
    for &(a, r) in &l.buttons {
        let r = round(r);
        let off = l.disabled.contains(&a);
        let k = if off { DIM * k_all } else { k_all };
        let (hov, prs) = (hover == Some(a) && !v.stopping && !off, pressed == Some(a) && !v.stopping && !off);
        let rad = 8.0 * s;
        if a == BarAct::Stop {
            let bg = if prs {
                mix(REC, C4::rgb(0, 0, 0), 0.15)
            } else if hov {
                mix(REC, C4::rgb(255, 255, 255), 0.12)
            } else {
                REC
            };
            fill(f, r, rad, bg.fade(k));
            let q = (STOP_SQ * s).round();
            let sq = FRect { x: (r.x + STOP_L * s).round(), y: (r.y + (r.h - q) / 2.0).round(), w: q, h: q };
            fill(f, sq, 2.0 * s, ON_REC.fade(k));
            text(f, ui, 12.0, "Stop", sq.x1() + DOT_GAP * s, r.y + r.h / 2.0, ON_REC.fade(k));
            continue;
        }
        // Paused: Pause is the active (accent) Resume. Muted / hidden: the
        // slashed glyph on a 14 % red tint.
        let accent = a == BarAct::Pause && v.paused;
        let red = match a {
            BarAct::Mute => v.mic == Some(true),
            BarAct::Camera => v.cam == Some(true),
            BarAct::System => v.system == Some(true),
            _ => false,
        };
        if accent {
            fill(f, r, rad, if hov { th.accent_bg_hover } else { th.accent_bg }.fade(k));
            ring(f, ui, r, rad, th.accent_ring.fade(k));
        } else if red {
            fill(f, r, rad, REC.fade(if hov { 0.22 } else { 0.14 } * k));
        } else if prs {
            fill(f, r, rad, th.bg_pressed.fade(k));
        } else if hov {
            fill(f, r, rad, if a == BarAct::Discard { th.danger_bg } else { th.bg_hover }.fade(k));
        }
        let fg = if accent {
            th.accent_fg
        } else if red {
            REC
        } else if a == BarAct::Discard && hov {
            th.danger_fg
        } else if hov || prs {
            th.icon_hover
        } else {
            th.icon
        }
        .fade(k);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let isz = 20.0 * s;
        draw_icon(f, icon_of(a, v), cx - isz / 2.0, cy - isz / 2.0, isz, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::DARK;

    fn ui(s: f32) -> Ui<'static> {
        Ui { th: &DARK, s, font: Some(crate::fonts::ui_font()) }
    }

    fn view(mic: Option<bool>) -> BarView {
        BarView {
            elapsed: Duration::from_secs(65),
            paused: false,
            mic,
            cam: None,
            system: None,
            stopping: false,
            compact: false,
            asking: false,
        }
    }

    fn acts(l: &BarLayout) -> Vec<BarAct> {
        l.buttons.iter().map(|b| b.0).collect()
    }

    /// The full pill: Pause, Stop | microphone, camera, system sound |
    /// Discard; what is not recorded is there but disabled.
    #[test]
    fn the_full_pill_and_its_disabled_toggles() {
        use BarAct::*;
        let l = layout(&ui(1.0), &view(None));
        assert_eq!(acts(&l), [Pause, Stop, Mute, Camera, System, Discard]);
        assert_eq!(l.disabled, [Mute, Camera, System]);
        assert_eq!(l.seps.len(), 3);
        for d in &l.disabled {
            let r = l.buttons.iter().find(|b| b.0 == *d).unwrap().1;
            assert_eq!(hit(&l, Pt::new(r.x + 4.0, r.y + 4.0)), None, "{d:?} disabled");
        }
        let all = BarView { mic: Some(false), cam: Some(false), system: Some(false), ..view(None) };
        let l2 = layout(&ui(1.0), &all);
        assert!(l2.disabled.is_empty());
        assert_eq!(l2.size, l.size, "same width either way");
        assert_eq!(l.size.1 as f32, BTN + 2.0 * PAD + 2.0 * MARGIN);
        assert_eq!((l.panel.x, l.panel.y), (MARGIN, MARGIN));
        assert_eq!(l.panel.x1() + MARGIN, l.size.0 as f32);
    }

    #[test]
    fn the_compact_pill_has_the_timer_and_stop_only() {
        let full = layout(&ui(1.0), &view(Some(false)));
        let l = layout(&ui(1.0), &BarView { compact: true, ..view(Some(false)) });
        assert_eq!(acts(&l), [BarAct::Stop]);
        assert!(l.size.0 < full.size.0);
        assert_eq!(l.timer, full.timer);
        // The timer toggles it (H's mouse twin).
        assert_eq!(hit(&l, Pt::new(l.timer.x + 4.0, l.timer.y + 4.0)), Some(BarAct::Compact));
    }

    /// Narrower than 520 logical px, or asked for: compact.
    #[test]
    fn compact_rule() {
        assert!(super::super::compact_pill(519, 1.0, false));
        assert!(!super::super::compact_pill(520, 1.0, false));
        assert!(super::super::compact_pill(1000, 2.0, false), "500 logical px at 2x");
        assert!(super::super::compact_pill(1920, 1.0, true), "H");
    }

    #[test]
    fn asking_shows_the_word_and_stop_only() {
        let l = layout(&ui(1.0), &BarView { asking: true, ..view(Some(false)) });
        assert_eq!(acts(&l), [BarAct::Stop]);
        assert_eq!(timer_text(&BarView { asking: true, ..view(None) }), ASKING);
        assert_eq!(timer_text(&view(None)), "1:05");
    }

    #[test]
    fn hits_and_scale() {
        let v = BarView { mic: Some(false), cam: Some(false), system: Some(false), ..view(None) };
        let l = layout(&ui(1.0), &v);
        for (a, r) in &l.buttons {
            assert_eq!(hit(&l, Pt::new(r.x + 4.0, r.y + 4.0)), Some(*a));
        }
        assert_eq!(hit(&l, Pt::new(1.0, 1.0)), None, "shadow margin");
        let l2 = layout(&ui(2.0), &v);
        assert!((l2.panel.w - 2.0 * l.panel.w).abs() <= 4.0, "{} vs {}", l2.panel.w, l.panel.w);
    }

    #[test]
    fn width_is_steady_while_the_seconds_tick() {
        let a = layout(&ui(1.0), &BarView { elapsed: Duration::from_secs(1), ..view(None) });
        let b = layout(&ui(1.0), &BarView { elapsed: Duration::from_secs(59 * 60 + 58), ..view(None) });
        assert_eq!(a.size, b.size);
    }

    fn px_at(px: &[u8], w: usize, x: f32, y: f32) -> (u8, u8, u8, u8) {
        let p = &px[(y as usize * w + x as usize) * 4..][..4];
        (p[2], p[1], p[0], p[3])
    }

    /// Stop is a red button; paused, the dot is amber and Pause the accent
    /// Resume; muted and hidden show the slashed glyphs on a red tint.
    #[test]
    fn states_are_drawn() {
        let u = ui(1.0);
        let v = BarView { mic: Some(false), cam: Some(false), system: Some(false), ..view(None) };
        let l = layout(&u, &v);
        let (w, h) = (l.size.0 as usize, l.size.1 as usize);
        let draw_v = |v: &BarView| super::super::render_layer(w, h, |f| draw(f, &u, &l, v, None, None));
        let px = draw_v(&v);
        let stop = l.buttons.iter().find(|b| b.0 == BarAct::Stop).unwrap().1;
        assert_eq!(px_at(&px, w, stop.x1() - 3.0, stop.y + stop.h / 2.0), (REC.r, REC.g, REC.b, 255));
        assert_eq!(px[3], 0, "the shadow margin's corner is transparent");
        // Paused: the dot is amber.
        let dot = (l.timer.x + (TIMER_L + DOT) * u.s, l.timer.y + l.timer.h / 2.0);
        assert_eq!(px_at(&draw_v(&BarView { paused: true, ..v.clone() }), w, dot.0, dot.1), (AMBER.r, AMBER.g, AMBER.b, 255));
        assert_eq!(px_at(&px, w, dot.0, dot.1), (REC.r, REC.g, REC.b, 255));
        assert_eq!(icon_of(BarAct::Mute, &BarView { mic: Some(true), ..v.clone() }), "mic-off");
        assert_eq!(icon_of(BarAct::Camera, &BarView { cam: Some(true), ..v.clone() }), "video-off");
        // Muted: a red tint behind the glyph (its corner, clear of the glyph).
        let mic = l.buttons.iter().find(|b| b.0 == BarAct::Mute).unwrap().1;
        let muted = draw_v(&BarView { mic: Some(true), ..v.clone() });
        let (r0, g0, ..) = px_at(&px, w, mic.x + 4.0, mic.y + 6.0);
        let (r1, g1, ..) = px_at(&muted, w, mic.x + 4.0, mic.y + 6.0);
        assert!(r1 > r0 + 10 && r1 > g1, "{r0},{g0} -> {r1},{g1}");
    }
}
