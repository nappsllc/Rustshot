//! Screen capture primitives shared across platforms: monitor metadata,
//! shots, region parsing and cropping. Per-OS capture lives in
//! `capture_win.rs` / `capture_linux.rs` / `capture_macos.rs`.

use crate::pixbuf::PixBuf;
use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone)]
pub struct MonInfo {
    /// Physical position in virtual-screen coordinates.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f32,
    pub primary: bool,
}

impl MonInfo {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w as i32 && y >= self.y && y < self.y + self.h as i32
    }
}

pub fn monitor_at_cursor(mons: &[MonInfo]) -> usize {
    let (cx, cy) = cursor_pos();
    mons
        .iter()
        .position(|m| m.contains(cx, cy))
        .unwrap_or_else(|| mons.iter().position(|m| m.primary).unwrap_or(0))
}

/// `Some(scale)` when every monitor reports the same scale factor.
pub fn uniform_scale(mons: &[MonInfo]) -> Option<f32> {
    let first = mons.first()?.scale;
    mons.iter().all(|m| (m.scale - first).abs() < 0.01).then_some(first)
}

/// A rectangle in image pixels: x, y, w, h.
pub type IRect = (i32, i32, u32, u32);

/// A captured region of the desktop in physical pixels.
#[derive(Clone)]
pub struct Shot {
    /// Physical position of the top-left corner in virtual-screen coords.
    pub origin: (i32, i32),
    pub size: (u32, u32),
    /// Scale factor that maps physical pixels to logical points for the
    /// window that will display this shot.
    pub scale: f32,
    pub image: PixBuf,
    /// Monitors covered by this shot, image coordinates; never empty.
    pub monitors: Vec<IRect>,
}

