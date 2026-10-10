//! The recording control bar: a small toolbar-style panel with a red dot
//! and the timer, Pause/Resume, the microphone mute (when recording one)
//! and Stop. Logical px, scaled by `s`; drawn into a transparent layer
//! with `MARGIN` around the panel for its shadow.

use crate::editor::style::{fill, ring, round, surface, Ui, BTN};
use crate::icon_path::draw_icon;
use crate::objects::{FRect, Pt};
use crate::uifb::{text_height, text_width, Fb, C4};
use std::time::Duration;

/// Room around the panel for the shadow (logical px).
pub const MARGIN: f32 = 22.0;
const PAD: f32 = 6.0;
const BTN_GAP: f32 = 2.0;
const SEP_GAP: f32 = 6.0;
const TIMER_PAD: f32 = 10.0;
const DOT: f32 = 4.0;
const DOT_GAP: f32 = 8.0;
const TEXT: f32 = 13.0;
/// Shadow layers (`theme::SHADOW` indices) of the light theme's bar.
const LIGHT_LAYERS: [usize; 2] = [0, 1];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarAct {
    Pause,
    Mute,
    Stop,
}

/// What the bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct BarView {
    pub elapsed: Duration,
    pub paused: bool,
    /// The microphone's mute state when one is recorded.
    pub mic: Option<bool>,
    /// Stop pressed; finishing the file.
    pub stopping: bool,
    /// The minimal pill (timer and Stop only): on X11, when the bar has to
    /// sit inside the recorded area (a full-screen recording on the only
    /// monitor), where it is recorded too.
    pub compact: bool,
}

/// The bar in its window (physical px; the panel sits `MARGIN` inside).
#[derive(Clone, Debug, PartialEq)]
pub struct BarLayout {
    pub size: (u32, u32),
    pub panel: FRect,
    pub timer: FRect,
    pub sep: FRect,
    pub buttons: Vec<(BarAct, FRect)>,
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
    super::fmt_time(v.elapsed)
}

/// Lay the bar out for `v` at scale `ui.s` (the timer is as wide as
/// "00:00", so the bar does not jitter as the seconds tick).
pub fn layout(ui: &Ui, v: &BarView) -> BarLayout {
    let s = ui.s;
    let tt = timer_text(v);
    let digits = tw(ui, TEXT, "00:00").max(tw(ui, TEXT, &tt)) / s;
    let timer_w = TIMER_PAD + 2.0 * DOT + DOT_GAP + digits.ceil() + 4.0;
    let mut acts = Vec::new();
    if !v.compact {
        acts.push(BarAct::Pause);
        if v.mic.is_some() {
            acts.push(BarAct::Mute);
        }
    }
    acts.push(BarAct::Stop);
    let n = acts.len() as f32;
    let w = PAD + timer_w + SEP_GAP + 1.0 + SEP_GAP + n * BTN + (n - 1.0) * BTN_GAP + PAD;
    let h = BTN + 2.0 * PAD;
    let m = MARGIN;
    let px = |v: f32| v * s;
    let panel = FRect { x: px(m), y: px(m), w: px(w), h: px(h) };
    let timer = FRect { x: px(m + PAD), y: px(m + PAD), w: px(timer_w), h: px(BTN) };
    let sx = m + PAD + timer_w + SEP_GAP;
    let sep = FRect { x: px(sx), y: px(m + PAD + (BTN - 16.0) / 2.0), w: px(1.0).round().max(1.0), h: px(16.0) };
    let mut x = sx + 1.0 + SEP_GAP;
    let mut buttons = Vec::new();
    for a in acts {
        buttons.push((a, FRect { x: px(x), y: px(m + PAD), w: px(BTN), h: px(BTN) }));
        x += BTN + BTN_GAP;
    }
    let size = ((px(w + 2.0 * m)).ceil() as u32, (px(h + 2.0 * m)).ceil() as u32);
    BarLayout { size, panel, timer, sep, buttons }
}

/// The button under `p` (window px).
pub fn hit(l: &BarLayout, p: Pt) -> Option<BarAct> {
    l.buttons.iter().find(|(_, r)| p.x >= r.x && p.x < r.x1() && p.y >= r.y && p.y < r.y1()).map(|(a, _)| *a)
}

fn icon_of(a: BarAct, v: &BarView) -> &'static str {
    match a {
        BarAct::Pause if v.paused => "play",
        BarAct::Pause => "pause",
        BarAct::Mute if v.mic == Some(true) => "mic-off",
        BarAct::Mute => "mic",
        BarAct::Stop => "",
    }
}

