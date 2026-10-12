//! The 3-2-1 countdown (Recording board): a 96 px disc centred on the
//! recorded area with the number (44 px), ringed by a 4 px accent arc that
//! drains over each second (excluded from the capture, click-through).

use crate::editor::style::{surface, Ui, ALL_LAYERS};
use crate::objects::{FRect, Pt};
use crate::raster::Blend;
use crate::uifb::{text_height, text_width, Fb};

/// Disc diameter, the arc's width, and the shadow room around it (logical px).
const D: f32 = 96.0;
const ARC: f32 = 4.0;
const MARGIN: f32 = 24.0;
const NUM: f32 = 44.0;

/// The window size at scale `s` (physical px).
pub fn size(s: f32) -> (u32, u32) {
    let n = ((D + 2.0 * MARGIN) * s).ceil() as u32;
    (n, n)
}

/// Points of the arc around (`cx`, `cy`) at radius `r`: from 12 o'clock,
/// clockwise, `frac` of the way round.
pub fn arc_points(cx: f32, cy: f32, r: f32, frac: f32) -> Vec<Pt> {
    let frac = frac.clamp(0.0, 1.0);
    let n = ((96.0 * frac).ceil() as usize).max(1);
    (0..=n)
        .map(|i| {
            let a = std::f32::consts::TAU * frac * i as f32 / n as f32;
            Pt::new(cx + r * a.sin(), cy - r * a.cos())
        })
        .collect()
}

/// Draw `n` into a window of [`size`], the arc `frac` (1 → 0) of the way
/// round: the part of the second still to go.
pub fn draw(f: &mut Fb, ui: &Ui, n: u32, frac: f32) {
    let s = ui.s;
    let th = ui.th;
    let r = FRect { x: MARGIN * s, y: MARGIN * s, w: (D * s).round(), h: (D * s).round() };
    surface(f, ui, r, D / 2.0, &ALL_LAYERS, 1.0);
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    // The track (the board's 35 % accent ring), then the arc on it.
    let (ra, aw) = (r.w / 2.0 + ARC * s / 2.0, ARC * s);
    f.stroke_circle(cx, cy, ra, aw, th.accent.fade(0.35));
    if frac > 0.0 {
        f.surf().stroke_polyline(&arc_points(cx, cy, ra, frac), aw, th.accent, Blend::Normal);
    }
    let Some(font) = ui.font else { return };
    let num = n.to_string();
    let px = NUM * s;
    f.draw_text(font, px, &num, cx - text_width(font, px, &num) / 2.0, cy - text_height(font, px) / 2.0, th.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{DARK, LIGHT};

    #[test]
    fn number_is_drawn_inside_a_round_panel() {
        for th in [&DARK, &LIGHT] {
            let ui = Ui { th, s: 1.0, font: Some(crate::fonts::ui_font()) };
            let (w, h) = size(1.0);
            let px = super::super::render_layer(w as usize, h as usize, |f| draw(f, &ui, 3, 1.0));
            let a = |x: usize, y: usize| px[(y * w as usize + x) * 4 + 3];
            assert_eq!(a(0, 0), 0, "corner transparent");
            let c = (w / 2) as usize;
            assert!(a(c, (MARGIN + 6.0) as usize) > 200, "panel edge opaque");
            assert!(a((MARGIN + 4.0) as usize, (MARGIN + 4.0) as usize) < 120, "round: the square's corner is not filled");
        }
        assert_eq!(size(2.0).0, 2 * size(1.0).0);
    }

    /// The arc drains: full at the start of a second, half at its middle
    /// (the right half left, from 12 o'clock clockwise), none at the end.
    #[test]
    fn the_arc_drains_over_the_second() {
        let ui = Ui { th: &DARK, s: 1.0, font: None };
        let (w, h) = size(1.0);
        let at = |frac: f32, x: f32, y: f32| {
            let px = super::super::render_layer(w as usize, h as usize, |f| draw(f, &ui, 2, frac));
            let p = &px[(y as usize * w as usize + x as usize) * 4..][..4];
            (p[2], p[1], p[0])
        };
        let c = w as f32 / 2.0;
        let ra = D / 2.0 + ARC / 2.0;
        let (right, left) = ((c + ra, c), (c - ra, c));
        let ac = DARK.accent;
        let near = |p: (u8, u8, u8)| (p.0 as i32 - ac.r as i32).abs() < 12 && (p.2 as i32 - ac.b as i32).abs() < 12;
        assert!(near(at(1.0, right.0, right.1)) && near(at(1.0, left.0, left.1)), "full ring");
        assert!(near(at(0.5, right.0, right.1)) && !near(at(0.5, left.0, left.1)), "half: the right half");
        assert!(!near(at(0.0, right.0, right.1)), "drained: the track only");
        let pts = arc_points(0.0, 0.0, 10.0, 0.25);
        let end = pts.last().unwrap();
        assert!((end.x - 10.0).abs() < 1e-3 && end.y.abs() < 1e-3, "a quarter ends at 3 o'clock: {end:?}");
    }
}
