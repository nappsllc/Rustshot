//! The scrolling capture's progress pill: a pulsing dot, "Scrolling…
//! 4 820 px" and a red Stop button, in the recording pill's style
//! (`rec_ui::bar`), beside the region (Windows also keeps it out of the
//! capture; on X11, where nothing can be kept out, it goes only where it
//! does not cover the region, and is left out otherwise: Esc stops).
//!
//! The drawing ([`layout`], [`draw`]) is pure; the window is per platform
//! (a layered window on Windows, an override-redirect window on X11; none
//! on macOS yet).
#![cfg_attr(target_os = "macos", allow(dead_code))] // no pill window there yet

use super::{text, Ctl};
use crate::capture::IRect;
use crate::editor::style::{fill, surface, Ui, BTN};
use crate::objects::{FRect, Pt};
use crate::rec_ui::{ON_REC, REC};
use crate::theme::Theme;
use crate::uifb::{text_height, text_width, Fb, C4};

/// Room around the panel for its shadow (logical px).
pub const MARGIN: f32 = 22.0;
const PAD: f32 = 6.0;
const DOT: f32 = 5.0;
const DOT_L: f32 = 8.0;
const GAP: f32 = 8.0;
const TEXT: f32 = 13.0;
const STOP_SQ: f32 = 10.0;
const STOP_L: f32 = 10.0;
const STOP_R: f32 = 12.0;

/// "4 820 px": thousands apart.
pub fn px_text(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    format!("{out} px")
}

/// The pill's words for `height`.
pub fn label(height: u32) -> String {
    format!("{} {}", text::RUNNING, px_text(height))
}

/// The pill in its window (physical px; the panel `MARGIN` inside).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub size: (u32, u32),
    pub panel: FRect,
    pub stop: FRect,
}

fn tw(ui: &Ui, size: f32, s: &str) -> f32 {
    ui.font.map_or(s.chars().count() as f32 * size * 0.6 * ui.s, |f| text_width(f, size * ui.s, s))
}

fn put_text(f: &mut Fb, ui: &Ui, size: f32, s: &str, x: f32, cy: f32, c: C4) {
    if let Some(font) = ui.font {
        let px = size * ui.s;
        f.draw_text(font, px, s, x, cy - text_height(font, px) / 2.0, c);
    }
}

/// Lay the pill out at `ui.s`; as wide as "Scrolling… 20 000 px", so it
/// does not jitter as the height grows.
pub fn layout(ui: &Ui) -> Layout {
    let s = ui.s;
    let words = (tw(ui, TEXT, &label(20_000)) / s).ceil();
    let stop_w = STOP_L + STOP_SQ + GAP + (tw(ui, 12.0, text::STOP) / s).ceil() + STOP_R;
    let w = PAD + DOT_L + 2.0 * DOT + GAP + words + GAP + stop_w + PAD;
    let h = BTN + 2.0 * PAD;
    let px = |v: f32| v * s;
    let panel = FRect { x: px(MARGIN), y: px(MARGIN), w: px(w), h: px(h) };
    let stop = FRect { x: px(MARGIN + w - PAD - stop_w), y: px(MARGIN + PAD), w: px(stop_w), h: px(BTN) };
    let size = (px(w + 2.0 * MARGIN).ceil() as u32, px(h + 2.0 * MARGIN).ceil() as u32);
    Layout { size, panel, stop }
}

/// Whether `p` (window px) is on Stop.
pub fn on_stop(l: &Layout, p: Pt) -> bool {
    p.x >= l.stop.x && p.x < l.stop.x1() && p.y >= l.stop.y && p.y < l.stop.y1()
}

