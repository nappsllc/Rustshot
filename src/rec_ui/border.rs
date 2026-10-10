//! The red recording border: four thin strips just outside the recorded
//! area (click-through and excluded from the capture). Where the area
//! touches the edge of the screen the strip on that side moves inside it.

use crate::capture::IRect;

/// Border thickness in logical px (`border 2 px red`).
pub const WIDTH: f32 = 2.0;

/// Thickness at scale `s` (physical px, at least 1).
pub fn thickness(s: f32) -> i32 {
    ((WIDTH * s).round() as i32).max(1)
}

/// Top, bottom, left and right strips around `area`, `t` px thick, kept
/// inside `screen` (the virtual screen's bounds). Empty strips (a side
/// with no room at all) have zero size.
pub fn strips(area: IRect, t: i32, screen: IRect) -> [IRect; 4] {
    let (sx0, sy0) = (screen.0, screen.1);
    let (sx1, sy1) = (screen.0 + screen.2 as i32, screen.1 + screen.3 as i32);
    let x0 = (area.0 - t).max(sx0);
    let y0 = (area.1 - t).max(sy0);
    let x1 = (area.0 + area.2 as i32 + t).min(sx1);
    let y1 = (area.1 + area.3 as i32 + t).min(sy1);
    let (w, h) = ((x1 - x0).max(0), (y1 - y0).max(0));
    let t = t.min(w / 2).min(h / 2).max(0);
    let side_h = (h - 2 * t).max(0) as u32;
    [
        (x0, y0, w as u32, t as u32),
        (x0, y1 - t, w as u32, t as u32),
        (x0, y0 + t, t as u32, side_h),
        (x1 - t, y0 + t, t as u32, side_h),
    ]
}

/// The virtual screen's bounds: the union of `monitors`.
pub fn bounds(monitors: &[IRect]) -> IRect {
    let Some(first) = monitors.first() else { return (0, 0, 0, 0) };
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0 + first.2 as i32, first.1 + first.3 as i32);
    for m in monitors {
        x0 = x0.min(m.0);
        y0 = y0.min(m.1);
        x1 = x1.max(m.0 + m.2 as i32);
        y1 = y1.max(m.1 + m.3 as i32);
    }
    (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: IRect = (0, 0, 1920, 1080);

    #[test]
    fn strips_hug_the_area_from_outside() {
        let s = strips((100, 200, 300, 150), 2, SCREEN);
        assert_eq!(s[0], (98, 198, 304, 2));
        assert_eq!(s[1], (98, 350, 304, 2));
        assert_eq!(s[2], (98, 200, 2, 150));
        assert_eq!(s[3], (400, 200, 2, 150));
    }

    #[test]
    fn a_full_screen_area_gets_the_border_inside() {
        let s = strips(SCREEN, 2, SCREEN);
        assert_eq!(s[0], (0, 0, 1920, 2));
        assert_eq!(s[1], (0, 1078, 1920, 2));
        assert_eq!(s[2], (0, 2, 2, 1076));
        assert_eq!(s[3], (1918, 2, 2, 1076));
    }

    #[test]
    fn thickness_scales() {
        assert_eq!(thickness(1.0), 2);
        assert_eq!(thickness(1.5), 3);
        assert_eq!(thickness(2.0), 4);
        assert_eq!(thickness(0.2), 1);
    }

    #[test]
    fn bounds_of_monitors() {
        assert_eq!(bounds(&[(0, 0, 1920, 1080), (-1280, 100, 1280, 1024)]), (-1280, 0, 3200, 1124));
        assert_eq!(bounds(&[]), (0, 0, 0, 0));
    }
}
