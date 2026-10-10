//! Tray menu model shared by the per-OS backends.
#![cfg_attr(not(windows), allow(dead_code))] // used by the macOS/Linux trays (later tasks)

use crate::hotkey::HotEvent;
use std::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// Shown (first) only while a recording runs.
    StopRecording,
    Capture,
    Settings,
    CheckUpdates,
    /// Start at login; payload = currently enabled.
    Autostart(bool),
    Quit,
}

impl MenuItem {
    pub fn label(self) -> &'static str {
        match self {
            MenuItem::StopRecording => "Stop recording",
            MenuItem::Capture => "Capture",
            MenuItem::Settings => "Settings…",
            MenuItem::CheckUpdates => "Check for updates",
            MenuItem::Autostart(_) => "Start at login",
            MenuItem::Quit => "Quit Rustshot",
        }
    }
}

/// Menu contents; `autostart` = None hides "Start at login" (managed
/// installs); `recording` puts "Stop recording" first.
pub fn menu(autostart: Option<bool>, recording: bool) -> Vec<MenuItem> {
    let mut v = Vec::new();
    if recording {
        v.push(MenuItem::StopRecording);
    }
    v.extend([MenuItem::Capture, MenuItem::Settings, MenuItem::CheckUpdates]);
    if let Some(on) = autostart {
        v.push(MenuItem::Autostart(on));
    }
    v.push(MenuItem::Quit);
    v
}

/// Current menu for this install.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn current_menu() -> Vec<MenuItem> {
    let auto = crate::update::managed_install().is_none().then(crate::autostart::is_enabled);
    menu(auto, is_recording())
}

static RECORDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a recording runs (the tray menu and icon follow it).
pub fn is_recording() -> bool {
    RECORDING.load(std::sync::atomic::Ordering::SeqCst)
}

/// A recording started or ended: the menu gets (or loses) "Stop
/// recording" and the tray icon its red dot.
pub fn set_recording(on: bool) {
    RECORDING.store(on, std::sync::atomic::Ordering::SeqCst);
    #[cfg(windows)]
    crate::tray_win::refresh_state();
}

/// "Saved <path>": a tray balloon whose click opens the folder (Windows);
/// stdout elsewhere.
pub fn notify_saved(path: &std::path::Path) {
    #[cfg(windows)]
    crate::tray_win::notify_saved(path);
    #[cfg(not(windows))]
    println!("Saved {}", path.display());
}

/// Start the tray for the running daemon. `tx` receives Capture/Quit.
pub fn spawn(tx: Sender<HotEvent>) {
    #[cfg(windows)]
    crate::tray_win::spawn(tx);
    #[cfg(not(windows))]
    let _ = tx; // macOS / Linux backends arrive in later tasks.
}

/// Tell the user `text` outside any window: a tray balloon on Windows,
/// stderr elsewhere (for now).
pub fn notify(text: &str) {
    #[cfg(windows)]
    crate::tray_win::notify(text);
    #[cfg(not(windows))]
    eprintln!("rustshot: {text}");
}

/// Tray glyph on a 16-unit grid: four rounded corner brackets (stroked) and a centre dot.
const GLYPH_PATH: &str = "M1.5 6V3.5a2 2 0 0 1 2-2H6 M10 1.5h2.5a2 2 0 0 1 2 2V6 M14.5 10v2.5a2 2 0 0 1-2 2H10 M6 14.5H3.5a2 2 0 0 1-2-2V10";
const GLYPH_STROKE: f32 = 1.6;
const GLYPH_DOT_R: f32 = 2.4;

/// Draw the app glyph (the tray icon's shape) in a `size`×`size` box at
/// (x, y), physical px; also the update dialog's app icon.
pub fn draw_glyph(fb: &mut crate::uifb::Fb, x: f32, y: f32, size: f32, c: crate::uifb::C4) {
    use crate::raster::Blend;
    let k = size / 16.0;
    if let Some(lines) = crate::icon_path::parse_path(GLYPH_PATH) {
        let mut surf = fb.surf();
        for mut line in lines {
            for p in &mut line {
                p.x = x + p.x * k;
                p.y = y + p.y * k;
            }
            surf.stroke_polyline(&line, GLYPH_STROKE * k, c, Blend::Normal);
        }
    }
    fb.fill_circle(x + 8.0 * k, y + 8.0 * k, GLYPH_DOT_R * k, c);
}