/// Draw the pill showing `height` (window px); `pulse` lights the halo.
pub fn draw(f: &mut Fb, ui: &Ui, l: &Layout, height: u32, hover: bool, pulse: bool) {
    let s = ui.s;
    surface(f, ui, l.panel, 12.0, &crate::editor::style::ALL_LAYERS, 1.0);
    let cy = l.panel.y + l.panel.h / 2.0;
    let dx = l.panel.x + (PAD + DOT_L + DOT) * s;
    if pulse {
        f.fill_circle(dx, cy, (DOT + 3.0) * s, ui.th.accent.fade(0.25));
    }
    f.fill_circle(dx, cy, DOT * s, ui.th.accent);
    put_text(f, ui, TEXT, &label(height), dx + (DOT + GAP) * s, cy, ui.th.text);
    let r = crate::editor::style::round(l.stop);
    let bg = if hover { C4::new(REC.r.saturating_add(20), REC.g.saturating_add(20), REC.b.saturating_add(20), 255) } else { REC };
    fill(f, r, 8.0 * s, bg);
    let q = (STOP_SQ * s).round();
    let sq = FRect { x: (r.x + STOP_L * s).round(), y: (r.y + (r.h - q) / 2.0).round(), w: q, h: q };
    fill(f, sq, 2.0 * s, ON_REC);
    put_text(f, ui, 12.0, text::STOP, sq.x1() + GAP * s, r.y + r.h / 2.0, ON_REC);
}

/// Where the pill goes for `area`: beside it (else, where the window can
/// stay out of the capture, inside at the bottom; else nowhere).
#[cfg_attr(test, allow(dead_code))]
fn spot(area: IRect, size: (u32, u32), s: f32, inside_ok: bool) -> Option<(i32, i32)> {
    let mons: Vec<IRect> = crate::capture::monitors().unwrap_or_default().iter().map(|m| (m.x, m.y, m.w, m.h)).collect();
    let m = (MARGIN * s).round() as i32;
    let (w, h) = (size.0 as i32 - 2 * m, size.1 as i32 - 2 * m);
    let gap = (8.0 * s).round() as i32;
    let at = if inside_ok {
        Some(crate::rec_ui::place_bar(area, &mons, (w, h), gap, gap))
    } else {
        crate::rec_ui::place_bar_away(area, &mons, (w, h), gap, gap)
    };
    at.map(|(x, y)| (x - m, y - m))
}

/// The UI scale where `area` is.
#[cfg_attr(test, allow(dead_code))]
fn scale_at(area: IRect) -> f32 {
    let (cx, cy) = (area.0 + area.2 as i32 / 2, area.1 + area.3 as i32 / 2);
    let mons = crate::capture::monitors().unwrap_or_default();
    let s = mons.iter().find(|m| cx >= m.x && cy >= m.y && cx < m.x + m.w as i32 && cy < m.y + m.h as i32).map_or(1.0, |m| m.scale);
    s * crate::uisize::mult()
}

/// The pill's window.
pub struct Pill {
    #[cfg(any(windows, target_os = "linux"))]
    win: imp::PillWin,
    th: Theme,
    scale: f32,
    at: (i32, i32),
    drawn: Option<(u32, bool, bool)>,
    started: std::time::Instant,
}

impl Pill {
    /// Show the pill for a run over `area`.
    #[cfg_attr(test, allow(dead_code))] // shown outside tests
    pub fn show(area: IRect, th: Theme, ctl: &Ctl) -> anyhow::Result<Pill> {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let scale = scale_at(area);
            let ui = Ui { th: &th, s: scale, font: Some(crate::fonts::ui_font()) };
            let l = layout(&ui);
            let at = spot(area, l.size, scale, cfg!(windows)).ok_or_else(|| anyhow::anyhow!("no room beside the region"))?;
            let win = imp::PillWin::new()?;
            let mut p = Pill { win, th, scale, at, drawn: None, started: std::time::Instant::now() };
            p.tick(ctl);
            Ok(p)
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = (area, th, ctl);
            anyhow::bail!("no pill on this platform yet")
        }
    }

    /// Follow the run's progress; a click on Stop stops it.
    pub fn tick(&mut self, ctl: &Ctl) {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let ui = Ui { th: &self.th, s: self.scale, font: Some(crate::fonts::ui_font()) };
            let l = layout(&ui);
            let (hover, clicked) = self.win.input(&l);
            if clicked {
                ctl.stop();
            }
            let pulse = (self.started.elapsed().as_millis() / 600).is_multiple_of(2);
            let state = (ctl.progress().height, hover, pulse);
            if self.drawn == Some(state) {
                return;
            }
            self.drawn = Some(state);
            let (w, h) = (l.size.0 as usize, l.size.1 as usize);
            let px = crate::rec_ui::render_layer(w, h, |f| draw(f, &ui, &l, state.0, hover, pulse));
            self.win.put(self.at, l.size, &px);
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = ctl;
    }
}