/// Draw the bar (window px, origin at the window's top-left).
pub fn draw(f: &mut Fb, ui: &Ui, l: &BarLayout, v: &BarView, hover: Option<BarAct>, pressed: Option<BarAct>) {
    let th = ui.th;
    let s = ui.s;
    // Over the desktop a light theme's full shadow looks heavy: the two
    // tight layers only (1 px and 3 px down).
    let layers: &[usize] = if th.dark { &crate::editor::style::ALL_LAYERS } else { &LIGHT_LAYERS };
    surface(f, ui, l.panel, 12.0, layers, 1.0);
    // Timer: a red dot (grey while paused) and the time.
    let cy = l.timer.y + l.timer.h / 2.0;
    let dx = l.timer.x + TIMER_PAD * s + DOT * s;
    let dot = if v.paused || v.stopping { th.text_muted } else { super::RED };
    f.fill_circle(dx, cy, DOT * s, dot);
    let tc = if v.paused { th.text_muted } else { th.text };
    text(f, ui, TEXT, &timer_text(v), dx + DOT * s + DOT_GAP * s, cy, tc);
    let sep = round(l.sep);
    f.fill_rect(sep.x as i32, sep.y as i32, sep.w as i32, sep.h as i32, th.separator);
    let k = if v.stopping { 0.4 } else { 1.0 };
    for &(a, r) in &l.buttons {
        let r = round(r);
        let (hov, prs) = (hover == Some(a) && !v.stopping, pressed == Some(a) && !v.stopping);
        let danger = a == BarAct::Stop;
        let rad = 8.0 * s;
        let active = a == BarAct::Mute && v.mic == Some(true);
        if prs {
            fill(f, r, rad, th.bg_pressed);
        } else if hov {
            fill(f, r, rad, if danger { th.danger_bg } else { th.bg_hover });
        } else if active {
            fill(f, r, rad, th.danger_bg);
        }
        if active {
            ring(f, ui, r, rad, th.danger_fg.fade(0.35));
        }
        let fg = if active || (danger && hov) {
            th.danger_fg
        } else if hov || prs {
            th.icon_hover
        } else {
            th.icon
        }
        .fade(k);
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        if a == BarAct::Stop {
            // A filled red square.
            let q = (11.0 * s).round();
            let sq = FRect { x: (cx - q / 2.0).round(), y: (cy - q / 2.0).round(), w: q, h: q };
            fill(f, sq, 2.5 * s, super::RED.fade(k));
        } else {
            let isz = 18.0 * s;
            draw_icon(f, icon_of(a, v), cx - isz / 2.0, cy - isz / 2.0, isz, fg);
        }
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
        BarView { elapsed: Duration::from_secs(65), paused: false, mic, stopping: false, compact: false }
    }

    #[test]
    fn buttons_follow_the_microphone() {
        let l = layout(&ui(1.0), &view(None));
        let acts: Vec<BarAct> = l.buttons.iter().map(|b| b.0).collect();
        assert_eq!(acts, [BarAct::Pause, BarAct::Stop]);
        let l2 = layout(&ui(1.0), &view(Some(false)));
        let acts: Vec<BarAct> = l2.buttons.iter().map(|b| b.0).collect();
        assert_eq!(acts, [BarAct::Pause, BarAct::Mute, BarAct::Stop]);
        assert_eq!(l2.size.0 as f32 - l.size.0 as f32, BTN + BTN_GAP);
        assert_eq!(l.size.1 as f32, BTN + 2.0 * PAD + 2.0 * MARGIN);
        // The panel sits MARGIN inside the window.
        assert_eq!((l.panel.x, l.panel.y), (MARGIN, MARGIN));
        assert_eq!(l.panel.x1() + MARGIN, l.size.0 as f32);
    }

    #[test]
    fn the_compact_pill_has_the_timer_and_stop_only() {
        let full = layout(&ui(1.0), &view(Some(false)));
        let l = layout(&ui(1.0), &BarView { compact: true, ..view(Some(false)) });
        let acts: Vec<BarAct> = l.buttons.iter().map(|b| b.0).collect();
        assert_eq!(acts, [BarAct::Stop]);
        assert_eq!(full.size.0 as f32 - l.size.0 as f32, 2.0 * (BTN + BTN_GAP));
        assert_eq!(l.timer, full.timer);
    }

    #[test]
    fn hits_and_scale() {
        let l = layout(&ui(1.0), &view(Some(false)));
        for (a, r) in &l.buttons {
            assert_eq!(hit(&l, Pt::new(r.x + 4.0, r.y + 4.0)), Some(*a));
        }
        assert_eq!(hit(&l, Pt::new(l.timer.x + 4.0, l.timer.y + 4.0)), None);
        assert_eq!(hit(&l, Pt::new(1.0, 1.0)), None, "shadow margin");
        let l2 = layout(&ui(2.0), &view(Some(false)));
        assert!((l2.panel.w - 2.0 * l.panel.w).abs() <= 2.0, "{} vs {}", l2.panel.w, l.panel.w);
    }

    #[test]
    fn width_is_steady_while_the_seconds_tick() {
        let a = layout(&ui(1.0), &BarView { elapsed: Duration::from_secs(1), ..view(None) });
        let b = layout(&ui(1.0), &BarView { elapsed: Duration::from_secs(59 * 60 + 58), ..view(None) });
        assert_eq!(a.size, b.size);
    }

    #[test]
    fn stop_is_a_red_square_and_paused_shows_play() {
        let u = ui(1.0);
        let v = view(None);
        let l = layout(&u, &v);
        let (w, h) = (l.size.0 as usize, l.size.1 as usize);
        let px = super::super::render_layer(w, h, |f| draw(f, &u, &l, &v, None, None));
        let stop = l.buttons.iter().find(|b| b.0 == BarAct::Stop).unwrap().1;
        let (cx, cy) = ((stop.x + stop.w / 2.0) as usize, (stop.y + stop.h / 2.0) as usize);
        let p = &px[(cy * w + cx) * 4..][..4];
        assert_eq!((p[2], p[1], p[0], p[3]), (super::super::RED.r, super::super::RED.g, super::super::RED.b, 255));
        assert_eq!(icon_of(BarAct::Pause, &BarView { paused: true, ..v.clone() }), "play");
        assert_eq!(icon_of(BarAct::Pause, &v), "pause");
        assert_eq!(icon_of(BarAct::Mute, &view(Some(true))), "mic-off");
        // The shadow margin's corner is transparent.
        assert_eq!(px[3], 0);
    }
}