/// Monochrome tray glyph as straight-alpha RGBA, `size`x`size`, in colour `rgb`.
/// Coverage is rasterised white-on-black into an opaque scratch buffer (the
/// rasterizer assumes an opaque destination) and its red channel becomes alpha.
#[cfg_attr(not(test), allow(dead_code))] // tests
pub fn tray_glyph_rgba(size: u32, rgb: (u8, u8, u8)) -> Vec<u8> {
    tray_glyph_rgba_rec(size, rgb, false)
}

/// The recording dot's colour.
pub const REC_RED: (u8, u8, u8) = (0xF0, 0x3E, 0x3E);
const REC_DOT_R: f32 = 3.6;

/// [`tray_glyph_rgba`]; with `recording` the centre dot is a larger red
/// one (the tray icon while a recording runs).
pub fn tray_glyph_rgba_rec(size: u32, rgb: (u8, u8, u8), recording: bool) -> Vec<u8> {
    use crate::uifb::{C4, Fb};
    let n = size as usize;
    let white = C4::rgb(255, 255, 255);
    let mut scratch = [0u8, 0, 0, 255].repeat(n * n);
    if !recording {
        draw_glyph(&mut Fb::new(&mut scratch, n), 0.0, 0.0, size as f32, white);
        let mut out = Vec::with_capacity(n * n * 4);
        for px in scratch.as_chunks::<4>().0 {
            out.extend_from_slice(&[rgb.0, rgb.1, rgb.2, px[0]]);
        }
        return out;
    }
    // The brackets in `rgb`, the red dot over them (coverage of each).
    let k = size as f32 / 16.0;
    draw_brackets(&mut Fb::new(&mut scratch, n), size as f32, white);
    let mut dot = [0u8, 0, 0, 255].repeat(n * n);
    Fb::new(&mut dot, n).fill_circle(8.0 * k, 8.0 * k, REC_DOT_R * k, white);
    let mut out = Vec::with_capacity(n * n * 4);
    for (g, d) in scratch.as_chunks::<4>().0.iter().zip(dot.as_chunks::<4>().0) {
        let (ga, da) = (g[0] as f32 / 255.0, d[0] as f32 / 255.0);
        let a = da + ga * (1.0 - da);
        let mix = |base: u8, red: u8| {
            if a > 0.0 { ((red as f32 * da + base as f32 * ga * (1.0 - da)) / a).round() as u8 } else { 0 }
        };
        out.extend_from_slice(&[
            mix(rgb.0, REC_RED.0),
            mix(rgb.1, REC_RED.1),
            mix(rgb.2, REC_RED.2),
            (a * 255.0).round() as u8,
        ]);
    }
    out
}

