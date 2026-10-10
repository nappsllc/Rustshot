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

/// The strips around `area` on each monitor it is on (`(monitor, t)`:
/// physical rect and thickness there). Each strip lies on one monitor: a
/// side is drawn on the monitor its edge is on, just outside the area, or
/// inside it where the edge touches that monitor's edge. Empty strips are
/// left out.
pub fn strips(area: IRect, monitors: &[(IRect, i32)]) -> Vec<IRect> {
    let mut out = Vec::new();
    for &(m, t) in monitors {
        for r in strips_on(area, t, m) {
            if r.2 > 0 && r.3 > 0 {
                out.push(r);
            }
        }
    }
    out
}

/// [`strips`] without the ones inside the area (where it touches a
/// screen edge): for X11, which cannot keep the border out of the capture.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn strips_outside(area: IRect, monitors: &[(IRect, i32)]) -> Vec<IRect> {
    let (ax0, ay0, ax1, ay1) = (area.0, area.1, area.0 + area.2 as i32, area.1 + area.3 as i32);
    strips(area, monitors)
        .into_iter()
        .filter(|r| {
            let (x0, y0, x1, y1) = (r.0, r.1, r.0 + r.2 as i32, r.1 + r.3 as i32);
            x0.max(ax0) >= x1.min(ax1) || y0.max(ay0) >= y1.min(ay1)
        })
        .collect()
}

/// The strips of `area` on monitor `m`, `t` px thick (none when the area
/// is not on it). Sides whose edge lies on another monitor are skipped;
/// the others span the area's outer box clipped to `m`.
pub fn strips_on(area: IRect, t: i32, m: IRect) -> Vec<IRect> {
    let (ax0, ay0, ax1, ay1) = (area.0, area.1, area.0 + area.2 as i32, area.1 + area.3 as i32);
    let (mx0, my0, mx1, my1) = (m.0, m.1, m.0 + m.2 as i32, m.1 + m.3 as i32);
    if ax0.max(mx0) >= ax1.min(mx1) || ay0.max(my0) >= ay1.min(my1) {
        return Vec::new();
    }
    let (x0, y0) = ((ax0 - t).max(mx0), (ay0 - t).max(my0));
    let (x1, y1) = ((ax1 + t).min(mx1), (ay1 + t).min(my1));
    let (w, h) = (x1 - x0, y1 - y0);
    let t = t.min(w / 2).min(h / 2).max(0);
    let (top, bottom, left, right) = (ay0 >= my0, ay1 <= my1, ax0 >= mx0, ax1 <= mx1);
    let side_y0 = if top { y0 + t } else { y0 };
    let side_h = (if bottom { y1 - t } else { y1 } - side_y0).max(0) as u32;
    let mut v = Vec::new();
    if top {
        v.push((x0, y0, w as u32, t as u32));
    }
    if bottom {
        v.push((x0, y1 - t, w as u32, t as u32));
    }
    if left {
        v.push((x0, side_y0, t as u32, side_h));
    }
    if right {
        v.push((x1 - t, side_y0, t as u32, side_h));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: IRect = (0, 0, 1920, 1080);

    #[test]
    fn strips_hug_the_area_from_outside() {
        let s = strips((100, 200, 300, 150), &[(SCREEN, 2)]);
        assert_eq!(s, [(98, 198, 304, 2), (98, 350, 304, 2), (98, 200, 2, 150), (400, 200, 2, 150)]);
    }

    #[test]
    fn a_full_screen_area_gets_the_border_inside() {
        let s = strips(SCREEN, &[(SCREEN, 2)]);
        assert_eq!(s, [(0, 0, 1920, 2), (0, 1078, 1920, 2), (0, 2, 2, 1076), (1918, 2, 2, 1076)]);
    }

    /// A 1080 px monitor beside a 1440 px one (scale 1 and 1.5): each strip
    /// stays on the monitor its edge is on, at that monitor's thickness.
    #[test]
    fn strips_per_monitor_of_different_heights() {
        let (left, right): (IRect, IRect) = ((0, 0, 1920, 1080), (1920, 0, 2560, 1440));
        let mons = [(left, 2), (right, 3)];
        // Across both, down to y = 1200 (below the left monitor's bottom).
        let s = strips((1000, 500, 2000, 700), &mons);
        assert_eq!(
            s,
            [
                // Left monitor: top and left sides; no bottom (its edge is off this
                // monitor) and no right (the area goes on to the right one).
                (998, 498, 922, 2),
                (998, 500, 2, 580),
                // Right monitor: top, bottom (outside, at y 1200) and right.
                (1920, 497, 1083, 3),
                (1920, 1200, 1083, 3),
                (3000, 500, 3, 700),
            ]
        );
        for r in &s {
            let on = |m: IRect| r.0 >= m.0 && r.1 >= m.1 && r.0 + r.2 as i32 <= m.0 + m.2 as i32 && r.1 + r.3 as i32 <= m.1 + m.3 as i32;
            assert!(on(left) || on(right), "{r:?} is on one monitor");
        }
        // The whole right monitor: inside on all four sides; nothing on the left one.
        let s = strips(right, &mons);
        assert_eq!(s, [(1920, 0, 2560, 3), (1920, 1437, 2560, 3), (1920, 3, 3, 1434), (4477, 3, 3, 1434)]);
        // The bottom of the left monitor: the bottom strip moves inside there.
        let s = strips((100, 900, 400, 180), &mons);
        assert_eq!(s[1], (98, 1078, 404, 2));
    }

    #[test]
    fn outside_strips_never_cover_the_area() {
        let s = strips_outside((100, 200, 300, 150), &[(SCREEN, 2)]);
        assert_eq!(s, strips((100, 200, 300, 150), &[(SCREEN, 2)]), "all four are outside");
        assert!(strips_outside(SCREEN, &[(SCREEN, 2)]).is_empty(), "a full screen area has none");
        // Touching the top and left edges: only the bottom and right strips.
        let s = strips_outside((0, 0, 300, 150), &[(SCREEN, 2)]);
        assert_eq!(s, [(0, 150, 302, 2), (300, 2, 2, 148)]);
    }

    #[test]
    fn thickness_scales() {
        assert_eq!(thickness(1.0), 2);
        assert_eq!(thickness(1.5), 3);
        assert_eq!(thickness(2.0), 4);
        assert_eq!(thickness(0.2), 1);
    }
}