/// Monitors intersected with a shot at `origin`/`size`, in image coordinates.
/// Never empty: falls back to the whole shot.
pub fn monitors_in_shot(mons: &[MonInfo], origin: (i32, i32), size: (u32, u32)) -> Vec<IRect> {
    let (sw, sh) = (size.0 as i32, size.1 as i32);
    let mut out: Vec<IRect> = mons
        .iter()
        .filter_map(|m| {
            let x0 = (m.x - origin.0).max(0);
            let y0 = (m.y - origin.1).max(0);
            let x1 = (m.x + m.w as i32 - origin.0).min(sw);
            let y1 = (m.y + m.h as i32 - origin.1).min(sh);
            (x1 > x0 && y1 > y0).then(|| (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
        })
        .collect();
    if out.is_empty() {
        out.push((0, 0, size.0, size.1));
    }
    out
}

impl Shot {
    /// Convert a global (virtual-screen physical) point to image coordinates.
    pub fn global_to_image(&self, p: (i32, i32)) -> (i32, i32) {
        (p.0 - self.origin.0, p.1 - self.origin.1)
    }
}

fn union_rect(mons: &[MonInfo]) -> (i32, i32, u32, u32) {
    let min_x = mons.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = mons.iter().map(|m| m.y).min().unwrap_or(0);
    let max_x = mons
        .iter()
        .map(|m| m.x + m.w as i32)
        .max()
        .unwrap_or(0);
    let max_y = mons
        .iter()
        .map(|m| m.y + m.h as i32)
        .max()
        .unwrap_or(0);
    (min_x, min_y, (max_x - min_x) as u32, (max_y - min_y) as u32)
}

/// Geometry of a single monitor's shot (no pixels yet).
fn monitor_geom(mons: &[MonInfo], index: usize) -> Result<Shot> {
    let m = mons
        .get(index)
        .ok_or_else(|| anyhow!("monitor index {index} out of range"))?;
    Ok(Shot {
        origin: (m.x, m.y),
        size: (m.w, m.h),
        scale: m.scale,
        image: PixBuf::default(),
        monitors: vec![(0, 0, m.w, m.h)],
    })
}

/// Geometry of the whole virtual desktop's shot (no pixels yet).
fn span_geom(mons: &[MonInfo]) -> Shot {
    let (ox, oy, w, h) = union_rect(mons);
    let scale =
        uniform_scale(mons).unwrap_or_else(|| mons.first().map(|m| m.scale).unwrap_or(1.0));
    Shot {
        origin: (ox, oy),
        size: (w, h),
        scale,
        image: PixBuf::default(),
        monitors: monitors_in_shot(mons, (ox, oy), (w, h)),
    }
}

/// Fill a geometry-only shot with the screen pixels.
fn grab_into(mut shot: Shot) -> Result<Shot> {
    shot.image = gdi_capture(shot.origin.0, shot.origin.1, shot.size.0, shot.size.1)?;
    // HDR monitors (Windows): their part read in FP16 and tone-mapped.
    #[cfg(windows)]
    for (r, px) in crate::hdr::win::grab((shot.origin.0, shot.origin.1, shot.size.0, shot.size.1)) {
        crate::hdr::win::paste(&mut shot.image, (r.0 - shot.origin.0, r.1 - shot.origin.1), &px);
    }
    Ok(shot)
}

/// Capture a single monitor.
pub fn grab_monitor(index: usize) -> Result<Shot> {
    grab_into(monitor_geom(&monitors()?, index)?)
}

/// What the interactive editor should capture: a specific monitor, the
/// monitor under the cursor, or the full span (only when all monitors
/// share one scale factor). Geometry only: `image` stays empty, so a
/// backend can capture the pixels itself (the Windows GDI overlay).
pub fn plan_edit(screen: Option<u32>, active_monitor_only: bool) -> Result<Shot> {
    let mons = monitors()?;
    let span_ok = uniform_scale(&mons).is_some() && !active_monitor_only;
    match screen {
        Some(n) => monitor_geom(&mons, n as usize),
        None if span_ok => Ok(span_geom(&mons)),
        None => monitor_geom(&mons, monitor_at_cursor(&mons)),
    }
}

/// `plan_edit` plus the pixels.
pub fn grab_edit(screen: Option<u32>, active_monitor_only: bool) -> Result<Shot> {
    grab_into(plan_edit(screen, active_monitor_only)?)
}

/// Global rect (virtual-screen physical) of a monitor or `all`.
pub fn region_of(spec: &str) -> Result<(i32, i32, u32, u32)> {
    // A geometry needs no display; only "all" / "screenN" ask for the monitors.
    if spec == "all" {
        return Ok(union_rect(&monitors()?));
    }
    if let Some(n) = spec.strip_prefix("screen") {
        let n: usize = n.parse().context("bad screen number")?;
        let mons = monitors()?;
        let m = mons
            .get(n)
            .ok_or_else(|| anyhow!("monitor index {n} out of range"))?;
        return Ok((m.x, m.y, m.w, m.h));
    }
    parse_geometry(spec)
}

/// Parse `WxH+X+Y` (Flameshot `--region` format) into a global rect.
/// Signs belong to the numbers: `100x100+-20+50` or `100x100-20-30` both
/// mean x = -20, y = -30 in the second case only because the separator `-`
/// doubles as the sign; `+0+0` style is the canonical form.
pub fn parse_geometry(spec: &str) -> Result<(i32, i32, u32, u32)> {
    let x_pos = spec
        .find('x')
        .ok_or_else(|| anyhow!("region must look like WxH+X+Y, got {spec:?}"))?;
    let rest_start = spec[x_pos + 1..]
        .find(['+', '-'])
        .map(|i| x_pos + 1 + i)
        .ok_or_else(|| anyhow!("region must look like WxH+X+Y, got {spec:?}"))?;
    let (wh, rest) = spec.split_at(rest_start);
    let (w, h) = wh
        .split_once('x')
        .ok_or_else(|| anyhow!("region must look like WxH+X+Y, got {spec:?}"))?;
    let w: u32 = w.parse().context("bad region width")?;
    let h: u32 = h.parse().context("bad region height")?;
    let (x, y) = parse_signed_pair(rest)?;
    Ok((x, y, w, h))
}

/// Parse something like `+100+50`, `-20-30`, `100+50` into two signed ints.
fn parse_signed_pair(s: &str) -> Result<(i32, i32)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut nums = Vec::new();
    while i < b.len() {
        let mut sign = 1i32;
        while i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            if b[i] == b'-' {
                sign = -sign;
            }
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if start == i {
            return Err(anyhow!("bad region offsets in {s:?}"));
        }
        nums.push(sign * s[start..i].parse::<i32>()?);
    }
    match nums.len() {
        2 => Ok((nums[0], nums[1])),
        _ => Err(anyhow!("region must have exactly two offsets, got {s:?}")),
    }
}

/// Crop a global rect out of a shot, clamped to its bounds.
pub fn crop_global(shot: &Shot, rect: (i32, i32, u32, u32)) -> Result<PixBuf> {
    let (lx, ly) = shot.global_to_image((rect.0, rect.1));
    let x = lx.clamp(0, shot.image.width() as i32 - 1) as u32;
    let y = ly.clamp(0, shot.image.height() as i32 - 1) as u32;
    let w = rect.2.min(shot.image.width() - x);
    let h = rect.3.min(shot.image.height() - y);
    if w == 0 || h == 0 {
        return Err(anyhow!("region is outside the captured screen"));
    }
    Ok(shot.image.crop(x, y, w, h))
}

/// macOS units. Quartz lays displays out in points; the shared code
/// (monitors, shots, the editor's selection, the recording area, the
/// camera bubble's area) works in pixels. On macOS those pixels are global
/// points × one backing factor `k` for the whole desktop: the largest
/// display scale (2 with a Retina display; 1 without). So shots and the
/// overlay are drawn at Retina density, a 1× display beside a Retina one
/// is captured at `k` too (the window server scales it down), and every
/// AppKit / CoreGraphics / ScreenCaptureKit call converts back to points
/// with these helpers. Platform-neutral so they are tested everywhere.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod pts {
    use super::IRect;

    /// The backing factor for displays of these pixels-per-point scales:
    /// the largest (finite, positive) one, at least 1.
    pub fn factor(scales: &[f64]) -> f64 {
        scales.iter().copied().filter(|s| s.is_finite() && *s > 0.0).fold(1.0, f64::max)
    }

    /// Pixels per point of a display mode `px_w` pixels wide that is
    /// `pt_w` points wide (1 when unknown).
    pub fn mode_scale(px_w: usize, pt_w: usize) -> f64 {
        if px_w > 0 && pt_w > 0 { px_w as f64 / pt_w as f64 } else { 1.0 }
    }

    fn k_of(k: f64) -> f64 {
        if k.is_finite() && k > 0.0 { k } else { 1.0 }
    }

    /// A coordinate in points → pixels.
    pub fn px(v: f64, k: f64) -> i32 {
        (v * k_of(k)).round() as i32
    }

    /// A coordinate in pixels → points.
    pub fn pt(v: i32, k: f64) -> f64 {
        f64::from(v) / k_of(k)
    }

    /// A rect in points (x, y, w, h) → pixels; the edges are rounded (so
    /// rects that touch in points touch in pixels).
    pub fn rect_px((x, y, w, h): (f64, f64, f64, f64), k: f64) -> IRect {
        let (x0, y0, x1, y1) = (px(x, k), px(y, k), px(x + w.max(0.0), k), px(y + h.max(0.0), k));
        (x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32)
    }

    /// A rect in pixels → points (exact).
    pub fn rect_pt(r: IRect, k: f64) -> (f64, f64, f64, f64) {
        (pt(r.0, k), pt(r.1, k), f64::from(r.2) / k_of(k), f64::from(r.3) / k_of(k))
    }

    /// A rect in pixels → whole points (edges rounded).
    pub fn rect_pt_i(r: IRect, k: f64) -> IRect {
        let k = k_of(k);
        let e = |v: i64| (v as f64 / k).round() as i32;
        let (x0, y0) = (e(i64::from(r.0)), e(i64::from(r.1)));
        let (x1, y1) = (e(i64::from(r.0) + i64::from(r.2)), e(i64::from(r.1) + i64::from(r.3)));
        (x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32)
    }
}