#[cfg(windows)]
mod imp {
    //! A top-most layered tool window that never takes focus and is kept
    //! out of screen capture; its clicks are read on the owner's tick.
    #![cfg_attr(test, allow(dead_code))]
    use super::{on_stop, Layout};
    use crate::objects::Pt;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const CLASS: PCWSTR = w!("rustshot_scroll_pill");
    static DOWN: AtomicBool = AtomicBool::new(false);
    static UP: AtomicBool = AtomicBool::new(false);
    static MX: AtomicI32 = AtomicI32::new(-1);
    static MY: AtomicI32 = AtomicI32::new(-1);

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let (x, y) = ((lp.0 & 0xFFFF) as u16 as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32);
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE => {
                MX.store(x, Ordering::Relaxed);
                MY.store(y, Ordering::Relaxed);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                MX.store(x, Ordering::Relaxed);
                MY.store(y, Ordering::Relaxed);
                DOWN.store(true, Ordering::Relaxed);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                MX.store(x, Ordering::Relaxed);
                MY.store(y, Ordering::Relaxed);
                UP.store(true, Ordering::Relaxed);
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    pub struct PillWin {
        hwnd: HWND,
        dib: crate::rec_ui::Dib,
        shown: bool,
    }

    impl PillWin {
        pub fn new() -> anyhow::Result<PillWin> {
            static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            let ok = *OK.get_or_init(|| unsafe {
                let Ok(inst) = GetModuleHandleW(None) else { return false };
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: inst.into(),
                    lpszClassName: CLASS,
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                RegisterClassW(&wc) != 0
            });
            anyhow::ensure!(ok, "register the pill's window class");
            let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
            let hwnd = unsafe {
                let inst = GetModuleHandleW(None)?;
                CreateWindowExW(ex, CLASS, w!("Rustshot scrolling capture"), WS_POPUP, 0, 0, 1, 1, None, None, Some(inst.into()), None)?
            };
            // Never in the frames it stitches (Windows 10 2004+).
            let _ = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
            DOWN.store(false, Ordering::Relaxed);
            UP.store(false, Ordering::Relaxed);
            Ok(PillWin { hwnd, dib: crate::rec_ui::Dib::default(), shown: false })
        }

        /// Whether the pointer is on Stop, and whether Stop was clicked.
        pub fn input(&mut self, l: &Layout) -> (bool, bool) {
            let p = Pt::new(MX.load(Ordering::Relaxed) as f32, MY.load(Ordering::Relaxed) as f32);
            let hover = on_stop(l, p);
            let clicked = UP.swap(false, Ordering::Relaxed) && DOWN.swap(false, Ordering::Relaxed) && hover;
            (hover, clicked)
        }

        pub fn put(&mut self, at: (i32, i32), size: (u32, u32), px: &[u8]) {
            self.dib.put(self.hwnd, at.0, at.1, size.0, size.1, px);
            if !self.shown {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.shown = true;
            }
        }
    }

    impl Drop for PillWin {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    //! An override-redirect window on its own Display, placed off the
    //! region (X11 cannot keep it out of the capture).
    use super::{on_stop, Layout};
    use crate::objects::Pt;
    use crate::rec_ui::x11 as xw;

    pub struct PillWin {
        disp: xw::Disp,
        win: xw::Win,
        mouse: Pt,
        pressed: bool,
        clicked: bool,
    }

    impl PillWin {
        pub fn new() -> anyhow::Result<PillWin> {
            let disp = xw::Disp::open()?;
            let mut win = xw::Win::with_kind(&disp, true, None, xw::Kind::Overlay)?;
            win.set_name("Rustshot scrolling capture", "NOTIFICATION");
            Ok(PillWin { disp, win, mouse: Pt::new(-1.0, -1.0), pressed: false, clicked: false })
        }

        pub fn input(&mut self, l: &Layout) -> (bool, bool) {
            while let Some(ev) = xw::next_event(&self.disp) {
                // SAFETY: every event starts with its type; pointer events share this head.
                let (ty, p) = unsafe { (ev.type_, ev.pointer) };
                match ty {
                    xw::EV_MOTION => self.mouse = Pt::new(p.x as f32, p.y as f32),
                    xw::EV_BUTTON_PRESS if p.button == 1 => {
                        self.mouse = Pt::new(p.x as f32, p.y as f32);
                        self.pressed = on_stop(l, self.mouse);
                    }
                    xw::EV_BUTTON_RELEASE if p.button == 1 => {
                        self.mouse = Pt::new(p.x as f32, p.y as f32);
                        self.clicked |= self.pressed && on_stop(l, self.mouse);
                        self.pressed = false;
                    }
                    xw::EV_LEAVE => self.mouse = Pt::new(-1.0, -1.0),
                    _ => {}
                }
            }
            (on_stop(l, self.mouse), std::mem::take(&mut self.clicked))
        }

        pub fn put(&mut self, at: (i32, i32), size: (u32, u32), px: &[u8]) {
            self.win.put(at.0, at.1, size.0, size.1, px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{DARK, LIGHT};

    fn ui(th: &Theme, s: f32) -> Ui<'_> {
        Ui { th, s, font: Some(crate::fonts::ui_font()) }
    }

    #[test]
    fn heights_read_in_thousands() {
        assert_eq!(px_text(0), "0 px");
        assert_eq!(px_text(980), "980 px");
        assert_eq!(px_text(4820), "4 820 px");
        assert_eq!(px_text(20_000), "20 000 px");
        assert_eq!(label(1234), "Scrolling\u{2026} 1 234 px");
    }

    #[test]
    fn stop_is_hit_and_the_size_scales() {
        let l = layout(&ui(&DARK, 1.0));
        let c = Pt::new(l.stop.x + l.stop.w / 2.0, l.stop.y + l.stop.h / 2.0);
        assert!(on_stop(&l, c));
        assert!(!on_stop(&l, Pt::new(l.panel.x + 4.0, c.y)));
        assert_eq!(l.size.1 as f32, BTN + 2.0 * PAD + 2.0 * MARGIN);
        let l2 = layout(&ui(&DARK, 2.0));
        assert!((l2.panel.w - 2.0 * l.panel.w).abs() <= 4.0);
    }

    /// The pill in both themes (preview PNGs under target/previews).
    #[test]
    fn pill_preview() {
        for (name, th) in [("dark", &DARK), ("light", &LIGHT)] {
            let u = ui(th, 1.0);
            let l = layout(&u);
            let (w, h) = (l.size.0 as usize, l.size.1 as usize);
            let px = crate::rec_ui::render_layer(w, h, |f| draw(f, &u, &l, 4820, false, true));
            let stop = (l.stop.x1() - 3.0, l.stop.y + l.stop.h / 2.0);
            let p = &px[(stop.1 as usize * w + stop.0 as usize) * 4..][..4];
            assert_eq!((p[2], p[1], p[0], p[3]), (REC.r, REC.g, REC.b, 255), "{name}: Stop is red");
            assert_eq!(px[3], 0, "{name}: the shadow margin's corner is clear");
            // `RUSTSHOT_PREVIEW_DIR=<dir>`: the PNG to look at.
            if let Some(dir) = std::env::var_os("RUSTSHOT_PREVIEW_DIR") {
                let rgba = crate::rec_ui::layer_over(&px, if th.dark { [30, 30, 34] } else { [235, 235, 238] });
                let png = crate::pixbuf::PixBuf::from_raw(w as u32, h as u32, rgba).expect("size").to_png().expect("png");
                std::fs::create_dir_all(&dir).expect("dir");
                std::fs::write(std::path::Path::new(&dir).join(format!("scroll_pill_{name}.png")), png).expect("write");
            }
        }
    }
}