/// The glyph's corner brackets alone (no centre dot), at the origin.
fn draw_brackets(fb: &mut crate::uifb::Fb, size: f32, c: crate::uifb::C4) {
    use crate::raster::Blend;
    let k = size / 16.0;
    if let Some(lines) = crate::icon_path::parse_path(GLYPH_PATH) {
        let mut surf = fb.surf();
        for mut line in lines {
            for p in &mut line {
                p.x *= k;
                p.y *= k;
            }
            surf.stroke_polyline(&line, GLYPH_STROKE * k, c, Blend::Normal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_composition() {
        assert_eq!(
            menu(Some(true), false),
            vec![
                MenuItem::Capture,
                MenuItem::Settings,
                MenuItem::CheckUpdates,
                MenuItem::Autostart(true),
                MenuItem::Quit
            ]
        );
        assert_eq!(menu(None, false).len(), 4);
        assert!(!menu(None, false).iter().any(|m| matches!(m, MenuItem::Autostart(_))));
        assert_eq!(*menu(None, false).last().unwrap(), MenuItem::Quit);
        // While recording, "Stop recording" comes first.
        let m = menu(None, true);
        assert_eq!(m[0], MenuItem::StopRecording);
        assert_eq!(&m[1..], &menu(None, false)[..]);
        assert!(!menu(Some(false), false).contains(&MenuItem::StopRecording));
    }

    #[test]
    fn recording_icon_has_a_red_centre() {
        for size in [16u32, 32] {
            let plain = tray_glyph_rgba(size, (255, 255, 255));
            let rec = tray_glyph_rgba_rec(size, (255, 255, 255), true);
            let c = size / 2;
            let i = ((c * size + c) * 4) as usize;
            assert_eq!(&plain[i..i + 4], &[255, 255, 255, 255]);
            assert_eq!(&rec[i..i + 4], &[REC_RED.0, REC_RED.1, REC_RED.2, 255]);
            // The brackets keep the theme colour.
            let u = size as f32 / 16.0;
            let j = ((((2.1 * u) as u32) * size + (2.1 * u) as u32) * 4) as usize;
            assert!(rec[j + 3] > 100 && rec[j] == 255 && rec[j + 1] == 255, "{:?}", &rec[j..j + 4]);
            // The red dot is larger than the plain one.
            let off = ((c * size + c + (3.0 * u) as u32) * 4 + 3) as usize;
            assert!(rec[off] > plain[off], "dot radius");
        }
    }

    #[test]
    fn labels() {
        assert_eq!(MenuItem::StopRecording.label(), "Stop recording");
        assert_eq!(MenuItem::Capture.label(), "Capture");
        assert_eq!(MenuItem::Settings.label(), "Settings…");
        assert_eq!(MenuItem::CheckUpdates.label(), "Check for updates");
        assert_eq!(MenuItem::Autostart(false).label(), "Start at login");
        assert_eq!(MenuItem::Quit.label(), "Quit Rustshot");
    }

    fn alpha(buf: &[u8], size: u32, x: u32, y: u32) -> u8 {
        buf[((y * size + x) * 4 + 3) as usize]
    }

    #[test]
    fn glyph_shape() {
        for size in [16u32, 32] {
            let b = tray_glyph_rgba(size, (27, 28, 32));
            assert_eq!(b.len(), (size * size * 4) as usize);
            let c = size / 2;
            let i = ((c * size + c) * 4) as usize;
            assert_eq!(&b[i..i + 4], &[27, 28, 32, 255]);
            let u = size as f32 / 16.0;
            // Corner brackets: the arc apex and the straight runs have coverage.
            assert!(alpha(&b, size, (2.1 * u) as u32, (2.1 * u) as u32) > 100);
            assert!(alpha(&b, size, (13.9 * u) as u32, (13.9 * u) as u32) > 100);
            // Gap between bracket and dot, and the image corners, are empty.
            assert_eq!(alpha(&b, size, (4.5 * u) as u32, (4.5 * u) as u32), 0);
            for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
                assert_eq!(alpha(&b, size, x, y), 0);
            }
        }
    }

    /// Writes enlarged previews for eyeballing; run with --ignored.
    #[test]
    #[ignore = "writes preview PNGs to the scratchpad"]
    fn write_glyph_previews() {
        let dir = std::path::Path::new(
            "C:/Users/plato/AppData/Local/Temp/claude/E--Personal-rustshot/388c4929-22ab-4934-9dbf-1b81d7818b10/scratchpad/glyph",
        );
        std::fs::create_dir_all(dir).unwrap();
        for size in [16u32, 20, 24, 32] {
            for (name, fg, bg, rec) in [
                ("dark", (255u8, 255u8, 255u8), 0x20u8, false),
                ("light", (27, 28, 32), 0xF3, false),
                ("dark-rec", (255, 255, 255), 0x20, true),
                ("light-rec", (27, 28, 32), 0xF3, true),
            ] {
                let g = tray_glyph_rgba_rec(size, fg, rec);
                let scale = 8u32;
                let w = size * scale;
                let mut img = Vec::with_capacity((w * w * 3) as usize);
                for y in 0..w {
                    for x in 0..w {
                        let i = (((y / scale) * size + x / scale) * 4) as usize;
                        let a = g[i + 3] as u32;
                        for ch in 0..3 {
                            let v = (g[i + ch] as u32 * a + bg as u32 * (255 - a) + 127) / 255;
                            img.push(v as u8);
                        }
                    }
                }
                let f = std::fs::File::create(dir.join(format!("glyph_{size}_{name}.png"))).unwrap();
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, w);
                enc.set_color(png::ColorType::Rgb);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header().unwrap().write_image_data(&img).unwrap();
            }
        }
    }
}