#[cfg(windows)]
#[path = "capture_win.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "capture_linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "capture_macos.rs"]
mod imp;

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_points_and_pixels_convert_by_the_backing_factor() {
        assert_eq!(pts::factor(&[1.0, 2.0]), 2.0, "the largest display scale");
        assert_eq!(pts::factor(&[]), 1.0);
        assert_eq!(pts::factor(&[0.0, f64::NAN, 0.5]), 1.0, "never below 1");
        assert_eq!(pts::mode_scale(2880, 1440), 2.0, "Retina");
        assert_eq!(pts::mode_scale(3360, 1680), 2.0, "a scaled 'more space' mode");
        assert_eq!(pts::mode_scale(1920, 1920), 1.0);
        assert_eq!(pts::mode_scale(0, 1440), 1.0, "unknown");
        // A display at -1440 pt left of the main one.
        assert_eq!(pts::rect_px((-1440.0, 0.0, 1440.0, 900.0), 2.0), (-2880, 0, 2880, 1800));
        assert_eq!(pts::rect_pt((-2880, 0, 2880, 1800), 2.0), (-1440.0, 0.0, 1440.0, 900.0));
        assert_eq!(pts::rect_pt((1, 3, 5, 7), 2.0), (0.5, 1.5, 2.5, 3.5), "odd pixels are half points");
        // Edges are rounded (halves away from zero), not the size.
        assert_eq!(pts::rect_pt_i((-2881, 1, 2882, 1801), 2.0), (-1441, 1, 1442, 900));
        assert_eq!(pts::rect_pt_i((201, 99, 2000, 1000), 2.0), (101, 50, 1000, 500));
        assert_eq!(pts::px(10.25, 2.0), 21);
        assert_eq!(pts::pt(21, 2.0), 10.5);
        // Round trips, and k = 1 changes nothing.
        for r in [(0, 0, 640, 360), (-1920, 200, 1920, 1080), (7, 9, 3, 5)] {
            assert_eq!(pts::rect_px(pts::rect_pt(r, 2.0), 2.0), r);
            assert_eq!(pts::rect_pt_i(r, 1.0), r);
        }
        assert_eq!(pts::rect_pt((4, 4, 4, 4), 0.0), (4.0, 4.0, 4.0, 4.0), "a bad factor is 1");
    }

    #[test]
    fn monitors_in_shot_are_image_relative_and_clipped() {
        let mons = vec![
            MonInfo { x: -1920, y: 200, w: 1920, h: 1080, scale: 1.0, primary: false },
            MonInfo { x: 0, y: 0, w: 2560, h: 1440, scale: 1.0, primary: true },
        ];
        let (ox, oy, w, h) = union_rect(&mons);
        assert_eq!((ox, oy, w, h), (-1920, 0, 4480, 1440));
        let r = monitors_in_shot(&mons, (ox, oy), (w, h));
        assert_eq!(r, vec![(0, 200, 1920, 1080), (1920, 0, 2560, 1440)]);
    }

    #[test]
    fn monitors_in_shot_never_empty() {
        assert_eq!(monitors_in_shot(&[], (0, 0), (800, 600)), vec![(0, 0, 800, 600)]);
    }

    #[test]
    fn parses_geometry() {
        assert_eq!(parse_geometry("800x600+100+50").unwrap(), (100, 50, 800, 600));
        assert_eq!(parse_geometry("10x10-20-30").unwrap(), (-20, -30, 10, 10));
        assert!(parse_geometry("nope").is_err());
    }

    #[test]
    #[ignore = "live display access"]
    fn live_monitor_capture() {
        enable_dpi_awareness();
        let mons = monitors().expect("monitors");
        assert!(!mons.is_empty());
        let shot = grab_monitor(0).expect("grab monitor 0");
        assert_eq!(shot.size, (mons[0].w, mons[0].h));
        assert_eq!(shot.image.dimensions(), shot.size);
        assert!(shot.scale >= 0.25);
    }

    /// The editor's capture paths on the live display (X11 under Xvfb in
    /// CI): the whole desktop, `--region all`/`screenN`, a global crop.
    #[test]
    #[ignore = "live display access"]
    fn live_desktop_capture_and_regions() {
        enable_dpi_awareness();
        let mons = monitors().expect("monitors");
        let all = region_of("all").expect("all");
        assert_eq!(all, union_rect(&mons));
        assert_eq!(region_of("screen0").unwrap(), (mons[0].x, mons[0].y, mons[0].w, mons[0].h));
        assert!(region_of(&format!("screen{}", mons.len())).is_err(), "past the last monitor");
        assert!(region_of("screenX").is_err());
        let shot = grab_edit(None, false).expect("desktop");
        assert_eq!(shot.image.dimensions(), shot.size);
        if uniform_scale(&mons).is_some() {
            assert_eq!((shot.origin, shot.size), ((all.0, all.1), (all.2, all.3)), "the whole span");
            assert_eq!(shot.monitors, monitors_in_shot(&mons, (all.0, all.1), (all.2, all.3)));
        } else {
            assert!(mons.iter().any(|m| (shot.origin, shot.size) == ((m.x, m.y), (m.w, m.h))), "one monitor");
        }
        let one = grab_edit(Some(0), false).expect("monitor 0");
        assert_eq!((one.origin, one.size), ((mons[0].x, mons[0].y), (mons[0].w, mons[0].h)));
        let crop = crop_global(&shot, (shot.origin.0 + 1, shot.origin.1 + 2, 10, 5)).expect("crop");
        assert_eq!(crop.dimensions(), (10, 5));
        assert_eq!(crop.as_raw()[..4], shot.image.as_raw()[(2 * shot.size.0 as usize + 1) * 4..][..4]);
    }
}
