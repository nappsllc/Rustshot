//! The 3-2-1 countdown: a round panel with the number, centred on the
//! recorded area (excluded from the capture, click-through).

use crate::editor::style::{surface, Ui, ALL_LAYERS};
use crate::objects::FRect;
use crate::uifb::{text_height, text_width, Fb};

/// Panel diameter and the shadow room around it (logical px).
const D: f32 = 112.0;
const MARGIN: f32 = 24.0;
const NUM: f32 = 56.0;
const CAPTION: f32 = 11.0;

/// The window size at scale `s` (physical px).
pub fn size(s: f32) -> (u32, u32) {
    let n = ((D + 2.0 * MARGIN) * s).ceil() as u32;
    (n, n)
}

/// Draw `n` into a window of [`size`].
pub fn draw(f: &mut Fb, ui: &Ui, n: u32) {
    let s = ui.s;
    let r = FRect { x: MARGIN * s, y: MARGIN * s, w: (D * s).round(), h: (D * s).round() };
    surface(f, ui, r, D / 2.0, &ALL_LAYERS, 1.0);
    let Some(font) = ui.font else { return };
    let num = n.to_string();
    let px = NUM * s;
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0 - 6.0 * s);
    f.draw_text(font, px, &num, cx - text_width(font, px, &num) / 2.0, cy - text_height(font, px) / 2.0, ui.th.text);
    let cap = "Recording in";
    let cpx = CAPTION * s;
    let y = r.y1() - 24.0 * s;
    f.draw_text(font, cpx, cap, cx - text_width(font, cpx, cap) / 2.0, y - text_height(font, cpx) / 2.0, ui.th.text_muted);
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
            let px = super::super::render_layer(w as usize, h as usize, |f| draw(f, &ui, 3));
            let a = |x: usize, y: usize| px[(y * w as usize + x) * 4 + 3];
            assert_eq!(a(0, 0), 0, "corner transparent");
            let c = (w / 2) as usize;
            assert!(a(c, (MARGIN + 6.0) as usize) > 200, "panel edge opaque");
            assert!(a((MARGIN + 4.0) as usize, (MARGIN + 4.0) as usize) < 120, "round: the square's corner is not filled");
        }
        assert_eq!(size(2.0).0, 2 * size(1.0).0);
    }
}
